//! The clock: `(game.tick, wall_ms)` pairs, host-side.
//!
//! **The mod cannot write this file.** Factorio's control stage has no clock
//! at all -- `os` is not in the sandbox and `LuaProfiler` is explicitly
//! designed to refuse handing a number to Lua. See
//! `docs/superpowers/specs/2026-09-02-video-capture-design.md` §1.1. What the
//! host *does* already have is a stream of ticks: every `remote_call_timed`
//! reply arrives stamped with `game.tick` from inside the game, and
//! [`crate::factorio::rcon::FactorioRcon::game_tick`] asks for one directly.
//! So both halves of a pair are read on one machine against one monotonic
//! clock, and there is no cross-process skew to reason about.
//!
//! JSONL rather than one JSON array, for the reason `events.jsonl`,
//! `samples.jsonl` and `map.jsonl` are already JSONL: a recorder that is
//! killed leaves a readable file with a torn last line, not an unparseable one
//! with no closing bracket.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// What one `ticks.jsonl` line is.
///
/// Short keys (`t`, `w`, `k`) because there are thousands of these lines and
/// this file's only job is to be a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TickKind {
    /// An ordinary observation, and the default a line without `k` reads as.
    #[default]
    Sample,
    /// The first observation of the recording.
    Start,
    /// **No tick was observed here.** Written when the floor poll fails or is
    /// starved. This is the video's equivalent of a missing frame file, and
    /// the viewer must refuse to interpolate across it: a video has no null,
    /// so while the game stalls the recorder keeps writing frames of the last
    /// drawn image and the result looks exactly like a game that was running
    /// and doing nothing. Only this line can tell those apart.
    Gap,
    /// The last observation of the recording.
    Stop,
}

/// One line of `ticks.jsonl`, and one element of `GET /api/v1/video/ticks`.
///
/// Deliberately one type for both. The file *is* the wire format; a second
/// struct for the HTTP side is how two halves of a system come to disagree
/// about what a sample is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TickSample {
    /// `game.tick` at the observation.
    ///
    /// Absent on a [`TickKind::Gap`] line, and only there: a gap records that
    /// nothing was observed, which is not a tick with an unknown value but the
    /// absence of an observation altogether.
    #[serde(rename = "t", default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    /// Milliseconds since the recorder's own epoch -- the same epoch
    /// `VideoRecord::calibration` measures the video's PTS against.
    ///
    /// Stamped at the **midpoint of send and receive**, never at receipt. The
    /// tick is the value inside the game at the moment the command ran, which
    /// is somewhere between the two; receipt time carries a systematic late
    /// bias of one full round trip, while the midpoint bounds the error at
    /// half a round trip and centres it on zero.
    #[serde(rename = "w")]
    pub wall_ms: u64,
    /// Always on the wire, even for the ordinary [`TickKind::Sample`].
    ///
    /// The design's example lines omit it, and dropping it would save ~11 bytes
    /// a line -- 60 KB across a 45-minute run, against a recording measured in
    /// hundreds of megabytes. It is kept because this struct is also the HTTP
    /// wire shape, and a key that is sometimes absent has to be declared
    /// optional in `app/src/api/types.ts`, where "absent" and "null" then become
    /// a distinction the client has to make and this format never intends. A
    /// line written *without* `k` still parses, via `default`, so every document
    /// the design describes is still readable.
    ///
    /// `#[schema(required)]` because the **key is always on the wire**: there is
    /// no `skip_serializing_if`, so serde always emits it. Without this utoipa
    /// would publish it as optional purely because `#[serde(default)]` lets a
    /// *reader* omit it, and a generated client would then have to handle an
    /// absence this server never produces.
    #[serde(rename = "k", default)]
    #[schema(required)]
    pub kind: TickKind,
    /// Why, for a gap. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl TickSample {
    /// An ordinary observation.
    pub fn observed(tick: u64, wall_ms: u64) -> Self {
        Self {
            tick: Some(tick),
            wall_ms,
            kind: TickKind::Sample,
            reason: None,
        }
    }

    /// A gap: the recorder looked and the game did not answer.
    pub fn gap(wall_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            tick: None,
            wall_ms,
            kind: TickKind::Gap,
            reason: Some(reason.into()),
        }
    }
}

/// What [`parse_tick_samples`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadTicks {
    pub samples: Vec<TickSample>,
    /// Lines that did not parse -- in practice the torn last line of a
    /// recorder that was killed. Reported rather than swallowed: a reader that
    /// silently drops input cannot be told apart from one with nothing to
    /// drop.
    pub skipped: usize,
}

/// Parses `ticks.jsonl`, tolerating a torn tail.
///
/// Lives in `core` rather than beside the HTTP handler that first needed it,
/// for the same reason [`crate::record::parse_frame_name`] does: the archive
/// and the live endpoint need the same answer, and two parsers for one format
/// is how two halves of a system come to disagree.
pub fn parse_tick_samples(text: &str) -> ReadTicks {
    let mut read = ReadTicks::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TickSample>(line) {
            Ok(sample) => read.samples.push(sample),
            Err(_) => read.skipped += 1,
        }
    }
    read
}

