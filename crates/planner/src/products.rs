//! Which recipes produce a given *product*, and a refusal by name when none
//! this planner can run does.
//!
//! # The defect this exists for
//!
//! `method::util::recipe_for` looks a product up **by recipe name**:
//!
//! ```ignore
//! state.base().recipes.get(item)
//! ```
//!
//! A recipe table is keyed by recipe name, and a product is not a recipe. That
//! lookup is right only where the two happen to coincide, and it answers
//! `None` — silently — everywhere else. Measured against
//! `crates/core/tests/live-2.1.17-world-snapshot.json`, a real Factorio 2.1.17
//! RCON capture nobody on this branch wrote:
//!
//! | | count |
//! |---|---|
//! | recipes | 662 |
//! | recipes named after one of their own products | **268** |
//! | recipes that are **not** | **394** |
//! | distinct products | 330 |
//! | products with **no** same-named recipe (`recipe_for` → `None`) | **62** |
//! | products made by more than one recipe | **155** |
//! | recipes with more than one product | **230** |
//!
//! The 62 include every raw resource (`iron-ore`, `coal`, `stone`, `wood`),
//! every early fluid (`crude-oil`, `petroleum-gas`, `light-oil`, `heavy-oil`,
//! `steam`, `water`) and real *items* the planner will eventually want
//! (`solid-fuel`, `uranium-235`, `uranium-238`). So this is not a fluid
//! problem that happens to bite fluids first; it is a lookup that is wrong in
//! general, and the fluid frontier is simply where it starts to matter.
//!
//! # Why nothing has broken yet, stated precisely
//!
//! Within the two categories this planner actually runs —
//! [`util::CRAFTING_CATEGORY`] (a character's hands) and
//! [`util::SMELTING_CATEGORY`] (a furnace) — the same capture says:
//!
//! * 194 products, and **zero** of them are made by more than one recipe;
//! * **zero** of them lack a same-named recipe;
//! * **zero** recipes in those categories have more than one product.
//!
//! In other words `recipe_for` is *accidentally exact* over exactly the set
//! the planner can reach today, and wrong immediately outside it. That
//! measurement is what makes this module safe to introduce without moving a
//! single plan, and it is re-measured on every run of
//! `tests/product_index_live_capture.rs` rather than trusted from this
//! paragraph.
//!
//! # One-to-many in both directions
//!
//! * `petroleum-gas` is produced by five recipes — `advanced-oil-processing`,
//!   `basic-oil-processing`, `coal-liquefaction`, `light-oil-cracking` and
//!   `empty-petroleum-gas-barrel`.
//! * `advanced-oil-processing` produces three — 25 heavy-oil, 45 light-oil and
//!   55 petroleum-gas, transcribed from
//!   `workspace/server/data/base/prototypes/recipe.lua` itself.
//!
//! A map keyed one-to-one cannot express either, which is why this is an index
//! rather than a fix to the existing `get`.
//!
//! # A silent `None` is the defect, so this does not answer silently
//!
//! [`ProductIndex::sole_recipe_producing`] returns a [`ProductRefusal`] with
//! three ordered tiers, in the shape `method::extract` uses: each names the
//! next thing that is missing, and a reader can act on each in turn.
//!
//! 1. [`ProductRefusal::NotProduced`] — no recipe in this world produces the
//!    name at all. The message says whether it is an item (so it comes out of
//!    the ground), a fluid, or a name this world has never heard of.
//! 2. [`ProductRefusal::NoRunnableCategory`] — recipes produce it, and every
//!    one of them is in a category this planner cannot run. Names them and
//!    their categories, which is the whole diagnosis for every fluid.
//! 3. [`ProductRefusal::Ambiguous`] — several *runnable* recipes produce it
//!    and nothing here can choose. Measured empty in vanilla 2.1.17 for
//!    crafting+smelting; it exists so that a modded world says so instead of
//!    having this module pick the alphabetically first one and call it an
//!    answer.
//!
//! # Cost
//!
//! Building walks every recipe once and clones it, which is 662 clones on a
//! real capture. **Build one per expansion, not one per lookup** — the same
//! rule [`crate::substance::SubstanceTable`] states, for the same reason. The
//! recipe table lives in `PlanState::base()`, which no plan overlay touches,
//! so an index is valid for a whole expansion.
//!
//! # Wiring
//!
//! [`NoProducer`] is a [`Method`] that answers [`Method::refusal`] and claims
//! nothing, so registering it can change what a caller is *told* and cannot
//! change any plan. It is **not** in `method::have::default_registry` — that
//! file belongs to another task right now — so today its only callers are its
//! own tests, which drive it through the real [`MethodRegistry`] and through
//! [`crate::method::expand`]. See the branch report.

