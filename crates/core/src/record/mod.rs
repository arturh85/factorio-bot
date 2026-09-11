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
use std::time::{SystemTime, UNIX_EPOCH};

use crate::graph::entity_graph::EntityGraph;
use crate::types::Position;
use std::sync::Arc;

pub mod planning;
pub mod module_delivery;
pub mod exposure;
pub mod flow;
pub mod lanes;
pub mod map;
pub mod provenance;
pub mod retention;
pub mod rocket_launch;
pub mod run_mode;
pub mod samples;
pub mod savepoint;
pub mod splits;
pub mod standing;
pub mod video;
pub use lanes::{Lane, derive_lanes};
pub use provenance::{
    GitProvenance, PROVENANCE_FILE, Provenance, choose_map_exchange_string, git_provenance,
    read_provenance,
};
pub use retention::{DEFAULT_KEEP, KEEP_MARKER, Reaped, reap};
pub use run_mode::{
    BotMode, RUN_MODE_MARKER, RunMode, clear_run_mode, read_run_mode, write_run_mode,
};
pub use samples::{
    BotSample, IngestProgress, MachineSample, NetworkPower, PowerSample, ProductionSample,
    ReadSamples, ResearchSample, Sample, SampleKind, TravelObservation, ingest_samples_incremental,
    read_samples,
};
pub use savepoint::{
    ModCheck, ModFingerprint, SAVEPOINT_SCHEMA, Savepoint, SavepointError, check_mods,
};
pub use rocket_launch::RocketLaunchEvidence;
pub use splits::{Split, derive_splits};
pub use video::{
    TickSample, VideoManifest, VideoOptions, VideoRecord, VideoRecorder, VideoStatus,
    archive_video, parse_tick_samples, read_video_dir,
};

