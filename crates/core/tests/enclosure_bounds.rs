//! What the walled-in check refuses to claim, and what it costs.
//!
//! The failure this detector is built against is a bot that was stuck and
//! nobody knew. The failure it could *cause* is worse: a working bot reported
//! as walled in would fence off ground it was using, on the strength of a
//! model rather than of the game. So the interesting tests here are the ones
//! where the answer is deliberately not "enclosed".

use factorio_bot_core::graph::enclosure::{Escape, EscapeUnknown, SEARCH_RADIUS, escape_from};
use factorio_bot_core::graph::entity_graph::EntityGraph;
use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_recipes};
use factorio_bot_core::types::{FactorioEntity, FactorioTile, Position};
use std::sync::Arc;
use std::time::Instant;

fn empty_graph() -> EntityGraph {
    EntityGraph::new(
        Arc::new(fixture_entity_prototypes()),
        Arc::new(fixture_recipes()),
    )
}

/// One water tile. `position` is the tile's *corner*, the convention
/// `EntityGraph::add_tiles` reads: the tile it inserts covers
/// `[x, x + 1] x [y, y + 1]`.
fn water(x: i32, y: i32) -> FactorioTile {
    FactorioTile {
        name: "water".into(),
        player_collidable: true,
        position: Position::new(f64::from(x), f64::from(y)),
        color: None,
        // Nothing observed this tile, so it claims no surface. See
        // `FactorioTile::surface`.
        surface: None,
        fluid: factorio_bot_core::types::TileFluid::Yields {
            fluid: "water".into(),
        },
    }
}

/// A lake `half * 2` tiles square, with a channel of dry land `gap` tiles wide
/// running north-south through it, occupying columns `0..gap`.
///
/// So the dry ground is `[0, gap] x [-half, half]` and a character standing at
/// `(gap / 2, 0.5)` is in the middle of it.
fn lake_with_channel(half: i32, gap: i32) -> Vec<FactorioTile> {
    let mut tiles = Vec::new();
    for x in -half..half {
        if (0..gap).contains(&x) {
            continue;
        }
        for y in -half..half {
            tiles.push(water(x, y));
        }
    }
    tiles
}

/// The searched window is the bound, not the map.
///
/// 240 000 blocking boxes in the tree and the answer still comes back in
/// microseconds, because the quad tree is asked **once**, for a window
/// `SEARCH_RADIUS` tiles across, and the fill then runs over a fixed grid. An
/// unbounded fill over the same tree is the thing this design exists to avoid:
/// the water work alone put ~410 000 boxes in it on a real map.
///
/// No timing is asserted -- a wall-clock assertion in a test suite is a flake
/// waiting to happen. What is asserted is that the work is bounded at all; the
/// measured numbers, from a release build of a 636 000-box world, were 144-348
/// microseconds per call.
#[test]
fn a_dense_map_does_not_make_the_search_expensive() {
    let graph = empty_graph();
    let tiles = lake_with_channel(250, 6);
    let boxes = tiles.len();
    graph.add_tiles(tiles, None).expect("tiles load");
    assert!(boxes > 240_000, "{boxes} blocking boxes");

    let started = Instant::now();
    let verdict = escape_from(&graph, &Position::new(3., 0.5));
    let elapsed = started.elapsed();

    assert_eq!(
        verdict,
        Escape::Open,
        "a 6-tile channel is a way out, whatever is on either side of it"
    );
    assert!(
        elapsed.as_secs() < 5,
        "the fill is bounded by its window, not by the tree: {elapsed:?}"
    );
}

/// A gap a character fits through is a way out, and a wall built along a lake
/// does not change that.
///
/// This is the false-positive guard at its tightest. The channel is one tile
/// wide, on the tile grid; the character's box centred on each of its tiles
/// clears the water on both sides by 0.3 of a tile, so the game's pathfinder
/// routes along it and so must this. A crack *narrower* than a tile is a
/// different case, and the opposite answer -- see `enclosure_run73005.rs`.
#[test]
fn a_gap_a_character_fits_through_is_not_an_enclosure() {
    let graph = empty_graph();
    graph
        .add_tiles(lake_with_channel(40, 1), None)
        .expect("tiles load");
    assert_eq!(escape_from(&graph, &Position::new(0.5, 0.5)), Escape::Open);
}

