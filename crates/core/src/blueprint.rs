//! Decoding a Factorio blueprint string into entities we can place.
//!
//! The format is a version byte (`0`), then base64, then zlib, then JSON.
//! Nothing here touches the game: a blueprint decodes in a unit test, and a
//! block can be planned against a world dump with nothing running.
//!
//! **This decoder refuses by allowlist, not by blocklist, at both levels it
//! reads.** An entity is understood only if every key it carries is one of
//! `entity_number`, `name`, `position`, `direction`, `type`; the blueprint
//! object itself is understood only if every key it carries is one of
//! `entities`, `tiles`, `version`, `item`, `label`, `icons`, `description`,
//! `snap-to-grid`, `absolute-snapping`. Anything else at either level
//! (`items`, `recipe`, `filters`, `connections` on an entity; `wires`,
//! `schedules` on the blueprint object) fails the whole decode with
//! `BlueprintError::Unsupported(<key name>)`. An enumerated blocklist is
//! always one feature behind: naming `items` and `recipe` alone would still
//! silently drop `filters` (inserter/splitter filters) and `connections`
//! (circuit wiring), neither of which happens to appear in this crate's own
//! fixtures, so a blocklist would have looked correct while being wrong. The
//! same is true one level up -- Factorio 2.0 moved circuit wiring out of
//! per-entity `connections` into a top-level `wires` array, and a train
//! blueprint carries a top-level `schedules`; a struct with no
//! `deny_unknown_fields` would decode either successfully with the content
//! silently dropped. A block that quietly builds two thirds of itself is
//! worse than one that refuses loudly, so deny-by-default is the promise
//! here: every refusal names the exact key that tripped it, and there is no
//! path -- entity or blueprint-object -- that drops a key silently.

use crate::types::{Direction, Position, blueprint_direction};
use base64::Engine;
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which half of an underground-belt pair an entity is.
///
/// Carried onto `FactorioEntity::underground_half` (`crates/core/src/types.rs`)
/// so the placement path -- ultimately `rcon_place_entity` in
/// `mods/BotBridge/control.lua`, which sends this as `type` to
/// `surface.create_entity` -- can tell the two halves of a pair apart.
/// `#[serde(rename_all = "snake_case")]` gives the wire spelling `"input"` /
/// `"output"`, which is also Factorio's own `belt_to_ground_type` spelling.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UndergroundHalf {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct BlueprintEntity {
    pub name: String,
    /// Offset from the blueprint's own origin, not a world position.
    pub offset: Position,
    /// Already migrated to Factorio 2.0's 16-point scale.
    pub direction: u8,
    /// `Some` only for `underground-belt`; the blueprint names which half.
    pub underground_half: Option<UndergroundHalf>,
}

/// What a blueprint says about its own grid.
///
/// **`snap-to-grid` is the author's own statement of the block's PITCH** -- how
/// far apart two copies sit when they tile. Measured across
/// `scripts/rcontest.lua`: `FurnaceLine` is 29x11, `MinerLine` 7x21,
/// `StarterScience` 6x11. Nothing else in a blueprint says this: a bounding box
/// over the entities gives the extent of what was drawn, not the period the
/// author intended, and the two differ wherever a design leaves deliberate
/// space beside itself.
///
/// That makes it the missing input for placing a second block *next to* a
/// first, which is the open half of the anchor-persistence work: a recorded
/// anchor says where one block is, and a pitch says where the next one goes.
#[derive(Debug, Clone, PartialEq)]
pub struct BlueprintGrid {
    /// Tiles between one copy of the block and the next, per axis.
    pub pitch: Position,
    /// Whether the author pinned the grid to the world's own origin rather
    /// than to wherever the blueprint is dropped.
    pub absolute: bool,
    /// Where the block sits within its own grid cell, when the author moved it.
    /// `None` is **not** `(0, 0)`: it means the field was absent, and a caller
    /// that needs to know whether the author positioned it deliberately can
    /// tell the difference.
    pub relative_position: Option<Position>,
}

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub entities: Vec<BlueprintEntity>,
    pub version: u64,
    /// The block's own grid, when it declares one.
    ///
    /// **`None` means the blueprint said nothing, never that the pitch is
    /// zero.** Three of the fifteen fixtures declare a grid and twelve do not;
    /// a fabricated default would make "the author tiled this at 29x11" and
    /// "the author never said" the same answer, which is the conflation this
    /// repo has now paid for seven times.
    pub grid: Option<BlueprintGrid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlueprintError {
    NotBase64,
    NotDeflate,
    NotJson,
    NoBlueprint,
    /// Content this build refuses to place rather than silently drop.
    /// Carries the offending key's name, e.g. `"items"` or `"filters"`.
    Unsupported(String),
    /// The compressed payload inflates past [`MAX_DECOMPRESSED_BYTES`].
    ///
    /// zlib is happy to turn a few kilobytes into gigabytes, and the string
    /// reaching `decode` is attacker-shaped: `POST /api/v1/scripts/execute`
    /// is unauthenticated and a script may hand any string at all to
    /// `goal.built`. An unbounded `read_to_end` on the decoder is therefore
    /// a remote memory-exhaustion primitive, and one that reports nothing at
    /// all until the allocator gives up. Carries the cap, not the actual
    /// size -- the actual size is precisely what must never be materialised.
    TooLarge(u64),
}