/// The executor's replay, written beside a run's `events.jsonl` since Phase 2
/// of Run Anatomy. See [`crate::record`]'s module doc for the other file
/// names a run directory holds.
pub const REPLAY_FILE: &str = "replay.json";

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
    /// The world as it stood when a milestone closed was written to disk, and
    /// a later run can start from it. See [`savepoint`].
    ///
    /// Recorded only once the file is *finished* -- `game.server_save` returns
    /// long before the engine has written anything, and a savepoint announced
    /// on the strength of the call returning would be a claim about a file
    /// that may never appear.
    SavepointWritten {
        milestone_index: u32,
        /// Path relative to the run directory, e.g.
        /// `savepoints/milestone-3.zip`. Relative because the archive is
        /// moved and copied, and an absolute path in it would name a machine.
        file: String,
        bytes: u64,
        /// Wall-clock milliseconds between asking and the file being complete.
        /// The one number that says whether saving is cheap enough to do at
        /// every milestone; it is wall clock rather than ticks because the
        /// engine writes outside the tick this was requested in.
        wrote_ms: u64,
    },
    /// A savepoint was asked for and did not arrive.
    ///
    /// Present so that a milestone with no savepoint can be told apart from a
    /// run that never asked for one. Four separate mechanisms in this project
    /// have been found reporting nothing while broken; a failed save is not
    /// going to be the fifth.
    SavepointFailed {
        milestone_index: u32,
        error: String,
    },
    PlanCreated {
        milestone_index: u32,
        steps: u32,
        makespan: u64,
        /// The **roster the planner expanded this plan against**, ascending --
        /// every bot it was allowed to give work to, not the bots it happened
        /// to use. Present-and-null when the caller did not say, never absent.
        ///
        /// Two wrong answers have been recorded here, in opposite directions,
        /// and the field's history is the reason it now comes from the caller:
        ///
        /// 1. **The bots in the steps.** That deleted exactly the bots a reader
        ///    is asking about -- a live run of four recorded `bots: [2]`,
        ///    indistinguishable from a run of one, and "why did bot 4 do
        ///    nothing" could not be asked of it at all.
        /// 2. **The binding's ambient roster**, i.e. the bots the *process* was
        ///    started with. That is not the roster either, whenever a script
        ///    plans with `goal.plan{bots = ...}` -- which the shipped driver
        ///    does, from `rcon.players()`. Run 30 recorded `bots: [1, 2]` for a
        ///    plan made for `[2]` alone (bot 1 was invisible to
        ///    `rcon.players()` for the 750 ticks of freeplay's crash-site
        ///    cutscene, during which `LuaPlayer::character` is nil), so the
        ///    record asserted that a bot had been offered work it was never
        ///    offered. See `docs/superpowers/notes/2026-09-02-bot-one-idle.md`.
        ///
        /// The only thing that knows the answer is whatever called
        /// `goal.plan`, and `plan.bots` is where it reads it. So the Lua
        /// binding takes it as an argument and writes `null` when it is not
        /// given: a plan whose roster nobody stated is a plan whose roster this
        /// record does not know, and the ambient value is a plausible-looking
        /// substitute for it rather than a weaker version of it.
        ///
        /// This is also the field to be most careful with. `plan_created` is
        /// written at planning time and carries the whole DAG, which makes it
        /// the part of an archived run that stays trustworthy where outcomes do
        /// not -- a lie here is worse than a lie in a record already known to
        /// be suspect.
        bots: Option<Vec<u32>>,
        /// The steps the planner actually produced, in enough detail to draw
        /// the DAG: who runs each one, what it waits on, and when the
        /// planner expected it to start and finish. `#[serde(default)]` so a
        /// run recorded before this field existed keeps opening -- it just
        /// has nothing to draw here.
        #[serde(default)]
        plan: Vec<PlannedStep>,
    },
    /// A plan is being executed **right now**, and this is how far it has got.
    ///
    /// # The silence this exists to break
    ///
    /// Every other event about a plan's execution is written *after* the whole
    /// batch has finished: the driver calls `record.actions()`/`record.walks()`
    /// on the supervisor's "ran" transition, once `goal.run` has returned
    /// (`scripts/factory_stage2.lua`). So between `plan_created` and the first
    /// `action_dispatched` the record says **nothing at all**, for however long
    /// the batch takes -- sixteen minutes is an ordinary figure for a
    /// four-bot plan.
    ///
    /// That made two entirely different situations byte-identical from outside:
    ///
    ///   * a run executing a long plan perfectly well, and
    ///   * a run that planned and then dispatched nothing, ever.
    ///
    /// `run-1788465258-49050` is the second one being diagnosed and killed when
    /// it was in fact the first. Its last line was a `plan_created` for a
    /// 152-step plan; fourteen minutes later it was killed as hung. The mod's
    /// own `script-output/botbridge/samples.jsonl` -- which nothing in the run
    /// record reads -- showed bot 1 crafting copper-cable and the force's
    /// iron-plate production climbing at the moment of the kill. Nine of the
    /// twenty-four runs archived at the time end on a `plan_created` with
    /// nothing after it, and not one of them can be told apart from that run.
    ///
    /// # What it is, and what it is not
    ///
    /// It is a heartbeat with counters: written on a fixed wall-clock interval
    /// while a batch is in flight, carrying only facts that were true when it
    /// was written. It states **no verdict** -- there is no `stalled` flag and
    /// no threshold anywhere in the writer, deliberately, because "how long is
    /// too long" depends on the batch and would need tuning, whereas
    /// `dispatched: 0` on a plan of 152 steps needs none: it is unambiguous at
    /// any duration.
    ///
    /// The interval decides the *resolution* of the answer, never its
    /// correctness. A batch shorter than one interval writes none of these and
    /// is none the worse for it -- its `action_dispatched` lines arrive
    /// immediately afterwards.
    ///
    /// It names no milestone and no plan, because the thing that writes it --
    /// the executor's side of `goal.start` -- knows neither: a milestone is a
    /// concept of the supervisor script, and `milestone_index` reaches
    /// [`EventKind::PlanCreated`] from Lua. A reader attributes one of these to
    /// the `plan_created` it follows in the log, which is exact **while one
    /// batch runs at a time** -- the shipped drivers all wait on each run
    /// before planning the next. A script that ran two plans concurrently with
    /// `goal.start` would interleave two heartbeats with nothing to tell them
    /// apart, and that is the case to fix here if it ever arises rather than to
    /// paper over in the reader.
    ///
    /// Its `tick` is the game's own clock, like every live event's (see
    /// [`RunRecorder::not_before`]) -- and unlike the batched
    /// `action_dispatched`/`action_settled` lines, which carry the tick each
    /// action really happened at rather than the moment they were flushed. The
    /// wall-clock durations below are on the variant rather than on
    /// [`Event`] for that exact reason: `Event::wall_ms` was removed because a
    /// batched event's write time is not its happening time, and these two
    /// numbers are honest only because this event *is* written at the moment
    /// it describes.
    BatchProgress {
        /// Wall-clock milliseconds since the executor was handed this batch.
        ///
        /// Wall clock, not ticks: the question this event answers is "is
        /// anything happening", and a stopped game is exactly the case where
        /// the tick clock cannot answer it.
        elapsed_ms: u64,
        /// How many actions the plan has in total -- the denominator for
        /// every count below. Actions only: walks have no `ActionId` and are
        /// counted separately.
        total: u32,
        /// How many actions have been dispatched at least once, i.e. are no
        /// longer `pending`. **This is the number the whole event is for.**
        /// `dispatched: 0` beside a `plan_created` with steps is a plan that
        /// is not being executed, whatever else the run looks like.
        dispatched: u32,
        /// How many are in flight right now: dispatched, no verdict yet.
        in_flight: u32,
        /// How many have reached a verdict of any kind (succeeded, failed or
        /// lost).
        settled: u32,
        /// Of the settled, how many the game judged and refused.
        failed: u32,
        /// Of the settled, how many were acknowledged and never answered.
        /// Kept apart from `failed` here for the same reason
        /// [`EventKind::ActionSettled`] keeps them apart.
        lost: u32,
        /// Actions the run decided not to dispatch because a predecessor did
        /// not succeed, or the bot's walk halted it. **Not** in `dispatched`
        /// or `settled`: the game never saw them. This is the count that says
        /// a batch is being truncated while it is still running -- 48 of 878
        /// in `run-1788914717-24351`, which nothing reported until the batch
        /// was over and the number had become `pending`. `0` on lines
        /// written before the field existed.
        #[serde(default)]
        abandoned: u32,
        /// Walks dispatched and walks settled, counted separately because a
        /// walk has no `ActionId` and appears in none of the counts above.
        ///
        /// Without these a batch whose every bot is walking reports
        /// `in_flight: 0` and reads as four idle bots, which is the misreading
        /// this event exists to prevent rather than to introduce. Walking is
        /// most of the wall clock in these plans.
        walks_dispatched: u32,
        walks_settled: u32,
        /// Wall-clock milliseconds since `dispatched` last went up, or since
        /// the batch began when it has never gone up at all.
        ///
        /// Deliberately measured against *dispatches* and not against settles:
        /// a bot waiting out a legitimate lag edge (a furnace working, machine
        /// time the plan modelled) settles nothing and dispatches nothing, and
        /// this number growing is the honest report of that. It is a
        /// measurement, not an accusation -- nothing here decides what value of
        /// it is too large.
        since_last_dispatch_ms: u64,
        /// Which bots have an action in flight, ascending. Empty is not by
        /// itself a problem -- see `walks_dispatched`.
        bots_in_flight: Vec<u32>,
        /// **What each waiting step is waiting for**, longest wait first.
        ///
        /// # The counters name a bot; this names the work
        ///
        /// Every field above is a number or a bot id, and that was enough to
        /// prove a run was alive and not enough to say what it was doing. On
        /// `2026-09-04` one action sat in flight for **eleven minutes**: the
        /// heartbeats showed frozen counters, `bots_in_flight: [n]`, `lost: 0`,
        /// and nothing else. Identifying which action it was needed a live
        /// `factorio-bot rcon -s localhost` query against the running game,
        /// and even then only by inference -- and a run that dies mid-batch
        /// cannot be queried at all, so the action would never have been
        /// identified. That is not a hypothetical: **a batch's
        /// `action_dispatched` lines are written only after `goal.run`
        /// returns**, so while a batch is in flight the dispatches are not in
        /// the file yet. In that run the newest recorded dispatch was tick
        /// 258,893 while the record's own latest tick was 260,531.
        ///
        /// So each entry names the step (id, bot, the plan's own label, the
        /// target), the state it is in, and how long it has been in it -- all
        /// read from `factorio_bot_executor::ExecutionLog`'s wait registry,
        /// which the executor maintains as it enters and leaves each state.
        ///
        /// # Still no verdict
        ///
        /// `waiting_ms` is a measurement and nothing here compares it against
        /// anything. A 12,000-tick smelt and a reply that will never come both
        /// appear as a growing number; what separates them is
        /// [`WaitingStep::waiting_on`], not a threshold this writer applied.
        ///
        /// # Bounded, and it says when it truncated
        ///
        /// At most [`MAX_WAITING_REPORTED`] entries, longest wait first, with
        /// `waiting_total` giving the true count. The longest wait is
        /// therefore always present, which is the one a reader is looking for.
        ///
        /// # An empty list is a claim, and a checkable one
        ///
        /// Empty means nothing is waiting. It is **contradicted** by
        /// `in_flight > 0`, because a dispatched action is by definition
        /// waiting for its reply, and those two numbers come from different
        /// writers -- the statuses and the wait registry. A heartbeat with
        /// `in_flight: 3` and an empty `waiting` is not a quiet run, it is a
        /// broken reporter, and it is the one shape of failure this project
        /// has hit four separate times.
        #[serde(default)]
        waiting: Vec<WaitingStep>,
        /// How many steps were waiting in total, before the list above was
        /// truncated to [`MAX_WAITING_REPORTED`]. Equal to `waiting.len()`
        /// whenever nothing was dropped.
        #[serde(default)]
        waiting_total: u32,
        /// How many walks have come to rest outside the reach of the action
        /// they served and needed a corrective step, cumulative since the
        /// actuator was built.
        ///
        /// A walk stops on the **outer** ring of its annulus — as far from the
        /// target as the action's reach allows, less
        /// [`crate::factorio::rcon::ARRIVAL_MARGIN`], which is worth
        /// 8,200-10,200 ticks of a four-bot green run. That margin was
        /// measured (128 probe walks; worst overshoot 0.301 tiles) and
        /// measurement is not proof, so the executor re-checks reach after
        /// every walk and this is how often it had to act.
        ///
        /// **It states no verdict either.** Zero says the margin is not being
        /// tested and could be tightened; a number that grows says it is too
        /// thin. Which of those is worth acting on is not decided here, and
        /// nothing compares this against a threshold.
        ///
        /// `#[serde(default)]` so a run recorded before the outer ring existed
        /// still opens — and reads zero, which for such a run is the truth:
        /// its walks stopped on the inner ring and had no margin to be wrong
        /// about.
        #[serde(default)]
        reach_corrections: u32,
        /// **The disclosure for the one act in this project a player cannot
        /// perform.** How many times this batch asked the game to *create
        /// ground* (`generate_chunks`), how many chunks that actually made,
        /// and how many of those asks failed.
        ///
        /// It is here rather than in `provenance.json` for the reason
        /// [`EventKind::VisionMeasured`] gives: provenance is written once at
        /// run start, before any survey has run, and the manifest exists only
        /// for runs that finished -- while a killed run still generated its
        /// ground. This event is flushed per line, so the disclosure survives
        /// the kill.
        ///
        /// **Why it is needed at all.** A bot cannot walk into ungenerated
        /// ground: the pathfinder refuses. A human crosses that edge by
        /// walking and the engine makes the ground as they go; a bot steered
        /// through `request_path` cannot, so exploration asks for the ground
        /// explicitly. The ask is clamped mod-side to the reveal a character
        /// standing there would have got for free, but it still arrives
        /// *before* the bot does, which is a small piece of free vision and is
        /// therefore counted rather than argued about. See
        /// `docs/superpowers/notes/2026-09-06-exploration.md`.
        ///
        /// **States no verdict.** `ground_generate_calls: 0` is a run that
        /// never explored -- every run before 2026-09-06. Calls with zero
        /// chunks and zero failures is ground that already existed; failures
        /// equal to calls is the verb not working, which is the case that
        /// would otherwise be invisible because the executor has no logger.
        ///
        /// `#[serde(default)]` so every run recorded before this existed still
        /// opens, and reads zero -- which for those runs is the truth.
        #[serde(default)]
        ground_generate_calls: u32,
        #[serde(default)]
        ground_generated_chunks: u32,
        #[serde(default)]
        ground_generate_failures: u32,
    },
    ActionDispatched {
        id: u32,
        bot: u32,
        action: String,
        /// Where the plan sent this action, if it sends it anywhere.
        ///
        /// `None` for a `craft`/`research` action, which act on no location --
        /// a true absence, not a gap in what was recorded. Every `mine`,
        /// `place`, `insert` or `remove` action carries a real position here,
        /// always, so `None` can never be read as "we failed to observe the
        /// target" for those four.
        ///
        /// This is the **planner's intent**, not the game's resolution: it
        /// travels straight from `ActionKind::target_position()`
        /// (`crates/planner/src/action.rs`), computed before the action ever
        /// reaches the game. Only a `place` action has a game-side answer to
        /// compare it against at all -- `RconActuator::place` hands back the
        /// entity the game actually created, recorded separately as this run's
        /// `map.jsonl` `placed` line (`actual`, next to `intent`, which is the
        /// same position as this field) -- so a bot sent to a tile the planner
        /// chose that the game resolved elsewhere shows up as a mismatch
        /// between the two records, not inside this one field.
        target: Option<Position>,
        /// What this action put INTO a machine or a chest, when it put
        /// anything in. `None` for every action that delivers nothing -- a
        /// walk, a craft, a place, a `mine`, a `take`.
        ///
        /// # Why the label was not enough
        ///
        /// The quantity was always in `action`, as prose: *"fuel the
        /// stone-furnace with 23 coal (36800 ticks, 153 iron-plate, then it
        /// stops)"*. Reading it back means parsing five different sentence
        /// shapes written at five call sites in the planner, and a shape
        /// nobody anticipated reads as "no delivery" rather than as an error.
        ///
        /// The **hand-credit mass balance** (`tools/run_analysis.py`) needs
        /// these numbers to be right, because it accounts for what the roster
        /// put in against what the machines made and reports `unknown` when a
        /// single delivery cannot be priced. It is the check that replaces a
        /// hand-sized lead-in parameter, which failed *towards a false pass*
        /// on `run-1788674059-90744`
        /// (`docs/superpowers/notes/2026-09-06-standing-goals-first-rung.md`).
        ///
        /// `#[serde(default)]` so every run archived before this field opens
        /// unchanged, and reads `None` -- for those runs the analyser parses
        /// the label and says which source it used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<Delivery>,
    },
    /// An action reached a verdict.
    ///
    /// `elapsed_ticks` is null when the game never reported a dispatch tick to
    /// subtract from -- a duration nobody measured, which is not the same as a
    /// duration of zero.
    ///
    /// EVERY ATTEMPT THAT REACHED A VERDICT GETS EXACTLY ONE OF THESE, whether
    /// or not the game stamped a tick for it. `record.actions`
    /// (`crates/scripting_lua/src/globals/record.rs`) used to make it
    /// conditional on a measured reply tick, which made `status: "lost"`
    /// structurally unrecordable -- a lost action is *defined* by no reply
    /// arriving, so it never has a reply tick -- and `run-1788347034-00981`
    /// recorded 179 `action_dispatched` lines against 170 of these as a result,
    /// nine lost `craft`s with a dispatch and nothing after it. Counting the two
    /// kinds over a run is the cheapest check that this still holds.
    ///
    /// `status` is `"success"`, `"failed"` or `"lost"`, straight from the
    /// executor's `Status`. `failed` and `lost` are different facts and are
    /// never collapsed: `failed` is the game judging the action and saying no,
    /// `lost` is the game acknowledging it and never answering. Only the first
    /// is a verdict; only the second leaves work possibly still outstanding.
    ///
    /// A settle with NO `action_dispatched` beside it is legal and is a finding,
    /// not a gap: the action reached a verdict before the game acknowledged any
    /// dispatch, so there was no dispatch to record.
    ///
    /// `elapsed_ticks` is also what tells a reader that this event's own `tick`
    /// is the record's high-water mark rather than the game's clock (see
    /// [`RunRecorder::not_before`]): a settle the game timed always carries a
    /// duration, so a null one marks a synthesized stamp.
    ActionSettled {
        id: u32,
        bot: u32,
        status: String,
        elapsed_ticks: Option<u64>,
        /// The human-readable verdict, as the game or the executor reported
        /// it. Kept *beside* `failure`, never replaced by it: this is what a
        /// person reads, `failure` is what a query groups by, and one is not
        /// a substitute for the other.
        ///
        /// **Not only failures**, despite the name. `status` is what says
        /// whether anything went wrong; this is whatever this run has to say
        /// about the attempt in words, and three states write it (see
        /// `Attempt::error`, `crates/executor/src/log.rs`). A `success` with a
        /// message is a success that needs qualifying, and today that is
        /// exactly one thing: an `insert` whose destination had no room for
        /// the rest, which reads `destination full: moved 3 of 17 coal, which
        /// now holds 50`. Without it, a run whose every boiler top-up moved 3
        /// of 17 would be indistinguishable from one whose top-ups all moved
        /// 17 -- and the difference is the diagnosis. See
        /// [`FailureKind::PartialTransfer`] for the failure it is kept apart
        /// from.
        error: Option<String>,
        /// The same failure, classified. `None` on success, and also on a
        /// failure recorded before this field existed -- `#[serde(default)]`
        /// makes that an absence rather than a parse error.
        #[serde(default)]
        failure: Option<ActionFailure>,
    },
    /// A bot was sent walking.
    ///
    /// The walking half of [`EventKind::ActionDispatched`], and it exists for
    /// the same reason that one does: without it the record shows a bot at one
    /// place and then at another with nothing in between. Walking is most of a
    /// run's wall clock, and until this variant existed **no walk reached
    /// `events.jsonl` at all** -- run 30 failed three of them and left exactly
    /// one `last_error` string behind, the other two surviving only in
    /// `workspace/server-log.txt`, which the next run overwrites. See
    /// `docs/superpowers/notes/2026-09-02-walks-in-the-record.md`.
    ///
    /// Written only when the game stamped a dispatch tick, exactly as
    /// `action_dispatched` is: a walk the game never acknowledged was never
    /// dispatched, and saying otherwise would be an invention.
    WalkDispatched {
        bot: u32,
        /// Which walk this is: its index in **this bot's own slice** of the
        /// schedule, in schedule order (`run_bot_signalled`,
        /// `crates/executor/src/run.rs`).
        ///
        /// A walk has no `ActionId` -- the scheduler emits it as its own
        /// `StepKind::Walk`, which names no action -- so `(bot, step_index)`
        /// is the only thing that identifies one. **It is neither an
        /// `ActionId` nor an index into [`EventKind::PlanCreated`]'s `plan`**,
        /// which is indexed over every step of every bot and omits walks
        /// entirely. Joining it to either produces confident nonsense, the
        /// same trap [`EventKind::Teleport`]'s `action_id` documents.
        step_index: u32,
        /// Where the **schedule** sent the bot, straight from
        /// `StepKind::Walk`'s own `to`.
        ///
        /// An intent, and the only endpoint this pair records. No *arrival*
        /// position is recorded anywhere here, on purpose:
        /// `on_player_changed_position` fires once per **tile crossed**, so a
        /// bot that comes to rest partway into a tile reports its entry and
        /// nothing corrects it. Every parked bot's observed position is
        /// therefore up to a tile stale -- enough to produce a wrong
        /// diagnosis, and it did. The only *observed* positions in the walk
        /// record are [`WalkFailure`]'s, which the mod wrote at the instant it
        /// gave up.
        ///
        /// This is also routinely a position the bot **cannot stand on**: `to`
        /// comes from the `Condition::AtPosition` the walk exists to satisfy,
        /// so it is often the tile a furnace occupies. Arrival means within
        /// the step's radius of it, never on it.
        to: Position,
        /// The tick the *scheduler* placed this walk's start at, counted from
        /// the plan's start. **Not a `game.tick`**, and not comparable with
        /// this event's own `tick` -- the same two clocks
        /// [`PlannedStep::planned_start`] keeps apart, for the same reason.
        planned_start: u64,
        /// How long the scheduler expected the walk to take, in ticks.
        ///
        /// Carried here because it is carried nowhere else:
        /// [`EventKind::PlanCreated`]'s `plan` holds only the steps that have
        /// an `ActionId`, so a walk's prediction has never been in the record
        /// and a measured walk had nothing to be compared against.
        planned_duration: u64,
    },
    /// A walk reached a verdict.
    ///
    /// EVERY WALK THAT REACHED A VERDICT GETS EXACTLY ONE OF THESE, whether or
    /// not the game stamped a tick for it -- the rule
    /// [`EventKind::ActionSettled`] already states, applied to the other half
    /// of the schedule. It is stated again rather than assumed because the
    /// action side reached it the expensive way: the settle used to be written
    /// inside `if let Some(replied) = replied`, and a lost attempt never has a
    /// reply tick, so `status: "lost"` was structurally unrecordable. This
    /// variant was built with that already fixed.
    ///
    /// `status` is `"success"`, `"failed"` or `"lost"`, straight from the
    /// executor's `Status`. `failed` and `lost` are different facts and are
    /// never collapsed: `failed` is the game refusing the walk, `lost` is the
    /// game acknowledging it and never answering -- a bot that may still be
    /// walking as far as anyone here knows. `pending` and `running` are not
    /// verdicts and write nothing.
    ///
    /// `elapsed_ticks` is null when the walk was not timed at both ends, which
    /// is a duration nobody measured rather than a duration of zero, and is
    /// also what marks this event's `tick` as the record's high-water mark
    /// rather than the game's clock (see [`RunRecorder::not_before`]).
    WalkSettled {
        bot: u32,
        /// The same `(bot, step_index)` identity as
        /// [`EventKind::WalkDispatched`] -- see its documentation.
        step_index: u32,
        /// Where the schedule sent the bot.
        ///
        /// Repeated from [`EventKind::WalkDispatched`] rather than looked up
        /// there, because a walk the game never acknowledged has no dispatch
        /// line at all and a settle carrying only a bot and an index would
        /// name nothing: unlike an action, a walk has no label anywhere in the
        /// record. It is the schedule's destination, with all the caveats
        /// stated on the dispatch -- in particular it is **not** where the bot
        /// ended up.
        to: Position,
        status: String,
        elapsed_ticks: Option<u64>,
        /// The verdict as the game or the executor worded it, kept beside
        /// `failure` and never replaced by it: this is what a person reads,
        /// `failure` is what a query groups by.
        error: Option<String>,
        /// The same failure, classified. `None` on success.
        failure: Option<WalkFailure>,
        /// How many of this bot's remaining steps were abandoned because this
        /// walk failed. `None` on every walk that did not halt its bot.
        ///
        /// **A failed walk halts the bot** -- deliberately, because a walk's
        /// effect is a position and no plan edge carries it
        /// (`factorio_bot_executor::run::run_bot_signalled`). Everything after
        /// it is dropped without ever being dispatched, so it produces no
        /// attempt, no `ActionSettled`, and no trace of any kind: the record
        /// showed those steps as `pending`, which is also exactly what a run
        /// somebody killed early shows.
        ///
        /// `run-1788833726-34821` -- the first run in this project's history
        /// in which a bot died -- ended `success=246 failed=7 lost=1
        /// **pending=2041**` of 2,295, and the 2,041 were four halts. The
        /// largest was **not** a death: bot 3's walk stalled on a `tree-01` at
        /// tick 7,980 and took **474 steps** with it, 19,000 ticks before any
        /// bot was killed. Three deaths accounted for the other three halts
        /// and 1,572 steps. None of that was derivable from the record as it
        /// stood; it took reconstructing the plan and diffing it against the
        /// dispatches.
        ///
        /// `Some(0)` is written for a bot halted on its own last step and
        /// means "halted, taking nothing with it" -- a different fact from
        /// `None`, which means "did not halt".
        abandoned: Option<u32>,
        /// Every stall this walk was retried through, in the order the game
        /// answered them. See [`WalkStallRecord`] for what their absence cost.
        ///
        /// **`null` and `[]` are different answers.** `null` is "nobody was
        /// watching for stalls on this walk" -- an actuator that does not
        /// report them, or a run archived before this field existed, which
        /// `#[serde(default)]` makes an absence rather than a parse error.
        /// `[]` is "watched, and this walk did not stall". A reader that
        /// collapses them turns an uninstrumented run into a clean one, which
        /// is the substitution this field was added to end.
        ///
        /// A **failed** walk's last stall is not here: the attempt that is not
        /// retried becomes this event's own `error` and `failure.kind:
        /// stalled`. So a stalled failure's true total is `stalls.len() + 1`,
        /// and on a successful walk `stalls.len()` is the whole story.
        #[serde(default)]
        stalls: Option<Vec<WalkStallRecord>>,
    },
    /// A bot was moved by `player.teleport` rather than by walking.
    ///
    /// Before this variant existed, none of the mod's three teleport sites
    /// were observable to Rust at all: `on_player_changed_position` fires
    /// identically whether the position changed by walking or by teleport, so
    /// a run whose bots teleported repeatedly recorded ordinary-looking walk
    /// durations with nothing to say otherwise. Every walk duration in a
    /// record from before this variant was ever emitted may be fiction for
    /// that reason, and there is no way to tell after the fact -- this event
    /// only exists going forward.
    Teleport {
        bot: u32,
        /// Why the mod teleported the bot rather than moving it normally --
        /// e.g. `walk_stuck` (the leg timed out), `revive_ghost_blocked` or
        /// `place_blueprint_blocked` (the bot stood inside the entity's
        /// bounding box). Free text rather than an enum: the set of reasons
        /// is small and mod-defined, and a reader filtering on it can match
        /// substrings without this crate publishing a closed list the mod
        /// must stay in lockstep with.
        reason: String,
        from: Position,
        to: Position,
        distance: f64,
        /// The walk action this teleport happened during, when there is one.
        /// `None` for the two blueprint/ghost-revive sites, which are
        /// synchronous RCON calls with no dispatched action to attach to.
        //
        // NOT JOINABLE TO `ActionDispatched::id`, and the two names look far
        // more alike than the things they name. This is the RCON/mod action id
        // minted by `FactorioRcon` from `FactorioSurface::next_action_id`, a
        // run-global counter that wraps at 1000; `ActionDispatched::id` is the
        // planner's `ActionId`, which restarts at 0 with every plan. Joining
        // them produces confident nonsense.
        //
        // Worse, there is no join to be had even in principle: a `walk_stuck`
        // teleport belongs to a *walk leg*, and a walk is a `StepKind::Walk`
        // with no `ActionId` at all (see `build_observation`'s `walks` array,
        // keyed by `(bot, step_index)`). So "which action was this bot walking
        // for" cannot be answered by unifying these two fields. Answering it
        // needs walks in the record -- there is no walk `EventKind` today, so
        // `obs.walks` never reaches `events.jsonl` -- plus the mod's action id
        // carried back out of `FactorioRcon::move_player` and attached to the
        // walk it belonged to. See
        // `docs/superpowers/notes/2026-09-02-actions-that-never-settle.md`.
        action_id: Option<u32>,
    },
    /// The game refused a build, and the planner has stopped offering that
    /// site.
    ///
    /// Written by `record.refusals()` (`crates/scripting_lua`) from the
    /// ledger two writers fill: `FactorioRcon::place_entity_timed`, when a
    /// dispatched build is refused *without* naming the acting player as the
    /// cause, and `FactorioRcon::can_place_entities`, the pre-flight check
    /// `goal.plan` runs over a plan's chosen sites before returning it.
    /// `source` says which. From here to the end of the run,
    /// `PlanState::from_world` excludes the collision box of `entity` centred
    /// at `position`, so every later plan sites around it.
    ///
    /// That consequence is the reason this is a record line at all. A planner
    /// that has silently started preferring distant tiles is a planner nobody
    /// can diagnose; four consecutive runs were spent on refusals whose only
    /// trace was an error string, and the next reader should be able to see
    /// which ground the planner has written off and when it learned to.
    ///
    /// A refusal the acting player caused is deliberately absent: the mod
    /// names that one, the RCON layer walks the bot aside and retries it, and
    /// the ground is fine once the bot moves.
    PlacementRefused {
        /// The item the bot was holding, which is the name the collision box
        /// excluded from later plans is looked up under.
        entity: String,
        /// The centre the build was aimed at. The excluded region is that
        /// entity's collision box centred here, not this single tile -- the
        /// game tested the box, so the box is what the refusal is about.
        position: Position,
        /// The `defines.direction` the build was aimed with, when known. The
        /// box the game tested is the prototype's box turned this way, so
        /// this is what makes the excluded region the right shape for a
        /// building that is not square: a steam engine facing east claims
        /// 4.7 by 2.5 tiles, not 2.5 by 4.7. `null` on lines written before
        /// the direction was carried.
        direction: Option<u8>,
        /// `"dispatch"` or `"pre_check"` -- whether a bot flew to this site
        /// and was refused, or the planner asked before committing to it.
        ///
        /// The two cost very different things and a reader must not have to
        /// guess which happened. `dispatch` means an action failed here and
        /// its dependents were abandoned; there is an
        /// [`EventKind::ActionSettled`] failure beside it. `pre_check` means
        /// no action was ever created for this site: `goal.plan` asked the
        /// game, re-expanded, and the plan that reached the executor sites
        /// somewhere else. There is deliberately **no** `action_settled` line
        /// next to a `pre_check` refusal, and its absence is not a gap.
        source: String,
        /// The distinct names of the entities the game found in the tested
        /// collision box, sorted.
        ///
        /// Filled on both sources. The pre-check always reported it; a
        /// `dispatch` refusal carries it since the mod started appending what
        /// it found to its `said 'no'` line (`rcon_place_entity`,
        /// `mods/BotBridge/control.lua`) -- before that it was always empty
        /// here, and that absence of a cause is what five consecutive runs
        /// were spent on. Empty now means the game scanned the box and found
        /// no entity in it, so the ground itself is the answer -- see `tile`.
        /// Empty **and** `tile: null` on a `dispatch` line is the older
        /// shape, where nothing was asked.
        blockers: Vec<String>,
        /// The tile under the refused centre, when the game named it. `None`
        /// only on a `dispatch` line whose reply named nothing.
        tile: Option<String>,
    },
    /// A character cannot reach open ground from where it stands.
    ///
    /// **The condition this whole event exists for went unnamed for a whole
    /// run.** In `run-1788432181-42528` bots 2 and 3 reported byte-identical
    /// positions from tick ~48 000 to the end -- 77% of the run -- while bot 1
    /// performed 746 of 831 dispatches. Twenty walks failed, none of them
    /// bot 1's. Every artefact the run produced was consistent with a
    /// scheduling quirk, and the truth was found only by reading
    /// `samples.jsonl` by hand. A `walk_settled` with `failure.kind: "no_path"`
    /// says a *destination* could not be reached; it cannot say the bot could
    /// reach nothing at all, and nineteen of them in a row still cannot.
    ///
    /// Written by `record.enclosures()` (`crates/scripting_lua`) from the
    /// ledger `crates/executor`'s `walk_memory` fills. The check runs when the
    /// game's pathfinder has refused a route from this spot, so the event
    /// always sits beside a failed walk -- it is the *diagnosis* of that
    /// failure, not a second report of it.
    ///
    /// Nothing acts on this. It changes no plan and moves no bot; it exists so
    /// the next reader can see the condition without reconstructing it.
    BotEnclosed {
        bot: u32,
        /// Where the character stood. The fill was seeded here, so this is the
        /// position the claim is about rather than an approximation of it --
        /// and unlike a walk's `to`, it is *observed*: the world reported it
        /// at the instant the path was refused.
        position: Position,
        /// How much ground is still reachable, in whole tiles of the game's
        /// own pathfinding grid -- a tile counts when the character's box
        /// centred on it is clear, which is the test `request_path` makes.
        /// The tile the character stands on is always one of them. Reported
        /// because "boxed into 4 tiles" and "boxed into 300" are different
        /// situations.
        pocket_tiles: f64,
        /// How far the fill was allowed to look, in tiles.
        ///
        /// This is what bounds the claim, and it is on the event rather than
        /// implied by the build that wrote it: an enclosure wider than this
        /// window is invisible to the search and produces **no event at all**.
        /// So the absence of this event is not evidence that no bot was walled
        /// in -- only that none was walled into a pen this small. A reader
        /// comparing two runs whose builds disagree about the radius needs the
        /// number that was actually used.
        searched_tiles: f64,
    },
    /// The game said this bot cannot move, and the next plan will not send it
    /// anywhere.
    ///
    /// After a walk the pathfinder refused, the executor asked the game for a
    /// short path from the character in each of four directions
    /// (`FactorioRcon::probe_player_hops`) and every one was refused. That
    /// is the game's own verdict on the *character*, which neither
    /// [`EventKind::BotEnclosed`] (a fill over this process's occupancy
    /// model) nor a `no_path` walk (a verdict on one destination) can give:
    /// in `run-1788614781-38058` bot 6 stood overlapping a furnace, the fill
    /// said open, four walks said `no_path`, and seven plans in a row sent it
    /// the same walk until the run halted `stuck` with seven healthy bots
    /// idle.
    ///
    /// Unlike `bot_enclosed`, this one is acted on: `crates/planner`'s
    /// `PlanState::from_world` reads the bench and gives the bot no step
    /// that would need it to walk. It is lifted by [`EventKind::BotReleased`].
    ///
    /// Written by `record.enclosures()` from `FactorioSurface::benches`.
    BotBenched {
        bot: u32,
        /// Where the character stood when every hop was refused. Observed.
        position: Position,
        /// How many hops were asked for and refused.
        refused_hops: u32,
        /// How far each hop was aimed, in tiles.
        hop_tiles: f64,
    },
    /// A benched bot can move again, and the next plan may send it.
    ///
    /// Written by `record.enclosures()` when the executor lifted a bench:
    /// `why` is `"walked"` when a walk for the bot succeeded, or `"probed"`
    /// when the re-probe before a plan found a hop the game would path.
    BotReleased {
        bot: u32,
        /// Where the bench had been earned.
        position: Position,
        why: String,
    },
    /// A bot was walked clear of a placement that would otherwise have sealed
    /// it in -- the [`EventKind::BotEnclosed`] that did not happen.
    ///
    /// Written by `record.enclosures()` from the queue `crates/executor`'s
    /// pre-place check fills (`FactorioSurface::step_asides`). The check runs
    /// before every placement: with the footprint added to the occupancy
    /// model, would the fill around the character about to build it close?
    /// In `run-1788552801-73005` it would have, and did -- bot 1 placed an
    /// `assembling-machine-1` from the one patch of ground whose every exit
    /// crossed it, and stood there for 40 000 ticks.
    ///
    /// This is a bot doing something the plan did not ask for. It is in the
    /// record so that a reader comparing the plan to what happened can see
    /// why a `place` was preceded by a walk the schedule has no step for.
    BotSteppedAside {
        bot: u32,
        /// Where the character stood when it was about to place. Observed.
        from: Position,
        /// The tile centre it was walked to: the nearest tile reachable from
        /// `from` that stays connected to open ground once the placement
        /// stands and is still within building reach of `site`.
        to: Position,
        /// The entity that was about to be placed.
        placing: String,
        /// Where it was about to be placed.
        site: Position,
        /// `"enclosure"` or `"footprint"`: whether the placement would have
        /// walled the character in, or would have stood on the tile the
        /// character was standing on. The second is the acting bot inside
        /// its own placement's collision box -- the game refuses that build
        /// for the actor's sake, and `run-1788569499-05724` lost a 407-step
        /// plan to exactly that refusal being read as a verdict about the
        /// ground.
        reason: String,
        /// How many tiles the character would have been left with, had it
        /// stayed -- the same count [`EventKind::BotEnclosed`] reports. `0`
        /// for `reason = "footprint"`, where the placement stands on the
        /// character's own tile.
        pocket_tiles: f64,
    },
    /// A bot lost its character: the game's `on_player_died`, as the mod
    /// reported it.
    ///
    /// Before this variant, nothing in the record could say a bot had died.
    /// A dead player keeps its `LuaPlayer` and loses its character, the mod's
    /// every entry point answered `not connected` for it, and the roster --
    /// fixed at script start -- kept assigning it work. So a killed bot
    /// degraded into an ordinary stream of failures with no distinguishing
    /// kind, the same silent roster degradation run 30 paid for from the
    /// other direction (`docs/superpowers/notes/2026-09-02-bot-one-idle.md`).
    /// This event, [`FailureKind::NoCharacter`] and
    /// [`EventKind::RosterChanged`] are the three halves of the fix.
    ///
    /// Written by `record.deaths()` (`crates/scripting_lua`), drained from
    /// the queue `OutputParser` fills. **No archived run contains one**: the
    /// nearest charted enemy structure on the benchmark seed is 246 tiles
    /// out, so the first real instance of this event will be produced by the
    /// first run that goes there.
    BotDied {
        bot: u32,
        /// Where the character stood when it died, as the mod read the
        /// ghost's position. `None` when the mod could not read one.
        position: Option<Position>,
        /// The entity that did it (`medium-worm-turret`, `small-biter`), when
        /// the game named one. `None` is "the game named nothing", which it
        /// does for a death with no attacker.
        cause: Option<String>,
        /// Its prototype type (`turret`, `unit`). Null exactly when `cause` is.
        cause_type: Option<String>,
        /// `ticks_to_respawn` as it stood the tick after death. The default
        /// character prototype respawns after 600; `None` means the mod could
        /// not read the timer, not that the bot will never respawn.
        respawn_in: Option<u32>,
    },
    /// The character is back: the game's `on_player_respawned`.
    ///
    /// Pairs with [`EventKind::BotDied`] by `bot`; the gap between the two is
    /// the stretch during which every action for that bot was refused with
    /// [`FailureKind::NoCharacter`].
    BotRespawned {
        bot: u32,
        /// Where the new character stands -- the spawn point, ordinarily,
        /// which is not where the walk in flight was going.
        position: Option<Position>,
    },
    /// The mod completed a Factorio 2.0 *trigger* technology on a headless
    /// run because the force had already done what the trigger names.
    ///
    /// A server-side character has no player, and the game fires `craft-item`
    /// for machine output but never for a character's hand craft, and never
    /// `build-entity` for a mod-placed entity
    /// (`docs/superpowers/notes/2026-09-05-research-triggers.md`). So
    /// `emulate_research_triggers` in `mods/BotBridge/control.lua` completes
    /// such a technology from the counters it keeps, only once every
    /// prerequisite is researched, and writes this line with the count that
    /// earned it. It is the answer to "why does `on_research_finished` for
    /// `automation-science-pack` appear with no research ever started?", and
    /// a run whose log has the finish without this line is one in which the
    /// game fired the trigger itself. Written by `record.research_triggers()`.
    ResearchTriggerEmulated {
        technology: String,
        /// The trigger's `type`: `craft-item` or `build-entity`.
        trigger: String,
        /// The item a `craft-item` trigger counted; `null` for `build-entity`.
        item: Option<String>,
        /// The entity a `build-entity` trigger counted; `null` for `craft-item`.
        entity: Option<String>,
        /// What the trigger asked for.
        needed: u32,
        /// What the force had done when the sweep read it -- at least
        /// `needed`, by construction.
        count: u32,
    },
    /// The mod discarded generated chunks because they are not on Nauvis.
    ///
    /// **This project supports exactly one surface, and this is the line that
    /// says when it met a second.** `on_chunk_generated` in
    /// `mods/BotBridge/control.lua` returns early for any surface but Nauvis,
    /// so those chunks' entities, resources and tiles never reach
    /// `EntityGraph`. The guard is deliberate -- the world model keys by
    /// position alone, so a Vulcanus chunk would merge into Nauvis with no
    /// error anywhere -- but until 2026-09-06 it was also invisible: it
    /// printed a bare `"unknown surface"` with no `§tick§key§` envelope, which
    /// reached the server log and no artefact. A run that discarded a whole
    /// planet read exactly like one that never left home.
    ///
    /// **Space Age is enabled in this workspace** (`workspace/mods/
    /// mod-list.json`), so every run ever measured here has been a Space Age
    /// run that happened never to leave Nauvis. A single one of these rows
    /// means the world model this run planned against is incomplete, and any
    /// map fingerprint, resource distance or `entity_at` answer from it
    /// describes Nauvis only. Written by `record.surface_chunks_dropped()`;
    /// see `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.
    SurfaceChunkDropped {
        /// The surface that was refused, by name (`LuaSurface.name`, which is
        /// unique among surfaces; the *index* is reused after a deletion).
        surface: String,
        /// How many chunks were dropped for this surface since the last
        /// flush -- **not** the run's total, which is the sum over every row
        /// naming this surface.
        chunks: u64,
        /// One example: the top-left **tile** of the first chunk dropped in
        /// this window -- (-32, 64), not chunk (-1, 2). A count alone cannot
        /// be looked at; a coordinate can.
        first_left_top_x: f64,
        first_left_top_y: f64,
    },
    /// The supervisor changed the roster it plans for.
    ///
    /// The roster used to be computed once at script start and never again.
    /// `scripts/supervisor.lua` now re-checks `rcon.players()` before every
    /// plan: a rostered bot that has had no character for longer than the
    /// bounded respawn wait is dropped, and one that was dropped and came
    /// back is picked up again. Written by `record.roster_changed()` from the
    /// supervisor's `rerostered` transition, so the reader of a run whose
    /// `plan_created.bots` shrinks from four to three finds the line that
    /// says why, rather than the silence run 30 left.
    ///
    /// **Only removals and returns, never additions.** A bot that was never
    /// in the roster this run started with is not picked up mid-run, because
    /// "the first non-empty answer is the roster" is the bug that froze run
    /// 30 on `[2]`, and re-rostering must not reintroduce it.
    RosterChanged {
        /// The roster from here on, ascending.
        bots: Vec<u32>,
        /// Bots that were in the roster and are not any more.
        left: Vec<u32>,
        /// Bots that had been dropped earlier in this run and are back.
        returned: Vec<u32>,
        /// The supervisor's own sentence for why: which bots it waited for,
        /// for how many checks, and what it found.
        reason: String,
    },
    /// How much ground the world model was given, against how much ground a
    /// bot actually covered.
    ///
    /// # The cheat nobody had counted
    ///
    /// The mod ingests every entity of every chunk the *engine generates*
    /// (`on_chunk_generated`, `mods/BotBridge/control.lua`) and never consults
    /// the force's charted area. That is `force.chart` with extra steps: the
    /// model gains complete knowledge of ground no character has been near,
    /// including ore, water and nests, and it has been in every run this
    /// project has ever made. In `run-1788532631-48030` the furthest any bot
    /// reached was **63.8 tiles** while the model that run produced held crude
    /// oil at **380 and 505 tiles**, 559 uranium tiles and 36 biter spawners
    /// out to 500.
    ///
    /// The repo's rule is that a final measured run must be as cheat-free as
    /// we can get it, and that whatever is not must be *recorded*. This event
    /// is that record. It changes nothing and refuses nothing -- like
    /// [`EventKind::BatchProgress`] it states no verdict, because "how much
    /// free vision is too much" is a judgement for the reader of a result and
    /// not for the writer of a measurement. See
    /// `docs/superpowers/specs/2026-09-04-exploration-design.md`.
    ///
    /// # Why an event rather than `provenance.json` or the manifest
    ///
    /// [`provenance::Provenance`] is written once at run start and never
    /// rewritten, which is right for what a run was *launched* with and wrong
    /// for a figure whose second half only exists once bots have moved.
    /// [`Manifest`] is written in [`RunRecorder::finish`] and so exists only
    /// for runs that finished -- nine of the twenty-four runs archived when
    /// provenance was added had no manifest at all, and they are the killed
    /// ones. A run that was killed still got its free vision, so the
    /// disclosure has to survive the kill.
    ///
    /// So it is written into `events.jsonl`, which is flushed per line: once
    /// at the run's first recorded event (a baseline, before any bot has
    /// moved), every [`VISION_MEASURE_INTERVAL_TICKS`] thereafter, and once
    /// more at `finish` after the last samples are ingested. The **last** one
    /// in the log is the run's answer; the earlier ones show it growing.
    ///
    /// # What a reader sees when the measurement is short
    ///
    /// Every field that can be unknown is `Option`, and each `None` has a
    /// counter beside it that says why -- see `travelled_tiles` and
    /// `model_tiles`. A run with no travel data reports `null` and
    /// `bot_samples: 0`, never `0.0`: a bot that was never observed has not
    /// been observed at the origin.
    VisionMeasured {
        /// The furthest any bot has been observed from the map origin so far
        /// this run, in tiles, Euclidean.
        ///
        /// **`None` is "nobody has observed a bot position", not "no bot
        /// moved".** It comes from the archived sample stream, so it is null
        /// for the whole of a run whose sampling never started, and null on
        /// the baseline measurement of every run -- the first event is
        /// recorded before the first sample is ingested. `bot_samples` is what
        /// separates the two.
        travelled_tiles: Option<f64>,
        /// Which bot was that far out. Null exactly when `travelled_tiles`
        /// is. A single distance for a whole run hides which bot earned it,
        /// and "one bot went and three sat still" is a finding this project
        /// has already had to reconstruct by hand once.
        travelled_bot: Option<u32>,
        /// The tick that bot was seen there. Null exactly when
        /// `travelled_tiles` is. Not the tick of this event: the high-water
        /// mark is carried forward from whenever it was set.
        travelled_at_tick: Option<u64>,
        /// How many bot positions this run has archived so far -- the
        /// denominator behind `travelled_tiles`. Zero beside a null distance
        /// means the instrument said nothing; non-zero beside a null distance
        /// would mean the instrument spoke and this writer dropped it, which
        /// is a defect and is visible here rather than invisible.
        bot_samples: u32,
        /// The furthest resource tile or enemy structure the world model
        /// holds, measured from the map origin, in tiles. `None` when the model holds no resource tile and no enemy
        /// structure at all -- a world nothing has read yet, which is not a
        /// model that reaches zero tiles.
        ///
        /// **Bounded by the mod, not by the map.** `on_chunk_generated` drops
        /// every chunk outside a +/-512-tile box (`control.lua`, unexplained
        /// since the mod's first commit in 2021), so this can never exceed
        /// `hypot(512, 512)` = 724 and a value near that is the model sitting
        /// against the mod's wall rather than the edge of what exists. A run
        /// whose figure is at the wall has no idea how much more it was not
        /// shown.
        model_tiles: Option<f64>,
        /// What that furthest thing is: an entity name such as `"crude-oil"`
        /// or `"biter-spawner"`. A distance alone reads as an abstraction;
        /// the name is what makes a reader look. Null exactly when
        /// `model_tiles` is.
        model_furthest: Option<String>,
        /// Where it is. Null exactly when `model_tiles` is.
        model_furthest_position: Option<Position>,
        /// How many resource tiles the model holds -- the census behind
        /// `model_tiles`. Zero beside a null `model_tiles` is a consistent
        /// "nothing has been read"; non-zero beside a null would be the same
        /// defect `bot_samples` guards against.
        model_resource_tiles: u32,
        /// How many enemy structures the model holds. Counted apart from the
        /// resource tiles because a nest 500 tiles out and an ore tile 500
        /// tiles out are the same disclosure but not the same finding.
        model_enemy_structures: u32,
        /// `model_tiles / travelled_tiles`: how many times further the model
        /// sees than the furthest bot went.
        ///
        /// The one number this event exists to produce, and `None` whenever
        /// either half is unknown or no bot has left the origin. It is
        /// **not** floored at 1: a value below 1 means the bots outran the
        /// model, which is a legitimate reading and not an error to clamp
        /// away.
        unearned_ratio: Option<f64>,
    },
    /// What one `goal.plan` cost: wall time, and what the game clock did
    /// while the planner thought.
    ///
    /// The planner is wall-clock work -- ~3 s for automation, ~20 s for green
    /// -- and a game left running through it is charged `60 * speed` ticks
    /// per second of thinking: 334 ticks for automation at 1x, 1,837 at 10x,
    /// 6,438 for green at 5x (2026-09-05). That was the whole of the
    /// "faster game, longer run" tax. `goal.plan` now pauses the clock around
    /// expansion, and this event is the receipt: `tick_after - tick_before`
    /// is what the run was charged, which is zero when `paused` held.
    ///
    /// Written by the plan itself rather than folded into `plan_created`,
    /// which the driver script records later with the plan's steps: a plan
    /// that raised has no `plan_created` and still cost time.
    PlanningTimed {
        /// Wall clock spent inside expansion and scheduling, including the
        /// placement pre-check round trips.
        planning_ms: u64,
        /// Whether the game clock was stopped for the duration. `false` on a
        /// build without RCON, or when the pause request failed -- in which
        /// case `tick_after - tick_before` says what it cost.
        paused: bool,
        /// Why the clock was left running, when it was: `"attached server,
        /// clock left running"` for a `--connect` / `--server` run, whose
        /// game may be somebody's live multiplayer session, or `"pause
        /// request failed"`. `None` whenever `paused` is true, and on a
        /// build with no game to ask.
        reason: Option<String>,
        /// `game.tick` when planning began; `None` when nobody could ask.
        tick_before: Option<u64>,
        /// `game.tick` when planning ended; `None` when nobody could ask.
        tick_after: Option<u64>,
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

/// How many [`WaitingStep`]s one [`EventKind::BatchProgress`] carries at most.
///
/// A batch has one exclusive step per bot plus whatever background work each
/// bot queued, so a four-bot plan waits on a handful of things at once; eight
/// covers that with room to spare. The cap exists because this event beats
/// every thirty seconds into a file that is already the largest thing a run
/// writes, and because a plan with 152 steps blocked behind one predecessor
/// would otherwise put 151 near-identical entries on every line.
///
/// Truncation is never silent: `waiting_total` carries the real count, and the
/// list is sorted longest-wait-first so the entry a reader is looking for is
/// the one that survives.
pub const MAX_WAITING_REPORTED: usize = 8;

/// What a bot's hands put into a machine or a chest, on
/// [`EventKind::ActionDispatched::delivery`].
///
/// **The planner's intent, not the game's answer**, on the same terms as
/// [`EventKind::ActionDispatched::target`]: it travels from
/// `ActionKind::Insert`'s own fields, which is what the run asked for. The
/// action's settle says whether the game did it; nothing here does.
///
/// One variant of one enum reaches this struct -- `ActionKind::Insert` --
/// and it covers every hand delivery the planner can express, fuel included:
/// a `fuel` action IS an `Insert` into `InventorySlot::Fuel`, and the label
/// is the only place the two are spelled differently. `ActionKind::Remove`
/// (a `take`) deliberately writes nothing here: it moves material *out* of a
/// machine and into a bot, which credits no production.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Delivery {
    /// What was put in.
    pub item: String,
    /// How many. A count of items, never of stacks.
    pub count: u32,
    /// The prototype name of what received it -- `stone-furnace`,
    /// `burner-mining-drill`, `wooden-chest`. Needed because the same coal
    /// buys 26.7 s in a drill and 44.4 s in a furnace, so a fuel delivery
    /// cannot be converted into output without knowing which.
    pub entity: String,
    /// Which inventory: the planner's own `InventorySlot` name, as
    /// `goal.plan`'s steps publish it (`fuel`, `furnace_source`,
    /// `assembler_input`, `chest`, ...) rather than Factorio's unified
    /// `defines.inventory` key, so a reader can still tell a furnace's input
    /// from an assembler's.
    pub slot: String,
}

