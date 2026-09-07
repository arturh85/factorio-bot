//! What a prototype name *is*: an item a character can hold, or a fluid that
//! no character can hold in any amount.
//!
//! # Why this is a classifier and not a quantity
//!
//! The planner's quantity model is `u32`, keyed by [`crate::ids::ItemId`],
//! which is a `String`. The obvious reading of "the planner cannot represent
//! fluids" is that the *number* is wrong -- Factorio reports fluid amounts as
//! a `double`, so surely a fluid wants a float. It does not, and the measured
//! reason is in `docs/superpowers/notes/2026-09-06-a-fluid-is-not-an-item.md`
//! §0b: **no vanilla recipe declares a fractional amount**
//! (`grep -c 'amount *= *[0-9]+\.[0-9]' workspace/data/base/prototypes/recipe.lua`
//! → 0), and `factorio_bot_core::types::RawFactorioIngredient` already narrows
//! the game's `double` to `u32` at the crate boundary, deliberately and with a
//! comment saying so.
//!
//! What is actually missing is a **place to put one**. A fluid has no
//! addressable storage anywhere in this planner:
//!
//! * `BotState::inventory` is a character main inventory, and the game will
//!   not put a fluid in one;
//! * `PlanState::available` sums character inventories, so a fluid is
//!   permanently `available == 0` -- not scarce, *unreachable*;
//! * [`crate::action::InventorySlot`] has seven variants and none addresses a
//!   fluidbox, and no Factorio API shape would let one -- a fluidbox is
//!   addressed by index and pipe connection, not by an inventory define;
//! * `Insert` and `Remove` bottom out in `LuaPlayer.insert` / `remove_item`,
//!   which cannot take a fluid at all;
//! * `withdraw_slot` returns `None` for `storage-tank`, `pipe` and `pumpjack`,
//!   so no fluid container can become a `Buffer`.
//!
//! So a bot cannot carry 100 crude-oil, and a bot also cannot carry *any*
//! crude oil. Changing the number type leaves every one of those five facts
//! standing. What the planner needs first is the ability to **say** that a
//! name is a fluid, and then to refuse -- which is what this module is.
//!
//! # Determinism
//!
//! No float appears here, so `total_cmp` never arises. [`SubstanceTable`] is a
//! `BTreeMap`, [`Substance`] derives `Ord` on variant order, and every
//! iterator this module hands out is in name order regardless of the order the
//! world's `DashMap`s were walked in.
//!
//! # This module has no caller
//!
//! Deliberately, and it is stated here because this repository has just paid
//! for the opposite (`method::connect::connect_steps`, four clean reviews, a
//! materials bill its first real caller refused). Wiring it up needs
//! `error.rs`, `method/have.rs` and `method/util.rs`; the note's §5 spells out
//! the three edits and their blast radius. Until then this is a hypothesis
//! with good evidence about its inputs, not a working refusal.

use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::types::FactorioRecipe;
use std::collections::BTreeMap;
use thiserror::Error;

/// The value Factorio uses for a fluid in `ingredient_type` / `product_type`.
///
/// Named rather than inlined so a test asserts over the *actual* string the
/// classifier matches; a second copy in a test would keep passing if this one
/// changed.
pub const FLUID_TYPE: &str = "fluid";

/// The value Factorio uses for an item in the same fields.
pub const ITEM_TYPE: &str = "item";

/// What a prototype name is.
///
/// Two variants and no `Unknown`: absence of evidence is expressed by
/// `Option<Substance>` at every boundary, so a caller cannot accidentally
/// pattern-match "we do not know" as if it were a third kind of thing that
/// exists in the game.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Substance {
    /// A character can hold it, an inserter can move it, a chest can store it.
    Item,
    /// It lives in a fluidbox. No character inventory can hold it.
    Fluid,
}

impl std::fmt::Display for Substance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Substance::Item => write!(f, "an item"),
            Substance::Fluid => write!(f, "a fluid"),
        }
    }
}

