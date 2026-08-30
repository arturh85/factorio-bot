//! Run one bot's slice of a `Schedule` against an `Actuator`.

use crate::actuator::{Actuator, ActuatorError};
use crate::log::ExecutionLog;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, Schedule, StepKind};

/// Walk one bot's slice of the schedule, in order.
///
/// Stops that bot at its first failure: later steps in a chain depend on
/// earlier ones, and pressing on would issue commands whose preconditions the
/// game no longer satisfies. Recovery is the caller's decision (see `recover`).
pub async fn run_bot<A: Actuator + ?Sized>(
    act: &A,
    bot: BotId,
    sched: &Schedule,
    net: &ActionNetwork,
) -> ExecutionLog {
    let mut log = ExecutionLog::default();
    for step in sched.steps.iter().filter(|s| s.bot == bot) {
        match &step.what {
            StepKind::Walk { to } => {
                if act.walk(bot, to.clone()).await.is_err() {
                    return log;
                }
            }
            StepKind::Act { action, .. } => {
                log.start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    log.fail(*action, step.start, "action not in network".to_string());
                    return log;
                };
                match perform(act, bot, &a.kind).await {
                    Ok(()) => log.succeed(*action, step.end),
                    Err(e) => {
                        log.fail(*action, step.end, e.to_string());
                        return log;
                    }
                }
            }
        }
    }
    log
}

