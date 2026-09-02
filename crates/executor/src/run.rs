//! Run every bot's slice of a `Schedule` against an `Actuator`.
//!
//! Bots run concurrently and wait on each other through one
//! `tokio::sync::watch` channel per action, replacing the old executor's
//! 100 ms poll loop (`crates/core/src/plan/execute.rs:79`) with a completion
//! signal per action.

use crate::actuator::{ActionTicks, Actuator, ActuatorError, ActuatorFailure};
use crate::log::{ExecutionLog, Status};
use factorio_bot_core::petgraph::algo::toposort;
use factorio_bot_core::petgraph::graph::{DiGraph, NodeIndex};
use factorio_bot_planner::{
    ActionId, ActionKind, ActionNetwork, BotId, Condition, Schedule, ScheduledStep, StepKind, Ticks,
};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::watch;

enum PredOutcome {
    Ready,
    Abandoned,
}

/// Why a run refused to start.
///
/// Every variant is raised before a single command reaches the game, so a
/// caller holding one knows nothing was executed and the log it handed in is
/// untouched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error(
        "the schedule and the network imply a circular wait through action {0:?}: \
         it could never start, so the run would never finish"
    )]
    CircularWait(ActionId),
}

/// Run every bot, recording progress into `progress` as it happens.
///
/// The log is shared rather than merged at the end because `run:progress()`
/// must be able to read it mid-run. Keys are disjoint — each action belongs to
/// exactly one bot — so concurrent writers never collide on a key, and because
/// `ExecutionLog` is a `BTreeMap` the finished state is identical regardless of
/// the order the writes landed. That is what keeps this deterministic despite
/// being concurrent.
///
/// `std::sync::Mutex`, not tokio's: every critical section is a few map
/// operations with no `.await` inside. Never hold this guard across an await.
///
/// It fails only before doing anything, which is why the error carries no log:
/// the caller already owns `progress`, and on the error path it is still
/// empty, so handing it back inside the error would offer nothing to read.
pub async fn run_into(
    act: &dyn Actuator,
    sched: &Schedule,
    net: &ActionNetwork,
    progress: &Mutex<ExecutionLog>,
) -> Result<(), ExecutionError> {
    let steps = planned_steps(sched);
    check_wait_graph(net, &steps)?;

    let mut senders: BTreeMap<ActionId, watch::Sender<Status>> = BTreeMap::new();
    let mut receivers: BTreeMap<ActionId, watch::Receiver<Status>> = BTreeMap::new();
    for id in net.actions().map(|a| a.id) {
        let (tx, rx) = watch::channel(Status::Pending);
        senders.insert(id, tx);
        receivers.insert(id, rx);
    }

    // An action the network knows about but the schedule never assigns to any
    // bot will never be dispatched by anyone, so its signal has to be published
    // here or a dependent waits for it forever — the same argument as
    // `abandon_rest` below.
    //
    // *Which* signal comes from the log, not from the omission. An unassigned
    // action the log already records as `Success` is not a gap in the plan: it
    // is work that finished on an earlier run, and recovery deliberately leaves
    // it out of the new schedule while keeping it in the network so its lag
    // edges survive (see `Recovery::Rescheduled`). Publishing `Failed` for it
    // would abandon the very retry that was waiting behind it, and the run
    // would return `Ok(())` having dispatched nothing. Anything else unassigned
    // — never attempted, or attempted and failed — is a genuine gap and still
    // releases waiters as `Failed`.
    //
    // Nothing is written to the log here either way: we attempted nothing in
    // this run, so a never-attempted action stays `Pending`.
    //
    // This is why a recovery proposal must be run through `run_into` with the
    // log recovery was given, not through `run`, which starts an empty one.
    let scheduled: BTreeSet<ActionId> = steps.iter().filter_map(|s| act_id(s)).collect();
    let already_done: BTreeSet<ActionId> = {
        let log = lock(progress);
        senders
            .keys()
            .copied()
            .filter(|id| log.status(*id) == Status::Success)
            .collect()
    };
    for (id, tx) in &senders {
        if !scheduled.contains(id) {
            let _ = tx.send(if already_done.contains(id) {
                Status::Success
            } else {
                Status::Failed
            });
        }
    }

    // Everything below this line is the dispatching part of the run, and it is
    // the part that can stop without finishing: this future may be dropped
    // (a cancelled script, an aborted task, a runtime shutting down) or a bot's
    // future may panic and unwind through `join_all`. Either way the log is
    // left holding dispatches nobody will ever hear back about, and reporting
    // those as `Running` shows busy bots to a session that has lost the thread.
    // The guard's `Drop` runs on all three exits — normal, dropped, unwinding —
    // and on the normal one it finds nothing outstanding and does nothing.
    let _outstanding = LoseTrackOnDrop(progress);

    // BTreeSet, so the future order is a function of the schedule alone, not of
    // task completion timing.
    let bots: BTreeSet<BotId> = steps.iter().map(|s| s.bot).collect();
    join_all(
        bots.iter()
            .map(|&bot| run_bot_signalled(act, bot, &steps, net, progress, &senders, &receivers)),
    )
    .await;
    Ok(())
}

/// Why an attempt outstanding when the run stopped is marked
/// [`Status::Lost`].
///
/// Phrased for a reader of the log, who may be looking at it long after the
/// session that wrote it is gone.
const RUN_ENDED_OUTSTANDING: &str = "the run ended before the game reported an outcome";

/// Turns whatever is still in flight into [`Status::Lost`] when the run stops
/// driving it, however it stops.
///
/// A guard rather than a line after `join_all` because the two exits that
/// matter never reach such a line: a dropped future stops being polled, and a
/// panicking bot unwinds straight through `join_all`. Both are exactly the
/// cases where the log is left claiming a bot is busy.
struct LoseTrackOnDrop<'a>(&'a Mutex<ExecutionLog>);

impl Drop for LoseTrackOnDrop<'_> {
    fn drop(&mut self) {
        // No `.await` here and none possible: `lose_track_of_outstanding` is a
        // couple of map walks, so taking the log guard in `Drop` is safe even
        // while unwinding (`lock` recovers a poisoned mutex).
        lock(self.0).lose_track_of_outstanding(RUN_ENDED_OUTSTANDING);
    }
}

fn act_id(step: &ScheduledStep) -> Option<ActionId> {
    match &step.what {
        StepKind::Act { action, .. } => Some(*action),
        StepKind::Walk { .. } => None,
    }
}

/// The steps the run will actually execute, in schedule order.
///
/// A `Schedule` naming the same `ActionId` in two steps is malformed: the
/// action was meant to happen once. Running it twice would send the command to
/// the game twice and give the log two completions for one key, and which of
/// them the log ended up reporting would depend on which the game answered
/// first. Keeping the first occurrence and dropping the rest makes both the
/// dispatches and the log a function of the schedule alone. Walk steps carry
/// no id and are never dropped.
fn planned_steps(sched: &Schedule) -> Vec<&ScheduledStep> {
    let mut seen: BTreeSet<ActionId> = BTreeSet::new();
    sched
        .steps
        .iter()
        .filter(|s| match act_id(s) {
            Some(action) => seen.insert(action),
            None => true,
        })
        .collect()
}

/// Refuse a schedule whose waits can never all be satisfied.
///
/// A bot's steps run strictly in order, so the schedule contributes wait edges
/// the `ActionNetwork` knows nothing about: each of a bot's actions waits on
/// the one scheduled before it. What decides whether the run terminates is the
/// union of the two edge sets, and only the union.
///
/// Checking the network alone is false comfort. A network holding `1 -> 0` is
/// perfectly acyclic, yet a bot scheduled to run `0` then `1` waits on itself
/// forever. The same deadlock reaches across bots with no network cycle at all:
/// bot 0 running `[a, b]` and bot 1 running `[c, d]` hangs the moment the
/// network carries `d -> a` and `b -> c`. A check that passes while the hang
/// remains is worse than no check, because it reads as protection.
///
/// Only *scheduled* actions are nodes. An action the network holds but nobody
/// is scheduled to run has its signal published before the run starts —
/// `Success` if the log already records it, `Failed` otherwise — so a wait on
/// it resolves at once either way and it can never be part of a circular wait.
/// Including it would reject schedules that in fact terminate.
fn check_wait_graph(net: &ActionNetwork, steps: &[&ScheduledStep]) -> Result<(), ExecutionError> {
    let mut graph: DiGraph<ActionId, ()> = DiGraph::new();
    let mut node: BTreeMap<ActionId, NodeIndex> = BTreeMap::new();
    for id in steps.iter().filter_map(|s| act_id(s)) {
        let ix = graph.add_node(id);
        node.insert(id, ix);
    }

    for (&id, &to) in &node {
        for (pred, _lag) in net.preds(id) {
            if let Some(&from) = node.get(&pred) {
                graph.add_edge(from, to, ());
            }
        }
    }

    // The edges only the schedule knows: consecutive actions of one bot.
    let mut previous: BTreeMap<BotId, NodeIndex> = BTreeMap::new();
    for step in steps {
        let Some(id) = act_id(step) else { continue };
        let ix = node[&id];
        if let Some(before) = previous.insert(step.bot, ix) {
            graph.add_edge(before, ix, ());
        }
    }

    match toposort(&graph, None) {
        Ok(_) => Ok(()),
        Err(cycle) => Err(ExecutionError::CircularWait(graph[cycle.node_id()])),
    }
}