/// A name the world's two independent sources of truth describe differently.
///
/// Measured at **zero on both real captures** -- `workspace/scripts/map.json`
/// and `crates/core/tests/live-2.1.17-world-snapshot.json`, 662 recipes and
/// 342 item prototypes each. That zero is what makes [`SubstanceTable`]'s tier
/// order safe; this type exists so that a capture where it stops being zero
/// *says so* rather than having the tier order silently pick a side.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Disagreement {
    pub name: String,
    /// What the disagreement is, in words a reader can act on.
    pub why: String,
}

/// Every name this world describes, and what it is.
///
/// Built once from a world and then only read. Building walks the recipe table
/// and the item-prototype table, which is 662 + 342 entries on a real capture
/// -- cheap enough to build per refusal, too expensive to build per lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubstanceTable {
    by_name: BTreeMap<String, Substance>,
}

impl SubstanceTable {
    /// Classify from the two tables, without needing a whole world.
    ///
    /// Split out from [`SubstanceTable::from_world`] so a test can build one
    /// straight from a `WorldSnapshot` -- the byte-for-byte RCON capture at
    /// `crates/core/tests/live-2.1.17-world-snapshot.json` -- rather than from
    /// a fixture the same hand wrote.
    ///
    /// # The evidence, in order
    ///
    /// 1. the name is in `item_prototypes` → [`Substance::Item`]. A fluid is
    ///    never in the item table; this is the game's own statement.
    /// 2. some recipe declares it [`FLUID_TYPE`] → [`Substance::Fluid`].
    /// 3. some recipe declares it [`ITEM_TYPE`] → [`Substance::Item`].
    /// 4. otherwise the name is absent from the table, and [`Self::of`]
    ///    answers `None`.
    ///
    /// A declared type of `""` -- which `#[serde(default)]` produces for a
    /// capture taken before the mod forwarded the field -- is **no evidence**,
    /// not evidence of an item. Both real captures have zero of them; an older
    /// one may not.
    pub fn from_parts<'a, R, I>(recipes: R, item_prototype_names: I) -> Self
    where
        R: IntoIterator<Item = &'a FactorioRecipe>,
        I: IntoIterator<Item = &'a str>,
    {
        let mut by_name: BTreeMap<String, Substance> = BTreeMap::new();

        // Tier 2 and 3 first, so tier 1 can overwrite -- see `insert` below.
        for recipe in recipes {
            for (name, declared) in declared_types(recipe) {
                match declared {
                    FLUID_TYPE => {
                        by_name.insert(name.to_string(), Substance::Fluid);
                    }
                    ITEM_TYPE => {
                        // Do not let an item declaration overwrite a fluid
                        // one: a name declared both ways is a disagreement,
                        // and `disagreements()` is where it gets reported.
                        // Silently preferring `Item` would put a fluid in a
                        // bot's hands, which is the whole failure this module
                        // exists to stop.
                        by_name.entry(name.to_string()).or_insert(Substance::Item);
                    }
                    _ => {}
                }
            }
        }

        // Tier 1 last and unconditionally: the item table is the game's direct
        // statement about what an item is, and it outranks an inference drawn
        // from how a recipe happened to label an ingredient.
        for name in item_prototype_names {
            by_name.insert(name.to_string(), Substance::Item);
        }

        SubstanceTable { by_name }
    }

    /// Classify from a live or dumped world.
    pub fn from_world(world: &FactorioSurface) -> Self {
        // Collected rather than streamed because the recipe table is behind a
        // `DashMap` whose guards cannot be held across the closure boundary
        // `from_parts` wants, and because the result must not depend on the
        // order the map was walked in.
        let recipes: Vec<FactorioRecipe> = world
            .globals
            .recipes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let items: Vec<String> = world
            .globals
            .item_prototypes
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        SubstanceTable::from_parts(recipes.iter(), items.iter().map(String::as_str))
    }

    /// Classify from the world a plan state overlays.
    ///
    /// Reads `PlanState::base()` and nothing else: prototype and recipe tables
    /// are properties of the game's data, and no plan overlay adds or removes
    /// one.
    pub fn from_state(state: &crate::state::PlanState) -> Self {
        SubstanceTable::from_world(state.base())
    }

