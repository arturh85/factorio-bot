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
//!
//! # Reading is not building, and they are two functions
//!
//! [`decode`] answers *"can this be stood up by `Goal::Built`"*. Its allowlist
//! is therefore a statement about the **placement path**, not about the
//! format: `tiles`, `wires` and `schedules` are refused because nothing in
//! this project can lay a tile, run a wire or write a train schedule, and
//! accepting them would be exactly the silent two-thirds build the paragraph
//! above forbids.
//!
//! [`decode_document`] answers the different question *"what does this
//! blueprint say"*. It refuses nothing structural and **drops nothing**: every
//! body key and every entity key it does not model in its own right is carried
//! verbatim in an `extras` map, and [`encode_document`] writes them all back.
//! That is what makes the round trip a real check rather than a tautology --
//! the JSON is rebuilt from the model, so anything the model failed to keep
//! shows up as a difference.
//!
//! The two share one parse: `decode` is `decode_document` plus the placement
//! policy, so the allowlist can never drift from what is actually read.
//!
//! The motivating file is `crates/core/tests/blueprints/the_rook_3_1.txt`, a
//! Space Age space platform of **7,179 entities and 12,568
//! `space-platform-foundation` tiles** -- the owner's stated win condition. For
//! a platform the tiles *are* the ship, so a decoder that refuses tiles cannot
//! read the one blueprint this project is aimed at.

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

/// One tile of ground the blueprint paints.
///
/// On Nauvis this is concrete or brick; on a space platform it is
/// `space-platform-foundation`, and **there is no ship without it** -- The Rook
/// is 12,568 of these and 7,179 entities standing on them.
#[derive(Debug, Clone, PartialEq)]
pub struct BlueprintTile {
    pub name: String,
    /// Offset from the blueprint's own origin, not a world position.
    pub offset: Position,
}

/// One circuit or copper wire, in Factorio 2.0's top-level `wires` form.
///
/// 2.0 moved wiring out of per-entity `connections` into a flat array of
/// four-element rows, each naming both ends by `entity_number` and connector
/// id. **The entity numbers are the join**, which is why
/// [`DocumentEntity::number`] is kept as written rather than re-derived from
/// the position in the array: renumber the entities and every wire in the
/// blueprint points somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlueprintWire {
    pub from_entity: u64,
    pub from_connector: u64,
    pub to_entity: u64,
    pub to_connector: u64,
}

/// An entity as the blueprint wrote it, with nothing thrown away.
///
/// The difference from [`BlueprintEntity`] is `extras` and the two `raw_`
/// fields. `BlueprintEntity` is the *placement* view -- what a bot needs to
/// stand the thing up -- and deliberately models only what this project can
/// act on. This is the *reading* view, so a key like `control_behavior`,
/// `recipe`, `quality` or `items` is kept verbatim instead of failing the
/// decode. 567 of The Rook's entities carry `control_behavior` alone.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentEntity {
    /// `entity_number` exactly as written, and `None` when the key was absent
    /// -- which is a blueprint no wire can reference. Wires join on this; see
    /// [`BlueprintWire`].
    pub number: Option<u64>,
    pub name: String,
    /// Offset from the blueprint's own origin, not a world position.
    pub offset: Position,
    /// Migrated to Factorio 2.0's 16-point scale, like
    /// [`BlueprintEntity::direction`].
    pub direction: u8,
    /// `direction` as the file wrote it, *before* migration, and `None` when
    /// the key was absent. Both facts are needed to write the blueprint back:
    /// the 1.x migration doubles and so cannot be inverted, and 2,728 of The
    /// Rook's entities omit the key entirely rather than writing a zero.
    pub raw_direction: Option<u64>,
    /// `type` verbatim. `underground_half` interprets it; this keeps it, so a
    /// `type` that is neither `input` nor `output` is not silently lost.
    pub kind: Option<String>,
    /// `Some` when `kind` is one of the two underground-belt halves.
    pub underground_half: Option<UndergroundHalf>,
    /// Every other key the entity carried, verbatim and in key order.
    pub extras: serde_json::Map<String, Value>,
}

