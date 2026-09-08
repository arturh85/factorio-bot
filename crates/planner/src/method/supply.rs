//! Stand a fluid's source up so that [`crate::method::fabricate`] can adopt
//! one.
//!
//! # The missing edge, not a missing subsystem
//!
//! `method::fabricate`'s own doc states the position this module is the other
//! half of: it *"adopts a source and never builds one"*. That is right --
//! a tank this plan places is empty, and a method that quietly built its own
//! source would be promising a fluid nobody put there. But nothing anywhere
//! built one either: before this module, `Goal::Gathered` was constructed in
//! exactly two places in the tree, both of them the **Lua/CLI goal parser**
//! (`crates/scripting_lua/src/globals/goal/value.rs`), and by no method at
//! all. So the capability existed and was unreachable from any goal a caller
//! would naturally state.
//!
//! That is why the oil rig had to be asked for as two goals:
//!
//! ```text
//! --goal gathered:crude-oil --goal produced:petroleum-gas:45:basic-oil-processing
//! ```
//!
//! The second implies the first -- petroleum comes from crude -- and the
//! planner could not say so. It refused instead, in
//! [`crate::method::fabricate::FabricateRefusal::NoFluidSource`]'s own words:
//! *"Nothing standing on this map can be shown to supply crude-oil"*. That
//! refusal is honest about the **world** and wrong about the **plan**: a plan
//! is exactly the thing that can change what is standing.
//!
//! # What it does
//!
//! For a goal `Fabricate` would claim, whose recipe wants a fluid that
//! [`pipe::sources_of`] cannot attribute to anything standing, and which this
//! world could be made to supply, this emits:
//!
//! 1. one goal per such fluid -- `Goal::Gathered { entity: <fluid> }` where
//!    [`gather::Gather`] could stand a source up on the ground, and
//!    `Goal::Produced { item: <fluid>, .. }` where a recipe makes it instead
//!    (the chemistry rung, below) -- and
//! 2. the original goal again.
//!
//! The driver expands subgoals depth-first and applies each emitted action's
//! effects to the plan overlay as it goes (`method::run_steps`), so by the
//! time (2) is expanded the pumpjack and the buffer tank
//! `gather::Gather::expand` placed are `Effect::CreateEntity`-standing in the
//! overlay -- and `pipe::sources_of` rule 3, *"a `storage-tank`-typed entity
//! within `gather::FIELD_RADIUS` of a charted tile of the fluid"*, attributes
//! the tank. This method is then no longer applicable to (2) and `Fabricate`
//! claims it, unchanged.
//!
//! **So no new machinery is built here and none is duplicated.** The rig, the
//! pipe run, the siting and the refusals all stay where they are; what was
//! missing was a method willing to say `gathered:` on the planner's own
//! behalf.
//!
//! # Termination
//!
//! Re-issuing the caller's own goal is the shape `Withdraw` and `Stockpile`
//! already use, and it terminates for the same reason they do: the state the
//! next expansion reads has changed. If `Gather` were ever to succeed without
//! leaving anything `sources_of` can attribute, (2) would come back here and
//! the driver's `MAX_EXPANSION_DEPTH` would name it
//! (`PlannerError::ExpansionTooDeep`) rather than hang. That is a guard, not
//! the argument.
//!
//! # The chemistry rung, added 2026-09-08
//!
//! The paragraph that stood here said a fluid *made* rather than gathered was
//! "a separate decision because the sink -- who the plant pipes **to** -- is
//! not settled". **The sink was already settled, in `Fabricate`**, and this
//! was the missing-edge shape all over again rather than a missing subsystem.
//! Two facts, both read off the code rather than inferred:
//!
//! 1. `method::fabricate` *"gives a fluid product a sink or refuses by
//!    name"* -- it sites a buffer beside the machine, pipes the output into
//!    it, and refuses as `SinkTooSmall` / `NoSinkSite` when it cannot. So a
//!    plant this planner stands up is never a machine whose output has
//!    nowhere to go.
//! 2. [`pipe::sources_of`] rule 1 attributes a machine by the recipe it is
//!    running -- *"a refinery running `basic-oil-processing` is a petroleum
//!    source"* -- and `Effect::SetRecipe` writes exactly that field into the
//!    plan overlay. So the plant this method asks for **becomes** the source
//!    the caller's goal needs, with no new attribution rule.
//!
//! What was missing was only the edge: nothing emitted a goal for the fluid.
//! So a fluid the ground cannot yield but a recipe can make now gets
//! `Goal::Produced { item: <fluid>, .. }` in place of `Goal::Gathered`, and
//! the rest of the machinery is the same machinery.
//!
//! **`pipe::ACCEPTING` is still absent and is still not needed.** Adopting a
//! *standing* consumer as the sink is the optimisation that note describes;
//! building one is what `Fabricate` already does.
//!
//! # What it deliberately does not do
//!
//! * **It does not build a source for a fluid this world can neither yield
//!   nor make.** Both tests are asked before anything is claimed: `Gather`'s
//!   own -- charted patches plus a nameable extractor -- and then
//!   [`ProductIndex::recipe_producing`](crate::products::ProductIndex::recipe_producing),
//!   the same one `Fabricate` selects with. A fluid neither answers for
//!   reaches `Fabricate`'s `NoFluidSource` exactly as it does today. `water`
//!   on an archived dump is precisely that case: it is not a charted
//!   resource and no recipe makes it, so it is reachable only from a dump
//!   whose tiles carry `fluid` (see `products::ground_supply`), and
//!   `have:sulfur:10` still refuses on water there.
//! * **It does not decide which recipe.** The choice is
//!   `recipe_producing`'s, so this method and `Fabricate` cannot disagree
//!   about it -- the same reason `recipe_fabricate_would_run` calls
//!   `job_for` instead of mirroring it. Where that choice lands on a
//!   **multi-output** recipe -- `advanced-oil-processing`, three fluids out
//!   -- `FabricateRefusal::ManyFluidProducts` is a wall reached before a
//!   single step is emitted, which is the honest answer rather than a rig
//!   that plans green and backs up in the game. See the note under
//!   `FluidPlan::Produced`.
//! * **It does not promise the tank has anything in it.** Nothing in
//!   `PlanState` models fluid contents (`pipe::sources_of`, "what it cannot
//!   say"), so this is a *connectivity* derivation and nothing stronger --
//!   which is the same thing the two-goal form the owner types today buys.

