//! **`SurfaceId`: which surface, by name -- carried, disclosed, and keyed on
//! by nothing yet.**
//!
//! Rung one of `docs/superpowers/notes/2026-09-06-surfaces-survey.md`. Two
//! things are pinned here, and they are pinned separately because they can
//! break separately:
//!
//! 1. **A surface reaches the record.** The mod's `surface_chunk_dropped`
//!    writeout -- `on_chunk_generated` refusing a chunk that is not on Nauvis
//!    -- parses, and folds into one tally per surface rather than one row per
//!    chunk. A generated planet is tens of thousands of chunks; tens of
//!    thousands of identical `events.jsonl` rows is not a disclosure.
//! 2. **Old data still loads.** `surface` is an `Option` on `FactorioEntity`,
//!    `FactorioPlayer` and `FactorioTile`, because `workspace/scripts/` holds
//!    865 MB world dumps written before the field existed. A missing field is
//!    `None`,
//!    which means **"the sender did not say"** -- deliberately not
//!    `Some(nauvis)`. Every archived run *is* Nauvis-only, but that is a fact
//!    about the mod's guard, not about those bytes, and a record field that is
//!    confidently about the wrong object is the one defect a reader cannot
//!    detect.

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::{FactorioEntity, FactorioPlayer, FactorioTile, SurfaceId};
use std::sync::Arc;