/// A blueprint read WHOLE: everything the body carries, nothing refused for
/// being unplaceable and nothing dropped for being unmodelled.
///
/// See the module doc for why this is a second function rather than a relaxed
/// [`decode`]. In short: `decode`'s refusals are a promise about the placement
/// path, and this type answers a question placement never asks.
///
/// The `Option`s are load-bearing throughout. `wires: None` means the body
/// carried no `wires` key; `Some(vec![])` means it carried an empty one, which
/// is a different statement about the blueprint and would be indistinguishable
/// under a bare `Vec`.
#[derive(Debug, Clone, PartialEq)]
pub struct BlueprintDocument {
    pub entities: Vec<DocumentEntity>,
    /// Empty when the body carried no `tiles`; a blueprint that paints no
    /// ground and one that carries `"tiles":[]` are the same blueprint, so
    /// unlike `wires` this does not need an `Option` to stay honest -- and
    /// `encode_document` omits the key when it is empty, which is what
    /// Factorio itself writes.
    pub tiles: Vec<BlueprintTile>,
    /// `None` = no `wires` key. See the type doc.
    pub wires: Option<Vec<BlueprintWire>>,
    /// `None` when the body declared no `version`, rather than a fabricated 0.
    pub version: Option<u64>,
    /// The block's own grid, when it declares one. Same rules as
    /// [`Blueprint::grid`].
    pub grid: Option<BlueprintGrid>,
    /// Every body key this type does not model in its own right -- `item`,
    /// `label`, `icons`, `description`, `schedules`, and anything a future
    /// Factorio adds -- verbatim and in key order. **This is the field that
    /// makes the round trip meaningful**: nothing reaches it by being
    /// understood, only by being kept.
    pub extras: serde_json::Map<String, Value>,
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

/// The entity keys this module models in its own right, and so exactly the
/// keys the placement path understands. Anything else present on an entity
/// fails [`decode`] by name -- see the module doc for why this is an
/// allowlist rather than an enumerated blocklist -- and is kept verbatim in
/// [`DocumentEntity::extras`] by [`decode_document`]. One list, so "what we
/// model" and "what we will build" cannot drift apart.
const ALLOWED_ENTITY_KEYS: &[&str] = &["entity_number", "name", "position", "direction", "type"];

/// The only keys the **placement** path understands on the blueprint object
/// itself (the value of the envelope's `"blueprint"` key). Anything else --
/// `wires` (2.0 circuit connections moved out of per-entity `connections`),
/// `schedules` (train blueprints), or any future field -- fails [`decode`] by
/// name, for the same reason as `ALLOWED_ENTITY_KEYS`.
///
/// [`decode_document`] does not consult this: reading a blueprint and building
/// one are different questions, and this list answers the second.
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

/// The body keys [`BlueprintDocument`] models in its own right. Everything
/// else lands in `BlueprintDocument::extras` -- kept, never dropped.
/// The three grid keys are deliberately NOT here: they stay in `extras`
/// verbatim and [`BlueprintDocument::grid`] is *read off* them. Modelling them
/// would have meant deciding whether an `absolute-snapping` written as `false`
/// is the same blueprint as one that omits it -- a question the round trip
/// would then have had to answer, and one nobody needs answered. Keeping the
/// bytes sidesteps it.
const MODELLED_BODY_KEYS: &[&str] = &["entities", "tiles", "wires", "version"];

/// The keys a tile may carry. Unlike the entity and body cases there is no
/// "kept verbatim" half here: a tile is a name and a place, and a key beyond
/// those two would be a fact about the ground this decoder has no way to
/// represent, so it is refused by name.
const ALLOWED_TILE_KEYS: &[&str] = &["name", "position"];

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
    compress(&envelope.to_string())
}

