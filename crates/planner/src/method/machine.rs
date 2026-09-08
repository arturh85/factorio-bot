//! Which machine runs a recipe category, read off the game's own prototypes.
//!
//! # The question this answers, and why it could not be answered before
//!
//! A recipe carries a `category`. A method has to name the **machine** that
//! runs that category, and until 2026-09-07 nothing on our wire said what a
//! machine crafts. `oil-refinery`, `chemical-plant`, `centrifuge`,
//! `electromagnetic-plant` and all three assembling machines share one
//! `entity_type` (`assembling-machine`), so *"it crafts"* was knowable and
//! *"what it crafts"* was not — measured in
//! `docs/superpowers/notes/2026-09-07-a-recipe-the-planner-cannot-run.md`.
//!
//! `LuaEntityPrototype.crafting_categories` now crosses the bridge
//! ([`FactorioEntityPrototype::crafting_categories`]). This module is the one
//! place that reads it, so that "which machine runs X" has a single encoding
//! rather than one per method.
//!
//! # `None` is not an empty list
//!
//! The field is `Option<Vec<String>>`, and the two are different facts:
//! `None` is *the sender did not say* — every dump taken before 2026-09-07,
//! including `workspace/scripts/map.json` and `map-31337-explored.json` — and
//! `Some(vec![])` is *the game says this prototype crafts nothing*. A table
//! built from a world where **no** prototype declares anything therefore says
//! so ([`MachineTable::world_declares_categories`]), and its refusals say
//! "this world model does not carry the field" rather than "no machine runs
//! this", which would be a lie about the game.
//!
//! # The two the planner brings with it
//!
//! [`CRAFTING_CATEGORY`] and [`SMELTING_CATEGORY`] resolve to
//! [`Machine::Hands`] and to the literal [`crate::method::produce::FURNACE`]
//! **before** the table is consulted, and they resolve that way on every
//! world, declared or not. That is not a special case bolted on: they are the
//! two categories this planner already had a machine for, and the ones every
//! archived plan was measured against. Deriving them instead would make a
//! nine-month archive of dumps — which carry no `crafting_categories` at all —
//! plan nothing. It also keeps the answer for `smelting` at *the furnace a
//! player starts with* rather than at whichever of the three vanilla furnaces
//! sorts first.
//!
//! # `parameters` is not a recipe category
//!
//! Four of the five vanilla crafting machines report a `parameters` category
//! at runtime that appears in no data file and names no recipe — it is the
//! blueprint-parameter pseudo-category. Measured on a live 2.1.17 game:
//! `oil-refinery` reports `[oil-processing, parameters]`. Counting it would
//! make `parameters` look like a runnable category with one machine.
//!
//! # Ambiguity is answered by a PREFERENCE, and refused when the preference
//! cannot choose
//!
//! This install declares `advanced-crafting` on all three assembling machines
//! and `crafting-with-fluid` on two (counted from
//! `workspace/scripts/map-31337-water-and-oil.json`, not recalled). Until
//! 2026-09-08 several machines was always [`MachineRefusal::Ambiguous`], and
//! that refusal cascaded: `processing-unit` has exactly one recipe after the
//! recipe-ambiguity work and it is `crafting-with-fluid`, so the rung read
//! *"none is in a category this planner runs"* while the game runs it in a
//! machine anybody can build.
//!
//! Picking the first by name would be the silent answer this crate's
//! [`crate::products`] module exists to stop. What is used instead is the
//! discriminator the planner already applied by hand: **what does it cost to
//! obtain the machine**, summed transitively over the recipes the world
//! carries down to things nothing makes ([`obtain_costs`]). Cheapest wins,
//! and only a **strict** minimum wins — a tie is still
//! [`MachineRefusal::Ambiguous`], because two machines that cost the same are
//! genuinely interchangeable and nothing here has grounds to choose.
//!
//! **It is a preference, never a filter**, the shape
//! [`crate::products::ProductIndex::sole_recipe_producing`] established: with
//! no recipes to price (every world model built by [`Self::from_parts`], and
//! any world whose recipe table is empty) the preference finds nothing and the
//! refusal is byte-identical to the one that stood before. It can only ever
//! fire where the code already refused.
//!
//! # Why cost, and how it is checked
//!
//! Cost is not asserted to be the right discriminator — it is the one that
//! **reproduces the two answers this module already hard-codes**. Priced
//! against a Space Age dump, `smelting`'s three furnaces come out
//! stone (5) < steel (>90) < electric, i.e. exactly the
//! [`crate::method::produce::FURNACE`] the planner brings with it, and
//! `crafting`'s assemblers lose to hands, which cost nothing. A discriminator
//! that independently re-derives a hand-picked constant is evidence; the
//! alternatives are not so checkable — `crafting_speed` picks
//! `assembling-machine-3`, which cannot be built at t=0, and a mod list is
//! forbidden here for the reason the owner's standing rule gives.
//!
//! # Cost
//!
//! Building walks the entity-prototype table once. **Build one per expansion,
//! not one per lookup** — the same rule [`crate::products::ProductIndex`] and
//! [`crate::substance::SubstanceTable`] state, for the same reason.

use crate::method::produce::FURNACE;
use crate::method::util::{CRAFTING_CATEGORY, SMELTING_CATEGORY};
use crate::state::PlanState;
use factorio_bot_core::types::{FactorioEntityPrototype, FactorioRecipe};
use miette::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// The blueprint-parameter pseudo-category, which names no recipe.
///
/// See the module doc. Named rather than inlined so a test asserts over the
/// string the filter actually matches.
pub const PARAMETERS_PSEUDO_CATEGORY: &str = "parameters";

