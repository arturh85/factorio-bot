//! Recording a run instead of only photographing it.
//!
//! Design: `docs/superpowers/specs/2026-09-02-video-capture-design.md`. This is
//! **v0**: it films an existing graphical client's window, unsteered. Nothing
//! here steers a camera, spawns a spectator peer or depends on `--host`; those
//! are gated on questions the design leaves open, and v0 is first precisely so
//! the clock, the encode and the viewer are proven before the steering question
//! is opened.
//!
//! # Frames stay the record; video is opt-in
//!
//! A frame's tick is *in its filename*, written by the game. A video's tick is
//! derived from a table the host built. Those are different epistemic
//! categories, and the stronger one is not retired for the weaker one. More
//! bluntly: **a video cannot render "nothing was captured here."** While the
//! game stalls the recorder keeps writing frames of the last drawn image, and
//! the result looks exactly like a game that was running and doing nothing.
//! Only [`clock::TickKind::Gap`] can tell those apart, and only if the viewer
//! refuses to interpolate across it.
//!
//! # Where things live
//!
//! ```text
//! <workspace>/video/          # host-written, wiped per run
//!   run.json                  # {"run": "<run id>"} -- same rule as frames/run.json
//!   video.mp4                 # fMP4, see `ffmpeg`
//!   ticks.jsonl               # the clock, see `clock`
//!   video.json                # encoder manifest + calibration + status
//! <run_dir>/video/            # archived copy, same four files
//! ```
//!
//! `run.json` sits **inside** `video/` for the reason `frames/run.json` does: a
//! per-run wipe clears the identifier together with the thing it identifies. A
//! sidecar one level up survives the wipe and goes on describing a file that no
//! longer exists, which converts "I cannot tell whether these match" into "I
//! checked, they match".
//!
//! `<workspace>/video/` rather than `<workspace>/server/script-output/video/`
//! because the *host* writes it. `script-output` is the game's outbox; putting
//! a host artefact there would imply the game produced it, and the next person
//! to debug a missing video would go looking in the mod.

pub mod clock;
pub mod ffmpeg;
pub mod recorder;
pub mod window;

pub use clock::{ReadTicks, TickKind, TickSample, observed_tick_range, parse_tick_samples};
pub use recorder::{TickSource, VideoOptions, VideoRecorder};
pub use window::Resolution;

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The directory name, under both the workspace and a run directory.
pub const VIDEO_DIR: &str = "video";
/// The sidecar naming the run this directory belongs to.
pub const RUN_SIDECAR: &str = "run.json";
/// The encoder manifest.
pub const RECORD_FILE: &str = "video.json";
/// The clock.
pub const TICKS_FILE: &str = "ticks.jsonl";
/// The recording itself.
pub const VIDEO_FILE: &str = "video.mp4";

/// How far `(out₁-out₀)/(wall₁-wall₀)` may stray from 1.0 before the video's
/// clock is declared not to be the host's clock.
///
/// Dropped or duplicated capture frames make the video run slow or fast against
/// the sidecar, and nothing else can detect that: a **one**-point calibration
/// fixes the offset and cannot see the rate at all, which is the whole reason
/// there are two.
pub const RATE_TOLERANCE: f64 = 0.01;

/// Where a recording got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VideoStatus {
    /// Written at start and rewritten at stop. **A `recording` status in an
    /// archived run is, by itself, proof that the recorder was never stopped**
    /// -- the run finished and nobody told the encoder. A viewer must report
    /// that as a defect rather than show the video as if it were complete.
    Recording,
    /// ffmpeg was asked to stop and exited on its own.
    Stopped,
    /// ffmpeg did not exit when asked and was killed. The file is still
    /// playable: that is what the fragmented container is for.
    Killed,
    /// The encoder stopped advancing while the run was still live -- e.g. its
    /// window disappeared. Distinct from ffmpeg *exiting*, which the child
    /// handle catches on its own.
    Died,
    /// Never started. `reason` says why, and the run continued with frames.
    Failed,
    /// Stopped early to leave the disk to `events.jsonl` and `samples.jsonl`.
    /// Those are the artefacts that must survive; the video is the one that may
    /// be sacrificed, and it is the one filling the disk.
    StoppedLowDisk,
}