/// The inverse of [`decode_document`].
///
/// Writes every key back: the modelled ones from their fields, the rest from
/// the `extras` maps, verbatim. **This is what makes the round trip evidence.**
/// If the JSON came back from `doc.extras` alone the check would only prove
/// `serde_json` can hold a map; because entities, tiles and wires are rebuilt
/// from their own structs, anything the decoder failed to keep is missing here
/// and the comparison fails.
///
/// One deliberate normalisation: an `entities` or `tiles` key present but
/// *empty* is written back as absent, because an empty array and no array say
/// the same thing about a blueprint. `wires` is not normalised that way --
/// `Some(vec![])` writes `"wires":[]` -- because a wire list is a claim about
/// circuit intent and its emptiness is worth preserving distinctly.
pub fn encode_document(doc: &BlueprintDocument) -> String {
    let mut body = doc.extras.clone();
    if let Some(v) = doc.version {
        body.insert("version".to_owned(), Value::from(v));
    }
    if !doc.entities.is_empty() {
        let entities: Vec<Value> = doc
            .entities
            .iter()
            .map(|e| {
                let mut obj = serde_json::Map::new();
                if let Some(n) = e.number {
                    obj.insert("entity_number".to_owned(), Value::from(n));
                }
                obj.insert("name".to_owned(), Value::from(e.name.clone()));
                obj.insert("position".to_owned(), position_json(&e.offset));
                if let Some(d) = e.raw_direction {
                    obj.insert("direction".to_owned(), Value::from(d));
                }
                if let Some(k) = &e.kind {
                    obj.insert("type".to_owned(), Value::from(k.clone()));
                }
                for (k, v) in &e.extras {
                    obj.insert(k.clone(), v.clone());
                }
                Value::Object(obj)
            })
            .collect();
        body.insert("entities".to_owned(), Value::Array(entities));
    }
    if !doc.tiles.is_empty() {
        let tiles: Vec<Value> = doc
            .tiles
            .iter()
            .map(|t| {
                let mut obj = serde_json::Map::new();
                obj.insert("name".to_owned(), Value::from(t.name.clone()));
                obj.insert("position".to_owned(), position_json(&t.offset));
                Value::Object(obj)
            })
            .collect();
        body.insert("tiles".to_owned(), Value::Array(tiles));
    }
    if let Some(wires) = &doc.wires {
        let wires: Vec<Value> = wires
            .iter()
            .map(|w| {
                Value::Array(vec![
                    Value::from(w.from_entity),
                    Value::from(w.from_connector),
                    Value::from(w.to_entity),
                    Value::from(w.to_connector),
                ])
            })
            .collect();
        body.insert("wires".to_owned(), Value::Array(wires));
    }
    let mut envelope = serde_json::Map::new();
    envelope.insert("blueprint".to_owned(), Value::Object(body));
    compress(&Value::Object(envelope).to_string())
}

/// A blueprint position as Factorio writes one.
///
/// **An integral coordinate is emitted as an integer, not as `-36.0`.** That is
/// what the game does -- there is not one `.0` literal anywhere in The Rook's
/// 1.7 MB of JSON -- and it is what makes a round trip comparable as
/// `serde_json::Value`, where `-36` and `-36.0` are different numbers.
fn position_json(p: &Position) -> Value {
    let num = |v: f64| -> Value {
        // 2^53: past it an f64 is not an exact integer anyway, so the
        // narrowing would be the lie rather than the float.
        if v.fract() == 0.0 && v.abs() <= 9_007_199_254_740_992.0 {
            Value::from(v as i64)
        } else {
            Value::from(v)
        }
    };
    let mut obj = serde_json::Map::new();
    obj.insert("x".to_owned(), num(p.x()));
    obj.insert("y".to_owned(), num(p.y()));
    Value::Object(obj)
}

