//! Run every bot's slice of a `Schedule` against an `Actuator`.
//!
//! Bots run concurrently and wait on each other through one
//! `tokio::sync::watch` channel per action, replacing the old executor's
//! 100 ms poll loop (`crates/core/src/plan/execute.rs:79`) with a completion
//! signal per action.

use crate::actuator::{ActionTicks, Actuator, ActuatorError, ActuatorFailure};
use crate::log::{ExecutionLog, Status, WaitKey, WaitKind};
use crate::occupancy::{Occupancy, inventory_footprint, occupancy, shares_inventory};
use factorio_bot_core::petgraph::algo::toposort;
use factorio_bot_core::petgraph::graph::{DiGraph, NodeIndex};
use factorio_bot_planner::{
    ActionId, ActionKind, ActionNetwork, BotId, Condition, ItemId, Schedule, ScheduledStep,
    StepKind, Ticks,
};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
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
/// # One *exclusive* action in flight per bot, not one action
///
/// The invariant this loop used to keep was "one action per bot at a time".
/// That is right for work the character does and wrong for work the game does
/// — see [`crate::occupancy`], and the 0-of-99 measurement in its docs. What it
/// keeps now:
///
/// - at most one [`Occupancy::Exclusive`] action, or one `Walk` step, is in
///   flight for a bot;
/// - any number of [`Occupancy::Background`] actions may be in flight beside
///   it, **provided their [`inventory_footprint`]s are disjoint** from the step
///   about to start and from each other. Where they are not, this loop waits
///   for the conflicting ones to settle, exactly as it always did.
///
/// # Ordering is untouched — only the bot's exclusivity is relaxed
///
/// A background action still runs [`await_preds`] before it dispatches, so
/// every network edge and every lag edge still means what it meant. And every
/// consumer still waits on that action's own completion signal, which is
/// published when the game *settles* it, not when it was queued: queuing
/// `craft 10 electronic-circuit` early is only correct because whatever needs
/// those circuits still waits for the craft to finish.
///
/// The wait edges this loop imposes are therefore a **subset** of the ones
/// [`check_wait_graph`] approved before the run started — it drops some of the
/// bot-order edges and adds none — and dropping edges from a graph already
/// proved acyclic cannot introduce a cycle. That is why the pre-flight check
/// stays exactly as it was: it now rejects a little more than it strictly must,
/// which is the safe direction for a check whose failure mode is a hung run.
///
/// # A failed action costs that action and what depends on it, not the batch
///
/// This loop used to stop the bot at its **first** failure, on the argument
/// that later steps in a chain depend on earlier ones. That is true of the
/// steps that depend on it and false of the rest, and the difference is
/// expensive: in `run-1788663566-25023` one transport-belt refused because
/// bot 4 was standing on its tile ended the batch and left **~50 of a
/// 179-entity block never dispatched**. Losing fifty entities to one occupied
/// tile is a blast radius nobody chose.
///
/// So a failed or lost `Act` no longer stops the bot. It publishes its own
/// verdict, exactly as before, and the bot moves on to its next step. What
/// depends on the failure is abandoned by the mechanism that already exists
/// for it: [`await_preds`] reads the predecessor's signal and returns
/// [`PredOutcome::Abandoned`], so a belt nobody built takes down the inserter
/// that feeds it — and takes down nothing else. **The dependency graph
/// decides the blast radius**; this loop does not second-guess it. A step
/// abandoned that way publishes `Failed` for itself, which propagates the
/// same way one further hop.
///
/// The consequence to be honest about: an edge the network is *missing* used
/// to be masked by the bot stopping, and is now not. Two things narrow that.
/// `crate::occupancy` still holds the inventory ordering within a bot, since
/// nothing here reorders steps. And the failure is still recorded and still
/// counts towards `recover`'s escalation budget, so a batch that fails
/// repeatedly replans rather than grinding on.
///
/// # A failed WALK still stops the bot, and deliberately
///
/// A walk carries no signal, so nothing downstream can read its verdict — and
/// its effect is the bot's *position*, which every later step of the slice
/// depends on without any edge saying so. Pressing on there would dispatch
/// every remaining action from wherever the bot got stuck. That is the one
/// dependency the network genuinely does not hold, so it stays a stop.
async fn run_bot_signalled<'a>(
    act: &'a dyn Actuator,
    bot: BotId,
    steps: &[&'a ScheduledStep],
    net: &'a ActionNetwork,
    log: &'a Mutex<ExecutionLog>,
    senders: &'a BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &'a BTreeMap<ActionId, watch::Receiver<Status>>,
) {
    let mine: Vec<&'a ScheduledStep> = steps.iter().copied().filter(|s| s.bot == bot).collect();
    let mut flight: Vec<InFlight<'a>> = Vec::new();

    for (i, step) in mine.iter().copied().enumerate() {
        let footprint = footprint_of(net, step);

        // Anything already queued that touches the same items has to land
        // first. The plan sized this step against an inventory the queued
        // action is about to change, and `crates/planner` reasons about that
        // inventory by walking the bot's steps *in order* — see
        // `crate::occupancy` for the two mechanisms that rely on it.
        //
        // This is the one wait that is not a plan edge, and the one that makes
        // a bot look most idle: it has dispatched everything the plan allowed
        // and is queued behind its own background craft. Recorded against the
        // step that is blocked, naming the queued action that is blocking it.
        let conflict = flight
            .iter()
            .find(|queued| shares_inventory(&queued.footprint, &footprint))
            .map(|queued| {
                WaitGuard::enter(
                    log,
                    act_id(step).map_or(WaitKey::Walk(bot, i), WaitKey::Action),
                    bot,
                    WaitKind::BackgroundConflict { on: queued.action },
                )
            });
        settle_background(&mut flight, |queued| {
            shares_inventory(&queued.footprint, &footprint)
        })
        .await;
        drop(conflict);

        if let Some(action) = act_id(step)
            && net
                .action(action)
                .is_some_and(|a| occupancy(a) == Occupancy::Background)
        {
            // Queued and walked away from. Note this is not a dispatch yet:
            // `run_action` does its own `await_preds` first, so what the bot
            // walks away from is a *promise* to dispatch as soon as the plan
            // allows — which is the whole point, since waiting for a
            // predecessor is exactly the time the old loop threw away.
            flight.push(InFlight {
                action,
                footprint,
                fut: Box::pin(run_action(act, bot, step, net, log, senders, receivers)),
            });
            continue;
        }

        // The exclusive step, driven alongside whatever is still in flight, so
        // a queued craft keeps making progress while the character works.
        match &step.what {
            // A walk is the one step whose failure still stops the bot: its
            // effect is a position no signal carries. See this function's
            // doc.
            StepKind::Walk { .. } => {
                if drive(&mut flight, run_walk(act, bot, i, step, log))
                    .await
                    .is_err()
                {
                    halt(&mine, i, &mut flight, senders);
                    return;
                }
            }
            // A failed action has published its own verdict; whatever waits on
            // it abandons itself, and whatever does not keeps going.
            StepKind::Act { .. } => {
                drive(
                    &mut flight,
                    run_action(act, bot, step, net, log, senders, receivers),
                )
                .await;
            }
        }
    }

    // The bot's steps are done; what it queued and walked away from is not.
    // Returning here would drop those futures, and `LoseTrackOnDrop` would then
    // report as `Lost` crafts the game was in the middle of finishing — and
    // release their waiters as abandoned, which would take down bots that were
    // about to succeed.
    settle_background(&mut flight, |_| true).await;
}

/// A background action a bot queued and walked away from.
struct InFlight<'a> {
    action: ActionId,
    /// What it may move in or out of the bot's inventory. See
    /// [`crate::occupancy`].
    footprint: BTreeSet<ItemId>,
    fut: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
}

/// The items a step may move in or out of its bot's inventory.
///
/// A `Walk` moves none, so it never blocks a queued craft and a queued craft
/// never blocks it — which is the case the whole change exists for. An action
/// the network does not hold is about to be failed by [`run_action`] anyway,
/// and an empty set for it blocks nothing that its failure will not.
fn footprint_of(net: &ActionNetwork, step: &ScheduledStep) -> BTreeSet<ItemId> {
    act_id(step)
        .and_then(|id| net.action(id))
        .map(inventory_footprint)
        .unwrap_or_default()
}

/// Poll every in-flight background action once, dropping the ones that
/// finished from the set.
///
/// It reports nothing, because there is nothing left for the caller to decide:
/// a background action that failed has already published its own verdict, and
/// what waits on that verdict abandons itself through [`await_preds`]. This
/// used to hand back a reason to stop the whole bot, which is the batch-wide
/// blast radius `run_bot_signalled` documents having given up.
fn poll_background(flight: &mut Vec<InFlight<'_>>, cx: &mut Context<'_>) {
    let mut i = 0;
    while i < flight.len() {
        match flight[i].fut.as_mut().poll(cx) {
            Poll::Ready(()) => {
                flight.remove(i);
            }
            Poll::Pending => i += 1,
        }
    }
}