/// Convenience wrapper for callers that only want the final state.
///
/// **Not for recovery proposals.** This starts an empty `ExecutionLog`, and
/// `run_into` reads the log to decide what to publish for the actions its
/// schedule does not assign. A `Recovery::Rescheduled` network deliberately
/// keeps already-succeeded actions as nodes so their lag edges survive; against
/// an empty log they read as never-attempted, get published `Failed`, and
/// abandon the retries waiting behind them. Run a recovery proposal with
/// `run_into` and the log recovery was given.
pub async fn run(
    act: &dyn Actuator,
    sched: &Schedule,
    net: &ActionNetwork,
) -> Result<ExecutionLog, ExecutionError> {
    let progress = Mutex::new(ExecutionLog::default());
    run_into(act, sched, net, &progress).await?;
    Ok(progress.into_inner().unwrap_or_else(|e| e.into_inner()))
}

/// Walk one bot's slice of the schedule, in order, waiting on the completion
/// signal of every predecessor before each action.
///
/// Stops that bot at its first failure: later steps in a chain depend on
/// earlier ones, and pressing on would issue commands whose preconditions the
/// game no longer satisfies. Every stop publishes `Failed` for the steps it
/// will now never reach, so waiters elsewhere are released.
async fn run_bot_signalled(
    act: &dyn Actuator,
    bot: BotId,
    steps: &[&ScheduledStep],
    net: &ActionNetwork,
    log: &Mutex<ExecutionLog>,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) {
    let mine: Vec<&ScheduledStep> = steps.iter().copied().filter(|s| s.bot == bot).collect();

    for (i, step) in mine.iter().enumerate() {
        match &step.what {
            StepKind::Walk { to, radius } => {
                // A walk needs no `ActionId` to be recorded. This loop is one
                // bot's steps in schedule order, so `(bot, i)` is already a
                // unique, stable key — the ticks the actuator has always
                // measured here now have somewhere to go. Walking is most of
                // the wall-clock in these plans, so dropping them was the
                // biggest hole in the timeline.
                lock(log).start_walk(bot, i, to.clone(), step.start, step.end);
                match act.walk(bot, to.clone(), *radius).await {
                    Ok(ticks) => {
                        // Same order and same reasoning as the action arm:
                        // the observation, then the outcome.
                        let mut log = lock(log);
                        log.observe_walk(bot, i, ticks);
                        log.succeed_walk(bot, i);
                    }
                    Err(f) => {
                        // Whatever the game stamped before this went wrong is
                        // recorded first, exactly as on the success path: a
                        // walk the game acknowledged and then refused really
                        // was dispatched at a tick, and dropping that number
                        // would make it indistinguishable from a walk the game
                        // never saw.
                        //
                        // The entry's `status` is then what keeps a walk that
                        // did not happen distinguishable from one that happened
                        // unobserved — and from one whose outcome the game
                        // never reported, which is neither.
                        {
                            let mut log = lock(log);
                            log.observe_walk(bot, i, f.ticks);
                            match &f.error {
                                ActuatorError::NoVerdict(_) => {
                                    log.lose_track_walk(bot, i, &f.to_string());
                                }
                                _ => log.fail_walk(bot, i, f.to_string()),
                            }
                        }
                        abandon_rest(&mine[i..], senders);
                        return;
                    }
                }
            }
            StepKind::Act { action, .. } => {
                if let PredOutcome::Abandoned = await_preds(act, net, *action, receivers).await {
                    abandon_rest(&mine[i..], senders);
                    return;
                }
                lock(log).start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    lock(log).fail(*action, step.start, "action not in network".to_string());
                    abandon_rest(&mine[i..], senders);
                    return;
                };
                // `perform` awaits, so the guard is taken and dropped around
                // it, never held across it.
                match perform(act, bot, &a.kind).await {
                    Ok(ticks) => {
                        // Observation first, then the plan-side outcome: both
                        // writes are under the same guard as far as any reader
                        // is concerned, and `succeed` is what marks the attempt
                        // finished, after which `observe` would refuse.
                        {
                            let mut log = lock(log);
                            log.observe(*action, ticks);
                            log.succeed(*action, step.end);
                            // Drained here, not inside `perform`/`place`
                            // itself: this scope is the first place with both
                            // the actuator and the scheduler's `ActionId` for
                            // what just finished, which is exactly what
                            // `Actuator::take_placement` needs to attach the
                            // fact to. A placement waits in the actuator at
                            // most this long -- a bot's own steps run
                            // strictly in order, so nothing can queue a
                            // second one behind it before it is claimed.
                            if let Some(placement) = act.take_placement(bot) {
                                log.record_placement(*action, placement);
                            }
                        }
                        if let Some(tx) = senders.get(action) {
                            let _ = tx.send(Status::Success);
                        }
                    }
                    Err(f) => {
                        // The observation first, same order as the success
                        // path and for the same reason. `f.ticks` is what the
                        // game had stamped before it went wrong — often
                        // nothing, sometimes a real dispatch tick — and it is
                        // the only number allowed anywhere near these fields.
                        // What is *not* allowed is the planned tick sitting in
                        // the same record.
                        //
                        // A verdict of failure and no verdict at all are then
                        // different facts and are recorded as different states:
                        // `Failed` says the game judged this and the judgement
                        // was no, `Lost` says nobody will ever know. Recovery
                        // counts the first towards its escalation budget and
                        // not the second, which is the whole reason the two
                        // must not be collapsed here.
                        let lost = matches!(f.error, ActuatorError::NoVerdict(_));
                        {
                            let mut log = lock(log);
                            log.observe(*action, f.ticks);
                            if lost {
                                log.lose_track(*action, &f.to_string());
                            } else {
                                log.fail(*action, step.end, f.to_string());
                            }
                        }
                        // The waiters are released either way — a dependent
                        // cannot run on a precondition nobody can vouch for —
                        // but they are told *which* it was. `abandon_rest`
                        // starts one past this step, because this step's own
                        // signal has just been published with the truth.
                        if let Some(tx) = senders.get(action) {
                            let _ = tx.send(if lost { Status::Lost } else { Status::Failed });
                        }
                        abandon_rest(&mine[i + 1..], senders);
                        return;
                    }
                }
            }
        }
    }
}

/// One place to take the log guard, so poisoning is handled identically
/// everywhere. A panic mid-run should not turn every later write into a second
/// panic; the log is observational, so recovering the inner value is right.
fn lock(log: &Mutex<ExecutionLog>) -> std::sync::MutexGuard<'_, ExecutionLog> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

/// Publish `Failed` for every action this bot will now never reach.
///
/// Without this the executor deadlocks: a bot that stops early leaves its
/// remaining actions at `Pending` forever, and any bot waiting on one of them
/// waits forever too. Abandonment has to propagate for the run to terminate.
fn abandon_rest(rest: &[&ScheduledStep], senders: &BTreeMap<ActionId, watch::Sender<Status>>) {
    for step in rest {
        if let StepKind::Act { action, .. } = &step.what
            && let Some(tx) = senders.get(action)
        {
            let _ = tx.send(Status::Failed);
        }
    }
}

async fn await_preds(
    act: &dyn Actuator,
    net: &ActionNetwork,
    id: ActionId,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) -> PredOutcome {
    let mut max_lag: Ticks = 0;
    for (pred, lag) in net.preds(id) {
        let Some(rx) = receivers.get(&pred) else {
            continue;
        };
        let mut rx = rx.clone();
        loop {
            // Bind by value so the watch borrow is dropped before the await.
            let status = *rx.borrow_and_update();
            match status {
                Status::Success => break,
                // A predecessor whose outcome nobody knows is no basis for
                // dispatching what depends on it: `Lost` is not `Failed`, but
                // it is just as much a reason not to proceed, because the
                // precondition this action needs is unvouched for either way.
                Status::Failed | Status::Lost => return PredOutcome::Abandoned,
                Status::Pending | Status::Running => {}
            }
            if rx.changed().await.is_err() {
                return PredOutcome::Abandoned;
            }
        }
        max_lag = max_lag.max(lag);
    }

    // A lag edge is machine time, not bot time: the furnace keeps working after
    // the bot walks away, and the plate is not there until it has. The
    // predecessor's own completion signal does not cover that wait, so honour
    // the lag once every predecessor has succeeded.
    if max_lag > 0 {
        wait_out_lag(act, max_lag).await;
    }
    // A predecessor's success is not the same fact as its *effect* having
    // landed. See `await_research`.
    await_research(act, net, id).await;
    PredOutcome::Ready
}

/// How long a dispatch may wait for a technology its own plan unlocks.
///
/// The one measurement there is says 16 game ticks — ~0.27 s at 60 UPS, more on
/// a server running behind, which this one always is. Five seconds is that with
/// an order of magnitude of headroom, and is still short enough that a
/// technology which is genuinely never coming costs one wait rather than a
/// stalled run.
const RESEARCH_SETTLE_BUDGET: Duration = Duration::from_secs(5);

/// How often to re-ask while waiting out [`RESEARCH_SETTLE_BUDGET`].
///
/// The question is a read of the world map the mod's stdout already fills in,
/// not a round trip, so polling is cheap; this is only the granularity of the
/// answer.
const RESEARCH_POLL: Duration = Duration::from_millis(100);

