//! The walled-in check, against run `run-1788552801-73005`'s own record.
//!
//! # What happened
//!
//! Green science on seed `31337`. Bot 1 walked to place the transport-belt
//! cell's `assembling-machine-1 [31.5, -4.5]`, and the actuator's
//! `approach_annulus` put it on the target's *east* side -- the side it had
//! come from -- at `(34.42, -4.63)`, which is the two-by-two patch of ground
//! between the older cell's chest column (`iron-chest [33.5, -6.5]`,
//! `[33.5, -3.5]`, `[33.5, -2.5]` with an inserter beside each) and its pole
//! and machines (`small-electric-pole [35.5, -4.5]`, `assembling-machine-1
//! [36.5, -6.5]` and `[36.5, -2.5]`). Every exit from that patch that a
//! character can use was to the west, across the ground the assembler was
//! about to cover. Four ticks after arriving the bot placed it, and stood on
//! the same 1/256th of a tile from tick 210 900 to the last sample at
//! 251 040. The game's pathfinder refused it three walks to three different
//! targets, all `failed to path find`, and `record.enclosures()` wrote
//! nothing.
//!
//! # Why the check said nothing, and what this fixture pins
//!
//! `run-1788552801-73005-walled-in.json` is that run's own data: bot 1's
//! frozen position from `samples.jsonl` and the 42 entities within the
//! last keyframe of `map.jsonl` around it, copied verbatim. Unlike run 13's
//! fixture, **this one contains the wall**: it is made of chests, inserters,
//! poles and assemblers, which are exactly the classes a keyframe keeps.
//!
//! The pocket is sealed at the resolution the game's pathfinder uses --
//! `LuaSurface.request_path` at its default `path_resolution_modifier` of 0
//! walks a 1x1-tile grid and tests the character's box at each tile's centre
//! -- and it is *not* sealed at the 1/8-tile configuration-space resolution
//! the fill used to be run at. The chest column leaves a 0.5-tile crack
//! between a chest and its inserter and a 0.65-tile one between that
//! inserter and the assembler; a 0.4-wide character fits through either
//! geometrically, so the fine fill found them and reported `Open`, while
//! the game, testing at tile centres, finds both tiles occupied and refuses.
//! The old module doc argued the fill was "incomplete in the safe direction";
//! this run is the direction it was wrong in.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::graph::enclosure::{Escape, escape_from, escape_with, step_aside_target};
use factorio_bot_core::graph::entity_graph::EntityGraph;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_recipes};
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct WalledIn {
    bot: FrozenBot,
    before_the_walk: Position,
    entities: Vec<Snapshot>,
}

#[derive(Debug, Deserialize)]
struct FrozenBot {
    position: Position,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    name: String,
    position: Position,
    direction: u8,
}

fn run_73005() -> WalledIn {
    serde_json::from_str(include_str!("run-1788552801-73005-walled-in.json"))
        .expect("the run-73005 fixture parses")
}

/// The run's recorded entities, rebuilt through the prototypes so each gets
/// the collision box the game gave it; `skip` names entities left out, so the
/// world can be rebuilt as it stood before a given placement.
fn graph_of(entities: &[Snapshot], skip: &[(&str, f64, f64)]) -> EntityGraph {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let graph = EntityGraph::new(prototypes.clone(), Arc::new(fixture_recipes()));
    let mut built = Vec::with_capacity(entities.len());
    for snapshot in entities {
        if skip.iter().any(|(name, x, y)| {
            *name == snapshot.name && *x == snapshot.position.x() && *y == snapshot.position.y()
        }) {
            continue;
        }
        let entity = FactorioEntity::from_prototype(
            &snapshot.name,
            snapshot.position.clone(),
            Direction::from_u8(snapshot.direction),
            None,
            None,
            prototypes.clone(),
        )
        .expect("from_prototype does not fail");
        assert!(
            entity.bounding_box.width() > 0.,
            "no collision box for {} -- it would vanish from the occupancy model",
            snapshot.name
        );
        built.push(entity);
    }
    graph.add(built, None).expect("the run's entities load");
    graph
}

/// The footprint of the placement that sealed the pocket, as the game
/// collides it: the `assembling-machine-1` box, 2.4 tiles square, at
/// `[31.5, -4.5]`.
fn the_assembler() -> Rect {
    let prototypes = Arc::new(fixture_entity_prototypes());
    FactorioEntity::from_prototype(
        "assembling-machine-1",
        Position::new(31.5, -4.5),
        None,
        None,
        None,
        prototypes,
    )
    .expect("from_prototype does not fail")
    .bounding_box
}

