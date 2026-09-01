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
pub mod lanes;
pub mod map;
pub mod retention;
pub mod samples;
pub mod splits;
pub use frames::{ArchivedFrame, archive_frames, parse_frame_name};
pub use lanes::{Lane, derive_lanes};
pub use retention::{DEFAULT_KEEP, KEEP_MARKER, Reaped, reap};
pub use samples::{
    BotSample, PowerSample, ProductionSample, ReadSamples, ResearchSample, Sample, SampleKind,
    ingest_samples, read_samples,
};
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
        /// Why no further work was needed. `run-1788277287-11819` closed a
        /// milestone at the tick it started, with zero iterations, and the
        /// record could not say whether the world already had it or the
        /// planner gave up empty-handed -- this field exists to answer that.
        /// Defaulted for every event recorded before it existed, via
        /// [`SatisfiedReason::unknown`] rather than the ordinary derived
        /// default, so an old record reads as "we don't know" and not as a
        /// guess at either real answer.
        #[serde(default = "SatisfiedReason::unknown")]
        reason: SatisfiedReason,
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
        /// The steps the planner actually produced, in enough detail to draw
        /// the DAG: who runs each one, what it waits on, and when the
        /// planner expected it to start and finish. `#[serde(default)]` so a
        /// run recorded before this field existed keeps opening -- it just
        /// has nothing to draw here.
        #[serde(default)]
        plan: Vec<PlannedStep>,
    },
    ActionDispatched {
        id: u32,
        bot: u32,
        action: String,
        target: Option<Position>,
    },
    /// An action reached a verdict.
    ///
    /// `elapsed_ticks` is null when the game never reported a dispatch tick to
    /// subtract from -- a duration nobody measured, which is not the same as a
    /// duration of zero.
    ActionSettled {
        id: u32,
        bot: u32,
        status: String,
        elapsed_ticks: Option<u64>,
        /// The human-readable verdict, as the game or the executor reported
        /// it. Kept *beside* `failure`, never replaced by it: this is what a
        /// person reads, `failure` is what a query groups by, and one is not
        /// a substitute for the other.
        error: Option<String>,
        /// The same failure, classified. `None` on success, and also on a
        /// failure recorded before this field existed -- `#[serde(default)]`
        /// makes that an absence rather than a parse error.
        #[serde(default)]
        failure: Option<ActionFailure>,
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

/// One scheduled step, as the planner intended it -- carried on
/// [`EventKind::PlanCreated`] so a run's record shows what was planned, not
/// only what happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlannedStep {
    pub id: u32,
    pub bot: u32,
    /// What the plan called it, e.g. `mine 10 iron-ore` -- the same label a
    /// [`Lane`](lanes::Lane) carries, so the two can be read side by side.
    pub action: String,
    /// Ids this step waits on.
    pub deps: Vec<u32>,
    /// Ticks from the plan's *start*, not an absolute `game.tick`: a plan is
    /// computed before it is dispatched and does not know its own origin.
    /// The viewer converts planned ticks to observed ticks in exactly one
    /// place (`observedOrigin()`); a field that already carried an absolute
    /// tick would make that conversion ambiguous about which ticks were
    /// already absolute.
    pub planned_start: u64,
    pub planned_duration: u64,
}

/// Why a milestone needed no work -- carried on
/// [`EventKind::MilestoneSatisfied`].
///
/// A milestone can close after zero iterations for two entirely different
/// reasons, and until this field existed the record could not tell them
/// apart: `run-1788277287-11819` closed milestone 4 at the tick it started,
/// and there was no way to know whether the bots already had what it asked
/// for or the planner returned an empty plan that the supervisor's "an empty
/// plan means satisfied" rule then reported as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SatisfiedReason {
    /// The world already met the goal before planning was ever attempted.
    AlreadySatisfied,
    /// The planner produced no steps. Indistinguishable from
    /// [`SatisfiedReason::AlreadySatisfied`] in a record with only
    /// `iterations: 0` to go on, and not the same thing at all.
    PlanEmpty,
    /// Recorded before this field existed. No live writer ever emits this --
    /// it is only what an old file on disk deserialises to, via
    /// [`SatisfiedReason::unknown`]'s `#[serde(default)]`. Readers must not
    /// treat it as either of the other two variants -- defaulting an old run
    /// to [`SatisfiedReason::AlreadySatisfied`] would be guessing precisely
    /// the thing this type exists to stop guessing, and a live call site
    /// guessing its way to `Unknown` would be the same mistake from the other
    /// direction. `record.milestone_satisfied`'s Lua binding
    /// (`crates/scripting_lua/src/globals/record.rs`) refuses any reason
    /// string it does not recognise rather than falling back here, and
    /// `scripts/supervisor.lua` never has this string to pass in the first
    /// place -- the planner exposes no way for a script to check whether a
    /// goal already holds independently of planning it, so an empty plan is
    /// always reported as [`SatisfiedReason::PlanEmpty`], never guessed at as
    /// [`SatisfiedReason::AlreadySatisfied`].
    #[serde(other)]
    Unknown,
}

