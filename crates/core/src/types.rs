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
use typescript_definitions::TypeScriptify;

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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
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
    pub energy: Box<R64>,
    pub order: String,
    pub group: String,
    pub subgroup: String,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
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
#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case", from = "RawFactorioProduct")]
pub struct FactorioProduct {
    pub name: String,
    #[serde(default)]
    pub product_type: String,
    pub amount: u32,
    pub probability: Box<R64>,
}

/// The `Ingredient` shape as it arrives from `mods/BotBridge`.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestEntity {
    pub name: String,
    pub position: Position,
}

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
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

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub fn distance(&self, other: &Position) -> f64 {
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
    pub fn distance(&self, other: &Pos) -> u32 {
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
pub enum Direction {
    #[default]
    North = 0,
    NorthEast = 1,
    East = 2,
    SouthEast = 3,
    South = 4,
    SouthWest = 5,
    West = 6,
    NorthWest = 7,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AreaFilter {
    Rect(Rect),
    PositionRadius((Position, Option<f64>)),
}

impl Direction {
    pub fn all() -> Vec<Direction> {
        (0..8).map(|n| Direction::from_u8(n).unwrap()).collect()
    }
    pub fn orthogonal() -> Vec<Direction> {
        (0..8)
            .filter(|n| n % 2 == 0)
            .map(|n| Direction::from_u8(n).unwrap())
            .collect()
    }
    pub fn opposite(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 4) % 8).unwrap()
    }
    pub fn clockwise(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 2) % 8).unwrap()
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

    pub fn turn(&self, direction: Direction) -> Position {
        match direction {
            Direction::North => self.clone(),
            Direction::East => self
                .rotate_clockwise()
                .rotate_clockwise()
                .rotate_clockwise(),
            Direction::South => self.rotate_clockwise().rotate_clockwise(),
            Direction::West => self.rotate_clockwise(),
            _ => panic!("diagonal turning not supported"),
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
    Debug,
    Clone,
    Default,
    PartialEq,
    TypeScriptify,
    Serialize,
    Deserialize,
    JsonSchema,
    utoipa::ToSchema,
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

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTile {
    pub name: String,
    pub player_collidable: bool,
    pub position: Position,
    pub color: Option<[u8; 4]>,
}

#[derive(Debug, Clone, Default, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FactorioChunk {
    pub entities: Vec<FactorioEntity>,
    // pub tiles: Vec<FactorioTile>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkResource {
    pub name: String,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
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
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioForce {
    pub name: String,
    pub force_id: u32,
    // The current technology in research, or None if no research is currently ongoing.
    pub current_research: Option<String>,
    // Progress of current research, as a number in range [0, 1].
    pub research_progress: Option<Box<R64>>,
    pub technologies: Box<BTreeMap<String, FactorioTechnology>>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioGraphic {
    pub entity_name: String,
    pub image_path: String,
    pub width: u32,
    pub height: u32, // FIXME: add whatever this is, width&height are the first
                     // 1:1:0:0:0:0:1

                     //picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..yy..":"..scale
}

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
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
//     TypeScriptify,
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
//     TypeScriptify,
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

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioFluidBoxConnection {
    pub max_underground_distance: Option<u32>,
    pub connection_type: Option<String>,
    pub positions: Vec<Position>,
}
#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
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
}

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    TypeScriptify,
    Serialize,
    Deserialize,
    JsonSchema,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioEntity {
    pub name: String,
    pub entity_type: String,
    pub position: Position,
    pub bounding_box: Rect,
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
    pub fn from_blueprint_entity(
        entity: Entity,
        prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    ) -> Result<Self> {
        let position: Position = entity.position.into();
        let direction: Option<Direction> = entity
            .direction
            .map(|d| Direction::from_u8(d % 8).expect("should always work"));
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
        if let Some(prototype) = prototypes.get(name) {
            Ok(FactorioEntity {
                bounding_box: add_to_rect_turned(&prototype.collision_box, &position, direction),
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                entity_type: prototype.entity_type.clone(),
                pickup_position,
                drop_position,
                ..Default::default()
            })
        } else {
            Ok(FactorioEntity {
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                pickup_position,
                drop_position,
                ..Default::default()
            })
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
            drop_position: Some(position.add(&Position::new(0., 1.).turn(direction))),
            pickup_position: Some(position.add(&Position::new(0., -1.).turn(direction))),
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
            drop_position: Some(position.add(&Position::new(-0.5, -1.296875).turn(direction))),
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
            drop_position: Some(position.add(&Position::new(0., -2.).turn(direction))),
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
    Debug,
    Clone,
    PartialEq,
    TypeScriptify,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    JsonSchema,
    utoipa::ToSchema,
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
    Debug,
    Clone,
    PartialEq,
    TypeScriptify,
    Serialize,
    Deserialize,
    JsonSchema,
    utoipa::ToSchema,
    Hash,
    Eq,
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioResult {
    pub success: bool,
    pub output: Vec<String>,
}

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct PlaceEntityResult {
    pub player: FactorioPlayer,
    pub entity: FactorioEntity,
}

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
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
//
// Deliberately a `//` comment, not `///`: `TypeScriptify` mirrors doc comments
// into the generated `app/src/models/types.ts`, and this rationale is about
// Rust trait derives, which mean nothing there.
#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedPositionEvent {
    pub player_id: PlayerId,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
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

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerLeftEvent {
    pub player_id: PlayerId,
}

#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq, utoipa::ToSchema,
)]
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