    /// What `name` is, or `None` when this world says nothing about it.
    ///
    /// `None` is a real answer and callers must handle it as one. The crate's
    /// hand-built fixture worlds are full of names no prototype table
    /// mentions; reading `None` as [`Substance::Fluid`] would refuse every one
    /// of them, and reading it as [`Substance::Item`] would state something
    /// nobody established.
    pub fn of(&self, name: &str) -> Option<Substance> {
        self.by_name.get(name).copied()
    }

    /// Whether `name` is known to be a fluid.
    ///
    /// **Positive evidence only**: an unknown name is `false`, which is the
    /// behaviour every existing call site already has. That fallback is stated
    /// here rather than left implicit, and a caller that needs the difference
    /// should ask [`Self::of`].
    pub fn is_fluid(&self, name: &str) -> bool {
        self.of(name) == Some(Substance::Fluid)
    }

    /// Every fluid this world declares, in name order.
    pub fn fluids(&self) -> impl Iterator<Item = &str> {
        self.by_name
            .iter()
            .filter(|(_, kind)| **kind == Substance::Fluid)
            .map(|(name, _)| name.as_str())
    }

    /// How many names are classified at all.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Where the world's two sources of truth contradict each other, in name
    /// order.
    ///
    /// Two shapes, both of which the tier order in [`Self::from_parts`] would
    /// otherwise resolve silently:
    ///
    /// * a name declared [`FLUID_TYPE`] by some recipe *and* present in
    ///   `item_prototypes`;
    /// * a name declared [`FLUID_TYPE`] by one recipe and [`ITEM_TYPE`] by
    ///   another.
    ///
    /// This is the instrument's own self-check, and it is the reason the tier
    /// order is defensible: it is safe because the answer is measured at zero,
    /// not because the order is obviously right.
    pub fn disagreements<'a, R, I>(recipes: R, item_prototype_names: I) -> Vec<Disagreement>
    where
        R: IntoIterator<Item = &'a FactorioRecipe>,
        I: IntoIterator<Item = &'a str>,
    {
        let mut as_fluid: BTreeMap<&str, ()> = BTreeMap::new();
        let mut as_item: BTreeMap<&str, ()> = BTreeMap::new();
        let recipes: Vec<&FactorioRecipe> = recipes.into_iter().collect();
        for recipe in &recipes {
            for (name, declared) in declared_types(recipe) {
                match declared {
                    FLUID_TYPE => {
                        as_fluid.insert(name, ());
                    }
                    ITEM_TYPE => {
                        as_item.insert(name, ());
                    }
                    _ => {}
                }
            }
        }
        let items: BTreeMap<&str, ()> = item_prototype_names
            .into_iter()
            .map(|name| (name, ()))
            .collect();

        let mut out: Vec<Disagreement> = Vec::new();
        for name in as_fluid.keys() {
            if items.contains_key(name) {
                out.push(Disagreement {
                    name: (*name).to_string(),
                    why: "a recipe declares it a fluid, and it is also in item_prototypes"
                        .to_string(),
                });
            }
            if as_item.contains_key(name) {
                out.push(Disagreement {
                    name: (*name).to_string(),
                    why: "one recipe declares it a fluid and another declares it an item"
                        .to_string(),
                });
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Every `(name, declared type)` a recipe states, ingredients then products.
///
/// The field this reads is exactly what `method::util::ingredients_of`
/// discards -- see the note's §5, edit 3.
fn declared_types(recipe: &FactorioRecipe) -> Vec<(&str, &str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    if let Some(ingredients) = recipe.ingredients.as_ref() {
        for i in ingredients {
            out.push((i.name.as_str(), i.ingredient_type.as_str()));
        }
    }
    for p in &recipe.products {
        out.push((p.name.as_str(), p.product_type.as_str()));
    }
    out
}

/// A recipe's inputs, split by what a bot could actually carry.
///
/// The lossless counterpart of `method::util::ingredients_of`, which maps
/// `(name, amount)` and drops the type. Nothing calls this yet; it exists so
/// that when `HandCraft` stops emitting `Condition::HasItem { 20
/// sulfuric-acid }` at a bot, the split it needs already has a home and a
/// test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bill {
    /// In recipe order, which is the game's own order.
    pub items: Vec<(String, u32)>,
    pub fluids: Vec<(String, u32)>,
}

impl Bill {
    pub fn has_fluids(&self) -> bool {
        !self.fluids.is_empty()
    }
}

/// Split a recipe's ingredients into what a character can hold and what it
/// cannot.
///
/// The ingredient's **own declared type wins** when it says something, because
/// that is the game speaking about this specific use. `table` is consulted
/// only for a blank declaration (an older capture), and a name neither source
/// knows is counted as an item -- which is the behaviour every current call
/// site has, made explicit here rather than left to a fall-through.
pub fn split_bill(table: &SubstanceTable, recipe: &FactorioRecipe) -> Bill {
    let mut bill = Bill::default();
    let Some(ingredients) = recipe.ingredients.as_ref() else {
        return bill;
    };
    for i in ingredients {
        let kind = match i.ingredient_type.as_str() {
            FLUID_TYPE => Some(Substance::Fluid),
            ITEM_TYPE => Some(Substance::Item),
            _ => table.of(&i.name),
        };
        let entry = (i.name.clone(), i.amount);
        match kind {
            Some(Substance::Fluid) => bill.fluids.push(entry),
            _ => bill.items.push(entry),
        }
    }
    bill
}

/// Where a fluid could come from, for a refusal that has to say more than
/// "no".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FluidSource {
    /// Nothing in this world produces it: no recipe lists it as a product.
    Nothing,
    /// Recipes produce it, with their categories. Both lists are in name
    /// order and the same length is not implied -- several recipes can share a
    /// category and the categories are deduplicated.
    Recipes {
        recipes: Vec<String>,
        categories: Vec<String>,
    },
}

impl FluidSource {
    /// What produces `fluid` in this world, read off the recipe table.
    ///
    /// This is deliberately *not* `method::util::recipe_for`, which looks a
    /// product up by recipe **name** and therefore answers `None` for every
    /// fluid in the game -- there is no recipe called `petroleum-gas`. That
    /// mismatch is why a fluid goal fails quietly today; see the note's §5,
    /// which asks for a proper product→recipes index as its own task. This
    /// function is a scan, correct but linear, and it is only ever run on a
    /// refusal path.
    pub fn of<'a, R>(recipes: R, fluid: &str) -> FluidSource
    where
        R: IntoIterator<Item = &'a FactorioRecipe>,
    {
        let mut names: Vec<String> = Vec::new();
        let mut categories: Vec<String> = Vec::new();
        for recipe in recipes {
            if recipe.products.iter().any(|p| p.name == fluid) {
                names.push(recipe.name.clone());
                categories.push(recipe.category.clone());
            }
        }
        if names.is_empty() {
            return FluidSource::Nothing;
        }
        names.sort();
        names.dedup();
        categories.sort();
        categories.dedup();
        FluidSource::Recipes {
            recipes: names,
            categories,
        }
    }

    pub fn of_world(world: &FactorioSurface, fluid: &str) -> FluidSource {
        let recipes: Vec<FactorioRecipe> = world
            .globals
            .recipes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        FluidSource::of(recipes.iter(), fluid)
    }
}

impl std::fmt::Display for FluidSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FluidSource::Nothing => {
                write!(f, "nothing in this world produces it")
            }
            FluidSource::Recipes {
                recipes,
                categories,
            } => write!(
                f,
                "it would come from {} (category {}), which no character can craft",
                recipes.join(", "),
                categories.join(", ")
            ),
        }
    }
}

