//! The encoder: what we ask `ffmpeg` for, and how we read what it answers.
//!
//! Every choice here is argued in
//! `docs/superpowers/specs/2026-09-02-video-capture-design.md` §5. The two
//! that are correctness requirements rather than preferences:
//!
//! - **fMP4, never `+faststart`.** A plain MP4 writes its `moov` atom at the
//!   end, so a recorder that is SIGKILLed, OOM-killed or stopped by a crashed
//!   host leaves a file **no player will open at all**. A fragmented MP4 is
//!   playable and seekable up to the last written fragment. A recording that
//!   dies mid-run must still be watchable up to where it died, and
//!   `+faststart` -- a second pass over a *complete* file -- cannot offer that.
//! - **No `scale` filter.** The resolution is reached by sizing the window
//!   (see [`super::window`]); scaling here would render every pixel and then
//!   pay to resample it.
//!
//! `-video_size` is passed explicitly and is the size `xwininfo` **observed**,
//! not the size we asked the window manager for. x11grab captures a fixed
//! frame size, so a mismatch crops or pads for the whole run.

use std::path::Path;

/// Everything the encoder needs to be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeSettings {
    /// The X11 window id, as [`super::window::window_id_hex`] renders it.
    pub window_id: String,
    /// The `DISPLAY` to grab from, e.g. `":0"`.
    pub display: String,
    pub fps: u32,
    /// The **observed** window size.
    pub width: u32,
    pub height: u32,
}

/// A uniform GOP of two seconds.
///
/// This buys seek *latency*, not precision: browsers seek accurately on
/// `currentTime` by decoding forward from the preceding keyframe, so scrubbing
/// precision is the frame period regardless of the GOP. Two seconds bounds the
/// decode at `2 * fps` frames, and `scenecut=0` makes that bound the same
/// everywhere rather than dependent on how busy the screen was.
pub const GOP_SECONDS: u32 = 2;

/// The recording command line, as separate arguments (never a shell string:
/// the output path is a filesystem path and must not be word-split).
///
/// **Deliberately without `-nostdin`**, and this is a correction to the design:
/// §5 of the spec lists `-nostdin` while §8 stops the recorder by sending `q`
/// on ffmpeg's stdin, and those two cannot both hold -- `-nostdin` is exactly
/// the flag that makes ffmpeg ignore stdin. The clean-stop path is the one
/// worth keeping (a `q` lets ffmpeg finalise the file itself), and `-nostdin`
/// buys nothing here anyway: it exists to stop ffmpeg swallowing an *inherited*
/// terminal's stdin, and this child's stdin is a private pipe only the recorder
/// writes to. The probe below keeps the flag, because there it is true.
pub fn record_args(settings: &EncodeSettings, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    args.extend(capture_args(settings, settings.fps));
    args.extend(output_args(settings));
    args.extend([
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-y".to_string(),
        output.to_string_lossy().into_owned(),
    ]);
    args
}