/// Version byte, base64, zlib -- the envelope every blueprint string wears.
fn compress(json: &str) -> String {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(json.as_bytes())
        .expect("an in-memory zlib write cannot fail");
    let compressed = encoder
        .finish()
        .expect("an in-memory zlib finish cannot fail");
    format!(
        "0{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    )
}

/// Unwrap a blueprint string down to the body object, with the inflation cap
/// applied. Shared by [`decode`] and [`decode_document`] so the two can never
/// disagree about what a blueprint string even is.
fn body_of(text: &str) -> Result<serde_json::Map<String, Value>, BlueprintError> {
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
    match body_value {
        Value::Object(map) => Ok(map),
        _ => Err(BlueprintError::NoBlueprint),
    }
}

/// A `{"x":..,"y":..}` object, and nothing else.
///
/// A third key here is refused by name (`position.<key>`) rather than ignored:
/// a coordinate that carries something extra is a coordinate this decoder does
/// not understand.
fn position_of(v: &Value) -> Result<Position, BlueprintError> {
    let obj = v
        .as_object()
        .ok_or_else(|| BlueprintError::Unsupported("position".into()))?;
    let (mut x, mut y) = (None, None);
    for (key, val) in obj {
        match key.as_str() {
            "x" => x = val.as_f64(),
            "y" => y = val.as_f64(),
            other => return Err(BlueprintError::Unsupported(format!("position.{other}"))),
        }
    }
    match (x, y) {
        (Some(x), Some(y)) => Ok(Position::new(x, y)),
        _ => Err(BlueprintError::NotJson),
    }
}

/// Read a blueprint WHOLE -- see the module doc, and [`BlueprintDocument`].
///
/// Refuses only what it cannot represent at all (a malformed position, a wire
/// row that is not four numbers, a tile carrying a key beyond name and place).
/// Everything it does not model, it keeps.
pub fn decode_document(text: &str) -> Result<BlueprintDocument, BlueprintError> {
    let body = body_of(text)?;
    let mut modelled = serde_json::Map::new();
    let mut extras = serde_json::Map::new();
    for (key, value) in body {
        if MODELLED_BODY_KEYS.contains(&key.as_str()) {
            modelled.insert(key, value);
        } else {
            extras.insert(key, value);
        }
    }

    let version = match modelled.get("version") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| BlueprintError::Unsupported("version".into()))?,
        ),
    };

    let mut entities = Vec::new();
    if let Some(v) = modelled.get("entities") {
        let arr = v
            .as_array()
            .ok_or_else(|| BlueprintError::Unsupported("entities".into()))?;
        entities.reserve(arr.len());
        for raw_entity in arr {
            let obj = raw_entity
                .as_object()
                .ok_or_else(|| BlueprintError::Unsupported("entity is not an object".into()))?;
            let (mut number, mut name, mut offset, mut raw_direction, mut kind) =
                (None, None, None, None, None);
            let mut entity_extras = serde_json::Map::new();
            for (key, value) in obj {
                if !ALLOWED_ENTITY_KEYS.contains(&key.as_str()) {
                    entity_extras.insert(key.clone(), value.clone());
                    continue;
                }
                match key.as_str() {
                    "entity_number" => {
                        number = Some(value.as_u64().ok_or(BlueprintError::NotJson)?);
                    }
                    "name" => {
                        name = Some(value.as_str().ok_or(BlueprintError::NotJson)?.to_owned())
                    }
                    "position" => offset = Some(position_of(value)?),
                    "direction" => {
                        let d = value.as_u64().ok_or(BlueprintError::NotJson)?;
                        // Refused rather than truncated: `blueprint_direction`
                        // takes a `u8`, and casting 300 to 44 would be a silent
                        // rotation of the entity.
                        if d > u64::from(u8::MAX) {
                            return Err(BlueprintError::Unsupported("direction".into()));
                        }
                        raw_direction = Some(d);
                    }
                    "type" => {
                        kind = Some(value.as_str().ok_or(BlueprintError::NotJson)?.to_owned())
                    }
                    _ => unreachable!("ALLOWED_ENTITY_KEYS and this match are the same list"),
                }
            }
            let underground_half = match kind.as_deref() {
                Some("input") => Some(UndergroundHalf::Input),
                Some("output") => Some(UndergroundHalf::Output),
                _ => None,
            };
            entities.push(DocumentEntity {
                number,
                name: name.ok_or(BlueprintError::NotJson)?,
                offset: offset.ok_or(BlueprintError::NotJson)?,
                // The CANONICAL migration (`crates/core/src/types.rs`), not a
                // second copy of it. This module carried its own `VERSION_2_0`
                // and `migrate_direction` for one commit, and they had already
                // drifted: the canonical pair folds with `% 8` / `% 16` before
                // doubling, the copy used `saturating_mul(2)` and folded
                // nothing, so a blueprint carrying a direction >= 8 produced
                // 16..=254 -- a number outside `defines.direction` entirely,
                // handed straight to `create_entity`.
                direction: Direction::to_u8(&blueprint_direction(
                    raw_direction.unwrap_or(0) as u8,
                    version.unwrap_or(0),
                ))
                .expect("blueprint_direction folds into 0..=15"),
                raw_direction,
                kind,
                underground_half,
                extras: entity_extras,
            });
        }
    }

    let mut tiles = Vec::new();
    if let Some(v) = modelled.get("tiles") {
        let arr = v
            .as_array()
            .ok_or_else(|| BlueprintError::Unsupported("tiles".into()))?;
        tiles.reserve(arr.len());
        for raw_tile in arr {
            let obj = raw_tile
                .as_object()
                .ok_or_else(|| BlueprintError::Unsupported("tile is not an object".into()))?;
            let (mut name, mut offset) = (None, None);
            for (key, value) in obj {
                if !ALLOWED_TILE_KEYS.contains(&key.as_str()) {
                    return Err(BlueprintError::Unsupported(format!("tile.{key}")));
                }
                match key.as_str() {
                    "name" => {
                        name = Some(value.as_str().ok_or(BlueprintError::NotJson)?.to_owned())
                    }
                    "position" => offset = Some(position_of(value)?),
                    _ => unreachable!("ALLOWED_TILE_KEYS and this match are the same list"),
                }
            }
            tiles.push(BlueprintTile {
                name: name.ok_or(BlueprintError::NotJson)?,
                offset: offset.ok_or(BlueprintError::NotJson)?,
            });
        }
    }

    let wires = match modelled.get("wires") {
        None => None,
        Some(v) => {
            let arr = v
                .as_array()
                .ok_or_else(|| BlueprintError::Unsupported("wires".into()))?;
            let mut out = Vec::with_capacity(arr.len());
            for row in arr {
                let row = row
                    .as_array()
                    .filter(|r| r.len() == 4)
                    .ok_or_else(|| BlueprintError::Unsupported("wire".into()))?;
                let mut n = [0u64; 4];
                for (slot, value) in n.iter_mut().zip(row) {
                    *slot = value
                        .as_u64()
                        .ok_or_else(|| BlueprintError::Unsupported("wire".into()))?;
                }
                out.push(BlueprintWire {
                    from_entity: n[0],
                    from_connector: n[1],
                    to_entity: n[2],
                    to_connector: n[3],
                });
            }
            Some(out)
        }
    };

    // A grid exists only when the author gave a pitch. `absolute-snapping`
    // alone is meaningless -- it says how to interpret a pitch that is not
    // there -- so it does not conjure one.
    let grid = match extras.get("snap-to-grid") {
        None => None,
        Some(pitch) => Some(BlueprintGrid {
            pitch: position_of(pitch)?,
            absolute: extras
                .get("absolute-snapping")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            relative_position: match extras.get("position-relative-to-grid") {
                None => None,
                Some(p) => Some(position_of(p)?),
            },
        }),
    };

    Ok(BlueprintDocument {
        entities,
        tiles,
        wires,
        version,
        grid,
        extras,
    })
}