/// The refusal for asking a character to hold a fluid.
///
/// # One tier, not four, and that is the design
///
/// `method::extract` refuses in four ordered tiers because a reader can act on
/// each in turn: chart the ground, find a drill, research it, wait for the
/// method. A fluid asked of a character has exactly one wall and it is
/// **first, always** -- no change to the world, no research and no charting
/// gets past it, because the goal shape itself is the thing that cannot be
/// satisfied. Emitting the downstream facts as separate tiers would imply a
/// ladder that does not exist, so they are attached to this one refusal as
/// [`FluidSource`] instead.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FluidRefusal {
    #[error(
        "{fluid} is a fluid, and no character inventory can hold a fluid in any amount, so \
         `have {count} {fluid}` is not unsatisfiable -- it is inexpressible. A fluid lives in a \
         fluidbox (a pipe, a storage tank, or a machine's own), and this planner has no fluidbox \
         concept, no inventory slot that addresses one, and no action that moves one. {produced_by}"
    )]
    NotCarryable {
        fluid: String,
        count: u32,
        produced_by: FluidSource,
    },
}

#[cfg(test)]
mod substance_tests {
    use super::*;
    use factorio_bot_core::serde_json;

    /// Every recipe below is **transcribed byte-for-byte** out of
    /// `crates/core/tests/live-2.1.17-world-snapshot.json`, a real RCON reply
    /// from a Factorio 2.1.17 game checked in by someone else. They are
    /// deserialised rather than built with struct literals for two reasons:
    /// `FactorioRecipe::energy` is a `noisy_float` this crate does not depend
    /// on directly (the same reason `method::have`'s steel fixture does it),
    /// and a JSON blob can be diffed against the capture by a reader who
    /// distrusts me.
    ///
    /// Note `products` here carries `independent_probability` /
    /// `shared_probability` and **no `probability` field at all** -- that is
    /// the 2.1 shape, and it is what the game actually sends.
    fn parse(json: &str) -> FactorioRecipe {
        serde_json::from_str(json).expect("a recipe from the live capture parses")
    }

