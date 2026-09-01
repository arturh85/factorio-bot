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
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::types::Position;

pub mod frames;
pub mod splits;
pub use frames::{ArchivedFrame, archive_frames, parse_frame_name};
pub use splits::{Split, derive_splits};

/// What happened. Internally tagged as `kind`, so a line is one flat object.
///
/// [`EventKind::Unknown`] is the `serde(other)` catch-all: a reader built
/// before a kind existed must skip it rather than refuse the file, so an older
/// viewer can still open a newer run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
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
    /// A milestone was reached.
    ///
    /// Carries no duration on purpose: the event says *when*, and
    /// [`splits`] derives *how long* from the started/satisfied tick pair.
    /// It briefly carried an `elapsed_ticks` the caller could not compute, so
    /// every live run recorded a confident zero beside a split that had the
    /// real number.
    MilestoneSatisfied {
        index: u32,
        iterations: u32,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Event {
    pub tick: u64,
    pub wall_ms: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// What a run was and how it ended, without reading its whole log.
///
/// Everything here except the identity and the wall clock is *derived from the
/// event log at finish*, not tracked alongside it, so the manifest cannot drift
/// from the events it summarises. The listing endpoint reads this file; only a
/// viewer opening one run reads the log itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Manifest {
    pub run_id: String,
    /// Unix seconds. For "when was this run", never for comparing two runs --
    /// runs are compared on ticks.
    pub started_unix: u64,
    /// `null` while the run is still going or if it crashed before finishing.
    pub finished_unix: Option<u64>,
    /// `null` for a run that never reached `finish`. A crashed run keeps its
    /// events and its frames; what it lacks is a verdict, and inventing one
    /// would make it look complete.
    pub outcome: Option<String>,
    pub elapsed_ticks: Option<u64>,
    pub events: usize,
    pub frames: usize,
    pub splits: usize,
}

/// Writes a run's event log.
pub struct RunRecorder {
    dir: PathBuf,
    run_id: String,
    events: File,
    started: Instant,
    started_unix: u64,
    /// The game tick of the first event recorded, so a duration can be a
    /// duration. Absent until something has been recorded.
    start_tick: Option<u64>,
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
            start_tick: None,
            started_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
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
        if self.start_tick.is_none() {
            self.start_tick = Some(tick);
        }
        let mut line = serde_json::to_string(&event).map_err(io::Error::other)?;
        line.push('\n');
        self.events.write_all(line.as_bytes())?;
        self.events.flush()
    }
}

impl RunRecorder {
    /// Closes the run: records `run_finished`, then derives and materialises
    /// everything a reader needs.
    ///
    /// Order matters. The finishing event is written *first*, so that a crash
    /// during archiving still leaves a log that says how the run ended. Splits
    /// and the manifest are then derived from the log on disk rather than from
    /// state held in memory -- if the two could disagree, the summary would be
    /// the one that is wrong, and it is the one everybody reads.
    ///
    /// `workspace` is where `client<N>` directories live; pass `None` for a
    /// planning-only run that captured no frames. That is a valid run, not a
    /// degenerate one.
    /// How long the run has lasted, in ticks.
    ///
    /// A duration, never the absolute tick it happened at. The first live run
    /// reported `elapsed_ticks: 60246` for a run that took 871 ticks, because
    /// the tick a thing happened at looks exactly like a duration when you are
    /// reading a number out of JSON.
    fn elapsed_at(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.start_tick.unwrap_or(tick))
    }

    pub fn finish(
        &mut self,
        tick: u64,
        outcome: &str,
        workspace: Option<&Path>,
    ) -> io::Result<Manifest> {
        self.record(
            tick,
            EventKind::RunFinished {
                outcome: outcome.to_string(),
                elapsed_ticks: self.elapsed_at(tick),
            },
        )?;

        let read = read_events(&self.dir.join("events.jsonl"))?;
        let splits = splits::derive_splits(&read.events);
        fs::write(
            self.dir.join("splits.json"),
            serde_json::to_vec_pretty(&splits).map_err(io::Error::other)?,
        )?;

        let frames = match workspace {
            Some(workspace) => frames::archive_frames(workspace, &self.dir, &self.run_id)?.len(),
            None => 0,
        };

        let manifest = Manifest {
            run_id: self.run_id.clone(),
            started_unix: self.started_unix,
            finished_unix: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default(),
            ),
            outcome: Some(outcome.to_string()),
            elapsed_ticks: Some(self.elapsed_at(tick)),
            events: read.events.len(),
            frames,
            splits: splits.len(),
        };
        fs::write(
            self.dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
        )?;
        Ok(manifest)
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

