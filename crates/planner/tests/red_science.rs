//! The first vertical slice: ten red science packs, four bots, no Factorio.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::schedule::StepKind;
use factorio_bot_planner::{BotId, PlanState, mermaid_gantt, schedule};
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

/// The same world with **empty pockets**, which is what makes a furnace worth
/// handing to another bot.
///
/// `world_with_furnaces` gives every bot two stone furnaces, and a taker
/// holding one saves nothing by asking a supplier to place it — so
/// `worth_handing_a_furnace_over` refuses and no chain in that fixture ever
/// depends on another. Take the furnaces away and the five stone under each
/// one become real work to move, which is the condition a live run is
/// permanently in after its seed inventory is spent.
///
/// Bot 2 stands where it does in `world_with_furnaces`, for the same reason:
/// so that at least one comparison in the suite is settled by a walk.
fn world_without_furnaces(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(fixture_world()), bots);
    if bots.contains(&BotId(2)) {
        state.set_position(BotId(2), Position::new(30., 0.));
    }
    state
}

/// `world_without_furnaces`, and with the fixture's rocks taken away as well.
///
/// **This is what keeps `every_expansion_replays_in_time_order` discriminating,
/// and it was added on 2026-09-04 because that test caught its own decay.**
/// `Chop` moved ahead of `Mine` that day, and `fixture_world` stands three
/// `rock-huge` and one `rock-big` -- so the first chop in any of these plans
/// hands the acting bot twenty-four coal *and* twenty-four stone at once. A bot
/// holding that is short of nothing, `worth_handing_a_furnace_over` reads a
/// saving of zero and refuses, and every cross-chain edge in this file
/// vanished: the replay went back to checking a property nothing could break.
/// The rocks did not break the handover -- they made it genuinely pointless on
/// this fixture, which is a different thing and is why the fixture moves rather
/// than the predicate.
///
/// Removed through `EntityGraph::retire_minable`, the same door the executor
/// uses when a bot mines one, rather than by building a world without them:
/// the population under test has to be one the live game can produce.
fn world_without_furnaces_or_rocks(bots: &[BotId]) -> PlanState {
    let world = fixture_world();
    for name in ["rock-huge", "rock-big", "sand-rock-big"] {
        for position in world.entity_graph.minable_positions(name) {
            world.entity_graph.retire_minable(name, &position);
        }
    }
    assert!(
        world.entity_graph.minables_yielding("stone").is_empty(),
        "control: no standing entity may still yield stone, or this fixture is \
         the same as the one above"
    );
    let mut state = PlanState::from_world(Arc::new(world), bots);
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

    // Measured on this scenario: one = 6691, many = 2100, a 3.186x speedup.
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
    // one 4749 -> 4751, many 1843 -> 1870, ratio 2.577x -> 2.541x. Finally,
    // reserving each entity's real collision box instead of a single tile —
    // a stone furnace is 1.398 wide, so single-tile siting produced furnaces
    // the game refuses to place — moved the first one off the copper patch
    // edge: one 4751 -> 4765, many 1870 -> 1881, ratio 2.541x -> 2.533x.
    // Then hand mining stopped ignoring the character's 0.5 mining speed, so
    // every mine action doubled: one 4765 -> 6691, many 1881 -> 2100, ratio
    // 2.533x -> 3.186x. `one` doubled harder than `many` because a single bot
    // does all the mining serially while four share it, so mining is a larger
    // fraction of the one-bot makespan.
    //
    // The floor stays 2x rather than the measured 3.186x, so ordinary
    // makespan movement does not trip it.
    //
    // Re-measured at `3e08f8af` (the per-bot share-sizing design doc's
    // baseline, before any of that work landed): one = 7075, many = 2100. The
    // `one = 6691` above was already stale by then; nothing here had touched
    // `one`, so this is the same drift ordinary movement always produces, not
    // evidence of a bug -- but it went unrecorded until now, so record it.
    //
    // Re-measured again at the tip of that work -- per-bot share sizing
    // (`6fcbba5c`, `8cc73153`, `7d614681`), the interchangeable-bots guard's
    // deletion (`56870959`, `f3a22e29`), and binding a `Holder::Share` chain's
    // ownership to the bot it was sized against (`c470388b`): one = 7075
    // (unchanged -- a solo bot's plan has no share to size differently, so a
    // rule about sizing shares cannot move it), many = 2063 (2100 -> 2063, an
    // *improvement* of 37 ticks, not a regression). The share-binding commit's
    // own cost -- serialising a `Researched` chain's trigger and pack
    // subtrees onto one bot, recorded in `crates/planner/src/goal.rs`'s
    // `Holder::Share` doc as 22,072 ticks on a live four-bot run -- does not
    // show up here: this goal is `automation-science-pack`, which this
    // fixture's recipes reach with no unlocking technology, so no `Researched`
    // goal and no share-bound trigger/pack subtree ever enters this plan. This
    // test does not exercise that cost; it is not evidence the cost is absent
    // elsewhere.
    assert!(
        many.saturating_mul(2) < one,
        "four bots ({} ticks) must beat one ({} ticks) by more than 2x",
        many,
        one
    );
    // Absolute ceiling: catches a regression even if `one` also moves in a
    // way that keeps the 2x ratio satisfied. Retightened from 2350 to 2310
    // now that the measured figure dropped to 2063 -- 2310 sits 247 ticks
    // (12%) above it, the same margin the previous ceiling kept above 2100.
    // Retighten it whenever the measured figure drops again: a ceiling with
    // 74% headroom, which 3200 became, guards nothing.
    //
    // **Raised to 2950 on 2026-09-04, measured at 2682.** Not drift: a
    // furnace's fuel load now carries the smelting lag like its ore does
    // (`have::every_fuel_load_gates_the_take_by_the_whole_smelting_time`).
    // On this fixture the fuel lands well after the ore insert -- the coal is
    // mined after the ore -- so the old 2063 took plates from a furnace that
    // had not yet started, which is exactly what `run-1788552801-73005` did
    // live (`tried to remove 5 copper-plate but removed 3`). The 619 ticks
    // are the furnace's real start; scheduling the coal ahead of the ore
    // would win most of them back and is a planning improvement, not a
    // correctness question.
    assert!(many < 2950, "four bots regressed past 2950 ticks: {}", many);
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
/// **It used not to discriminate, and it does now.** The note this replaces
/// said so in as many words: stubbing `infer_edges` to return immediately left
/// every test in this file passing, because red science's chains had no
/// cross-chain dependency at all — `free_tile_near` handed each chain its own
/// furnace tile, and inside one chain the scheduler's per-bot feasibility
/// check plus a single `free_at` cursor reconstruct the order an edge would
/// have stated. It named the moment that would stop being true: "a method
/// reusing an existing furnace, an `Actor::Bound` holder, a `Consolidate`".
///
/// R3 is that moment. `smelt_steps` now hands a furnace it would have had to
/// build — the stone, the craft, the placement and the coal — to another bot
/// as a `Step::Owned` block, so a placement in one chain gates inserts in
/// another and a fuel load in one chain gates a take in another. Neither edge
/// survives being dropped: the placement's would let a bot insert into a
/// furnace that does not stand yet, and the fuel's carries the furnace's whole
/// smelting lag, which `infer_edges` could not supply even if it kept the
/// pairing.
///
/// So the loop runs over **three** fixtures. `world_with_furnaces` is the one
/// that has always been here, and it still hands nothing over: every bot is
/// carrying two stone furnaces, so moving a placement saves the taker nothing
/// and `worth_handing_a_furnace_over` refuses. `world_without_furnaces` is the
/// same world with empty pockets, which used to be what made the handover pay.
///
/// **It stopped paying on 2026-09-04 and this guard is what said so.** `Chop`
/// moved ahead of `Mine`, one swing at the fixture's `rock-huge` now hands a
/// bot twenty-four coal and twenty-four stone, and a bot holding that is short
/// of nothing for a furnace to save it -- so `world_without_furnaces` produced
/// zero cross-chain edges and this test failed on its own discrimination check
/// rather than on any assertion about a plan. `world_without_furnaces_or_rocks`
/// is the third fixture, added for that reason; see its own doc.
#[test]
fn every_expansion_replays_in_time_order() {
    let mut cross_chain_edges = 0usize;
    let rosters: [Vec<BotId>; 3] = [
        vec![BotId(1)],
        vec![BotId(1), BotId(2)],
        vec![BotId(1), BotId(2), BotId(3), BotId(4)],
    ];
    for bots in &rosters {
        for count in [1u32, 2, 4, 10] {
            for (name, state) in [
                ("with furnaces", world_with_furnaces(bots)),
                ("without furnaces", world_without_furnaces(bots)),
                (
                    "without furnaces or rocks",
                    world_without_furnaces_or_rocks(bots),
                ),
            ] {
                let net = expand(&[goal(count)], &state, &registry_for(bots), BotId(1))
                    .unwrap_or_else(|e| {
                        panic!("{} packs on {} bots, {}: {}", count, bots.len(), name, e)
                    });
                let plan = schedule(&net, &state, bots).unwrap_or_else(|e| {
                    panic!("{} packs on {} bots, {}: {}", count, bots.len(), name, e)
                });
                common::assert_preconditions_hold_over_time(&net, &state, &plan);
                for action in net.actions() {
                    for (from, _) in net.preds(action.id) {
                        if net.chain_of(from) != net.chain_of(action.id) {
                            cross_chain_edges += 1;
                        }
                    }
                }
            }
        }
    }
    // The guard the note above turns on: if no expansion in this loop puts one
    // chain's action in front of another's, the replay is once again checking
    // a property nothing can break, and the doc comment is a lie.
    assert!(
        cross_chain_edges > 0,
        "no expansion here crossed a chain boundary; this test has stopped \
         discriminating and the comment above says it should not have"
    );
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
