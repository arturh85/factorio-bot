//! Durable run records.
//!
//! A run leaves `runs/<id>/events.jsonl` behind: one JSON object per line,
//! flushed as it happens. See
//! `docs/superpowers/specs/2026-09-01-run-recording-and-replay-design.md`.
//!
//! Append-only JSONL rather than one JSON document, because a document is only
//! readable once it is closed and the runs most worth reading are the ones that
//! crashed. Every complete line written before a crash stays valid.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::types::Position;

pub mod splits;
pub use splits::{Split, derive_splits};

/// What happened. Internally tagged as `kind`, so a line is one flat object.
///
/// [`EventKind::Unknown`] is the `serde(other)` catch-all: a reader built
/// before a kind existed must skip it rather than refuse the file, so an older
/// viewer can still open a newer run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    RunStarted {
        run_id: String,
        bots: Vec<u32>,
        /// Present-and-null when unknown, never absent: a key that is always
        /// there says "we looked", where a missing key cannot be told apart
        /// from "we never asked".
        seed: Option<u64>,
        factorio: Option<String>,
        git: Option<String>,
    },
    MilestoneStarted {
        index: u32,
        goal: String,
    },
    MilestoneSatisfied {
        index: u32,
        iterations: u32,
        elapsed_ticks: u64,
    },
    MilestoneStuck {
        index: u32,
        outcome: String,
        best_steps: Option<u32>,
        last_error: Option<String>,
    },
    PlanCreated {
        milestone_index: u32,
        steps: u32,
        makespan: u64,
        bots: Vec<u32>,
    },
    ActionDispatched {
        id: u32,
        bot: u32,
        action: String,
        target: Option<Position>,
    },
    ActionSettled {
        id: u32,
        bot: u32,
        status: String,
        elapsed_ticks: u64,
        error: Option<String>,
    },
    Frame {
        bot: u32,
        camera: String,
        file: String,
    },
    RunFinished {
        outcome: String,
        elapsed_ticks: u64,
    },
    /// A kind this build does not know. Readers skip it; writers never emit it.
    #[serde(other)]
    Unknown,
}

/// One line of `events.jsonl`.
///
/// `tick` is *when this happened*; a duration is always `elapsed_ticks`. The
/// two were briefly both called `ticks`, which reads fine until someone plots
/// it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub tick: u64,
    pub wall_ms: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// Writes a run's event log.
pub struct RunRecorder {
    dir: PathBuf,
    run_id: String,
    events: File,
    started: Instant,
}

