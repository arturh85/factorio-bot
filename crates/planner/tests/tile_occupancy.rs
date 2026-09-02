//! A bot mining a tile has to *stand* somewhere, and that somewhere must not
//! be another bot's tile.
//!
//! Run `run-1788313837-06402`, rung 1 (`gather iron ore x20`), died after
//! seven of thirteen mine actions with
//!
//! ```text
//! ERROR: could not start mining for 301 ticks: another character is standing on the iron-ore
//! ```
//!
//! Tile reservation (`tile_reservation.rs`) had already stopped two bots being
//! sent to one tile — and that is what made this visible. Before it, four bots
//! raced for one tile and the losers failed cleanly; after it they were spread
//! neatly across four *adjacent* tiles and blocked each other physically,
//! because the selector had no notion of where a miner has to stand.
//!
//! These tests are written against the plan's tiles and the standing positions
//! the executor can legally produce from them, not against a helper: the
//! defect is a relationship between two actions, and neither action is wrong
//! on its own.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::util::nearest_resource_tile;
use factorio_bot_planner::{
    ActionKind, ActionNetwork, BotId, PlanState, Schedule, expand, registry_for, schedule,
};
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

/// The tile every `Mine` action in the network was assigned.
fn mined_tiles(net: &ActionNetwork) -> Vec<Position> {
    let mut out: Vec<Position> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, .. } => Some(pos.clone()),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    out
}

/// Every scheduled `Mine` action's tile and the bot that will swing at it.
///
/// The bot comes from the schedule rather than from the chain owner because
/// the schedule is the last word: a chain the driver left unowned still ends
/// up on exactly one bot, and it is that bot the ore has to be spaced from.
///
/// Sorted by tile so the pair walk below is reproducible.
fn scheduled_mines(net: &ActionNetwork, plan: &Schedule) -> Vec<(Position, BotId)> {
    let mut out: Vec<(Position, BotId)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, .. } => Some((
                pos.clone(),
                plan.assignment(a.id).expect("every action is scheduled"),
            )),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
    out
}

/// The legal standing position for a bot mining `tile` that comes closest to
/// `other`.
///
/// A bot that mines at all is within `reach` of its own tile: the game enforces
/// `resource_reach_distance` silently, and both `FactorioRcon::player_mine_timed`
/// (`within_resource_reach`) and BotBridge's mining branch refuse a swing from
/// outside it. So the set of positions a miner of `tile` can occupy is
/// `disc(tile, reach)`, and the worst case for `other` is the point of that
/// disc nearest to it — the executor's own aim, `approach_radius(reach)`, is
/// half as far out, which is why this is a bound and not a prediction.
fn worst_standing_position(tile: &Position, other: &Position, reach: f64) -> Position {
    let distance = calculate_distance(tile, other);
    if distance <= reach {
        // The miner could stand on `other` outright.
        return other.clone();
    }
    Position::new(
        tile.x() + (other.x() - tile.x()) / distance * reach,
        tile.y() + (other.y() - tile.y()) / distance * reach,
    )
}

