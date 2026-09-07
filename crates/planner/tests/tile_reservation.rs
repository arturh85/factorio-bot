//! One tile, one mining action.
//!
//! The run that reached milestone 4 died on `the target stone was gone before
//! mining finished -- something else mined it first`, which is what two bots
//! sent to one resource tile look like from the game's side. Expansion picked
//! tiles per bot out of a modelled 500-per-tile capacity and never recorded
//! that a tile had been spoken for, so every bot asked for a small share found
//! the same nearest tile still looking almost full.
//!
//! These tests are written against the *plan*, not against a helper, because
//! the defect was never inside one call to `resource_tiles_for` — that always
//! returned distinct tiles. It was between calls.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::util::nearest_resource_tile;
use factorio_bot_planner::state::DEFAULT_RESOURCE_PER_TILE;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::collections::BTreeMap;
use std::sync::Arc;

fn world(bots: &[BotId]) -> PlanState {
    PlanState::from_world(Arc::new(fixture_world()), bots)
}

/// Every distinct iron tile the fixture carries, in a fixed order.
fn iron_tiles(state: &PlanState) -> Vec<Position> {
    let mut tiles: Vec<Position> = state
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    tiles.dedup_by(|a, b| a.x.total_cmp(&b.x).is_eq() && a.y.total_cmp(&b.y).is_eq());
    assert!(tiles.len() > 1, "the fixture's iron field has many tiles");
    tiles
}

fn gather(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
        via: None,
    }
}

