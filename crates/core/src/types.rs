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

/// Which surface something is on, identified by **name**.
///
/// Space Age is enabled in this workspace, so five planets and any number of
/// orbital platforms are one rocket away. Every run measured here so far has
/// been a Space Age run that happened never to leave Nauvis, and the only
/// thing that kept it that way is one `if` in `on_chunk_generated`
/// (`mods/BotBridge/control.lua`) that drops a chunk from any other surface.
/// See `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.
///
/// **The name, not the index**, verified against
/// `workspace/factorio-api-docs/runtime-api.json` (`application_version`
/// 2.1.17): `LuaSurface.name` is *"unique among surfaces"*, while
/// `LuaSurface.index` *"is assigned when a surface is created, and remains so
/// until it is deleted. **Indexes of deleted surfaces can be reused.**"* An
/// index is therefore an identity only within one save and only until a
/// deletion; a record somebody reads next year needs the name.
///
/// **This does not go on [`Position`], deliberately.** A coordinate is only
/// ever comparable within one surface, which `Position { x, y }` already says
/// correctly; putting a surface on it would force `p1 - p2`, `Sub`,
/// `manhattan_distance` and `PartialEq` to answer "what is the distance
/// between two planets?", whose honest answer -- undefined -- cannot be
/// returned as an `f64`. The surface belongs on the **container**, which is
/// rung one's successor, not on the value.
#[derive(
    Clone,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    JsonSchema,
    utoipa::ToSchema,
)]
// `transparent` is explicit rather than necessary: serde already renders a
// newtype struct as its inner value (verified, same standalone experiment as
// the note on `FactorioEntity::surface`). It is stated so that the wire shape
// -- a bare `"nauvis"`, which is what the mod emits -- is a declared contract
// rather than an emergent one, and so that adding a second field to this type
// fails loudly instead of silently reshaping every archived payload.
#[serde(transparent)]
pub struct SurfaceId(pub String);

