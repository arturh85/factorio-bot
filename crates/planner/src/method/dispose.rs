//! Where a fluid co-product goes when nothing in the plan wants it.
//!
//! # The rung this is
//!
//! [`crate::method::fabricate`] gives every fluid product of a recipe its own
//! buffer, and a buffer is a sink for a **bounded** goal and nothing more: it
//! holds what this goal's crafts put in it and then the machine stalls.
//! `advanced-oil-processing` makes heavy-oil, light-oil **and** petroleum-gas
//! out of one craft, and a refinery whose heavy has nowhere to go stops
//! producing light and petroleum as well -- so a plan that wants light oil has
//! to say what becomes of the other two.
//!
//! This module answers that, and answers only that: it picks the recipe that
//! consumes the surplus. Standing the machine up, siting it, piping it and
//! refusing when it cannot be done are all `fabricate`'s, unchanged --
//! disposal is emitted as `Goal::Produced { item, count, via }`, which is an
//! existing goal shape and needs no tenth goal kind.
//!
//! # Termination is by TYPE, not by a depth limit
//!
//! **A terminal consumer of fluid `f` is a recipe that takes `f` and whose
//! products contain no fluid.** Disposal selects only from that set, so a
//! disposal step introduces zero new fluid outputs, so the fluid recursion is
//! **exactly one deep by construction**. The chain ends in items, which is the
//! planner's home ground: `Goal::Produced { item: "solid-fuel", .. }` is an
//! ordinary item goal from the moment it is stated.
//!
//! A depth limit would have bounded the same recursion by counting, and would
//! have said nothing about what the last rung *is*. This says it: the last
//! rung makes an item.
//!
//! # Cracking is deliberately excluded, and the exclusion is the point
//!
//! `heavy-oil-cracking` and `light-oil-cracking` are fluid-to-fluid -- exactly
//! the recursive case the terminal rule removes. They are not disposal: they
//! are the **ratio** instrument, reconciling a refinery's fixed split (25
//! heavy / 45 light / 55 petroleum per craft) with a demand vector that is
//! not fixed. That is a rate question, [`crate::method::have::demand`] answers
//! only for bounded goals, and the owner has sequenced it as its own rung. See
//! `fabricate`'s module doc.
//!
//! Nothing here needs to *know* about cracking to exclude it. A cracking
//! recipe has a fluid product and is therefore not terminal, and
//! [`a_fluid_to_fluid_recipe_is_never_chosen_even_when_it_would_win`] pins
//! that against a fixture built to make cracking win every tiebreak it can.
//!
//! # Selection, in order
//!
//! 1. **Fewest ingredients other than `f`.** Disposal must not open a new
//!    bill: `solid-fuel-from-heavy-oil` takes 20 heavy-oil and nothing else,
//!    while `plastic-bar` wants coal and `sulfur` wants water. A disposal that
//!    drags in a second supply chain is a worse plan than the surplus it
//!    removes.
//! 2. **Existing obtain cost of that bill**, by
//!    [`crate::products::ProductIndex::bill_cost_excluding`] -- the same metric
//!    [`crate::products::ProductIndex`] ranks producers by, called rather than
//!    reproduced. A recipe whose bill cannot be priced is ranked *after* every
//!    priced one rather than dropped: unpriceable is not expensive, and this
//!    is a preference rather than a filter.
//! 3. **Recipe name**, so two runs over one world make the same plan.
//!
//! Measured on the seed-31337 water-and-oil capture, rule 1 alone decides
//! every Nauvis oil fluid, because each has exactly one terminal consumer with
//! an empty bill:
//!
//! ```text
//! heavy-oil      solid-fuel-from-heavy-oil       (0 other ingredients)
//! light-oil      solid-fuel-from-light-oil       (0)
//! petroleum-gas  solid-fuel-from-petroleum-gas   (0)
//! ```
//!
//! and the runners-up are the ones the rule is written to beat -- `plastic-bar`
//! (coal), `sulfur` (water), `superconductor`, and the three `fill-*-barrel`
//! recipes, which are not disposal at all since `empty-*-barrel` inverts them.
//!
//! # A candidate is verified by the method that will have to run it
//!
//! The winner is not merely category-admitted here: the goal that would be
//! emitted is handed to [`crate::method::fabricate::job_for`], the same
//! function `Fabricate::applicable` asks. So a disposal goal this module emits
//! is one `Fabricate` will claim, rather than one that looks plausible from
//! here and is then declined -- the failure mode where a method emits a
//! subgoal nothing can satisfy.

