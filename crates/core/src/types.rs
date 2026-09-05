use std::collections::{BTreeMap, HashMap};
use std::ops::Sub;
use std::str::FromStr;
use std::sync::Arc;

use dashmap::DashMap;
use euclid::{Point2D, Size2D};
use factorio_blueprint::objects::Entity;
use noisy_float::prelude::*;
use num_traits::ToPrimitive;
use serde_json::Value;

use crate::errors::RectInvalid;
use crate::factorio::util::{add_to_rect, add_to_rect_turned, calculate_distance, rect_floor_ceil};
use crate::graph::entity_graph::QuadTreeRect;
use crate::num_traits::FromPrimitive;
use miette::{IntoDiagnostic, Result};
use mlua::prelude::*;

pub type FactorioInventory = HashMap<String, u32>;

/// Custom deserializer to handle Lua tables that serialize as {} instead of []
mod deserialize_helpers {
    use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
    use std::fmt;
    use std::marker::PhantomData;

    /// Deserializes a Vec<T> that may be represented as either [] or {} (empty map)
    /// This is needed because Lua's JSON serializer converts empty tables to {} instead of []
    pub fn vec_or_empty_map<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        struct VecOrEmptyMap<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for VecOrEmptyMap<T>
        where
            T: serde::Deserialize<'de>,
        {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence or empty map")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut vec = Vec::new();
                while let Some(elem) = seq.next_element()? {
                    vec.push(elem);
                }
                Ok(vec)
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                // Drain the map (should be empty for this to make sense)
                while map
                    .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(Vec::new())
            }
        }

        deserializer.deserialize_any(VecOrEmptyMap(PhantomData))
    }

    /// Deserializes a list of names that the mod may have written as a bare
    /// string (`"crude-oil"`), a filter table with a `name` (`{"name":
    /// "lab"}`), a list of either, `{}` (Lua's empty table) or `null`.
    ///
    /// Written for `ResearchTrigger`'s `entities`, whose runtime shape is
    /// documented as singular and shipped as a list -- see that type. The
    /// mod normalises to a list, so the other spellings are for dumps written
    /// by hand or by a mod version that forwarded the table untouched.
    pub fn names_one_or_many<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Names;

        fn name_of<E: de::Error>(value: serde_json::Value) -> Result<Option<String>, E> {
            match value {
                serde_json::Value::String(name) => Ok(Some(name)),
                serde_json::Value::Object(map) => match map.get("name") {
                    Some(serde_json::Value::String(name)) => Ok(Some(name.clone())),
                    Some(other) => Err(E::custom(format!(
                        "a name filter's `name` must be a string, got {other}"
                    ))),
                    None => Ok(None),
                },
                serde_json::Value::Null => Ok(None),
                other => Err(E::custom(format!("expected a name, got {other}"))),
            }
        }

        impl<'de> Visitor<'de> for Names {
            type Value = Vec<String>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a name, a {name} table, a list of either, or an empty table")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(vec![v.to_string()])
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Vec::new())
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Vec::new())
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut names = Vec::new();
                while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                    names.extend(name_of::<A::Error>(elem)?);
                }
                Ok(names)
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut fields = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    fields.insert(key, value);
                }
                Ok(name_of::<M::Error>(serde_json::Value::Object(fields))?
                    .into_iter()
                    .collect())
            }
        }

        deserializer.deserialize_any(Names)
    }

    // A custom `deserialize_with` opts a field out of serde's built-in
    // "missing key means None" handling for `Option<T>` fields, so every
    // caller of these two helpers must pair them with `#[serde(default)]` to
    // keep that behaviour for fields Lua omits entirely (e.g. a container
    // with no fuel inventory sends no `fuel_inventory` key at all).

    /// Deserializes an `Option<Vec<T>>` that may be represented as `[]`, `{}`
    /// (Lua's empty table, same as `vec_or_empty_map`), or `null`.
    pub fn option_vec_or_empty_map<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        struct OptionVecOrEmptyMap<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for OptionVecOrEmptyMap<T>
        where
            T: serde::Deserialize<'de>,
        {
            type Value = Option<Vec<T>>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence, an empty map, or null")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut vec = Vec::new();
                while let Some(elem) = seq.next_element()? {
                    vec.push(elem);
                }
                Ok(Some(vec))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                // Drain the map (should be empty for this to make sense)
                while map
                    .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(Some(Vec::new()))
            }
        }

        deserializer.deserialize_any(OptionVecOrEmptyMap(PhantomData))
    }

    /// The `Box<Option<Vec<T>>>`-shaped companion to `option_vec_or_empty_map`,
    /// for fields boxed to keep the enclosing struct small.
    pub fn boxed_option_vec_or_empty_map<'de, D, T>(
        deserializer: D,
    ) -> Result<Box<Option<Vec<T>>>, D::Error>
    where
        D: Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        option_vec_or_empty_map(deserializer).map(Box::new)
    }

    /// Deserializes an inventory into a name -> count map, accepting both shapes
    /// the game reports it in.
    ///
    /// `LuaInventory.get_contents()` returned a `name -> count` dictionary
    /// before Factorio 2.0 and returns an array of `{name, count, quality}`
    /// since. The mod passes the result through untouched (see
    /// `mods/BotBridge/types.lua`, `serialize_player`), so both arrive here.
    /// Quality is dropped and equal names are summed, because this field counts
    /// items by name.
    pub fn item_counts_map_or_seq<'de, D>(
        deserializer: D,
    ) -> Result<std::collections::BTreeMap<String, u32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use std::collections::BTreeMap;

        #[derive(serde::Deserialize)]
        struct ItemCount {
            name: String,
            count: u32,
        }

        struct MapOrSeq;

        impl<'de> Visitor<'de> for MapOrSeq {
            type Value = BTreeMap<String, u32>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a name -> count map or a sequence of {name, count}")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some((name, count)) = map.next_entry::<String, u32>()? {
                    *out.entry(name).or_insert(0) += count;
                }
                Ok(out)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some(item) = seq.next_element::<ItemCount>()? {
                    *out.entry(item.name).or_insert(0) += item.count;
                }
                Ok(out)
            }
        }

        deserializer.deserialize_any(MapOrSeq)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioRecipe {
    pub name: String,
    pub valid: bool,
    pub enabled: bool,
    pub category: String,
    pub ingredients: Option<Vec<FactorioIngredient>>,
    #[serde(deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub products: Vec<FactorioProduct>,
    pub hidden: bool,
    /// How long one craft takes, in seconds.
    // `R64` rather than `f64` so this type can derive `Hash`. It serialises as
    // a plain number, which is what `schemars(with)` tells the schema.
    #[schemars(with = "f64")]
    pub energy: Box<R64>,
    pub order: String,
    pub group: String,
    pub subgroup: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioBlueprintInfo {
    pub label: String,
    pub blueprint: String,
    pub width: u16,
    pub height: u16,
    pub rect: Rect,
    pub data: Value,
}

// Deserialised through `RawFactorioIngredient` because Factorio's
// `Ingredient.amount` is a `double` (see
// `workspace/factorio-api-docs/runtime-api.json`, concept `Ingredient`), which
// a bare `u32` field would reject as soon as the game reports `2.5` for a
// fluid.
/// One input of a recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq, JsonSchema)]
#[serde(rename_all = "snake_case", from = "RawFactorioIngredient")]
pub struct FactorioIngredient {
    pub name: String,
    #[serde(default)]
    pub ingredient_type: String,
    pub amount: u32,
}

// Deserialised through `RawFactorioProduct`, which normalises the shapes the
// different Factorio versions report. Everything downstream keeps reading a
// single expected `amount` and an effective `probability`.
/// One output of a recipe: how much of what, and how likely it is produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq, JsonSchema)]
#[serde(rename_all = "snake_case", from = "RawFactorioProduct")]
pub struct FactorioProduct {
    pub name: String,
    #[serde(default)]
    pub product_type: String,
    pub amount: u32,
    /// How likely this product is produced at all, from 0 to 1.
    ///
    /// # Parsed, and deliberately not read
    ///
    /// Nothing consumes this. That is a decision, not an oversight, and it has
    /// an expiry date — so here is the whole of it.
    ///
    /// A planner that ignores a probability under-plans by `1 / probability`:
    /// `crates/planner`'s `output_per_craft` sizes a craft as
    /// `runs = ceil(need / amount)` where the honest sum is
    /// `runs = ceil(need / (amount * probability))`. It does not divide, and
    /// that is safe for exactly one reason: **no recipe the planner can reach
    /// carries a probability below 1**. Reachability is decided by category,
    /// and the planner admits only `crafting` and `smelting`.
    ///
    /// In the live 2.1.17 capture (`crates/core/tests/live-2.1.17-world-snapshot.json`)
    /// 123 of 662 recipes' products are uncertain, and every one of them is
    /// `recycling` (102), `crushing` (15), `organic` (4) or `centrifuging` (2)
    /// — all disabled, none reachable. The nearest are the four `organic`
    /// ones, a single gate-widening away: `yumako-processing` and
    /// `jellynut-processing` at 0.02, `iron-bacteria` and `copper-bacteria` at
    /// 0.1, where the planner would be 10x short. `crates/planner/tests/recipe_probability.rs`
    /// asserts both halves of that — that the capture really does carry
    /// probabilities, and that none is reachable — so widening a gate breaks
    /// a test rather than quietly breaking a plan.
    ///
    /// Deleting the field instead was rejected: unlike the `durability` and
    /// `speed` pair removed from `FactorioItemPrototype`, this is not a wire
    /// value dropped at deserialisation. It is the *normalisation* of three
    /// version-specific keys the mod forwards on purpose (see
    /// `mods/BotBridge/types.lua::serialize_product` and `RawFactorioProduct`
    /// below): Factorio 2.1 replaced `probability` with
    /// `independent_probability` times the `shared_probability` window, and
    /// this field is the only place that knows it. Whoever widens the gate
    /// needs that arithmetic and should not have to rediscover it — and when
    /// they add the division, a probability of 0 must not divide: a product
    /// that never appears makes the recipe no route to the item at all.
    // `R64` rather than `f64` so this type can derive `Hash`. It serialises as
    // a plain number, which is what `schemars(with)` tells the schema.
    #[schemars(with = "f64")]
    pub probability: Box<R64>,
}

/// The `Ingredient` shape as it arrives from `mods/BotBridge`.
// `JsonSchema` is required by schemars 1.x, which -- unlike 0.8 -- honours
// `#[serde(from = "...")]` and so needs the *source* type described to build
// the deserialize contract. Nothing documents this type directly: `types.lua`
// is generated from the serialize contract, which is the shape Lua receives.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RawFactorioIngredient {
    pub name: String,
    #[serde(default)]
    pub ingredient_type: String,
    /// `double` in the runtime API, integral for every vanilla item recipe.
    pub amount: f64,
}

impl From<RawFactorioIngredient> for FactorioIngredient {
    fn from(raw: RawFactorioIngredient) -> Self {
        FactorioIngredient {
            name: raw.name,
            ingredient_type: raw.ingredient_type,
            amount: round_to_u32(raw.amount),
        }
    }
}