/// One step that is waiting rather than working, carried on
/// [`EventKind::BatchProgress`].
///
/// The identity half is deliberately the same three fields
/// [`EventKind::ActionDispatched`] carries -- `id`, `bot`, `action`, `target`
/// -- so a reader who finds a step here and later finds its dispatch line is
/// looking at the same names, and can join on `id` without a second vocabulary
/// to learn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WaitingStep {
    /// The action's `ActionId`, joinable to [`EventKind::ActionDispatched`]'s
    /// `id` and to [`PlannedStep::id`].
    ///
    /// `None` for a **walk**, which has no `ActionId` at all -- the scheduler
    /// emits it as its own step kind. A true absence, exactly as in
    /// [`EventKind::WalkDispatched`], and not a failure to look it up. A walk
    /// is identified by `bot` plus `step_index` instead.
    pub id: Option<u32>,
    /// Which walk this is, in this bot's own slice of the schedule -- the same
    /// key [`EventKind::WalkDispatched::step_index`] uses, and subject to the
    /// same warning: **it is not an `ActionId` and not an index into
    /// [`EventKind::PlanCreated`]'s `plan`.** `None` for an action.
    pub step_index: Option<u32>,
    /// The bot whose step this is, from the schedule -- the only thing that
    /// knows, since an action does not carry its bot.
    pub bot: u32,
    /// What the plan called it, verbatim: `Action::label`, the same string
    /// [`EventKind::ActionDispatched::action`] and [`PlannedStep::action`]
    /// carry. For a walk, a description of the walk, because a walk has no
    /// label of its own.
    pub action: String,
    /// Where the plan sent it, when it sends it anywhere. Same meaning and
    /// same caveats as [`EventKind::ActionDispatched::target`]: the planner's
    /// intent, `None` for a `craft`/`research`, and the walk's destination for
    /// a walk.
    pub target: Option<Position>,
    /// **What it is waiting for.** One of `predecessor`,
    /// `background_conflict`, `lag_deadline`, `research`, `reply` or `walk` --
    /// `factorio_bot_executor::WaitKind::name`, which is a stable string
    /// rather than a `Debug` rendering so archived runs keep grouping with new
    /// ones.
    ///
    /// The three that used to be indistinguishable are `predecessor` (blocked
    /// on another step, possibly another bot's), `lag_deadline` (serving
    /// machine time the plan asked for -- **not a fault**, however long) and
    /// `reply` (dispatched, and the game has not answered).
    ///
    /// A `reply` whose `waiting_ms` is past ~360,000 is a finding on its own:
    /// the RCON layer gives a dispatched action 360 wall-clock seconds and
    /// then reports it lost, so a longer one means the time went into a path
    /// request, an out-of-reach `move_player` or the placement retry loop --
    /// all of which happen inside the same call. This is why the run that
    /// prompted the field reported `lost: 0` through an eleven-minute silence
    /// and was **right** to.
    pub waiting_on: String,
    /// The step this one is blocked by, for `predecessor` and
    /// `background_conflict`. `None` for the others, which are not waiting on
    /// another step at all.
    pub blocked_by: Option<u32>,
    /// For `lag_deadline`, the absolute `game.tick` the wait runs until when a
    /// predecessor supplied a finish tick to anchor it to. A reader compares
    /// it to the event's own `tick` to see how much of the wait is left --
    /// which is the difference between "obediently smelting" and "stuck".
    /// `None` for every other state, and also for a lag nothing could anchor.
    pub deadline_tick: Option<u64>,
    /// How long this step has been in this state, in wall-clock milliseconds,
    /// measured by the executor from the moment it entered the state.
    ///
    /// **Not** the interval rounded, and not "since a heartbeat first noticed
    /// it": a wait that began between two beats reports its true age on the
    /// first beat that sees it. Wall clock rather than ticks for the same
    /// reason `elapsed_ms` is -- a stopped game is exactly the case the tick
    /// clock cannot answer.
    ///
    /// The clock restarts when the state changes, so a step that waited on a
    /// predecessor and then served that predecessor's lag reports the age of
    /// the lag wait, not of both together.
    pub waiting_ms: u64,
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
    ///
    /// **No live writer emits this any more**, as of `ee623717`. It was written
    /// when `goal.holds` answered `nil` — the planner declining to model a goal
    /// at all — and calling that *satisfied* meant a milestone could report
    /// success on no evidence whatever. `holds` returning `nil` is now a halt
    /// carrying a reason, not a satisfaction.
    ///
    /// The variant stays because archived runs contain it: several records on
    /// disk were written before that fix, and a reader that cannot parse
    /// `plan_empty` cannot read them at all.
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
    /// place.
    ///
    /// The reasoning here used to run "the planner exposes no way for a script
    /// to check whether a goal already holds independently of planning it, so
    /// an empty plan is always reported as [`SatisfiedReason::PlanEmpty`]".
    /// **`goal.holds` has existed since well before this comment was read
    /// again**, and it is what the supervisor asks; an empty plan is no longer
    /// reported as satisfied at all when `holds` cannot answer. The conclusion
    /// survives its premise — nothing here is guessed at as
    /// [`SatisfiedReason::AlreadySatisfied`] — but the premise was false.
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
    /// Never dispatched: a predecessor did not succeed, or the bot's own walk
    /// halted it. The executor's verdict, not the game's -- the game never saw
    /// the action. `ActionFailure.detail` names the cause, e.g. `predecessor
    /// 535 failed`. Before this kind existed such actions read as `pending`
    /// and wrote no settle at all, which is how `run-1788914717-24351` dropped
    /// 48 actions behind two refused belts with nothing in the record saying
    /// so.
    Abandoned,
    /// Some of what was asked for moved, and the rest did not.
    ///
    /// Split out of [`FailureKind::Rejected`] because the two leave the world
    /// in opposite states and a reader cannot tell them apart from the kind
    /// alone. A rejection is a command the game declined: nothing moved, and
    /// re-issuing it unchanged is a coherent thing to want. A partial transfer
    /// *already happened* -- BotBridge's `rcon_remove_from_inventory` removes
    /// whatever the inventory had and hands it to the player before it
    /// complains -- so the 18 plates are in the bot's hands and the only
    /// honest retry is for the 2 that are not.
    ///
    /// [`ActionFailure::detail`] carries both numbers and the item, e.g.
    /// `moved 18 of 20 iron-plate`, because "it failed" is not enough to
    /// replan against and the numbers were otherwise reachable only by
    /// re-parsing the human-readable `error` string.
    ///
    /// # What this no longer covers
    ///
    /// A partial *insert* has two causes and they are opposite outcomes. This
    /// kind is the one that is genuinely a failure: **the source came up
    /// short**, so the bot could not deliver what the plan believed it held.
    /// The other -- **the destination had no room** -- is a success, because
    /// nobody can make that inventory hold more of that item and the remainder
    /// stays with the bot. It arrives as a settle with `status: "success"`,
    /// `failure: null` and an `error` reading `destination full: moved 3 of 17
    /// coal, which now holds 50`, so the two are told apart by status and both
    /// keep their numbers.
    ///
    /// They used to collapse into this one kind, which is how a planner sizing
    /// boiler top-ups from demand alone stayed invisible until
    /// `run-1788432181-42528` -- the furthest a run had ever got -- died on one
    /// at tick 211399. `judge_transfer_reply`
    /// (`crates/core/src/factorio/rcon.rs`) makes the call, on the
    /// destination's own state, which only `mods/BotBridge/control.lua` can
    /// see and now reports.
    PartialTransfer,
    Rejected,
    Timeout,
    /// The bot had no character to act with: dead and waiting to respawn,
    /// in a cutscene, or under some other controller.
    ///
    /// The mod's wording is `player <n> has no character: <why>`, printed by
    /// `get_player` and the entry points that do not go through it
    /// (`mods/BotBridge/control.lua`), and by `on_player_died` for the walk,
    /// mine or craft that was in flight when the character vanished. Until
    /// it existed a dead bot read as `not connected` -- the wording for a
    /// client that never joined -- and every one of its failures classified
    /// as [`FailureKind::Rejected`] or [`FailureKind::Other`].
    ///
    /// [`ActionFailure::detail`] carries the `<why>` clause, e.g. `dead,
    /// respawns in 587 ticks`, so a reader can tell a transient death from a
    /// cutscene without re-parsing `error`.
    NoCharacter,
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

