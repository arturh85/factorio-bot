//! What to do about a partially failed execution.
//!
//! Three tiers, cheapest first. Every one of them is a *decision* returned as a
//! value: nothing here talks to the game, waits on a clock, or mutates the log
//! it was handed. That purity is the whole point — the tier boundaries are the
//! interesting behaviour, and they are only testable if deciding is separable
//! from acting.

use crate::log::{ExecutionLog, Status};
use factorio_bot_planner::{
    expand, registry_for, schedule, ActionId, ActionNetwork, BotId, Goal, PlanState, Schedule,
};
use std::collections::BTreeSet;

/// What recovery decided.
///
/// # Action ids are only comparable within one variant
///
/// `Rescheduled` carries a *subset* of the original network — the same actions
/// with the same ids, minus the ones that already succeeded — so its ids mean
/// what they meant before and the caller's existing `ExecutionLog` still
/// describes them.
///
/// **`Reexpanded` does not.** `expand` builds a fresh `ActionIdGen` starting at
/// zero, so a re-expanded network numbers its actions from zero and its ids
/// collide with the old network's and with the keys of the log that was handed
/// in — `ActionId(0)` in the new plan is an unrelated action that merely shares
/// a number with `ActionId(0)` in the old one. A caller that carries the old
/// log forward across a `Reexpanded` therefore joins unrelated work: actions
/// would come back already `Success` without ever having run, and `failed()`
/// would blame ids belonging to a plan that no longer exists.
///
/// So: **on `Reexpanded`, start a fresh `ExecutionLog`.** The old one is
/// evidence about a plan that has been discarded; keep it for diagnostics if
/// you like, but never as the progress log of the new schedule.
///
/// Seeding the generators past the old high-water mark was the alternative and
/// was rejected: it would make the ids unique but not *meaningful*, and it
/// would quietly invite exactly the log reuse described above — a plan built
/// from scratch is a different plan whether or not its numbering overlaps.
/// Renumbering is not what makes the log invalid; re-expanding is.
///
/// # Re-running an interrupted action can execute it twice
///
/// Both `Rescheduled` and `Reexpanded` may contain work the game has **already
/// done**. `recover` retires an action only on `Status::Success`, so an action
/// left `Running` — started, never finished, because the run died mid-dispatch
/// — comes back in the proposal. `Running` means precisely that nobody knows
/// whether it landed, so this is a genuine double-execution risk and not a
/// theoretical one.
///
/// Three `ActionKind`s are **not idempotent** and will visibly double if
/// re-run: `Insert` (the items go into the chest or furnace a second time),
/// `Remove` (a second withdrawal, or a failure because the first already
/// emptied the slot), and `Place` (a second entity, or a blocked tile). `Mine`,
/// `Craft` and `Research` are comparatively benign — re-running them
/// over-produces or no-ops rather than corrupting the world.
///
/// Retrying is the default here on purpose: it is right for the common case
/// (`Running` almost always means the dispatch never reached the game), and the
/// alternative — surfacing every interrupted action for a decision — needs a
/// caller that does not exist yet. But it is a default, not a guarantee.
/// **A caller that cannot tolerate double execution must inspect the log for
/// `Status::Running` itself, before dispatching either variant's schedule, and
/// decide per action whether to run it, skip it, or ask a human.** `recover`
/// hands back a proposal; it cannot make that call, because deciding needs a
/// look at the world and this function is pure.
#[derive(Debug)]
pub enum Recovery {
    /// Every action in the network already succeeded. There is nothing to
    /// dispatch and nothing to decide.
    ///
    /// Distinct from `Rescheduled` with an empty schedule on purpose: "the work
    /// is done" and "here is a new plan" are different answers, and collapsing
    /// them into one with a zero in it means a caller that loops — recover,
    /// run, recover — cannot see a terminating condition without inspecting
    /// the schedule's insides. It would just keep dispatching nothing.
    Complete,
    /// The plan still fits the world. `sched` dispatches the actions of the
    /// original network that have not yet succeeded. The existing
    /// `ExecutionLog` carries forward unchanged — unlike `Reexpanded`, the ids
    /// still mean what they meant.
    ///
    /// **`net` deliberately holds more actions than `sched` assigns.** Besides
    /// the unfinished work it keeps every succeeded action that an unfinished
    /// one depends on. Those carry no work and are never dispatched; they are
    /// there to carry their **edges**, because an edge carries a lag and a lag
    /// is machine time. `link(smelt, collect, 6000)` says the plate is not in
    /// the furnace for another hundred seconds no matter who is standing there,
    /// and `await_preds` reads that lag off this network at run time. Drop the
    /// succeeded `smelt` node and the lag vanishes with it: the retry of
    /// `collect` waits zero and reaches into a furnace that is still smelting.
    ///
    /// **Run `sched` against this `net`, with the log you gave `recover` —
    /// `run_into`, not `run`.** Two things depend on it. Handing `run_into` the
    /// *original* network puts already-succeeded actions back in the dispatch
    /// set. Handing it an *empty* log — which is exactly what `run` builds —
    /// makes the succeeded nodes above read as never-attempted, so they are
    /// published `Failed` and abandon the retries waiting behind them, and the
    /// run returns `Ok(())` having dispatched nothing at all.
    ///
    /// Carrying the network here rather than leaving the caller to rebuild it
    /// is what makes the first mispairing unrepresentable, and is why this
    /// variant is shaped exactly like `Reexpanded`: both are "here is a plan: a
    /// network and a schedule over it", and nothing about handling one should
    /// differ.
    ///
    /// The **whole** lag is preserved, not the part still outstanding. Plan time
    /// restarts at zero and there is no game clock here to say how much of the
    /// furnace's 6000 ticks already burned (see `Attempt`), so the retry waits
    /// it out again. That over-waits. Forgetting it acts too early, and only one
    /// of those two is safe.
    ///
    /// May contain interrupted (`Running`) actions — see the type-level note on
    /// double execution.
    Rescheduled { net: ActionNetwork, sched: Schedule },
    /// The world no longer affords the plan's approach, but it does afford the
    /// goal. This is a **new plan** — see the type-level note on ids, and start
    /// a fresh `ExecutionLog` for it.
    ///
    /// **This variant has no loop breaker and cannot have one.** Tier 2 never
    /// reads the failures: it re-expands the same goal against observed state,
    /// so if the world has not changed in a way the methods can see, it will
    /// propose the same shape again, and again. A pure function owns no retry
    /// budget and no memory of what it proposed last time — that is the price
    /// of being testable — so **the caller inherits the responsibility**: cap
    /// the number of re-expansions, or check that a new proposal actually
    /// differs from the one that just failed, before feeding it back in. A
    /// caller that does neither will loop forever on an unwinnable goal.
    ///
    /// May contain interrupted (`Running`) actions — see the type-level note on
    /// double execution.
    Reexpanded { net: ActionNetwork, sched: Schedule },
    /// Nothing mechanical is left to try: the remaining work will not schedule
    /// and the goal will not re-expand. The failed action ids are surfaced so a
    /// human — or, later, an LLM — can decide. Rare, high level, and not time
    /// critical, which is exactly why this seam is a plain value.
    Surfaced(Vec<ActionId>),
}