    /// Two fluid ingredients, one item product, category `chemistry`.
    fn sulfur() -> FactorioRecipe {
        parse(
            r#"{
              "name": "sulfur", "valid": true, "enabled": false, "hidden": false,
              "energy": 1, "order": "b[chemistry]-c[sulfur]", "category": "chemistry",
              "ingredients": [
                { "name": "water", "ingredient_type": "fluid", "amount": 30 },
                { "name": "petroleum-gas", "ingredient_type": "fluid", "amount": 30 }
              ],
              "products": [
                { "name": "sulfur", "product_type": "item", "amount": 2,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "raw-material"
            }"#,
        )
    }

    /// One fluid in, one fluid out, category `oil-processing`.
    fn basic_oil_processing() -> FactorioRecipe {
        parse(
            r#"{
              "name": "basic-oil-processing", "valid": true, "enabled": false, "hidden": false,
              "energy": 5, "order": "a[oil-processing]-a[basic-oil-processing]",
              "category": "oil-processing",
              "ingredients": [
                { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "petroleum-gas", "product_type": "fluid", "amount": 45,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "fluid-recipes"
            }"#,
        )
    }

    /// Two items, one item, category `crafting`. The control: nothing about
    /// this recipe should reach the fluid path.
    fn iron_gear() -> FactorioRecipe {
        parse(
            r#"{
              "name": "iron-gear-wheel", "valid": true, "enabled": true, "hidden": false,
              "energy": 0.5, "order": "a[basic-intermediates]-a[iron-gear-wheel]",
              "category": "crafting",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 2 }
              ],
              "products": [
                { "name": "iron-gear-wheel", "product_type": "item", "amount": 1,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "intermediate-product"
            }"#,
        )
    }

    /// The mixed bill: two items and a fluid in, a fluid out.
    fn sulfuric_acid() -> FactorioRecipe {
        parse(
            r#"{
              "name": "sulfuric-acid", "valid": true, "enabled": false, "hidden": false,
              "energy": 1, "order": "c[oil-products]-b[sulfuric-acid]", "category": "chemistry",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 1 },
                { "name": "sulfur", "ingredient_type": "item", "amount": 5 },
                { "name": "water", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "sulfuric-acid", "product_type": "fluid", "amount": 50,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "fluid-recipes"
            }"#,
        )
    }

