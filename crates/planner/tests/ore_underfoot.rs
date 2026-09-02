//! A bot parked on ore is an obstacle to whoever is sent to mine that ore.
//!
//! # `run-1788329146-40305`
//!
//! 175 actions dispatched, 172 succeeded, rungs 1-3 satisfied. Rung 4
//! (`research automation`) then ran eight iterations of ~80-step plans and
//! reported `stuck`, having lost two mines to
//!
//! ```text
//! ERROR: could not start mining for 301 ticks: another character is standing on the copper-ore
//! ```
//!
//! **Which bot, and from which stream.** `events.jsonl` gives the tile and the
//! actor: action 40 at tick 31 452 was `mine 5 copper-ore` on bot 1 with
//! `target (23.5, 53.5)`, and action 58 at tick 46 949 was `mine 10
//! copper-ore` on bot 1 with `target (22.5, 48.5)`. Every one of rung 4's
//! `action_settled` rows names bot 1, so bots 2, 3 and 4 were given no work at
//! all. `samples.jsonl` says where they spent it: after tick 6000 — rung 4
//! began at 6052 — bots 2, 3 and 4 report exactly one position each for the
//! rest of the run, `(22.203, 48.785)`, `(24.324, 57.203)` and
//! `(23.305, 53.77)`, which is where rung 2's copper mine had left them 25 000
//! ticks earlier. Bot 4's box covers `(23.5, 53.5)`; bot 2's covers
//! `(22.5, 48.5)`. The blocker was a parked roster bot both times, and it was
//! not mining anything.
//!
//! # Why the existing spacing did not cover it
//!
//! `PlanState::mining_tile_separation` (see `tile_occupancy.rs`) keeps the
//! tiles of *one plan* far enough apart that no miner can stand on another
//! miner's tile. Both of these characters were standing on ore before the plan
//! existed, and claims are per-`PlanState` — every plan starts with an empty
//! ledger — so nothing in the spacing rule had anything to say about them.
//!
//! # The check these tests make
//!
//! A tile with *any* character's collision box over it is out of selection.
//! Not "any character except the assignee": three of the four selectors that
//! read `PlanState::resource_unclaimed` are handed no bot at all
//! (`resource_supply_at_least` and `resource_seats` come from
//! `Method::applicable` and `Method::concurrency`), and `Mine::expand`'s
//! `ctx.chain_actor` is not the runner — methods emit `Actor::Role` with
//! `pinned: None` and a chain opened because its method `converges` gets no
//! owner, so `schedule` decides. See `PlanState::resource_tile_occupied`.
//!
//! The occupant is not exempted and does not need to be: the last test here is
//! the control that a bot parked on ore still mines its own patch, one tile
//! over.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, PlayerChangedPositionEvent, Position};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::util::{
    nearest_resource_tile, resource_seats, resource_tiles_for,
};
use factorio_bot_planner::state::DEFAULT_RESOURCE_PER_TILE;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

/// Where bot 4 stood, relative to the centre of the tile bot 1 was sent to.
///
/// `(23.305, 53.77)` against `(23.5, 53.5)`, from the run. Used rather than
/// parking a character dead on the centre so the tests exercise the box test
/// rather than an equality, and with the same clearance the run had: the
/// character's own half-box is 0.19921875, so this offset leaves it wholly
/// inside the one tile and shadows no neighbour.
const PARKED_OFFSET: (f64, f64) = (-0.195, 0.27);

/// The fixture's copper field, sorted by distance from `from`, nearest first.
///
/// `PlanState::resource_patches` already sorts each patch's tiles, but the
/// fixture's contiguous field comes back as two or three patches whose
/// membership varies between calls (see that method's doc), so the tiles are
/// flattened and re-sorted here. Ties break on `(x, y)`, the same order every
/// selector uses.
fn tiles_by_distance(state: &PlanState, item: &str, from: &Position) -> Vec<Position> {
    let mut tiles: Vec<Position> = state
        .resource_patches(item)
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| {
        calculate_distance(from, a)
            .total_cmp(&calculate_distance(from, b))
            .then(a.x.total_cmp(&b.x))
            .then(a.y.total_cmp(&b.y))
    });
    tiles
}

/// `fixture_world` carries no players at all, so every character in these
/// worlds is one this function put there and nothing else can account for a
/// refusal.
fn state_with_parked(parked: &[(u8, Position)], roster: &[BotId]) -> PlanState {
    let world = fixture_world();
    for (player_id, position) in parked {
        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: *player_id,
                position: position.clone(),
            })
            .expect("park a player");
    }
    PlanState::from_world(Arc::new(world), roster)
}

/// A character standing on `tile`, offset exactly as the run's bot 4 was.
fn parked_on(tile: &Position) -> Position {
    Position::new(tile.x() + PARKED_OFFSET.0, tile.y() + PARKED_OFFSET.1)
}