impl SatisfiedReason {
    /// The `#[serde(default = "...")]` for [`EventKind::MilestoneSatisfied`]'s
    /// `reason` field. A named function rather than deriving `Default`,
    /// because `Unknown` is a fact about *when the event was recorded*, not
    /// a default anyone should reach for when writing a new one.
    fn unknown() -> Self {
        SatisfiedReason::Unknown
    }
}

/// How an [`ActionFailure`] failed, coarse enough to group by in a query and
/// specific enough to be worth grouping by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    MissingItem,
    Unreachable,
    Blocked,
    Rejected,
    Timeout,
    /// A kind this build does not know, or one not worth a variant yet.
    #[serde(other)]
    Other,
}

/// A structured failure, carried *beside* [`EventKind::ActionSettled`]'s
/// `error` string rather than instead of it: the string is what a person
/// reads when they open the log, `kind` is what a query groups by, and
/// replacing one with the other loses whichever audience it was serving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ActionFailure {
    pub kind: FailureKind,
    /// Free-text detail, e.g. the item name for [`FailureKind::MissingItem`].
    pub detail: Option<String>,
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
    /// How many samples were archived. A count, not a size: a reader deciding
    /// whether to fetch the stream cares how many records it will get.
    #[serde(default)]
    pub samples: usize,
    /// How many lines are in `map.jsonl`. `#[serde(default)]` for the same
    /// reason `samples` has it: a run recorded before this field existed has
    /// no map at all, and that is zero, not a parse failure.
    #[serde(default)]
    pub map: usize,
}

