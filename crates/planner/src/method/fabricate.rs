//! Stand a crafting machine up and run one recipe in it, with the machine
//! chosen by **what it crafts**.
//!
//! # The rung this is
//!
//! `Craft` runs the `crafting` category in a character's hands and `Smelt`
//! runs `smelting` in a `stone-furnace`. Those were not a scope decision:
//! they were the only two categories whose machine the planner could *name*,
//! because nothing on our wire said what a machine crafts and
//! `oil-refinery`, `chemical-plant`, `centrifuge` and all three assembling
//! machines share one `entity_type`. See
//! `docs/superpowers/notes/2026-09-07-a-recipe-the-planner-cannot-run.md`.
//!
//! [`crate::method::machine::MachineTable`] closed that, so this method is
//! the third: it names its machine the way [`crate::method::machine`] does —
//! one encoding, shared with [`crate::products::Categories::planner_runs`],
//! rather than a second table agreeing with the first until it does not.
//!
//! # It refuses before it emits, and that is most of what it does today
//!
//! The same promise `method::connect` makes: every refusal is returned before
//! a step is emitted or an entity lands in the plan overlay. A half-built
//! machine with its bill half-spent is worse than none.
//!
//! And the refusals are where the frontier actually is. Opening the category
//! gate does **not** make `produced:petroleum-gas` plan, and was never going
//! to: `basic-oil-processing` is 100 crude-oil in and 45 petroleum-gas out,
//! both fluids. What this method does is move the refusal from *"none is in a
//! category this planner runs"* — which sends a reader looking for a missing
//! machine — to the thing that is actually missing, named:
//!
//! * [`FabricateRefusal::FluidIngredient`] — the machine is named, the bill
//!   was walked, and one of its ingredients is a fluid. This is
//!   [`crate::substance::split_bill`]'s first production caller; it existed
//!   with unit tests and no caller until a category-aware method could give
//!   it one.
//! * [`FabricateRefusal::FluidProduct`] — the machine is named and would run,
//!   and the thing it makes has nowhere to land. A fluid product needs a
//!   fluidbox concept or a `Goal::Stored`, which is an **open owner
//!   decision** (`docs/superpowers/notes/2026-09-07-decisions-waiting-for-the-owner.md`
//!   §3) and deliberately not invented here.
//!
//! Without these, opening the gate makes the *message worse*, which the
//! predecessor measured: `products::NoProducer` declines the moment one
//! runnable recipe exists, and the driver falls through to `no method can
//! satisfy goal`. That degradation is the reason this method claims the goal
//! rather than being a refusal-only observer.
//!
//! # Registration
//!
//! **Second to last, immediately ahead of [`crate::products::NoProducer`].**
//! Every method above it is asked first, so a goal that plans today reaches
//! an applicable method before this one is consulted and **no existing plan
//! can move**. It can only claim goals that previously refused.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ItemId, Ticks};
use crate::method::have::{Demand, demand};
use crate::method::machine::{Machine, MachineTable};
use crate::method::util::{
    CRAFTING_CATEGORY, RecipeGate, SMELTING_CATEGORY, free_area_near, output_per_craft, recipe_for,
    recipe_gate, smelting_ticks,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::products::{Categories, ProductIndex};
use crate::state::PlanState;
use crate::substance::{FluidSource, split_bill};
use factorio_bot_core::types::{FactorioEntity, FactorioRecipe};
use miette::Diagnostic;
use thiserror::Error;

/// Ticks a bot spends walking up to a machine and placing it.
///
/// The same figure `method::have` charges for a stone furnace; a machine is a
/// machine as far as the placement verb is concerned.
const PLACE_TICKS: Ticks = 30;

/// Ticks one hand-to-machine transfer costs, as everywhere else in the crate.
const TRANSFER_TICKS: Ticks = 10;

/// Ticks spent telling a machine which recipe to run.
const SET_RECIPE_TICKS: Ticks = 10;

/// Why a named machine still cannot run a recipe.
///
/// **Both variants are returned before anything is emitted.** They are walls,
/// not shortfalls: no amount of mining, research or walking moves either.
#[derive(Clone, Debug, PartialEq, Eq, Error, Diagnostic)]
pub enum FabricateRefusal {
    /// The bill contains something no character can carry to the machine.
    ///
    /// The machine is named because that is the new information: before
    /// `crafting_categories` crossed the bridge, a reader was told only that
    /// the category was unrunnable and could not tell a missing *machine*
    /// from a missing *fluid*.
    #[error(
        "{recipe} runs in {machine} (category {category}), and this planner can name that \
         machine now -- but the recipe wants {amount} {fluid}, which is a fluid. No character \
         inventory holds a fluid, no `InventorySlot` addresses a fluidbox, and no action in this \
         planner moves one, so the {amount} {fluid} cannot be delivered to the {machine}. \
         {produced_by}"
    )]
    #[diagnostic(
        code(planner::fluid_ingredient),
        help(
            "the machine is no longer the blocker; a fluid ingredient needs pipes from a source \
             to the machine's fluidbox, which this planner does not model. `gathered:<fluid>` \
             stands a pumpjack and a tank up on a field, which is as close as it gets today"
        )
    )]
    FluidIngredient {
        recipe: String,
        category: String,
        machine: String,
        fluid: String,
        amount: u32,
        produced_by: FluidSource,
    },

    /// The machine would run and the thing it makes has nowhere to go.
    #[error(
        "{recipe} runs in {machine} (category {category}) and produces {}, which {} -- and this \
         planner has no goal that names a fluidbox, so there is nowhere for it to land",
        fluids.join(", "),
        if fluids.len() == 1 { "is a fluid" } else { "are fluids" }
    )]
    #[diagnostic(
        code(planner::fluid_product),
        help(
            "landing a fluid product needs a `Goal::Stored`-shaped goal, which is an open owner \
             decision -- see docs/superpowers/notes/2026-09-07-decisions-waiting-for-the-owner.md"
        )
    )]
    FluidProduct {
        recipe: String,
        category: String,
        machine: String,
        /// In recipe order, which is the game's own order.
        fluids: Vec<String>,
    },
}

