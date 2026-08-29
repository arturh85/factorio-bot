//! Scheduling of the action network across bots. Filled in by Task 5/7.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, Ticks};
use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::Position;
use std::collections::{BTreeMap, BTreeSet};

/// Character walking speed in tiles per tick (roughly 9 tiles/second).
pub const WALK_TILES_PER_TICK: f64 = 0.15;

/// Ticks to get from `from` to within `radius` of `to`. Zero if already there.
pub fn travel_ticks(from: &Position, to: &Position, radius: f64) -> Ticks {
    let distance = calculate_distance(from, to);
    if distance <= radius {
        return 0;
    }
    ((distance - radius) / WALK_TILES_PER_TICK).ceil() as Ticks
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepKind {
    Act { action: ActionId, label: String },
    Walk { to: Position },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledStep {
    pub what: StepKind,
    pub bot: BotId,
    pub start: Ticks,
    pub end: Ticks,
}

/// An immutable assignment of actions to bots over time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    pub steps: Vec<ScheduledStep>,
    pub makespan: Ticks,
}

impl Schedule {
    pub fn steps_for(&self, bot: BotId) -> Vec<&ScheduledStep> {
        self.steps.iter().filter(|s| s.bot == bot).collect()
    }

    pub fn assignment(&self, action: ActionId) -> Option<BotId> {
        self.steps.iter().find_map(|s| match &s.what {
            StepKind::Act { action: id, .. } if *id == action => Some(s.bot),
            _ => None,
        })
    }
}

struct Candidate {
    action: ActionId,
    bot: BotId,
    start: Ticks,
    travel: Ticks,
    end: Ticks,
}

/// Assign every action in `net` to one of `bots`, travel-aware and greedy.
///
/// A pure function of its three arguments. Ties break on `(ActionId, BotId)`
/// ascending, so the output is stable across runs.
pub fn schedule(
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
) -> Result<Schedule, PlannerError> {
    if bots.is_empty() {
        return Err(PlannerError::NoBots);
    }
    net.validate()?;

    let mut sim = state.fork();
    let mut free_at: BTreeMap<BotId, Ticks> = bots.iter().map(|b| (*b, 0)).collect();
    let mut finished: BTreeMap<ActionId, Ticks> = BTreeMap::new();
    let mut done: BTreeSet<ActionId> = BTreeSet::new();
    let mut steps: Vec<ScheduledStep> = Vec::new();

    while done.len() < net.len() {
        let ready: Vec<&crate::action::Action> = net
            .actions()
            .filter(|a| !done.contains(&a.id))
            .filter(|a| net.preds(a.id).iter().all(|(p, _)| done.contains(p)))
            .collect();

        let stuck = net.actions().find(|a| !done.contains(&a.id)).map(|a| a.id);
        if ready.is_empty() {
            return Err(PlannerError::Deadlock {
                action: stuck.expect("loop condition guarantees an unfinished action"),
            });
        }

        let mut best: Option<Candidate> = None;
        for action in &ready {
            let deps_ready = net
                .preds(action.id)
                .iter()
                .map(|(p, lag)| finished[p] + lag)
                .max()
                .unwrap_or(0);

            let candidate_bots: Vec<BotId> = match action.pinned {
                Some(pinned) if bots.contains(&pinned) => vec![pinned],
                Some(pinned) => return Err(PlannerError::UnknownBot(pinned)),
                None => bots.to_vec(),
            };

            for bot in candidate_bots {
                let from = &sim.bot(bot).ok_or(PlannerError::UnknownBot(bot))?.position;
                let travel = match action.required_position() {
                    Some((ref pos, radius)) => travel_ticks(from, pos, radius),
                    None => 0,
                };
                let start = free_at[&bot].max(deps_ready);
                let end = start + travel + action.duration;
                let better = match &best {
                    None => true,
                    Some(b) => (end, action.id, bot) < (b.end, b.action, b.bot),
                };
                if better {
                    best = Some(Candidate {
                        action: action.id,
                        bot,
                        start,
                        travel,
                        end,
                    });
                }
            }
        }

        let chosen = best.expect("ready is non-empty and bots is non-empty");
        let action = net
            .action(chosen.action)
            .expect("candidate came from this network");

        // Walk first, so the AtPosition precondition can hold when checked.
        if chosen.travel > 0 {
            let (target, _) = action
                .required_position()
                .expect("travel is non-zero only when a position is required");
            steps.push(ScheduledStep {
                what: StepKind::Walk { to: target.clone() },
                bot: chosen.bot,
                start: chosen.start,
                end: chosen.start + chosen.travel,
            });
            sim.set_position(chosen.bot, target);
        }

        for condition in &action.pre {
            if !condition.holds(&sim, chosen.bot) {
                return Err(PlannerError::PreconditionUnsatisfied {
                    action: action.id,
                    bot: chosen.bot,
                    condition: condition.to_string(),
                });
            }
        }

        for effect in &action.eff {
            effect.apply(&mut sim, chosen.bot)?;
        }

        steps.push(ScheduledStep {
            what: StepKind::Act {
                action: action.id,
                label: action.label.clone(),
            },
            bot: chosen.bot,
            start: chosen.start + chosen.travel,
            end: chosen.end,
        });

        free_at.insert(chosen.bot, chosen.end);
        finished.insert(chosen.action, chosen.end);
        done.insert(chosen.action);
    }

    let makespan = steps.iter().map(|s| s.end).max().unwrap_or(0);
    Ok(Schedule { steps, makespan })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind, Actor, Condition, Effect};
    use crate::ids::ActionIdGen;
    use crate::network::ActionNetwork;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        let mut s = PlanState::from_world(Arc::new(fixture_world()), bots);
        for bot in bots {
            s.set_position(*bot, Position::new(0., 0.));
        }
        s
    }

    /// An action needing nothing, at the origin, taking `duration` ticks.
    fn free(gen: &mut ActionIdGen, label: &str, duration: Ticks) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration,
            pinned: None,
            label: label.into(),
        }
    }

    /// An action requiring the bot to stand within `radius` of `pos`.
    fn at(gen: &mut ActionIdGen, label: &str, pos: Position, radius: f64) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius,
            }],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: label.into(),
        }
    }

    #[test]
    fn no_bots_is_an_error() {
        let net = ActionNetwork::new();
        let s = state(&[]);
        assert!(schedule(&net, &s, &[]).is_err());
    }

    #[test]
    fn an_empty_network_schedules_to_zero() {
        let net = ActionNetwork::new();
        let s = state(&[BotId(1)]);
        let result = schedule(&net, &s, &[BotId(1)]).unwrap();
        assert_eq!(result.makespan, 0);
        assert!(result.steps.is_empty());
    }

    #[test]
    fn independent_actions_run_in_parallel_on_two_bots() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut gen, "a", 100));
        net.add(free(&mut gen, "b", 100));
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 100, "two bots should overlap the work");
    }

    #[test]
    fn one_bot_serialises_the_same_actions() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut gen, "a", 100));
        net.add(free(&mut gen, "b", 100));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 200);
    }

    #[test]
    fn a_dependency_lag_delays_the_successor() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(free(&mut gen, "insert", 10));
        let b = net.add(free(&mut gen, "remove", 10));
        net.link(a, b, 200);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        // insert ends at 10, lag 200 -> remove starts no earlier than 210.
        assert_eq!(result.makespan, 220);
    }

    #[test]
    fn a_walk_is_emitted_when_the_bot_is_out_of_range() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut gen, "far", Position::new(30., 0.), 3.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert!(
            matches!(result.steps[0].what, StepKind::Walk { .. }),
            "first step should be the walk"
        );
        assert!(matches!(result.steps[1].what, StepKind::Act { .. }));
        assert!(result.makespan > 60, "walking must cost time");
    }

    #[test]
    fn no_walk_is_emitted_when_already_in_range() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut gen, "near", Position::new(1., 1.), 5.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.makespan, 60);
    }

    #[test]
    fn a_pinned_action_goes_to_its_bot() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut gen, "pinned", 60);
        action.pinned = Some(BotId(2));
        let id = net.add(action);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(id), Some(BotId(2)));
    }

    #[test]
    fn an_unsatisfiable_precondition_is_an_error() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut gen, "needs plates", 60);
        action.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        net.add(action);
        let bots = [BotId(1)];
        assert!(schedule(&net, &state(&bots), &bots).is_err());
    }

    #[test]
    fn effects_accumulate_so_a_later_action_sees_them() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut producer = free(&mut gen, "produce", 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let mut consumer = free(&mut gen, "consume", 10);
        consumer.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let p = net.add(producer);
        let c = net.add(consumer);
        net.link(p, c, 0);
        // One bot only, so the consumer sees the producer's items.
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(c), Some(BotId(1)));
    }

    #[test]
    fn scheduling_is_deterministic() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        for i in 0..6 {
            net.add(free(&mut gen, &format!("a{}", i), 40));
        }
        let bots = [BotId(1), BotId(2), BotId(3)];
        let first = schedule(&net, &state(&bots), &bots).unwrap();
        let second = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(first.steps, second.steps);
    }
}