/// The `Product` shape as it arrives from `mods/BotBridge`, across game
/// versions.
///
/// Factorio 2.1's `ItemProduct`/`FluidProduct` (see
/// `workspace/factorio-api-docs/runtime-api.json`) have **no `probability`
/// field at all**: it was split into `independent_probability` (a `double`) and
/// `shared_probability` (a `{min, max}` window on a per-craft shared roll).
/// `amount` is optional there too — randomised outputs report `amount_min` /
/// `amount_max` instead. Factorio 2.0 and earlier sent `probability` and a
/// mandatory `amount`.
///
/// Every field is therefore optional, and missing probability information means
/// "produced with certainty" (1.0), which is what the game does when a product
/// declares none.
// `JsonSchema` is required by schemars 1.x, which -- unlike 0.8 -- honours
// `#[serde(from = "...")]` and so needs the *source* type described to build
// the deserialize contract. Nothing documents this type directly: `types.lua`
// is generated from the serialize contract, which is the shape Lua receives.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RawFactorioProduct {
    pub name: String,
    #[serde(default)]
    pub product_type: String,
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub amount_min: Option<f64>,
    #[serde(default)]
    pub amount_max: Option<f64>,
    /// Factorio <= 2.0 only; removed in 2.1.
    #[serde(default)]
    pub probability: Option<f64>,
    /// Factorio 2.1: chance for this product on its own roll.
    #[serde(default)]
    pub independent_probability: Option<f64>,
    /// Factorio 2.1: the window of the per-craft shared roll in which this
    /// product is given.
    #[serde(default)]
    pub shared_probability: Option<SharedProbabilityDefinition>,
}

/// `SharedProbabilityDefinition` from the runtime API: the product is given
/// when the craft's single shared roll falls into `[min, max]`.
// `JsonSchema` for the same reason as the Raw types above: reachable from a
// deserialize contract schemars 1.x now builds, never documented itself.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SharedProbabilityDefinition {
    pub min: f64,
    pub max: f64,
}

impl From<RawFactorioProduct> for FactorioProduct {
    fn from(raw: RawFactorioProduct) -> Self {
        let amount = raw
            .amount
            .or(match (raw.amount_min, raw.amount_max) {
                (Some(min), Some(max)) => Some((min + max) / 2.0),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            })
            .unwrap_or(0.0);
        // A missing shared window is the full range, i.e. no restriction.
        let shared_window = raw
            .shared_probability
            .map_or(1.0, |shared| (shared.max - shared.min).clamp(0.0, 1.0));
        let probability = raw
            .probability
            .or(raw
                .independent_probability
                .map(|independent| independent * shared_window))
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        FactorioProduct {
            name: raw.name,
            product_type: raw.product_type,
            amount: round_to_u32(amount),
            probability: Box::new(r64(probability)),
        }
    }
}

/// Rounds a wire amount to the whole units the rest of the codebase counts in.
/// Negative and non-finite values become 0 rather than wrapping.
fn round_to_u32(amount: f64) -> u32 {
    if !amount.is_finite() || amount <= 0.0 {
        return 0;
    }
    amount.round().to_u32().unwrap_or(u32::MAX)
}

pub type PlayerId = u8;
pub type ActionId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioPlayer {
    pub player_id: PlayerId,
    pub position: Position,
    #[serde(deserialize_with = "deserialize_helpers::item_counts_map_or_seq")]
    pub main_inventory: BTreeMap<String, u32>,
    // Widths follow `LuaControl` in Factorio's `runtime-api.json`: the first
    // three are `uint32`, the last three are `double`. Modelling the doubles as
    // integers made every real player unparseable, because a character's
    // `resource_reach_distance` is 2.7.
    pub build_distance: u32,     // uint32 — for place_entity
    pub reach_distance: u32,     // uint32 — for insert_to_inventory
    pub drop_item_distance: u32, // uint32 — remove_from_inventory
    /// `double`. Not in use; for picking up items from the ground.
    pub item_pickup_distance: f64,
    /// `double`. Not in use; for picking up items from the ground automatically.
    pub loot_pickup_distance: f64,
    /// `double`, for mine. Documented as "the resource reach distance of this
    /// character **or max double** when not a character or player connected to
    /// a character", so this is out of integer range as well as fractional.
    pub resource_reach_distance: f64,
}

impl Default for FactorioPlayer {
    fn default() -> Self {
        FactorioPlayer {
            player_id: 0,
            position: Position::default(),
            main_inventory: BTreeMap::new(),
            build_distance: 10,
            reach_distance: 10,
            drop_item_distance: 10,
            item_pickup_distance: 1.0,
            loot_pickup_distance: 2.0,
            resource_reach_distance: 3.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestEntity {
    pub name: String,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct InventoryResponse {
    pub name: String,
    pub position: Position,
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::boxed_option_vec_or_empty_map"
    )]
    pub output_inventory: Box<Option<Vec<InventoryItemWithQuality>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::boxed_option_vec_or_empty_map"
    )]
    pub fuel_inventory: Box<Option<Vec<InventoryItemWithQuality>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ChunkPosition {
    pub x: i32,
    pub y: i32,
}

impl From<&Pos> for ChunkPosition {
    fn from(pos: &Pos) -> ChunkPosition {
        ChunkPosition {
            x: pos.0 / 32,
            y: pos.1 / 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    /// **Manhattan** distance, `|dx| + |dy|` — not Euclidean.
    ///
    /// Named explicitly because the old name `distance` was misread twice in
    /// one day: it made a power pole invisible at 196 tiles (`f5af71bc`) and
    /// it made every charted nest read as further away than it is.
    pub fn manhattan_distance(&self, other: &Position) -> f64 {
        self.x.sub(other.x).abs() + self.y.sub(other.y).abs()
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "[{}, {}]",
            (self.x() * 100.).round() / 100.,
            (self.y() * 100.).round() / 100.
        ))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Pos(pub i32, pub i32);

impl Pos {
    /// **Manhattan** distance, `|dx| + |dy|` — not Euclidean. See
    /// [`Position::manhattan_distance`].
    pub fn manhattan_distance(&self, other: &Pos) -> u32 {
        self.0.abs_diff(other.0) + self.1.abs_diff(other.1)
    }
}

impl From<Point2D<f32, Rect>> for Position {
    fn from(point: Point2D<f32, Rect>) -> Position {
        Position::new(point.x as f64, point.y as f64)
    }
}

impl From<Position> for Point2D<f32, Rect> {
    fn from(pos: Position) -> Point2D<f32, Rect> {
        Point2D::new(pos.x() as f32, pos.y() as f32)
    }
}

impl From<&Position> for Pos {
    fn from(position: &Position) -> Pos {
        Pos(position.x().floor() as i32, position.y().floor() as i32)
    }
}

impl From<&Pos> for Position {
    fn from(pos: &Pos) -> Position {
        Position::new(pos.0 as f64, pos.1 as f64)
    }
}

#[derive(Primitive, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
/// Factorio 2.x's `defines.direction`, all sixteen values.
///
/// Names and discriminants are taken verbatim from the shipped API definition
/// (`workspace/factorio-api-docs/runtime-api.json`, `application_version`
/// 2.1.17, `defines.direction`). Before this widening the enum carried Factorio
/// **1.x**'s eight-value scale, so `East` was `2` -- which 2.x reads as
/// *northeast* -- and the game's `east` (`4`) was read back as `South`. North
/// was the only value that survived the round trip.
///
/// The odd values are the half-diagonals. Factorio 2.x reports them for rails
/// and other 16-way entities, so they must be readable; most placement and
/// pathing helpers (`move_position`, `move_pos`, `Position::turn`) have no tile
/// offset for them and return `None`.
pub enum Direction {
    #[default]
    North = 0,
    NorthNorthEast = 1,
    NorthEast = 2,
    EastNorthEast = 3,
    East = 4,
    EastSouthEast = 5,
    SouthEast = 6,
    SouthSouthEast = 7,
    South = 8,
    SouthSouthWest = 9,
    SouthWest = 10,
    WestSouthWest = 11,
    West = 12,
    WestNorthWest = 13,
    NorthWest = 14,
    NorthNorthWest = 15,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AreaFilter {
    Rect(Rect),
    PositionRadius((Position, Option<f64>)),
}

/// The first blueprint `version` written by Factorio 2.0.
///
/// A blueprint's `version` packs `major` into the top 16 bits, so 2.0.0.0 is
/// `2 << 48`.
pub const BLUEPRINT_VERSION_2_0: u64 = 2 << 48;

/// Read a blueprint entity's `direction` on the scale of the Factorio version
/// that wrote the blueprint.
///
/// Blueprint strings are a persisted format, and the eight-value scale did not
/// vanish when 2.0 widened `defines.direction` -- a blueprint exported from 1.1
/// still says `east = 2`, and the crates that parse them still hand that number
/// over unchanged. Reading a 1.x blueprint on the 2.x scale turns every belt in
/// it 45 degrees; reading a 2.x one on the 1.x scale wraps `west` round to
/// `south`. The version field is the only thing that distinguishes them, so it
/// is what this reads.
///
/// Never `None`: a 1.x value is folded into `0..=7` before doubling, and a 2.x
/// value into `0..=15`.
pub fn blueprint_direction(direction: u8, blueprint_version: u64) -> Direction {
    let value = if blueprint_version >= BLUEPRINT_VERSION_2_0 {
        direction % 16
    } else {
        // 1.x's eight values sit on 2.x's even ones.
        (direction % 8) * 2
    };
    Direction::from_u8(value).expect("folded into 0..=15")
}

impl Direction {
    /// All sixteen values of `defines.direction`, half-diagonals included.
    ///
    /// This returned **eight** before the 2.x widening. Callers that wanted the
    /// eight compass points want [`Direction::compass`].
    pub fn all() -> Vec<Direction> {
        (0..16).map(|n| Direction::from_u8(n).unwrap()).collect()
    }
    /// The eight compass points -- the even values. This is what [`all`] used
    /// to return, and what any 8-connected neighbourhood wants.
    ///
    /// [`all`]: Direction::all
    pub fn compass() -> Vec<Direction> {
        (0..16)
            .step_by(2)
            .map(|n| Direction::from_u8(n as u8).unwrap())
            .collect()
    }
    /// The four cardinals: north, east, south, west. On the 2.x scale those are
    /// 0, 4, 8 and 12, so the step is four, not two.
    pub fn orthogonal() -> Vec<Direction> {
        (0..16u8)
            .filter(|n| n.is_multiple_of(4))
            .map(|n| Direction::from_u8(n).unwrap())
            .collect()
    }
    /// True for the eight compass points, false for the eight half-diagonals.
    pub fn is_compass(&self) -> bool {
        Direction::to_u8(self).unwrap().is_multiple_of(2)
    }
    /// 180 degrees. Half of sixteen is **eight**, not four.
    pub fn opposite(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 8) % 16).unwrap()
    }
    /// 90 degrees clockwise. A quarter of sixteen is **four**, not two.
    ///
    /// The semantics are deliberately unchanged: `flow_graph` compares belt
    /// orientations against `clockwise()`, and turning this into a 45-degree
    /// step would make every one of those comparisons false.
    pub fn clockwise(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 4) % 16).unwrap()
    }
}

impl Position {
    pub fn new(x: f64, y: f64) -> Position {
        Position { x, y }
    }

    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn add(&self, position: &Position) -> Position {
        Position::new(self.x() + position.x(), self.y() + position.y())
    }

    /// Rotate by a cardinal direction, or `None` if there is no such rotation.
    ///
    /// Only the four cardinals name a rotation this can perform. Before the 2.x
    /// widening the unhandled arm was `panic!("diagonal turning not
    /// supported")` and covered four values; it now covers twelve, and this
    /// crate builds `panic = "abort"`, so it returns `None` instead -- the same
    /// posture as the direction handling in 64decdcd.
    pub fn turn(&self, direction: Direction) -> Option<Position> {
        match direction {
            Direction::North => Some(self.clone()),
            Direction::East => Some(
                self.rotate_clockwise()
                    .rotate_clockwise()
                    .rotate_clockwise(),
            ),
            Direction::South => Some(self.rotate_clockwise().rotate_clockwise()),
            Direction::West => Some(self.rotate_clockwise()),
            _ => None,
        }
    }

    /*
    https://limnu.com/sketch-easy-90-degree-rotate-vectors/#:~:text=Normally%20rotating%20vectors%20involves%20matrix,swap%20X%20and%20Y%20values.
    Normally rotating vectors involves matrix math, but there’s a really simple trick for rotating a 2D vector by 90° clockwise:
    just multiply the X part of the vector by -1, and then swap X and Y values.
     */
    pub fn rotate_clockwise(&self) -> Position {
        Position::new(self.y(), -self.x())
    }
}

