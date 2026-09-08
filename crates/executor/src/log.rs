use factorio_bot_core::factorio::rcon::WalkStall;
use factorio_bot_core::factorio::ticks::ActionTicks;
use factorio_bot_core::record::map::Placement;
use factorio_bot_core::types::Position;
use factorio_bot_planner::{ActionId, BotId, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::Instant;

/// Narrows a game tick to the planner's [`Ticks`].
///
/// `game.tick` is a `MapTick` (uint64); `Ticks` is 32-bit, which covers about
/// 2.3 years of game time. A value that does not fit comes back **absent**
/// rather than wrapped: a wrapped tick would look like a perfectly plausible
/// early-game measurement and quietly corrupt anything aligned to it, whereas
/// `None` is a fact a consumer can act on.
fn narrow(tick: Option<u64>) -> Option<Ticks> {
    tick.and_then(|t| Ticks::try_from(t).ok())
}

/// Where one attempt (or one walk) stands, as far as the run can tell.
///
/// # `Running` and `Lost` are two different facts, and used to be one
///
/// `Running` used to mean both "this dispatch is in flight" and "this dispatch
/// was made and nobody is following it any more", because the second had
/// nowhere else to go. They need opposite renderings: the first is a bot at
/// work, the second is a session that has lost the thread, and drawing the
/// second as the first shows a busy bot for work nobody is watching. That is
/// the failure mode most worth surfacing, so it has its own name.
///
/// `Lost` claims nothing about the action itself. It does not say the action
/// failed — no verdict arrived, which is exactly the point — and it does not
/// say it will never finish; the game may well be finishing it right now. It
/// says only that **this run will not learn the outcome**. Two things put an
/// attempt here, and both are that same statement:
///
/// - The game answered and the answer carried no readable verdict
///   ([`crate::ActuatorError::NoVerdict`]). `crates/core`'s output parser
///   deliberately records no completion for an `action_completed` whose status
///   it cannot read — an unreadable status is not evidence of success — and
///   this is what that looks like from here.
/// - The run ended without an outcome for a dispatch it had made: the future
///   was dropped, the task aborted, the process went down. See
///   [`ExecutionLog::lose_track_of_outstanding`], which `run_into` calls as it
///   unwinds.
///
/// A `Lost` attempt is **not** finished: it never reached an outcome, so
/// [`ExecutionLog::planned_duration`] stays `None` and a later
/// [`ExecutionLog::start`] supersedes it as a retry, counting it exactly as it
/// counts a retry of an interrupted `Running` attempt.
///
/// The four states that existed before this one keep their meanings and their
/// order. `Lost` is added at the end so nothing that already reads this enum —
/// the Lua `status` string, a serialized log — changes what it says.
///
/// # `Success` asserts a different thing for each action kind
///
/// `Success` always means "the game gave a verdict and the verdict was good".
/// *What the game checked before saying so* is decided by the BotBridge handler
/// the action dispatches to, and the handlers do not all check the same
/// strength of thing. A consumer that renders one sentence over every green row
/// will be right about some kinds and wrong about others.
///
/// **`Insert` and `Remove` carry the strong reading: the items moved, in full.**
/// The mod's transfer handlers `complain` — which is `rcon.print`, so it writes
/// into the RPC's own reply body — whenever the entity or inventory is missing,
/// whenever the bot holds fewer items than the command asked to insert, and
/// whenever the count the inventory actually accepted or yielded differs from
/// the count requested. `factorio_bot_core::factorio::rcon`'s
/// `judge_transfer_reply` turns any surviving line into a refusal, so a
/// transfer of 10 that moved 7 — or 0 — reports [`Status::Failed`]. A green
/// transfer row may therefore be rendered as *items moved*; that is the one
/// kind where the count is part of the verdict. It is pinned by
/// `transfer_guarantee_tests` in that module, which runs the real
/// `mods/BotBridge/control.lua` rather than a fixture, because the property is
/// an interaction between the mod and the reply parser and neither half asserts
/// it alone.
///
/// **No other kind carries it, and none of them asserts a quantity.**
/// A `Place` is checked against a returned JSON entity — the thing exists, not
/// that it is the thing the plan wanted somewhere else. A `Mine` is checked
/// against an `action_completed` whose result is `"ok"`; the mod says the
/// mining finished, not how many items landed in the bot. A `Walk` is checked
/// against the path's endpoint distance, so it asserts the *path* would have
/// arrived, not that the bot is standing there now. Craft and research are
/// likewise `"ok"`/not-`"ok"`.
///
/// So: read a green `Insert`/`Remove` as a quantity, and every other green row
/// as "the game accepted this and did not object". Widening the transfer
/// reading to the rest would overstate exactly as much as the old one-line
/// "the game reported it done" understated the transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    /// Never dispatched. The log has no attempt for it at all.
    Pending,
    /// Dispatched, in flight, and somebody is still waiting for the reply.
    Running,
    /// The game reported it done. **What that is worth depends on the action
    /// kind** — see the type docs' section below.
    Success,
    /// The game reported a verdict, and the verdict was a failure — or the
    /// dispatch never reached the game. Either way something is known.
    Failed,
    /// Dispatched, and this run will not learn what became of it. See the type
    /// docs: this is a statement about what the run knows, not about the
    /// action.
    Lost,
}

/// One execution attempt of one action — always the **latest** one.
///
/// # Two kinds of tick live here, and they must never be confused
///
/// `planned_start_tick` and `planned_end_tick` are named for what they actually
/// hold. The only writer is `run_into`, which passes `ScheduledStep::start` and
/// `ScheduledStep::end` — numbers the *scheduler* computed from
/// `Action::duration` before anything ran. They are the estimate, not a
/// measurement of it, and `planned_duration()` is that estimate round-tripped
/// through the log.
///
/// `dispatched_tick` and `replied_tick` are the measurement. They are
/// `game.tick` as the game itself reported it — when it received the command
/// and when it reported the outcome — carried back through
/// [`Actuator`](crate::Actuator) as an [`ActionTicks`] and written by
/// [`ExecutionLog::observe`]. Nothing else may write them.
///
/// **The two are kept side by side deliberately.** The drift between the plan
/// and the game is the signal — it is what tells you the scheduler's model of
/// `Action::duration` is wrong, and it is what a consumer aligning a recording
/// to a plan needs in order to pin a moment to the tick it actually happened
/// at. Renaming `planned_*` to something that sounds measured, or filling
/// `dispatched_tick`/`replied_tick` in from the schedule when the game did not
/// answer, would destroy exactly that signal while leaving every reading
/// plausible.
///
/// # An absent tick is a value, and it means exactly one thing
///
/// `dispatched_tick` and `replied_tick` are `Option` and stay `None` whenever
/// **the game did not tell us**: a command that failed before it ever reached
/// the game, a reply whose stamp could not be parsed, a tick too large for
/// [`Ticks`], or an actuator with no clock at all. `None` is never to be
/// replaced with zero, with the planned tick, or with the previous action's
/// tick. A consumer must be able to say "no reply tick for this action"; a
/// fabricated number is worse than a missing one, because it will be built on.
///
/// That list used to have one more entry, and it did not belong: **any**
/// failure. The actuator's error carried no ticks, so a dispatch the game had
/// stamped and then refused arrived here empty — absent by *plumbing* rather
/// than by fact, a `None` that meant "we were handed a measurement and dropped
/// it" sitting beside a `None` that meant "there was nothing to measure", with
/// no way for a reader to tell which was which. [`crate::ActuatorFailure`]
/// carries the observation alongside the error, so a failed attempt now keeps
/// whatever the game stamped before it went wrong, and an absent tick here is
/// once again a statement about the game rather than about the wiring.
///
/// # `status` carries facts the ticks cannot
///
/// The ticks say when; `status` says what is known. Four attempts can hold
/// identical (even identically absent) ticks and still be four different
/// facts — succeeded unobserved, failed before dispatch, in flight right now,
/// dispatched and never accounted for — and [`Status`] is the only thing that
/// separates them.
///
/// The last of those is [`Status::Lost`], and it is the one worth naming here:
/// an attempt whose outcome this run will never learn, because the game's
/// answer carried no readable verdict or because the run ended still holding
/// the dispatch. It is not `Running` (nothing is in flight) and not `Failed`
/// (no verdict was ever given). Such an attempt has no `planned_end_tick`
/// either, because it never finished — so `planned_duration()` is `None` for
/// it, exactly as it is for one still in flight.
// `Eq` dropped when `placed` was added: `Placement` carries a `Position`,
// which carries `f64`, and `f64` has no total ordering to derive `Eq` from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub status: Status,
    /// Which attempt this is, counting from 1. Greater than 1 means the action
    /// was dispatched again after a failure or an interruption — see
    /// `ExecutionLog::start`. Saturates rather than wrapping: at `u32::MAX` the
    /// count stops being exact, which is harmless, whereas wrapping to 0 would
    /// make `attempts()` report an action that has run four billion times as
    /// never started, and every retry budget reading it would reset.
    pub number: u32,
    /// The tick the *schedule* placed this attempt's start at. Not observed.
    pub planned_start_tick: Ticks,
    /// The tick the *schedule* placed this attempt's end at, once finished.
    /// Not observed.
    pub planned_end_tick: Option<Ticks>,
    /// `game.tick` when the game **received** this attempt's command.
    /// `None` if the game never said — see the type docs.
    pub dispatched_tick: Option<Ticks>,
    /// `game.tick` when the game **reported the outcome** of this attempt.
    /// `None` if the game never said — see the type docs.
    pub replied_tick: Option<Ticks>,
    /// What this run has to say about the attempt in words, whatever that
    /// turns out to be. **Not "why it failed"** — [`Status`] is what says
    /// whether anything failed, and this field is deliberately shared by every
    /// state that has something to say, so a reader asking "what does this log
    /// say about this attempt?" never has to know which of several message
    /// fields to look in. [`ExecutionLog::lose_track`] already writes a
    /// not-a-verdict here for the same reason.
    ///
    /// Three writers, told apart by `status` alone:
    ///
    /// - [`Status::Failed`] — the verdict, from [`ExecutionLog::fail`].
    /// - [`Status::Lost`] — why the outcome is unknown, from
    ///   [`ExecutionLog::lose_track`].
    /// - [`Status::Success`] — a qualification on the success, from
    ///   [`ExecutionLog::record_note`]. Today that is exactly one thing: an
    ///   `insert` that filled its destination and kept the remainder.
    ///
    /// It reaches the run record verbatim as `action_settled`'s `error`, which
    /// `factorio_bot_core::record::EventKind::ActionSettled` documents as *the
    /// human-readable verdict* rather than as an error, and which is written
    /// beside `status` and the classified `failure` — `null` on a success — so
    /// the three cases stay distinguishable in `events.jsonl`.
    pub error: Option<String>,
    /// What this attempt placed, when it placed anything.
    ///
    /// Set only for a `Place` action, and only once the game has answered —
    /// see `RconActuator::place` (`crates/executor/src/rcon_actuator.rs`),
    /// which is the only place with both halves of it: the arguments are the
    /// intent, the entity the game hands back is the truth. Nothing in this
    /// crate reads the field yet; it exists so a later task can carry it out
    /// to `map.jsonl` alongside the rest of the run record.
    pub placed: Option<Placement>,
}