/// What to grab, and how fast: the x11grab input, addressed by window id.
///
/// `framerate` is a parameter rather than `settings.fps` because the probe
/// wants one frame *now* while the recording wants the run's rate; everything
/// else about the capture is identical, and identical is the point.
fn capture_args(settings: &EncodeSettings, framerate: u32) -> Vec<String> {
    [
        "-f",
        "x11grab",
        "-framerate",
        &framerate.to_string(),
        "-video_size",
        &format!("{}x{}", settings.width, settings.height),
        "-window_id",
        &settings.window_id,
        "-i",
        &settings.display,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Everything between the input and the output file: codec, rate control,
/// GOP, container.
///
/// **Factored out so the probe cannot diverge from the recording.** This is the
/// half that killed a 26-minute capture: `-movflags +...+default_base_is_moof`
/// is not a flag name ffmpeg 9 knows, it exits 234 at option parsing before
/// writing a byte, and the probe of the day never passed these arguments at
/// all -- it grabbed one frame to `-f null`, which builds no container and so
/// validates no container flag. A shared function plus
/// [`tests::the_probe_exercises_the_same_output_chain_as_the_recording`] means
/// any argument added here is exercised by the probe automatically, before the
/// run starts, rather than 26 minutes into it.
pub fn output_args(settings: &EncodeSettings) -> Vec<String> {
    let keyint = (settings.fps * GOP_SECONDS).max(1);
    [
        "-pix_fmt",
        "yuv420p",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "28",
        "-tune",
        "zerolatency",
        "-x264-params",
        &format!("keyint={keyint}:min-keyint={keyint}:scenecut=0"),
        "-movflags",
        "+frag_keyframe+empty_moov+default_base_moof",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// A **one-frame trial recording**, written through the real muxer to
/// `output` and deleted by the caller.
///
/// The design was already explicit that the probe must not be a version
/// string: the stock nixpkgs `ffmpeg` has no x11grab at all and would fail only
/// at the first grab, which on a 45-minute run means discovering it from a
/// broken file at the end. That instinct was right and still let a whole
/// capture through, because a single frame discarded to `-f null` exercises
/// x11grab and *nothing downstream of it*. So the probe now carries
/// [`output_args`] verbatim and writes a real fragmented MP4: it opens the
/// display, addresses the window, encodes a frame with the run's codec
/// settings, and muxes it with the run's container flags. An argument ffmpeg
/// will reject is rejected here, in a fraction of a second, before the run
/// starts.
///
/// `output` must end in `.mp4`: the muxer is chosen by extension, exactly as it
/// is for the recording, and probing a different container would be probing a
/// different command.
pub fn probe_args(settings: &EncodeSettings, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // One frame, grabbed immediately: `-framerate 1` makes x11grab hand over
    // the first frame without waiting for a second one.
    args.extend(capture_args(settings, 1));
    args.push("-frames:v".to_string());
    args.push("1".to_string());
    args.extend(output_args(settings));
    args.push("-y".to_string());
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Pulls `out_time_us=<n>` off one line of the `-progress` stream.
///
/// `-progress` writes `key=value` lines continuously, ending each block with
/// `progress=continue` or `progress=end`. Only `out_time_us` is read: it is the
/// encoder's own presentation clock, which is the quantity the calibration
/// pairs need, and it is monotone even when the wall clock is not.
///
/// `N/A` is a value ffmpeg does write before the first frame; it parses to
/// `None`, which is *no reading yet* rather than a reading of zero.
pub fn parse_out_time_us(line: &str) -> Option<u64> {
    line.trim()
        .strip_prefix("out_time_us=")?
        .trim()
        .parse()
        .ok()
}

/// Whether a `-progress` line closes the stream.
pub fn is_progress_end(line: &str) -> bool {
    line.trim() == "progress=end"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> EncodeSettings {
        EncodeSettings {
            window_id: "0x2c00007".to_string(),
            display: ":0".to_string(),
            fps: 15,
            width: 1280,
            height: 720,
        }
    }

    #[test]
    fn the_container_is_fragmented_and_never_faststart() {
        let args = record_args(&settings(), Path::new("/tmp/video.mp4"));
        let movflags = args
            .iter()
            .position(|a| a == "-movflags")
            .map(|i| args[i + 1].clone())
            .expect("-movflags is passed");
        assert!(movflags.contains("frag_keyframe"), "{movflags}");
        assert!(movflags.contains("empty_moov"), "{movflags}");
        assert!(
            !args.iter().any(|a| a.contains("faststart")),
            "a SIGKILLed +faststart recording opens in no player at all"
        );
    }

    /// The resolution is reached by sizing the window. A `scale` filter here
    /// would mean the game rendered every pixel and ffmpeg paid to resample --
    /// worse on both ends, which is the exact mistake the decision section
    /// warns about.
    #[test]
    fn nothing_is_scaled_in_the_encoder() {
        let args = record_args(&settings(), Path::new("/tmp/video.mp4"));
        assert!(!args.iter().any(|a| a == "-vf"), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("scale=")), "{args:?}");
    }

    #[test]
    fn the_capture_is_sized_to_the_observed_window_and_addressed_by_id() {
        let observed = EncodeSettings {
            width: 1278,
            height: 715,
            ..settings()
        };
        let args = record_args(&observed, Path::new("/tmp/video.mp4"));
        let size = args
            .iter()
            .position(|a| a == "-video_size")
            .map(|i| args[i + 1].clone())
            .expect("-video_size is passed");
        assert_eq!(size, "1278x715", "the observed size, not the requested one");
        assert!(args.iter().any(|a| a == "0x2c00007"), "{args:?}");
        assert!(args.iter().any(|a| a == "-window_id"), "{args:?}");
    }

    #[test]
    fn the_gop_is_uniform_and_two_seconds_long() {
        for (fps, keyint) in [(15u32, 30u32), (30, 60)] {
            let args = record_args(
                &EncodeSettings { fps, ..settings() },
                Path::new("/tmp/video.mp4"),
            );
            let params = args
                .iter()
                .position(|a| a == "-x264-params")
                .map(|i| args[i + 1].clone())
                .expect("-x264-params is passed");
            assert_eq!(
                params,
                format!("keyint={keyint}:min-keyint={keyint}:scenecut=0")
            );
        }
    }

    #[test]
    fn the_output_path_travels_as_one_argument() {
        let args = record_args(&settings(), Path::new("/tmp/a b/video.mp4"));
        assert_eq!(args.last().unwrap(), "/tmp/a b/video.mp4");
    }

    #[test]
    fn the_probe_grabs_exactly_one_frame_into_a_real_file() {
        let args = probe_args(&settings(), Path::new("/tmp/probe.mp4"));
        assert!(args.windows(2).any(|w| w == ["-frames:v", "1"]), "{args:?}");
        assert!(args.iter().any(|a| a == "x11grab"), "{args:?}");
        assert_eq!(args.last().unwrap(), "/tmp/probe.mp4");
        assert!(
            !args.iter().any(|a| a == "-version"),
            "a version string proves nothing about x11grab support"
        );
    }

    /// `-nostdin` and "stop it by sending `q`" are mutually exclusive. The
    /// stop path wins; see [`record_args`].
    #[test]
    fn the_recording_command_leaves_stdin_open_so_it_can_be_stopped_cleanly() {
        let args = record_args(&settings(), Path::new("/tmp/video.mp4"));
        assert!(
            !args.iter().any(|a| a == "-nostdin"),
            "-nostdin would make the `q` on stdin a no-op: {args:?}"
        );
        // The probe never gets a `q`, so there the flag is honest.
        assert!(
            probe_args(&settings(), Path::new("/tmp/probe.mp4"))
                .iter()
                .any(|a| a == "-nostdin")
        );
    }

    #[test]
    fn progress_lines_yield_the_encoder_clock_and_nothing_else() {
        assert_eq!(parse_out_time_us("out_time_us=59598000"), Some(59_598_000));
        assert_eq!(parse_out_time_us("out_time_us=N/A"), None);
        assert_eq!(parse_out_time_us("frame=42"), None);
        assert_eq!(parse_out_time_us("out_time=00:00:59.598000"), None);
        assert!(is_progress_end("progress=end"));
        assert!(!is_progress_end("progress=continue"));
    }

    /// **The probe must exercise the muxer it will actually use.**
    ///
    /// A one-frame grab discarded to `-f null` proves x11grab works and proves
    /// nothing at all about `-movflags`, `-c:v` or the container -- so an
    /// encoder argument ffmpeg rejects passes the probe and fails 26 minutes
    /// later, having written zero bytes. Every output argument the recording
    /// uses must appear in the probe, contiguously and in order: a *subset*
    /// check would let a future flag be added to one and not the other, which
    /// is the same hole in a smaller shape.
    #[test]
    fn the_probe_exercises_the_same_output_chain_as_the_recording() {
        let probe = probe_args(&settings(), Path::new("/tmp/probe.mp4"));
        let record = record_args(&settings(), Path::new("/tmp/video.mp4"));
        let chain = output_args(&settings());
        assert!(!chain.is_empty());
        for args in [&probe, &record] {
            assert!(
                args.windows(chain.len()).any(|w| w == chain.as_slice()),
                "the whole output chain must be here, in order: {args:?}"
            );
        }
        assert!(
            probe
                .windows(2)
                .any(|w| w == ["-movflags", &chain[chain.len() - 1]]),
            "the probe must touch the muxer that failed: {probe:?}"
        );
        assert!(
            !probe.windows(2).any(|w| w == ["-f", "null"]),
            "a null muxer writes no container and validates no container flag: {probe:?}"
        );
    }
}