/// Actions of `net` that execution has not already completed successfully.
///
/// `Success` is the only status that retires an action. `Failed` comes back
/// because retrying it is the entire point; `Pending` never ran at all.
///
/// **`Running` comes back too, and that is the risky one.** An action is
/// `Running` exactly when nobody knows whether it completed — the run died
/// between dispatch and the reply — so putting it back in the plan may execute
/// it a second time. For `Insert`, `Remove` and `Place` that is visible in the
/// world: items inserted twice, a slot emptied twice, a second entity on the
/// tile. `Mine`, `Craft` and `Research` merely over-produce.
///
/// It comes back anyway, because in practice a dead run almost never reached
/// the game, and dropping the action would leave the plan silently
/// under-executed — a missing furnace is harder to notice than a duplicate one,
/// and the plan's own preconditions will catch the duplicate before the
/// omission. This is a judgement about the common case, **not** a safety
/// property, so it is stated rather than assumed: a caller that cannot tolerate
/// double execution must filter `Status::Running` itself before dispatching
/// what `recover` proposed. See the note on `Recovery`.
fn unfinished(net: &ActionNetwork, log: &ExecutionLog) -> BTreeSet<ActionId> {
    net.actions()
        .map(|a| a.id)
        .filter(|id| log.status(*id) != Status::Success)
        .collect()
}