/// Close that same channel and the verdict flips -- so the test above is
/// measuring the gap and not the fill failing to reach the water at all.
#[test]
fn closing_the_last_gap_is_what_makes_it_an_enclosure() {
    let graph = empty_graph();
    let mut tiles = lake_with_channel(40, 1);
    // Plugs across the channel itself, north and south of the character,
    // leaving it in a corridor with no way out. The only difference from the
    // test above is these two tiles.
    tiles.push(water(0, -6));
    tiles.push(water(0, 6));
    graph.add_tiles(tiles, None).expect("tiles load");

    match escape_from(&graph, &Position::new(0.5, 0.5)) {
        Escape::Enclosed { pocket_tiles } => {
            assert!(
                pocket_tiles == 11.,
                "a 1x11 corridor of whole tiles: {pocket_tiles}"
            );
        }
        other => panic!("the channel is plugged at both ends, got {other:?}"),
    }
}

/// Uncharted ground reads as open, and that is the safe direction.
///
/// `blocked_tree` holds only what the game has told us about. A bot at the edge
/// of the charted area is surrounded, in the model, by nothing -- so the fill
/// escapes through ground nobody has looked at. That produces a missed
/// enclosure, never an invented one, which is the error this detector is
/// allowed to make.
#[test]
fn ground_the_model_knows_nothing_about_reads_as_open() {
    assert_eq!(
        escape_from(&empty_graph(), &Position::new(0., 0.)),
        Escape::Open
    );
}

/// Outside the modelled area the answer is *unknown*, never "open".
///
/// The quad trees are built over a fixed ±5120 tiles from the origin. What the
/// tree does with a box outside that is not established anywhere in this
/// workspace, so a window that reaches the edge would be answered from a query
/// that may have come back short -- and a short query looks exactly like open
/// ground. Three answers, never two: this is the same distinction the walk
/// record keeps between `no_path` and `pathfinder_busy`.
#[test]
fn a_window_off_the_edge_of_the_model_is_unknown_not_open() {
    let graph = empty_graph();
    let just_inside = 5120. - SEARCH_RADIUS - 1.;
    assert_eq!(
        escape_from(&graph, &Position::new(just_inside, 0.)),
        Escape::Open
    );
    assert_eq!(
        escape_from(&graph, &Position::new(5120. - SEARCH_RADIUS + 1., 0.)),
        Escape::Unknown(EscapeUnknown::OutsideModel)
    );
}

/// A character standing inside a collision box still gets an answer.
///
/// The mod teleports a character clear of a ghost it is standing in, so this is
/// a state the game really produces. Refusing to seed the fill there would
/// answer "unknown" for the case most likely to be stuck; instead the
/// character is admitted as free on its own tile, and when every neighbouring
/// tile is under the same machine the pocket comes back as exactly that one
/// tile -- one whole tile, because the grid is the pathfinder's.
#[test]
fn a_character_wedged_inside_a_box_is_enclosed_in_one_tile() {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let graph = EntityGraph::new(prototypes.clone(), Arc::new(fixture_recipes()));
    // A 3x3 machine on tiles [-1, 2) x [-1, 2); the character on its centre
    // tile has the machine on all four sides.
    let machine = FactorioEntity::from_prototype(
        "assembling-machine-1",
        Position::new(0.5, 0.5),
        None,
        None,
        None,
        prototypes,
    )
    .expect("from_prototype does not fail");
    graph.add(vec![machine], None).expect("the machine loads");
    match escape_from(&graph, &Position::new(0.5, 0.5)) {
        Escape::Enclosed { pocket_tiles } => {
            assert_eq!(pocket_tiles, 1., "its own tile, not a pocket");
        }
        other => panic!("the character is inside the machine's box, got {other:?}"),
    }
}