    fn recipes() -> Vec<FactorioRecipe> {
        vec![
            sulfur(),
            basic_oil_processing(),
            iron_gear(),
            sulfuric_acid(),
        ]
    }

    /// The item prototypes the capture really carries for these names. It has
    /// 342 of them and **not one is a fluid** -- asserted over the whole
    /// capture in `tests/substance_live_capture.rs`.
    fn table() -> SubstanceTable {
        let recipes = recipes();
        SubstanceTable::from_parts(recipes.iter(), ["sulfur", "iron-plate", "iron-gear-wheel"])
    }

    /// The premise, checked so the assertions below cannot pass vacuously:
    /// these transcriptions really do declare fluids.
    #[test]
    fn the_fixtures_really_declare_fluids() {
        let all = recipes();
        let declared: Vec<&str> = all
            .iter()
            .flat_map(|r| {
                declared_types(r)
                    .into_iter()
                    .filter(|(_, t)| *t == FLUID_TYPE)
                    .map(|(n, _)| n)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            declared.len(),
            6,
            "water+gas, crude-oil, gas, water, acid: {declared:?}"
        );
    }

    #[test]
    fn a_fluid_ingredient_is_a_fluid() {
        let t = table();
        assert_eq!(t.of("water"), Some(Substance::Fluid));
        assert_eq!(t.of("petroleum-gas"), Some(Substance::Fluid));
        assert_eq!(t.of("crude-oil"), Some(Substance::Fluid));
        assert_eq!(t.of("sulfuric-acid"), Some(Substance::Fluid));
        assert!(t.is_fluid("crude-oil"));
    }

    #[test]
    fn an_items_product_is_an_item() {
        let t = table();
        assert_eq!(t.of("sulfur"), Some(Substance::Item));
        assert_eq!(t.of("iron-plate"), Some(Substance::Item));
        assert_eq!(t.of("iron-gear-wheel"), Some(Substance::Item));
        assert!(!t.is_fluid("iron-plate"));
    }

    /// The third answer. A name nothing describes is *unknown*, and unknown
    /// must not become either kind by accident: the crate's fixture worlds are
    /// full of such names, and both wrong answers are silent.
    #[test]
    fn an_unmentioned_name_is_unknown_not_an_item() {
        let t = table();
        assert_eq!(t.of("wood"), None, "no evidence either way");
        assert!(
            !t.is_fluid("wood"),
            "and is_fluid is positive-evidence only"
        );
    }

    /// A capture taken before the mod forwarded the type field declares `""`.
    /// That is no evidence, not evidence of an item -- so the name stays
    /// unknown and a later caller can tell the difference.
    #[test]
    fn a_blank_declared_type_is_no_evidence() {
        let blank = parse(
            r#"{
              "name": "mystery", "valid": true, "enabled": true, "hidden": false,
              "energy": 1, "order": "", "category": "crafting",
              "ingredients": [ { "name": "goo", "amount": 1 } ],
              "products": [ { "name": "thing", "amount": 1, "probability": 1.0 } ],
              "group": "", "subgroup": ""
            }"#,
        );
        let recipes = [blank];
        let t = SubstanceTable::from_parts(recipes.iter(), std::iter::empty());
        assert_eq!(t.of("goo"), None);
        assert_eq!(t.of("thing"), None);
        assert_eq!(t.len(), 0, "nothing was classified at all");
    }

    #[test]
    fn fluids_are_listed_in_name_order() {
        let t = table();
        assert_eq!(
            t.fluids().collect::<Vec<_>>(),
            vec!["crude-oil", "petroleum-gas", "sulfuric-acid", "water"]
        );
    }