impl SurfaceId {
    /// The starting planet, and the only surface anything in this project has
    /// ever observed.
    ///
    /// This is a *constructor*, not an assumption: use it where the surface is
    /// genuinely known to be Nauvis, never to fill in a surface the mod did
    /// not report. Absence on the wire is modelled as `None`, because a record
    /// that confidently says "nauvis" about a line whose surface was never
    /// transmitted is worse than one that says nothing -- it answers a
    /// question nobody asked and the reader cannot tell.
    pub fn nauvis() -> Self {
        SurfaceId(String::from("nauvis"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SurfaceId {
    /// Nauvis. Every world this project builds is a Nauvis world today, and a
    /// `Default` that had to be spelled out at 1,400 construction sites would
    /// buy nothing.
    fn default() -> Self {
        SurfaceId::nauvis()
    }
}

impl std::fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SurfaceId {
    fn from(name: String) -> Self {
        SurfaceId(name)
    }
}

impl From<&str> for SurfaceId {
    fn from(name: &str) -> Self {
        SurfaceId(name.to_owned())
    }
}

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
    /// Which surface this player's character stands on, from
    /// `LuaPlayer.surface.name`. `None` means the mod did not say -- see
    /// [`FactorioEntity::surface`].
    ///
    /// A bot that dies on another surface respawns on Nauvis today
    /// (`create_bot_character(game.surfaces[1], ...)`), and nothing in the
    /// planner can express a bot being anywhere but "the" surface. This field
    /// is what makes that observable rather than invisible.
    #[serde(default)]
    pub surface: Option<SurfaceId>,
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
            // Not `SurfaceId::nauvis()`: a synthesised roster has no observed
            // surface, and saying "nauvis" here would be the default asserting
            // something nobody measured.
            surface: None,
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
    /// What the machine has been given and has not turned into anything yet --
    /// a furnace's ore, an assembler's ingredients, a lab's science.
    ///
    /// # `None` is not empty, and neither is a count of zero
    ///
    /// `None` means the entity has no input inventory at all (a
    /// `wooden-chest`, a belt) **or** the reply came from a mod build that
    /// predates the field -- `serde(default)` is what makes the second of
    /// those possible, and the archived `inventory_contents_at` payload in
    /// `crates/core/tests/live_2_1_payloads.rs` is exactly such a reply.
    /// `Some(empty)` means the entity has one and it is standing empty, which
    /// is a real observation. The mod omits the key rather than sending an
    /// empty table for the first case; see `rcon_inventory_contents_at` in
    /// `mods/BotBridge/control.lua`.
    ///
    /// # Never a buffer
    ///
    /// Nothing withdraws from here. `crates/planner`'s `withdraw_slot` maps a
    /// furnace to its *result* slot, which is what an `ActionKind::Remove` can
    /// address, so ore sitting in an input slot is never counted as material
    /// the plan may spend. It is a reading about whether a machine is busy,
    /// on the same terms as the fuel slot beside it.
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::boxed_option_vec_or_empty_map"
    )]
    pub input_inventory: Box<Option<Vec<InventoryItemWithQuality>>>,
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
    /// Which surface this tile belongs to, from `LuaTile.surface.name`
    /// (verified present on `LuaTile` in `runtime-api.json` 2.1.17). `None`
    /// means the mod did not say -- see [`FactorioEntity::surface`].
    ///
    /// `rcon_find_tiles_filtered` asks `game.surfaces[1]` unconditionally
    /// today, so a shoreline found for a boiler is a *Nauvis* shoreline
    /// whatever the bot asking is standing on. Carrying the answer's surface
    /// is what will let a later reader notice that.
    #[serde(default)]
    pub surface: Option<SurfaceId>,
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
/// Enumerated from the live game on 2026-09-05 (Space Age 2.1.17, all 32 --
/// `docs/superpowers/notes/2026-09-05-research-triggers.md`), the runtime
/// shapes are: `craft-item` `{item = {name}, count}`, `mine-entity`
/// `{entities = {...}}`, `build-entity` `{entity = {name}}` (singular -- the
/// one shipped use is `space-science-pack`, an asteroid collector),
/// `capture-spawner` `{}` and `create-space-platform` `{}`. No shipped
/// technology uses `craft-fluid` or `send-item-to-orbit`; their fields follow
/// the runtime API definition.
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
    /// Build any one of `entities`. Shipped once: `space-science-pack`, an
    /// `asteroid-collector`, spelled `entity = {name}` at runtime.
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
            ResearchTrigger::CaptureSpawner { entity: None } => write!(f, "capture a spawner"),
            ResearchTrigger::CreateSpacePlatform => write!(f, "create a space platform"),
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
    /// Every effect the technology's prototype declares, `unlock-recipe`
    /// included.
    ///
    /// [`Self::unlocked_recipes`] above is the same data narrowed to the one
    /// effect kind that had a reader. This is the rest of it, and the reason
    /// it exists is that **a rate is `prototype x force bonus x module
    /// effect` and only the first factor was ever carried**. The mod kept
    /// `unlock-recipe` and threw the other 50 modifier kinds away, so nothing
    /// downstream could know that vanilla's `steel-axe` grants
    /// `character-mining-speed +1` and halves the cost of every hand mine
    /// scheduled after it. That was not a planner oversight — the planner had
    /// never been told.
    ///
    /// Both keys are sent. The duplication is deliberate: every archived
    /// payload and every existing consumer reads `unlocked_recipes`, and a
    /// seam that does not move is worth more than the handful of bytes.
    ///
    /// `#[serde(default)]` with `vec_or_empty_map` for the same two reasons
    /// its neighbours have it: 865 MB world dumps written before this field
    /// existed must still load, and the mod's `table_to_json` renders an
    /// empty Lua table as `{}` rather than `[]`.
    #[serde(default, deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub effects: Vec<FactorioTechnologyEffect>,
}

/// One entry of a technology's `effects` list — a `TechnologyModifier`, in
/// the runtime API's terms — flattened to three fields.
///
/// # Why flat, and not 51 variants
///
/// `TechnologyModifier` in `workspace/factorio-api-docs/runtime-api.json`
/// (2.1.17) is a table tagged by `type`, with **51 variant parameter groups**,
/// of which 44 carry exactly one field, `modifier`. [`ResearchTrigger`] next
/// door is a tagged enum because it has eight variants with genuinely
/// different shapes and each one drives a different planner decision. This has
/// neither property. Mirroring 51 variants would turn every new Factorio
/// version and every mod that adds a modifier type into a shape this build
/// cannot express — the exact failure carrying the data is meant to end. The
/// owner's standing rule applies directly: derive rates from the game's own
/// data *because that is what survives mods*.
///
/// So `kind` is the `type` string verbatim, and a reader matches on the
/// strings it knows and ignores the rest. An unknown kind is data, not an
/// error.
// No `JsonSchema` / `ToSchema`, matching `FactorioTechnology` which owns it:
// neither is published through the OpenAPI seam, and deriving them here would
// add a schema nothing hands out — which `documented_type_schemas()` fails the
// build over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTechnologyEffect {
    /// The `ModifierType` string as the game spells it — `"unlock-recipe"`,
    /// `"character-mining-speed"`, `"laboratory-speed"`, and 48 others in
    /// 2.1.17 base. Kebab-case, because that is what Factorio sends; it is
    /// not renamed on the way through.
    pub kind: String,
    /// The number the effect adds, where the variant has one.
    ///
    /// 44 of the 51 variants spell it `modifier`. `change-recipe-productivity`
    /// spells it `change` and `give-item` spells it `count`; the mod
    /// normalises both to this field. The seven boolean variants (e.g.
    /// `mining-with-fluid`) arrive as `1` or `0` rather than being dropped, so
    /// "this technology enables it" survives as a number a reader can test.
    /// `None` is a variant with no number at all — `unlock-recipe`,
    /// `unlock-quality`, `nothing`.
    ///
    /// `R64` rather than `f64` because [`FactorioTechnology`] derives `Hash`
    /// and `Eq`, the same reason `FactorioRecipe::energy` is one.
    #[serde(default)]
    pub modifier: Option<Box<R64>>,
    /// What the effect acts on, where the variant names one: the `recipe` of
    /// an `unlock-recipe` or a `change-recipe-productivity`, the
    /// `ammo_category` of a `gun-speed`, the `turret_id` of a
    /// `turret-attack`, the `item` of a `give-item`, the `quality` of an
    /// `unlock-quality`, the `space_location` of an `unlock-space-location`.
    ///
    /// One flat field rather than six because exactly one of them is present
    /// per variant and `kind` already says which. `None` for the 44 variants
    /// that target nothing but the force itself.
    #[serde(default)]
    pub target: Option<String>,
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
    /// The other force bonuses that scale a **rate**.
    ///
    /// `manual_mining_speed_modifier` above was the only one of these the mod
    /// ever sent, so `FactorioForce` had six fields and one rate term. Every
    /// other rate in the system was a prototype constant treated as
    /// permanent, which is a mod-compatibility defect as much as a modelling
    /// one: a mod that grants crafting speed through research is invisible to
    /// a model that only ever reads the prototype.
    ///
    /// **Carried, not yet read.** Today only mining speed reaches a duration
    /// ([`factorio_bot_planner::method::util::character_mining_speed`]);
    /// `research_ticks_in_labs` has no speed term at all, no inserter or belt
    /// throughput is costed, and no drill rate is. Sending the data does not
    /// fix any of those. It removes the reason none of them *could* be fixed.
    ///
    /// Every name is `LuaForce`'s own, checked against
    /// `workspace/factorio-api-docs/runtime-api.json` at 2.1.17 — all
    /// non-optional read attributes there, `double` except the two `uint32`
    /// ones noted below. `Option` + `#[serde(default)]` throughout, so that
    /// dumps written before this field existed still load; `None` means "the
    /// world did not report one", which a reader treats as the documented
    /// default of `0`, exactly as `manual_mining_speed_modifier` does.
    ///
    /// `R64` for the same reason its neighbours are: this type derives `Hash`
    /// and `Eq`.
    #[serde(default)]
    pub manual_crafting_speed_modifier: Option<Box<R64>>,
    /// `LuaForce::character_running_speed_modifier`. Walking is ~21% of a
    /// reference run and is charged at one fixed tiles-per-tick
    /// (`schedule::WALK_TILES_PER_TICK`), which this would scale. See
    /// [`Self::manual_crafting_speed_modifier`] for the shared rationale.
    #[serde(default)]
    pub character_running_speed_modifier: Option<Box<R64>>,
    /// `LuaForce::laboratory_speed_modifier`. Six base technologies grant it
    /// (`research-speed-1..6`).
    #[serde(default)]
    pub laboratory_speed_modifier: Option<Box<R64>>,
    /// `LuaForce::laboratory_productivity_bonus`. No base technology grants
    /// it in 2.1.17; carried because a mod can.
    #[serde(default)]
    pub laboratory_productivity_bonus: Option<Box<R64>>,
    /// `LuaForce::mining_drill_productivity_bonus`. Extra output per
    /// operation rather than a faster swing — a smaller bill, not a shorter
    /// duration. Four base technologies, the fourth infinite.
    #[serde(default)]
    pub mining_drill_productivity_bonus: Option<Box<R64>>,
    /// `LuaForce::inserter_stack_size_bonus`. Two base technologies.
    #[serde(default)]
    pub inserter_stack_size_bonus: Option<Box<R64>>,
    /// `LuaForce::bulk_inserter_capacity_bonus`. Eight base technologies take
    /// it to 12. A `uint32` in the runtime API; kept as `R64` so that the
    /// whole group has one shape and a mod reporting a fraction cannot make
    /// the world unreadable.
    #[serde(default)]
    pub bulk_inserter_capacity_bonus: Option<Box<R64>>,
    /// `LuaForce::belt_stack_size_bonus`. Also a `uint32`; see
    /// [`Self::bulk_inserter_capacity_bonus`].
    #[serde(default)]
    pub belt_stack_size_bonus: Option<Box<R64>>,
    /// `LuaForce::worker_robots_speed_modifier`. Six base technologies. No
    /// bot in this project is a construction robot, so this is carried for
    /// completeness of the rate group rather than for a reader.
    #[serde(default)]
    pub worker_robots_speed_modifier: Option<Box<R64>>,
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
    // HOW MUCH THIS BOX HOLDS, in fluid units, at normal quality.
    //
    // `pipe_connections` and `production_type` say where a fluid box may be
    // joined and which way fluid flows through it; neither says what it can
    // store. A caller planning storage without this has one option left --
    // a hard-coded table of vanilla capacities -- which is the same
    // mod-compatibility defect as `pole_supply_half_extent` and the copied
    // smelting rate the world-record base falsified.
    //
    // `None` means the sender did not say: an older mod, an archived dump, or
    // a Factorio whose `LuaFluidBoxPrototype` has no `get_volume()`. It is a
    // METHOD there and not an attribute, so reading it wrong yields silence
    // rather than an error -- see the read in `mods/BotBridge/types.lua`.
    #[schemars(
        description = "How much this fluid box holds, in fluid units, at normal quality (`LuaFluidBoxPrototype::get_volume()`). `null` means the sender did not say -- an older mod or an archived record -- never zero."
    )]
    #[serde(default)]
    pub volume: Option<f64>,
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
    /// How far a mining drill reaches **beyond the tile it stands on**, which
    /// [`Self::collision_box`] cannot say. Measured on a live 2.1.17 game:
    ///
    /// | entity | footprint | radius | tiles worked |
    /// |---|---|---|---|
    /// | `burner-mining-drill` | 2x2 | 0.99 | **2x2**, its own footprint |
    /// | `electric-mining-drill` | 3x3 | 2.49 | **5x5**, a ring beyond it |
    /// | `pumpjack` | 3x3 | 0.49 | **1x1**, a single tile |
    ///
    /// A **pumpjack is a `mining-drill` too**, and the tightest-reaching one:
    /// it must be centred on the crude-oil well itself, and its 3x3 box
    /// overstates its reach by eight tiles.
    ///
    /// # Compare in TILES, never in collision-box extents
    ///
    /// `radius > collision_box_half_width` is wrong for **every** drill in the
    /// game. A burner drill's 0.99 exceeds its half-width of 0.699, so that
    /// test says it reaches beyond itself -- it does not: 2 x 0.99 = 1.98,
    /// which is the 2x2 its box already occupies. Factorio sizes a 2x2 box
    /// slightly under 2 so neighbours do not touch, and comparing a radius
    /// against that shaved number measures the shaving. Ceil both sides to
    /// tiles first. This was caught by a test written the wrong way round; see
    /// `crates/core/tests/mining_drill_radius.rs`.
    ///
    /// # The field has two names
    ///
    /// `resource_searching_radius` in the prototype definitions,
    /// `mining_drill_radius` on the runtime API. Grepping the data files for
    /// the runtime name finds nothing, which looks exactly like the field not
    /// existing. Both sources were read: `entity/mining-drill.lua` lines 1757,
    /// 1853 and 1900, and a live 2.1.17 RCON probe, agreeing.
    ///
    /// Without this the planner cannot express "a drill mines a tile it does
    /// not stand on", and that one gap made two unrelated behaviours
    /// needlessly conservative: ore-aware siting asked whether ore lay under a
    /// drill's own 3x3 because that was all the model offered, and the
    /// will-not-bury placement rule could not distinguish a belt over ore a
    /// drill can still reach from a belt over ore nobody can mine. With the
    /// radius, burying the outer ring under an electric drill is **not a cost
    /// at all**; under a burner drill it genuinely is.
    ///
    /// `default`, so a dump or snapshot written before this field existed
    /// still loads, with `None` meaning *unknown reach* -- never zero reach.
    #[serde(default)]
    pub mining_drill_radius: Option<f64>,
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
    /// The recipe categories this prototype's machine runs. Sorted by the mod
    /// so the order is the data's, not `pairs()`'s. Read live off a Space Age
    /// 2.1.17 game, 18 of 1028 prototypes report any:
    ///
    /// ```text
    /// stone-furnace         furnace              smelting
    /// assembling-machine-1  assembling-machine   advanced-crafting, crafting, parameters
    /// oil-refinery          assembling-machine   oil-processing, parameters
    /// chemical-plant        assembling-machine   chemistry, parameters
    /// centrifuge            assembling-machine   centrifuging, parameters
    /// ```
    ///
    /// **`parameters` is not a category anybody makes anything in** -- it is
    /// Factorio 2.0's blueprint-parameter pseudo-category, and it sits on four
    /// of those five. A reader must not take a `parameters` match as evidence
    /// that a machine runs a recipe. (The chemical plant's real category is
    /// spelled `chemistry`, not `chemical`.)
    ///
    /// # This is the field that says *what* a machine crafts
    ///
    /// [`Self::crafting_speed`] already says a prototype crafts. Nothing said
    /// what, and it could not be derived: measured over
    /// `crates/core/tests/live-2.1.17-world-snapshot.json`, twelve prototypes
    /// there declare a crafting speed, and `oil-refinery`, `chemical-plant`,
    /// `centrifuge`, `electromagnetic-plant` and all three assembling machines
    /// are one and the same `entity_type`, `assembling-machine`. So a reader
    /// holding a recipe's category had no field, and no combination of fields,
    /// from which the machine that runs it follows.
    ///
    /// It is the crafting half of the rule
    /// [`Self::resource_categories`] already carries for mining: a recipe
    /// carries a category, a machine carries the categories it supports, and
    /// the machine runs the recipe iff the former is in the latter.
    ///
    /// # `None` is not `Some(vec![])`
    ///
    /// `None` is *the sender did not say* -- every world dumped or snapshotted
    /// before 2026-09-07, and every prototype that is not a crafting machine,
    /// for which the optional attribute reads nil. `Some(vec![])` is *the game
    /// says this machine runs no category*. A reader that folded the two
    /// together would answer "no machine runs `oil-processing`" for an archive
    /// that never carried the question.
    ///
    /// `deserialize_with`, because the mod sends an empty set as the JSON
    /// object `{}` -- `helpers.table_to_json` renders an empty Lua table that
    /// way -- and a bare `Option<Vec<String>>` would reject it.
    ///
    /// # It is an attribute
    ///
    /// `LuaEntityPrototype.crafting_categories` is an attribute in 2.1.17,
    /// checked in `workspace/client1/doc-html/runtime-api.json`, unlike
    /// `get_crafting_speed()`, `get_supply_area_distance()` and
    /// `get_max_wire_distance()`, which are methods with no attribute at all.
    /// That distinction is not cosmetic: reading a method as an attribute
    /// raises, the mod's `pcall` swallows the error, and the field arrives
    /// `None` for every prototype in the game with nothing anywhere saying it
    /// should not have -- which is how [`Self::crafting_speed`] was nil for
    /// 1028 live prototypes.
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub crafting_categories: Option<Vec<String>>,
    /// `mineable_properties.required_fluid`: the fluid that must be piped in
    /// to mine this at all. Uranium ore needs sulfuric acid, and a character
    /// has no pipe.
    #[serde(default)]
    pub mining_fluid: Option<String>,
    /// Half the side of the square this **electric pole or beacon** supplies,
    /// so `2.5` for a `small-electric-pole`'s 5x5. `LuaEntityPrototype`'s
    /// subclasses for it are exactly `ElectricPole` and `Beacon`.
    ///
    /// **`None` is "no supply area", never zero.** A furnace has none; a
    /// beacon with a zero one would be `Some(0.0)`, and a caller that wants to
    /// know whether beacon ground is worth reserving must be able to tell
    /// those apart. Every dump and snapshot written before this field existed
    /// also reads `None`, i.e. "the sender did not say".
    ///
    /// # It is a method on the runtime API, and that is not a detail
    ///
    /// There is no `supply_area_distance` **attribute** in 2.1.17 --
    /// `get_supply_area_distance(quality)` is the only way to it. Reading the
    /// attribute raises, the mod's `pcall` swallows it, and the field arrives
    /// `None` with nothing anywhere saying it should not have. That is exactly
    /// how [`Self::crafting_speed`] arrived nil for all 1028 prototypes of a
    /// live game while `mining_speed`, still an attribute, arrived fine.
    ///
    /// # What it is for
    ///
    /// `crates/planner/src/method/power.rs` hard-codes
    /// `pole_supply_half_extent` as a table of vanilla pole names, and says in
    /// its own doc that sending this field is the follow-up that deletes it.
    /// **Nothing reads this yet** -- carrying the datum and consuming it are
    /// two changes, and the second one moves plans.
    #[serde(default)]
    pub supply_area_distance: Option<f64>,
    /// How far this entity can throw a wire, in tiles — `7.5` for a
    /// `small-electric-pole`, `32` for a `big-electric-pole`.
    ///
    /// Independent of [`Self::supply_area_distance`], which is what a pole
    /// *covers*: a big pole covers 4x4 and spans 32 tiles, a `substation`
    /// covers 18x18 and spans 18. The two are not orderable against each other
    /// and neither can be derived from the other.
    ///
    /// # This is one pole's reach, never a verdict about a pair
    ///
    /// **The game wires two poles when their centres are within the SMALLER of
    /// the two distances**, so a reader holding one of these numbers holds an
    /// input to a pairwise minimum. `method::power`'s `POLE_WIRE_REACH_TILES`
    /// is what happens when that is forgotten: a single 7.5 — a *small* pole's
    /// reach — applied to every pole in a blueprint, manufacturing
    /// `disconnected_poles` refusals for blocks whose poles really are wired.
    ///
    /// # It is the maximum over EVERY wire kind, so it is not "is this a pole"
    ///
    /// Measured over all 1,028 prototypes of a live 2.1.17 game (seed 31337,
    /// 2026-09-07): **4 report a pole's copper span** — 7.5, 9, 32, 18,
    /// matching the data stage exactly — **94 more report a CIRCUIT wire
    /// distance** (`stone-furnace` 9, `wooden-chest` 9, `power-switch` 10,
    /// `agricultural-tower` 30), and 930 report 0. So a caller that read a
    /// positive number here as "this is a pole" would find 98 poles in
    /// vanilla, and would wire a network through an assembling machine.
    /// `crates/planner/src/state.rs`'s `pole_wire_reach` gates on
    /// `entity_type == "electric-pole"` before believing it; this field is
    /// what the game says, not a classification.
    ///
    /// # `Some(0.0)` and `None` are different answers
    ///
    /// `get_max_wire_distance()` answers **0** for an entity nothing connects
    /// to — a tree, an explosion, a corpse, and also a `steam-engine` — and
    /// the mod sends that zero rather than dropping it, so `Some(0.0)` is the
    /// game speaking. `None` is *the sender did not say*: every dump and
    /// snapshot written before 2026-09-07, `workspace/scripts/map.json`
    /// included. Only the second falls back to
    /// `crates/planner/src/state.rs`'s `vanilla_pole_wire_reach`, and merging
    /// the two would either blind the planner on every archived world or
    /// silently re-credit a wireless entity with a vanilla pole's span.
    ///
    /// # It is a method on the runtime API, and the attribute spelling exists
    ///
    /// `maximum_wire_distance` is the **data-stage** name — it is what
    /// `base/prototypes/entity/entities.lua` writes and what a reader would
    /// try first — and there is no such attribute on `LuaEntityPrototype` in
    /// 2.1.17. `get_max_wire_distance(quality)` is the only way to it.
    /// Reading the attribute raises, the mod's `pcall` swallows it, and the
    /// field arrives `None` for every prototype in the game with nothing
    /// saying it should not have; see [`Self::crafting_speed`], which did
    /// exactly that for all 1028 prototypes of a live game.
    #[serde(default)]
    pub maximum_wire_distance: Option<f64>,
    /// A **beacon's** `distribution_effectivity`: the fraction of a module's
    /// effect a receiver in range actually gets. `None` for anything that is
    /// not a beacon.
    ///
    /// Not sufficient on its own -- see [`Self::beacon_profile`], which scales
    /// it by how many beacons reach the same machine.
    #[serde(default)]
    pub distribution_effectivity: Option<f64>,
    /// The beacon's `profile`: an extra multiplier applied to what a receiver
    /// gets, **indexed by how many beacons reach that receiver**. Factorio 2.0
    /// added it, and it is the reason beacon effect is not a single scalar:
    /// the second beacon on a machine is worth a different amount from the
    /// first.
    ///
    /// Kept under a name that says what it profiles. `LuaEntityPrototype`
    /// calls it `profile`, which on a struct describing every prototype in the
    /// game says nothing; the mod emits `beacon_profile` to match, and
    /// `a_serialised_beacon_prototype_carries_its_geometry` pins the pairing,
    /// because serde drops an unrecognised key in silence and this repo has
    /// paid for that twice.
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub beacon_profile: Option<Vec<f64>>,
    /// What one of these draws from an **electric** network while running, in
    /// **joules per tick** — the game's own unit, unconverted. Use
    /// [`Self::energy_usage_kw`] rather than scaling it at a call site.
    ///
    /// # The gate is the field, not a caller's responsibility
    ///
    /// The mod sends this **only when the prototype has an electric energy
    /// source**. A `stone-furnace` has an `energy_usage` of 90 kW *of coal*,
    /// and a burner machine charged against an electric budget is a number in
    /// the wrong units that every test would agree with. That is why
    /// `crates/planner/src/state.rs`'s `consumer_kw` leaves burner machines
    /// out rather than zeroing them, and the gate lives upstream where the
    /// energy source is visible.
    ///
    /// # An inserter is electric and still has no figure here
    ///
    /// Measured on all 1,028 prototypes of a live 2.1.17 game: **28 carry this
    /// field and not one of them reports 0.** An `inserter` has an electric
    /// energy source and passes the gate, but `energy_usage` is an *optional*
    /// attribute and is simply absent on it — its cost is
    /// `energy_per_movement` and `energy_per_rotation`, per swing, not a
    /// standing draw. What a budget wants is what a *busy* one costs, which is
    /// a duty cycle rather than a prototype field, so `state.rs` keeps its
    /// `INSERTER_DUTY_KW` and reaches it through the ordinary absent-field
    /// fallback.
    ///
    /// `state.rs` treats a zero the same way for safety, but that path has
    /// never fired in vanilla and the reason to expect absence rather than
    /// zero is stated here so nobody re-derives it from the guard.
    ///
    /// `default`, so every dump and snapshot written before this field
    /// existed reads `None` — *the sender did not say*, never zero draw.
    #[serde(default)]
    pub electric_energy_usage: Option<f64>,
    /// What one of these contributes to an electric network at full output, in
    /// **joules per tick**. [`Self::max_energy_production_kw`] converts it.
    ///
    /// `LuaEntityPrototype::get_max_energy_production()`, a **method** in
    /// 2.1.17 — there is no attribute of that name, and reading one raises.
    ///
    /// # It is the only way to a generator's output, because the inputs are
    /// not exposed
    ///
    /// A `steam-engine` carries no output figure at the data stage either: its
    /// 900 kW is `fluid_usage_per_tick * 60 * heat_capacity *
    /// (maximum_temperature - default_temperature) * effectivity`. Of those,
    /// `LuaEntityPrototype` exposes `maximum_temperature` and `effectivity`
    /// and **not** `fluid_usage_per_tick` -- doclint-allow: a Factorio
    /// data-stage name, and its ABSENCE from this tree is exactly the claim.
    /// So the physics cannot be reassembled downstream: the runtime does it
    /// and hands over the answer.
    ///
    /// # Nameplate, and for a solar panel that is a trap
    ///
    /// This is maximum production, so a `solar-panel` reports its **noon**
    /// figure and an `accumulator` reports its discharge limit. Neither is
    /// what a day averages, and neither is derivable from any prototype: the
    /// day/night curve lives on `LuaSurface` and arrives as
    /// [`SurfaceDaylight`], whose [`SurfaceDaylight::average_solar_fraction`]
    /// turns this figure into the average and whose
    /// [`SurfaceDaylight::night_deficit_fraction`] sizes the accumulators.
    /// `state.rs` still credits only deterministic sources in
    /// `electric_supply_kw` and exposes the solar answers separately.
    ///
    /// `default`: `None` on every world written before this field, meaning
    /// *unknown*, never "produces nothing".
    #[serde(default)]
    pub max_energy_production: Option<f64>,
    /// The **fraction of [`Self::max_energy_production`] a solar panel makes
    /// while the sun is fully up**, from `solar_panel_performance_at_day`.
    ///
    /// One endpoint of the daylight curve. `1.0` in vanilla, so it looks
    /// redundant and is not: it is the scale a mod moves, and it is also the
    /// field whose *presence* says "this prototype is a solar panel". The
    /// attribute carries `subclasses: ["SolarPanel"]` in 2.1.17, so reading it
    /// on anything else raises and the mod's `pcall` drops it.
    ///
    /// The other terms of the average — how much of a day is spent up there —
    /// are on `LuaSurface` and arrive as [`SurfaceDaylight`], never on a
    /// prototype. `default`: `None` is *the sender did not say*, never "makes
    /// nothing by day".
    #[serde(default)]
    pub solar_panel_performance_at_day: Option<f64>,
    /// The other endpoint, from `solar_panel_performance_at_night`: the
    /// fraction of [`Self::max_energy_production`] a solar panel makes at
    /// midnight. `0.0` in vanilla.
    ///
    /// **A zero here is a real answer and must not be read as absence.** That
    /// is the whole reason this is `Option<f64>` and not `f64` — a vanilla
    /// panel genuinely produces nothing at night, and a modded one that
    /// produces a little is a different world, not a better-populated one.
    #[serde(default)]
    pub solar_panel_performance_at_night: Option<f64>,
    /// How many **joules** this entity's own electric buffer holds, from
    /// `LuaElectricEnergySourcePrototype::buffer_capacity`.
    ///
    /// **Reached through a sub-prototype, which is why solar sizing was
    /// previously blocked at the panel.** `electric_energy_source_prototype`
    /// is an optional attribute on `LuaEntityPrototype` returning a
    /// `LuaElectricEnergySourcePrototype`, and `buffer_capacity` is an
    /// attribute on that; nothing in this project had ever read through a
    /// sub-prototype before.
    ///
    /// An `accumulator`'s is the number that answers "how many accumulators
    /// per panel" — 5 MJ in vanilla — and [`Self::max_energy_production`]
    /// answers only its 300 kW discharge *rate*, which is a different
    /// question. Sent for every electric entity, so a machine's small internal
    /// buffer appears here too; that is not storage anybody plans with, and
    /// filtering it upstream would put the "what counts as an accumulator"
    /// decision in Lua where no Rust test can see it.
    ///
    /// `default`: `None` means the sender did not say — an entity with no
    /// electric energy source at all reports nothing here, and so does every
    /// world written before this field existed.
    #[serde(default)]
    pub electric_buffer_capacity: Option<f64>,
}