/// Keep the background set running until nothing `blocking` names is left in
/// it.
///
/// `|_| true` drains the set completely; a footprint predicate waits out only
/// the actions that could disturb what is about to start. Everything else is
/// polled either way — a craft that shares no items keeps making progress
/// while a conflicting one is waited out.
async fn settle_background<'a, F>(flight: &mut Vec<InFlight<'a>>, blocking: F)
where
    F: Fn(&InFlight<'a>) -> bool,
{
    std::future::poll_fn(|cx| {
        poll_background(flight, cx);
        if flight.iter().any(&blocking) {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await
}

/// Run `fut` to completion while the background set keeps making progress.
///
/// A background failure does **not** cancel `fut`: by the time it is noticed
/// `fut` may already be a command the game has, and dropping it there would
/// turn a dispatch with a verdict coming into one nobody will ever hear about.
async fn drive<'a, T>(flight: &mut Vec<InFlight<'a>>, fut: impl Future<Output = T>) -> T {
    let mut fut = Box::pin(fut);
    std::future::poll_fn(|cx| {
        // The background set first, so a craft queued a moment ago reaches the
        // game ahead of the exclusive step that follows it. Correctness does
        // not rest on that ordering — disjoint footprints are what make the
        // two safe in either order, see `crate::occupancy` — but the plan's
        // order is still the best order to ask the game for.
        poll_background(flight, cx);
        fut.as_mut().poll(cx)
    })
    .await
}

/// Stop this bot: give up on everything it queued, and release every waiter on
/// the steps it will now never reach.
///
/// `from` is the first index of the bot's slice that will now never run. Only
/// a failed walk reaches here — see [`run_bot_signalled`] — and a walk
/// publishes no verdict of its own, so abandonment starts *at* it.
fn halt(
    mine: &[&ScheduledStep],
    from: usize,
    flight: &mut Vec<InFlight<'_>>,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
) {
    // Whatever is still queued is dropped where it stands, and dropped is
    // `Lost`, not `Failed`: the game may well finish the craft, and nobody in
    // this run will ever hear about it. `await_preds` abandons on either, so
    // this does not change who is released — but a run record must not claim a
    // verdict the game never gave, and `recover` counts failures towards its
    // escalation budget and losses not at all.
    for queued in flight.iter() {
        publish(senders, queued.action, Status::Lost);
    }
    let dropped: BTreeSet<ActionId> = flight.iter().map(|queued| queued.action).collect();
    flight.clear();
    abandon_rest(&mine[from.min(mine.len())..], senders, &dropped);
}

/// One `Walk` step.
///
/// A walk needs no `ActionId` to be recorded. `run_bot_signalled` iterates one
/// bot's steps in schedule order, so `(bot, index)` is already a unique, stable
/// key — the ticks the actuator has always measured here now have somewhere to
/// go. Walking is most of the wall-clock in these plans, so dropping them was
/// the biggest hole in the timeline.
async fn run_walk(
    act: &dyn Actuator,
    bot: BotId,
    index: usize,
    step: &ScheduledStep,
    log: &Mutex<ExecutionLog>,
) -> Result<(), ()> {
    // Unreachable by construction: the caller matched the step kind before
    // choosing this. Answering `Ok` rather than panicking keeps a future
    // mis-wiring a step that did not happen instead of a run that aborts.
    let StepKind::Walk {
        to,
        min_radius,
        radius,
    } = &step.what
    else {
        return Ok(());
    };
    lock(log).start_walk(bot, index, to.clone(), step.start, step.end);
    // Walking is most of the wall clock in these plans, and a bot that is
    // walking has nothing in flight -- which is the misreading
    // `EventKind::BatchProgress` already warns about. Naming the destination
    // here is what turns "in_flight: 0" from a symptom into an answer.
    let walking = WaitGuard::enter(
        log,
        WaitKey::Walk(bot, index),
        bot,
        WaitKind::Walk { to: to.clone() },
    );
    let outcome = act.walk(bot, to.clone(), *min_radius, *radius).await;
    // Left before the verdict is written, so nothing can read a settled walk
    // that is still listed as walking.
    drop(walking);
    match outcome {
        Ok(ticks) => {
            // Same order and same reasoning as the action arm: the
            // observation, then the outcome.
            let mut log = lock(log);
            log.observe_walk(bot, index, ticks);
            log.succeed_walk(bot, index);
            Ok(())
        }
        Err(f) => {
            // Whatever the game stamped before this went wrong is recorded
            // first, exactly as on the success path: a walk the game
            // acknowledged and then refused really was dispatched at a tick,
            // and dropping that number would make it indistinguishable from a
            // walk the game never saw.
            //
            // The entry's `status` is then what keeps a walk that did not
            // happen distinguishable from one that happened unobserved — and
            // from one whose outcome the game never reported, which is neither.
            let mut log = lock(log);
            log.observe_walk(bot, index, f.ticks);
            match &f.error {
                ActuatorError::NoVerdict(_) => {
                    log.lose_track_walk(bot, index, &f.to_string());
                }
                _ => log.fail_walk(bot, index, f.to_string()),
            }
            // A walk carries no signal of its own, so the step it was going to
            // enable is abandoned along with the rest.
            Err(())
        }
    }
}

/// One `Act` step: wait out its predecessors, dispatch it, and publish what the
/// game said.
///
/// The same function serves an exclusive action, which the bot stands and waits
/// for, and a background one, which it queues and walks away from. Nothing in
/// here knows which it is: that is the point — the ordering guarantees are
/// identical, and only the caller's willingness to move on differs.
async fn run_action(
    act: &dyn Actuator,
    bot: BotId,
    step: &ScheduledStep,
    net: &ActionNetwork,
    log: &Mutex<ExecutionLog>,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) {
    // Unreachable by construction, as in `run_walk`.
    let Some(action) = act_id(step) else {
        return;
    };
    if let PredOutcome::Abandoned = await_preds(act, bot, net, action, log, receivers).await {
        // Nothing was dispatched, so nothing is written to the log -- but the
        // signal has to carry a verdict, because the bot no longer stops here
        // and nobody else will ever publish one for this action. Without it a
        // dependent of *this* step waits forever on a `Pending` that has no
        // writer left. `abandon_rest` used to do this for the whole slice at
        // once; abandoning one action at a time is the same propagation, one
        // hop per step.
        publish(senders, action, Status::Failed);
        return;
    }
    lock(log).start(action, step.start);
    let Some(a) = net.action(action) else {
        lock(log).fail(action, step.start, "action not in network".to_string());
        publish(senders, action, Status::Failed);
        return;
    };
    // `perform` awaits, so the guard is taken and dropped around it, never
    // held across it.
    //
    // The scope around the dispatch is the one unbounded wait in this file:
    // nothing here times out a command the game has acknowledged, so an RCON
    // reply that never comes back looks from outside exactly like an action
    // that is legitimately taking a long time. `WaitKind::Reply` is what lets
    // a reader tell "dispatched N minutes ago and still nothing" apart from
    // "not dispatched yet" -- both of which used to be a frozen counter and a
    // bot id. The guard is dropped *before* the outcome is written so a
    // settled action can never be listed as still awaiting its reply.
    let dispatched = {
        let _awaiting_reply = WaitGuard::enter(log, WaitKey::Action(action), bot, WaitKind::Reply);
        perform(act, bot, &a.kind, a.duration).await
    };
    match dispatched {
        Ok(ticks) => {
            // Observation first, then the plan-side outcome: both writes are
            // under the same guard as far as any reader is concerned, and
            // `succeed` is what marks the attempt finished, after which
            // `observe` would refuse.
            {
                let mut log = lock(log);
                log.observe(action, ticks);
                log.succeed(action, step.end);
                // Drained here, not inside `perform`/`place` itself: this
                // scope is the first place with both the actuator and the
                // scheduler's `ActionId` for what just finished, which is
                // exactly what `Actuator::take_placement` needs to attach the
                // fact to. A placement waits in the actuator at most this long
                // -- only an *exclusive* action can park one, and a bot runs
                // at most one of those at a time, so nothing can queue a
                // second one behind it before it is claimed. See
                // `crate::occupancy`, which is where that guarantee is stated
                // and where a new background kind would have to break it.
                if let Some(placement) = act.take_placement(bot) {
                    log.record_placement(action, placement);
                }
                // Drained in the same breath and for the same reason. This one
                // qualifies a *success*: an `insert` whose destination had no
                // room for the rest delivered less than the plan asked for and
                // still satisfied the goal, and without this the record would
                // show it as an ordinary full delivery. The short-source case
                // -- the bot not holding what the plan believed -- is a failure
                // and arrives on the `Err` arm below instead, so the two never
                // share a row.
                if let Some(full) = act.take_destination_full(bot) {
                    log.record_note(action, full.to_string());
                }
            }
            publish(senders, action, Status::Success);
        }
        Err(f) => {
            // The observation first, same order as the success path and for
            // the same reason. `f.ticks` is what the game had stamped before
            // it went wrong — often nothing, sometimes a real dispatch tick —
            // and it is the only number allowed anywhere near these fields.
            // What is *not* allowed is the planned tick sitting in the same
            // record.
            //
            // A verdict of failure and no verdict at all are then different
            // facts and are recorded as different states: `Failed` says the
            // game judged this and the judgement was no, `Lost` says nobody
            // will ever know. Recovery counts the first towards its escalation
            // budget and not the second, which is the whole reason the two
            // must not be collapsed here.
            let lost = matches!(f.error, ActuatorError::NoVerdict(_));
            {
                let mut log = lock(log);
                log.observe(action, f.ticks);
                if lost {
                    log.lose_track(action, &f.to_string());
                } else {
                    log.fail(action, step.end, f.to_string());
                }
            }
            // The waiters are released either way — a dependent cannot run on
            // a precondition nobody can vouch for — but they are told *which*
            // it was. Abandonment then starts one past this step, because this
            // step's own signal has just been published with the truth.
            publish(
                senders,
                action,
                if lost { Status::Lost } else { Status::Failed },
            );
        }
    }
}

/// One place to take the log guard, so poisoning is handled identically
/// everywhere. A panic mid-run should not turn every later write into a second
/// panic; the log is observational, so recovering the inner value is right.
fn lock(log: &Mutex<ExecutionLog>) -> std::sync::MutexGuard<'_, ExecutionLog> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

/// Holds one [`WaitKind`] open in the log for exactly as long as the executor
/// is in that state.
///
/// # Why a guard and not a pair of calls
///
/// Every wait in this file is entered immediately before an `.await` that can
/// be **cancelled**: `halt` drops whatever is still queued, `drive` drops a
/// background future the moment the bot stops, and `run_into`'s own future can
/// be dropped by a cancelled script. A `leave_wait` written after the await
/// runs on exactly the paths where nothing went wrong -- which would leave the
/// registry claiming a bot is blocked on a predecessor forever, and a stale
/// wait is worse than no wait at all. The whole point of this registry is that
/// a reader can believe it.
///
/// `Drop` takes the log guard, which is safe for the same reason
/// [`LoseTrackOnDrop`]'s is: `leave_wait` is one map removal with no await in
/// it, and `lock` recovers a poisoned mutex rather than panicking again while
/// unwinding.
struct WaitGuard<'a> {
    log: &'a Mutex<ExecutionLog>,
    key: WaitKey,
}

impl<'a> WaitGuard<'a> {
    fn enter(log: &'a Mutex<ExecutionLog>, key: WaitKey, bot: BotId, kind: WaitKind) -> Self {
        lock(log).enter_wait(key, bot, kind);
        WaitGuard { log, key }
    }
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        lock(self.log).leave_wait(self.key);
    }
}

/// Publish `Failed` for every action this bot will now never reach, skipping
/// the ones in `except`.
///
/// Without this the executor deadlocks: a bot that stops early leaves its
/// remaining actions at `Pending` forever, and any bot waiting on one of them
/// waits forever too. Abandonment has to propagate for the run to terminate.
///
/// `except` is what [`halt`] has already published `Lost` for — actions still
/// in flight when the bot gave up, which can sit anywhere in the slice rather
/// than only past the stop.
fn abandon_rest(
    rest: &[&ScheduledStep],
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    except: &BTreeSet<ActionId>,
) {
    for step in rest {
        if let StepKind::Act { action, .. } = &step.what
            && !except.contains(action)
        {
            publish(senders, *action, Status::Failed);
        }
    }
}

/// Publish `status` for `id`, unless something has already been published for
/// it.
///
/// The senders only ever carry a terminal verdict — `Success`, `Failed` or
/// `Lost` — so `Pending` is exactly "nobody has said anything yet", and the
/// first verdict to arrive is the one the game gave.
///
/// **The guard is new with per-bot concurrency and it is load-bearing.** While
/// a bot could only have one action in flight, everything from the stop
/// onwards was necessarily unstarted. Now a step *later* in the slice can have
/// finished before an earlier one failed — a craft that settled while the bot
/// walked on to a mine that then failed — and sending `Failed` over that
/// craft's `Success` would abandon the dependents of work that really was done.
fn publish(senders: &BTreeMap<ActionId, watch::Sender<Status>>, id: ActionId, status: Status) {
    if let Some(tx) = senders.get(&id)
        && *tx.borrow() == Status::Pending
    {
        let _ = tx.send(status);
    }
}

async fn await_preds(
    act: &dyn Actuator,
    bot: BotId,
    net: &ActionNetwork,
    id: ActionId,
    log: &Mutex<ExecutionLog>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) -> PredOutcome {
    let key = WaitKey::Action(id);
    let mut wait = LagWait::default();
    for (pred, lag) in net.preds(id) {
        let Some(rx) = receivers.get(&pred) else {
            continue;
        };
        let mut rx = rx.clone();
        // Entered only when we are actually about to block. A predecessor that
        // has already succeeded costs no entry at all, so the registry lists
        // waits rather than intentions -- an action that reports nothing here
        // really is not waiting for anything.
        let mut blocked: Option<WaitGuard> = None;
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
            if blocked.is_none() {
                blocked = Some(WaitGuard::enter(
                    log,
                    key,
                    bot,
                    WaitKind::Predecessor { on: pred },
                ));
            }
            if rx.changed().await.is_err() {
                return PredOutcome::Abandoned;
            }
        }
        // Left before the next predecessor is considered, so one key never
        // holds two waits and the clock restarts on each state it enters.
        drop(blocked);
        // Read *after* the signal, never before: the settle path writes the
        // observation into the log and only then publishes `Success`, so a
        // waiter that has seen `Success` is guaranteed to find the finish tick
        // already there. Reading it any earlier would race the writer and get
        // `None`, which this function would then quietly treat as "no clock".
        let finished = lock(log).attempt(pred).and_then(|a| a.replied_tick);
        wait.charge(lag, finished);
    }

    // A lag edge is machine time, not bot time: the furnace keeps working after
    // the bot walks away, and the plate is not there until it has. The
    // predecessor's own completion signal does not cover that wait, so honour
    // the lag once every predecessor has succeeded.
    if wait.owes_anything() {
        // The wait the plan asked for, and the one most likely to be
        // misdiagnosed: a bot serving a 12,240-tick smelt is doing exactly what
        // it was told, and reads from outside as a bot that has stopped. The
        // deadline is carried so a reader can see how much of it is left
        // instead of inferring it.
        let _lagging = WaitGuard::enter(
            log,
            key,
            bot,
            WaitKind::LagDeadline {
                deadline_tick: wait.deadline,
                largest_lag: wait.largest,
            },
        );
        wait_out_lag(act, wait).await;
    }
    // A predecessor's success is not the same fact as its *effect* having
    // landed. See `await_research`.
    await_research(act, bot, net, id, log).await;
    PredOutcome::Ready
}