    /// The split `ingredients_of` cannot do: `sulfuric-acid`'s bill is two
    /// items and one fluid, and only the two items are things a bot could be
    /// asked to carry. Order is the recipe's own, so `iron-plate` precedes
    /// `sulfur`.
    #[test]
    fn a_bill_separates_what_a_bot_could_carry() {
        let bill = split_bill(&table(), &sulfuric_acid());
        assert_eq!(
            bill.items,
            vec![("iron-plate".to_string(), 1), ("sulfur".to_string(), 5)]
        );
        assert_eq!(bill.fluids, vec![("water".to_string(), 100)]);
        assert!(bill.has_fluids());
    }

    /// The control: an all-item recipe produces no fluid half at all.
    #[test]
    fn an_item_recipes_bill_has_no_fluids() {
        let bill = split_bill(&table(), &iron_gear());
        assert_eq!(bill.items, vec![("iron-plate".to_string(), 2)]);
        assert!(!bill.has_fluids());
    }

    /// A blank declaration falls back to the table rather than to "item".
    #[test]
    fn a_bill_falls_back_to_the_table_for_a_blank_type() {
        let older = parse(
            r#"{
              "name": "older", "valid": true, "enabled": true, "hidden": false,
              "energy": 1, "order": "", "category": "chemistry",
              "ingredients": [ { "name": "water", "amount": 30 } ],
              "products": [], "group": "", "subgroup": ""
            }"#,
        );
        let bill = split_bill(&table(), &older);
        assert!(bill.items.is_empty(), "water is not an item");
        assert_eq!(bill.fluids, vec![("water".to_string(), 30)]);
    }

    #[test]
    fn a_source_names_the_recipe_and_its_category() {
        let recipes = recipes();
        assert_eq!(
            FluidSource::of(recipes.iter(), "petroleum-gas"),
            FluidSource::Recipes {
                recipes: vec!["basic-oil-processing".to_string()],
                categories: vec!["oil-processing".to_string()],
            }
        );
        assert_eq!(
            FluidSource::of(recipes.iter(), "lava"),
            FluidSource::Nothing
        );
    }

    /// The refusal has to name the fluid, the count, and the wall behind it.
    #[test]
    fn the_refusal_names_the_fluid_the_count_and_the_wall() {
        let recipes = recipes();
        let refusal = FluidRefusal::NotCarryable {
            fluid: "petroleum-gas".to_string(),
            count: 100,
            produced_by: FluidSource::of(recipes.iter(), "petroleum-gas"),
        };
        let said = refusal.to_string();
        assert!(said.contains("petroleum-gas is a fluid"), "{said}");
        assert!(said.contains("have 100 petroleum-gas"), "{said}");
        assert!(said.contains("inexpressible"), "{said}");
        assert!(said.contains("fluidbox"), "{said}");
        assert!(said.contains("basic-oil-processing"), "{said}");
        assert!(said.contains("category oil-processing"), "{said}");
    }

    /// A fluid nothing produces still refuses, and says so rather than
    /// trailing off.
    #[test]
    fn a_sourceless_fluid_still_refuses_clearly() {
        let said = FluidRefusal::NotCarryable {
            fluid: "lava".to_string(),
            count: 1,
            produced_by: FluidSource::Nothing,
        }
        .to_string();
        assert!(said.contains("nothing in this world produces it"), "{said}");
    }

    /// Clean data disagrees about nothing. The interesting half of this
    /// assertion is in `tests/substance_live_capture.rs`, against the whole
    /// capture; here it only fixes the shape.
    #[test]
    fn clean_data_has_no_disagreements() {
        let recipes = recipes();
        assert_eq!(
            SubstanceTable::disagreements(
                recipes.iter(),
                ["sulfur", "iron-plate", "iron-gear-wheel"]
            ),
            vec![]
        );
    }

    /// Dirty data is reported, not resolved. A name declared a fluid by a
    /// recipe and also present in `item_prototypes` is exactly the case the
    /// tier order would otherwise settle in silence.
    #[test]
    fn a_contradiction_is_reported() {
        let recipes = [basic_oil_processing()];
        let found = SubstanceTable::disagreements(recipes.iter(), ["petroleum-gas", "iron-plate"]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "petroleum-gas");
        assert!(found[0].why.contains("item_prototypes"), "{:?}", found[0]);
    }
}