/// Wait for the technologies this action's own preconditions name.
///
/// # Why an edge is not enough
///
/// `Condition::Researched(tech)` on a craft is turned by
/// `ActionNetwork::infer_edges` into an edge from whichever action carries the
/// matching `Effect::Researched` — normally the craft of the item a Factorio
/// 2.0 `craft-item` trigger technology watches for. The edge is right and
/// `await_preds` honours it. What it cannot say is that **the game applies the
/// unlock some ticks after the craft that triggers it**, because that is not a
/// fact about the plan.
///
/// `run-1788365280-15443` is the measurement. The plan was correct — milestone
/// 6's `craft 10 automation-science-pack` carried `deps: [4, 18, 22]` with 18
/// being `craft 1 lab` — and the executor waited: the same edge in the next
/// iteration dispatched four ticks after the lab settled. But
/// `workspace/server-log.txt` has `§53469§action_completed§ok 44` (the lab) and
/// `§53485§on_research_finished§` (the technology), **16 ticks apart**. The
/// dependent craft landed inside that window and the mod answered "recipe
/// automation-science-pack is not enabled for this force", which abandoned the
/// iteration and cost 27,462 ticks of work that was then done again.
///
/// So the precondition is *checked* rather than inferred from a predecessor's
/// success. This is the same discipline as [`wait_out_lag`], for the same kind
/// of reason: the plan's ordering describes when work may start, not when the
/// world has caught up with it.
///
/// # What it will not do
///
/// It will not wait forever, and it will not fail the action. An actuator that
/// cannot answer ([`Actuator::technology_researched`] returning `Ok(None)`) or
/// that errors is not waited on at all, and a budget spent without an answer
/// dispatches anyway — the action's own verdict is then the report, which is a
/// far better failure than a bot that never moves again. Same trade, and the
/// same wording, as [`LAG_CHASE_BUDGET`].
async fn await_research(act: &dyn Actuator, net: &ActionNetwork, id: ActionId) {
    let Some(action) = net.action(id) else {
        return;
    };
    for tech in action.pre.iter().filter_map(|c| match c {
        Condition::Researched(tech) => Some(tech.as_str()),
        _ => None,
    }) {
        let deadline = tokio::time::Instant::now() + RESEARCH_SETTLE_BUDGET;
        loop {
            match act.technology_researched(tech).await {
                // Researched, or nobody can say. Neither is a reason to wait:
                // `Ok(None)` is "this actuator has no answer", and waiting on
                // it would spend the whole budget for something that is never
                // going to change.
                Ok(Some(true)) | Ok(None) | Err(_) => break,
                Ok(Some(false)) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(RESEARCH_POLL).await;
        }
    }
}

/// How much *extra* wall clock a lag wait may spend chasing the game's clock,
/// as a multiple of the wait it first estimated.
///
/// The loop below re-reads the game tick and sleeps again for whatever is
/// still owed, which converges fast against a server merely running behind —
/// a 10% deficit is gone in two extra readings. It does not converge at all
/// against a game that is *stopped*: `/editor`, a paused single-player host,
/// a server between saves. That case must not hang the run, so the budget
/// caps it. Spending the budget and dispatching anyway is deliberate: the
/// action's own verdict is then the report, which is a far better failure than
/// a bot that never moves again.
const LAG_CHASE_BUDGET: u32 = 1;

/// Wait for `lag` ticks of machine time to actually pass.
///
/// # The clock this reads, and the one it does not
///
/// `lag` counts *game* ticks. The obvious implementation — divide by 60,
/// sleep that long — silently reinterprets it as a wall-clock duration, and
/// the two are only equal on a server that is keeping up. One that is not
/// hands back a wait that is short by exactly the fraction it is behind, and
/// nothing says so: an early collection from a furnace looks identical to a
/// furnace that was slow.
///
/// `run-1788320177-77989` is what that costs. A 4032-tick lag before taking 20
/// iron plates was slept as 67.2 s; ~3599 ticks passed (six 1920x1080
/// screenshots every 300 ticks will do that); the furnace had made 18; the
/// take failed and rung 4 spent the rest of its budget replanning. The
/// planner's one-cycle headroom (192 ticks, 4.8%) could not cover a ~10.7%
/// deficit, and the deficit scales with the batch while the headroom does not
/// — which is why the same run's earlier, smaller smelts all came back clean.
///
/// So: ask the game where its clock is, sleep the *estimated* remaining time,
/// then ask again. The wall clock survives only as the estimate for how long
/// to sleep between readings, where being wrong costs an extra round trip
/// instead of a plan.
///
/// An actuator with no clock ([`Actuator::game_tick`] returning `None`) keeps
/// the old wall-clock wait, which is the honest fallback: it is the best
/// available claim when nobody can be asked what time it is.
async fn wait_out_lag(act: &dyn Actuator, lag: Ticks) {
    // A speed the actuator cannot report falls back to normal speed rather
    // than aborting the run over a missing nicety: a wrong-but-finite wait is
    // recoverable (recovery re-checks preconditions before dispatching the
    // next action), a hung run is not. It is only an estimate now either way.
    let speed = act.game_speed().await.unwrap_or(1.0);
    let Some(started) = act.game_tick().await.ok().flatten() else {
        tokio::time::sleep(ticks_to_wall_clock(lag, speed)).await;
        return;
    };
    let deadline = started.saturating_add(u64::from(lag));

    let mut estimate = lag;
    let mut budget = lag.saturating_mul(LAG_CHASE_BUDGET);
    loop {
        tokio::time::sleep(ticks_to_wall_clock(estimate, speed)).await;
        // A clock that stops answering mid-wait leaves us with the wait we
        // already did and no way to check it. Returning is right: we have
        // slept at least the estimate, which is what the old code did on its
        // own, and inventing a further wait on no evidence would be worse.
        let Some(now) = act.game_tick().await.ok().flatten() else {
            return;
        };
        let owed = deadline.saturating_sub(now);
        if owed == 0 {
            return;
        }
        // Saturating rather than wrapping: `owed` cannot exceed `lag` in
        // practice (the clock only moves forward), and if a game somehow
        // rewound its tick the right answer is still "wait what is left of the
        // budget", not "wait 4 billion ticks".
        estimate = u32::try_from(owed).unwrap_or(Ticks::MAX).min(budget);
        if estimate == 0 {
            return;
        }
        budget -= estimate;
    }
}

/// Factorio *aims* to run at `60 * speed` ticks per second; `speed` is
/// `game.speed` (`Actuator::game_speed`), where `1.0` is normal. A non-default
/// speed scales how fast machine time passes without changing how many ticks a
/// lag edge represents, so wall-clock time is `ticks / (60 * speed)`.
///
/// **An estimate, not a measurement.** A server that cannot keep up delivers
/// fewer ticks per second than this says and reports nothing about it, so
/// [`wait_out_lag`] uses this only to decide how long to sleep before asking
/// the game's clock again — never as the answer to "has the machine had its
/// ticks yet". See [`Actuator::game_tick`] for what that cost once.
///
/// `speed` is defensively floored to normal rather than trusted blindly:
/// Factorio's own minimum is `0.01`, but an actuator stub or a future bug
/// could still hand this `0.0` or negative, and dividing by that would sleep
/// forever or panic rather than degrade to the old (wrong-but-bounded)
/// behaviour.
fn ticks_to_wall_clock(ticks: Ticks, speed: f64) -> std::time::Duration {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    std::time::Duration::from_secs_f64(f64::from(ticks) / (60.0 * speed))
}

/// Dispatches one action and hands back the game ticks the actuator observed
/// for it.
async fn perform<A: Actuator + ?Sized>(
    act: &A,
    bot: BotId,
    kind: &ActionKind,
) -> Result<ActionTicks, ActuatorFailure> {
    match kind {
        ActionKind::Mine { pos, item, count } => {
            act.mine(bot, item.as_str(), pos.clone(), *count).await
        }
        ActionKind::Craft { item, count } => act.craft(bot, item.as_str(), *count).await,
        ActionKind::Place { entity } => {
            act.place(bot, &entity.name, entity.position.clone(), entity.direction)
                .await
        }
        ActionKind::Insert {
            pos,
            entity,
            slot,
            item,
            count,
        } => {
            act.insert(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Remove {
            pos,
            entity,
            slot,
            item,
            count,
        } => {
            act.remove(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Research { tech } => act.research(tech).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::record::map::{EntitySnapshot, Placement};
    use factorio_bot_core::types::{FactorioEntity, Position};
    use factorio_bot_planner::{Action, Actor, Condition, Effect, InventorySlot};
    use mockall::mock;
    use std::time::Duration;

    mock! {
        pub Act {}
        #[async_trait::async_trait]
        impl Actuator for Act {
            async fn walk(&self, bot: BotId, to: Position, radius: f64) -> Result<ActionTicks, ActuatorFailure>;
            async fn mine(&self, bot: BotId, item: &str, at: Position, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn place(&self, bot: BotId, item: &str, at: Position, direction: u8) -> Result<ActionTicks, ActuatorFailure>;
            async fn insert(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn remove(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn research(&self, tech: &str) -> Result<ActionTicks, ActuatorFailure>;
        }
    }

    /// The tolerance `walk_then_mine_fixture`'s walk carries, and the one its
    /// mine's `AtPosition` asks for. Deliberately not 1.0 — Factorio's own
    /// default path radius — so a walk dispatched with the radius thrown away
    /// is distinguishable from one dispatched with the plan's.
    const WALK_FIXTURE_RADIUS: f64 = 3.0;

    /// The tick pair a mocked dispatch reports when a test does not care which
    /// numbers come back. Deliberately not zero and deliberately far from any
    /// tick these fixtures schedule, so a test that *does* care can tell an
    /// observation from a plan value at a glance.
    fn some_ticks() -> ActionTicks {
        ActionTicks::new(Some(900_001), Some(900_002))
    }

    // ---------------------------------------------------------------- fixtures

    fn mine_action_id() -> ActionId {
        ActionId(0)
    }

    /// The action scheduled right after the mine in `walk_then_mine_fixture`.
    /// Exists so `a_failed_step_is_logged_and_stops_that_bot` has something
    /// to prove was *never attempted* — a fixture whose failing step is last
    /// cannot distinguish "stops" from "there was nothing left to do".
    fn craft_action_id() -> ActionId {
        ActionId(1)
    }

    fn mine_of(id: ActionId, item: &str) -> Action {
        Action {
            id,
            kind: ActionKind::Mine {
                pos: Position::new(10., 10.),
                item: item.into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: Position::new(10., 10.),
                radius: WALK_FIXTURE_RADIUS,
                min_radius: 0.0,
            }],
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: item.into(),
                count: 1,
            }],
            duration: 60,
            pinned: None,
            label: format!("mine 1 {item}"),
        }
    }

    fn act_step(action: ActionId, bot: BotId, start: Ticks, end: Ticks) -> ScheduledStep {
        ScheduledStep {
            what: StepKind::Act {
                action,
                label: format!("act {action:?}"),
            },
            bot,
            start,
            end,
        }
    }

    fn walk_step(bot: BotId, start: Ticks, end: Ticks) -> ScheduledStep {
        ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(10., 10.),
                radius: WALK_FIXTURE_RADIUS,
            },
            bot,
            start,
            end,
        }
    }

    /// A two-action network (`Mine` then `Craft`) preceded by a `Walk` step,
    /// all for `BotId(0)`.
    fn walk_then_mine_fixture() -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        net.add(mine_of(mine_action_id(), "iron-ore"));
        net.add(Action {
            id: craft_action_id(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "craft 1 iron-gear-wheel".into(),
        });

        let sched = Schedule {
            steps: vec![
                walk_step(BotId(0), 0, 60),
                act_step(mine_action_id(), BotId(0), 60, 120),
                act_step(craft_action_id(), BotId(0), 120, 150),
            ],
            makespan: 150,
        };
        (net, sched)
    }

    fn first_action_id() -> ActionId {
        ActionId(0)
    }

    fn second_action_id() -> ActionId {
        ActionId(1)
    }

    /// Action 0 mines iron-ore on bot 0; action 1 mines copper-ore on bot 1 and
    /// has an ordering edge from action 0. Each bot walks first, so a bot can
    /// be delayed before its action without delaying the action's dispatch
    /// record.
    fn cross_bot_fixture(lag: Ticks) -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        net.link(first_action_id(), second_action_id(), lag);

        let sched = Schedule {
            steps: vec![
                walk_step(BotId(0), 0, 60),
                act_step(first_action_id(), BotId(0), 60, 120),
                walk_step(BotId(1), 0, 60),
                act_step(second_action_id(), BotId(1), 120, 180),
            ],
            makespan: 180,
        };
        (net, sched)
    }

    /// The shape run 30's milestone 6 had: a craft whose recipe a technology
    /// unlocks, ordered behind the craft that triggers that technology.
    ///
    /// `craft 1 lab` carries `Effect::Researched`, `craft 10
    /// automation-science-pack` carries the matching `Condition::Researched`,
    /// and an edge joins them — exactly what `21a1228a` made the planner emit
    /// and what `events.jsonl` recorded as `deps: [4, 18, 22]`. Both crafts run
    /// on one bot, because that is what the run did and because it removes any
    /// question of cross-bot timing from the assertion.
    fn trigger_unlock_fixture() -> (ActionNetwork, Schedule) {
        const TECH: &str = "automation-science-pack";
        let mut net = ActionNetwork::new();
        net.add(Action {
            id: first_action_id(),
            kind: ActionKind::Craft {
                item: "lab".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![Effect::Researched(TECH.into())],
            duration: 120,
            pinned: None,
            label: "craft 1 lab".into(),
        });
        net.add(Action {
            id: second_action_id(),
            kind: ActionKind::Craft {
                item: TECH.into(),
                count: 10,
            },
            pre: vec![Condition::Researched(TECH.into())],
            eff: vec![],
            duration: 3000,
            pinned: None,
            label: format!("craft 10 {TECH}"),
        });
        net.link(first_action_id(), second_action_id(), 0);

        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 120),
                act_step(second_action_id(), BotId(0), 120, 3120),
            ],
            makespan: 3120,
        };
        (net, sched)
    }

    // ------------------------------------------------------- recording actuator

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Dispatch {
        Walk(BotId),
        MineStart(String),
        MineEnd(String),
    }

    /// What the actuator should do, keyed so a test can reverse the timing of
    /// two bots without touching anything else.
    #[derive(Clone)]
    struct Script {
        walk_delay_ms: BTreeMap<BotId, u64>,
        mine_delay_ms: BTreeMap<String, u64>,
        fail_walk: BTreeSet<BotId>,
        fail_mine: BTreeSet<String>,
        /// What `RecordingAct::game_speed` reports. Defaults to `1.0`, not the
        /// derived `f64` default of `0.0` — a script nobody configures must
        /// behave exactly like normal speed, the same as every test written
        /// before this field existed.
        speed: f64,
        /// What `RecordingAct::take_placement` hands back, once, to whichever
        /// bot asks first. `None` for every test written before this field
        /// existed, matching `Actuator::take_placement`'s own default.
        placement: Option<Placement>,
        /// How many game ticks this actuator's clock advances per second of
        /// (virtual) wall clock, or `None` for an actuator with no clock.
        ///
        /// `None` by default, matching `Actuator::game_tick`'s own default, so
        /// every test written before this field existed keeps taking the
        /// wall-clock path. A value **below 60** is a server running behind —
        /// the case that broke `run-1788320177-77989` — and `Some(0.0)` is a
        /// clock that has stopped.
        ticks_per_second: Option<f64>,
        /// How many times `technology_researched` must be asked before it
        /// answers `Some(true)`; every earlier ask answers `Some(false)`.
        ///
        /// `None` — the default, matching `Actuator::technology_researched`'s
        /// own — answers `Ok(None)`: *this actuator cannot say*. That is what
        /// every test written before this field existed gets, and it is what
        /// keeps them dispatching without a wait.
        ///
        /// `Some(1)` is a technology the world already has. `Some(u32::MAX)` is
        /// one that never lands, which is how the budget is pinned.
        research_lands_after: Option<u32>,
    }

    impl Default for Script {
        fn default() -> Self {
            Script {
                walk_delay_ms: BTreeMap::new(),
                mine_delay_ms: BTreeMap::new(),
                fail_walk: BTreeSet::new(),
                fail_mine: BTreeSet::new(),
                speed: 1.0,
                placement: None,
                ticks_per_second: None,
                research_lands_after: None,
            }
        }
    }

    /// A hand-written actuator rather than `MockAct`: mockall's `returning`
    /// closure is synchronous, so it cannot `await` a delay, and every
    /// concurrency test here needs one bot to be slower than another.
    struct RecordingAct {
        script: Script,
        origin: tokio::time::Instant,
        seen: std::sync::Mutex<Vec<(Dispatch, Duration)>>,
        /// Every `technology_researched` question, in order.
        tech_queries: std::sync::Mutex<Vec<String>>,
        /// Every craft dispatch, paired with how many technology questions had
        /// been asked by the time it went out.
        ///
        /// The pairing is the assertion: "the craft happened after the unlock
        /// was confirmed" is an ordering claim, and a count taken at dispatch
        /// is the only way to state it that a mutation cannot satisfy by
        /// accident.
        crafts: std::sync::Mutex<Vec<(String, usize)>>,
    }

    impl RecordingAct {
        fn new(script: Script) -> Self {
            Self {
                script,
                origin: tokio::time::Instant::now(),
                seen: std::sync::Mutex::new(Vec::new()),
                tech_queries: std::sync::Mutex::new(Vec::new()),
                crafts: std::sync::Mutex::new(Vec::new()),
            }
        }

        /// How many times this actuator was asked about a technology.
        fn tech_query_count(&self) -> usize {
            self.tech_queries.lock().unwrap().len()
        }

        /// How many technology questions had been asked when `recipe` was
        /// dispatched, or `None` if it never was.
        fn crafted_after_queries(&self, recipe: &str) -> Option<usize> {
            self.crafts
                .lock()
                .unwrap()
                .iter()
                .find(|(name, _)| name == recipe)
                .map(|(_, n)| *n)
        }

        fn record(&self, what: Dispatch) {
            self.seen
                .lock()
                .unwrap()
                .push((what, self.origin.elapsed()));
        }

        fn order(&self) -> Vec<Dispatch> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(d, _)| d.clone())
                .collect()
        }

        fn mine_starts(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(d, _)| match d {
                    Dispatch::MineStart(item) => Some(item.clone()),
                    _ => None,
                })
                .collect()
        }

        /// Virtual time elapsed when `item` was dispatched to be mined.
        fn mine_started_at(&self, item: &str) -> Option<Duration> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .find(|(d, _)| d == &Dispatch::MineStart(item.to_string()))
                .map(|(_, at)| *at)
        }

        async fn delay(ms: u64) {
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }
    }

    /// Reports [`some_ticks`] for every dispatch. The numbers are far outside
    /// anything these fixtures schedule, so a test can tell an observation from
    /// a plan value without knowing the schedule.
    #[async_trait::async_trait]
    impl Actuator for RecordingAct {
        async fn walk(
            &self,
            bot: BotId,
            _to: Position,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.record(Dispatch::Walk(bot));
            Self::delay(self.script.walk_delay_ms.get(&bot).copied().unwrap_or(0)).await;
            if self.script.fail_walk.contains(&bot) {
                return Err(ActuatorError::Rejected("blocked".into()).into());
            }
            Ok(some_ticks())
        }

        async fn mine(
            &self,
            _bot: BotId,
            item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.record(Dispatch::MineStart(item.to_string()));
            Self::delay(self.script.mine_delay_ms.get(item).copied().unwrap_or(0)).await;
            self.record(Dispatch::MineEnd(item.to_string()));
            if self.script.fail_mine.contains(item) {
                return Err(ActuatorError::Rejected("no ore".into()).into());
            }
            Ok(some_ticks())
        }

        async fn craft(
            &self,
            _bot: BotId,
            recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            let asked = self.tech_query_count();
            self.crafts
                .lock()
                .unwrap()
                .push((recipe.to_string(), asked));
            Ok(some_ticks())
        }

        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<ActionTicks, ActuatorFailure> {
            Ok(some_ticks())
        }

        async fn insert(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            Ok(some_ticks())
        }

        async fn remove(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            Ok(some_ticks())
        }

        async fn research(&self, _tech: &str) -> Result<ActionTicks, ActuatorFailure> {
            Ok(some_ticks())
        }

        async fn game_speed(&self) -> Result<f64, ActuatorError> {
            Ok(self.script.speed)
        }

        /// A game clock running at `script.ticks_per_second`, read off the
        /// same virtual clock the sleeps use.
        ///
        /// This is the whole point of the fixture: a real server's tick rate
        /// is *not* `60 * game.speed`, it is whatever the machine manages, and
        /// no test that derives the tick from the sleep it just did can tell
        /// the two apart.
        async fn game_tick(&self) -> Result<Option<u64>, ActuatorError> {
            let Some(rate) = self.script.ticks_per_second else {
                return Ok(None);
            };
            Ok(Some((self.origin.elapsed().as_secs_f64() * rate) as u64))
        }

        /// Answers `Some(false)` until it has been asked
        /// `script.research_lands_after` times, then `Some(true)`; `Ok(None)`
        /// when the script names no number at all.
        async fn technology_researched(&self, tech: &str) -> Result<Option<bool>, ActuatorError> {
            let asked = {
                let mut q = self.tech_queries.lock().unwrap();
                q.push(tech.to_string());
                q.len() as u32
            };
            Ok(self.script.research_lands_after.map(|lands| asked >= lands))
        }

        fn take_placement(&self, _bot: BotId) -> Option<Placement> {
            self.script.placement.clone()
        }
    }

    /// Every concurrency test runs under a deadline, because the failure mode
    /// this code guards against is a hang, and a hung test suite looks like a
    /// slow one. Under `tokio::time::pause()` the runtime auto-advances to the
    /// next deadline once nothing is runnable, so a genuine deadlock reports a
    /// failure here immediately rather than blocking CI.
    async fn within_deadline<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(60), f)
            .await
            .expect("run did not terminate: some bot is waiting on a signal nobody will send")
    }

    // ------------------------------------------------- single-bot behaviour
    // (these were `run_bot`'s tests in task 4; `run_into` supersedes it, so
    // they now drive the signalled path with a single-bot schedule.)

    #[tokio::test]
    async fn a_bot_performs_its_steps_in_schedule_order() {
        let mut act = MockAct::new();
        let mut seq = mockall::Sequence::new();
        act.expect_walk()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(some_ticks()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![]);
        assert_eq!(log.status(mine_action_id()), Status::Success);
        assert_eq!(log.status(craft_action_id()), Status::Success);
    }

    #[tokio::test]
    async fn the_log_records_the_ticks_the_game_reported_not_the_ones_it_planned() {
        // The test that a field populated from the schedule would fail. The
        // fixture schedules the mine at 60..120; the actuator reports
        // 900_001/900_002. Asserting only that the fields *exist* would pass
        // either way, so this asserts they are not the plan's numbers.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net).await.expect("the run should start");

        let a = log
            .attempt(mine_action_id())
            .expect("the mine was attempted");
        assert_eq!(a.dispatched_tick, Some(900_001));
        assert_eq!(a.replied_tick, Some(900_002));
        assert_eq!(a.planned_start_tick, 60, "the estimate is untouched");
        assert_eq!(a.planned_end_tick, Some(120));
        assert_ne!(
            a.dispatched_tick,
            Some(a.planned_start_tick),
            "an observed tick equal to the planned one means the field is \
             being filled from the schedule"
        );
        assert_ne!(a.replied_tick, a.planned_end_tick);
        // Non-decreasing within the attempt, and across the run.
        assert!(a.dispatched_tick <= a.replied_tick);
        let craft = log.attempt(craft_action_id()).expect("the craft ran");
        assert!(craft.dispatched_tick.is_some());
    }

    #[tokio::test]
    async fn a_failed_dispatch_keeps_the_tick_the_game_stamped_on_it() {
        // The game received the command, stamped the tick it received it at,
        // and only then reported that it had gone wrong. That tick is a
        // measurement we were handed. It used to be dropped on the floor
        // because the error had nowhere to carry it, which made this `None`
        // indistinguishable from the `None` of a failure that happened before
        // the game ever saw the command.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| {
            Err(ActuatorError::Rejected("no ore here".into())
                .at(ActionTicks::new(Some(900_101), Some(900_140))))
        });

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net).await.expect("the run should start");

        let a = log
            .attempt(mine_action_id())
            .expect("the mine was attempted");
        assert_eq!(a.status, Status::Failed);
        assert_eq!(
            a.dispatched_tick,
            Some(900_101),
            "the game stamped this; failing afterwards is no reason to lose it"
        );
        assert_eq!(a.replied_tick, Some(900_140));
        assert_eq!(log.observed_duration(mine_action_id()), Some(39));
        assert_ne!(
            a.dispatched_tick,
            Some(a.planned_start_tick),
            "kept from the game, not borrowed from the schedule"
        );
        assert_eq!(
            a.error.as_deref(),
            Some("game rejected the command: no ore here"),
            "and the failure still says what it said"
        );
    }

    #[tokio::test]
    async fn a_failed_walk_keeps_the_tick_the_game_stamped_on_it() {
        // The same rule for the step that has no action id.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| {
            Err(ActuatorError::Rejected("path blocked".into())
                .at(ActionTicks::new(Some(900_007), None)))
        });

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net).await.expect("the run should start");

        let w = log.walk(BotId(0), 0).expect("the walk was dispatched");
        assert_eq!(w.status, Status::Failed);
        assert_eq!(w.dispatched_tick, Some(900_007));
        assert_eq!(
            w.replied_tick, None,
            "the game never reported an outcome, and nothing may invent one"
        );
        assert_eq!(log.observed_walk_duration(BotId(0), 0), None);
        assert_ne!(w.dispatched_tick, Some(w.planned_start_tick));
    }

    #[tokio::test]
    async fn a_failure_leaves_the_ticks_absent_rather_than_borrowing_the_plans() {
        // The other direction, and the reason the fix is not just "record a
        // tick on the failure path". Nothing was stamped here — the failure
        // happened before the game saw anything — so the ticks are absent as a
        // *fact*, and must stay absent. The planned numbers are sitting right
        // there in the same struct; the test exists because reaching for them
        // is the tempting wrong thing to do.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into()).into()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net).await.expect("the run should start");

        let a = log
            .attempt(mine_action_id())
            .expect("the mine was attempted");
        assert_eq!(a.status, Status::Failed);
        assert_eq!(a.dispatched_tick, None);
        assert_eq!(a.replied_tick, None);
        assert_eq!(
            a.planned_start_tick, 60,
            "the plan is still reported -- it is just not a measurement"
        );
    }

    #[tokio::test]
    async fn an_action_the_game_gave_no_verdict_for_is_lost_rather_than_failed() {
        // The executor-visible shape of what commit 0cb7636f left behind: the
        // game answered and the answer could not be read, so there is no
        // verdict to record. `Failed` would claim one — and recovery counts
        // failures towards escalation — while `Running` would draw a bot that
        // is busy. Neither is true; the run lost the thread.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| {
            Err(ActuatorError::NoVerdict("unreadable action_completed status".into()).into())
        });
        // The bot still stops: an outcome nobody knows is no basis for running
        // the step that depended on it.
        act.expect_craft().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net).await.expect("the run should start");

        assert_eq!(log.status(mine_action_id()), Status::Lost);
        assert_ne!(
            log.status(mine_action_id()),
            Status::Running,
            "the run is over; nothing is in flight"
        );
        assert!(
            log.failed().is_empty(),
            "no verdict arrived, so nothing may be reported as having failed"
        );
        assert_eq!(
            log.status(craft_action_id()),
            Status::Pending,
            "the rest of the bot's slice was never dispatched"
        );
    }

    #[tokio::test]
    async fn a_failed_step_is_logged_and_stops_that_bot() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into()).into()));
        // The craft step follows the failing mine in the schedule. If the run
        // loop pressed on after the failure instead of stopping, this is what
        // it would dispatch next.
        act.expect_craft().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![mine_action_id()]);
        assert_eq!(
            log.status(craft_action_id()),
            Status::Pending,
            "the run loop must stop at the first failure, not merely record it \
             and press on"
        );
    }

    #[tokio::test]
    async fn each_step_is_dispatched_by_the_bot_it_names() {
        // Guards the `.filter(|s| s.bot == bot)` line: without it, every bot's
        // slice would issue every other bot's steps too.
        use mockall::predicate::eq;

        let mut act = MockAct::new();
        act.expect_walk()
            .with(
                eq(BotId(0)),
                eq(Position::new(10., 10.)),
                eq(WALK_FIXTURE_RADIUS),
            )
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));
        act.expect_walk()
            .with(eq(BotId(1)), eq(Position::new(5., 5.)), eq(4.0))
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine()
            .with(
                eq(BotId(0)),
                eq("iron-ore"),
                eq(Position::new(10., 10.)),
                eq(1u32),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft()
            .with(eq(BotId(0)), eq("iron-gear-wheel"), eq(1u32))
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));

        let (net, mut sched) = walk_then_mine_fixture();
        sched.steps.push(ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(5., 5.),
                radius: 4.0,
            },
            bot: BotId(1),
            start: 0,
            end: 10,
        });

        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");
        assert_eq!(log.failed(), vec![]);
        assert_eq!(log.status(mine_action_id()), Status::Success);
        assert_eq!(log.status(craft_action_id()), Status::Success);
    }

    /// The plan's tolerance is what the actuator is told to walk to.
    ///
    /// `Condition::AtPosition` has always carried a radius and the scheduler
    /// has always used it to *estimate* travel; what it did not do was hand it
    /// on. So a step meaning "stand within 3 of the ore" arrived at the game
    /// as "stand on the ore", and for a place/insert/remove — whose target is
    /// the entity's own tile — the pathfinder could not answer at all.
    ///
    /// Two walks with different radii, and `.with` matchers rather than a
    /// captured value, so the assertion cannot be satisfied by any constant:
    /// mockall fails the call outright if the radius does not match.
    #[tokio::test]
    async fn each_walk_is_dispatched_with_its_own_steps_radius() {
        use mockall::predicate::eq;
        let mut act = MockAct::new();
        act.expect_walk()
            .with(
                eq(BotId(0)),
                eq(Position::new(10., 10.)),
                eq(WALK_FIXTURE_RADIUS),
            )
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));
        act.expect_walk()
            .with(eq(BotId(1)), eq(Position::new(5., 5.)), eq(9.5))
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

        let (net, mut sched) = walk_then_mine_fixture();
        sched.steps.push(ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(5., 5.),
                radius: 9.5,
            },
            bot: BotId(1),
            start: 0,
            end: 10,
        });

        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");
        assert_eq!(log.failed(), vec![]);
        assert_eq!(
            log.walk(BotId(0), 0).expect("bot 0 walked").status,
            Status::Success
        );
        assert_eq!(
            log.walk(BotId(1), 0).expect("bot 1 walked").status,
            Status::Success
        );
    }

    #[tokio::test]
    async fn a_walk_failure_stops_the_bot_before_the_action_runs() {
        let mut act = MockAct::new();
        act.expect_walk()
            .times(1)
            .returning(|_, _, _| Err(ActuatorError::Rejected("blocked".into()).into()));
        act.expect_mine().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        // The action never even started: the log has no attempt for it at all,
        // not merely a non-Failed status.
        //
        // Asserted as "no attempts", not `log.is_empty()`, since the log is no
        // longer empty here — the failed walk is now recorded, which is the
        // point of recording walks at all. The claim this test makes is about
        // the *action*, and that is what it now says.
        assert_eq!(log.failed(), vec![]);
        assert!(log.attempt(mine_action_id()).is_none());
        assert!(log.attempt(craft_action_id()).is_none());
        assert_eq!(log.status(mine_action_id()), Status::Pending);

        // And the walk that stopped the bot left the only record of why.
        let w = log.walk(BotId(0), 0).expect("the failed walk is recorded");
        assert_eq!(w.status, Status::Failed);
        assert_eq!(
            w.error.as_deref(),
            Some("game rejected the command: blocked")
        );
        assert_eq!(w.dispatched_tick, None, "a walk that failed has no ticks");
        assert_eq!(w.replied_tick, None);
    }

    #[tokio::test]
    async fn a_walks_game_ticks_are_recorded_and_are_not_the_scheduled_ones() {
        // The regression this whole change exists for. `Actuator::walk` has
        // always returned `ActionTicks`; `run.rs` kept only the failure bit,
        // so the largest span in a plan — walking — was unobservable.
        //
        // The mocked ticks are deliberately nowhere near the fixture's
        // schedule, so a field quietly populated from `ScheduledStep` would
        // fail here rather than look plausible.
        let mut act = MockAct::new();
        act.expect_walk()
            .times(1)
            .returning(|_, _, _| Ok(ActionTicks::new(Some(800_010), Some(800_910))));
        act.expect_mine()
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft()
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));

        let (net, sched) = walk_then_mine_fixture();
        // The plan-side numbers this must not be confused with.
        let walk_step = sched
            .steps
            .iter()
            .find(|s| matches!(s.what, StepKind::Walk { .. }))
            .expect("the fixture has a walk");

        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        let w = log.walk(BotId(0), 0).expect("the walk is recorded");
        assert_eq!(w.status, Status::Success);
        assert_eq!(
            w.to,
            Position::new(10., 10.),
            "the destination it was sent to"
        );
        assert_eq!(w.dispatched_tick, Some(800_010));
        assert_eq!(w.replied_tick, Some(800_910));
        assert_eq!(log.observed_walk_duration(BotId(0), 0), Some(900));

        // The estimate travels out beside the measurement, unchanged.
        assert_eq!(w.planned_start_tick, walk_step.start);
        assert_eq!(w.planned_end_tick, walk_step.end);
        assert_ne!(
            w.dispatched_tick,
            Some(w.planned_start_tick),
            "a field named for a measurement must not hold the plan"
        );
        assert_ne!(w.replied_tick, Some(w.planned_end_tick));

        // Exactly one walk, and it is the only thing keyed by a bot rather
        // than an action.
        assert_eq!(log.walks().count(), 1);
    }

    #[tokio::test]
    async fn a_walk_the_actuator_could_not_time_stays_absent_rather_than_defaulting() {
        // An actuator with no game clock is a legitimate implementation
        // (`ActionTicks::UNKNOWN`), and the walk still happened. "When" is
        // simply not known, and must not be filled in from the schedule.
        let mut act = MockAct::new();
        act.expect_walk()
            .times(1)
            .returning(|_, _, _| Ok(ActionTicks::UNKNOWN));
        act.expect_mine()
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft()
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        let w = log.walk(BotId(0), 0).expect("the walk is recorded");
        assert_eq!(
            w.status,
            Status::Success,
            "it walked; we just cannot time it"
        );
        assert_eq!(w.dispatched_tick, None);
        assert_eq!(w.replied_tick, None);
        assert_ne!(w.dispatched_tick, Some(0));
        assert_ne!(w.dispatched_tick, Some(w.planned_start_tick));
        assert_eq!(log.observed_walk_duration(BotId(0), 0), None);
        assert_eq!(w.error, None, "an unmeasured walk is not a failed one");
    }

    #[tokio::test]
    async fn an_action_missing_from_the_network_is_logged_as_a_failure() {
        let act = MockAct::new();
        let (_net, sched) = walk_then_mine_fixture();
        // Deliberately give an empty network: the schedule references an
        // action id the network does not contain.
        let empty_net = ActionNetwork::new();
        // Bypass the walk step's own expectations by only scheduling the Act.
        let sched = Schedule {
            steps: sched
                .steps
                .into_iter()
                .filter(|s| matches!(s.what, StepKind::Act { .. }))
                .collect(),
            makespan: sched.makespan,
        };

        let log = run(&act, &sched, &empty_net)
            .await
            .expect("the run should have started");
        assert_eq!(log.failed(), vec![mine_action_id()]);
    }

    #[tokio::test]
    async fn insert_dispatches_the_entity_and_slot_the_action_named() {
        // Regression guard for the inventory-slot hazard: the executor must
        // pass through exactly the (entity, slot) pair the action carries,
        // never substitute one that "looks right" for the item.
        use mockall::predicate::eq;

        let mut act = MockAct::new();
        act.expect_insert()
            .with(
                eq(BotId(0)),
                eq("stone-furnace"),
                eq(Position::new(1., 1.)),
                eq(InventorySlot::FurnaceSource),
                eq("iron-ore"),
                eq(4u32),
            )
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(some_ticks()));

        let kind = ActionKind::Insert {
            pos: Position::new(1., 1.),
            entity: "stone-furnace".into(),
            slot: InventorySlot::FurnaceSource,
            item: "iron-ore".into(),
            count: 4,
        };
        perform(&act, BotId(0), &kind).await.unwrap();
    }

    // ------------------------------------------------------- cross-bot waits

    #[tokio::test(start_paused = true)]
    async fn a_bot_waits_for_another_bots_action_before_its_own() {
        // Bot 0 takes a long walk before its action; bot 1's walk is
        // instantaneous. Without the predecessor wait, bot 1 would mine
        // copper-ore first while bot 0 was still walking.
        let mut script = Script::default();
        script.walk_delay_ms.insert(BotId(0), 1_000);

        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![]);
        assert_eq!(
            act.mine_starts(),
            vec!["iron-ore".to_string(), "copper-ore".to_string()]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_dependent_action_is_abandoned_when_its_predecessor_fails() {
        let mut script = Script::default();
        script.fail_mine.insert("iron-ore".to_string());

        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![first_action_id()]);
        assert_eq!(log.status(second_action_id()), Status::Pending);
        assert_eq!(
            act.mine_starts(),
            vec!["iron-ore".to_string()],
            "the dependent action must never be dispatched"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn abandonment_propagates_along_a_chain_of_waiting_bots() {
        // 0 -> 1 -> 2, one bot each. Bot 1 never runs its action, so bot 2's
        // release depends on bot 1 publishing its own abandonment rather than
        // simply returning.
        let mut net = ActionNetwork::new();
        net.add(mine_of(ActionId(0), "iron-ore"));
        net.add(mine_of(ActionId(1), "copper-ore"));
        net.add(mine_of(ActionId(2), "coal"));
        net.link(ActionId(0), ActionId(1), 0);
        net.link(ActionId(1), ActionId(2), 0);
        let sched = Schedule {
            steps: vec![
                act_step(ActionId(0), BotId(0), 0, 60),
                act_step(ActionId(1), BotId(1), 60, 120),
                act_step(ActionId(2), BotId(2), 120, 180),
            ],
            makespan: 180,
        };

        let mut script = Script::default();
        script.fail_mine.insert("iron-ore".to_string());
        let act = RecordingAct::new(script);

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![ActionId(0)]);
        assert_eq!(log.status(ActionId(1)), Status::Pending);
        assert_eq!(log.status(ActionId(2)), Status::Pending);
    }

    #[tokio::test(start_paused = true)]
    async fn a_walk_failure_releases_the_bots_waiting_on_that_bot() {
        let mut script = Script::default();
        script.fail_walk.insert(BotId(0));
        let act = RecordingAct::new(script);

        let (net, sched) = cross_bot_fixture(0);
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        // Bot 0 never reached its action, so nothing is logged for it — but
        // bot 1 must still be released rather than waiting on a signal that
        // will never come.
        assert_eq!(log.status(first_action_id()), Status::Pending);
        assert_eq!(log.status(second_action_id()), Status::Pending);
        assert!(act.mine_starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_action_missing_from_the_network_releases_the_waiting_bots() {
        // Bot 0's first step names an id the network does not contain; its
        // second step is the one bot 1 is waiting on.
        let (net, _) = cross_bot_fixture(0);
        let sched = Schedule {
            steps: vec![
                act_step(ActionId(99), BotId(0), 0, 30),
                act_step(first_action_id(), BotId(0), 30, 90),
                act_step(second_action_id(), BotId(1), 90, 150),
            ],
            makespan: 150,
        };

        let act = RecordingAct::new(Script::default());
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![ActionId(99)]);
        assert_eq!(log.status(first_action_id()), Status::Pending);
        assert_eq!(log.status(second_action_id()), Status::Pending);
    }

    #[tokio::test(start_paused = true)]
    async fn a_predecessor_nobody_is_scheduled_to_run_does_not_strand_its_dependent() {
        // The network knows about action 0, but no bot is scheduled to run it.
        // Nothing will ever signal it, so its dependent has to be released at
        // the start of the run instead of waiting forever.
        let (net, _) = cross_bot_fixture(0);
        let sched = Schedule {
            steps: vec![act_step(second_action_id(), BotId(1), 0, 60)],
            makespan: 60,
        };

        let act = RecordingAct::new(Script::default());
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.status(second_action_id()), Status::Pending);
        assert!(act.mine_starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_lag_edge_delays_the_dependent_action_past_its_predecessor() {
        // 60 ticks of machine time at 60 ticks/second is one second: the
        // furnace keeps working after the bot walks away, and the predecessor's
        // own completion signal does not cover that wait.
        let act = RecordingAct::new(Script::default());
        let (net, sched) = cross_bot_fixture(60);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let iron = act.mine_started_at("iron-ore").expect("iron-ore mined");
        let copper = act.mine_started_at("copper-ore").expect("copper-ore mined");
        assert!(
            copper - iron >= Duration::from_millis(1_000),
            "expected the 60-tick lag to be honoured, but copper started \
             {:?} after iron",
            copper - iron
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_faster_game_speed_shortens_the_lag_wait() {
        // Same 60-tick lag as the test above, but the actuator reports the
        // game running at double speed. Machine time passes twice as fast, so
        // the same number of ticks is half the wall-clock wait: this is what
        // distinguishes "the executor reads game speed" from "the executor
        // still assumes 1.0 and this is a hardcoded 1-second wait dressed up
        // with an unread speed field".
        let script = Script {
            speed: 2.0,
            ..Default::default()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(60);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let iron = act.mine_started_at("iron-ore").expect("iron-ore mined");
        let copper = act.mine_started_at("copper-ore").expect("copper-ore mined");
        let gap = copper - iron;
        assert!(
            gap >= Duration::from_millis(500) && gap < Duration::from_millis(1_000),
            "expected a ~500ms wait at double speed, got {gap:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lag_edge_waits_for_game_ticks_not_for_seconds() {
        // The defect from `run-1788320177-77989`, in miniature. The game runs
        // at 50 ticks a second rather than 60 -- a server behind by a sixth,
        // which is what six 1920x1080 screenshots every 300 ticks cost -- and
        // `game.speed` still reports 1.0, because it is the rate the game is
        // *asked* for and says nothing about the rate it achieves.
        //
        // 60 ticks of machine time is therefore 1.2 s of wall clock, not 1.0.
        // The old code slept 1.0 and dispatched into a furnace that had not
        // finished; anything under 1.2 s here is that same bug.
        let script = Script {
            ticks_per_second: Some(50.0),
            ..Default::default()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(60);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let iron = act.mine_started_at("iron-ore").expect("iron-ore mined");
        let copper = act.mine_started_at("copper-ore").expect("copper-ore mined");
        let gap = copper - iron;
        assert!(
            gap >= Duration::from_millis(1_150),
            "60 game ticks at 50 ticks/second is 1.2s of wall clock; waiting \
             {gap:?} means the lag was spent as seconds rather than as ticks"
        );
        assert!(
            gap < Duration::from_millis(1_500),
            "the wait must stop once the game's clock reaches the deadline, \
             not overshoot: got {gap:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_game_clock_ends_the_lag_wait_instead_of_hanging_the_run() {
        // A clock that never advances -- a paused host, `/editor`, a server
        // mid-save. Chasing it would wait forever, and a bot that never moves
        // again is a far worse failure than an action the game gets to judge
        // and refuse. So the chase is budgeted: it gives up and dispatches.
        let script = Script {
            ticks_per_second: Some(0.0),
            ..Default::default()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(60);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let iron = act.mine_started_at("iron-ore").expect("iron-ore mined");
        let copper = act
            .mine_started_at("copper-ore")
            .expect("copper-ore must still be mined: a stopped clock is not a reason to hang");
        let gap = copper - iron;
        assert!(
            gap >= Duration::from_millis(1_000),
            "the first estimated wait must still be served in full, got {gap:?}"
        );
        assert!(
            gap <= Duration::from_millis(2_100),
            "LAG_CHASE_BUDGET caps the chase at one extra estimate, so this \
             may not exceed ~2x the 1s estimate: got {gap:?}"
        );
    }

    #[tokio::test]
    async fn a_placement_the_actuator_reports_lands_on_its_attempts_placed_field() {
        // `RconActuator::place` cannot be driven here without a live game, but
        // `Actuator::take_placement`'s contract is exactly what it hands back
        // once one is claimed. This proves the settle path in `run_bot_signalled`
        // actually drains it onto the right `Attempt` -- the kind of wiring
        // that compiles and does nothing if nobody calls it.
        let placement = Placement {
            intent: EntitySnapshot {
                name: "stone-furnace".to_string(),
                position: Position::new(-12.0, 8.0),
                direction: 0,
            },
            actual: EntitySnapshot {
                name: "stone-furnace".to_string(),
                position: Position::new(-12.0, 8.0),
                direction: 0,
            },
            drift: None,
        };
        let script = Script {
            placement: Some(placement.clone()),
            ..Default::default()
        };
        let act = RecordingAct::new(script);

        let place_id = ActionId(0);
        let mut net = ActionNetwork::new();
        net.add(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".to_string(),
                    position: Position::new(-12.0, 8.0),
                    direction: 0,
                    ..Default::default()
                }),
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "place stone-furnace".into(),
        });
        let sched = Schedule {
            steps: vec![act_step(place_id, BotId(0), 0, 30)],
            makespan: 30,
        };

        let log = run(&act, &sched, &net).await.expect("the run should start");

        let a = log.attempt(place_id).expect("the place was attempted");
        assert_eq!(
            a.placed,
            Some(placement),
            "the actuator's placement must reach the attempt it belongs to"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_final_log_is_the_same_whichever_bot_finishes_first() {
        // Two bots, two independent actions, no edge between them — so the
        // order they finish in is decided purely by the actuator's timing.
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                act_step(second_action_id(), BotId(1), 10, 100),
            ],
            makespan: 100,
        };

        let run_with = |iron_ms: u64, copper_ms: u64| {
            let mut script = Script::default();
            script.mine_delay_ms.insert("iron-ore".to_string(), iron_ms);
            script
                .mine_delay_ms
                .insert("copper-ore".to_string(), copper_ms);
            RecordingAct::new(script)
        };

        let slow_iron = run_with(1_000, 10);
        let log_a = within_deadline(run(&slow_iron, &sched, &net))
            .await
            .expect("the run should have started");

        let slow_copper = run_with(10, 1_000);
        let log_b = within_deadline(run(&slow_copper, &sched, &net))
            .await
            .expect("the run should have started");

        assert_ne!(
            slow_iron.order(),
            slow_copper.order(),
            "the two runs must actually differ in completion order, or this \
             test proves nothing"
        );
        assert_eq!(
            log_a, log_b,
            "the finished log must not depend on which bot got there first"
        );
        assert_eq!(log_a.status(first_action_id()), Status::Success);
        assert_eq!(log_a.status(second_action_id()), Status::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn progress_is_readable_while_the_run_is_still_going() {
        // Task 7 reads `progress` mid-run; the shared log is why `run_into`
        // exists separately from `run`.
        let mut script = Script::default();
        script.mine_delay_ms.insert("copper-ore".to_string(), 1_000);
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);

        // Let bot 0 finish and bot 1 get as far as its (slow) mine.
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the run should still be in progress");
        {
            let seen = lock(&progress);
            assert_eq!(seen.status(first_action_id()), Status::Success);
            assert_eq!(seen.status(second_action_id()), Status::Running);
        }

        within_deadline(running)
            .await
            .expect("the run should have started");
        assert_eq!(lock(&progress).status(second_action_id()), Status::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_dropped_mid_dispatch_stops_reporting_its_actions_as_running() {
        // The same fixture as `progress_is_readable_while_the_run_is_still_going`,
        // and deliberately so: the two differ only in whether anybody is still
        // driving the run. While it is driven, the action is `Running` and that
        // is true. Once the future is dropped — a cancelled script, an aborted
        // task, a process going down — nothing will ever poll for that reply
        // again, and leaving it `Running` draws a bot that looks busy for a
        // session that has lost the thread.
        let mut script = Script::default();
        script.mine_delay_ms.insert("copper-ore".to_string(), 1_000);
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);

        let progress = Mutex::new(ExecutionLog::default());
        {
            let running = run_into(&act, &sched, &net, &progress);
            tokio::pin!(running);
            let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
            assert!(stalled.is_err(), "the run should still be in progress");
            assert_eq!(
                lock(&progress).status(second_action_id()),
                Status::Running,
                "while the run is alive, this really is in flight"
            );
        }

        let seen = lock(&progress);
        assert_eq!(
            seen.status(second_action_id()),
            Status::Lost,
            "the run was dropped between dispatch and reply"
        );
        assert_eq!(
            seen.status(first_action_id()),
            Status::Success,
            "an action that already had an outcome keeps it"
        );
    }

    // ------------------------------------------------------ circular waits

    #[tokio::test(start_paused = true)]
    async fn a_schedule_that_runs_a_network_edge_backwards_is_rejected() {
        // The network says 1 must precede 0, and `1 -> 0` on its own is a
        // perfectly acyclic graph — validating the network would report
        // success. But the schedule puts 0 first on the same bot, so the bot
        // waits for 1 at its first step and can never reach the second step
        // that would run it.
        let mut net = ActionNetwork::new();
        net.add(mine_of(ActionId(0), "iron-ore"));
        net.add(mine_of(ActionId(1), "copper-ore"));
        net.link(ActionId(1), ActionId(0), 0);
        let sched = Schedule {
            steps: vec![
                act_step(ActionId(0), BotId(0), 0, 60),
                act_step(ActionId(1), BotId(0), 60, 120),
            ],
            makespan: 120,
        };

        let act = RecordingAct::new(Script::default());
        let err = within_deadline(run(&act, &sched, &net))
            .await
            .expect_err("a circular wait must be refused, not run");
        assert!(
            matches!(err, ExecutionError::CircularWait(id) if id == ActionId(0) || id == ActionId(1)),
            "unexpected error: {err}"
        );
        assert!(
            act.mine_starts().is_empty(),
            "the run must be refused before anything reaches the game"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_circular_wait_that_spans_two_bots_is_rejected() {
        // No cycle exists in the network (only `d -> a` and `b -> c`), and
        // none exists in either bot's own sequence. The cycle appears only in
        // their union: a -> b (bot 0's order) -> c (network) -> d (bot 1's
        // order) -> a (network).
        let (a, b, c, d) = (ActionId(0), ActionId(1), ActionId(2), ActionId(3));
        let mut net = ActionNetwork::new();
        for (id, item) in [
            (a, "iron-ore"),
            (b, "copper-ore"),
            (c, "coal"),
            (d, "stone"),
        ] {
            net.add(mine_of(id, item));
        }
        net.link(d, a, 0);
        net.link(b, c, 0);
        let sched = Schedule {
            steps: vec![
                act_step(a, BotId(0), 0, 60),
                act_step(b, BotId(0), 60, 120),
                act_step(c, BotId(1), 0, 60),
                act_step(d, BotId(1), 60, 120),
            ],
            makespan: 120,
        };

        let act = RecordingAct::new(Script::default());
        let err = within_deadline(run(&act, &sched, &net))
            .await
            .expect_err("a circular wait must be refused, not run");
        assert!(
            matches!(err, ExecutionError::CircularWait(_)),
            "unexpected error: {err}"
        );
        assert!(act.mine_starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_schedule_that_merely_reorders_independent_actions_is_not_rejected() {
        // The guard must reject circular waits, not "the schedule disagrees
        // with the network's ordering". Bot 0 runs 1 before 0 with no edge
        // between them at all, which is perfectly runnable.
        let mut net = ActionNetwork::new();
        net.add(mine_of(ActionId(0), "iron-ore"));
        net.add(mine_of(ActionId(1), "copper-ore"));
        let sched = Schedule {
            steps: vec![
                act_step(ActionId(1), BotId(0), 0, 60),
                act_step(ActionId(0), BotId(0), 60, 120),
            ],
            makespan: 120,
        };

        let act = RecordingAct::new(Script::default());
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");
        assert_eq!(log.failed(), vec![]);
        assert_eq!(
            act.mine_starts(),
            vec!["copper-ore".to_string(), "iron-ore".to_string()]
        );
    }

    // ------------------------------------------------------ duplicate ids

    #[tokio::test(start_paused = true)]
    async fn an_action_scheduled_twice_for_one_bot_runs_once() {
        let (net, _) = cross_bot_fixture(0);
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                // Same id again, with a very different span, so "the first
                // occurrence's timing survived" is distinguishable from "the
                // second's did".
                act_step(first_action_id(), BotId(0), 60, 500),
                act_step(second_action_id(), BotId(1), 500, 560),
            ],
            makespan: 560,
        };

        let act = RecordingAct::new(Script::default());
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");
        assert_eq!(
            act.mine_starts(),
            vec!["iron-ore".to_string(), "copper-ore".to_string()],
            "the duplicated action must reach the game exactly once"
        );
        assert_eq!(
            log.planned_duration(first_action_id()),
            Some(60),
            "the first occurrence is the one recorded"
        );
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn an_action_scheduled_for_two_bots_runs_once_without_taking_down_the_run() {
        // Both bots used to complete the same log key, and `ExecutionLog`'s
        // double-completion `debug_assert!` panicked — inside `join_all`,
        // which unwinds every other bot with it.
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                act_step(first_action_id(), BotId(1), 60, 500),
                act_step(second_action_id(), BotId(1), 500, 560),
            ],
            makespan: 560,
        };

        let act = RecordingAct::new(Script::default());
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");
        assert_eq!(
            act.mine_starts(),
            vec!["iron-ore".to_string(), "copper-ore".to_string()]
        );
        assert_eq!(log.planned_duration(first_action_id()), Some(60));
        assert_eq!(
            log.status(second_action_id()),
            Status::Success,
            "the other bot's remaining work must survive the bad input"
        );
    }

    // ---------------------------------------------- research the plan unlocks

    /// The defect `run-1788365280-15443` recorded, as a test.
    ///
    /// The plan was right and the edge was honoured; the game applied the
    /// trigger technology 16 ticks after the craft that triggered it
    /// (`§53469§action_completed§ok 44` against `§53485§on_research_finished§`
    /// in `workspace/server-log.txt`), and the dependent craft went out inside
    /// that window. Without `await_research` this asserts `Some(0)` — the
    /// craft was dispatched having asked the game nothing at all.
    #[tokio::test(start_paused = true)]
    async fn a_craft_waits_for_the_unlock_its_predecessor_triggers() {
        let (net, sched) = trigger_unlock_fixture();
        let act = RecordingAct::new(Script {
            research_lands_after: Some(3),
            ..Default::default()
        });

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should start");

        assert_eq!(
            act.crafted_after_queries("automation-science-pack"),
            Some(3),
            "the craft must go out only after the game confirmed the unlock, \
             not merely after the craft that triggers it succeeded"
        );
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    /// The wait is bounded. A technology that never lands must cost one budget,
    /// not the run: the craft's own verdict is a far better report than a bot
    /// that never moves again.
    ///
    /// A mutation that drops the deadline hangs here, and
    /// `within_deadline` says so rather than blocking CI.
    #[tokio::test(start_paused = true)]
    async fn an_unlock_that_never_lands_still_dispatches_after_the_budget() {
        let (net, sched) = trigger_unlock_fixture();
        let act = RecordingAct::new(Script {
            research_lands_after: Some(u32::MAX),
            ..Default::default()
        });

        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should start");

        assert!(
            act.crafted_after_queries("automation-science-pack")
                .is_some(),
            "the craft is dispatched anyway once the budget is spent"
        );
        assert!(
            act.tech_query_count() > 1,
            "and it really did wait first: {} question(s) asked",
            act.tech_query_count()
        );
    }

    /// `Ok(None)` is "this actuator cannot say", and waiting on it would spend
    /// the whole budget on every craft for an answer that is never going to
    /// change. Asked once, then dispatched.
    #[tokio::test(start_paused = true)]
    async fn an_actuator_that_cannot_answer_is_asked_once_and_not_waited_on() {
        let (net, sched) = trigger_unlock_fixture();
        let act = RecordingAct::new(Script {
            research_lands_after: None,
            ..Default::default()
        });

        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should start");

        assert_eq!(
            act.tech_query_count(),
            1,
            "cannot answer is not the same claim as not researched"
        );
        assert_eq!(
            act.crafted_after_queries("automation-science-pack"),
            Some(1)
        );
    }

    /// The common case must stay free: a technology the world already has costs
    /// one question and no sleep at all.
    #[tokio::test(start_paused = true)]
    async fn a_technology_the_world_already_has_costs_one_question() {
        let (net, sched) = trigger_unlock_fixture();
        let act = RecordingAct::new(Script {
            research_lands_after: Some(1),
            ..Default::default()
        });

        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should start");

        assert_eq!(act.tech_query_count(), 1);
        assert_eq!(
            act.crafted_after_queries("automation-science-pack"),
            Some(1)
        );
    }

    /// And an action that states no research precondition asks nothing. The
    /// wait keys on the plan's own condition, not on "this is a craft".
    #[tokio::test(start_paused = true)]
    async fn an_action_with_no_research_precondition_asks_nothing() {
        let (net, sched) = walk_then_mine_fixture();
        let act = RecordingAct::new(Script {
            research_lands_after: Some(u32::MAX),
            ..Default::default()
        });

        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should start");

        assert_eq!(
            act.tech_query_count(),
            0,
            "nothing here names a technology, so nothing may be waited on"
        );
        assert!(act.crafted_after_queries("iron-gear-wheel").is_some());
    }
}
