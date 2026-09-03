//! The walled-in check, against run `run-1788432181-42528`'s own record.
//!
//! # What this run was
//!
//! The furthest this project has got. Four bots, 211 800 ticks, 831 dispatches
//! -- 746 of them bot 1's. Bots 2 and 3 reported *byte-identical* positions
//! from tick 47 940 and 48 000 respectively to the last sample: 163 800 ticks,
//! 77% of the run, without moving. Twenty walks failed, all on bots 2/3/4 and
//! none on bot 1, nineteen of them `failed to path find` before dispatch --
//! including one asking bot 2 for a route to a point **4.2 tiles away**. The
//! run finished its budget and no artefact anywhere named the condition.
//!
//! # The fixture, and the hole in it
//!
//! `run-1788432181-42528-frozen-bots.json` is that run's own data: the last
//! `kind: "bots"` line of `samples.jsonl` (the four real positions) and the
//! `game` array of the last keyframe in `map.jsonl` (1 028 real entities), both
//! copied verbatim.
//!
//! **The keyframe cannot contain what trapped them, and that is structural.**
//! `EntityGraph::snapshot_within` -- the only writer of a keyframe -- reads
//! `entity_tree`, and `EntityGraph::add` admits an allow-list of types to that
//! tree: furnaces, inserters, belts, pipes, machines, poles, generators,
//! containers, `rock-big` and `rock-huge`. Trees, small rocks, cliffs and water
//! reach `blocked_tree` **only**. So the whole class of obstacle a bot is most
//! likely to be boxed in by is absent from the record by construction, and the
//! 1 028 entities here are furnaces and ore.
//!
//! [`the_records_own_entities_leave_every_bot_of_run_13_open`] is that fact,
//! pinned: run against the record alone, all four bots -- including the two
//! that had not moved for 163 800 ticks -- are open ground. Nothing on disk can
//! be joined to reconstruct the wall; `workspace/server-log.txt` holds a full
//! entity and tile dump, but of a different world (its chunks do not contain
//! this run's copper field).
//!
//! So the positive case is a **reconstruction**, and is labelled one:
//! [`the_ring_the_keyframe_drops_encloses_the_two_frozen_bots`] adds the class
//! of entity the keyframe structurally drops, at the radius the game's own
//! refusals bound it to, and asks the same question of the same four real
//! positions. What it establishes is not "this is what happened" but the thing
//! that actually has to hold: the check names bots 2 and 3, and does not name
//! bot 1, which worked freely all run.

use factorio_bot_core::factorio::world::Enclosure;
use factorio_bot_core::graph::enclosure::{Escape, SEARCH_RADIUS, enclosure_at, escape_from};
use factorio_bot_core::graph::entity_graph::EntityGraph;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_recipes};
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Frozen {
    bots: Vec<FrozenBot>,
    entities: Vec<Snapshot>,
}

#[derive(Debug, Deserialize)]
struct FrozenBot {
    id: u32,
    position: Position,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    name: String,
    position: Position,
    direction: u8,
}

fn run_13() -> Frozen {
    serde_json::from_str(include_str!("run-1788432181-42528-frozen-bots.json"))
        .expect("the run-13 fixture parses")
}

impl Frozen {
    fn at(&self, bot: u32) -> Position {
        self.bots
            .iter()
            .find(|b| b.id == bot)
            .unwrap_or_else(|| panic!("run 13 recorded bot {bot}"))
            .position
            .clone()
    }
}