/// The `entity_type` of the player character.
///
/// The game's own classification, the same field `method::produce::cell_spec`
/// reads to tell a resource from an item. A character declares
/// `{crafting, hand-crafting}` and is not a thing anybody places, so it maps
/// to [`Machine::Hands`] rather than to [`Machine::Entity`].
const CHARACTER_TYPE: &str = "character";

/// What runs a recipe.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Machine {
    /// A character's own hands. Needs no prototype and no placement, which is
    /// exactly why `crafting` was one of the two categories reachable before
    /// this module existed.
    Hands,
    /// A machine that has to be obtained, placed and fed, by prototype name.
    Entity(String),
}

impl std::fmt::Display for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Machine::Hands => write!(f, "a character's hands"),
            Machine::Entity(name) => write!(f, "{name}"),
        }
    }
}

/// Why no single machine could be named for a category.
///
/// Owned strings: a refusal outlives the table it came from and travels into
/// a [`crate::error::PlannerError`] that reaches Lua.
#[derive(Clone, Debug, PartialEq, Eq, Error, Diagnostic)]
pub enum MachineRefusal {
    /// Nothing in this world declares the category.
    ///
    /// The two halves of `world_declares_categories` are the whole point: a
    /// world that carries the field and does not mention this category really
    /// has no machine for it; a world that carries the field nowhere has not
    /// been asked.
    #[error(
        "no machine in this world model runs recipe category {category}: {}",
        if *world_says { "the world declares crafting categories and none of them is this one".to_string() } else {
            format!("no prototype in this world declares any crafting category at all, so the model \
                     predates `crafting_categories` -- it did not say, which is not the same as \
                     saying nothing crafts {category}")
        }
    )]
    #[diagnostic(
        code(planner::no_machine_for_category),
        help(
            "a world dumped before 2026-09-07 carries no `crafting_categories`; re-dump it, or \
             ask a running game"
        )
    )]
    NoMachine {
        category: String,
        /// Whether *any* prototype in this world declares any category.
        world_says: bool,
    },

    /// Several machines declare the category and nothing here can choose.
    ///
    /// Vanilla `advanced-crafting` (three assembling machines) and
    /// `crafting-with-fluid` (two) are the live instances.
    #[error(
        "recipe category {category} is run by {} machines in this world -- {} -- and nothing here \
         chooses between them",
        machines.len(),
        machines.join(", ")
    )]
    #[diagnostic(
        code(planner::ambiguous_machine),
        help("name the machine explicitly, or narrow the goal to a recipe whose category has one")
    )]
    Ambiguous {
        category: String,
        /// In name order, so the message is byte-identical across runs.
        machines: Vec<String>,
    },
}

/// Every recipe category this world has a machine for, and which machine.
///
/// Built once from a world and then only read; see the module doc on cost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MachineTable {
    /// Category -> the prototypes declaring it, in name order. Only what the
    /// world *declared*; the planner's own two are answered by
    /// [`Self::machine_for`] before this map is consulted.
    declared: BTreeMap<String, Vec<String>>,
    /// Category -> the character declares it.
    by_hand: BTreeSet<String>,
    /// Whether any prototype declared anything at all. See the module doc.
    declares_anything: bool,
    /// Machine prototype name -> what it costs to obtain one, in thousandths
    /// of a raw input. Empty when the caller brought no recipes, which is
    /// what makes the preference inert rather than wrong on such a world.
    ///
    /// Fixed point rather than `f64` so this type can stay `Eq`, and so the
    /// comparison that picks a winner is exact.
    obtain_cost: BTreeMap<String, u64>,
}

impl MachineTable {
    /// Index prototypes by the categories they declare.
    ///
    /// Split out from [`Self::from_state`] so a test can build one from a
    /// prototype list without a whole world — the same seam
    /// [`crate::products::ProductIndex::from_parts`] has.
    ///
    /// Deterministic: the input may arrive in any order (the world's
    /// prototype table is a `DashMap`), and every list this builds is sorted.
    /// A table with **no recipes to price**, so the cost preference is inert
    /// and an ambiguous category refuses exactly as it did before 2026-09-08.
    pub fn from_parts<'a, P>(prototypes: P) -> Self
    where
        P: IntoIterator<Item = &'a FactorioEntityPrototype>,
    {
        MachineTable::from_parts_and_recipes(prototypes, std::iter::empty())
    }

    /// Index prototypes by category **and** price every machine against the
    /// recipes this world carries.
    ///
    /// The recipes are only ever read to break a tie between machines that
    /// declare the same category; nothing else in this module consults them.
    pub fn from_parts_and_recipes<'a, 'r, P, R>(prototypes: P, recipes: R) -> Self
    where
        P: IntoIterator<Item = &'a FactorioEntityPrototype>,
        R: IntoIterator<Item = &'r FactorioRecipe>,
    {
        MachineTable::from_parts_recipes_and_ground(prototypes, recipes, &BTreeSet::new())
    }