/// How many times one action may be dispatched before tier 1 stops proposing
/// it and the decision escalates.
///
/// Three, counting the original run. One retry covers a transient — an RCON
/// timeout, a bot that was mid-walk, a chest that was briefly full. A second
/// covers the coincidence of two of those. An action that has failed three
/// times against a plan the scheduler still calls feasible is not being
/// unlucky: the plan is wrong in a way `schedule` cannot see, and re-proposing
/// it just issues the same command to a live server again.
///
/// The exact number is a judgement, not a derivation. What matters is that it
/// is finite and small — the cost of escalating early is one wasted
/// re-expansion, and the cost of never escalating is the unbounded loop this
/// constant exists to break.
pub const MAX_TIER_ONE_ATTEMPTS: u32 = 3;

/// Whether tier 1 has run out of road: some action is currently `Failed` and
/// has already been dispatched `MAX_TIER_ONE_ATTEMPTS` times.
///
/// This is the only place `recover` reads *history* rather than the present
/// state of the world. Without it the tier choice is a function of
/// schedulability alone, and a schedulable plan whose action fails every time
/// is proposed forever — the caller cannot break that from outside, because
/// every proposal it gets back is the same valid plan.
///
/// Only `Failed` actions count. An interrupted (`Running`) action has no
/// verdict yet, and a `Pending` one has not been tried.
fn exhausted_tier_one(net: &ActionNetwork, log: &ExecutionLog) -> bool {
    net.actions()
        .map(|a| a.id)
        .any(|id| log.status(id) == Status::Failed && log.attempts(id) >= MAX_TIER_ONE_ATTEMPTS)
}

/// `keep`, plus every succeeded action that a kept action depends on.
///
/// Those extra nodes are not work — the schedule never assigns them — they are
/// there so `ActionNetwork::retaining` keeps the *edges* into the kept actions,
/// and with them the lags. A lag is machine time: `link(smelt, collect, 6000)`
/// says the plate is not in the furnace's output for another hundred seconds,
/// whether or not a bot is standing there.
///
/// Direct predecessors only. A succeeded action's own predecessors constrain
/// nothing further — it has already finished, so whatever had to elapse before
/// it did elapse.
///
/// **The full lag is preserved, not the part of it still outstanding.** Time in
/// a plan restarts at zero, and knowing how much of the furnace's 6000 ticks
/// already burned needs a game clock, which this crate does not have (see
/// `Attempt`). Waiting the whole lag again over-waits; forgetting it acts too
/// early. Only one of those two is safe.
fn keep_with_lag_bearing_preds(
    net: &ActionNetwork,
    keep: &BTreeSet<ActionId>,
) -> BTreeSet<ActionId> {
    let mut wider = keep.clone();
    for id in keep {
        for (pred, _lag) in net.preds(*id) {
            wider.insert(pred);
        }
    }
    wider
}

