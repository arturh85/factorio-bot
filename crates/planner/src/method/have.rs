//! Methods that satisfy `Goal::Have`.
//!
//! Every method here emits actions with `Actor::Role` and `pinned: None`. The
//! scheduler decides who runs each one — see the plan's note on why nothing is
//! pinned.
//!
//! A chain stays with one bot because the driver stamps a whole subtree with
//! one `ChainId`, and the scheduler assigns chains rather than actions. Two
//! things open a chain, and only these two: a caller naming a bot
//! (`Holder::Bot`), which additionally records that bot as the chain's owner,
//! and a method whose decomposition makes several *produced* items meet in one
//! inventory (`Method::converges`). A goal that merely sits inside a split
//! opens none, because welding it would serialise work that could have run in
//! parallel. `HasItem` preconditions alone are not enough:
//! they keep a *linear* chain together, since only the bot holding the items
//! can run the next step, but a recipe with two ingredients that each need
//! producing is a chain with two roots, and neither root has a `HasItem`
//! precondition to hold it near the other. Red science is exactly that shape.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{BotId, Ticks};
use crate::method::util::{
    free_area_near, ingredients_of, mining_ticks, nearest_resource_tile, output_per_craft,
    recipe_for, recipe_ticks, research_ingredients, research_ticks, resource_supply_at_least,
    resource_tiles_for,
};
use crate::method::{ExpansionCtx, GoalSite, Method, MethodRegistry, Step};
use crate::state::PlanState;
use factorio_bot_core::types::FactorioEntity;
use std::collections::BTreeSet;

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
        .any(|bot| state.inventory_count(*bot, item) >= count)
}

/// How much of `item` still needs producing, given what is already held.
fn shortfall(state: &PlanState, item: &str, count: u32, whose: &Holder) -> u32 {
    let held = match whose {
        Holder::Anyone => state.total_count(item),
        Holder::Bot(id) | Holder::Share(id) => state.inventory_count(*id, item),
    };
    count.saturating_sub(held)
}

/// The goal is already met. Emits nothing.
///
/// Registered first everywhere, so "we already have this" is decided in exactly
/// one place rather than re-tested inside every method that could otherwise
/// have satisfied the goal.
pub struct AlreadySatisfied;

