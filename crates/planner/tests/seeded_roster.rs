//! Planning for the roster the game actually hands us.
//!
//! `tests/red_science.rs` builds its world with `world_with_furnaces`, which
//! gives **every** bot two stone furnaces. The live seeding —
//! `Planner::initiate_missing_players_with_default_inventory` — gives each bot
//! *one* furnace, one burner mining drill and one piece of wood, and that one
//! difference is the whole distance between a suite of green tests and a plan
//! that dies against a real server. A fixture generous enough to be readable is
//! not automatically generous in the same direction the code is wrong.
//!
//! So every world here is seeded the way the game seeds it, and the goal asked
//! for is the one `scripts/goal_smoke.lua` asks for: five iron plates, which
//! needs one furnace *per chain* and therefore more furnace than any single bot
//! is carrying as soon as the roster is split.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{schedule, ActionNetwork, BotId, PlanState, PlannerError};
use std::sync::Arc;

/// Bots seeded the way `initiate_missing_players_with_default_inventory` seeds
/// a fresh player: one furnace, one drill, one wood. Not two of anything.
fn seeded_world(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(fixture_world()), bots);
    for bot in bots {
        state.gain(*bot, "wood", 1);
        state.gain(*bot, "stone-furnace", 1);
        state.gain(*bot, "burner-mining-drill", 1);
    }
    state
}

fn roster(n: u8) -> Vec<BotId> {
    (1..=n).map(BotId).collect()
}

fn smoke_goal() -> Goal {
    Goal::Have {
        item: "iron-plate".into(),
        count: 5,
        whose: Holder::Anyone,
    }
}

fn places(net: &ActionNetwork) -> usize {
    net.actions()
        .filter(|a| a.label.starts_with("place stone-furnace"))
        .count()
}

/// The live smoke test's goal, planned and scheduled over the roster it was
/// planned for. Passes for one, two and four bots.
#[test]
fn a_roster_seeded_like_the_game_plans_and_schedules_the_smoke_goal() {
    for n in [1u8, 2, 4] {
        let bots = roster(n);
        let state = seeded_world(&bots);
        let net = expand(&[smoke_goal()], &state, &registry_for(&bots), BotId(1)).expect("expands");
        // One furnace per share, and each bot holds exactly one: the plan needs
        // every bot's own furnace, which is what makes the roster load-bearing.
        assert_eq!(
            places(&net),
            n as usize,
            "{n} bots: one furnace is placed per share"
        );
        let plan =
            schedule(&net, &state, &bots).expect("schedulable on the roster it was split for");
        assert!(plan.makespan > 0, "{n} bots: a real plan takes real time");
    }
}

/// **The invariant behind the fix in `factorio-bot-scripting-lua`.**
///
/// A network is only schedulable on the roster it was expanded for.
/// `SplitAcrossBots` sizes each share against the holdings of the bot it names,
/// so a four-way split spends four bots' starting furnaces. Assigning the whole
/// of it to one bot asks that bot for four furnaces it never had, and the plan
/// dies on whichever `Place` was left for last — reported against the *first*
/// action of the network, because ids say nothing about scheduling order.
///
/// This is what `goal.schedule(plan, bot_count)` used to do when a script
/// scheduled over fewer bots than the run had, and the reason it now expands
/// the goal again for the bots that will actually run it. Asserted here rather
/// than merely described: if a future change lets a narrower roster schedule
/// after all, this test fails and the re-expansion can go.
#[test]
fn a_plan_split_over_a_roster_is_not_schedulable_on_a_subset_of_it() {
    for n in [2u8, 4] {
        let bots = roster(n);
        let split = expand(
            &[smoke_goal()],
            &seeded_world(&bots),
            &registry_for(&bots),
            BotId(1),
        )
        .expect("expands");

        let alone = roster(1);
        let err = schedule(&split, &seeded_world(&alone), &alone)
            .expect_err("one bot cannot spend the whole roster's furnaces");
        match err {
            PlannerError::PreconditionUnsatisfied { bot, condition, .. } => {
                assert_eq!(bot, BotId(1), "{n} bots: the one bot asked to run it");
                assert!(
                    condition.contains("stone-furnace"),
                    "{n} bots: the furnace is what runs out, not something else: {condition}"
                );
            }
            other => panic!("{n} bots: expected a precondition failure, got {other}"),
        }
    }
}

/// One bot, one furnace, a goal needing two — the branch the two-furnace
/// fixture in `red_science.rs` can never reach.
///
/// A red science pack needs iron plates and copper plates, so a single bot's
/// chain smelts twice and places two furnaces. Expansion simulates the first
/// `Place` consuming the one furnace it holds, so the second smelt sees a
/// shortfall and crafts a furnace out of mined stone. Handing every bot two
/// furnaces up front means nothing ever tests that.
#[test]
fn a_bot_holding_one_furnace_crafts_the_second_one_it_needs() {
    let bots = roster(1);
    let state = seeded_world(&bots);
    let goal = Goal::Have {
        item: "automation-science-pack".into(),
        count: 4,
        whose: Holder::Anyone,
    };
    let net = expand(&[goal], &state, &registry_for(&bots), BotId(1)).expect("expands");

    assert_eq!(places(&net), 2, "iron and copper are smelted separately");
    assert!(
        net.actions().any(|a| a.label == "craft 1 stone-furnace"),
        "the second furnace is crafted, since the bot was seeded with one"
    );
    let plan = schedule(&net, &state, &bots).expect("schedulable");
    assert!(plan.makespan > 0);
}