impl Attempt {
    /// The world-model divergence this attempt's verdict reports, if it is one.
    ///
    /// `Some` only for a [`Status::Failed`] attempt whose `error` is one of the
    /// mod's short-transfer complaints (see [`crate::divergence`]): the plan
    /// believed a container held `asked` of an item and the game moved `moved`.
    /// A `Success` carrying a full-destination note is not a divergence -- the
    /// bot delivered everything it had and the world is as the plan believed
    /// -- and a `Lost` attempt has no verdict to read one off.
    ///
    /// This is the failure *class* the log used to carry only as text. The
    /// record's classifier (`crates/scripting_lua/src/globals/record.rs`)
    /// reads the same text into `FailureKind::PartialTransfer`; `recover`
    /// reads it here, because it cannot see the record and must not retry
    /// what this names.
    pub fn divergence(&self) -> Option<crate::divergence::Divergence> {
        if self.status != Status::Failed {
            return None;
        }
        self.error
            .as_deref()
            .and_then(crate::divergence::divergence)
    }
}

/// One walk step, as the run observed it.
///
/// # Why walks are recorded at all, and why they need no `ActionId`
///
/// A walk is not an `Action` — the scheduler emits it as its own `StepKind`
/// with no id — and for that reason its ticks used to be measured and thrown
/// away: `Actuator::walk` has always returned [`ActionTicks`], and `run.rs`
/// kept only the failure bit. The stated reason was that "the walk needs an id
/// first". It does not. `run_bot_signalled` walks **one bot's steps in schedule
/// order**, so the pair `(bot, step_index)` is already unique and stable for
/// the run, and that pair is what these are keyed by.
///
/// Walking is most of the wall-clock in these plans, so this was the largest
/// hole in the timeline: without it a consumer aligning a recording to a plan
/// can only render an undifferentiated "walk + wait" span, not because the split is
/// unobservable but because the observation was being discarded.
///
/// # The same two kinds of tick as [`Attempt`], under the same rules
///
/// `planned_start_tick`/`planned_end_tick` are `ScheduledStep::start`/`end` —
/// what the *scheduler* predicted before anything ran. Unlike [`Attempt`]'s,
/// both are known the moment the step exists (a walk's span is fixed by the
/// schedule, not discovered by finishing), so neither is an `Option`.
///
/// `dispatched_tick`/`replied_tick` are the measurement: `game.tick` as the
/// game reported it, and they are `None` — never zero, never the planned value,
/// never the previous step's — whenever the game did not say.
///
/// # An unmeasured walk and a failed walk are different facts
///
/// Both carry `None` ticks, and `status` is what tells them apart:
///
/// - **Unmeasured**: `status == Success`, `error == None`. The walk happened;
///   the actuator had no clock, or the reply's stamp did not parse. Nothing
///   went wrong and nothing was learned about when.
/// - **Failed**: `status == Failed`, `error == Some(..)`. The walk did not
///   happen — or the game refused it after acknowledging it, in which case the
///   dispatch tick it stamped *is* kept here (see [`Attempt`]).
/// - **In flight**: `status == Running`. Dispatched, and somebody is still
///   waiting for the reply.
/// - **Lost**: `status == Lost`, `error == Some(..)`. Dispatched, and this run
///   will never learn how it went — see [`Status::Lost`]. A bot drawn as
///   walking when nobody is following it is the reason this is not `Running`.
///
/// Collapsing these into "no ticks" would make a bot that never moved
/// indistinguishable from one that moved unobserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkObservation {
    pub status: Status,
    /// Where the schedule sent the bot. `StepKind::Walk`'s own `to`.
    pub to: Position,
    /// The tick the *schedule* placed this walk's start at. Not observed.
    pub planned_start_tick: Ticks,
    /// The tick the *schedule* placed this walk's end at. Not observed.
    pub planned_end_tick: Ticks,
    /// `game.tick` when the game **received** the walk command.
    /// `None` if the game never said — see the type docs.
    pub dispatched_tick: Option<Ticks>,
    /// `game.tick` when the game **reported the bot had arrived**.
    /// `None` if the game never said — see the type docs.
    pub replied_tick: Option<Ticks>,
    pub error: Option<String>,
    /// How many of this bot's remaining steps were abandoned because this walk
    /// failed. `None` on every walk that did not halt its bot, and **on no
    /// other walk**: a failed walk that was the bot's last step abandons
    /// nothing and is written `Some(0)`, so `abandoned.is_some()` is exactly
    /// "this walk halted its bot" and nothing narrower. See
    /// [`ExecutionLog::halt_walk`], and
    /// `crate::recover::walk_halted_on_a_stall`, which is the query that
    /// depends on it. This sentence used to say the opposite about the
    /// last-step case, while
    /// `a_walk_that_fails_as_the_last_step_halts_with_nothing_behind_it`
    /// asserted `Some(0)` two files away.
    ///
    /// **A failed walk stops the bot** (`crate::run::run_bot_signalled` says
    /// why: a walk's effect is a position, and no plan edge carries it), and
    /// until 2026-09-08 that stop wrote *nothing anywhere*. `halt` published
    /// `Failed`/`Lost` over the `watch` senders so dependents would abandon
    /// themselves, and the senders are not the record: the abandoned steps
    /// were never dispatched, so `record.actions` had no attempt to write, and
    /// the run's own counters reported them as `pending`.
    ///
    /// What that cost, measured on `run-1788833726-34821`: the run ended
    /// `success=246 failed=7 lost=1 **pending=2041**` out of 2,295, and
    /// nothing in `events.jsonl` said where the 2,041 went. They went to four
    /// halts, and the largest was not a death — **bot 3 stalled on a tree at
    /// tick 7,980 and took 474 steps with it**, 19,000 ticks before the first
    /// bot died. Reading the record without this field, the run looks like one
    /// that simply ran out of time.
    ///
    /// It rides on the walk rather than in a `halts` map of its own so that
    /// every existing caller of `record.walks` gets it with no new call to
    /// forget — which is exactly how `record.deaths()` came to be uncalled for
    /// the one run in this project's history that had deaths in it.
    pub abandoned: Option<u32>,
    /// Every stall this walk survived, in the order the game answered them.
    ///
    /// `None` means **the actuator does not report stalls** -- a mock, a
    /// replay, anything that is not the RCON actuator -- and `Some(vec![])`
    /// means it looked and this walk did not stall. Collapsing those would
    /// make a run with no instrumentation read exactly like a run with no
    /// trouble, which is the whole reason this field exists.
    ///
    /// A stall that was *not* retried is not here: the third one becomes the
    /// walk's own `error` and its `failure.kind: stalled`. So the honest
    /// reading is "stalls the retry answered with a fresh path", and a failed
    /// walk's total is `stalls.len() + 1`.
    ///
    /// **What its absence cost**, counted 2026-09-08 over
    /// `workspace/session-logs`: 17 stalls were recovered by
    /// `FactorioRcon::move_player_timed`'s retry and **not one of them reached
    /// `events.jsonl`**, against the single stall the archive holds -- the
    /// `tree-01` the retry could not fix. The record could not tell a run
    /// where walking went fine from one where it failed 17 times and recovery
    /// saved it.
    #[serde(default)]
    pub stalls: Option<Vec<WalkStall>>,
}