/// The run's shape: four bots, one patch, one goal.
///
/// Before spacing this planned four tiles inside a two-tile square, and any
/// bot standing to mine one of them was standing on another. The assertion is
/// the game's own condition, not a proxy for it: `character_stands_on_tile` is
/// `another character is standing on the <ore>` stated as geometry.
#[test]
fn no_bots_standing_position_lands_on_another_bots_tile() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world(&bots);
    let reach = state
        .bot(BotId(1))
        .expect("bot 1 is in the roster")
        .resource_reach_distance;

    let net = expand(
        &[gather("iron-ore", 20)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");

    let tiles = mined_tiles(&net);
    assert_eq!(tiles.len(), 4, "one mining action per share: {tiles:?}");

    for mine in &tiles {
        for other in &tiles {
            if mine == other {
                continue;
            }
            let stand = worst_standing_position(mine, other, reach);
            assert!(
                !state.character_stands_on_tile(&stand, other),
                "a bot mining {mine} may stand at {stand}, which is on {other} — \
                 the tiles are {:.3} apart, under the {:.3} separation",
                calculate_distance(mine, other),
                state.mining_tile_separation()
            );
        }
    }
}

/// The same property one level up, where the ore is an ingredient of several
/// different items rather than the goal itself, so the mining actions come
/// from four unrelated chains instead of one split.
///
/// **Stated between bots, because that is what the rule is about.** A bot
/// mining one tile stands on the tiles around it and stops *somebody else*
/// mining them; it does not stop itself, because `schedule` runs a bot's
/// actions one at a time (`free_at`). Since claims learned whose timeline they
/// sit on (`crate::state::ClaimRunner`), one bot really does pack its own
/// tiles side by side — and the same-bot half of this test asserts that it
/// does, so the cross-bot half cannot be passing merely because nothing got
/// close.
///
/// Read off the *schedule*, not off the chain owners, so the assertion is
/// about the bots that will really swing. Same runner implies same bot in both
/// directions an owner can be known — an owned chain is offered to one bot with
/// no fallback tier, and an unowned one is welded by `chain_binding` — so two
/// mines on two bots are always two runners, and this is the strongest
/// spelling of the invariant available.
#[test]
fn a_multi_item_plan_never_seats_a_bot_on_another_bots_tile() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let mut state = world(&bots);
    for bot in &bots {
        state.gain(*bot, "stone-furnace", 2);
    }
    let reach = state
        .bot(BotId(1))
        .expect("bot 1 is in the roster")
        .resource_reach_distance;

    let net = expand(
        &[gather("automation-science-pack", 10)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");
    let plan = schedule(&net, &state, &bots).expect("schedulable");

    let mines = scheduled_mines(&net, &plan);
    assert!(mines.len() >= 4, "a real plan mines: {mines:?}");

    let mut cross_bot_pairs = 0;
    let mut packed_same_bot_pairs = 0;
    for (mine, miner) in &mines {
        for (other, owner) in &mines {
            if mine == other {
                continue;
            }
            if miner == owner {
                if calculate_distance(mine, other) < state.mining_tile_separation() {
                    packed_same_bot_pairs += 1;
                }
                continue;
            }
            cross_bot_pairs += 1;
            let stand = worst_standing_position(mine, other, reach);
            assert!(
                !state.character_stands_on_tile(&stand, other),
                "bot {miner:?} mining {mine} may stand at {stand}, which is on \
                 bot {owner:?}'s {other} — the tiles are {:.3} apart, under the \
                 {:.3} separation",
                calculate_distance(mine, other),
                state.mining_tile_separation()
            );
        }
    }
    assert!(
        cross_bot_pairs > 0,
        "nothing was compared across bots: {mines:?}"
    );
    assert!(
        packed_same_bot_pairs > 0,
        "no bot packed two of its own tiles together, so the cross-bot check \
         above is not distinguishing anything: {mines:?}"
    );
}

/// The negative control. Spacing is a rule between *different* mining actions;
/// with only one, it must change nothing at all — the lone bot still gets the
/// tile nearest it, in one action, with no share spread over neighbours.
#[test]
fn one_bot_on_a_large_patch_is_not_spaced_away_from_the_nearest_tile() {
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

    let tiles = mined_tiles(&net);
    assert_eq!(tiles.len(), 1, "no spurious spreading: {tiles:?}");
    assert_eq!(tiles[0], expected);
}

/// And the control that says the assertion above can fail: four tiles picked
/// the way the pre-fix planner picked them — the four nearest, ignoring where
/// anybody stands — really do put a bot on another bot's ore. Without this,
/// `no_bots_standing_position_lands_on_another_bots_tile` could be passing on
/// a patch where no arrangement could ever have collided.
#[test]
fn the_four_nearest_tiles_would_have_collided() {
    let state = world(&[BotId(1)]);
    let reach = state
        .bot(BotId(1))
        .expect("bot 1 is in the roster")
        .resource_reach_distance;
    let origin = Position::new(0., 0.);

    let mut tiles: Vec<Position> = state
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| {
        calculate_distance(&origin, a)
            .total_cmp(&calculate_distance(&origin, b))
            .then(a.x.total_cmp(&b.x))
            .then(a.y.total_cmp(&b.y))
    });
    tiles.truncate(4);
    assert_eq!(tiles.len(), 4);

    let collides = tiles.iter().any(|mine| {
        tiles.iter().any(|other| {
            mine != other
                && state
                    .character_stands_on_tile(&worst_standing_position(mine, other, reach), other)
        })
    });
    assert!(
        collides,
        "the four nearest tiles {tiles:?} must be a collision, or the positive \
         test proves nothing"
    );
}
