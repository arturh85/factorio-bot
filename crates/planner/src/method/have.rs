//! Methods that satisfy `Goal::Have`.
//!
//! Every method here emits actions with `Actor::Role` and `pinned: None`. The
//! scheduler decides who runs each one — see the plan's note on why nothing is
//! pinned.
//!
//! A chain stays with one bot because the driver stamps a whole subtree with
//! one `ChainId`, and the scheduler assigns chains rather than actions. Three
//! things open a chain, and only these three: a caller naming a bot
//! (`Holder::Bot`), which additionally records that bot as the chain's owner;
//! a `Holder::Share`, which states that the holding ends up in one inventory
//! and leaves who runs it to the scheduler; and a method whose decomposition
//! makes several *produced* items meet in one inventory (`Method::converges`).
//!
//! `HasItem` preconditions alone are not enough, for two different reasons.
//! They keep a *linear* chain together, since only the bot holding the items
//! can run the next step — but a recipe with two ingredients that each need
//! producing is a chain with two roots, and neither root has a `HasItem` to
//! hold it near the other. Red science is exactly that shape, and
//! `Method::converges` is the answer to it. And even a linear chain is only
//! held together while *one* bot holds the items: with several smelts in
//! flight over four bots, several bots hold ore, the scheduler offers the
//! insert to whichever is cheapest rather than to the one that mined for it,
//! and the pools fragment until no bot holds a whole insert's worth. A smelt
//! is welded because it sits under a share, not because it converges — see
//! `smelting_never_converges`, which is still true and says why.

use crate::ItemId;
use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{BotId, Ticks};
use crate::method::util::{
    CRAFTING_CATEGORY, RecipeGate, SMELTING_CATEGORY, free_area_near, ingredients_of, mining_ticks,
    nearest_resource_tile, output_per_craft, recipe_for, recipe_gate, recipe_ticks,
    research_ingredients, research_ticks, resource_supply_at_least, resource_tiles_for,
    smelting_ticks, trigger_requirement,
};
use crate::method::{ExpansionCtx, GoalSite, Method, MethodRegistry, Step};
use crate::state::PlanState;
use factorio_bot_core::types::FactorioEntity;
use std::collections::{BTreeMap, BTreeSet};

/// Does `item` still have to be *produced*, in the sense that no single bot
/// already holds the whole `count`?
///
/// Deliberately not `shortfall(.., Holder::Anyone) > 0`, which asks whether the
/// roster holds `count` *between them*. That is the right question for a goal
/// the roster can split; it is the wrong question for an action that reads one
/// bot's inventory, and answering it there is a bug with a shape worth
/// recording: four bots each starting with one stone furnace satisfy
/// `Have { stone-furnace, 1, Anyone }` four times over, so nothing is crafted,
/// and the second `Place` a plan needs then fails on the one bot actually
/// holding it. Used by `converges`, which has no chain actor to size a
/// `Holder::Share` against and must answer the same question without one.
fn needs_producing(state: &PlanState, item: &str, count: u32) -> bool {
    !state
        .bot_ids()
        .iter()
        .any(|bot| state.available(&Holder::Bot(*bot), item) >= count)
}

/// How much of `item` still needs producing, given what is already held *and
/// not already promised elsewhere*.
///
/// `PlanState::available` rather than the raw holding, and the difference is
/// the whole of this crate's shared-intermediate bug: a recipe whose
/// ingredients both reduce to one intermediate has two sub-goals asking this
/// question about the same items, and answering it from the raw holding lets
/// the second one count what the first has already earmarked.
fn shortfall(state: &PlanState, item: &str, count: u32, whose: &Holder) -> u32 {
    count.saturating_sub(state.available(whose, item))
}

/// What a producing method has to make, for either goal kind.
///
/// [`Goal::Have`] asks for a *shortfall* against what a bot already holds.
/// [`Goal::Produced`] asks for the whole count regardless, because possession
/// is not production: a bot carrying six labs has not crafted one, and a
/// `craft-item` trigger fires on the act of producing.
///
/// One helper for both, so `applicable` and `expand` cannot answer differently
/// -- which is how a method comes to claim a goal it then refuses.
struct Demand<'a> {
    item: &'a ItemId,
    need: u32,
    whose: &'a Holder,
    /// A technology this production unlocks. Always `None` for `Have`.
    unlocks: Option<&'a str>,
}

fn demand<'a>(goal: &'a Goal, state: &PlanState) -> Option<Demand<'a>> {
    match goal {
        Goal::Have { item, count, whose } => Some(Demand {
            item,
            need: shortfall(state, item, *count, whose),
            whose,
            unlocks: None,
        }),
        Goal::Produced {
            item,
            count,
            whose,
            unlocks,
        } => Some(Demand {
            item,
            need: *count,
            whose,
            unlocks: unlocks.as_deref(),
        }),
        _ => None,
    }
}

/// Hangs a trigger's `Effect::Researched` on whichever action produces `item`.
///
/// Found by what the action *does* -- it carries `Effect::GainItem` for the
/// goal's item -- rather than by where it was written, because the three
/// producing methods express that gain three different ways: an inline `eff:`
/// on an action literal, an element of a `vec![]`, and a `push`.
///
/// The effect has to live on the producing action and nowhere else: it is what
/// `infer_edges` turns into the ordering edge that keeps anything needing the
/// technology after the production, and a method cannot attach it to a
/// subgoal's action because it never sees their ids.
fn attach_unlock(steps: &mut [Step], item: &ItemId, unlocks: Option<&str>) {
    let Some(tech) = unlocks else {
        return;
    };
    for step in steps.iter_mut() {
        if let Step::Act(action) = step
            && action
                .eff
                .iter()
                .any(|e| matches!(e, Effect::GainItem { item: got, .. } if got == item))
        {
            action.eff.push(Effect::Researched(tech.to_string()));
            return;
        }
    }
    // A method that claimed a `Produced` goal and emitted nothing producing it
    // would drop the unlock silently, and the plan would look complete while
    // the technology never arrived.
    debug_assert!(
        false,
        "no action produces {item}, so {tech} has nowhere to go"
    );
}

/// Does `goal` already hold, in this state, right now?
///
/// **The question satisfaction is, as opposed to the one it was inferred
/// from.** A caller that runs a goal to completion has to decide when it is
/// done, and until this existed the only signal available was "the planner
/// returned an empty network". Those coincide *today* — `AlreadySatisfied` is
/// registered ahead of every other method and every other method refuses a
/// goal with no shortfall — but that is an internal invariant of this
/// registry, not a fact about goals, and a caller asserting satisfaction from
/// it is asserting something it cannot check. `scripts/supervisor.lua` said so
/// in place, and reported `plan_empty` rather than `already_satisfied` because
/// it could only observe the plan. Now it can ask.
///
/// Three-valued, and the third value is the point:
///
/// * `Some(true)` / `Some(false)` — the goal names a *state*, and the state
///   either holds or does not.
/// * `None` — the goal names an **event**, or something this planner does not
///   model, so possession cannot answer it. [`Goal::Produced`] is the first
///   kind: a bot carrying six labs has not crafted one, so no inventory read
///   ever settles it. [`Goal::Producing`] is the second: no method satisfies a
///   throughput yet, so nothing here can say whether one is met.
///
/// Collapsing `None` into `false` would report unfinished work for a goal that
/// may well be done, and into `true` would be the very lie this exists to stop.
///
/// A [`Goal::All`] is the conjunction, with `false` beating `None`: one member
/// definitely unmet settles the bundle whatever the rest are.
pub fn holds(goal: &Goal, state: &PlanState) -> Option<bool> {
    match goal {
        Goal::Have { item, count, whose } => Some(shortfall(state, item, *count, whose) == 0),
        // `PlanState::is_researched` answers over two sources: the
        // technologies this plan has already scheduled research for (its own
        // overlay) and the ones the world reports as researched (reality).
        // Either alone is a wrong answer here — skipping the overlay would
        // plan the same research twice for two goals that share a
        // prerequisite, and skipping the world would re-research what the
        // force already has.
        //
        // They cannot contradict each other, which is why "which wins" has no
        // bite: research is monotone. Nothing in the game or in this planner
        // ever un-researches a technology, so the overlay can only ever add to
        // what the world reports, and the union is the whole truth. If a
        // future Factorio grew a way to lose a technology, the overlay would
        // have to learn to subtract and this comment would be wrong — that is
        // the assumption to check first.
        Goal::Researched(tech) => Some(state.is_researched(tech)),
        Goal::Produced { .. } | Goal::Producing { .. } => None,
        Goal::All(goals) => {
            let mut answer = Some(true);
            for g in goals {
                match holds(g, state) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => answer = None,
                }
            }
            answer
        }
    }
}

/// The goal is already met. Emits nothing.
///
/// Registered first everywhere, so "we already have this" is decided in
/// exactly one place — [`holds`] — rather than re-tested inside every method
/// that could otherwise have satisfied the goal.
pub struct AlreadySatisfied;

impl Method for AlreadySatisfied {
    fn name(&self) -> &'static str {
        "already-satisfied"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        // A `Goal::All` is the driver's business, not a method's: it never
        // reaches `registry.find`, returning from `expand_goal_body` above the
        // lookup. `holds` answers for one anyway, because a *caller* asking
        // "is my bundle done" deserves an answer; claiming one here would be
        // dead code that looked meaningful.
        match goal {
            Goal::All(_) => false,
            other => holds(other, state) == Some(true),
        }
    }

    fn expand(&self, _goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        Ok(vec![])
    }
}

/// How long one coal keeps a stone furnace running.
///
/// A coal carries 4 MJ and a stone furnace draws 90 kW, so one coal sustains
/// 4 MJ / 90 kW = 44.4 s of smelting, which is 2666 ticks at 60 ticks per
/// second. Rounding down over-fuels very slightly, which is the safe
/// direction: a furnace that runs out mid-batch strands the plan.
///
/// Fuel is worked out from the recipe's own smelting time — a *plates* per
/// coal figure would be recipe-blind, and applying iron plate's 3.2 s to
/// steel's 16 s under-fuels by a factor of five.
///
/// Both halves of that division are stone-furnace numbers, which is why the
/// coal bill is computed from `recipe_ticks` — the speed-1 duration — and not
/// from the speed-divided `smelting_ticks` the furnace lag uses. Energy is
/// `power x active time`, so a *correct* generalisation needs the machine's
/// own `energy_usage`, which the mod does not send: a steel furnace is speed 2
/// at the same 90 kW (so genuinely half the coal per plate) while an electric
/// furnace is speed 2 at 180 kW and burns no coal at all. Dividing the coal by
/// crafting speed alone would get the steel case right by accident and the
/// electric case wrong, so neither is attempted. Sending `energy_usage` and
/// costing fuel from energy is the follow-up.
///
/// This is still an approximation even for a stone furnace: it ignores partial
/// burns carried between smelts. Calibrating it against observed burn rates is
/// follow-up work for the execution increment.
pub const COAL_BURN_TICKS: Ticks = 2666;

/// Time to put items into or take them out of a machine.
const TRANSFER_TICKS: Ticks = 10;

/// Time to place an entity.
const PLACE_TICKS: Ticks = 30;

/// Smelt the shortfall in a stone furnace.
pub struct Smelt;

impl Method for Smelt {
    fn name(&self) -> &'static str {
        "smelt"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Some(Demand { item, need, .. }) = demand(goal, state) else {
            return false;
        };
        if need == 0 {
            return false;
        }
        let Some(recipe) = recipe_for(state, item) else {
            return false;
        };
        recipe.category == SMELTING_CATEGORY
            && recipe_gate(state, &recipe) != RecipeGate::Unobtainable
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            whose,
            unlocks,
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let recipe =
            recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;
        let per_craft = output_per_craft(&recipe, item);
        let runs = need.div_ceil(per_craft);
        // `recipe_ticks`, deliberately, where the lag below uses
        // `smelting_ticks`. Coal is a quantity of *energy*, not of elapsed
        // time: this expression is `energy per run / energy per coal`, written
        // in ticks because both halves are calibrated at the stone furnace's
        // 90 kW (see `COAL_BURN_TICKS`). Feeding it the speed-divided duration
        // would make a faster furnace look like it needed less coal *because
        // it finished sooner*, which is the wrong mechanism even where it
        // lands on a plausible number. The two must stay decoupled until the
        // machine's own `energy_usage` is available to divide by properly.
        let coal = recipe_ticks(&recipe)
            .saturating_mul(runs)
            .div_ceil(COAL_BURN_TICKS)
            .max(1);
        let ingredients = ingredients_of(&recipe);

        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        // Site the furnace by the ore rather than by the bot's start, which
        // never advances during expansion — otherwise every chain walks
        // ore-patch, origin, ore-patch.
        let anchor = ingredients
            .first()
            .and_then(|(ingredient, _)| nearest_resource_tile(&ctx.state, ingredient, &from, 1))
            .unwrap_or(from.clone());
        let furnace_entity: String = "stone-furnace".into();
        let pos = free_area_near(&ctx.state, &anchor, &furnace_entity).ok_or_else(|| {
            PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            }
        })?;
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

        let furnace = FactorioEntity {
            name: furnace_entity.clone(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };

        let mut steps: Vec<Step> = Vec::new();

        // A smelting recipe the force has not unlocked yet has to be researched
        // first — a furnace will not smelt what the force cannot make. The
        // condition goes on both the inserts and the removal rather than on the
        // removal alone, so the plan does not load a furnace it may not yet
        // fire. See `HandCraft::expand` for why the subgoal is emitted first.
        let mut research_pre: Vec<Condition> = Vec::new();
        if let RecipeGate::NeedsResearch(tech) = recipe_gate(&ctx.state, &recipe) {
            steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
            research_pre.push(Condition::Researched(tech));
        }

        // Ingredients, fuel, and the furnace itself, as subgoals.
        for (ingredient, amount) in &ingredients {
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient.clone(),
                count: amount.saturating_mul(runs),
                whose: whose.clone(),
            }));
        }
        steps.push(Step::Subgoal(Goal::Have {
            item: "coal".into(),
            count: coal,
            whose: whose.clone(),
        }));
        steps.push(Step::Subgoal(Goal::Have {
            item: "stone-furnace".into(),
            count: 1,
            whose: whose.clone(),
        }));

        let place_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(furnace.clone()),
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: pos.clone(),
                    radius: build,
                },
                Condition::AreaFree {
                    pos: pos.clone(),
                    entity: furnace_entity.clone(),
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
            ],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
                Effect::CreateEntity(Box::new(furnace)),
            ],
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place stone-furnace at {}", pos),
        })));

        let mut insert_ids = Vec::new();
        for (ingredient, amount) in &ingredients {
            let total = amount.saturating_mul(runs);
            let id = ctx.ids.next();
            insert_ids.push(id);
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: pos.clone(),
                    entity: furnace_entity.clone(),
                    slot: InventorySlot::FurnaceSource,
                    item: ingredient.clone(),
                    count: total,
                },
                pre: {
                    let mut pre = vec![
                        Condition::AtPosition {
                            who: Actor::Role,
                            pos: pos.clone(),
                            radius: reach,
                        },
                        Condition::EntityAt {
                            pos: pos.clone(),
                            name: "stone-furnace".into(),
                        },
                        Condition::HasItem {
                            who: Actor::Role,
                            item: ingredient.clone(),
                            count: total,
                        },
                    ];
                    pre.extend(research_pre.iter().cloned());
                    pre
                },
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: ingredient.clone(),
                    count: total,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("insert {} {}", total, ingredient),
            })));
        }

        let fuel_id = ctx.ids.next();
        insert_ids.push(fuel_id);
        steps.push(Step::Act(Box::new(Action {
            id: fuel_id,
            kind: ActionKind::Insert {
                pos: pos.clone(),
                entity: furnace_entity.clone(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: coal,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: pos.clone(),
                    radius: reach,
                },
                Condition::EntityAt {
                    pos: pos.clone(),
                    name: "stone-furnace".into(),
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count: coal,
                },
            ],
            eff: vec![Effect::LoseItem {
                who: Actor::Role,
                item: "coal".into(),
                count: coal,
            }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("fuel the furnace with {} coal", coal),
        })));

        let remove_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: remove_id,
            kind: ActionKind::Remove {
                pos: pos.clone(),
                entity: furnace_entity.clone(),
                slot: InventorySlot::FurnaceResult,
                item: item.clone(),
                count: need,
            },
            pre: {
                let mut pre = vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: pos.clone(),
                        radius: reach,
                    },
                    Condition::EntityAt {
                        pos: pos.clone(),
                        name: "stone-furnace".into(),
                    },
                ];
                pre.extend(research_pre.iter().cloned());
                pre
            },
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: item.clone(),
                count: need,
            }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {} {} from the furnace", need, item),
        })));

        // The furnace runs between the last insert and the removal. The bot is
        // free to do other work across this lag — that is what it is for.
        //
        // `smelting_ticks`, not `recipe_ticks`: a machine divides the recipe's
        // time by its own crafting speed. `furnace_entity` is the machine
        // actually acting, so the speed is read for *that* entity rather than
        // assumed — see `machine_crafting_speed` for why this is written now
        // even though it changes nothing while the furnace is always stone.
        // One craft cycle of headroom, because this lag is a *schedule
        // constraint* and not a report. A removal placed at exactly the
        // predicted completion is right half the time by construction, and
        // being early costs an entire replan cycle while being late costs
        // scheduled slack the bot spends on other work anyway.
        //
        // The mechanism the headroom covers: the furnace cannot begin before
        // the ore lands, and the insert action's reply tick is when the *mod*
        // returned, not when the furnace next looked at its input slot. A start
        // that misses the current craft boundary loses up to one cycle.
        //
        // Observed before this: a removal at insert+1924 against a modelled
        // 1920 came back with nine plates out of ten, and the run spent the
        // rest of its iteration budget replanning around the one that was
        // missing.
        let per_run = smelting_ticks(&ctx.state, &recipe, &furnace_entity);
        let smelt_lag = per_run.saturating_mul(runs).saturating_add(per_run);
        for id in insert_ids {
            let lag = if id == fuel_id { 0 } else { smelt_lag };
            steps.push(Step::Link {
                from: id,
                to: remove_id,
                lag,
            });
        }

        attach_unlock(&mut steps, item, unlocks);
        Ok(steps)
    }
}