use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::util::{CRAFTING_CATEGORY, SMELTING_CATEGORY};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use crate::substance::{Substance, SubstanceTable};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::types::FactorioRecipe;
use std::collections::{BTreeMap, BTreeSet};

/// Which recipe categories a caller is willing to run.
///
/// Explicit rather than a closure so that a refusal can *name* what it
/// admitted. A predicate can say "no candidate passed"; this can say "none of
/// these five is crafting or smelting", which is the sentence that tells a
/// reader an oil refinery is the missing prerequisite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Categories {
    /// `None` admits everything. An empty set admits nothing, and is a
    /// legitimate (if useless) thing to ask for.
    only: Option<BTreeSet<String>>,
}

impl Categories {
    /// Admit every category, including the ones no machine in this planner
    /// exists for. Use for a survey; never for choosing a recipe to execute.
    pub fn any() -> Self {
        Categories { only: None }
    }

    /// Admit exactly these.
    pub fn only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Categories {
            only: Some(names.into_iter().map(Into::into).collect()),
        }
    }

    /// The two categories this planner has a machine for: `crafting`, which a
    /// character's hands run, and `smelting`, which a furnace runs.
    ///
    /// Built from [`CRAFTING_CATEGORY`] and [`SMELTING_CATEGORY`] rather than
    /// from string literals, so that widening either gate widens this too
    /// instead of leaving a second copy behind — the same reason those
    /// constants are named at all.
    pub fn planner_runs() -> Self {
        Categories::only([CRAFTING_CATEGORY, SMELTING_CATEGORY])
    }

    pub fn admits(&self, category: &str) -> bool {
        match &self.only {
            None => true,
            Some(set) => set.contains(category),
        }
    }

    /// The admitted categories in name order; empty when everything is
    /// admitted (ask [`Self::admits_everything`] to tell the two apart).
    pub fn names(&self) -> Vec<String> {
        self.only
            .as_ref()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn admits_everything(&self) -> bool {
        self.only.is_none()
    }
}

/// One recipe that produces the asked-for product, as a refusal describes it.
///
/// Owned strings rather than borrows: a refusal outlives the index it was
/// derived from, and travels into a [`PlannerError`] that reaches Lua.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub recipe: String,
    pub category: String,
    /// How many of the asked-for product one run yields.
    pub amount: u32,
    /// The game's own `hidden` flag. Reported, never used as a filter: no
    /// existing gate in this planner reads it, and quietly dropping hidden
    /// recipes would be exactly the silent answer this module exists to stop.
    pub hidden: bool,
    /// The world's `enabled` flag. Likewise reported, never filtered —
    /// `method::util::recipe_gate` is the thing that turns a disabled recipe
    /// into "needs this technology", and it needs a recipe to be handed to it
    /// first.
    pub enabled: bool,
}

impl std::fmt::Display for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (category {})", self.recipe, self.category)
    }
}

fn list(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why no single recipe answers "what makes this product".
///
/// Three ordered tiers, each naming the next missing prerequisite, in the
/// shape `method::extract::world_refusal` established. See the module doc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductRefusal {
    /// Tier 1: nothing in this world's recipe table lists it as a product.
    NotProduced {
        product: String,
        /// What the world says the name is, if it says anything. `None` is a
        /// real answer — see [`SubstanceTable::of`] — and the message says so.
        substance: Option<Substance>,
    },
    /// Tier 2: recipes produce it, and none is in an admitted category.
    NoRunnableCategory {
        product: String,
        /// Every producer, in recipe-name order. Never empty: an empty list is
        /// tier 1.
        candidates: Vec<Candidate>,
        /// What the caller was willing to run, in name order. Empty means the
        /// caller admitted everything, which makes this tier unreachable.
        admitted: Vec<String>,
        substance: Option<Substance>,
    },
    /// Tier 3: several admitted recipes produce it, and nothing here can
    /// choose between them.
    Ambiguous {
        product: String,
        /// The admitted producers only, in recipe-name order. Always at least
        /// two.
        candidates: Vec<Candidate>,
    },
}

