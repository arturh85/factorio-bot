//! `OutputParser::parse` reads a stream produced by a game whose JSON schema
//! drifts between versions (see `live_2_1_payloads.rs`). Before this fix, a
//! deserialization failure on any single line was a `panic!`, and this crate
//! builds with `panic = "abort"` in release — so one unparseable event ends
//! the whole bot process, not just the one line that failed to parse. The
//! same was true of `entity_prototypes`/`item_prototypes`/`recipes` (one bad
//! element among many aborts startup) and of an `action_completed` status
//! this parser doesn't recognize.
//!
//! These tests feed `parse` deliberately malformed input and assert two
//! things: it does not panic, and — the half that actually matters — a
//! subsequent well-formed element is still processed afterwards. A parser
//! that stopped silently after the bad line, without ever panicking, would
//! pass a test that only checked "didn't panic"; it would not pass this
//! one. The `action_completed` test additionally asserts the unrecognized
//! status was not recorded as a completion.

use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::Position;

/// A minimal, well-formed `on_some_entity_created` payload: the fields
/// `FactorioEntity` requires, nothing optional populated.
const GOOD_ENTITY: &str = r#"{"name":"stone-furnace","entity_type":"furnace","position":{"x":10.0,"y":10.0},"bounding_box":{"left_top":{"x":9.3,"y":9.3},"right_bottom":{"x":10.7,"y":10.7}},"direction":0}"#;

#[test]
fn a_malformed_entity_created_event_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    // Deliberately truncated / invalid JSON.
    let result = parser.parse(1, "on_some_entity_created", "{\"name\":\"stone-furnace\"");
    assert!(
        result.is_ok(),
        "a malformed event must not propagate as an Err either -- it should be \
         logged and skipped: {result:?}"
    );

    // The event after the bad one must still be parsed and applied to the
    // world -- this is the assertion that distinguishes "skip one event and
    // keep going" from "silently stop parsing after the first failure".
    let result = parser.parse(2, "on_some_entity_created", GOOD_ENTITY);
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );

    let world = parser.world();
    let found = world
        .entity_graph
        .entity_at(&Position::new(10.0, 10.0))
        .is_some();
    assert!(
        found,
        "the entity from the well-formed event that followed the malformed \
         one must have reached the world"
    );
}

#[test]
fn a_malformed_entity_updated_event_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let result = parser.parse(1, "on_some_entity_updated", "not json at all");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let result = parser.parse(2, "on_some_entity_created", GOOD_ENTITY);
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );

    let world = parser.world();
    assert!(
        world
            .entity_graph
            .entity_at(&Position::new(10.0, 10.0))
            .is_some(),
        "parsing must continue past the malformed update"
    );
}

#[test]
fn a_malformed_force_event_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let result = parser.parse(1, "force", "{ this is not valid json");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let result = parser.parse(2, "on_some_entity_created", GOOD_ENTITY);
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );

    let world = parser.world();
    assert!(
        world
            .entity_graph
            .entity_at(&Position::new(10.0, 10.0))
            .is_some(),
        "parsing must continue past the malformed force event"
    );
}

/// A minimal, well-formed `entity_prototypes` element.
const GOOD_ENTITY_PROTOTYPE: &str = r#"{"name":"stone-furnace","entity_type":"furnace","collision_mask":null,"collision_box":{"left_top":{"x":-0.5,"y":-0.5},"right_bottom":{"x":0.5,"y":0.5}},"mine_result":null,"mining_time":null,"mining_speed":null,"crafting_speed":null,"max_underground_distance":null,"fluidbox_prototypes":null}"#;

/// A minimal, well-formed `item_prototypes` element.
const GOOD_ITEM_PROTOTYPE: &str = r#"{"name":"iron-plate","item_type":"item","stack_size":100,"fuel_value":0,"place_result":"","group":"intermediate-products","subgroup":"raw-material"}"#;

/// A minimal, well-formed `recipes` element.
const GOOD_RECIPE: &str = r#"{"name":"iron-plate","valid":true,"enabled":true,"category":"smelting","ingredients":null,"products":{},"hidden":false,"energy":3.2,"order":"a","group":"intermediate-products","subgroup":"raw-material"}"#;

#[test]
fn a_malformed_entity_prototype_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let rest = format!("not json at all${GOOD_ENTITY_PROTOTYPE}");
    let result = parser.parse(1, "entity_prototypes", &rest);
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    assert!(
        world.entity_prototypes.get("stone-furnace").is_some(),
        "the well-formed prototype that followed the malformed one must \
         still reach the world"
    );
}

#[test]
fn a_malformed_item_prototype_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let rest = format!("not json at all${GOOD_ITEM_PROTOTYPE}");
    let result = parser.parse(1, "item_prototypes", &rest);
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    assert!(
        world.item_prototypes.get("iron-plate").is_some(),
        "the well-formed prototype that followed the malformed one must \
         still reach the world"
    );
}

#[test]
fn a_malformed_recipe_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let rest = format!("not json at all${GOOD_RECIPE}");
    let result = parser.parse(1, "recipes", &rest);
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    assert!(
        world.recipes.get("iron-plate").is_some(),
        "the well-formed recipe that followed the malformed one must still \
         reach the world"
    );
}