/// What a step is doing while it is not dispatching anything.
///
/// # The three silences this separates
///
/// A bot with nothing in flight looks exactly the same from outside whether it
/// is waiting on another bot, serving a lag deadline the plan asked for, or
/// holding a dispatch the game never answered. In `run-1788550000`-era records
/// they were literally indistinguishable: [`crate::ExecutionLog`] said
/// `Pending` for the first two and `Running` for the third, and `Running` is
/// also what a perfectly healthy in-flight action says. One action sat that
/// way for **eleven minutes** and the record could show frozen counters, a bot
/// id, and nothing else -- identifying it needed a live `rcon` query against
/// the running game, which is not available at all once the run is over.
///
/// These are not verdicts and nothing here decides that any of them is too
/// long. They are the answer to "waiting for what", which nobody could ask.
///
/// # Why it lives in the log rather than beside it
///
/// The log is already the one shared, lock-guarded thing the executor writes
/// and an outside watcher reads -- `run:progress()` and the batch heartbeat
/// both go through it. A second channel would mean threading a reporter
/// through `run_into`, which is exactly what `beat_batch_progress`
/// (`crates/scripting_lua/src/globals/goal/run.rs`) declined to do for the
/// recorder.
///
/// It is deliberately **not** part of the log as a record: see
/// [`ExecutionLog`]'s own docs for why it is skipped by serde and excluded
/// from equality. A wait is true right now and false a moment later; the
/// durable facts are the attempts.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitKind {
    /// Blocked in `await_preds` on a predecessor that has not settled.
    ///
    /// The dependent has not been dispatched and will not be until `on`
    /// publishes a verdict. `on` may belong to another bot entirely, which is
    /// the case a reader cannot reconstruct from the log alone: the blocked
    /// action has no attempt, so nothing in `attempts` mentions it.
    Predecessor { on: ActionId },
    /// Blocked in `run_bot_signalled` on a **background** action this same bot
    /// queued earlier, because the two touch the same items.
    ///
    /// Distinct from `Predecessor` because it is not a plan edge: the plan
    /// permits these in either order and the executor serialises them anyway
    /// to protect the inventory arithmetic (see [`crate::occupancy`]). A bot
    /// stopped here has dispatched everything the plan let it and is waiting
    /// on itself.
    BackgroundConflict { on: ActionId },
    /// Serving a lag edge in `wait_out_lag`: machine time the plan modelled,
    /// which the predecessor's own completion does not cover.
    ///
    /// **This is a wait the plan asked for, and a long one here is not by
    /// itself a fault** -- a smelt is thousands of ticks. It is separated from
    /// the others precisely so that "the run is doing what it was told" stops
    /// looking like "the run is stuck".
    LagDeadline {
        /// The absolute `game.tick` the wait is being served until, when a
        /// predecessor supplied a finish tick to anchor it to. `None` when
        /// none did and the wait can only run from now -- see
        /// `run::LagWait`.
        deadline_tick: Option<u64>,
        /// The largest lag any predecessor imposed, in game ticks. The
        /// upper bound on what this wait can cost.
        largest_lag: Ticks,
    },
    /// Polling `technology_researched` for a technology this plan's own
    /// earlier steps unlock. Bounded by `RESEARCH_SETTLE_BUDGET`, so it cannot
    /// be the cause of a long silence -- which is exactly why it is worth
    /// being able to rule out.
    Research { tech: String },
    /// **Dispatched, and the game has not answered.**
    ///
    /// # A `reply` older than six minutes is a finding in itself
    ///
    /// Nothing in *this crate* times out a dispatch, but the layer below does:
    /// `ACTION_RESULT_DEADLINE` (`crates/core/src/factorio/rcon.rs`) gives a
    /// dispatched action 360 wall-clock seconds to produce a verdict and then
    /// reports `Dispatch::NoVerdict`, which arrives here as [`Status::Lost`].
    ///
    /// So a `reply` wait past ~360s **cannot** be an action sitting in
    /// `sleep_for_action_result`, and the time has gone somewhere else inside
    /// the same call: a path request, the `move_player` a mine or a placement
    /// does when the bot is out of reach, or the placement retry loop. Each of
    /// those is its own bounded wait, and a chain of them is unbounded.
    ///
    /// This is what settles the question the eleven-minute silence raised.
    /// `lost` stayed `0` and that was **correct**, not a miscount: no single
    /// dispatch had gone 360s unanswered. Splitting `reply` further would mean
    /// instrumenting the RPC helpers themselves, which is a change to
    /// `crates/core` rather than to this registry.
    Reply,
    /// Walking. Has no `ActionId` of its own, like every walk.
    Walk { to: Position },
}

impl WaitKind {
    /// A short, stable name for this state, for a record consumer to group by.
    ///
    /// Deliberately not the `Debug` rendering: that would change whenever a
    /// field is added, and archived runs would stop grouping with new ones.
    pub fn name(&self) -> &'static str {
        match self {
            WaitKind::Predecessor { .. } => "predecessor",
            WaitKind::BackgroundConflict { .. } => "background_conflict",
            WaitKind::LagDeadline { .. } => "lag_deadline",
            WaitKind::Research { .. } => "research",
            WaitKind::Reply => "reply",
            WaitKind::Walk { .. } => "walk",
        }
    }
}

/// What a [`WaitKind`] is recorded against.
///
/// Two key spaces because the executor has two: an action has an `ActionId`, a
/// walk has only `(bot, step_index)` -- the same split [`WalkObservation`]
/// documents, and for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WaitKey {
    Action(ActionId),
    Walk(BotId, usize),
}

/// One entry of [`ExecutionLog::waiting`]: who is waiting, for what, and for
/// how long.
#[derive(Debug, Clone, PartialEq)]
pub struct Wait {
    pub key: WaitKey,
    /// The bot the step belongs to. Read from the *schedule* at the moment the
    /// wait was entered, because neither an `Action` nor an `Attempt` knows it
    /// -- assigning work to a bot is the scheduler's job alone.
    pub bot: BotId,
    pub kind: WaitKind,
    /// How long this state has held, measured from the moment the executor
    /// entered it. **A measurement, not an estimate**: it is not the beat
    /// interval rounded, and it is not inferred from the first heartbeat that
    /// noticed the wait.
    pub elapsed: Duration,
}

/// The private half of [`Wait`]: the same facts with the start instant instead
/// of the elapsed time.
#[derive(Debug, Clone)]
struct WaitEntry {
    bot: BotId,
    kind: WaitKind,
    since: Instant,
}

/// Execution state, keyed by action.
///
/// Deliberately separate from `Schedule`: the schedule is an immutable plan
/// value, and progress is a join over the two rather than a mutation of the
/// plan. `BTreeMap` because iteration order is part of the contract —
/// `failed()` returns ids in a stable order so recovery is reproducible.
///
/// Only the latest attempt of each action is kept; `Attempt::number` says how
/// many there have been. See `start` for why a count rather than a history.
///
/// Walks live in a second map because they are keyed by something else
/// entirely: they have no `ActionId`, and `(bot, step_index)` is what
/// identifies them (see [`WalkObservation`]). It is nested rather than keyed by
/// a `(BotId, usize)` tuple so the whole log stays JSON-serializable — serde's
/// JSON map keys must be scalars, and a tuple key would silently turn this
/// derive into a runtime error for anyone who ever serialized it.
///
/// # Why `Eq` is derived on [`Attempt`] but not here
///
/// [`WalkObservation`] holds a [`Position`], whose fields are `f64`. Float
/// equality is not an equivalence relation, so `Eq` would be a false claim.
/// `PartialEq` — which is all any caller and every `assert_eq!` needs — is
/// kept.
///
/// # `waiting` is a live view, not a record, and is excluded from both
///
/// [`WaitKind`] says what a step is doing *right now*. It is `#[serde(skip)]`
/// and left out of the hand-written [`PartialEq`] below, for three reasons
/// that all point the same way:
///
/// - it holds an [`Instant`], which has no meaning outside the process that
///   read it and no serialized form at all;
/// - two logs of the same run are the same log, and a difference in what each
///   happened to be waiting for at the instant it was cloned is not a
///   difference in what the run did — `the_final_log_is_the_same_whichever_bot_finishes_first`
///   asks that question and must keep getting the old answer;
/// - a wait is entered and left within one `run_into`, so a log that outlives
///   the run (a seed for a recovery, a replay) has nothing true to say here
///   anyway.
///
/// The durable record of a wait is the attempt's ticks. This is the answer to
/// "what is happening at this moment", which nothing else could give.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionLog {
    attempts: BTreeMap<ActionId, Attempt>,
    /// `bot -> step_index -> observation`. Both levels are `BTreeMap` for the
    /// same reason `attempts` is: iteration order is part of the contract, so
    /// `walks()` yields the same sequence on any two runs of the same log.
    walks: BTreeMap<BotId, BTreeMap<usize, WalkObservation>>,
    /// What each step that is waiting is waiting for. `BTreeMap` so
    /// [`ExecutionLog::waiting`] answers in a stable order; see the type docs
    /// for why it is skipped by serde and by equality.
    #[serde(skip)]
    waiting: BTreeMap<WaitKey, WaitEntry>,
}