impl std::fmt::Display for ProductRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProductRefusal::NotProduced { product, substance } => {
                write!(f, "no recipe in this world produces {product}")?;
                match substance {
                    Some(Substance::Item) => write!(
                        f,
                        "; it is an item, so it comes out of the ground -- mine or extract it, \
                         there is nothing to craft"
                    ),
                    Some(Substance::Fluid) => write!(
                        f,
                        "; it is a fluid, and one this world never makes, so it can only be \
                         extracted from the ground"
                    ),
                    None => write!(
                        f,
                        ", and no prototype table in this world mentions the name at all -- \
                         check the spelling, or the mod that was meant to add it"
                    ),
                }
            }
            ProductRefusal::NoRunnableCategory {
                product,
                candidates,
                admitted,
                substance,
            } => {
                write!(
                    f,
                    "{} produce {product} -- {} -- and none is in a category this planner runs \
                     ({})",
                    plural(candidates.len(), "recipe", "recipes"),
                    list(candidates),
                    admitted.join(", "),
                )?;
                if *substance == Some(Substance::Fluid) {
                    write!(
                        f,
                        ". {product} is also a fluid, which no character inventory can hold in \
                         any amount"
                    )?;
                }
                Ok(())
            }
            ProductRefusal::Ambiguous {
                product,
                candidates,
            } => write!(
                f,
                "{} this planner can run produce {product} -- {} -- and nothing here can choose \
                 between them; ask for a recipe by name rather than for the product",
                plural(candidates.len(), "recipe", "recipes"),
                list(candidates),
            ),
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

impl std::error::Error for ProductRefusal {}

impl ProductRefusal {
    /// The product this refusal is about, for a caller that wants the name
    /// without parsing the message.
    pub fn product(&self) -> &str {
        match self {
            ProductRefusal::NotProduced { product, .. }
            | ProductRefusal::NoRunnableCategory { product, .. }
            | ProductRefusal::Ambiguous { product, .. } => product,
        }
    }
}

/// Every recipe in a world, indexed by what it produces.
///
/// Immutable once built. See the module doc for the cost and for why one
/// index is valid for a whole expansion.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProductIndex {
    by_recipe: BTreeMap<String, FactorioRecipe>,
    /// product name → recipe names, sorted and deduplicated. Sorted so that
    /// two runs over the same world produce the same refusal text and the same
    /// choice; a `DashMap` walk order is not stable and this crate's contract
    /// is determinism.
    by_product: BTreeMap<String, Vec<String>>,
    substances: SubstanceTable,
}

impl ProductIndex {
    /// Index a set of recipes, with the item-prototype names the world
    /// declares.
    ///
    /// The item names feed [`SubstanceTable`] only, which is what lets tier 1
    /// say "it is an item, so mine it" rather than "no". Pass an empty
    /// iterator when a caller has no item table; the classifier then falls
    /// back to what recipes declare, and says `None` for a name nothing
    /// mentions.
    pub fn from_parts<'a, R, I>(recipes: R, item_prototype_names: I) -> Self
    where
        R: IntoIterator<Item = &'a FactorioRecipe>,
        I: IntoIterator<Item = &'a str>,
    {
        let recipes: Vec<&FactorioRecipe> = recipes.into_iter().collect();
        let substances = SubstanceTable::from_parts(recipes.iter().copied(), item_prototype_names);

        let mut by_recipe: BTreeMap<String, FactorioRecipe> = BTreeMap::new();
        let mut by_product: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for recipe in recipes {
            for product in &recipe.products {
                by_product
                    .entry(product.name.clone())
                    .or_default()
                    .push(recipe.name.clone());
            }
            by_recipe.insert(recipe.name.clone(), recipe.clone());
        }
        for names in by_product.values_mut() {
            names.sort();
            // A recipe listing the same product twice would otherwise appear
            // twice and read as an ambiguity between a recipe and itself.
            // `coal-liquefaction` is the near miss: it both consumes and
            // produces heavy-oil, which is not this case, but is the shape
            // that made the question worth answering.
            names.dedup();
        }
        ProductIndex {
            by_recipe,
            by_product,
            substances,
        }
    }