fn gather_for(bot: BotId, item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Bot(bot),
    }
}

fn mined_tiles(net: &ActionNetwork) -> Vec<Position> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, .. } => Some(pos.clone()),
            _ => None,
        })
        .collect()
}

/// Every character in `state` that would be standing on `tile` — the mod's own
/// condition, stated as geometry.
fn occupants(state: &PlanState, tile: &Position, parked: &[(u8, Position)]) -> Vec<u8> {
    parked
        .iter()
        .filter(|(_, stand)| state.character_stands_on_tile(stand, tile))
        .map(|(id, _)| *id)
        .collect()
}

/// The run's shape: three bots parked on ore, a fourth sent to mine it.
///
/// The tiles parked on are the ones the planner picks when nobody is standing
/// there, so this is the pre-fix planner's own choice handed back to it.
#[test]
fn a_tile_under_a_parked_bot_is_not_handed_to_another_bot() {
    let roster = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let origin = Position::new(0., 0.);

    // What the planner picks with the ground clear.
    let clear = state_with_parked(&[], &roster);
    let nearest = tiles_by_distance(&clear, "copper-ore", &origin);
    assert!(nearest.len() > 4, "the fixture ships a copper field");
    assert_eq!(
        nearest_resource_tile(&clear, "copper-ore", &origin, 1).as_ref(),
        Some(&nearest[0]),
        "with nobody standing anywhere the nearest tile is the nearest tile"
    );

    // Bots 2, 3 and 4 parked on the three nearest, as rung 2 left them.
    let parked: Vec<(u8, Position)> = [2u8, 3, 4]
        .into_iter()
        .zip(nearest.iter())
        .map(|(id, tile)| (id, parked_on(tile)))
        .collect();
    let state = state_with_parked(&parked, &roster);

    for tile in &nearest[..3] {
        assert_eq!(
            occupants(&state, tile, &parked).len(),
            1,
            "each parked bot shadows exactly the tile it stands on: {tile:?}"
        );
        // Covered, not gone. `Condition::ResourceAvailable` reads the physical
        // count and must stay truthful; only *new selection* is refused.
        assert!(
            state.resource_available(tile, "copper-ore") > 0,
            "the ore under a character is still there: {tile:?}"
        );
        assert_eq!(
            state.resource_unclaimed(tile, "copper-ore"),
            0,
            "but it must not be offered to a mining action: {tile:?}"
        );
    }

    let picked = nearest_resource_tile(&state, "copper-ore", &origin, 1)
        .expect("the rest of the field is free");
    assert!(
        !nearest[..3].contains(&picked),
        "the three tiles with someone standing on them must be walked past, got {picked:?}"
    );
    assert_eq!(
        picked, nearest[3],
        "and the walk goes to the next-nearest tile, not somewhere else"
    );

    // The same thing one level up: bot 1 is asked for the ore, and every tile
    // the network mines is a tile nobody is standing on.
    let net = expand(
        &[gather_for(BotId(1), "copper-ore", 20)],
        &state,
        &registry_for(&roster),
        BotId(1),
    )
    .expect("expands");
    let tiles = mined_tiles(&net);
    assert!(!tiles.is_empty(), "the goal is satisfied by mining");
    for tile in &tiles {
        assert!(
            occupants(&state, tile, &parked).is_empty(),
            "bot 1 was sent to {tile:?}, which {:?} is standing on",
            occupants(&state, tile, &parked)
        );
    }
}

/// The negative control on the control: without the exclusion, the tiles above
/// really were the ones a plan chose.
///
/// If the fixture's geometry were such that the planner never wanted the three
/// nearest tiles anyway, the test above would pass on a planner that had not
/// changed at all.
#[test]
fn the_tiles_the_bots_are_parked_on_are_the_ones_a_clear_plan_takes() {
    let roster = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let origin = Position::new(0., 0.);
    let clear = state_with_parked(&[], &roster);
    let nearest = tiles_by_distance(&clear, "copper-ore", &origin);

    let net = expand(
        &[gather_for(BotId(1), "copper-ore", 20)],
        &clear,
        &registry_for(&roster),
        BotId(1),
    )
    .expect("expands");
    let tiles = mined_tiles(&net);
    assert_eq!(tiles.len(), 1, "20 ore comes out of one tile");
    assert_eq!(
        tiles[0], nearest[0],
        "with the ground clear the plan takes the nearest tile — the one this \
         run's bot 2 or bot 4 was standing on"
    );
}