/// What this action's lag edges still owe, once every predecessor has
/// succeeded.
///
/// # Why a lag is anchored to its own predecessor
///
/// The planner's semantics for a lag edge is
/// `deps_ready = max over preds (finished[pred] + lag)`
/// (`crates/planner/src/schedule.rs`), and a lag is machine time that starts
/// running the moment the predecessor settles — the furnace begins smelting
/// when the coal goes in, not when the bot comes back for the plates.
///
/// The executor used to collapse this to a single `max_lag` counted from the
/// moment the dependent's bot *arrived*. The two agree only when the bot
/// arrives on the exact tick its predecessor settled, and they diverge by
/// exactly the time the bot spent doing something else in between — which is
/// the parallelism the schedule was built to create. On
/// `run-1788465258-49050` that cost bot 1 24,583 of its 27,853 idle ticks:
/// 88% of its idle time was spent waiting out timers for machine time the
/// world had already spent, with cumulative `iron-plate` production provably
/// flat across the whole wait. See
/// `docs/superpowers/notes/2026-09-03-the-lag-clock-starts-too-late.md`.
///
/// So each lag is charged against the tick *its own* predecessor finished, and
/// only then maxed. Taking the max over lags alone and adding it to one
/// arrival tick — what the old code did — charges an early-finishing
/// predecessor's lag from a late-finishing predecessor's clock, which is the
/// same defect one level down.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LagWait {
    /// The latest `finish(pred) + lag(pred)` over predecessors the game gave a
    /// finish tick for. An absolute `game.tick`, directly comparable to
    /// [`Actuator::game_tick`], and `None` when no predecessor supplied one.
    deadline: Option<u64>,
    /// The largest lag whose predecessor has **no** recorded finish tick — a
    /// failure the game never stamped, a reply whose tick could not be parsed,
    /// an actuator with no clock. There is no anchor to hang it on, so it can
    /// only be served from now, which is the old behaviour and here is the
    /// honest fallback rather than the rule.
    unanchored: Ticks,
    /// The largest lag of any predecessor, anchored or not. Used only when the
    /// actuator cannot report a clock at all, where an absolute deadline is not
    /// comparable to anything and this is the best claim available.
    largest: Ticks,
}

impl LagWait {
    /// Record one predecessor's lag against the tick that predecessor finished,
    /// or against nothing if the game never said.
    fn charge(&mut self, lag: Ticks, finished: Option<Ticks>) {
        self.largest = self.largest.max(lag);
        match finished {
            Some(at) => {
                let due = u64::from(at).saturating_add(u64::from(lag));
                self.deadline = Some(self.deadline.map_or(due, |d| d.max(due)));
            }
            None => self.unanchored = self.unanchored.max(lag),
        }
    }

