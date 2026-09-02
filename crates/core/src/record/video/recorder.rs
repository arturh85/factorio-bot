//! The recorder: one ffmpeg child, one tick sampler, and the honesty machinery
//! that makes a recording which outlived its run or died mid-run *detectable*.
//!
//! Three independent mechanisms, none of which needs the others to work:
//!
//! 1. **`video/run.json`, wiped and rewritten at start.** [`super::archive_video`]
//!    copies only if it names this run, so an orphaned recording cannot be
//!    archived into a run it does not belong to.
//! 2. **[`super::VideoStatus`], written `recording` at start.** A status still
//!    saying `recording` in an archived run is proof the recorder was never
//!    stopped.
//! 3. **Progress liveness.** ffmpeg's `-progress` pipe carries `out_time_us`.
//!    If it stops advancing while the run is live, the recorder writes a
//!    [`super::TickKind::Gap`] line, sets `died`, and narrates. This is what
//!    catches ffmpeg dying *silently* -- as distinct from ffmpeg exiting, which
//!    the child handle catches on its own.
//!
//! **A failure to start is never fatal to the run.** The run continues with
//! frames, `video.json` is written with `status: "failed"` and the reason, and
//! both a `tracing::error!` and a `paris` narration line say so at the moment it
//! happens. A run that silently lost its video and only reveals it when
//! somebody opens the viewer a day later is the failure this avoids.