/// Mine the shortfall straight out of the ground.
pub struct Mine;

impl Method for Mine {
    fn name(&self) -> &'static str {
        "mine"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Some(Demand { item, need, .. }) = demand(goal, state) else {
            return false;
        };
        if need == 0 {
            return false;
        }
        // Position-independent, as before: whether the patches can supply
        // `need` in total does not depend on which bot is asking. Which tiles
        // are nearest is `expand`'s business, where the chain actor is known —
        // so this asks the total directly instead of building and sorting the
        // union of every tile of every patch only to test it for emptiness.
        resource_supply_at_least(state, item, need)
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            unlocks,
            ..
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let bot = ctx.state.bot(ctx.chain_actor);
        let from = bot.map(|b| b.position.clone()).unwrap_or_default();
        let reach = bot.map(|b| b.resource_reach_distance).unwrap_or(3.0);
        let tiles = resource_tiles_for(&ctx.state, item, &from, need);
        if tiles.is_empty() {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        }

        let mut steps: Vec<Step> = Vec::new();
        for (pos, take) in tiles {
            let action = Action {
                id: ctx.ids.next(),
                kind: ActionKind::Mine {
                    pos: pos.clone(),
                    item: item.clone(),
                    count: take,
                },
                pre: vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: pos.clone(),
                        radius: reach,
                    },
                    Condition::ResourceAvailable {
                        pos: pos.clone(),
                        item: item.clone(),
                        count: take,
                    },
                ],
                eff: vec![
                    Effect::ConsumeResource {
                        pos,
                        item: item.clone(),
                        count: take,
                    },
                    Effect::GainItem {
                        who: Actor::Role,
                        item: item.clone(),
                        count: take,
                    },
                ],
                duration: mining_ticks(&ctx.state, item).saturating_mul(take),
                pinned: None,
                label: format!("mine {} {}", take, item),
            };
            steps.push(Step::Act(Box::new(action)));
        }
        attach_unlock(&mut steps, item, unlocks);
        Ok(steps)
    }
}

/// Craft the shortfall by hand, expanding each ingredient as a subgoal.
pub struct HandCraft;

impl Method for HandCraft {
    fn name(&self) -> &'static str {
        "hand-craft"
    }

    fn converges(&self, goal: &Goal, state: &PlanState) -> bool {
        let Some(Demand {
            item, need, whose, ..
        }) = demand(goal, state)
        else {
            return false;
        };
        if need == 0 {
            return false;
        }
        let Some(recipe) = recipe_for(state, item) else {
            return false;
        };
        // One craft action carries a `HasItem` for every ingredient, so each
        // one that still has to be produced is a separate sub-chain that must
        // land in the same inventory. Two or more of those is a convergence.
        ingredients_of(&recipe)
            .iter()
            .filter(|(ingredient, amount)| shortfall(state, ingredient, *amount, whose) > 0)
            .count()
            >= 2
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Some(Demand { item, need, .. }) = demand(goal, state) else {
            return false;
        };
        if need == 0 {
            return false;
        }
        let Some(recipe) = recipe_for(state, item) else {
            return false;
        };
        recipe.category == CRAFTING_CATEGORY
            && recipe_gate(state, &recipe) != RecipeGate::Unobtainable
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            whose,
            unlocks,
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let recipe =
            recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;
        let runs = need.div_ceil(output_per_craft(&recipe, item));

        let mut steps: Vec<Step> = Vec::new();
        let mut pre = Vec::new();
        let mut eff = Vec::new();

        // A recipe the force has not unlocked yet is craftable only after its
        // technology is researched, so say so — both as a subgoal that does the
        // research and as a precondition, which is what `infer_edges` turns
        // into the ordering edge that keeps the craft after the research.
        //
        // Emitted before the ingredients on purpose: the research subgoal
        // applies `Effect::Researched` as it expands, so a sibling ingredient
        // whose own recipe the same technology unlocks comes out `Open` and
        // costs nothing further.
        if let RecipeGate::NeedsResearch(tech) = recipe_gate(&ctx.state, &recipe) {
            steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
            pre.push(Condition::Researched(tech));
        }

        for (ingredient, amount) in ingredients_of(&recipe) {
            let total = amount.saturating_mul(runs);
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient.clone(),
                count: total,
                whose: whose.clone(),
            }));
            pre.push(Condition::HasItem {
                who: Actor::Role,
                item: ingredient.clone(),
                count: total,
            });
            eff.push(Effect::LoseItem {
                who: Actor::Role,
                item: ingredient,
                count: total,
            });
        }
        eff.push(Effect::GainItem {
            who: Actor::Role,
            item: item.clone(),
            count: runs.saturating_mul(output_per_craft(&recipe, item)),
        });

        steps.push(Step::Act(Box::new(Action {
            id: ctx.ids.next(),
            kind: ActionKind::Craft {
                item: item.clone(),
                count: runs,
            },
            pre,
            eff,
            duration: recipe_ticks(&recipe).saturating_mul(runs),
            pinned: None,
            label: format!("craft {} {}", runs, item),
        })));

        attach_unlock(&mut steps, item, unlocks);
        Ok(steps)
    }
}

/// The methods this crate ships, in preference order.
/// Research a technology: get its prerequisites researched, gather its science
/// packs, then run the research itself.
///
/// **Prerequisites recurse.** `Researched(t)` emits a `Researched(p)` subgoal
/// for each of `t`'s prerequisites rather than refusing when one is missing,
/// because refusing would make the goal useless: a caller asking for `military`
/// wants the technology, and telling them to go ask for `logistics` first —
/// and then for `automation` first — is asking them to walk the tech tree by
/// hand, which is exactly the decomposition this method exists to do. It also
/// makes the goal composable, since the prerequisite subgoals are ordinary
/// goals and pick up `AlreadySatisfied` for free.
///
/// Recursion terminates because the technology graph is a DAG in every world
/// the game produces, and because each research that *is* emitted applies
/// `Effect::Researched` immediately, so a technology reached twice through two
/// different prerequisites is expanded once and then satisfied. Neither of
/// those is a guarantee about arbitrary data, so the backstop is the driver's:
/// a cycle in a hand-written or modded technology table runs the expansion into
/// `MAX_EXPANSION_DEPTH` and comes back as `ExpansionTooDeep`, naming the goal.
/// It cannot hang.
///
/// **On consumption.** The research action carries `LoseItem` for every pack it
/// needs, so a second research cannot be planned out of the same packs. That is
/// right about the packs and approximate about who spends them: in the game a
/// lab consumes them, not the bot, and nothing here yet moves packs from a bot
/// into a lab. Until a `Supply the labs` method exists, the plan debits the
/// bot, which is the conservative direction — it over-counts what has to be
/// produced rather than under-counting it.
pub struct Researched;

impl Method for Researched {
    fn name(&self) -> &'static str {
        "research"
    }

    fn converges(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Researched(name) = goal else {
            return false;
        };
        let Some(tech) = state.technology(name) else {
            return false;
        };
        // One research action carries a `HasItem` for every pack, exactly like
        // one craft action carries one for every ingredient, so the same
        // reasoning applies: each pack that still has to be produced is a
        // separate sub-chain, and two or more of them have to land in one
        // inventory. Prerequisites are not counted — a `Researched` effect is
        // world-scoped and satisfied by whoever ran it, so it pulls nothing
        // into anyone's inventory.
        //
        // A trigger technology's requirement counts the same way: it is one
        // more thing the research action needs in the acting bot's inventory,
        // so a trigger plus a pack bill converge exactly as two packs would.
        // `trigger_requirement` cannot report here — `converges` has no error
        // channel — so an inexpressible trigger contributes nothing and the
        // refusal is left to `expand`, which is reached either way.
        let trigger = trigger_requirement(state, &tech)
            .ok()
            .flatten()
            .into_iter()
            .collect::<Vec<_>>();
        research_ingredients(&tech)
            .iter()
            .chain(trigger.iter())
            .filter(|(item, count)| needs_producing(state, item, *count))
            .count()
            >= 2
    }

    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        // Deliberately not conditioned on the technology existing. A goal
        // naming a technology no force has heard of has to *reach* `expand`, so
        // that it can be refused by name; declining it here would leave the
        // registry with no method for it and the caller would get
        // `NoApplicableMethod` — "no method can satisfy goal: research foo",
        // which reads as "that technology is out of reach in this world"
        // rather than "there is no such technology".
        //
        // Nor is it conditioned on the technology being unresearched:
        // `AlreadySatisfied` is registered ahead of this and owns that
        // question for every goal kind.
        matches!(goal, Goal::Researched(_))
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Researched(name) = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let tech = ctx
            .state
            .technology(name)
            .ok_or_else(|| PlannerError::UnknownTechnology {
                technology: name.clone(),
            })?;

        // A `BTreeSet` rather than the listed order: it dedupes a table that
        // names a prerequisite twice, and it fixes an order for the emitted
        // conditions that does not depend on how the force's data happened to
        // be written down.
        let prerequisites: BTreeSet<String> = tech
            .prerequisites
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let ingredients = research_ingredients(&tech);

        // A Factorio 2.0 trigger technology, if this is one. The `?` is the
        // point: a trigger this planner cannot express, or one that only this
        // technology could unlock, refuses here rather than falling through to
        // the pack path — where the empty bill and zero energy below would
        // plan it as free and hand the caller a makespan missing the work.
        let trigger = trigger_requirement(&ctx.state, &tech)?;

        let mut steps: Vec<Step> = Vec::new();
        for prerequisite in &prerequisites {
            steps.push(Step::Subgoal(Goal::Researched(prerequisite.clone())));
        }
        // A `craft-item` trigger fires on the **act of producing**, and the
        // game researches the technology itself. So the trigger path emits one
        // subgoal and **no research action at all**: there is nothing to issue,
        // and `add_research` refuses a trigger technology outright.
        //
        // `Produced`, not `Have`: `Have` is satisfied by possession, so a bot
        // already carrying the item would produce nothing and the trigger would
        // never fire. `Produced` carries the technology it unlocks so that
        // whichever method makes the item -- craft, smelt or mine -- can hang
        // `Effect::Researched` on the action that does it.
        //
        // `Holder::Share` for the same reason the pack bill uses it: one action
        // reading one bot's inventory.
        if let Some((item, count)) = &trigger {
            steps.push(Step::Subgoal(Goal::Produced {
                item: item.clone(),
                count: *count,
                whose: Holder::Share(ctx.chain_actor),
                unlocks: Some(name.clone()),
            }));
            return Ok(steps);
        }
        for (item, count) in &ingredients {
            // `Holder::Share`, not `Holder::Anyone`. The research is one action
            // reading one bot's inventory, so the packs have to end up in one
            // inventory, and `Holder::Anyone` sizes its shortfall against the
            // sum across the roster instead. The difference is not academic:
            // every bot the Lua runner starts carries one stone furnace, so a
            // roster of four satisfies `Have { stone-furnace, 1, Anyone }`
            // without crafting anything, and the second furnace a science pack
            // chain needs is then placed by a bot that has already spent its
            // own. `Share` sizes against the chain actor — the same bot the
            // driver simulates every effect in this subtree against — and
            // commits nobody to running the work, which stays the scheduler's
            // decision.
            steps.push(Step::Subgoal(Goal::Have {
                item: item.clone(),
                count: *count,
                whose: Holder::Share(ctx.chain_actor),
            }));
        }

        let mut pre: Vec<Condition> = prerequisites
            .iter()
            .map(|prerequisite| Condition::Researched(prerequisite.clone()))
            .collect();
        let mut eff: Vec<Effect> = Vec::new();
        for (item, count) in &ingredients {
            pre.push(Condition::HasItem {
                who: Actor::Role,
                item: item.clone(),
                count: *count,
            });
            eff.push(Effect::LoseItem {
                who: Actor::Role,
                item: item.clone(),
                count: *count,
            });
        }
        // No trigger handling here: the trigger path returned above. Anything
        // reaching this point is unlocked by science packs, which is what the
        // bill and the research action below have always assumed.
        eff.push(Effect::Researched(name.clone()));

        // No explicit `Link` steps: every edge this action needs is stated as a
        // precondition, and `ActionNetwork::infer_edges` turns
        // `Effect::GainItem`/`HasItem` and `Effect::Researched`/
        // `Condition::Researched` into ordering edges. A method cannot link to
        // its subgoals' actions in any case — it never sees their ids.
        steps.push(Step::Act(Box::new(Action {
            id: ctx.ids.next(),
            kind: ActionKind::Research { tech: name.clone() },
            pre,
            eff,
            duration: research_ticks(&tech),
            pinned: None,
            label: format!("research {}", name),
        })));

        Ok(steps)
    }
}

pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(Smelt))
        .with(Box::new(HandCraft))
        .with(Box::new(Mine))
        .with(Box::new(Researched))
}

/// Split a shared goal into one independent chain per bot.
///
/// The chains never coordinate: each mines, smelts and crafts its own share.
/// They are emitted as `Holder::Share(_)` subgoals so the other methods handle
/// them without recursing back into this one — and so that the driver opens a
/// chain per share, which is what keeps each share's steps in one inventory.
/// A `Share` rather than a `Bot` because a share sizes against a bot without
/// instructing anyone to run it; the chain it opens therefore has no owner.
///
/// A share of one is still worth emitting: it produces a single chain rather
/// than a split, and that chain is the whole point. Without it a top-level goal
/// with a shortfall of one — `Have(automation-science-pack, 1)`, or the last
/// iteration of any incremental plan — would expand with no chain at all, and a
/// branching recipe's two roots would land on different bots.
pub struct SplitAcrossBots {
    pub bots: Vec<BotId>,
}

impl Method for SplitAcrossBots {
    fn name(&self) -> &'static str {
        "split-across-bots"
    }

    /// Only a goal the caller asked for may be scattered.
    ///
    /// A subgoal exists because some action downstream consumes it, out of one
    /// inventory: `Smelt` and `HandCraft` propagate `whose` verbatim, so a
    /// shared goal stays `Holder::Anyone` all the way down, and claiming an
    /// *intermediate* one hands two bots half the ingredients each for a craft
    /// that needs them together. `in_chain` is redundant given `top_level`
    /// here — a top-level goal can never itself be `in_chain`, because
    /// `in_chain` reflects only the chain a goal *inherited*, computed before
    /// this goal's own method (this one, or a converging one) gets to open
    /// one — but it states the rule the whole way round: never scatter what a
    /// chain is already gathering.
    fn claims(&self, site: GoalSite) -> bool {
        site.top_level && !site.in_chain
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if !matches!(whose, Holder::Anyone) {
            return false;
        }
        // An empty roster has no share to give out, and `expand` would divide
        // by the chain count.
        !self.bots.is_empty() && shortfall(state, item, *count, whose) > 0
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let need = shortfall(&ctx.state, item, *count, whose);

        // `registry_for` copies the caller's slice verbatim, so a caller can
        // list the same `BotId` twice. Without deduping, that used to open
        // two chains for one bot, the second sized after the first had
        // already reserved its share against the *same* raw holding, so it
        // over-asked. Reading the distinct bots' `spare` first makes the
        // split independent of how many times a bot's id appears in the
        // slice, only whether it appears at all.
        //
        // `spare(b)` is `available(&Holder::Share(b), item)`, the same
        // ledger the emitted subgoal's own shortfall is taken against (see
        // the comment on the target below) -- not the raw holding, which
        // does not see what an earlier split already reserved and produced
        // 24 ore for two shortfalls of 8 across four identical bots instead
        // of 16.
        //
        // Candidates are ordered `(spare, BotId)` ascending -- poorest
        // first, `BotId` breaking ties -- and only the first `k` participate.
        // `BotId` is unique within the deduped roster, so the key is a total
        // order and `sort_unstable` is exactly as deterministic as a stable
        // sort would be; nobody should "fix" this to `sort`. The order does
        // not depend on the caller's slice order at all, only on the set of
        // bots and their holdings.
        let mut seen = BTreeSet::new();
        let mut candidates: Vec<(u32, BotId)> = self
            .bots
            .iter()
            .copied()
            .filter(|b| seen.insert(*b))
            .map(|bot| (ctx.state.available(&Holder::Share(bot), item), bot))
            .collect();
        candidates.sort_unstable();

        let chains = (candidates.len() as u32).min(need);
        let base = need / chains;
        let remainder = need % chains;

        // The work itself is split evenly across participants; holdings
        // decide only *who* participates and *who carries the remainder*,
        // never how much a participant is asked to produce. The obvious
        // alternative -- levelling final holdings, so a bot already holding
        // more produces less -- reads more principled but is worse: on a
        // `have(iron-plate, 20)` goal with one bot ahead by 8, equal work per
        // participant measured 2156 ticks against levelling's 2427. Equal
        // work keeps every participant busy for the same stretch; levelling
        // concentrates the same total work onto fewer bots and lengthens the
        // makespan. So the remainder -- the one place holdings change the
        // *amount* of work -- goes to the poorest participants, not to
        // whichever bots the caller happened to list first.
        let mut shares: BTreeMap<BotId, u32> = BTreeMap::new();
        for (index, &(spare, bot)) in candidates.iter().take(chains as usize).enumerate() {
            let work = base + if (index as u32) < remainder { 1 } else { 0 };
            // A `Have` goal states a holding, not a delivery, so a share of
            // one handed to a bot already holding five is a goal that is
            // already met — and the share evaporates. Ask for what the bot
            // has *plus* its share, so the shortfall the other methods see
            // is the share: the subgoal below is claimed a frame later by
            // whichever method satisfies `Have { count, whose: Share(bot) }`,
            // and that method computes its own shortfall against `available`
            // (see `shortfall`/`demand` above), so the target must be stated
            // in that same ledger or the chain is asked for `share +
            // reserved` instead of `share`.
            shares.insert(bot, spare.saturating_add(work));
        }

        // Emit in ascending `BotId`, not the sorted participation order:
        // emission order fixes `ActionId` allocation and therefore
        // `schedule`'s `(end, ActionId, BotId)` tie-break, so a symmetric
        // roster's plan does not move when only the *order* candidates were
        // considered in changes. `BTreeMap` gives ascending order for free.
        let steps = shares
            .into_iter()
            .map(|(bot, target)| {
                Step::Subgoal(Goal::Have {
                    item: item.clone(),
                    count: target,
                    whose: Holder::Share(bot),
                })
            })
            .collect();
        Ok(steps)
    }
}