    /// Index a live or dumped world.
    pub fn from_world(world: &FactorioSurface) -> Self {
        // Collected rather than streamed: the tables are `DashMap`s whose
        // guards cannot be held across `from_parts`, and the result must not
        // depend on the order they were walked in.
        let recipes: Vec<FactorioRecipe> = world
            .recipes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let items: Vec<String> = world
            .item_prototypes
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        ProductIndex::from_parts(recipes.iter(), items.iter().map(String::as_str))
    }

    /// Index the world a plan state overlays.
    ///
    /// Reads `PlanState::base()` and nothing else: recipes are game data and
    /// no plan overlay adds or removes one.
    pub fn from_state(state: &PlanState) -> Self {
        ProductIndex::from_world(state.base())
    }

    /// Every recipe that lists `product` among its outputs, in recipe-name
    /// order. Empty when nothing does.
    ///
    /// **Produces, not nets.** `coal-liquefaction` consumes 25 heavy-oil and
    /// produces 90, and it appears here; a recipe that consumed more than it
    /// made would appear too. Netting is a caller's question and would need a
    /// caller's context (is the input already in hand?), so it is not decided
    /// here.
    pub fn recipes_producing(&self, product: &str) -> Vec<&FactorioRecipe> {
        self.by_product
            .get(product)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.by_recipe.get(name))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether any recipe at all produces it.
    pub fn produces(&self, product: &str) -> bool {
        self.by_product.contains_key(product)
    }

    /// Every product any recipe makes, in name order.
    pub fn products(&self) -> impl Iterator<Item = &str> {
        self.by_product.keys().map(String::as_str)
    }

    /// A recipe by its own name — the lookup `method::util::recipe_for`
    /// performs, made available honestly, under a name that says what it keys
    /// on.
    pub fn recipe(&self, recipe_name: &str) -> Option<&FactorioRecipe> {
        self.by_recipe.get(recipe_name)
    }

    /// How the world classifies the names this index knows.
    pub fn substances(&self) -> &SubstanceTable {
        &self.substances
    }

    pub fn len(&self) -> usize {
        self.by_recipe.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_recipe.is_empty()
    }

    /// The one recipe in an admitted category that produces `product`, or a
    /// refusal naming which of the three walls was hit.
    ///
    /// Never guesses. A product two admitted recipes make is
    /// [`ProductRefusal::Ambiguous`], not the first one sorted — see the
    /// module doc for why that tier is measured empty in vanilla and kept
    /// anyway.
    pub fn sole_recipe_producing(
        &self,
        product: &str,
        categories: &Categories,
    ) -> Result<&FactorioRecipe, ProductRefusal> {
        let all = self.recipes_producing(product);
        if all.is_empty() {
            return Err(ProductRefusal::NotProduced {
                product: product.to_string(),
                substance: self.substances.of(product),
            });
        }
        let runnable: Vec<&FactorioRecipe> = all
            .iter()
            .copied()
            .filter(|r| categories.admits(&r.category))
            .collect();
        match runnable.len() {
            0 => Err(ProductRefusal::NoRunnableCategory {
                product: product.to_string(),
                candidates: all.iter().map(|r| candidate(r, product)).collect(),
                admitted: categories.names(),
                substance: self.substances.of(product),
            }),
            1 => Ok(runnable[0]),
            _ => Err(ProductRefusal::Ambiguous {
                product: product.to_string(),
                candidates: runnable.iter().map(|r| candidate(r, product)).collect(),
            }),
        }
    }
}

fn candidate(recipe: &FactorioRecipe, product: &str) -> Candidate {
    Candidate {
        recipe: recipe.name.clone(),
        category: recipe.category.clone(),
        amount: crate::method::util::output_per_craft(recipe, product),
        hidden: recipe.hidden,
        enabled: recipe.enabled,
    }
}