use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::method::machine::MachineTable;
use crate::method::util::{RecipeGate, recipe_gate};
use crate::method::{ExpansionCtx, Method, Step};
use crate::method::{extract, pipe};
use crate::products::{Categories, ProductIndex};
use crate::state::PlanState;
use factorio_bot_core::types::{FactorioRecipe, Position};

/// The fluid ingredient type, as the game declares it on a recipe.
///
/// Deliberately the ingredient's **own** declaration and not
/// [`crate::substance::SubstanceTable`]: this runs inside
/// [`Method::applicable`], which has a `&PlanState` and no
/// [`ExpansionCtx`] to hold the table's cache, and building one per
/// applicability probe would put a whole-world scan on the hot path of every
/// goal no earlier method claimed. The cost of the narrower test is a recipe
/// from an old capture that leaves `ingredient_type` blank: that fluid is not
/// derived here and refuses in `Fabricate` as it does today, which is the
/// current behaviour rather than a new failure. Positive evidence only.
const FLUID_TYPE: &str = "fluid";

/// The recipe `Fabricate` would run for this goal, chosen the way it chooses.
///
/// # It asks `Fabricate`, rather than reproducing its reasoning
///
/// This was a ~30-line mirror of `fabricate::job_for` while that function was
/// private, with the drift documented as tolerable because a disagreement
/// would make this method decline and leave the goal to `Fabricate` unchanged.
/// The mirror is gone: `job_for` is `pub(crate)` and this calls it, so the two
/// **cannot** disagree about which recipe.
///
/// Removing it also closed a real gap rather than merely deduplicating. The
/// mirror never applied `recipe_gate`, so it would answer for a recipe
/// `Fabricate` rejects as [`RecipeGate::Unobtainable`](crate::method::util::RecipeGate)
/// — a recipe no technology can ever unlock. This method would then have
/// emitted a `Goal::Gathered` to feed a recipe that can never run.
fn recipe_fabricate_would_run(goal: &Goal, state: &PlanState) -> Option<FactorioRecipe> {
    super::fabricate::job_for(goal, state).map(|job| job.recipe)
}

