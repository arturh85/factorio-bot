//! **No ground for another furnace is a reason to queue, not a reason to
//! refuse.**
//!
//! # The gate
//!
//! `smelt_steps` sites the furnaces adoption did not supply with
//! `free_area_near`, and that call used to end in
//! `.ok_or_else(|| PlannerError::NoApplicableMethod)?`. So a patch with no
//! free tile inside `FREE_TILE_SEARCH_RADIUS` took out the whole goal —
//! measured on `workspace/scripts/map.json` at four bots, where
//! `have:pumpjack:1` and `researched:oil-gathering` both refused with *no
//! ground for another furnace within 12 tiles* on a patch that was full of
//! furnaces this same plan had built.
//!
//! Raising the radius is not the fix: `power::PLANT_ADOPT_RADIUS` is derived
//! from it, and it would not be one anyway — a patch ringed with this plan's
//! own furnaces wants one of *those*, which is exactly what
//! `adoptable_furnaces` had already ranked and handed over. The growth
//! decision degrades into the reuse decision instead.
//!
//! # The fixture, and why each half of it is the way it is
//!
//! Ground a furnace may not be sited on comes in two kinds here, and the test
//! needs both:
//!
//! * **water**, which `is_area_free` refuses through `collides_with_water`.
//!   The iron patch of `fixture_world` (`x −45..−35, y 35..45`) is drowned out
//!   to 14 tiles, comfortably past the 12-ring `free_area_near` walks. Water
//!   is *tiles*; the ore *entities* on top of it are untouched, so the patch
//!   is still minable and only the siting is blocked;
//! * **ore**, which `free_area_near_where` — the siting search itself, not
//!   `is_area_free` — refuses. That distinction is new as of
//!   `ore-does-not-block` and does not change this fixture: the game builds
//!   over a patch and `is_area_free` now says so, while the search still
//!   keeps a plan off the ore it is about to mine
//!   (`docs/superpowers/notes/2026-09-06-ore-does-not-block.md`). The patch is
//!   where the one standing furnace goes — a tile no search can settle on, so
//!   the furnace is reachable to adoption and invisible to siting. Putting it
//!   on open ground instead would leave the tile beside it free and the plan
//!   would simply build there, testing nothing.
//!
//! `the_fixture_leaves_nowhere_to_site_a_furnace` asserts that, because three
//! of the four tests below would pass by accident on a fixture that left
//! ground free.
//!
//! Two takers then ask for plates. The first adopts the standing furnace and
//! queues a batch into it. The second is a bot with no furnace of its own on
//! the patch, which is `smelt_steps`' `own_grow` — the one arm that asks for a
//! *new* furnace whatever else stands there — and there is nowhere to put it.

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::test_utils::{fixture_world, spawn_water};
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioTile, Position, Rect};
use factorio_bot_planner::action::{ActionKind, InventorySlot};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{ActionNetwork, BotId, PlanState};
use std::sync::Arc;

/// The one tile the standing furnace occupies. Inside the iron patch, so no
/// siting search can ever return it.
const STANDING: Position = Position { x: -40., y: 40. };

fn boxed_in_world() -> FactorioWorld {
    let world = fixture_world();
    // Four bands round the iron patch (`x -45..-35, y 35..45`), 14 tiles deep
    // on every side -- comfortably past the 12 rings `free_area_near` walks
    // from an anchor that is itself a patch tile.
    //
    // **The patch itself stays dry, and that is not a detail.** Drowning it
    // too was the first version of this fixture, and it failed with
    // `NoApplicableMethod { goal: "have 5 iron-ore (bot 1)" }`: a water tile
    // is `player_collidable`, so the bot had nowhere to stand and the ore
    // became unmineable. The test would have been red for a reason that has
    // nothing to do with furnace ground. Ore tiles refuse a furnace to the
    // *siting search* (`free_area_near_where`) and take a character happily,
    // which is exactly the asymmetry this fixture needs -- they do not refuse
    // the placement itself, which the game would allow and
    // `PlanState::is_area_free` allows too.
    let mut tiles: Vec<FactorioTile> = Vec::new();
    for band in [
        Rect::new(&Position::new(-59., 21.), &Position::new(-21., 34.)),
        Rect::new(&Position::new(-59., 46.), &Position::new(-21., 59.)),
        Rect::new(&Position::new(-59., 35.), &Position::new(-46., 45.)),
        Rect::new(&Position::new(-34., 35.), &Position::new(-21., 45.)),
    ] {
        spawn_water(&mut tiles, band);
    }
    world
        .update_chunk_tiles(tiles)
        .expect("the fixture accepts tiles");
    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(
            &STANDING,
            Direction::North,
        ))
        .expect("a furnace may stand on ore even though none may be sited there");
    world
}

fn two_takers() -> Goal {
    Goal::All(vec![
        Goal::Have {
            item: "iron-plate".into(),
            count: 5,
            whose: Holder::Bot(BotId(1)),
        },
        Goal::Have {
            item: "iron-plate".into(),
            count: 5,
            whose: Holder::Bot(BotId(2)),
        },
    ])
}