/// Seats must not promise a bot a tile selection then refuses to hand out.
///
/// Three isolated `uranium-ore` tiles ten tiles apart — far more than
/// `mining_tile_separation`, so each is trivially its own seat — with a roster
/// bot parked on one. `fixture_world` ships no uranium, so every number here
/// is exact.
#[test]
fn seats_and_selection_agree_about_a_tile_with_someone_on_it() {
    const A: (f64, f64) = (100.5, 100.5);
    const B: (f64, f64) = (110.5, 100.5);
    const C: (f64, f64) = (120.5, 100.5);

    fn uranium(parked: &[(u8, Position)]) -> PlanState {
        let world = fixture_world();
        world
            .update_chunk_entities(
                [A, B, C]
                    .into_iter()
                    .map(|(x, y)| {
                        FactorioEntity::new_resource(
                            &Position::new(x, y),
                            Direction::North,
                            "uranium-ore",
                        )
                    })
                    .collect(),
            )
            .unwrap();
        for (player_id, position) in parked {
            world
                .player_changed_position(PlayerChangedPositionEvent {
                    player_id: *player_id,
                    position: position.clone(),
                })
                .expect("park a player");
        }
        PlanState::from_world(Arc::new(world), &[BotId(1), BotId(2)])
    }

    let a = Position::new(A.0, A.1);
    let from = Position::new(95., A.1);

    // Control: nobody standing anywhere, three tiles, three seats, three
    // tiles handed out.
    let clear = uranium(&[]);
    assert_eq!(resource_seats(&clear, "uranium-ore", 100), 3);
    assert_eq!(
        resource_tiles_for(&clear, "uranium-ore", &from, 3 * DEFAULT_RESOURCE_PER_TILE).len(),
        3
    );

    // Bot 2 — on the roster, and given no work — parked on the nearest tile.
    let state = uranium(&[(2, parked_on(&a))]);
    let seats = resource_seats(&state, "uranium-ore", 100);
    assert_eq!(seats, 2, "one of the three tiles has a character on it");

    // `need` is an amount of ore, not a tile count, so the capacity `seats`
    // tiles promise is `seats * DEFAULT_RESOURCE_PER_TILE`. Selection must
    // fulfil exactly that: no more, since the third tile is not available to
    // draw from, and no less, since that would be a seat no bot could be
    // placed on.
    let tiles = resource_tiles_for(
        &state,
        "uranium-ore",
        &from,
        seats * DEFAULT_RESOURCE_PER_TILE,
    );
    assert_eq!(tiles.len() as u32, seats);
    assert!(
        tiles.iter().all(|(tile, _)| *tile != a),
        "the occupied tile must never appear in a selection"
    );
    assert!(
        resource_tiles_for(&state, "uranium-ore", &from, 3 * DEFAULT_RESOURCE_PER_TILE).is_empty(),
        "asking for one more tile's worth than the two free tiles hold must fail closed"
    );
}

/// The control the fix is judged by: a bot standing on ore is not fenced off
/// its own patch.
///
/// A bot parked on a tile is the case where refusing that tile is *not* what
/// the game needs — the mod only complains about a character that is not the
/// miner. The rule makes no exception for it, on purpose (see
/// `PlanState::resource_tile_occupied`: three of the four selectors are handed
/// no bot, and the one that is handed `ctx.chain_actor` is not handed the
/// runner). What that costs is asserted here and it is one tile: the plan is
/// not refused, the patch is not lost, and the bot mines the tile next door.
#[test]
fn a_bot_parked_on_ore_still_mines_its_own_patch_one_tile_over() {
    let roster = [BotId(1)];
    let origin = Position::new(0., 0.);
    let clear = state_with_parked(&[], &roster);
    let underfoot = tiles_by_distance(&clear, "copper-ore", &origin)[0].clone();

    // The one character in the world is the bot that will do the mining.
    let state = state_with_parked(&[(1, parked_on(&underfoot))], &roster);

    let net = expand(
        &[gather_for(BotId(1), "copper-ore", 20)],
        &state,
        &registry_for(&roster),
        BotId(1),
    )
    .expect("a bot standing on ore can still be planned to mine");
    let tiles = mined_tiles(&net);
    assert_eq!(tiles.len(), 1, "20 ore comes out of one tile");

    // Not the patch: the tile it gets is a neighbour of the one under its
    // feet, so the whole cost of not exempting the occupant is one step of
    // `resource_tiles_for`'s already-sorted walk.
    assert!(
        calculate_distance(&tiles[0], &underfoot) <= 1.5,
        "expected the next tile over, got {:?} from {underfoot:?}",
        tiles[0]
    );

    // And the deliberate half: the tile under its own feet is excluded too.
    // If this ever becomes an exemption, it has to be an exemption the
    // *scheduler* can honour, not one expansion guesses at.
    assert_eq!(
        state.resource_unclaimed(&underfoot, "copper-ore"),
        0,
        "no exemption for the occupant, including when the occupant is the only bot"
    );
    assert!(
        state.resource_available(&underfoot, "copper-ore") > 0,
        "and the ore under it is still physically there"
    );
}