/// A fluid this goal's recipe wants, that nothing standing supplies, and the
/// way this planner would come to have a source of it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FluidPlan {
    /// **The ground yields it.** `Gather` stands an extractor and a buffer
    /// tank up on a charted patch, and `pipe::sources_of` rule 3 attributes
    /// the tank.
    Gathered(String),
    /// **A recipe makes it.** `Fabricate` stands the plant up, sets its
    /// recipe, and sites a buffer for the output; `pipe::sources_of` rule 1
    /// then attributes the plant *by the recipe it is running*.
    ///
    /// # The multi-output trap, and what is done about it
    ///
    /// `basic-oil-processing` produces only petroleum-gas.
    /// `advanced-oil-processing` produces heavy-oil, light-oil **and**
    /// petroleum-gas from one recipe, and a refinery running it whose heavy
    /// and light have nowhere to go backs up and then produces no petroleum
    /// either -- a rig that plans green, places 100% and moves nothing, this
    /// repo's signature failure.
    ///
    /// **Nothing new guards it, because the guard already exists and is a
    /// wall**: `FabricateRefusal::ManyFluidProducts` is returned before a
    /// step is emitted or an entity lands in the overlay. So a `via` that
    /// lands on a multi-output recipe refuses **by name** rather than
    /// building the rig. That is the deliberate decision here: co-product
    /// disposal (cracking) does not exist yet, so a multi-output recipe is
    /// refused until it does.
    ///
    /// It is not academic. On the seed-31337 explored dump the obtain-cost
    /// preference ranks `basic-oil-processing` (2.222) strictly ahead of
    /// `advanced-oil-processing` (2.273), so **petroleum-gas plans**; but
    /// light-oil and heavy-oil have no single-output producer this surface
    /// can feed, so `advanced-oil-processing` wins there and
    /// `have:rocket-fuel:1` refuses as `ManyFluidProducts`. Correct, named,
    /// and blocked on cracking rather than on this method.
    Produced { fluid: String, amount: u32 },
}

impl FluidPlan {
    fn fluid(&self) -> &str {
        match self {
            FluidPlan::Gathered(fluid) => fluid,
            FluidPlan::Produced { fluid, .. } => fluid,
        }
    }
}

/// Could this world be made to *make* `fluid`, by the same reasoning
/// `Fabricate` would use when it got there?
///
/// [`ProductIndex::recipe_producing`] is the one encoding of "which recipe",
/// so this cannot disagree with what `Fabricate` will then pick for the
/// emitted goal. Two things are checked on top of it, both of which would
/// otherwise emit a goal that can never be satisfied:
///
/// * an [`RecipeGate::Unobtainable`] recipe -- one no technology unlocks --
///   is exactly what `job_for`'s own gate rejects, and emitting a goal for it
///   would be the gap the mirror in `recipe_fabricate_would_run` used to have;
/// * a recipe that **is the caller's own** would re-issue the goal with
///   nothing changed. `MAX_EXPANSION_DEPTH` would eventually name it, but a
///   guard that costs a string comparison is better than a depth limit.
fn makeable(state: &PlanState, fluid: &str, consumer: &FactorioRecipe) -> bool {
    let machines = MachineTable::from_state(state);
    let index = ProductIndex::from_state(state);
    // **A fluid the GROUND gives you is never manufactured**, whatever recipe
    // also names it as a product.
    //
    // This guard is not theoretical -- it is the first thing the chemistry
    // rung got wrong, on `have:sulfur:10` against the seed-31337
    // water-and-oil dump. Sulfur wants water; water has no charted *patch*,
    // so the `Gathered` branch above declines; and the one recipe producing
    // water is **`empty-water-barrel`**, so the rung asked for a barrelling
    // plant and the plan died two stages later on `no run of poles ...
    // carries power to it`. A rig to unbottle water beside a lake.
    //
    // The tell is that the two halves disagree about the same fact:
    // [`ProductIndex::ground_supplies`] knows the ground yields water (it
    // reads `LuaTilePrototype::fluid`), while `Gather`'s test is
    // `has_resource_patches`, and water is not a resource entity. Until
    // `Gather` can stand an offshore pump on tile-yielded water, the honest
    // answer for water is the refusal it always gave -- `NoFluidSource` --
    // and **not** a plant. This makes the ground the authority for both
    // branches, so the day `Gather` learns about tiles, water flows through
    // the gathering branch with nothing here changed.
    if index.ground_supplies(fluid) {
        return false;
    }
    let Ok(recipe) =
        index.recipe_producing(fluid, &Categories::planner_runs(&machines), None, &machines)
    else {
        return false;
    };
    recipe.name != consumer.name && recipe_gate(state, recipe) != RecipeGate::Unobtainable
}

