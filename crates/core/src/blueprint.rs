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

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub entities: Vec<BlueprintEntity>,
    pub version: u64,
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
];

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    entities: Vec<Value>,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    tiles: Vec<Value>,
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
    Ok(Blueprint {
        entities,
        version: body.version,
    })
}