use crate::goal::{Goal, Holder};
use crate::ids::ItemId;
use crate::method::machine::MachineTable;
use crate::products::{Categories, ProductIndex};
use crate::state::PlanState;
use crate::substance::SubstanceTable;
use factorio_bot_core::types::FactorioRecipe;

/// A recipe that will consume a fluid surplus, and the arithmetic that turns
/// an amount of fluid into a count of the item it becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Disposal {
    /// The recipe to run, named on the emitted goal's `via`.
    pub(crate) recipe: String,
    /// The item to ask for. Named rather than left to the planner because
    /// `via` pins the recipe and the goal has to state a product of it.
    pub(crate) product: String,
    /// How much of `product` one craft makes. Non-zero by construction.
    pub(crate) product_amount: u32,
    /// How much of the fluid one craft eats. Non-zero by construction.
    pub(crate) consumed: u32,
}

impl Disposal {
    /// The goal that disposes of `total` of the fluid.
    ///
    /// **Rounded up**, because the question is *"is there somewhere for all of
    /// it to go"*: a plant sized to eat 24 of 25 heavy-oil leaves the refinery
    /// to jam on the twenty-fifth, which is the whole failure this rung
    /// exists to remove. Rounding up over-provisions instead, and an idle
    /// chemical plant costs a plan nothing that a stalled refinery does not
    /// cost it more.
    ///
    /// `count` is `crafts * product_amount` and not `crafts` so that
    /// `Fabricate`'s own `need.div_ceil(output_per_craft)` recovers exactly
    /// `crafts` -- the two arithmetics meet rather than approximately agree.
    pub(crate) fn goal(&self, total: u64, whose: &Holder) -> Goal {
        let crafts = total.div_ceil(u64::from(self.consumed));
        let count = crafts.saturating_mul(u64::from(self.product_amount));
        Goal::Produced {
            item: ItemId::from(self.product.clone()),
            count: u32::try_from(count).unwrap_or(u32::MAX),
            whose: whose.clone(),
            unlocks: None,
            via: Some(self.recipe.clone()),
        }
    }
}

/// Is every product of `recipe` an item?
///
/// **The whole termination argument is this predicate**, so it is written once
/// and asked once. A recipe with no products at all answers `true` vacuously
/// and is harmless: it consumes the fluid and produces nothing to place, which
/// is the strongest possible form of terminal. It is also refused later by
/// [`Disposal`]'s own `product_amount` requirement, since there is no product
/// to name.
fn is_terminal(substances: &SubstanceTable, recipe: &FactorioRecipe) -> bool {
    !recipe
        .products
        .iter()
        .any(|product| substances.is_fluid(&product.name))
}

/// The item product of a terminal recipe that a `Goal::Produced` should name.
///
/// The largest amount, then the name. Largest first because the goal's count
/// is `crafts * amount` and `Fabricate` divides it back out: a bigger amount
/// is not more accurate here (the division is exact either way), but it is the
/// product a reader would call the recipe's output, and a tie broken by name
/// alone would name `barrel` rather than `heavy-oil-barrel`.
fn principal_product(recipe: &FactorioRecipe) -> Option<(String, u32)> {
    recipe
        .products
        .iter()
        .filter(|p| p.amount > 0)
        .max_by(|a, b| a.amount.cmp(&b.amount).then_with(|| b.name.cmp(&a.name)))
        .map(|p| (p.name.clone(), p.amount))
}

/// How much of `fluid` one craft of `recipe` consumes, or `None` when it does
/// not consume it at all.
fn consumed_amount(recipe: &FactorioRecipe, fluid: &str) -> Option<u32> {
    recipe
        .ingredients
        .iter()
        .flatten()
        .find(|i| i.name == fluid)
        .map(|i| i.amount)
        .filter(|amount| *amount > 0)
}