/// The registry to use for a given bot roster.
pub fn registry_for(bots: &[BotId]) -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(SplitAcrossBots {
            bots: bots.to_vec(),
        }))
        .with(Box::new(Smelt))
        .with(Box::new(HandCraft))
        .with(Box::new(Mine))
        .with(Box::new(Researched))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ActionId;
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::method::util::unlocking_technology;
    use crate::network::ActionNetwork;
    use crate::schedule::{StepKind, schedule};
    use crate::state::PlanState;
    use factorio_bot_core::factorio::util::calculate_distance;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{Position, ResearchTrigger};
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), bots)
    }

    /// The same world, plus the one force `crate::test_world` bolts on. Every
    /// research test uses this; nothing else does, so the fixtures the
    /// makespan figures are pinned to stay exactly as they were.
    fn tech_state(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(crate::test_world::world_with_technologies()), bots)
    }

    /// The steps `Researched` emits for `tech`, without running the driver
    /// over them. Asserting a bill of materials against the network the
    /// subgoals eventually expand into would be asserting it against the
    /// *shortfall* — which the bot's starting inventory moves — rather than
    /// against the technology's stated cost.
    fn research_steps(state: &PlanState, tech: &str) -> Vec<Step> {
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        Researched
            .expand(&Goal::Researched(tech.into()), &mut ctx)
            .expect("the fixture technologies all expand")
    }

    fn subgoals(steps: &[Step]) -> Vec<Goal> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(goal) => Some(goal.clone()),
                _ => None,
            })
            .collect()
    }

    fn research_actions(net: &ActionNetwork) -> Vec<&Action> {
        net.actions()
            .filter(|a| matches!(a.kind, ActionKind::Research { .. }))
            .collect()
    }

    fn researched_techs(net: &ActionNetwork) -> Vec<String> {
        let mut names: Vec<String> = research_actions(net)
            .iter()
            .filter_map(|a| match &a.kind {
                ActionKind::Research { tech } => Some(tech.clone()),
                _ => None,
            })
            .collect();
        names.sort();
        names
    }

    /// `automation` is the one fixture technology carrying the real game's
    /// numbers: 10 units of one automation science pack each, 600 ticks per
    /// unit. Both the pack bill and the duration are the product, and both are
    /// written out rather than recomputed from the fixture — a test that says
    /// `count == tech.research_unit_count * amount` passes just as happily
    /// against a method that forgot to multiply at all, because it would be
    /// making the same mistake twice.
    #[test]
    fn a_research_asks_for_one_unit_bill_times_the_unit_count() {
        let s = tech_state(&[BotId(1)]);
        let steps = research_steps(&s, "automation");
        assert_eq!(
            subgoals(&steps),
            vec![Goal::Have {
                item: "automation-science-pack".into(),
                count: 10,
                whose: Holder::Share(BotId(1)),
            }]
        );

        let Some(Step::Act(action)) = steps.last() else {
            panic!("the last step must be the research action, got {steps:?}");
        };
        assert_eq!(
            action.kind,
            ActionKind::Research {
                tech: "automation".into()
            }
        );
        assert!(
            action.pre.contains(&Condition::HasItem {
                who: Actor::Role,
                item: "automation-science-pack".into(),
                count: 10,
            }),
            "the action must require the whole bill, got {:?}",
            action.pre
        );
        assert!(
            action.eff.contains(&Effect::LoseItem {
                who: Actor::Role,
                item: "automation-science-pack".into(),
                count: 10,
            }),
            "the packs are spent, got {:?}",
            action.eff
        );
        assert!(
            action
                .eff
                .contains(&Effect::Researched("automation".into()))
        );
        assert_eq!(action.duration, 6000, "10 units at 600 ticks each");
    }

    /// The multiply, on a technology whose ingredient `amount` is not 1 and
    /// whose unit count is not `automation`'s. `military` costs 5 units of two
    /// packs each. A method that dropped the `amount` would ask for 5; one
    /// that dropped `research_unit_count` would ask for 2; one that read the
    /// wrong technology would ask for 10 of `automation`'s or 20 of
    /// `logistics`'.
    #[test]
    fn an_ingredient_amount_is_multiplied_by_the_unit_count() {
        let s = tech_state(&[BotId(1)]);
        let steps = research_steps(&s, "military");
        assert_eq!(
            subgoals(&steps),
            vec![
                Goal::Researched("logistics".into()),
                Goal::Have {
                    item: "automation-science-pack".into(),
                    count: 10,
                    whose: Holder::Share(BotId(1)),
                },
            ]
        );

        // ... and a different technology gets a different bill, so the 10
        // above cannot be a constant the method returns for everything.
        let logistics = research_steps(&s, "logistics");
        assert!(
            subgoals(&logistics).contains(&Goal::Have {
                item: "automation-science-pack".into(),
                count: 20,
                whose: Holder::Share(BotId(1)),
            }),
            "logistics costs 20 units of one pack, got {:?}",
            subgoals(&logistics)
        );
    }

    /// Two ingredient types, one of which is spelled out with `amount` 3 and
    /// `research_unit_count` 2. Both bills, in the technology's own order.
    #[test]
    fn a_multi_ingredient_research_bills_every_ingredient() {
        let s = tech_state(&[BotId(1)]);
        assert_eq!(
            subgoals(&research_steps(&s, "mixed-research")),
            vec![
                Goal::Have {
                    item: "automation-science-pack".into(),
                    count: 2,
                    whose: Holder::Share(BotId(1)),
                },
                Goal::Have {
                    item: "iron-plate".into(),
                    count: 6,
                    whose: Holder::Share(BotId(1)),
                },
            ]
        );
    }

    /// Prerequisites recurse rather than being refused, and they recurse all
    /// the way: `military` needs `logistics`, which needs `automation`.
    #[test]
    fn prerequisites_expand_into_their_own_research() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let net = expand(
            &[Goal::Researched("military".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("military must be reachable");
        assert_eq!(
            researched_techs(&net),
            vec!["automation", "logistics", "military"]
        );
    }

    /// The order between them is stated, not left to chance: each research
    /// carries `Condition::Researched` for its prerequisites, and inference
    /// turns that into an edge from the action that provides it.
    #[test]
    fn a_research_is_ordered_after_its_prerequisite() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let net = expand(
            &[Goal::Researched("logistics".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("logistics must be reachable");

        let find = |name: &str| {
            research_actions(&net)
                .into_iter()
                .find(|a| a.kind == ActionKind::Research { tech: name.into() })
                .unwrap_or_else(|| panic!("no research action for {name}"))
                .id
        };
        let automation = find("automation");
        let logistics = find("logistics");
        assert!(
            net.action(logistics)
                .expect("logistics action")
                .pre
                .contains(&Condition::Researched("automation".into())),
            "the prerequisite must be stated as a precondition"
        );
        assert!(
            net.preds(logistics).iter().any(|(id, _)| *id == automation),
            "logistics must be ordered after automation, preds were {:?}",
            net.preds(logistics)
        );
    }

    /// The world says `steel-processing` is done. Nothing is planned — not a
    /// research action, and not the 50 science packs it would otherwise cost.
    #[test]
    fn a_technology_the_world_already_has_expands_to_nothing() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        assert!(s.is_researched("steel-processing"));
        let net = expand(
            &[Goal::Researched("steel-processing".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("an already-researched technology is satisfiable");
        assert_eq!(net.len(), 0, "nothing to do");
    }

    /// The plan's own overlay counts too: `logistics` researches `automation`
    /// on the way, so a second goal naming `automation` adds nothing. Without
    /// the overlay this would plan `automation` twice and buy 20 packs for it.
    #[test]
    fn a_technology_this_plan_already_researched_is_not_researched_again() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let net = expand(
            &[Goal::All(vec![
                Goal::Researched("logistics".into()),
                Goal::Researched("automation".into()),
            ])],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("both goals must be reachable");
        assert_eq!(researched_techs(&net), vec!["automation", "logistics"]);
    }

    /// A technology no force defines is refused by name, not as "no method can
    /// satisfy goal: research …", which would read as "unreachable in this
    /// world" and send the caller hunting prerequisites.
    #[test]
    fn an_unknown_technology_is_refused_by_name() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let err = expand(
            &[Goal::Researched("nuclear-alchemy".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect_err("an unknown technology cannot be planned");
        assert!(
            matches!(&err, PlannerError::UnknownTechnology { technology } if technology == "nuclear-alchemy"),
            "expected UnknownTechnology, got {err:?}"
        );
        assert!(
            err.to_string().contains("nuclear-alchemy"),
            "the message must name the technology, got: {err}"
        );
    }

    /// A world with no forces at all — the shared `fixture_world` — is the
    /// same story: every technology is unknown, and says so.
    #[test]
    fn a_world_without_forces_knows_no_technologies() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let err = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect_err("a world with no forces has no technologies");
        assert!(
            matches!(&err, PlannerError::UnknownTechnology { technology } if technology == "automation"),
            "expected UnknownTechnology, got {err:?}"
        );
    }

    /// Real technology data is a DAG, so this cannot happen in a live world —
    /// but a hand-written or modded table can say anything, and the recursion
    /// must come back rather than run forever. `loop-a` requires `loop-b`
    /// requires `loop-a`.
    #[test]
    fn a_cycle_in_the_prerequisites_terminates_as_an_error() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let err = expand(
            &[Goal::Researched("loop-a".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect_err("a prerequisite cycle cannot be planned");
        // Not `depth == MAX_EXPANSION_DEPTH`: the error is *built from* that
        // constant, so the clause cannot fail whatever the code does. What the
        // code could get wrong is *which* goal it blames — reporting the
        // caller's goal, or the last `Have` it happened to hold — so that is
        // what is asserted. The two cycle members carry no ingredients, so the
        // only goals in this recursion are the two research goals, and naming
        // either is right.
        let PlannerError::ExpansionTooDeep { goal, .. } = &err else {
            panic!("expected ExpansionTooDeep, got {err:?}");
        };
        assert!(
            goal == "research loop-a" || goal == "research loop-b",
            "the error must blame the research goal that ran too deep, got {goal:?}"
        );
    }

    /// Convergence, for the same reason hand-crafting converges: one research
    /// action carries a `HasItem` for every pack, so two packs that both have
    /// to be produced must meet in one inventory. One that does not — because
    /// the bot already holds it — is not a convergence.
    #[test]
    fn research_converges_only_when_two_ingredients_need_producing() {
        let s = tech_state(&[BotId(1)]);
        let mixed = Goal::Researched("mixed-research".into());
        assert!(
            Researched.converges(&mixed, &s),
            "a science pack and an iron plate both have to be made"
        );

        let mut stocked = s.fork();
        stocked.gain(BotId(1), "iron-plate", 6);
        assert!(
            !Researched.converges(&mixed, &stocked),
            "with the plates in hand only one thing is still produced"
        );

        assert!(
            !Researched.converges(&Goal::Researched("automation".into()), &s),
            "one ingredient type is never a convergence"
        );
    }

    /// Stock that is already promised to a pending action is not stock this
    /// research can count on, so a convergence it would otherwise have been
    /// spared is a convergence after all.
    ///
    /// This is `needs_producing`'s half of the reservation rule. `shortfall`'s
    /// half is exercised everywhere; this one has its own question — "does any
    /// single bot hold the whole count" — and its own way of getting the
    /// answer wrong, which is to read the raw holding and count items another
    /// action has already been promised.
    #[test]
    fn a_research_converges_again_once_its_stock_is_promised_elsewhere() {
        let s = tech_state(&[BotId(1)]);
        let mixed = Goal::Researched("mixed-research".into());

        let mut stocked = s.fork();
        stocked.gain(BotId(1), "iron-plate", 6);
        assert!(
            !Researched.converges(&mixed, &stocked),
            "the plates are in hand, so only the packs are still produced"
        );

        stocked.reserve(&Holder::Share(BotId(1)), "iron-plate", 6);
        assert!(
            Researched.converges(&mixed, &stocked),
            "but plates promised to another action have to be made again"
        );
    }

    /// The roster the Lua runner actually starts: four bots, each carrying the
    /// default inventory `Planner::initiate_missing_players_with_default_
    /// inventory` hands out — one stone furnace apiece, among other things.
    ///
    /// This is the case that caught the ingredient subgoals asking for
    /// `Holder::Anyone`. Four furnaces spread over four bots satisfy
    /// `Have { stone-furnace, 1, Anyone }` without crafting one, so the science
    /// pack chain's second smelt places a furnace the acting bot has already
    /// spent, and expansion dies with `bot 1 has 0 stone-furnace, needs 1`. A
    /// single-bot roster cannot show it — with one bot the roster total *is*
    /// that bot's inventory — which is exactly the fixture-too-small trap.
    #[test]
    fn a_research_plans_against_a_roster_whose_bots_each_hold_one_furnace() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = tech_state(&bots);
        for bot in bots {
            s.gain(bot, "stone-furnace", 1);
        }
        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a stocked roster must not make the research unplannable");
        assert_eq!(research_actions(&net).len(), 1);
        schedule(&net, &s, &bots).expect("and it must still schedule");
    }

    // ---- `holds`: satisfaction asked directly ------------------------------

    /// The live-run milestone that made this necessary.
    ///
    /// `run-1788300756-94802`'s third milestone was `goal.have("iron-plate",
    /// 10)`, and it closed in zero ticks with zero iterations because the plan
    /// came back empty. It was *named* "smelt iron plates x10", and nothing was
    /// smelted — but the goal as stated genuinely held: freeplay starts every
    /// player with eight iron plates, and three bots hold twenty-four between
    /// them. The empty plan was right. What was missing was any way for the
    /// caller to establish that rather than infer it.
    #[test]
    fn a_roster_holding_the_count_between_them_satisfies_a_shared_goal() {
        let bots = [BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for bot in bots {
            s.gain(bot, "iron-plate", 8);
        }
        let goal = Goal::Have {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Anyone,
        };
        assert_eq!(holds(&goal, &s), Some(true), "24 between them covers 10");

        let per_bot = Goal::Have {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Share(BotId(2)),
        };
        assert_eq!(
            holds(&per_bot, &s),
            Some(false),
            "but no single bot holds ten, and the holder is what decides"
        );
    }

    /// The third value, and why it is not `false`. A production is an event:
    /// no inventory read settles whether it happened, so the honest answer is
    /// that this question cannot be answered by looking.
    #[test]
    fn a_production_goal_has_no_answer_from_possession() {
        let bots = [BotId(1)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-plate", 50);
        assert_eq!(
            holds(
                &Goal::Produced {
                    item: "iron-plate".into(),
                    count: 50,
                    whose: Holder::Share(BotId(1)),
                    unlocks: None,
                },
                &s
            ),
            None,
            "fifty in hand says nothing about fifty having been made"
        );
        assert_eq!(
            holds(
                &Goal::Producing {
                    item: "iron-plate".into(),
                    rate: 30.0,
                },
                &s
            ),
            None,
            "and nothing here models a throughput at all"
        );
    }

    #[test]
    fn a_bundle_holds_only_when_every_member_does() {
        let bots = [BotId(1)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-plate", 8);
        let met = Goal::Have {
            item: "iron-plate".into(),
            count: 4,
            whose: Holder::Anyone,
        };
        let unmet = Goal::Have {
            item: "iron-plate".into(),
            count: 40,
            whose: Holder::Anyone,
        };
        let unanswerable = Goal::Producing {
            item: "iron-plate".into(),
            rate: 30.0,
        };
        assert_eq!(holds(&Goal::All(vec![met.clone()]), &s), Some(true));
        assert_eq!(
            holds(&Goal::All(vec![met.clone(), unmet.clone()]), &s),
            Some(false)
        );
        assert_eq!(
            holds(&Goal::All(vec![met.clone(), unanswerable.clone()]), &s),
            None,
            "one unanswerable member leaves the bundle unanswerable"
        );
        assert_eq!(
            holds(&Goal::All(vec![unanswerable, unmet]), &s),
            Some(false),
            "but a member that definitely does not hold settles it anyway"
        );
    }

    /// `holds` and the empty plan must agree wherever `holds` has an opinion.
    /// This is the invariant `supervisor.lua` was assuming and could not
    /// check; it is checked here instead, so the Lua side may rely on it.
    #[test]
    fn an_empty_expansion_and_a_held_goal_agree() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-plate", 8);
        s.gain(BotId(2), "iron-plate", 8);
        for count in [1u32, 10, 16, 17, 40] {
            let goal = Goal::Have {
                item: "iron-plate".into(),
                count,
                whose: Holder::Anyone,
            };
            let net = expand(
                std::slice::from_ref(&goal),
                &s,
                &registry_for(&bots),
                BotId(1),
            )
            .expect("iron plate is reachable in the fixture world");
            assert_eq!(
                net.is_empty(),
                holds(&goal, &s) == Some(true),
                "an empty plan and a held goal must be the same thing for {count}"
            );
        }
    }

    /// D1, end to end. Two forces disagree about `automation`: `alpha`, which
    /// sorts first and is therefore the one this plan acts for, has not
    /// researched it; `zeta` has. The plan must research it.
    ///
    /// Before the acting force was fixed, `is_researched` answered over *any*
    /// force and said yes, `AlreadySatisfied` claimed the goal, and `expand`
    /// returned an empty network — the planner silently declining to research
    /// something the acting force lacks. An empty network is the failure mode,
    /// so the assertion is on what the plan contains, not on it being `Ok`.
    #[test]
    fn a_force_that_lacks_a_technology_researches_it_whatever_other_forces_have() {
        let bots = [BotId(1)];
        let world = Arc::new(crate::test_world::world_with_forces(&[
            ("alpha", false),
            ("zeta", true),
        ]));
        let s = PlanState::from_world(world, &bots);
        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("the acting force can research it");
        assert_eq!(
            research_actions(&net).len(),
            1,
            "the acting force has not researched automation, so the plan must"
        );
        assert!(
            net.actions()
                .any(|a| matches!(&a.kind, ActionKind::Mine { .. })),
            "and it must pay alpha's price rather than assume zeta's stock"
        );
    }

    /// The mirror, so neither half is a constant: when the acting force *has*
    /// researched it, nothing is planned even though another force has not.
    #[test]
    fn a_force_that_has_a_technology_plans_nothing_whatever_other_forces_lack() {
        let bots = [BotId(1)];
        let world = Arc::new(crate::test_world::world_with_forces(&[
            ("alpha", true),
            ("zeta", false),
        ]));
        let s = PlanState::from_world(world, &bots);
        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("an already-researched technology is satisfiable");
        assert_eq!(net.len(), 0, "nothing to do");
    }

    /// `default_registry()` is a public export, and its wiring is separate
    /// from `registry_for`'s: deleting `Researched` from one leaves the other
    /// working, so every other research test here passes with the export
    /// broken. This is the only test that would notice.
    #[test]
    fn the_default_registry_can_satisfy_a_research_goal() {
        let bots = [BotId(1)];
        let s = tech_state(&bots);
        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &default_registry(),
            BotId(1),
        )
        .expect("the default registry must hold a research method");
        assert_eq!(research_actions(&net).len(), 1);
    }

    /// A legal, acyclic prerequisite chain 20 deep expands, one research per
    /// link. `MAX_EXPANSION_DEPTH` is shared between recipe nesting and this
    /// recursion now, and this is the half that can grow without bound in real
    /// game data — Factorio's own tree runs to roughly this depth.
    ///
    /// Stated as a chain length the planner must cope with rather than as
    /// arithmetic on the constant, so raising or lowering `MAX_EXPANSION_DEPTH`
    /// cannot make this pass by definition.
    #[test]
    fn a_twenty_deep_prerequisite_chain_expands() {
        let bots = [BotId(1)];
        let world = Arc::new(crate::test_world::world_with_prerequisite_chain(20));
        let s = PlanState::from_world(world, &bots);
        let net = expand(
            &[Goal::Researched("chain-0".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a legal chain of twenty must be plannable");
        assert_eq!(research_actions(&net).len(), 20);
    }

    /// And a chain past the bound is refused rather than run forever. The
    /// cycle test above proves an *illegal* tree terminates; this proves the
    /// bound is what stops it, by hitting it with a tree that is perfectly
    /// legal and merely too deep.
    #[test]
    fn a_prerequisite_chain_past_the_bound_is_refused_not_run() {
        let bots = [BotId(1)];
        let world = Arc::new(crate::test_world::world_with_prerequisite_chain(64));
        let s = PlanState::from_world(world, &bots);
        let err = expand(
            &[Goal::Researched("chain-0".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect_err("a chain of sixty-four is past the bound");
        assert!(
            matches!(&err, PlannerError::ExpansionTooDeep { goal, .. } if goal.starts_with("research chain-")),
            "expected ExpansionTooDeep naming a chain link, got {err:?}"
        );
    }

    /// End to end: a research goal reaches a schedule, with the whole science
    /// pack chain under it, and every precondition holds when its action runs.
    #[test]
    fn a_research_goal_expands_and_schedules() {
        let bots = [BotId(1), BotId(2)];
        let mut s = tech_state(&bots);
        s.gain(BotId(1), "stone-furnace", 2);
        s.gain(BotId(2), "stone-furnace", 2);
        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("automation must be reachable in the fixture world");

        assert_eq!(research_actions(&net).len(), 1);
        let kinds: Vec<&str> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { .. } => "mine",
                ActionKind::Craft { .. } => "craft",
                ActionKind::Place { .. } => "place",
                ActionKind::Insert { .. } => "insert",
                ActionKind::Remove { .. } => "remove",
                ActionKind::Research { .. } => "research",
            })
            .collect();
        assert!(
            kinds.contains(&"mine") && kinds.contains(&"craft") && kinds.contains(&"research"),
            "the packs must actually be produced, got {kinds:?}"
        );

        // The research reaches the schedule as a step of its own, occupying the
        // 6000 ticks the technology costs. Asserted as the step's own span
        // rather than as a lower bound on the makespan: the science pack chain
        // under it is long enough that `makespan >= 6000` passes even when the
        // research is given no duration at all, which makes it a bound that
        // guards nothing.
        let plan = schedule(&net, &s, &bots).expect("a research plan must schedule");
        let research_id = research_actions(&net)[0].id;
        let steps: Vec<&crate::schedule::ScheduledStep> = plan
            .steps
            .iter()
            .filter(
                |step| matches!(&step.what, StepKind::Act { action, .. } if *action == research_id),
            )
            .collect();
        assert_eq!(steps.len(), 1, "the research runs once");
        assert_eq!(
            steps[0].end - steps[0].start,
            6000,
            "10 units at 600 ticks each must reach the schedule"
        );
    }

    /// The scenario the stack exists for, checked for the thing the game
    /// checks: no two entities the plan places may share ground.
    ///
    /// Before footprints, this plan sited stone furnaces at `[-34, -1]` and
    /// `[-34, 0]` — one tile apart, where a stone furnace is 1.398 tiles
    /// across. `mods/BotBridge/control.lua`'s `can_place_entity` refuses the
    /// second, and the executor's `abandon_rest` then drops that bot's whole
    /// remaining slice, so a single overlap costs a quarter of the run.
    ///
    /// Stated over collision boxes read from the prototypes, never over a
    /// clearance constant: `dx > 1.398` would pin the very number the fix must
    /// not hardcode, and would quietly stop meaning anything for any other
    /// entity. The overlap test below is written out rather than borrowed from
    /// `state.rs` so it is not the production predicate checking itself; it is
    /// deliberately stricter (no touch slack), which is safe here because
    /// placements sit on integer tiles and 1.398 is not an integer, so two
    /// furnaces can never come to rest exactly touching.
    #[test]
    fn nothing_the_red_science_plan_places_overlaps_anything_else_it_places() {
        use factorio_bot_core::types::Rect;

        fn intersect(a: &Rect, b: &Rect) -> bool {
            a.left_top.x() < b.right_bottom.x()
                && b.left_top.x() < a.right_bottom.x()
                && a.left_top.y() < b.right_bottom.y()
                && b.left_top.y() < a.right_bottom.y()
        }

        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        // `tests/red_science.rs`'s world, so this is the real headline plan and
        // not a scenario invented to be easy.
        let mut s = state(&bots);
        for bot in bots {
            s.gain(bot, "stone-furnace", 2);
        }
        s.set_position(BotId(2), Position::new(30., 0.));

        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 10,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("ten red science expands");

        let placed: Vec<(String, Position)> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } => {
                    Some((entity.name.clone(), entity.position.clone()))
                }
                _ => None,
            })
            .collect();
        assert!(
            placed.len() > 1,
            "this plan must place at least two entities or the pairwise check \
             below is vacuous; it placed {}",
            placed.len()
        );

        for (i, (name_a, pos_a)) in placed.iter().enumerate() {
            let box_a = s
                .collision_area(name_a, pos_a)
                .unwrap_or_else(|| panic!("no prototype for {name_a}, which was placed anyway"));
            for (name_b, pos_b) in placed.iter().skip(i + 1) {
                let box_b = s
                    .collision_area(name_b, pos_b)
                    .unwrap_or_else(|| panic!("no prototype for {name_b}"));
                assert!(
                    !intersect(&box_a, &box_b),
                    "{name_a} at {pos_a} and {name_b} at {pos_b} overlap: \
                     {box_a:?} against {box_b:?}"
                );
            }
        }
    }

    #[test]
    fn an_already_held_item_expands_to_nothing() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-ore", 10);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 5,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 0, "nothing to do");
    }

    #[test]
    fn mining_produces_one_action_that_yields_the_requested_count() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 5,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 1);
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Mine { item, count, .. } => {
                assert_eq!(item, "iron-ore");
                assert_eq!(*count, 5);
            }
            other => panic!("expected a mine action, got {:?}", other),
        }
        // One second of mining time per ore in the fixture, divided by the
        // character's 0.5 mining speed: two seconds, so 120 ticks each.
        assert_eq!(action.duration, 600);
    }

    #[test]
    fn mining_only_asks_for_what_is_missing() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-ore", 3);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 5,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Mine { count, .. } => assert_eq!(*count, 2, "only the shortfall"),
            other => panic!("expected a mine action, got {:?}", other),
        }
    }

    #[test]
    fn a_mine_action_carries_its_reach_and_resource_preconditions() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have {
                item: "coal".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert!(
            action
                .pre
                .iter()
                .any(|c| matches!(c, Condition::AtPosition { .. }))
        );
        assert!(action
            .pre
            .iter()
            .any(|c| matches!(c, Condition::ResourceAvailable { item, count, .. } if item == "coal" && *count == 2)));
        assert!(action.eff.iter().any(
            |e| matches!(e, Effect::GainItem { item, count, .. } if item == "coal" && *count == 2)
        ));
        assert!(
            action
                .eff
                .iter()
                .any(|e| matches!(e, Effect::ConsumeResource { .. }))
        );
    }

    #[test]
    fn emitted_actions_are_unpinned_and_use_the_role_actor() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have {
                item: "coal".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert_eq!(action.pinned, None, "methods must never pin");
        assert!(action.eff.iter().all(|e| match e {
            Effect::GainItem { who, .. } | Effect::LoseItem { who, .. } => *who == Actor::Role,
            _ => true,
        }));
    }

    #[test]
    fn a_mined_goal_schedules() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 4,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        assert_eq!(plan.steps.len(), 2, "a walk and the mine");
        assert!(
            plan.makespan > 240,
            "walking to the patch plus four seconds mining"
        );
    }

    #[test]
    fn an_unobtainable_item_has_no_method() {
        let s = state(&[BotId(1)]);
        let result = expand(
            &[Goal::Have {
                item: "uranium-ore".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        );
        assert!(matches!(
            result,
            Err(PlannerError::NoApplicableMethod { .. })
        ));
    }

    #[test]
    fn smelting_emits_place_insert_insert_remove() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let kinds: Vec<&str> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { .. } => "mine",
                ActionKind::Craft { .. } => "craft",
                ActionKind::Place { .. } => "place",
                ActionKind::Insert { .. } => "insert",
                ActionKind::Remove { .. } => "remove",
                ActionKind::Research { .. } => "research",
            })
            .collect();
        assert_eq!(kinds.iter().filter(|k| **k == "place").count(), 1);
        assert_eq!(
            kinds.iter().filter(|k| **k == "insert").count(),
            2,
            "ore and fuel"
        );
        assert_eq!(kinds.iter().filter(|k| **k == "remove").count(), 1);
        assert_eq!(
            kinds.iter().filter(|k| **k == "mine").count(),
            2,
            "iron ore and coal"
        );
    }

    #[test]
    fn the_removal_waits_for_the_smelting_time() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();

        let remove = net
            .actions()
            .find(|a| matches!(a.kind, ActionKind::Remove { .. }))
            .expect("a removal");

        // Identify each insert by what it inserts, then check its own edge to
        // the removal — a blind max() over all predecessors would pass even if
        // the ore and fuel lags were swapped.
        let lag_from = |item: &str| -> Ticks {
            let insert = net
                .actions()
                .find(|a| matches!(&a.kind, ActionKind::Insert { item: i, .. } if i == item))
                .unwrap_or_else(|| panic!("expected an insert of {}", item));
            net.preds(remove.id)
                .into_iter()
                .find(|(from, _)| *from == insert.id)
                .unwrap_or_else(|| {
                    panic!("expected an edge from the {} insert to the removal", item)
                })
                .1
        };

        // iron-plate is 3.2 s each, so two plates are 2 * 192, plus one cycle
        // of headroom for the furnace's start: 3 * 192 = 576.
        assert_eq!(
            lag_from("iron-ore"),
            576,
            "the ore insert carries the smelting time plus its start headroom"
        );
        // Fuel must be in before the removal, but does not itself take smelting time.
        assert_eq!(lag_from("coal"), 0, "the fuel insert carries no lag");
    }

    #[test]
    fn smelting_without_a_furnace_crafts_one_first() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert!(
            net.actions().any(
                |a| matches!(&a.kind, ActionKind::Craft { item, .. } if item == "stone-furnace")
            ),
            "the bot has no furnace, so it must make one"
        );
        assert!(
            net.actions()
                .any(|a| matches!(&a.kind, ActionKind::Mine { item, .. } if item == "stone")),
            "and mine the stone for it"
        );
    }

    #[test]
    fn a_smelted_goal_schedules() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 1);
        s.gain(BotId(2), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        assert!(plan.makespan > 384, "at least the smelting time");
    }

    #[test]
    fn hand_crafting_expands_its_ingredients() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-plate", 4);
        let net = expand(
            &[Goal::Have {
                item: "iron-gear-wheel".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(
            net.len(),
            1,
            "the plates are already held, so just the craft"
        );
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Craft { item, count } => {
                assert_eq!(item, "iron-gear-wheel");
                assert_eq!(*count, 2);
            }
            other => panic!("expected a craft, got {:?}", other),
        }
        // 0.5 s per gear, two gears.
        assert_eq!(action.duration, 60);
    }

    #[test]
    fn hand_crafting_consumes_its_ingredients() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-plate", 4);
        let net = expand(
            &[Goal::Have {
                item: "iron-gear-wheel".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert!(action.pre.iter().any(
            |c| matches!(c, Condition::HasItem { item, count, .. } if item == "iron-plate" && *count == 4)
        ));
        assert!(action.eff.iter().any(
            |e| matches!(e, Effect::LoseItem { item, count, .. } if item == "iron-plate" && *count == 4)
        ));
    }

    /// Two sub-goals of one craft drawing on the same intermediate.
    ///
    /// A lab needs 10 iron gear wheels *and* 4 transport belts, and a
    /// transport belt is itself made of gears — one gear per two belts. Sized
    /// against the inventory the expansion started with, the belt sub-goal
    /// sees the ten gears the lab's *own* gear sub-goal has just produced,
    /// calls itself supplied, and spends two of them; the lab craft is then
    /// left holding 8 where it needs 10.
    ///
    /// The count is asserted exactly. `>= 10` would also pass against a fix
    /// that simply over-crafts, which is a different bug wearing this one's
    /// clothes.
    #[test]
    fn a_shared_intermediate_is_crafted_for_every_sub_goal_that_draws_on_it() {
        let mut s = state(&[BotId(1)]);
        // Plates enough that nothing has to be mined or smelted: the question
        // here is how a craft is sized, and ore would only add noise.
        s.gain(BotId(1), "iron-plate", 200);
        s.gain(BotId(1), "copper-plate", 200);
        let net = expand(
            &[Goal::Have {
                item: "lab".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .expect("a lab is craftable from plates alone");

        let crafted = |item: &str| -> u32 {
            net.actions()
                .filter_map(|a| match &a.kind {
                    ActionKind::Craft {
                        item: crafted,
                        count,
                    } if crafted == item => Some(*count),
                    _ => None,
                })
                .sum()
        };
        let labels: Vec<&String> = net.actions().map(|a| &a.label).collect();
        assert_eq!(
            crafted("iron-gear-wheel"),
            12,
            "ten gears for the lab and two more for its four transport belts: {labels:?}"
        );
        assert_eq!(
            crafted("transport-belt"),
            2,
            "two runs of a recipe that yields two belts each: {labels:?}"
        );
    }

    #[test]
    fn the_whole_science_chain_expands() {
        /// The one action whose kind matches, or a panic naming what was
        /// wanted. Uniqueness matters: `preds` membership means nothing if
        /// there are three candidates and the test picked whichever came first.
        fn only(
            net: &crate::network::ActionNetwork,
            what: &str,
            matching: impl Fn(&ActionKind) -> bool,
        ) -> crate::ids::ActionId {
            let hits: Vec<&Action> = net.actions().filter(|a| matching(&a.kind)).collect();
            match hits.as_slice() {
                [one] => one.id,
                other => panic!(
                    "wanted exactly one {}, found {:?} among {:?}",
                    what,
                    other.iter().map(|a| &a.label).collect::<Vec<_>>(),
                    net.actions().map(|a| &a.label).collect::<Vec<_>>()
                ),
            }
        }

        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();

        // Both ores are dug.
        only(
            &net,
            "iron-ore mine",
            |k| matches!(k, ActionKind::Mine { item, .. } if item == "iron-ore"),
        );
        only(
            &net,
            "copper-ore mine",
            |k| matches!(k, ActionKind::Mine { item, .. } if item == "copper-ore"),
        );

        // The shape, not the spelling. Label substrings are blind to exactly
        // the defect this chain keeps hitting: actions that exist but are not
        // ordered against each other, or are ordered against the wrong thing.
        let gear = only(
            &net,
            "gear craft",
            |k| matches!(k, ActionKind::Craft { item, .. } if item == "iron-gear-wheel"),
        );
        let pack = only(
            &net,
            "science craft",
            |k| matches!(k, ActionKind::Craft { item, .. } if item == "automation-science-pack"),
        );
        let iron_out = only(
            &net,
            "iron-plate removal",
            |k| matches!(k, ActionKind::Remove { item, .. } if item == "iron-plate"),
        );
        let copper_out = only(
            &net,
            "copper-plate removal",
            |k| matches!(k, ActionKind::Remove { item, .. } if item == "copper-plate"),
        );

        let preds = |id| -> Vec<crate::ids::ActionId> {
            net.preds(id).into_iter().map(|(p, _)| p).collect()
        };
        assert!(
            preds(gear).contains(&iron_out),
            "the gear craft must wait for its plates to come out of the furnace: {:?}",
            preds(gear)
        );
        assert!(
            preds(pack).contains(&gear),
            "the science craft must wait for the gear: {:?}",
            preds(pack)
        );
        assert!(
            preds(pack).contains(&copper_out),
            "and for the copper plate: {:?}",
            preds(pack)
        );
    }

    #[test]
    fn the_science_chain_schedules_without_a_precondition_failure() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 2);
        s.gain(BotId(2), "stone-furnace", 2);
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("the chain must be schedulable");
        assert_eq!(
            plan.steps
                .iter()
                .filter(|s| matches!(s.what, crate::schedule::StepKind::Act { .. }))
                .count(),
            net.len(),
            "every action is scheduled"
        );
    }

    #[test]
    fn a_shared_goal_splits_into_one_chain_per_bot() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "iron-ore", 0);
        }
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 8,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 4, "one mine per bot");
        for action in net.actions() {
            match &action.kind {
                ActionKind::Mine { count, .. } => assert_eq!(*count, 2, "8 split four ways"),
                other => panic!("expected mines, got {:?}", other),
            }
        }
    }

    #[test]
    fn an_uneven_split_distributes_the_remainder() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 10,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let mut counts: Vec<u32> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected mines, got {:?}", other),
            })
            .collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![2, 2, 3, 3], "10 across four bots");
        assert_eq!(counts.iter().sum::<u32>(), 10);
    }

    #[test]
    fn a_count_smaller_than_the_roster_uses_only_as_many_chains_as_needed() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 2, "two chains for two units");
    }

    /// Two top-level splits of the same item, over four *identical* bots, must
    /// not double-count what the first split already promised. Each `Have`
    /// asks for 8, so the roster shortfall is 16 — but the second split's
    /// per-bot target used to read `inventory_count`, which does not see the
    /// first split's reservations, so it re-asked for `share + reserved`
    /// instead of `share` and the roster mined 24.
    #[test]
    fn two_top_level_splits_do_not_double_count_what_the_first_produced() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let net = expand(
            &[Goal::All(vec![
                Goal::Have {
                    item: "iron-ore".into(),
                    count: 8,
                    whose: Holder::Anyone,
                },
                Goal::Have {
                    item: "iron-ore".into(),
                    count: 8,
                    whose: Holder::Anyone,
                },
            ])],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let mined: u32 = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected mines, got {:?}", other),
            })
            .sum();
        assert_eq!(mined, 16, "two shortfalls of 8 sum to 16, not 24");
    }

    /// A smelting recipe the fixture does not ship: steel plate, 16 s a run
    /// against iron plate's 3.2 s. Deserialised rather than built, because
    /// `FactorioRecipe::energy` is a `noisy_float` this crate does not depend
    /// on directly.
    fn state_knowing_steel() -> PlanState {
        state_knowing_steel_at_furnace_speed(None)
    }

    /// `state_knowing_steel`, with the stone furnace's crafting speed
    /// optionally overridden — the only way to ask what `Smelt` would do with
    /// a faster machine, since it places a `stone-furnace` and only that.
    fn state_knowing_steel_at_furnace_speed(speed: Option<f64>) -> PlanState {
        use factorio_bot_core::serde_json;
        use factorio_bot_core::types::FactorioRecipe;
        let steel: FactorioRecipe = serde_json::from_str(
            r#"{
                "name": "steel-plate",
                "valid": true,
                "enabled": true,
                "category": "smelting",
                "ingredients": [
                    { "name": "iron-plate", "ingredient_type": "item", "amount": 5 }
                ],
                "products": [
                    { "name": "steel-plate", "product_type": "item",
                      "amount": 1, "probability": 1.0 }
                ],
                "hidden": false,
                "energy": 16.0,
                "order": "c[steel-plate]",
                "group": "intermediate-products",
                "subgroup": "raw-material"
            }"#,
        )
        .expect("the steel recipe parses");
        let world = fixture_world();
        world.update_recipes(vec![steel]).expect("recipes update");
        if let Some(speed) = speed {
            let mut furnace = world
                .entity_prototypes
                .get("stone-furnace")
                .expect("the fixture ships a stone furnace")
                .clone();
            furnace.crafting_speed = Some(speed);
            world
                .entity_prototypes
                .insert("stone-furnace".into(), furnace);
        }
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// The coal a `Smelt` asks for, given a goal.
    fn fuel_for(state: &PlanState, item: &str, count: u32) -> u32 {
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let steps = Smelt
            .expand(
                &Goal::Have {
                    item: item.into(),
                    count,
                    whose: Holder::Anyone,
                },
                &mut ctx,
            )
            .expect("smelting expands");
        steps
            .iter()
            .find_map(|step| match step {
                Step::Subgoal(Goal::Have { item, count, .. }) if item == "coal" => Some(*count),
                _ => None,
            })
            .expect("a fuel subgoal")
    }

    /// The furnace lag a `Smelt` emits, given a goal.
    ///
    /// The lag lives on the `Link` edges between the ore inserts and the
    /// removal, not on any action's duration, so it has to be read off the
    /// steps rather than off a schedule. The fuel edge carries 0 by
    /// construction; the largest is the smelt.
    fn smelt_lag_for(state: &PlanState, item: &str, count: u32) -> Ticks {
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let steps = Smelt
            .expand(
                &Goal::Have {
                    item: item.into(),
                    count,
                    whose: Holder::Anyone,
                },
                &mut ctx,
            )
            .expect("smelting expands");
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Link { lag, .. } => Some(*lag),
                _ => None,
            })
            .max()
            .expect("a smelt emits lag edges")
    }

    /// `fixture_world()` with the stone furnace's crafting speed overridden.
    ///
    /// `Smelt` places a `stone-furnace` and only ever that, so overriding
    /// *that* prototype is the only way to ask what the method would do with a
    /// faster machine without first inventing furnace adoption.
    fn state_with_furnace_speed(speed: f64) -> PlanState {
        let world = fixture_world();
        let mut furnace = world
            .entity_prototypes
            .get("stone-furnace")
            .expect("the fixture ships a stone furnace")
            .clone();
        furnace.crafting_speed = Some(speed);
        world
            .entity_prototypes
            .insert("stone-furnace".into(), furnace);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    #[test]
    fn the_lag_carries_one_cycle_of_headroom_over_the_smelting_time() {
        // A removal placed at exactly the predicted completion is right half
        // the time by construction. Live, one at insert+1924 against a
        // modelled 1920 came back with nine plates of ten, and the run spent
        // its remaining iteration budget replanning around the missing one.
        let s = state_with_furnace_speed(1.0);
        let bare = 10 * 192;
        assert!(
            smelt_lag_for(&s, "iron-plate", 10) > bare,
            "the lag is a schedule constraint, so it must be an upper bound \
             rather than a point estimate"
        );
        assert_eq!(smelt_lag_for(&s, "iron-plate", 10) - bare, 192);
    }

    #[test]
    fn the_furnace_lag_divides_by_the_furnaces_crafting_speed() {
        // Ten iron plates at 3.2 s each is 192 ticks per run in a stone
        // furnace (speed 1) and 96 in a steel or electric one (speed 2).
        //
        // This is the delayed fuse the change is really about: `Smelt` places
        // a stone furnace unconditionally today, so the live plan cannot
        // reach the second row. It is asserted through the method rather than
        // through `smelting_ticks` alone so that the day someone teaches
        // `Smelt` to use a better furnace, the wiring is already proved.
        // Eleven cycles, not ten: the lag carries one cycle of headroom for a
        // start that misses the current craft boundary. The speed divisor is
        // what this test is about, and it still halves both figures.
        assert_eq!(
            smelt_lag_for(&state_with_furnace_speed(1.0), "iron-plate", 10),
            11 * 192
        );
        assert_eq!(
            smelt_lag_for(&state_with_furnace_speed(2.0), "iron-plate", 10),
            11 * 96,
            "a furnace at speed 2 smelts the same ten plates in half the time"
        );
    }

    #[test]
    fn a_faster_furnace_does_not_change_the_coal_bill() {
        // The coupling check, and the reason the coal keeps using
        // `recipe_ticks` while the lag uses `smelting_ticks`.
        //
        // Coal is a quantity of energy, not of elapsed time. A duration fix
        // that also moved the fuel would mean the two are joined somewhere
        // they should not be — the plan would be claiming a furnace needs
        // less coal *because it finished sooner*, which is not how burning
        // works. Ten iron plates need one coal at speed 1 and must still need
        // one at speed 4, even though the lag drops fourfold.
        for speed in [1.0, 2.0, 4.0] {
            let state = state_with_furnace_speed(speed);
            assert_eq!(
                fuel_for(&state, "iron-plate", 10),
                1,
                "crafting speed {speed} must not move the coal bill"
            );
        }
        // Same on a recipe whose fuel bill is not pinned at the one-coal
        // floor, where a coupled implementation would actually be visible:
        // ten steel plates burn four coal, and four is not the floor. Under
        // the coupling this test forbids, speed 4 would ask for one.
        let steel_speeds: Vec<(f64, u32)> = [1.0, 2.0, 4.0]
            .into_iter()
            .map(|speed| {
                let state = state_knowing_steel_at_furnace_speed(Some(speed));
                (speed, fuel_for(&state, "steel-plate", 10))
            })
            .collect();
        assert_eq!(
            steel_speeds,
            vec![(1.0, 4), (2.0, 4), (4.0, 4)],
            "the coal for ten steel plates is fixed by their energy, not by \
             how quickly the furnace gets through it"
        );
    }

    #[test]
    fn fuel_scales_with_the_recipes_smelting_time() {
        let state = state_knowing_steel();
        // Ten runs either way. Iron plate burns 10 x 192 = 1920 ticks, inside
        // one coal's 2666; steel burns 10 x 960 = 9600, which is four.
        assert_eq!(fuel_for(&state, "iron-plate", 10), 1);
        assert_eq!(
            fuel_for(&state, "steel-plate", 10),
            4,
            "a flat plates-per-coal figure would say one, and the furnace \
             would go out a quarter of the way through"
        );
    }

    #[test]
    fn one_coal_is_the_floor_however_little_is_smelted() {
        let state = state(&[BotId(1)]);
        assert_eq!(fuel_for(&state, "iron-plate", 1), 1);
    }

    #[test]
    fn a_single_unit_goal_becomes_one_action_not_a_split() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 1, "a shortfall of one is one share, not several");
        // A share states that the holding ends up in one inventory, so it is
        // chained however trivial its subtree — but it names nobody, so the
        // chain has no owner and the scheduler still picks the runner.
        let only = net.actions().next().expect("one action");
        let chain = net
            .chain_of(only.id)
            .expect("a share is welded to one runner");
        assert_eq!(net.owner_of(chain), None, "a share commits nobody");
    }

    #[test]
    fn a_single_pack_expands_and_schedules_on_a_roster_of_four() {
        // The headline goal at its smallest. A shortfall of one is no split,
        // but it must still open a chain: without one, `HandCraft` propagates
        // `Holder::Anyone` into its ingredient subgoals, the first of those
        // with a shortfall above one is scattered instead, and the craft that
        // needs both ingredients in one inventory has nowhere to run.
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for bot in bots {
            s.gain(bot, "stone-furnace", 2);
        }
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("one pack across four bots must expand");
        assert!(net.len() > 10, "a real chain: {} actions", net.len());

        let chains: std::collections::BTreeSet<Option<crate::ids::ChainId>> =
            net.actions().map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains.len(), 1, "one chain, not several: {:?}", chains);
        assert!(
            chains.iter().next().expect("one entry").is_some(),
            "and a chain it is, not the chainless free-for-all"
        );

        schedule(&net, &s, &bots).expect("one pack across four bots must schedule");
    }

    #[test]
    fn an_intermediate_goal_is_never_split() {
        // A gear needs two plates, asked for as `Holder::Anyone` because
        // `HandCraft` propagates `whose` verbatim. Splitting *that* goal hands
        // each bot one plate for a craft that needs both, so the ore behind it
        // must be mined in one place, not two.
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-gear-wheel".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("one gear for two bots must expand");
        let ore: Vec<u32> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Mine { item, count, .. } if item == "iron-ore" => Some(*count),
                _ => None,
            })
            .collect();
        assert_eq!(
            ore,
            vec![2],
            "both plates' worth of ore is mined by one bot, in one action"
        );
    }

    #[test]
    fn a_share_is_added_to_what_the_bot_already_holds() {
        // Every bot holds one of the six wanted, so two remain. A `Have` goal
        // states a holding rather than a delivery: asking a bot for "one" when
        // it already holds one would be a goal it already meets, and its share
        // would evaporate. The holdings are equal across the roster here only
        // because equal holdings are the simplest case to read at a glance:
        // each share is in fact sized against *its own* bot's real spare
        // stock, so an asymmetric fixture works too -- see
        // `the_remainder_of_a_split_goes_to_the_bots_holding_least` and
        // `a_share_skips_the_bots_that_already_hold_the_item` for holdings
        // that differ across the roster.
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for bot in bots {
            s.gain(bot, "iron-ore", 1);
        }
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 6,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let mined: Vec<u32> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected mines, got {:?}", other),
            })
            .collect();
        assert_eq!(mined, vec![1, 1], "the two missing units, one per chain");
    }

    #[test]
    fn a_split_emits_shares_not_bot_instructions() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        // Reach into the method directly: the driver rewrites nothing, so what
        // SplitAcrossBots emits is what the rest of the plan sees.
        let split = SplitAcrossBots {
            bots: bots.to_vec(),
        };
        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let goal = Goal::Have {
            item: "iron-ore".into(),
            count: 4,
            whose: Holder::Anyone,
        };
        let steps = split.expand(&goal, &mut ctx).unwrap();
        for step in steps {
            match step {
                Step::Subgoal(Goal::Have { whose, .. }) => {
                    assert!(
                        matches!(whose, Holder::Share(_)),
                        "a split emits shares, not instructions: {:?}",
                        whose
                    );
                }
                other => panic!("expected only subgoals, got {:?}", other),
            }
        }
    }

    #[test]
    fn a_roster_listing_a_bot_twice_still_splits_the_whole_shortfall() {
        // `registry_for` copies the caller's slice verbatim, so a caller can
        // hand the same bot twice. Sizing chains against the slice's length
        // rather than the distinct bots would open two chains for bot 1, the
        // second sized after the first had already reserved its share, and
        // over-ask the roster. The participant set must be deduped before
        // `chains`/`base`/`remainder` are computed from it.
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let split = SplitAcrossBots {
            bots: vec![BotId(1), BotId(1), BotId(2)],
        };
        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let goal = Goal::Have {
            item: "iron-ore".into(),
            count: 4,
            whose: Holder::Anyone,
        };
        let steps = split.expand(&goal, &mut ctx).unwrap();
        assert_eq!(steps.len(), 2, "one subgoal per distinct bot, not per slot");
        let mut total = 0u32;
        let mut seen = BTreeSet::new();
        for step in steps {
            let Step::Subgoal(Goal::Have { count, whose, .. }) = step else {
                panic!("expected only subgoals");
            };
            let Holder::Share(bot) = whose else {
                panic!("expected a share, got {:?}", whose);
            };
            assert!(seen.insert(bot), "bot {} named twice", bot);
            total += count;
        }
        assert_eq!(total, 4, "the shares still sum to the whole shortfall");
    }

    #[test]
    fn the_remainder_of_a_split_goes_to_the_bots_holding_least() {
        // Four bots, bot 1 already holding 3 of the 10 wanted. `need = 7`,
        // `k = 4`, `base = 1`, `rem = 3`. Sorted by `(spare, BotId)` the
        // order is `(0,2),(0,3),(0,4),(3,1)`, so the three units of remainder
        // go to the three *poorest* bots -- bots 2, 3 and 4 -- not to
        // whichever bots sit first in the roster. Bot 1 gets only the base
        // share of 1. Sum: 2+2+2+1 = 7.
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-ore", 3);
        let split = SplitAcrossBots {
            bots: bots.to_vec(),
        };
        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let goal = Goal::Have {
            item: "iron-ore".into(),
            count: 10,
            whose: Holder::Anyone,
        };
        let steps = split.expand(&goal, &mut ctx).unwrap();
        let mut targets: BTreeMap<BotId, u32> = BTreeMap::new();
        for step in steps {
            let Step::Subgoal(Goal::Have { count, whose, .. }) = step else {
                panic!("expected only subgoals");
            };
            let Holder::Share(bot) = whose else {
                panic!("expected a share, got {:?}", whose);
            };
            targets.insert(bot, count);
        }
        // `available` for the split item equals raw holding here: no method
        // has reserved anything yet. Targets are `spare + work`, so bot 1's
        // work is `target - 3` and every other bot's work is its target
        // outright.
        assert_eq!(
            targets.get(&BotId(1)).map(|t| t - 3),
            Some(1),
            "the richest bot gets the least work"
        );
        for bot in [BotId(2), BotId(3), BotId(4)] {
            assert_eq!(
                targets.get(&bot).copied(),
                Some(2),
                "the poorest bots carry the remainder"
            );
        }
    }

    #[test]
    fn a_share_skips_the_bots_that_already_hold_the_item() {
        // The worked example from the design: four bots, bot 1 holding 8 of
        // the 10 wanted. `need = 2`, `k = min(4, 2) = 2`. Sorted by
        // `(spare, BotId)`: `(0,2), (0,3), (0,4), (8,1)`. Only the two
        // poorest -- bots 2 and 3 -- participate; bot 1 is asked for
        // nothing, and does not smelt a plate it does not need to.
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-plate", 8);
        let split = SplitAcrossBots {
            bots: bots.to_vec(),
        };
        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let goal = Goal::Have {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Anyone,
        };
        let steps = split.expand(&goal, &mut ctx).unwrap();
        let mut targets: BTreeMap<BotId, u32> = BTreeMap::new();
        for step in steps {
            let Step::Subgoal(Goal::Have { count, whose, .. }) = step else {
                panic!("expected only subgoals");
            };
            let Holder::Share(bot) = whose else {
                panic!("expected a share, got {:?}", whose);
            };
            targets.insert(bot, count);
        }
        assert_eq!(
            targets,
            BTreeMap::from([(BotId(2), 1), (BotId(3), 1)]),
            "bots 2 and 3 each get a target of 1 (spare 0 + work 1); \
             bots 1 and 4 are not asked for anything"
        );
    }

    #[test]
    fn the_split_does_not_depend_on_the_order_the_roster_was_listed_in() {
        // Bot 1 alone starts with a head start on the goal's chain -- the
        // freeplay-style inventory used elsewhere in this suite -- while the
        // split item itself, automation-science-pack, is zero for every bot.
        // The interchangeable-bots guard (still active; removing it is a
        // later step) only compares the goal's own item across the roster,
        // so this asymmetric roster is not refused by it either way.
        //
        // Before this rule, participants were chosen by position in the
        // caller's slice (`self.bots.iter().take(chains)`), so reversing the
        // roster could change who is asked to produce and therefore the
        // shape of the expansion. The new rule orders candidates by
        // `(spare, BotId)`, which depends only on the set of bots and their
        // holdings, never on the order the caller listed them in.
        fn asymmetric_state(bots: &[BotId]) -> PlanState {
            let mut s = state(bots);
            s.gain(BotId(1), "iron-plate", 8);
            s.gain(BotId(1), "stone-furnace", 1);
            s.gain(BotId(1), "burner-mining-drill", 1);
            s.gain(BotId(1), "wood", 1);
            s
        }
        let goal = Goal::Have {
            item: "automation-science-pack".into(),
            count: 10,
            whose: Holder::Anyone,
        };

        let forward = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let forward_state = asymmetric_state(&forward);
        let forward_net = expand(
            std::slice::from_ref(&goal),
            &forward_state,
            &registry_for(&forward),
            BotId(1),
        )
        .expect("expands with the roster listed forward");

        let reverse = [BotId(4), BotId(3), BotId(2), BotId(1)];
        let reverse_state = asymmetric_state(&reverse);
        let reverse_net = expand(&[goal], &reverse_state, &registry_for(&reverse), BotId(1))
            .expect("expands with the roster listed in reverse");

        let forward_labels: Vec<String> = forward_net.actions().map(|a| a.label.clone()).collect();
        let reverse_labels: Vec<String> = reverse_net.actions().map(|a| a.label.clone()).collect();
        assert_eq!(
            forward_labels, reverse_labels,
            "the split must depend on the set of bots and their holdings, \
             not the order the caller listed them in"
        );
    }

    #[test]
    fn hand_crafting_converges_only_when_two_ingredients_need_producing() {
        let mut s = state(&[BotId(1)]);
        let asp = Goal::Have {
            item: "automation-science-pack".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        // Both copper-plate and iron-gear-wheel must be produced: they have to
        // meet in one inventory, so this converges.
        assert!(HandCraft.converges(&asp, &s));

        // With the copper already held, only the gear needs producing, so
        // nothing has to meet anything.
        s.gain(BotId(1), "copper-plate", 5);
        assert!(!HandCraft.converges(&asp, &s));

        // A single-ingredient recipe never converges.
        let gear = Goal::Have {
            item: "iron-gear-wheel".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert!(!HandCraft.converges(&gear, &s));
    }

    /// `Smelt` still does not *converge*, and the distinction is worth keeping.
    ///
    /// A furnace is fed by three separate actions — place, insert ore, insert
    /// coal — and no one of them needs two produced items together, so three
    /// bots really could each supply one input. `converges` asks exactly that
    /// question and the honest answer here is no.
    ///
    /// A smelt is nevertheless welded to one runner, because the goal above it
    /// is a `Holder::Share` and a share states that the holding ends up in one
    /// inventory. That is a different reason, enforced in the driver rather
    /// than here, and flipping this to `true` to get the same effect would put
    /// a false claim about a furnace in the place where the claim is read.
    #[test]
    fn smelting_never_converges() {
        let s = state(&[BotId(1)]);
        let plate = Goal::Have {
            item: "iron-plate".into(),
            count: 2,
            whose: Holder::Anyone,
        };
        assert!(!Smelt.converges(&plate, &s));
    }

    /// One smelt is one runner's work; several smelts are still several bots'.
    ///
    /// The first half is the fix for the scattering defect — a smelt's ore,
    /// coal and furnace are a share, and a share is one inventory. The second
    /// half is the bound on it: welding *within* a smelt must not weld the
    /// roster, or the planner has bought correctness with the only thing it
    /// exists for.
    #[test]
    fn a_smelt_is_one_runners_work_but_the_roster_still_splits() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "stone-furnace", 1);
        }
        let one = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let chains: std::collections::BTreeSet<_> =
            one.actions().map(|a| one.chain_of(a.id)).collect();
        assert_eq!(
            chains.len(),
            1,
            "one smelt is one chain, so its ore, its coal and its furnace \
             cannot land on three bots: {chains:?}"
        );
        assert!(
            chains.iter().all(Option::is_some),
            "and that chain is a real one: {chains:?}"
        );

        // Four plates over four bots is four independent shares, and they must
        // still spread.
        let four = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 4,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&four, &s, &bots).expect("schedulable");
        let used: std::collections::BTreeSet<_> = plan.steps.iter().map(|s| s.bot).collect();
        assert!(
            used.len() > 1,
            "four independent smelts must not serialise onto one bot, got {used:?}"
        );
    }

    #[test]
    fn a_converging_goal_gets_one_chain_over_its_whole_subtree() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "stone-furnace", 2);
        }
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let chains: std::collections::BTreeSet<_> =
            net.actions().filter_map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains.len(), 1, "one convergence point, one chain");
        assert!(
            net.actions().all(|a| net.chain_of(a.id).is_some()),
            "the ingredients must be welded to the craft that consumes them"
        );
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        let used: std::collections::BTreeSet<_> = plan.steps.iter().map(|s| s.bot).collect();
        assert_eq!(used.len(), 1, "a chain runs on one bot");
    }

    #[test]
    fn a_request_larger_than_one_tile_mines_several() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 1200,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let takes: Vec<(Position, u32)> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { pos, count, .. } => (pos.clone(), *count),
                other => panic!("expected only mines, got {:?}", other),
            })
            .collect();
        let mined: u32 = takes.iter().map(|(_, count)| count).sum();
        assert_eq!(mined, 1200);
        assert_eq!(takes.len(), 3, "500 + 500 + 200");
        // Each action must draw from a *different* tile. Two actions on one
        // tile would sum to more than it holds, so the totals above would
        // still look right while `ResourceAvailable` failed at schedule time.
        let tiles: std::collections::BTreeSet<factorio_bot_core::types::Pos> = takes
            .iter()
            .map(|(pos, _)| factorio_bot_core::types::Pos::from(pos))
            .collect();
        assert_eq!(tiles.len(), 3, "three distinct tiles, got {:?}", takes);
    }

    #[test]
    fn a_furnace_is_sited_near_the_ore_it_smelts() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 2,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();

        let furnace = net
            .actions()
            .find_map(|a| match &a.kind {
                ActionKind::Place { entity } => Some(entity.position.clone()),
                _ => None,
            })
            .expect("a placement");
        let ore = net
            .actions()
            .find_map(|a| match &a.kind {
                ActionKind::Mine { pos, item, .. } if item == "iron-ore" => Some(pos.clone()),
                _ => None,
            })
            .expect("an iron-ore mine");

        let to_ore = calculate_distance(&furnace, &ore);
        let to_origin = calculate_distance(&furnace, &Position::new(0., 0.));
        assert!(
            to_ore < to_origin,
            "the furnace should sit by the ore ({} away) not the bot's start ({} away)",
            to_ore,
            to_origin
        );
        // `to_ore < to_origin` alone passes for an anchor anywhere in the half
        // of the map nearer the ore than the origin, which is most of it. The
        // siting is `free_area_near` from the ore tile itself, and that
        // searches at most 12 tiles out, so 20 catches the regression this
        // guards: the anchor slipping back towards the bot's start, 49.5 away.
        //
        // It is structural, not tight. The measured `to_ore` here is
        // **sqrt(2) ~= 1.414** — the ring search leaves the ore tile it starts
        // on, because `is_position_free` now counts ore as occupying its tile,
        // and settles on the first free diagonal neighbour — and no
        // ore-anchored siting can exceed ~17. So this bound discriminates
        // ore-anchored from origin-anchored and nothing finer.
        assert!(
            to_ore < 20.,
            "the furnace must be within reach of the ore, not merely nearer it: {}",
            to_ore
        );
        // Zero would mean the furnace sits on the ore tile, which the game
        // refuses to build on however well the plan's arithmetic works out.
        assert!(to_ore > 0., "the furnace was sited on the ore tile itself");
    }

    #[test]
    fn a_caller_naming_a_bot_gets_that_bot() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        // Park bot 2 far away, so the scheduler would otherwise never choose it.
        s.set_position(BotId(2), Position::new(300., 0.));
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 3,
                whose: Holder::Bot(BotId(2)),
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        for step in &plan.steps {
            assert_eq!(step.bot, BotId(2), "the caller named bot 2");
        }
    }

    // ---------------------------------------------------------------
    // Locked recipes.
    //
    // The shared fixture marks every recipe `enabled`, which is precisely how
    // the live defect went unseen: `world.recipe("automation-science-pack")`
    // returned nil against a real 2.1.17 game because the mod only serialised
    // recipes enabled for the force, and the goal failed with "no method can
    // satisfy" while every unit test stayed green. These build a world where a
    // recipe really is locked.
    // ---------------------------------------------------------------

    fn locked_state(recipe: &str, unlockers: &[&str], bots: &[BotId]) -> PlanState {
        PlanState::from_world(
            Arc::new(crate::test_world::world_with_locked_recipe(
                recipe, unlockers,
            )),
            bots,
        )
    }

    /// The premise: the fixture really does present a disabled recipe.
    #[test]
    fn the_locked_fixture_disables_the_recipe_it_names() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let recipe = recipe_for(&s, "automation-science-pack").expect("still present, just off");
        assert!(!recipe.enabled, "the fixture must disable it");
        assert_eq!(
            recipe.category, "crafting",
            "and must not otherwise disturb it"
        );
    }

    /// A disabled recipe with a known unlocker is craftable *after* research.
    #[test]
    fn a_locked_recipe_gates_on_its_unlocking_technology() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let recipe = recipe_for(&s, "automation-science-pack").unwrap();
        assert_eq!(
            recipe_gate(&s, &recipe),
            RecipeGate::NeedsResearch("asp-tech".into())
        );
    }

    /// An enabled recipe needs no research, and asks no technology table any
    /// questions.
    #[test]
    fn an_enabled_recipe_is_open() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let gear = recipe_for(&s, "iron-gear-wheel").expect("untouched by the fixture");
        assert_eq!(recipe_gate(&s, &gear), RecipeGate::Open);
    }

    /// Disabled with nothing to turn it on. Live 2.1.17 really has eight of
    /// these (`loader`, `pistol`, `infinity-chest`, ...), and planning a craft
    /// for one would be planning something the game refuses to run.
    #[test]
    fn a_locked_recipe_no_technology_unlocks_is_unobtainable() {
        let s = locked_state("automation-science-pack", &[], &[BotId(1)]);
        let recipe = recipe_for(&s, "automation-science-pack").unwrap();
        assert_eq!(recipe_gate(&s, &recipe), RecipeGate::Unobtainable);
    }

    /// ...and `HandCraft` declines it, rather than emitting a craft that cannot
    /// run. Declining is what lets the goal come back as "no method can
    /// satisfy", which is the honest answer.
    #[test]
    fn hand_craft_declines_an_unobtainable_recipe() {
        let s = locked_state("automation-science-pack", &[], &[BotId(1)]);
        let goal = Goal::Have {
            item: "automation-science-pack".into(),
            count: 1,
            whose: Holder::Share(BotId(1)),
        };
        assert!(
            !HandCraft.applicable(&goal, &s),
            "nothing can ever unlock this recipe"
        );
    }

    /// The same goal *is* claimed when a technology can unlock it — otherwise
    /// the test above would pass against a method that declined everything.
    #[test]
    fn hand_craft_claims_a_locked_recipe_that_can_be_unlocked() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let goal = Goal::Have {
            item: "automation-science-pack".into(),
            count: 1,
            whose: Holder::Share(BotId(1)),
        };
        assert!(HandCraft.applicable(&goal, &s));
    }

    /// Crafting through a locked recipe emits the research as a subgoal *and*
    /// states it as a precondition. The subgoal is what gets the technology
    /// researched; the precondition is what `infer_edges` turns into the edge
    /// that keeps the craft after it. One without the other is a plan that
    /// either never researches or researches too late.
    #[test]
    fn crafting_a_locked_recipe_emits_and_requires_the_research() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(s.fork(), BotId(1));
        let steps = HandCraft
            .expand(
                &Goal::Have {
                    item: "automation-science-pack".into(),
                    count: 1,
                    whose: Holder::Share(BotId(1)),
                },
                &mut ctx,
            )
            .expect("a locked recipe with an unlocker expands");

        assert!(
            subgoals(&steps).contains(&Goal::Researched("asp-tech".into())),
            "the research has to be asked for: {:?}",
            subgoals(&steps)
        );
        let craft = steps
            .iter()
            .find_map(|step| match step {
                Step::Act(action) if matches!(action.kind, ActionKind::Craft { .. }) => {
                    Some(action)
                }
                _ => None,
            })
            .expect("a craft action is emitted");
        assert!(
            craft
                .pre
                .contains(&Condition::Researched("asp-tech".into())),
            "the craft has to wait for it: {:?}",
            craft.pre
        );
    }

    /// An *enabled* recipe emits no research at all. Without this, a method
    /// that gated every craft on some technology would pass the test above.
    #[test]
    fn crafting_an_enabled_recipe_emits_no_research() {
        let s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(s.fork(), BotId(1));
        let steps = HandCraft
            .expand(
                &Goal::Have {
                    item: "iron-gear-wheel".into(),
                    count: 1,
                    whose: Holder::Share(BotId(1)),
                },
                &mut ctx,
            )
            .expect("the gear recipe is enabled");
        assert!(
            !subgoals(&steps)
                .iter()
                .any(|g| matches!(g, Goal::Researched(_))),
            "an unlocked recipe needs no research: {:?}",
            subgoals(&steps)
        );
    }

    /// A technology already researched leaves the recipe open and costs no
    /// second subgoal — which is what keeps the common case free, since
    /// `Researched` expands its prerequisites before its science packs.
    #[test]
    fn an_already_researched_unlocker_leaves_the_recipe_open() {
        let mut s = locked_state("automation-science-pack", &["asp-tech"], &[BotId(1)]);
        let recipe = recipe_for(&s, "automation-science-pack").unwrap();
        assert_eq!(
            recipe_gate(&s, &recipe),
            RecipeGate::NeedsResearch("asp-tech".into()),
            "baseline: locked before the research"
        );
        s.set_researched("asp-tech");
        assert_eq!(
            recipe_gate(&s, &recipe),
            RecipeGate::Open,
            "and open after it"
        );
    }

    /// Several technologies may unlock one recipe (live 2.1.17 has seven such
    /// recipes). They are alternatives, so choosing one is sound; choosing the
    /// *same* one every run is what planning determinism requires.
    #[test]
    fn several_unlockers_resolve_to_the_lexicographically_smallest() {
        let s = locked_state(
            "automation-science-pack",
            &["zeta-tech", "alpha-tech", "mid-tech"],
            &[BotId(1)],
        );
        assert_eq!(
            unlocking_technology(&s, "automation-science-pack"),
            Some("alpha-tech".into())
        );
    }

    /// ...unless one of them is already researched, in which case it wins
    /// whatever its name, because the world has already paid for it.
    #[test]
    fn an_already_researched_unlocker_beats_a_smaller_named_one() {
        let mut s = locked_state(
            "automation-science-pack",
            &["zeta-tech", "alpha-tech"],
            &[BotId(1)],
        );
        s.set_researched("zeta-tech");
        assert_eq!(
            unlocking_technology(&s, "automation-science-pack"),
            Some("zeta-tech".into()),
            "the free one, not the alphabetically first"
        );
    }

    /// End to end through the driver: the whole goal schedules, and the craft
    /// really is ordered after the research rather than merely mentioning it.
    #[test]
    fn a_locked_recipe_schedules_its_research_before_its_craft() {
        let bots = vec![BotId(1)];
        let s = locked_state("automation-science-pack", &["asp-tech"], &bots);
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Share(BotId(1)),
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a locked recipe with an unlocker plans");
        assert_eq!(researched_techs(&net), vec!["asp-tech".to_string()]);

        let plan = schedule(&net, &s, &bots).expect("schedulable");
        // A scheduled step names an `ActionId`, not a kind, so the kinds come
        // back from the network the schedule was built over.
        let kind_of = |id: ActionId| net.actions().find(|a| a.id == id).map(|a| a.kind.clone());
        let mut research_end = None;
        let mut craft_start = None;
        for step in &plan.steps {
            let StepKind::Act { action, .. } = &step.what else {
                continue;
            };
            match kind_of(*action) {
                Some(ActionKind::Research { tech }) if tech == "asp-tech" => {
                    research_end = Some(step.end);
                }
                Some(ActionKind::Craft { item, .. }) if item == "automation-science-pack" => {
                    craft_start = Some(step.start);
                }
                _ => {}
            }
        }
        let research_end = research_end.expect("the research is scheduled");
        let craft_start = craft_start.expect("the craft is scheduled");
        assert!(
            craft_start >= research_end,
            "craft starts at {craft_start}, research ends at {research_end}"
        );
    }

    // ---- research_trigger technologies -------------------------------------
    //
    // Factorio 2.0 technologies that complete when the player *does* something
    // rather than when a lab eats packs. `research_unit_ingredients` is empty
    // and `research_unit_energy` is zero for all of them, so before this the
    // planner costed them at nothing and produced a plan that was correctly
    // ordered and wrongly timed.

    fn trigger_state(
        tech: &str,
        trigger_json: &str,
        locked_recipe: Option<&str>,
        unlocked_by: Option<&str>,
    ) -> PlanState {
        PlanState::from_world(
            Arc::new(crate::test_world::world_with_trigger(
                tech,
                trigger_json,
                locked_recipe,
                unlocked_by,
            )),
            &[BotId(1)],
        )
    }

    /// The premise: the fixture really does present a technology whose pack
    /// bill is empty and whose trigger is set. Without this, the tests below
    /// could pass against a fixture that quietly grew a science cost.
    #[test]
    fn the_trigger_fixture_carries_a_trigger_and_no_pack_bill() {
        let s = trigger_state(
            "steam-power",
            r#"{"type": "craft-item", "item": "iron-plate", "count": 50}"#,
            None,
            None,
        );
        let tech = s.technology("steam-power").expect("the fixture defines it");
        assert!(
            tech.research_unit_ingredients.is_empty(),
            "a trigger technology consumes no packs"
        );
        assert_eq!(research_ticks(&tech), 0, "and takes no lab time");
        assert_eq!(
            tech.research_trigger,
            Some(ResearchTrigger::CraftItem {
                item: "iron-plate".into(),
                count: 50,
            })
        );
    }

    /// The fix. `steam-power` is really "craft 50 iron plates", and that work
    /// has to appear in the plan as a subgoal — the step list must *grow*.
    #[test]
    fn produced_ignores_what_a_bot_already_holds() {
        // The whole difference between the two goals. A bot carrying ten has
        // not *made* one, and a craft-item trigger fires on the making.
        let bots = [BotId(1)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-plate", 10);

        let held = Goal::Have {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Share(BotId(1)),
        };
        let made = Goal::Produced {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Share(BotId(1)),
            unlocks: None,
        };

        assert!(
            AlreadySatisfied.applicable(&held, &s),
            "ten in hand satisfies `Have`"
        );
        assert!(
            !AlreadySatisfied.applicable(&made, &s),
            "ten in hand must NOT satisfy `Produced` -- that is the bug this \
             goal exists to prevent, and `AlreadySatisfied` is registered ahead \
             of every producing method"
        );
        assert!(
            Smelt.applicable(&made, &s),
            "a producing method must still claim it"
        );
    }

    #[test]
    fn the_unlock_lands_on_the_action_that_produces_the_item() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let reg = registry_for(&bots);
        let net = expand(
            &[Goal::Produced {
                item: "iron-plate".into(),
                count: 1,
                whose: Holder::Share(BotId(1)),
                unlocks: Some("steam-power".into()),
            }],
            &s,
            &reg,
            BotId(1),
        )
        .expect("a smelted trigger item must plan");

        // The case the inlined-hand-craft attempt broke: iron-plate is smelted,
        // so only a goal that any producing method can claim reaches it.
        let carriers: Vec<&Action> = net
            .actions()
            .filter(|a| a.eff.contains(&Effect::Researched("steam-power".into())))
            .collect();
        assert_eq!(
            carriers.len(),
            1,
            "exactly one action carries the unlock, got {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );
        assert!(
            carriers[0].eff.iter().any(|e| matches!(
                e,
                Effect::GainItem { item, .. } if item == "iron-plate"
            )),
            "the unlock must ride on the action that produces the item, not a \
             separate marker, got {:?}",
            carriers[0]
        );
    }

    #[test]
    fn a_craft_item_trigger_becomes_the_subgoal_it_names() {
        let s = trigger_state(
            "steam-power",
            r#"{"type": "craft-item", "item": "iron-plate", "count": 50}"#,
            None,
            None,
        );
        let steps = research_steps(&s, "steam-power");
        assert_eq!(
            subgoals(&steps),
            vec![Goal::Produced {
                item: "iron-plate".into(),
                count: 50,
                whose: Holder::Share(BotId(1)),
                unlocks: Some("steam-power".into()),
            }],
            "the trigger's own work must be planned, and as a production -- \
             `Have` is satisfied by possession, so a bot already carrying fifty \
             would produce nothing and the trigger would never fire"
        );
    }

    /// The trigger watches a craft; it does not eat the result. So the research
    /// requires the items to exist and must *not* debit them, unlike the pack
    /// path which spends what it consumes.
    #[test]
    fn a_trigger_emits_no_research_action() {
        // The trigger firing *is* the research: the game does it. Issuing one
        // as well does nothing -- `add_research` refuses a trigger technology
        // outright -- and a run that did so planned `research electronics` five
        // times, was told success five times, and looped until the supervisor
        // called it `stuck_silent`.
        let s = trigger_state(
            "steam-power",
            r#"{"type": "craft-item", "item": "iron-plate", "count": 50}"#,
            None,
            None,
        );
        let steps = research_steps(&s, "steam-power");
        assert!(
            !steps.iter().any(|step| matches!(
                step,
                Step::Act(action) if matches!(action.kind, ActionKind::Research { .. })
            )),
            "a trigger technology must issue no research, got {steps:?}"
        );
        assert!(
            subgoals(&steps).iter().any(
                |g| matches!(g, Goal::Produced { unlocks: Some(t), .. } if t == "steam-power")
            ),
            "the unlock must ride on the production goal, got {:?}",
            subgoals(&steps)
        );
    }
    /// **The four-bot run of 2026-09-02, reduced to a test.**
    ///
    /// `run-1788300756-94802` gathered ore for two milestones and then raised
    /// `precondition has 50 iron-ore of action ActionId(8) does not hold for
    /// bot 2` on `goal.researched("automation")`. Everything here is taken
    /// from that run's `samples.jsonl` at the tick it died: three bots, the
    /// freeplay starting inventory, and the ore each had actually mined —
    /// **unequal**, which is the condition no fixture had until now and which
    /// the whole defect needed. The same goal planned fine against bots
    /// holding nothing, which is why every existing test passed.
    ///
    /// `automation` sits above `steam-power`, a `craft-item` trigger for fifty
    /// iron plates, so the plan carries a production subtree and a
    /// science-pack subtree at once. Before the driver read `whose` off
    /// `Produced` as well as `Have`, the production's `insert 50 iron-ore` was
    /// welded to nothing while the mining under it opened a chain of its own,
    /// and the two landed on different bots.
    #[test]
    fn the_live_four_bot_research_run_plans_and_schedules() {
        let bots = [BotId(2), BotId(3), BotId(4)];
        let mut s = PlanState::from_world(
            Arc::new(crate::test_world::world_with_trigger_prerequisite()),
            &bots,
        );
        for bot in bots {
            // `initiate_missing_players_with_default_inventory`, plus the eight
            // iron plates freeplay really starts a player with.
            s.gain(bot, "wood", 1);
            s.gain(bot, "stone-furnace", 1);
            s.gain(bot, "burner-mining-drill", 1);
            s.gain(bot, "iron-plate", 8);
        }
        // Milestones 1 and 2 of the run, as the samples recorded them.
        s.gain(BotId(2), "iron-ore", 8);
        s.gain(BotId(3), "iron-ore", 8);
        s.gain(BotId(4), "iron-ore", 4);
        s.gain(BotId(2), "copper-ore", 2);
        s.gain(BotId(3), "copper-ore", 5);
        s.gain(BotId(4), "copper-ore", 13);

        let net = expand(
            &[Goal::Researched("automation".into())],
            &s,
            &registry_for(&bots),
            BotId(2),
        )
        .expect("the goal expands");

        // The claim under the fix, stated on the network rather than inferred
        // from the schedule: the furnace load and the mining that supplies it
        // are one chain, so no assignment can separate them.
        let insert = net
            .actions()
            .find(|a| a.label == "insert 50 iron-ore")
            .expect("the trigger's fifty plates are smelted");
        let mine = net
            .actions()
            .find(|a| a.label == "mine 42 iron-ore")
            .expect("and the ore for them is mined");
        assert_eq!(
            net.chain_of(insert.id),
            net.chain_of(mine.id),
            "the ore and the furnace it goes into must be welded to one runner"
        );
        assert!(net.chain_of(insert.id).is_some(), "and to a real chain");

        schedule(&net, &s, &bots).expect("and the plan schedules on the roster it was made for");
    }

    #[test]
    fn an_absent_trigger_count_means_one_not_none() {
        let s = trigger_state(
            "automation-science-pack",
            r#"{"type": "craft-item", "item": "iron-plate"}"#,
            None,
            None,
        );
        let steps = research_steps(&s, "automation-science-pack");
        assert_eq!(
            subgoals(&steps),
            vec![Goal::Produced {
                item: "iron-plate".into(),
                count: 1,
                whose: Holder::Share(BotId(1)),
                unlocks: Some("automation-science-pack".into()),
            }]
        );
    }

    /// A trigger kind this planner cannot express as a goal must be *refused*,
    /// by name, rather than costed at nothing. Silently planning it as free is
    /// the defect being fixed, so the failure has to be louder than the bug.
    #[test]
    fn an_inexpressible_trigger_is_refused_by_name() {
        let s = trigger_state(
            "uranium-processing",
            r#"{"type": "mine-entity"}"#,
            None,
            None,
        );
        let mut ctx = ExpansionCtx::new(s.fork(), BotId(1));
        let err = Researched
            .expand(&Goal::Researched("uranium-processing".into()), &mut ctx)
            .expect_err("a mine-entity trigger cannot be expressed as a goal");
        assert!(
            matches!(
                &err,
                PlannerError::UnsupportedResearchTrigger { technology, trigger }
                    if technology == "uranium-processing" && trigger == "mine-entity"
            ),
            "expected an UnsupportedResearchTrigger naming the kind, got {err:?}"
        );
    }

    /// The cycle the shipped game really contains: `foundry` is triggered by
    /// crafting a foundry, and is the only technology that unlocks the foundry
    /// recipe. Expanding that naively recurses until `ExpansionTooDeep`, which
    /// tells a caller "a method is probably expanding into itself" — true, and
    /// useless. It has to be diagnosed where it is understood.
    #[test]
    fn a_self_unlocking_trigger_is_refused_rather_than_recursing() {
        let s = trigger_state(
            "foundry",
            r#"{"type": "craft-item", "item": "automation-science-pack", "count": 1}"#,
            Some("automation-science-pack"),
            None,
        );
        let mut ctx = ExpansionCtx::new(s.fork(), BotId(1));
        let err = Researched
            .expand(&Goal::Researched("foundry".into()), &mut ctx)
            .expect_err("a technology whose trigger only it can unlock is unreachable");
        assert!(
            matches!(
                &err,
                PlannerError::SelfUnlockingResearchTrigger { technology, item }
                    if technology == "foundry" && item == "automation-science-pack"
            ),
            "expected a SelfUnlockingResearchTrigger, got {err:?}"
        );
    }

    /// The guard above must not fire on the ordinary case: a trigger item whose
    /// recipe is unlocked by a *different* technology is fine, and is how
    /// `automation-science-pack` (craft a lab, unlocked by `electronics`)
    /// really works. Without this, refusing everything would pass the test
    /// above.
    #[test]
    fn a_trigger_item_unlocked_by_another_technology_still_expands() {
        // The trigger item's recipe is *locked* here, so the guard is actually
        // reached -- but a different technology unlocks it, so it must not
        // fire. With an enabled trigger item `recipe_gate` returns `Open` and
        // the guard is short-circuited, which asserts nothing about it.
        let s = trigger_state(
            "automation-science-pack",
            r#"{"type": "craft-item", "item": "automation-science-pack", "count": 1}"#,
            Some("automation-science-pack"),
            Some("asp-tech"),
        );
        let recipe = recipe_for(&s, "automation-science-pack").expect("still present, just off");
        assert_eq!(
            recipe_gate(&s, &recipe),
            RecipeGate::NeedsResearch("asp-tech".into()),
            "the fixture must lock the trigger item behind a *different* technology"
        );

        let steps = research_steps(&s, "automation-science-pack");
        assert_eq!(
            subgoals(&steps),
            vec![Goal::Produced {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Share(BotId(1)),
                unlocks: Some("automation-science-pack".into()),
            }],
            "another technology unlocks it, so this trigger is reachable"
        );
    }

    /// A pack-researched technology must be completely unaffected: it still
    /// bills packs, still spends them, and still takes lab time. This is the
    /// control for every test above.
    #[test]
    fn a_pack_researched_technology_is_untouched_by_the_trigger_path() {
        let s = tech_state(&[BotId(1)]);
        let tech = s.technology("automation").expect("the fixture defines it");
        assert_eq!(tech.research_trigger, None);
        let steps = research_steps(&s, "automation");
        let Some(Step::Act(action)) = steps.last() else {
            panic!("the last step must be the research action");
        };
        assert_eq!(action.duration, 6000, "10 units at 600 ticks each");
        assert!(action.eff.contains(&Effect::LoseItem {
            who: Actor::Role,
            item: "automation-science-pack".into(),
            count: 10,
        }));
    }
}