/// Ticks in a Factorio second at nominal speed, which is what turns the
/// runtime API's joules-per-tick into watts.
///
/// The runtime reports every energy figure per **tick**, while every number a
/// human reads — the 180 kW on an electric furnace's tooltip, the 900 kW of a
/// steam engine — is per second. So `J/tick * 60 / 1000` is kW, and this
/// constant exists so that conversion is written once. It is deliberately not
/// scaled by `game.speed`: a prototype's draw per tick does not change when
/// the game runs faster, only how many ticks pass per wall second does.
const TICKS_PER_SECOND: f64 = 60.;

/// One surface's daylight curve — **surface state, which no prototype can
/// answer**.
///
/// # Why this type had to exist before solar could be planned
///
/// `FactorioEntityPrototype::max_energy_production` says a `solar-panel` makes
/// 60 kW. That is its **noon** figure, and a base sized on it is dead every
/// night. The owner's ruling is that solar must be planned at *average*
/// output, and the average is the noon figure times the fraction of a day the
/// sun is up — a quantity every term of which is on `LuaSurface` and none of
/// which is on any prototype. Until this record existed
/// `crates/planner/src/state.rs` credited solar at nothing at all, which
/// refuses a solar base outright: the safe direction, and still wrong.
///
/// Every field is a `read_type`/`write_type` **attribute** on `LuaSurface` in
/// 2.1.17, checked against `runtime-api.json` rather than recalled. There is
/// no `get_dawn()` and no `get_ticks_per_day()` -- doclint-allow: names that
/// deliberately do NOT exist, in either Factorio or this tree, and their
/// absence is exactly the claim. The neighbouring pair `energy_usage`
/// (attribute) and `get_max_energy_production()` (method) landed one each way,
/// and reading a method as an attribute raises inside the mod's `pcall` and
/// arrives here as a silently missing field.
///
/// # The clock
///
/// [`Self::daytime`] runs `[0, 1)` and **0 is noon**, so the sunlit half
/// straddles the wrap. The vanilla order is
/// `dusk` 0.25 -> `evening` 0.45 -> `morning` 0.55 -> `dawn` 0.75:
///
/// ```text
///   0        0.25       0.45      0.55       0.75        1
///   |  full   |   fall   |  night  |   rise   |   full    |
///  noon      dusk     evening   morning     dawn        noon
/// ```
///
/// [`Self::average_solar_fraction`] integrates that trapezoid.
///
/// # Absent is not dark
///
/// Every field is `Option`, and `None` means **the sender did not say** — a
/// world recorded before this channel existed, which is every archived dump.
/// A surface that never reported daylight is not a surface in permanent
/// darkness, and [`Self::average_solar_fraction`] answers `None` rather than
/// zero for one.
#[derive(
    Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct SurfaceDaylight {
    /// The surface this curve was read from, by name.
    ///
    /// A curve is only ever about one surface — `solar_power_multiplier` is
    /// exactly the knob that makes an array a different size on a different
    /// planet — so a record that cannot say which one is not a fact about
    /// anything.
    #[serde(default)]
    pub surface: Option<SurfaceId>,
    /// How many ticks a full day-night cycle takes on this surface.
    ///
    /// **25,200 on Nauvis in 2.1.17 — seven minutes exactly — and not the
    /// 25,000 every reference repeats.** Measured off a running game on
    /// 2026-09-07 and pinned by `crates/core/tests/live-2.1.17-daylight.json`,
    /// because the difference is not cosmetic: it is proportional to the
    /// accumulator ratio below, and it is the whole 0.8% by which the derived
    /// figure misses the familiar 25:21 rule of thumb.
    ///
    /// **This is the term that turns a fraction into an energy**, and it is
    /// why accumulator sizing needs the surface and not just the panel: the
    /// deficit a night leaves is a fraction of a day times the length of a
    /// day. [`Self::average_solar_fraction`] does *not* depend on it at all,
    /// which is why the panel average survives the correction and the
    /// accumulator ratio does not.
    #[serde(default)]
    pub ticks_per_day: Option<u32>,
    /// The `daytime` at which dawn starts — the end of the dark half, where
    /// brightness finishes rising. 0.75 in vanilla.
    #[serde(default)]
    pub dawn: Option<f64>,
    /// The `daytime` at which dusk starts — the end of the sunlit half, where
    /// brightness begins to fall. 0.25 in vanilla.
    #[serde(default)]
    pub dusk: Option<f64>,
    /// The `daytime` at which evening starts, i.e. where the fall from full
    /// sun reaches the night floor. 0.45 in vanilla.
    #[serde(default)]
    pub evening: Option<f64>,
    /// The `daytime` at which morning starts, i.e. where brightness begins
    /// rising off the night floor. 0.55 in vanilla.
    #[serde(default)]
    pub morning: Option<f64>,
    /// Where the surface's clock stood when this was read, in `[0, 1)`.
    ///
    /// **Not part of the average and deliberately kept anyway.** A planner
    /// must not depend on it — that is the whole reason the average exists —
    /// but [`Self::freeze_daytime`] makes it the *only* thing that matters,
    /// and a record that dropped it could not tell a frozen noon from a frozen
    /// midnight.
    #[serde(default)]
    pub daytime: Option<f64>,
    /// The surface's own multiplier on solar output. 1.0 on Nauvis.
    #[serde(default)]
    pub solar_power_multiplier: Option<f64>,
    /// True when the sun never sets on this surface, whatever the four
    /// boundaries say.
    ///
    /// Sent because it makes the integral wrong in a way no boundary would
    /// reveal: an `always_day` surface produces the full-sun figure around the
    /// clock and its `dusk`/`dawn` still read as vanilla.
    #[serde(default)]
    pub always_day: Option<bool>,
    /// True when the clock is stopped, so the surface produces whatever
    /// [`Self::daytime`] was frozen at, forever. The other way the boundaries
    /// can be true and the integral still wrong.
    #[serde(default)]
    pub freeze_daytime: Option<bool>,
}

