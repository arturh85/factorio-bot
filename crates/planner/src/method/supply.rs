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
//! [`pipe::sources_of`] cannot attribute to anything standing, and which
//! [`gather::Gather`] could stand a source up for, this emits:
//!
//! 1. `Goal::Gathered { entity: <fluid> }`, one per such fluid, and
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
//! # What it deliberately does not do
//!
//! * **It does not build a source for a fluid the map cannot yield.** The
//!   test is `Gather`'s own -- charted patches plus a nameable extractor --
//!   asked before anything is claimed, so a recipe wanting `sulfuric-acid`
//!   (no patch, no extractor) is declined here and reaches `Fabricate`'s
//!   `NoFluidSource` exactly as it does today. Deriving *that* is the
//!   chemistry rung: it wants a `Producing`/`Have` on the fluid rather than a
//!   `Gathered`, and it is a separate decision because the sink -- who the
//!   plant pipes *to* -- is not settled (see `pipe::SUPPLYING`'s note on the
//!   absent `ACCEPTING`).
//! * **It does not promise the tank has anything in it.** Nothing in
//!   `PlanState` models fluid contents (`pipe::sources_of`, "what it cannot
//!   say"), so this is a *connectivity* derivation and nothing stronger --
//!   which is the same thing the two-goal form the owner types today buys.

use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::{ExpansionCtx, Method, Step};
use crate::method::{extract, pipe};
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

/// The fluids this goal's recipe wants that nothing standing supplies and
/// that this map could be made to supply by standing an extractor up.
///
/// In recipe order, which is the game's own order, so the emitted subgoals
/// are deterministic.
fn gatherable_shortfalls(goal: &Goal, state: &PlanState) -> Vec<String> {
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
        if missing.iter().any(|f| f == fluid) {
            continue;
        }
        // Already supplied: `Fabricate` will adopt it, and standing a second
        // wellhead beside the first would be work nobody asked for.
        if !pipe::sources_of(state, fluid, &origin).0.is_empty() {
            continue;
        }
        // `Gather::applicable`'s own two halves, asked before claiming
        // anything: a fluid with no charted patch and no nameable extractor
        // is not one this rung can stand a source up for.
        if state.has_resource_patches(fluid) && extract::extractor_for(state, fluid).is_ok() {
            missing.push(fluid.to_string());
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
            && !gatherable_shortfalls(goal, state).is_empty()
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
        let missing = gatherable_shortfalls(goal, &ctx.state);
        if missing.is_empty() {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        }
        let mut steps: Vec<Step> = missing
            .into_iter()
            .map(|fluid| {
                Step::Subgoal(Goal::Gathered {
                    entity: fluid,
                    // Nothing here knows a technology this stands up: the
                    // goal's own `unlocks` belongs to the item being made,
                    // not to the fluid feeding it, and claiming one would be
                    // the grant `goal.unlocks` is warned about.
                    unlocks: None,
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
        PlanState::from_world(Arc::new(world), &BOTS)
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
            gatherable_shortfalls(&petroleum(), &state),
            vec!["crude-oil".to_string()],
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
    /// there is no ground to stand a pumpjack on, so `Fabricate`'s
    /// `NoFluidSource` is still the right answer and still the one a caller
    /// gets. Without this, "claims the goal" above would be consistent with
    /// claiming every fluid goal on every map.
    #[test]
    fn no_charted_crude_derives_nothing_and_leaves_the_refusal_alone() {
        let state = refinery_state(false);
        assert!(
            gatherable_shortfalls(&petroleum(), &state).is_empty(),
            "nothing charted can be gathered"
        );
        assert!(!SupplyFluid.applicable(&petroleum(), &state));
    }
}
