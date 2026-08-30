//! The first vertical slice: ten red science packs, four bots, no Factorio.

use factorio_bot_core::test_utils::fixture_world;
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

    assert!(
        many < one,
        "four bots ({} ticks) must beat one ({} ticks)",
        many,
        one
    );
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
