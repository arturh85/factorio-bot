//! **A book contains books, and the owner's file proves it.**
//!
//! `blueprints/the_rook_book.txt` is `final-spaceship-blueprint.txt` verbatim:
//! a three-entry book whose third entry is another three-entry book. Most 2.x
//! designs are shared this way, and until `decode_item` existed every one of
//! them came back as `NoBlueprint` -- "there is no blueprint here", said of a
//! file containing five.
//!
//! The counts below were taken with Python against the owner's file before any
//! Rust was written:
//!
//! ```text
//! [book] The Rook Platform          active_index 0
//!   - The Rook 3.1                  7,179 entities · 12,568 tiles
//!   - The Rook 3.1 - No Storage     3,373 entities ·  8,378 tiles
//!   [book] Old Version              active_index 0
//!     - The Rook                    4,717 entities ·  8,642 tiles
//!     - The Rook 2.0                6,600 entities · 11,020 tiles
//!     - The Rook 3.0                7,136 entities · 12,568 tiles
//! ```
//!
//! The fixture is committed whole rather than trimmed. A trimmed one would have
//! had to be built by this decoder, and a fixture derived from the code cannot
//! falsify the code -- a trap this repo has been caught by four times. 550 kB
//! is a cheap price for a left-hand side nobody here authored.

use factorio_bot_core::blueprint::{
    BlueprintError, BlueprintItem, ItemKind, MAX_BOOK_DEPTH, decode, decode_document, decode_item,
    encode_item,
};
use std::path::PathBuf;

fn the_book() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/blueprints/the_rook_book.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the fixture at {} is readable: {e}", path.display()))
}

/// The file's own JSON, decompressed without touching the code under test, so
/// the round trip below has an independent left-hand side.
fn the_book_json() -> serde_json::Value {
    use std::io::Read;
    let text = the_book();
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

fn json_of(text: &str) -> serde_json::Value {
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
    serde_json::from_str(&json).expect("our own output is json")
}

/// **The nesting is read as nesting, not flattened.**
///
/// A decoder that walked only the first level would find three entries and two
/// blueprints and look entirely healthy -- which is why this asserts the shape
/// at each level rather than only the flattened total.
#[test]
fn the_book_is_read_two_levels_deep() {
    let item = decode_item(&the_book()).expect("the owner's book reads");
    let BlueprintItem::Book(outer) = &item else {
        panic!("the top level is a book, got {:?}", item.kind());
    };
    assert_eq!(outer.entries.len(), 3, "outer entries");
    assert_eq!(outer.active_index, Some(0));
    assert_eq!(
        outer.entries.iter().map(|e| e.index).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2)],
        "the index rides on the entry's envelope, beside the item"
    );
    assert_eq!(outer.entries[0].item.kind(), ItemKind::Blueprint);
    assert_eq!(outer.entries[1].item.kind(), ItemKind::Blueprint);

    let BlueprintItem::Book(inner) = &outer.entries[2].item else {
        panic!("the third entry is itself a book");
    };
    assert_eq!(inner.entries.len(), 3, "inner entries");
    assert_eq!(inner.active_index, Some(0));

    // And the flattening walks the whole tree, in book order.
    let shapes: Vec<(usize, usize)> = item
        .blueprints()
        .iter()
        .map(|d| (d.entities.len(), d.tiles.len()))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (7_179, 12_568),
            (3_373, 8_378),
            (4_717, 8_642),
            (6_600, 11_020),
            (7_136, 12_568),
        ],
        "five variants, counted with Python before any of this was written"
    );
}

/// **Decode the whole book, re-encode it, compare against the file's own
/// JSON.** The same standard the single blueprint already meets, and the only
/// check a decoder that quietly flattens or drops a sub-book cannot pass.
#[test]
fn the_book_survives_a_round_trip() {
    let item = decode_item(&the_book()).expect("reads");
    assert_eq!(
        json_of(&encode_item(&item)),
        the_book_json(),
        "the re-encoded book must be the book"
    );
}

/// And decoding our own output gives the same model, so the encoder is not
/// merely echoing bytes it was handed.
#[test]
fn the_book_round_trip_is_a_fixed_point() {
    let once = decode_item(&the_book()).expect("reads");
    let twice = decode_item(&encode_item(&once)).expect("our own output reads");
    assert_eq!(once, twice);
}

/// **A book is a document question, not a placement question.**
///
/// Neither single-blueprint entry point may quietly start returning the first
/// entry of a book: there is no single anchor and no single footprint, and a
/// caller asking for "the blueprint" would silently get one fifth of the file.
/// Both refuse -- and they now refuse *by name*, where they used to say
/// `NoBlueprint`, which is true of a book and useless.
#[test]
fn both_single_blueprint_paths_refuse_a_book_by_name() {
    assert_eq!(
        decode(&the_book()).unwrap_err(),
        BlueprintError::Unsupported("blueprint_book".into()),
    );
    assert_eq!(
        decode_document(&the_book()).unwrap_err(),
        BlueprintError::Unsupported("blueprint_book".into()),
    );
}

/// **The recursion is bounded by a named constant, not by the stack.**
///
/// An unbounded recursive decode of attacker-supplied base64 is a
/// stack-overflow primitive, and a stack overflow aborts the process with
/// nothing said about the cause -- the same argument that put
/// `MAX_DECOMPRESSED_BYTES` on the inflater. Built here by nesting empty books
/// one past the cap, so the refusal is attributable to depth and to nothing
/// else, with the just-legal depth checked beside it.
#[test]
fn a_book_nested_past_the_cap_is_refused_rather_than_overflowing() {
    fn nest(depth: usize) -> String {
        let mut json = r#"{"blueprint":{"item":"blueprint","version":562949953421312}}"#.to_owned();
        for _ in 0..depth {
            json = format!(
                r#"{{"blueprint_book":{{"item":"blueprint-book","blueprints":[{json}]}}}}"#
            );
        }
        compress(&json)
    }
    fn compress(json: &str) -> String {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(json.as_bytes()).expect("in-memory write");
        format!(
            "0{}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                e.finish().expect("in-memory finish")
            )
        )
    }
    decode_item(&nest(MAX_BOOK_DEPTH)).expect("a book exactly at the cap still reads");
    assert_eq!(
        decode_item(&nest(MAX_BOOK_DEPTH + 1)).unwrap_err(),
        BlueprintError::TooDeep(MAX_BOOK_DEPTH),
        "one past the cap is refused by name, carrying the cap rather than the depth"
    );
}