/// Why a walk did not arrive.
///
/// A separate enum from [`FailureKind`] rather than more variants on it,
/// because the two classify different things and share only the word
/// "failure": every distinction here is about the *pathfinder*, and none of
/// [`FailureKind`]'s five substantive variants has any meaning for a walk.
/// Merging them would produce one enum where two thirds of the variants are
/// inapplicable to whichever event you are holding.
///
/// The distinction the run record exists to preserve is
/// [`WalkFailureKind::NoPath`] against [`WalkFailureKind::PathfinderBusy`].
/// `FactorioRcon::player_path_attempt` (`crates/core/src/factorio/rcon.rs`)
/// branches on exactly that: `try again later` means the request queue was full
/// and **nothing was searched**, so asking again is the right thing to do and
/// it does; `failed to path find` means the pathfinder searched and found
/// nothing, which is a fact about the destination that no amount of repeating
/// will change. A record that collapsed them would say "the walk failed" for
/// both and leave the reader to guess which of "try again" and "this place is
/// unreachable" applies.
///
/// **Two of these variants are archive-only.** BotBridge used to re-path a
/// stalled walk for itself, and produced `PathfinderBusy` and `RepathLimit`
/// from wordings of its own; that machinery is gone -- retrying a stuck walk
/// lives in `FactorioRcon::move_player_timed`, where the goal, the radius and
/// the standability judgement are. The variants stay because
/// `workspace/runs` is full of runs that used them, and this enum is what
/// reads those runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalkFailureKind {
    /// The pathfinder searched and found no path. The destination is not
    /// reachable from where the bot stood, and re-issuing the same walk gets
    /// the same answer.
    ///
    /// Beware what it does *not* say. Run 30's three `NoPath` walks were all
    /// aimed at a point inside a stone furnace the same run had placed, so the
    /// terrain was fine and the ore field was reached minutes later; what was
    /// unreachable was the endpoint, not the region. `from` and `destination`
    /// are on [`WalkFailure`] precisely so that question can be asked of the
    /// record rather than of a log file that no longer exists.
    NoPath,
    /// The pathfinder never searched: the request queue would not take the
    /// re-path (`the game refused a re-path request`), or it took it and never
    /// answered within the mod's budget. **Nothing was learned about whether
    /// the destination is reachable**, which is exactly what separates this
    /// from [`WalkFailureKind::NoPath`], and it is worth trying again.
    ///
    /// **Archive only.** Both wordings came from the mod's own re-path, which
    /// no longer exists. A full queue is now retried inside
    /// `FactorioRcon::player_path_attempt` before any walk is dispatched, so it
    /// reaches a walk failure only as an ordinary pre-dispatch error.
    PathfinderBusy,
    /// Re-paths kept succeeding and the bot kept not arriving, until the mod's
    /// `WALK_REPATH_LIMIT` ran out. Distinct from both of the above: a path
    /// existed every time it was asked for, so this is a fact about the
    /// walking, not about the map.
    ///
    /// **Archive only**, for the same reason as
    /// [`WalkFailureKind::PathfinderBusy`]. Its successor is
    /// [`WalkFailureKind::Stalled`], which a walk now reaches only after
    /// `move_player_timed` has spent its whole retry budget on fresh paths.
    RepathLimit,
    /// A leg stopped progressing and the walk was abandoned -- the mod's
    /// `made no progress for <t> ticks`, or, from an older build,
    /// `aborted before reaching last waypoint`.
    ///
    /// Says the *walking* was stuck, not that the map is: every fresh path
    /// `FactorioRcon::move_player_timed` asked for was found and judged
    /// arrivable, and the character still did not get there.
    Stalled,
    /// No verdict ever arrived: the executor's deadline expired. Pairs with
    /// `status: "lost"`, and is the one kind here that says nothing at all
    /// about the walk itself.
    Timeout,
    /// The bot had no character to walk with -- dead and waiting to respawn,
    /// or otherwise character-less. The mod's `has no character` wording,
    /// which reaches a walk either from the path request `get_player`
    /// refused before dispatch, or from `on_player_died` failing the leg in
    /// flight. Says nothing about the map: no path was searched.
    NoCharacter,
    /// The pathfinder found a route, and the route ends inside a collision
    /// box the entity graph has seen -- so the walk was refused *before*
    /// dispatch, by `judge_path` (crates/core/src/factorio/rcon.rs), as
    /// `RconWalkEndsWhereNobodyCanStand`. Nothing was walked.
    ///
    /// Says the *aim* was wrong, not the map: there is a way there, and the
    /// bot was sent to a point on it that a character cannot occupy -- a
    /// neighbouring rock (`run-1788608011-14361`, bot 4, step 16, archived as
    /// `other` until this variant existed), or a building the plan itself
    /// put there. Since `approach_standing` the aim is chosen off every box
    /// the graph knows, so a walk that still lands here is one the graph
    /// learned about too late.
    DestinationBlocked,
    /// The pathfinder refused the walk **and** refused every short hop from
    /// where the character stands: the bot cannot leave its own tile, so the
    /// destination is not what was unreachable. The executor's mobility probe
    /// (`FactorioRcon::probe_player_hops`, judged by
    /// `crates/executor::walk_memory::judge_mobility`) established it and
    /// benched the bot; a `bot_benched` event sits beside this one.
    ///
    /// The opposite of what [`WalkFailureKind::NoPath`] is careful not to
    /// claim. `run-1788614781-38058` produced four `no_path` rows for bot 6,
    /// all to a destination other bots reached, before this kind existed to
    /// say the bot was the problem.
    BoxedIn,
    /// A kind this build does not know, or one not worth a variant yet.
    #[serde(other)]
    Other,
}