/// What this method would do for a goal, or nothing.
///
/// Shared by [`Method::applicable`] and [`Fabricate::expand`] so the two
/// cannot disagree about which recipe and which machine — the failure mode
/// where a method claims a goal and then refuses it for a different reason
/// than the one it was selected on.
struct Job {
    recipe: FactorioRecipe,
    machine: String,
}

/// The cheap half of the test, run before a [`ProductIndex`] is built.
///
/// Building an index clones 662 recipes on a real capture, and
/// [`Method::applicable`] is asked for every goal no earlier method claimed.
/// A goal whose product has a same-named recipe in `crafting` or `smelting`
/// is the overwhelming majority and is settled here without touching one.
fn cannot_possibly_apply(state: &PlanState, item: &str) -> bool {
    recipe_for(state, item)
        .is_some_and(|r| r.category == CRAFTING_CATEGORY || r.category == SMELTING_CATEGORY)
}

fn job_for(goal: &Goal, state: &PlanState) -> Option<Job> {
    let Demand {
        item, need, via, ..
    } = demand(goal, state)?;
    if need == 0 {
        return None;
    }
    // Skipped when the caller named a recipe: the shortcut asks whether the
    // *product* has a same-named `crafting`/`smelting` recipe, which answers
    // a different question than "is the recipe you asked for one of mine".
    // A goal naming a non-crafting recipe for a product that also has a
    // crafting one -- the modded case -- would otherwise be declined here and
    // then claimed by `HandCraft`, silently running the other recipe.
    if via.is_none() && cannot_possibly_apply(state, item) {
        return None;
    }
    let machines = MachineTable::from_state(state);
    let index = ProductIndex::from_state(state);
    // `via` is `None` for every goal written before 2026-09-07, and this call
    // then *is* `sole_recipe_producing` -- see `recipe_producing`'s doc.
    let recipe = index
        .recipe_producing(item, &Categories::planner_runs(&machines), via, &machines)
        .ok()?;
    // `crafting` and `smelting` belong to `HandCraft` and `Smelt`, which are
    // registered ahead of this and know far more about them -- banks,
    // adoption, shared ore. Declining them here is what keeps this method
    // incapable of moving an existing plan.
    if recipe.category == CRAFTING_CATEGORY || recipe.category == SMELTING_CATEGORY {
        return None;
    }
    let Machine::Entity(machine) = machines.machine_for(&recipe.category).ok()? else {
        return None;
    };
    if recipe_gate(state, recipe) == RecipeGate::Unobtainable {
        return None;
    }
    Some(Job {
        recipe: recipe.clone(),
        machine,
    })
}