/// The placement view: a blueprint `Goal::Built` can actually stand up, or a
/// refusal naming the first key it cannot.
///
/// This is [`decode_document`] plus the placement policy, in that order, so
/// the allowlists below can never drift from what is really in the file. The
/// refusal order is body keys, then tiles, then entity keys -- unchanged from
/// when each check was a separate pass.
pub fn decode(text: &str) -> Result<Blueprint, BlueprintError> {
    let doc = decode_document(text)?;

    // A `BTreeSet` so the key reported is the first in key order, which is
    // what a single sorted pass over the body reported before.
    let mut unplaceable: std::collections::BTreeSet<&str> = doc
        .extras
        .keys()
        .map(String::as_str)
        .filter(|k| !ALLOWED_BODY_KEYS.contains(k))
        .collect();
    if doc.wires.is_some() {
        unplaceable.insert("wires");
    }
    if let Some(key) = unplaceable.first() {
        return Err(BlueprintError::Unsupported((*key).to_owned()));
    }
    if !doc.tiles.is_empty() {
        return Err(BlueprintError::Unsupported("tiles".into()));
    }

    let mut entities = Vec::with_capacity(doc.entities.len());
    for e in doc.entities {
        if let Some(key) = e.extras.keys().next() {
            return Err(BlueprintError::Unsupported(key.clone()));
        }
        entities.push(BlueprintEntity {
            name: e.name,
            offset: e.offset,
            direction: e.direction,
            underground_half: e.underground_half,
        });
    }
    Ok(Blueprint {
        entities,
        version: doc.version.unwrap_or(0),
        grid: doc.grid,
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
            r#"{"blueprint":{"item":"blueprint","version":562949953421312,
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
            r#"{"blueprint":{"item":"blueprint","version":562949953421312,
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
            r#"{"blueprint":{"item":"blueprint","version":562949953421312,
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
            r#"{"blueprint":{"item":"blueprint","version":562949953421312,
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

#[cfg(test)]
mod document_tests {
    use super::{
        BlueprintError, BlueprintWire, UndergroundHalf, decode, decode_document, encode_document,
    };

    fn wrap(json: &str) -> String {
        super::compress(&format!(r#"{{"blueprint":{json}}}"#))
    }

    /// **An empty wire list and no wire list are different facts**, and the
    /// distinction has to survive a round trip in both directions. A blueprint
    /// whose author wired nothing is not the same document as one from a
    /// Factorio version that had no wires at all -- collapsing the two is the
    /// conflation this module already refuses for `snap-to-grid`.
    #[test]
    fn an_empty_wire_list_is_not_the_absence_of_one() {
        let empty = decode_document(&wrap(
            r#"{"item":"blueprint","version":562949953421312,"wires":[]}"#,
        ))
        .expect("reads");
        let absent = decode_document(&wrap(r#"{"item":"blueprint","version":562949953421312}"#))
            .expect("reads");
        assert_eq!(empty.wires, Some(vec![]));
        assert_eq!(absent.wires, None);
        assert_ne!(empty, absent);
        // And the encoder keeps them apart rather than normalising both away.
        assert_ne!(encode_document(&empty), encode_document(&absent));
    }

    /// A wire is kept as the four numbers it is, both ends named by
    /// `entity_number`. The join is what makes wiring meaningful, so the
    /// numbers are kept as written rather than re-derived from array order.
    #[test]
    fn a_wire_names_both_ends_by_entity_number() {
        let doc = decode_document(&wrap(
            r#"{"item":"blueprint","version":562949953421312,
                "entities":[{"entity_number":7,"name":"arithmetic-combinator","position":{"x":0,"y":0}}],
                "wires":[[7,1,9,4]]}"#,
        ))
        .expect("reads");
        assert_eq!(doc.entities[0].number, Some(7));
        assert_eq!(
            doc.wires.as_deref(),
            Some(
                [BlueprintWire {
                    from_entity: 7,
                    from_connector: 1,
                    to_entity: 9,
                    to_connector: 4,
                }]
                .as_slice()
            )
        );
    }

    /// A tile is a name and a place. Both are modelled, so a tiles-only
    /// blueprint -- which is what a bare space platform foundation is -- reads
    /// as tiles rather than as an empty blueprint.
    #[test]
    fn tiles_are_read_rather_than_refused() {
        let doc = decode_document(&wrap(
            r#"{"item":"blueprint","version":562949953421312,
                "tiles":[{"name":"space-platform-foundation","position":{"x":-36,"y":-149}}]}"#,
        ))
        .expect("reads");
        assert_eq!(doc.entities.len(), 0);
        assert_eq!(doc.tiles.len(), 1);
        assert_eq!(doc.tiles[0].name, "space-platform-foundation");
        assert_eq!(
            (doc.tiles[0].offset.x(), doc.tiles[0].offset.y()),
            (-36.0, -149.0)
        );
        // ...and the placement path still refuses it, by name.
        assert_eq!(
            decode(&wrap(
                r#"{"item":"blueprint","version":562949953421312,
                    "tiles":[{"name":"space-platform-foundation","position":{"x":-36,"y":-149}}]}"#,
            ))
            .unwrap_err(),
            BlueprintError::Unsupported("tiles".into())
        );
    }

    /// A key beyond name and place on a tile is refused by name rather than
    /// dropped: unlike an entity, a tile has no `extras` to keep it in, so
    /// silence would be a real loss.
    #[test]
    fn an_unknown_tile_key_is_refused_by_name() {
        assert_eq!(
            decode_document(&wrap(
                r#"{"item":"blueprint","version":562949953421312,
                    "tiles":[{"name":"concrete","position":{"x":0,"y":0},"quality":"rare"}]}"#,
            ))
            .unwrap_err(),
            BlueprintError::Unsupported("tile.quality".into())
        );
    }

    /// **What `decode` refuses, `decode_document` keeps.** The same entity key
    /// that fails the placement path lands verbatim in `extras`, and the
    /// modelled fields beside it are read normally.
    #[test]
    fn an_unplaceable_entity_key_is_kept_rather_than_refused() {
        let json = r#"{"item":"blueprint","version":562949953421312,
            "entities":[{"entity_number":1,"name":"underground-belt","position":{"x":0.5,"y":0.5},
                         "direction":4,"type":"output","recipe":"iron-gear-wheel"}]}"#;
        assert_eq!(
            decode(&wrap(json)).unwrap_err(),
            BlueprintError::Unsupported("recipe".into()),
            "the placement path cannot set a recipe, so it must still refuse"
        );
        let doc = decode_document(&wrap(json)).expect("reading it is a different question");
        let e = &doc.entities[0];
        assert_eq!(e.name, "underground-belt");
        assert_eq!(e.direction, 4);
        assert_eq!(e.raw_direction, Some(4));
        assert_eq!(e.underground_half, Some(UndergroundHalf::Output));
        assert_eq!(
            e.extras.get("recipe").and_then(|v| v.as_str()),
            Some("iron-gear-wheel")
        );
    }

    /// An absent `direction` is not a written zero, and the encoder must not
    /// invent one -- 2,728 of The Rook's entities omit the key.
    #[test]
    fn an_absent_direction_is_not_a_written_zero() {
        let doc = decode_document(&wrap(
            r#"{"item":"blueprint","version":562949953421312,
                "entities":[{"entity_number":1,"name":"stone-furnace","position":{"x":0,"y":0}}]}"#,
        ))
        .expect("reads");
        assert_eq!(doc.entities[0].raw_direction, None);
        assert_eq!(doc.entities[0].direction, 0, "read as north all the same");
        assert!(
            !encode_document(&doc).is_empty() && {
                let again = decode_document(&encode_document(&doc)).expect("reads");
                again.entities[0].raw_direction.is_none()
            },
            "the encoder must not write a direction the author never wrote"
        );
    }

    /// The three grid keys stay in `extras` verbatim and `grid` is read off
    /// them, so a blueprint that declares a pitch survives a round trip
    /// without the encoder having to decide what an absent
    /// `absolute-snapping` means.
    #[test]
    fn a_declared_grid_is_read_and_also_kept_verbatim() {
        let doc = decode_document(&wrap(
            r#"{"item":"blueprint","version":562949953421312,
                "snap-to-grid":{"x":29,"y":11},"absolute-snapping":true,
                "entities":[{"entity_number":1,"name":"stone-furnace","position":{"x":0,"y":0}}]}"#,
        ))
        .expect("reads");
        let grid = doc.grid.clone().expect("declares a grid");
        assert_eq!((grid.pitch.x(), grid.pitch.y()), (29.0, 11.0));
        assert!(grid.absolute);
        assert!(doc.extras.contains_key("snap-to-grid"));
        assert_eq!(
            decode_document(&encode_document(&doc)).expect("reads"),
            doc,
            "a round trip is a fixed point"
        );
    }
}