/// Every terminal consumer of `fluid` this planner could run, ranked by the
/// module doc's three keys, best first.
///
/// Separate from [`choose`] so a test can see the whole ordering rather than
/// only the winner -- an ordering asserted through its first element alone is
/// three rules of which one is exercised.
pub(crate) fn ranked_consumers<'a>(
    index: &'a ProductIndex,
    categories: &Categories,
    machines: &MachineTable,
    fluid: &str,
) -> Vec<&'a FactorioRecipe> {
    let mut ranked: Vec<(usize, Option<u64>, &str, &FactorioRecipe)> = index
        .recipes_consuming(fluid)
        .into_iter()
        .filter(|recipe| is_terminal(index.substances(), recipe))
        .filter(|recipe| consumed_amount(recipe, fluid).is_some())
        .filter(|recipe| principal_product(recipe).is_some())
        .filter(|recipe| categories.admits(&recipe.category))
        .filter(|recipe| machines.machine_for(&recipe.category).is_ok())
        .map(|recipe| {
            let others = recipe
                .ingredients
                .iter()
                .flatten()
                .filter(|i| i.name != fluid)
                .count();
            (
                others,
                index.bill_cost_excluding(recipe, fluid),
                recipe.name.as_str(),
                recipe,
            )
        })
        .collect();
    // `Option<u64>` orders `None` last under this comparator by construction:
    // it is compared as `(is_none, value)`, so an unpriceable bill sits after
    // every priced one instead of ahead of a bill costing zero. `Ord` on
    // `Option` would put `None` **first**, which is the opposite of what
    // "cannot be priced" should mean here.
    fn key<'k>(x: &(usize, Option<u64>, &'k str, &FactorioRecipe)) -> (usize, bool, u64, &'k str) {
        (x.0, x.1.is_none(), x.1.unwrap_or(0), x.2)
    }
    ranked.sort_by(|a, b| key(a).cmp(&key(b)));
    ranked.into_iter().map(|(_, _, _, recipe)| recipe).collect()
}