    /// The full form: prototypes, the recipes to price them with, and **what
    /// this surface supplies with no recipe at all**.
    ///
    /// The ground seed is not a refinement, it is what makes pricing work at
    /// all on a real world. Space Age produces `iron-ore`, `stone` and
    /// `coal` from asteroid crushing, so *nothing* in the shipped recipe set
    /// is un-produced and a cost graph seeded only on "no recipe makes this"
    /// has no base case: measured against
    /// `workspace/scripts/map-31337-water-and-oil.json`, every machine and
    /// every furnace priced infinite and the preference stayed inert. The
    /// seed is [`crate::products::ground_supply`] — the same charted
    /// resources and ground fluids the recipe preference is built on, so the
    /// two rules answer "what can this map supply" once.
    pub fn from_parts_recipes_and_ground<'a, 'r, P, R>(
        prototypes: P,
        recipes: R,
        ground: &BTreeSet<String>,
    ) -> Self
    where
        P: IntoIterator<Item = &'a FactorioEntityPrototype>,
        R: IntoIterator<Item = &'r FactorioRecipe>,
    {
        let mut table = MachineTable::default();
        for proto in prototypes {
            let Some(categories) = proto.crafting_categories.as_ref() else {
                continue;
            };
            // `Some(vec![])` still counts as the world having spoken: the
            // sender said this prototype crafts nothing, which is knowledge.
            table.declares_anything = true;
            for category in categories {
                if category == PARAMETERS_PSEUDO_CATEGORY {
                    continue;
                }
                if proto.entity_type == CHARACTER_TYPE {
                    table.by_hand.insert(category.clone());
                    continue;
                }
                table
                    .declared
                    .entry(category.clone())
                    .or_default()
                    .push(proto.name.clone());
            }
        }
        for machines in table.declared.values_mut() {
            machines.sort();
            machines.dedup();
        }
        let wanted: BTreeSet<String> = table
            .declared
            .values()
            .filter(|m| m.len() > 1)
            .flat_map(|m| m.iter().cloned())
            .collect();
        table.obtain_cost = obtain_costs(recipes, &wanted, ground);
        table
    }

    /// Index the prototypes a plan's world carries.
    pub fn from_state(state: &PlanState) -> Self {
        let protos: Vec<FactorioEntityPrototype> = state
            .base()
            .globals
            .entity_prototypes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let recipes: Vec<FactorioRecipe> = state
            .base()
            .globals
            .recipes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        MachineTable::from_parts_recipes_and_ground(
            protos.iter(),
            recipes.iter(),
            &crate::products::ground_supply(state.base()),
        )
    }

    /// Does any prototype in this world declare any crafting category?
    ///
    /// `false` means the world model predates the field, not that nothing in
    /// the game crafts. See the module doc.
    pub fn world_declares_categories(&self) -> bool {
        self.declares_anything
    }

    /// The single machine that runs `category`, or why there is not one.
    ///
    /// The planner's own two answer first and on every world; see the module
    /// doc for why that is not a special case.
    pub fn machine_for(&self, category: &str) -> Result<Machine, MachineRefusal> {
        if category == CRAFTING_CATEGORY {
            return Ok(Machine::Hands);
        }
        if category == SMELTING_CATEGORY {
            return Ok(Machine::Entity(FURNACE.to_string()));
        }
        if self.by_hand.contains(category) {
            return Ok(Machine::Hands);
        }
        let machines = self.declared.get(category).cloned().unwrap_or_default();
        match machines.len() {
            0 => Err(MachineRefusal::NoMachine {
                category: category.to_string(),
                world_says: self.declares_anything,
            }),
            1 => Ok(Machine::Entity(machines[0].clone())),
            _ => match self.cheapest(&machines) {
                Some(machine) => Ok(Machine::Entity(machine)),
                None => Err(MachineRefusal::Ambiguous {
                    category: category.to_string(),
                    machines,
                }),
            },
        }
    }

    /// Every category [`Self::machine_for`] answers `Ok` for, in name order.
    ///
    /// This is what [`crate::products::Categories::planner_runs`] is built
    /// from, so the set a refusal names and the set a method acts on are one
    /// computation and cannot drift — the defect
    /// `Categories::planner_runs`'s own doc was written to avoid, now closed
    /// with a derivation rather than with a second constant.
    pub fn runnable_categories(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        out.insert(CRAFTING_CATEGORY.to_string());
        out.insert(SMELTING_CATEGORY.to_string());
        out.extend(self.by_hand.iter().cloned());
        // Asked of `machine_for` rather than re-deciding here: the set a
        // refusal names and the set a method acts on are one computation, and
        // the cost preference reaches both or neither.
        for category in self.declared.keys() {
            if self.machine_for(category).is_ok() {
                out.insert(category.clone());
            }
        }
        out.into_iter().collect()
    }

    /// The single cheapest machine to obtain among `machines`, or `None` when
    /// nothing can be priced or the cheapest is not unique.
    ///
    /// `None` is the whole preference-not-filter promise: the caller refuses
    /// exactly as it did before this existed.
    fn cheapest(&self, machines: &[String]) -> Option<String> {
        let mut priced: Vec<(u64, &String)> = machines
            .iter()
            .filter_map(|m| self.obtain_cost.get(m).map(|c| (*c, m)))
            .collect();
        priced.sort();
        match priced.as_slice() {
            [] => None,
            [(_, only)] => Some((*only).clone()),
            [(best, winner), (second, _), ..] if best < second => Some((*winner).clone()),
            _ => None,
        }
    }
}

