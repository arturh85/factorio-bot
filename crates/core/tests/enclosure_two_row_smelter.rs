//! An open corridor is not a pocket: `TwoRowSmelter`, and the belt that was
//! read as a wall.
//!
//! # What was reported
//!
//! A live run of `TwoRowSmelter` (27 entities, two furnace rows either side of
//! a nine-tile belt row) logged, fourteen times, for a bot that never moved:
//!
//! ```text
//! pre-place check: the character is already walled in here; this placement
//! does not change that and is allowed  player=1 from=[0.5, 0.5]
//! pocket_tiles=1.0
//! ```
//!
//! One reachable tile, for a bot in a corridor that is open at both ends.
//!
//! # What the block actually is
//!
//! Decoded from `scripts/rcontest.lua`'s own blueprint string, the geometry is
//! [`TWO_ROW_SMELTER`] below: `stone-furnace` (a 1.4-tile box, so 2x2 tiles) at
//! `y = -2` and `y = 3`, `burner-inserter` rows at `y = -0.5` and `y = 1.5`,
//! and nine `transport-belt` at `y = 0.5`. Grown by a character's own half-box
//! the furnaces block tile rows `-2.5/-1.5` and `2.5/3.5`, which leaves rows
//! `-0.5`, `0.5` and `1.5` — the three-tile corridor.
//!
//! A bot on the belt row **between an input and an output inserter**, at
//! `[7.5, 0.5]`, has an inserter north, an inserter south, and a belt on
//! either side. If a belt is a wall, that is a pocket of exactly one tile, and
//! `pocket_tiles=1.0` is honest. It is not a wall: a character walks over a
//! belt, and out either end of the row.
//!
//! (The live reading was at `[0.5, 0.5]` because the block was **sited by
//! search** from the roster's centroid, so its world anchor is not the
//! blueprint's origin. The shape is what reproduces, not the coordinate.)
//!
//! # Why a wrong `Enclosed` is worse than a wrong number
//!
//! `crates/executor::pre_place` matches `(Escape::Enclosed, _)` and allows the
//! placement — correctly, since a bot already sealed in is not this
//! placement's doing. So **one false `Enclosed` switches the guard off for
//! that bot for the rest of the run**, and every later placement logs its own
//! informed consent.

use factorio_bot_core::graph::enclosure::{Escape, escape_from};
use factorio_bot_core::graph::entity_graph::EntityGraph;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_recipes};
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use std::sync::Arc;

/// `TwoRowSmelter`, decoded from the blueprint string in
/// `scripts/rcontest.lua`: `(name, x, y, direction)`.
const TWO_ROW_SMELTER: [(&str, f64, f64, u8); 27] = [
    ("iron-chest", 5.5, -3.5, 0),
    ("burner-inserter", 5.5, -2.5, 0),
    ("stone-furnace", 7.0, -2.0, 0),
    ("stone-furnace", 9.0, -2.0, 0),
    ("stone-furnace", 11.0, -2.0, 0),
    ("iron-chest", 3.5, -1.5, 0),
    ("transport-belt", 5.5, -1.5, 8),
    ("burner-inserter", 3.5, -0.5, 0),
    ("transport-belt", 5.5, -0.5, 8),
    ("burner-inserter", 7.5, -0.5, 8),
    ("burner-inserter", 9.5, -0.5, 8),
    ("burner-inserter", 11.5, -0.5, 8),
    ("transport-belt", 3.5, 0.5, 4),
    ("transport-belt", 4.5, 0.5, 4),
    ("transport-belt", 5.5, 0.5, 4),
    ("transport-belt", 6.5, 0.5, 4),
    ("transport-belt", 7.5, 0.5, 4),
    ("transport-belt", 8.5, 0.5, 4),
    ("transport-belt", 9.5, 0.5, 4),
    ("transport-belt", 10.5, 0.5, 4),
    ("transport-belt", 11.5, 0.5, 4),
    ("burner-inserter", 7.5, 1.5, 0),
    ("burner-inserter", 9.5, 1.5, 0),
    ("burner-inserter", 11.5, 1.5, 0),
    ("stone-furnace", 7.0, 3.0, 0),
    ("stone-furnace", 9.0, 3.0, 0),
    ("stone-furnace", 11.0, 3.0, 0),
];