/// One row of the surface census: what a surface **is**, not what is on it.
///
/// # It exists so that "one surface" stops looking like "we never looked"
///
/// The mod's `on_chunk_generated` drops every non-Nauvis chunk and names the
/// surface it dropped, so each individual refusal is honest. But **nothing
/// enumerated `game.surfaces`**, so a world model holding one surface was
/// equally consistent with a save that has one surface and with a save whose
/// other surfaces were never mentioned. The world-record base was loaded and
/// its census read 39,237 entities, every one on `nauvis`; that number could
/// not distinguish the two readings either, because the entities that would
/// have said otherwise were dropped upstream.
///
/// So this reports **what exists**. It does not ingest anything, and the mod's
/// Nauvis guard is untouched.
///
/// # Absent is not empty
///
/// **Store a census as `Option<Vec<Self>>`, never a bare `Vec`.** `None` is
/// *nobody enumerated* -- every world dumped or snapshotted before 2026-09-07,
/// and any BotBridge older than the field -- while an empty list would be the
/// claim that the game has no surfaces at all, which is impossible. Folding
/// the two together puts the field straight back into the silence it exists to
/// end.
///
/// # Where it comes from, and where it does not go yet
///
/// The mod answers `remote.call('botbridge', 'surfaces')` and carries the same
/// list as a `surfaces` field on its `world_snapshot` reply. **Nothing in Rust
/// receives it yet**: the landing sites are `WorldSnapshot`, a home on the
/// world aggregate, and a `"surfaces"` arm in `output_parser.rs` (which must
/// land in the same commit as the mod's `writeout_surfaces`, because the
/// parser logs an *error* for a writeout key it has no arm for). The extra
/// snapshot key is ignored rather than rejected meanwhile. See
/// `docs/superpowers/notes/2026-09-07-two-things-the-mod-could-not-say.md`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct FactorioSurfaceInfo {
    /// `LuaSurface.name`: `nauvis`, `vulcanus`, or a platform's own name.
    /// This is the key `SurfaceId` is built from.
    pub name: String,
    /// `LuaSurface.index`, and the reason `game.surfaces[1]` is a different
    /// claim from `game.surfaces['nauvis']` -- the numeric index says
    /// "whichever surface was made first", which happens to be Nauvis on
    /// every save this project has run and is not guaranteed to be.
    ///
    /// The census is sorted by it, so two reads of one game are comparable.
    pub index: u32,
    /// The name of the planet this surface **is**, from `LuaSurface.planet`
    /// (an optional attribute returning a `LuaPlanet`, checked against
    /// `runtime-api.json` rather than recalled) -- so `nauvis` for Nauvis and
    /// `vulcanus` for Vulcanus.
    ///
    /// **`None` is a real answer here, not absence of one**, and it is the
    /// distinction worth having: a space platform is a surface that is not a
    /// planet, and a platform moves while a planet does not. A caller reading
    /// `None` as "we did not ask" would mistake every platform for a gap in
    /// the record.
    #[serde(default)]
    pub planet: Option<String>,
}

