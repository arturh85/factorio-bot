//! A split is as wide as the world can seat, not as wide as the roster.
//!
//! `SplitAcrossBots` used to size a split from the roster and the shortfall
//! alone. Four bots on a patch with three seats therefore made four shares,
//! the fourth of which had nowhere to stand, and the *whole* expansion came
//! back `NoApplicableMethod` — a three-bot plan on a three-seat patch is a
//! perfectly good plan and was refused. Two earlier notes flagged it and
//! declined to fix it, because the obvious fix (hand the splitter a tile
//! count) welds a generic item-splitting method to resources.
//!
//! What crosses the boundary instead is a *number*: `Method::concurrency`
//! asks "how many holders can pursue this goal at once", `Mine` answers it in
//! seats, everything else keeps the default `None`, and `SplitAcrossBots`
//! reads the number without ever learning what ore is.
//!
//! These tests build their own worlds rather than claiming tiles out of
//! `fixture_world`, because a claim also *crowds* its neighbours: the seat
//! count of a partly-claimed fixture patch is an emergent number nobody can
//! read off the test, and a fixture whose central quantity has to be
//! discovered is a fixture that proves whatever it happens to contain. Three
//! pairs of touching tiles, ten tiles apart, is three seats by construction —
//! and, being *six* tiles, also says the count is seats rather than tiles.

use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::test_utils::{
    fixture_entity_prototypes, fixture_item_prototypes, fixture_recipes,
};
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use factorio_bot_planner::error::PlannerError;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::util::resource_seats;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

/// The fixture's prototypes and recipes, with an iron field of exactly the
/// tiles asked for and nothing else on the map.
///
/// Deliberately not `fixture_world()` plus claims — see the module doc.
fn world_with_iron_at(tiles: &[Position]) -> FactorioSurface {
    let world = FactorioSurface::new();
    world
        .update_entity_prototypes(
            fixture_entity_prototypes()
                .iter()
                .map(|p| p.clone())
                .collect(),
        )
        .expect("fixture prototypes load");
    world
        .update_item_prototypes(
            fixture_item_prototypes()
                .iter()
                .map(|p| p.clone())
                .collect(),
        )
        .expect("fixture item prototypes load");
    world
        .update_recipes(fixture_recipes().iter().map(|r| r.clone()).collect())
        .expect("fixture recipes load");
    world
        .update_chunk_entities(
            tiles
                .iter()
                .map(|pos| FactorioEntity::new_resource(pos, Direction::North, "iron-ore"))
                .collect(),
        )
        .expect("the ore field loads");
    world
}

fn state(tiles: &[Position], bots: &[BotId]) -> PlanState {
    PlanState::from_world(Arc::new(world_with_iron_at(tiles)), bots)
}

