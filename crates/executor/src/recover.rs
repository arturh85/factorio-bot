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
/// `Rescheduled` keeps the original network, so its ids mean what they meant
/// before and the caller's existing `ExecutionLog` still describes them.
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
#[derive(Debug)]
pub enum Recovery {
    /// The plan still fits the world. Drop what already succeeded and run the
    /// rest of the *same* network under this new schedule; the existing log
    /// carries forward unchanged.
    Rescheduled(Schedule),
    /// The world no longer affords the plan's approach, but it does afford the
    /// goal. This is a **new plan** — see the type-level note on ids, and start
    /// a fresh `ExecutionLog` for it.
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
/// because retrying it is the entire point; `Running` comes back because an
/// action that started and never finished has no outcome, and re-running it
/// against observed state is safer than assuming it landed; `Pending` never
/// ran at all.
fn unfinished(net: &ActionNetwork, log: &ExecutionLog) -> BTreeSet<ActionId> {
    net.actions()
        .map(|a| a.id)
        .filter(|id| log.status(*id) != Status::Success)
        .collect()
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
/// Pure: no I/O, no async, no wall clock. `state` is observed state passed in
/// as a value, and the returned `Recovery` is a proposal the caller may ignore.
pub fn recover(
    goal: &Goal,
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
    log: &ExecutionLog,
) -> Recovery {
    // Tier 1 — the same plan, minus what is already done.
    let remaining = net.retaining(&unfinished(net, log));
    if let Ok(sched) = schedule(&remaining, state, bots) {
        return Recovery::Rescheduled(sched);
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

    #[test]
    fn a_recoverable_failure_reschedules_without_reexpanding() {
        let (goal, net, state, bots, log) = one_failed_mine_but_ore_still_reachable();
        match recover(&goal, &net, &state, &bots, &log) {
            Recovery::Rescheduled(s) => assert!(s.makespan > 0),
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

        let Recovery::Rescheduled(s) = recover(&goal, &net, &state, &bots, &log) else {
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