impl Default for Position {
    fn default() -> Self {
        Position::new(0., 0.)
    }
}

impl FromStr for Position {
    type Err = miette::Report;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = str.split(',').collect();
        if parts.len() != 2 {
            return Err(RectInvalid {
                invalid_input: str.into(),
            }
            .into());
        }
        Ok(Position::new(
            parts[0].parse().into_diagnostic()?,
            parts[1].parse().into_diagnostic()?,
        ))
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct Rect {
    pub left_top: Position,
    pub right_bottom: Position,
}

impl Rect {
    pub fn new(left_top: &Position, right_bottom: &Position) -> Rect {
        Rect {
            left_top: left_top.clone(),
            right_bottom: right_bottom.clone(),
        }
    }
    pub fn contains(&self, position: &Position) -> bool {
        position.x() > self.left_top.x()
            && position.x() < self.right_bottom.x()
            && position.y() > self.left_top.y()
            && position.y() < self.right_bottom.y()
    }

    pub fn from_wh(width: f64, height: f64) -> Rect {
        Rect {
            left_top: Position::new(-width / 2., -height / 2.),
            right_bottom: Position::new(width / 2., height / 2.),
        }
    }
    pub fn width(&self) -> f64 {
        self.right_bottom.x() - self.left_top.x()
    }
    pub fn height(&self) -> f64 {
        self.right_bottom.y() - self.left_top.y()
    }
    pub fn center(&self) -> Position {
        Position::new(
            (self.left_top.x() + self.right_bottom.x()) / 2.,
            (self.left_top.y() + self.right_bottom.y()) / 2.,
        )
    }
    pub fn rotate_clockwise(&self) -> Rect {
        Rect::from_wh(self.height(), self.width())
    }
}

#[allow(clippy::from_over_into)]
impl Into<QuadTreeRect> for Rect {
    fn into(self) -> QuadTreeRect {
        QuadTreeRect::new(
            self.left_top.clone().into(),
            Size2D::new(self.width() as f32, self.height() as f32),
        )
    }
}

impl FromStr for Rect {
    type Err = miette::Report;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = str.split(';').collect();
        if parts.len() != 2 {
            return Err(RectInvalid {
                invalid_input: str.into(),
            }
            .into());
        }
        Ok(Rect {
            left_top: parts[0].parse()?,
            right_bottom: parts[1].parse()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTile {
    pub name: String,
    pub player_collidable: bool,
    pub position: Position,
    pub color: Option<[u8; 4]>,
}

impl FactorioTile {
    /// Every tile name vanilla Factorio gives water, and there are **two**.
    ///
    /// Read off a live 2.1.17 capture rather than assumed:
    /// `crates/core/tests/live-2.1.17-tiles.json` holds 48 `water`, 14
    /// `deepwater` and 2 `grass-1`, and the first two are the only ones that
    /// come back `player_collidable`. A shoreline search that asks only for
    /// `"water"` -- as
    /// `FactorioRcon::find_offshore_pump_placement_options` does -- misses
    /// every edge of every lake deep enough to have a middle, which on a real
    /// map is most of them: the archived stdout in `workspace/*-log.txt` holds
    /// 330,346 `deepwater` tiles against 79,717 `water`.
    ///
    /// Sorted, because a caller may reasonably iterate it and the order it is
    /// written in should not be a source of one.
    pub const WATER_NAMES: [&'static str; 2] = ["deepwater", "water"];

    /// Whether this tile is water, by name.
    ///
    /// By name and not by `player_collidable`: cliffs and `out-of-map` are
    /// collidable too, and an offshore pump may stand in the first and never
    /// in the others. "Blocked" and "water" are different questions and
    /// conflating them is what
    /// [`EntityGraph::blocking_boxes_within`](crate::graph::entity_graph::EntityGraph::blocking_boxes_within)
    /// forces on a caller today.
    #[must_use]
    pub fn is_water(&self) -> bool {
        Self::WATER_NAMES.contains(&self.name.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FactorioChunk {
    pub entities: Vec<FactorioEntity>,
    // pub tiles: Vec<FactorioTile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkObject {
    pub name: String,
    pub position: Position,
    pub direction: String,
    pub bounding_box: Rect,
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::boxed_option_vec_or_empty_map"
    )]
    pub output_inventory: Box<Option<Vec<InventoryItemWithQuality>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::boxed_option_vec_or_empty_map"
    )]
    pub fuel_inventory: Box<Option<Vec<InventoryItemWithQuality>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkResource {
    pub name: String,
    pub position: Position,
}

/// How a Factorio 2.0 `research_trigger` technology is unlocked.
///
/// These technologies do not consume science packs at all: they complete when
/// the player *does* something. `research_unit_ingredients` is empty and
/// `research_unit_energy` is zero for every one of them, so a planner that only
/// reads the pack fields costs them at nothing — which is correct about the
/// research itself and silently wrong about the work that triggers it. Live
/// 2.1.17 has 32 such technologies, including `electronics`, `steam-power`,
/// `automation-science-pack` and `steel-axe`.
///
/// Source: `LuaTechnologyPrototype::research_trigger` in the shipped
/// `runtime-api.json` (`application_version` 2.1.17, `api_version` 6), whose
/// `ResearchTrigger` concept is a table tagged by `type` with eight variants.
/// The trigger lives on the **prototype**, not on `LuaTechnology` — the same
/// split that already forced `effects` to be read through `.prototype`.
///
/// # What each variant carries, and why every payload field is `default`
///
/// The mod (`mods/BotBridge/types.lua`, `serialize_technology`) sends one
/// normalised shape per kind. Until 2026-09-05 only `craft-item` had a
/// payload, because the two shipped schemas disagree about `mine-entity` --
/// `runtime-api.json` documents a singular `entity :: string`, the shipped
/// prototype data (`data/base/prototypes/technology.lua`) writes `entities =
/// {...}`, a list -- and the mod refused to guess. It now reads both spellings
/// and always sends `entities`, a list, so the disagreement is settled on the
/// Lua side where the live table is.
///
/// Every payload field defaults, so a dump or a snapshot written when the mod
/// sent the bare type still loads: `{"type": "mine-entity"}` becomes
/// `MineEntity { entities: [], count: 1 }`. An **empty** list is therefore a
/// distinct state -- "the mod that wrote this could not describe the trigger"
/// -- and the planner names it as such (`UndescribedResearchTrigger`) rather
/// than treating it as either free or unsupported.
///
/// Read off the prototype data, the shipped 2.1.17 shapes are: `craft-item`
/// `{item, count?}`, `mine-entity` `{entities = {...}}`, `send-item-to-orbit`
/// `{item}`, `capture-spawner` `{}`, `create-space-platform` `{}`. No shipped
/// technology uses `craft-fluid` or `build-entity`; their fields follow the
/// runtime API definition (`fluid`/`amount`, `entity`/`count`).
///
/// Carrying the kind is still the whole point: it is what lets a planner
/// distinguish "this research is genuinely free" from "this research has a cost
/// I cannot see", and refuse loudly instead of costing the second at zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ResearchTrigger {
    /// Craft `count` of `item`. The variant the early tree is built from.
    CraftItem {
        item: String,
        /// Absent in the prototype data for a trigger that wants a single item
        /// (`automation-science-pack` is `{type = "craft-item", item = "lab"}`
        /// with no count), so an absent key means one, not none. Costing it at
        /// zero would reintroduce the very defect this type exists to fix.
        #[serde(default = "one_item")]
        count: u32,
    },
    /// Craft (in a machine) `amount` of `fluid`. Unused by shipped 2.1.17.
    CraftFluid {
        #[serde(default)]
        fluid: Option<String>,
        /// `R64` rather than `f64` so this type keeps deriving `Hash`, like
        /// `FactorioRecipe::energy`.
        #[serde(default = "one_amount")]
        amount: Box<R64>,
    },
    /// Mine any one of `entities` -- with a hand, a drill or a pumpjack;
    /// the game does not care which. `oil-processing` is `["crude-oil"]`,
    /// `uranium-processing` is `["uranium-ore"]`, and Space Age lists up to
    /// four rocks for one trigger.
    MineEntity {
        #[serde(default, deserialize_with = "deserialize_helpers::names_one_or_many")]
        entities: Vec<String>,
        #[serde(default = "one_item")]
        count: u32,
    },
    /// Build any one of `entities`. Unused by shipped 2.1.17.
    BuildEntity {
        #[serde(default, deserialize_with = "deserialize_helpers::names_one_or_many")]
        entities: Vec<String>,
        #[serde(default = "one_item")]
        count: u32,
    },
    SendItemToOrbit {
        #[serde(default)]
        item: Option<String>,
    },
    /// `entity` is optional in the runtime definition, and the shipped
    /// `captivity` trigger names none: any spawner.
    CaptureSpawner {
        #[serde(default)]
        entity: Option<String>,
    },
    CreateSpacePlatform,
    Scripted,
    /// A `type` this build does not know — a newer Factorio or a mod. Kept as a
    /// variant rather than a deserialisation failure so that one unrecognised
    /// trigger cannot make a whole world unreadable; it still reaches the
    /// planner as "trigger-based, inexpressible", which is the safe answer.
    #[serde(other)]
    Unknown,
}

fn one_item() -> u32 {
    1
}

fn one_amount() -> Box<R64> {
    Box::new(r64(1.0))
}

impl ResearchTrigger {
    /// The `type` string this trigger came from, for diagnostics that have to
    /// name it back to a caller.
    pub fn kind(&self) -> &'static str {
        match self {
            ResearchTrigger::CraftItem { .. } => "craft-item",
            ResearchTrigger::CraftFluid { .. } => "craft-fluid",
            ResearchTrigger::MineEntity { .. } => "mine-entity",
            ResearchTrigger::BuildEntity { .. } => "build-entity",
            ResearchTrigger::SendItemToOrbit { .. } => "send-item-to-orbit",
            ResearchTrigger::CaptureSpawner { .. } => "capture-spawner",
            ResearchTrigger::CreateSpacePlatform => "create-space-platform",
            ResearchTrigger::Scripted => "scripted",
            ResearchTrigger::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ResearchTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResearchTrigger::CraftItem { item, count } => {
                write!(f, "craft {} {}", count, item)
            }
            ResearchTrigger::CraftFluid {
                fluid: Some(fluid),
                amount,
            } => write!(f, "craft {} {}", amount.raw(), fluid),
            ResearchTrigger::MineEntity { entities, count } if !entities.is_empty() => {
                write!(f, "mine {} {}", count, entities.join(" or "))
            }
            ResearchTrigger::BuildEntity { entities, count } if !entities.is_empty() => {
                write!(f, "build {} {}", count, entities.join(" or "))
            }
            ResearchTrigger::SendItemToOrbit { item: Some(item) } => {
                write!(f, "send {} to orbit", item)
            }
            ResearchTrigger::CaptureSpawner {
                entity: Some(entity),
            } => write!(f, "capture a {}", entity),
            other => write!(f, "{}", other.kind()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTechnology {
    pub name: String,
    pub enabled: bool,
    pub upgrade: bool,
    pub researched: bool,
    pub prerequisites: Option<Vec<String>>,
    #[serde(deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub research_unit_ingredients: Vec<FactorioIngredient>,
    pub research_unit_count: u64,
    pub research_unit_energy: Box<R64>,
    pub order: String,
    pub level: u32,
    pub valid: bool,
    /// Recipes this technology unlocks, from the prototype's `unlock-recipe`
    /// effects.
    ///
    /// The planner needs this to plan *through* a recipe that is disabled
    /// today: `collect_recipes` sends every recipe with its `enabled` flag, and
    /// this is the other half — which technology turns a disabled one on.
    /// Without it a locked recipe would be visible but unreachable, and the
    /// planner would emit plans that can never execute.
    ///
    /// `#[serde(default)]` because payloads captured before this field existed
    /// (and fixtures written against them) simply have no key here; an absent
    /// key means "we do not know of any", which is the same shape as an empty
    /// list and is what the old enabled-only world implicitly assumed.
    #[serde(default, deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub unlocked_recipes: Vec<String>,
    /// What the player has to *do* to unlock this, for the Factorio 2.0
    /// technologies that are not researched with science packs at all.
    ///
    /// `None` is the ordinary pack-researched technology, and is what every
    /// payload captured before this field existed deserialises to — hence
    /// `#[serde(default)]`. `Some(_)` means the pack fields are meaningless
    /// here and the real cost is whatever the trigger names.
    #[serde(default)]
    pub research_trigger: Option<ResearchTrigger>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioForce {
    pub name: String,
    pub force_id: u32,
    // The current technology in research, or None if no research is currently ongoing.
    pub current_research: Option<String>,
    // Progress of current research, as a number in range [0, 1].
    pub research_progress: Option<Box<R64>>,
    /// `LuaForce::manual_mining_speed_modifier`: "the actual mining speed will
    /// be multiplied by `1 + manual_mining_speed_modifier`". Default `0`.
    ///
    /// This is per-force and moves with research — vanilla's `steel-axe`
    /// grants `character-mining-speed +1`, doubling hand mining — so it cannot
    /// be a constant on the planner's side; see
    /// `factorio_bot_planner::method::util::character_mining_speed`.
    ///
    /// `R64` rather than `f64` because `FactorioForce` derives `Hash` and
    /// `Eq`, which is the same reason `research_progress` above is one.
    /// `Option` with `#[serde(default)]` so that payloads captured before this
    /// field existed still parse: `None` means "the world did not report one",
    /// which the planner reads as the documented default of `0`.
    #[serde(default)]
    pub manual_mining_speed_modifier: Option<Box<R64>>,
    pub technologies: Box<BTreeMap<String, FactorioTechnology>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioGraphic {
    pub entity_name: String,
    pub image_path: String,
    pub width: u32,
    pub height: u32, // FIXME: add whatever this is, width&height are the first
                     // 1:1:0:0:0:0:1

                     //picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..yy..":"..scale
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioFluidBoxPrototype {
    pub pipe_connections: Box<Option<Vec<FactorioFluidBoxConnection>>>,
    pub production_type: String,
}

// #[derive(
//     EnumString,
//     Display,
//     Debug,
//     Clone,
//     PartialEq,
//     Serialize,
//     Deserialize,
//     Hash,
//     Eq,
// )]
// #[strum(serialize_all = "kebab-case")]
// #[serde(rename_all = "kebab-case")]
// pub enum FactorioFluidBoxConnectionType {
//     Input,
//     Output,
//     InputOutput,
// }
// #[derive(
//     EnumString,
//     Display,
//     Debug,
//     Clone,
//     PartialEq,
//     Serialize,
//     Deserialize,
//     Hash,
//     Eq,
// )]
// #[strum(serialize_all = "kebab-case")]
// #[serde(rename_all = "kebab-case")]
// pub enum FactorioFluidBoxProductionType {
//     Input,
//     Output,
//     InputOutput,
//     None,
// }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioFluidBoxConnection {
    pub max_underground_distance: Option<u32>,
    pub connection_type: Option<String>,
    pub positions: Vec<Position>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioEntityPrototype {
    pub name: String,
    pub entity_type: String,
    pub collision_mask: Option<Vec<String>>,
    pub collision_box: Rect,
    pub mine_result: Option<BTreeMap<String, u32>>,
    pub mining_time: Option<f64>,
    pub mining_speed: Option<f64>,
    pub crafting_speed: Option<f64>,
    pub max_underground_distance: Option<u8>,
    pub fluidbox_prototypes: Option<Vec<FactorioFluidBoxPrototype>>,
    // The three fields below decide whether a *character* can mine a resource
    // by hand, which `mine_result` alone cannot: crude oil is `minable` with a
    // product of ten crude oil, exactly like iron ore is `minable` with a
    // product of one iron ore, and the mod's `products_to_dict` flattens the
    // product's `type` away. `minable` is the flag a pumpjack uses. Verified
    // live on 2026-09-04: `character.mine_entity(crude-oil)` returns false and
    // leaves the well untouched.
    //
    // Every one is `default`, so a dump or a snapshot written before these
    // existed still loads with `None`, and `hand_mining_obstacle` says what it
    // can and cannot conclude from that.
    /// A `resource` prototype's category -- `basic-solid`, `basic-fluid`,
    /// `hard-solid` -- named `category` at data stage and `resource_category`
    /// at runtime. This is the discriminator the game itself uses: a character
    /// or a drill mines a resource iff the category is in its own
    /// `resource_categories`.
    #[serde(default)]
    pub resource_category: Option<String>,
    /// The categories a `character` or `mining-drill` prototype can mine.
    /// Sorted by the mod so the order is the data's, not `pairs()`'s.
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub resource_categories: Option<Vec<String>>,
    /// `mineable_properties.required_fluid`: the fluid that must be piped in
    /// to mine this at all. Uranium ore needs sulfuric acid, and a character
    /// has no pipe.
    #[serde(default)]
    pub mining_fluid: Option<String>,
}

/// The resource categories a vanilla character mines, used when the world's
/// prototype table has no `character` entry or one captured before
/// [`FactorioEntityPrototype::resource_categories`] existed.
///
/// The data-stage default of `CharacterPrototype::mining_categories`, and the
/// value the base game ships. Not a guess about what is *reachable*: this is
/// the game's own answer, and a modded character that mines more will say so
/// in its own prototype.
pub const VANILLA_CHARACTER_RESOURCE_CATEGORIES: [&str; 1] = ["basic-solid"];

/// Why a character cannot mine a resource by hand.
///
/// Every variant is a fact read off prototypes, never off the map, so the
/// answer does not change with charting and never needs a bot to be anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HandMiningObstacle {
    /// The resource's category is not one the character mines.
    Category {
        resource_category: String,
        character_categories: Vec<String>,
    },
    /// Mining it needs a fluid piped in, and a character has no pipe.
    NeedsFluid { fluid: String },
    /// What it yields is not an item, so no inventory can receive it. This is
    /// the reading of a prototype captured *before* `resource_category` was
    /// serialized -- crude oil's product is a fluid, and a fluid is not in the
    /// item table.
    YieldsNoItem { product: String },
}

impl std::fmt::Display for HandMiningObstacle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandMiningObstacle::Category {
                resource_category,
                character_categories,
            } => write!(
                f,
                "its resource category is {resource_category}, and a character mines only {}",
                character_categories.join(", ")
            ),
            HandMiningObstacle::NeedsFluid { fluid } => {
                write!(
                    f,
                    "mining it needs {fluid} piped in, and a character has no pipe"
                )
            }
            HandMiningObstacle::YieldsNoItem { product } => write!(
                f,
                "it yields {product}, which is not an item, so no inventory can receive it"
            ),
        }
    }
}

impl FactorioEntityPrototype {
    /// Why a character could not mine this prototype by hand, or `None` when
    /// nothing in the prototype says it cannot.
    ///
    /// `character_categories` is the character prototype's
    /// `resource_categories`, falling back to
    /// [`VANILLA_CHARACTER_RESOURCE_CATEGORIES`]; `is_item` answers whether a
    /// name is in the world's item table, or `true` when the table is empty
    /// and cannot say.
    ///
    /// Three checks, each independent of the others, in the order a reader
    /// can act on them:
    ///
    /// 1. `mining_fluid` is set -- the resource needs a fluid piped in, whoever
    ///    is mining it.
    /// 2. `resource_category` is known and not one the character mines. This
    ///    is the game's own rule, and it is the one that catches crude oil
    ///    (`basic-fluid`) and Space Age's tungsten ore (`hard-solid`).
    /// 3. A product of mining is not an item. This is what an *old* capture
    ///    can still say: `mine_result` for crude oil reads `{crude-oil: 10}`
    ///    and there is no crude-oil item, so the well still refuses even when
    ///    field 2 is `None`. Checked whether or not the category is known,
    ///    because a category the character mines with a product nobody can
    ///    hold is a contradiction worth refusing rather than trusting.
    ///
    /// `None` for a prototype whose `entity_type` is not `resource`: trees
    /// and rocks are mined by a different path and never asked.
    pub fn hand_mining_obstacle(
        &self,
        character_categories: &[String],
        is_item: &dyn Fn(&str) -> bool,
    ) -> Option<HandMiningObstacle> {
        if self.entity_type != "resource" {
            return None;
        }
        if let Some(fluid) = &self.mining_fluid {
            return Some(HandMiningObstacle::NeedsFluid {
                fluid: fluid.clone(),
            });
        }
        if let Some(category) = &self.resource_category
            && !character_categories.iter().any(|c| c == category)
        {
            return Some(HandMiningObstacle::Category {
                resource_category: category.clone(),
                character_categories: character_categories.to_vec(),
            });
        }
        // `mine_result` is a `BTreeMap`, so the first non-item is the same one
        // on every call.
        if let Some(products) = &self.mine_result
            && let Some((product, _)) = products.iter().find(|(name, _)| !is_item(name))
        {
            return Some(HandMiningObstacle::YieldsNoItem {
                product: product.clone(),
            });
        }
        None
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioEntity {
    pub name: String,
    pub entity_type: String,
    pub position: Position,
    pub bounding_box: Rect,
    // Deliberately a `schemars(description)` and not a `///`: `utoipa::ToSchema`
    // reads doc comments too, and this type's OpenAPI schema is snapshotted in
    // `app/src/api/openapi.snapshot.json`. Saying it here reaches the Lua docs
    // without moving the published API surface.
    #[schemars(
        description = "Factorio 2.x `defines.direction`: 0..=15, north at 0 and increasing clockwise. See the `Direction` table in the globals module."
    )]
    pub direction: u8,
    pub drop_position: Option<Position>,
    pub pickup_position: Option<Position>, // only type = inserter
    // Factorio 2.0 format; empty arrives as `{}` (Lua's empty table), so this
    // needs the map-or-sequence tolerant deserializer, not a plain `Option<Vec<_>>`.
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub output_inventory: Option<Vec<InventoryItemWithQuality>>,
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub fuel_inventory: Option<Vec<InventoryItemWithQuality>>,
    pub amount: Option<u32>,        // only type = resource
    pub recipe: Option<String>,     // only CraftingMachines
    pub ghost_name: Option<String>, // only type = entity-ghost
    pub ghost_type: Option<String>, // only type = entity-ghost
}

impl crate::aabb_quadtree::Spatial<Rect> for FactorioEntity {
    fn aabb(&self) -> QuadTreeRect {
        self.bounding_box.clone().into()
    }
}

impl FactorioEntity {
    /// `blueprint_version` is the blueprint's own `version` field, which packs
    /// the Factorio major version into its top 16 bits. It decides how to read
    /// `direction`: see [`blueprint_direction`].
    pub fn from_blueprint_entity(
        entity: Entity,
        blueprint_version: u64,
        prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    ) -> Result<Self> {
        let position: Position = entity.position.into();
        let direction: Option<Direction> = entity
            .direction
            .map(|d| blueprint_direction(d, blueprint_version));
        Self::from_prototype(
            &entity.name,
            position,
            direction,
            entity.pickup_position.map(|p| p.into()),
            entity.drop_position.map(|p| p.into()),
            prototypes,
        )
    }

    pub fn from_prototype(
        name: &str,
        position: Position,
        direction: Option<Direction>,
        pickup_position: Option<Position>,
        drop_position: Option<Position>,
        prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    ) -> Result<Self> {
        let direction = direction.unwrap_or(Direction::North);
        match prototypes.get(name) {
            Some(prototype) => Ok(FactorioEntity {
                bounding_box: add_to_rect_turned(&prototype.collision_box, &position, direction),
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                entity_type: prototype.entity_type.clone(),
                pickup_position,
                drop_position,
                ..Default::default()
            }),
            _ => Ok(FactorioEntity {
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                pickup_position,
                drop_position,
                ..Default::default()
            }),
        }
    }
    pub fn new_transport_belt(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::TransportBelt.to_string(),
            entity_type: EntityType::TransportBelt.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    /// `output = true` is the surfacing half of the pair; Factorio calls the
    /// two halves `input` and `output` and they take the same direction.
    ///
    /// FIXME: `FactorioEntity` has no field to record which half this is
    /// (checked: there is no `entity_data`, no `belt_to_ground_type`, nothing
    /// else that fits -- see the struct above). `output` is accepted so a
    /// caller can express intent and so the signature matches the two
    /// distinct entities a route actually places, but it is otherwise
    /// dropped here: the returned entity cannot be told apart from its mate
    /// by inspecting it, only by its position relative to the other half. In
    /// game, Factorio infers input/output automatically from direction and
    /// the presence of a matching underground belt within range, so this may
    /// not block placement -- but a caller that needs to *read back* which
    /// half an entity is will need a new field on `FactorioEntity`, which is
    /// out of scope for this constructor (it is a shared type used well
    /// beyond routing).
    pub fn new_underground_belt(
        position: &Position,
        direction: Direction,
        output: bool,
    ) -> FactorioEntity {
        let _ = output; // see FIXME above: nowhere to carry this yet.
        FactorioEntity {
            name: "underground-belt".into(),
            entity_type: "underground-belt".into(),
            position: position.clone(),
            // Same footprint as `new_transport_belt`'s: the real prototype's
            // collision box is 0.796875 x 0.796875 (checked against
            // `crates/core/tests/entity-prototype-fixtures.json`), and that
            // sibling already rounds the same box to 0.8 x 0.8.
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_splitter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Splitter.to_string(),
            entity_type: EntityType::Splitter.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.78, 0.78), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_inserter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Inserter.to_string(),
            entity_type: EntityType::Inserter.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.78, 0.78), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Position::new(0., 1.)
                .turn(direction)
                .map(|p| position.add(&p)),
            pickup_position: Position::new(0., -1.)
                .turn(direction)
                .map(|p| position.add(&p)),
            ..Default::default()
        }
    }
    pub fn new_burner_mining_drill(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::BurnerMiningDrill.to_string(),
            entity_type: EntityType::MiningDrill.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Position::new(-0.5, -1.296875)
                .turn(direction)
                .map(|p| position.add(&p)),
            ..Default::default()
        }
    }
    pub fn new_electric_mining_drill(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::ElectricMiningDrill.to_string(),
            entity_type: EntityType::MiningDrill.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Position::new(0., -2.)
                .turn(direction)
                .map(|p| position.add(&p)),
            ..Default::default()
        }
    }
    pub fn new_resource(position: &Position, direction: Direction, name: &str) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: EntityType::Resource.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_tree(position: &Position) -> FactorioEntity {
        FactorioEntity {
            name: "tree-42".into(),
            entity_type: EntityType::Tree.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(0.8, 0.8), position),
            ..Default::default()
        }
    }
    pub fn new_rock(position: &Position, name: &str) -> FactorioEntity {
        FactorioEntity {
            name: name.to_owned(),
            entity_type: EntityType::SimpleEntity.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(1.2, 1.2), position),
            ..Default::default()
        }
    }
    pub fn new_coal(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Coal.to_string(),
            entity_type: EntityType::Resource.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_stone_furnace(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::StoneFurnace.to_string(),
            entity_type: EntityType::Furnace.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }

    pub fn is_minable(&self) -> bool {
        self.entity_type == EntityType::Tree.to_string()
            || self.entity_type == EntityType::SimpleEntity.to_string()
    }
}

impl From<factorio_blueprint::objects::Position> for Position {
    fn from(pos: factorio_blueprint::objects::Position) -> Self {
        Position {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioItemPrototype {
    pub name: String,
    pub item_type: String,
    pub stack_size: u32,
    pub fuel_value: u64,
    pub place_result: String,
    pub group: String,
    pub subgroup: String,
}

#[derive(EnumString, Display, Debug)]
#[strum(serialize_all = "kebab-case")]
pub enum EntityName {
    // raw resources
    Water,
    Wood,
    Stone,
    Coal,
    IronOre,
    CopperOre,
    UraniumOre,
    CrudeOil,

    // processed
    StoneBrick,
    CopperPlate,
    IronPlate,
    Steel,

    // entities
    StoneFurnace,
    Inserter,
    BurnerMiningDrill,
    TransportBelt,
    Splitter,
    ElectricMiningDrill,
    Pumpjack,
}

#[derive(EnumString, Display, Debug, PartialEq, Clone, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
pub enum EntityType {
    AssemblingMachine,
    LogisticContainer,
    Boiler,
    Lab,
    Container,
    Resource,
    SimpleEntity,
    Tree,
    Inserter,
    MiningDrill,
    Furnace,
    TransportBelt,
    Splitter,
    UndergroundBelt,
    Pipe,
    PipeToGround,
    StorageTank,
    OffshorePump,
    // The electric network. Added 2026-09-02: without these three,
    // `EntityType::from_str("electric-pole")` is an `Err` and
    // `EntityGraph::add` drops the entity before its whitelist is consulted,
    // so a power plant a live world already contains is collidable and
    // otherwise invisible -- and the planner's `electric_supply_kw` scores
    // every real base 0 kW. See
    // `docs/superpowers/notes/2026-09-02-building-power.md`.
    //
    // `SolarPanel` is here to be *readable*, not to be credited: the planner
    // deliberately refuses to count a panel as generation, because its output
    // depends on the in-game time of day and the same plan would be feasible
    // or not according to when the run started.
    //
    // `Accumulator` is deliberately absent, and so is every other electrical
    // type: an unrecognised type is skipped exactly as it was before, so
    // naming one here is a decision to model it, not a formality.
    ElectricPole,
    Generator,
    SolarPanel,
    FlyingText,
    StraightRail,
    CurvedRail,
    Fish,
}

impl EntityType {
    pub fn is_fluid_input(&self) -> bool {
        *self == EntityType::Pipe
            || *self == EntityType::StorageTank
            || *self == EntityType::PipeToGround
            || *self == EntityType::Boiler
    }
}

#[derive(Debug)]
pub struct ResourcePatch {
    pub name: String,
    pub id: u32,
    pub rect: Rect,
    pub elements: Vec<Position>,
}

impl ResourcePatch {
    pub fn contains(&self, pos: Pos) -> bool {
        self.elements
            .iter()
            .map(|e| e.into())
            .any(|x: Pos| x == pos)
    }
    pub fn find_free_rect(
        &self,
        width: u32,
        height: u32,
        near: &Position,
        // blocked: &ReadHandle<Pos, bool>,
    ) -> Option<Rect> {
        let mut elements = self.elements.clone();
        elements.sort_by(|a, b| {
            let da = r64(calculate_distance(a, near));
            let db = r64(calculate_distance(b, near));
            da.cmp(&db)
        });

        let mut element_map: HashMap<Pos, bool> = HashMap::new();
        for element in &elements {
            element_map.insert(element.into(), true);
        }
        for element in &elements {
            let mut invalid = false;
            for y in 0i32..height as i32 {
                for x in 0i32..width as i32 {
                    // `floor`, not `as i32`: elements are tile centres, so a
                    // negative one is `-40.5`, which truncates to `-40` but
                    // floors to `-41` -- and `element_map` is keyed by `Pos`,
                    // which floors.
                    let pos = Pos(
                        element.x().floor() as i32 + x,
                        element.y().floor() as i32 + y,
                    );
                    // let blocked_pos = blocked.get_one(&pos);
                    // if blocked_pos.is_some() && !*blocked_pos.unwrap() {
                    //     invalid = true;
                    //     break;
                    // }
                    if !element_map.contains_key(&pos) {
                        invalid = true;
                        break;
                    }
                }
                if invalid {
                    break;
                }
            }
            if !invalid {
                // Anchor on the element's tile *corner*, not its centre.
                // Elements are centres (`-36.5`), so building the rect from one
                // directly and then `rect_floor_ceil`ing it widens the box by a
                // tile in each axis -- floor(-36.5) = -37 and ceil(-34.5) = -34
                // spans three tiles for a two-tile request. Flooring first makes
                // the rect exactly `width` x `height`, and leaves
                // `rect_floor_ceil` a no-op rather than an inflator.
                let left = element.x().floor();
                let top = element.y().floor();
                return Some(rect_floor_ceil(&Rect {
                    left_top: Position::new(left, top),
                    right_bottom: Position::new(left + width as f64, top + height as f64),
                }));
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct InventoryItem {
    pub name: String,
    pub count: u32,
}

impl InventoryItem {
    pub fn new(name: &str, count: u32) -> InventoryItem {
        InventoryItem {
            name: name.into(),
            count,
        }
    }
}

/// Inventory item with quality field (Factorio 2.0 format)
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema, Hash, Eq,
)]
#[serde(rename_all = "snake_case")]
pub struct InventoryItemWithQuality {
    pub name: String,
    pub quality: String,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct InventoryLocation {
    pub entity_name: String,
    pub position: Position,
    pub inventory_type: u32,
}

#[derive(Debug, Clone)]
pub struct EntityPlacement {
    pub item_name: String,
    pub position: Position,
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct PositionRadius {
    pub position: Position,
    pub radius: f64,
}

impl PositionRadius {
    pub fn new(x: f64, y: f64, radius: f64) -> PositionRadius {
        PositionRadius {
            position: Position::new(x, y),
            radius,
        }
    }
    pub fn from_position(pos: &Position, radius: f64) -> PositionRadius {
        PositionRadius {
            position: pos.clone(),
            radius,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MineTarget {
    pub position: Position,
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioResult {
    pub success: bool,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlaceEntityResult {
    pub player: FactorioPlayer,
    pub entity: FactorioEntity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlaceEntitiesResult {
    pub player: FactorioPlayer,
    pub entities: Vec<FactorioEntity>,
}

// The `on_player_changed_distance` payload: the same six `LuaControl`
// distances as `FactorioPlayer`, and so the same widths.
//
// Not `Hash`/`Eq`: three of the six are `double`, which has neither. Nothing
// keyed this type; it is only serialized to websocket clients and deserialized
// from the mod.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedDistanceEvent {
    pub player_id: PlayerId,
    pub build_distance: u32,
    pub reach_distance: u32,
    pub drop_item_distance: u32,
    pub item_pickup_distance: f64,
    pub loot_pickup_distance: f64,
    pub resource_reach_distance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedPositionEvent {
    pub player_id: PlayerId,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedMainInventoryEvent {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub main_inventory: Vec<InventoryItemWithQuality>,
}

impl PlayerChangedMainInventoryEvent {
    /// Create an event from a BTreeMap (for internal use/simulation)
    pub fn from_btreemap(player_id: PlayerId, inventory: BTreeMap<String, u32>) -> Self {
        let main_inventory = inventory
            .into_iter()
            .map(|(name, count)| InventoryItemWithQuality {
                name,
                quality: "normal".to_string(),
                count,
            })
            .collect();
        Self {
            player_id,
            main_inventory,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerLeftEvent {
    pub player_id: PlayerId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ScriptTreeNode {
    pub key: String,
    pub label: String,
    pub leaf: bool,
    // Self-referential (`ScriptTreeNode` -> `ScriptTreeNode`): without
    // `no_recursion`, utoipa's OpenAPI schema generation recurses into this
    // field forever and aborts the process with a stack overflow the first
    // time any route referencing this type builds its schema.
    #[schema(no_recursion)]
    pub children: Vec<ScriptTreeNode>,
}

impl IntoLua for InventoryResponse {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        lua.to_value(&self)
    }
}

impl FromLuaMulti for Position {
    fn from_lua_multi(values: LuaMultiValue, _lua: &Lua) -> LuaResult<Self> {
        let values: Vec<LuaValue> = values.into_iter().collect();
        if values.len() < 2 {
            return Err(LuaError::RuntimeError(
                "invalid position: too few values".into(),
            ));
        }
        if let LuaValue::Number(x) = values[0] {
            if let LuaValue::Number(y) = values[1] {
                Ok(Position::new(x, y))
            } else {
                Err(LuaError::RuntimeError(
                    "invalid position: y is not a number".into(),
                ))
            }
        } else {
            Err(LuaError::RuntimeError(
                "invalid position: x is not a number".into(),
            ))
        }
    }
}

impl IntoLua for Rect {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        lua.to_value(&self)
    }
}

impl IntoLua for FactorioEntity {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        lua.to_value(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factorio::util::{move_pos, move_position};

    fn resource_prototype(name: &str, product: &str) -> FactorioEntityPrototype {
        FactorioEntityPrototype {
            name: name.into(),
            entity_type: "resource".into(),
            collision_mask: None,
            collision_box: Rect::from_wh(1., 1.),
            mine_result: Some(BTreeMap::from([(product.to_string(), 1)])),
            mining_time: Some(1.),
            mining_speed: None,
            crafting_speed: None,
            max_underground_distance: None,
            fluidbox_prototypes: None,
            resource_category: None,
            resource_categories: None,
            mining_fluid: None,
        }
    }

    fn character_mines(categories: &[&str]) -> Vec<String> {
        categories.iter().map(|c| (*c).to_string()).collect()
    }

    /// The game's rule: a resource whose category the character does not
    /// list is not hand-minable, whatever `minable` and `mine_result` say.
    /// Crude oil is `basic-fluid`; the character mines `basic-solid`.
    #[test]
    fn a_resource_outside_the_characters_categories_refuses_a_hand() {
        let mut well = resource_prototype("crude-oil", "crude-oil");
        well.resource_category = Some("basic-fluid".into());
        // `is_item` says yes to everything here, so the category alone
        // carries the refusal.
        let obstacle = well.hand_mining_obstacle(&character_mines(&["basic-solid"]), &|_| true);
        assert_eq!(
            obstacle,
            Some(HandMiningObstacle::Category {
                resource_category: "basic-fluid".into(),
                character_categories: character_mines(&["basic-solid"]),
            })
        );
        // And a character that does list it may mine it.
        let obstacle =
            well.hand_mining_obstacle(&character_mines(&["basic-solid", "basic-fluid"]), &|_| true);
        assert_eq!(obstacle, None);
    }

    /// Uranium ore is `basic-solid` and still refuses a hand: mining it needs
    /// sulfuric acid piped in.
    #[test]
    fn a_resource_needing_a_fluid_refuses_a_hand() {
        let mut ore = resource_prototype("uranium-ore", "uranium-ore");
        ore.resource_category = Some("basic-solid".into());
        ore.mining_fluid = Some("sulfuric-acid".into());
        assert_eq!(
            ore.hand_mining_obstacle(&character_mines(&["basic-solid"]), &|_| true),
            Some(HandMiningObstacle::NeedsFluid {
                fluid: "sulfuric-acid".into()
            })
        );
    }

    /// A prototype captured before `resource_category` existed -- every dump
    /// written before 2026-09-04, and `tests/entity-prototype-fixtures.json`
    /// -- still refuses crude oil, because its product is not an item.
    #[test]
    fn an_old_capture_still_refuses_a_product_that_is_not_an_item() {
        let well = resource_prototype("crude-oil", "crude-oil");
        assert_eq!(well.resource_category, None, "the old shape");
        let is_item = |name: &str| name != "crude-oil";
        assert_eq!(
            well.hand_mining_obstacle(&character_mines(&["basic-solid"]), &is_item),
            Some(HandMiningObstacle::YieldsNoItem {
                product: "crude-oil".into()
            })
        );
        // The same old shape for iron ore says nothing against a hand.
        let ore = resource_prototype("iron-ore", "iron-ore");
        assert_eq!(
            ore.hand_mining_obstacle(&character_mines(&["basic-solid"]), &is_item),
            None
        );
    }

    /// Trees and rocks are mined by another path and are never this
    /// method's business, whatever their products are.
    #[test]
    fn only_resource_prototypes_are_judged() {
        let mut tree = resource_prototype("tree-01", "wood");
        tree.entity_type = "tree".into();
        tree.resource_category = Some("basic-fluid".into());
        assert_eq!(
            tree.hand_mining_obstacle(&character_mines(&["basic-solid"]), &|_| false),
            None
        );
    }

    /// The three fields are optional on the wire, and the mod's empty-table
    /// spelling of "no categories" is tolerated: a prototype record written
    /// before the fields existed, or by a Lua table that serialised `{}`,
    /// loads with `None`.
    #[test]
    fn hand_mining_fields_default_when_absent_or_empty() {
        let old = r#"{"name":"crude-oil","entity_type":"resource","collision_box":{"left_top":{"x":-1.4,"y":-1.4},"right_bottom":{"x":1.4,"y":1.4}},"mine_result":{"crude-oil":10},"mining_time":1.0}"#;
        let proto: FactorioEntityPrototype = serde_json::from_str(old).expect("old shape loads");
        assert_eq!(proto.resource_category, None);
        assert_eq!(proto.resource_categories, None);
        assert_eq!(proto.mining_fluid, None);

        let empty = r#"{"name":"character","entity_type":"character","collision_box":{"left_top":{"x":-0.2,"y":-0.2},"right_bottom":{"x":0.2,"y":0.2}},"resource_categories":{}}"#;
        let proto: FactorioEntityPrototype =
            serde_json::from_str(empty).expect("an empty Lua table loads");
        // The helper's contract: an empty table is an empty list, not `None`.
        // `PlanState::hand_mining_obstacle` reads an empty list as "not said".
        assert_eq!(proto.resource_categories, Some(vec![]));

        let new = r#"{"name":"crude-oil","entity_type":"resource","collision_box":{"left_top":{"x":-1.4,"y":-1.4},"right_bottom":{"x":1.4,"y":1.4}},"mine_result":{"crude-oil":10},"mining_time":1.0,"resource_category":"basic-fluid"}"#;
        let proto: FactorioEntityPrototype = serde_json::from_str(new).expect("new shape loads");
        assert_eq!(proto.resource_category.as_deref(), Some("basic-fluid"));
    }

    /// The trigger shapes, verbatim from the shipped API definition:
    /// `workspace/factorio-api-docs/runtime-api.json`, `application_version`
    /// 2.1.17, concept `ResearchTrigger`. Eight `type` values. The bare type
    /// -- what the mod sent for everything but `craft-item` until 2026-09-05,
    /// and so what every archived dump holds -- must still load, with every
    /// payload at its default.
    #[test]
    fn every_documented_trigger_type_deserialises_to_its_own_variant() {
        let cases: [(&str, ResearchTrigger); 7] = [
            (
                r#"{"type":"craft-fluid"}"#,
                ResearchTrigger::CraftFluid {
                    fluid: None,
                    amount: Box::new(r64(1.0)),
                },
            ),
            (
                r#"{"type":"mine-entity"}"#,
                ResearchTrigger::MineEntity {
                    entities: vec![],
                    count: 1,
                },
            ),
            (
                r#"{"type":"build-entity"}"#,
                ResearchTrigger::BuildEntity {
                    entities: vec![],
                    count: 1,
                },
            ),
            (
                r#"{"type":"send-item-to-orbit"}"#,
                ResearchTrigger::SendItemToOrbit { item: None },
            ),
            (
                r#"{"type":"capture-spawner"}"#,
                ResearchTrigger::CaptureSpawner { entity: None },
            ),
            (
                r#"{"type":"create-space-platform"}"#,
                ResearchTrigger::CreateSpacePlatform,
            ),
            (r#"{"type":"scripted"}"#, ResearchTrigger::Scripted),
        ];
        for (json, expected) in cases {
            let got: ResearchTrigger =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} must parse: {e}"));
            assert_eq!(got, expected, "for {json}");
            assert_ne!(
                got,
                ResearchTrigger::Unknown,
                "{json} is documented and must not fall through to Unknown"
            );
        }
    }

    /// The shape the mod sends since 2026-09-05 for `oil-processing`, read
    /// off `data/base/prototypes/technology.lua` (`entities = {"crude-oil"}`)
    /// and normalised to a list. `count` is absent there and means one.
    #[test]
    fn a_mine_entity_trigger_keeps_its_entity_list() {
        let got: ResearchTrigger =
            serde_json::from_str(r#"{"type":"mine-entity","entities":["crude-oil"]}"#)
                .expect("parses");
        assert_eq!(
            got,
            ResearchTrigger::MineEntity {
                entities: vec!["crude-oil".into()],
                count: 1,
            }
        );
        assert_eq!(got.to_string(), "mine 1 crude-oil");

        // Space Age's `tungsten-carbide` names four rocks for one trigger.
        let got: ResearchTrigger = serde_json::from_str(
            r#"{"type":"mine-entity","entities":["big-volcanic-rock","huge-volcanic-rock"],"count":2}"#,
        )
        .expect("parses");
        assert_eq!(
            got.to_string(),
            "mine 2 big-volcanic-rock or huge-volcanic-rock"
        );
    }

    /// The three other spellings a trigger field can arrive in: the singular
    /// `entity` string `runtime-api.json` documents, a filter table with a
    /// `name` (what `EntityIDFilter` is), and Lua's `{}` for an empty list.
    /// The mod normalises all of them, but a dump written by hand or by an
    /// older mod that forwarded the table untouched must not fail to load.
    #[test]
    fn a_mine_entity_trigger_accepts_the_singular_and_filter_spellings() {
        let singular: ResearchTrigger =
            serde_json::from_str(r#"{"type":"mine-entity","entities":"crude-oil"}"#)
                .expect("a bare name parses");
        assert!(matches!(
            singular,
            ResearchTrigger::MineEntity { ref entities, .. } if entities == &["crude-oil".to_string()]
        ));
        let filters: ResearchTrigger = serde_json::from_str(
            r#"{"type":"build-entity","entities":[{"name":"radar","quality":"normal"}],"count":3}"#,
        )
        .expect("a filter list parses");
        assert_eq!(
            filters,
            ResearchTrigger::BuildEntity {
                entities: vec!["radar".into()],
                count: 3,
            }
        );
        let empty: ResearchTrigger =
            serde_json::from_str(r#"{"type":"mine-entity","entities":{}}"#)
                .expect("Lua's empty table parses");
        assert_eq!(
            empty,
            ResearchTrigger::MineEntity {
                entities: vec![],
                count: 1,
            }
        );
    }

    /// The payloads of the remaining kinds, per `runtime-api.json` 2.1.17.
    #[test]
    fn the_other_trigger_kinds_keep_their_payloads() {
        let fluid: ResearchTrigger =
            serde_json::from_str(r#"{"type":"craft-fluid","fluid":"steam","amount":200.0}"#)
                .expect("parses");
        assert_eq!(
            fluid,
            ResearchTrigger::CraftFluid {
                fluid: Some("steam".into()),
                amount: Box::new(r64(200.0)),
            }
        );
        assert_eq!(fluid.to_string(), "craft 200 steam");
        let orbit: ResearchTrigger =
            serde_json::from_str(r#"{"type":"send-item-to-orbit","item":"satellite"}"#)
                .expect("parses");
        assert_eq!(orbit.to_string(), "send satellite to orbit");
        let spawner: ResearchTrigger =
            serde_json::from_str(r#"{"type":"capture-spawner","entity":"biter-spawner"}"#)
                .expect("parses");
        assert_eq!(spawner.to_string(), "capture a biter-spawner");
    }

    /// `craft-item` is the one variant the planner can act on, so its payload
    /// has to survive. The count is the number of items that must be crafted.
    #[test]
    fn a_craft_item_trigger_keeps_its_item_and_count() {
        let got: ResearchTrigger =
            serde_json::from_str(r#"{"type":"craft-item","item":"steel-plate","count":50}"#)
                .expect("parses");
        assert_eq!(
            got,
            ResearchTrigger::CraftItem {
                item: "steel-plate".into(),
                count: 50,
            }
        );
        assert_eq!(got.to_string(), "craft 50 steel-plate");
    }

    /// The shipped prototype omits `count` when it means one --
    /// `automation-science-pack` is `{type = "craft-item", item = "lab"}`. An
    /// absent count must mean one, not zero: zero would silently restore the
    /// free-research defect this type exists to remove.
    #[test]
    fn an_absent_craft_item_count_defaults_to_one() {
        let got: ResearchTrigger =
            serde_json::from_str(r#"{"type":"craft-item","item":"lab"}"#).expect("parses");
        assert_eq!(
            got,
            ResearchTrigger::CraftItem {
                item: "lab".into(),
                count: 1,
            }
        );
    }

    /// A newer Factorio or a mod may add a trigger type this build has never
    /// heard of. That must not make the whole world unreadable -- one strange
    /// technology would otherwise take every other technology down with it --
    /// so it lands in `Unknown`, which the planner treats as inexpressible and
    /// refuses rather than costs at zero.
    #[test]
    fn an_unrecognised_trigger_type_falls_back_instead_of_failing_the_parse() {
        let got: ResearchTrigger =
            serde_json::from_str(r#"{"type":"teleport-a-biter","entity":"small-biter"}"#)
                .expect("an unknown type must still parse");
        assert_eq!(got, ResearchTrigger::Unknown);
        assert_eq!(got.kind(), "unknown");
    }

    /// A technology captured before this field existed has no `research_trigger`
    /// key at all. That is the ordinary pack-researched technology and must
    /// deserialise to `None`, leaving every existing fixture readable.
    #[test]
    fn a_technology_without_a_trigger_key_is_pack_researched() {
        let tech: FactorioTechnology = serde_json::from_str(
            r#"{"name":"automation","enabled":true,"upgrade":false,"researched":false,
                "prerequisites":[],"research_unit_ingredients":[],"research_unit_count":10,
                "research_unit_energy":600.0,"order":"a","level":1,"valid":true}"#,
        )
        .expect("parses without the new key");
        assert_eq!(tech.research_trigger, None);
    }

    /// The sixteen names and discriminants, verbatim from the shipped API
    /// definition: `workspace/factorio-api-docs/runtime-api.json`,
    /// `application_version` 2.1.17, `defines.direction`. Restating the list
    /// from memory is how the intercardinal names get subtly wrong, so this
    /// pins every one of them against the file rather than against four
    /// well-known cardinals.
    #[test]
    fn the_sixteen_directions_match_factorio_2_x_defines_direction() {
        let expected: [(Direction, u8); 16] = [
            (Direction::North, 0),
            (Direction::NorthNorthEast, 1),
            (Direction::NorthEast, 2),
            (Direction::EastNorthEast, 3),
            (Direction::East, 4),
            (Direction::EastSouthEast, 5),
            (Direction::SouthEast, 6),
            (Direction::SouthSouthEast, 7),
            (Direction::South, 8),
            (Direction::SouthSouthWest, 9),
            (Direction::SouthWest, 10),
            (Direction::WestSouthWest, 11),
            (Direction::West, 12),
            (Direction::WestNorthWest, 13),
            (Direction::NorthWest, 14),
            (Direction::NorthNorthWest, 15),
        ];
        for (direction, value) in expected {
            assert_eq!(
                direction.to_u8(),
                Some(value),
                "{direction:?} is {value} in defines.direction"
            );
        }
    }

    /// Every value the game can send survives the round trip.
    ///
    /// Before the widening exactly **one** did: north. The game's `east` (4)
    /// read back as `South`, and `East = 2` was read by the game as northeast.
    #[test]
    fn every_2_x_direction_survives_the_round_trip() {
        for n in 0u8..=15 {
            let direction = Direction::from_u8(n).unwrap_or_else(|| panic!("no direction for {n}"));
            assert_eq!(
                direction.to_u8(),
                Some(n),
                "{direction:?} does not round-trip from {n}"
            );
        }
        assert!(
            Direction::from_u8(16).is_none(),
            "16 is outside defines.direction, and the log-and-skip path needs a value that is"
        );
    }

    /// `opposite` is 180 degrees, which is a step of **eight** on a sixteen-value
    /// scale, not four. A renumber that left the `+ 4` in place would still
    /// compile and still return a `Direction` -- it would just return the wrong
    /// one, silently, for all sixteen inputs.
    #[test]
    fn opposite_is_a_half_turn_on_the_sixteen_value_scale() {
        assert_eq!(Direction::North.opposite(), Direction::South);
        assert_eq!(Direction::South.opposite(), Direction::North);
        assert_eq!(Direction::East.opposite(), Direction::West);
        assert_eq!(Direction::West.opposite(), Direction::East);
        assert_eq!(Direction::NorthEast.opposite(), Direction::SouthWest);
        // A half-diagonal has an opposite too, and it is another half-diagonal.
        assert_eq!(
            Direction::NorthNorthEast.opposite(),
            Direction::SouthSouthWest
        );
        for direction in Direction::all() {
            assert_eq!(
                direction.opposite().opposite(),
                direction,
                "{direction:?} must be its own double-opposite"
            );
        }
    }

    /// `clockwise` stays a **90 degree** turn, which is a step of four here.
    ///
    /// Keeping the arithmetic at `+ 2` would have quietly demoted it to 45
    /// degrees, and `flow_graph`'s belt-orientation comparisons against
    /// `clockwise()` would then all be false -- no panic, no failing type, just
    /// a flow graph with no corners in it.
    #[test]
    fn clockwise_is_a_quarter_turn_on_the_sixteen_value_scale() {
        assert_eq!(Direction::North.clockwise(), Direction::East);
        assert_eq!(Direction::East.clockwise(), Direction::South);
        assert_eq!(Direction::South.clockwise(), Direction::West);
        assert_eq!(Direction::West.clockwise(), Direction::North);
        assert_eq!(Direction::NorthEast.clockwise(), Direction::SouthEast);
        for direction in Direction::all() {
            assert_eq!(
                direction.clockwise().clockwise(),
                direction.opposite(),
                "two quarter turns from {direction:?} must be a half turn"
            );
        }
    }

    /// Sixteen, eight and four -- and which four.
    ///
    /// `orthogonal()` still returns the cardinals, but their *values* moved
    /// from 0/2/4/6 to 0/4/8/12, so the `n % 2 == 0` filter that used to select
    /// them now selects all eight compass points instead.
    #[test]
    fn all_compass_and_orthogonal_return_sixteen_eight_and_four() {
        assert_eq!(Direction::all().len(), 16);
        assert_eq!(Direction::compass().len(), 8);
        assert_eq!(
            Direction::orthogonal(),
            vec![
                Direction::North,
                Direction::East,
                Direction::South,
                Direction::West
            ],
            "orthogonal must still be the four cardinals, at 0/4/8/12"
        );
        assert!(
            Direction::compass().iter().all(Direction::is_compass),
            "compass must contain no half-diagonals"
        );
        assert_eq!(
            Direction::all()
                .into_iter()
                .filter(|d| !d.is_compass())
                .count(),
            8,
            "the other eight are the half-diagonals rails use"
        );
    }

    /// Blueprint strings are a *persisted* format and did not renumber when
    /// `defines.direction` did, so the version that wrote the blueprint decides
    /// how to read its directions.
    ///
    /// This is the call site the compiler cannot point at: `from_blueprint_entity`
    /// took `direction` straight through, and a 1.x blueprint's `east = 2` read
    /// on the 2.x scale is *northeast* -- every belt in the blueprint turned 45
    /// degrees, no error anywhere. `entity_graph`'s splitter fixtures are 1.x
    /// blueprints (`version` 0x1000000000000) and are what caught it.
    #[test]
    fn a_blueprints_direction_is_read_on_the_scale_its_version_names() {
        // Factorio 1.x: eight values, so 2 is east and lands on 2.x's 4.
        assert_eq!(blueprint_direction(0, 1 << 48), Direction::North);
        assert_eq!(blueprint_direction(2, 1 << 48), Direction::East);
        assert_eq!(blueprint_direction(4, 1 << 48), Direction::South);
        assert_eq!(blueprint_direction(6, 1 << 48), Direction::West);
        // Factorio 2.x: sixteen values, taken as they are.
        assert_eq!(
            blueprint_direction(0, BLUEPRINT_VERSION_2_0),
            Direction::North
        );
        assert_eq!(
            blueprint_direction(4, BLUEPRINT_VERSION_2_0),
            Direction::East
        );
        assert_eq!(
            blueprint_direction(8, BLUEPRINT_VERSION_2_0),
            Direction::South
        );
        assert_eq!(
            blueprint_direction(12, BLUEPRINT_VERSION_2_0),
            Direction::West
        );
        assert_eq!(
            blueprint_direction(1, BLUEPRINT_VERSION_2_0),
            Direction::NorthNorthEast,
            "a 2.x blueprint can contain a rail's half-diagonal"
        );
        assert_eq!(
            BLUEPRINT_VERSION_2_0, 562949953421312,
            "blueprint version packs major into the top 16 bits"
        );
    }

    /// The half-diagonals are readable but name no tile, so the offset helpers
    /// report their absence instead of inventing one. Before the widening the
    /// unhandled arm of `turn` was `panic!`, and this crate builds
    /// `panic = "abort"`.
    #[test]
    fn a_half_diagonal_has_no_tile_offset_and_no_rotation() {
        for direction in Direction::all().into_iter().filter(|d| !d.is_compass()) {
            let origin = Position::new(0., 0.);
            assert!(
                move_position(&origin, direction, 1.).is_none(),
                "{direction:?} names no tile offset"
            );
            assert!(
                move_pos(&Pos(0, 0), direction, 1).is_none(),
                "{direction:?} names no tile offset"
            );
            assert!(
                origin.turn(direction).is_none(),
                "{direction:?} names no rotation"
            );
        }
        assert!(
            Position::new(1., 0.).turn(Direction::NorthEast).is_none(),
            "turn only handles the four cardinals, and the other twelve must not abort"
        );
        assert_eq!(
            move_position(&Position::new(0., 0.), Direction::East, 1.),
            Some(Position::new(1., 0.)),
            "east is +x, and it is 4 now"
        );
    }

    /// A free rect must cover exactly the tiles that were asked for.
    ///
    /// The coordinate-specific `test_free_rect_*` cases in `scripting_lua` pin
    /// *where* the rect lands; this pins its *size*, which is the property that
    /// broke when resource elements moved from tile corners to tile centres.
    /// Anchoring on a centre and then `rect_floor_ceil`ing widened every rect by
    /// one tile in each axis, so a 2x2 request silently returned 3x3 -- a
    /// difference no positional assertion states outright.
    #[test]
    fn a_free_rect_is_exactly_the_requested_size() {
        // A solid 6x6 ore field, elements at tile centres as the game reports
        // them.
        let elements: Vec<Position> = (0..6)
            .flat_map(|y| {
                (0..6).map(move |x| Position::new(f64::from(x) - 40.5, f64::from(y) + 35.5))
            })
            .collect();
        let patch = ResourcePatch {
            name: "iron-ore".into(),
            id: 1,
            rect: Rect {
                left_top: Position::new(-41.0, 35.0),
                right_bottom: Position::new(-35.0, 41.0),
            },
            elements,
        };

        let mut checked = 0;
        for (w, h) in [(1u32, 1u32), (2, 2), (3, 2), (2, 3), (4, 4)] {
            let rect = patch
                .find_free_rect(w, h, &Position::new(0.0, 0.0))
                .unwrap_or_else(|| panic!("no {w}x{h} rect in a solid 6x6 field"));
            assert_eq!(
                rect.right_bottom.x() - rect.left_top.x(),
                f64::from(w),
                "width for a {w}x{h} request: {rect:?}"
            );
            assert_eq!(
                rect.right_bottom.y() - rect.left_top.y(),
                f64::from(h),
                "height for a {w}x{h} request: {rect:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 5, "every size was exercised");
    }

    /// The shape `mods/BotBridge/types.lua` produces on Factorio 2.0, where
    /// `LuaInventory.get_contents()` returns an array with quality.
    #[test]
    fn a_player_parses_with_a_2_0_inventory_array() {
        let json = r#"{
            "name": "client1",
            "player_id": 2,
            "position": {"x": 1.5, "y": -2.0},
            "main_inventory": [
                {"name": "iron-plate", "count": 5, "quality": "normal"},
                {"name": "iron-plate", "count": 2, "quality": "uncommon"},
                {"name": "coal", "count": 3, "quality": "normal"}
            ],
            "build_distance": 10,
            "reach_distance": 10,
            "drop_item_distance": 10,
            "item_pickup_distance": 1,
            "loot_pickup_distance": 2,
            "resource_reach_distance": 3
        }"#;
        let player: FactorioPlayer = serde_json::from_str(json).expect("parses");
        assert_eq!(player.player_id, 2);
        // Qualities collapse into one count per name.
        assert_eq!(player.main_inventory.get("iron-plate").copied(), Some(7));
        assert_eq!(player.main_inventory.get("coal").copied(), Some(3));
    }

    #[test]
    fn a_player_parses_with_a_name_to_count_inventory_map() {
        let json = r#"{
            "player_id": 0,
            "position": {"x": 0.0, "y": 0.0},
            "main_inventory": {"iron-plate": 4},
            "build_distance": 10,
            "reach_distance": 10,
            "drop_item_distance": 10,
            "item_pickup_distance": 1,
            "loot_pickup_distance": 2,
            "resource_reach_distance": 3
        }"#;
        let player: FactorioPlayer = serde_json::from_str(json).expect("parses");
        assert_eq!(player.main_inventory.get("iron-plate").copied(), Some(4));
    }

    /// Lua serializes an empty table as `{}`, never `[]`.
    #[test]
    fn an_empty_inventory_arrives_as_an_empty_object() {
        let json = r#"{
            "player_id": 1,
            "position": {"x": 0.0, "y": 0.0},
            "main_inventory": {},
            "build_distance": 10,
            "reach_distance": 10,
            "drop_item_distance": 10,
            "item_pickup_distance": 1,
            "loot_pickup_distance": 2,
            "resource_reach_distance": 3
        }"#;
        let player: FactorioPlayer = serde_json::from_str(json).expect("parses");
        assert!(player.main_inventory.is_empty());
    }

    /// The exact `players` reply a live Factorio 2.1.17 server sent, taken from
    /// the `failed to parse players` error it caused. `LuaControl`'s
    /// `resource_reach_distance` is a `double`, and a character's real value is
    /// 2.7 — which serde cannot read into an integer, so *every* real player
    /// failed to deserialize and `RconActuator::new` could never be built.
    ///
    /// Kept verbatim, fractional digits and all: a fixture with whole-number
    /// distances parses fine even against the broken integer types and so
    /// cannot detect this.
    const LIVE_2_1_PLAYERS: &str = r#"[{"name":"client1","player_id":1,"position":{"y":0,"x":0},"build_distance":10,"reach_distance":10,"drop_item_distance":10,"item_pickup_distance":1,"loot_pickup_distance":2,"resource_reach_distance":2.70000000000000017763568394002504646778106689453125,"main_inventory":[{"name":"burner-mining-drill","quality":"normal","count":1},{"name":"stone-furnace","quality":"normal","count":1},{"name":"wood","quality":"normal","count":1}]}]"#;

    #[test]
    fn the_live_2_1_players_reply_parses_with_a_fractional_resource_reach() {
        // Deserialized exactly as `RconClient::connected_players` does it.
        let players: Vec<FactorioPlayer> = serde_json::from_str(LIVE_2_1_PLAYERS).expect("parses");
        assert_eq!(players.len(), 1);
        let player = &players[0];
        assert_eq!(player.player_id, 1);

        // The value the integer type could not hold, kept fractional rather
        // than silently rounded to 3.
        assert!(
            (player.resource_reach_distance - 2.7).abs() < 1e-9,
            "expected ~2.7, got {}",
            player.resource_reach_distance
        );
        assert_ne!(
            player.resource_reach_distance,
            player.resource_reach_distance.trunc(),
            "the fractional part must survive; rounding here is the bug"
        );

        // The three `uint32` distances stay whole.
        assert_eq!(player.build_distance, 10);
        assert_eq!(player.reach_distance, 10);
        assert_eq!(player.drop_item_distance, 10);
        // The other two `double` distances happened to arrive whole here.
        assert_eq!(player.item_pickup_distance, 1.0);
        assert_eq!(player.loot_pickup_distance, 2.0);

        assert_eq!(
            player.main_inventory.get("burner-mining-drill").copied(),
            Some(1)
        );
        assert_eq!(player.main_inventory.get("stone-furnace").copied(), Some(1));
        assert_eq!(player.main_inventory.get("wood").copied(), Some(1));
    }

    /// `LuaControl.resource_reach_distance` is documented as "the resource
    /// reach distance of this character **or max double** when not a character
    /// or player connected to a character", so the integer types were wrong on
    /// range as well as on fractionality: this value does not fit in a `u64`.
    ///
    /// Written in scientific notation, which is how a JSON encoder emits a
    /// double this large. Spelled out in full decimal digits it is a bare
    /// integer literal, and serde_json rejects those above `u64::MAX` with
    /// "number out of range" whatever the target field's type is — so that
    /// spelling would test the parser's integer path, not this field.
    #[test]
    fn a_player_parses_with_the_documented_max_double_resource_reach() {
        let json = format!(
            r#"{{"player_id":1,"position":{{"x":0,"y":0}},"main_inventory":{{}},
                "build_distance":10,"reach_distance":10,"drop_item_distance":10,
                "item_pickup_distance":1,"loot_pickup_distance":2,
                "resource_reach_distance":{:e}}}"#,
            f64::MAX
        );
        let player: FactorioPlayer = serde_json::from_str(&json).expect("parses");
        assert_eq!(player.resource_reach_distance, f64::MAX);
        assert!(player.resource_reach_distance > u64::MAX as f64);
    }

    /// The `on_player_changed_distance` payload carries the same six
    /// `LuaControl` distances and must accept the same doubles.
    #[test]
    fn a_distance_change_event_parses_with_a_fractional_resource_reach() {
        let json = r#"{"player_id":1,"build_distance":10,"reach_distance":10,
            "drop_item_distance":10,"item_pickup_distance":1.5,
            "loot_pickup_distance":2.25,
            "resource_reach_distance":2.70000000000000017763568394002504646778106689453125}"#;
        let event: PlayerChangedDistanceEvent = serde_json::from_str(json).expect("parses");
        assert!((event.resource_reach_distance - 2.7).abs() < 1e-9);
        assert_eq!(event.item_pickup_distance, 1.5);
        assert_eq!(event.loot_pickup_distance, 2.25);
    }

    /// The exact product object a live Factorio 2.1.17 server sent, taken from
    /// the panic in `output_parser.rs` that it caused: 2.1 has no
    /// `probability` field on `ItemProduct`, so requiring one rejected every
    /// recipe in the game.
    const LIVE_2_1_PRODUCT: &str = r#"{"name":"wooden-chest","product_type":"item","amount":1}"#;

    #[test]
    fn the_live_2_1_product_parses_and_is_certain() {
        let product: FactorioProduct = serde_json::from_str(LIVE_2_1_PRODUCT).expect("parses");
        assert_eq!(product.name, "wooden-chest");
        assert_eq!(product.product_type, "item");
        assert_eq!(product.amount, 1);
        assert_eq!(*product.probability, r64(1.0));
    }

    /// Factorio 2.0 and earlier sent `probability` directly. A workspace that
    /// still holds the old mod must keep working.
    #[test]
    fn a_legacy_probability_is_taken_as_is() {
        let json = r#"{"name":"coal","product_type":"item","amount":1,"probability":0.25}"#;
        let product: FactorioProduct = serde_json::from_str(json).expect("parses");
        assert_eq!(*product.probability, r64(0.25));
    }

    /// 2.1 splits the chance into an independent roll and a window on a shared
    /// roll; the effective chance is their product.
    #[test]
    fn a_2_1_probability_combines_the_independent_and_shared_chances() {
        let json = r#"{"name":"uranium-235","product_type":"item","amount":1,
            "independent_probability":0.5,"shared_probability":{"min":0.25,"max":0.75}}"#;
        let product: FactorioProduct = serde_json::from_str(json).expect("parses");
        assert_eq!(*product.probability, r64(0.25));

        let full_window = r#"{"name":"uranium-238","product_type":"item","amount":1,
            "independent_probability":0.993,"shared_probability":{"min":0.0,"max":1.0}}"#;
        let product: FactorioProduct = serde_json::from_str(full_window).expect("parses");
        assert_eq!(*product.probability, r64(0.993));
    }

    /// `amount` is optional in 2.1: randomised outputs report a range instead.
    #[test]
    fn a_randomised_product_amount_is_the_midpoint_of_its_range() {
        let json = r#"{"name":"raw-fish","product_type":"item","amount_min":1,"amount_max":5}"#;
        let product: FactorioProduct = serde_json::from_str(json).expect("parses");
        assert_eq!(product.amount, 3);
        assert_eq!(*product.probability, r64(1.0));
    }

    /// `Ingredient.amount` is a `double` in the runtime API.
    #[test]
    fn a_fractional_ingredient_amount_rounds_to_whole_units() {
        let json = r#"{"name":"water","ingredient_type":"fluid","amount":49.5}"#;
        let ingredient: FactorioIngredient = serde_json::from_str(json).expect("parses");
        assert_eq!(ingredient.amount, 50);
    }

    /// The whole recipe as the live 2.1.17 server sends it once
    /// `mods/BotBridge` fills in `category` from `LuaRecipe.categories`.
    /// Reconstructed from the live panic: without the `category` key the
    /// product object ends at column 250, exactly where serde_json reported
    /// the missing `probability`.
    #[test]
    fn the_live_2_1_recipe_parses() {
        let json = r#"{"name":"wooden-chest","valid":true,"enabled":true,"category":"crafting","hidden":false,"energy":0.5,"order":"a[items]-a[wooden-chest]","ingredients":[{"name":"wood","ingredient_type":"item","amount":2}],"products":[{"name":"wooden-chest","product_type":"item","amount":1}],"group":"logistics","subgroup":"storage"}"#;
        let recipe: FactorioRecipe = serde_json::from_str(json).expect("parses");
        assert_eq!(recipe.category, "crafting");
        assert_eq!(recipe.ingredients.expect("has ingredients")[0].amount, 2);
        assert_eq!(recipe.products[0].amount, 1);
        assert_eq!(*recipe.products[0].probability, r64(1.0));
    }

    /// `category` decides whether the planner smelts or crafts an item
    /// (`crates/planner/src/method/have.rs`), so a payload without it must
    /// fail loudly instead of quietly planning with an empty category.
    #[test]
    fn a_recipe_without_a_category_is_rejected() {
        let json = r#"{"name":"wooden-chest","valid":true,"enabled":true,"hidden":false,"energy":0.5,"order":"a[items]-a[wooden-chest]","ingredients":[],"products":[{"name":"wooden-chest","product_type":"item","amount":1}],"group":"logistics","subgroup":"storage"}"#;
        let err = serde_json::from_str::<FactorioRecipe>(json).expect_err("must not parse");
        assert!(
            err.to_string().contains("category"),
            "expected a complaint about category, got: {err}"
        );
    }
}