fn plan() -> (ActionNetwork, PlanState) {
    let bots = [BotId(1), BotId(2)];
    let mut state = PlanState::from_world(Arc::new(boxed_in_world()), &bots);
    // Coal in hand, so nothing here turns on whether the coal patch is
    // reachable: the question is the iron patch's ground and only that.
    for bot in bots {
        state.gain(bot, "coal", 40);
    }
    let net = expand(&[two_takers()], &state, &registry_for(&bots), BotId(1))
        .expect("no ground for a new furnace must not refuse the goal");
    (net, state)
}

fn furnaces_placed(net: &ActionNetwork) -> Vec<Position> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } if entity.name == "stone-furnace" => {
                Some(entity.position.clone())
            }
            _ => None,
        })
        .collect()
}

/// Every furnace the plan loads, by the tile its inserts name.
fn furnaces_loaded(net: &ActionNetwork) -> Vec<Position> {
    let mut seen: Vec<Position> = Vec::new();
    for action in net.actions() {
        let ActionKind::Insert {
            slot: InventorySlot::FurnaceSource,
            pos,
            ..
        } = &action.kind
        else {
            continue;
        };
        if !seen.iter().any(|p| p == pos) {
            seen.push(pos.clone());
        }
    }
    seen
}

/// The fixture leaves nowhere to site a furnace.
///
/// Asserted rather than assumed, because every other test in this file is
/// about a plan that had nowhere to put one, and three of them would pass on a
/// fixture that left ground free -- verified by removing the water and
/// watching them stay green while 91 sites opened up.
///
/// It asks `free_area_near`, which is the question `smelt_steps` asks, and NOT
/// `PlanState::is_area_free`: since `ore-does-not-block` the ore tiles of this
/// patch take a furnace as far as the ground is concerned, and what keeps the
/// plan off them is the siting search's own policy.
#[test]
fn the_fixture_leaves_nowhere_to_site_a_furnace() {
    let bots = [BotId(1)];
    let state = PlanState::from_world(Arc::new(boxed_in_world()), &bots);
    assert!(
        state.is_area_free("stone-furnace", &STANDING.add(&Position::new(3., 0.))),
        "the ground three tiles along the patch is clear -- ore is not an \
         obstacle, so this fixture's refusal cannot come from the ground"
    );
    for anchor in [
        STANDING.clone(),
        Position::new(-45., 35.),
        Position::new(-35., 45.),
    ] {
        assert_eq!(
            factorio_bot_planner::method::util::free_area_near(&state, &anchor, "stone-furnace"),
            None,
            "a furnace was sited from {anchor:?}"
        );
    }
}

/// The gate itself: the goal plans at all.
///
/// Before the fallback this expansion returned
/// `NoApplicableMethod { goal: "have 5 iron-plate for bot 2" }` — the second
/// taker's `own_grow` slot asking for ground that does not exist — and the
/// whole `Goal::All` went with it.
#[test]
fn no_ground_for_a_new_furnace_does_not_refuse_the_goal() {
    let (net, _) = plan();
    assert!(
        net.actions().count() > 0,
        "the plan is not empty; it must actually smelt"
    );
}

/// And it queues on the furnace that is there rather than inventing one.
///
/// Both halves are needed. "Places nothing" alone would pass on a plan that
/// gave up and smelted nothing at all; "loads the standing furnace" alone
/// would pass on a plan that also built one somewhere the water is not.
#[test]
fn it_reuses_the_standing_furnace_instead_of_building_one() {
    let (net, _) = plan();
    assert_eq!(
        furnaces_placed(&net),
        Vec::<Position>::new(),
        "there is nowhere to put a furnace, so none may be placed"
    );
    assert_eq!(
        furnaces_loaded(&net),
        vec![STANDING],
        "both smelts load the one furnace that stands on the ore"
    );
}

/// The reuse is *ordered*, not merely co-located.
///
/// A second batch may only go into a furnace after the take that empties it,
/// and nothing in `infer_edges` can supply that edge — a take's effect is
/// `GainItem`, which satisfies no precondition of an insert. So the fallback
/// has to carry the release of the batch it queues behind, exactly as an
/// adopted slot does. If it handed back `None` there the plan would still
/// place nothing and still load one furnace, and would be wrong.
#[test]
fn the_second_batch_waits_for_the_first_to_be_taken() {
    let (net, _) = plan();
    let inserts: Vec<_> = net
        .actions()
        .filter(|a| {
            matches!(
                &a.kind,
                ActionKind::Insert {
                    slot: InventorySlot::FurnaceSource,
                    ..
                }
            )
        })
        .collect();
    assert!(
        inserts.len() >= 2,
        "two smelts, so at least two loads: {}",
        inserts.len()
    );
    let behind_a_take = inserts.iter().any(|insert| {
        net.preds(insert.id).iter().any(|(pred, _)| {
            matches!(
                net.action(*pred).map(|a| &a.kind),
                Some(ActionKind::Remove {
                    slot: InventorySlot::FurnaceResult,
                    ..
                })
            )
        })
    });
    assert!(
        behind_a_take,
        "the queued batch must be ordered after the take that drains the furnace"
    );
}