/// The most a blueprint is allowed to inflate to.
///
/// Deliberately far above anything real: the 179-entity `FurnaceLine` this
/// decoder was built for is ~35 kB of JSON, and a large blueprint *book* is
/// low single-digit megabytes, so 32 MiB refuses nothing a person would send
/// and still bounds the damage a crafted string can do.
pub const MAX_DECOMPRESSED_BYTES: u64 = 32 * 1024 * 1024;

/// The only entity keys this decoder understands. Anything else present on
/// an entity fails the decode by name -- see the module doc for why this is
/// an allowlist rather than an enumerated blocklist.
const ALLOWED_ENTITY_KEYS: &[&str] = &["entity_number", "name", "position", "direction", "type"];

/// The only keys this decoder understands on the blueprint object itself
/// (the value of the envelope's `"blueprint"` key). Anything else -- `wires`
/// (2.0 circuit connections moved out of per-entity `connections`),
/// `schedules` (train blueprints), or any future field -- fails the decode
/// by name, for the same reason as `ALLOWED_ENTITY_KEYS`.
const ALLOWED_BODY_KEYS: &[&str] = &[
    "entities",
    "tiles",
    "version",
    "item",
    "label",
    "icons",
    "description",
    "snap-to-grid",
    "absolute-snapping",
    "position-relative-to-grid",
];

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    entities: Vec<Value>,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    tiles: Vec<Value>,
    // The three grid fields. `snap-to-grid` and `absolute-snapping` were
    // ALLOWLISTED and then had nowhere to go -- decoded without complaint and
    // silently discarded -- which is worse than refusing them, because the
    // allowlist is this module's promise that a key is handled rather than
    // dropped.
    #[serde(default, rename = "snap-to-grid")]
    snap_to_grid: Option<RawPos>,
    #[serde(default, rename = "absolute-snapping")]
    absolute_snapping: bool,
    #[serde(default, rename = "position-relative-to-grid")]
    position_relative_to_grid: Option<RawPos>,
}

#[derive(Deserialize)]
struct RawEntity {
    name: String,
    position: RawPos,
    #[serde(default)]
    direction: u8,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct RawPos {
    x: f64,
    y: f64,
}

/// The inverse of [`decode`]: a blueprint string Factorio -- and `decode` --
/// will read back as exactly these entities.
///
/// Exists so a layout **generated in Rust** can be handed to the same paths a
/// hand-exported blueprint takes: `Goal::Built` takes blueprint *text*, and
/// `scripts/rate_three_drills.lua` measures a block from its string. Without
/// this, a searched layout could be scored offline but never stood up in a
/// game, and the score would have nothing to be checked against.
///
/// Writes a **2.0** blueprint (`BLUEPRINT_VERSION_2_0`), so `direction` is
/// carried on the 16-point scale `BlueprintEntity::direction` already uses
/// and `decode` folds it with `% 16` rather than doubling it. The `type` key
/// is written only for an underground belt, and the entity keys emitted are
/// exactly the ones `ALLOWED_ENTITY_KEYS` admits, so the round trip cannot
/// trip the decoder's own allowlist.
pub fn encode(entities: &[BlueprintEntity]) -> String {
    use std::io::Write;
    let entity_json: Vec<Value> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut obj = serde_json::json!({
                "entity_number": i + 1,
                "name": e.name,
                "position": {"x": e.offset.x(), "y": e.offset.y()},
                "direction": e.direction,
            });
            if let Some(half) = e.underground_half {
                obj["type"] = Value::String(
                    match half {
                        UndergroundHalf::Input => "input",
                        UndergroundHalf::Output => "output",
                    }
                    .to_owned(),
                );
            }
            obj
        })
        .collect();
    let envelope = serde_json::json!({
        "blueprint": {
            "item": "blueprint",
            "version": crate::types::BLUEPRINT_VERSION_2_0,
            "entities": entity_json,
        }
    });
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(envelope.to_string().as_bytes())
        .expect("an in-memory zlib write cannot fail");
    let compressed = encoder
        .finish()
        .expect("an in-memory zlib finish cannot fail");
    format!(
        "0{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    )
}