/// A structured walk failure, carried *beside* [`EventKind::WalkSettled`]'s
/// `error` string rather than instead of it, for the same reason
/// [`ActionFailure`] is: the string is what a person reads, the kind is what a
/// query groups by, and replacing one with the other loses an audience.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WalkFailure {
    pub kind: WalkFailureKind,
    /// **Observed.** Where the character actually stood when the mod gave up,
    /// as the mod's own message named it. `None` for every failure whose
    /// wording carries no position.
    ///
    /// This is the one place the walk record states a real position of a bot,
    /// and it is trustworthy in a way `on_player_changed_position` is not: the
    /// mod read `player.character.position` at that instant rather than
    /// inferring it from the last tile boundary crossed.
    pub from: Option<Position>,
    /// **Observed.** The destination the mod was actually steering to: the
    /// last waypoint of the path the *game* returned.
    ///
    /// Not the same thing as [`EventKind::WalkSettled`]'s `to`, and the
    /// difference is the finding. `to` is what the schedule asked for; this is
    /// what the walk was really trying to reach after Factorio answered the
    /// path request, and in run 30 all three failures had this land strictly
    /// inside the collision box of a furnace the run had placed, up to 1.2
    /// tiles from the `to` that was requested. Recording only `to` would have
    /// hidden exactly the fact that had to be reconstructed by hand from
    /// `workspace/server-log.txt`.
    pub destination: Option<Position>,
}

/// One stall a walk was retried through, as `events.jsonl` carries it.
///
/// The archive's shape of [`crate::factorio::rcon::WalkStall`]: the same facts,
/// with the blocker reduced to what a query groups by. The full clause is still
/// in `error`, and [`crate::factorio::rcon::walk_blocker`] re-reads it, so
/// nothing is lost -- the reduction only spares the record a nested object
/// whose fields no reader has asked for yet.
///
/// **This exists because a recovered stall used to leave no trace at all.**
/// `FactorioRcon::move_player_timed` answers a stalled leg with a fresh path
/// and the walk then settles `success`; the only evidence was one `warn!` on
/// stdout, which the next run overwrites. Counted 2026-09-08 over
/// `workspace/session-logs`: **17 recovered stalls, none of them in any
/// `events.jsonl`**, against exactly one stall the whole 80-file archive holds
/// -- the `tree-01` of `run-1788833726-34821`, which is the one the retry could
/// not fix. A record that shows 1 of 18 cannot tell a healthy run from a run
/// recovery is carrying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WalkStallRecord {
    /// `game.tick` the stalled attempt was dispatched at, when the game
    /// stamped one. `None` is a tick nobody observed, never tick zero.
    pub tick: Option<u64>,
    /// Which attempt stalled, counting from 1.
    pub attempt: u32,
    /// What the mod's probe found in the way, as
    /// [`crate::factorio::rcon::WalkBlockerKind::name`] spells it
    /// (`character`, `tree`, `entity`, `nothing`, `probe_failed`, ...).
    ///
    /// **`None` means the message carried no clause this build could read** --
    /// an older mod, or a reworded one. It does not mean the tile was clear:
    /// that is `"nothing"`, which is an answer. And a wording this build's
    /// grammar does not cover is `"unknown"`, a third thing again; the census
    /// that read 19 of 20 walk failures as `other` is what those three
    /// separate answers exist to prevent.
    ///
    /// A string rather than the enum, for the reason
    /// [`EventKind::Teleport`]'s `reason` gives: the record does not need to
    /// publish a closed list, and a name that survives a re-read verbatim
    /// cannot be silently remapped onto the wrong variant on the way in.
    pub blocker: Option<String>,
    /// The prototype name of whatever was in the way (`tree-01`,
    /// `stone-furnace`), when the blocker had one.
    pub blocker_name: Option<String>,
    /// The verdict as the game and the mod worded it. Kept because `blocker`
    /// is derived from it and a reader must be able to check the derivation --
    /// the `kind: "other"` census that cost this project a day was exactly a
    /// classifier that had fallen behind the wording.
    pub error: String,
}

/// One line of `events.jsonl`.
///
/// `tick` is *when this happened*; a duration is always `elapsed_ticks`. The
/// two were briefly both called `ticks`, which reads fine until someone plots
/// it.
///
/// There used to be a `wall_ms` here too, and it is gone on purpose --
/// see [`RunRecorder::record`] for the full account. In short: it was
/// stamped from `self.started.elapsed()` at the moment `record()` was
/// *called*, but `record.actions()`/`record.teleports()`/`record.refusals()`
/// (`crates/scripting_lua/src/globals/record.rs`) are each called once per
/// supervisor "ran" transition, i.e. once an entire multi-bot plan has
/// finished executing -- which can be minutes of real time and thousands of
/// ticks after the earliest event in that same call flushed. Every event in
/// such a batch therefore got the wall clock reading from the moment the
/// *batch* was written, not the moment each event actually happened, which
/// is exactly the `docs/superpowers/notes/2026-09-02-inventory-shortfall.md`
/// finding of `wall_ms` jumping `33780 -> 738866` while `tick` moved only
/// `10`. Making it mean "real time this event happened" would require the
/// executor to capture `SystemTime::now()` at the point it actually
/// dispatches/observes each action (`ExecutionLog::start`/`observe`/
/// `start_walk`/`observe_walk` in `crates/executor/src/log.rs`, called from
/// `crates/executor/src/run.rs`) and carry that across the mlua boundary
/// (`build_observation` in `crates/scripting_lua/src/globals/goal/run.rs`)
/// into `record.actions()` -- a cross-crate change to the executor's public
/// attempt/walk types and every test that asserts their shape, not a fix to
/// a wrong timestamp. Nothing reads the field today (`app/src/lib/
/// runTimeline.ts`, `runDiff.ts` and the analysis page never touch it; the
/// only Rust readers were test-fixture helpers), so there is no consumer to
/// preserve, and a batch-stamped column that looks like a per-event
/// timestamp is worse than no column at all. `#[serde(default)]` is not
/// needed for removal -- an old `events.jsonl` line's leftover `"wall_ms"`
/// key deserializes as an ordinary ignored extra field (serde's default for
/// a struct with no `deny_unknown_fields`), so every run already on disk
/// stays readable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Event {
    pub tick: u64,
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
    /// events and its samples; what it lacks is a verdict, and inventing one
    /// would make it look complete.
    pub outcome: Option<String>,
    pub elapsed_ticks: Option<u64>,
    pub events: usize,
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
    /// Ticks of the run's own span that the archived sample stream does *not*
    /// cover: the closing tick minus the last sample archived, floored at
    /// zero. `Some(0)` is a run whose samples reach its end; a large value is
    /// a run that stopped being sampled -- or stopped being *ingested* --
    /// while it went on running.
    ///
    /// Recorded because that failure is otherwise invisible. Run
    /// `run-1788459085-32452` sampled cleanly for its whole 281,000 ticks and
    /// archived only the first 78,840 of them, and nothing said so: the loss
    /// was found by comparing the last tick of `samples.jsonl` against the
    /// last tick of `events.jsonl` by hand. This is that comparison, made once
    /// by the only party that knows both numbers for certain.
    ///
    /// `None` for a run with no samples at all, which is a different fact from
    /// "the samples fell short" -- a planning-only run captures none by design
    /// and a lag of zero would claim full coverage of nothing.
    #[serde(default)]
    pub samples_lag_ticks: Option<u64>,
}