/// One `(host clock, encoder clock)` observation.
///
/// The first pair fixes the video's zero -- ffmpeg's PTS starts when it
/// captured its first frame, which is some unknown time after we spawned it
/// (process start, X11 connection, window lookup, first grab), and guessing
/// that offset is the mistake that makes every seek wrong by a constant nobody
/// can see. The second pair, at stop, fixes the *rate*, which is a check rather
/// than a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Calibration {
    /// Milliseconds since the recorder's epoch -- the same epoch
    /// [`TickSample::wall_ms`] counts from.
    pub host_wall_ms: u64,
    /// ffmpeg's own `out_time_us`, in milliseconds.
    pub out_time_ms: u64,
}

/// `video.json`: what the encoder was asked for, what it did, and how it ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VideoRecord {
    /// The run this recording belongs to.
    pub run: String,
    /// The recording's filename inside the video directory.
    pub file: String,
    /// The geometry `xwininfo` **observed**, which is what was actually
    /// captured. Zero on a recording that never started.
    pub width: u32,
    pub height: u32,
    /// The geometry that was *asked for*. Kept beside the observed one rather
    /// than instead of it: a tiling compositor ignores a size request unless
    /// the window is floated, and a mismatch is a warning on the run, not a
    /// failure -- the video is still usable and still joins on ticks.
    pub requested_width: u32,
    pub requested_height: u32,
    pub fps: u32,
    pub status: VideoStatus,
    /// Why, for a `failed`, `died` or `stopped_low_disk` recording.
    #[schema(required)]
    pub reason: Option<String>,
    /// ffmpeg's exit code, `null` when it never ran or was killed by a signal.
    #[schema(required)]
    pub ffmpeg_exit: Option<i32>,
    /// The calibration pairs, in order. Zero on a failed start, one on a
    /// recording that never reached a clean stop, two normally.
    pub calibration: Vec<Calibration>,
    /// Whether the two calibration pairs agree on the rate within
    /// [`RATE_TOLERANCE`]. `null` when there are not two pairs to compare --
    /// *unknown*, never "fine".
    #[schema(required)]
    pub rate_ok: Option<bool>,
    /// The X11 window that was filmed, `0x`-prefixed. `null` when none was
    /// resolved.
    #[schema(required)]
    pub window_id: Option<String>,
}

impl VideoRecord {
    /// Whether the window manager gave us the size we asked for. A `false` here
    /// is a warning on the run, never a failure.
    pub fn geometry_as_requested(&self) -> bool {
        self.width == self.requested_width && self.height == self.requested_height
    }
}

/// The observed span of a recording's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TickRange {
    pub from: u64,
    pub to: u64,
}

/// Everything a caller needs to know about one video directory, derived from a
/// directory listing every time it is asked for.
///
/// **One type for the live directory and the archived one.** They hold the same
/// four files and answer the same questions; two shapes would be two chances to
/// disagree about what a recording is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VideoManifest {
    /// The opaque run identifier from `video/run.json`, or `null` when the
    /// directory has no readable sidecar.
    ///
    /// `null` means *unknown*, never *no match*: a consumer compares it for
    /// equality and does nothing else with it. Absence is not evidence of a
    /// mismatch -- treating it as one refuses a good join.
    #[schema(required)]
    pub run: Option<String>,
    /// `video.json`, or `null` when the recorder never wrote one (which for a
    /// live directory means video was never asked for on this run).
    #[schema(required)]
    pub video: Option<VideoRecord>,
    /// The recording's size on disk, `null` when there is no file. A caller can
    /// tell "the encoder was started and produced nothing" from "no encoder was
    /// started" by pairing this with `video`.
    #[schema(required)]
    pub bytes: Option<u64>,
    /// How many lines `ticks.jsonl` holds, gaps included.
    pub samples: usize,
    /// Lines of `ticks.jsonl` that did not parse -- in practice the torn last
    /// line of a killed recorder. Reported rather than swallowed.
    pub skipped: usize,
    /// The span of ticks the clock actually observed, `null` when it observed
    /// none.
    ///
    /// Published so the viewer can size its axis without fetching thousands of
    /// samples: `runTimeline.ts` feeds this into the `drawn` set that decides
    /// where the axis starts.
    #[schema(required)]
    pub tick_range: Option<TickRange>,
}