/// Decide what to do about a partially failed execution.
///
/// Tier 1 is cheap: keep the network, drop what succeeded, and re-run
/// `schedule` against observed state. That covers the common case where the
/// world moved but the approach still works — the ore is further away, a bot
/// died, a furnace ended up somewhere else.
///
/// Tier 2 re-expands from the method layer, which is what is needed when the
/// world no longer affords the plan's approach at all: the ore patch is gone,
/// not merely further away. It produces a genuinely new plan — see `Recovery`
/// on why the caller must not carry the old log into it.
///
/// Tier 3 gives up and names the failures. This is the seam an LLM plugs into
/// later.
///
/// Before any of them: if nothing is left unfinished, the answer is `Complete`.
/// That case reaches no tier at all — there is no plan to propose, and saying so
/// with a distinct variant is what lets a caller loop on `recover` and stop.
///
/// Pure: no I/O, no async, no wall clock. `state` is observed state passed in
/// as a value, and the returned `Recovery` is a proposal the caller may ignore.
/// In particular the caller, not this function, owns the retry budget that keeps
/// repeated `Reexpanded` proposals from looping — see that variant.
pub fn recover(
    goal: &Goal,
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
    log: &ExecutionLog,
) -> Recovery {
    // Tier 0 — nothing to recover. Answered before tier 1 rather than falling
    // out of it as a zero-makespan `Rescheduled`, so a caller looping on
    // `recover` has a terminating condition it can match on.
    let keep = unfinished(net, log);
    if keep.is_empty() {
        return Recovery::Complete;
    }

    // Tier 1 — the same plan, minus what is already done. Skipped once an
    // action has burned through its attempts: proposing the same plan again is
    // the loop `MAX_TIER_ONE_ATTEMPTS` exists to break.
    if !exhausted_tier_one(net, log) {
        // Two different networks, on purpose.
        //
        // `schedule` runs over the strictly unfinished actions, so nothing that
        // already succeeded is dispatched a second time.
        //
        // The network that ships with the schedule is wider: it also keeps every
        // succeeded action that some unfinished action depends on. Those nodes
        // carry no work — nothing schedules them — but they carry their
        // **edges**, and an edge carries a lag. Drop the node and the lag goes
        // with it, and `await_preds` stops waiting for a furnace that is still
        // smelting. `run_into` publishes `Success` for them straight from the
        // log, so they release their dependents at once and cost nothing but
        // the wait they are there to preserve.
        let to_run = net.retaining(&keep);
        if let Ok(sched) = schedule(&to_run, state, bots) {
            return Recovery::Rescheduled {
                net: net.retaining(&keep_with_lag_bearing_preds(net, &keep)),
                sched,
            };
        }
    }

    // Tier 2 — the same goal, planned again from the method layer.
    //
    // `expand` wants a `chain_actor`: the bot whose simulated inventory the
    // outermost expansion is sized against. The goal being re-expanded is a
    // caller's goal, not bot-specific, and `expand` rejects a state whose bots
    // are not interchangeable, so *which* bot is chosen cannot change the
    // expansion — only whether it is deterministic. The lowest id in the
    // roster is therefore the choice: anchored to the roster we are about to
    // schedule against (so a bot the state has never heard of is caught as
    // `UnknownBot` rather than planned for), and stable no matter what order
    // the caller listed its bots in. An empty roster has no such bot, and
    // `schedule` would refuse it anyway, so tier 2 is skipped entirely.
    if let Some(chain_actor) = bots.iter().copied().min() {
        if let Ok(fresh) = expand(
            std::slice::from_ref(goal),
            state,
            &registry_for(bots),
            chain_actor,
        ) {
            if let Ok(sched) = schedule(&fresh, state, bots) {
                return Recovery::Reexpanded { net: fresh, sched };
            }
        }
    }

    // Tier 3 — nothing mechanical is left.
    Recovery::Surfaced(log.failed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect};
    use factorio_bot_planner::goal::Holder;
    use factorio_bot_planner::ids::ActionIdGen;
    use factorio_bot_planner::{InventorySlot, Ticks};
    use std::sync::Arc;

    const BOTS: [BotId; 2] = [BotId(1), BotId(2)];

    fn state() -> PlanState {
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &BOTS);
        for bot in BOTS {
            s.set_position(bot, Position::new(0., 0.));
        }
        s
    }

    /// A tile the fixture world actually has iron ore on.
    fn ore_tile(s: &PlanState) -> Position {
        s.resource_patches("iron-ore")
            .first()
            .expect("fixture has iron ore")
            .elements
            .first()
            .expect("patch has tiles")
            .clone()
    }

    /// Somewhere with no ore at all — the "patch is gone" stand-in. Observed
    /// state reports zero ore here, so `ResourceAvailable` can never hold for
    /// any bot and no amount of re-scheduling helps.
    fn barren_tile() -> Position {
        Position::new(500., 500.)
    }

    fn mine_at(gen: &mut ActionIdGen, pos: &Position, count: u32) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Mine {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: pos.clone(),
                    radius: 3.0,
                },
                Condition::ResourceAvailable {
                    pos: pos.clone(),
                    item: "iron-ore".into(),
                    count,
                },
            ],
            eff: vec![
                Effect::ConsumeResource {
                    pos: pos.clone(),
                    item: "iron-ore".into(),
                    count,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: "iron-ore".into(),
                    count,
                },
            ],
            duration: 60,
            pinned: None,
            label: format!("mine {} iron-ore", count),
        }
    }

    fn ore_goal(count: u32) -> Goal {
        Goal::Have {
            item: "iron-ore".into(),
            count,
            whose: Holder::Anyone,
        }
    }

    /// Two mines on a live patch; the first succeeded, the second failed.
    /// Nothing about the world has changed, so the remaining half still
    /// schedules.
    fn one_failed_mine_but_ore_still_reachable(
    ) -> (Goal, ActionNetwork, PlanState, Vec<BotId>, ExecutionLog) {
        let s = state();
        let tile = ore_tile(&s);
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let done = net.add(mine_at(&mut gen, &tile, 2));
        let failed = net.add(mine_at(&mut gen, &tile, 2));

        let mut log = ExecutionLog::default();
        log.start(done, 0);
        log.succeed(done, 60);
        log.start(failed, 60);
        log.fail(failed, 90, "walked into a biter".to_string());

        (ore_goal(4), net, s, BOTS.to_vec(), log)
    }

    /// The plan's tiles no longer hold ore. The remaining action's
    /// `ResourceAvailable` precondition cannot hold for *any* bot, so tier 1
    /// is impossible — but the goal is still satisfiable from a live patch
    /// elsewhere, which is exactly what tier 2 is for.
    fn ore_patch_exhausted() -> (Goal, ActionNetwork, PlanState, Vec<BotId>, ExecutionLog) {
        let s = state();
        let gone = barren_tile();
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let done = net.add(mine_at(&mut gen, &gone, 2));
        let failed = net.add(mine_at(&mut gen, &gone, 2));

        let mut log = ExecutionLog::default();
        log.start(done, 0);
        log.succeed(done, 60);
        log.start(failed, 60);
        log.fail(failed, 90, "no ore left here".to_string());

        (ore_goal(4), net, s, BOTS.to_vec(), log)
    }

    /// The same dead plan, but asking for something no method in the registry
    /// can decompose — `Goal::Producing` has no method at all. Both mechanical
    /// tiers are therefore closed.
    fn nothing_can_satisfy_the_goal() -> (Goal, ActionNetwork, PlanState, Vec<BotId>, ExecutionLog)
    {
        let (_, net, s, bots, log) = ore_patch_exhausted();
        let goal = Goal::Producing {
            item: "iron-plate".into(),
            rate: 30.0,
        };
        (goal, net, s, bots, log)
    }

    /// Every action of the plan succeeded. Nothing is left to run.
    fn everything_succeeded() -> (Goal, ActionNetwork, PlanState, Vec<BotId>, ExecutionLog) {
        let s = state();
        let tile = ore_tile(&s);
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(mine_at(&mut gen, &tile, 2));
        let second = net.add(mine_at(&mut gen, &tile, 2));

        let mut log = ExecutionLog::default();
        for (id, start) in [(first, 0), (second, 60)] {
            log.start(id, start);
            log.succeed(id, start + 60);
        }

        (ore_goal(4), net, s, BOTS.to_vec(), log)
    }

    #[test]
    fn a_fully_succeeded_plan_is_complete_rather_than_an_empty_reschedule() {
        // A zero-makespan `Rescheduled` says "here is a new plan" when it means
        // "the work is done". A caller looping recover-run-recover would keep
        // dispatching nothing and never stop, so the two answers are different
        // variants and not one variant with a zero in it.
        let (goal, net, state, bots, log) = everything_succeeded();
        assert!(
            net.actions().all(|a| log.status(a.id) == Status::Success),
            "the fixture must leave nothing unfinished"
        );
        // The trap this variant exists to avoid: tier 1 *would* have answered,
        // and answered with an empty schedule.
        let empty = net.retaining(&unfinished(&net, &log));
        assert!(empty.is_empty());
        assert_eq!(
            schedule(&empty, &state, &bots)
                .expect("an empty network schedules fine")
                .makespan,
            0,
            "tier 1 would have returned a zero-makespan Rescheduled"
        );

        assert!(matches!(
            recover(&goal, &net, &state, &bots, &log),
            Recovery::Complete
        ));
    }

    /// Counts every command that reaches "the game", and always succeeds.
    #[derive(Default)]
    struct CountingAct {
        mined: std::sync::Mutex<Vec<(BotId, u32)>>,
    }

    /// No game clock: every dispatch reports [`ActionTicks::UNKNOWN`], which is
    /// the honest answer for a stub and keeps the absent-tick path exercised.
    #[async_trait::async_trait]
    impl crate::actuator::Actuator for CountingAct {
        async fn walk(
            &self,
            _: BotId,
            _: Position,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn mine(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            count: u32,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            self.mined.lock().unwrap().push((bot, count));
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn craft(
            &self,
            _: BotId,
            _: &str,
            _: u32,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn place(
            &self,
            _: BotId,
            _: &str,
            _: Position,
            _: u8,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn insert(
            &self,
            _: BotId,
            _: &str,
            _: Position,
            _: InventorySlot,
            _: &str,
            _: u32,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn remove(
            &self,
            _: BotId,
            _: &str,
            _: Position,
            _: InventorySlot,
            _: &str,
            _: u32,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
        async fn research(
            &self,
            _: &str,
        ) -> Result<crate::actuator::ActionTicks, crate::ActuatorError> {
            Ok(crate::actuator::ActionTicks::UNKNOWN)
        }
    }

    /// The `recover` -> `run_into` seam, walked exactly as `Rescheduled`'s own
    /// documentation instructs.
    ///
    /// This is the composition defect, and neither half could see it alone.
    /// `run_into` publishes `Failed` for every action of *its* network that the
    /// schedule does not assign — correct, so that a dependent waiting on an
    /// unscheduled action is released rather than hanging. A tier-1 schedule
    /// deliberately omits every already-succeeded action. Pair the two and the
    /// succeeded predecessor is broadcast as a failure, `await_preds` abandons
    /// the retry waiting behind it, and the run returns `Ok(())` having called
    /// the actuator zero times: a silent no-op reported as success. Carrying
    /// the retained network in the variant is what fixes it.
    #[tokio::test]
    async fn a_tier_one_proposal_run_as_documented_actually_reaches_the_game() {
        let s = state();
        let tile = ore_tile(&s);
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let done = net.add(mine_at(&mut gen, &tile, 2));
        let stuck = net.add(mine_at(&mut gen, &tile, 2));
        // The edge is the trigger: without it nothing waits on the succeeded
        // action and the abandonment has nobody to strand.
        net.link(done, stuck, 0);

        let mut log = ExecutionLog::default();
        log.start(done, 0);
        log.succeed(done, 60);
        log.start(stuck, 60);
        log.fail(stuck, 90, "player was busy".to_string());

        let Recovery::Rescheduled {
            net: retained,
            sched,
        } = recover(&ore_goal(4), &net, &s, &BOTS, &log)
        else {
            panic!("expected a reschedule");
        };
        assert!(
            sched.assignment(stuck).is_some(),
            "the retry is in the proposal"
        );

        let act = CountingAct::default();
        let progress = std::sync::Mutex::new(log.clone());
        // Exactly what the variant's doc says to do: the schedule against the
        // network it came back with.
        crate::run::run_into(&act, &sched, &retained, &progress)
            .await
            .expect("the run should start");

        assert_eq!(
            act.mined.lock().unwrap().len(),
            1,
            "the retried action must actually reach the game"
        );
        let after = progress.into_inner().unwrap();
        assert_eq!(
            after.status(stuck),
            Status::Success,
            "and its success must be recorded, or tier 1 proposes it forever"
        );
        assert_eq!(after.attempts(stuck), 2, "recorded as the second attempt");
        assert_eq!(
            after.status(done),
            Status::Success,
            "the action that already succeeded is untouched"
        );
    }

    /// D4: a lag edge from a *succeeded* predecessor must survive into the
    /// retry, or the executor reaches into a furnace that is still smelting.
    ///
    /// `link(a, b, 6000)` is a hundred seconds of machine time. `a` succeeded,
    /// so tier 1 leaves it out of the schedule — but dropping its node dropped
    /// the edge, `await_preds` found no predecessor, and the retry of `b` slept
    /// zero. The clock is paused, so the assertion is on virtual time and the
    /// test still runs instantly.
    #[tokio::test(start_paused = true)]
    async fn a_retry_still_waits_out_a_succeeded_predecessors_lag() {
        const LAG: Ticks = 6_000;

        let s = state();
        let tile = ore_tile(&s);
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let smelt = net.add(mine_at(&mut gen, &tile, 2));
        let collect = net.add(mine_at(&mut gen, &tile, 2));
        net.link(smelt, collect, LAG);

        let mut log = ExecutionLog::default();
        log.start(smelt, 0);
        log.succeed(smelt, 60);
        log.start(collect, 60);
        log.fail(collect, 90, "furnace was empty".to_string());

        let Recovery::Rescheduled {
            net: retained,
            sched,
        } = recover(&ore_goal(4), &net, &s, &BOTS, &log)
        else {
            panic!("expected a reschedule");
        };

        // The succeeded action is in the network but not in the schedule: it
        // carries the lag, it does not carry work.
        assert_eq!(
            retained.preds(collect),
            vec![(smelt, LAG)],
            "the lag edge must survive the predecessor's success"
        );
        assert!(
            sched.assignment(smelt).is_none(),
            "succeeded work must not be dispatched again"
        );
        assert!(sched.assignment(collect).is_some());

        let act = CountingAct::default();
        let progress = std::sync::Mutex::new(log.clone());
        let began = tokio::time::Instant::now();
        crate::run::run_into(&act, &sched, &retained, &progress)
            .await
            .expect("the run should start");
        let waited = began.elapsed();

        assert_eq!(act.mined.lock().unwrap().len(), 1, "the retry did dispatch");
        assert!(
            waited >= std::time::Duration::from_secs(100),
            "the retry must wait out the full {LAG}-tick lag; it waited {waited:?}"
        );
        assert_eq!(
            progress.into_inner().unwrap().status(collect),
            Status::Success
        );
    }

    /// D3: tier 1 must stop proposing a plan whose action keeps failing.
    ///
    /// The tier choice used to be a function of schedulability alone, so a
    /// perfectly schedulable plan whose action fails every single time was
    /// re-proposed forever, each round issuing the command to a live server
    /// again. The caller could not break it from outside: every proposal it got
    /// back was a valid plan.
    ///
    /// Asserted at the boundary in both directions, so this pins the threshold
    /// and not merely "escalation happens eventually".
    #[test]
    fn tier_one_escalates_once_an_action_has_burned_its_attempts() {
        // The rule below is written in terms of the constant, so it holds for
        // any value and says nothing about which one we picked. Pin the value
        // separately: it is a judgement about how much bad luck to absorb
        // before spending a re-expansion, and moving it changes how many real
        // commands a doomed action fires at a live server. That should be a
        // deliberate edit, not a drift.
        assert_eq!(MAX_TIER_ONE_ATTEMPTS, 3);

        let fixture = |attempts: u32| {
            let s = state();
            let tile = ore_tile(&s);
            let mut gen = ActionIdGen::new();
            let mut net = ActionNetwork::new();
            let done = net.add(mine_at(&mut gen, &tile, 2));
            let doomed = net.add(mine_at(&mut gen, &tile, 2));
            net.link(done, doomed, 0);

            let mut log = ExecutionLog::default();
            log.start(done, 0);
            log.succeed(done, 60);
            for round in 0..attempts {
                log.start(doomed, 60 + round);
                log.fail(doomed, 90 + round, "player is stuck".to_string());
            }
            assert_eq!(log.attempts(doomed), attempts);
            (net, s, log)
        };

        // One short of the limit: the world still affords the plan, so tier 1
        // is still the right, cheap answer.
        let (net, s, log) = fixture(MAX_TIER_ONE_ATTEMPTS - 1);
        assert!(
            matches!(
                recover(&ore_goal(4), &net, &s, &BOTS, &log),
                Recovery::Rescheduled { .. }
            ),
            "below the limit tier 1 must still fire, or the constant is dead weight"
        );

        // At the limit: the same plan is still schedulable, so nothing about
        // the *world* has escalated — only the history has. That is the point.
        let (net, s, log) = fixture(MAX_TIER_ONE_ATTEMPTS);
        let still_schedulable = {
            let keep = unfinished(&net, &log);
            schedule(&net.retaining(&keep), &s, &BOTS).is_ok()
        };
        assert!(
            still_schedulable,
            "tier 1 is being skipped on history alone, not because scheduling broke"
        );
        assert!(
            matches!(
                recover(&ore_goal(4), &net, &s, &BOTS, &log),
                Recovery::Reexpanded { .. }
            ),
            "at the limit the decision must escalate instead of looping"
        );
    }

    #[test]
    fn a_recoverable_failure_reschedules_without_reexpanding() {
        let (goal, net, state, bots, log) = one_failed_mine_but_ore_still_reachable();
        match recover(&goal, &net, &state, &bots, &log) {
            Recovery::Rescheduled { sched, .. } => assert!(sched.makespan > 0),
            other => panic!("expected a reschedule, got {other:?}"),
        }
    }

    #[test]
    fn a_rescheduled_plan_leaves_out_what_already_succeeded() {
        // The half of tier 1 that `makespan > 0` cannot see: a reschedule that
        // re-ran the succeeded mine would also have a positive makespan.
        let (goal, net, state, bots, log) = one_failed_mine_but_ore_still_reachable();
        let succeeded = ActionId(0);
        let failed = ActionId(1);
        assert_eq!(log.status(succeeded), Status::Success);

        let Recovery::Rescheduled { sched: s, .. } = recover(&goal, &net, &state, &bots, &log)
        else {
            panic!("expected a reschedule");
        };
        assert!(
            s.assignment(failed).is_some(),
            "the failed action must be re-run"
        );
        assert!(
            s.assignment(succeeded).is_none(),
            "an action that already succeeded must not be scheduled again"
        );
    }

    #[test]
    fn an_unschedulable_state_triggers_reexpansion() {
        let (goal, net, state, bots, log) = ore_patch_exhausted();
        // Tier 1 is genuinely closed, not merely un-taken: the remaining
        // network cannot schedule at all against observed state.
        let remaining = net.retaining(&unfinished(&net, &log));
        assert!(
            !remaining.is_empty(),
            "there is work left, so tier 1 was reached and refused"
        );
        assert!(
            schedule(&remaining, &state, &bots).is_err(),
            "tier 2 is only being tested if tier 1 could not have fired"
        );

        assert!(matches!(
            recover(&goal, &net, &state, &bots, &log),
            Recovery::Reexpanded { .. }
        ));
    }

    #[test]
    fn a_reexpanded_plan_is_new_work_whose_ids_must_not_be_read_against_the_old_log() {
        // Pins the semantics documented on `Recovery`: re-expansion numbers
        // from zero, so the ids collide with the handed-in log's keys and the
        // caller must start a fresh log. If this ever stops holding, the doc
        // comment is a lie and callers reusing the log break silently.
        let (goal, net, state, bots, log) = ore_patch_exhausted();
        let Recovery::Reexpanded { net: fresh, .. } = recover(&goal, &net, &state, &bots, &log)
        else {
            panic!("expected re-expansion");
        };

        let fresh_ids: BTreeSet<ActionId> = fresh.actions().map(|a| a.id).collect();
        let stale: Vec<ActionId> = fresh_ids
            .iter()
            .copied()
            .filter(|id| log.status(*id) != Status::Pending)
            .collect();
        assert!(
            !stale.is_empty(),
            "re-expansion is expected to reuse ids the old log already knows: \
             that collision is why the log must not be carried forward"
        );
        // And the collision is not benign: the old log calls one of them done.
        assert!(
            stale.iter().any(|id| log.status(*id) == Status::Success),
            "at least one re-expanded action would come back already Success"
        );
    }

    #[test]
    fn an_unexpandable_goal_is_surfaced_with_the_failures_that_caused_it() {
        let (goal, net, state, bots, log) = nothing_can_satisfy_the_goal();
        // Both mechanical tiers must be closed for this to be a tier-3 test.
        let remaining = net.retaining(&unfinished(&net, &log));
        assert!(
            schedule(&remaining, &state, &bots).is_err(),
            "tier 1 must be closed"
        );
        assert!(
            expand(
                std::slice::from_ref(&goal),
                &state,
                &registry_for(&bots),
                BotId(1)
            )
            .is_err(),
            "tier 2 must be closed"
        );

        match recover(&goal, &net, &state, &bots, &log) {
            Recovery::Surfaced(ids) => assert_eq!(ids, log.failed()),
            other => panic!("expected surfacing, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_roster_surfaces_rather_than_expanding_for_nobody() {
        // `schedule` refuses an empty roster, and tier 2 has no bot to anchor
        // the expansion to. Both closed means tier 3, not a panic.
        let (goal, net, state, _, log) = one_failed_mine_but_ore_still_reachable();
        match recover(&goal, &net, &state, &[], &log) {
            Recovery::Surfaced(ids) => assert_eq!(ids, log.failed()),
            other => panic!("expected surfacing, got {other:?}"),
        }
    }

    // Deliberately no "recover does not mutate the log" test: `recover` takes
    // `&ExecutionLog`, so no implementation could fail it. A test that cannot
    // fail asserts nothing.
}
