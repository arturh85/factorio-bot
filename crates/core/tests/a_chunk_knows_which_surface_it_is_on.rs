//! **The parser routes an entity to the surface the mod named.**
//!
//! Until this landed, `OutputParser` held one `Arc<FactorioSurface>` and every
//! writeout went into it, whatever `FactorioEntity::surface` said. The field
//! had been carried on the wire since 2026-09-06 (`serialize_entity` in
//! `mods/BotBridge/types.lua`) and read by nothing.
//!
//! **That was not hypothetical, and it was measured rather than reasoned
//! about.** Loading the world-record save (`workspace/wrload.toml`, ten
//! surfaces) on 2026-09-07 produced 2,609 `on_some_entity_deleted` writeouts,
//! of which **2,601 named `platform-4`, `platform-2` or `platform-3`** — a
//! space platform's asteroids being destroyed. `on_some_entity_deleted` calls
//! `forget_inventory(&entity.position)` and `entity_graph.remove(&entity)`,
//! both keyed by position alone, so every one of those 2,601 events was
//! applied to **Nauvis**, at coordinates a few tiles from the starting base.
//! That is precisely the aliasing `FactorioWorld` exists to prevent, happening
//! on the only multi-surface world this project has.
//!
//! The mod's Nauvis guard in `on_chunk_generated` never saw any of it: the
//! guard covers *chunk* ingest, and a loaded save generates no chunks, so its
//! drop tally read **0** for the whole run. Per-entity events do not pass
//! through it at all.
//!
//! ## What is pinned here, and what is deliberately not
//!
//! Routing is pinned for the four writeouts whose payload is a
//! `FactorioEntity` and therefore **already names its surface**: the bulk
//! `entities` line and the three `on_some_entity_*` events.
//!
//! `tiles` was **not** routed when this file was written, because its compact
//! header (`x,y;x,y: ...`) had no surface slot. It grew a third field on
//! 2026-09-07 and is routed now; its own tests are in
//! `ground_names_its_own_surface.rs`. `resources` shares that header in the mod
//! but has no caller there and no arm here -- resource entities arrive on the
//! bulk `entities` line below.
//!
//! **The mod's Nauvis guard still stands**, for reasons that are no longer
//! about the wire: `tile_chunks` and `storage.map_area` in `on_chunk_generated`
//! are keyed and accumulated per *chunk coordinate* with no surface in the key,
//! and the initial-discovery replay is hard-coded to `game.surfaces[1]`.

use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::{Position, SurfaceId};
use std::sync::Arc;

/// One entity record in the shape `serialize_entity` emits, with or without a
/// surface. `surface: None` is what every archived run and every 865 MB world
/// dump contains, because the field is newer than they are.
fn entity_json(name: &str, x: f64, y: f64, surface: Option<&str>) -> String {
    let surface = match surface {
        Some(s) => format!(r#","surface":"{s}""#),
        None => String::new(),
    };
    format!(
        r#"{{"name":"{name}","entity_type":"container","direction":0,"position":{{"x":{x},"y":{y}}},"bounding_box":{{"left_top":{{"x":{lx},"y":{ly}}},"right_bottom":{{"x":{rx},"y":{ry}}}}}{surface}}}"#,
        lx = x - 0.4,
        ly = y - 0.4,
        rx = x + 0.4,
        ry = y + 0.4,
    )
}

/// The bulk `entities` writeout: a chunk header, a colon, then the JSON array.
fn entities_line(records: &[String]) -> String {
    format!("0,0;32,32:[{}]", records.join(","))
}

/// What the surface's graph holds at `at`, by name. `find_entities_in_radius`
/// rather than `entity_at`, which answers with an opaque `ItemId`.
fn name_at(surface: &FactorioSurface, at: &Position) -> Option<String> {
    surface
        .entity_graph
        .find_entities_in_radius(at.clone(), 0.1, None, None)
        .first()
        .map(|e| e.name.clone())
}

fn world() -> Arc<FactorioWorld> {
    Arc::new(FactorioWorld::nauvis_only(Arc::new(FactorioSurface::new())))
}

/// **The aliasing case, made a test rather than a claim.**
///
/// A chest at (10, 10) on Nauvis and a chest at (10, 10) on Vulcanus are two
/// chests. Both halves are asserted: each surface holds *its own* chest by
/// name, not merely "a different one". Asserting only that Vulcanus lacks the
/// Nauvis chest would pass if the Vulcanus ingest never ran at all.
#[test]
fn a_chest_at_the_same_tile_on_two_surfaces_does_not_collide() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            10,
            "entities",
            &entities_line(&[
                entity_json("iron-chest", 10.5, 10.5, Some("nauvis")),
                entity_json("wooden-chest", 10.5, 10.5, Some("vulcanus")),
            ]),
        )
        .expect("the entities line must parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");

    let at = Position::new(10.5, 10.5);
    assert_eq!(
        name_at(&nauvis, &at),
        Some("iron-chest".to_string()),
        "the nauvis record must land on nauvis"
    );
    assert_eq!(
        name_at(&vulcanus, &at),
        Some("wooden-chest".to_string()),
        "and the vulcanus record on vulcanus, at the same coordinates"
    );
}