/// Says *why* nothing can make an item, when no method claimed the goal.
///
/// # It cannot change a plan, by construction
///
/// [`Method::applicable`] is always false and [`Method::claims`] is always
/// false, so the driver never selects it. It answers [`Method::refusal`],
/// which `expand_goal_body` consults only after every method has declined —
/// i.e. only on a path that was already going to return
/// `PlannerError::NoApplicableMethod`. The most it can do is replace that
/// text with a named one.
///
/// # Register it LAST
///
/// `MethodRegistry::refusal` takes the *first* method that offers one, so a
/// method that knows more (`Mine` knows the ore is uncharted; `Extract` walks
/// a four-tier ladder) must be asked first. This one is the fallback that
/// keeps the driver from saying "no method can satisfy goal: have 100
/// petroleum-gas", which is true and unactionable.
///
/// It is not registered anywhere today — `method::have::default_registry` is
/// another task's file. Its tests build a registry directly and drive
/// [`crate::method::expand`], which is the same path production would take.
pub struct NoProducer;

impl Method for NoProducer {
    fn name(&self) -> &'static str {
        "no-producer"
    }

    /// Never. This method satisfies nothing; see the type doc.
    fn applicable(&self, _goal: &Goal, _state: &PlanState) -> bool {
        false
    }

    /// Never, at any site — belt and braces with `applicable`, because the two
    /// are consulted independently and a future driver change to either alone
    /// must not be able to select this.
    fn claims(&self, _site: crate::method::GoalSite) -> bool {
        false
    }

    fn expand(&self, goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        Err(PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })
    }

    fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        let item = match goal {
            Goal::Have { item, .. } | Goal::Produced { item, .. } => item,
            _ => return None,
        };
        let index = ProductIndex::from_state(&ctx.state);
        match index.sole_recipe_producing(item, &Categories::planner_runs()) {
            // Something can make it. Whatever stopped this goal, it is not
            // the recipe table, and saying anything here would be guessing.
            Ok(_) => None,
            Err(refusal) => Some(PlannerError::ProductNotMakeable(refusal)),
        }
    }
}

#[cfg(test)]
mod product_index_tests {
    use super::*;
    use factorio_bot_core::serde_json;

    /// Recipes transcribed **byte-for-byte** out of
    /// `crates/core/tests/live-2.1.17-world-snapshot.json`, a real RCON reply
    /// from a Factorio 2.1.17 game checked in by an earlier task. Deserialised
    /// rather than built with struct literals because
    /// `FactorioRecipe::energy` is a `noisy_float` this crate does not depend
    /// on directly, and because a JSON blob can be diffed against the capture
    /// by a reader who distrusts me.
    ///
    /// The whole-capture assertions live in
    /// `tests/product_index_live_capture.rs`; these four exist for the shapes
    /// a single test can read.
    fn parse(json: &str) -> FactorioRecipe {
        serde_json::from_str(json).expect("a recipe from the live capture parses")
    }