/// `count` pairs of touching tiles, each pair ten tiles from the next.
///
/// A pair is one seat, not two: its tiles are one tile apart and the
/// separation is ~3.99, so a bot mining either stands on the other. Ten tiles
/// between pairs is comfortably over the separation, so the pairs never
/// interfere.
fn seat_clusters(count: i32) -> Vec<Position> {
    let mut tiles = Vec::new();
    for i in 0..count {
        let x = -20. - 10. * f64::from(i);
        tiles.push(Position::new(x, 40.));
        tiles.push(Position::new(x, 41.));
    }
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

/// Every `Mine` action, as `(tile, count)`, in a stable order.
fn mining(net: &ActionNetwork) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, count, .. } => Some((format!("{}", pos), *count)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// The point of the whole change: a patch that can seat three plans three.
///
/// Before this, the fourth share found no tile, `Mine` refused it, and the
/// error that came back named the *goal* — so the run lost all four shares to
/// a shortage of one seat.
#[test]
fn four_bots_on_a_three_seat_patch_plan_three_shares() {
    let tiles = seat_clusters(3);
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let s = state(&tiles, &bots);

    // The fixture's central quantity, stated rather than assumed: six tiles,
    // three seats.
    assert_eq!(tiles.len(), 6);
    assert_eq!(resource_seats(&s, "iron-ore", 10), 3);

    let net = expand(
        &[gather("iron-ore", 40)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("three of four bots still make a plan");

    let mined = mining(&net);
    assert_eq!(
        mined.len(),
        3,
        "one share per seat, not one per bot: {mined:?}"
    );
    assert_eq!(
        mined.iter().map(|(_, count)| count).sum::<u32>(),
        40,
        "the whole goal is still planned, just over fewer bots: {mined:?}"
    );
    let mut tiles_used: Vec<&String> = mined.iter().map(|(tile, _)| tile).collect();
    tiles_used.dedup();
    assert_eq!(tiles_used.len(), 3, "three distinct tiles: {mined:?}");
}

/// The narrow end of the same rule. One seat is one share — not a refusal,
/// and not four bots sent to one tile.
#[test]
fn four_bots_on_a_one_seat_patch_plan_one_share() {
    let tiles = seat_clusters(1);
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let s = state(&tiles, &bots);
    assert_eq!(resource_seats(&s, "iron-ore", 10), 1);

    let net = expand(
        &[gather("iron-ore", 12)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("one seat is still a plan");
    assert_eq!(
        mining(&net).len(),
        1,
        "one mining action: {:?}",
        mining(&net)
    );
    assert_eq!(mining(&net)[0].1, 12, "and it carries the whole goal");
}

/// A patch that can seat nobody still refuses — and now says so.
///
/// Planning zero work would be worse than refusing, because a caller cannot
/// tell an empty plan from a finished goal. The refusal names the shortage:
/// `NoApplicableMethod` said only that the goal could not be met, which reads
/// the same whether the world has no iron ore at all or the plan has already
/// taken every seat on the patch it does have.
#[test]
fn a_patch_that_seats_nobody_is_refused_by_name() {
    let tiles = seat_clusters(1);
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let mut s = state(&tiles, &bots);
    for tile in &tiles {
        s.claim_resource(tile);
    }
    assert_eq!(resource_seats(&s, "iron-ore", 10), 0);

    let error = expand(
        &[gather("iron-ore", 12)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect_err("nowhere to work is not a plan");

    assert!(
        matches!(&error, PlannerError::NoRoomToWork { holders, .. } if *holders == 4),
        "expected NoRoomToWork naming the four bots that were available, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("seat"),
        "the refusal must say what is short, got: {message}"
    );
    assert!(
        message.contains("iron-ore"),
        "and which goal it was short for, got: {message}"
    );
}

/// The negative control, on the very world where iron ore seats three.
///
/// A crafting goal is capped by nothing — `HandCraft` names no limit and
/// `Mine` has nothing to say about an item that is not a resource — so it
/// still splits across the whole roster. Without this, a cap that fired
/// globally rather than per goal would pass every test above.
#[test]
fn a_crafting_goal_still_splits_across_the_whole_roster() {
    let tiles = seat_clusters(3);
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let mut s = state(&tiles, &bots);
    // Two plates each, so the craft needs no mining and the ore's three seats
    // cannot reach this plan by the back door.
    for bot in bots {
        s.gain(bot, "iron-plate", 2);
    }
    assert_eq!(resource_seats(&s, "iron-ore", 10), 3);

    let net = expand(
        &[gather("iron-gear-wheel", 4)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("four gears over four bots");

    let crafts: Vec<u32> = net
        .actions()
        .map(|a| match &a.kind {
            ActionKind::Craft { count, .. } => *count,
            other => panic!("expected crafts only, got {other:?}"),
        })
        .collect();
    assert_eq!(crafts, vec![1, 1, 1, 1], "four shares, one gear each");
    assert_eq!(
        mining(&net),
        Vec::new(),
        "nothing is mined, so nothing was capped"
    );
}

/// Which bots lose their seats is stable. The candidate order is
/// `(spare, BotId)` with `BotId` unique, so the participants and the tiles
/// they get are the same on every run.
#[test]
fn a_narrowed_split_is_deterministic() {
    let tiles = seat_clusters(3);
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let s = state(&tiles, &bots);
    let first = expand(
        &[gather("iron-ore", 40)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");
    let second = expand(
        &[gather("iron-ore", 40)],
        &s,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");
    assert_eq!(mining(&first), mining(&second));

    // ... and it does not depend on the order the caller listed the roster in
    // either, which is what makes "which three of the four" a fact about the
    // world rather than about the call.
    let reversed = [BotId(4), BotId(3), BotId(2), BotId(1)];
    let third = expand(
        &[gather("iron-ore", 40)],
        &state(&tiles, &reversed),
        &registry_for(&reversed),
        BotId(1),
    )
    .expect("expands");
    assert_eq!(mining(&first), mining(&third));
}
