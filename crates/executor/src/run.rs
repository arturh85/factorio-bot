//! Run every bot's slice of a `Schedule` against an `Actuator`.
//!
//! Bots run concurrently and wait on each other through one
//! `tokio::sync::watch` channel per action, replacing the old executor's
//! 100 ms poll loop (`crates/core/src/plan/execute.rs:79`) with a completion
//! signal per action.

use crate::actuator::{ActionTicks, Actuator, ActuatorError};
use crate::log::{ExecutionLog, Status};
use factorio_bot_core::petgraph::algo::toposort;
use factorio_bot_core::petgraph::graph::{DiGraph, NodeIndex};
use factorio_bot_planner::{
    ActionId, ActionKind, ActionNetwork, BotId, Schedule, ScheduledStep, StepKind, Ticks,
};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
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
            StepKind::Walk { to } => {
                // A walk step is not an action and has no log entry, so its
                // ticks have nowhere to be recorded. They are still real; if a
                // consumer ever needs them, the walk needs an id first.
                if act.walk(bot, to.clone()).await.is_err() {
                    abandon_rest(&mine[i..], senders);
                    return;
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
                        }
                        if let Some(tx) = senders.get(action) {
                            let _ = tx.send(Status::Success);
                        }
                    }
                    Err(e) => {
                        // No ticks to record. `ActuatorError` carries none, and
                        // inventing one here — the planned tick, or the tick of
                        // whatever ran last — is exactly what these fields must
                        // never hold. They stay `None`.
                        lock(log).fail(*action, step.end, e.to_string());
                        abandon_rest(&mine[i..], senders);
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
        if let StepKind::Act { action, .. } = &step.what {
            if let Some(tx) = senders.get(action) {
                let _ = tx.send(Status::Failed);
            }
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
                Status::Failed => return PredOutcome::Abandoned,
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
        // A speed the actuator cannot report falls back to normal speed
        // rather than aborting the run over a missing nicety: a wrong-but-
        // finite wait is recoverable (recovery re-checks preconditions before
        // dispatching the next action), a hung run is not.
        let speed = act.game_speed().await.unwrap_or(1.0);
        tokio::time::sleep(ticks_to_wall_clock(max_lag, speed)).await;
    }
    PredOutcome::Ready
}

/// Factorio runs at `60 * speed` ticks per second; `speed` is `game.speed`
/// (`Actuator::game_speed`), where `1.0` is normal. A non-default speed
/// scales how fast machine time passes without changing how many ticks a lag
/// edge represents, so wall-clock time is `ticks / (60 * speed)`.
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
) -> Result<ActionTicks, ActuatorError> {
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
    use factorio_bot_core::types::Position;
    use factorio_bot_planner::{Action, Actor, Condition, Effect, InventorySlot};
    use mockall::mock;
    use std::time::Duration;

    mock! {
        pub Act {}
        #[async_trait::async_trait]
        impl Actuator for Act {
            async fn walk(&self, bot: BotId, to: Position) -> Result<ActionTicks, ActuatorError>;
            async fn mine(&self, bot: BotId, item: &str, at: Position, count: u32) -> Result<ActionTicks, ActuatorError>;
            async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<ActionTicks, ActuatorError>;
            async fn place(&self, bot: BotId, item: &str, at: Position, direction: u8) -> Result<ActionTicks, ActuatorError>;
            async fn insert(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorError>;
            async fn remove(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<ActionTicks, ActuatorError>;
            async fn research(&self, tech: &str) -> Result<ActionTicks, ActuatorError>;
        }
    }

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
                radius: 3.0,
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
    }

    impl Default for Script {
        fn default() -> Self {
            Script {
                walk_delay_ms: BTreeMap::new(),
                mine_delay_ms: BTreeMap::new(),
                fail_walk: BTreeSet::new(),
                fail_mine: BTreeSet::new(),
                speed: 1.0,
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
    }

    impl RecordingAct {
        fn new(script: Script) -> Self {
            Self {
                script,
                origin: tokio::time::Instant::now(),
                seen: std::sync::Mutex::new(Vec::new()),
            }
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
        async fn walk(&self, bot: BotId, _to: Position) -> Result<ActionTicks, ActuatorError> {
            self.record(Dispatch::Walk(bot));
            Self::delay(self.script.walk_delay_ms.get(&bot).copied().unwrap_or(0)).await;
            if self.script.fail_walk.contains(&bot) {
                return Err(ActuatorError::Rejected("blocked".into()));
            }
            Ok(some_ticks())
        }

        async fn mine(
            &self,
            _bot: BotId,
            item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorError> {
            self.record(Dispatch::MineStart(item.to_string()));
            Self::delay(self.script.mine_delay_ms.get(item).copied().unwrap_or(0)).await;
            self.record(Dispatch::MineEnd(item.to_string()));
            if self.script.fail_mine.contains(item) {
                return Err(ActuatorError::Rejected("no ore".into()));
            }
            Ok(some_ticks())
        }

        async fn craft(
            &self,
            _bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorError> {
            Ok(some_ticks())
        }

        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<ActionTicks, ActuatorError> {
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
        ) -> Result<ActionTicks, ActuatorError> {
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
        ) -> Result<ActionTicks, ActuatorError> {
            Ok(some_ticks())
        }

        async fn research(&self, _tech: &str) -> Result<ActionTicks, ActuatorError> {
            Ok(some_ticks())
        }

        async fn game_speed(&self) -> Result<f64, ActuatorError> {
            Ok(self.script.speed)
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
            .returning(|_, _| Ok(some_ticks()));
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
        act.expect_walk().returning(|_, _| Ok(some_ticks()));
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
    async fn a_failure_leaves_the_ticks_absent_rather_than_borrowing_the_plans() {
        // The actuator's error carries no ticks, so there is nothing to
        // record. The planned numbers are sitting right there in the same
        // struct; the test exists because reaching for them is the tempting
        // wrong thing to do.
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into())));

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
    async fn a_failed_step_is_logged_and_stops_that_bot() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _| Ok(some_ticks()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into())));
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
            .with(eq(BotId(0)), eq(Position::new(10., 10.)))
            .times(1)
            .returning(|_, _| Ok(some_ticks()));
        act.expect_walk()
            .with(eq(BotId(1)), eq(Position::new(5., 5.)))
            .times(1)
            .returning(|_, _| Ok(some_ticks()));
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

    #[tokio::test]
    async fn a_walk_failure_stops_the_bot_before_the_action_runs() {
        let mut act = MockAct::new();
        act.expect_walk()
            .times(1)
            .returning(|_, _| Err(ActuatorError::Rejected("blocked".into())));
        act.expect_mine().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run(&act, &sched, &net)
            .await
            .expect("the run should have started");

        // The action never even started: the log has no attempt for it at all,
        // not merely a non-Failed status.
        assert!(log.is_empty());
        assert_eq!(log.status(mine_action_id()), Status::Pending);
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
}
