//! Assignment of a bot-free action network to concrete bots over time.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ChainId, Ticks};
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
    /// When the bot sets off. It walks as soon as it is free, even if the
    /// action's dependencies are not ready yet — see `schedule`.
    walk_start: Ticks,
    travel: Ticks,
    /// When the action itself begins: after the walk *and* after the
    /// dependencies, whichever is later.
    act_start: Ticks,
    end: Ticks,
}

impl Candidate {
    /// The ranking key. Ascending, so the smallest wins.
    fn key(&self) -> (Ticks, ActionId, BotId) {
        (self.end, self.action, self.bot)
    }
}

/// A pair rejected because one of the action's preconditions would not hold
/// for that bot. Kept only so the error names a plausible pair if *no* pair is
/// feasible.
struct Rejected {
    candidate: Candidate,
    condition: String,
}

/// Assign every action in `net` to one of `bots`, travel-aware and greedy.
///
/// A pure function of its three arguments. Ties break on `(ActionId, BotId)`
/// ascending, so the output is stable across runs.
///
/// A bot walks the moment it is free, not the moment its dependencies are:
///
/// ```text
/// walk_start = free_at[bot]
/// walk_end   = walk_start + travel
/// act_start  = max(walk_end, deps_ready)
/// end        = act_start + duration
/// ```
///
/// Walking across a lag is most of where multi-bot parallelism comes from —
/// heading for the next furnace while the current one smelts. Adding travel
/// *after* `max(free_at, deps_ready)` instead would idle the bot for the lag
/// and then walk, which is strictly worse and never better.
///
/// Assignment happens at **chain** granularity, not action granularity. The
/// first action of a chain (`ActionNetwork::chain_of`) is assigned freely, by
/// the rule above; every later action of that chain goes to the bot the chain
/// is already bound to. A chain's steps pass items to each other through one
/// inventory, and a branching chain — two ingredients that each need producing
/// — has two roots whose first actions carry no `HasItem` precondition to hold
/// them together, so without this they land on different bots and the action
/// consuming both has no feasible bot at all. A chain being opened prefers a
/// bot not already carrying one, so independent chains spread rather than pile
/// up on a bot that is merely nearby.
///
/// Binding only ever *narrows the candidate set*: how a candidate is ranked and
/// judged feasible is unchanged. Actions belonging to no chain stay
/// individually assignable and take exactly the path they took before chains
/// existed.
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
    let mut chain_binding: BTreeMap<ChainId, BotId> = BTreeMap::new();

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
        let mut best_rejected: Option<Rejected> = None;
        for action in &ready {
            let deps_ready = net
                .preds(action.id)
                .iter()
                .map(|(p, lag)| finished[p] + lag)
                .max()
                .unwrap_or(0);

            // The chain this action belongs to, and the bot already running it.
            let chain = net.chain_of(action.id);
            let bound = chain.and_then(|c| chain_binding.get(&c).copied());

            let candidate_bots: Vec<BotId> = match action.pinned {
                Some(pinned) if !bots.contains(&pinned) => {
                    return Err(PlannerError::UnknownBot(pinned))
                }
                // Pinning takes precedence over everything: it is an explicit
                // instruction, chain binding is an inference. But rather than
                // silently overriding the binding — which would hand a chain's
                // items to a bot that does not hold them, and fail later with a
                // confusing precondition error somewhere else — a contradiction
                // is reported here, where its cause is still visible. Nothing
                // sets `pinned` today, so this is defensive.
                Some(pinned) => {
                    if let (Some(chain), Some(bound)) = (chain, bound) {
                        if bound != pinned {
                            return Err(PlannerError::ChainConflict {
                                chain,
                                action: action.id,
                                bound_to: bound,
                                pinned_to: pinned,
                            });
                        }
                    }
                    vec![pinned]
                }
                // An action already in a running chain follows it.
                None if bound.is_some() => vec![bound.expect("just checked")],
                // A chain being *opened* prefers a bot that is not already
                // carrying a different chain, so independent chains spread
                // instead of piling onto whichever bot happens to be cheapest.
                // Chains never outnumber bots today, but nothing makes
                // `chain_binding` injective on its own: `free_at` is the only
                // thing pushing the next chain elsewhere, and a bot parked far
                // away can be dearer than a near one running work already —
                // exactly what `travel_cost_can_outweigh_an_idle_bot` asserts.
                // Two smelting chains on one bot then want four furnaces from a
                // stock of two, and since binding leaves that action a single
                // candidate there is no recovery from it.
                //
                // This can cost time: a chain may open on a distant idle bot
                // where a nearer busy one would have finished sooner. Failing
                // to schedule at all is worse than scheduling slowly.
                //
                // Falls back to the full roster once every bot carries a chain.
                // An action in no chain at all is unaffected — it has nothing to
                // keep together, so it keeps the whole roster and the old path.
                None if chain.is_some() => {
                    let busy: BTreeSet<BotId> = chain_binding.values().copied().collect();
                    let free: Vec<BotId> =
                        bots.iter().copied().filter(|b| !busy.contains(b)).collect();
                    if free.is_empty() {
                        bots.to_vec()
                    } else {
                        free
                    }
                }
                None => bots.to_vec(),
            };

            for bot in candidate_bots {
                let from = &sim.bot(bot).ok_or(PlannerError::UnknownBot(bot))?.position;
                let travel = match action.required_position() {
                    Some((ref pos, radius)) => travel_ticks(from, pos, radius),
                    None => 0,
                };
                let walk_start = free_at[&bot];
                let act_start = (walk_start + travel).max(deps_ready);
                let end = act_start + action.duration;
                let candidate = Candidate {
                    action: action.id,
                    bot,
                    walk_start,
                    travel,
                    act_start,
                    end,
                };

                // Feasibility is part of selection, not a check on the winner:
                // a pair whose preconditions cannot hold is never offered, so
                // another bot can take the action. A positional precondition is
                // satisfied by the walk this very pair would emit, so it is
                // tested against a fork with the bot already moved. Forking is
                // an overlay clone — cheap enough to do per candidate, which is
                // what the shared `Arc` base is for.
                let mut trial = sim.fork();
                if travel > 0 {
                    if let Some((pos, _)) = action.required_position() {
                        trial.set_position(bot, pos);
                    }
                }
                let failing = action.pre.iter().find(|c| !c.holds(&trial, bot));

                match failing {
                    None => {
                        if best.as_ref().is_none_or(|b| candidate.key() < b.key()) {
                            best = Some(candidate);
                        }
                    }
                    Some(condition) => {
                        if best_rejected
                            .as_ref()
                            .is_none_or(|r| candidate.key() < r.candidate.key())
                        {
                            best_rejected = Some(Rejected {
                                condition: condition.to_string(),
                                candidate,
                            });
                        }
                    }
                }
            }
        }

        // Only when no bot can run any ready action is the plan actually stuck.
        let chosen = match best {
            Some(candidate) => candidate,
            None => {
                let rejected = best_rejected.expect("ready and bots are both non-empty");
                return Err(PlannerError::PreconditionUnsatisfied {
                    action: rejected.candidate.action,
                    bot: rejected.candidate.bot,
                    condition: rejected.condition,
                });
            }
        };
        let action = net
            .action(chosen.action)
            .expect("candidate came from this network");

        // Walk first, so the AtPosition precondition holds by `act_start`. The
        // walk may finish well before it, if the action waits on a lag.
        if chosen.travel > 0 {
            let (target, _) = action
                .required_position()
                .expect("travel is non-zero only when a position is required");
            steps.push(ScheduledStep {
                what: StepKind::Walk { to: target.clone() },
                bot: chosen.bot,
                start: chosen.walk_start,
                end: chosen.walk_start + chosen.travel,
            });
            sim.set_position(chosen.bot, target);
        }

        // No precondition check here: selection already proved every one of them
        // holds for this pair against a fork that made exactly the move above.

        for effect in &action.eff {
            effect.apply(&mut sim, chosen.bot)?;
        }

        steps.push(ScheduledStep {
            what: StepKind::Act {
                action: action.id,
                label: action.label.clone(),
            },
            bot: chosen.bot,
            start: chosen.act_start,
            end: chosen.end,
        });

        // The chain now has a runner; every later action of it follows.
        if let Some(chain) = net.chain_of(chosen.action) {
            chain_binding.insert(chain, chosen.bot);
        }

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

    /// Like `at`, but with a caller-chosen duration.
    fn at_for(
        gen: &mut ActionIdGen,
        label: &str,
        pos: Position,
        radius: f64,
        duration: Ticks,
    ) -> Action {
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
            duration,
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
        // 30 tiles, radius 3: ceil(27 / 0.15) = 180 travel ticks, then 60 duration.
        assert_eq!(result.makespan, 240);
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
    fn a_bot_walks_across_a_dependency_lag() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Insert the ore, then remove the plate 200 ticks later, 30 tiles away.
        let insert = net.add(free(&mut gen, "insert", 10));
        let remove = net.add(at_for(&mut gen, "remove", Position::new(30., 0.), 3.0, 60));
        net.link(insert, remove, 200);

        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();

        // 30 tiles, radius 3: ceil(27 / 0.15) = 180 travel ticks. The bot is free
        // at 10 and the lag clears at 210, so it walks [10, 190] *during* the
        // lag, waits 20 ticks, and acts [210, 270]. Idling first and walking
        // afterwards would give 210 + 180 + 60 = 450.
        let walk = result
            .steps
            .iter()
            .find(|s| matches!(s.what, StepKind::Walk { .. }))
            .expect("the bot must walk");
        assert_eq!((walk.start, walk.end), (10, 190));
        let act = result
            .steps
            .iter()
            .find(|s| matches!(&s.what, StepKind::Act { action, .. } if *action == remove))
            .expect("the removal is scheduled");
        assert_eq!((act.start, act.end), (210, 270));
        assert_eq!(result.makespan, 270);
    }

    #[test]
    fn a_consumer_goes_to_the_bot_that_holds_the_items() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(100., 0.));

        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Producing happens at the origin, where only bot 1 stands.
        let mut producer = at_for(&mut gen, "produce", Position::new(0., 0.), 3.0, 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-ore".into(),
            count: 5,
        }];
        // Consuming happens where only bot 2 stands, but needs the producer's ore.
        let mut consumer = at_for(&mut gen, "consume", Position::new(100., 0.), 3.0, 10);
        consumer.pre.push(Condition::HasItem {
            who: Actor::Role,
            item: "iron-ore".into(),
            count: 5,
        });

        let p = net.add(producer);
        let c = net.add(consumer);
        net.link(p, c, 0);

        let result = schedule(&net, &s, &bots).expect("one bot can do both");

        assert_eq!(result.assignment(p), Some(BotId(1)));
        // Bot 2 is the cheapest by time — zero travel, so it would finish at 20
        // against bot 1's 667 — but it has no ore, so it is not a candidate at
        // all. Ranking without feasibility would bind it and fail the plan.
        assert_eq!(result.assignment(c), Some(BotId(1)));
        // Bot 1 walks 97 tiles: ceil(97 / 0.15) = 647, then acts for 10.
        assert_eq!(result.makespan, 667);
    }

    #[test]
    fn a_branching_chain_lands_wholly_on_one_bot() {
        use crate::ids::ChainIdGen;
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Two independent producers of *different* items — a branching chain
        // has two roots, and neither carries a `HasItem` precondition that
        // could keep it near the other. The consumer needs both, from one
        // inventory, because `Actor::Role` binds to the single bot that runs
        // it.
        let mut iron = free(&mut gen, "make iron", 10);
        iron.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }];
        let mut copper = free(&mut gen, "make copper", 10);
        copper.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "copper-plate".into(),
            count: 1,
        }];
        let mut consumer = free(&mut gen, "craft the pack", 10);
        consumer.pre = vec![
            Condition::HasItem {
                who: Actor::Role,
                item: "iron-plate".into(),
                count: 1,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: "copper-plate".into(),
                count: 1,
            },
        ];

        let i = net.add(iron);
        let c = net.add(copper);
        let pack = net.add(consumer);
        net.link(i, pack, 0);
        net.link(c, pack, 0);

        let mut chains = ChainIdGen::new();
        let chain = chains.next();
        for id in [i, c, pack] {
            net.set_chain(id, chain);
        }

        let bots = [BotId(1), BotId(2)];
        let result =
            schedule(&net, &state(&bots), &bots).expect("a chain bound to one bot is schedulable");

        // Both bots are idle at the origin, so without chain binding the two
        // roots are equally cheap and the greedy rule spreads them: the second
        // producer's idle bot finishes at 10 against the first bot's 20. The
        // consumer then finds one plate on each bot and no bot with both.
        let bot = result
            .assignment(i)
            .expect("the iron producer is scheduled");
        assert_eq!(result.assignment(c), Some(bot), "both roots on one bot");
        assert_eq!(result.assignment(pack), Some(bot), "and the consumer too");
    }

    #[test]
    fn a_new_chain_prefers_a_bot_that_is_not_carrying_one() {
        use crate::ids::ChainIdGen;
        // The shape of `travel_cost_can_outweigh_an_idle_bot`: bot 2 is parked
        // 200 tiles from the work, so greedy hands *both* actions to bot 1
        // (1200 beats 1914). Make them the roots of two different chains and
        // that piles two chains onto one bot — which for two smelting chains
        // means four furnaces from a stock of two, with no recovery, because
        // binding leaves each later action a single candidate.
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(200., 0.));

        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut gen, "second", Position::new(0., 0.), 3.0, 600));

        let mut chains = ChainIdGen::new();
        net.set_chain(first, chains.next());
        net.set_chain(second, chains.next());

        let result = schedule(&net, &s, &bots).unwrap();

        assert_eq!(result.assignment(first), Some(BotId(1)));
        assert_eq!(
            result.assignment(second),
            Some(BotId(2)),
            "the second chain must open on the bot carrying none, dear though it is"
        );
        // The price of that: bot 2 walks 197 tiles (1314 ticks) and finishes at
        // 1914, where piling both onto bot 1 would have finished at 1200. The
        // chainless version of this network is `travel_cost_can_outweigh_an_idle_bot`,
        // which still asserts 1200 — the preference applies only to chains.
        assert_eq!(result.makespan, 1914);
    }

    #[test]
    fn a_chain_bound_elsewhere_refuses_a_contradicting_pin() {
        use crate::ids::ChainIdGen;
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(free(&mut gen, "opens the chain", 10));
        let mut pinned = free(&mut gen, "pinned elsewhere", 10);
        pinned.pinned = Some(BotId(2));
        let second = net.add(pinned);
        net.link(first, second, 0);

        let mut chains = ChainIdGen::new();
        let chain = chains.next();
        net.set_chain(first, chain);
        net.set_chain(second, chain);

        let bots = [BotId(1), BotId(2)];
        // The chain opens on bot 1 (ties break on the lower id), so the pin to
        // bot 2 contradicts it. That is a contradiction in the plan, not a
        // surprise about the world, so it must not arrive as a
        // `PreconditionUnsatisfied` — re-planning from observed state would
        // meet the very same conflict again, forever.
        match schedule(&net, &state(&bots), &bots) {
            Err(PlannerError::ChainConflict {
                chain: c,
                action,
                bound_to,
                pinned_to,
            }) => {
                assert_eq!(c, chain);
                assert_eq!(action, second);
                assert_eq!(bound_to, BotId(1));
                assert_eq!(pinned_to, BotId(2));
            }
            other => panic!(
                "expected a ChainConflict, got {:?}",
                other.map(|s| s.makespan)
            ),
        }
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

    #[test]
    fn travel_cost_can_outweigh_an_idle_bot() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(200., 0.));

        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut gen, "second", Position::new(0., 0.), 3.0, 600));

        let result = schedule(&net, &s, &bots).unwrap();

        // Round 1: bot 1 is on the spot, finishing at 600.
        assert_eq!(result.assignment(first), Some(BotId(1)));
        // Round 2: bot 1 queues and finishes at 1200. Bot 2 is idle but must walk
        // 197 tiles: ceil(197 / 0.15) = 1314 travel, so it would finish at 1914.
        // A travel-blind scheduler would hand this to the idle bot and claim 600.
        assert_eq!(result.assignment(second), Some(BotId(1)));
        assert_eq!(result.makespan, 1200);
    }

    #[test]
    fn an_idle_bot_wins_when_its_travel_is_affordable() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(30., 0.));

        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut gen, "second", Position::new(0., 0.), 3.0, 600));

        let result = schedule(&net, &s, &bots).unwrap();

        assert_eq!(result.assignment(first), Some(BotId(1)));
        // Bot 1 would queue to 1200. Bot 2 walks 27 tiles: ceil(27 / 0.15) = 180
        // travel, finishing at 780. The idle bot wins this time.
        assert_eq!(result.assignment(second), Some(BotId(2)));
        assert_eq!(result.makespan, 780);
    }

    #[test]
    fn ties_break_on_end_time_before_action_id() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Lower id, longer duration. Higher id, shorter duration. One bot.
        let long = net.add(free(&mut gen, "long", 100));
        let short = net.add(free(&mut gen, "short", 10));
        let bots = [BotId(1)];

        let result = schedule(&net, &state(&bots), &bots).unwrap();

        let order: Vec<_> = result
            .steps
            .iter()
            .map(|s| match &s.what {
                StepKind::Act { action, .. } => *action,
                StepKind::Walk { .. } => panic!("no walks in this scenario"),
            })
            .collect();
        // With (end, action.id, bot) the shorter action is picked first despite its
        // higher id. With (action.id, end, bot) the order would be reversed.
        assert_eq!(order, vec![short, long]);
        assert_eq!(result.makespan, 110);
    }
}