/// What it costs to obtain one of each named item, in **thousandths of a raw
/// input**, summed transitively over the recipes this world carries.
///
/// A raw input is anything no recipe in this world produces — ore, a fluid out
/// of the ground, an item a mod hands out. Each counts 1. Everything else
/// costs the cheapest of its recipes: the ingredients' costs, divided by how
/// many the recipe yields.
///
/// # Why a fixpoint and not recursion
///
/// Recipe graphs have cycles (a plate recycles to a plate, a barrel fills and
/// empties), so a depth-first walk needs a visited set and still answers
/// "infinite" for a cycle it entered from the wrong side. Relaxing every
/// recipe until nothing improves has no such order dependence and terminates:
/// each round either lowers some cost or stops. `RELAX_ROUNDS` bounds it
/// regardless, and a cost that has not converged by then is simply higher than
/// the truth — which makes the preference *less* decisive, never wrong.
///
/// # Recycling is excluded, for the reason [`crate::products`] excludes it
///
/// `X-recycling` turns a thing into its ingredients. Costing through it would
/// price an item by what it can be destroyed into and could make a machine
/// look free.
///
/// Only `wanted` names are returned: this exists to break ties between
/// machines, and nothing else reads it.
fn obtain_costs<'r, R>(
    recipes: R,
    wanted: &BTreeSet<String>,
    ground: &BTreeSet<String>,
) -> BTreeMap<String, u64>
where
    R: IntoIterator<Item = &'r FactorioRecipe>,
{
    /// Enough rounds for any real recipe chain (Space Age's deepest is well
    /// under 30), and a hard bound on the work.
    const RELAX_ROUNDS: usize = 64;
    /// Thousandths, so the comparison is exact and the type stays `Eq`.
    const SCALE: f64 = 1000.0;

    let useful: Vec<&FactorioRecipe> = recipes
        .into_iter()
        .filter(|r| r.category != crate::products::RECYCLING_CATEGORY)
        .collect();
    let produced: BTreeSet<&str> = useful
        .iter()
        .flat_map(|r| r.products.iter().map(|p| p.name.as_str()))
        .collect();
    let mut cost: BTreeMap<&str, f64> = BTreeMap::new();
    for name in &produced {
        cost.insert(name, f64::INFINITY);
    }
    // Anything nothing produces is raw and costs one. Read off the recipes'
    // own ingredients rather than from an item table, so a world that carries
    // recipes and no prototypes still prices.
    for recipe in &useful {
        for ingredient in recipe.ingredients.iter().flatten() {
            if !produced.contains(ingredient.name.as_str()) {
                cost.insert(&ingredient.name, 1.0);
            }
        }
    }
    // And anything the GROUND supplies is raw too, whatever recipes also make
    // it. Without this there is no base case on a real Space Age world -- see
    // `MachineTable::from_parts_recipes_and_ground`.
    for name in ground {
        cost.insert(name.as_str(), 1.0);
    }
    for _ in 0..RELAX_ROUNDS {
        let mut improved = false;
        for recipe in &useful {
            let mut inputs = 0.0;
            let mut known = true;
            for ingredient in recipe.ingredients.iter().flatten() {
                match cost.get(ingredient.name.as_str()) {
                    Some(c) if c.is_finite() => inputs += f64::from(ingredient.amount) * c,
                    _ => known = false,
                }
            }
            if !known {
                continue;
            }
            for product in &recipe.products {
                if product.amount == 0 {
                    continue;
                }
                let each = inputs / f64::from(product.amount);
                let slot = cost.entry(product.name.as_str()).or_insert(f64::INFINITY);
                if each < *slot {
                    *slot = each;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    wanted
        .iter()
        .filter_map(|name| {
            let c = cost.get(name.as_str()).copied()?;
            c.is_finite().then(|| (name.clone(), (c * SCALE) as u64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prototypes built by **deserialising the wire shape**, not by struct
    /// literal: the mod sends `crafting_categories` as a JSON list (and an
    /// empty one as `{}`), and a literal would bypass the serde attribute
    /// that decides how it is read. Same reason `substance`'s fixtures parse
    /// their recipes.
    ///
    /// The categories are read off
    /// `workspace/server/data/base/prototypes/entity/entities.lua` rather
    /// than recalled -- the fixture has to match what a live prototype
    /// reports, not what the code finds convenient.
    fn proto(
        name: &str,
        entity_type: &str,
        categories: Option<&[&str]>,
    ) -> FactorioEntityPrototype {
        let cats = match categories {
            None => "null".to_string(),
            Some(list) => factorio_bot_core::serde_json::to_string(list).expect("json"),
        };
        factorio_bot_core::serde_json::from_str(&format!(
            r#"{{ "name": "{name}", "entity_type": "{entity_type}",
                  "collision_mask": [],
                  "collision_box": {{ "left_top": {{"x": -0.5, "y": -0.5}},
                                     "right_bottom": {{"x": 0.5, "y": 0.5}} }},
                  "crafting_categories": {cats} }}"#
        ))
        .expect("a prototype in the shape the mod sends")
    }

    /// A recipe in the wire shape, for the cost table. Same reason `proto`
    /// deserialises rather than building a literal: the serde attributes
    /// decide how `ingredients` and `products` are read.
    fn recipe(
        name: &str,
        category: &str,
        ingredients: &[(&str, u32)],
        yields: u32,
    ) -> FactorioRecipe {
        let ing: Vec<String> = ingredients
            .iter()
            .map(|(n, a)| format!(r#"{{"name": "{n}", "amount": {a}}}"#))
            .collect();
        factorio_bot_core::serde_json::from_str(&format!(
            r#"{{ "name": "{name}", "valid": true, "enabled": true,
                  "category": "{category}", "hidden": false, "energy": 1.0,
                  "order": "a", "group": "g", "subgroup": "s",
                  "ingredients": [{}],
                  "products": [{{"name": "{name}", "amount": {yields}}}] }}"#,
            ing.join(", ")
        ))
        .expect("a recipe in the shape the mod sends")
    }

    /// The prices behind the two ambiguous categories and the three furnaces,
    /// transcribed from the game's own recipes (2.1.17 + Space Age).
    ///
    /// Not the whole tree: `electronic-circuit` and `steel-plate` are given
    /// their real ingredients so the transitive sum is a real one rather than
    /// a flat count, and `iron-plate` / `copper-plate` / `stone` are left
    /// unproduced so they price as raw — which is what they are to a planner
    /// that mines and smelts them.
    fn vanilla_recipes() -> Vec<FactorioRecipe> {
        vec![
            recipe("iron-gear-wheel", "crafting", &[("iron-plate", 2)], 1),
            recipe("copper-cable", "crafting", &[("copper-plate", 1)], 2),
            recipe(
                "electronic-circuit",
                "crafting",
                &[("iron-plate", 1), ("copper-cable", 3)],
                1,
            ),
            recipe("steel-plate", "smelting", &[("iron-plate", 5)], 1),
            recipe("stone-furnace", "crafting", &[("stone", 5)], 1),
            recipe(
                "steel-furnace",
                "crafting",
                &[("steel-plate", 6), ("stone-brick", 10)],
                1,
            ),
            recipe("stone-brick", "smelting", &[("stone", 2)], 1),
            recipe(
                "electric-furnace",
                "crafting",
                &[
                    ("steel-plate", 10),
                    ("advanced-circuit", 5),
                    ("stone-brick", 10),
                ],
                1,
            ),
            recipe(
                "advanced-circuit",
                "crafting",
                &[
                    ("electronic-circuit", 2),
                    ("plastic-bar", 2),
                    ("copper-cable", 4),
                ],
                1,
            ),
            recipe(
                "assembling-machine-1",
                "crafting",
                &[
                    ("electronic-circuit", 3),
                    ("iron-gear-wheel", 5),
                    ("iron-plate", 9),
                ],
                1,
            ),
            recipe(
                "assembling-machine-2",
                "crafting",
                &[
                    ("electronic-circuit", 3),
                    ("iron-gear-wheel", 5),
                    ("steel-plate", 9),
                    ("assembling-machine-1", 1),
                ],
                1,
            ),
            recipe(
                "assembling-machine-3",
                "crafting",
                &[("speed-module", 4), ("assembling-machine-2", 2)],
                1,
            ),
            recipe(
                "speed-module",
                "crafting",
                &[("advanced-circuit", 5), ("electronic-circuit", 5)],
                1,
            ),
        ]
    }

    /// Vanilla 2.1.17, transcribed from the game's own data files.
    fn vanilla() -> Vec<FactorioEntityPrototype> {
        vec![
            proto(
                "character",
                "character",
                Some(&["crafting", "hand-crafting"]),
            ),
            proto("stone-furnace", "furnace", Some(&["smelting"])),
            proto("steel-furnace", "furnace", Some(&["smelting"])),
            proto("electric-furnace", "furnace", Some(&["smelting"])),
            proto(
                "assembling-machine-1",
                "assembling-machine",
                Some(&["crafting", "advanced-crafting", "parameters"]),
            ),
            proto(
                "assembling-machine-2",
                "assembling-machine",
                Some(&[
                    "crafting",
                    "advanced-crafting",
                    "crafting-with-fluid",
                    "parameters",
                ]),
            ),
            proto(
                "assembling-machine-3",
                "assembling-machine",
                Some(&[
                    "crafting",
                    "advanced-crafting",
                    "crafting-with-fluid",
                    "parameters",
                ]),
            ),
            // The measured value: `[oil-processing, parameters]`, the second
            // of which is not a recipe category.
            proto(
                "oil-refinery",
                "assembling-machine",
                Some(&["oil-processing", "parameters"]),
            ),
            proto(
                "chemical-plant",
                "assembling-machine",
                Some(&["chemistry", "parameters"]),
            ),
            proto(
                "centrifuge",
                "assembling-machine",
                Some(&["centrifuging", "parameters"]),
            ),
            // Carries no crafting categories at all, like most of the table.
            proto("iron-chest", "container", None),
        ]
    }

    /// The headline: the machine `oil-processing` needs is named, and it is
    /// named from the world rather than from a literal here.
    #[test]
    fn a_category_with_one_machine_names_it() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert_eq!(
            table.machine_for("oil-processing"),
            Ok(Machine::Entity("oil-refinery".into()))
        );
        assert_eq!(
            table.machine_for("chemistry"),
            Ok(Machine::Entity("chemical-plant".into()))
        );
        assert_eq!(
            table.machine_for("centrifuging"),
            Ok(Machine::Entity("centrifuge".into()))
        );
    }

    /// The paired non-accidental control: the same table refuses the
    /// categories vanilla really does declare on several machines, and names
    /// them all. Without this, "one machine" would be equally explained by a
    /// table that answers one machine for everything.
    ///
    /// **This table has no recipes**, so nothing can be priced and the
    /// preference is inert -- which is the "never a filter" half of the rule,
    /// asserted rather than described.
    #[test]
    fn a_category_with_several_machines_is_refused_by_name() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert_eq!(
            table.machine_for("advanced-crafting"),
            Err(MachineRefusal::Ambiguous {
                category: "advanced-crafting".into(),
                machines: vec![
                    "assembling-machine-1".into(),
                    "assembling-machine-2".into(),
                    "assembling-machine-3".into(),
                ],
            })
        );
        assert_eq!(
            table.machine_for("crafting-with-fluid"),
            Err(MachineRefusal::Ambiguous {
                category: "crafting-with-fluid".into(),
                machines: vec!["assembling-machine-2".into(), "assembling-machine-3".into(),],
            })
        );
    }

    /// `parameters` is on four of the five crafting machines and names no
    /// recipe, so it must not read as a runnable category.
    #[test]
    fn the_blueprint_pseudo_category_is_not_a_recipe_category() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert!(matches!(
            table.machine_for(PARAMETERS_PSEUDO_CATEGORY),
            Err(MachineRefusal::NoMachine { .. })
        ));
        assert!(
            !table
                .runnable_categories()
                .contains(&PARAMETERS_PSEUDO_CATEGORY.to_string())
        );
    }

    /// The planner's own two answer on a vanilla world without being derived
    /// from it -- `smelting` is the starting furnace, not whichever of the
    /// three sorts first, and `crafting` is hands rather than three ambiguous
    /// assemblers.
    #[test]
    fn the_planners_own_two_are_not_ambiguous_and_not_derived() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert_eq!(table.machine_for(CRAFTING_CATEGORY), Ok(Machine::Hands));
        assert_eq!(
            table.machine_for(SMELTING_CATEGORY),
            Ok(Machine::Entity(FURNACE.into()))
        );
        // The control that shows this was a decision and not an accident:
        // three furnaces and three assemblers really are in the table, so the
        // derived answer for both would have been `Ambiguous`.
        assert_eq!(
            MachineTable::from_parts(vanilla().iter()).declared["smelting"].len(),
            3
        );
        assert_eq!(
            MachineTable::from_parts(vanilla().iter()).declared["crafting"].len(),
            3
        );
    }

    /// A category only the character declares is hands, derived.
    #[test]
    fn a_category_the_character_declares_is_hands() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert_eq!(table.machine_for("hand-crafting"), Ok(Machine::Hands));
    }

    /// A world model that predates the field says so, and still answers the
    /// planner's own two -- which is what keeps every archived dump planning.
    #[test]
    fn a_world_that_never_declared_the_field_says_so() {
        let older: Vec<FactorioEntityPrototype> = vanilla()
            .into_iter()
            .map(|mut p| {
                p.crafting_categories = None;
                p
            })
            .collect();
        let table = MachineTable::from_parts(older.iter());
        assert!(!table.world_declares_categories());
        assert_eq!(
            table.machine_for("oil-processing"),
            Err(MachineRefusal::NoMachine {
                category: "oil-processing".into(),
                world_says: false,
            })
        );
        // The refusal has to distinguish the two, and it does, in words.
        let said = table.machine_for("oil-processing").unwrap_err().to_string();
        assert!(said.contains("it did not say"), "{said}");
        // The paired control: the planner's own two still answer, so an
        // archived dump plans exactly as it did.
        assert_eq!(table.machine_for(CRAFTING_CATEGORY), Ok(Machine::Hands));
        assert_eq!(
            table.machine_for(SMELTING_CATEGORY),
            Ok(Machine::Entity(FURNACE.into()))
        );
        assert_eq!(
            table.runnable_categories(),
            vec![CRAFTING_CATEGORY.to_string(), SMELTING_CATEGORY.to_string()]
        );
    }

    /// A world that declares the field and does not mention the category says
    /// the *other* thing. Paired with the test above: same call, same
    /// category, opposite half of the `None`/`Some` split.
    #[test]
    fn a_world_that_declared_the_field_refuses_differently() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert!(table.world_declares_categories());
        let said = table.machine_for("recycling").unwrap_err().to_string();
        assert!(said.contains("none of them is this one"), "{said}");
        assert!(!said.contains("it did not say"), "{said}");
    }

    /// The runnable set is what a refusal names and what a method acts on,
    /// and it is one computation -- here on a table with **no recipes**, so
    /// the cost preference is inert and the answer is the pre-2026-09-08 one.
    #[test]
    fn the_runnable_set_is_what_machine_for_answers() {
        let table = MachineTable::from_parts(vanilla().iter());
        assert_eq!(
            table.runnable_categories(),
            vec![
                "centrifuging".to_string(),
                "chemistry".to_string(),
                "crafting".to_string(),
                "hand-crafting".to_string(),
                "oil-processing".to_string(),
                "smelting".to_string(),
            ]
        );
        // Every member answers `Ok`, and every refused category is absent --
        // the two halves of "one computation".
        for category in table.runnable_categories() {
            assert!(table.machine_for(&category).is_ok(), "{category}");
        }
        for category in ["advanced-crafting", "crafting-with-fluid"] {
            assert!(table.machine_for(category).is_err(), "{category}");
            assert!(!table.runnable_categories().contains(&category.to_string()));
        }
    }
    // -----------------------------------------------------------------------
    // The cost preference (2026-09-08)
    // -----------------------------------------------------------------------

    /// The headline: the two categories this install declares on several
    /// machines now name one, and it is the **cheapest to obtain**, not the
    /// first by name and not the fastest.
    ///
    /// Both answers here also sort first by name, so this pair alone cannot
    /// tell a cost rule from an alphabetical one -- a sweep mutation proved
    /// exactly that, and `cost_decides_and_not_the_name` is the case that
    /// separates them. What this pair does show is that it is not
    /// `crafting_speed`: that would pick `assembling-machine-3` for both, and
    /// `-3` cannot be built at t=0.
    #[test]
    fn an_ambiguous_category_names_the_cheapest_machine() {
        let table =
            MachineTable::from_parts_and_recipes(vanilla().iter(), vanilla_recipes().iter());
        assert_eq!(
            table.machine_for("advanced-crafting"),
            Ok(Machine::Entity("assembling-machine-1".into()))
        );
        assert_eq!(
            table.machine_for("crafting-with-fluid"),
            Ok(Machine::Entity("assembling-machine-2".into()))
        );
    }

    /// The validation the module doc claims: priced by the same function, the
    /// three furnaces come out in the order that re-derives the constant this
    /// module hard-codes. `smelting` is answered before the table is
    /// consulted, so this asserts over the costs directly -- if it ever
    /// disagreed with [`FURNACE`], the discriminator would be the thing to
    /// doubt.
    #[test]
    fn the_cost_rule_independently_re_derives_the_starting_furnace() {
        let wanted: BTreeSet<String> = ["stone-furnace", "steel-furnace", "electric-furnace"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let costs = obtain_costs(vanilla_recipes().iter(), &wanted, &BTreeSet::new());
        let cheapest = costs
            .iter()
            .min_by_key(|(_, c)| **c)
            .map(|(n, _)| n.clone())
            .expect("three furnaces priced");
        assert_eq!(cheapest, FURNACE);
        assert_eq!(costs.len(), 3, "{costs:?}");
        assert!(costs["stone-furnace"] < costs["steel-furnace"], "{costs:?}");
        assert!(
            costs["steel-furnace"] < costs["electric-furnace"],
            "{costs:?}"
        );
    }

    /// A preference, never a filter: two machines that cost the same are
    /// genuinely interchangeable, and nothing here has grounds to choose, so
    /// the refusal is the one that stood before -- naming both.
    #[test]
    fn a_tie_is_still_ambiguous() {
        let protos = [
            proto("twin-a", "assembling-machine", Some(&["twinning"])),
            proto("twin-b", "assembling-machine", Some(&["twinning"])),
        ];
        let recipes = [
            recipe("twin-a", "crafting", &[("iron-plate", 4)], 1),
            recipe("twin-b", "crafting", &[("iron-plate", 2), ("stone", 2)], 1),
        ];
        let table = MachineTable::from_parts_and_recipes(protos.iter(), recipes.iter());
        assert_eq!(
            table.machine_for("twinning"),
            Err(MachineRefusal::Ambiguous {
                category: "twinning".into(),
                machines: vec!["twin-a".into(), "twin-b".into()],
            })
        );
        assert!(
            !table
                .runnable_categories()
                .contains(&"twinning".to_string())
        );
        // The control that keeps the tie from being explained by "no price at
        // all": both really were priced, and equally.
        assert_eq!(table.obtain_cost["twin-a"], table.obtain_cost["twin-b"]);
    }

    /// A machine nothing produces cannot be obtained, so it is not preferred
    /// -- and one priced candidate beside one unpriceable one is not a tie.
    #[test]
    fn an_unbuildable_machine_never_wins_and_does_not_block_the_other() {
        let protos = [
            proto("buildable", "assembling-machine", Some(&["odd"])),
            proto("scenery", "assembling-machine", Some(&["odd"])),
        ];
        let recipes = [recipe("buildable", "crafting", &[("iron-plate", 9)], 1)];
        let table = MachineTable::from_parts_and_recipes(protos.iter(), recipes.iter());
        assert_eq!(
            table.machine_for("odd"),
            Ok(Machine::Entity("buildable".into()))
        );
    }

    /// The other half of "never a filter": the same prototypes with the
    /// recipe table taken away refuse exactly as they did, so a world model
    /// that carries no recipes plans byte-identically.
    #[test]
    fn with_no_recipes_to_price_the_refusal_is_unchanged() {
        let priced =
            MachineTable::from_parts_and_recipes(vanilla().iter(), vanilla_recipes().iter());
        let unpriced = MachineTable::from_parts(vanilla().iter());
        assert!(priced.machine_for("advanced-crafting").is_ok());
        assert_eq!(
            unpriced.machine_for("advanced-crafting"),
            Err(MachineRefusal::Ambiguous {
                category: "advanced-crafting".into(),
                machines: vec![
                    "assembling-machine-1".into(),
                    "assembling-machine-2".into(),
                    "assembling-machine-3".into(),
                ],
            })
        );
        assert_eq!(
            unpriced.runnable_categories(),
            vec![
                "centrifuging".to_string(),
                "chemistry".to_string(),
                "crafting".to_string(),
                "hand-crafting".to_string(),
                "oil-processing".to_string(),
                "smelting".to_string(),
            ]
        );
    }

    /// What the fix is for: the two categories join the runnable set, which
    /// is what [`crate::products::Categories::planner_runs`] admits, so a
    /// recipe in one of them stops reading as "no category this planner
    /// runs".
    #[test]
    fn pricing_adds_exactly_the_two_ambiguous_categories_to_the_runnable_set() {
        let priced =
            MachineTable::from_parts_and_recipes(vanilla().iter(), vanilla_recipes().iter());
        assert_eq!(
            priced.runnable_categories(),
            vec![
                "advanced-crafting".to_string(),
                "centrifuging".to_string(),
                "chemistry".to_string(),
                "crafting".to_string(),
                "crafting-with-fluid".to_string(),
                "hand-crafting".to_string(),
                "oil-processing".to_string(),
                "smelting".to_string(),
            ]
        );
    }

    /// Recycling is not a way to obtain a machine: a recipe that destroys one
    /// must not make it look free. Without the exclusion `twin-b` would price
    /// at nothing and win.
    #[test]
    fn a_recycling_recipe_cannot_make_a_machine_cheap() {
        let protos = [
            proto("twin-a", "assembling-machine", Some(&["twinning"])),
            proto("twin-b", "assembling-machine", Some(&["twinning"])),
        ];
        let recipes = [
            recipe("twin-a", "crafting", &[("iron-plate", 4)], 1),
            recipe("twin-b", "crafting", &[("iron-plate", 40)], 1),
            recipe("twin-b", "recycling", &[("scrap", 1)], 1),
        ];
        let table = MachineTable::from_parts_and_recipes(protos.iter(), recipes.iter());
        assert_eq!(
            table.machine_for("twinning"),
            Ok(Machine::Entity("twin-a".into()))
        );
    }

    /// A cycle in the recipe graph terminates and prices the way in, not the
    /// way round: a barrel filled from an emptied barrel must not lower the
    /// cost of anything.
    #[test]
    fn a_cycle_terminates_and_prices_the_way_in() {
        let wanted: BTreeSet<String> = ["full-barrel", "empty-barrel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let recipes = [
            recipe("empty-barrel", "crafting", &[("steel-plate", 1)], 1),
            recipe("steel-plate", "smelting", &[("iron-plate", 5)], 1),
            recipe(
                "full-barrel",
                "crafting",
                &[("empty-barrel", 1), ("water", 50)],
                1,
            ),
            // The other half of the cycle, which returns the barrel.
            recipe("empty-barrel", "crafting", &[("full-barrel", 1)], 1),
        ];
        let costs = obtain_costs(recipes.iter(), &wanted, &BTreeSet::new());
        assert_eq!(costs["empty-barrel"], 5_000);
        assert_eq!(costs["full-barrel"], 55_000);
    }
    /// The seam a unit test on `from_parts_and_recipes` cannot cover:
    /// [`MachineTable::from_state`] must hand the world's **recipes** to the
    /// pricing, not only its prototypes. Passing an empty recipe list there
    /// would make the whole preference inert on every real world while every
    /// other test in this module still passed.
    #[test]
    fn from_state_prices_against_the_worlds_own_recipes() {
        use crate::ids::BotId;
        use crate::state::PlanState;
        use factorio_bot_core::factorio::world::FactorioSurface;
        use factorio_bot_core::test_utils::spawn_ore;
        use factorio_bot_core::types::{Position, Rect};
        use std::sync::Arc;

        let world = FactorioSurface::new();
        world
            .update_entity_prototypes(vanilla())
            .expect("update_entity_prototypes");
        // The real shape, not the fixture's: `iron-plate` is produced here --
        // by a crushing arm, as Space Age produces its ore -- so the ONLY base
        // case is the charted ore under the bots' feet.
        let mut recipes = vanilla_recipes();
        recipes.push(recipe("iron-plate", "smelting", &[("iron-ore", 1)], 1));
        recipes.push(recipe("iron-ore", "crushing", &[("iron-plate", 2)], 1));
        recipes.push(recipe("copper-plate", "smelting", &[("iron-ore", 1)], 1));
        recipes.push(recipe("stone", "crushing", &[("iron-ore", 2)], 1));
        world.update_recipes(recipes).expect("update_recipes");
        let mut ore = Vec::new();
        spawn_ore(
            &mut ore,
            Rect::new(&Position::new(-4.0, -4.0), &Position::new(-1.0, -1.0)),
            "iron-ore",
        );
        world.entity_graph.add(ore, None).expect("ore is charted");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let table = MachineTable::from_state(&state);
        assert_eq!(
            table.machine_for("crafting-with-fluid"),
            Ok(Machine::Entity("assembling-machine-2".into()))
        );
    }
    /// **Cost, not name.** Every candidate in the shipped fixtures happens to
    /// sort in cost order (`assembling-machine-1` is both first and cheapest),
    /// so those tests are equally explained by an alphabetical rule -- a
    /// falsification sweep said so, mutating the sort to compare names and
    /// finding nothing that objected. Here the cheapest sorts LAST.
    #[test]
    fn cost_decides_and_not_the_name() {
        let protos = [
            proto("alpha-mill", "assembling-machine", Some(&["milling"])),
            proto("zeta-mill", "assembling-machine", Some(&["milling"])),
        ];
        let recipes = [
            recipe("alpha-mill", "crafting", &[("iron-plate", 40)], 1),
            recipe("zeta-mill", "crafting", &[("iron-plate", 4)], 1),
        ];
        let table = MachineTable::from_parts_and_recipes(protos.iter(), recipes.iter());
        assert_eq!(
            table.machine_for("milling"),
            Ok(Machine::Entity("zeta-mill".into()))
        );
    }

    /// The arithmetic, in thousandths of a raw input, spelled out for three
    /// items: a recipe that **yields two** costs half as much each, and that
    /// halving carries all the way up.
    ///
    /// Also from the sweep: without this, dropping the division by
    /// `product.amount` changed every price and no test objected, because the
    /// *ordering* survived.
    #[test]
    fn a_price_counts_the_yield_and_the_ingredient_amounts() {
        let wanted: BTreeSet<String> =
            ["copper-cable", "electronic-circuit", "assembling-machine-1"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let costs = obtain_costs(vanilla_recipes().iter(), &wanted, &BTreeSet::new());
        // One copper plate makes two cables.
        assert_eq!(costs["copper-cable"], 500);
        // One iron plate plus three cables.
        assert_eq!(costs["electronic-circuit"], 2_500);
        // 3 circuits + 5 gears (2 iron each) + 9 iron plates.
        assert_eq!(costs["assembling-machine-1"], 26_500);
    }
    /// **The measured defect the fixtures could not show.** Space Age makes
    /// `iron-ore`, `stone` and `coal` out of asteroids, so on a real world
    /// nothing at all is un-produced and a cost graph with no ground seed has
    /// no base case: every machine prices infinite and the preference goes
    /// inert. That is the safe direction -- the refusal is the one that stood
    /// before -- which is exactly why it was invisible until the rung was
    /// planned against the dump and did not move.
    ///
    /// Here `ore` is both a recipe's product and a thing in the ground.
    #[test]
    fn a_world_where_everything_is_produced_needs_the_ground_to_price() {
        let protos = [
            proto("small-mill", "assembling-machine", Some(&["milling"])),
            proto("big-mill", "assembling-machine", Some(&["milling"])),
        ];
        let recipes = [
            // The asteroid arm: ore comes from a recipe as well as the ground.
            recipe("ore", "crushing", &[("chunk", 1)], 1),
            recipe("chunk", "crushing", &[("ore", 2)], 1),
            recipe("small-mill", "crafting", &[("ore", 4)], 1),
            recipe("big-mill", "crafting", &[("ore", 40)], 1),
        ];
        let unseeded = MachineTable::from_parts_and_recipes(protos.iter(), recipes.iter());
        assert!(
            unseeded.obtain_cost.is_empty(),
            "nothing can be priced without a base case: {:?}",
            unseeded.obtain_cost
        );
        assert!(matches!(
            unseeded.machine_for("milling"),
            Err(MachineRefusal::Ambiguous { .. })
        ));

        let ground: BTreeSet<String> = ["ore".to_string()].into_iter().collect();
        let seeded =
            MachineTable::from_parts_recipes_and_ground(protos.iter(), recipes.iter(), &ground);
        assert_eq!(
            seeded.machine_for("milling"),
            Ok(Machine::Entity("small-mill".into()))
        );
    }
}