/// Reads `ticks.jsonl` from disk. A file that is not there is *no samples*,
/// not an error: a run that never recorded video has none, and that is a
/// state, not a failure.
pub fn read_tick_samples(path: &Path) -> io::Result<ReadTicks> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(ReadTicks::default()),
        Err(err) => return Err(err),
    };
    let mut read = ReadTicks::default();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TickSample>(&line) {
            Ok(sample) => read.samples.push(sample),
            Err(_) => read.skipped += 1,
        }
    }
    Ok(read)
}

/// The span of ticks a set of samples actually observed.
///
/// Gap lines contribute nothing -- they carry no tick. `None` when nothing was
/// ever observed, which a caller must read as "cannot judge" rather than as an
/// empty range.
pub fn observed_tick_range(samples: &[TickSample]) -> Option<(u64, u64)> {
    let mut span: Option<(u64, u64)> = None;
    for sample in samples {
        let Some(tick) = sample.tick else { continue };
        span = Some(match span {
            None => (tick, tick),
            Some((low, high)) => (low.min(tick), high.max(tick)),
        });
    }
    span
}

/// Appends `ticks.jsonl`, flushing every line.
///
/// Flushed per line for the reason [`crate::record::RunRecorder::record`]
/// flushes per event: the recordings most worth reading are the ones that
/// crashed, and a buffered tail is exactly the part a crash takes.
pub struct TickLog {
    file: File,
}

impl TickLog {
    /// Creates (truncating) `<dir>/ticks.jsonl`.
    pub fn create(dir: &Path) -> io::Result<Self> {
        Ok(Self {
            file: File::create(dir.join(super::TICKS_FILE))?,
        })
    }

    pub fn append(&mut self, sample: &TickSample) -> io::Result<()> {
        let mut line = serde_json::to_string(sample).map_err(io::Error::other)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_sample_round_trips_through_the_short_keys() {
        // `k` is always written, but a line without it still parses -- every
        // document the design's examples show is readable.
        assert_eq!(
            serde_json::from_str::<TickSample>(r#"{"t":1,"w":2}"#)
                .unwrap()
                .kind,
            TickKind::Sample
        );
        let line = serde_json::to_string(&TickSample::observed(60551, 0)).unwrap();
        assert_eq!(line, r#"{"t":60551,"w":0,"k":"sample"}"#);
        assert_eq!(
            serde_json::from_str::<TickSample>(&line).unwrap(),
            TickSample::observed(60551, 0)
        );
    }

    #[test]
    fn a_gap_line_carries_no_tick_and_says_why() {
        let sample = TickSample::gap(41207, "no tick observed for 4.1 s");
        let line = serde_json::to_string(&sample).unwrap();
        assert!(!line.contains("\"t\""), "a gap observed no tick: {line}");
        assert!(line.contains(r#""k":"gap""#), "{line}");
        assert_eq!(serde_json::from_str::<TickSample>(&line).unwrap(), sample);
    }

    /// The exact document the design specifies, parsed as written there --
    /// including the `start`/`gap`/`stop` line kinds.
    #[test]
    fn the_documented_file_shape_parses() {
        let text = concat!(
            "{\"t\": 60551, \"w\": 0, \"k\": \"start\"}\n",
            "{\"t\": 60581, \"w\": 508}\n",
            "{\"w\": 41207, \"k\": \"gap\", \"reason\": \"no tick observed for 4.1 s\"}\n",
            "{\"t\": 121336, \"w\": 60408, \"k\": \"stop\"}\n",
        );
        let read = parse_tick_samples(text);
        assert_eq!(read.skipped, 0);
        assert_eq!(read.samples.len(), 4);
        assert_eq!(read.samples[0].kind, TickKind::Start);
        assert_eq!(read.samples[1].kind, TickKind::Sample);
        assert_eq!(read.samples[2].tick, None);
        assert_eq!(read.samples[3].kind, TickKind::Stop);
    }

    #[test]
    fn a_torn_last_line_is_counted_not_fatal() {
        let read = parse_tick_samples("{\"t\":1,\"w\":0}\n{\"t\":2,\"w\"");
        assert_eq!(read.samples.len(), 1, "the whole lines still parse");
        assert_eq!(read.skipped, 1, "the torn one is reported, not hidden");
    }

    #[test]
    fn a_gap_contributes_no_tick_to_the_observed_range() {
        let samples = vec![
            TickSample::observed(100, 0),
            TickSample::gap(500, "starved"),
            TickSample::observed(300, 1000),
        ];
        assert_eq!(observed_tick_range(&samples), Some((100, 300)));
        assert_eq!(observed_tick_range(&[TickSample::gap(0, "x")]), None);
    }

    #[test]
    fn a_missing_ticks_file_is_no_samples_rather_than_an_error() {
        let dir = std::env::temp_dir().join(format!("fb-ticks-{}", std::process::id()));
        let read = read_tick_samples(&dir.join("nope.jsonl")).expect("absence is not an error");
        assert!(read.samples.is_empty());
    }

    #[test]
    fn the_log_writes_one_flushed_line_per_sample() {
        let dir =
            std::env::temp_dir().join(format!("fb-ticklog-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = TickLog::create(&dir).unwrap();
        log.append(&TickSample::observed(1, 0)).unwrap();
        log.append(&TickSample::observed(61, 1000)).unwrap();
        let read = read_tick_samples(&dir.join(super::super::TICKS_FILE)).unwrap();
        assert_eq!(read.samples.len(), 2);
        assert_eq!(read.samples[1].tick, Some(61));
    }
}