/// **The live world-record-save bug.** A platform asteroid being destroyed at
/// a position where Nauvis holds a chest must not remove the chest.
///
/// Paired with the non-accidental half: the platform surface exists after the
/// deletion and is empty at that position, so "nothing was removed" cannot be
/// explained by the deletion having been dropped on the floor with the surface
/// never created.
#[test]
fn a_deletion_on_another_surface_leaves_this_one_standing() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    let at = Position::new(-14.5, -38.5);
    parser
        .parse(
            1,
            "on_some_entity_created",
            &entity_json("iron-chest", -14.5, -38.5, Some("nauvis")),
        )
        .expect("the created line must parse");

    parser
        .parse(
            2,
            "on_some_entity_deleted",
            &entity_json("small-carbonic-asteroid", -14.5, -38.5, Some("platform-4")),
        )
        .expect("the deleted line must parse");

    let nauvis = world.nauvis().expect("nauvis");
    assert_eq!(
        name_at(&nauvis, &at),
        Some("iron-chest".to_string()),
        "a platform-4 deletion must not reach the nauvis graph"
    );

    let platform = world
        .surface(&SurfaceId::from("platform-4"))
        .expect("the deletion must have routed to a platform-4 surface, not been discarded");
    assert!(
        name_at(&platform, &at).is_none(),
        "and platform-4 holds nothing there either -- the deletion landed, on the right surface"
    );
}

/// **Old data still loads.** Every archived server log and every world dump
/// predates the `surface` field, so a record that names no surface must land
/// on the default rather than on a surface called nothing.
#[test]
fn an_entity_that_names_no_surface_lands_on_the_default() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            1,
            "entities",
            &entities_line(&[entity_json("iron-chest", 4.5, 4.5, None)]),
        )
        .expect("entities line");

    assert_eq!(
        world.len(),
        1,
        "a record that says nothing must not invent a surface"
    );
    let nauvis = world.nauvis().expect("nauvis");
    assert_eq!(
        name_at(&nauvis, &Position::new(4.5, 4.5)),
        Some("iron-chest".to_string()),
    );
}

/// **A routed surface shares the world's globals, by identity.**
///
/// This is the invariant `FactorioWorld::insert_surface` enforces with
/// `Arc::ptr_eq`, and the reason the globals moved off the surface in the
/// first place: two surfaces holding two copies of the research state is a
/// plan that thinks a technology is open on one planet and closed on the
/// other. A surface the *parser* creates has to satisfy it too, and nothing
/// else checks that a created surface went through `with_globals`.
#[test]
fn a_surface_the_parser_creates_shares_the_worlds_globals() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            1,
            "on_some_entity_created",
            &entity_json("iron-chest", 0.5, 0.5, Some("gleba")),
        )
        .expect("created line");

    let gleba = world.surface(&SurfaceId::from("gleba")).expect("gleba");
    assert!(
        Arc::ptr_eq(&gleba.globals, world.globals()),
        "a parser-created surface must share the world's globals object, not an equal copy"
    );
    // Non-accidental: the surface is not merely present, it received the
    // entity. A surface created and then written to somewhere else would pass
    // the ptr_eq above on its own.
    assert!(
        name_at(&gleba, &Position::new(0.5, 0.5)).is_some(),
        "and it is the surface the entity landed on"
    );
}

