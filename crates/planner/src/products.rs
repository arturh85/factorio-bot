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
use crate::method::machine::MachineTable;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use crate::substance::{Substance, SubstanceTable};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::types::{FactorioRecipe, Position, Rect};

/// The prototype category Space Age's `X-recycling` recipes are in.
///
/// **A category name is game data; a mod name is not.** This is the one
/// discriminator in the shipped prototypes that separates *destroying* a thing
/// from *making* one, and it survives a mod that adds recycling recipes of its
/// own because the mod puts them in this category too. It does not survive a
/// mod inventing a second destructive category under another name -- that is a
/// real limit and there is nothing in the prototype data that would close it.
pub(crate) const RECYCLING_CATEGORY: &str = "recycling";

/// How far from the origin [`ProductIndex::from_world`] looks for ground that
/// yields a fluid.
///
/// The same 256 tiles `score-map` scores a map over, and for the same reason:
/// far enough to find the lake a plant would be built on, small enough that
/// the scan is one bounded quad-tree query rather than a walk of every charted
/// tile. A fluid whose only pool is outside it is missed, and missing a seed
/// makes the preference **less** decisive, never wrong -- see
/// [`ProductIndex::supply`].
const GROUND_FLUID_SCAN_RADIUS: f64 = 256.0;
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

    /// Every category this planner has a machine for **in this world**.
    ///
    /// Was a two-element constant (`crafting`, `smelting`) until 2026-09-07,
    /// and the note that ended that era said why: those were not a scope
    /// decision, they were the two categories reachable without knowing what
    /// a machine crafts. `LuaEntityPrototype.crafting_categories` now crosses
    /// the bridge, so the set is **derived** — from
    /// [`MachineTable::runnable_categories`], which is the same computation
    /// [`crate::method::fabricate::Fabricate`] uses to name the machine it
    /// stands up. One encoding, so the set a refusal names and the set a
    /// method acts on cannot drift.
    ///
    /// On a world model that predates the field — every archived dump — the
    /// table declares nothing and this answers exactly the old two, so no
    /// archived plan moves.
    pub fn planner_runs(machines: &MachineTable) -> Self {
        Categories::only(machines.runnable_categories())
    }

    /// The two categories the planner brings with it, for a caller with no
    /// world: `crafting` (hands) and `smelting` (a furnace).
    ///
    /// **Not a second copy of the rule.** It is
    /// `planner_runs(&MachineTable::default())` — an empty table is a world
    /// that declared nothing — spelled out so that a test which has no
    /// prototypes says what it means.
    pub fn planner_brings() -> Self {
        Categories::planner_runs(&MachineTable::default())
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
        /// Ingredients of the refused recipes that this surface cannot supply
        /// **either way** — no recipe in a runnable category produces them,
        /// and they are not a resource here. Empty when the ingredients are
        /// all locally reachable, or when nothing filled it in.
        ///
        /// **This is what turns a misleading refusal into an honest one.**
        /// Every off-world science pack refuses at this tier, and the message
        /// without this clause reads as "add a machine for category organic" —
        /// so a reader goes and looks at biochambers. A biochamber on Nauvis
        /// still cannot make `agricultural-science-pack`, because its
        /// ingredients are `bioflux` and `pentapod-egg`, which come from
        /// Gleba. The category is true and the conclusion it invites is
        /// wrong, which is the same defect shape as an occupancy error naming
        /// terrain for a footprint the game refused.
        unreachable_inputs: Vec<String>,
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

    // ---- the four below exist only when a caller NAMED a recipe ----
    //
    // They are the answers to `Goal::Produced::via`, which is the vocabulary
    // tier 3 asks for in as many words ("ask for a recipe by name rather than
    // for the product"). Each one is reachable only from `Some(name)`, never
    // from `None`: absence of a choice is not a wrong choice, and conflating
    // the two is the eighth instance of a mistake this repo has catalogued
    // seven times.
    /// The caller named a recipe this world's recipe table does not have.
    ///
    /// Distinct from [`Self::NotProduced`], which is about the *product*: the
    /// product may be perfectly makeable and the name simply misspelt, so the
    /// recipes that do produce it are listed.
    NoSuchRecipe {
        product: String,
        /// Exactly what the caller wrote.
        recipe: String,
        /// Every recipe that produces the product, in recipe-name order.
        /// Empty is legitimate -- the product may be unmakeable too.
        producers: Vec<Candidate>,
    },

    /// The named recipe exists and does not produce the asked-for product.
    ///
    /// **Refused, never quietly replaced with the planner's own pick.** A
    /// caller that names `iron-gear-wheel` for a goal about `copper-plate`
    /// has made a mistake that a plan built from the other recipe would hide.
    RecipeDoesNotProduce {
        product: String,
        recipe: String,
        /// What the named recipe actually produces, in the game's own order.
        produces: Vec<String>,
        producers: Vec<Candidate>,
    },

    /// The named recipe produces the product, and its category has no machine
    /// in this world.
    ///
    /// **The machine reason, not the ambiguity one.** Naming a recipe answers
    /// "which of these", so tier 3 is unreachable here by construction; what
    /// is left to say is the thing [`crate::method::machine::MachineTable`]
    /// says, and it is quoted verbatim rather than paraphrased so that "the
    /// model predates `crafting_categories`" is not flattened into "no
    /// machine runs this".
    NamedRecipeNotRunnable {
        product: String,
        recipe: String,
        category: String,
        /// The machine table's own words.
        machine: crate::method::machine::MachineRefusal,
    },

    /// The named recipe is one the planner runs in a character's hands or in
    /// a furnace, and those two methods choose their recipe by **product
    /// name** and cannot be told otherwise.
    ///
    /// # Why this is refused rather than ignored
    ///
    /// `HandCraft` and `Smelt` reach `method::util::recipe_for`, a lookup
    /// keyed on the *recipe* name that happens to be exact over
    /// `crafting`+`smelting` **on the game this project runs** -- Factorio
    /// 2.1.17 with Space Age, which is what
    /// `crates/core/tests/live-2.1.17-world-snapshot.json` captured. Measured
    /// there: zero products in those two categories are made by more than one
    /// recipe, and zero lack a same-named one. So here a `via` naming a
    /// crafting recipe is either the one they would pick -- allowed, and a
    /// no-op -- or one they cannot honour. Accepting the second and planning
    /// the first would be the silent substitution this whole module exists to
    /// stop.
    ///
    /// **Reachable only under a mod that adds a second `crafting` recipe for
    /// an existing product.** Space Age itself does not: its extra producers
    /// (`casting-*`, `*-recycling`, the asteroid crushers) are all in other
    /// categories, which is why they surface as
    /// [`Self::NamedRecipeNotRunnable`] rather than here.
    NamedRecipeNotHonoured {
        product: String,
        recipe: String,
        category: String,
        /// The recipe those methods would run instead, if any.
        would_run: Option<String>,
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
                unreachable_inputs,
            } => {
                write!(
                    f,
                    "{} produce {product} -- {} -- and none is in a category this planner runs \
                     ({})",
                    plural(candidates.len(), "recipe", "recipes"),
                    list(candidates),
                    admitted.join(", "),
                )?;
                if !unreachable_inputs.is_empty() {
                    write!(
                        f,
                        ". Its {} {} {} not producible here and not a resource on this \
                         surface, so this is a SUPPLY problem rather than a missing machine -- \
                         a machine for that category would still have nothing to feed it",
                        if unreachable_inputs.len() == 1 {
                            "ingredient"
                        } else {
                            "ingredients"
                        },
                        list_plain(unreachable_inputs),
                        if unreachable_inputs.len() == 1 {
                            "is"
                        } else {
                            "are"
                        },
                    )?;
                }
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
            ProductRefusal::NoSuchRecipe {
                product,
                recipe,
                producers,
            } => {
                write!(
                    f,
                    "this goal asks for {product} via the recipe {recipe}, and no recipe of that \
                     name exists in this world"
                )?;
                if producers.is_empty() {
                    write!(f, "; nothing here produces {product} either")
                } else {
                    write!(
                        f,
                        ". {} produce {product}: {}",
                        plural(producers.len(), "recipe", "recipes"),
                        list(producers),
                    )
                }
            }
            ProductRefusal::RecipeDoesNotProduce {
                product,
                recipe,
                produces,
                producers,
            } => {
                write!(
                    f,
                    "this goal asks for {product} via the recipe {recipe}, and {recipe} does not \
                     produce {product} -- it produces {}",
                    if produces.is_empty() {
                        "nothing at all".to_string()
                    } else {
                        produces.join(", ")
                    },
                )?;
                if producers.is_empty() {
                    write!(f, "; nothing in this world produces {product}")
                } else {
                    write!(
                        f,
                        ". {} produce {product}: {}",
                        plural(producers.len(), "recipe", "recipes"),
                        list(producers),
                    )
                }
            }
            ProductRefusal::NamedRecipeNotRunnable {
                product,
                recipe,
                category,
                machine,
            } => write!(
                f,
                "this goal asks for {product} via the recipe {recipe}, which does produce it \
                 -- but {recipe} is category {category}, and {machine}"
            ),
            ProductRefusal::NamedRecipeNotHonoured {
                product,
                recipe,
                category,
                would_run,
            } => write!(
                f,
                "this goal asks for {product} via the recipe {recipe}, which is category \
                 {category} -- and the methods that run {category} choose their recipe by \
                 product name, so they cannot be told to use {recipe}. {}",
                match would_run {
                    Some(other) => format!(
                        "they would run {other} instead, which is a different recipe, so this \
                         goal is refused rather than silently answered with it"
                    ),
                    None => format!(
                        "they would find no recipe named {product} at all, so there is nothing \
                         they could run"
                    ),
                },
            ),
        }
    }
}