/// The fluids this goal's recipe wants that nothing standing supplies, each
/// with the way this planner would obtain it.
///
/// In recipe order, which is the game's own order, so the emitted subgoals
/// are deterministic.
fn fluid_shortfalls(goal: &Goal, state: &PlanState) -> Vec<FluidPlan> {
    let Some(recipe) = recipe_fabricate_would_run(goal, state) else {
        return Vec::new();
    };
    let Some(ingredients) = recipe.ingredients.as_ref() else {
        return Vec::new();
    };
    // `sources_of` uses `from` only to *rank* what it found, and this asks
    // whether it found anything at all, which no ordering changes. The
    // origin is therefore a constant rather than a bot's position -- which
    // `applicable` does not have, and which would make applicability depend
    // on who happens to be asking.
    let origin = Position::default();
    let mut missing = Vec::new();
    for ingredient in ingredients {
        if ingredient.ingredient_type != FLUID_TYPE {
            continue;
        }
        let fluid = ingredient.name.as_str();
        if missing.iter().any(|p: &FluidPlan| p.fluid() == fluid) {
            continue;
        }
        // Already supplied: `Fabricate` will adopt it, and standing a second
        // wellhead beside the first would be work nobody asked for.
        if !pipe::sources_of(state, fluid, &origin).0.is_empty() {
            continue;
        }
        // `Gather::applicable`'s own two halves first: the ground is the
        // cheaper source and the one the owner's topology prefers, so a fluid
        // this map yields is never made in a plant instead.
        if state.has_resource_patches(fluid) && extract::extractor_for(state, fluid).is_ok() {
            missing.push(FluidPlan::Gathered(fluid.to_string()));
        } else if makeable(state, fluid, &recipe) {
            missing.push(FluidPlan::Produced {
                fluid: fluid.to_string(),
                // **The consuming recipe's own per-craft amount, not the
                // goal's count.** Owner ruling: a fluid ingredient is
                // satisfied by CONNECTIVITY, not by a quantity -- there is
                // nothing to carry and nothing to count. This is the number
                // `NoFluidSource` already quotes for the same reason: it says
                // which recipe was walked. Asking for more would only risk
                // `SinkTooSmall` on a buffer nobody needs to fill.
                amount: ingredient.amount,
            });
        }
    }
    missing
}

/// Derive `gathered:<fluid>` from a goal that needs the fluid.
///
/// Registered immediately **ahead of** [`crate::method::fabricate::Fabricate`]
/// in both registries, and applicable only to goals `Fabricate` itself claims
/// and would then refuse for want of a fluid source. So it cannot move a plan
/// that works today: every goal that reaches `Fabricate` and plans still
/// reaches it, because this method declines whenever a source already stands.
pub struct SupplyFluid;