pub fn decode(text: &str) -> Result<Blueprint, BlueprintError> {
    // The leading byte is the format version, not part of the payload.
    let payload = text.strip_prefix('0').unwrap_or(text);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|_| BlueprintError::NotBase64)?;
    let mut json = Vec::new();
    {
        use std::io::Read;
        // `.take(cap + 1)`, not `.take(cap)`: reading exactly the cap cannot
        // tell "this is precisely the largest allowed blueprint" from "there
        // was more and the reader stopped", and refusing a legal payload for
        // being exactly at the limit is the wrong half of that ambiguity.
        // One byte of headroom makes the overflow observable.
        flate2::read::ZlibDecoder::new(&raw[..])
            .take(MAX_DECOMPRESSED_BYTES + 1)
            .read_to_end(&mut json)
            .map_err(|_| BlueprintError::NotDeflate)?;
        if json.len() as u64 > MAX_DECOMPRESSED_BYTES {
            return Err(BlueprintError::TooLarge(MAX_DECOMPRESSED_BYTES));
        }
    }
    let envelope: Value = serde_json::from_slice(&json).map_err(|_| BlueprintError::NotJson)?;
    let body_value = envelope
        .get("blueprint")
        .cloned()
        .ok_or(BlueprintError::NoBlueprint)?;
    let body_obj = body_value.as_object().ok_or(BlueprintError::NoBlueprint)?;
    for key in body_obj.keys() {
        if !ALLOWED_BODY_KEYS.contains(&key.as_str()) {
            return Err(BlueprintError::Unsupported(key.clone()));
        }
    }
    let body: Body = serde_json::from_value(body_value).map_err(|_| BlueprintError::NotJson)?;

    if !body.tiles.is_empty() {
        return Err(BlueprintError::Unsupported("tiles".into()));
    }

    let mut entities = Vec::with_capacity(body.entities.len());
    for raw_entity in body.entities {
        let obj = raw_entity
            .as_object()
            .ok_or_else(|| BlueprintError::Unsupported("entity is not an object".into()))?;
        for key in obj.keys() {
            if !ALLOWED_ENTITY_KEYS.contains(&key.as_str()) {
                return Err(BlueprintError::Unsupported(key.clone()));
            }
        }
        let e: RawEntity =
            serde_json::from_value(raw_entity).map_err(|_| BlueprintError::NotJson)?;
        let underground_half = match e.kind.as_deref() {
            Some("input") => Some(UndergroundHalf::Input),
            Some("output") => Some(UndergroundHalf::Output),
            _ => None,
        };
        entities.push(BlueprintEntity {
            name: e.name,
            offset: Position::new(e.position.x, e.position.y),
            // The CANONICAL migration (`crates/core/src/types.rs`), not a
            // second copy of it. This module carried its own `VERSION_2_0`
            // and `migrate_direction` for one commit, and they had already
            // drifted: the canonical pair folds with `% 8` / `% 16` before
            // doubling, the copy used `saturating_mul(2)` and folded
            // nothing, so a blueprint carrying a direction >= 8 produced
            // 16..=254 -- a number outside `defines.direction` entirely,
            // handed straight to `create_entity`.
            direction: Direction::to_u8(&blueprint_direction(e.direction, body.version))
                .expect("blueprint_direction folds into 0..=15"),
            underground_half,
        });
    }
    // A grid exists only when the author gave a pitch. `absolute-snapping`
    // alone is meaningless -- it says how to interpret a pitch that is not
    // there -- so it does not conjure one.
    let grid = body.snap_to_grid.map(|pitch| BlueprintGrid {
        pitch: Position::new(pitch.x, pitch.y),
        absolute: body.absolute_snapping,
        relative_position: body
            .position_relative_to_grid
            .map(|p| Position::new(p.x, p.y)),
    });
    Ok(Blueprint {
        entities,
        version: body.version,
        grid,
    })
}

