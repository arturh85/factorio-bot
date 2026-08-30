//! The first vertical slice: ten red science packs, four bots, no Factorio.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::schedule::StepKind;
use factorio_bot_planner::{mermaid_gantt, schedule, BotId, PlanState};
use std::sync::Arc;

mod common;

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

    // Measured on this scenario: one = 4751, many = 1870, a 2.541x speedup.
    //
    // Both figures moved three times during the planner-hardening pass and this
    // comment is the crate's only record of them, so it states what was
    // actually measured rather than what an earlier task predicted. Task 2
    // narrowed `infer_edges` to drop only inventory-scoped pairings across
    // chains and cost 194 ticks here (2523 -> 2717), a greedy-list-scheduling
    // anomaly rather than a correctness defect; task 4 then sited furnaces at
    // the ore instead of the origin and took one from 6173 to 4749 and many
    // from 2717 to 1843, lifting the ratio from 2.27x to 2.577x. Teaching
    // `is_position_free` to see ore then pushed the furnace off the patch it
    // had been standing on and one tile out, adding that step to every trip:
    // one 4749 -> 4751, many 1843 -> 1870, ratio 2.577x -> 2.541x.
    //
    // The floor stays 2x rather than the measured 2.577x, so ordinary
    // makespan movement does not trip it.
    assert!(
        many.saturating_mul(2) < one,
        "four bots ({} ticks) must beat one ({} ticks) by more than 2x",
        many,
        one
    );
    // Absolute ceiling: catches a regression even if `one` also moves in a
    // way that keeps the 2x ratio satisfied. 2100 sits 230 ticks (12%) above
    // the measured 1870 — room for ordinary movement, but tight enough that a
    // repeat of task 2's 194-tick regression fails here instead of passing
    // silently. Retighten it whenever the measured figure drops again: a
    // ceiling with 74% headroom, which 3200 became, guards nothing.
    assert!(many < 2100, "four bots regressed past 2100 ticks: {}", many);
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

/// The engine's own property test, run against the engine's own output.
///
/// `assert_preconditions_hold_over_time` had only ever been pointed at
/// hand-built networks with no chains set, so the whole cross-chain path in
/// `infer_edges` — where an edge is *dropped* on the argument that the
/// scheduler re-derives it — was never checked against a real expansion. This
/// replays twelve of them: four goal sizes over one, two and four bots.
///
/// It is the only test *in this file* that would catch a dropped edge as such:
/// a missing dependency does not make `schedule()` fail, it makes `schedule()`
/// return a plan whose precondition is false at the tick the action starts.
/// Two hand-built replays in `tests/scheduling.rs` check the same property, but
/// only over networks they construct themselves — never over a real expansion.
///
/// **On today's fixture it does not yet discriminate**, and saying so is the
/// point of writing it down. Stubbing `infer_edges` to return immediately was
/// measured to leave all seven tests in this file passing, this one included,
/// while eleven `network` unit tests fail. Red science's chains have no
/// cross-chain dependency — `free_tile_near` hands each chain its own furnace
/// tile — and inside one chain the scheduler's per-bot feasibility check plus
/// a single `free_at` cursor reconstruct the order an edge would have stated.
/// So this guards the moment that stops being true (a method reusing an
/// existing furnace, an `Actor::Bound` holder, a `Consolidate`), not anything
/// the crate does today.
#[test]
fn every_expansion_replays_in_time_order() {
    let rosters: [Vec<BotId>; 3] = [
        vec![BotId(1)],
        vec![BotId(1), BotId(2)],
        vec![BotId(1), BotId(2), BotId(3), BotId(4)],
    ];
    for bots in &rosters {
        for count in [1u32, 2, 4, 10] {
            let state = world_with_furnaces(bots);
            let net = expand(&[goal(count)], &state, &registry_for(bots), BotId(1))
                .unwrap_or_else(|e| panic!("{} packs on {} bots: {}", count, bots.len(), e));
            let plan = schedule(&net, &state, bots)
                .unwrap_or_else(|e| panic!("{} packs on {} bots: {}", count, bots.len(), e));
            common::assert_preconditions_hold_over_time(&net, &state, &plan);
        }
    }
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