/// One straight run of the daylight curve: a length in days, and the
/// brightness at each end.
///
/// The curve is piecewise linear with exactly four runs, so integrating it is
/// exact rather than sampled — which matters, because
/// [`SurfaceDaylight::night_deficit_fraction`] integrates a *clipped* version
/// of it and a sampled clip would land the accumulator ratio somewhere near
/// the answer with no way to tell how near.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DaylightRun {
    days: f64,
    from: f64,
    to: f64,
}

impl SurfaceDaylight {
    /// The curve as four straight runs, starting at dawn, or `None` when this
    /// record cannot describe one.
    ///
    /// Refuses rather than guesses. A boundary set that is not
    /// `0 <= dusk <= evening <= morning <= dawn <= 1` is not a day this code
    /// understands, and computing an average from it would produce a number
    /// with no meaning that every downstream test would then agree with.
    fn runs(&self) -> Option<[DaylightRun; 4]> {
        let (dusk, evening, morning, dawn) = (self.dusk?, self.evening?, self.morning?, self.dawn?);
        if !(0. ..=1.).contains(&dusk)
            || !(dusk..=1.).contains(&evening)
            || !(evening..=1.).contains(&morning)
            || !(morning..=1.).contains(&dawn)
        {
            return None;
        }
        Some([
            // Dawn through the wrap at noon to dusk: the sunlit half.
            DaylightRun {
                days: (1. - dawn) + dusk,
                from: 1.,
                to: 1.,
            },
            DaylightRun {
                days: evening - dusk,
                from: 1.,
                to: 0.,
            },
            DaylightRun {
                days: morning - evening,
                from: 0.,
                to: 0.,
            },
            DaylightRun {
                days: dawn - morning,
                from: 0.,
                to: 1.,
            },
        ])
    }

    /// Brightness in `[0, 1]` at one instant, from the four boundaries.
    fn brightness_at(&self, daytime: f64) -> Option<f64> {
        let (dusk, evening, morning, dawn) = (self.dusk?, self.evening?, self.morning?, self.dawn?);
        self.runs()?;
        let t = daytime.rem_euclid(1.);
        if t <= dusk || t >= dawn {
            Some(1.)
        } else if t < evening {
            Some(1. - (t - dusk) / (evening - dusk))
        } else if t < morning {
            Some(0.)
        } else {
            Some((t - morning) / (dawn - morning))
        }
    }

    /// What fraction of its [`FactorioEntityPrototype::max_energy_production`]
    /// a solar panel makes **averaged over a whole day** on this surface.
    ///
    /// This is the number the owner's ruling is about: solar planned at
    /// average output rather than at nameplate. On vanilla Nauvis with a
    /// vanilla panel it is **0.7** — half the day at full sun, two fifths of
    /// it on a linear ramp worth half each, a tenth of it dark:
    ///
    /// ```text
    /// 0.50 * 1  +  0.20 * 0.5  +  0.10 * 0  +  0.20 * 0.5  =  0.7
    /// ```
    ///
    /// Nothing here is written down: every term comes from the four
    /// boundaries, the two endpoints come from the panel's own prototype, and
    /// the whole is scaled by [`Self::solar_power_multiplier`]. That is the
    /// standing rule — a rate copied from a table is a mod-compatibility
    /// defect — and it is why `0.7` appears in this doc as a *result* and
    /// nowhere in the code as an input.
    ///
    /// `at_day` and `at_night` are
    /// [`FactorioEntityPrototype::solar_panel_performance_at_day`] and
    /// `..._at_night`, the curve's two endpoints. They are 1 and 0 in vanilla,
    /// so passing them looks redundant and is not: a modded panel with a night
    /// floor has a genuinely different average, and a `0.0` night endpoint is
    /// a real answer rather than a missing one.
    ///
    /// `None` when this record cannot describe a day — see [`Self::runs`].
    /// Never zero for that case: a surface nobody reported daylight for is not
    /// a surface in permanent darkness.
    ///
    /// [`Self::always_day`] short-circuits to the full-sun figure, and
    /// [`Self::freeze_daytime`] to whatever instant the clock was stopped at,
    /// because either makes the integral wrong in a way the boundaries alone
    /// would not reveal.
    pub fn average_solar_fraction(&self, at_day: f64, at_night: f64) -> Option<f64> {
        let multiplier = self.solar_power_multiplier.unwrap_or(1.);
        let output = |brightness: f64| at_night + (at_day - at_night) * brightness;
        if self.always_day == Some(true) {
            return Some(output(1.) * multiplier);
        }
        if self.freeze_daytime == Some(true) {
            let frozen = self.brightness_at(self.daytime?)?;
            return Some(output(frozen) * multiplier);
        }
        let mean = self
            .runs()?
            .iter()
            .map(|run| run.days * (output(run.from) + output(run.to)) / 2.)
            .sum::<f64>();
        Some(mean * multiplier)
    }