impl Method for AlreadySatisfied {
    fn name(&self) -> &'static str {
        "already-satisfied"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        match goal {
            Goal::Have { item, count, whose } => shortfall(state, item, *count, whose) == 0,
            // `PlanState::is_researched` answers over two sources: the
            // technologies this plan has already scheduled research for (its
            // own overlay) and the ones the world reports as researched
            // (reality). Either alone is a wrong answer here — skipping the
            // overlay would plan the same research twice for two goals that
            // share a prerequisite, and skipping the world would re-research
            // what the force already has.
            //
            // They cannot contradict each other, which is why "which wins" has
            // no bite: research is monotone. Nothing in the game or in this
            // planner ever un-researches a technology, so the overlay can only
            // ever add to what the world reports, and the union is the whole
            // truth. If a future Factorio grew a way to lose a technology, the
            // overlay would have to learn to subtract and this comment would be
            // wrong — that is the assumption to check first.
            Goal::Researched(tech) => state.is_researched(tech),
            _ => false,
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
/// steel's 16 s under-fuels by a factor of five. This is still an
/// approximation: it ignores partial burns carried between smelts, and it
/// assumes stone-furnace speed. Calibrating it against observed burn rates is
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
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
            return false;
        }
        matches!(recipe_for(state, item), Some(r) if r.category == "smelting")
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let recipe =
            recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;
        let per_craft = output_per_craft(&recipe, item);
        let runs = need.div_ceil(per_craft);
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
            ],
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
        let smelt_lag = recipe_ticks(&recipe).saturating_mul(runs);
        for id in insert_ids {
            let lag = if id == fuel_id { 0 } else { smelt_lag };
            steps.push(Step::Link {
                from: id,
                to: remove_id,
                lag,
            });
        }

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
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        let need = shortfall(state, item, *count, whose);
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
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
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
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
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
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
            return false;
        }
        matches!(recipe_for(state, item), Some(r) if r.category == "crafting")
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let recipe =
            recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;
        let runs = need.div_ceil(output_per_craft(&recipe, item));

        let mut steps: Vec<Step> = Vec::new();
        let mut pre = Vec::new();
        let mut eff = Vec::new();

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
        research_ingredients(&tech)
            .iter()
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

        let mut steps: Vec<Step> = Vec::new();
        for prerequisite in &prerequisites {
            steps.push(Step::Subgoal(Goal::Researched(prerequisite.clone())));
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
/// They are emitted as `Holder::Bot(_)` subgoals so the other methods handle
/// them without recursing back into this one — and so that the driver opens a
/// chain per share, which is what keeps each share's steps in one inventory.
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
        let chains = (self.bots.len() as u32).min(need);
        let base = need / chains;
        let remainder = need % chains;

        let mut steps = Vec::new();
        for (index, bot) in self.bots.iter().take(chains as usize).enumerate() {
            let share = base + if (index as u32) < remainder { 1 } else { 0 };
            // A `Have` goal states a holding, not a delivery, so a share of one
            // handed to a bot already holding five is a goal that is already
            // met — and the share evaporates. Ask for what the bot has *plus*
            // its share, so the shortfall the other methods see is the share.
            // Shares sum to the shortfall, so the roster ends up with at least
            // `count` between them however the holdings started.
            let held = ctx.state.inventory_count(*bot, item);
            steps.push(Step::Subgoal(Goal::Have {
                item: item.clone(),
                count: held.saturating_add(share),
                whose: Holder::Share(*bot),
            }));
        }
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
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::network::ActionNetwork;
    use crate::schedule::{schedule, StepKind};
    use crate::state::PlanState;
    use factorio_bot_core::factorio::util::calculate_distance;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
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
        assert!(action
            .eff
            .contains(&Effect::Researched("automation".into())));
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
        // One second per ore in the fixture.
        assert_eq!(action.duration, 300);
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
        assert!(action
            .pre
            .iter()
            .any(|c| matches!(c, Condition::AtPosition { .. })));
        assert!(action
            .pre
            .iter()
            .any(|c| matches!(c, Condition::ResourceAvailable { item, count, .. } if item == "coal" && *count == 2)));
        assert!(action.eff.iter().any(
            |e| matches!(e, Effect::GainItem { item, count, .. } if item == "coal" && *count == 2)
        ));
        assert!(action
            .eff
            .iter()
            .any(|e| matches!(e, Effect::ConsumeResource { .. })));
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

        // iron-plate is 3.2 s each, so two plates lag 2 * 192 = 384 ticks.
        assert_eq!(
            lag_from("iron-ore"),
            384,
            "the ore insert carries the smelting time"
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

    /// A smelting recipe the fixture does not ship: steel plate, 16 s a run
    /// against iron plate's 3.2 s. Deserialised rather than built, because
    /// `FactorioRecipe::energy` is a `noisy_float` this crate does not depend
    /// on directly.
    fn state_knowing_steel() -> PlanState {
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
        // Mining never converges, so the single share needs no chain either:
        // nothing downstream needs its output gathered with anything else.
        let only = net.actions().next().expect("one action");
        assert!(
            net.chain_of(only.id).is_none(),
            "a non-converging single share has nothing to weld to a runner"
        );
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
        // would evaporate. The holdings are equal across the roster because
        // expansion sizes each share against one bot's inventory and assumes
        // any bot would do — an asymmetric fixture here would describe a plan
        // whose chains are only feasible on the bot they were sized for.
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

    #[test]
    fn smelting_never_converges() {
        let s = state(&[BotId(1)]);
        // A furnace is fed by three separate actions — place, insert ore,
        // insert coal — so three different bots can each supply one input.
        // Nothing has to meet in a single inventory.
        let plate = Goal::Have {
            item: "iron-plate".into(),
            count: 2,
            whose: Holder::Anyone,
        };
        assert!(!Smelt.converges(&plate, &s));
    }

    #[test]
    fn a_linear_goal_gets_no_chain_and_stays_parallel() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "stone-furnace", 1);
        }
        let net = expand(
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
        assert!(
            net.actions().all(|a| net.chain_of(a.id).is_none()),
            "a smelt converges nowhere, so nothing needs welding to one bot"
        );
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        let used: std::collections::BTreeSet<_> = plan.steps.iter().map(|s| s.bot).collect();
        assert!(
            used.len() > 1,
            "the mining roots must not serialise onto one bot"
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
}