#[test]
fn an_unrecognized_action_status_is_skipped_not_recorded_and_parsing_continues() {
    let mut parser = OutputParser::new();

    // Neither "ok" nor "fail" -- e.g. a status name introduced by a future
    // BotBridge/Factorio version this parser doesn't know about yet.
    let result = parser.parse(1, "action_completed", "pending 7");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    assert!(
        world.actions.get(&7).is_none(),
        "an action whose status could not be understood must NOT be \
         recorded as completed -- doing so would make the caller believe \
         action 7 finished when it did not"
    );

    // A subsequent, well-understood completion must still be recorded --
    // proving the unrecognized status only skipped its own event rather
    // than wedging the parser.
    let result = parser.parse(2, "action_completed", "ok 8");
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );
    assert_eq!(
        world.actions.get(&8).map(|v| v.clone()),
        Some(String::from("ok")),
        "parsing must continue past the unrecognized status"
    );
}

/// A `fail` completion with no message at all: `"<action_id>"` with nothing
/// after it. `rest.find(' ').unwrap()` panicked on exactly this shape --
/// and a run whose expected outcome is failing actions is precisely the run
/// that produces messageless failures under load, so this is the
/// worst-placed panic in the file: the more the run fails, the more certain
/// it is that no evidence survives. A missing message must still be
/// recorded as a failure -- not dropped, and never silently upgraded to
/// "ok", since that would tell the executor a failing action succeeded.
#[test]
fn a_fail_completion_with_no_message_is_recorded_as_failed_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let result = parser.parse(1, "action_completed", "fail 7");
    assert!(
        result.is_ok(),
        "a fail completion with no message must not propagate as an Err: {result:?}"
    );

    let world = parser.world();
    let recorded = world.actions.get(&7).map(|v| v.clone());
    assert!(
        recorded.is_some(),
        "a failure with no message is still a failure and must be recorded, not dropped"
    );
    assert_ne!(
        recorded.as_deref(),
        Some("ok"),
        "a messageless failure must never be recorded as success"
    );

    // A subsequent, well-formed completion must still be recorded --
    // proving the messageless failure only affected its own event.
    let result = parser.parse(2, "action_completed", "ok 8");
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );
    assert_eq!(
        world.actions.get(&8).map(|v| v.clone()),
        Some(String::from("ok")),
        "parsing must continue past the messageless failure"
    );
}

/// A malformed `entities` line with no `:` separator between the rect and
/// the JSON payload. `rest.find(':').unwrap()` panicked on this shape.
#[test]
fn a_malformed_entities_line_with_no_colon_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let result = parser.parse(1, "entities", "no colon in this line at all");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let result = parser.parse(2, "on_some_entity_created", GOOD_ENTITY);
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );

    let world = parser.world();
    assert!(
        world
            .entity_graph
            .entity_at(&Position::new(10.0, 10.0))
            .is_some(),
        "parsing must continue past the malformed entities line"
    );
}

/// The `tiles` arm has the same rect:payload shape and the same
/// `rest.find(':').unwrap()` as `entities`.
#[test]
fn a_malformed_tiles_line_with_no_colon_is_skipped_and_parsing_continues() {
    let mut parser = OutputParser::new();

    let result = parser.parse(1, "tiles", "no colon in this line at all");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let result = parser.parse(2, "on_some_entity_created", GOOD_ENTITY);
    assert!(
        result.is_ok(),
        "the well-formed event must parse: {result:?}"
    );

    let world = parser.world();
    assert!(
        world
            .entity_graph
            .entity_at(&Position::new(10.0, 10.0))
            .is_some(),
        "parsing must continue past the malformed tiles line"
    );
}

/// One malformed tile in an otherwise well-formed `tiles` chunk. The tile
/// mapper indexed `parts[1]` directly for `player_collidable`, which panics
/// on a tile string with no `:` (a short split has no index 1), and also
/// called `.unwrap()` on the `u8` parse.
#[test]
fn a_malformed_tile_in_a_chunk_is_skipped_and_the_rest_of_the_chunk_is_still_applied() {
    let mut parser = OutputParser::new();

    // "bogus" has no ':' at all (out-of-bounds parts[1]); "grass:0" is
    // well-formed and must still reach the world.
    let result = parser.parse(1, "tiles", "0,0;1,1: bogus,grass:0");
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    let tiles: Vec<String> = world
        .entity_graph
        .tile_tree()
        .iter()
        .map(|(_, (tile, _))| tile.name.clone())
        .collect();
    assert_eq!(
        tiles,
        vec!["grass".to_string()],
        "the malformed tile must be skipped while the well-formed tile in \
         the same chunk is still applied"
    );
}

/// The `graphics` format is `filename:width:height:...` (see the format
/// comment on the arm and `mods/BotBridge/control.lua`'s
/// `writeout_proto_picture_dir`), but the parser read `parts[1]` for both
/// `width` and `height` -- `height` silently got `width`'s value. The only
/// sample data present (`width == height == 1`, both in the format comment
/// and in `FactorioGraphic`'s own `1:1:0:0:0:0:1` doc comment) cannot
/// distinguish correct from buggy, so this test deliberately uses
/// `width != height`.
#[test]
fn a_malformed_graphic_is_skipped_and_a_well_formed_one_keeps_distinct_width_and_height() {
    let mut parser = OutputParser::new();

    // "no-colon-here" has no ':' at all (out-of-bounds parts[1]/parts[2]);
    // the second entry is well-formed with width=3, height=5.
    let result = parser.parse(
        1,
        "graphics",
        "no-colon-here|furnace*__core__/graphics/furnace.png:3:5:0:0:0:0:1",
    );
    assert!(result.is_ok(), "must not propagate as an Err: {result:?}");

    let world = parser.world();
    let graphic = world
        .graphics
        .get("furnace")
        .expect("the well-formed graphic that followed the malformed one must reach the world");
    assert_eq!(graphic.width, 3, "width must come from the width field");
    assert_eq!(
        graphic.height, 5,
        "height must come from the height field, not be a copy of width"
    );
}