/// The wire shape `mods/BotBridge/control.lua` emits, verified against the mod
/// itself in `crates/core/tests/botbridge_surface_guard.rs` -- `left_top` is the
/// chunk's top-left **tile**, not a chunk coordinate.
fn drop_line(surface: &str, x: i32, y: i32) -> String {
    format!(r#"{{"left_top":{{"x":{x},"y":{y}}},"surface":"{surface}"}}"#)
}

#[test]
fn a_dropped_chunk_reaches_the_world_as_a_tally() {
    let world = Arc::new(FactorioWorld::new());
    let mut parser = OutputParser::with_world(world.clone());

    parser
        .parse(
            100,
            "surface_chunk_dropped",
            &drop_line("vulcanus", -32, 64),
        )
        .expect("the drop line must parse");

    let drops = world.drain_surface_chunk_drops();
    let entry = drops
        .get(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must be tallied");
    assert_eq!(entry.chunks, 1);
    assert_eq!(entry.first_tick, 100);
    assert_eq!(entry.first_left_top.x, -32.0);
    assert_eq!(entry.first_left_top.y, 64.0);
}

/// **The fold, which is the design.** Three Vulcanus chunks and one Gleba chunk
/// are two rows, not four -- and the Vulcanus row keeps the *first* chunk and
/// the *first* tick, not the last, because the moment a surface first appeared
/// is the interesting one.
#[test]
fn chunks_fold_per_surface_and_keep_the_first_one_seen() {
    let world = Arc::new(FactorioWorld::new());
    let mut parser = OutputParser::with_world(world.clone());

    for (tick, line) in [
        (100, drop_line("vulcanus", -32, 64)),
        (101, drop_line("vulcanus", 0, 0)),
        (102, drop_line("gleba", 96, -96)),
        (103, drop_line("vulcanus", 32, 32)),
    ] {
        parser
            .parse(tick, "surface_chunk_dropped", &line)
            .expect("drop line");
    }

    let drops = world.drain_surface_chunk_drops();
    assert_eq!(
        drops.keys().map(SurfaceId::as_str).collect::<Vec<_>>(),
        vec!["gleba", "vulcanus"],
        "one row per surface, in name order"
    );

    let vulcanus = &drops[&SurfaceId::from("vulcanus")];
    assert_eq!(vulcanus.chunks, 3);
    assert_eq!(vulcanus.first_tick, 100, "the FIRST tick, not the last");
    assert_eq!(
        (vulcanus.first_left_top.x, vulcanus.first_left_top.y),
        (-32.0, 64.0),
        "the FIRST chunk, not the last"
    );

    let gleba = &drops[&SurfaceId::from("gleba")];
    assert_eq!(gleba.chunks, 1);
    assert_eq!(gleba.first_tick, 102);
}

/// A drain empties the tally: the count on a row is "since the last flush",
/// not "for the run". A reader summing rows must get the run total, and a
/// second flush with nothing in between must write no row at all.
#[test]
fn a_drain_empties_the_tally() {
    let world = Arc::new(FactorioWorld::new());
    let mut parser = OutputParser::with_world(world.clone());

    parser
        .parse(1, "surface_chunk_dropped", &drop_line("vulcanus", 0, 0))
        .expect("drop line");
    assert_eq!(world.drain_surface_chunk_drops().len(), 1);
    assert!(
        world.drain_surface_chunk_drops().is_empty(),
        "a second drain with nothing in between must be empty"
    );

    parser
        .parse(2, "surface_chunk_dropped", &drop_line("vulcanus", 32, 0))
        .expect("drop line");
    let drops = world.drain_surface_chunk_drops();
    assert_eq!(
        drops[&SurfaceId::from("vulcanus")].chunks,
        1,
        "the window restarted at zero, so this row counts one chunk and not two"
    );
    assert_eq!(
        drops[&SurfaceId::from("vulcanus")].first_tick,
        2,
        "and the window's own first tick, not the run's"
    );
}

/// A malformed line must be skipped, not abort the process -- this crate builds
/// with `panic = "abort"` in release, so one unparseable line would end the
/// bot. Same contract as every other arm; see
/// `output_parser_recovers_from_malformed_events.rs`.
#[test]
fn a_malformed_drop_line_is_skipped_and_nothing_is_tallied() {
    let world = Arc::new(FactorioWorld::new());
    let mut parser = OutputParser::with_world(world.clone());

    let result = parser.parse(1, "surface_chunk_dropped", r#"{"surface":"vulcanus""#);
    assert!(result.is_ok(), "must be logged and skipped: {result:?}");
    assert!(world.drain_surface_chunk_drops().is_empty());

    parser
        .parse(2, "surface_chunk_dropped", &drop_line("vulcanus", 0, 0))
        .expect("the next well-formed line must still be processed");
    assert_eq!(world.drain_surface_chunk_drops().len(), 1);
}

/// **The dump-compatibility contract.** These three payloads are the shape the
/// mod emitted *before* this change, so they are what every archived run record
/// and every `workspace/scripts/*.json` dump contains. They must deserialise,
/// and the missing field must read as "not said".
///
/// **What this pins is the behaviour, not the `#[serde(default)]` attribute.**
/// Removing that attribute as a falsification left this test green, and the
/// diagnosis is "the break was not a break": serde already treats an
/// `Option<T>` field as optional, confirmed in a standalone crate. The
/// regression this test does catch is the real one -- somebody making the
/// field required, which is what would silently strand every dump in
/// `workspace/scripts/`.
#[test]
fn a_payload_written_before_surfaces_existed_still_loads() {
    const OLD_ENTITY: &str = r#"{"name":"stone-furnace","entity_type":"furnace","position":{"x":10.0,"y":10.0},"bounding_box":{"left_top":{"x":9.3,"y":9.3},"right_bottom":{"x":10.7,"y":10.7}},"direction":0}"#;
    let entity: FactorioEntity = serde_json::from_str(OLD_ENTITY).expect("old entity must load");
    assert_eq!(entity.surface, None, "absent means the sender did not say");

    const OLD_TILE: &str =
        r#"{"name":"water","player_collidable":true,"position":{"x":1.0,"y":2.0},"color":null}"#;
    let tile: FactorioTile = serde_json::from_str(OLD_TILE).expect("old tile must load");
    assert_eq!(tile.surface, None);

    const OLD_PLAYER: &str = r#"{"player_id":1,"position":{"x":0.0,"y":0.0},"main_inventory":{},"build_distance":10,"reach_distance":10,"drop_item_distance":10,"item_pickup_distance":1.0,"loot_pickup_distance":2.0,"resource_reach_distance":2.7}"#;
    let player: FactorioPlayer = serde_json::from_str(OLD_PLAYER).expect("old player must load");
    assert_eq!(player.surface, None);
}

/// And the new shape round-trips: a bare JSON string, which is what the mod
/// emits (`entity.surface.name`) -- not `{"0":"nauvis"}`.
///
/// Same caveat as above. Removing `#[serde(transparent)]` leaves this green,
/// because a serde newtype struct is already rendered as its inner value; the
/// attribute states the contract rather than creating it. What this test does
/// catch is `SurfaceId` growing a second field, or becoming a struct with
/// named fields -- either of which reshapes every payload the mod sends.
#[test]
fn a_payload_that_names_its_surface_round_trips_as_a_bare_string() {
    const NEW_ENTITY: &str = r#"{"name":"stone-furnace","entity_type":"furnace","position":{"x":10.0,"y":10.0},"bounding_box":{"left_top":{"x":9.3,"y":9.3},"right_bottom":{"x":10.7,"y":10.7}},"direction":0,"surface":"vulcanus"}"#;
    let entity: FactorioEntity = serde_json::from_str(NEW_ENTITY).expect("new entity must load");
    assert_eq!(entity.surface, Some(SurfaceId::from("vulcanus")));

    let json = serde_json::to_value(&entity).expect("serialises");
    assert_eq!(
        json.get("surface"),
        Some(&serde_json::Value::String(String::from("vulcanus"))),
        "a bare string, not a wrapper object"
    );
}

/// The name is the identity, and the reason is written down in the type's own
/// doc: `LuaSurface.name` is *"unique among surfaces"*, while `LuaSurface.index`
/// *"is assigned when a surface is created, and remains so until it is deleted.
/// Indexes of deleted surfaces can be reused"* (verified in
/// `workspace/factorio-api-docs/runtime-api.json`, `application_version`
/// 2.1.17). An index is an identity within one save; a record read next year
/// needs a name.
#[test]
fn the_default_surface_is_nauvis_and_it_is_a_name() {
    assert_eq!(SurfaceId::default(), SurfaceId::nauvis());
    assert_eq!(SurfaceId::nauvis().as_str(), "nauvis");
    assert_eq!(SurfaceId::nauvis().to_string(), "nauvis");
    assert_eq!(
        serde_json::to_string(&SurfaceId::nauvis()).expect("serialises"),
        r#""nauvis""#
    );
}

/// **The default must NOT leak into a synthesised roster.**
/// `Planner::initiate_missing_players_with_default_inventory` builds bots out of
/// `FactorioPlayer::default()` when no player exists (`--clients 0`). Those bots
/// were never observed anywhere, so they must claim no surface -- otherwise the
/// record would assert a place for a character the game does not have.
#[test]
fn a_default_player_claims_no_surface() {
    assert_eq!(
        FactorioPlayer::default().surface,
        None,
        "a fabricated roster observed nothing and must say so"
    );
}