impl RunRecorder {
    /// Creates `<runs_root>/<run_id>/` and opens its event log.
    ///
    /// The caller supplies `run_id` because the same id must reach
    /// `frame_capture_start`; minting it in two places is how the log and the
    /// frames come to disagree about which run they belong to.
    pub fn start(runs_root: &Path, run_id: impl Into<String>) -> io::Result<Self> {
        let run_id = run_id.into();
        let dir = runs_root.join(&run_id);
        fs::create_dir_all(&dir)?;
        let events = File::create(dir.join("events.jsonl"))?;
        Ok(Self {
            dir,
            run_id,
            events,
            started: Instant::now(),
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Appends one event and flushes it.
    ///
    /// Flushed per event on purpose: a crashed run must leave a readable
    /// record, and these are hundreds of events over minutes, not millions,
    /// so buffering buys nothing worth the loss.
    pub fn record(&mut self, tick: u64, kind: EventKind) -> io::Result<()> {
        let event = Event {
            tick,
            wall_ms: self.started.elapsed().as_millis() as u64,
            kind,
        };
        let mut line = serde_json::to_string(&event).map_err(io::Error::other)?;
        line.push('\n');
        self.events.write_all(line.as_bytes())?;
        self.events.flush()
    }
}

/// What [`read_events`] found.
pub struct ReadEvents {
    pub events: Vec<Event>,
    /// Lines that did not parse -- in practice the truncated final line of a
    /// crashed run. Returned rather than swallowed: a reader that silently
    /// drops input cannot be told apart from one with nothing to drop.
    pub skipped: usize,
}

/// Reads an event log, tolerating a truncated tail.
pub fn read_events(path: &Path) -> io::Result<ReadEvents> {
    let mut events = Vec::new();
    let mut skipped = 0usize;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(&line) {
            Ok(event) => events.push(event),
            Err(_) => skipped += 1,
        }
    }
    Ok(ReadEvents { events, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Seek;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-record-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_event_kind_round_trips() {
        let kinds = vec![
            EventKind::RunStarted {
                run_id: "r1".into(),
                bots: vec![1, 2],
                seed: Some(7),
                factorio: Some("2.1.17".into()),
                git: None,
            },
            EventKind::MilestoneStarted {
                index: 0,
                goal: "researched(automation)".into(),
            },
            EventKind::MilestoneSatisfied {
                index: 0,
                iterations: 3,
                elapsed_ticks: 24587,
            },
            EventKind::MilestoneStuck {
                index: 1,
                outcome: "stuck_silent".into(),
                best_steps: Some(42),
                last_error: None,
            },
            EventKind::PlanCreated {
                milestone_index: 0,
                steps: 105,
                makespan: 24587,
                bots: vec![1, 2, 3, 4],
            },
            EventKind::ActionDispatched {
                id: 17,
                bot: 2,
                action: "mine".into(),
                target: None,
            },
            EventKind::ActionSettled {
                id: 17,
                bot: 2,
                status: "success".into(),
                elapsed_ticks: 120,
                error: None,
            },
            EventKind::Frame {
                bot: 1,
                camera: "overview".into(),
                file: "frames/1/overview/300.jpg".into(),
            },
            EventKind::RunFinished {
                outcome: "done".into(),
                elapsed_ticks: 41000,
            },
        ];
        let dir = tmpdir("roundtrip");
        let mut rec = RunRecorder::start(&dir, "r1").unwrap();
        for (i, k) in kinds.iter().enumerate() {
            rec.record(i as u64, k.clone()).unwrap();
        }
        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        assert_eq!(read.skipped, 0);
        assert_eq!(read.events.len(), kinds.len());
        for (i, (got, want)) in read.events.iter().zip(kinds.iter()).enumerate() {
            assert_eq!(&got.kind, want, "kind {i} did not survive the round trip");
            assert_eq!(got.tick, i as u64);
        }
    }

    #[test]
    fn a_truncated_final_line_costs_only_that_line() {
        let dir = tmpdir("truncated");
        let mut rec = RunRecorder::start(&dir, "r2").unwrap();
        for i in 0..3 {
            rec.record(
                i,
                EventKind::Frame {
                    bot: 1,
                    camera: "c".into(),
                    file: format!("{i}.jpg"),
                },
            )
            .unwrap();
        }
        // Simulate a crash mid-write: lop the tail off the last line.
        let path = rec.dir().join("events.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let cut = text.len() - 12;
        fs::write(&path, &text[..cut]).unwrap();

        let read = read_events(&path).unwrap();
        assert_eq!(read.events.len(), 2, "the two complete lines must survive");
        assert_eq!(
            read.skipped, 1,
            "the truncated line must be reported, not hidden"
        );
    }

    #[test]
    fn an_unknown_kind_is_skipped_rather_than_rejecting_the_file() {
        let dir = tmpdir("unknown");
        let path = dir.join("events.jsonl");
        fs::write(
            &path,
            r#"{"tick":1,"wall_ms":1,"kind":"invented_later","whatever":true}
{"tick":2,"wall_ms":2,"kind":"run_finished","outcome":"done","elapsed_ticks":9}
"#,
        )
        .unwrap();
        let read = read_events(&path).unwrap();
        assert_eq!(read.skipped, 0, "an unknown kind is not a parse failure");
        assert_eq!(read.events.len(), 2);
        assert_eq!(read.events[0].kind, EventKind::Unknown);
        assert!(matches!(read.events[1].kind, EventKind::RunFinished { .. }));
    }

    #[test]
    fn null_bearing_keys_are_written_not_omitted() {
        let dir = tmpdir("nulls");
        let mut rec = RunRecorder::start(&dir, "r3").unwrap();
        rec.record(
            0,
            EventKind::ActionSettled {
                id: 1,
                bot: 1,
                status: "success".into(),
                elapsed_ticks: 5,
                error: None,
            },
        )
        .unwrap();
        let text = fs::read_to_string(rec.dir().join("events.jsonl")).unwrap();
        assert!(
            text.contains("\"error\":null"),
            "an absent key cannot be told apart from a question never asked: {text}"
        );
    }

    #[test]
    fn the_log_is_flushed_per_event_so_a_crash_leaves_a_record() {
        let dir = tmpdir("flush");
        let mut rec = RunRecorder::start(&dir, "r4").unwrap();
        rec.record(
            0,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        // Read while the recorder is still open and holding the handle.
        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        assert_eq!(read.events.len(), 1, "the event must be on disk already");
        let _ = rec.events.stream_position();
    }
}