    /// How much energy the dark part of a day leaves short, as a fraction of
    /// what one panel would make in a day at **full** output —
    /// i.e. `deficit_joules / (max_energy_production * ticks_per_day)`.
    ///
    /// # What question this answers
    ///
    /// An array sized at [`Self::average_solar_fraction`] carries a load equal
    /// to its own average. For part of the day it makes more than that and for
    /// part of it less, and the *less* has to come out of accumulators. This
    /// integrates exactly that shortfall: `max(0, average - instantaneous)`
    /// over the whole day, which is the area between the flat load line and
    /// the curve wherever the curve is underneath.
    ///
    /// On vanilla Nauvis with a vanilla panel it is **0.168 of a full day**.
    /// Multiplying by a panel's 1,000 J/tick and the live 25,200-tick day
    /// gives **4.234 MJ per panel**, which against an accumulator's 5 MJ
    /// ([`FactorioEntityPrototype::electric_buffer_capacity`]) is **0.8467
    /// accumulators per panel**. On the 25,000-tick day every reference
    /// quotes, the same arithmetic is exactly **0.84** — the familiar 25:21
    /// ratio. So the derivation reproduces the rule of thumb and says where
    /// the remaining 0.8% comes from: day length, measured rather than
    /// recalled.
    ///
    /// `None` on the same terms as [`Self::average_solar_fraction`], and
    /// `Some(0.0)` for an `always_day` surface, which is a real answer: no
    /// night, no deficit, no accumulators.
    pub fn night_deficit_fraction(&self, at_day: f64, at_night: f64) -> Option<f64> {
        let average = self.average_solar_fraction(at_day, at_night)?;
        let multiplier = self.solar_power_multiplier.unwrap_or(1.);
        let output = |brightness: f64| (at_night + (at_day - at_night) * brightness) * multiplier;
        if self.always_day == Some(true) {
            return Some(0.);
        }
        if self.freeze_daytime == Some(true) {
            // A frozen clock produces its one value forever, so it is never
            // above or below its own average: nothing to store.
            return Some(0.);
        }
        let deficit = self
            .runs()?
            .iter()
            .map(|run| clipped_shortfall(run.days, output(run.from), output(run.to), average))
            .sum::<f64>();
        Some(deficit)
    }
}