/// The terminal consumer to dispose of `fluid` with, or `None` when this world
/// offers none this planner can run.
///
/// `None` is a genuine *"there is nowhere for it to go"*, and the caller must
/// treat it as a wall rather than proceed with the buffer alone -- see
/// [`crate::method::fabricate::FabricateRefusal::NoDisposal`]. A machine whose
/// co-product has only a buffer places 100% correctly and stops when the
/// buffer fills.
pub(crate) fn choose(state: &PlanState, fluid: &str, whose: &Holder) -> Option<Disposal> {
    let machines = MachineTable::from_state(state);
    let index = ProductIndex::from_state(state);
    let categories = Categories::planner_runs(&machines);
    for recipe in ranked_consumers(&index, &categories, &machines, fluid) {
        let (product, product_amount) = principal_product(recipe)?;
        let Some(consumed) = consumed_amount(recipe, fluid) else {
            continue;
        };
        let disposal = Disposal {
            recipe: recipe.name.clone(),
            product,
            product_amount,
            consumed,
        };
        // **Asked of the method that will have to run it.** One craft's worth
        // is enough to settle whether the goal is claimable -- `job_for` reads
        // the recipe, the category and the gate, none of which depend on the
        // count.
        if crate::method::fabricate::job_for(&disposal.goal(u64::from(consumed), whose), state)
            .is_some()
        {
            return Some(disposal);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::types::FactorioRecipe;

    fn recipe(json: &str) -> FactorioRecipe {
        serde_json::from_str(json).expect("the fixture recipe parses")
    }

    /// The three Nauvis oil fluids' real competitors, transcribed from the
    /// seed-31337 water-and-oil capture rather than invented -- amounts,
    /// categories and product types all as the game reports them.
    fn oil_recipes() -> Vec<FactorioRecipe> {
        vec![
            recipe(
                r#"{"name":"solid-fuel-from-light-oil","valid":true,"enabled":true,
                   "category":"chemistry",
                   "ingredients":[{"name":"light-oil","ingredient_type":"fluid","amount":10}],
                   "products":[{"name":"solid-fuel","product_type":"item","amount":1,
                                "probability":1.0}],
                   "hidden":false,"energy":1.0,"order":"a","group":"g","subgroup":"s"}"#,
            ),
            recipe(
                r#"{"name":"light-oil-cracking","valid":true,"enabled":true,
                   "category":"chemistry",
                   "ingredients":[{"name":"water","ingredient_type":"fluid","amount":30},
                                  {"name":"light-oil","ingredient_type":"fluid","amount":30}],
                   "products":[{"name":"petroleum-gas","product_type":"fluid","amount":20,
                                "probability":1.0}],
                   "hidden":false,"energy":2.0,"order":"a","group":"g","subgroup":"s"}"#,
            ),
            recipe(
                r#"{"name":"fill-light-oil-barrel","valid":true,"enabled":true,
                   "category":"crafting-with-fluid",
                   "ingredients":[{"name":"light-oil","ingredient_type":"fluid","amount":50},
                                  {"name":"barrel","ingredient_type":"item","amount":1}],
                   "products":[{"name":"light-oil-barrel","product_type":"item","amount":1,
                                "probability":1.0}],
                   "hidden":false,"energy":0.2,"order":"a","group":"g","subgroup":"s"}"#,
            ),
            recipe(
                r#"{"name":"barrel","valid":true,"enabled":true,"category":"crafting",
                   "ingredients":[{"name":"steel-plate","ingredient_type":"item","amount":1}],
                   "products":[{"name":"barrel","product_type":"item","amount":1,
                                "probability":1.0}],
                   "hidden":false,"energy":1.0,"order":"a","group":"g","subgroup":"s"}"#,
            ),
        ]
    }

    fn index_of(recipes: &[FactorioRecipe]) -> ProductIndex {
        ProductIndex::from_parts(
            recipes.iter(),
            ["solid-fuel", "barrel", "light-oil-barrel", "steel-plate"],
        )
        .with_ground_supply(["steel-plate", "water", "light-oil"])
    }

    /// A table naming a machine for the categories the fixtures use, so the
    /// `machine_for` filter is exercised rather than short-circuited by an
    /// empty world.
    fn machines() -> MachineTable {
        MachineTable::from_parts_and_recipes(
            [
                &prototype("chemical-plant", &["chemistry"]),
                &prototype("assembling-machine-2", &["crafting", "crafting-with-fluid"]),
            ],
            std::iter::empty(),
        )
    }

    /// Deserialised rather than built as a literal, for the reason
    /// `machine.rs`'s own fixture gives: the serde attributes decide the wire
    /// shape, and a literal would not read them.
    fn prototype(
        name: &str,
        categories: &[&str],
    ) -> factorio_bot_core::types::FactorioEntityPrototype {
        let cats = factorio_bot_core::serde_json::to_string(categories).expect("json");
        factorio_bot_core::serde_json::from_str(&format!(
            r#"{{ "name": "{name}", "entity_type": "assembling-machine",
                  "collision_mask": [],
                  "collision_box": {{ "left_top": {{"x": -1.5, "y": -1.5}},
                                     "right_bottom": {{"x": 1.5, "y": 1.5}} }},
                  "crafting_categories": {cats} }}"#
        ))
        .expect("a prototype in the shape the mod sends")
    }

    fn names(recipes: &[&FactorioRecipe]) -> Vec<String> {
        recipes.iter().map(|r| r.name.clone()).collect()
    }

    /// Rule 1, and the whole point of the module: cracking is not a candidate
    /// at all, however the tiebreaks would fall.
    #[test]
    fn a_fluid_to_fluid_recipe_is_never_chosen_even_when_it_would_win() {
        // A cracking recipe built to win every rule this module has: an empty
        // bill beyond the fluid (rule 1 ties at 0), nothing to price (rule 2
        // ties at 0), and a name that sorts first (rule 3 would take it).
        let mut recipes = oil_recipes();
        recipes.push(recipe(
            r#"{"name":"aaa-cracking","valid":true,"enabled":true,"category":"chemistry",
               "ingredients":[{"name":"light-oil","ingredient_type":"fluid","amount":30}],
               "products":[{"name":"petroleum-gas","product_type":"fluid","amount":20,
                            "probability":1.0}],
               "hidden":false,"energy":2.0,"order":"a","group":"g","subgroup":"s"}"#,
        ));
        let index = index_of(&recipes);
        let machines = machines();
        let ranked = ranked_consumers(
            &index,
            &Categories::planner_runs(&machines),
            &machines,
            "light-oil",
        );
        assert!(
            !names(&ranked).iter().any(|n| n.ends_with("cracking")),
            "a recipe with a fluid product is not a terminal consumer, and this fixture's \
             `aaa-cracking` would otherwise win all three keys: {:?}",
            names(&ranked)
        );
        assert_eq!(
            names(&ranked).first().map(String::as_str),
            Some("solid-fuel-from-light-oil"),
        );
    }

    /// Rule 1 on its own: the barrel route is terminal and legal, and loses
    /// because it opens a bill.
    #[test]
    fn the_terminal_consumer_with_the_emptier_bill_wins() {
        let recipes = oil_recipes();
        let index = index_of(&recipes);
        let machines = machines();
        assert_eq!(
            names(&ranked_consumers(
                &index,
                &Categories::planner_runs(&machines),
                &machines,
                "light-oil"
            )),
            vec!["solid-fuel-from-light-oil", "fill-light-oil-barrel"],
            "both are terminal; the one that needs a barrel is second"
        );
    }

    /// Rules 2 and 3, which rule 1 hides on the real oil fluids because every
    /// winner there has an empty bill.
    #[test]
    fn a_tie_on_bill_size_is_broken_by_cost_and_then_by_name() {
        let mut recipes = oil_recipes();
        // Three one-ingredient terminal consumers. `zzz` is cheapest, so it
        // must beat both alphabetically-earlier ones; `aab` and `aac` tie on
        // cost and must then order by name.
        for (name, ingredient) in [
            ("zzz-dump", "coal"),
            ("aab-dump", "steel-plate"),
            ("aac-dump", "steel-plate"),
        ] {
            recipes.push(recipe(&format!(
                r#"{{"name":"{name}","valid":true,"enabled":true,"category":"chemistry",
                   "ingredients":[{{"name":"light-oil","ingredient_type":"fluid","amount":10}},
                                  {{"name":"{ingredient}","ingredient_type":"item","amount":1}}],
                   "products":[{{"name":"solid-fuel","product_type":"item","amount":1,
                                "probability":1.0}}],
                   "hidden":false,"energy":1.0,"order":"a","group":"g","subgroup":"s"}}"#
            )));
        }
        recipes.push(recipe(
            r#"{"name":"coal-from-nothing","valid":true,"enabled":true,"category":"crafting",
               "ingredients":[],
               "products":[{"name":"coal","product_type":"item","amount":10,
                            "probability":1.0}],
               "hidden":false,"energy":1.0,"order":"a","group":"g","subgroup":"s"}"#,
        ));
        let index = ProductIndex::from_parts(
            recipes.iter(),
            [
                "solid-fuel",
                "barrel",
                "light-oil-barrel",
                "steel-plate",
                "coal",
            ],
        )
        .with_ground_supply(["steel-plate", "water", "light-oil"]);
        let machines = machines();
        let ranked = names(&ranked_consumers(
            &index,
            &Categories::planner_runs(&machines),
            &machines,
            "light-oil",
        ));
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("solid-fuel-from-light-oil"),
            "rule 1 still decides the winner: {ranked:?}"
        );
        let one_ingredient: Vec<&String> = ranked.iter().filter(|n| n.ends_with("-dump")).collect();
        assert_eq!(
            one_ingredient,
            vec!["zzz-dump", "aab-dump", "aac-dump"],
            "cheapest bill first, then name -- not name alone"
        );
    }

    /// The arithmetic, which is what stops a plant being sized to eat all but
    /// the last of a surplus.
    #[test]
    fn the_disposal_count_rounds_up_and_fabricate_divides_it_back_out() {
        let disposal = Disposal {
            recipe: "solid-fuel-from-heavy-oil".into(),
            product: "solid-fuel".into(),
            product_amount: 1,
            consumed: 20,
        };
        // One refinery craft makes 25 heavy-oil; two disposal crafts eat 40.
        let Goal::Produced { count, via, .. } = disposal.goal(25, &Holder::Anyone) else {
            panic!("a disposal goal is a Goal::Produced");
        };
        assert_eq!(count, 2, "25 heavy-oil needs two crafts, not one");
        assert_eq!(via.as_deref(), Some("solid-fuel-from-heavy-oil"));
        assert_eq!(
            disposal.goal(0, &Holder::Anyone),
            Goal::Produced {
                item: ItemId::from("solid-fuel".to_string()),
                count: 0,
                whose: Holder::Anyone,
                unlocks: None,
                via: Some("solid-fuel-from-heavy-oil".into()),
            },
            "nothing to dispose of asks for nothing -- the caller skips it"
        );
    }

    /// A recipe whose product is a fluid the world has never heard of would be
    /// classified by name alone; this pins that the *substance table* decides,
    /// so a world that calls something a fluid makes it non-terminal.
    #[test]
    fn terminality_is_asked_of_the_worlds_own_substance_table() {
        let recipes = [recipe(
            r#"{"name":"x-to-y","valid":true,"enabled":true,"category":"chemistry",
               "ingredients":[{"name":"x","ingredient_type":"fluid","amount":10}],
               "products":[{"name":"y","product_type":"fluid","amount":1,
                            "probability":1.0}],
               "hidden":false,"energy":1.0,"order":"a","group":"g","subgroup":"s"}"#,
        )];
        let index = ProductIndex::from_parts(recipes.iter(), std::iter::empty());
        assert!(
            !is_terminal(index.substances(), &recipes[0]),
            "`y` is declared a fluid by the recipe that makes it"
        );
    }
}