/// Writes a run's event log.
pub struct RunRecorder {
    dir: PathBuf,
    run_id: String,
    events: File,
    /// `map.jsonl`: what got built, and whether the game agreed. Written
    /// straight into the run directory as it happens, exactly like `events`
    /// -- there is no ingestion step, because nothing produces this file but
    /// this recorder.
    map: File,
    started: Instant,
    started_unix: u64,
    /// The game tick of the first event recorded, so a duration can be a
    /// duration. Absent until something has been recorded.
    start_tick: Option<u64>,
    /// The highest tick recorded so far. See [`RunRecorder::not_before`].
    high_tick: u64,
    /// How many lines have been written to `map.jsonl`. Counted here rather
    /// than by rereading the file at `finish`, the way `events` is: `map`
    /// lines are written far less often, and a run-side counter cannot
    /// disagree with what this recorder itself just wrote.
    map_count: usize,
    /// The bounding box of every entity `record_map` has seen placed so far
    /// this run, before any margin. `None` until the first placement, and a
    /// run that never places anything keeps it `None` forever -- that is
    /// exactly the signal [`RunRecorder::placed_bounds`] uses to write no
    /// keyframe at all rather than one over a zero-area box.
    placed_bounds: Option<map::Bounds>,
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
        let map = File::create(dir.join("map.jsonl"))?;
        Ok(Self {
            dir,
            run_id,
            events,
            map,
            started: Instant::now(),
            start_tick: None,
            high_tick: 0,
            map_count: 0,
            placed_bounds: None,
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
        self.high_tick = self.high_tick.max(tick);
        let mut line = serde_json::to_string(&event).map_err(io::Error::other)?;
        line.push('\n');
        self.events.write_all(line.as_bytes())?;
        self.events.flush()
    }

    /// Appends one line to `map.jsonl` and flushes it, for the same reason
    /// `record` flushes `events.jsonl` per line: a crashed run must leave a
    /// readable map, not just a readable event log.
    ///
    /// A [`map::MapKind::Placed`] line also folds its actual position into
    /// [`RunRecorder::placed_bounds`] -- the only bookkeeping this recorder
    /// does beyond writing the line, and the reason a keyframe can be asked
    /// for without the caller tracking placements itself.
    pub fn record_map(&mut self, record: map::MapRecord) -> io::Result<()> {
        if let map::MapKind::Placed { actual, .. } = &record.kind {
            self.placed_bounds = Some(match self.placed_bounds.take() {
                None => map::Bounds {
                    left: actual.position.x(),
                    top: actual.position.y(),
                    right: actual.position.x(),
                    bottom: actual.position.y(),
                },
                Some(b) => map::Bounds {
                    left: b.left.min(actual.position.x()),
                    top: b.top.min(actual.position.y()),
                    right: b.right.max(actual.position.x()),
                    bottom: b.bottom.max(actual.position.y()),
                },
            });
        }
        let mut line = serde_json::to_string(&record).map_err(io::Error::other)?;
        line.push('\n');
        self.map.write_all(line.as_bytes())?;
        self.map_count += 1;
        self.map.flush()
    }

    /// The bounding box of everything placed so far this run, expanded by
    /// `margin` tiles on every side, or `None` when nothing has been placed
    /// yet.
    ///
    /// `None` propagates rather than becoming a zero-area box: a keyframe over
    /// a box nothing has ever occupied is not a fact about the run, it is an
    /// artifact of calling this before anything happened.
    pub fn placed_bounds(&self, margin: f64) -> Option<map::Bounds> {
        self.placed_bounds.as_ref().map(|b| map::Bounds {
            left: b.left - margin,
            top: b.top - margin,
            right: b.right + margin,
            bottom: b.bottom + margin,
        })
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
    /// A clock reading raised to the latest tick already recorded.
    ///
    /// For events recorded *as they happen*, whose only clock is the tick on
    /// the last RCON reply -- which lags, because an action's completion
    /// reaches us through the mod's stdout rather than through a reply, and
    /// nothing about that path touches `last_tick`. Without this a milestone
    /// was stamped 59755 while the action it was waiting for settled at 60238:
    /// finished before its own work did, and the split read 381 ticks for a
    /// span of 864.
    ///
    /// **Only for live events.** Events recorded after the fact -- action
    /// dispatch and settle, replayed from an observation -- carry their own
    /// real ticks and must keep them; clamping those forward would flatten the
    /// timeline onto the moment they were written.
    pub fn not_before(&self, tick: u64) -> u64 {
        tick.max(self.high_tick)
    }

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
        keep: usize,
    ) -> io::Result<(Manifest, retention::Reaped)> {
        // The lower bound for an un-attributed sample (one written before the
        // `run` field existed) must be the run's *start*, not its end: such a
        // sample has no other way to prove it belongs to this run, and a
        // cutoff drawn from the run's last moment would discard essentially
        // every sample recorded while the run was actually in progress.
        // `high_tick` -- the latest tick recorded so far -- is exactly the
        // wrong value here; `start_tick` is the first tick this recorder ever
        // saw, defaulting to 0 for a recorder that logged nothing before
        // finishing.
        let sample_cutoff = self.start_tick.unwrap_or(0);

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

        let samples = match workspace {
            Some(workspace) => {
                samples::ingest_samples(workspace, &self.dir, &self.run_id, sample_cutoff)?
            }
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
            samples,
            map: self.map_count,
        };
        fs::write(
            self.dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
        )?;

        // Reaped *after* this run's manifest exists, so it sorts as the newest
        // and cannot delete itself. Returned rather than logged: a caller must
        // not be able to miss that data went away.
        let reaped = self
            .dir
            .parent()
            .map(|runs_root| retention::reap(runs_root, keep))
            .transpose()?
            .unwrap_or(retention::Reaped {
                deleted: Vec::new(),
                protected: Vec::new(),
            });
        Ok((manifest, reaped))
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
                reason: SatisfiedReason::AlreadySatisfied,
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
                plan: vec![PlannedStep {
                    id: 0,
                    bot: 1,
                    action: "mine 10 iron-ore".into(),
                    deps: vec![],
                    planned_start: 0,
                    planned_duration: 300,
                }],
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
                status: "failed".into(),
                elapsed_ticks: Some(120),
                error: Some("not enough iron-plate in inventory".into()),
                failure: Some(ActionFailure {
                    kind: FailureKind::MissingItem,
                    detail: Some("iron-plate".into()),
                }),
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
    fn a_milestone_satisfied_without_doing_anything_says_which_kind() {
        // run-1788277287-11819 closed milestone 4 in zero ticks and filed the run
        // as done. Either the bots already had the plates or the planner returned
        // an empty plan, and the record could not tell you which.
        let json = serde_json::to_string(&EventKind::MilestoneSatisfied {
            index: 4,
            iterations: 0,
            reason: SatisfiedReason::PlanEmpty,
        })
        .unwrap();
        assert!(json.contains(r#""reason":"plan_empty""#));
    }

    #[test]
    fn an_old_event_without_a_reason_still_reads() {
        // Every run recorded before this field existed must stay openable.
        let old = r#"{"kind":"milestone_satisfied","index":4,"iterations":0}"#;
        let kind: EventKind = serde_json::from_str(old).unwrap();
        let EventKind::MilestoneSatisfied { reason, .. } = kind else {
            panic!("expected milestone_satisfied");
        };
        assert_eq!(reason, SatisfiedReason::Unknown);
    }

    #[test]
    fn a_plan_carries_its_steps_and_their_dependencies() {
        let kind = EventKind::PlanCreated {
            milestone_index: 1,
            steps: 2,
            makespan: 400,
            bots: vec![1, 2],
            plan: vec![
                PlannedStep {
                    id: 0,
                    bot: 1,
                    action: "mine 10 iron-ore".into(),
                    deps: vec![],
                    planned_start: 0,
                    planned_duration: 300,
                },
                PlannedStep {
                    id: 1,
                    bot: 2,
                    action: "craft iron-gear-wheel".into(),
                    deps: vec![0],
                    planned_start: 300,
                    planned_duration: 100,
                },
            ],
        };
        let round: EventKind =
            serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
        assert_eq!(round, kind);
    }

    #[test]
    fn a_failure_keeps_its_string_beside_its_kind() {
        // The string is what a person reads; the kind is what a query groups by.
        // Replacing one with the other loses an audience.
        let failure = ActionFailure {
            kind: FailureKind::MissingItem,
            detail: Some("iron-plate".into()),
        };
        let settled = EventKind::ActionSettled {
            id: 3,
            bot: 1,
            status: "failed".into(),
            elapsed_ticks: Some(120),
            error: Some("not enough iron-plate in inventory".into()),
            failure: Some(failure),
        };
        let json = serde_json::to_string(&settled).unwrap();
        assert!(json.contains("not enough iron-plate"));
        assert!(json.contains(r#""kind":"missing_item""#));
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
                elapsed_ticks: Some(5),
                error: None,
                failure: None,
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
                reason: SatisfiedReason::AlreadySatisfied,
            },
        )
        .unwrap();

        let (manifest, _) = rec.finish(400, "done", None, DEFAULT_KEEP).unwrap();
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
        let _ = rec.finish(99, "stuck", Some(&missing), DEFAULT_KEEP);

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
    fn a_live_event_is_never_stamped_before_something_already_recorded() {
        // The lagging-clock case: an action settled at 60238 is recorded from
        // the observation, then the milestone that was waiting for it is
        // recorded live with a stale reading of 59755. Left alone the
        // milestone finishes before its own work does.
        let root = tmpdir("highwater");
        let mut rec = RunRecorder::start(&root, "r6").unwrap();
        rec.record(
            59_374,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        rec.record(
            60_238,
            EventKind::ActionSettled {
                id: 0,
                bot: 1,
                status: "success".into(),
                elapsed_ticks: Some(483),
                error: None,
                failure: None,
            },
        )
        .unwrap();
        assert_eq!(rec.not_before(59_755), 60_238, "a stale reading is raised");
        assert_eq!(
            rec.not_before(60_500),
            60_500,
            "a reading ahead of the log is left alone"
        );
    }

    #[test]
    fn a_historical_event_keeps_its_own_tick() {
        // Action events are replayed from an observation and carry real ticks
        // from the past. Clamping those forward would flatten the timeline
        // onto the moment they happened to be written.
        let root = tmpdir("historical");
        let mut rec = RunRecorder::start(&root, "r7").unwrap();
        rec.record(
            60_000,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        rec.record(
            59_000,
            EventKind::ActionDispatched {
                id: 0,
                bot: 1,
                action: "mine".into(),
                target: None,
            },
        )
        .unwrap();
        let read = read_events(&rec.dir().join("events.jsonl")).unwrap();
        assert_eq!(
            read.events[1].tick, 59_000,
            "record() must not clamp; only not_before() raises a reading"
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
        let (manifest, _) = rec.finish(60_246, "done", None, DEFAULT_KEEP).unwrap();
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
    fn finishing_reaps_older_runs_and_says_which() {
        let root = tmpdir("reapatfinish");
        // Two older runs already archived.
        for id in ["run-1000-1", "run-2000-1"] {
            let dir = root.join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("events.jsonl"), b"{}\n").unwrap();
        }
        let mut rec = RunRecorder::start(&root, "run-9000-1").unwrap();
        let (_, reaped) = rec.finish(10, "done", None, 1).unwrap();

        assert_eq!(
            reaped.deleted,
            vec!["run-1000-1", "run-2000-1"],
            "the caller is told exactly what went away"
        );
        assert!(
            rec.dir().exists(),
            "a run must never reap itself -- it is the newest, and its manifest \
             is written before the reap so it sorts that way"
        );
    }

    #[test]
    fn finish_counts_the_samples_it_archived() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join("samples.jsonl"),
            "{\"kind\":\"bots\",\"schema\":1,\"tick\":850,\"bots\":[]}\n",
        )
        .unwrap();

        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-1").unwrap();
        rec.record(
            800,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        // A later event before `finish` is what distinguishes the recorder's
        // start tick (800) from its high-water mark (950) -- without this,
        // an ordinary mid-run sample would pass under either cutoff and the
        // test would not catch a regression to the run's last moment.
        rec.record(
            950,
            EventKind::MilestoneStarted {
                index: 2,
                goal: "g2".into(),
            },
        )
        .unwrap();
        let (manifest, _) = rec.finish(1000, "done", Some(&workspace), 5).unwrap();

        assert_eq!(
            manifest.samples, 1,
            "an ordinary mid-run sample (tick 850) must survive: the cutoff is the run's start tick (800), not its high-water mark (950)"
        );
    }

    #[test]
    fn a_run_with_no_sample_file_reports_zero_rather_than_failing() {
        // The mod may not have shipped, or the run may predate sampling. Neither
        // is a reason to lose the events the run did produce.
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-2").unwrap();
        let (manifest, _) = rec
            .finish(1000, "done", Some(&tmp.path().join("workspace")), 5)
            .unwrap();
        assert_eq!(manifest.samples, 0);
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

#[cfg(test)]
mod map_tests {
    use super::*;
    use crate::record::map::{Bounds, EntitySnapshot, MapKind, MapRecord};
    use crate::types::Position;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-record-map-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn snap(name: &str, x: f64, y: f64) -> EntitySnapshot {
        EntitySnapshot {
            name: name.to_string(),
            position: Position::new(x, y),
            direction: 0,
        }
    }

    fn placed_record(tick: u64, bot: u32, at: (f64, f64)) -> MapRecord {
        let snapshot = snap("stone-furnace", at.0, at.1);
        MapRecord {
            tick,
            kind: MapKind::Placed {
                bot,
                intent: snapshot.clone(),
                actual: snapshot,
                drift: None,
            },
        }
    }

    fn read_map(path: &Path) -> Vec<MapRecord> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn record_map_appends_a_line_and_counts_it_in_the_manifest() {
        let root = tmpdir("append");
        let mut rec = RunRecorder::start(&root, "r1").unwrap();
        rec.record_map(placed_record(100, 3, (-12.0, 8.0))).unwrap();
        rec.record_map(placed_record(200, 3, (-10.0, 8.0))).unwrap();

        let lines = read_map(&rec.dir().join("map.jsonl"));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].tick, 100);
        match &lines[0].kind {
            MapKind::Placed {
                bot, actual, drift, ..
            } => {
                assert_eq!(*bot, 3);
                assert_eq!(actual.position, Position::new(-12.0, 8.0));
                assert_eq!(*drift, None);
            }
            other => panic!("expected placed, got {other:?}"),
        }

        let (manifest, _) = rec.finish(300, "done", None, DEFAULT_KEEP).unwrap();
        assert_eq!(
            manifest.map, 2,
            "the manifest counts the lines actually written"
        );
    }

    #[test]
    fn placed_bounds_is_none_before_anything_is_placed() {
        let root = tmpdir("empty");
        let rec = RunRecorder::start(&root, "r2").unwrap();
        assert_eq!(
            rec.placed_bounds(16.0),
            None,
            "a run that placed nothing must not synthesise a zero-area box"
        );
    }

    #[test]
    fn placed_bounds_covers_every_placement_plus_the_margin() {
        let root = tmpdir("bounds");
        let mut rec = RunRecorder::start(&root, "r3").unwrap();
        rec.record_map(placed_record(100, 1, (-12.0, 8.0))).unwrap();
        rec.record_map(placed_record(200, 1, (10.0, 20.0))).unwrap();

        assert_eq!(
            rec.placed_bounds(16.0),
            Some(Bounds {
                left: -12.0 - 16.0,
                top: 8.0 - 16.0,
                right: 10.0 + 16.0,
                bottom: 20.0 + 16.0,
            })
        );
    }
}
