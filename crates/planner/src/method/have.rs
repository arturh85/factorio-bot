//! Methods that satisfy `Goal::Have`.
//!
//! Every method here emits actions with `Actor::Role` and `pinned: None`. The
//! scheduler decides who runs each one, and a chain stays with one bot because
//! its `HasItem` preconditions are only satisfiable by the bot holding the
//! items — see the plan's note on why nothing is pinned.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::method::util::{mining_ticks, nearest_resource_tile};
use crate::method::{ExpansionCtx, Method, MethodRegistry, Step};
use crate::state::PlanState;

/// How much of `item` still needs producing, given what is already held.
fn shortfall(state: &PlanState, item: &str, count: u32, whose: &Holder) -> u32 {
    let held = match whose {
        Holder::Anyone => state.total_count(item),
        Holder::Bot(id) => state.inventory_count(*id, item),
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
        // Position-independent on purpose: applicability asks only whether a
        // tile with enough left exists anywhere. Which one is nearest is
        // `expand`'s business, and depends on the chain actor it has and this
        // method does not.
        state.resource_patches(item).iter().any(|patch| {
            patch
                .elements
                .iter()
                .any(|tile| state.resource_available(tile, item) >= need)
        })
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
        let pos = nearest_resource_tile(&ctx.state, item, &from, need).ok_or_else(|| {
            PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            }
        })?;

        let action = Action {
            id: ctx.ids.next(),
            kind: ActionKind::Mine {
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
                Condition::ResourceAvailable {
                    pos: pos.clone(),
                    item: item.clone(),
                    count: need,
                },
            ],
            eff: vec![
                Effect::ConsumeResource {
                    pos,
                    item: item.clone(),
                    count: need,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: item.clone(),
                    count: need,
                },
            ],
            duration: mining_ticks(&ctx.state, item).saturating_mul(need),
            pinned: None,
            label: format!("mine {} {}", need, item),
        };
        Ok(vec![Step::Act(Box::new(action))])
    }
}

/// The methods this crate ships, in preference order.
pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
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
}