    /// Whether any predecessor imposed a wait at all. A network with no lag
    /// edges, or only zero-lag ones, must not cost a clock read.
    fn owes_anything(&self) -> bool {
        self.largest > 0
    }
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
async fn await_research(
    act: &dyn Actuator,
    bot: BotId,
    net: &ActionNetwork,
    id: ActionId,
    log: &Mutex<ExecutionLog>,
) {
    let Some(action) = net.action(id) else {
        return;
    };
    for tech in action.pre.iter().filter_map(|c| match c {
        Condition::Researched(tech) => Some(tech.as_str()),
        _ => None,
    }) {
        let deadline = tokio::time::Instant::now() + RESEARCH_SETTLE_BUDGET;
        // Lazily, exactly as in `await_preds`: a technology the game already
        // has costs one question and no entry. This wait is bounded by
        // `RESEARCH_SETTLE_BUDGET` and so can never be a long silence -- which
        // is why being able to *rule it out* is what it is worth.
        let mut waiting: Option<WaitGuard> = None;
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
            if waiting.is_none() {
                waiting = Some(WaitGuard::enter(
                    log,
                    WaitKey::Action(id),
                    bot,
                    WaitKind::Research {
                        tech: tech.to_string(),
                    },
                ));
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

/// Wait until the machine time every lag edge names has actually passed.
///
/// # Which moment the clock starts from
///
/// The deadline is absolute and comes from [`LagWait`]: `finish(pred) + lag`,
/// where `finish(pred)` is the `game.tick` the game reported for the
/// predecessor's own outcome. It is emphatically **not** `now + lag`. Every
/// tick the bot spent between its predecessor settling and arriving here — the
/// other work the schedule gave it, and the walk that got it here — is machine
/// time that has already elapsed, and is subtracted rather than added. A bot
/// that arrives after the deadline waits **zero**. See [`LagWait`] for what
/// counting from arrival cost, and note that this is what makes the
/// walk-before-wait ordering (a `Walk` step precedes the `Act` step that waits)
/// harmless: the walk is now spent *inside* the lag rather than before it.
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
/// # What this costs at speed -- measured, not assumed
///
/// A wall-clock sleep is worth `60 * speed` ticks per second, so the obvious
/// worry is that at 10x every sleep overshoots by ten times as many ticks.
/// It does not, because the sleep is *sized* at that speed too: the same
/// 960-tick lag (`insert 4 copper-ore` -> `take 4 copper-plate`, four smelts
/// plus one cycle of headroom) was served in **962 ticks at 5x and 965 at
/// 10x**, and its five-plate sibling in 1,155 at 1x against 1,152 owed -- an
/// overshoot of two to five ticks at every speed, one RCON round trip's worth
/// (`run-1788582657-14978`, `run-1788614064-08543`, `run-1788614294-64261`,
/// 2026-09-05). The speed tax those runs showed (+4% at 5x, +9% at 10x on
/// the same plan) was entirely *before* the first dispatch: the planner is
/// wall-clock work and the game ran through it at speed. `goal.plan` now
/// stops the clock while it thinks; this loop was never the mechanism.
///
/// An actuator with no clock ([`Actuator::game_tick`] returning `None`) keeps
/// the old wall-clock wait, which is the honest fallback: it is the best
/// available claim when nobody can be asked what time it is.
async fn wait_out_lag(act: &dyn Actuator, wait: LagWait) {
    // A speed the actuator cannot report falls back to normal speed rather
    // than aborting the run over a missing nicety: a wrong-but-finite wait is
    // recoverable (recovery re-checks preconditions before dispatching the
    // next action), a hung run is not. It is only an estimate now either way.
    let speed = act.game_speed().await.unwrap_or(1.0);
    let Some(started) = act.game_tick().await.ok().flatten() else {
        // No clock: an absolute deadline is a number with nothing to compare
        // it against, so this falls all the way back to the arrival-based
        // wait. It is the old behaviour and it is wrong in the same way, but
        // it is the only claim available and it errs towards waiting.
        tokio::time::sleep(ticks_to_wall_clock(wait.largest, speed)).await;
        return;
    };
    // Anchored edges bring their own deadline; unanchored ones can only be
    // served from now. Whichever is later governs.
    let deadline = wait
        .deadline
        .unwrap_or(started)
        .max(started.saturating_add(u64::from(wait.unanchored)));

    // The whole point of the change: what is owed is what is *left*, and a bot
    // that arrived after the deadline owes nothing and dispatches at once.
    let owed = deadline.saturating_sub(started);
    if owed == 0 {
        return;
    }
    // The budget is a multiple of the wait actually being served, not of the
    // raw lag: a bot arriving with 200 of a 12,240-tick lag still to run gets
    // 200 ticks of chase, not 12,240.
    let mut estimate = u32::try_from(owed).unwrap_or(Ticks::MAX);
    let mut budget = estimate.saturating_mul(LAG_CHASE_BUDGET);
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
    expected_ticks: u32,
) -> Result<ActionTicks, ActuatorFailure> {
    match kind {
        ActionKind::Mine { pos, item, count } => {
            act.mine(bot, item.as_str(), pos.clone(), *count).await
        }
        // The same actuator call: the mod's `rcon_action_start_mining` finds
        // its target with `surface.find_entity(name, position)` and asks only
        // that it be `minable`, so an ore tile and a tree take the same route
        // to the game. What differs is which field carries the name -- see
        // `ActionKind::Chop`.
        ActionKind::Chop {
            pos, entity, count, ..
        } => act.mine(bot, entity.as_str(), pos.clone(), *count).await,
        ActionKind::Craft { item, count } => act.craft(bot, item.as_str(), *count).await,
        ActionKind::Place { entity } => {
            act.place(
                bot,
                &entity.name,
                entity.position.clone(),
                entity.direction,
                entity.underground_half,
            )
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
        ActionKind::Research { tech } => act.research(tech, expected_ticks).await,
        ActionKind::SetRecipe {
            pos,
            entity,
            recipe,
        } => {
            act.set_recipe(bot, entity, pos.clone(), recipe.as_str())
                .await
        }
        // The scheduler has already walked the bot here to satisfy this
        // action's own `AtPosition` precondition -- see
        // `ActionKind::Evacuate`'s own doc for why there is nothing else for
        // this dispatch to ask the game for. The call is not a formality:
        // without it this action would never reach a verdict, and
        // `EventKind::ActionSettled`'s own rule ("every attempt that reaches
        // a verdict gets exactly one of these") would have nothing to record
        // against the dispatch the scheduler already wrote down.
        ActionKind::Evacuate { to } => act.walk(bot, to.clone(), 0.0, EVACUATE_RADIUS).await,
        // Same shape as `Evacuate` and for the same reason: the scheduler has
        // already walked the bot here to satisfy this action's own
        // `AtPosition`, and there is nothing else to ask the game for --
        // charting is the *engine's* response to a character standing
        // somewhere new, not a verb anybody calls. The re-confirmation walk is
        // what gives the action a verdict to record.
        //
        // The radius is the planner's own `SURVEY_RADIUS` rather than
        // `EVACUATE_RADIUS`: a survey buys chunks, and half a chunk of slack
        // costs no ground. Asking for evacuation's precision here would fail
        // walks over float noise for no gain.
        ActionKind::Survey { to } => {
            // **Generate before walking, or the walk cannot happen at all.**
            // A bot cannot path into ungenerated ground -- measured on seed
            // 31337, where x=200 is reached and x=300 through x=600 all fail
            // with `failed to path find` -- so a survey aimed at unexplored
            // ground is refused before it is dispatched unless the ground is
            // made first. This is the one call in the executor a human player
            // could not make, it is clamped mod-side to the reveal a character
            // gets by standing somewhere, and `Actuator::ground_generated`
            // counts it into `EventKind::BatchProgress` so a run discloses it.
            //
            // A failure here is **not** fatal to the action: the ground may
            // already exist, and the walk is the thing that decides whether
            // the survey worked. Logged and stepped past, so a server whose
            // BotBridge predates this verb still walks its surveys over ground
            // it already has rather than failing every one of them.
            // A failure here is **not** fatal to the action: the ground may
            // already exist, and the walk is what decides whether the survey
            // worked. It is not silent either -- this crate has no logger by
            // design, so the failure is *counted*
            // (`Actuator::ground_generated`'s third number) and reaches
            // `EventKind::BatchProgress`. A run against a server whose
            // BotBridge predates this verb therefore reads as "asked N times,
            // failed N times, made 0 chunks" rather than as a run that simply
            // found no new ground.
            let _ = act
                .generate_chunks(to, factorio_bot_planner::method::scout::SURVEY_CHUNK_RADIUS)
                .await;
            act.walk(
                bot,
                to.clone(),
                0.0,
                factorio_bot_planner::method::scout::SURVEY_RADIUS,
            )
            .await
        }
    }
}

/// How close to the chosen escape tile counts as "there", for
/// [`ActionKind::Evacuate`]'s own re-confirmation walk.
///
/// Half a tile: the escape target is a tile centre on the pathfinder's own
/// grid (`factorio_bot_core::graph::enclosure::CELL` is one tile), so anywhere
/// inside that tile is the tile that was proven safe. The game's own pathing
/// does not promise to land a character on an exact float, and asking for
/// tighter than this would risk the walk itself being refused over noise
/// smaller than a character's own collision box.
const EVACUATE_RADIUS: f64 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::factorio::rcon::DestinationFull;
    use factorio_bot_core::record::map::{EntitySnapshot, Placement};
    use factorio_bot_core::types::{FactorioEntity, Position};
    use factorio_bot_planner::{Action, Actor, Condition, Effect, InventorySlot};
    use mockall::mock;
    use std::time::Duration;

    mock! {
        pub Act {}
        #[async_trait::async_trait]
        impl Actuator for Act {
            async fn walk(&self, bot: BotId, to: Position, min_radius: f64, radius: f64) -> Result<ActionTicks, ActuatorFailure>;
            async fn mine(&self, bot: BotId, item: &str, at: Position, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn place(&self, bot: BotId, item: &str, at: Position, direction: u8, underground_half: Option<factorio_bot_core::blueprint::UndergroundHalf>) -> Result<ActionTicks, ActuatorFailure>;
            async fn insert(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn remove(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn research(&self, tech: &str, expected_ticks: u32) -> Result<ActionTicks, ActuatorFailure>;
            async fn set_recipe(&self, bot: BotId, entity: &str, at: Position, recipe: &str) -> Result<ActionTicks, ActuatorFailure>;
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

    /// **The executor link.** `perform`'s `ActionKind::Place` arm used to
    /// drop `entity.underground_half` on the floor -- `act.place(bot,
    /// &entity.name, entity.position.clone(), entity.direction)`, four
    /// arguments, the half nowhere in them -- so a planned underground-belt
    /// pair, whichever half the planner put on the entity, would have reached
    /// `Actuator::place` (and from there `rcon_place_entity`) as `None` every
    /// time: exactly the untyped double-placement the whole task exists to
    /// prevent. This drives `perform` directly (no `Schedule`, no `run()`,
    /// nothing else in between) with a `Place` action for the `Input` half
    /// and asserts the mock actuator's `place()` is called with that same
    /// `Some(UndergroundHalf::Input)`, not `None` and not `Output`.
    #[tokio::test]
    async fn a_place_actions_underground_half_reaches_the_actuator() {
        use factorio_bot_core::blueprint::UndergroundHalf;

        let mut act = MockAct::new();
        act.expect_place()
            .times(1)
            .withf(|_bot, item, _at, _direction, half| {
                item == "underground-belt" && *half == Some(UndergroundHalf::Input)
            })
            .returning(|_, _, _, _, _| Ok(some_ticks()));

        let entity = FactorioEntity {
            name: "underground-belt".to_string(),
            underground_half: Some(UndergroundHalf::Input),
            ..Default::default()
        };
        let kind = ActionKind::Place {
            entity: Box::new(entity),
        };

        perform(&act, BotId(1), &kind, 0)
            .await
            .expect("the mock actuator accepted the placement");
    }

    /// The other side of the same guarantee: an ordinary placement -- no
    /// underground half at all -- must reach the actuator as `None`, not as
    /// some default `Some(_)` a careless refactor of the arm above could
    /// introduce.
    #[tokio::test]
    async fn an_ordinary_place_action_carries_no_underground_half() {
        let mut act = MockAct::new();
        act.expect_place()
            .times(1)
            .withf(|_bot, item, _at, _direction, half| item == "stone-furnace" && half.is_none())
            .returning(|_, _, _, _, _| Ok(some_ticks()));

        let entity = FactorioEntity {
            name: "stone-furnace".to_string(),
            ..Default::default()
        };
        let kind = ActionKind::Place {
            entity: Box::new(entity),
        };

        perform(&act, BotId(1), &kind, 0)
            .await
            .expect("the mock actuator accepted the placement");
    }

    // ---------------------------------------------------------------- fixtures

    fn mine_action_id() -> ActionId {
        ActionId(0)
    }

    /// The action scheduled right after the mine in `walk_then_mine_fixture`.
    /// Exists so `a_failed_action_abandons_the_step_that_depends_on_it` has something
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
                min_radius: 0.0,
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

    fn third_action_id() -> ActionId {
        ActionId(2)
    }

    /// Two predecessors whose lags belong to two different clocks.
    ///
    /// Action 0 (bot 0) settles immediately and carries `early_lag`; action 1
    /// (bot 1) is whatever the script makes it and carries `late_lag`; action 2
    /// (bot 2) waits on both and has nothing else to do, so it arrives the
    /// instant the second of them settles.
    ///
    /// The point is that `early_lag` and `late_lag` are anchored to *different*
    /// finish ticks. Any implementation that collapses the two lags before
    /// pairing each with its own predecessor — `max(finish) + max(lag)`, or the
    /// original `arrival + max(lag)` — gets a different answer from
    /// `max(finish(pred) + lag(pred))`, which is the planner's.
    fn two_pred_fixture(early_lag: Ticks, late_lag: Ticks) -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        net.add(mine_of(third_action_id(), "coal"));
        net.link(first_action_id(), third_action_id(), early_lag);
        net.link(second_action_id(), third_action_id(), late_lag);

        let sched = Schedule {
            steps: vec![
                walk_step(BotId(0), 0, 60),
                act_step(first_action_id(), BotId(0), 60, 120),
                walk_step(BotId(1), 0, 60),
                act_step(second_action_id(), BotId(1), 60, 120),
                walk_step(BotId(2), 0, 60),
                act_step(third_action_id(), BotId(2), 120, 180),
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
        /// The moment `begin_crafting` would reach the game. Recorded
        /// separately from `CraftEnd` because a background craft's whole point
        /// is that the two can be far apart with the bot elsewhere in between.
        CraftStart(String),
        CraftEnd(String),
        ResearchStart(String),
        ResearchEnd(String),
    }

    /// What the actuator should do, keyed so a test can reverse the timing of
    /// two bots without touching anything else.
    #[derive(Clone)]
    struct Script {
        walk_delay_ms: BTreeMap<BotId, u64>,
        mine_delay_ms: BTreeMap<String, u64>,
        fail_walk: BTreeSet<BotId>,
        fail_mine: BTreeSet<String>,
        /// How long a craft of each recipe takes to settle, keyed by recipe.
        ///
        /// Zero by default, which is what every test written before background
        /// crafting existed sees. A craft that takes no time cannot show
        /// whether anything overlapped it, so the concurrency tests all set
        /// one — and set it far from every other delay in the same fixture, so
        /// "the mine started when the craft was queued" and "the mine started
        /// when the craft settled" are different numbers rather than the same
        /// one seen twice.
        craft_delay_ms: BTreeMap<String, u64>,
        fail_craft: BTreeSet<String>,
        /// How long the lab takes, for the same reason as `craft_delay_ms`.
        research_delay_ms: u64,
        /// What `RecordingAct::game_speed` reports. Defaults to `1.0`, not the
        /// derived `f64` default of `0.0` — a script nobody configures must
        /// behave exactly like normal speed, the same as every test written
        /// before this field existed.
        speed: f64,
        /// What `RecordingAct::take_placement` hands back, once, to whichever
        /// bot asks first. `None` for every test written before this field
        /// existed, matching `Actuator::take_placement`'s own default.
        placement: Option<Placement>,
        /// What `RecordingAct::take_destination_full` hands back, to whichever
        /// bot asks first. `None` for every test written before this field
        /// existed, matching `Actuator::take_destination_full`'s own default.
        destination_full: Option<DestinationFull>,
        /// The verdict `RecordingAct::insert` refuses with, if it refuses.
        ///
        /// The other half of `destination_full`: a transfer that moved less
        /// than asked because the *bot* did not hold it is a real failure and
        /// must stay one, and the pair of them is what a run record has to be
        /// able to tell apart.
        fail_insert: Option<String>,
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
                craft_delay_ms: BTreeMap::new(),
                fail_craft: BTreeSet::new(),
                research_delay_ms: 0,
                speed: 1.0,
                placement: None,
                destination_full: None,
                fail_insert: None,
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
            self.dispatched_at(&Dispatch::MineStart(item.to_string()))
        }

        /// Virtual time elapsed when `what` happened, or `None` if it never
        /// did.
        ///
        /// The concurrency tests assert on these numbers rather than on an
        /// order, because an order is satisfied by a run that overlapped
        /// nothing: "the mine came after the craft was queued" is true of the
        /// serialised executor too. A moment is not.
        fn dispatched_at(&self, what: &Dispatch) -> Option<Duration> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .find(|(d, _)| d == what)
                .map(|(_, at)| *at)
        }

        async fn delay(ms: u64) {
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }

        /// The tick this actuator's own clock reads, or `None` when the script
        /// gives it no clock.
        fn tick_now(&self) -> Option<u64> {
            self.script
                .ticks_per_second
                .map(|rate| (self.origin.elapsed().as_secs_f64() * rate) as u64)
        }

        /// What a dispatch reports as having happened *now*.
        ///
        /// With a clock, both halves are read off that same clock, so the
        /// finish tick a predecessor writes into the log and the tick
        /// `wait_out_lag` later compares it against come from one source. They
        /// used to not: every dispatch reported the fixed [`some_ticks`] pair
        /// while `game_tick` counted from zero, which is harmless for a wait
        /// measured from arrival and nonsense for one measured from a
        /// predecessor's finish.
        ///
        /// With no clock it stays [`some_ticks`], which is what every test
        /// written before this existed sees.
        fn ticks_now(&self) -> ActionTicks {
            match self.tick_now() {
                Some(t) => ActionTicks::new(Some(t), Some(t)),
                None => some_ticks(),
            }
        }
    }

    /// Reports [`RecordingAct::ticks_now`] for every dispatch: the script's own
    /// clock when it has one, and otherwise [`some_ticks`], whose numbers are
    /// far outside anything these fixtures schedule so a test can tell an
    /// observation from a plan value without knowing the schedule.
    #[async_trait::async_trait]
    impl Actuator for RecordingAct {
        async fn walk(
            &self,
            bot: BotId,
            _to: Position,
            _min_radius: f64,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.record(Dispatch::Walk(bot));
            Self::delay(self.script.walk_delay_ms.get(&bot).copied().unwrap_or(0)).await;
            if self.script.fail_walk.contains(&bot) {
                return Err(ActuatorError::Rejected("blocked".into()).into());
            }
            Ok(self.ticks_now())
        }

        async fn mine(
            &self,
            _bot: BotId,
            item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.record(Dispatch::MineStart(item.to_string()));
            let dispatched = self.tick_now();
            Self::delay(self.script.mine_delay_ms.get(item).copied().unwrap_or(0)).await;
            self.record(Dispatch::MineEnd(item.to_string()));
            if self.script.fail_mine.contains(item) {
                return Err(ActuatorError::Rejected("no ore".into()).into());
            }
            // A mine that took time reports the tick it started and the tick
            // it finished, not one tick twice: this is the only fixture verb
            // whose dispatch and reply can be far apart, and a lag edge behind
            // it is anchored to the *reply*.
            Ok(match (dispatched, self.tick_now()) {
                (Some(from), Some(to)) => ActionTicks::new(Some(from), Some(to)),
                _ => some_ticks(),
            })
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
            self.record(Dispatch::CraftStart(recipe.to_string()));
            let dispatched = self.tick_now();
            Self::delay(self.script.craft_delay_ms.get(recipe).copied().unwrap_or(0)).await;
            self.record(Dispatch::CraftEnd(recipe.to_string()));
            if self.script.fail_craft.contains(recipe) {
                return Err(ActuatorError::Rejected("the game started 0".into()).into());
            }
            // Two different ticks when the craft took time, for the same
            // reason `mine` reports two: a lag edge behind it is anchored to
            // the *reply*.
            Ok(match (dispatched, self.tick_now()) {
                (Some(from), Some(to)) => ActionTicks::new(Some(from), Some(to)),
                _ => some_ticks(),
            })
        }

        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
            _underground_half: Option<factorio_bot_core::blueprint::UndergroundHalf>,
        ) -> Result<ActionTicks, ActuatorFailure> {
            Ok(self.ticks_now())
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
            if let Some(why) = &self.script.fail_insert {
                return Err(ActuatorError::Rejected(why.clone()).at(self.ticks_now()));
            }
            Ok(self.ticks_now())
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
            Ok(self.ticks_now())
        }

        async fn research(&self, tech: &str, _: u32) -> Result<ActionTicks, ActuatorFailure> {
            self.record(Dispatch::ResearchStart(tech.to_string()));
            Self::delay(self.script.research_delay_ms).await;
            self.record(Dispatch::ResearchEnd(tech.to_string()));
            Ok(self.ticks_now())
        }

        async fn set_recipe(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _recipe: &str,
        ) -> Result<ActionTicks, ActuatorFailure> {
            Ok(self.ticks_now())
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
            Ok(self.tick_now())
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

        fn take_destination_full(&self, _bot: BotId) -> Option<DestinationFull> {
            self.script.destination_full.clone()
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
            .returning(|_, _, _, _| Ok(some_ticks()));
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
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
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
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| {
            Err(ActuatorError::Rejected("no ore here".into())
                .at(ActionTicks::new(Some(900_101), Some(900_140))))
        });
        // The craft does not depend on the mine, so since a failed action
        // costs only its dependents it is still dispatched. Nothing here is
        // about the craft; it just has to be allowed to happen.
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

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
        act.expect_walk().returning(|_, _, _, _| {
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
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into()).into()));
        // Independent of the mine, so it still runs. See
        // `a_failed_action_does_not_take_down_an_independent_later_step`.
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

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
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| {
            Err(ActuatorError::NoVerdict("unreadable action_completed status".into()).into())
        });
        // The craft does not depend on the mine, so it is dispatched: an
        // outcome nobody knows is no basis for running the step that *depended*
        // on it, and this one does not. See
        // `a_lost_action_still_abandons_the_step_that_depends_on_it`.
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

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
    }

    /// **A failed action costs that action, not the batch.**
    ///
    /// `run-1788663566-25023`: one transport-belt refused because a bot of
    /// ours was standing on its tile ended the batch and left ~50 of a
    /// 179-entity block never dispatched. The mine and the craft in this
    /// fixture share no edge, so nothing about the craft was made untrue by
    /// the mine failing, and the bot has no business skipping it.
    ///
    /// **The fixture and the code changed in the same task**, which the
    /// fixtures note warns about — so the pair matters more than either half.
    /// This one asserts an independent step *runs*;
    /// `a_failed_action_abandons_the_step_that_depends_on_it` asserts a
    /// dependent one does not, over the identical fixture plus one edge. A
    /// change that made the loop ignore dependencies would pass this and fail
    /// that.
    #[tokio::test]
    async fn a_failed_action_does_not_take_down_an_independent_later_step() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into()).into()));
        act.expect_craft()
            .times(1)
            .returning(|_, _, _| Ok(some_ticks()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![mine_action_id()]);
        assert_eq!(
            log.status(craft_action_id()),
            Status::Success,
            "the craft depends on nothing that failed, so losing it would be \
             fifty entities lost to one occupied tile"
        );
    }

    /// The other half of the rule: what the network says depends on the
    /// failure is abandoned, and by the mechanism that already existed for it
    /// — `await_preds` reading the predecessor's own signal.
    ///
    /// The same fixture as
    /// `a_failed_action_does_not_take_down_an_independent_later_step` with one
    /// edge added, so the edge is the only difference between running and not.
    /// A belt nobody built is a real dependency for the inserter that feeds
    /// it, and this is that case.
    #[tokio::test]
    async fn a_failed_action_abandons_the_step_that_depends_on_it() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into()).into()));
        act.expect_craft().times(0);