    /// **Three** fluid products from one recipe — the shape a one-to-one index
    /// cannot hold at all. Cross-checked against
    /// `workspace/server/data/base/prototypes/recipe.lua`, which states
    /// heavy-oil 25, light-oil 45, petroleum-gas 55.
    fn advanced_oil_processing() -> FactorioRecipe {
        parse(
            r#"{
              "name": "advanced-oil-processing", "valid": true, "enabled": false,
              "hidden": false, "energy": 5, "category": "oil-processing",
              "order": "a[oil-processing]-b[advanced-oil-processing]",
              "ingredients": [
                { "name": "water", "ingredient_type": "fluid", "amount": 50 },
                { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "heavy-oil", "product_type": "fluid", "amount": 25,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } },
                { "name": "light-oil", "product_type": "fluid", "amount": 45,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } },
                { "name": "petroleum-gas", "product_type": "fluid", "amount": 55,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "fluid-recipes"
            }"#,
        )
    }

    /// The second producer of petroleum-gas, so the index is one-to-many in
    /// both directions at once.
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

    /// The control: an ordinary `crafting` recipe named after its own product,
    /// which is the only case `recipe_for` gets right.
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

    /// An **item** whose recipe is not named after it, and whose category the
    /// planner does run: `casting-iron-gear-wheel` is `metallurgy`, so the
    /// pair (this and `iron_gear`) is the ambiguity tier-3 exists for once a
    /// caller admits both categories.
    fn casting_iron_gear() -> FactorioRecipe {
        parse(
            r#"{
              "name": "casting-iron-gear-wheel", "valid": true, "enabled": false,
              "hidden": false, "energy": 1, "category": "metallurgy",
              "order": "b[casting]-a[iron-gear-wheel]",
              "ingredients": [
                { "name": "molten-iron", "ingredient_type": "fluid", "amount": 10 }
              ],
              "products": [
                { "name": "iron-gear-wheel", "product_type": "item", "amount": 1,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "intermediate-product"
            }"#,
        )
    }

    fn recipes() -> Vec<FactorioRecipe> {
        vec![
            advanced_oil_processing(),
            basic_oil_processing(),
            iron_gear(),
            casting_iron_gear(),
        ]
    }

    fn index() -> ProductIndex {
        let recipes = recipes();
        ProductIndex::from_parts(recipes.iter(), ["iron-gear-wheel", "iron-plate"])
    }

    /// The premise, so nothing below can pass vacuously: the fixture really
    /// does contain a three-product recipe and two producers of one product.
    #[test]
    fn the_fixture_really_is_many_to_many() {
        let all = recipes();
        let advanced = all
            .iter()
            .find(|r| r.name == "advanced-oil-processing")
            .expect("transcribed above");
        assert_eq!(
            advanced.products.len(),
            3,
            "heavy-oil, light-oil, petroleum-gas: {:?}",
            advanced.products
        );
        let gas_makers = all
            .iter()
            .filter(|r| r.products.iter().any(|p| p.name == "petroleum-gas"))
            .count();
        assert_eq!(gas_makers, 2, "basic and advanced oil processing");
    }

    /// The defect, stated as a test: the recipe table has no key
    /// `petroleum-gas`, which is exactly what `recipe_for` looks for.
    #[test]
    fn no_recipe_is_named_after_the_fluid_it_makes() {
        let i = index();
        assert!(
            i.recipe("petroleum-gas").is_none(),
            "a lookup by recipe name finds nothing -- this is the silent None"
        );
        assert_eq!(
            i.recipes_producing("petroleum-gas")
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            vec!["advanced-oil-processing", "basic-oil-processing"],
            "a lookup by product finds both, in name order"
        );
    }

    #[test]
    fn one_recipe_indexes_under_every_product_it_makes() {
        let i = index();
        for (product, amount) in [("heavy-oil", 25), ("light-oil", 45), ("petroleum-gas", 55)] {
            let makers = i.recipes_producing(product);
            assert!(
                makers.iter().any(|r| r.name == "advanced-oil-processing"),
                "{product} is not indexed to advanced-oil-processing"
            );
            let advanced = makers
                .iter()
                .find(|r| r.name == "advanced-oil-processing")
                .expect("just asserted");
            assert_eq!(
                crate::method::util::output_per_craft(advanced, product),
                amount,
                "{product} per run, from recipe.lua"
            );
        }
    }

    #[test]
    fn products_are_in_name_order_and_deduplicated() {
        let i = index();
        assert_eq!(
            i.products().collect::<Vec<_>>(),
            vec!["heavy-oil", "iron-gear-wheel", "light-oil", "petroleum-gas"]
        );
        assert_eq!(i.len(), 4, "four recipes indexed by their own names");
    }

    #[test]
    fn a_runnable_product_resolves_to_its_one_recipe() {
        let i = index();
        let r = i
            .sole_recipe_producing("iron-gear-wheel", &Categories::planner_runs())
            .expect("crafting admits exactly one of the two");
        assert_eq!(r.name, "iron-gear-wheel");
        assert_eq!(r.category, CRAFTING_CATEGORY);
    }

    #[test]
    fn tier_one_names_the_ground_for_an_item_nothing_makes() {
        let i = index();
        let err = i
            .sole_recipe_producing("iron-plate", &Categories::any())
            .expect_err("the fixture has no recipe producing iron-plate");
        assert_eq!(
            err,
            ProductRefusal::NotProduced {
                product: "iron-plate".to_string(),
                substance: Some(Substance::Item),
            }
        );
        let text = err.to_string();
        assert!(
            text.contains("no recipe in this world produces iron-plate"),
            "{text}"
        );
        assert!(text.contains("mine or extract it"), "{text}");
    }

    #[test]
    fn tier_one_says_so_when_the_world_has_never_heard_the_name() {
        let i = index();
        let err = i
            .sole_recipe_producing("unobtainium", &Categories::any())
            .expect_err("nothing makes it");
        assert_eq!(
            err,
            ProductRefusal::NotProduced {
                product: "unobtainium".to_string(),
                substance: None,
            }
        );
        assert!(err.to_string().contains("check the spelling"), "{err}");
    }

    #[test]
    fn tier_two_names_every_producer_and_its_category() {
        let i = index();
        let err = i
            .sole_recipe_producing("petroleum-gas", &Categories::planner_runs())
            .expect_err("oil-processing is not a category this planner runs");
        let ProductRefusal::NoRunnableCategory {
            candidates,
            admitted,
            substance,
            ..
        } = &err
        else {
            panic!("expected tier 2, got {err:?}");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|c| (c.recipe.as_str(), c.category.as_str(), c.amount))
                .collect::<Vec<_>>(),
            vec![
                ("advanced-oil-processing", "oil-processing", 55),
                ("basic-oil-processing", "oil-processing", 45),
            ]
        );
        assert_eq!(
            admitted,
            &vec!["crafting".to_string(), "smelting".to_string()]
        );
        assert_eq!(*substance, Some(Substance::Fluid));
        let text = err.to_string();
        assert!(text.contains("2 recipes produce petroleum-gas"), "{text}");
        assert!(
            text.contains("basic-oil-processing (category oil-processing)"),
            "{text}"
        );
        assert!(text.contains("no character inventory can hold"), "{text}");
    }

    #[test]
    fn tier_three_refuses_to_choose_between_two_runnable_recipes() {
        let i = index();
        let both = Categories::only(["crafting", "metallurgy"]);
        let err = i
            .sole_recipe_producing("iron-gear-wheel", &both)
            .expect_err("two admitted recipes make it");
        let ProductRefusal::Ambiguous { candidates, .. } = &err else {
            panic!("expected tier 3, got {err:?}");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.recipe.as_str())
                .collect::<Vec<_>>(),
            vec!["casting-iron-gear-wheel", "iron-gear-wheel"]
        );
        assert!(
            err.to_string()
                .contains("nothing here can choose between them"),
            "{err}"
        );
    }

    /// Tier order is a claim, so it is asserted: a product nothing makes is
    /// tier 1 even when the caller admits nothing, rather than tier 2 with an
    /// empty candidate list.
    #[test]
    fn tier_one_beats_tier_two() {
        let i = index();
        let err = i
            .sole_recipe_producing("iron-plate", &Categories::only(Vec::<String>::new()))
            .expect_err("nothing makes it");
        assert!(matches!(err, ProductRefusal::NotProduced { .. }), "{err:?}");
    }

    #[test]
    fn any_admits_a_category_the_planner_cannot_run() {
        let i = index();
        let r = i
            .sole_recipe_producing("heavy-oil", &Categories::any())
            .expect("exactly one recipe makes heavy-oil in the fixture");
        assert_eq!(r.name, "advanced-oil-processing");
    }
}