/// Ingredients of `product`'s recipes that this surface cannot supply at all.
///
/// An ingredient counts as unsupplyable when **both** are true: it is not a
/// resource this world has, and no recipe in a runnable category produces it.
/// Either alone is not enough — `iron-plate` is not a resource but smelting
/// makes it, and `coal` is not craftable but the ground supplies it.
///
/// **Shallow on purpose.** One level of ingredients, no recursion. A deeper
/// walk would have to cope with the recipe graph being cyclic, which it is:
/// Space Age's `X-recycling` recipes produce `X` from `X`. Those are skipped
/// here for the same reason — a recycling recipe as evidence of how to obtain
/// something is circular, since you must already have it.
///
/// One level is enough for what this is for. Every off-world science pack
/// names its foreign material immediately: `agricultural-science-pack` asks
/// for `bioflux` and `pentapod-egg`, `metallurgic-science-pack` for
/// `tungsten-plate` and `molten-copper`, `electromagnetic-science-pack` for
/// `holmium-solution`. None needs a search to find.
fn inputs_this_surface_cannot_supply(
    state: &PlanState,
    index: &ProductIndex,
    categories: &Categories,
    product: &str,
) -> Vec<String> {
    let here: std::collections::BTreeSet<String> = state.resource_names().into_iter().collect();
    let unreachable_for = |recipe: &FactorioRecipe| -> Vec<String> {
        let mut out: std::collections::BTreeSet<String> = Default::default();
        for ingredient in recipe.ingredients.iter().flatten() {
            let name = &ingredient.name;
            if here.contains(name) {
                continue;
            }
            let makeable_here = index
                .recipes_producing(name)
                .iter()
                .any(|r| categories.admits(&r.category));
            if !makeable_here {
                out.insert(name.clone());
            }
        }
        out.into_iter().collect()
    };

    // **The NEAREST recipe's inputs, not the union of every recipe's.**
    //
    // Pooling them names ingredients the caller does not all need. `plastic-bar`
    // has a local chemistry recipe wanting `petroleum-gas` and a Gleba
    // `bioplastic` recipe wanting `bioflux` and `yumako-mash`; the union reads
    // as though all three were required, when in fact oil alone opens the path.
    // Reporting the recipe with the fewest unreachable inputs answers the
    // question a reader is actually asking -- what is the least that has to
    // change -- and a union answers no question at all.
    //
    // Ties break on recipe name so the message is stable between runs; the
    // planner is deterministic and its refusals have to be too.
    index
        .recipes_producing(product)
        .into_iter()
        .filter(|r| r.category != RECYCLING_CATEGORY)
        .map(|r| (unreachable_for(r), r.name.clone()))
        .filter(|(missing, _)| !missing.is_empty())
        .min_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.1.cmp(&b.1)))
        .map(|(missing, _)| missing.into_iter().take(4).collect())
        .unwrap_or_default()
}

/// What a surface supplies with no recipe at all: charted resources, and the
/// fluids its ground yields.
///
/// Both halves are read off the world rather than named here, which is what
/// keeps the preference they seed from being a list of things somebody knew
/// about in September 2026:
///
/// - `EntityGraph::resource_names_present` -- the resources **charted**, not
///   the twelve `entity_type == "resource"` prototypes every surface declares.
///   That distinction is the entire difference between `iron-plate` and
///   `casting-iron`: the second wants molten iron, which wants `calcite`,
///   which is a prototype this install has and a resource a Nauvis map does
///   not. `no resource patch found for 'calcite'` has scrolled past in every
///   log this project has ever written.
/// - `TileFluid::named` on the tiles within [`GROUND_FLUID_SCAN_RADIUS`] --
///   *"the fluid an offshore pump produces on this tile"*, straight from
///   `LuaTilePrototype::fluid`. Not `yields_water`, and not the tile-name
///   pair: a modded planet whose lakes are something else seeds that instead,
///   with nothing here changed.
///
/// **A dump written before tiles carried `fluid` seeds no fluid**, because
/// every tile in it is `TileFluid::Unknown` and `named()` is `None` for that.
/// So this returns resources only on every archived world, and the preference
/// built on it stays inert there -- which is why no baseline taken on
/// `map.json` can move.
pub(crate) fn ground_supply(world: &FactorioSurface) -> BTreeSet<String> {
    let mut supply: BTreeSet<String> = world
        .entity_graph
        .resource_names_present()
        .into_iter()
        .collect();
    let r = GROUND_FLUID_SCAN_RADIUS;
    let bounds = Rect::new(&Position::new(-r, -r), &Position::new(r, r));
    for tile in world.entity_graph.tiles_within(&bounds) {
        if let Some(fluid) = tile.fluid.named() {
            supply.insert(fluid.to_string());
        }
    }
    supply
}