/// How far behind the mod's live sample stream the archive is allowed to fall
/// while a run is in progress, in game ticks (30 s at 60 UPS).
///
/// This exists because "at a milestone boundary" turned out not to be a
/// cadence. Ingestion used to run only inside `finish()`, and run
/// `run-1788315106-86443` -- killed by a wall-clock timeout -- left no
/// `samples.jsonl` at all; the answer was to ingest at every milestone
/// boundary too. That bounded the loss by the length of a milestone, which
/// was fine until a milestone got long: `run-1788459085-32452` spent 199,449
/// ticks (55 minutes) inside milestone 2, was killed before it closed, and
/// lost every one of those ticks' samples -- including the entire window in
/// which the thing the run was built to observe was built and fed. The mod
/// had written all of them; nothing had copied them.
///
/// A tick interval is the fix because it does not depend on the run reaching
/// anything. Ingestion is cheap by construction (it seeks to a remembered
/// offset and reads only what is new), so the only cost of a short interval
/// is more of those seeks.
pub const SAMPLE_INGEST_INTERVAL_TICKS: u64 = 1800;

/// How often [`EventKind::VisionMeasured`] is written while a run is going, in
/// game ticks (5 minutes at 60 UPS).
///
/// Longer than [`SAMPLE_INGEST_INTERVAL_TICKS`] on purpose: what this measures
/// moves slowly. The model's extent only changes when the engine generates a
/// chunk nobody has been to, and a bot's furthest point only changes when a
/// bot walks further than it ever has. A 5-minute beat puts a dozen or so
/// lines in a long run's log -- enough to see the two numbers diverge, few
/// enough that nobody scrolls past them.
///
/// The interval decides the *resolution* of the answer, never its
/// correctness, exactly as [`EventKind::BatchProgress`]'s does: the closing
/// measurement at `finish` is unconditional, so a run shorter than one
/// interval still states its figure.
pub const VISION_MEASURE_INTERVAL_TICKS: u64 = 18_000;

/// Writes a run's event log.
pub struct RunRecorder {
    dir: PathBuf,
    /// Every hold this run has performed, and the file they are written to.
    ///
    /// Held in memory as well as on disk because a hold *appends*: the run can
    /// fault, be held, be released with `continue`, replan and fault again,
    /// and each of those is its own exposure window. Rereading the file to
    /// append would make this recorder read back its own writes, and it is
    /// the only writer. See [`exposure`] for why this is not a provenance
    /// field.
    exposure: exposure::Exposure,
    run_id: String,
    events: File,
    /// `map.jsonl`: what got built, and whether the game agreed. Written
    /// straight into the run directory as it happens, exactly like `events`
    /// -- there is no ingestion step, because nothing produces this file but
    /// this recorder.
    map: File,
    flow: File,
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
    /// Byte offset already consumed from the mod's
    /// `script-output/botbridge/samples.jsonl`. [`RunRecorder::ingest_samples`]
    /// is called from more than one place -- a milestone boundary and
    /// `finish` -- and this is what makes each call pick up only what the mod
    /// wrote since the last one, rather than re-reading and re-archiving the
    /// whole source every time.
    samples_offset: u64,
    /// How many sample lines have been archived into this run's
    /// `samples.jsonl` so far. Kept incrementally, like `map_count`, rather
    /// than reread from disk at `finish`: this recorder is the only writer of
    /// that file, so it cannot disagree with what it just wrote.
    samples_count: usize,
    /// The workspace holding the mod's live `samples.jsonl`, once a caller has
    /// named it via [`RunRecorder::watch_samples`].
    ///
    /// Held rather than passed, unlike [`RunRecorder::ingest_samples`]'s
    /// argument, because the periodic ingest is driven from
    /// [`RunRecorder::record`], which is called from everywhere and must not
    /// grow a parameter that every caller would have to know the answer to.
    /// `None` keeps the pre-existing behaviour exactly: ingestion then happens
    /// only where a caller passes the workspace in.
    samples_workspace: Option<PathBuf>,
    /// The tick at which the periodic ingest last ran. See
    /// [`SAMPLE_INGEST_INTERVAL_TICKS`].
    samples_ingested_at: u64,
    /// The highest tick of any sample archived into this run, or `None` when
    /// none has been. Tracked here rather than by rereading the archive at
    /// `finish`, for the same reason `samples_count` is, and used for exactly
    /// one thing: [`Manifest::samples_lag_ticks`].
    samples_high_tick: Option<u64>,
    /// The furthest any bot has been observed from the map origin this run,
    /// taken from the sample lines as they are archived (see
    /// [`samples::TravelObservation`]).
    ///
    /// `None` means no bot position has ever been archived -- not that no bot
    /// moved. A recorder that has never seen a sample must be able to say so;
    /// see [`EventKind::VisionMeasured`].
    travel: Option<TravelObservation>,
    /// How many bot positions have been archived, the denominator behind
    /// `travel`.
    bot_samples: usize,
    /// The world model this run's vision is measured against, once a caller
    /// has named it via [`RunRecorder::watch_model`].
    ///
    /// Held rather than passed for the same reason `samples_workspace` is: the
    /// measurement is driven from [`RunRecorder::record`], which is called
    /// from everywhere. `None` writes no [`EventKind::VisionMeasured`] at all,
    /// which is the behaviour of every caller that has not opted in -- and is
    /// visible as the *absence* of the event, never as a measurement of zero.
    model: Option<Arc<EntityGraph>>,
    /// The tick the vision measurement last ran at, or `None` when it never
    /// has. `None` rather than `0` so the first recorded event of a run always
    /// writes a baseline, whatever tick the run opens at -- a run opening at
    /// tick 3,658 would otherwise write nothing until tick 18,000.
    vision_measured_at: Option<u64>,
    /// The run's video recorder, when one was asked for.
    ///
    /// Held here rather than beside the run in the caller so that the two
    /// artefacts cannot disagree about which run they belong to -- the same
    /// reason `record.start()` mints one id and hands it to the mod's
    /// sampling session.
    /// `None` for every run that did not ask for video, which is the default.
    video: Option<video::VideoRecorder>,
}

impl RunRecorder {
    /// Creates `<runs_root>/<run_id>/` and opens its event log.
    ///
    /// The caller supplies `run_id` because the same id must reach
    /// `sampling_start`; minting it in two places is how the log and the
    /// samples come to disagree about which run they belong to.
    pub fn start(runs_root: &Path, run_id: impl Into<String>) -> io::Result<Self> {
        let run_id = run_id.into();
        let dir = runs_root.join(&run_id);
        fs::create_dir_all(&dir)?;
        let events = File::create(dir.join("events.jsonl"))?;
        let map = File::create(dir.join("map.jsonl"))?;
        let flow = File::create(dir.join("flow.jsonl"))?;
        let recorder = Self {
            dir,
            run_id,
            exposure: exposure::Exposure::none_yet(),
            events,
            map,
            flow,
            start_tick: None,
            high_tick: 0,
            map_count: 0,
            placed_bounds: None,
            samples_offset: 0,
            samples_count: 0,
            samples_workspace: None,
            samples_ingested_at: 0,
            samples_high_tick: None,
            travel: None,
            bot_samples: 0,
            model: None,
            vision_measured_at: None,
            video: None,
            started_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
        };
        // Written immediately, empty, and never gated on a hold happening.
        // That is the whole point: an absent `exposure.json` must go on
        // meaning "this build never looked", so "this run was not held" has
        // to be a file on disk rather than the absence of one.
        //
        // Not fatal. A run that cannot write it leaves behind exactly the
        // honest answer -- unknown -- which is what an absent file already
        // means, so there is nothing to fail for.
        if let Err(error) = recorder.write_exposure() {
            crate::tracing::warn!(
                %error,
                "could not write exposure.json; whether this run was held will read as \
                 unknown rather than as not-held"
            );
        }
        Ok(recorder)
    }

    /// Serialises [`RunRecorder::exposure`] over `exposure.json`.
    ///
    /// Rewritten in full on every hold rather than appended to, because it is
    /// a handful of records and a truncated JSON document is unreadable in a
    /// way a truncated JSONL line is not. Unlike `provenance.json`, losing
    /// this to a mid-write kill costs a *weaker* claim, never a wrong one: the
    /// reader falls back to "not captured".
    fn write_exposure(&self) -> io::Result<()> {
        fs::write(
            self.dir.join(exposure::EXPOSURE_FILE),
            serde_json::to_vec_pretty(&self.exposure).map_err(io::Error::other)?,
        )
    }

    /// Records that this run was **held**: paused mid-run with an invitation
    /// for a human to attach, and therefore exposed to whatever they did.
    ///
    /// Called once per hold, after it ends, because what it records -- how
    /// long, how it was released, what the game saw -- is only known then.
    ///
    /// See [`exposure`] for why being held is not the same as having been
    /// mutated, and why the two must never be reported as one fact.
    pub fn record_hold(&mut self, hold: exposure::HoldExposure) -> io::Result<()> {
        self.exposure.holds.push(hold);
        self.write_exposure()
    }