/// The run's recorded entities, rebuilt into a graph through the prototypes so
/// each one gets the collision box the game gave it.
///
/// A keyframe carries name, position and direction and no box -- the box is
/// prototype data, which is exactly what `FactorioEntity::from_prototype`
/// looks up. An entity whose prototype the (1.1-era) fixture set does not know
/// comes back with a zero-width box and `EntityGraph::add` skips it, so a
/// silent hole would be indistinguishable from open ground; the count is
/// asserted instead.
fn graph_of(entities: &[Snapshot], extra: Vec<FactorioEntity>) -> EntityGraph {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let graph = EntityGraph::new(prototypes.clone(), Arc::new(fixture_recipes()));
    let mut built: Vec<FactorioEntity> = Vec::with_capacity(entities.len() + extra.len());
    for snapshot in entities {
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
    built.extend(extra);
    graph.add(built, None).expect("the run's entities load");
    graph
}

/// A closed square ring of trees, spaced so no character-sized gap is left.
///
/// Trees, because trees are the class `snapshot_within` structurally drops (see
/// the module docs) and the class this map is full of -- the one full entity
/// dump in `workspace/` holds over 10 000 of them against 118 big rocks. Half
/// a tile apart, because a tree's box is 0.8 and the character's is 0.4 wide,
/// so anything up to a full tile apart is already sealed; 0.5 leaves no
/// argument about the fill's resolution being what closed the ring.
fn tree_ring(around: &Position, radius: f64) -> Vec<FactorioEntity> {
    let mut ring = Vec::new();
    let mut offset = -radius;
    while offset <= radius {
        for (x, y) in [
            (around.x() + offset, around.y() - radius),
            (around.x() + offset, around.y() + radius),
            (around.x() - radius, around.y() + offset),
            (around.x() + radius, around.y() + offset),
        ] {
            ring.push(FactorioEntity::new_tree(&Position::new(x, y)));
        }
        offset += 0.5;
    }
    ring
}

/// Every bot of run 13 reads as open ground against the run's **own record**,
/// including the two that had not moved for 163 800 ticks.
///
/// This is not the check failing. It is the record being unable to state the
/// condition: the keyframe holds furnaces and ore, and a bot is not boxed in by
/// eleven scattered furnaces. See the module docs for why the class of obstacle
/// that could have done it cannot appear in a keyframe at all.
///
/// It is worth pinning for its own sake, because it is the false-positive
/// guard: 1 028 real entities from a real base, four real positions, and the
/// check claims nothing about any of them.
#[test]
fn the_records_own_entities_leave_every_bot_of_run_13_open() {
    let run = run_13();
    let graph = graph_of(&run.entities, Vec::new());
    for bot in [1, 2, 3, 4] {
        assert_eq!(
            escape_from(&graph, &run.at(bot)),
            Escape::Open,
            "bot {bot} against the record alone"
        );
    }
}

/// With the ring the keyframe drops, the two frozen bots are named and the two
/// working bots are not.
///
/// The radius is bounded by the game, not chosen: at tick 123 918 the
/// pathfinder refused bot 2 a route to `(-56.5, 18.5)`, 4.2 tiles from where it
/// stood, and refused bot 3 a route to `(-60.5, 17.5)`, 5.2 tiles from where it
/// stood -- after `player_path` had also tried four rotated goals around each.
/// Whatever the wall was, it was inside that. Three tiles is the ring that
/// holds both bots (they stood 0.4 tiles apart) and clears the furnaces the run
/// really did build around them.
#[test]
fn the_ring_the_keyframe_drops_encloses_the_two_frozen_bots() {
    let run = run_13();
    let between = Position::new(
        (run.at(2).x() + run.at(3).x()) / 2.,
        (run.at(2).y() + run.at(3).y()) / 2.,
    );
    let graph = graph_of(&run.entities, tree_ring(&between, 3.));

    for bot in [2, 3] {
        match escape_from(&graph, &run.at(bot)) {
            Escape::Enclosed { pocket_tiles } => {
                assert!(
                    pocket_tiles > 0. && pocket_tiles < 36.,
                    "bot {bot}'s pocket is inside a 6x6 ring, got {pocket_tiles}"
                );
            }
            other => panic!("bot {bot} was frozen for 163 800 ticks, got {other:?}"),
        }
    }
    // The half of the pair that matters most. Bot 1 walked this map all run and
    // failed zero walks; a check that fenced it off would be worse than the bug
    // it is here to find. Bot 4 moved as late as tick 204 480.
    for bot in [1, 4] {
        assert_eq!(
            escape_from(&graph, &run.at(bot)),
            Escape::Open,
            "bot {bot} was still walking"
        );
    }
}

/// The condition reaches the ledger naming the bot, the place and the tick --
/// which is the whole point, the run having had all three and recorded none.
#[test]
fn an_enclosure_names_the_bot_the_place_and_the_tick() {
    let run = run_13();
    let between = Position::new(
        (run.at(2).x() + run.at(3).x()) / 2.,
        (run.at(2).y() + run.at(3).y()) / 2.,
    );
    let graph = graph_of(&run.entities, tree_ring(&between, 3.));

    let found = enclosure_at(&graph, 3, &run.at(3), Some(123_918))
        .expect("bot 3 is enclosed by the reconstructed ring");
    assert_eq!(
        (found.player, found.tick, found.searched_tiles),
        (3, Some(123_918), SEARCH_RADIUS)
    );
    assert_eq!(found.at, run.at(3));
    assert!(found.pocket_tiles > 0.);

    assert_eq!(
        enclosure_at(&graph, 1, &run.at(1), Some(123_918)),
        None,
        "an open bot yields no row at all, so nothing has to filter one out later"
    );
}

/// The same bot found stuck in the same place twice is one condition, and the
/// ledger says so.
///
/// It matters here more than it does for placements: run 13 refused bot 3's
/// walk from that one spot on five separate plans, so an exact float test would
/// have written the same finding five times and made one stuck bot read as
/// five.
#[test]
fn one_bot_stuck_in_one_place_is_recorded_once() {
    let world = factorio_bot_core::factorio::world::FactorioWorld::new();
    let row = |at: Position| Enclosure {
        tick: None,
        player: 3,
        at,
        pocket_tiles: 4.,
        searched_tiles: SEARCH_RADIUS,
    };
    assert!(world.record_enclosure(row(Position::new(-56.26953125, 14.68359375))));
    assert!(
        !world.record_enclosure(row(Position::new(-56.2578125, 14.74609375))),
        "the same spot, drifting in the last bits, is the same condition"
    );
    assert!(
        world.record_enclosure(row(Position::new(-40.28515625, -38.66015625))),
        "somewhere else is a new condition"
    );
    assert_eq!(world.enclosures().len(), 2);
    assert_eq!(world.unreported_enclosures().len(), 2);
    assert!(
        world.unreported_enclosures().is_empty(),
        "reported once, and still readable by anyone else"
    );
    assert_eq!(world.enclosures().len(), 2);
}
