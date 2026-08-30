//! `OutputParser::parse` reads a stream produced by a game whose JSON schema
//! drifts between versions (see `live_2_1_payloads.rs`). Before this fix, a
//! deserialization failure on any single line was a `panic!`, and this crate
//! builds with `panic = "abort"` in release — so one unparseable event ends
//! the whole bot process, not just the one line that failed to parse.
//!
//! These tests feed `parse` a deliberately malformed `on_some_entity_created`
//! line and assert two things: it does not panic, and — the half that
//! actually matters — a subsequent well-formed line is still processed
//! afterwards. A parser that stopped silently after the bad line, without
//! ever panicking, would pass a test that only checked "didn't panic"; it
//! would not pass this one.

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
    assert!(result.is_ok(), "the well-formed event must parse: {result:?}");

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
    assert!(result.is_ok(), "the well-formed event must parse: {result:?}");

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
    assert!(result.is_ok(), "the well-formed event must parse: {result:?}");

    let world = parser.world();
    assert!(
        world
            .entity_graph
            .entity_at(&Position::new(10.0, 10.0))
            .is_some(),
        "parsing must continue past the malformed force event"
    );
}