/// Where the bot stands: the belt tile with an inserter directly north and
/// another directly south.
fn in_the_corridor() -> Position {
    Position::new(7.5, 0.5)
}

/// The block, with every `transport-belt` optionally replaced by `swap`.
fn smelter_graph(swap: Option<&str>) -> EntityGraph {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let graph = EntityGraph::new(prototypes.clone(), Arc::new(fixture_recipes()));
    let mut built = Vec::with_capacity(TWO_ROW_SMELTER.len());
    for (name, x, y, direction) in TWO_ROW_SMELTER {
        let name = match (swap, name) {
            (Some(replacement), "transport-belt") => replacement,
            _ => name,
        };
        let entity = FactorioEntity::from_prototype(
            name,
            Position::new(x, y),
            Direction::from_u8(direction),
            None,
            None,
            prototypes.clone(),
        )
        .expect("every prototype in the block is in the fixture");
        assert!(
            entity.bounding_box.width() > 0.,
            "no collision box for {name} -- it would vanish from the occupancy model",
        );
        built.push(entity);
    }
    graph.add(built, None).expect("the block loads");
    graph
}

/// The defect, stated as the thing that must not happen: the fill must not
/// call an open-ended belt corridor a pocket.
///
/// Before the fix this answered `Enclosed { pocket_tiles: 1.0 }` -- the live
/// run's own number, for the live run's own shape.
#[test]
fn a_bot_on_the_belt_row_can_walk_out_either_end() {
    let graph = smelter_graph(None);
    assert_eq!(
        escape_from(&graph, &in_the_corridor()),
        Escape::Open,
        "a character walks over a belt, so the belt row is a corridor and not a wall"
    );
}

/// The control, and the reason the test above is not passing because the fill
/// went blind: swap the nine belts for nine `iron-chest`, which are the same
/// one-tile obstacle in every respect except that a character collides with
/// them, and the *same* fill finds the *same* shape and calls it a pocket of
/// exactly one tile.
///
/// So the fill still sees this geometry, and `pocket_tiles=1.0` is the number
/// it produces for it. What changed is only whether a belt is counted.
#[test]
fn the_same_corridor_walled_with_chests_is_a_one_tile_pocket() {
    let graph = smelter_graph(Some("iron-chest"));
    assert_eq!(
        escape_from(&graph, &in_the_corridor()),
        Escape::Enclosed { pocket_tiles: 1.0 },
        "inserter north, inserter south, chest east and west is a real pocket"
    );
}

/// The other two rows of the corridor, at an x with no inserter on either.
///
/// They are open for the same reason the belt row is -- the way out of each
/// runs *through* the belt row -- so this pins that the whole three-tile
/// corridor is one connected piece of walkable ground, not just the tile the
/// bot happened to stand on. Both rows also read `pocket_tiles=1.0` while a
/// belt was a wall.
#[test]
fn the_corridor_rows_beside_the_belt_are_open_ground() {
    let graph = smelter_graph(None);
    for y in [-0.5, 1.5] {
        // x = 8.5 has no inserter on either row.
        assert_eq!(
            escape_from(&graph, &Position::new(8.5, y)),
            Escape::Open,
            "the corridor row at y = {y} is not sealed"
        );
    }
}

/// `blocks_character` is read off the prototype's own mask, in both spellings,
/// and an absent mask blocks.
///
/// The fixture in this repo speaks Factorio 1.x (`player-layer`) and the live
/// 2.1.17 capture speaks 2.0 (`player`); matching one would make this true in
/// tests and false in a run.
#[test]
fn a_belt_does_not_collide_with_a_character_and_a_furnace_does() {
    use factorio_bot_core::graph::enclosure::blocks_character;
    let prototypes = fixture_entity_prototypes();
    for walkable in ["transport-belt", "underground-belt", "splitter"] {
        assert!(
            !blocks_character(&prototypes, walkable),
            "{walkable} declares no player layer, so a character walks over it"
        );
    }
    for wall in ["stone-furnace", "burner-inserter", "iron-chest", "pipe"] {
        assert!(
            blocks_character(&prototypes, wall),
            "{wall} declares the player layer"
        );
    }
    assert!(
        blocks_character(&prototypes, "no-such-entity"),
        "an entity with no prototype at all blocks: an unknown name is not \
         something this can wave a character through"
    );
}
