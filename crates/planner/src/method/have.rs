//! Methods that satisfy `Goal::Have`.
//!
//! Every method here emits actions with `Actor::Role` and `pinned: None`. The
//! scheduler decides who runs each one — see the plan's note on why nothing is
//! pinned.
//!
//! A chain stays with one bot because the driver stamps everything it expands
//! under a `Holder::Bot` goal with one `ChainId`, and the scheduler assigns
//! chains rather than actions. `HasItem` preconditions alone are not enough:
//! they keep a *linear* chain together, since only the bot holding the items
//! can run the next step, but a recipe with two ingredients that each need
//! producing is a chain with two roots, and neither root has a `HasItem`
//! precondition to hold it near the other. Red science is exactly that shape.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{BotId, Ticks};
use crate::method::util::{
    free_tile_near, ingredients_of, mining_ticks, output_per_craft, recipe_for, recipe_ticks,
    resource_tiles_for,
};
use crate::method::{ExpansionCtx, GoalSite, Method, MethodRegistry, Step};
use crate::state::PlanState;
use factorio_bot_core::types::{FactorioEntity, Position};

/// How much of `item` still needs producing, given what is already held.
fn shortfall(state: &PlanState, item: &str, count: u32, whose: &Holder) -> u32 {
    let held = match whose {
        Holder::Anyone => state.total_count(item),
        Holder::Bot(id) | Holder::Share(id) => state.inventory_count(*id, item),
    };
    count.saturating_sub(held)
}

/// The goal is already met. Emits nothing.
pub struct AlreadySatisfied;

impl Method for AlreadySatisfied {
    fn name(&self) -> &'static str {
        "already-satisfied"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        match goal {
            Goal::Have { item, count, whose } => shortfall(state, item, *count, whose) == 0,
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
        let pos =
            free_tile_near(&ctx.state, &from).ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
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
            name: "stone-furnace".into(),
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
                Condition::PositionFree { pos: pos.clone() },
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
        // Position-independent, as before: `resource_tiles_for` returns empty
        // exactly when the patches cannot supply `need` in total, whatever the
        // origin, so applicability does not depend on which bot is asking.
        // Which tiles are nearest is `expand`'s business, where the chain actor
        // is known.
        !resource_tiles_for(state, item, &Position::default(), need).is_empty()
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
pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(Smelt))
        .with(Box::new(HandCraft))
        .with(Box::new(Mine))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::schedule::schedule;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), bots)
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
        // Bot 1 holds three of the five wanted, so two remain. A `Have` goal
        // states a holding rather than a delivery: asking bot 1 for "one"
        // would be a goal it already meets, and its share would evaporate.
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        s.gain(BotId(1), "iron-ore", 3);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 5,
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
        let mined: u32 = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected only mines, got {:?}", other),
            })
            .sum();
        assert_eq!(mined, 1200);
        assert_eq!(net.len(), 3, "500 + 500 + 200");
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