/// The area of `max(0, target - g)` over one straight run of length `days`
/// where `g` goes linearly from `from` to `to`.
///
/// Exact, including the case where the run crosses `target` partway: the
/// piece below the line is a triangle whose base is the crossed fraction of
/// the run. A version of this that tested only the endpoints would silently
/// score a crossing run as entirely above or entirely below.
fn clipped_shortfall(days: f64, from: f64, to: f64, target: f64) -> f64 {
    let (below_from, below_to) = (from < target, to < target);
    match (below_from, below_to) {
        (false, false) => 0.,
        (true, true) => days * ((target - from) + (target - to)) / 2.,
        // One end is under the line and the other is not, so the run crosses
        // it exactly once. `to != from` here, because equal endpoints cannot
        // straddle a value.
        _ => {
            let crossing = (target - from) / (to - from);
            let (base, depth) = if below_from {
                (crossing, target - from)
            } else {
                (1. - crossing, target - to)
            };
            days * base * depth / 2.
        }
    }
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
    /// [`Self::electric_energy_usage`] in kW, or `None` when the sender did
    /// not say.
    ///
    /// **`Some(0.0)` is passed through rather than folded into `None`.** An
    /// `inserter` genuinely reports zero and a reader has to be able to tell
    /// "electric, and the prototype says zero" from "nobody told us", because
    /// the first needs a duty cycle and the second needs a fallback table.
    #[must_use]
    pub fn energy_usage_kw(&self) -> Option<f64> {
        self.electric_energy_usage
            .map(|joules_per_tick| joules_per_tick * TICKS_PER_SECOND / 1000.)
    }

    /// [`Self::max_energy_production`] in kW, or `None` when the sender did
    /// not say.
    #[must_use]
    pub fn max_energy_production_kw(&self) -> Option<f64> {
        self.max_energy_production
            .map(|joules_per_tick| joules_per_tick * TICKS_PER_SECOND / 1000.)
    }

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

/// One of a belt-like entity's transport lines, and what is riding on it.
///
/// A belt is not an inventory and modelling it as one loses the thing that
/// makes it a belt: a `transport-belt` has **two** lanes, an
/// `underground-belt` four and a `splitter` eight, and which lane an item is
/// on decides whether a furnace arm can reach it. Two of this project's own
/// measured failures are lane failures -- an inserter drops on the belt's
/// **far** lane while a side-loading belt lands on the **near** one, and
/// getting either backwards puts ore and coal on one lane where they crowd
/// each other out while every entity still places 100% correctly.
///
/// **Counts, not positions.** `LuaTransportLine` also offers
/// `get_detailed_contents()`, which is every item with its position along the
/// line; the owner's ruling at scale is *"the direction and, for a whole
/// chain, what types of items are on it"*, so this carries
/// `get_contents()` -- one aggregated count per item kind -- and no item
/// positions at all.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct TransportLine {
    /// `defines.transport_line`'s own name for this line: `left_line`,
    /// `right_line`, `left_underground_line`, `secondary_right_line`,
    /// `left_split_line` and so on.
    ///
    /// **The name, not the raw index**, because the index alone is
    /// uninterpretable: line 3 is `left_underground_line` on an
    /// underground-belt and something else on a splitter, so a caller reading
    /// a number would have to re-derive the mapping from the entity type and
    /// would be inventing it. An index this build's `defines` cannot name
    /// arrives as `unmapped_<n>` rather than as a bare number, the same way
    /// `machine_row` reports an unknown entity status.
    pub line: String,
    /// What is on this lane, by item kind. Empty is a real and ordinary
    /// answer -- most lanes of most belts are empty most of the time -- and it
    /// is *not* the same as the lane not existing, which is expressed by the
    /// lane being absent from [`FactorioEntity::transport_lines`] entirely.
    #[serde(default, deserialize_with = "deserialize_helpers::vec_or_empty_map")]
    pub contents: Vec<InventoryItemWithQuality>,
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
    // Deliberately a `schemars(description)` and not a `///`, like `direction`
    // above: this type's OpenAPI schema is snapshotted, and the doc comment
    // this field would otherwise carry belongs to the reader of the Lua docs.
    // The long version is in `mods/BotBridge/types.lua`, at the read itself.
    //
    // What a machine has been GIVEN and not yet turned into anything: a
    // furnace's ore, an assembler's ingredients, a lab's science. The sibling
    // fields say what came out and what is burning, and without this one "the
    // furnace holds ore and is not smelting it" is indistinguishable from "no
    // ore ever arrived" -- a run that mined 46 ore for 17 plates could not say
    // where the other 29 were.
    //
    // **`None` is not empty.** A belt, a chest or a tree has no input
    // inventory and the mod sends no key at all, which arrives here as `None`;
    // a furnace standing empty sends `{}`, which the tolerant deserializer
    // below turns into `Some(vec![])`. Every record written before this field
    // existed also reads as `None`, i.e. "the sender did not say".
    #[schemars(
        description = "What the machine has been given and not yet consumed -- a furnace's ore, an assembler's ingredients, a lab's science. `null` means the entity has no input inventory (or the sender predates the field); an empty list means it has one and it is empty."
    )]
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub input_inventory: Option<Vec<InventoryItemWithQuality>>,
    // The lanes of a belt-like entity, in `defines.transport_line` order --
    // see `TransportLine`, which carries the reasoning. `None` for anything
    // that is not belt-connectable; a belt whose lanes are all empty is
    // `Some` of a list of empty lanes, because "this belt is running empty"
    // and "this is not a belt" are the two answers a belt diagnosis has to
    // separate, and `LuaTransportLine::get_contents()` has blocked four
    // distinct questions here for want of exactly that.
    //
    // A `schemars(description)` rather than a `///` for the same reason
    // `direction` and `input_inventory` above have one: the doc reaches the
    // Lua docs without enlarging the published API surface.
    #[schemars(
        description = "The lanes of a belt, underground-belt, splitter or loader, in `defines.transport_line` order, each with the items riding on it by kind. `null` for anything that is not belt-connectable; an empty lane list never occurs, but a lane with empty `contents` does and means the lane is running empty."
    )]
    #[serde(
        default,
        deserialize_with = "deserialize_helpers::option_vec_or_empty_map"
    )]
    pub transport_lines: Option<Vec<TransportLine>>,
    // WHAT THE MACHINE IS DOING, by `defines.entity_status`' own name for it:
    // `working`, `no_ingredients`, `no_power`, `waiting_for_space_in_destination`.
    //
    // A `schemars(description)` rather than a `///`, like `direction`,
    // `input_inventory` and `transport_lines` above: this type's OpenAPI schema
    // is snapshotted in `app/src/api/openapi.snapshot.json`, and the reasoning
    // belongs to whoever reads the code. The long version is in
    // `mods/BotBridge/types.lua`, at the read itself.
    //
    // The three inventory fields above say what a machine HOLDS and none of
    // them says whether it is running. That absence is the dominant and only
    // unbounded term in `FlowGraph`'s error against a real base -- +16% to
    // +23% on the world-record save -- and the back-pressure half of it
    // (194 drills at `waiting_for_space_in_destination` on that base) is not
    // derivable from the graph at any price. See
    // `docs/superpowers/notes/2026-09-07-a-machine-standing-still.md`.
    //
    // **The NAME, never the number.** `defines.entity_status` is an enum whose
    // numbering is a Factorio implementation detail, so an archived `12` would
    // need that exact version's table to be readable and could silently change
    // meaning across versions. A value the mod's build cannot name arrives as
    // `unmapped_<n>` rather than being dropped.
    //
    // **`None` is not "working", and absent is not empty.** A tree, a chest or
    // a belt has no status concept and the mod sends no key at all; a machine
    // that is *stopped* has a name for being stopped, and that name is the
    // measurement. Every archived record and world dump written before this
    // field existed also reads as `None`, i.e. "the sender did not say" --
    // which is why `#[serde(default)]` is here and why nothing may default it
    // to a running machine.
    #[schemars(
        description = "What the entity is doing, as `defines.entity_status`' own name for it -- `working`, `no_power`, `no_ingredients`, `waiting_for_space_in_destination`. A name and never the raw enum number; a value the mod cannot resolve arrives as `unmapped_<n>`. `null` means the entity has no status concept (a tree, a chest) or the sender predates the field -- never \"working\"."
    )]
    #[serde(default)]
    pub status: Option<String>,
    pub amount: Option<u32>,        // only type = resource
    pub recipe: Option<String>,     // only CraftingMachines
    pub ghost_name: Option<String>, // only type = entity-ghost
    pub ghost_type: Option<String>, // only type = entity-ghost
    /// `Some` only for one half of an underground-belt pair -- see
    /// `crate::blueprint::UndergroundHalf`. An `Option` defaulting to `None`
    /// on a missing field, so every archived run record and world dump
    /// written before this field existed still deserialises.
    #[serde(default)]
    pub underground_half: Option<crate::blueprint::UndergroundHalf>,
    /// Which surface this entity stands on, as the mod read it from
    /// `LuaEntity.surface.name`.
    ///
    /// **`None` means the sender did not say, not "Nauvis".** Every world dump
    /// and run record written before this field existed lacks it, and while
    /// every one of them is in fact a Nauvis-only run, that is a fact about
    /// the mod's `on_chunk_generated` guard rather than something the archived
    /// bytes assert. So an 865 MB dump from yesterday still deserialises --
    /// pinned by `crates/core/tests/surface_id.rs`.
    ///
    /// Nothing keys on it yet -- the containers (`EntityGraph`, `PlanState`)
    /// are still position-only and would alias two surfaces into one. This
    /// carries the fact so the record can be read; keying is the next rung.
    //
    // **The `#[serde(default)]` is documentation, not mechanism.** A plain
    // `//` comment because this struct's doc comments are published in
    // `app/src/api/openapi.snapshot.json` and this is an internal note.
    // Removing the attribute as a falsification changed nothing, and that
    // green was nearly misread as "the test is hollow": serde already treats
    // an `Option<T>` field as optional, verified against serde 1 in a
    // standalone crate on 2026-09-06 -- a struct with a plain, unattributed
    // `Option<Newtype>` deserialises a payload lacking that key to `None`. It
    // is the `Option` that carries old dumps, here and on
    // `underground_half`, whose own doc makes the same slightly-too-strong
    // claim. Kept because it states the intent, and because the day this
    // field stops being an `Option` it becomes load-bearing.
    #[serde(default)]
    pub surface: Option<SurfaceId>,
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
            bounding_box: add_to_rect_turned(
                &Rect::from_wh(0.796875, 0.796875),
                position,
                direction,
            ),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    /// One half of an underground pair, now nameable.
    ///
    /// Factorio calls the two halves `input` and `output`; they take the same
    /// direction and are distinguished by `belt_to_ground_type`. An earlier
    /// version of this constructor took an `output: bool` and immediately
    /// discarded it with `let _ = output;`, because `FactorioEntity` had
    /// nowhere to put it -- a signature that promised the returned entity
    /// carried the half when it did not. `underground_half`
    /// (`crate::blueprint::UndergroundHalf`) is that place, and the mod's
    /// `rcon_place_entity` now takes a fifth argument it forwards to
    /// `surface.create_entity` as `type`, so `half` is honoured all the way
    /// to the game.
    ///
    /// `method::connect` still never emits these -- it calls `route_belt`
    /// with `max_underground: None` -- but the reason is now "not wired up
    /// yet", not "cannot be expressed": `TileKind::UndergroundEntry` /
    /// `UndergroundExit` map onto `UndergroundHalf::Input` /
    /// `UndergroundHalf::Output` respectively, and the `unreachable!()` arm
    /// in `connect.rs` could construct exactly that the day `max_underground`
    /// is threaded through.
    pub fn new_underground_belt(
        position: &Position,
        direction: Direction,
        half: crate::blueprint::UndergroundHalf,
    ) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::UndergroundBelt.to_string(),
            entity_type: EntityType::UndergroundBelt.to_string(),
            position: position.clone(),
            // Same footprint as `new_transport_belt`'s: the real prototype's
            // collision box is 0.796875 x 0.796875 (checked against
            // `crates/core/tests/entity-prototype-fixtures.json`), and that
            // sibling already rounds the same box to 0.8 x 0.8.
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            underground_half: Some(half),
            ..Default::default()
        }
    }
    pub fn new_splitter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Splitter.to_string(),
            entity_type: EntityType::Splitter.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(
                &Rect::from_wh(1.796875, 0.796875),
                position,
                direction,
            ),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_inserter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity::new_named_inserter(EntityName::Inserter.to_string(), position, direction)
    }

    /// An inserter of a **named** prototype, for the callers that cannot use
    /// the electric one.
    ///
    /// `inserter` is not craftable at t=0 — its recipe takes an
    /// `electronic-circuit` and reads `enabled: false` on a freeplay force,
    /// checked against seed 31337's own dump — whereas `burner-inserter` is
    /// (1 iron plate, 1 gear). Anything a stage-1 plan places therefore has to
    /// name the prototype rather than inherit `new_inserter`'s.
    ///
    /// Every other field is shared, and deliberately so: both prototypes have
    /// the same 0.78 collision box, the same one-tile reach, and the same
    /// convention that `direction` names the side the arm **picks up** from.
    /// A burner inserter differs only in where its energy comes from, which
    /// is not a field this type carries.
    pub fn new_named_inserter(
        inserter: String,
        position: &Position,
        direction: Direction,
    ) -> FactorioEntity {
        FactorioEntity {
            name: inserter,
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
            bounding_box: add_to_rect_turned(
                &Rect::from_wh(1.3984375, 1.3984375),
                position,
                direction,
            ),
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
            bounding_box: add_to_rect_turned(
                &Rect::from_wh(2.6953125, 2.6953125),
                position,
                direction,
            ),
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
            bounding_box: add_to_rect_turned(
                &Rect::from_wh(1.3984375, 1.3984375),
                position,
                direction,
            ),
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
    UndergroundBelt,
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
            crafting_categories: None,
            mining_fluid: None,
            supply_area_distance: None,
            maximum_wire_distance: None,
            distribution_effectivity: None,
            beacon_profile: None,
            electric_energy_usage: None,
            max_energy_production: None,
            mining_drill_radius: None,
            solar_panel_performance_at_day: None,
            solar_panel_performance_at_night: None,
            electric_buffer_capacity: None,
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

    /// **Three inventories, three answers, and the missing key is the one that
    /// has to keep working.**
    ///
    /// A mod build that predates `input_inventory` sends no key at all, and so
    /// does a running mod asked about a `wooden-chest`. Both must reach
    /// `None` rather than failing to deserialize -- `Box<Option<_>>` is not
    /// optional to serde on its own, which is exactly how every
    /// `inventory_contents_at` reply once failed to parse over a camelCase
    /// spelling. `{}` is Lua's empty table and must reach `Some(empty)`,
    /// because a furnace with an empty input slot is a real observation.
    #[test]
    fn an_input_inventory_distinguishes_absent_from_empty_from_held() {
        let held: InventoryResponse = serde_json::from_str(
            r#"{"name":"stone-furnace","position":{"x":0.5,"y":0.5},
                "output_inventory":{},"fuel_inventory":{},
                "input_inventory":[{"name":"iron-ore","quality":"normal","count":34}]}"#,
        )
        .expect("parses");
        assert_eq!(
            held.input_inventory.as_ref().as_ref().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            held.input_inventory
                .as_ref()
                .as_ref()
                .and_then(|items| items.first())
                .map(|item| item.count),
            Some(34)
        );

        let empty: InventoryResponse = serde_json::from_str(
            r#"{"name":"stone-furnace","position":{"x":0.5,"y":0.5},
                "input_inventory":{}}"#,
        )
        .expect("parses");
        assert_eq!(
            *empty.input_inventory,
            Some(vec![]),
            "a furnace that answered with an empty input slot is standing \
             empty, which is not the same as having no input slot"
        );

        let absent: InventoryResponse =
            serde_json::from_str(r#"{"name":"wooden-chest","position":{"x":0.5,"y":0.5}}"#)
                .expect("a reply with no input key must still parse");
        assert_eq!(
            *absent.input_inventory, None,
            "no key means the entity has no input inventory, or the mod \
             predates the field -- never that it is empty"
        );
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

    /// `new_underground_belt` regained its half parameter because
    /// `FactorioEntity` can now carry it -- this pins that the two halves it
    /// builds actually differ, not merely that the parameter is accepted.
    #[test]
    fn new_underground_belt_halves_actually_differ() {
        use crate::blueprint::UndergroundHalf;

        let pos = Position::new(3.0, 4.0);
        let input =
            FactorioEntity::new_underground_belt(&pos, Direction::North, UndergroundHalf::Input);
        let output =
            FactorioEntity::new_underground_belt(&pos, Direction::North, UndergroundHalf::Output);

        assert_eq!(input.underground_half, Some(UndergroundHalf::Input));
        assert_eq!(output.underground_half, Some(UndergroundHalf::Output));
        assert_ne!(
            input.underground_half, output.underground_half,
            "the two halves must be told apart by the entity they build, not \
             only by their position relative to each other"
        );
        // Both halves are still ordinary underground belts otherwise --
        // same name, same type, same footprint -- only the half differs.
        assert_eq!(input.name, output.name);
        assert_eq!(input.entity_type, output.entity_type);
    }

    /// Vanilla Nauvis, as `LuaSurface` reports it.
    fn nauvis_daylight() -> SurfaceDaylight {
        SurfaceDaylight {
            surface: Some(SurfaceId::nauvis()),
            // The live 2.1.17 figure, not the 25,000 every reference
            // repeats. See `crates/core/tests/live-2.1.17-daylight.json`.
            ticks_per_day: Some(25_200),
            dawn: Some(0.75),
            dusk: Some(0.25),
            evening: Some(0.45),
            morning: Some(0.55),
            daytime: Some(0.0),
            solar_power_multiplier: Some(1.0),
            always_day: Some(false),
            freeze_daytime: Some(false),
        }
    }

    /// **The number the whole channel exists for.**
    ///
    /// A vanilla panel averages 0.7 of its noon output over a day — 42 kW of
    /// its 60 — and that falls out of the four boundaries with nothing written
    /// down: half the day at full sun, two fifths on a linear ramp worth half
    /// each, a tenth dark.
    ///
    /// It is asserted exactly, not approximately, because every term is a
    /// halving or a sum of exact binary fractions and a tolerance would hide
    /// a real drift.
    #[test]
    fn a_vanilla_panel_averages_seven_tenths_of_its_noon_output() {
        let fraction = nauvis_daylight()
            .average_solar_fraction(1.0, 0.0)
            .expect("vanilla boundaries describe a day");
        assert_eq!(fraction, 0.7);
    }

    /// The endpoints are the *scale* of the curve, so a modded panel with a
    /// night floor has a genuinely different average and the shape is
    /// unchanged: `night + (day - night) * 0.7`.
    ///
    /// Here `0.2 + 0.8 * 0.7 = 0.76`. This is the test that would fail if the
    /// endpoints were assumed to be 1 and 0 anywhere, which is the shape of
    /// the hard-coded-rate defect the standing rule is about.
    #[test]
    fn the_endpoints_scale_the_curve_rather_than_being_assumed() {
        let fraction = nauvis_daylight()
            .average_solar_fraction(1.0, 0.2)
            .expect("vanilla boundaries describe a day");
        assert!(
            (fraction - 0.76).abs() < 1e-12,
            "0.2 + 0.8 * 0.7, got {fraction}"
        );
    }

    /// `solar_power_multiplier` is the surface's own scaling and is exactly
    /// what differs between planets, so it must multiply the answer rather
    /// than being ignored on Nauvis where it happens to be 1.
    #[test]
    fn the_surfaces_own_multiplier_scales_the_average() {
        let mut daylight = nauvis_daylight();
        daylight.solar_power_multiplier = Some(0.5);
        assert_eq!(daylight.average_solar_fraction(1.0, 0.0), Some(0.35));
    }

    /// An `always_day` surface produces the full-sun figure around the clock
    /// while its four boundaries still read as vanilla, so the flag has to
    /// short-circuit the integral. A version that trusted the boundaries alone
    /// would under-credit such a surface by 30% and every boundary assertion
    /// above would still pass.
    #[test]
    fn always_day_is_not_visible_in_the_boundaries() {
        let mut daylight = nauvis_daylight();
        daylight.always_day = Some(true);
        assert_eq!(daylight.average_solar_fraction(1.0, 0.0), Some(1.0));
        assert_eq!(
            daylight.night_deficit_fraction(1.0, 0.0),
            Some(0.0),
            "no night, no deficit, no accumulators",
        );
    }

    /// The other way the boundaries can be true and the integral still wrong:
    /// a stopped clock produces whatever instant it stopped at, forever. Frozen
    /// at midnight a vanilla surface makes nothing at all.
    #[test]
    fn a_frozen_clock_produces_the_instant_it_stopped_at() {
        let mut daylight = nauvis_daylight();
        daylight.freeze_daytime = Some(true);
        daylight.daytime = Some(0.5);
        assert_eq!(daylight.average_solar_fraction(1.0, 0.0), Some(0.0));

        daylight.daytime = Some(0.35);
        let halfway = daylight
            .average_solar_fraction(1.0, 0.0)
            .expect("a frozen vanilla day");
        assert!(
            (halfway - 0.5).abs() < 1e-12,
            "halfway down the dusk-to-evening ramp, got {halfway}",
        );
    }

    /// A record that cannot describe a day refuses rather than returning a
    /// number nobody can interpret — and, critically, refuses rather than
    /// returning zero. A surface nobody reported daylight for is not a surface
    /// in permanent darkness, and `state.rs` reads the `None` as "may not put
    /// solar on a network" instead of "solar makes nothing here".
    #[test]
    fn an_unreported_or_impossible_day_is_unknown_and_not_dark() {
        assert_eq!(
            SurfaceDaylight::default().average_solar_fraction(1.0, 0.0),
            None,
            "every archived dump predates this channel and reports nothing",
        );

        let mut scrambled = nauvis_daylight();
        // dusk after evening: not a day this code understands.
        scrambled.dusk = Some(0.5);
        assert_eq!(scrambled.average_solar_fraction(1.0, 0.0), None);
        assert_eq!(scrambled.night_deficit_fraction(1.0, 0.0), None);
    }

    /// **The accumulator half, and the second of the two vanilla ratios this
    /// one channel reproduces.**
    ///
    /// The shortfall an array of vanilla panels leaves against its own average
    /// load is 0.168 of a full day's full output. Against a panel's 1,000
    /// J/tick it is 4.2 MJ on a 25,000-tick day — **0.84 accumulators per
    /// panel**, the familiar 25:21 ratio — and 4.234 MJ on the 25,200-tick day
    /// this install actually runs, which is 0.8467. Both are asserted, because
    /// the rule of thumb is what a reader will check against and the live
    /// figure is what the planner will use.
    ///
    /// The fraction itself is exact and independent of day length: the deficit
    /// is `0.14 * 0.35` on each ramp plus `0.1 * 0.7` of night, i.e.
    /// `0.049 + 0.07 + 0.049`.
    #[test]
    fn the_night_deficit_is_the_vanilla_accumulator_ratio() {
        let deficit = nauvis_daylight()
            .night_deficit_fraction(1.0, 0.0)
            .expect("vanilla boundaries describe a day");
        assert!(
            (deficit - 0.168).abs() < 1e-12,
            "0.049 + 0.07 + 0.049, got {deficit}"
        );

        let by_the_book = 1000.0 * 25_000.0 * deficit / 5_000_000.0;
        assert!(
            (by_the_book - 0.84).abs() < 1e-12,
            "on the 25,000-tick day every reference quotes, exactly 25 panels \
             to 21 accumulators, got {by_the_book}"
        );
        let live = 1000.0 * 25_200.0 * deficit / 5_000_000.0;
        assert!(
            (live - 0.84 * 25_200. / 25_000.).abs() < 1e-12,
            "and on the day the running game reports, that scaled by the day \
             length and nothing else, got {live}"
        );
    }

    /// 0.7 and 0.84 are answers to different questions from the same curve,
    /// and conflating them would size an array 20% short. Pinned here because
    /// the brief that commissioned this work read 25:21 as a statement that
    /// the *average* is 0.84, which it is not.
    #[test]
    fn the_average_and_the_accumulator_ratio_are_different_integrals() {
        let daylight = nauvis_daylight();
        let average = daylight.average_solar_fraction(1.0, 0.0).expect("a day");
        let deficit = daylight.night_deficit_fraction(1.0, 0.0).expect("a day");
        assert!(
            (average - 0.7).abs() < 1e-12 && (deficit - 0.168).abs() < 1e-12,
            "average {average}, deficit {deficit}",
        );
        let ratio = deficit * 25_200.0 * 1000.0 / 5_000_000.0;
        assert_ne!(
            average, ratio,
            "the average fraction and the accumulators-per-panel ratio are \
             not the same number and must never be derived from each other",
        );
    }

    /// A run that crosses the load line partway contributes a triangle, and a
    /// version of the integral that tested only its endpoints would score the
    /// whole run as above or below. Vanilla's ramps cross at 0.31 and 0.69, so
    /// this is exercised by every assertion above — but only implicitly, and a
    /// direct case makes the failure legible.
    #[test]
    fn a_run_that_crosses_the_load_line_contributes_a_triangle() {
        // Full sun for a quarter of the day, dark for the rest, with no ramp
        // at all: nothing crosses, and the deficit is the flat area under the
        // 0.25 average across the dark three quarters.
        let square = SurfaceDaylight {
            dusk: Some(0.125),
            evening: Some(0.125),
            morning: Some(0.875),
            dawn: Some(0.875),
            ..nauvis_daylight()
        };
        assert_eq!(square.average_solar_fraction(1.0, 0.0), Some(0.25));
        assert_eq!(
            square.night_deficit_fraction(1.0, 0.0),
            Some(0.75 * 0.25),
            "a square wave has no crossing run at all",
        );

        // The vanilla curve, whose two ramps each cross. Their triangles are
        // what separates 0.168 from the 0.21 an endpoint-only test would give
        // by charging each whole ramp at its mean depth.
        assert!(
            (nauvis_daylight()
                .night_deficit_fraction(1.0, 0.0)
                .expect("a day")
                - 0.168)
                .abs()
                < 1e-12
        );
    }
}
