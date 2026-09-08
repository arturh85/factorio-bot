//! **The blueprint this project is ultimately aimed at must be readable.**
//!
//! `blueprints/the_rook_3_1.txt` is a Space Age space platform -- the owner's
//! stated win condition is standing something like it up and sending it to the
//! edge of the solar system. Until this test existed the decoder refused it on
//! two independent counts before ever reaching a third: `wires` and `schedules`
//! are not on the placement allowlist, and `tiles` were refused wholesale. For
//! a platform **the tiles are the ship** -- 12,568 `space-platform-foundation`
//! -- and nothing on it can exist without them.
//!
//! The fixture is committed rather than read from `workspace/`, which is
//! gitignored. A number quoted in a note has to be reachable from `master`, and
//! this repo has already lost the reproducibility of three published results to
//! apparatus that lived only in a removed worktree.
//!
//! **The round trip is the real assertion.** A count only says the decoder
//! reached the end of the array; re-encoding from the model and comparing the
//! JSON says nothing was dropped on the way. Note where the comparison is made:
//! against the *original file's own* decompressed JSON, not against a fixture
//! written alongside the code, so no field can be right merely because the same
//! author wrote both sides of it.

use factorio_bot_core::blueprint::{BlueprintError, decode, decode_document, encode_document};
use std::path::PathBuf;

fn the_rook() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/blueprints/the_rook_3_1.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the fixture at {} is readable: {e}", path.display()))
}

/// The JSON the blueprint string actually carries, decompressed here rather
/// than via any of the code under test -- so the comparison below has an
/// independent left-hand side.
fn the_rook_json() -> serde_json::Value {
    use std::io::Read;
    let text = the_rook();
    let raw = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        text.trim().strip_prefix('0').expect("a version byte"),
    )
    .expect("base64");
    let mut json = Vec::new();
    flate2::read::ZlibDecoder::new(&raw[..])
        .read_to_end(&mut json)
        .expect("zlib");
    serde_json::from_slice(&json).expect("json")
}

/// The counts, taken independently with Python before any Rust was written:
/// 7,179 entities and 12,568 tiles.
#[test]
fn the_rook_decodes_whole() {
    let doc = decode_document(&the_rook()).expect("the target blueprint reads");
    assert_eq!(doc.entities.len(), 7_179, "entities");
    assert_eq!(doc.tiles.len(), 12_568, "tiles");
    assert!(
        doc.tiles
            .iter()
            .all(|t| t.name == "space-platform-foundation"),
        "every tile of a platform is its foundation"
    );
    let wires = doc.wires.expect("the body carries a wires key");
    assert_eq!(wires.len(), 822, "wires");
    assert!(
        doc.extras.contains_key("schedules"),
        "a schedule is kept verbatim rather than refused: {:?}",
        doc.extras.keys().collect::<Vec<_>>()
    );
}

/// **Decode, re-encode, and compare against the file's own JSON.**
///
/// This is the check that a decoder which quietly drops what it does not
/// understand cannot pass. 7,179 entities carry 27 distinct keys between them
/// -- `control_behavior`, `quality`, `priority-list`, `recipe_quality`,
/// `request_filters` and 22 more -- and every one of them has to come back.
#[test]
fn the_rook_survives_a_round_trip() {
    let doc = decode_document(&the_rook()).expect("reads");
    let reencoded = encode_document(&doc);
    let there_and_back: serde_json::Value =
        serde_json::from_str(&restore_json(&reencoded)).expect("our own output is json");
    assert_eq!(
        there_and_back,
        the_rook_json(),
        "the re-encoded blueprint must be the blueprint"
    );
}

/// And the round trip is stable: decoding our own output gives the same model,
/// so the encoder is not merely reproducing bytes it was handed.
#[test]
fn the_round_trip_is_a_fixed_point() {
    let once = decode_document(&the_rook()).expect("reads");
    let twice = decode_document(&encode_document(&once)).expect("our own output reads");
    assert_eq!(once, twice);
}

fn restore_json(text: &str) -> String {
    use std::io::Read;
    let raw = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        text.strip_prefix('0').expect("a version byte"),
    )
    .expect("base64");
    let mut json = String::new();
    flate2::read::ZlibDecoder::new(&raw[..])
        .read_to_string(&mut json)
        .expect("zlib");
    json
}

/// **Reading it is not building it, and `decode` still says so.**
///
/// Nothing in this project can lay a tile, run a wire, or set a recipe on a
/// placed machine, so the placement path must keep refusing The Rook -- by
/// name, as it always has. Accepting it there would be the silent two-thirds
/// build the module exists to prevent: 12,568 tiles and 822 wires would go
/// missing and a bot would be sent to stand turrets on nothing.
#[test]
fn the_placement_path_still_refuses_it_by_name() {
    match decode(&the_rook()) {
        Err(BlueprintError::Unsupported(what)) => {
            assert_eq!(
                what, "schedules",
                "the first body key in key order that placement cannot honour"
            );
        }
        other => panic!("placement must refuse a space platform, got {other:?}"),
    }
}