impl Method for SupplyFluid {
    fn name(&self) -> &'static str {
        "supply-fluid"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        // Ask `Fabricate` first, and cheaply: this is the goal shape that can
        // refuse for a fluid at all, and its own `applicable` is the one
        // encoding of it.
        crate::method::fabricate::Fabricate.applicable(goal, state)
            && !fluid_shortfalls(goal, state).is_empty()
    }

    /// The same claim `Fabricate` makes, for the same reason: everything the
    /// wellhead needs is stated as this method's own subgoals and placed out
    /// of one hand, so items pass hand to hand without converging. `Gather`
    /// answers [`Method::hands_over`] for exactly this and lost a plan to its
    /// absence -- see its doc.
    fn hands_over(&self, goal: &Goal, state: &PlanState) -> bool {
        self.applicable(goal, state)
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let missing = fluid_shortfalls(goal, &ctx.state);
        if missing.is_empty() {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        }
        let mut steps: Vec<Step> = missing
            .into_iter()
            .map(|plan| {
                // Nothing here knows a technology any of this stands up: the
                // goal's own `unlocks` belongs to the item being made, not to
                // the fluid feeding it, and claiming one would be the grant
                // `goal.unlocks` is warned about.
                Step::Subgoal(match plan {
                    FluidPlan::Gathered(fluid) => Goal::Gathered {
                        entity: fluid,
                        unlocks: None,
                    },
                    FluidPlan::Produced { fluid, amount } => Goal::Produced {
                        item: fluid,
                        count: amount,
                        // The neutral holder the CLI's own `produced:` parse
                        // uses. Naming a bot here would put the plant in one
                        // chain's name, which is not a fact this method knows.
                        whose: Holder::Anyone,
                        unlocks: None,
                        // Deliberately unset: `Fabricate` chooses, so the two
                        // cannot disagree. See `makeable`.
                        via: None,
                    },
                })
            })
            .collect();
        // The caller's goal, verbatim. `Fabricate` claims it on the way back
        // through, now that the tank stands in the overlay.
        steps.push(Step::Subgoal(goal.clone()));
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::FluidPlan;
    use super::*;
    use crate::goal::Holder;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::serde_json;
    use factorio_bot_core::types::{FactorioEntityPrototype, FactorioRecipe};
    use std::sync::Arc;

    const BOTS: [BotId; 1] = [BotId(1)];

    /// The fixture, plus the two prototypes that make `basic-oil-processing`
    /// a job this planner can name a machine for.
    ///
    /// `wells` is the whole variable: with them the map can be made to supply
    /// crude, without them it cannot, and nothing else about the world
    /// changes between the two cases.
    fn refinery_state(wells: bool) -> PlanState {
        PlanState::from_world(Arc::new(refinery_world(wells)), &BOTS)
    }

    /// The refinery half of the fixture, before a [`PlanState`] closes over
    /// it, so the chemistry tests can add a second machine to the *same*
    /// world rather than build a second one that has to agree with it.
    fn refinery_world(wells: bool) -> factorio_bot_core::factorio::world::FactorioSurface {
        let world = world_with_oil(OilFixture {
            wells,
            categories: true,
            pumpjack: PumpjackRecipe::LockedBy { researched: true },
            prerequisite: false,
        });
        let refinery: FactorioEntityPrototype = serde_json::from_str(
            r#"{
              "name": "oil-refinery", "entity_type": "assembling-machine",
              "collision_mask": [], "mine_result": null,
              "collision_box": { "left_top": { "x": -2.4, "y": -2.4 },
                                 "right_bottom": { "x": 2.4, "y": 2.4 } },
              "crafting_categories": ["oil-processing"]
            }"#,
        )
        .expect("the oil-refinery prototype parses");
        world
            .update_entity_prototypes(vec![refinery])
            .expect("update_entity_prototypes cannot fail for a well-formed prototype");
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "basic-oil-processing", "valid": true, "enabled": true,
              "category": "oil-processing",
              "ingredients": [
                { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "petroleum-gas", "product_type": "fluid", "amount": 45,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 5.0, "order": "a", "group": "fluids",
              "subgroup": "fluid-recipes"
            }"#,
        )
        .expect("the basic-oil-processing recipe parses");
        world
            .update_recipes(vec![recipe])
            .expect("update_recipes cannot fail for a well-formed recipe");
        world
    }

    /// The refinery world plus a `chemical-plant` and `plastic-bar`, so the
    /// consuming goal wants a fluid **no ground yields** and one recipe makes.
    ///
    /// This is the done-test in miniature: `plastic-bar` wants petroleum-gas,
    /// which has no charted patch and no extractor, so before the chemistry
    /// rung it declined here and refused as `NoFluidSource`.
    fn plastic_state(wells: bool) -> PlanState {
        let world = refinery_world(wells);
        let plant: FactorioEntityPrototype = serde_json::from_str(
            r#"{
              "name": "chemical-plant", "entity_type": "assembling-machine",
              "collision_mask": [], "mine_result": null,
              "collision_box": { "left_top": { "x": -1.4, "y": -1.4 },
                                 "right_bottom": { "x": 1.4, "y": 1.4 } },
              "crafting_categories": ["chemistry"]
            }"#,
        )
        .expect("the chemical-plant prototype parses");
        world
            .update_entity_prototypes(vec![plant])
            .expect("update_entity_prototypes cannot fail for a well-formed prototype");
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "plastic-bar", "valid": true, "enabled": true,
              "category": "chemistry",
              "ingredients": [
                { "name": "coal", "ingredient_type": "item", "amount": 1 },
                { "name": "petroleum-gas", "ingredient_type": "fluid", "amount": 20 }
              ],
              "products": [
                { "name": "plastic-bar", "product_type": "item", "amount": 2,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a", "group": "intermediate",
              "subgroup": "raw-material"
            }"#,
        )
        .expect("the plastic-bar recipe parses");
        world
            .update_recipes(vec![recipe])
            .expect("update_recipes cannot fail for a well-formed recipe");
        PlanState::from_world(Arc::new(world), &BOTS)
    }

    fn plastic() -> Goal {
        Goal::Have {
            item: "plastic-bar".into(),
            count: 10,
            whose: Holder::Anyone,
            via: None,
        }
    }

    fn petroleum() -> Goal {
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 45,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("basic-oil-processing".into()),
        }
    }

    /// **The edge.** A refinery goal on a map with charted crude and nothing
    /// standing derives `gathered:crude-oil` -- the goal the owner had to type
    /// by hand.
    #[test]
    fn a_refinery_goal_with_no_standing_source_derives_gathered_crude() {
        let state = refinery_state(true);
        assert_eq!(
            fluid_shortfalls(&petroleum(), &state),
            vec![FluidPlan::Gathered("crude-oil".to_string())],
            "crude is charted here and nothing standing supplies it"
        );
        assert!(
            SupplyFluid.applicable(&petroleum(), &state),
            "so this method claims the goal, ahead of `Fabricate`"
        );
        let mut ctx = ExpansionCtx::new(state, BOTS[0]);
        let steps = SupplyFluid
            .expand(&petroleum(), &mut ctx)
            .expect("the derivation emits");
        let goals: Vec<String> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(g) => Some(g.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(goals.len(), 2, "the wellhead, then the caller's own goal");
        assert!(
            goals[0].contains("crude-oil"),
            "the wellhead comes first, so the tank stands before the refinery is sited: {goals:?}"
        );
        assert_eq!(
            &goals[1],
            &petroleum().to_string(),
            "and the caller's goal is re-issued verbatim, for `Fabricate` to claim"
        );
    }

    /// **The control, and it is the one that keeps this method narrow.** The
    /// same goal on the same world with no charted crude derives nothing:
    /// there is no ground to stand a pumpjack on **and no recipe in this
    /// fixture makes crude either**, so `Fabricate`'s `NoFluidSource` is
    /// still the right answer and still the one a caller gets. Without this,
    /// "claims the goal" above would be consistent with claiming every fluid
    /// goal on every map.
    #[test]
    fn no_charted_crude_derives_nothing_and_leaves_the_refusal_alone() {
        let state = refinery_state(false);
        assert!(
            fluid_shortfalls(&petroleum(), &state).is_empty(),
            "nothing charted can be gathered, and nothing here makes crude"
        );
        assert!(!SupplyFluid.applicable(&petroleum(), &state));
    }

    /// **A recipe may not be asked to feed itself.**
    ///
    /// Found by a mutation, not by reading: deleting `recipe.name !=
    /// consumer.name` from [`makeable`] left the whole suite green, which is a
    /// finding about the tests rather than about the guard.
    ///
    /// `coal-liquefaction` is the real shape -- it takes heavy-oil in and puts
    /// heavy-oil out -- so a goal for heavy oil that `Fabricate` would run
    /// through it wants heavy oil in order to make heavy oil. Without the
    /// guard this rung emits `produced:heavy-oil` for a goal that *is*
    /// `produced:heavy-oil`, and only `MAX_EXPANSION_DEPTH` stops it.
    #[test]
    fn a_recipe_is_not_asked_to_feed_itself() {
        let world = refinery_world(true);
        let looped: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "coal-liquefaction", "valid": true, "enabled": true,
              "category": "oil-processing",
              "ingredients": [
                { "name": "heavy-oil", "ingredient_type": "fluid", "amount": 25 }
              ],
              "products": [
                { "name": "heavy-oil", "product_type": "fluid", "amount": 90,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 5.0, "order": "a", "group": "fluids",
              "subgroup": "fluid-recipes"
            }"#,
        )
        .expect("the coal-liquefaction recipe parses");
        world
            .update_recipes(vec![looped])
            .expect("update_recipes cannot fail for a well-formed recipe");
        let state = PlanState::from_world(Arc::new(world), &BOTS);
        let goal = Goal::Produced {
            item: "heavy-oil".into(),
            count: 90,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("coal-liquefaction".into()),
        };
        assert!(
            fluid_shortfalls(&goal, &state).is_empty(),
            "the only recipe producing heavy-oil is the one being run, so \
             deriving a goal for it would re-issue this very goal"
        );
    }

    /// **The guard that stops a fluid the ground gives being bottled.**
    ///
    /// This is the first thing the chemistry rung got wrong, live: on the
    /// seed-31337 water-and-oil dump `have:sulfur:10` derived a *barrelling
    /// plant for water*, because water has no charted resource patch (so the
    /// gathering branch declines) and the only recipe producing it is
    /// `empty-water-barrel`. The fixture reproduces exactly that shape -- a
    /// lake, no water patch, and an unbottling recipe -- and asserts the
    /// petroleum is derived while the water is not.
    ///
    /// The remaining refusal for water is `Fabricate`'s `NoFluidSource`,
    /// which is the answer it always gave and is honest: standing an offshore
    /// pump on tile-yielded water is `method::gather`'s gap, not this one's.
    #[test]
    fn a_fluid_the_ground_yields_is_never_bottled() {
        let world = refinery_world(true);
        let mut tiles = Vec::new();
        factorio_bot_core::test_utils::spawn_water(
            &mut tiles,
            factorio_bot_core::types::Rect::new(
                &factorio_bot_core::types::Position::new(6.0, 6.0),
                &factorio_bot_core::types::Position::new(10.0, 10.0),
            ),
        );
        world
            .update_chunk_tiles(tiles)
            .expect("a lake is well-formed");
        let plant: FactorioEntityPrototype = serde_json::from_str(
            r#"{
              "name": "chemical-plant", "entity_type": "assembling-machine",
              "collision_mask": [], "mine_result": null,
              "collision_box": { "left_top": { "x": -1.4, "y": -1.4 },
                                 "right_bottom": { "x": 1.4, "y": 1.4 } },
              "crafting_categories": ["chemistry", "crafting"]
            }"#,
        )
        .expect("the chemical-plant prototype parses");
        world
            .update_entity_prototypes(vec![plant])
            .expect("update_entity_prototypes cannot fail for a well-formed prototype");
        let sulfur: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "sulfur", "valid": true, "enabled": true,
              "category": "chemistry",
              "ingredients": [
                { "name": "water", "ingredient_type": "fluid", "amount": 30 },
                { "name": "petroleum-gas", "ingredient_type": "fluid", "amount": 30 }
              ],
              "products": [
                { "name": "sulfur", "product_type": "item", "amount": 2,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a", "group": "g",
              "subgroup": "s"
            }"#,
        )
        .expect("the sulfur recipe parses");
        let unbottle: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "empty-water-barrel", "valid": true, "enabled": true,
              "category": "crafting",
              "ingredients": [
                { "name": "water-barrel", "ingredient_type": "item", "amount": 1 }
              ],
              "products": [
                { "name": "barrel", "product_type": "item", "amount": 1,
                  "probability": 1.0 },
                { "name": "water", "product_type": "fluid", "amount": 50,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a", "group": "g",
              "subgroup": "s"
            }"#,
        )
        .expect("the empty-water-barrel recipe parses");
        world
            .update_recipes(vec![sulfur, unbottle])
            .expect("update_recipes cannot fail for well-formed recipes");
        let state = PlanState::from_world(Arc::new(world), &BOTS);
        let goal = Goal::Have {
            item: "sulfur".into(),
            count: 10,
            whose: Holder::Anyone,
            via: None,
        };
        let derived = fluid_shortfalls(&goal, &state);
        assert!(
            derived.iter().all(|plan| plan.fluid() != "water"),
            "the lake supplies water, so nothing here may bottle it: {derived:?}"
        );
        assert!(
            derived.contains(&FluidPlan::Produced {
                fluid: "petroleum-gas".to_string(),
                amount: 30,
            }),
            "and the fluid the ground does NOT give is still derived: {derived:?}"
        );
    }

    /// **The chemistry rung's own edge.** `have:plastic-bar:10` wants
    /// petroleum-gas, which no ground yields, and one recipe makes. It
    /// derives `produced:petroleum-gas:20` -- the consuming recipe's own
    /// per-craft amount -- and re-issues the caller's goal.
    #[test]
    fn a_fluid_no_ground_yields_is_derived_as_produced() {
        let state = plastic_state(true);
        assert_eq!(
            fluid_shortfalls(&plastic(), &state),
            vec![FluidPlan::Produced {
                fluid: "petroleum-gas".to_string(),
                amount: 20,
            }],
            "petroleum has no patch and no extractor, and `basic-oil-processing` makes it"
        );
        assert!(SupplyFluid.applicable(&plastic(), &state));
        let mut ctx = ExpansionCtx::new(state, BOTS[0]);
        let steps = SupplyFluid
            .expand(&plastic(), &mut ctx)
            .expect("the derivation emits");
        let goals: Vec<String> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(g) => Some(g.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(goals.len(), 2, "the plant, then the caller's own goal");
        assert!(
            goals[0].contains("petroleum-gas"),
            "the plant comes first, so it stands before the chemical plant is sited: {goals:?}"
        );
        assert_eq!(
            &goals[1],
            &plastic().to_string(),
            "and the caller's goal is re-issued verbatim, for `Fabricate` to claim"
        );
    }

    /// **The control that separates the two rungs.** The `Produced` derivation
    /// must not fire for a fluid the ground yields: crude is charted here, so
    /// the plastic goal's *own* fluid is still `Produced` while the refinery
    /// goal underneath it stays `Gathered`. Without this, "derives Produced"
    /// above would be consistent with never gathering anything again -- the
    /// owner's topology puts the wellhead on the ground, not in a plant.
    #[test]
    fn the_ground_still_wins_where_the_ground_can_answer() {
        let state = plastic_state(true);
        assert_eq!(
            fluid_shortfalls(&petroleum(), &state),
            vec![FluidPlan::Gathered("crude-oil".to_string())],
            "crude is charted, so it is gathered and not made"
        );
    }

    /// **The second control: no source, either way, derives nothing.** With
    /// the wells gone, `basic-oil-processing` is still a recipe that makes
    /// petroleum, so `plastic-bar` still derives `produced:petroleum-gas` --
    /// and *that* goal then finds neither ground nor recipe for crude and
    /// refuses in `Fabricate` exactly as it does today. The derivation stops
    /// where the world stops, one rung down rather than at the top.
    #[test]
    fn the_derivation_stops_where_the_world_stops() {
        let state = plastic_state(false);
        assert_eq!(
            fluid_shortfalls(&plastic(), &state),
            vec![FluidPlan::Produced {
                fluid: "petroleum-gas".to_string(),
                amount: 20,
            }],
            "a recipe makes petroleum whether or not there are wells"
        );
        assert!(
            fluid_shortfalls(&petroleum(), &state).is_empty(),
            "but crude has no source at all here, so nothing is derived for it"
        );
    }
}