/// [`NoProducer`] driven through the real driver, not called directly.
///
/// A module with no caller is a hypothesis
/// (`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`), and
/// `method::have::default_registry` -- the one place this would be registered
/// in production -- belongs to another task this week. So these tests build a
/// [`MethodRegistry`] themselves and go in through [`crate::method::expand`],
/// which is the *same* path `goal.plan` takes: the refusal has to survive
/// `MethodRegistry::refusal`, `expand_goal_body`'s fallback and the
/// `PlannerError` conversion, none of which a direct call would exercise.
#[cfg(test)]
mod no_producer_driver_tests {
    use super::*;
    use crate::goal::Holder;
    use crate::ids::BotId;
    use crate::method::{MethodRegistry, expand};
    use factorio_bot_core::factorio::world::FactorioSurface;
    use factorio_bot_core::serde_json;
    use factorio_bot_core::types::{FactorioItemPrototype, FactorioRecipe};
    use std::sync::Arc;

    /// A world whose only recipe is `basic-oil-processing` -- transcribed from
    /// the live capture in the module's other test -- plus one ordinary item
    /// prototype so the substance classifier has an item table to speak from.
    fn oil_state() -> PlanState {
        let recipe: FactorioRecipe = serde_json::from_str(
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
        .expect("the recipe fixture parses");
        let item: FactorioItemPrototype = serde_json::from_str(
            r#"{
              "name": "iron-plate", "item_type": "item", "stack_size": 100,
              "fuel_value": 0, "place_result": "",
              "group": "intermediate-products", "subgroup": "raw-material"
            }"#,
        )
        .expect("the item fixture parses");
        let world = FactorioSurface::new();
        world.update_recipes(vec![recipe]).expect("update_recipes");
        world
            .update_item_prototypes(vec![item])
            .expect("update_item_prototypes");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    fn have(item: &str) -> Goal {
        Goal::Have {
            item: item.to_string(),
            count: 100,
            whose: Holder::Anyone,
        }
    }

    /// The premise: with no method registered at all, the driver's own answer
    /// is the unactionable one. Asserted so the test below cannot pass because
    /// the goal happened to succeed.
    #[test]
    fn without_the_method_the_driver_says_only_no_method_can_satisfy() {
        let state = oil_state();
        let registry = MethodRegistry::new();
        let err = expand(&[have("petroleum-gas")], &state, &registry, BotId(1))
            .expect_err("nothing can satisfy this");
        assert!(
            matches!(err, PlannerError::NoApplicableMethod { .. }),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "no method can satisfy goal: have 100 petroleum-gas (anyone)"
        );
    }

    #[test]
    fn with_the_method_the_driver_names_the_recipe_and_its_category() {
        let state = oil_state();
        let registry = MethodRegistry::new().with(Box::new(NoProducer));
        let err = expand(&[have("petroleum-gas")], &state, &registry, BotId(1))
            .expect_err("still nothing can satisfy it -- but now it says why");
        assert!(
            matches!(err, PlannerError::ProductNotMakeable(_)),
            "{err:?}"
        );
        let text = err.to_string();
        assert!(
            text.contains("basic-oil-processing (category oil-processing)"),
            "{text}"
        );
        assert!(text.contains("crafting, smelting"), "{text}");
        assert!(text.contains("no character inventory can hold"), "{text}");
    }

    /// The method must not answer for an item this world simply cannot make
    /// *by any route it knows*, in a way that contradicts a method that does
    /// know: it speaks only about the recipe table. `iron-plate` here has no
    /// recipe at all, so tier 1 fires and says "mine it" -- which is the
    /// actionable truth, and is why registration order matters (see the type
    /// doc).
    #[test]
    fn an_item_with_no_recipe_is_told_to_come_out_of_the_ground() {
        let state = oil_state();
        let registry = MethodRegistry::new().with(Box::new(NoProducer));
        let err = expand(&[have("iron-plate")], &state, &registry, BotId(1))
            .expect_err("no recipe makes iron-plate in this world");
        let text = err.to_string();
        assert!(text.contains("mine or extract it"), "{text}");
    }

    /// It stays silent when the recipe table has an answer, so that a method
    /// which knows the *real* reason is never spoken over.
    #[test]
    fn it_says_nothing_about_a_product_this_planner_can_make() {
        let state = oil_state();
        let ctx_goal = have("petroleum-gas");
        let index = ProductIndex::from_state(&state);
        // The premise for this test: the fixture really would resolve a
        // craftable product, so a `None` below is the method declining rather
        // than the index failing.
        assert!(
            index
                .sole_recipe_producing("petroleum-gas", &Categories::planner_runs())
                .is_err()
        );
        assert!(matches!(ctx_goal, Goal::Have { .. }));

        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "iron-gear-wheel", "valid": true, "enabled": true, "hidden": false,
              "energy": 0.5, "order": "a", "category": "crafting",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 2 }
              ],
              "products": [
                { "name": "iron-gear-wheel", "product_type": "item", "amount": 1,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "g", "subgroup": "s"
            }"#,
        )
        .expect("parses");
        let world = FactorioSurface::new();
        world.update_recipes(vec![recipe]).expect("update_recipes");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let registry = MethodRegistry::new().with(Box::new(NoProducer));
        let err = expand(&[have("iron-gear-wheel")], &state, &registry, BotId(1))
            .expect_err("no method makes anything here");
        assert!(
            matches!(err, PlannerError::NoApplicableMethod { .. }),
            "the method must stay quiet when a runnable recipe exists: {err:?}"
        );
    }
}