impl VideoManifest {
    /// The state of a directory that has nothing in it.
    pub fn empty() -> Self {
        Self {
            run: None,
            video: None,
            bytes: None,
            samples: 0,
            skipped: 0,
            tick_range: None,
        }
    }
}

/// Reads the opaque run id from `<dir>/run.json`, or `None` for any reason at
/// all -- absent, unreadable, not JSON, no `run` key, or a `run` that is not a
/// string.
///
/// Every failure collapses to `None` on purpose, exactly as it does for frames:
/// the id exists to make a match *exact*, and a malformed sidecar means the
/// match cannot be established, which is precisely what `None` says.
pub fn read_run_id(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(RUN_SIDECAR)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("run")?.as_str().map(str::to_owned)
}

/// Reads `<dir>/video.json`, or `None` when it is absent or unreadable.
pub fn read_video_record(dir: &Path) -> Option<VideoRecord> {
    let text = fs::read_to_string(dir.join(RECORD_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Builds the manifest for one video directory -- live or archived.
///
/// A directory that does not exist answers [`VideoManifest::empty`] rather than
/// an error: "no video on this run" is a state, and much the commonest one.
pub fn read_video_dir(dir: &Path) -> VideoManifest {
    let record = read_video_record(dir);
    let file = record
        .as_ref()
        .map_or_else(|| VIDEO_FILE.to_string(), |r| r.file.clone());
    let bytes = fs::metadata(dir.join(&file)).ok().map(|m| m.len());
    let ticks = clock::read_tick_samples(&dir.join(TICKS_FILE)).unwrap_or_default();
    VideoManifest {
        run: read_run_id(dir),
        video: record,
        bytes,
        samples: ticks.samples.len(),
        skipped: ticks.skipped,
        tick_range: observed_tick_range(&ticks.samples).map(|(from, to)| TickRange { from, to }),
    }
}

/// Whether two calibration pairs agree that the encoder clock ran at the host's
/// rate.
///
/// `None` when there are not two usable pairs. A degenerate interval (no host
/// time elapsed between the two readings) is also `None`: it is not evidence
/// that the rate is fine, it is evidence that nothing was measured.
pub fn rate_ok(calibration: &[Calibration]) -> Option<bool> {
    let first = calibration.first()?;
    let last = calibration.last()?;
    let wall = last.host_wall_ms.checked_sub(first.host_wall_ms)?;
    if wall == 0 {
        return None;
    }
    let out = last.out_time_ms as f64 - first.out_time_ms as f64;
    Some(((out / wall as f64) - 1.0).abs() <= RATE_TOLERANCE)
}

/// Copies this run's video into `<run_dir>/video/` and returns its manifest.
///
/// **Only a directory whose `run.json` names `run_id` is copied.** A recording
/// left behind by an earlier run is skipped whole -- the rule
/// [`crate::record::archive_frames`] already enforces, and for the same reason:
/// "the decoy case, a stale directory left by an earlier run" already shipped
/// once as a bug. An orphaned recording therefore cannot be archived into this
/// run's directory and cannot appear in this run's manifest.
///
/// `Ok(None)` when there is nothing of ours to archive, which is the ordinary
/// case: video is opt-in.
pub fn archive_video(
    workspace: &Path,
    run_dir: &Path,
    run_id: &str,
) -> io::Result<Option<VideoManifest>> {
    let source = workspace.join(VIDEO_DIR);
    if read_run_id(&source).as_deref() != Some(run_id) {
        return Ok(None);
    }
    let target = run_dir.join(VIDEO_DIR);
    fs::create_dir_all(&target)?;

    let mut names: Vec<String> = match fs::read_dir(&source) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    for name in &names {
        fs::copy(source.join(name), target.join(name))?;
    }
    Ok(Some(read_video_dir(&target)))
}

/// Wipes and re-creates `<workspace>/video/`, then writes the run sidecar.
///
/// The wipe is what makes the sidecar meaningful: a directory holding one run's
/// `video.mp4` beside another run's `run.json` would be archived under the
/// wrong identity.
pub fn open_video_dir(workspace: &Path, run_id: &str) -> io::Result<PathBuf> {
    let dir = workspace.join(VIDEO_DIR);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(RUN_SIDECAR),
        serde_json::to_vec(&serde_json::json!({"run": run_id})).map_err(io::Error::other)?,
    )?;
    Ok(dir)
}

/// Writes `video.json`.
pub fn write_video_record(dir: &Path, record: &VideoRecord) -> io::Result<()> {
    fs::write(
        dir.join(RECORD_FILE),
        serde_json::to_vec_pretty(record).map_err(io::Error::other)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-video-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(run: &str) -> VideoRecord {
        VideoRecord {
            run: run.to_string(),
            file: VIDEO_FILE.to_string(),
            width: 1280,
            height: 720,
            requested_width: 1280,
            requested_height: 720,
            fps: 15,
            status: VideoStatus::Stopped,
            reason: None,
            ffmpeg_exit: Some(0),
            calibration: vec![
                Calibration {
                    host_wall_ms: 812,
                    out_time_ms: 0,
                },
                Calibration {
                    host_wall_ms: 60408,
                    out_time_ms: 59598,
                },
            ],
            rate_ok: Some(true),
            window_id: Some("0x2c00007".to_string()),
        }
    }

    fn seed(dir: &Path, run: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(RUN_SIDECAR), format!(r#"{{"run":"{run}"}}"#)).unwrap();
        fs::write(dir.join(VIDEO_FILE), b"not really an mp4").unwrap();
        fs::write(
            dir.join(TICKS_FILE),
            "{\"t\":60551,\"w\":0,\"k\":\"start\"}\n{\"t\":60581,\"w\":508}\n",
        )
        .unwrap();
        write_video_record(dir, &record(run)).unwrap();
    }

    #[test]
    fn a_recording_from_another_run_is_not_archived() {
        // The decoy case, inherited verbatim from `archive_frames`: a stale
        // directory left by an earlier run must not be adopted by this one.
        let ws = workspace("decoy");
        seed(&ws.join(VIDEO_DIR), "STALE-RUN-MUST-NOT-SURVIVE");
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        assert_eq!(archive_video(&ws, &run_dir, "ours").unwrap(), None);
        assert!(
            !run_dir.join(VIDEO_DIR).exists(),
            "the stale recording must not even get a directory"
        );
    }

    #[test]
    fn a_directory_with_no_sidecar_is_skipped() {
        let ws = workspace("nosidecar");
        let dir = ws.join(VIDEO_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(VIDEO_FILE), b"bytes").unwrap();
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();
        assert_eq!(archive_video(&ws, &run_dir, "ours").unwrap(), None);
    }

    #[test]
    fn our_own_recording_is_copied_whole_and_described() {
        let ws = workspace("ours");
        seed(&ws.join(VIDEO_DIR), "ours");
        let run_dir = ws.join("runs").join("ours");
        fs::create_dir_all(&run_dir).unwrap();

        let manifest = archive_video(&ws, &run_dir, "ours")
            .unwrap()
            .expect("our own recording is archived");
        assert_eq!(manifest.run.as_deref(), Some("ours"));
        assert_eq!(manifest.samples, 2);
        assert_eq!(
            manifest.tick_range,
            Some(TickRange {
                from: 60551,
                to: 60581
            })
        );
        assert_eq!(manifest.bytes, Some(17));
        for name in [RUN_SIDECAR, VIDEO_FILE, TICKS_FILE, RECORD_FILE] {
            assert!(run_dir.join(VIDEO_DIR).join(name).exists(), "{name}");
        }
    }

    #[test]
    fn a_directory_that_does_not_exist_reads_as_no_video_rather_than_an_error() {
        let ws = workspace("absent");
        let manifest = read_video_dir(&ws.join(VIDEO_DIR));
        assert_eq!(manifest, VideoManifest::empty());
    }

    /// The recorder writes `run.json` last-known-good only after wiping, so a
    /// directory can never hold one run's file beside another run's identity.
    #[test]
    fn opening_the_directory_clears_whatever_was_there() {
        let ws = workspace("wipe");
        seed(&ws.join(VIDEO_DIR), "old");
        let dir = open_video_dir(&ws, "new").unwrap();
        assert_eq!(read_run_id(&dir).as_deref(), Some("new"));
        assert!(!dir.join(VIDEO_FILE).exists(), "the old recording is gone");
        assert!(!dir.join(TICKS_FILE).exists(), "and so is its clock");
    }

    #[test]
    fn the_rate_check_needs_two_pairs_and_reports_unknown_otherwise() {
        assert_eq!(rate_ok(&[]), None);
        let one = Calibration {
            host_wall_ms: 812,
            out_time_ms: 0,
        };
        assert_eq!(rate_ok(&[one]), None, "one point cannot see a rate at all");
        assert_eq!(
            rate_ok(&[one, one]),
            None,
            "no host time elapsed: nothing was measured, which is not 'fine'"
        );
    }

    #[test]
    fn a_capture_that_dropped_frames_fails_the_rate_check() {
        let start = Calibration {
            host_wall_ms: 812,
            out_time_ms: 0,
        };
        // 59.598 s of video for 59.596 s of wall clock: within tolerance.
        assert_eq!(
            rate_ok(&[
                start,
                Calibration {
                    host_wall_ms: 60408,
                    out_time_ms: 59598
                }
            ]),
            Some(true)
        );
        // 54 s of video for 59.6 s of wall clock: the encoder lost ~9%.
        assert_eq!(
            rate_ok(&[
                start,
                Calibration {
                    host_wall_ms: 60408,
                    out_time_ms: 54000
                }
            ]),
            Some(false)
        );
    }

    #[test]
    fn an_observed_geometry_that_differs_from_the_request_is_visible_in_the_record() {
        let mut r = record("ours");
        assert!(r.geometry_as_requested());
        r.width = 1278;
        assert!(
            !r.geometry_as_requested(),
            "a tiling compositor's adjustment must be reportable"
        );
    }

    #[test]
    fn the_record_round_trips_through_its_file() {
        let ws = workspace("roundtrip");
        let dir = ws.join(VIDEO_DIR);
        fs::create_dir_all(&dir).unwrap();
        write_video_record(&dir, &record("ours")).unwrap();
        assert_eq!(read_video_record(&dir), Some(record("ours")));
    }

    /// A status still saying `recording` in an archived run is proof the
    /// recorder was never stopped. Pinned as a wire value because the viewer
    /// branches on it.
    #[test]
    fn statuses_serialise_as_snake_case_strings() {
        for (status, text) in [
            (VideoStatus::Recording, "\"recording\""),
            (VideoStatus::Stopped, "\"stopped\""),
            (VideoStatus::Killed, "\"killed\""),
            (VideoStatus::Died, "\"died\""),
            (VideoStatus::Failed, "\"failed\""),
            (VideoStatus::StoppedLowDisk, "\"stopped_low_disk\""),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), text);
        }
    }
}