/// Every `Mine` action in the network, as `(tile, item, count)`, in a stable
/// order so a failure message reads the same on every run.
fn mining(net: &ActionNetwork) -> Vec<(String, String, u32)> {
    let mut out: Vec<(String, String, u32)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, item, count } => {
                Some((format!("{}", pos), item.clone(), *count))
            }
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// The run's own shape: several bots gathering one item out of one patch.
///
/// Before the fix this produced four `mine 10 iron-ore` actions on the single
/// tile `[-34.5, 35.5]` — 40 ore committed to one tile — and the bots that
/// shared it raced each other to exhaust it.
#[test]
fn four_bots_gathering_one_item_never_share_a_tile() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world(&bots);
    let net = expand(
        &[gather("iron-ore", 40)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");

    let mined = mining(&net);
    assert_eq!(mined.len(), 4, "one mining action per share: {mined:?}");

    let mut per_tile: BTreeMap<String, u32> = BTreeMap::new();
    for (tile, _, count) in &mined {
        *per_tile.entry(tile.clone()).or_insert(0) += count;
    }
    assert_eq!(
        per_tile.len(),
        mined.len(),
        "no tile may carry two mining actions: {per_tile:?}"
    );
    for (tile, committed) in &per_tile {
        assert!(
            *committed <= DEFAULT_RESOURCE_PER_TILE,
            "tile {tile} committed {committed}, over the {DEFAULT_RESOURCE_PER_TILE} it holds"
        );
    }
    assert_eq!(
        per_tile.values().sum::<u32>(),
        40,
        "the whole goal is still planned: {per_tile:?}"
    );
}

/// The same shape one level up, where the ore is an ingredient rather than the
/// goal: red science needs iron, copper and coal, mined by four separate
/// chains. Every one of those chains used to start at its item's single
/// nearest tile.
#[test]
fn a_multi_item_plan_commits_each_tile_once() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let mut state = world(&bots);
    for bot in &bots {
        state.gain(*bot, "stone-furnace", 2);
    }
    let net = expand(
        &[gather("automation-science-pack", 10)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");

    let mined = mining(&net);
    assert!(mined.len() >= 4, "a real plan mines: {mined:?}");

    let mut per_tile: BTreeMap<String, u32> = BTreeMap::new();
    for (tile, _, count) in &mined {
        *per_tile.entry(tile.clone()).or_insert(0) += count;
    }
    assert_eq!(
        per_tile.len(),
        mined.len(),
        "no tile may carry two mining actions: {per_tile:?}"
    );
    assert!(
        per_tile.values().all(|c| *c <= DEFAULT_RESOURCE_PER_TILE),
        "a tile was committed beyond what it holds: {per_tile:?}"
    );
}

/// The negative control. Exclusivity must not make a lone bot wander: with
/// nothing else competing for the patch, it still gets the nearest tile and
/// one action, not a share of ore spread over several.
#[test]
fn one_bot_on_a_large_patch_still_gets_the_nearest_tile() {
    let bots = [BotId(1)];
    let state = world(&bots);
    let expected = nearest_resource_tile(&state, "iron-ore", &Position::new(0., 0.), 5)
        .expect("the fixture has iron ore");

    let net = expand(
        &[gather("iron-ore", 5)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");

    let mined = mining(&net);
    assert_eq!(mined.len(), 1, "no spurious spreading: {mined:?}");
    assert_eq!(mined[0].0, format!("{}", expected));
    assert_eq!(mined[0].2, 5);
}

/// Claims accumulate in a `BTreeSet` and are consulted by a distance-ordered
/// scan, so which tile each share gets is fixed by the tile set and the origins
/// alone. `expansion_is_deterministic` in `red_science.rs` pins the same
/// property for labels and makespan; this pins it for the thing the labels do
/// not carry — the tile.
#[test]
fn tile_assignment_is_deterministic() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world(&bots);
    let first = expand(
        &[gather("iron-ore", 37)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");
    let second = expand(
        &[gather("iron-ore", 37)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");
    assert_eq!(mining(&first), mining(&second));
}

/// A patch that cannot seat the whole roster is *narrowed*, never
/// overcommitted.
///
/// The patch is committed down to a single usable *seat* — not a single
/// unclaimed tile. Since tiles are spaced (`PlanState::mining_tile_separation`),
/// the last unclaimed tile of an otherwise fully committed patch is no longer
/// usable by anybody: it sits inside the standing room of the tiles around it,
/// which is exactly the condition that made run `run-1788313837-06402` block.
/// So the fixture claims the tiles a *separation* away from one anchor and
/// asserts, rather than assumes, that what is left is one seat.
///
/// **This test used to assert a refusal, and the old expectation was wrong.**
/// One seat and two bots is a one-bot plan, and returning `NoApplicableMethod`
/// for it threw away a plan the world could run — see
/// `tests/split_capacity.rs` for the design that fixed it. What must not
/// happen is the *other* failure, both bots sent to the one seat, and that is
/// what is asserted here: one mining action, carrying the whole goal, on the
/// tile that is actually free. A refusal is still right when the patch seats
/// *nobody*, which `split_capacity.rs` pins.
#[test]
fn a_patch_too_small_for_the_roster_is_narrowed_not_overcommitted() {
    let bots = [BotId(1), BotId(2)];
    let mut state = world(&bots);
    let separation = state.mining_tile_separation();
    let tiles = iron_tiles(&state);
    let anchor = nearest_resource_tile(&state, "iron-ore", &Position::new(0., 0.), 1)
        .expect("the fixture has iron ore");

    let claim: Vec<Position> = tiles
        .iter()
        .filter(|tile| calculate_distance(tile, &anchor) >= separation)
        .cloned()
        .collect();
    for tile in &claim {
        state.claim_resource(tile);
    }

    let seats: Vec<Position> = tiles
        .iter()
        .filter(|tile| state.resource_unclaimed(tile, "iron-ore") > 0)
        .cloned()
        .collect();
    assert!(!seats.is_empty(), "the anchor itself must still be usable");
    for a in &seats {
        for b in &seats {
            assert!(
                a == b || calculate_distance(a, b) < separation,
                "the fixture must leave one seat, not two: {a} and {b}"
            );
        }
    }

    // One seat, so one share of two ore — not two shares of one on the same
    // tile, which is the defect this file exists for.
    let net = expand(
        &[gather("iron-ore", 2)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("one seat is still a plan, for one bot");
    let mined = mining(&net);
    assert_eq!(mined.len(), 1, "one seat, one mining action: {mined:?}");
    assert_eq!(mined[0].2, 2, "and it carries the whole goal: {mined:?}");
    assert_eq!(
        mined[0].0,
        format!("{}", anchor),
        "on the tile that is actually free: {mined:?}"
    );

    // And the same seat plans the same way for a roster of one, so the
    // narrowing above is about the second bot having nowhere to stand and not
    // about the patch having become unreadable.
    let one = [BotId(1)];
    let mut solo = world(&one);
    for tile in &claim {
        solo.claim_resource(tile);
    }
    let net = expand(
        &[gather("iron-ore", 2)],
        &solo,
        &registry_for(&one),
        BotId(1),
    )
    .expect("one bot fits in one seat");
    assert_eq!(mining(&net).len(), 1);
}