async fn perform<A: Actuator + ?Sized>(
    act: &A,
    bot: BotId,
    kind: &ActionKind,
) -> Result<(), ActuatorError> {
    match kind {
        ActionKind::Mine { pos, item, count } => {
            act.mine(bot, item.as_str(), pos.clone(), *count).await
        }
        ActionKind::Craft { item, count } => act.craft(bot, item.as_str(), *count).await,
        ActionKind::Place { entity } => {
            act.place(bot, &entity.name, entity.position.clone(), entity.direction)
                .await
        }
        ActionKind::Insert {
            pos,
            entity,
            slot,
            item,
            count,
        } => {
            act.insert(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Remove {
            pos,
            entity,
            slot,
            item,
            count,
        } => {
            act.remove(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Research { tech } => act.research(tech).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Status;
    use factorio_bot_core::types::Position;
    use factorio_bot_planner::{Action, ActionId, Actor, Condition, Effect, ScheduledStep};
    use mockall::mock;

    mock! {
        pub Act {}
        #[async_trait::async_trait]
        impl Actuator for Act {
            async fn walk(&self, bot: BotId, to: Position) -> Result<(), ActuatorError>;
            async fn mine(&self, bot: BotId, item: &str, at: Position, count: u32) -> Result<(), ActuatorError>;
            async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<(), ActuatorError>;
            async fn place(&self, bot: BotId, item: &str, at: Position, direction: u8) -> Result<(), ActuatorError>;
            async fn insert(&self, bot: BotId, entity: &str, at: Position, slot: factorio_bot_planner::InventorySlot, item: &str, count: u32) -> Result<(), ActuatorError>;
            async fn remove(&self, bot: BotId, entity: &str, at: Position, slot: factorio_bot_planner::InventorySlot, item: &str, count: u32) -> Result<(), ActuatorError>;
            async fn research(&self, tech: &str) -> Result<(), ActuatorError>;
        }
    }

    fn mine_action_id() -> ActionId {
        ActionId(0)
    }

    /// The action scheduled right after the mine in `walk_then_mine_fixture`.
    /// Exists so `a_failed_step_is_logged_and_stops_that_bot` has something
    /// to prove was *never attempted* — a fixture whose failing step is last
    /// cannot distinguish "stops" from "there was nothing left to do".
    fn craft_action_id() -> ActionId {
        ActionId(1)
    }

    /// A two-action network (`Mine` then `Craft`) preceded by a `Walk` step,
    /// all for `BotId(0)`.
    fn walk_then_mine_fixture() -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        let mine = Action {
            id: mine_action_id(),
            kind: ActionKind::Mine {
                pos: Position::new(10., 10.),
                item: "iron-ore".into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: Position::new(10., 10.),
                radius: 3.0,
            }],
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: "iron-ore".into(),
                count: 1,
            }],
            duration: 60,
            pinned: None,
            label: "mine 1 iron-ore".into(),
        };
        net.add(mine);

        let craft = Action {
            id: craft_action_id(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "craft 1 iron-gear-wheel".into(),
        };
        net.add(craft);

        let sched = Schedule {
            steps: vec![
                ScheduledStep {
                    what: StepKind::Walk {
                        to: Position::new(10., 10.),
                    },
                    bot: BotId(0),
                    start: 0,
                    end: 60,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: mine_action_id(),
                        label: "mine 1 iron-ore".into(),
                    },
                    bot: BotId(0),
                    start: 60,
                    end: 120,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: craft_action_id(),
                        label: "craft 1 iron-gear-wheel".into(),
                    },
                    bot: BotId(0),
                    start: 120,
                    end: 150,
                },
            ],
            makespan: 150,
        };
        (net, sched)
    }

    #[tokio::test]
    async fn a_bot_performs_its_steps_in_schedule_order() {
        let mut act = MockAct::new();
        let mut seq = mockall::Sequence::new();
        act.expect_walk()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok(()));
        act.expect_mine()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _, _| Ok(()));
        act.expect_craft()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run_bot(&act, BotId(0), &sched, &net).await;

        assert_eq!(log.failed(), vec![]);
        assert_eq!(log.status(mine_action_id()), Status::Success);
        assert_eq!(log.status(craft_action_id()), Status::Success);
    }

    #[tokio::test]
    async fn a_failed_step_is_logged_and_stops_that_bot() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _| Ok(()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into())));
        // The craft step follows the failing mine in the schedule. If
        // `run_bot` pressed on after the failure instead of stopping, this
        // is what it would dispatch next.
        act.expect_craft().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run_bot(&act, BotId(0), &sched, &net).await;

        assert_eq!(log.failed(), vec![mine_action_id()]);
        assert_eq!(
            log.status(craft_action_id()),
            Status::Pending,
            "run_bot must stop at the first failure, not merely record it \
             and press on"
        );
    }

    #[tokio::test]
    async fn a_step_for_another_bot_is_not_dispatched() {
        // Guards the `.filter(|s| s.bot == bot)` line: without it, bot 0's run
        // would also issue bot 1's walk.
        let mut act = MockAct::new();
        act.expect_walk().times(1).returning(|_, _| Ok(()));
        act.expect_mine().times(1).returning(|_, _, _, _| Ok(()));
        act.expect_craft().times(1).returning(|_, _, _| Ok(()));

        let (net, mut sched) = walk_then_mine_fixture();
        sched.steps.push(ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(5., 5.),
            },
            bot: BotId(1),
            start: 0,
            end: 10,
        });

        let log = run_bot(&act, BotId(0), &sched, &net).await;
        assert_eq!(log.failed(), vec![]);
        assert_eq!(log.status(mine_action_id()), Status::Success);
        assert_eq!(log.status(craft_action_id()), Status::Success);
    }

    #[tokio::test]
    async fn a_walk_failure_stops_the_bot_before_the_action_runs() {
        let mut act = MockAct::new();
        act.expect_walk()
            .times(1)
            .returning(|_, _| Err(ActuatorError::Rejected("blocked".into())));
        act.expect_mine().times(0);

        let (net, sched) = walk_then_mine_fixture();
        let log = run_bot(&act, BotId(0), &sched, &net).await;

        // The action never even started: the log has no attempt for it at all,
        // not merely a non-Failed status.
        assert!(log.is_empty());
        assert_eq!(log.status(mine_action_id()), Status::Pending);
    }

    #[tokio::test]
    async fn an_action_missing_from_the_network_is_logged_as_a_failure() {
        let act = MockAct::new();
        let (_net, sched) = walk_then_mine_fixture();
        // Deliberately give an empty network: the schedule references an
        // action id the network does not contain.
        let empty_net = ActionNetwork::new();
        // Bypass the walk step's own expectations by only scheduling the Act.
        let sched = Schedule {
            steps: sched
                .steps
                .into_iter()
                .filter(|s| matches!(s.what, StepKind::Act { .. }))
                .collect(),
            makespan: sched.makespan,
        };

        let log = run_bot(&act, BotId(0), &sched, &empty_net).await;
        assert_eq!(log.failed(), vec![mine_action_id()]);
    }

    #[tokio::test]
    async fn insert_dispatches_the_entity_and_slot_the_action_named() {
        // Regression guard for the inventory-slot hazard: the executor must
        // pass through exactly the (entity, slot) pair the action carries,
        // never substitute one that "looks right" for the item.
        use factorio_bot_planner::InventorySlot;
        use mockall::predicate::eq;

        let mut act = MockAct::new();
        act.expect_insert()
            .with(
                eq(BotId(0)),
                eq("stone-furnace"),
                eq(Position::new(1., 1.)),
                eq(InventorySlot::FurnaceSource),
                eq("iron-ore"),
                eq(4u32),
            )
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(()));

        let kind = ActionKind::Insert {
            pos: Position::new(1., 1.),
            entity: "stone-furnace".into(),
            slot: InventorySlot::FurnaceSource,
            item: "iron-ore".into(),
            count: 4,
        };
        perform(&act, BotId(0), &kind).await.unwrap();
    }
}