/// Hand-written so `waiting` is excluded — see [`ExecutionLog`]'s docs. Every
/// other field is compared exactly as the derive did.
impl PartialEq for ExecutionLog {
    fn eq(&self, other: &Self) -> bool {
        self.attempts == other.attempts && self.walks == other.walks
    }
}

impl ExecutionLog {
    pub fn attempt(&self, id: ActionId) -> Option<&Attempt> {
        self.attempts.get(&id)
    }

    pub fn status(&self, id: ActionId) -> Status {
        self.attempts.get(&id).map_or(Status::Pending, |a| a.status)
    }

    /// How many times `id` has been started. Zero if never.
    ///
    /// Greater than one means `recover` proposed the action again after a
    /// failure and the caller ran it. A caller that wants to stop retrying a
    /// hopeless action reads this — `recover` itself is pure and keeps no
    /// memory across rounds, so this counter is the only record that a retry
    /// happened at all.
    pub fn attempts(&self, id: ActionId) -> u32 {
        self.attempts.get(&id).map_or(0, |a| a.number)
    }

    /// Records that an attempt has begun.
    ///
    /// Two cases hide behind "the attempt already finished", and they want
    /// opposite answers:
    ///
    /// - **A duplicate dispatch.** One `ActionId` in two schedule steps, or two
    ///   bots handed the same action. The action was meant to happen once, and
    ///   re-opening the attempt would destroy the outcome already recorded for
    ///   it. First completion wins.
    /// - **A retry.** `recover` returns the failed action in its tier-1
    ///   proposal *by design*, so the caller runs it again. Refusing the write
    ///   here is what made a successful retry unrecordable: the attempt stayed
    ///   `Failed` forever, `failed()` kept naming it, and the caller kept being
    ///   handed the same proposal — an unbounded retry loop issuing real
    ///   commands to a live server.
    ///
    /// The status separates them, and it separates them exactly. `run_into`
    /// drops duplicate ids from a schedule before dispatching (`planned_steps`),
    /// so within one run an action is started once; a second `start` on a
    /// `Success` attempt can therefore only be a duplicate reaching the log by
    /// some other route, and re-running succeeded work is a bug either way. A
    /// `start` on a `Failed` attempt has no such reading: nothing re-dispatches
    /// a failure inside one run, so it is a retry.
    ///
    /// Hence: **a retry supersedes a `Failed` attempt; a `Success` attempt is
    /// left exactly as it is.** The superseding attempt inherits `number + 1`
    /// so the fact of the retry outlives the attempt it replaced.
    ///
    /// A count, not a `Vec<Attempt>` history. The question anyone actually has
    /// here is "was this retried, and how often" — a retry budget, a stuck-action
    /// check — and a history answers it while forcing every reader (`status`,
    /// `planned_duration`, `failed`, and every consumer of the serialized log)
    /// to first answer "which attempt?", turning one field into a decision at
    /// each of them. The cost is real and is stated rather than hidden: the
    /// superseded attempt's error message is lost. That is tolerable because the
    /// caller that chose to retry had that message in hand — `recover` surfaced
    /// it in the round that produced the retry — and a `Vec` is the obvious
    /// upgrade if diagnosing across attempts ever becomes the job.
    pub fn start(&mut self, id: ActionId, tick: Ticks) {
        let number = match self.attempts.get(&id) {
            // A duplicate dispatch of work that already succeeded.
            Some(a) if a.status == Status::Success => return,
            // A retry of a failure: supersede it, and remember it happened.
            Some(a) if a.status == Status::Failed => a.number.saturating_add(1),
            // A retry of an *interrupted* attempt. `Running` means the last run
            // died between dispatch and reply, so starting it again is a retry
            // exactly like the `Failed` case — and the case a retry budget is
            // most needed for, since an interrupted action produces no verdict
            // to escalate on. Counting it flat was a hole: three interrupted
            // runs left `attempts() == 1`, so no budget built on this counter
            // could ever trip.
            //
            // Nothing double-starts a `Running` attempt inside one run:
            // `planned_steps` drops duplicate ids before dispatch and each
            // surviving step calls `start` once. So a second `start` on a
            // `Running` attempt is always a later run picking the action up
            // again.
            Some(a) => a.number.saturating_add(1),
            None => 1,
        };
        self.attempts.insert(
            id,
            Attempt {
                status: Status::Running,
                number,
                planned_start_tick: tick,
                planned_end_tick: None,
                dispatched_tick: None,
                replied_tick: None,
                error: None,
                placed: None,
            },
        );
    }

    /// Records the game ticks observed for `id`'s current attempt.
    ///
    /// Separate from [`ExecutionLog::succeed`]/[`ExecutionLog::fail`] on
    /// purpose: those take *plan* numbers, this one takes *game* numbers, and
    /// keeping the two writers apart is what makes it impossible to pass a
    /// schedule value here by slipping an argument. It writes nothing but the
    /// two observed fields.
    ///
    /// Only an attempt that already exists is annotated. There is deliberately
    /// no upsert: an observation with no attempt to attach to would be an
    /// observation of nothing, and inventing an attempt for it would put an
    /// action in the log that the run never started.
    ///
    /// Guarded by `has_finished` for the same reason `succeed` is — a duplicate
    /// completion inside one run must not overwrite the timing already
    /// recorded, or which numbers survive would depend on which writer the game
    /// answered first. A retry is unaffected: `start` reopens the attempt (and
    /// clears these fields) before the retry can observe anything.
    pub fn observe(&mut self, id: ActionId, ticks: ActionTicks) {
        if self.has_finished(id) {
            return;
        }
        if let Some(a) = self.attempts.get_mut(&id) {
            a.dispatched_tick = narrow(ticks.dispatched);
            a.replied_tick = narrow(ticks.replied);
        }
    }

    /// Whether this attempt has already reached an outcome. A superseding
    /// `start` clears it, which is what lets a retry record its own.
    fn has_finished(&self, id: ActionId) -> bool {
        self.attempts
            .get(&id)
            .is_some_and(|a| a.planned_end_tick.is_some())
    }