/// Everything bot 1 placed at or after the assembler that sealed it in --
/// the world as it stood when the walk into the pocket was answered.
const PLACED_AFTER_THE_WALK: [(&str, f64, f64); 12] = [
    ("assembling-machine-1", 31.5, -4.5),
    ("assembling-machine-1", 31.5, -8.5),
    ("inserter", 29.5, -8.5),
    ("inserter", 29.5, -7.5),
    ("inserter", 31.5, -6.5),
    ("inserter", 29.5, -4.5),
    ("inserter", 29.5, -5.5),
    ("inserter", 39.5, -0.5),
    ("inserter", 40.5, -0.5),
    ("inserter", 41.5, -2.5),
    ("inserter", 42.5, -0.5),
    ("inserter", 43.5, -0.5),
];

/// Against the run's own entities, bot 1 is enclosed where the game refused
/// it -- in a pocket of exactly four tiles, `x 33..35, y -6..-4`, the patch
/// between the old cell's chest column and its pole.
///
/// **Exact on purpose.** Four is what the game's grid sees: the two tiles the
/// chests sit on are blocked, the two beside the pole are blocked, and the
/// assembler covers the two to the west. A fill that reported any other size
/// is not modelling the pathfinder that refused the walk.
#[test]
fn bot_1_is_walled_into_four_tiles_by_the_runs_own_placements() {
    let run = run_73005();
    let graph = graph_of(&run.entities, &[]);
    assert_eq!(
        escape_from(&graph, &run.bot.position),
        Escape::Enclosed { pocket_tiles: 4. },
        "the game refused three walks from here; the model must agree"
    );
}

/// Take away the one placement and the same spot is open ground -- through
/// the tiles the bot had walked in across four ticks earlier. So the
/// enclosure is the assembler's doing and nothing else's.
#[test]
fn without_the_assembler_the_same_spot_is_open() {
    let run = run_73005();
    let before = graph_of(&run.entities, &PLACED_AFTER_THE_WALK);
    assert_eq!(escape_from(&before, &run.bot.position), Escape::Open);
    assert_eq!(escape_from(&before, &run.before_the_walk), Escape::Open);
}

/// Asked *before* the placement, with the footprint handed in as a
/// hypothetical, the fill closes -- which is the pre-place check the executor
/// now makes, on this exact geometry.
#[test]
fn the_footprint_alone_closes_the_pocket_before_it_is_built() {
    let run = run_73005();
    let before = graph_of(&run.entities, &PLACED_AFTER_THE_WALK);
    assert_eq!(
        escape_with(&before, &run.bot.position, &[the_assembler()]),
        Escape::Enclosed { pocket_tiles: 4. }
    );
}

/// And there was somewhere to step aside to: a tile the bot could reach from
/// the pocket at that moment, that stays connected to open ground once the
/// assembler stands, that is clear of the footprint the character must not
/// stand in, and that is still within building reach of the site -- so the
/// placement goes ahead from there instead of failing.
#[test]
fn a_step_aside_within_reach_existed() {
    let run = run_73005();
    let before = graph_of(&run.entities, &PLACED_AFTER_THE_WALK);
    let site = Position::new(31.5, -4.5);
    let footprint = the_assembler();
    let build_reach = 10.0;
    let to = step_aside_target(
        &before,
        &run.bot.position,
        std::slice::from_ref(&footprint),
        |tile| {
            let clear = tile.x() < footprint.left_top.x() - 0.5
                || tile.x() > footprint.right_bottom.x() + 0.5
                || tile.y() < footprint.left_top.y() - 0.5
                || tile.y() > footprint.right_bottom.y() + 0.5;
            clear && calculate_distance(tile, &site) <= build_reach - 0.5
        },
    )
    .expect("a safe tile within reach of the site");
    // Whole-tile centre, as the pathfinder's grid names it.
    assert_eq!(to.x().fract().abs(), 0.5);
    assert_eq!(to.y().fract().abs(), 0.5);
    // Standing there, the placement no longer seals the bot in.
    let after = graph_of(&run.entities, &PLACED_AFTER_THE_WALK[1..]);
    assert_eq!(escape_from(&after, &to), Escape::Open, "from {to:?}");
    assert!(
        calculate_distance(&to, &site) <= build_reach,
        "{to:?} is within reach"
    );
}