    /// What this run's holds add up to, for a caller that wants to say so in
    /// its own output without rereading the file.
    pub fn exposure(&self) -> &exposure::Exposure {
        &self.exposure
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Names the workspace whose `server/script-output/botbridge/samples.jsonl`
    /// this run is being sampled into, so that [`RunRecorder::record`] can keep
    /// the archive within [`SAMPLE_INGEST_INTERVAL_TICKS`] of it on its own.
    ///
    /// Call this immediately after [`RunRecorder::start`] on any run that
    /// started a sampling session. Without it the recorder still archives
    /// everything at the moments a caller passes the workspace to
    /// [`RunRecorder::ingest_samples`] -- but only at those moments, which is
    /// how a run that never reached one of them came to lose 55 minutes of
    /// samples the mod had already written.
    pub fn watch_samples(&mut self, workspace: impl Into<PathBuf>) {
        self.samples_workspace = Some(workspace.into());
    }

    /// Names the world model whose extent this run's free vision is measured
    /// against, turning [`EventKind::VisionMeasured`] on for this run.
    ///
    /// Call this immediately after [`RunRecorder::start`], beside
    /// [`RunRecorder::watch_samples`] -- the two halves of the measurement come
    /// from the two of them, and a run with only one gets an event with the
    /// other half null rather than no event at all.
    ///
    /// Opt-in rather than required, because a recorder is also driven by tests
    /// and by tools that have no world. What it must never do is report a
    /// model it was not given as an empty one, so a recorder with no model
    /// writes no measurement; see the field.
    pub fn watch_model(&mut self, model: Arc<EntityGraph>) {
        self.model = Some(model);
    }

    /// Writes `provenance.json` -- what this run was launched with.
    ///
    /// Call this once, immediately after [`RunRecorder::start`], before
    /// anything that can fail. That timing is the whole design: `manifest.json`
    /// is written in [`RunRecorder::finish`] and so exists only for runs that
    /// finished, and of the 24 runs archived when this was added, nine had no
    /// manifest at all -- the killed and crashed ones, which are exactly the
    /// runs whose identity someone later needs. See [`provenance`].
    ///
    /// Written once and never rewritten. Everything in it describes what was
    /// launched, so no later moment knows any of it better, and a file updated
    /// at finish would lose both halves to a process killed mid-rewrite.
    pub fn record_provenance(&self, provenance: &Provenance) -> io::Result<()> {
        fs::write(
            self.dir.join(provenance::PROVENANCE_FILE),
            serde_json::to_vec_pretty(provenance).map_err(io::Error::other)?,
        )
    }

    /// The `started_unix` this recorder stamped, so a caller building a
    /// [`Provenance`] states the same instant the manifest will.
    pub fn started_unix(&self) -> u64 {
        self.started_unix
    }

    /// Ingests samples if the run has advanced far enough since the last time,
    /// and swallows -- after logging -- anything that goes wrong.
    ///
    /// Deliberately not fallible to its caller. This runs inside
    /// [`RunRecorder::record`], whose job is to get the event on disk; a
    /// problem reading the *mod's* sample file must not be able to stop the
    /// event log, which is the artefact a diagnosis needs most and the one
    /// that would still be readable if everything else failed.
    ///
    /// Nothing is lost by not raising here. A failed call leaves
    /// `samples_offset` untouched (see [`samples::ingest_samples_incremental`]),
    /// so the next attempt -- at the next interval, at a milestone boundary,
    /// or at `finish`, where it *does* propagate -- reads the same bytes and
    /// reports the same error.
    fn ingest_samples_if_due(&mut self, tick: u64) {
        if self.samples_workspace.is_none()
            || tick.saturating_sub(self.samples_ingested_at) < SAMPLE_INGEST_INTERVAL_TICKS
        {
            return;
        }
        self.samples_ingested_at = tick;
        let workspace = self.samples_workspace.clone();
        if let Err(error) = self.ingest_samples(workspace.as_deref()) {
            tracing::warn!(
                %error,
                "could not ingest the mod's samples mid-run; \
                 the next attempt will re-read the same bytes"
            );
        }
    }

    /// Hands this run its video recorder.
    ///
    /// Opt-in by construction: nothing here starts one, and a run that never
    /// calls this behaves exactly as it did before video existed. Video is the
    /// visual record now that the per-camera screenshots are gone; every other
    /// artefact of a run is text.
    pub fn attach_video(&mut self, recorder: video::VideoRecorder) {
        self.video = Some(recorder);
    }

    /// What the video recorder has to say, if there is one.
    pub fn video(&self) -> Option<&video::VideoRecorder> {
        self.video.as_ref()
    }

    /// Stops the video recorder, if there is one, and returns its final record.
    ///
    /// **Call this before [`RunRecorder::finish`].** `finish` archives the
    /// recording but cannot stop it (it is not async), and a recording that was
    /// never stopped is archived with `status: "recording"` -- which the viewer
    /// reports as a defect rather than showing the video as if it were
    /// complete.
    ///
    /// `tick` is the run's closing tick, written as the clock's `stop` line so
    /// the sidecar's span and the run's span agree.
    pub async fn stop_video(
        &mut self,
        tick: Option<u64>,
    ) -> io::Result<Option<video::VideoRecord>> {
        match self.video.as_mut() {
            Some(recorder) => recorder.stop(tick).await.map(Some),
            None => Ok(None),
        }
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
        self.write_event(tick, kind)?;
        // After the event is on disk, never before: sample ingestion is a
        // convenience this call performs on the way past, and the event is the
        // thing it was asked to do.
        self.ingest_samples_if_due(tick);
        // After ingestion, so a measurement written on this beat counts the
        // samples this beat just archived rather than last beat's.
        self.measure_vision_if_due(tick);
        Ok(())
    }

    /// Writes one event and nothing else.
    ///
    /// The half of [`RunRecorder::record`] that only appends. Split out so
    /// that the periodic ingest and measurement `record` performs on the way
    /// past can write their own events without re-entering the call that
    /// triggered them -- an event written from inside `record` must not start
    /// another round of ingest-and-measure.
    fn write_event(&mut self, tick: u64, kind: EventKind) -> io::Result<()> {
        let event = Event { tick, kind };
        if self.start_tick.is_none() {
            self.start_tick = Some(tick);
        }
        self.high_tick = self.high_tick.max(tick);
        let mut line = serde_json::to_string(&event).map_err(io::Error::other)?;
        line.push('\n');
        self.events.write_all(line.as_bytes())?;
        self.events.flush()?;
        Ok(())
    }

    /// Writes [`EventKind::VisionMeasured`] if the run has advanced far enough
    /// since the last one, and swallows -- after logging -- a failed write.
    ///
    /// Not fallible to its caller, for the same reason
    /// [`RunRecorder::ingest_samples_if_due`] is not: this runs inside
    /// `record`, whose job is to get *that* event on disk, and a disclosure
    /// that could abort the event log would be worse than the thing it
    /// discloses.
    fn measure_vision_if_due(&mut self, tick: u64) {
        if self.model.is_none() {
            return;
        }
        if let Some(last) = self.vision_measured_at
            && tick.saturating_sub(last) < VISION_MEASURE_INTERVAL_TICKS
        {
            return;
        }
        if let Err(error) = self.measure_vision(tick) {
            tracing::warn!(
                %error,
                "could not record this run's vision measurement; how much ground \
                 the model was given without visiting it will be missing from \
                 this beat"
            );
        }
    }

    /// Writes one [`EventKind::VisionMeasured`] now, whatever the interval
    /// says. A no-op for a recorder with no model (see
    /// [`RunRecorder::watch_model`]).
    ///
    /// Public so `finish` -- and a caller that wants a measurement at a
    /// milestone boundary -- can take one at a moment that matters rather than
    /// waiting for the next beat.
    ///
    /// Answers whether an event was actually written, so that a caller
    /// counting the log (which is what [`Manifest::events`] is) cannot
    /// disagree with the file. A manifest that undercounts its own log by one
    /// is the sort of quiet inconsistency that makes every other number in it
    /// suspect.
    pub fn measure_vision(&mut self, tick: u64) -> io::Result<bool> {
        let Some(model) = self.model.clone() else {
            return Ok(false);
        };
        self.vision_measured_at = Some(tick);
        let extent = model.vision_extent();
        let travelled_tiles = self.travel.as_ref().map(|t| t.tiles);
        let model_tiles = extent.as_ref().map(|e| e.tiles);
        // Both halves known, and a bot that actually left the origin. A ratio
        // against zero travel is not "infinitely unearned", it is undefined,
        // and a reader must be handed the null rather than a large number.
        let unearned_ratio = match (model_tiles, travelled_tiles) {
            (Some(model), Some(travelled)) if travelled > 0.0 => Some(model / travelled),
            _ => None,
        };
        self.write_event(
            tick,
            EventKind::VisionMeasured {
                travelled_tiles,
                travelled_bot: self.travel.as_ref().map(|t| t.bot),
                travelled_at_tick: self.travel.as_ref().map(|t| t.tick),
                bot_samples: self.bot_samples as u32,
                model_tiles,
                model_furthest: extent.as_ref().map(|e| e.name.clone()),
                model_furthest_position: extent.as_ref().map(|e| e.position.clone()),
                model_resource_tiles: extent.as_ref().map_or(0, |e| e.resource_tiles as u32),
                model_enemy_structures: extent.as_ref().map_or(0, |e| e.enemy_structures as u32),
                unearned_ratio,
            },
        )?;
        Ok(true)
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

    /// Appends one line to `flow.jsonl` and flushes it, for the same
    /// reason [`Self::record_map`] does: a crashed run must leave a
    /// readable flow history, not just a readable event log.
    ///
    /// Unlike `record_map`, this does no bookkeeping beyond the write --
    /// there is no equivalent of `placed_bounds` to maintain, because a
    /// flow keyframe is a full snapshot rather than a delta a later
    /// reconstruction depends on.
    pub fn record_flow(
        &mut self,
        export: &crate::graph::flow_export::FlowExport,
    ) -> io::Result<()> {
        let mut line = serde_json::to_string(export).map_err(io::Error::other)?;
        line.push('\n');
        self.flow.write_all(line.as_bytes())?;
        self.flow.flush()
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

    /// Copies whatever the mod has written to
    /// `script-output/botbridge/samples.jsonl` since the last call into this
    /// run's `samples.jsonl`.
    ///
    /// Meant to be called at every milestone boundary *and* at `finish` --
    /// the two moments `archive_frames`/keyframes already run at -- so that a
    /// run killed mid-milestone (no `finish()` ever reached) still has
    /// everything up through its last closed milestone on disk, instead of
    /// losing the entire stream. Cheap to call often: it resumes from
    /// [`RunRecorder::samples_offset`] rather than re-reading the source from
    /// the top, so a long run does not pay to re-ingest what an earlier call
    /// already archived, and nothing gets archived twice.
    ///
    /// `workspace` is where `server/script-output/botbridge/samples.jsonl`
    /// lives; `None` for a planning-only run that captured no samples, in
    /// which case this is a no-op returning `0`.
    pub fn ingest_samples(&mut self, workspace: Option<&Path>) -> io::Result<usize> {
        let Some(workspace) = workspace else {
            return Ok(0);
        };
        // The lower bound for an un-attributed sample (one written before the
        // `run` field existed) must be the run's *start*, not wherever
        // `high_tick` has climbed to by the time this is called: such a
        // sample has no other way to prove it belongs to this run, and a
        // cutoff drawn from a later moment would discard samples recorded
        // earlier in a run that is still legitimately in progress.
        let cutoff = self.start_tick.unwrap_or(0);
        let progress = samples::ingest_samples_incremental(
            workspace,
            &self.dir,
            &self.run_id,
            cutoff,
            self.samples_offset,
        )?;
        if progress.skipped > 0 {
            tracing::warn!(
                skipped = progress.skipped,
                "some sample lines did not parse and were skipped"
            );
        }
        self.samples_offset = progress.offset;
        self.samples_count += progress.appended;
        if let Some(high) = progress.high_tick {
            self.samples_high_tick = Some(self.samples_high_tick.map_or(high, |t| t.max(high)));
        }
        self.bot_samples += progress.bot_positions;
        // A high-water mark, so a bot walking back does not shrink it: the
        // question is how far a bot ever got, not where it is now.
        if let Some(seen) = progress.travel
            && self
                .travel
                .as_ref()
                .is_none_or(|best| seen.tiles > best.tiles)
        {
            self.travel = Some(seen);
        }
        Ok(progress.appended)
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
    /// planning-only run with nothing on disk to collect. That is a valid run,
    /// not a degenerate one.
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

        // Copy nothing unless `video/run.json` names this run. An orphaned
        // recording left by an earlier run therefore cannot be archived into
        // this one.
        //
        // A still-attached recorder is *not* stopped here -- `stop` is async
        // and this is not. That is not a hole: the recording's own
        // `video.json` still says `recording`, which the design makes proof
        // that the run finished and nobody told the encoder. The warning below
        // says so at the moment it happens rather than leaving it to be found
        // in the archive.
        if let Some(workspace) = workspace {
            if self
                .video
                .as_ref()
                .is_some_and(video::VideoRecorder::is_recording)
            {
                tracing::warn!(
                    "the run is finishing with its video recorder still running; \
                     call stop_video() first"
                );
            }
            video::archive_video(workspace, &self.dir, &self.run_id)?;
        }

        // Catches up on anything the mod wrote since the last milestone
        // boundary (or everything, if this run never reached one). Cheap
        // even on a long run: it resumes from `samples_offset` rather than
        // re-reading the source from the top.
        self.ingest_samples(workspace)?;
        let samples = self.samples_count;

        // The closing disclosure, unconditional and after the last ingest, so
        // it counts every bot position this run ever archived. It lands *after*
        // `run_finished` in the log on purpose: the finishing event is written
        // first so that a crash during archiving still leaves a log that says
        // how the run ended, and nothing here is allowed to come between that
        // event and the crash. A reader takes the LAST `vision_measured` in the
        // file, not the last line.
        //
        // Never fatal: a run that could not state its free vision is still a
        // run worth finishing, and the missing event is itself the honest
        // report -- an absent measurement is unknown, and no reader may take it
        // for zero.
        //
        // Counted into the manifest below, because it is written *after* the
        // log was read for that count: a manifest saying 240 events over a
        // file holding 241 would make a reader doubt every other number in it.
        let mut events_written = read.events.len();
        match self.measure_vision(tick) {
            Ok(true) => events_written += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                %error,
                "could not record this run's closing vision measurement"
            ),
        }

        // Does the sample stream actually cover the run it belongs to?
        //
        // Asked here because this is the only place both numbers are known for
        // certain, and because nobody was asking it. Sampling is the
        // instrument every other diagnosis reads through -- power draw,
        // production totals, whether a bot ever moved -- and it can stop
        // without stopping the run: the mod goes quiet, or, as in
        // `run-1788459085-32452`, keeps writing while nothing copies what it
        // writes. Either way `samples.jsonl` simply ends early, which looks
        // exactly like a short run until you compare it against something.
        let samples_lag_ticks = self.samples_high_tick.map(|high| tick.saturating_sub(high));
        if let Some(lag) = samples_lag_ticks
            && lag > SAMPLE_INGEST_INTERVAL_TICKS
        {
            tracing::warn!(
                lag_ticks = lag,
                last_sample_tick = self.samples_high_tick,
                run_tick = tick,
                samples,
                "this run's samples stop well before the run does -- \
                 the last {} ticks of it were never sampled or never archived, \
                 so nothing observed in that window can be read back",
                lag
            );
        }
        if samples_lag_ticks.is_none() && workspace.is_some() {
            // A run with a workspace to sample from and not one sample to show
            // for it. Distinct from the lag warning above, which needs at least
            // one sample to measure from, and not derivable from it.
            tracing::warn!(
                "this run archived no samples at all, though it had a workspace \
                 to read them from"
            );
        }

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
            events: events_written,
            splits: splits.len(),
            samples,
            map: self.map_count,
            samples_lag_ticks,
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
    fn provenance_lands_in_the_run_directory_and_survives_a_run_that_never_finishes() {
        // The seam the unit tests in `provenance` cannot reach: that the file
        // goes *into this run's directory* and is readable by the run id alone.
        //
        // The second half of the name is the whole design. `finish` is never
        // called here, exactly as it was never called for the nine archived
        // runs that were killed -- and provenance still has to be there
        // afterwards, because those are the runs whose identity somebody needs.
        let root = tmpdir("provenance");
        let recorder = RunRecorder::start(&root, "run-prov-1").unwrap();
        let provenance = Provenance {
            schema: Provenance::SCHEMA,
            run_id: "run-prov-1".into(),
            started_unix: recorder.started_unix(),
            started_tick: 4330,
            seed: Some("20260903".into()),
            map_exchange_string: None,
            map: None,
            factorio: Some("2.1.17".into()),
            mods: None,
            git: None,
            profile: "debug".into(),
            roster_requested: vec![1, 2, 3, 4],
            workspace: None,
            resumed_from: None,
            bot_mode: None,
            game_speed: None,
            peaceful: None,
        };
        recorder.record_provenance(&provenance).unwrap();
        drop(recorder);

        let read = read_provenance(&root.join("run-prov-1")).expect("provenance is on disk");
        assert_eq!(read, provenance);
        assert!(!root.join("run-prov-1").join("manifest.json").exists());
    }

    #[test]
    fn every_event_kind_round_trips() {
        let kinds = [
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
                bots: Some(vec![1, 2, 3, 4]),
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
                delivery: None,
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
            bots: Some(vec![1, 2]),
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
                EventKind::Teleport {
                    bot: 1,
                    reason: format!("walk_stuck_{i}"),
                    from: Position::new(0.0, 0.0),
                    to: Position::new(1.0, 1.0),
                    distance: 1.0,
                    action_id: None,
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

    /// Pins the removal of `wall_ms` (see [`Event`]'s doc comment for why):
    /// a run already on disk with the old field must keep opening, and a
    /// freshly written run must not resurrect it.
    ///
    /// This is also the test that would have failed under the batch-stamping
    /// behaviour, had `wall_ms` been kept instead of removed: that behaviour
    /// stamped every event `record()` wrote in one call with the *same*
    /// `self.started.elapsed()` reading, so two events from the same
    /// `record.actions()` batch could carry identical `wall_ms` despite
    /// covering very different ticks -- the exact shape of the
    /// `33780 -> 738866` / `10-tick` finding this fix responds to.
    #[test]
    fn wall_ms_is_gone_but_an_old_line_carrying_it_still_reads() {
        let dir = tmpdir("wall_ms_removed");
        let path = dir.join("events.jsonl");
        fs::write(
            &path,
            r#"{"tick":1,"wall_ms":33780,"kind":"run_started","run_id":"r","bots":[1],"seed":null,"factorio":null,"git":null}
{"tick":11,"wall_ms":738866,"kind":"run_finished","outcome":"done","elapsed_ticks":10}
"#,
        )
        .unwrap();
        let read = read_events(&path).unwrap();
        assert_eq!(
            read.skipped, 0,
            "a leftover wall_ms key is not a parse failure"
        );
        assert_eq!(read.events.len(), 2);
        assert_eq!(read.events[0].tick, 1);
        assert_eq!(read.events[1].tick, 11);

        let mut rec = RunRecorder::start(&dir.join("fresh"), "r-fresh").unwrap();
        rec.record(
            0,
            EventKind::RunFinished {
                outcome: "done".into(),
                elapsed_ticks: 0,
            },
        )
        .unwrap();
        let text = fs::read_to_string(rec.dir().join("events.jsonl")).unwrap();
        assert!(
            !text.contains("wall_ms"),
            "a freshly written event must not carry wall_ms: {text}"
        );
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
                delivery: None,
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
            "{\"kind\":\"bots\",\"schema\":3,\"tick\":850,\"bots\":[]}\n",
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

    #[test]
    fn a_recorder_killed_before_finish_still_has_samples_on_disk() {
        // The gap this closes: run-1788315106-86443 was killed by a
        // wall-clock timeout mid-execution and left no `samples.jsonl` at
        // all, because ingestion only ever ran inside `finish()`. Calling
        // `ingest_samples` at a milestone boundary -- as this test stands in
        // for -- means a run that dies before `finish()` still keeps
        // everything through its last closed milestone.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join("samples.jsonl"),
            "{\"kind\":\"bots\",\"schema\":3,\"tick\":100,\"run\":\"r5\",\"bots\":[]}\n",
        )
        .unwrap();

        let root = tmpdir("killed");
        let mut rec = RunRecorder::start(&root, "r5").unwrap();
        rec.record(
            100,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        // Stands in for the milestone-boundary keyframe call
        // (`record.keyframe()` in the Lua supervisor), which also ingests
        // samples now.
        let appended = rec.ingest_samples(Some(&workspace)).unwrap();
        assert_eq!(appended, 1);
        // No finish() -- the process died here, exactly like run 11.

        assert!(
            !rec.dir().join("manifest.json").exists(),
            "a crashed run still has no verdict"
        );
        let samples_path = rec.dir().join("samples.jsonl");
        assert!(
            samples_path.exists(),
            "samples.jsonl must survive a run that never reached finish()"
        );
        let archived = read_samples(&samples_path).unwrap();
        assert_eq!(archived.samples.len(), 1);
        assert_eq!(archived.samples[0].tick, 100);
    }

    #[test]
    fn ingesting_samples_twice_before_finish_does_not_duplicate_lines() {
        // A long run may close many milestones, each calling
        // `ingest_samples`. The source only grows between calls (the mod
        // appends), and each call must pick up only what is new.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        let source = out.join("samples.jsonl");
        fs::write(
            &source,
            "{\"kind\":\"bots\",\"schema\":3,\"tick\":100,\"run\":\"r6\",\"bots\":[]}\n",
        )
        .unwrap();

        let root = tmpdir("dedup");
        let mut rec = RunRecorder::start(&root, "r6").unwrap();
        rec.record(
            100,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();

        assert_eq!(rec.ingest_samples(Some(&workspace)).unwrap(), 1);
        // Nothing new written yet: a second call at the next milestone must
        // append nothing further.
        assert_eq!(rec.ingest_samples(Some(&workspace)).unwrap(), 0);

        // The mod appends one more sample before the run finishes.
        {
            use std::io::Write as _;
            let mut f = fs::OpenOptions::new().append(true).open(&source).unwrap();
            writeln!(
                f,
                "{{\"kind\":\"bots\",\"schema\":3,\"tick\":200,\"run\":\"r6\",\"bots\":[]}}"
            )
            .unwrap();
        }

        let (manifest, _) = rec
            .finish(300, "done", Some(&workspace), DEFAULT_KEEP)
            .unwrap();
        assert_eq!(
            manifest.samples, 2,
            "finish() must pick up only the newly-appended sample, on top of the one already archived"
        );

        let archived = read_samples(&root.join("r6").join("samples.jsonl")).unwrap();
        assert_eq!(
            archived.samples.len(),
            2,
            "no sample line may be archived twice: {:?}",
            archived.samples
        );
    }

    /// Appends one `bots` sample line for `run` at `tick` to the mod's file.
    fn mod_writes_sample(source: &Path, run: &str, tick: u64) {
        use std::io::Write as _;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(source)
            .unwrap();
        writeln!(
            f,
            "{{\"kind\":\"bots\",\"schema\":3,\"tick\":{tick},\"run\":\"{run}\",\"bots\":[]}}"
        )
        .unwrap();
    }

    #[test]
    fn a_long_milestone_does_not_hold_the_whole_run_s_samples_hostage() {
        // Run `run-1788459085-32452`, verbatim in shape: milestone 1 closed at
        // tick 78,885 and milestone 2 was still open 199,449 ticks later when
        // the run was killed. Ingestion only ran at those boundaries and at
        // `finish`, so the archive stopped at tick 78,840 and the entire
        // window in which the red-science cell was built, powered and fed --
        // the thing the run existed to observe -- was never copied out of the
        // mod's own file, which had it all along.
        //
        // No `ingest_samples` call and no `finish` anywhere below: the point is
        // that `record` alone keeps up.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        let source = out.join("samples.jsonl");

        let root = tmpdir("long-milestone");
        let mut rec = RunRecorder::start(&root, "r7").unwrap();
        rec.watch_samples(&workspace);

        // One milestone, opened and never closed.
        mod_writes_sample(&source, "r7", 4740);
        rec.record(
            4732,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "the long one".into(),
            },
        )
        .unwrap();

        // The mod samples on its beat; the run reports actions on its own.
        // Neither of them is a milestone boundary.
        let mut tick = 4732;
        for step in 0..40u64 {
            tick += 5_000;
            mod_writes_sample(&source, "r7", tick - 20);
            rec.record(
                tick,
                EventKind::ActionDispatched {
                    id: step as u32,
                    bot: 1,
                    action: "mine 4 iron-ore".into(),
                    target: None,
                    delivery: None,
                },
            )
            .unwrap();
        }

        // Killed here, exactly like run 32452: no milestone ever closed, so no
        // keyframe ever ran, so `finish` never ran either.
        assert!(!rec.dir().join("manifest.json").exists());

        let archived = read_samples(&rec.dir().join("samples.jsonl")).unwrap();
        let last = archived.samples.iter().map(|s| s.tick).max().unwrap();
        assert!(
            tick.saturating_sub(last) <= SAMPLE_INGEST_INTERVAL_TICKS,
            "the archive must stay within {SAMPLE_INGEST_INTERVAL_TICKS} ticks of the run \
             even with no milestone boundary to hang ingestion on -- \
             run reached {tick}, last archived sample was {last}"
        );
    }

    #[test]
    fn finish_says_how_far_short_the_samples_fell() {
        // The mod goes quiet at tick 1,000 and the run carries on to 90,000.
        // Nothing else about the run looks wrong -- the events are complete,
        // the outcome is `done` -- so the shortfall has to be stated, or it
        // reads as a run that simply had less to say.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        mod_writes_sample(&out.join("samples.jsonl"), "r8", 1_000);

        let root = tmpdir("lag");
        let mut rec = RunRecorder::start(&root, "r8").unwrap();
        rec.record(
            900,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        let (manifest, _) = rec
            .finish(90_000, "done", Some(&workspace), DEFAULT_KEEP)
            .unwrap();

        assert_eq!(
            manifest.samples_lag_ticks,
            Some(89_000),
            "the manifest must carry the gap between the last sample and the run's end"
        );
    }

    #[test]
    fn a_fully_sampled_run_reports_no_lag_worth_the_name() {
        // The other half of the guard: it must not cry wolf on a healthy run,
        // or the warning becomes something to scroll past.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        mod_writes_sample(&out.join("samples.jsonl"), "r9", 8_940);

        let root = tmpdir("no-lag");
        let mut rec = RunRecorder::start(&root, "r9").unwrap();
        rec.record(
            900,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        let (manifest, _) = rec
            .finish(9_000, "done", Some(&workspace), DEFAULT_KEEP)
            .unwrap();

        let lag = manifest.samples_lag_ticks.unwrap();
        assert!(
            lag <= SAMPLE_INGEST_INTERVAL_TICKS,
            "a run sampled to its last beat must not be reported as short: lag was {lag}"
        );
    }

    #[test]
    fn a_run_with_no_samples_reports_no_lag_rather_than_a_lag_of_zero() {
        // A planning-only run captures nothing by design. `Some(0)` would
        // claim its samples covered the whole run, which is a stronger
        // statement than "there were none".
        let root = tmpdir("no-samples");
        let mut rec = RunRecorder::start(&root, "r10").unwrap();
        rec.record(
            10,
            EventKind::MilestoneStarted {
                index: 1,
                goal: "g".into(),
            },
        )
        .unwrap();
        let (manifest, _) = rec.finish(500, "done", None, DEFAULT_KEEP).unwrap();
        assert_eq!(manifest.samples_lag_ticks, None);
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

#[cfg(test)]
mod flow_tests {
    use super::*;

    #[test]
    fn record_flow_appends_one_line_per_call_and_flushes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut recorder = RunRecorder::start(tmp.path(), "run-test-flow").unwrap();
        let export = crate::graph::flow_export::FlowExport {
            tick: 100,
            nodes: vec![],
            edges: vec![],
        };
        recorder.record_flow(&export).unwrap();
        let export2 = crate::graph::flow_export::FlowExport {
            tick: 200,
            nodes: vec![],
            edges: vec![],
        };
        recorder.record_flow(&export2).unwrap();

        let read = crate::record::flow::read_flow(&recorder.dir().join("flow.jsonl")).unwrap();
        assert_eq!(read.records.len(), 2);
        assert_eq!(read.records[0].tick, 100);
        assert_eq!(read.records[1].tick, 200);
    }
}

/// The free-vision disclosure: how much ground the model was given against how
/// much ground a bot covered. See [`EventKind::VisionMeasured`].
#[cfg(test)]
mod vision_tests {
    use super::*;
    use crate::test_utils::entity_graph_from;
    use crate::types::{Direction, FactorioEntity};

    /// A model holding one crude-oil tile 500.7 tiles from the origin -- the
    /// shape of `run-1788532631-48030`, whose bots never left 63.8.
    fn model_with_oil_far_out() -> Arc<EntityGraph> {
        Arc::new(
            entity_graph_from(vec![FactorioEntity::new_resource(
                &Position::new(300.5, 400.5),
                Direction::North,
                "crude-oil",
            )])
            .expect("adding must not fail"),
        )
    }

    fn vision_events(rec: &RunRecorder) -> Vec<EventKind> {
        read_events(&rec.dir().join("events.jsonl"))
            .unwrap()
            .events
            .into_iter()
            .map(|e| e.kind)
            .filter(|k| matches!(k, EventKind::VisionMeasured { .. }))
            .collect()
    }

    fn workspace_with_bot_at(root: &Path, tick: u64, run: &str, x: f64, y: f64) -> PathBuf {
        let workspace = root.join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        fs::create_dir_all(&out).unwrap();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out.join("samples.jsonl"))
            .unwrap();
        writeln!(
            file,
            r#"{{"kind":"bots","schema":3,"tick":{tick},"run":"{run}","bots":[{{"id":1,"position":{{"x":{x},"y":{y}}},"inventory":{{}},"crafting_queue":0,"mining":null}}]}}"#
        )
        .unwrap();
        workspace
    }

    /// **The disclosure exists from the run's first event, not from its last.**
    ///
    /// `provenance.json` is written at start and never rewritten, and
    /// `manifest.json` only exists for runs that finished -- nine of the
    /// twenty-four runs archived when provenance was added had none, and they
    /// are the killed ones. A killed run still got its free vision, so this
    /// has to be in the append-only log and it has to be there early.
    #[test]
    fn a_run_discloses_its_free_vision_from_its_first_event() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v1").unwrap();
        rec.watch_model(model_with_oil_far_out());
        rec.record(
            3658,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();

        let events = vision_events(&rec);
        assert_eq!(
            events.len(),
            1,
            "one baseline, written on the run's first recorded event whatever \
             tick the run opened at"
        );
        let EventKind::VisionMeasured {
            model_tiles,
            model_furthest,
            model_resource_tiles,
            ..
        } = &events[0]
        else {
            unreachable!()
        };
        assert_eq!(model_furthest.as_deref(), Some("crude-oil"));
        assert_eq!(*model_resource_tiles, 1);
        assert!((model_tiles.unwrap() - 300.5f64.hypot(400.5)).abs() < 1e-9);
    }

    /// **A run with no travel data says so; it does not report zero.**
    ///
    /// A bot that was never observed has not been observed at the origin, and
    /// a ratio computed against it would be an invention. `bot_samples: 0` is
    /// what tells a reader which of the two the null means.
    #[test]
    fn missing_travel_data_reads_as_unknown_and_never_as_no_travel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v2").unwrap();
        rec.watch_model(model_with_oil_far_out());
        rec.record(
            10,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();

        let text = fs::read_to_string(rec.dir().join("events.jsonl")).unwrap();
        assert!(
            text.contains(r#""travelled_tiles":null"#),
            "present-and-null, so a reader can tell we looked: {text}"
        );
        assert!(text.contains(r#""bot_samples":0"#));
        assert!(
            text.contains(r#""unearned_ratio":null"#),
            "no ratio can be computed against a travel nobody observed: {text}"
        );
    }

    /// The number the whole event exists for: the model sees N times further
    /// than the furthest bot got.
    #[test]
    fn the_ratio_is_the_model_s_reach_over_the_furthest_bot_s() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = workspace_with_bot_at(tmp.path(), 900, "run-v3", 30.0, 40.0);
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v3").unwrap();
        rec.watch_model(model_with_oil_far_out());
        rec.record(
            800,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        let (manifest, _) = rec
            .finish(1000, "done", Some(&workspace), DEFAULT_KEEP)
            .unwrap();
        // The closing measurement is written after the log is read for the
        // manifest's count, so the count has to include it explicitly. A
        // manifest that undercounts its own log makes every number in it
        // suspect.
        assert_eq!(
            manifest.events,
            read_events(&rec.dir().join("events.jsonl"))
                .unwrap()
                .events
                .len(),
            "the manifest must count every line in the log it summarises"
        );

        let events = vision_events(&rec);
        let EventKind::VisionMeasured {
            travelled_tiles,
            travelled_bot,
            bot_samples,
            unearned_ratio,
            ..
        } = events.last().expect("a closing measurement")
        else {
            unreachable!()
        };
        assert_eq!(*travelled_bot, Some(1));
        assert_eq!(*bot_samples, 1);
        assert!((travelled_tiles.unwrap() - 50.0).abs() < 1e-9, "3-4-5");
        assert!(
            (unearned_ratio.unwrap() - 300.5f64.hypot(400.5) / 50.0).abs() < 1e-9,
            "the model reaches 10x further than the bot ever went"
        );
    }

    /// Travel is a high-water mark: the question is how far a bot *ever* got,
    /// not where it is standing now, so a bot walking home must not shrink it.
    #[test]
    fn travel_is_a_high_water_mark_and_walking_back_does_not_shrink_it() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = workspace_with_bot_at(tmp.path(), 900, "run-v4", 30.0, 40.0);
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v4").unwrap();
        rec.watch_model(model_with_oil_far_out());
        rec.record(
            800,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        rec.ingest_samples(Some(&workspace)).unwrap();
        // The bot comes home.
        workspace_with_bot_at(tmp.path(), 950, "run-v4", 0.5, 0.5);
        rec.finish(1000, "done", Some(&workspace), DEFAULT_KEEP)
            .unwrap();

        let events = vision_events(&rec);
        let EventKind::VisionMeasured {
            travelled_tiles,
            travelled_at_tick,
            bot_samples,
            ..
        } = events.last().expect("a closing measurement")
        else {
            unreachable!()
        };
        assert!((travelled_tiles.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(*travelled_at_tick, Some(900), "the tick it was furthest at");
        assert_eq!(*bot_samples, 2, "both positions were read");
    }

    /// **A recorder with no model writes no measurement.**
    ///
    /// The alternative -- reporting a model nobody handed us as one reaching
    /// zero tiles -- would be a run claiming it cheated by nothing, asserted by
    /// an instrument that never looked. An absent event is unknown; a zero is
    /// a claim.
    #[test]
    fn a_recorder_with_no_model_discloses_nothing_rather_than_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v5").unwrap();
        rec.record(
            10,
            EventKind::MilestoneStarted {
                index: 0,
                goal: "g".into(),
            },
        )
        .unwrap();
        rec.finish(20, "done", None, DEFAULT_KEEP).unwrap();
        assert!(vision_events(&rec).is_empty());
    }

    /// Periodic, on [`VISION_MEASURE_INTERVAL_TICKS`]: a run that goes long
    /// enough to be killed still shows the two numbers diverging over time.
    #[test]
    fn measurements_beat_on_the_interval_and_close_unconditionally() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-v6").unwrap();
        rec.watch_model(model_with_oil_far_out());
        for tick in [100, 200, VISION_MEASURE_INTERVAL_TICKS + 100] {
            rec.record(
                tick,
                EventKind::MilestoneStarted {
                    index: 0,
                    goal: "g".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(
            vision_events(&rec).len(),
            2,
            "the baseline, then one beat -- tick 200 is inside the interval"
        );

        // The closing one ignores the interval entirely, so a run shorter than
        // one beat still states its figure.
        rec.finish(
            VISION_MEASURE_INTERVAL_TICKS + 200,
            "done",
            None,
            DEFAULT_KEEP,
        )
        .unwrap();
        assert_eq!(vision_events(&rec).len(), 3);

        // ...and it lands after `run_finished`, which is written first so that
        // a crash during archiving still leaves a log saying how the run
        // ended. A reader takes the last `vision_measured`, not the last line.
        let events = read_events(&rec.dir().join("events.jsonl")).unwrap().events;
        assert!(matches!(
            events[events.len() - 2].kind,
            EventKind::RunFinished { .. }
        ));
        assert!(matches!(
            events[events.len() - 1].kind,
            EventKind::VisionMeasured { .. }
        ));
    }
}