/// `a`, `a and b`, `a, b and c` -- for names that are not recipe candidates.
fn list_plain(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
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
            | ProductRefusal::Ambiguous { product, .. }
            | ProductRefusal::NoSuchRecipe { product, .. }
            | ProductRefusal::RecipeDoesNotProduce { product, .. }
            | ProductRefusal::NamedRecipeNotRunnable { product, .. }
            | ProductRefusal::NamedRecipeNotHonoured { product, .. } => product,
        }
    }

    /// The recipe the caller named, for the four refusals that only exist
    /// because one was. `None` for the three that are about the product.
    ///
    /// Absence here is the same absence `Goal::Produced::via` carries: no
    /// recipe was named, not a recipe that was not found.
    pub fn named_recipe(&self) -> Option<&str> {
        match self {
            ProductRefusal::NoSuchRecipe { recipe, .. }
            | ProductRefusal::RecipeDoesNotProduce { recipe, .. }
            | ProductRefusal::NamedRecipeNotRunnable { recipe, .. }
            | ProductRefusal::NamedRecipeNotHonoured { recipe, .. } => Some(recipe),
            _ => None,
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
    /// What this surface supplies without a recipe: the resources charted on
    /// it, and the fluids its ground yields.
    ///
    /// The seed of [`Self::reachable_here`], and the whole reason that
    /// preference can tell `iron-plate` (smelting, from charted iron ore) from
    /// `casting-iron` (metallurgy, from molten iron, which wants calcite this
    /// map does not have).
    ///
    /// # Empty means *unknown*, and disables the preference rather than
    /// emptying it
    ///
    /// [`Self::from_parts`] has no world and leaves this empty; so does a
    /// world charting nothing, and so does **every dump written before tiles
    /// carried `fluid`** -- their tiles are [`TileFluid::Unknown`], which
    /// knows nothing either way and seeds nothing. With an empty supply no
    /// candidate's ingredients are reachable, the preference finds no
    /// survivors, and [`Self::sole_recipe_producing`] falls back to the whole
    /// runnable set: exactly the answer it gave before this field existed.
    ///
    /// That is the safe direction and it is deliberate. A preference that
    /// cannot see the ground refuses to choose; it does not choose wrongly.
    supply: BTreeSet<String>,
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
            supply: BTreeSet::new(),
        }
    }

    /// Index a live or dumped world.
    pub fn from_world(world: &FactorioSurface) -> Self {
        // Collected rather than streamed: the tables are `DashMap`s whose
        // guards cannot be held across `from_parts`, and the result must not
        // depend on the order they were walked in.
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
        let mut index = ProductIndex::from_parts(recipes.iter(), items.iter().map(String::as_str));
        index.supply = ground_supply(world);
        index
    }

    /// Replace the surface supply [`Self::from_world`] read off the ground.
    ///
    /// For a caller that knows what a surface supplies by other means -- a
    /// fixture with no world, or a survey asking *"what could be made if this
    /// were charted"*. Passing an empty set restores the pre-supply behaviour,
    /// in which [`Self::sole_recipe_producing`] never prefers one candidate
    /// over another.
    #[must_use]
    pub fn with_ground_supply<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.supply = names.into_iter().map(Into::into).collect();
        self
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
    /// Does this surface supply `name` with **no recipe at all** -- a charted
    /// resource, or a fluid its ground yields?
    ///
    /// [`Self::supply`]'s answer, exposed because a caller deciding whether to
    /// *manufacture* something needs it and must not re-derive it: recomputing
    /// [`ground_supply`] scans every tile in the fluid radius, and a second
    /// copy of the rule would be a second thing to keep true.
    ///
    /// **Empty means unknown**, as everywhere else here, so this answers
    /// `false` on a world that charted nothing -- the same safe direction the
    /// field's own doc takes.
    pub fn ground_supplies(&self, name: &str) -> bool {
        self.supply.contains(name)
    }

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

    /// Everything this surface can make, starting from what it supplies with
    /// no recipe and closing over the recipes this planner runs.
    ///
    /// A least fixpoint: seed with [`Self::supply`], then add the products of
    /// every runnable non-recycling recipe all of whose ingredients are
    /// already in. Recycling is excluded here for a second reason on top of
    /// rule 1 -- the recipe graph is *cyclic* through it, and a closure over a
    /// cycle admits everything reachable from anything.
    ///
    /// **Ingredients, not amounts.** This asks whether a thing can be obtained
    /// at all, never whether enough of it can be; costing is the scheduler's
    /// question and a different one.
    ///
    /// # `presuming` is the thing being chosen for, and it is withheld
    ///
    /// Rule 2 asks *"which of these recipes could this surface feed"* about a
    /// product it does not yet have. Seeding the closure with that product
    /// answers a different question -- *"which could it feed if it already had
    /// one"* -- and the two differ exactly on the recipes that go **through**
    /// the product, which are the ones no plan can execute.
    ///
    /// Measured, not supposed. Over the live 2.1.17 capture with the full
    /// Space Age category set and `water` in the ground supply, water's three
    /// producers are `ice-melting` (wants ice: not reachable here),
    /// `steam-condensation` (wants steam: likewise) and `empty-water-barrel`
    /// (wants a `water-barrel`, which `fill-water-barrel` makes **out of
    /// water**). With water seeded, the barrel route is the sole survivor and
    /// rule 2 hands the choice to it -- a rig to unbottle water beside a lake,
    /// which is the same wrong answer `method::supply` records in its own
    /// comment, arrived at by a different road.
    ///
    /// **Rule 3 cannot catch this**, and its doc claiming barrelling "can
    /// never be the strict minimum" is true only of rule 3: rule 2 runs first
    /// and had already eliminated every real producer, so rule 3 was handed
    /// one candidate and never priced anything.
    ///
    /// Withholding it restores the safe direction: no candidate is reachable,
    /// the whole set comes back, and rule 3 -- the preference that was designed
    /// to reject barrelling and *can* -- decides instead. Rule 2 abstaining is
    /// exactly the "unknown supply leaves the set alone" behaviour this
    /// function already promises for a world that charted nothing.
    ///
    /// It can only ever fire for a product in [`Self::supply`], since nothing
    /// else was in the seed to withhold; every plan on a world model that
    /// declares no crafting categories is untouched, because rule 2 is
    /// consulted only when more than one candidate is runnable and the
    /// crafting/smelting map is one-to-one.
    fn reachable_here(&self, categories: &Categories, presuming: &str) -> BTreeSet<String> {
        let mut reachable = self.supply.clone();
        reachable.remove(presuming);
        if reachable.is_empty() {
            return reachable;
        }
        let usable: Vec<&FactorioRecipe> = self
            .by_recipe
            .values()
            .filter(|r| r.category != RECYCLING_CATEGORY && categories.admits(&r.category))
            .collect();
        loop {
            let mut grew = false;
            for recipe in &usable {
                if !recipe
                    .ingredients
                    .iter()
                    .flatten()
                    .all(|i| reachable.contains(&i.name))
                {
                    continue;
                }
                for product in &recipe.products {
                    grew |= reachable.insert(product.name.clone());
                }
            }
            if !grew {
                return reachable;
            }
        }
    }

    /// The candidates whose every ingredient this surface can obtain, or all
    /// of them when that leaves none.
    ///
    /// # This is how "prefer the base-game recipe" is said without a mod list
    ///
    /// Sulfur has four producers here and three of them are Space Age, but
    /// *"which mod shipped it"* is not a field on anything and asking for it
    /// would be a mod-compatibility defect the day somebody installs a fifth
    /// mod. What is a field is the ingredient list, and it separates them
    /// cleanly on a Nauvis map:
    ///
    /// - `sulfur` wants petroleum gas and water -- oil is charted here and the
    ///   ground yields water;
    /// - `biosulfur` wants bioflux, which closes back to Gleba fruit that no
    ///   recipe here makes and no tile here grows;
    /// - `advanced-carbonic-asteroid-crushing` wants a carbonic asteroid
    ///   chunk, which arrives from orbit or not at all.
    ///
    /// So the answer is not *"the base one"*, it is *"the one this planet can
    /// feed"* -- which happens to be the base one on Nauvis, and would
    /// correctly be `biosulfur` on Gleba. That is a stronger statement than
    /// the one asked for, and it needed no mod names to make.
    ///
    /// # Why this is not a fifth [`crate::method::util::RecipeGate`] verdict
    ///
    /// It looks like one -- `biosulfur` is a recipe this planner should not
    /// pick, and `RecipeGate` is where "should not pick" is decided. The gate
    /// even has the neighbouring verdict: `Unobtainable`, *"disabled and no
    /// technology unlocks it"*. `biosulfur` has one, so it comes back
    /// `NeedsResearch` and stays live, and the planner will bill research for
    /// a recipe whose ingredients nothing on this surface can make. That gap
    /// is real and worth closing on its own terms.
    ///
    /// It could not have closed this one, and the call sites say why rather
    /// than the bodies. **Every one of the eleven `recipe_gate` callers is
    /// handed a recipe that has already been chosen** -- by
    /// `method::util::recipe_for`, which keys on a name, or by
    /// [`ProductIndex::recipe_producing`], which is this function's caller.
    /// `method::fabricate::job_for` is the shape of all of them: it selects on
    /// one line and gates on the next, twelve lines later. A verdict computed
    /// for a recipe already selected cannot decide between candidates, because
    /// the refusal that stopped these goals was raised before any of them was
    /// selected -- and `sole_recipe_producing` has no `PlanState` to ask a gate
    /// with, by design, so that an index is valid for a whole expansion.
    ///
    /// So the two belong at different times, not in one place. This chooses;
    /// the gate judges what was chosen.
    ///
    /// # It is a preference and never a filter
    ///
    /// When nothing survives -- an unknown supply, or a product genuinely
    /// out of reach -- the whole set comes back and the caller refuses as
    /// `Ambiguous` exactly as before. Choosing nothing is not an improvement
    /// on choosing wrongly, and this must not be able to turn an answerable
    /// goal into an unanswerable one.
    fn fed_from_the_ground<'a>(
        &'a self,
        candidates: &[&'a FactorioRecipe],
        categories: &Categories,
        product: &str,
    ) -> Vec<&'a FactorioRecipe> {
        // `product` is withheld from the closure's seed -- see
        // `reachable_here`'s own doc for the water/barrel case that is.
        let reachable = self.reachable_here(categories, product);
        let fed: Vec<&FactorioRecipe> = candidates
            .iter()
            .copied()
            .filter(|r| {
                r.ingredients
                    .iter()
                    .flatten()
                    .all(|i| reachable.contains(&i.name))
            })
            .collect();
        if fed.is_empty() {
            candidates.to_vec()
        } else {
            fed
        }
    }

    /// The one candidate whose inputs are strictly cheapest to obtain, or the
    /// whole set unchanged.
    ///
    /// # The same metric as machines, not a second one that agrees with it
    ///
    /// [`crate::method::machine::obtain_costs`] is called, not reproduced:
    /// a transitive fixpoint in thousandths of a raw input, recycling
    /// excluded, seeded from [`Self::supply`] -- the same charted ground rule
    /// 2 reads. Priced by that function, smelting comes out stone 5 < steel
    /// 50 < electric 120, re-deriving the machine table's hard-coded order,
    /// which is the validation it arrived with.
    ///
    /// # A preference, never a filter
    ///
    /// Only a **strict** minimum wins. A tie hands the set back and the caller
    /// refuses as [`ProductRefusal::Ambiguous`] exactly as it did before this
    /// existed, and a candidate whose ingredients cannot be priced is simply
    /// not preferred rather than eliminated. That is the property that makes
    /// this unable to turn an answerable goal into an unanswerable one -- and
    /// it is why it is consulted only when rules 1 and 2 left a choice.
    ///
    /// # Barrelling, and why the metric rejects it by construction
    ///
    /// `empty-petroleum-gas-barrel` is one of petroleum-gas' five producers,
    /// and the owner asked whether obtain cost ranks it below real
    /// production. It does, and **not by luck**: emptying a barrel costs one
    /// `X-barrel`, which costs one `barrel` plus 50 `X`, so the price of `X`
    /// by that route is *always* `cost(X) + cost(barrel)/50` -- the fixpoint's
    /// own answer for `X` plus a strictly positive constant. Since
    /// `cost(X)` is the minimum over the real producers and is achieved by
    /// one of them, barrelling can never be the strict minimum. Measured on
    /// the seed-31337 explored dump: `basic-oil-processing` 2.222,
    /// `advanced-oil-processing` 2.273, `empty-petroleum-gas-barrel` 2.322,
    /// `light-oil-cracking` 4.917.
    ///
    /// The one way barrelling could win is if every real producer were
    /// unpriceable -- and it cannot be, because the barrel route prices
    /// through `X-barrel`, which prices through `X`. Unpriceable `X` makes
    /// the barrel unpriceable too.
    ///
    /// **That argument is about rule 3 and covers only rule 3.** Rule 2 runs
    /// first and can eliminate every real producer as unreachable, leaving
    /// barrelling alone -- at which point this function is handed one candidate
    /// and never prices anything. Measured on `water`, whose real producers
    /// want ice and steam that Nauvis cannot make, while `water-barrel` is made
    /// out of water. `reachable_here` withholds the product it is choosing for,
    /// which is what keeps that from happening; see its doc and
    /// `a_fluid_the_ground_gives_is_not_obtained_by_unbottling_itself`.
    ///
    /// # It UNDER-prices a multi-output recipe, and that is the wrong
    /// direction
    ///
    /// `obtain_costs` charges a recipe's whole bill to each of its products,
    /// so `advanced-oil-processing`'s heavy and light oil are free and its
    /// petroleum looks cheap -- while a refinery running it *stalls* unless
    /// something cracks the other two away. On the seed-31337 dump it still
    /// loses (2.727 to `basic-oil-processing`'s 2.222) only because it also
    /// drinks 50 water; take the water out of the bill and it wins. See
    /// `the_metric_charges_a_whole_bill_to_each_co_product`, which pins that.
    ///
    /// Not patched here. The consequence is contained by
    /// `FabricateRefusal::ManyFluidProducts`, a wall returned before any step
    /// is emitted, so the bad pick refuses by name rather than building a rig
    /// that plans green and moves nothing; and splitting a bill across
    /// co-products is a decision with no obviously right answer rather than a
    /// bug with a fix.
    ///
    /// Deterministic: ties in cost are broken by recipe name for the *sort*
    /// only, and a tie in cost still refuses.
    fn cheapest_to_obtain<'a>(
        &'a self,
        candidates: &[&'a FactorioRecipe],
        product: &str,
    ) -> Vec<&'a FactorioRecipe> {
        /// Thousandths again on top of `obtain_costs`' own thousandths, so
        /// dividing by a recipe's yield keeps six digits rather than three.
        /// Integer throughout, so the comparison is exact and reproducible.
        const YIELD_SCALE: u128 = 1000;

        let wanted: BTreeSet<String> = candidates
            .iter()
            .flat_map(|r| r.ingredients.iter().flatten().map(|i| i.name.clone()))
            .collect();
        let cost =
            crate::method::machine::obtain_costs(self.by_recipe.values(), &wanted, &self.supply);
        let mut priced: Vec<(u128, &str, &'a FactorioRecipe)> = Vec::new();
        for recipe in candidates {
            let Some(amount) = recipe
                .products
                .iter()
                .find(|p| p.name == product)
                .map(|p| u128::from(p.amount))
                .filter(|a| *a > 0)
            else {
                continue;
            };
            let mut inputs: u128 = 0;
            let mut known = true;
            for ingredient in recipe.ingredients.iter().flatten() {
                match cost.get(&ingredient.name) {
                    Some(c) => inputs += u128::from(ingredient.amount) * u128::from(*c),
                    None => {
                        known = false;
                        break;
                    }
                }
            }
            if known {
                priced.push((inputs * YIELD_SCALE / amount, recipe.name.as_str(), recipe));
            }
        }
        priced.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        match priced.as_slice() {
            [(_, _, only)] => vec![*only],
            [(best, _, winner), (second, ..), ..] if best < second => vec![*winner],
            _ => candidates.to_vec(),
        }
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
        let every = self.recipes_producing(product);
        if every.is_empty() {
            return Err(ProductRefusal::NotProduced {
                product: product.to_string(),
                substance: self.substances.of(product),
            });
        }
        // **Rule 1: a recycling recipe is not a way to make a thing.**
        //
        // `X-recycling` hands back a quarter of what X was built from, so it
        // is evidence of how to *destroy* X, and reading it as production is
        // circular -- you must already have the thing. That was written down
        // in `inputs_this_surface_cannot_supply` a day before this, for the
        // same reason and against the same category; it was never applied to
        // choosing a recipe, because until `crafting_categories` crossed the
        // bridge no machine declared `recycling` and `categories.admits` threw
        // every one of these away one line below. The moment a `recycler`
        // reported its category, 39 recycling recipes became candidates for
        // `processing-unit` -- every candidate it had -- and a more accurate
        // world produced a worse plan.
        //
        // A hard exclusion, not a preference. Falling back to the recycling
        // recipes when nothing else survives would put the 39 straight back.
        // The one thing kept is the diagnostic: if recycling is *all* there
        // is, the whole set is restored so the refusal still names it rather
        // than claiming nothing produces the item.
        let productive: Vec<&FactorioRecipe> = every
            .iter()
            .copied()
            .filter(|r| r.category != RECYCLING_CATEGORY)
            .collect();
        let all: Vec<&FactorioRecipe> = if productive.is_empty() {
            every
        } else {
            productive
        };
        let runnable: Vec<&FactorioRecipe> = all
            .iter()
            .copied()
            .filter(|r| categories.admits(&r.category))
            .collect();
        // **Rule 2: prefer a recipe this surface can actually feed.**
        //
        // Only consulted when there is a choice, so it can never move a plan
        // that had one answer -- it fires exactly where the old code refused.
        // See `reachable_here` for what "can feed" means and for why an
        // unknown supply leaves the set alone.
        let runnable = if runnable.len() > 1 {
            self.fed_from_the_ground(&runnable, categories, product)
        } else {
            runnable
        };
        // **Rule 3: prefer the recipe whose inputs are cheapest to obtain.**
        //
        // The same metric, the same function and the same
        // preference-not-filter shape [`crate::method::machine`] uses to pick
        // between machines. Consulted only when rules 1 and 2 have left more
        // than one candidate, so it fires exactly where the old code refused
        // as [`ProductRefusal::Ambiguous`] and cannot move a plan that had an
        // answer.
        let runnable = if runnable.len() > 1 {
            self.cheapest_to_obtain(&runnable, product)
        } else {
            runnable
        };
        match runnable.len() {
            0 => Err(ProductRefusal::NoRunnableCategory {
                product: product.to_string(),
                candidates: all.iter().map(|r| candidate(r, product)).collect(),
                admitted: categories.names(),
                substance: self.substances.of(product),
                // Filled in by `ProductRefusal::with_unreachable_inputs`,
                // which needs a `PlanState` this function does not have.
                // Empty here means "not examined", and the message says
                // nothing rather than claiming the inputs are fine.
                unreachable_inputs: Vec::new(),
            }),
            1 => Ok(runnable[0]),
            _ => Err(ProductRefusal::Ambiguous {
                product: product.to_string(),
                candidates: runnable.iter().map(|r| candidate(r, product)).collect(),
            }),
        }
    }

    /// [`Self::sole_recipe_producing`], with the caller's own choice of recipe
    /// honoured when they made one.
    ///
    /// # `via: None` is delegated verbatim
    ///
    /// Not "delegated in effect" -- the first line of the body is the call.
    /// Every goal written before 2026-09-07 carries `None`, so the unqualified
    /// path has to be the *same code*, not code that agrees with it. That is
    /// what makes an opt-in qualifier unable to move a baseline, and it is
    /// checked by `a_named_recipe_that_is_the_planners_own_pick_changes_nothing`.
    ///
    /// # The four things a named recipe can be wrong about
    ///
    /// In order, each refusing before the next is asked:
    ///
    /// 1. no recipe of that name -- [`ProductRefusal::NoSuchRecipe`];
    /// 2. a recipe that does not produce the product --
    ///    [`ProductRefusal::RecipeDoesNotProduce`];
    /// 3. `crafting`/`smelting`, whose methods pick by product name and
    ///    cannot be told -- [`ProductRefusal::NamedRecipeNotHonoured`],
    ///    unless the named recipe *is* the one they would pick, which is the
    ///    ordinary case and passes;
    /// 4. a category with no machine in this world --
    ///    [`ProductRefusal::NamedRecipeNotRunnable`], carrying
    ///    [`MachineTable`]'s own words rather than the ambiguity message,
    ///    because naming a recipe has already answered "which of these".
    pub fn recipe_producing(
        &self,
        product: &str,
        categories: &Categories,
        via: Option<&str>,
        machines: &MachineTable,
    ) -> Result<&FactorioRecipe, ProductRefusal> {
        let Some(via) = via else {
            return self.sole_recipe_producing(product, categories);
        };
        let producers = || -> Vec<Candidate> {
            self.recipes_producing(product)
                .iter()
                .map(|r| candidate(r, product))
                .collect()
        };
        let Some(recipe) = self.by_recipe.get(via) else {
            return Err(ProductRefusal::NoSuchRecipe {
                product: product.to_string(),
                recipe: via.to_string(),
                producers: producers(),
            });
        };
        if !recipe.products.iter().any(|p| p.name == product) {
            return Err(ProductRefusal::RecipeDoesNotProduce {
                product: product.to_string(),
                recipe: via.to_string(),
                produces: recipe.products.iter().map(|p| p.name.clone()).collect(),
                producers: producers(),
            });
        }
        if recipe.category == crate::method::util::CRAFTING_CATEGORY
            || recipe.category == crate::method::util::SMELTING_CATEGORY
        {
            // `recipe_for`'s own lookup, performed here rather than through it
            // so that this function needs no `PlanState` -- see
            // `ProductRefusal::NamedRecipeNotHonoured` for why the answer
            // matters.
            let would_run = self.by_recipe.get(product).map(|r| r.name.clone());
            if would_run.as_deref() != Some(via) {
                return Err(ProductRefusal::NamedRecipeNotHonoured {
                    product: product.to_string(),
                    recipe: via.to_string(),
                    category: recipe.category.clone(),
                    would_run,
                });
            }
            return Ok(recipe);
        }
        if !categories.admits(&recipe.category) {
            return Err(match machines.machine_for(&recipe.category) {
                Err(machine) => ProductRefusal::NamedRecipeNotRunnable {
                    product: product.to_string(),
                    recipe: via.to_string(),
                    category: recipe.category.clone(),
                    machine,
                },
                // A machine exists and the *caller* narrowed the categories
                // anyway. Nothing in production does this -- the one caller
                // passes `planner_runs(machines)`, built from this same table
                // -- but a survey caller could, and the honest answer is the
                // one that names what was admitted.
                Ok(_) => ProductRefusal::NoRunnableCategory {
                    product: product.to_string(),
                    candidates: vec![candidate(recipe, product)],
                    admitted: categories.names(),
                    substance: self.substances.of(product),
                    // Left empty rather than guessed: this branch has no
                    // `PlanState` and so cannot ask what this surface can
                    // supply. Empty is the field's documented "nothing filled
                    // it in", which is exactly what happened.
                    unreachable_inputs: Vec::new(),
                },
            });
        }
        Ok(recipe)
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
        // A named recipe never reaches here: `method::named_recipe_refusal`
        // has already refused every way one can be wrong, before any method
        // was asked. Passing it anyway so the two cannot disagree if that
        // guard is ever moved.
        let via = match goal {
            Goal::Have { via, .. } | Goal::Produced { via, .. } => via.as_deref(),
            _ => None,
        };
        let index = ProductIndex::from_state(&ctx.state);
        let machines = MachineTable::from_state(&ctx.state);
        match index.recipe_producing(item, &Categories::planner_runs(&machines), via, &machines) {
            // Something can make it. Whatever stopped this goal, it is not
            // the recipe table, and saying anything here would be guessing.
            Ok(_) => None,
            Err(mut refusal) => {
                // Say WHY the missing category is not the actionable fact,
                // when it is not. See `NoRunnableCategory::unreachable_inputs`.
                if let ProductRefusal::NoRunnableCategory {
                    unreachable_inputs, ..
                } = &mut refusal
                {
                    *unreachable_inputs = inputs_this_surface_cannot_supply(
                        &ctx.state,
                        &index,
                        &Categories::planner_runs(&machines),
                        item,
                    );
                }
                Some(PlannerError::ProductNotMakeable(Box::new(refusal)))
            }
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

    // -----------------------------------------------------------------
    // A caller that names its recipe
    // -----------------------------------------------------------------

    /// **`via: None` is delegated, not merely equivalent.** Over every product
    /// the fixture knows and both category sets, the two functions agree
    /// exactly -- including on which refusal, not just on refusing.
    ///
    /// This is what makes the qualifier unable to move a baseline: no goal
    /// written before it existed can take a different path.
    #[test]
    fn no_recipe_named_takes_the_path_it_always_took() {
        let i = index();
        let machines = MachineTable::default();
        let products: Vec<&str> = i.products().collect();
        assert_eq!(products.len(), 4, "the sweep is not vacuous");
        for cats in [Categories::planner_brings(), Categories::any()] {
            for product in &products {
                assert_eq!(
                    i.recipe_producing(product, &cats, None, &machines)
                        .map(|r| r.name.clone())
                        .map_err(|e| e.to_string()),
                    i.sole_recipe_producing(product, &cats)
                        .map(|r| r.name.clone())
                        .map_err(|e| e.to_string()),
                    "{product}"
                );
            }
        }
    }

    /// Naming a recipe **overrides the planner's own choice**, which is a
    /// stronger claim than the one this test used to make.
    ///
    /// It asserted that the unqualified question had *no answer* for
    /// `iron-gear-wheel`. Rule 3 gave it one: 2 iron-plate against 10
    /// molten-iron is not a tie, and cost picks the `crafting` recipe -- the
    /// same answer rule 2 reaches on any charted Nauvis map, and the reason
    /// rule 3 exists. So the property worth pinning is that `via` still wins
    /// over a preference the planner is now able to express.
    #[test]
    fn naming_a_recipe_overrides_the_planners_own_choice() {
        let i = index();
        let machines = MachineTable::default();
        let both = Categories::only(["crafting", "metallurgy"]);
        assert_eq!(
            i.sole_recipe_producing("iron-gear-wheel", &both)
                .map(|r| r.name.as_str()),
            Ok("iron-gear-wheel"),
            "unqualified, cost separates the pair and picks the cheaper bill"
        );
        assert_eq!(
            i.recipe_producing(
                "iron-gear-wheel",
                &both,
                Some("casting-iron-gear-wheel"),
                &machines
            )
            .expect("the caller chose")
            .name,
            "casting-iron-gear-wheel"
        );
    }

    /// The four refusals a named recipe can earn, each distinguishable from
    /// the others by variant and not only by wording.
    #[test]
    fn each_way_a_named_recipe_can_be_wrong_has_its_own_refusal() {
        let i = index();
        let machines = MachineTable::default();
        let runs = Categories::planner_brings();

        assert!(matches!(
            i.recipe_producing("iron-gear-wheel", &runs, Some("nope"), &machines),
            Err(ProductRefusal::NoSuchRecipe { .. })
        ));
        assert!(matches!(
            i.recipe_producing(
                "iron-gear-wheel",
                &runs,
                Some("basic-oil-processing"),
                &machines
            ),
            Err(ProductRefusal::RecipeDoesNotProduce { .. })
        ));
        // `metallurgy` has no machine in an empty table -- and the message is
        // the machine table's own, saying the world never declared anything.
        let no_machine = i
            .recipe_producing(
                "iron-gear-wheel",
                &runs,
                Some("casting-iron-gear-wheel"),
                &machines,
            )
            .expect_err("metallurgy has no machine here");
        assert!(matches!(
            no_machine,
            ProductRefusal::NamedRecipeNotRunnable { .. }
        ));
        assert!(
            no_machine.to_string().contains("it did not say"),
            "{no_machine}"
        );
        assert_eq!(no_machine.named_recipe(), Some("casting-iron-gear-wheel"));

        // And the three refusals that exist without a caller naming anything
        // report no recipe, which is the same distinction from the other side.
        assert_eq!(
            i.sole_recipe_producing("petroleum-gas", &runs)
                .expect_err("no runnable category")
                .named_recipe(),
            None
        );
    }

    #[test]
    fn a_runnable_product_resolves_to_its_one_recipe() {
        let i = index();
        let r = i
            .sole_recipe_producing("iron-gear-wheel", &Categories::planner_brings())
            .expect("crafting admits exactly one of the two");
        assert_eq!(r.name, "iron-gear-wheel");
        assert_eq!(r.category, crate::method::util::CRAFTING_CATEGORY);
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
            .sole_recipe_producing("petroleum-gas", &Categories::planner_brings())
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

    /// **Tier 3 still refuses -- on a genuine tie, which is the only case
    /// rule 3 hands back.**
    ///
    /// This used `index()`'s gear pair until 2026-09-08, where the bills are
    /// 2 iron-plate against 10 molten-iron; obtain cost separates those and
    /// now picks the crafting recipe. So the refusal is demonstrated where it
    /// still belongs: the same pair with the same bill, which no cost can
    /// order. The message and the candidate list are unchanged, because
    /// nothing about the refusal itself moved.
    #[test]
    fn tier_three_refuses_to_choose_between_two_recipes_of_equal_cost() {
        let tied = parse(
            r#"{
              "name": "casting-iron-gear-wheel", "valid": true, "enabled": false,
              "hidden": false, "energy": 1, "category": "metallurgy",
              "order": "b[casting]-a[iron-gear-wheel]",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 2 }
              ],
              "products": [
                { "name": "iron-gear-wheel", "product_type": "item", "amount": 1,
                  "independent_probability": 1, "shared_probability": { "min": 0, "max": 1 } }
              ],
              "group": "intermediate-products", "subgroup": "intermediate-product"
            }"#,
        );
        let recipes = [iron_gear(), tied];
        let i = ProductIndex::from_parts(recipes.iter(), ["iron-gear-wheel"]);
        let both = Categories::only(["crafting", "metallurgy"]);
        let err = i
            .sole_recipe_producing("iron-gear-wheel", &both)
            .expect_err("two admitted recipes make it, and they cost the same");
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
            via: None,
        }
    }

    /// A **`Produced`** goal, not a `Have`.
    ///
    /// These tests asked for `Have { petroleum-gas }` until 2026-09-07, when
    /// the driver learned to refuse a fluid `Have` by shape before any method
    /// is asked -- see `method::expand_goal_body`'s `fluid_have_refusal`. That
    /// guard is right and this method is not the thing it tests, so the goal
    /// moved to the shape `NoProducer` is the authority for. `Produced` says
    /// "cause this to come into existence", which is a meaningful request an
    /// oil refinery answers and which makes no claim about an inventory, so it
    /// reaches the registry exactly as it always did.
    fn produced(item: &str) -> Goal {
        Goal::Produced {
            item: item.to_string(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: None,
        }
    }

    /// The premise: with no method registered at all, the driver's own answer
    /// is the unactionable one. Asserted so the test below cannot pass because
    /// the goal happened to succeed.
    #[test]
    fn without_the_method_the_driver_says_only_no_method_can_satisfy() {
        let state = oil_state();
        let registry = MethodRegistry::new();
        let err = expand(&[produced("petroleum-gas")], &state, &registry, BotId(1))
            .expect_err("nothing can satisfy this");
        assert!(
            matches!(err, PlannerError::NoApplicableMethod { .. }),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "no method can satisfy goal: produce 100 petroleum-gas"
        );
    }

    #[test]
    fn with_the_method_the_driver_names_the_recipe_and_its_category() {
        let state = oil_state();
        let registry = MethodRegistry::new().with(Box::new(NoProducer));
        let err = expand(&[produced("petroleum-gas")], &state, &registry, BotId(1))
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
        let err = expand(&[produced("iron-plate")], &state, &registry, BotId(1))
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
                .sole_recipe_producing("petroleum-gas", &Categories::planner_brings())
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

#[cfg(test)]
mod unreachable_input_tests {
    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::factorio::world::FactorioSurface;
    use factorio_bot_core::serde_json;
    use std::sync::Arc;

    fn recipe(json: &str) -> FactorioRecipe {
        serde_json::from_str(json).expect("fixture recipe parses")
    }

    fn r(name: &str, category: &str, ingredient: &str, product: &str) -> FactorioRecipe {
        recipe(&format!(
            r#"{{"name":"{name}","valid":true,"enabled":true,"hidden":false,"energy":1,
                 "order":"a","category":"{category}","group":"g","subgroup":"s",
                 "ingredients":[{{"name":"{ingredient}","ingredient_type":"item","amount":1}}],
                 "products":[{{"name":"{product}","product_type":"item","amount":1,
                               "independent_probability":1,
                               "shared_probability":{{"min":0,"max":1}}}}]}}"#
        ))
    }

    /// Three recipes: an off-world pack, a locally craftable gear, and the
    /// plate the gear needs.
    fn parts() -> (PlanState, Vec<FactorioRecipe>) {
        let recipes = vec![
            // `bioflux` is produced by nothing here at all.
            r("agri-pack", "organic", "bioflux", "agri-pack"),
            // A recycling variant: produces the pack FROM the pack. A cycle,
            // and useless as evidence of how to obtain one.
            r("agri-pack-recycling", "recycling", "agri-pack", "agri-pack"),
            r("gear", "crafting", "iron-plate", "gear"),
            r("iron-plate", "smelting", "iron-ore", "iron-plate"),
        ];
        let world = FactorioSurface::new();
        world
            .update_recipes(recipes.clone())
            .expect("update_recipes");
        (PlanState::from_world(Arc::new(world), &[BotId(1)]), recipes)
    }

    fn index_of(recipes: &[FactorioRecipe]) -> ProductIndex {
        ProductIndex::from_parts(recipes.iter(), ["agri-pack", "gear", "iron-plate"])
    }

    /// **An ingredient nothing here can produce is named**, which is what stops
    /// the refusal reading as "add a machine for category organic". A machine
    /// for that category would still have no `bioflux` to feed it.
    #[test]
    fn an_ingredient_no_runnable_recipe_produces_is_reported() {
        let (state, recipes) = parts();
        let missing = inputs_this_surface_cannot_supply(
            &state,
            &index_of(&recipes),
            &Categories::only(["crafting", "smelting"]),
            "agri-pack",
        );
        assert_eq!(missing, vec!["bioflux".to_string()]);
    }

    /// **An ingredient a runnable recipe DOES produce is not reported**, or the
    /// clause would name ordinary local materials and become noise. `gear`
    /// needs `iron-plate`, which smelting makes, so nothing is unsupplyable.
    #[test]
    fn a_locally_craftable_ingredient_is_not_reported() {
        let (state, recipes) = parts();
        let missing = inputs_this_surface_cannot_supply(
            &state,
            &index_of(&recipes),
            &Categories::only(["crafting", "smelting"]),
            "gear",
        );
        assert!(
            missing.is_empty(),
            "iron-plate is smeltable here: {missing:?}"
        );
    }

    /// **A recycling recipe is never the evidence.** `agri-pack-recycling`
    /// produces the pack from the pack, so treating it as a path would say the
    /// way to obtain one is to already have one. Space Age adds one of these
    /// for nearly every item, so the recipe graph really is cyclic.
    #[test]
    fn a_recycling_recipe_is_not_treated_as_a_path() {
        let (state, recipes) = parts();
        let missing = inputs_this_surface_cannot_supply(
            &state,
            &index_of(&recipes),
            &Categories::only(["crafting", "smelting"]),
            "agri-pack",
        );
        assert!(
            !missing.iter().any(|m| m == "agri-pack"),
            "the pack must not be reported as its own missing input: {missing:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Rule 1 -- a recycling recipe is not a producer
    // -----------------------------------------------------------------------

    /// A recipe with any number of item ingredients.
    fn rn(name: &str, category: &str, ingredients: &[&str], product: &str) -> FactorioRecipe {
        let ing = ingredients
            .iter()
            .map(|i| format!(r#"{{"name":"{i}","ingredient_type":"item","amount":1}}"#))
            .collect::<Vec<_>>()
            .join(",");
        recipe(&format!(
            r#"{{"name":"{name}","valid":true,"enabled":true,"hidden":false,"energy":1,
                 "order":"a","category":"{category}","group":"g","subgroup":"s",
                 "ingredients":[{ing}],
                 "products":[{{"name":"{product}","product_type":"item","amount":1,
                               "independent_probability":1,
                               "shared_probability":{{"min":0,"max":1}}}}]}}"#
        ))
    }

    /// The `processing-unit` case, in miniature and with the same shape: the
    /// only recipe that *makes* the thing is in a category this planner cannot
    /// run, and three recycling recipes hand it back as scrap of something
    /// else.
    ///
    /// Before rule 1 this refused as `Ambiguous` over the three recyclers --
    /// on the live dump, over **39** of them, every candidate the product had.
    /// The recycling category became admissible the day a `recycler` reported
    /// its `crafting_categories`, so a *more accurate world* is what produced
    /// the worse refusal.
    #[test]
    fn recycling_recipes_are_not_candidates_for_making_a_thing() {
        let recipes = [
            rn("chip", "crafting-with-fluid", &["plate"], "chip"),
            rn("silo-recycling", "recycling", &["silo"], "chip"),
            rn("armour-recycling", "recycling", &["armour"], "chip"),
            rn("turret-recycling", "recycling", &["turret"], "chip"),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["chip"]);
        let err = index
            .sole_recipe_producing("chip", &Categories::only(["crafting", "recycling"]))
            .expect_err("crafting-with-fluid is not admitted, so this refuses");
        let ProductRefusal::NoRunnableCategory { candidates, .. } = &err else {
            panic!("expected the honest category refusal, got {err:?}");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.recipe.as_str())
                .collect::<Vec<_>>(),
            vec!["chip"],
            "only the recipe that makes a chip is a candidate"
        );
        // The control that says the recyclers were genuinely admissible and
        // were dropped by rule 1 rather than by the category filter: they are
        // in a category this `Categories` admits.
        assert!(Categories::only(["crafting", "recycling"]).admits(RECYCLING_CATEGORY));
    }

    /// Rule 1 chooses, it does not merely improve the message: with one
    /// runnable maker and two recyclers, the maker is the answer where before
    /// there was none.
    #[test]
    fn a_maker_wins_over_its_own_recyclers() {
        let recipes = [
            rn("gear", "crafting", &["plate"], "gear"),
            rn("belt-recycling", "recycling", &["belt"], "gear"),
            rn("car-recycling", "recycling", &["car"], "gear"),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["gear"]);
        assert_eq!(
            index
                .sole_recipe_producing("gear", &Categories::only(["crafting", "recycling"]))
                .map(|r| r.name.as_str()),
            Ok("gear")
        );
    }

    /// **The exclusion keeps the diagnostic.** When recycling is all there is,
    /// the refusal still names it -- saying `NotProduced` would claim no
    /// recipe in this world mentions the item, which is false and would send a
    /// reader looking for a resource patch.
    #[test]
    fn a_product_only_recycling_makes_is_still_named_in_the_refusal() {
        let recipes = [rn("hull-recycling", "recycling", &["hull"], "shard")];
        let index = ProductIndex::from_parts(recipes.iter(), ["shard"]);
        let err = index
            .sole_recipe_producing("shard", &Categories::only(["crafting"]))
            .expect_err("nothing runnable makes it");
        let ProductRefusal::NoRunnableCategory { candidates, .. } = &err else {
            panic!("expected NoRunnableCategory naming the recycler, got {err:?}");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.recipe.as_str())
                .collect::<Vec<_>>(),
            vec!["hull-recycling"]
        );
    }

    // -----------------------------------------------------------------------
    // Rule 2 -- prefer the recipe this surface can feed
    // -----------------------------------------------------------------------

    /// Sulfur's four producers, in miniature: one fed from charted ore, one
    /// wanting a Gleba intermediate that closes back to fruit nothing here
    /// grows, one wanting an asteroid chunk that arrives from orbit, and the
    /// recycler rule 1 already dropped.
    fn sulfur_shaped() -> Vec<FactorioRecipe> {
        vec![
            rn("sulfur", "chemistry", &["gas"], "sulfur"),
            rn("gas", "chemistry", &["oil"], "gas"),
            rn("biosulfur", "organic", &["bioflux"], "sulfur"),
            rn("bioflux", "organic", &["yumako"], "bioflux"),
            rn("crushing", "crushing", &["chunk"], "sulfur"),
            rn("sulfur-recycling", "recycling", &["sulfur"], "sulfur"),
        ]
    }

    fn cats() -> Categories {
        Categories::only(["chemistry", "organic", "crushing", "recycling", "crafting"])
    }

    /// The whole point, stated as one assertion: `oil` is charted, `yumako`
    /// and `chunk` are not, and that -- not which mod shipped which recipe --
    /// is what picks `sulfur`.
    #[test]
    fn the_recipe_this_ground_can_feed_is_the_one_chosen() {
        let recipes = sulfur_shaped();
        let index = ProductIndex::from_parts(recipes.iter(), ["sulfur", "gas", "bioflux"])
            .with_ground_supply(["oil"]);
        assert_eq!(
            index
                .sole_recipe_producing("sulfur", &cats())
                .map(|r| r.name.as_str()),
            Ok("sulfur")
        );
    }

    /// **Reachability is transitive, and one level would not have answered
    /// this.** `biosulfur` asks for `bioflux`, which a runnable `organic`
    /// recipe does produce -- so a shallow check sees no missing ingredient
    /// and cannot tell it from `sulfur`. The closure follows `bioflux` down to
    /// `yumako` and finds nothing that supplies it.
    #[test]
    fn a_shallow_check_could_not_have_separated_these() {
        let recipes = sulfur_shaped();
        let index = ProductIndex::from_parts(recipes.iter(), ["sulfur", "gas", "bioflux"])
            .with_ground_supply(["oil"]);
        assert!(
            index
                .recipes_producing("bioflux")
                .iter()
                .any(|r| cats().admits(&r.category)),
            "the shallow check's premise: bioflux IS produced by a runnable recipe"
        );
        assert!(
            !index.reachable_here(&cats(), "").contains("bioflux"),
            "and the closure still finds it out of reach"
        );
    }

    /// Charting the Gleba fruit instead flips the answer to `biosulfur`, with
    /// nothing in the code changed. That is the evidence the rule is about the
    /// ground rather than about a list of recipes somebody preferred -- and it
    /// is why the doc says the rule would be right on Gleba.
    #[test]
    fn the_same_rule_picks_the_space_age_recipe_where_that_is_what_grows() {
        let recipes = sulfur_shaped();
        let index = ProductIndex::from_parts(recipes.iter(), ["sulfur", "gas", "bioflux"])
            .with_ground_supply(["yumako"]);
        assert_eq!(
            index
                .sole_recipe_producing("sulfur", &cats())
                .map(|r| r.name.as_str()),
            Ok("biosulfur")
        );
    }

    /// **A preference, never a filter.** With nothing charted that feeds any
    /// candidate, the set comes back whole and the caller refuses as before.
    /// Choosing nothing is not an improvement on choosing wrongly.
    #[test]
    fn when_nothing_is_fed_the_refusal_is_the_old_one() {
        let recipes = sulfur_shaped();
        let index = ProductIndex::from_parts(recipes.iter(), ["sulfur", "gas", "bioflux"])
            .with_ground_supply(["stone"]);
        let err = index
            .sole_recipe_producing("sulfur", &cats())
            .expect_err("nothing distinguishes them");
        let ProductRefusal::Ambiguous { candidates, .. } = &err else {
            panic!("expected the unchanged ambiguity, got {err:?}");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.recipe.as_str())
                .collect::<Vec<_>>(),
            vec!["biosulfur", "crushing", "sulfur"],
            "all three runnable makers, and no recycler"
        );
    }

    /// **This is the guard that says no archived baseline can move.** An index
    /// with no supply -- which is every world whose dump predates tile fluid,
    /// and every fixture -- prefers nothing, so the answer is the one the old
    /// code gave.
    #[test]
    fn an_unknown_supply_prefers_nothing() {
        let recipes = sulfur_shaped();
        let index = ProductIndex::from_parts(recipes.iter(), ["sulfur", "gas", "bioflux"]);
        assert!(
            index.reachable_here(&cats(), "").is_empty(),
            "no seed, no closure"
        );
        assert!(matches!(
            index.sole_recipe_producing("sulfur", &cats()),
            Err(ProductRefusal::Ambiguous { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // The seed, read off a world
    // -----------------------------------------------------------------------

    /// `ground_supply` against a real surface: the charted resources and the
    /// lake's own fluid, and **not** the resource prototypes the world
    /// declares but has no patch of.
    ///
    /// The fluid comes from `TileFluid::named`, so this is also the test that
    /// says an archived dump seeds no fluid: a tile whose sender never filled
    /// the field is `TileFluid::Unknown`, `named()` is `None`, and the lake is
    /// invisible to the seed.
    #[test]
    fn the_seed_is_what_is_charted_plus_what_the_ground_yields() {
        let world = factorio_bot_core::test_utils::fixture_world();
        let supply = ground_supply(&world);
        assert!(
            supply.contains("water"),
            "the lake's own fluid seeds the closure: {supply:?}"
        );
        assert!(
            supply.contains("iron-ore"),
            "a charted patch seeds it: {supply:?}"
        );
        // **The discriminator, and the fixture can express it**: this world
        // declares six `entity_type == "resource"` prototypes and spawns four
        // patches. `uranium-ore` and `crude-oil` are the two it declares and
        // does not have, so an implementation reading the prototype table
        // instead of the charted map fails here.
        //
        // The assertion was `!supply.contains("calcite")` until a mutation
        // sweep found it green: no fixture world declares calcite, so both the
        // right answer and the wrong one satisfied it. It named a real
        // resource from the live install and tested nothing.
        let declared: Vec<String> = world
            .globals
            .entity_prototypes
            .iter()
            .filter(|p| p.value().entity_type == "resource")
            .map(|p| p.key().clone())
            .collect();
        assert!(
            declared.contains(&"uranium-ore".to_string()),
            "the premise: this world DECLARES uranium-ore -- {declared:?}"
        );
        assert!(
            !supply.contains("uranium-ore") && !supply.contains("crude-oil"),
            "a resource declared but never charted must not seed the closure: \
             {supply:?}"
        );

        let dry = factorio_bot_core::test_utils::fixture_world_without_water();
        assert!(
            !ground_supply(&dry).contains("water"),
            "and no lake means no water in the seed"
        );
    }

    /// **The fixpoint has to run more than once, and recipe-name order is
    /// what decides whether one pass would have been enough.** The closure
    /// walks `by_recipe`, which is sorted by name, so a chain whose steps
    /// happen to be in dependency order closes on the first pass by luck.
    /// Here `a-widget` sorts *before* the `z-part` it needs, so pass one
    /// reaches only `z-part` and pass two is what reaches the widget.
    ///
    /// Found by a mutation sweep: replacing the loop with a single pass left
    /// every other test green, because every other fixture's chain is in
    /// alphabetical dependency order. The rule this repo keeps relearning --
    /// a fixture that cannot express the case says nothing about it.
    #[test]
    fn the_closure_iterates_until_nothing_new_is_reached() {
        let recipes = [
            rn("a-widget", "crafting", &["z-part"], "a-widget"),
            rn("z-part", "crafting", &["ore"], "z-part"),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["a-widget", "z-part"])
            .with_ground_supply(["ore"]);
        let reachable = index.reachable_here(&Categories::only(["crafting"]), "");
        assert!(
            reachable.contains("a-widget"),
            "two links away from the ground, and named out of order: {reachable:?}"
        );
    }

    /// A recycling recipe is excluded from the closure too, not only from the
    /// candidate list: it is the cycle in the recipe graph, and a fixpoint run
    /// over a cycle admits whatever is reachable from anything.
    #[test]
    fn the_closure_does_not_walk_through_a_recycler() {
        let recipes = [
            rn("armour-recycling", "recycling", &["armour"], "plate"),
            rn("armour", "crafting", &["plate"], "armour"),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["plate", "armour"])
            .with_ground_supply(["armour"]);
        let reachable = index.reachable_here(&Categories::only(["crafting", "recycling"]), "");
        assert!(
            !reachable.contains("plate"),
            "recycling armour must not count as a way to obtain a plate: {reachable:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Rule 3: cheapest to obtain
    // -----------------------------------------------------------------------

    /// `rn` with amounts, which is the whole subject here: barrelling is
    /// cheap-looking precisely because one barrel carries fifty of the fluid,
    /// and a fixture that priced every amount at 1 could not express it.
    ///
    /// `ingredients` are `(name, amount)`; so is `product`.
    fn ra(
        name: &str,
        category: &str,
        ingredients: &[(&str, u32)],
        products: &[(&str, u32)],
    ) -> FactorioRecipe {
        let ing = ingredients
            .iter()
            .map(|(i, n)| format!(r#"{{"name":"{i}","ingredient_type":"item","amount":{n}}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let prod = products
            .iter()
            .map(|(p, n)| {
                format!(
                    r#"{{"name":"{p}","product_type":"item","amount":{n},
                         "independent_probability":1,
                         "shared_probability":{{"min":0,"max":1}}}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        recipe(&format!(
            r#"{{"name":"{name}","valid":true,"enabled":true,"hidden":false,"energy":1,
                 "order":"a","category":"{category}","group":"g","subgroup":"s",
                 "ingredients":[{ing}],"products":[{prod}]}}"#
        ))
    }

    /// Petroleum-gas' producers in miniature, with the real amounts: two
    /// refinery recipes off charted crude, and the barrel pair that makes
    /// `empty-gas-barrel` a candidate the ground-reachability rule **cannot**
    /// drop -- filling a barrel is reachable the moment the gas is, so the
    /// cycle closes and rule 2 keeps it.
    fn gas_shaped() -> Vec<FactorioRecipe> {
        vec![
            ra("basic", "oil", &[("crude", 100)], &[("gas", 45)]),
            // The real bill, water included, because the water is what
            // decides this -- see
            // `the_metric_charges_a_whole_bill_to_each_co_product`.
            ra(
                "advanced",
                "oil",
                &[("crude", 100), ("water", 50)],
                &[("gas", 55), ("heavy", 25)],
            ),
            ra("barrel", "crafting", &[("plate", 1)], &[("barrel", 1)]),
            ra(
                "fill-gas-barrel",
                "crafting",
                &[("barrel", 1), ("gas", 50)],
                &[("gas-barrel", 1)],
            ),
            ra(
                "empty-gas-barrel",
                "crafting",
                &[("gas-barrel", 1)],
                &[("barrel", 1), ("gas", 50)],
            ),
        ]
    }

    fn gas_cats() -> Categories {
        Categories::only(["oil", "crafting"])
    }

    fn gas_index(recipes: &[FactorioRecipe]) -> ProductIndex {
        ProductIndex::from_parts(recipes.iter(), ["gas", "barrel", "gas-barrel", "heavy"])
            .with_ground_supply(["crude", "plate", "water"])
    }

    /// **A finding about the metric, recorded as a test because it decides a
    /// real choice.**
    ///
    /// [`obtain_costs`](crate::method::machine::obtain_costs) charges a
    /// recipe's **whole** bill to every one of its products independently --
    /// `each = inputs / product.amount`, with no split across co-products. So
    /// a multi-output recipe is systematically **under-priced**: its
    /// co-products are free.
    ///
    /// That is exactly backwards for the case that matters here.
    /// `advanced-oil-processing` makes three fluids at once and a refinery
    /// running it stalls unless the other two are cracked away, so it is the
    /// recipe a `via` preference should be *least* eager to pick -- and the
    /// metric makes it look cheap. It loses on the real seed-31337 dump only
    /// because it also drinks 50 water: 150 raw units over 55 gas (2.727)
    /// against `basic-oil-processing`'s 100 over 45 (2.222). **Delete the
    /// water and it wins**, which is what this asserts.
    ///
    /// Left as-is rather than patched, for two reasons. The metric is the
    /// owner's ruling and is validated where it was validated (machines:
    /// stone 5 < steel 50 < electric 120); and the consequence is contained,
    /// because `FabricateRefusal::ManyFluidProducts` is a wall reached before
    /// any step is emitted -- a multi-output pick refuses by name rather than
    /// building a rig that backs up. Splitting a bill across co-products has
    /// no obviously right answer (by amount? by value? both are circular) and
    /// is a decision, not a fix.
    #[test]
    fn the_metric_charges_a_whole_bill_to_each_co_product() {
        let recipes: Vec<FactorioRecipe> = gas_shaped()
            .into_iter()
            .map(|r| {
                if r.name == "advanced" {
                    ra(
                        "advanced",
                        "oil",
                        &[("crude", 100)],
                        &[("gas", 55), ("heavy", 25)],
                    )
                } else {
                    r
                }
            })
            .collect();
        let index = gas_index(&recipes);
        assert_eq!(
            index
                .sole_recipe_producing("gas", &gas_cats())
                .map(|r| r.name.as_str()),
            Ok("advanced"),
            "100 crude over 55 gas beats 100 over 45 -- because the 25 heavy \
             oil that comes with it is charged nothing at all"
        );
    }

    /// **The owner's question, answered on the fixture rather than asserted.**
    /// Three runnable recipes produce `gas` and rule 2 drops none of them, so
    /// before rule 3 this was `Ambiguous` and the whole chemistry ladder
    /// dead-ended. Cost picks `basic`: 100 crude over 45 gas, against
    /// `advanced`'s 100 over 55 and the barrel's `cost(gas) + cost(barrel)/50`.
    #[test]
    fn obtain_cost_ranks_barrelling_below_real_production() {
        let recipes = gas_shaped();
        let index = gas_index(&recipes);
        assert!(
            index
                .fed_from_the_ground(&index.recipes_producing("gas"), &gas_cats(), "gas")
                .len()
                > 1,
            "the premise: reachability alone does NOT drop the barrel, because \
             filling one is reachable as soon as the gas is"
        );
        assert_eq!(
            index
                .sole_recipe_producing("gas", &gas_cats())
                .map(|r| r.name.as_str()),
            Ok("basic")
        );
    }

    /// **And it loses to the *worse* real recipe too, which is the claim that
    /// matters.** With `basic` gone, `advanced` is the only real producer and
    /// is strictly more expensive than `basic` was -- yet the barrel still
    /// does not win, because emptying one costs exactly `cost(gas)` plus the
    /// barrel, and `cost(gas)` is by construction the best any real recipe
    /// achieves. Without this test, the result above would be consistent with
    /// barrelling winning the moment production got expensive.
    #[test]
    fn barrelling_loses_to_the_expensive_real_recipe_as_well() {
        let recipes: Vec<FactorioRecipe> = gas_shaped()
            .into_iter()
            .filter(|r| r.name != "basic")
            .collect();
        let index = gas_index(&recipes);
        assert_eq!(
            index
                .sole_recipe_producing("gas", &gas_cats())
                .map(|r| r.name.as_str()),
            Ok("advanced")
        );
    }

    /// **A preference, never a filter: a tie still refuses.** Two recipes with
    /// the same bill and the same yield cost the same, and rule 3 hands the
    /// set back rather than breaking the tie alphabetically -- which is
    /// exactly the accident the last mutation sweep caught elsewhere, where a
    /// rule passed because every cheapest candidate also sorted first.
    #[test]
    fn a_tie_in_obtain_cost_still_refuses_by_name() {
        let recipes = [
            ra("alpha", "oil", &[("crude", 100)], &[("gas", 45)]),
            ra("omega", "oil", &[("crude", 100)], &[("gas", 45)]),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["gas"]).with_ground_supply(["crude"]);
        assert!(matches!(
            index.sole_recipe_producing("gas", &gas_cats()),
            Err(ProductRefusal::Ambiguous { .. })
        ));
    }

    /// **An unpriceable candidate is not preferred, and does not veto.**
    ///
    /// Asked of the private helper directly, because rule 2 drops an
    /// unreachable candidate one line earlier and rule 3 would never see it --
    /// so the property has to be tested where it lives.
    ///
    /// Building the case took a correction worth recording: an ingredient
    /// *nothing produces* is not unpriceable, it is **raw and costs 1**, which
    /// is `obtain_costs`' documented base case and would have made the
    /// mystery recipe the winner for the right reason. The only genuinely
    /// unpriceable thing is one reachable solely through a **cycle** -- here
    /// `loopy` and `loopy2` make each other and nothing else makes either --
    /// which is exactly the shape the fixpoint exists to survive.
    #[test]
    fn an_unpriceable_candidate_is_simply_not_preferred() {
        let recipes = [
            ra("basic", "oil", &[("crude", 100)], &[("gas", 45)]),
            ra("looped", "oil", &[("loopy", 1)], &[("gas", 45)]),
            ra("make-loopy", "crafting", &[("loopy2", 1)], &[("loopy", 1)]),
            ra("make-loopy2", "crafting", &[("loopy", 1)], &[("loopy2", 1)]),
        ];
        let index = ProductIndex::from_parts(recipes.iter(), ["gas", "loopy", "loopy2"])
            .with_ground_supply(["crude"]);
        let candidates: Vec<&FactorioRecipe> = index.recipes_producing("gas");
        assert_eq!(candidates.len(), 2, "both are candidates: {candidates:?}");
        let chosen = index.cheapest_to_obtain(&candidates, "gas");
        assert_eq!(chosen.len(), 1, "one strict winner: {chosen:?}");
        assert_eq!(
            chosen[0].name, "basic",
            "the priceable one wins; the cyclic one is not preferred, and \
             crucially does not make the pair refuse"
        );
    }
}