#[cfg(test)]
mod finish_tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-finish-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finishing_materialises_splits_and_a_manifest() {
        let root = tmpdir("materialise");
        let mut rec = RunRecorder::start(&root, "r1").unwrap();
        rec.record(
            10,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "researched(automation)".into(),
            },
        )
        .unwrap();
        rec.record(
            310,
            EventKind::MilestoneSatisfied {
                index: 0,
                iterations: 2,
            },
        )
        .unwrap();

        let manifest = rec.finish(400, "done", None).unwrap();
        assert_eq!(manifest.outcome.as_deref(), Some("done"));
        // 400 is where the run ended; it began at tick 10, so it lasted 390.
        assert_eq!(manifest.elapsed_ticks, Some(390));
        assert_eq!(manifest.splits, 1);
        assert_eq!(manifest.frames, 0, "a planning-only run is a valid run");
        assert_eq!(manifest.events, 3, "the finishing event counts");

        let splits: Vec<Split> =
            serde_json::from_slice(&fs::read(rec.dir().join("splits.json")).unwrap()).unwrap();
        assert_eq!(splits[0].elapsed_ticks, Some(300));

        let on_disk: Manifest =
            serde_json::from_slice(&fs::read(rec.dir().join("manifest.json")).unwrap()).unwrap();
        assert_eq!(on_disk, manifest);
    }

    #[test]
    fn the_finishing_event_is_written_before_anything_can_fail() {
        // Archiving from a workspace that does not exist must still leave a log
        // that says how the run ended.
        let root = tmpdir("orderfail");
        let mut rec = RunRecorder::start(&root, "r2").unwrap();
        let missing = root.join("no-such-workspace");
        let _ = rec.finish(99, "stuck", Some(&missing));

        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        assert!(
            matches!(
                read.events.last().map(|e| &e.kind),
                Some(EventKind::RunFinished { outcome, .. }) if outcome == "stuck"
            ),
            "the verdict must survive a failure in the steps after it"
        );
    }

    #[test]
    fn elapsed_ticks_is_a_duration_not_the_tick_it_happened_at() {
        // A run recorded live reported 60246 for a run that lasted 871 ticks:
        // the absolute tick and a duration are both just a number in JSON, and
        // only one of them is right.
        let root = tmpdir("duration");
        let mut rec = RunRecorder::start(&root, "r5").unwrap();
        rec.record(
            59_375,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        let manifest = rec.finish(60_246, "done", None).unwrap();
        assert_eq!(
            manifest.elapsed_ticks,
            Some(871),
            "elapsed must be measured from the run's first observed tick"
        );

        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        match read.events.last().map(|e| &e.kind) {
            Some(EventKind::RunFinished { elapsed_ticks, .. }) => {
                assert_eq!(
                    *elapsed_ticks, 871,
                    "the event must agree with the manifest"
                );
            }
            other => panic!("expected run_finished, got {other:?}"),
        }
    }

    #[test]
    fn a_run_that_never_finished_has_no_manifest_and_keeps_its_events() {
        let root = tmpdir("crashed");
        let mut rec = RunRecorder::start(&root, "r3").unwrap();
        rec.record(
            5,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        // No finish() -- the process died here.
        assert!(
            !rec.dir().join("manifest.json").exists(),
            "a crashed run must not acquire a verdict it never reached"
        );
        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        assert_eq!(read.events.len(), 1, "its events are still readable");
    }
}