#[cfg(test)]
mod grid_tests {
    use super::decode;

    /// Wrap raw blueprint JSON the way Factorio does: version byte, base64,
    /// zlib. Written here rather than reusing `encode`, which builds a body
    /// from entities and so cannot express the grid fields under test.
    fn encode_for_test(json: &str) -> String {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(json.as_bytes()).expect("in-memory write");
        let compressed = e.finish().expect("in-memory finish");
        format!(
            "0{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, compressed)
        )
    }

    /// **A blueprint that declares a grid keeps it.**
    ///
    /// `snap-to-grid` and `absolute-snapping` were in `ALLOWED_BODY_KEYS` and
    /// had nowhere to land, so they decoded without complaint and were thrown
    /// away. That is worse than refusing them: this module's allowlist is a
    /// promise that a key is *handled*, and the refusal path exists precisely
    /// so nothing is silently dropped.
    ///
    /// The pitch is the load-bearing part. It is the author's own statement of
    /// how far apart two copies of the block sit, and nothing else in a
    /// blueprint carries it — a bounding box over the entities measures what
    /// was drawn, not the period intended.
    #[test]
    fn a_declared_grid_survives_the_decode() {
        let text = encode_for_test(
            r#"{"blueprint":{"item":"blueprint","version":281479278886912,
                "snap-to-grid":{"x":29,"y":11},"absolute-snapping":true,
                "entities":[{"entity_number":1,"name":"stone-furnace",
                             "position":{"x":0,"y":0}}]}}"#,
        );
        let grid = decode(&text)
            .expect("decodes")
            .grid
            .expect("the blueprint declares a grid");
        assert_eq!((grid.pitch.x(), grid.pitch.y()), (29.0, 11.0));
        assert!(grid.absolute);
        assert_eq!(
            grid.relative_position, None,
            "absent is not (0,0): the author never moved it within its cell"
        );
    }

    /// **A blueprint that declares nothing reports nothing.**
    ///
    /// Twelve of the fifteen fixtures are like this. A fabricated default would
    /// make "the author tiled this at 29x11" and "the author never said" the
    /// same answer, which is the conflation this repo has paid for seven times.
    #[test]
    fn no_grid_is_none_and_not_a_zero_pitch() {
        let text = encode_for_test(
            r#"{"blueprint":{"item":"blueprint","version":281479278886912,
                "entities":[{"entity_number":1,"name":"stone-furnace",
                             "position":{"x":0,"y":0}}]}}"#,
        );
        assert_eq!(decode(&text).expect("decodes").grid, None);
    }

    /// `absolute-snapping` alone does not conjure a grid: it says how to
    /// interpret a pitch, and a pitch that is not there has no interpretation.
    #[test]
    fn absolute_snapping_without_a_pitch_is_still_no_grid() {
        let text = encode_for_test(
            r#"{"blueprint":{"item":"blueprint","version":281479278886912,
                "absolute-snapping":true,
                "entities":[{"entity_number":1,"name":"stone-furnace",
                             "position":{"x":0,"y":0}}]}}"#,
        );
        assert_eq!(decode(&text).expect("decodes").grid, None);
    }

    /// `position-relative-to-grid` used to fail the decode BY NAME, because it
    /// was not allowlisted at all — so a blueprint carrying it could not be
    /// built even though the other two grid fields were accepted and dropped.
    /// Now it is modelled, and its presence is distinguishable from absence.
    #[test]
    fn a_relative_position_is_kept_and_distinguishable_from_absent() {
        let text = encode_for_test(
            r#"{"blueprint":{"item":"blueprint","version":281479278886912,
                "snap-to-grid":{"x":6,"y":11},
                "position-relative-to-grid":{"x":-2,"y":3},
                "entities":[{"entity_number":1,"name":"stone-furnace",
                             "position":{"x":0,"y":0}}]}}"#,
        );
        let grid = decode(&text)
            .expect("decodes")
            .grid
            .expect("declares a grid");
        let rel = grid
            .relative_position
            .expect("declares a relative position");
        assert_eq!((rel.x(), rel.y()), (-2.0, 3.0));
        assert!(
            !grid.absolute,
            "absent absolute-snapping is false, not unknown"
        );
    }
}