/// **A surface is created once and then written to, not recreated per line.**
///
/// Found by a falsification that came back green: nothing asserted this, and a
/// `surface_or_create` that made a fresh graph on every call would have passed
/// the whole file. On the world-record save that is 2,264 platform-4 events,
/// 2,263 of which would land in graphs nobody holds — a routing bug that looks
/// exactly like a routing fix, because the entities do leave Nauvis.
///
/// Both halves asserted: the world holds **two** surfaces and not three, *and*
/// both records are in the one Vulcanus graph. Counting surfaces alone would
/// pass if the second write had been dropped entirely.
#[test]
fn two_records_on_one_surface_land_in_one_graph() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    for (n, (x, y)) in [("iron-chest", (1.5, 1.5)), ("wooden-chest", (9.5, 9.5))] {
        parser
            .parse(
                1,
                "on_some_entity_created",
                &entity_json(n, x, y, Some("vulcanus")),
            )
            .expect("created line");
    }

    assert_eq!(
        world.len(),
        2,
        "nauvis and vulcanus -- a second vulcanus record must not make a third surface"
    );
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus");
    assert_eq!(
        name_at(&vulcanus, &Position::new(1.5, 1.5)),
        Some("iron-chest".to_string()),
        "the first record must still be there after the second arrived"
    );
    assert_eq!(
        name_at(&vulcanus, &Position::new(9.5, 9.5)),
        Some("wooden-chest".to_string()),
        "and the second must be in the same graph, not in one nobody holds"
    );
}

/// **The porting seam still refuses to guess.** `only_surface()` answers only
/// while there is one surface; once the parser has seen a second, a caller
/// that never said which surface it meant stops working rather than silently
/// getting Nauvis. That is the designed behaviour of the seam, and routing is
/// the first thing that can make it fire.
#[test]
fn only_surface_stops_answering_once_a_second_surface_is_routed() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    assert!(
        world.only_surface().is_some(),
        "one surface: the seam answers"
    );

    parser
        .parse(
            1,
            "on_some_entity_created",
            &entity_json("iron-chest", 0.5, 0.5, Some("vulcanus")),
        )
        .expect("created line");

    assert_eq!(world.len(), 2);
    assert!(
        world.only_surface().is_none(),
        "two surfaces: the seam must refuse rather than pick one"
    );
}

/// **A routed deletion does its work, on the surface it was routed to.**
///
/// The gap this closes: `a_deletion_on_another_surface_leaves_this_one_standing`
/// asserts platform-4 is empty at that tile afterwards, and platform-4 was
/// *never* occupied there — so a deletion that routed correctly and then did
/// nothing at all passes it, as does one dropped on the floor into a surface
/// nobody holds. Both halves are asserted here instead: the chest is standing
/// on platform-4 after the create, and gone after the delete, in the same
/// graph the world holds.
///
/// This is the same shape as `two_records_on_one_surface_land_in_one_graph`
/// and exists for the same reason — a routing that returns a fresh surface per
/// call removes from a graph nobody reads, which looks exactly like a working
/// deletion.
#[test]
fn a_deletion_removes_from_the_surface_it_was_routed_to() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    let at = Position::new(-14.5, -38.5);
    parser
        .parse(
            1,
            "on_some_entity_created",
            &entity_json("iron-chest", -14.5, -38.5, Some("platform-4")),
        )
        .expect("the created line must parse");

    let platform = world
        .surface(&SurfaceId::from("platform-4"))
        .expect("the creation must have routed to a platform-4 surface");
    assert_eq!(
        name_at(&platform, &at),
        Some("iron-chest".to_string()),
        "the chest must be standing on platform-4 before anything is deleted"
    );

    parser
        .parse(
            2,
            "on_some_entity_deleted",
            &entity_json("iron-chest", -14.5, -38.5, Some("platform-4")),
        )
        .expect("the deleted line must parse");

    assert_eq!(
        world.len(),
        2,
        "nauvis and platform-4 -- the deletion must reuse the surface the creation made"
    );
    assert!(
        name_at(&platform, &at).is_none(),
        "and the deletion must have removed it from THAT graph, not from one nobody holds"
    );
}
