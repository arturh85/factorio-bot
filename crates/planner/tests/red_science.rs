//! The first vertical slice: ten red science packs, four bots, no Factorio.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::schedule::StepKind;
use factorio_bot_planner::{mermaid_gantt, schedule, BotId, PlanState};
use std::sync::Arc;

fn world_with_furnaces(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(fixture_world()), bots);
    for bot in bots {
        // Every bot starts the way `Planner` seeds a fresh player.
        state.gain(*bot, "stone-furnace", 2);
    }
    // `Planner` seeds real players where they actually stand, not all on one
    // tile. Collocating them removes travel cost as a signal entirely, leaving
    // bot id order to decide everything — the artificial condition that let the
    // chain-binding gap sit unnoticed. Bot 2 stands 30 tiles east so that at
    // least one comparison in this suite is settled by a walk.
    if bots.contains(&BotId(2)) {
        state.set_position(BotId(2), Position::new(30., 0.));
    }
    state
}

fn goal(count: u32) -> Goal {
    Goal::Have {
        item: "automation-science-pack".into(),
        count,
        whose: Holder::Anyone,
    }
}

#[test]
fn ten_red_science_across_four_bots_expands_and_schedules() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).expect("expands");
    assert!(
        net.len() > 10,
        "a real chain, not a stub: {} actions",
        net.len()
    );

    let plan = schedule(&net, &state, &bots).expect("schedulable");
    let acted: usize = plan
        .steps
        .iter()
        .filter(|s| matches!(s.what, StepKind::Act { .. }))
        .count();
    assert_eq!(acted, net.len(), "every action is scheduled exactly once");
}

/// The headline goal at its smallest: one pack, four bots.
///
/// A shortfall of one is not something to split, but it still has to open a
/// chain. Without one the expansion runs on with `Holder::Anyone` all the way
/// down, and the first intermediate goal wanting more than one unit — the two
/// iron plates behind a gear — is scattered across bots that the gear craft
/// needs in a single inventory. This is also every incremental plan's last
/// iteration, where nine of ten packs are already held.
#[test]
fn a_single_pack_across_four_bots_expands_and_schedules() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(1)], &state, &registry_for(&bots), BotId(1)).expect("expands");
    let plan = schedule(&net, &state, &bots).expect("schedulable");
    let acted: usize = plan
        .steps
        .iter()
        .filter(|s| matches!(s.what, StepKind::Act { .. }))
        .count();
    assert_eq!(acted, net.len(), "every action is scheduled exactly once");
}

#[test]
fn every_bot_is_used() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let plan = schedule(&net, &state, &bots).unwrap();
    for bot in bots {
        assert!(
            !plan.steps_for(bot).is_empty(),
            "{} was given nothing to do",
            bot
        );
    }
}

#[test]
fn more_bots_finish_sooner() {
    let state_one = world_with_furnaces(&[BotId(1)]);
    let net_one = expand(&[goal(4)], &state_one, &registry_for(&[BotId(1)]), BotId(1)).unwrap();
    let one = schedule(&net_one, &state_one, &[BotId(1)])
        .unwrap()
        .makespan;

    let four = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state_four = world_with_furnaces(&four);
    let net_four = expand(&[goal(4)], &state_four, &registry_for(&four), BotId(1)).unwrap();
    let many = schedule(&net_four, &state_four, &four).unwrap().makespan;

    // The four-vs-one speedup on this scenario is 2.27x (one = 6173, many =
    // 2717). That speedup is itself down from 2.45x (many was 2523) before
    // task 2 of the planner-hardening pass narrowed `infer_edges` to drop
    // only inventory-scoped pairings across chains — restricting inference
    // regressed this particular scenario's makespan by 194 ticks (7.7%),
    // a known greedy-list-scheduling anomaly (see task-2-report.md), not a
    // correctness defect. `many * 2 < one` still passes with 369 ticks of
    // headroom (13.6%): the floor is 2x, not the measured 2.27x, so this
    // guard survives ordinary makespan movement in later tasks without being
    // so loose that a repeat of this task's 194-tick regression is invisible.
    assert!(
        many.saturating_mul(2) < one,
        "four bots ({} ticks) must beat one ({} ticks) by more than 2x",
        many,
        one
    );
    // Absolute ceiling: catches a regression even if `one` also moves in a
    // way that keeps the 2x ratio satisfied. 3200 gives headroom above the
    // measured 2717 without being so loose that this task's own 194-tick
    // regression (2523 -> 2717) would have passed silently.
    assert!(many < 3200, "four bots regressed past 3200 ticks: {}", many);
}

#[test]
fn the_plan_renders_as_a_gantt_chart() {
    let bots = [BotId(1), BotId(2)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(2)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let plan = schedule(&net, &state, &bots).unwrap();
    let chart = mermaid_gantt(&plan, "Red science");
    assert!(chart.starts_with("gantt"));
    assert!(chart.contains("section bot 1"));
    assert!(chart.contains("section bot 2"));
    assert!(chart.contains("mine"));
}

#[test]
fn expansion_is_deterministic() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let first = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let second = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let labels_a: Vec<String> = first.actions().map(|a| a.label.clone()).collect();
    let labels_b: Vec<String> = second.actions().map(|a| a.label.clone()).collect();
    assert_eq!(labels_a, labels_b);

    let plan_a = schedule(&first, &state, &bots).unwrap();
    let plan_b = schedule(&second, &state, &bots).unwrap();
    assert_eq!(plan_a.makespan, plan_b.makespan);
    assert_eq!(plan_a.steps, plan_b.steps);
}