    /// Records a success. Upserts: if no `start()` was ever recorded for
    /// `id`, an attempt is created rather than the write being dropped, so
    /// completions are never lost from the log — the synthesized
    /// `planned_start_tick` is honest that we never observed a start either.
    ///
    /// A second completion of an already-finished attempt is **ignored**, not
    /// asserted against. This used to be a `debug_assert!`, but the only way
    /// to reach it is malformed input — the same `ActionId` in two schedule
    /// steps — and the panic fired inside a `join_all`, unwinding every other
    /// bot along with it. Bad input is not a reason to abort a run that is
    /// otherwise going fine. Ignoring the later write is also the only
    /// order-independent choice available here: overwriting would make the
    /// recorded outcome and duration depend on which writer the game answered
    /// first.
    ///
    /// This guard is about *concurrent duplicates within one run*, which is why
    /// it is untouched by the retry rule above: a retry re-opens the attempt in
    /// `start` first, so by the time it completes there is nothing finished to
    /// ignore.
    pub fn succeed(&mut self, id: ActionId, tick: Ticks) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            number: 1,
            planned_start_tick: tick,
            planned_end_tick: None,
            dispatched_tick: None,
            replied_tick: None,
            error: None,
            placed: None,
        });
        a.status = Status::Success;
        a.planned_end_tick = Some(tick);
    }

    /// Attaches a placement to `id`'s attempt, once the game has confirmed it.
    ///
    /// Called from `run.rs`'s settle path right after [`ExecutionLog::succeed`]
    /// for the same id, once `Actuator::take_placement` has handed the fact
    /// back — so by the time this runs the attempt it belongs to always
    /// exists. No upsert regardless: unlike `succeed`/`fail`, a placement with
    /// no attempt to attach to describes work this log never started, and
    /// inventing one for it would be the same fabrication `observe` refuses
    /// for the same shape of call. Silently a no-op in that case, following
    /// [`ExecutionLog::lose_track`]'s convention rather than panicking.
    pub fn record_placement(&mut self, id: ActionId, placement: Placement) {
        if let Some(a) = self.attempts.get_mut(&id) {
            a.placed = Some(placement);
        }
    }

    /// Attaches a note to `id`'s attempt: something true about it that its
    /// [`Status`] does not say.
    ///
    /// Called from `run.rs`'s settle path right after [`ExecutionLog::succeed`]
    /// for the same id, exactly like [`ExecutionLog::record_placement`], and
    /// with the same no-upsert rule: a note about an attempt this log never
    /// started describes nothing, so it is silently dropped rather than
    /// fabricating an attempt to hang it on.
    ///
    /// The one caller today is the full-destination insert — a success that did
    /// not deliver everything, because the destination had no room for the rest
    /// (`judge_transfer_reply`, `crates/core/src/factorio/rcon.rs`). It writes
    /// into [`Attempt::error`] on purpose; see that field's docs for why the
    /// message field is shared and `status` is what separates the cases.
    pub fn record_note(&mut self, id: ActionId, note: String) {
        if let Some(a) = self.attempts.get_mut(&id) {
            a.error = Some(note);
        }
    }

    /// Records a failure. Upserts, and ignores a second completion, for the
    /// same reasons as `succeed()`.
    pub fn fail(&mut self, id: ActionId, tick: Ticks, error: String) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            number: 1,
            planned_start_tick: tick,
            planned_end_tick: None,
            dispatched_tick: None,
            replied_tick: None,
            error: None,
            placed: None,
        });
        a.status = Status::Failed;
        a.planned_end_tick = Some(tick);
        a.error = Some(error);
    }

    /// Records that this run will not learn `id`'s outcome.
    ///
    /// The writer for [`Status::Lost`]. `why` explains **why the outcome is
    /// unknown** — it is not a verdict, and it is deliberately kept in the same
    /// `error` field a failure uses, because a reader asking "what does this
    /// log say about this attempt?" should not have to know which of two
    /// message fields to look in. `status` is what distinguishes them, and it
    /// is the only thing that can.
    ///
    /// Unlike [`ExecutionLog::fail`] this writes **no** `planned_end_tick`.
    /// That field means "the tick the schedule placed this attempt's end at,
    /// once finished", and an attempt whose outcome nobody knows never
    /// finished; filling it in would report a completed span for work that may
    /// still be running. The consequence is deliberate: a lost attempt is not
    /// `has_finished`, so a reply that somehow does arrive can still be
    /// recorded, and a retry supersedes it in [`ExecutionLog::start`] exactly
    /// as it supersedes an interrupted one.
    ///
    /// Only an attempt that exists is marked. Losing track of a dispatch that
    /// was never made would be a record of nothing — see
    /// [`ExecutionLog::observe`], same rule.
    pub fn lose_track(&mut self, id: ActionId, why: &str) {
        if let Some(a) = self.attempts.get_mut(&id) {
            a.status = Status::Lost;
            a.error = Some(why.to_string());
        }
    }

    /// [`ExecutionLog::lose_track`] for a walk.
    pub fn lose_track_walk(&mut self, bot: BotId, step_index: usize, why: &str) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.status = Status::Lost;
            w.error = Some(why.to_string());
        }
    }

    /// Marks everything still in flight as [`Status::Lost`], returning how many
    /// entries that was.
    ///
    /// For the moment a session stops following this log: the run's future was
    /// dropped, its task aborted, the process is going down. `run_into` calls
    /// it as it unwinds, so a run that is abandoned mid-dispatch stops
    /// reporting a bot as busy — but it is public because the caller that
    /// abandons a run is often the only one that knows it has.
    ///
    /// Only `Running` entries are touched. An attempt with an outcome keeps it
    /// (which numbers survive must not depend on when somebody gave up), and an
    /// action nothing ever dispatched stays `Pending` — it was never being
    /// followed, so there is nothing to lose track of.
    ///
    /// Calling it twice is harmless: the second call finds nothing running.
    ///
    /// It also clears [`ExecutionLog::waiting`], because nothing is waiting
    /// any more — nobody is following any of it. The RAII guards in `run.rs`
    /// normally do this as the bot futures unwind and this finds an empty map;
    /// it is repeated here so that a log which somehow outlives its guards
    /// cannot go on reporting a bot as blocked on a predecessor for the rest
    /// of the process's life. A stale wait would be exactly the kind of
    /// confident-but-wrong report the registry exists to replace.
    pub fn lose_track_of_outstanding(&mut self, why: &str) -> usize {
        self.waiting.clear();
        let mut lost = 0;
        for a in self.attempts.values_mut() {
            if a.status == Status::Running {
                a.status = Status::Lost;
                a.error = Some(why.to_string());
                lost += 1;
            }
        }
        for w in self.walks.values_mut().flat_map(BTreeMap::values_mut) {
            if w.status == Status::Running {
                w.status = Status::Lost;
                w.error = Some(why.to_string());
                lost += 1;
            }
        }
        lost
    }

    /// Ticks the schedule allotted this attempt, once finished.
    ///
    /// **Not a measurement.** Both endpoints come from the schedule, so this is
    /// `Action::duration` travelling back out of the log, and comparing it to
    /// the estimate compares the estimate with itself. The measured counterpart
    /// is [`ExecutionLog::observed_duration`], which is `None` whenever the game
    /// did not report both ends.
    pub fn planned_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        a.planned_end_tick.map(|end| {
            debug_assert!(
                end >= a.planned_start_tick,
                "planned_end_tick {end} precedes planned_start_tick {}",
                a.planned_start_tick
            );
            end.saturating_sub(a.planned_start_tick)
        })
    }

    /// Ticks the game actually spent on this attempt, when it reported both
    /// ends.
    ///
    /// `None` — never zero, never the planned duration — if either observation
    /// is missing, because a duration derived from a fabricated endpoint is a
    /// fabricated duration. `None` also if the reply tick precedes the dispatch
    /// tick, which cannot happen in a game whose clock only advances and so
    /// means one of the two numbers is not what it claims to be.
    pub fn observed_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        let (start, end) = (a.dispatched_tick?, a.replied_tick?);
        end.checked_sub(start)
    }

    /// Records that a walk step has been dispatched.
    ///
    /// Takes only *plan* numbers, exactly like [`ExecutionLog::start`] — the
    /// game ticks arrive separately through [`ExecutionLog::observe_walk`], and
    /// keeping the two writers apart is what makes it impossible to fill a
    /// measured field from the schedule by slipping an argument.
    ///
    /// Overwrites any entry already under this key, and does so deliberately.
    /// `(bot, step_index)` identifies a walk *within one run*; a later
    /// `run_into` against the same log (a recovery proposal, say) carries a
    /// different schedule, in which the same index is a different walk. The
    /// current run's dispatch is the current fact for that slot, and keeping
    /// the superseded schedule's ticks under it would report the bot as having
    /// walked somewhere this run never sent it.
    pub fn start_walk(
        &mut self,
        bot: BotId,
        step_index: usize,
        to: Position,
        planned_start: Ticks,
        planned_end: Ticks,
    ) {
        self.walks.entry(bot).or_default().insert(
            step_index,
            WalkObservation {
                status: Status::Running,
                to,
                planned_start_tick: planned_start,
                planned_end_tick: planned_end,
                dispatched_tick: None,
                replied_tick: None,
                error: None,
                abandoned: None,
                stalls: None,
            },
        );
    }

    /// Records the stalls this walk survived, as the actuator reported them.
    ///
    /// Called by [`crate::run`]'s `run_walk` after every walk, success or
    /// failure, because **a stall the retry recovered from is otherwise
    /// invisible**: the walk settles `success` and nothing anywhere says the
    /// game had to be asked three times. See [`WalkObservation::stalls`].
    ///
    /// `Some(vec![])` is written when the actuator looked and found none, and
    /// this writer is simply not called when the actuator cannot answer. The
    /// two are different facts and stay different, exactly as `abandoned`'s
    /// `Some(0)` does.
    pub fn note_walk_stalls(&mut self, bot: BotId, step_index: usize, stalls: Vec<WalkStall>) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.stalls = Some(stalls);
        }
    }

    /// Records that this walk's failure halted its bot, abandoning `abandoned`
    /// of the bot's remaining steps.
    ///
    /// Called by [`crate::run`]'s `halt` immediately after `fail_walk`, so the
    /// count lands on the walk that caused it and reaches `events.jsonl`
    /// through the plumbing `record.walks` already has. See
    /// [`WalkObservation::abandoned`] for what the absence of this cost.
    ///
    /// `abandoned == 0` is written as `Some(0)`, not skipped: a bot halted on
    /// its own last step really did halt, and "halted, taking nothing with it"
    /// is a different fact from "did not halt". The same absent-is-not-a-value
    /// rule the rest of this record keeps.
    pub fn halt_walk(&mut self, bot: BotId, step_index: usize, abandoned: u32) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.abandoned = Some(abandoned);
        }
    }

    /// Records the game ticks observed for a walk. The counterpart of
    /// [`ExecutionLog::observe`], and it writes nothing but the two observed
    /// fields.
    ///
    /// Only a walk that was started is annotated: an observation with no
    /// dispatch to attach to would be an observation of nothing, and inventing
    /// an entry for it would put a walk in the log that the run never made.
    pub fn observe_walk(&mut self, bot: BotId, step_index: usize, ticks: ActionTicks) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.dispatched_tick = narrow(ticks.dispatched);
            w.replied_tick = narrow(ticks.replied);
        }
    }

    /// Marks a dispatched walk as arrived. Its ticks, if the game reported
    /// any, came from [`ExecutionLog::observe_walk`]; a success with none is a
    /// walk that happened unobserved, which is a different fact from a failure
    /// — see [`WalkObservation`].
    pub fn succeed_walk(&mut self, bot: BotId, step_index: usize) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.status = Status::Success;
        }
    }

    /// Marks a dispatched walk as failed, keeping the actuator's message.
    ///
    /// The ticks are left exactly as they are: whatever the game stamped
    /// reached them through [`ExecutionLog::observe_walk`], which the run
    /// calls on the failure path too now that
    /// [`crate::ActuatorFailure`] carries an observation. This writer adds
    /// nothing — and emphatically not the planned span sitting in the same
    /// record, which is not a substitute for a measurement.
    pub fn fail_walk(&mut self, bot: BotId, step_index: usize, error: String) {
        if let Some(w) = self.walk_mut(bot, step_index) {
            w.status = Status::Failed;
            w.error = Some(error);
        }
    }

    fn walk_mut(&mut self, bot: BotId, step_index: usize) -> Option<&mut WalkObservation> {
        self.walks.get_mut(&bot)?.get_mut(&step_index)
    }

    /// One walk, if this run made it.
    pub fn walk(&self, bot: BotId, step_index: usize) -> Option<&WalkObservation> {
        self.walks.get(&bot)?.get(&step_index)
    }

    /// Every walk, ascending by bot and then by step index.
    ///
    /// The order is a function of the log alone, so a consumer rendering a
    /// timeline gets the same sequence every time.
    pub fn walks(&self) -> impl Iterator<Item = (BotId, usize, &WalkObservation)> {
        self.walks
            .iter()
            .flat_map(|(bot, steps)| steps.iter().map(move |(i, w)| (*bot, *i, w)))
    }

    /// Ticks the game actually spent on a walk, when it reported both ends.
    ///
    /// `None` — never zero, never the planned span — under exactly the same
    /// conditions as [`ExecutionLog::observed_duration`].
    pub fn observed_walk_duration(&self, bot: BotId, step_index: usize) -> Option<Ticks> {
        let w = self.walk(bot, step_index)?;
        w.replied_tick?.checked_sub(w.dispatched_tick?)
    }

    /// Records that `key` has entered a wait, replacing any wait it was
    /// already in and **restarting its clock**.
    ///
    /// Restarting is the point: a step that moves from waiting on a
    /// predecessor to serving that predecessor's lag edge has genuinely
    /// entered a different state, and reporting the two as one span would say
    /// a bot had been "waiting on a reply" since before it was dispatched.
    /// [`Wait::elapsed`] therefore always answers "how long in *this* state".
    pub fn enter_wait(&mut self, key: WaitKey, bot: BotId, kind: WaitKind) {
        self.waiting.insert(
            key,
            WaitEntry {
                bot,
                kind,
                since: Instant::now(),
            },
        );
    }

    /// Records that `key` is no longer waiting. Idempotent: leaving a wait
    /// nobody entered is a no-op, which is what makes it safe to call from a
    /// `Drop`.
    pub fn leave_wait(&mut self, key: WaitKey) {
        self.waiting.remove(&key);
    }

    /// Every step currently waiting, ascending by key, longest wait first is
    /// **not** the order — the caller sorts, because different readers want
    /// different orders and a stable key order is the one property this can
    /// promise.
    ///
    /// `elapsed` is computed against `Instant::now()` at the moment of the
    /// call, so two entries in one answer are consistent with each other to
    /// within the time it takes to walk a `BTreeMap`.
    ///
    /// # What an empty answer means
    ///
    /// Nothing is waiting: every bot is either dispatching, between steps, or
    /// finished. It does **not** mean "nothing is known" — a run with
    /// `in_flight` greater than zero and no `reply` entry here is a
    /// contradiction, and one worth acting on, because it means this registry
    /// has stopped being maintained. That is the failure mode every other
    /// observability hole in this project has had, and it is detectable
    /// precisely because the two numbers come from different writers.
    pub fn waiting(&self) -> Vec<Wait> {
        let now = Instant::now();
        self.waiting
            .iter()
            .map(|(key, entry)| Wait {
                key: *key,
                bot: entry.bot,
                kind: entry.kind.clone(),
                elapsed: now.saturating_duration_since(entry.since),
            })
            .collect()
    }

    /// Failed action ids, in ascending id order.
    pub fn failed(&self) -> Vec<ActionId> {
        self.attempts
            .iter()
            .filter(|(_, a)| a.status == Status::Failed)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether the log knows nothing at all.
    ///
    /// Walks count. A run that dispatched a walk and got no further has
    /// observed something — where it sent the bot, and how that turned out —
    /// and reporting that as an empty log would hide the only record of it.
    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty() && self.walks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_planner::ActionId;

    fn id(n: u32) -> ActionId {
        ActionId(n)
    }

    #[test]
    fn an_unrecorded_action_is_pending() {
        let log = ExecutionLog::default();
        assert_eq!(log.status(id(1)), Status::Pending);
    }

    #[test]
    fn starting_then_finishing_records_a_duration() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        assert_eq!(log.status(id(1)), Status::Running);
        log.succeed(id(1), 340);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.planned_duration(id(1)), Some(240));
    }

    #[test]
    fn a_failure_keeps_its_message() {
        let mut log = ExecutionLog::default();
        log.start(id(7), 10);
        log.fail(id(7), 20, "cannot reach target".to_string());
        assert_eq!(log.status(id(7)), Status::Failed);
        assert_eq!(
            log.attempt(id(7)).and_then(|a| a.error.as_deref()),
            Some("cannot reach target")
        );
    }

    #[test]
    fn failed_actions_are_listed_in_id_order() {
        let mut log = ExecutionLog::default();
        for n in [9u32, 3, 5] {
            log.start(id(n), 0);
            log.fail(id(n), 1, "x".to_string());
        }
        assert_eq!(log.failed(), vec![id(3), id(5), id(9)]);
    }

    #[test]
    fn succeed_without_a_prior_start_still_records_success() {
        let mut log = ExecutionLog::default();
        log.succeed(id(2), 50);
        assert_eq!(log.status(id(2)), Status::Success);
    }

    #[test]
    fn fail_without_a_prior_start_still_records_the_message() {
        let mut log = ExecutionLog::default();
        log.fail(id(3), 15, "never started".to_string());
        assert_eq!(log.status(id(3)), Status::Failed);
        assert_eq!(
            log.attempt(id(3)).and_then(|a| a.error.as_deref()),
            Some("never started")
        );
    }

    #[test]
    fn a_second_completion_leaves_the_first_outcome_and_its_timing_alone() {
        // Reachable only from malformed input (one `ActionId` in two schedule
        // steps). It used to panic, and the panic unwound every other bot in
        // the same `join_all`.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        log.fail(id(1), 900, "a second runner finished it".to_string());
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.planned_duration(id(1)), Some(240));
        assert_eq!(log.attempt(id(1)).and_then(|a| a.error.as_deref()), None);
    }

    #[test]
    fn re_dispatching_a_succeeded_action_does_not_reopen_it() {
        // A duplicate dispatch, not a retry: re-running succeeded work is a
        // bug, and reopening the attempt would destroy the outcome recorded
        // for it.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        log.start(id(1), 1_000);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.planned_duration(id(1)), Some(240));
        assert_eq!(log.attempts(id(1)), 1, "a refused start is not an attempt");
    }

    #[test]
    fn a_retry_after_a_failure_supersedes_it_and_can_record_success() {
        // `recover` returns failed actions in its tier-1 proposal by design.
        // While `start` refused to reopen a finished attempt, the retry's
        // command reached the game but its outcome could never be written:
        // the action stayed `Failed`, `failed()` kept naming it, and the
        // caller kept being handed the same proposal — an unbounded retry
        // loop against a live server.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.fail(id(1), 60, "player was busy".to_string());
        assert_eq!(log.failed(), vec![id(1)]);

        log.start(id(1), 500);
        assert_eq!(
            log.status(id(1)),
            Status::Running,
            "a retry must reopen the attempt, or its outcome is unrecordable"
        );
        log.succeed(id(1), 560);

        assert_eq!(log.status(id(1)), Status::Success);
        assert!(
            log.failed().is_empty(),
            "a succeeded retry must stop being reported as a failure"
        );
        assert_eq!(
            log.planned_duration(id(1)),
            Some(60),
            "the retry's own span"
        );
        assert_eq!(
            log.attempt(id(1)).and_then(|a| a.error.as_deref()),
            None,
            "the superseded failure's message does not linger on a success"
        );
    }

    #[test]
    fn an_interrupted_attempt_is_counted_when_it_is_picked_up_again() {
        // D5. `Running` means the previous run died between dispatch and reply.
        // Starting the action again is a retry exactly like the `Failed` case,
        // and it is the case a retry budget is *most* for, because an
        // interrupted action never produces a verdict to escalate on. While
        // this counted flat, three interrupted runs left `attempts() == 1` and
        // no budget reading the counter could ever trip.
        let mut log = ExecutionLog::default();
        for round in 0..3 {
            log.start(id(1), 100 * round);
            assert_eq!(log.status(id(1)), Status::Running, "never finished");
        }
        assert_eq!(log.attempts(id(1)), 3);
    }

    #[test]
    fn the_attempt_counter_saturates_rather_than_wrapping_to_never_started() {
        // D6. `number + 1` panicked in debug and wrapped in release, and a
        // wrapped counter reports an action that has run four billion times as
        // never started — resetting every retry budget reading it.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.fail(id(1), 1, "x".to_string());
        // Reach the ceiling without four billion round trips.
        if let Some(a) = log.attempts.get_mut(&id(1)) {
            a.number = u32::MAX;
        }
        log.start(id(1), 2);
        assert_eq!(log.attempts(id(1)), u32::MAX, "saturated, not wrapped");
        assert_ne!(log.attempts(id(1)), 0, "a wrap would read as never started");
    }

    #[test]
    fn an_attempt_with_no_observation_reports_absent_ticks_not_zero_or_the_plan() {
        // The whole point of the pair. An actuator with no game clock, or a
        // dispatch the game never answered, must leave these empty -- and
        // emphatically not fall back to the planned numbers sitting right
        // beside them, which is the failure mode that makes a fabricated
        // measurement look real.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, None);
        assert_eq!(a.replied_tick, None);
        assert_ne!(a.dispatched_tick, Some(0), "absent is not tick zero");
        assert_ne!(
            a.dispatched_tick,
            Some(a.planned_start_tick),
            "absent must never fall back to the plan"
        );
        assert_eq!(log.observed_duration(id(1)), None);
        assert_eq!(
            log.planned_duration(id(1)),
            Some(240),
            "the estimate is unaffected by there being no measurement"
        );
    }

    #[test]
    fn an_observation_is_recorded_and_is_not_the_planned_tick() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), Some(70_240)));
        log.succeed(id(1), 340);

        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(70_000));
        assert_eq!(a.replied_tick, Some(70_240));
        // Both kinds survive side by side: the drift between them is the
        // signal, so neither may overwrite the other.
        assert_eq!(a.planned_start_tick, 100);
        assert_eq!(a.planned_end_tick, Some(340));
        assert_ne!(a.dispatched_tick, Some(a.planned_start_tick));
        assert_ne!(a.replied_tick, a.planned_end_tick);
        assert_eq!(log.observed_duration(id(1)), Some(240));
    }

    #[test]
    fn half_an_observation_yields_no_observed_duration() {
        // The game acknowledged the dispatch and then never reported an
        // outcome. The dispatch tick is real and is kept; the duration is not
        // derivable and must not be invented from the planned end.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), None));
        log.succeed(id(1), 340);
        assert_eq!(log.attempt(id(1)).unwrap().dispatched_tick, Some(70_000));
        assert_eq!(log.attempt(id(1)).unwrap().replied_tick, None);
        assert_eq!(log.observed_duration(id(1)), None);
    }

    #[test]
    fn a_tick_too_large_for_the_planners_width_is_absent_rather_than_wrapped() {
        // `game.tick` is 64-bit and `Ticks` is 32-bit. A wrapped value would
        // look like an ordinary early-game measurement and silently misplace
        // anything aligned to it, so it is dropped instead.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(
            id(1),
            ActionTicks::new(Some(u64::from(u32::MAX) + 1), Some(5)),
        );
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, None, "dropped, not wrapped to 0");
        assert_eq!(a.replied_tick, Some(5), "the tick that does fit is kept");
    }

    #[test]
    fn observing_an_action_that_was_never_started_records_nothing() {
        // An observation of nothing. Inventing an attempt to hang it on would
        // put an action in the log that the run never dispatched.
        let mut log = ExecutionLog::default();
        log.observe(id(1), ActionTicks::new(Some(1), Some(2)));
        assert!(log.attempt(id(1)).is_none());
        assert_eq!(log.status(id(1)), Status::Pending);
    }

    #[test]
    fn a_retry_observes_afresh_rather_than_inheriting_the_failed_attempts_ticks() {
        // The superseded attempt's measurement belongs to the superseded
        // attempt. Carrying it forward would report the retry as having been
        // dispatched before it was.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.observe(id(1), ActionTicks::new(Some(1_000), Some(1_060)));
        log.fail(id(1), 60, "player was busy".to_string());

        log.start(id(1), 500);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(
            a.dispatched_tick, None,
            "a reopened attempt has observed nothing yet"
        );
        assert_eq!(a.replied_tick, None);

        log.observe(id(1), ActionTicks::new(Some(9_000), Some(9_060)));
        log.succeed(id(1), 560);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(9_000), "the retry's own ticks");
        assert_eq!(a.replied_tick, Some(9_060));
    }

    #[test]
    fn a_second_observation_of_a_finished_attempt_is_ignored() {
        // Same argument as the second-completion guard: which numbers survive
        // must not depend on which duplicate writer the game answered first.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), Some(70_240)));
        log.succeed(id(1), 340);
        log.observe(id(1), ActionTicks::new(Some(1), Some(2)));
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(70_000));
        assert_eq!(a.replied_tick, Some(70_240));
    }

    #[test]
    fn a_retry_is_counted_so_a_caller_can_tell_one_attempt_from_three() {
        // The only record that a retry happened: `recover` is pure and keeps
        // no memory across rounds, so a caller's retry budget has nothing else
        // to read.
        let mut log = ExecutionLog::default();
        assert_eq!(log.attempts(id(1)), 0, "never started");
        log.start(id(1), 0);
        assert_eq!(log.attempts(id(1)), 1);
        for round in 1..3 {
            log.fail(id(1), 60 * round, "still busy".to_string());
            log.start(id(1), 100 * round);
        }
        assert_eq!(log.attempts(id(1)), 3);
        assert_eq!(log.status(id(1)), Status::Running);
    }

    // ------------------------------------------------- outstanding vs running

    #[test]
    fn an_attempt_whose_completion_could_not_be_read_is_lost_not_running() {
        // The case commit 0cb7636f created: the game answered, the answer
        // carried a status the parser could not read, and no completion was
        // recorded. `Running` would say "this bot is busy"; it is not, and
        // nobody is going to find out what happened.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), None));
        log.lose_track(id(1), "the game reported a status this run could not read");

        assert_eq!(log.status(id(1)), Status::Lost);
        assert_ne!(
            log.status(id(1)),
            Status::Running,
            "an action nobody is following is not an action in progress"
        );
        assert_ne!(
            log.status(id(1)),
            Status::Failed,
            "losing the thread is not a verdict"
        );
        assert!(
            log.failed().is_empty(),
            "recovery escalates on failures; a lost action has not failed"
        );
        assert_eq!(
            log.attempt(id(1)).and_then(|a| a.dispatched_tick),
            Some(70_000),
            "the tick the game did stamp survives losing the thread"
        );
        assert_eq!(
            log.attempt(id(1)).and_then(|a| a.error.as_deref()),
            Some("the game reported a status this run could not read"),
            "why the outcome is unknown, not a verdict"
        );
    }

    #[test]
    fn an_attempt_still_in_flight_is_running_not_lost() {
        // The other half of the distinction. If this and the test above cannot
        // fail independently, the two facts are still collapsed into one.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        assert_eq!(log.status(id(1)), Status::Running);
        assert_ne!(
            log.status(id(1)),
            Status::Lost,
            "a dispatch still in flight has not been lost track of"
        );
        assert_eq!(log.planned_duration(id(1)), None, "and has not finished");
    }

    #[test]
    fn losing_track_of_what_is_outstanding_touches_only_what_was_outstanding() {
        // What a session that stops following a run does. A finished attempt
        // has an outcome and keeps it; an action nothing ever dispatched was
        // never being followed in the first place.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.succeed(id(1), 10);
        log.start(id(2), 0);
        log.fail(id(2), 10, "no ore".to_string());
        log.start(id(3), 0); // still outstanding
        dispatch_walk(&mut log, bot(0), 0);
        log.succeed_walk(bot(0), 0);
        dispatch_walk(&mut log, bot(0), 1); // still outstanding

        let lost = log.lose_track_of_outstanding("the run ended before the game answered");

        assert_eq!(lost, 2, "one action and one walk were outstanding");
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.status(id(2)), Status::Failed);
        assert_eq!(log.status(id(3)), Status::Lost);
        assert_eq!(log.status(id(4)), Status::Pending, "never dispatched");
        assert_eq!(log.walk(bot(0), 0).expect("walk").status, Status::Success);
        assert_eq!(log.walk(bot(0), 1).expect("walk").status, Status::Lost);
        assert_eq!(
            log.attempt(id(2)).and_then(|a| a.error.as_deref()),
            Some("no ore"),
            "a recorded verdict is not overwritten by the sweep"
        );
    }

    #[test]
    fn picking_up_a_lost_attempt_again_counts_as_a_retry() {
        // Exactly the `Running` rule (D5): an attempt with no verdict is the
        // case a retry budget is most for, and a flat count could never trip
        // one.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.lose_track(id(1), "the run ended before the game answered");
        log.start(id(1), 500);
        assert_eq!(log.status(id(1)), Status::Running, "reopened");
        assert_eq!(log.attempts(id(1)), 2);
        assert_eq!(
            log.attempt(id(1)).and_then(|a| a.error.as_deref()),
            None,
            "the superseded attempt's note does not linger on the retry"
        );
    }

    #[test]
    fn a_lost_walk_is_distinguishable_from_a_failed_one_and_from_a_running_one() {
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.fail_walk(bot(0), 0, "path blocked".to_string());
        dispatch_walk(&mut log, bot(1), 0);
        log.lose_track_walk(bot(1), 0, "the run ended before the game answered");
        dispatch_walk(&mut log, bot(2), 0);

        assert_eq!(log.walk(bot(0), 0).expect("walk").status, Status::Failed);
        assert_eq!(log.walk(bot(1), 0).expect("walk").status, Status::Lost);
        assert_eq!(log.walk(bot(2), 0).expect("walk").status, Status::Running);
    }

    // ------------------------------------------------------------------ walks

    fn bot(n: u8) -> BotId {
        BotId(n)
    }

    fn to() -> Position {
        Position::new(10., 12.)
    }

    /// Dispatch a walk with a planned span deliberately far from any tick the
    /// tests below observe, so a measured field filled in from the plan is
    /// visible at a glance.
    fn dispatch_walk(log: &mut ExecutionLog, b: BotId, i: usize) {
        log.start_walk(b, i, to(), 100, 340);
    }

    #[test]
    fn a_walk_is_recorded_under_its_bot_and_step_index_with_no_action_id() {
        // The whole point: `(bot, step_index)` is enough. Nothing had to be
        // invented for the walk to be identifiable.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 3);
        log.observe_walk(bot(0), 3, ActionTicks::new(Some(70_000), Some(70_500)));
        log.succeed_walk(bot(0), 3);

        let w = log.walk(bot(0), 3).expect("walk");
        assert_eq!(w.status, Status::Success);
        assert_eq!(w.to, to());
        assert_eq!(w.dispatched_tick, Some(70_000));
        assert_eq!(w.replied_tick, Some(70_500));
        assert_eq!(log.observed_walk_duration(bot(0), 3), Some(500));
        // Nothing leaked into the neighbouring keys.
        assert!(log.walk(bot(0), 2).is_none());
        assert!(log.walk(bot(1), 3).is_none());
    }

    #[test]
    fn a_walks_observed_ticks_are_never_its_planned_ones() {
        // The measurement and the estimate live side by side and the drift
        // between them is the signal. A test that only asserted the fields
        // exist would pass against fields populated from the schedule.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(bot(0), 0, ActionTicks::new(Some(70_000), Some(70_500)));
        log.succeed_walk(bot(0), 0);

        let w = log.walk(bot(0), 0).expect("walk");
        assert_eq!(w.planned_start_tick, 100);
        assert_eq!(w.planned_end_tick, 340);
        assert_ne!(w.dispatched_tick, Some(w.planned_start_tick));
        assert_ne!(w.replied_tick, Some(w.planned_end_tick));
        assert_ne!(
            log.observed_walk_duration(bot(0), 0),
            Some(w.planned_end_tick - w.planned_start_tick),
            "the observed span is not the planned span round-tripped"
        );
    }

    #[test]
    fn an_unmeasured_walk_reports_absent_ticks_not_zero_and_not_the_plan() {
        // An actuator with no game clock. The walk happened; when is unknown.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(bot(0), 0, ActionTicks::UNKNOWN);
        log.succeed_walk(bot(0), 0);

        let w = log.walk(bot(0), 0).expect("walk");
        assert_eq!(w.dispatched_tick, None);
        assert_eq!(w.replied_tick, None);
        assert_ne!(w.dispatched_tick, Some(0), "absent is not tick zero");
        assert_ne!(
            w.dispatched_tick,
            Some(w.planned_start_tick),
            "absent must never fall back to the plan"
        );
        assert_eq!(log.observed_walk_duration(bot(0), 0), None);
    }

    #[test]
    fn a_failed_walk_is_distinguishable_from_an_unmeasured_one() {
        // Both carry `None` ticks, and they are different facts: one bot never
        // moved, the other moved unobserved. `status` is what separates them,
        // and nothing else can.
        let mut log = ExecutionLog::default();

        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(bot(0), 0, ActionTicks::UNKNOWN);
        log.succeed_walk(bot(0), 0);

        dispatch_walk(&mut log, bot(1), 0);
        log.fail_walk(bot(1), 0, "path blocked".to_string());

        let unmeasured = log.walk(bot(0), 0).expect("walk");
        let failed = log.walk(bot(1), 0).expect("walk");

        assert_eq!(unmeasured.dispatched_tick, failed.dispatched_tick);
        assert_eq!(unmeasured.replied_tick, failed.replied_tick);
        assert_ne!(
            unmeasured.status, failed.status,
            "the ticks cannot tell these apart, so the status must"
        );
        assert_eq!(unmeasured.status, Status::Success);
        assert_eq!(unmeasured.error, None);
        assert_eq!(failed.status, Status::Failed);
        assert_eq!(failed.error.as_deref(), Some("path blocked"));
    }

    #[test]
    fn a_failed_walk_does_not_borrow_the_planned_span_as_a_measurement() {
        // The failure path has no ticks to record and must not reach for the
        // planned ones sitting in the same record.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.fail_walk(bot(0), 0, "path blocked".to_string());
        let w = log.walk(bot(0), 0).expect("walk");
        assert_eq!(w.dispatched_tick, None);
        assert_eq!(w.replied_tick, None);
        assert_eq!(w.planned_start_tick, 100, "the estimate is still recorded");
    }

    #[test]
    fn an_interrupted_walk_is_neither_a_success_nor_a_failure() {
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        assert_eq!(log.walk(bot(0), 0).expect("walk").status, Status::Running);
    }

    #[test]
    fn walks_iterate_ascending_by_bot_then_step_index() {
        // Iteration order is part of the contract, exactly as it is for
        // `failed()`: a timeline built from this must not reshuffle between
        // runs of the same log.
        let mut log = ExecutionLog::default();
        for (b, i) in [(1u8, 4usize), (0, 9), (1, 0), (0, 2)] {
            dispatch_walk(&mut log, bot(b), i);
        }
        let seen: Vec<(u8, usize)> = log.walks().map(|(b, i, _)| (b.0, i)).collect();
        assert_eq!(seen, vec![(0, 2), (0, 9), (1, 0), (1, 4)]);
    }

    #[test]
    fn observing_a_walk_that_was_never_dispatched_records_nothing() {
        // An observation of nothing. Inventing an entry would put a walk in
        // the log that the run never made.
        let mut log = ExecutionLog::default();
        log.observe_walk(bot(0), 0, ActionTicks::new(Some(1), Some(2)));
        log.succeed_walk(bot(0), 0);
        log.fail_walk(bot(0), 0, "x".to_string());
        assert!(log.walk(bot(0), 0).is_none());
        assert_eq!(log.walks().count(), 0);
    }

    #[test]
    fn a_walk_tick_too_large_for_the_planners_width_is_absent_rather_than_wrapped() {
        // Same rule as an action's: a wrapped tick would look like an ordinary
        // early-game measurement.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(
            bot(0),
            0,
            ActionTicks::new(Some(u64::from(u32::MAX) + 1), Some(5)),
        );
        let w = log.walk(bot(0), 0).expect("walk");
        assert_eq!(w.dispatched_tick, None, "dropped, not wrapped to 0");
        assert_eq!(w.replied_tick, Some(5), "the tick that does fit is kept");
        assert_eq!(
            log.observed_walk_duration(bot(0), 0),
            None,
            "half an observation yields no duration"
        );
    }

    #[test]
    fn a_reply_tick_before_the_dispatch_tick_yields_no_duration() {
        // Cannot happen in a game whose clock only advances, so it means one
        // of the two numbers is not what it claims to be. Better no duration
        // than a wrapped one.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(bot(0), 0, ActionTicks::new(Some(500), Some(400)));
        assert_eq!(log.observed_walk_duration(bot(0), 0), None);
    }

    #[test]
    fn a_later_run_supersedes_the_walk_under_a_reused_key() {
        // `(bot, step_index)` identifies a walk within one run. A recovery
        // proposal is a different schedule, in which index 0 is a different
        // walk; carrying the old ticks forward would report the bot as having
        // walked somewhere this run never sent it.
        let mut log = ExecutionLog::default();
        dispatch_walk(&mut log, bot(0), 0);
        log.observe_walk(bot(0), 0, ActionTicks::new(Some(70_000), Some(70_500)));
        log.succeed_walk(bot(0), 0);

        log.start_walk(bot(0), 0, Position::new(-3., -4.), 900, 1_000);
        let w = log.walk(bot(0), 0).expect("walk");
        assert_eq!(w.status, Status::Running);
        assert_eq!(w.to, Position::new(-3., -4.));
        assert_eq!(
            w.dispatched_tick, None,
            "a re-dispatched walk has observed nothing yet"
        );
        assert_eq!(w.replied_tick, None);
    }

    #[test]
    fn a_log_holding_only_a_walk_is_not_empty() {
        // A run that dispatched a walk and got no further has observed
        // something, and reporting that as an empty log would hide the only
        // record of it.
        let mut log = ExecutionLog::default();
        assert!(log.is_empty());
        dispatch_walk(&mut log, bot(0), 0);
        log.fail_walk(bot(0), 0, "path blocked".to_string());
        assert!(!log.is_empty());
    }

    #[test]
    fn walks_and_attempts_do_not_disturb_each_other() {
        // Two maps, two key spaces. A walk at step 1 is not the action with
        // `ActionId(1)`.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(60_000), Some(60_100)));
        log.succeed(id(1), 340);
        dispatch_walk(&mut log, bot(0), 1);
        log.observe_walk(bot(0), 1, ActionTicks::new(Some(70_000), Some(70_500)));
        log.succeed_walk(bot(0), 1);

        assert_eq!(
            log.attempt(id(1)).expect("attempt").replied_tick,
            Some(60_100)
        );
        assert_eq!(
            log.walk(bot(0), 1).expect("walk").replied_tick,
            Some(70_500)
        );
        assert_eq!(log.failed(), vec![]);
    }
}
