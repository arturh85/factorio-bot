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

fn gather(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
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

/// A patch that cannot seat the whole plan refuses it.
///
/// Every iron tile but one is committed before expansion starts, so the first
/// share takes the last tile and the second has nowhere to go. The planner
/// must say so rather than send both bots to the same tile — a refusal at plan
/// time is cheaper than a crash at action 73.
#[test]
fn a_patch_too_small_for_the_roster_is_refused_not_overcommitted() {
    let bots = [BotId(1), BotId(2)];
    let mut state = world(&bots);

    let mut tiles: Vec<Position> = state
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    tiles.dedup_by(|a, b| a.x.total_cmp(&b.x).is_eq() && a.y.total_cmp(&b.y).is_eq());
    assert!(tiles.len() > 1, "the fixture's iron field has many tiles");
    for tile in tiles.iter().take(tiles.len() - 1) {
        state.claim_resource(tile);
    }

    // One tile left, two shares of one ore each.
    let error = expand(
        &[gather("iron-ore", 2)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect_err("the last tile cannot seat two bots");
    assert!(
        error.to_string().contains("iron-ore"),
        "the refusal must name the goal it could not meet: {error}"
    );

    // And the one tile that was left is still plannable on its own, so the
    // refusal is about the second share, not about the patch being unreadable.
    let one = [BotId(1)];
    let mut solo = world(&one);
    for tile in tiles.iter().take(tiles.len() - 1) {
        solo.claim_resource(tile);
    }
    let net = expand(
        &[gather("iron-ore", 2)],
        &solo,
        &registry_for(&one),
        BotId(1),
    )
    .expect("one bot fits on one tile");
    assert_eq!(mining(&net).len(), 1);
}