/// Run one recipe in the machine that runs its category.
pub struct Fabricate;

impl Method for Fabricate {
    fn name(&self) -> &'static str {
        "fabricate"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        job_for(goal, state).is_some()
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            whose,
            unlocks,
            ..
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let Some(Job { recipe, machine }) = job_for(goal, &ctx.state) else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };

        // ---- refuse first, emit nothing until every wall is behind us ----

        // The expansion's own cached table -- built once per expansion, the
        // rule `SubstanceTable`'s doc states.
        let bill = split_bill(ctx.substances(), &recipe);
        if let Some((fluid, amount)) = bill.fluids.first() {
            let recipes: Vec<FactorioRecipe> = ctx
                .state
                .base()
                .globals
                .recipes
                .iter()
                .map(|entry| entry.value().clone())
                .collect();
            return Err(PlannerError::CannotFabricate(Box::new(
                FabricateRefusal::FluidIngredient {
                    recipe: recipe.name.clone(),
                    category: recipe.category.clone(),
                    machine,
                    fluid: fluid.clone(),
                    amount: *amount,
                    produced_by: FluidSource::of(recipes.iter(), fluid),
                },
            )));
        }
        // Asked of every product, not only the one the goal named: a recipe
        // yielding an item *and* a fluid still strands the fluid in the
        // machine, and the next run then jams on a full output. Nothing in
        // vanilla's runnable categories does this today; saying so costs one
        // branch and the alternative is a silent half-answer.
        let fluid_products: Vec<String> = recipe
            .products
            .iter()
            .filter(|p| ctx.substances().is_fluid(&p.name))
            .map(|p| p.name.clone())
            .collect();
        if !fluid_products.is_empty() {
            return Err(PlannerError::CannotFabricate(Box::new(
                FabricateRefusal::FluidProduct {
                    recipe: recipe.name.clone(),
                    category: recipe.category.clone(),
                    machine,
                    fluids: fluid_products,
                },
            )));
        }

        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let Some(site) = free_area_near(&ctx.state, &from, &machine) else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };

        // ---- nothing below refuses; from here it is all emission ----

        let per_craft = output_per_craft(&recipe, item);
        let runs = need.div_ceil(per_craft);
        // The machine's own speed against the recipe's own energy. Named
        // `smelting_ticks` for its first caller and generic in what it does:
        // it reads `crafting_speed` for whichever entity is acting.
        let per_run = smelting_ticks(&ctx.state, &recipe, &machine);

        let build = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.build_distance)
            .unwrap_or(10.0);
        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.reach_distance)
            .unwrap_or(10.0);
        let min_radius = ctx.state.placement_clearance(&machine).unwrap_or(0.0);

        let mut steps: Vec<Step> = Vec::new();
        let mut research_pre: Vec<Condition> = Vec::new();
        match recipe_gate(&ctx.state, &recipe) {
            RecipeGate::NeedsResearch(tech) => {
                steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
                research_pre.push(Condition::Researched(tech));
            }
            RecipeGate::PlannedResearch(tech) => research_pre.push(Condition::Researched(tech)),
            RecipeGate::Open | RecipeGate::Unobtainable => {}
        }

        // The machine itself, as a subgoal: obtaining it is somebody else's
        // problem, and `have:oil-refinery:1` already plans.
        steps.push(Step::Subgoal(Goal::Have {
            item: machine.clone(),
            count: 1,
            whose: whose.clone(),
            via: None,
        }));
        for (ingredient, amount) in &bill.items {
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient.clone(),
                count: amount.saturating_mul(runs),
                whose: whose.clone(),
                via: None,
            }));
        }

        let entity = FactorioEntity {
            name: machine.clone(),
            entity_type: ctx
                .state
                .base()
                .globals
                .entity_prototypes
                .get(machine.as_str())
                .map(|p| p.entity_type.clone())
                .unwrap_or_default(),
            position: site.clone(),
            ..Default::default()
        };
        let place_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(entity.clone()),
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: site.clone(),
                    radius: build,
                    min_radius,
                },
                Condition::AreaFree {
                    pos: site.clone(),
                    entity: machine.clone(),
                    direction: 0,
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: machine.clone(),
                    count: 1,
                },
            ],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: machine.clone(),
                    count: 1,
                },
                Effect::CreateEntity(Box::new(entity)),
            ],
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place {machine} at {site}"),
        })));
        ctx.state.create_entity(FactorioEntity {
            name: machine.clone(),
            position: site.clone(),
            ..Default::default()
        });

        // A furnace picks its recipe from what it is fed; every other crafting
        // machine has to be told. Emitted unconditionally rather than gated on
        // `entity_type`, because a machine reached through this method is by
        // construction not the furnace `Smelt` owns.
        let recipe_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: recipe_id,
            kind: ActionKind::SetRecipe {
                pos: site.clone(),
                entity: machine.clone(),
                recipe: recipe.name.clone(),
            },
            pre: {
                let mut pre = vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: site.clone(),
                        radius: reach,
                        min_radius: 0.0,
                    },
                    Condition::EntityAt {
                        pos: site.clone(),
                        name: machine.clone(),
                    },
                ];
                pre.extend(research_pre.iter().cloned());
                pre
            },
            eff: vec![Effect::SetRecipe {
                pos: site.clone(),
                recipe: recipe.name.clone(),
            }],
            duration: SET_RECIPE_TICKS,
            pinned: None,
            label: format!("set {machine} to {}", recipe.name),
        })));
        ctx.state.set_recipe(&site, &recipe.name)?;

        let mut insert_ids = Vec::new();
        for (ingredient, amount) in &bill.items {
            let total = amount.saturating_mul(runs);
            let id = ctx.ids.next();
            insert_ids.push(id);
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: site.clone(),
                    entity: machine.clone(),
                    slot: InventorySlot::AssemblerInput,
                    item: ingredient.clone(),
                    count: total,
                },
                pre: vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: site.clone(),
                        radius: reach,
                        min_radius: 0.0,
                    },
                    Condition::EntityAt {
                        pos: site.clone(),
                        name: machine.clone(),
                    },
                    Condition::HasItem {
                        who: Actor::Role,
                        item: ingredient.clone(),
                        count: total,
                    },
                ],
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: ingredient.clone(),
                    count: total,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("load the {machine} with {total} {ingredient}"),
            })));
        }

        let take = runs.saturating_mul(per_craft).min(need);
        let take_id = ctx.ids.next();
        let mut take_eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: ItemId::from(item.clone()),
            count: take,
        }];
        // The technology this production triggers, if any -- it has to land on
        // whichever action produces the item, and here that is the take. The
        // same contract `Goal::Produced`'s doc states.
        if let Some(tech) = unlocks {
            take_eff.push(Effect::Researched(tech.to_string()));
        }
        steps.push(Step::Act(Box::new(Action {
            id: take_id,
            kind: ActionKind::Remove {
                pos: site.clone(),
                entity: machine.clone(),
                slot: InventorySlot::AssemblerOutput,
                item: ItemId::from(item.clone()),
                count: take,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: site.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: site.clone(),
                    name: machine.clone(),
                },
            ],
            eff: take_eff,
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {take} {item} from the {machine} at {site}"),
        })));

        // The machine's own time, charged as a lag on each insert -- the
        // executor's rule for a take is `max over preds (finish(pred) + lag)`,
        // so charging the whole run on every insert is exact whichever lands
        // last, and a max is not a sum.
        let lag = per_run.saturating_mul(runs);
        for id in insert_ids {
            steps.push(Step::Link {
                from: id,
                to: take_id,
                lag,
            });
        }
        steps.push(Step::Link {
            from: place_id,
            to: recipe_id,
            lag: 0,
        });

        // The holder the goal named keeps the product; nothing here shares.
        debug_assert!(matches!(
            whose,
            Holder::Anyone | Holder::Bot(_) | Holder::Share(_)
        ));
        Ok(steps)
    }
}