        let (mut net, sched) = walk_then_mine_fixture();
        net.link(mine_action_id(), craft_action_id(), 0);
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![mine_action_id()]);
        assert_eq!(
            log.status(craft_action_id()),
            Status::Pending,
            "abandoned, not attempted: nothing was dispatched, so the log has \
             nothing to record. `expect_craft().times(0)` above is what says \
             it did not run; the verdict lives on the signal, and \
             `an_abandoned_step_still_releases_a_bot_waiting_behind_it` is \
             what proves the signal was published"
        );
    }

    /// A **lost** predecessor abandons its dependent too. `Lost` is not
    /// `Failed` — recovery counts one and not the other — but neither is a
    /// precondition anybody can vouch for.
    #[tokio::test]
    async fn a_lost_action_still_abandons_the_step_that_depends_on_it() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| {
            Err(ActuatorError::NoVerdict("unreadable action_completed status".into()).into())
        });
        act.expect_craft().times(0);

        let (mut net, sched) = walk_then_mine_fixture();
        net.link(mine_action_id(), craft_action_id(), 0);
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        assert_eq!(log.status(mine_action_id()), Status::Lost);
        assert_eq!(log.status(craft_action_id()), Status::Pending);
    }

    /// **The deadlock this change could have introduced, asserted against.**
    ///
    /// A bot used to abandon its whole remaining slice in one sweep
    /// (`abandon_rest`), which published a verdict for every action it would
    /// never reach. Continuing past a failure retires that sweep, so each
    /// abandoned step has to publish its own verdict as it is reached — and a
    /// step that abandons *silently* strands whoever waits on it forever.
    ///
    /// Three actions in a chain across two bots: bot 0's mine fails, bot 0's
    /// craft waits on the mine, and bot 1's second mine waits on the craft. If
    /// the craft published nothing, bot 1 would wait out the 60-second
    /// deadline `within_deadline` imposes.
    #[tokio::test(start_paused = true)]
    async fn an_abandoned_step_still_releases_a_bot_waiting_behind_it() {
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(Action {
            id: second_action_id(),
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
        net.add(mine_of(third_action_id(), "copper-ore"));
        net.link(first_action_id(), second_action_id(), 0);
        net.link(second_action_id(), third_action_id(), 0);

        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                act_step(second_action_id(), BotId(0), 60, 90),
                act_step(third_action_id(), BotId(1), 90, 150),
            ],
            makespan: 150,
        };

        let mut script = Script::default();
        script.fail_mine.insert("iron-ore".to_string());
        let act = RecordingAct::new(script);
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.failed(), vec![first_action_id()]);
        assert_eq!(
            act.mine_starts().len(),
            1,
            "only the failing mine was dispatched: the copper mine two hops \
             behind it must not run on a precondition nobody can vouch for. \
             Got {:?}",
            act.mine_starts()
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
                eq(0.0),
                eq(WALK_FIXTURE_RADIUS),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_walk()
            .with(eq(BotId(1)), eq(Position::new(5., 5.)), eq(0.0), eq(4.0))
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
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
                min_radius: 0.0,
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

    /// The plan's tolerance is what the actuator is told to walk to — **both
    /// halves of it**.
    ///
    /// `Condition::AtPosition` has always carried a radius and the scheduler
    /// has always used it to *estimate* travel; what it did not do was hand it
    /// on. So a step meaning "stand within 3 of the ore" arrived at the game
    /// as "stand on the ore", and for a place/insert/remove — whose target is
    /// the entity's own tile — the pathfinder could not answer at all.
    ///
    /// `min_radius` then repeated the defect. The scheduler lowered a
    /// placement's annulus to a single named coordinate with a zero radius,
    /// and this dispatch was handed no inner bound at all — so
    /// `approach_radius(0.0)` returned its 0.5 floor and the walk became a
    /// half-tile disc centred on ground the plan had explicitly forbidden.
    /// Run 10's walk to `[-61.72941750255542, 11]` is that, refused
    /// pre-dispatch for ending "inside a collision box".
    ///
    /// The second walk here is a real annulus — the `stone-furnace` clearance
    /// against a build reach of 10, the exact numbers of run 10's placement —
    /// so a dispatch that dropped `min_radius` again would arrive as
    /// `(0.0, 10.0]` and fail the matcher. `.with` matchers rather than a
    /// captured value, so no constant can satisfy them.
    #[tokio::test]
    async fn each_walk_is_dispatched_with_its_own_steps_bounds() {
        use mockall::predicate::eq;
        /// `PlanState::placement_clearance("stone-furnace")`: half the
        /// furnace's collision-box diagonal plus half the character's.
        const STONE_FURNACE_CLEARANCE: f64 = 1.2705824974445776;
        let mut act = MockAct::new();
        act.expect_walk()
            .with(
                eq(BotId(0)),
                eq(Position::new(10., 10.)),
                eq(0.0),
                eq(WALK_FIXTURE_RADIUS),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_walk()
            .with(
                eq(BotId(1)),
                eq(Position::new(-63., 11.)),
                eq(STONE_FURNACE_CLEARANCE),
                eq(10.0),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_mine().returning(|_, _, _, _| Ok(some_ticks()));
        act.expect_craft().returning(|_, _, _| Ok(some_ticks()));

        let (net, mut sched) = walk_then_mine_fixture();
        sched.steps.push(ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(-63., 11.),
                min_radius: STONE_FURNACE_CLEARANCE,
                radius: 10.0,
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
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("blocked".into()).into()));
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
            .returning(|_, _, _, _| Ok(ActionTicks::new(Some(800_010), Some(800_910))));
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
            .returning(|_, _, _, _| Ok(ActionTicks::UNKNOWN));
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
        // Both scheduled actions are missing from the network, and each is now
        // its own failure: the bot no longer stops at the first one, so the
        // second is reached and judged rather than silently skipped.
        assert_eq!(log.failed(), vec![mine_action_id(), craft_action_id()]);
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
        perform(&act, BotId(0), &kind, 0).await.unwrap();
    }

    /// The dispatch arm's whole contract: the machine's **name**, its
    /// **position** and the **recipe** all reach the actuator unchanged.
    ///
    /// Three separate strings, and two of them are recipe-shaped. An arm that
    /// passed the recipe where the entity name belongs would set a recipe on
    /// nothing -- `surface.find_entity` would answer `nil` and the mod would
    /// refuse -- but the reverse mistake is the dangerous one, because
    /// `assembling-machine-1` is also a recipe name and the game would take
    /// it.
    #[tokio::test]
    async fn set_recipe_dispatches_the_machine_and_the_recipe_the_action_named() {
        use mockall::predicate::eq;

        let mut act = MockAct::new();
        act.expect_set_recipe()
            .with(
                eq(BotId(0)),
                eq("assembling-machine-1"),
                eq(Position::new(12.5, 8.5)),
                eq("automation-science-pack"),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(some_ticks()));

        let kind = ActionKind::SetRecipe {
            pos: Position::new(12.5, 8.5),
            entity: "assembling-machine-1".into(),
            recipe: "automation-science-pack".into(),
        };
        perform(&act, BotId(0), &kind, 0).await.unwrap();
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
        // Nothing links action 99 to action 0, so bot 0 goes on to run it --
        // and bot 1, which waits on action 0, is released by its success
        // rather than by an abandonment. The name still holds: the point was
        // always that bot 1 does not hang on a signal with no writer.
        assert_eq!(log.status(first_action_id()), Status::Success);
        assert_eq!(log.status(second_action_id()), Status::Success);
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

    // ------------------------------------- where the lag clock starts from
    //
    // The four tests above pin the *duration* of a lag wait. These pin its
    // *origin*, which is what `run-1788465258-49050` proved nothing was
    // holding: `cross_bot_fixture` with no delays has the dependent bot arrive
    // on the exact tick its predecessor settled, so arrival-based and
    // finish-based timing produce identical numbers there and a wait counted
    // from the wrong moment passes every one of them.
    //
    // All three run at 60 ticks/second, so a tick is a millisecond of virtual
    // wall clock at 1/60th scale and the arithmetic below is exact rather than
    // approximate: under `start_paused` tokio advances straight to each timer.

    /// 60 ticks per second — a game keeping up exactly, so `n` ticks is `n/60`
    /// seconds and every deadline in these three tests is a round number.
    fn on_time_clock() -> Script {
        Script {
            ticks_per_second: Some(60.0),
            ..Default::default()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_lag_clock_starts_when_the_predecessor_finished_not_when_the_bot_arrived() {
        // 600 ticks (10 s) of machine time, and a bot that spends 4 s of it
        // walking to the site — which is exactly what the schedule intends a
        // lag to be overlapped with. The furnace started when the coal went
        // in, not when the bot came back, so 240 of the 600 ticks are already
        // spent on arrival and only 360 are owed.
        //
        // Counting from arrival instead dispatches at 14 s and calls the extra
        // 4 s machine time. It is not: nothing is running that was not already
        // running, so those 4 s are the parallelism the schedule created being
        // handed straight back.
        let script = Script {
            walk_delay_ms: [(BotId(1), 4_000)].into_iter().collect(),
            ..on_time_clock()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(600);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let iron = act.mine_started_at("iron-ore").expect("iron-ore mined");
        let copper = act.mine_started_at("copper-ore").expect("copper-ore mined");
        assert_eq!(iron, Duration::ZERO, "the predecessor settles at tick 0");
        // The whole assertion, and it is arithmetic rather than an inequality:
        // 600 ticks after the predecessor finished, not 600 after the bot got
        // there. "Waits less than before" would also be satisfied by shaving an
        // arbitrary amount off, which is why the number is pinned.
        assert!(
            (Duration::from_millis(9_950)..=Duration::from_millis(10_050)).contains(&copper),
            "the take is due 600 ticks after the predecessor settled at tick 0, \
             i.e. at 10s; dispatching at {copper:?} means the lag was counted \
             from somewhere else (4s walk + 10s lag = 14s is the arrival-based \
             answer)"
        );
        // And the part of the lag that had *not* elapsed is still served: a
        // fix that simply stopped waiting would pass the bound above only by
        // accident of these numbers, and would dispatch at 4s.
        assert!(
            copper > Duration::from_millis(4_000),
            "the 360 ticks still owed on arrival must still be waited out, \
             but the take went out at {copper:?}, which is when the bot arrived"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_arriving_after_the_deadline_waits_not_at_all() {
        // The live case from `run-1788465258-49050`. Bot 1 reached the cell
        // 5,131 ticks *after* the plates were due and then waited the full
        // 12,240 ticks over again, standing still next to a fuel-exhausted
        // furnace while cumulative iron-plate production stayed flat.
        //
        // Here: a 600-tick lag and a 12 s (720-tick) walk. The deadline passed
        // 120 ticks before the bot arrived, so the correct wait is zero and the
        // take goes out the moment the walk returns.
        let script = Script {
            walk_delay_ms: [(BotId(1), 12_000)].into_iter().collect(),
            ..on_time_clock()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(600);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let copper = act.mine_started_at("copper-ore").expect("copper-ore mined");
        assert!(
            copper < Duration::from_millis(12_050),
            "the deadline (tick 600) had passed 120 ticks before the bot \
             arrived (tick 720), so the correct wait is zero and the take is \
             due at 12s; it went out at {copper:?}, which is a second, \
             redundant lag for machine time already spent"
        );
        assert!(
            copper >= Duration::from_millis(12_000),
            "the take cannot precede the walk that gets the bot there: {copper:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn each_lag_is_charged_to_the_predecessor_it_belongs_to() {
        // `await_preds` used to collapse the lags of all predecessors into one
        // `max_lag` before waiting, which loses which predecessor each lag came
        // from. That is harmless when the wait starts at arrival — there is
        // only one clock then — and wrong the moment each lag is anchored to
        // its own predecessor's finish tick.
        //
        // Action 2 waits on action 0 (settles at tick 0, lag 900 -> due 900)
        // and action 1 (settles at tick 540, lag 60 -> due 600). The planner's
        // answer is max(900, 600) = 900, i.e. 15s. Every collapsing variant
        // gives something else: max(finish) + max(lag) = 1440 (24s), the old
        // arrival + max(lag) = 1440 (24s), and max(finish) + min(lag) = 600
        // (10s). Only the correct pairing lands on 15s.
        let script = Script {
            mine_delay_ms: [("copper-ore".to_string(), 9_000)].into_iter().collect(),
            ..on_time_clock()
        };
        let act = RecordingAct::new(script);
        let (net, sched) = two_pred_fixture(900, 60);
        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        let coal = act.mine_started_at("coal").expect("coal mined");
        assert!(
            (Duration::from_millis(14_950)..=Duration::from_millis(15_050)).contains(&coal),
            "action 2 is due at max(0 + 900, 540 + 60) = tick 900, i.e. 15s; \
             {coal:?} means the lags were maxed before being paired with the \
             predecessors they belong to"
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

    /// One `insert` for one bot, so the two runs below differ in exactly one
    /// thing: what the game said about the transfer.
    fn one_insert_fixture() -> (ActionNetwork, Schedule, ActionId) {
        let id = ActionId(0);
        let mut net = ActionNetwork::new();
        net.add(Action {
            id,
            kind: ActionKind::Insert {
                pos: Position::new(-7.0, -54.5),
                entity: "boiler".into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: 17,
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "top the boiler up with 17 coal".into(),
        });
        let sched = Schedule {
            steps: vec![act_step(id, BotId(0), 0, 30)],
            makespan: 30,
        };
        (net, sched, id)
    }

    /// **The fix.** An insert the destination had no room for is a success, and
    /// the run record says *why* it moved less than it was asked to.
    ///
    /// This is the shape that killed `run-1788432181-42528` at tick 211399:
    /// `boiler_coal` sizes a top-up from demand alone -- nothing in
    /// `FactorioWorld` reports a fuel level -- so it asked for 17 coal into a
    /// boiler already holding 47, the fuel slot took 3, and the executor read
    /// the mod's honest arithmetic as a verdict of failure and abandoned the
    /// rest of that bot's chain.
    ///
    /// `RconActuator::insert` cannot be driven here without a live game, but
    /// `Actuator::take_destination_full`'s contract is exactly what it hands
    /// back once the reply has been judged (`judge_transfer_reply`,
    /// `crates/core/src/factorio/rcon.rs`, which has its own tests against the
    /// real `control.lua`). What this proves is the settle wiring: that the
    /// fact reaches the `Attempt` it belongs to instead of being dropped.
    #[tokio::test]
    async fn an_insert_whose_destination_was_full_succeeds_and_says_so() {
        let script = Script {
            destination_full: Some(DestinationFull {
                item: "coal".to_string(),
                asked: 17,
                moved: 3,
                holds: 50,
            }),
            ..Default::default()
        };
        let act = RecordingAct::new(script);
        let (net, sched, id) = one_insert_fixture();

        let log = run(&act, &sched, &net).await.expect("the run should start");

        assert_eq!(
            log.status(id),
            Status::Success,
            "a destination with no room left is a goal that holds, not a failure"
        );
        let a = log.attempt(id).expect("the insert was attempted");
        assert_eq!(
            a.error.as_deref(),
            Some("destination full: moved 3 of 17 coal, which now holds 50"),
            "a success that did not deliver everything must say so, or a run \
             whose every top-up moves 3 of 17 looks exactly like one whose \
             top-ups all move 17"
        );
    }

    /// The other half, and the one that must **not** move: a bot that could not
    /// deliver what the plan believed it held is a real failure.
    #[tokio::test]
    async fn an_insert_the_bot_was_too_short_to_make_still_fails() {
        let script = Script {
            fail_insert: Some(
                "game rejected the command: [\"cannot insert 17x coal, because \
                 player #1 only has 3. clamping...\"]"
                    .to_string(),
            ),
            ..Default::default()
        };
        let act = RecordingAct::new(script);
        let (net, sched, id) = one_insert_fixture();

        let log = run(&act, &sched, &net).await.expect("the run should start");

        assert_eq!(
            log.status(id),
            Status::Failed,
            "the plan's model of what the bot holds is wrong; treating that as \
             success is the silent divergence this project keeps being bitten by"
        );
        let a = log.attempt(id).expect("the insert was attempted");
        assert!(
            a.error
                .as_deref()
                .unwrap_or_default()
                .contains("only has 3"),
            "the verdict must survive to the record: got {:?}",
            a.error
        );
    }

    /// **The property that keeps this fixed.** The same action, the same plan,
    /// the same bot -- and a record a reader can tell apart afterwards.
    ///
    /// Both outcomes used to arrive as `Failed` with an indistinguishable
    /// *"moved 3 of 17"* message, which is how a planner sizing top-ups from
    /// demand stayed invisible for thirteen runs. Two facts separate them now
    /// and both are asserted, because either alone can be satisfied by
    /// accident: the **status**, which is the executor's verdict, and the
    /// **message**, which is what a person reads in `events.jsonl`.
    #[tokio::test]
    async fn the_record_tells_a_full_destination_from_a_short_source() {
        let (net, sched, id) = one_insert_fixture();

        let full = RecordingAct::new(Script {
            destination_full: Some(DestinationFull {
                item: "coal".to_string(),
                asked: 17,
                moved: 3,
                holds: 50,
            }),
            ..Default::default()
        });
        let short = RecordingAct::new(Script {
            fail_insert: Some(
                "game rejected the command: [\"cannot insert 17x coal, because \
                 player #1 only has 3. clamping...\"]"
                    .to_string(),
            ),
            ..Default::default()
        });

        let full_log = run(&full, &sched, &net).await.expect("run starts");
        let short_log = run(&short, &sched, &net).await.expect("run starts");

        let full_row = full_log.attempt(id).expect("attempted").clone();
        let short_row = short_log.attempt(id).expect("attempted").clone();

        assert_ne!(
            full_row.status, short_row.status,
            "a full destination and a short source are opposite outcomes and \
             must not share a verdict"
        );
        assert_ne!(
            full_row.error, short_row.error,
            "and they must not share a message either: the status alone says \
             nothing about how much moved, which is the number that names the \
             defect"
        );
        // Named rather than merely different, so this cannot be satisfied by
        // two rows that are both wrong -- in particular not by a full-
        // destination row that says nothing at all, which is `assert_ne!`'s
        // blind spot: `None` differs from a message without being one.
        assert_eq!(full_row.status, Status::Success);
        assert_eq!(short_row.status, Status::Failed);
        let full_says = full_row.error.expect("the success has to qualify itself");
        assert!(
            full_says.contains("3 of 17") && full_says.contains("full"),
            "the surviving row must name how much moved and why the rest did \
             not: got {full_says:?}"
        );
        let short_says = short_row
            .error
            .expect("the failure has to say what happened");
        assert!(
            short_says.contains("only has 3"),
            "and the failing row must carry the game's own verdict: got {short_says:?}"
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

    // ------------------------------------------- background actions (task C)
    //
    // The defect these pin: the executor gave each bot exactly one action at a
    // time and waited for it to settle. On `run-1788465258-49050`, 0 of 99
    // consecutive same-bot dispatch pairs overlapped, and bot 1 spent 6,575
    // ticks inside `craft` and 5,999 inside one `research` doing nothing else.
    // See `crate::occupancy`.
    //
    // Every fixture below uses three delays that are three different numbers
    // (10 ms, 1,000 ms, 2,000 ms), because the previous defect in this file
    // survived on a fixture where the two quantities under test happened to be
    // equal.

    /// How long the background action takes to settle in these fixtures.
    const BACKGROUND_MS: u64 = 1_000;
    /// How long the exclusive action takes. Deliberately not a divisor or
    /// multiple of anything else here.
    const EXCLUSIVE_MS: u64 = 10;

    /// A craft of `item` out of `ingredient`, shaped like the ones
    /// `crates/planner`'s craft method emits: a `HasItem` for the ingredient, a
    /// `LoseItem` for spending it, a `GainItem` for the product. The ingredient
    /// is not decoration — it is half of what decides whether this craft may
    /// overlap another of the bot's steps.
    fn craft_of(id: ActionId, item: &str, ingredient: &str) -> Action {
        Action {
            id,
            kind: ActionKind::Craft {
                item: item.into(),
                count: 1,
            },
            pre: vec![Condition::HasItem {
                who: Actor::Role,
                item: ingredient.into(),
                count: 1,
            }],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: ingredient.into(),
                    count: 1,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: item.into(),
                    count: 1,
                },
            ],
            duration: 30,
            pinned: None,
            label: format!("craft 1 {item}"),
        }
    }

    /// One bot: a craft of `product` from `ingredient`, then a mine of `ore`.
    /// `edge` puts a network dependency from the craft to the mine.
    ///
    /// Three knobs, because the three tests below differ in exactly one of
    /// them each: whether the two share an item, and whether the plan orders
    /// them.
    fn craft_then_mine_fixture(
        product: &str,
        ingredient: &str,
        ore: &str,
        edge: bool,
    ) -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        net.add(craft_of(first_action_id(), product, ingredient));
        net.add(mine_of(second_action_id(), ore));
        if edge {
            net.link(first_action_id(), second_action_id(), 0);
        }
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 30),
                act_step(second_action_id(), BotId(0), 30, 90),
            ],
            makespan: 90,
        };
        (net, sched)
    }

    fn background_script() -> Script {
        let mut script = Script::default();
        script
            .craft_delay_ms
            .insert("iron-gear-wheel".to_string(), BACKGROUND_MS);
        script
            .craft_delay_ms
            .insert("stone-furnace".to_string(), BACKGROUND_MS);
        script
            .mine_delay_ms
            .insert("copper-ore".to_string(), EXCLUSIVE_MS);
        script
            .mine_delay_ms
            .insert("stone".to_string(), EXCLUSIVE_MS);
        script
            .mine_delay_ms
            .insert("iron-ore".to_string(), EXCLUSIVE_MS);
        script.research_delay_ms = BACKGROUND_MS;
        script
    }

    /// The change itself: a craft goes into the player's crafting queue, and
    /// the bot mines while it runs.
    ///
    /// The craft takes 1,000 ms and the mine 10 ms; the mine is dispatched at
    /// **zero**, not at 1,000. Asserting the moment rather than the order is
    /// deliberate: "the mine came after the craft was queued" is true of the
    /// serialised executor as well.
    #[tokio::test(start_paused = true)]
    async fn a_bot_with_a_craft_in_flight_still_starts_an_exclusive_action() {
        let (net, sched) =
            craft_then_mine_fixture("iron-gear-wheel", "iron-plate", "copper-ore", false);
        let act = RecordingAct::new(background_script());

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(
            act.dispatched_at(&Dispatch::CraftStart("iron-gear-wheel".into())),
            Some(Duration::ZERO),
        );
        assert_eq!(
            act.mine_started_at("copper-ore"),
            Some(Duration::ZERO),
            "the mine must be dispatched while the craft is still in the \
             queue, not after it settles",
        );
        assert_eq!(
            act.dispatched_at(&Dispatch::CraftEnd("iron-gear-wheel".into())),
            Some(Duration::from_millis(BACKGROUND_MS)),
            "the craft still takes as long as it takes",
        );
        assert_eq!(log.status(first_action_id()), Status::Success);
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    /// The same for research, which is the other background verb and the one
    /// that cost bot 1 a single 5,999-tick stretch of standing still: the lab
    /// does the work and `Actuator::research` does not even take a bot.
    #[tokio::test(start_paused = true)]
    async fn a_bot_with_a_research_in_flight_still_starts_an_exclusive_action() {
        let mut net = ActionNetwork::new();
        net.add(Action {
            id: first_action_id(),
            kind: ActionKind::Research {
                tech: "automation".into(),
            },
            pre: vec![],
            eff: vec![Effect::Researched("automation".into())],
            duration: 6000,
            pinned: None,
            label: "research automation".into(),
        });
        net.add(mine_of(second_action_id(), "copper-ore"));
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 6000),
                act_step(second_action_id(), BotId(0), 6000, 6060),
            ],
            makespan: 6060,
        };
        let act = RecordingAct::new(background_script());

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(
            act.mine_started_at("copper-ore"),
            Some(Duration::ZERO),
            "the bot must not stand still for a lab it is not operating",
        );
        assert_eq!(
            act.dispatched_at(&Dispatch::ResearchEnd("automation".into())),
            Some(Duration::from_millis(BACKGROUND_MS)),
        );
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    /// The invariant that is *kept*: one exclusive action per bot. The same
    /// shape as the test above with the first verb changed from a craft to a
    /// mine, and the answer is the opposite one.
    #[tokio::test(start_paused = true)]
    async fn a_bot_with_a_mine_in_flight_does_not_start_a_second_exclusive_action() {
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "iron-ore"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                act_step(second_action_id(), BotId(0), 60, 120),
            ],
            makespan: 120,
        };
        let mut script = background_script();
        // The slow one first, so an executor that overlapped them would show
        // the second starting at zero.
        script
            .mine_delay_ms
            .insert("iron-ore".to_string(), BACKGROUND_MS);
        let act = RecordingAct::new(script);

        within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(
            act.mine_started_at("copper-ore"),
            Some(Duration::from_millis(BACKGROUND_MS)),
            "the character can only swing one pick",
        );
    }

    /// Ordering survives: queuing a craft early is only correct if whatever
    /// needs its product still waits for it to settle.
    ///
    /// Identical to `a_bot_with_a_craft_in_flight_still_starts_an_exclusive_action`
    /// except for the one network edge, and the mine moves from tick zero to
    /// the craft's settle. That pairing is the assertion: the edge is doing the
    /// work, not the exclusivity.
    #[tokio::test(start_paused = true)]
    async fn a_consumer_of_a_background_craft_still_waits_for_it_to_settle() {
        let (net, sched) =
            craft_then_mine_fixture("iron-gear-wheel", "iron-plate", "copper-ore", true);
        let act = RecordingAct::new(background_script());

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(
            act.mine_started_at("copper-ore"),
            Some(Duration::from_millis(BACKGROUND_MS)),
            "a dependency edge still means what it meant",
        );
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    /// The inventory hazard, and the narrow rule chosen for it: two of a bot's
    /// actions may only overlap when the items they name are disjoint.
    ///
    /// Same shape again, and again one thing changes — the craft now spends
    /// **stone**, which is exactly what the next step mines. No network edge
    /// joins them, so nothing but the footprint rule can hold the mine back,
    /// and it does.
    #[tokio::test(start_paused = true)]
    async fn a_step_that_shares_an_item_with_a_queued_craft_waits_for_it() {
        let (net, sched) = craft_then_mine_fixture("stone-furnace", "stone", "stone", false);
        let act = RecordingAct::new(background_script());

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(
            act.mine_started_at("stone"),
            Some(Duration::from_millis(BACKGROUND_MS)),
            "the plan sized this mine against an inventory the craft is about \
             to change, and the planner's arithmetic walks the bot's steps in \
             order",
        );
        assert_eq!(log.status(second_action_id()), Status::Success);
    }

    /// A background action that fails still stops its bot — and must not
    /// retract the verdict of a step that already succeeded while it ran.
    ///
    /// Bot 0 queues a craft that fails at 1,000 ms, and mines copper at 0 ms
    /// while it is in flight. Bot 1 is walking until 2,000 ms and only then
    /// looks at the copper mine it depends on. Before the `publish` guard,
    /// abandoning bot 0's slice from the craft onwards would have sent
    /// `Failed` over the mine's `Success` at 1,000 ms and bot 1 would have
    /// given up on work that was done.
    #[tokio::test(start_paused = true)]
    async fn a_failed_craft_does_not_retract_a_later_step_that_already_succeeded() {
        let mut net = ActionNetwork::new();
        net.add(craft_of(first_action_id(), "iron-gear-wheel", "iron-plate"));
        net.add(mine_of(second_action_id(), "copper-ore"));
        net.add(mine_of(third_action_id(), "coal"));
        net.link(second_action_id(), third_action_id(), 0);
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 30),
                act_step(second_action_id(), BotId(0), 30, 90),
                walk_step(BotId(1), 0, 60),
                act_step(third_action_id(), BotId(1), 90, 150),
            ],
            makespan: 150,
        };
        let mut script = background_script();
        script.fail_craft.insert("iron-gear-wheel".to_string());
        script.walk_delay_ms.insert(BotId(1), 2_000);
        let act = RecordingAct::new(script);

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.status(first_action_id()), Status::Failed);
        assert_eq!(
            log.status(second_action_id()),
            Status::Success,
            "the mine settled at 10 ms; the craft failed at 1,000",
        );
        assert_eq!(
            act.mine_started_at("coal"),
            Some(Duration::from_millis(2_000)),
            "bot 1 read the copper mine's signal after the craft failed, and \
             it must still have said `Success`",
        );
        assert_eq!(log.status(third_action_id()), Status::Success);
    }

    /// A bot's slice is not finished while something it queued is still in
    /// flight. Dropping those futures at the end of the loop would report a
    /// craft the game was about to finish as `Lost`, and release its waiters as
    /// abandoned.
    #[tokio::test(start_paused = true)]
    async fn a_craft_queued_as_the_last_step_is_still_waited_out() {
        let mut net = ActionNetwork::new();
        net.add(mine_of(first_action_id(), "copper-ore"));
        net.add(craft_of(
            second_action_id(),
            "iron-gear-wheel",
            "iron-plate",
        ));
        // A second bot waiting on the craft: the failure mode is not only a
        // wrong status, it is a waiter released as abandoned.
        net.add(mine_of(third_action_id(), "coal"));
        net.link(second_action_id(), third_action_id(), 0);
        let sched = Schedule {
            steps: vec![
                act_step(first_action_id(), BotId(0), 0, 60),
                act_step(second_action_id(), BotId(0), 60, 90),
                act_step(third_action_id(), BotId(1), 90, 150),
            ],
            makespan: 150,
        };
        let act = RecordingAct::new(background_script());

        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");

        assert_eq!(log.status(second_action_id()), Status::Success);
        assert_eq!(log.status(third_action_id()), Status::Success);
        assert_eq!(
            act.dispatched_at(&Dispatch::CraftEnd("iron-gear-wheel".into())),
            Some(Duration::from_millis(EXCLUSIVE_MS + BACKGROUND_MS)),
            "the craft was queued once the copper mine finished, and settles a \
             full craft later",
        );
    }

    // ------------------------------------------------- what a bot is waiting on
    //
    // The registry exists because three different silences used to look
    // identical from outside: a bot blocked on another bot, a bot serving a lag
    // deadline the plan asked for, and a bot holding a dispatch the game never
    // answered. In the run that prompted this, one action sat in the third
    // state for eleven minutes and the record could show frozen counters and a
    // bot id -- nothing that named the action or said what it was on.
    //
    // Every test below asserts a *named* state, never merely that something is
    // reported: "a field that is always absent" and "a field that is always
    // `Reply`" are the same defect, and only naming the state catches the
    // second.

    /// One wait for `key`, or a failure message listing what was actually
    /// there -- so a broken registry reports what it said instead of a bare
    /// `unwrap` on `None`.
    fn wait_for(log: &Mutex<ExecutionLog>, key: WaitKey) -> crate::log::Wait {
        let waiting = lock(log).waiting();
        waiting
            .iter()
            .find(|w| w.key == key)
            .cloned()
            .unwrap_or_else(|| {
                panic!("no wait recorded for {key:?}; the registry holds {waiting:#?}")
            })
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_blocked_on_another_bots_action_names_the_action_and_the_bot() {
        // Bot 0 is mining iron slowly; bot 1's copper mine has an edge from it
        // and cannot start. Before this registry both bots reported the same
        // nothing: bot 1's action was `Pending`, which is also what an action
        // nobody has reached says.
        let mut script = Script::default();
        script.mine_delay_ms.insert("iron-ore".to_string(), 1_000);
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the run should still be in progress");

        let blocked = wait_for(&progress, WaitKey::Action(second_action_id()));
        assert_eq!(
            blocked.bot,
            BotId(1),
            "the bot that is stuck, not the one it waits for"
        );
        assert_eq!(
            blocked.kind,
            WaitKind::Predecessor {
                on: first_action_id()
            },
            "the whole point is naming what it is waiting for"
        );
        assert_eq!(blocked.kind.name(), "predecessor");

        // And the bot that *is* working is reported as working, on the one
        // state that is unbounded. Asserting both in one test is deliberate:
        // a registry that reported everything as `Reply` would satisfy either
        // half alone.
        let working = wait_for(&progress, WaitKey::Action(first_action_id()));
        assert_eq!(working.bot, BotId(0));
        assert_eq!(working.kind, WaitKind::Reply);
        assert!(
            working.elapsed >= Duration::from_millis(400),
            "the elapsed is measured from the dispatch, not from whenever a \
             heartbeat first noticed it: got {:?}",
            working.elapsed
        );

        within_deadline(running)
            .await
            .expect("the run should have started");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_serving_a_lag_edge_says_so_rather_than_looking_like_a_lost_reply() {
        // The distinction that decides whether a run is stuck or obedient. A
        // 600-tick lag at 60 ticks/second is ten seconds during which bot 1
        // dispatches nothing and settles nothing -- exactly the shape of the
        // eleven-minute silence, and completely different in cause.
        let act = RecordingAct::new(Script::default());
        let (net, sched) = cross_bot_fixture(600);

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the lag wait should still be running");

        let lagging = wait_for(&progress, WaitKey::Action(second_action_id()));
        assert_eq!(lagging.bot, BotId(1));
        assert_eq!(lagging.kind.name(), "lag_deadline");
        let WaitKind::LagDeadline { largest_lag, .. } = lagging.kind else {
            panic!("expected a lag deadline, got {:?}", lagging.kind);
        };
        assert_eq!(
            largest_lag, 600,
            "the size of the wait the plan asked for, so a reader can see how \
             much of it is left"
        );

        within_deadline(running)
            .await
            .expect("the run should have started");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_queued_behind_its_own_background_craft_says_which_craft() {
        // The state that looks most like an idle bot: everything the plan
        // allowed has been dispatched, and the next step shares an item with a
        // craft the bot walked away from. It is not a plan edge, so nothing in
        // the network explains it either.
        let (net, sched) = craft_then_mine_fixture("stone-furnace", "stone", "stone", false);
        let act = RecordingAct::new(background_script());

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the craft should still be in flight");

        let blocked = wait_for(&progress, WaitKey::Action(second_action_id()));
        assert_eq!(
            blocked.kind,
            WaitKind::BackgroundConflict {
                on: first_action_id()
            },
            "named as the bot waiting on itself, not as a plan edge"
        );

        within_deadline(running)
            .await
            .expect("the run should have started");
    }

    #[tokio::test(start_paused = true)]
    async fn a_walking_bot_is_reported_as_walking_and_names_where() {
        // `EventKind::BatchProgress` already warns that a batch whose every bot
        // is walking reports `in_flight: 0` and reads as four idle bots. This
        // is the entry that answers it.
        let mut script = Script::default();
        script.walk_delay_ms.insert(BotId(0), 1_000);
        let act = RecordingAct::new(script);
        let (net, sched) = walk_then_mine_fixture();

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the walk should still be running");

        let walking = wait_for(&progress, WaitKey::Walk(BotId(0), 0));
        assert_eq!(walking.bot, BotId(0));
        assert_eq!(walking.kind.name(), "walk");
        let WaitKind::Walk { to } = &walking.kind else {
            panic!("expected a walk, got {:?}", walking.kind);
        };
        assert_eq!(*to, Position::new(10., 10.));

        within_deadline(running)
            .await
            .expect("the run should have started");
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_run_leaves_nothing_waiting() {
        // A stale entry is worse than no entry: it would report a bot as
        // blocked on a predecessor for as long as the log survives, and a
        // reader who believed it would diagnose a finished run as a stuck one.
        let act = RecordingAct::new(Script::default());
        let (net, sched) = cross_bot_fixture(0);
        let log = within_deadline(run(&act, &sched, &net))
            .await
            .expect("the run should have started");
        assert!(
            log.waiting().is_empty(),
            "the run is over, so nothing is waiting: {:#?}",
            log.waiting()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_dropped_mid_dispatch_leaves_nothing_waiting() {
        // The cancellation half, and the reason every wait is held by a guard
        // rather than by a `leave_wait` after the await. The same fixture as
        // `a_run_dropped_mid_dispatch_stops_reporting_its_actions_as_running`,
        // asking the same question of the registry that that one asks of the
        // statuses.
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
            assert!(
                !lock(&progress).waiting().is_empty(),
                "while the run is alive something really is waiting, or this \
                 test proves nothing"
            );
        }
        assert!(
            lock(&progress).waiting().is_empty(),
            "nobody is following this run any more, so nothing is waiting on \
             anything: {:#?}",
            lock(&progress).waiting()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_registry_and_the_statuses_cannot_disagree_about_what_is_in_flight() {
        // The cross-check that makes a broken registry detectable rather than
        // merely wrong. `Status::Running` and a `reply` wait are written by
        // different code paths for the same fact; a run where the counts
        // diverge is one where this stopped being maintained, which is how
        // every other observability hole in this project has failed.
        let mut script = Script::default();
        script.mine_delay_ms.insert("iron-ore".to_string(), 1_000);
        script.mine_delay_ms.insert("copper-ore".to_string(), 1_000);
        let act = RecordingAct::new(script);
        let (net, sched) = cross_bot_fixture(0);

        let progress = Mutex::new(ExecutionLog::default());
        let running = run_into(&act, &sched, &net, &progress);
        tokio::pin!(running);
        let stalled = tokio::time::timeout(Duration::from_millis(500), &mut running).await;
        assert!(stalled.is_err(), "the run should still be in progress");

        // Both counts read under one guard, which is then released before the
        // run is awaited again -- a `MutexGuard` may not be held across an
        // await, and this is the same `std::sync::Mutex` the executor writes
        // into from four bot futures at a time.
        let (in_flight, replies) = {
            let seen = lock(&progress);
            let in_flight = net
                .actions()
                .filter(|a| seen.status(a.id) == Status::Running)
                .count();
            let replies = seen
                .waiting()
                .iter()
                .filter(|w| w.kind == WaitKind::Reply)
                .count();
            (in_flight, replies)
        };
        assert_eq!(in_flight, 1, "exactly one action is dispatched here");
        assert_eq!(
            replies, in_flight,
            "every dispatched action must have a reply wait and vice versa"
        );

        within_deadline(running)
            .await
            .expect("the run should have started");
    }
}