use super::clock::{TickKind, TickLog, TickSample};
use super::ffmpeg;
use super::window::{Resolution, Tools, WindowQuery, find_client_pid, window_id_hex};
use super::{
    Calibration, RECORD_FILE, VIDEO_FILE, VideoRecord, VideoStatus, open_video_dir, rate_ok,
    write_video_record,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Where the ticks come from.
///
/// A trait, not `FactorioRcon` directly, so the sampler's cadence, its midpoint
/// stamping and its gap handling can be tested without a game -- and so a
/// future denser source (the free pairs every `remote_call_timed` reply already
/// carries) can be wired in without reshaping the sampler.
#[async_trait::async_trait]
pub trait TickSource: Send + Sync {
    /// The game's tick *now*, or `None` when the game did not answer.
    ///
    /// Never a zero for "don't know": zero is tick zero.
    async fn tick_now(&self) -> Option<u64>;
}

#[async_trait::async_trait]
impl TickSource for crate::factorio::rcon::FactorioRcon {
    async fn tick_now(&self) -> Option<u64> {
        // `game_tick` is a plain `/silent-command`, so it needs no mod and
        // works against a save whose `workspace/mods` copy predates this
        // binary.
        self.game_tick().await.ok().flatten()
    }
}

/// What the caller asked for.
#[derive(Debug, Clone)]
pub struct VideoOptions {
    pub resolution: Resolution,
    pub fps: u32,
    /// Which `client<N>` window to film. v0 films it unsteered: the camera
    /// follows whatever that client shows.
    ///
    /// **1 by default**, and the default is a choice rather than an accident:
    /// client 1 is the only client a run is guaranteed to have (a run with any
    /// graphical client at all has that one), it is registered first with the
    /// mod so it is the lowest player index, and it is the same client on every
    /// run -- so two recordings of two runs are comparable without reading
    /// their manifests. Nothing about it is special otherwise, which is why it
    /// is configurable.
    pub client: u8,
    /// The client process's pid. Supplied by a caller that knows it, and
    /// otherwise **discovered** from the workspace directory the client runs
    /// out of ([`super::window::find_client_pid`]) -- so this being `None` at
    /// the call site does not mean the search runs blind.
    ///
    /// It is the search that matters: **SDL does set `_NET_WM_PID` under
    /// Xwayland here** (probed live, one window per graphical client), so the
    /// pid search is the one that can tell four identical Factorio windows
    /// apart. The name search is a fallback that provably cannot -- it returns
    /// all four.
    pub pid: Option<u32>,
    pub window_name: String,
    /// `None` reads `DISPLAY` from the environment; no display at all is a
    /// clean, narrated failure rather than a silent no-op.
    pub display: Option<String>,
    /// The encoder binary. Named because **the binary's PATH at run time is not
    /// guaranteed to be the dev shell's**: `flake.nix` supplies an
    /// `ffmpeg-full` with x11grab, and the stock nixpkgs `ffmpeg` has no
    /// x11grab at all.
    pub ffmpeg: String,
    pub tools: Tools,
    /// The floor cadence for the tick sampler. See [`Self::default`].
    pub floor_interval: Duration,
    /// How long to keep looking for the window. A client takes ~26 s to load
    /// sprites, so this must outlast that.
    pub window_timeout: Duration,
    /// How long `out_time_us` may stand still before the encoder is declared
    /// dead.
    pub liveness_timeout: Duration,
    /// Below this much free space, stop the recorder and let the run continue.
    /// `events.jsonl` and `samples.jsonl` are the artefacts that must survive.
    pub min_free_bytes: u64,
}

impl Default for VideoOptions {
    /// The defaults the design settles on.
    ///
    /// `floor_interval` is 500 ms -- a **2 Hz floor**. Between two samples the
    /// viewer interpolates linearly, and if the instantaneous rate stays within
    /// `[u_min, u_max]` ticks/second the worst-case error of that interpolation
    /// is `Δtick · (1/u_min − 1/u_max) / 4`. With the range this project has
    /// actually seen (53.6 UPS sustained under capture, 60 nominal), 2 Hz gives
    /// ~30 ticks between samples and **~25 ms** worst case -- under one frame
    /// period at 15 fps *and* at 30 fps, so raising the frame rate later does
    /// not silently invalidate the clock. 4 Hz would halve an error that is
    /// already not the bottleneck, at double the command rate on a queue shared
    /// with the executor.
    ///
    /// The bound is conditional and the condition can fail: if the game stalls
    /// outright, `u_min → 0` and the formula diverges. No cadence fixes that,
    /// which is why [`TickKind::Gap`] is not optional.
    fn default() -> Self {
        Self {
            resolution: Resolution::default(),
            fps: 15,
            client: 1,
            pid: None,
            window_name: "Factorio".to_string(),
            display: None,
            ffmpeg: "ffmpeg".to_string(),
            tools: Tools::default(),
            floor_interval: Duration::from_millis(500),
            window_timeout: Duration::from_secs(30),
            liveness_timeout: Duration::from_secs(10),
            min_free_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

/// Free bytes on the filesystem holding `path`, or `None` when it cannot be
/// determined.
///
/// Reads `df -kP`, which is POSIX-specified output, rather than taking a
/// `statvfs` dependency this crate does not otherwise have. `None` is
/// *unknown*, and the caller treats unknown as "do not stop the recording" --
/// refusing to record because a shell tool was missing would be a worse failure
/// than filling a disk that may not even be short.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .arg("-kP")
        .arg(path)
        .output()
        .ok()?;
    parse_df_available_kb(&String::from_utf8_lossy(&out.stdout)).map(|kb| kb * 1024)
}

/// Pulls the "Available" column out of `df -kP` output.
///
/// Field *index*, not a header search: `-P` fixes the column order, and the
/// header text is localised.
pub fn parse_df_available_kb(stdout: &str) -> Option<u64> {
    let line = stdout.lines().nth(1)?;
    line.split_whitespace().nth(3)?.parse().ok()
}

/// The pieces the background tasks share with [`VideoRecorder`].
struct Shared {
    log: Mutex<TickLog>,
    /// ffmpeg's latest `out_time_us`, or 0 before the first reading.
    out_time_us: AtomicU64,
    /// `wall_ms` at which `out_time_us` last advanced.
    progress_at_ms: AtomicU64,
    /// Whether any progress has been seen at all.
    progress_seen: AtomicBool,
    /// Set when the sampler should stop.
    stopping: AtomicBool,
    /// Set once a liveness failure has been reported, so it is narrated once.
    death_reported: AtomicBool,
    /// A reason the encoder must be shut down early, set by the sampler.
    early_stop: Mutex<Option<(VideoStatus, String)>>,
    /// The first calibration pair, taken at the first `out_time_us` reading.
    first_calibration: Mutex<Option<Calibration>>,
}

/// A running (or failed) recording.
pub struct VideoRecorder {
    dir: PathBuf,
    epoch: Instant,
    record: VideoRecord,
    child: Arc<Mutex<Option<Child>>>,
    shared: Arc<Shared>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    finished: bool,
}

impl VideoRecorder {
    /// Milliseconds since this recorder's epoch.
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// The directory this recording lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What has been recorded about the recording so far.
    pub fn record(&self) -> &VideoRecord {
        &self.record
    }

    /// Whether an encoder is actually running.
    pub fn is_recording(&self) -> bool {
        self.record.status == VideoStatus::Recording
    }

    /// Prepares `<workspace>/video/`, resolves and sizes the window, probes the
    /// encoder and starts it.
    ///
    /// `Err` only when the video directory itself cannot be written. **Every
    /// capture failure returns `Ok`** with `status: failed` and a reason: the
    /// run goes on with frames.
    ///
    /// `opened_at` is the run's opening tick, written as the `start` line so
    /// the clock and the run agree about where the recording begins.
    pub async fn start(
        workspace: &Path,
        run_id: &str,
        options: VideoOptions,
        source: Arc<dyn TickSource>,
        opened_at: Option<u64>,
    ) -> io::Result<Self> {
        let dir = open_video_dir(workspace, run_id)?;
        let epoch = Instant::now();
        let (requested_width, requested_height) = options.resolution.size();

        // The pid is what makes the window search able to tell four identical
        // Factorio windows apart; the name search cannot, and is measured
        // returning all four on this very machine. Discovering it here rather
        // than being handed it keeps that ability on every path -- including a
        // run attached to a game this process did not spawn. Never fatal: a
        // failure only means the search falls back to the name, which is a
        // real answer on a single-client run.
        let mut options = options;
        if options.pid.is_none() {
            match find_client_pid(workspace, options.client) {
                Ok(pid) => {
                    tracing::info!(
                        client = options.client,
                        pid,
                        "filming the window of the client running out of this workspace"
                    );
                    options.pid = Some(pid);
                }
                Err(err) => {
                    warn!(
                        "could not identify client{}'s process ({}); falling back to the window \
                         name, which cannot tell two clients apart",
                        options.client, err
                    );
                    tracing::warn!(client = options.client, error = %err, "no client pid; the window search falls back to the name");
                }
            }
        }
        let options = options;

        let mut log = TickLog::create(&dir)?;
        if let Some(tick) = opened_at {
            log.append(&TickSample {
                tick: Some(tick),
                wall_ms: 0,
                kind: TickKind::Start,
                reason: None,
            })?;
        }

        let mut recorder = Self {
            dir,
            epoch,
            record: VideoRecord {
                run: run_id.to_string(),
                file: VIDEO_FILE.to_string(),
                width: 0,
                height: 0,
                requested_width,
                requested_height,
                fps: options.fps,
                status: VideoStatus::Recording,
                reason: None,
                ffmpeg_exit: None,
                calibration: Vec::new(),
                rate_ok: None,
                window_id: None,
            },
            child: Arc::new(Mutex::new(None)),
            shared: Arc::new(Shared {
                log: Mutex::new(log),
                out_time_us: AtomicU64::new(0),
                progress_at_ms: AtomicU64::new(0),
                progress_seen: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
                death_reported: AtomicBool::new(false),
                early_stop: Mutex::new(None),
                first_calibration: Mutex::new(None),
            }),
            tasks: Vec::new(),
            finished: false,
        };

        match recorder.spawn_encoder(&options).await {
            Ok(()) => {
                recorder.spawn_tasks(options, source);
                info!(
                    "<bright-blue>recording</> video to {} at {}x{}@{}fps",
                    recorder.dir.join(VIDEO_FILE).display(),
                    recorder.record.width,
                    recorder.record.height,
                    recorder.record.fps
                );
                if !recorder.record.geometry_as_requested() {
                    // A warning on the run, not a failure: the video is still
                    // usable and still joins on ticks. A tiling compositor
                    // ignores a size request unless the window is floated.
                    warn!(
                        "video window is {}x{}, not the {}x{} requested -- recording what it is",
                        recorder.record.width,
                        recorder.record.height,
                        requested_width,
                        requested_height
                    );
                    tracing::warn!(
                        observed_width = recorder.record.width,
                        observed_height = recorder.record.height,
                        requested_width,
                        requested_height,
                        "the window manager did not honour the requested video size"
                    );
                }
            }
            Err(reason) => {
                recorder.record.status = VideoStatus::Failed;
                recorder.record.reason = Some(reason.clone());
                error!("<red>video capture unavailable</>: {}", reason);
                tracing::error!(reason = %reason, "video capture could not start; the run continues with frames only");
            }
        }
        write_video_record(&recorder.dir, &recorder.record)?;
        Ok(recorder)
    }

    /// Resolves the display and window, sizes it, probes the encoder and spawns
    /// it. `Err(reason)` is a human-readable explanation, already suitable for
    /// `video.json`.
    async fn spawn_encoder(&mut self, options: &VideoOptions) -> Result<(), String> {
        let display = options
            .display
            .clone()
            .or_else(|| std::env::var("DISPLAY").ok())
            .filter(|d| !d.is_empty())
            .ok_or_else(|| "no DISPLAY: nothing is rendering to capture".to_string())?;

        let query = WindowQuery {
            pid: options.pid,
            name: options.window_name.clone(),
        };
        let window = self.await_window(options, &query).await?;
        self.record.window_id = Some(window_id_hex(window));

        let (width, height) = options.resolution.size();
        // Best-effort: the answer is whatever `observe` says afterwards, and a
        // window manager that refuses is a warning, not a failure.
        if let Err(err) = options.tools.resize(window, width, height) {
            tracing::warn!(error = %err, "could not ask the window manager to resize the capture window");
        }
        let observed = options
            .tools
            .observe(window)
            .map_err(|err| format!("could not read the window geometry: {err}"))?;
        // **The recorded geometry is the observed one, never the requested
        // one** -- the same rule as logging where a bot landed rather than
        // where it was sent.
        self.record.width = observed.width;
        self.record.height = observed.height;

        let settings = ffmpeg::EncodeSettings {
            window_id: window_id_hex(window),
            display,
            fps: options.fps,
            width: observed.width,
            height: observed.height,
        };

        // A one-frame trial grab, not a version string: the stock nixpkgs
        // ffmpeg has no x11grab and would fail only at the first grab, i.e. at
        // the end of a 45-minute run.
        let probe = Command::new(&options.ffmpeg)
            .args(ffmpeg::probe_args(&settings))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|err| format!("could not run {}: {err}", options.ffmpeg))?;
        if !probe.status.success() {
            return Err(format!(
                "{} could not grab a frame from {}: {}",
                options.ffmpeg,
                settings.window_id,
                String::from_utf8_lossy(&probe.stderr).trim()
            ));
        }

        let mut child = Command::new(&options.ffmpeg)
            .args(ffmpeg::record_args(&settings, &self.dir.join(VIDEO_FILE)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| format!("could not start {}: {err}", options.ffmpeg))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "ffmpeg gave no progress pipe".to_string())?;
        let shared = self.shared.clone();
        let epoch = self.epoch;
        self.tasks.push(tokio::spawn(async move {
            read_progress(stdout, shared, epoch).await;
        }));
        *self.child.lock().await = Some(child);
        Ok(())
    }

    /// Looks for the window, backing off, until `window_timeout` runs out.
    ///
    /// A client takes ~26 s to load its sprites, so "not found yet" is the
    /// ordinary early state. **More than one match is fatal immediately** and
    /// is not retried: retrying an ambiguity does not resolve it, and picking
    /// one films an unknown peer.
    async fn await_window(
        &self,
        options: &VideoOptions,
        query: &WindowQuery,
    ) -> Result<u64, String> {
        use super::window::WindowError;
        let deadline = Instant::now() + options.window_timeout;
        loop {
            let last = match options.tools.resolve_window(query) {
                Ok(id) => return Ok(id),
                // Neither of these improves by waiting: an ambiguity does not
                // resolve itself, and a tool that is not on PATH will not
                // appear there.
                Err(err @ (WindowError::Ambiguous { .. } | WindowError::ToolMissing { .. })) => {
                    return Err(err.to_string());
                }
                Err(err) => err.to_string(),
            };
            if Instant::now() >= deadline {
                return Err(format!(
                    "gave up looking for the client {} window after {:?}: {last}",
                    options.client, options.window_timeout
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Starts the tick sampler. Only called when an encoder is actually
    /// running: a clock with no video to join is 2 RCON commands a second
    /// bought for nothing.
    fn spawn_tasks(&mut self, options: VideoOptions, source: Arc<dyn TickSource>) {
        let shared = self.shared.clone();
        let child = self.child.clone();
        let epoch = self.epoch;
        let dir = self.dir.clone();
        self.tasks.push(tokio::spawn(async move {
            sample_ticks(shared, child, source, options, epoch, dir).await;
        }));
    }

    /// Stops the recording and writes the final `video.json`.
    ///
    /// The ladder degrades rather than fails, because the container is
    /// fragmented: even the kill path leaves a playable file.
    ///
    /// `tick` is the run's closing tick, written as the `stop` line.
    pub async fn stop(&mut self, tick: Option<u64>) -> io::Result<VideoRecord> {
        if self.finished {
            return Ok(self.record.clone());
        }
        self.finished = true;
        self.shared.stopping.store(true, Ordering::Relaxed);

        let stopped_at = self.now_ms();
        if self.record.status == VideoStatus::Recording {
            // Take the second calibration pair *before* the encoder is asked to
            // stop, so it describes the recording rather than the teardown.
            if self.shared.progress_seen.load(Ordering::Relaxed) {
                let out_time_ms = self.shared.out_time_us.load(Ordering::Relaxed) / 1000;
                if let Some(first) = *self.shared.first_calibration.lock().await {
                    self.record.calibration = vec![
                        first,
                        Calibration {
                            host_wall_ms: stopped_at,
                            out_time_ms,
                        },
                    ];
                }
            }
        }

        if let Some((status, reason)) = self.shared.early_stop.lock().await.take() {
            self.record.status = status;
            self.record.reason = Some(reason);
        }

        {
            let mut log = self.shared.log.lock().await;
            log.append(&TickSample {
                tick,
                wall_ms: stopped_at,
                kind: TickKind::Stop,
                reason: None,
            })?;
        }

        let outcome = self.shut_down_encoder().await;
        for task in self.tasks.drain(..) {
            task.abort();
        }

        if let Some((clean, code)) = outcome {
            self.record.ffmpeg_exit = code;
            // An early stop (low disk, a death already detected) keeps the
            // status it was given: "why did this recording end" is a more
            // useful answer than "it ended".
            if self.record.status == VideoStatus::Recording {
                self.record.status = if clean {
                    VideoStatus::Stopped
                } else {
                    VideoStatus::Killed
                };
            }
        }
        self.record.rate_ok = rate_ok(&self.record.calibration);
        write_video_record(&self.dir, &self.record)?;
        info!(
            "video recording {} ({})",
            self.record
                .status
                .clone_as_str()
                .unwrap_or("finished")
                .to_string(),
            self.dir.join(RECORD_FILE).display()
        );
        Ok(self.record.clone())
    }

    /// `q`, then a kill. `None` when there was no encoder at all.
    ///
    /// Returns `(exited cleanly, exit code)`.
    ///
    /// There is no SIGTERM rung between the two, and that is a deliberate
    /// narrowing of the design's ladder: sending one needs a `libc` dependency
    /// this crate does not have, and the fragmented container makes the kill
    /// path leave a playable file anyway -- which is the property the ladder
    /// existed to protect.
    async fn shut_down_encoder(&mut self) -> Option<(bool, Option<i32>)> {
        let mut guard = self.child.lock().await;
        let child = guard.as_mut()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"q\n").await;
            let _ = stdin.flush().await;
            drop(stdin);
        }
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => Some((true, status.code())),
            Ok(Err(_)) => Some((false, None)),
            Err(_) => {
                let _ = child.start_kill();
                let code = tokio::time::timeout(Duration::from_secs(2), child.wait())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|s| s.code());
                Some((false, code))
            }
        }
    }
}

impl VideoStatus {
    fn clone_as_str(&self) -> Option<&'static str> {
        Some(match self {
            VideoStatus::Recording => "still recording",
            VideoStatus::Stopped => "stopped cleanly",
            VideoStatus::Killed => "killed",
            VideoStatus::Died => "died",
            VideoStatus::Failed => "never started",
            VideoStatus::StoppedLowDisk => "stopped: low disk",
        })
    }
}

/// Reads ffmpeg's `-progress` pipe, keeping the encoder clock and the moment it
/// last moved.
async fn read_progress(stdout: tokio::process::ChildStdout, shared: Arc<Shared>, epoch: Instant) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(us) = ffmpeg::parse_out_time_us(&line) else {
            continue;
        };
        let now_ms = epoch.elapsed().as_millis() as u64;
        let previous = shared.out_time_us.swap(us, Ordering::Relaxed);
        if !shared.progress_seen.swap(true, Ordering::Relaxed) {
            // The first reading fixes the video's zero. Guessing this offset is
            // the mistake that makes every seek wrong by a constant nobody can
            // see, so it is measured.
            *shared.first_calibration.lock().await = Some(Calibration {
                host_wall_ms: now_ms,
                out_time_ms: us / 1000,
            });
            shared.progress_at_ms.store(now_ms, Ordering::Relaxed);
        } else if us != previous {
            shared.progress_at_ms.store(now_ms, Ordering::Relaxed);
        }
    }
}

