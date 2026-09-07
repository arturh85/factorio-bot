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
//! # Ambiguity is refused, never resolved
//!
//! Vanilla declares `advanced-crafting` on all three assembling machines and
//! `crafting-with-fluid` on two. Picking the first by name would be the
//! silent answer this crate's [`crate::products`] module exists to stop, so
//! several machines is [`MachineRefusal::Ambiguous`] — the same shape as
//! [`crate::products::ProductRefusal::Ambiguous`].
//!
//! # Cost
//!
//! Building walks the entity-prototype table once. **Build one per expansion,
//! not one per lookup** — the same rule [`crate::products::ProductIndex`] and
//! [`crate::substance::SubstanceTable`] state, for the same reason.

use crate::method::produce::FURNACE;
use crate::method::util::{CRAFTING_CATEGORY, SMELTING_CATEGORY};
use crate::state::PlanState;
use factorio_bot_core::types::FactorioEntityPrototype;
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
    pub fn from_parts<'a, P>(prototypes: P) -> Self
    where
        P: IntoIterator<Item = &'a FactorioEntityPrototype>,
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
        MachineTable::from_parts(protos.iter())
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
            _ => Err(MachineRefusal::Ambiguous {
                category: category.to_string(),
                machines,
            }),
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
        for (category, machines) in &self.declared {
            if machines.len() == 1 {
                out.insert(category.clone());
            }
        }
        out.into_iter().collect()
    }
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
    /// and it is one computation.
    #[test]
    fn the_runnable_set_is_the_categories_with_exactly_one_machine() {
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
}