/// The floor: a deliberate `game.tick` poll at a fixed cadence, so the table
/// never goes sparse when the executor is quiet.
///
/// Also the place liveness and free disk are watched, because it is the one
/// task that already wakes on a timer.
async fn sample_ticks(
    shared: Arc<Shared>,
    child: Arc<Mutex<Option<Child>>>,
    source: Arc<dyn TickSource>,
    options: VideoOptions,
    epoch: Instant,
    dir: PathBuf,
) {
    let mut since_disk_check = 0u32;
    // Free space is checked every ~10 s rather than every poll: `df` is a
    // process spawn, and at 2 Hz that would be 120 of them a minute.
    let disk_check_every = (10_000 / options.floor_interval.as_millis().max(1)) as u32;

    while !shared.stopping.load(Ordering::Relaxed) {
        tokio::time::sleep(options.floor_interval).await;
        if shared.stopping.load(Ordering::Relaxed) {
            break;
        }

        // The pair is one observation: the tick is the value inside the game at
        // the moment the command ran, which is between these two readings.
        let before = epoch.elapsed();
        let tick = source.tick_now().await;
        let after = epoch.elapsed();
        let wall_ms = ((before + after) / 2).as_millis() as u64;

        let sample = match tick {
            Some(tick) => TickSample::observed(tick, wall_ms),
            None => TickSample::gap(wall_ms, "the game did not answer a tick query"),
        };
        if let Err(err) = shared.log.lock().await.append(&sample) {
            tracing::error!(error = %err, "could not append to the video tick log");
        }

        let now_ms = after.as_millis() as u64;
        if shared.progress_seen.load(Ordering::Relaxed) {
            let last = shared.progress_at_ms.load(Ordering::Relaxed);
            let still_for = now_ms.saturating_sub(last);
            if still_for > options.liveness_timeout.as_millis() as u64
                && !shared.death_reported.swap(true, Ordering::Relaxed)
            {
                let reason = format!(
                    "the encoder stopped advancing for {:.1} s while the run was live",
                    still_for as f64 / 1000.0
                );
                // A gap line, because from here the video's timestamps mean
                // nothing: it is showing a frozen image at a moment the clock
                // still says is fine.
                let _ = shared
                    .log
                    .lock()
                    .await
                    .append(&TickSample::gap(now_ms, reason.clone()));
                error!("<red>video encoder died</>: {}", reason);
                tracing::error!(reason = %reason, "the video encoder stopped advancing");
                *shared.early_stop.lock().await = Some((VideoStatus::Died, reason));
            }
        }

        since_disk_check += 1;
        if since_disk_check >= disk_check_every.max(1) {
            since_disk_check = 0;
            if let Some(free) = free_bytes(&dir)
                && free < options.min_free_bytes
            {
                let reason = format!(
                    "only {} MiB free on the workspace filesystem",
                    free / (1024 * 1024)
                );
                warn!("<yellow>stopping video capture</>: {}", reason);
                tracing::warn!(
                    free_bytes = free,
                    "stopping video capture to protect the run's own records"
                );
                *shared.early_stop.lock().await = Some((VideoStatus::StoppedLowDisk, reason));
                // Kill the encoder now rather than at finish: the point is to
                // stop *filling the disk*, and the run's own `events.jsonl` and
                // `samples.jsonl` are what must survive.
                if let Some(child) = child.lock().await.as_mut() {
                    let _ = child.start_kill();
                }
                shared.stopping.store(true, Ordering::Relaxed);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::video::{VideoManifest, read_video_dir};

    struct Silent;

    #[async_trait::async_trait]
    impl TickSource for Silent {
        async fn tick_now(&self) -> Option<u64> {
            None
        }
    }

    fn workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-vrec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn df_output_yields_the_available_column_by_position() {
        let out = "\
Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/nvme0n1p2   982891504 424273472 508654576      46% /
";
        assert_eq!(parse_df_available_kb(out), Some(508_654_576));
        assert_eq!(parse_df_available_kb(""), None);
        assert_eq!(parse_df_available_kb("only a header\n"), None);
    }

    /// A missing display is a **clean, narrated failure with a written
    /// reason**, never a silent no-op -- today a headless server already
    /// produces zero frames and the only evidence is an empty directory much
    /// later.
    #[tokio::test]
    async fn no_display_fails_the_recording_but_not_the_run() {
        let ws = workspace("nodisplay");
        let recorder = VideoRecorder::start(
            &ws,
            "run-1",
            VideoOptions {
                display: Some(String::new()),
                ..VideoOptions::default()
            },
            Arc::new(Silent),
            Some(60551),
        )
        .await
        .expect("a failed capture is not an error for the run");

        assert_eq!(recorder.record().status, VideoStatus::Failed);
        assert!(
            recorder
                .record()
                .reason
                .as_deref()
                .unwrap()
                .contains("DISPLAY"),
            "{:?}",
            recorder.record().reason
        );
        assert!(!recorder.is_recording());

        // Written at the moment it happened, not at finish: a run that lost its
        // video must not reveal it only when somebody opens the viewer.
        let manifest = read_video_dir(recorder.dir());
        assert_eq!(manifest.run.as_deref(), Some("run-1"));
        assert_eq!(manifest.video.unwrap().status, VideoStatus::Failed);
        assert_eq!(manifest.bytes, None, "no encoder ran, so there is no file");
    }

    /// The opening tick is written as a `start` line at `wall_ms` 0, so the
    /// clock and the run agree about where the recording begins.
    #[tokio::test]
    async fn the_opening_tick_anchors_the_clock() {
        let ws = workspace("opening");
        let recorder = VideoRecorder::start(
            &ws,
            "run-2",
            VideoOptions {
                display: Some(String::new()),
                ..VideoOptions::default()
            },
            Arc::new(Silent),
            Some(60551),
        )
        .await
        .unwrap();

        let read =
            super::super::clock::read_tick_samples(&recorder.dir().join(super::super::TICKS_FILE))
                .unwrap();
        assert_eq!(read.samples.len(), 1);
        assert_eq!(read.samples[0].kind, TickKind::Start);
        assert_eq!(read.samples[0].tick, Some(60551));
        assert_eq!(read.samples[0].wall_ms, 0);
    }

    /// Stopping a recording that never started still writes a final record --
    /// and must not overwrite *why* it failed with a generic "stopped".
    #[tokio::test]
    async fn stopping_a_failed_recording_keeps_the_reason_it_failed() {
        let ws = workspace("stopfailed");
        let mut recorder = VideoRecorder::start(
            &ws,
            "run-3",
            VideoOptions {
                display: Some(String::new()),
                ..VideoOptions::default()
            },
            Arc::new(Silent),
            None,
        )
        .await
        .unwrap();

        let record = recorder.stop(Some(121336)).await.unwrap();
        assert_eq!(record.status, VideoStatus::Failed);
        assert!(record.reason.is_some());
        assert_eq!(record.rate_ok, None, "nothing was calibrated");

        let read =
            super::super::clock::read_tick_samples(&recorder.dir().join(super::super::TICKS_FILE))
                .unwrap();
        assert_eq!(read.samples.last().unwrap().kind, TickKind::Stop);
        assert_eq!(read.samples.last().unwrap().tick, Some(121336));
    }

    #[tokio::test]
    async fn a_second_stop_is_a_no_op_rather_than_a_second_stop_line() {
        let ws = workspace("doublestop");
        let mut recorder = VideoRecorder::start(
            &ws,
            "run-4",
            VideoOptions {
                display: Some(String::new()),
                ..VideoOptions::default()
            },
            Arc::new(Silent),
            None,
        )
        .await
        .unwrap();
        recorder.stop(Some(1)).await.unwrap();
        recorder.stop(Some(2)).await.unwrap();
        let read =
            super::super::clock::read_tick_samples(&recorder.dir().join(super::super::TICKS_FILE))
                .unwrap();
        assert_eq!(
            read.samples
                .iter()
                .filter(|s| s.kind == TickKind::Stop)
                .count(),
            1
        );
    }

    /// An encoder that cannot be found is reported by name. This is the case a
    /// run started outside `nix develop` hits, and the message is the diagnosis.
    #[tokio::test]
    async fn a_missing_encoder_names_itself() {
        let ws = workspace("noffmpeg");
        // Everything up to the probe has to succeed for this to be reached, so
        // the window tools are stubbed out as missing instead and the failure
        // lands on them -- either way the reason names a binary.
        let recorder = VideoRecorder::start(
            &ws,
            "run-5",
            VideoOptions {
                display: Some(":99".to_string()),
                ffmpeg: "definitely-not-a-real-ffmpeg".to_string(),
                tools: Tools {
                    xdotool: "definitely-not-a-real-xdotool".to_string(),
                    xwininfo: "definitely-not-a-real-xwininfo".to_string(),
                },
                window_timeout: Duration::from_millis(1),
                ..VideoOptions::default()
            },
            Arc::new(Silent),
            None,
        )
        .await
        .unwrap();
        let reason = recorder.record().reason.clone().unwrap();
        assert!(reason.contains("xdotool"), "{reason}");
    }

    #[tokio::test]
    async fn a_run_with_no_video_reads_as_no_video() {
        let ws = workspace("novideo");
        assert_eq!(
            read_video_dir(&ws.join(super::super::VIDEO_DIR)),
            VideoManifest::empty()
        );
    }
}
