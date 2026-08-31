//! Assignment of a bot-free action network to concrete bots over time.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ChainId, Ticks};
use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::Position;
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StepKind {
    Act {
        action: ActionId,
        label: String,
    },
    /// Go stand within `radius` of `to`.
    ///
    /// **`to` is the thing to get near, not a tile to occupy.** Both fields
    /// come from the `Condition::AtPosition` this walk exists to satisfy, so
    /// `to` is routinely a position the bot can never stand on — the tile an
    /// insert's furnace sits on, or the ore a mine consumes. Anywhere within
    /// `radius` of it satisfies the condition, and the actuator is expected to
    /// aim for the ring rather than the centre.
    ///
    /// `radius` used to be dropped here while `travel_ticks` went on using it,
    /// which made every such walk execute as "stand exactly on it". The
    /// pathfinder cannot route onto an occupied tile, so it silently
    /// substituted a goal of its own and the bot ended up wherever that
    /// happened to be — the 2026-08-30 live smelt run's walk `s4`, 9.3 tiles
    /// from where the plan believed it stood.
    Walk {
        to: Position,
        radius: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduledStep {
    pub what: StepKind,
    pub bot: BotId,
    pub start: Ticks,
    pub end: Ticks,
}

/// An immutable assignment of actions to bots over time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Set when the rejected candidate was the sole candidate of a
    /// caller-owned chain, so the final error can name the caller's
    /// instruction as the cause instead of reporting a bare precondition
    /// failure.
    owned_chain: Option<ChainId>,
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
/// up on a bot that is merely nearby — a *preference*, offered as a first tier
/// and abandoned for the rest of the roster when no bot in it can feasibly run
/// the action.
///
/// Binding only ever *reorders and narrows the candidate set*: how a candidate
/// is ranked and judged feasible is unchanged, and no preference can leave an
/// action with no candidate at all. Actions belonging to no chain stay
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

            // The chain this action belongs to, the bot already running it,
            // and the bot a caller pinned it to, if any.
            let chain = net.chain_of(action.id);
            let bound = chain.and_then(|c| chain_binding.get(&c).copied());
            let owner = chain.and_then(|c| net.owner_of(c));

            // Candidate bots in preference tiers, tried in order. A later tier
            // is reached only when no bot in an earlier one can *feasibly* run
            // the action, so a preference can cost time but can never cost a
            // plan: ranking-then-checking is what the spec forbids, and a
            // preference with no fallback is that same mistake wearing a
            // narrower candidate set.
            let candidate_tiers: Vec<Vec<BotId>> = match action.pinned {
                Some(pinned) if !bots.contains(&pinned) => {
                    return Err(PlannerError::UnknownBot(pinned))
                }
                // Pinning takes precedence over a chain *binding*: the pin is an
                // explicit instruction, the binding an inference. But rather
                // than silently overriding it — which would hand a chain's
                // items to a bot that does not hold them, and fail later with a
                // confusing precondition error somewhere else — a contradiction
                // is reported here, where its cause is still visible. Nothing
                // sets `pinned` today, so this is defensive.
                //
                // A chain *owner* is checked first and on the same footing. An
                // owner is the same class of thing as a pin — a caller's
                // instruction — and the harder of the two: it is stated before
                // scheduling begins and gets no fallback tier. Letting a pin
                // quietly win over it would run the action on a bot the caller
                // did not name while the rest of the chain went where the
                // caller asked, which is precisely the silent override this
                // arm exists to prevent.
                Some(pinned) => {
                    if let (Some(chain), Some(owner)) = (chain, owner) {
                        if owner != pinned {
                            return Err(PlannerError::ChainConflict {
                                chain,
                                action: action.id,
                                bound_to: owner,
                                pinned_to: pinned,
                            });
                        }
                    }
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
                    // A pin is an instruction, not a preference: there is no
                    // second tier to fall back to.
                    vec![vec![pinned]]
                }
                // A chain owner is a caller's instruction, so it is a hard
                // constraint: no tier falls back past it. The spread
                // preference below is a preference precisely because nobody
                // asked for it.
                None if owner.is_some() => vec![vec![owner.expect("just checked")]],
                // An action already in a running chain follows it. Also not a
                // preference — the items are in that bot's inventory and
                // nowhere else.
                None if bound.is_some() => vec![vec![bound.expect("just checked")]],
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
                // Bots already carrying a chain are the *second* tier, not
                // excluded: the opening action may need something only one of
                // them holds, and offering it nobody is how a preference turns
                // into a failed plan. An action in no chain at all is
                // unaffected — it has nothing to keep together, so it keeps the
                // whole roster in one tier and the path it took before chains
                // existed.
                None if chain.is_some() => {
                    let busy: BTreeSet<BotId> = chain_binding.values().copied().collect();
                    let (carrying, free): (Vec<BotId>, Vec<BotId>) =
                        bots.iter().copied().partition(|b| busy.contains(b));
                    [free, carrying]
                        .into_iter()
                        .filter(|tier| !tier.is_empty())
                        .collect()
                }
                None => vec![bots.to_vec()],
            };

            // Feasibility is judged per action and per tier: another action
            // finding a bot in tier one says nothing about this one.
            let mut feasible_in_an_earlier_tier = false;
            for tier in &candidate_tiers {
                if feasible_in_an_earlier_tier {
                    break;
                }
                for bot in tier.iter().copied() {
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
                        // The radius is deliberately dropped *here* and only
                        // here. The simulated arrival is the centre, which
                        // satisfies the condition for every radius and so is
                        // the one point that cannot make a feasible pair look
                        // infeasible. Naming a concrete point on the ring
                        // instead would need to know which points are
                        // walkable — terrain, water, cliffs, other players —
                        // and `PlanState` knows only what entities occupy.
                        // A guess there would turn an optimistic estimate into
                        // a confidently wrong one, so the ring is resolved
                        // where the knowledge is: by the game's pathfinder,
                        // from the radius `StepKind::Walk` now carries.
                        if let Some((pos, _)) = action.required_position() {
                            trial.set_position(bot, pos);
                        }
                    }
                    let failing = action.pre.iter().find(|c| !c.holds(&trial, bot));

                    match failing {
                        None => {
                            // This tier can run the action, so no later tier is
                            // consulted for it — that is what makes the
                            // preference a preference and not a restriction.
                            feasible_in_an_earlier_tier = true;
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
                                    // `owner` names this action's chain owner
                                    // when it has one; pair it with `chain`
                                    // (guaranteed `Some` whenever `owner` is)
                                    // so a caller's instruction is what the
                                    // final error blames, not a bare
                                    // precondition.
                                    owned_chain: owner.and(chain),
                                    candidate,
                                });
                            }
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
                return Err(match rejected.owned_chain {
                    // A caller named this bot, so a precondition that fails
                    // for it is not "the world was not as planned" — it is
                    // the caller's own instruction that cannot be met.
                    Some(chain) => PlannerError::ChainOwnerInfeasible {
                        chain,
                        action: rejected.candidate.action,
                        bot: rejected.candidate.bot,
                        condition: rejected.condition,
                    },
                    None => PlannerError::PreconditionUnsatisfied {
                        action: rejected.candidate.action,
                        bot: rejected.candidate.bot,
                        condition: rejected.condition,
                    },
                });
            }
        };
        let action = net
            .action(chosen.action)
            .expect("candidate came from this network");

        // Walk first, so the AtPosition precondition holds by `act_start`. The
        // walk may finish well before it, if the action waits on a lag.
        if chosen.travel > 0 {
            let (target, radius) = action
                .required_position()
                .expect("travel is non-zero only when a position is required");
            steps.push(ScheduledStep {
                what: StepKind::Walk {
                    to: target.clone(),
                    radius,
                },
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
    fn free(id_gen: &mut ActionIdGen, label: &str, duration: Ticks) -> Action {
        Action {
            id: id_gen.next(),
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
    fn at(id_gen: &mut ActionIdGen, label: &str, pos: Position, radius: f64) -> Action {
        Action {
            id: id_gen.next(),
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
        id_gen: &mut ActionIdGen,
        label: &str,
        pos: Position,
        radius: f64,
        duration: Ticks,
    ) -> Action {
        Action {
            id: id_gen.next(),
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
    fn a_schedule_survives_a_json_round_trip() {
        // A schedule is the artefact the next increment serves over HTTP.
        use factorio_bot_core::serde_json;
        let plan = Schedule {
            steps: vec![
                ScheduledStep {
                    what: StepKind::Walk {
                        to: Position::new(10., 20.),
                        radius: 3.0,
                    },
                    bot: BotId(1),
                    start: 0,
                    end: 60,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(3),
                        label: "mine 5 iron-ore".into(),
                    },
                    bot: BotId(2),
                    start: 60,
                    end: 360,
                },
            ],
            makespan: 360,
        };
        let json = serde_json::to_string(&plan).expect("serialises");
        let back: Schedule = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, plan);
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
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut id_gen, "a", 100));
        net.add(free(&mut id_gen, "b", 100));
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 100, "two bots should overlap the work");
    }

    #[test]
    fn one_bot_serialises_the_same_actions() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut id_gen, "a", 100));
        net.add(free(&mut id_gen, "b", 100));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 200);
    }

    #[test]
    fn a_dependency_lag_delays_the_successor() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(free(&mut id_gen, "insert", 10));
        let b = net.add(free(&mut id_gen, "remove", 10));
        net.link(a, b, 200);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        // insert ends at 10, lag 200 -> remove starts no earlier than 210.
        assert_eq!(result.makespan, 220);
    }

    #[test]
    fn a_walk_is_emitted_when_the_bot_is_out_of_range() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "far", Position::new(30., 0.), 3.0));
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

    /// The tolerance a walk exists to satisfy travels with the walk.
    ///
    /// `Condition::AtPosition` carries a radius and `travel_ticks` has always
    /// used it, but the emitted step used to carry only `to` — so "stand within
    /// 10 tiles of the furnace" reached the executor as "stand on the furnace's
    /// tile", and the game's pathfinder was asked for a tile the plan had just
    /// built on.
    ///
    /// Two actions with **different** radii, not one, because a single walk
    /// cannot tell a radius read from its condition from any constant that
    /// happens to match.
    #[test]
    fn a_walk_carries_the_radius_of_the_condition_it_satisfies() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "wide", Position::new(30., 0.), 9.5));
        net.add(at(&mut id_gen, "tight", Position::new(-30., 0.), 2.25));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();

        let radii: Vec<f64> = result
            .steps
            .iter()
            .filter_map(|s| match &s.what {
                StepKind::Walk { radius, .. } => Some(*radius),
                StepKind::Act { .. } => None,
            })
            .collect();
        assert_eq!(radii.len(), 2, "one walk per action: {:?}", result.steps);
        let mut sorted = radii.clone();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(
            sorted,
            vec![2.25, 9.5],
            "each walk carries its own action's radius, not a shared constant"
        );
    }

    #[test]
    fn no_walk_is_emitted_when_already_in_range() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "near", Position::new(1., 1.), 5.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.makespan, 60);
    }

    #[test]
    fn a_pinned_action_goes_to_its_bot() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut id_gen, "pinned", 60);
        action.pinned = Some(BotId(2));
        let id = net.add(action);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(id), Some(BotId(2)));
    }

    #[test]
    fn an_unsatisfiable_precondition_is_an_error() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut id_gen, "needs plates", 60);
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
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut producer = free(&mut id_gen, "produce", 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let mut consumer = free(&mut id_gen, "consume", 10);
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
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Insert the ore, then remove the plate 200 ticks later, 30 tiles away.
        let insert = net.add(free(&mut id_gen, "insert", 10));
        let remove = net.add(at_for(&mut id_gen, "remove", Position::new(30., 0.), 3.0, 60));
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

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Producing happens at the origin, where only bot 1 stands.
        let mut producer = at_for(&mut id_gen, "produce", Position::new(0., 0.), 3.0, 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-ore".into(),
            count: 5,
        }];
        // Consuming happens where only bot 2 stands, but needs the producer's ore.
        let mut consumer = at_for(&mut id_gen, "consume", Position::new(100., 0.), 3.0, 10);
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
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Two independent producers of *different* items — a branching chain
        // has two roots, and neither carries a `HasItem` precondition that
        // could keep it near the other. The consumer needs both, from one
        // inventory, because `Actor::Role` binds to the single bot that runs
        // it.
        let mut iron = free(&mut id_gen, "make iron", 10);
        iron.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }];
        let mut copper = free(&mut id_gen, "make copper", 10);
        copper.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "copper-plate".into(),
            count: 1,
        }];
        let mut consumer = free(&mut id_gen, "craft the pack", 10);
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

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut id_gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut id_gen, "second", Position::new(0., 0.), 3.0, 600));

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
    fn a_new_chain_takes_a_carrying_bot_when_no_free_bot_can_run_it() {
        use crate::ids::ChainIdGen;
        // The preference must stay a preference. Bot 2 is free of chains and
        // idle at the same tile, so it wins the spread outright — but it does
        // not hold the furnace the second chain's opening action consumes.
        // Narrowing to the unbound bots and stopping there offers that action
        // nobody, and the whole plan fails on a precondition that bot 1 meets.
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 1);

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(free(&mut id_gen, "opens chain A", 10));
        let mut needs_furnace = free(&mut id_gen, "opens chain B", 10);
        needs_furnace.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "stone-furnace".into(),
            count: 1,
        }];
        needs_furnace.eff = vec![Effect::LoseItem {
            who: Actor::Role,
            item: "stone-furnace".into(),
            count: 1,
        }];
        let second = net.add(needs_furnace);

        let mut chains = ChainIdGen::new();
        net.set_chain(first, chains.next());
        net.set_chain(second, chains.next());

        let result = schedule(&net, &s, &bots).expect("bot 1 can run both chains");
        assert_eq!(result.assignment(first), Some(BotId(1)));
        assert_eq!(
            result.assignment(second),
            Some(BotId(1)),
            "the second chain falls back to the bot that holds the furnace"
        );
        // Bot 1 runs both, one after the other, at the origin.
        assert_eq!(result.makespan, 20);
    }

    #[test]
    fn a_chain_bound_elsewhere_refuses_a_contradicting_pin() {
        use crate::ids::ChainIdGen;
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(free(&mut id_gen, "opens the chain", 10));
        let mut pinned = free(&mut id_gen, "pinned elsewhere", 10);
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
    fn a_pin_that_contradicts_a_chain_owner_is_a_conflict() {
        // An owner is the same class of thing as a pin — a caller's
        // instruction — and the harder one, since it admits no fallback tier.
        // Before the owner was compared here the pin simply won: the action
        // ran on bot 1 while its chain belonged to bot 2, silently, with no
        // error anywhere.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut pinned = free(&mut id_gen, "pinned away from its owner", 10);
        pinned.pinned = Some(BotId(1));
        let action = net.add(pinned);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        let bots = [BotId(1), BotId(2)];
        match schedule(&net, &state(&bots), &bots) {
            Err(PlannerError::ChainConflict {
                chain: c,
                action: a,
                bound_to,
                pinned_to,
            }) => {
                assert_eq!(c, chain);
                assert_eq!(a, action);
                assert_eq!(
                    bound_to,
                    BotId(2),
                    "the owner is what the pin contradicts, not a binding"
                );
                assert_eq!(pinned_to, BotId(1));
            }
            other => panic!(
                "expected a ChainConflict, got {:?}",
                other.map(|s| s.makespan)
            ),
        }
    }

    #[test]
    fn a_pin_that_agrees_with_the_chain_owner_is_allowed() {
        // The companion to the test above: the check rejects contradiction,
        // not the presence of a pin on an owned chain.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut pinned = free(&mut id_gen, "pinned to its own owner", 10);
        pinned.pinned = Some(BotId(2));
        let action = net.add(pinned);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).expect("pin and owner agree");
        assert_eq!(result.assignment(action), Some(BotId(2)));
    }

    #[test]
    fn an_owner_that_cannot_run_its_chain_blames_the_caller_not_the_world() {
        // `PreconditionUnsatisfied` means "the world was not as planned", which
        // the spec answers by re-planning from observed state. A caller naming
        // a bot that cannot meet the chain's own precondition is not that: the
        // instruction itself is unsatisfiable, and re-planning meets it again.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut needs_ore = free(&mut id_gen, "the chain the caller asked for", 10);
        needs_ore.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "uranium-ore".into(),
            count: 1,
        }];
        let action = net.add(needs_ore);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        // Neither bot holds any, so this is not about bot 2 being unlucky —
        // but the owner tier means bot 2 is the only bot ever offered it.
        let bots = [BotId(1), BotId(2)];
        let err = schedule(&net, &state(&bots), &bots).expect_err("nobody holds uranium ore");
        assert_eq!(
            err.to_string(),
            "bot 2 owns chain ChainId(0) because a caller named it, \
             but has 1 uranium-ore does not hold there"
        );
        match err {
            PlannerError::ChainOwnerInfeasible {
                chain: c,
                action: a,
                bot,
                condition,
            } => {
                assert_eq!(c, chain);
                assert_eq!(
                    a, action,
                    "tier-1 recovery needs the action to re-plan around"
                );
                assert_eq!(bot, BotId(2));
                assert_eq!(condition, "has 1 uranium-ore");
            }
            other => panic!("expected a ChainOwnerInfeasible, got {:?}", other),
        }
    }

    #[test]
    fn scheduling_is_deterministic() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        for i in 0..6 {
            net.add(free(&mut id_gen, &format!("a{}", i), 40));
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

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut id_gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut id_gen, "second", Position::new(0., 0.), 3.0, 600));

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

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(&mut id_gen, "first", Position::new(0., 0.), 3.0, 600));
        let second = net.add(at_for(&mut id_gen, "second", Position::new(0., 0.), 3.0, 600));

        let result = schedule(&net, &s, &bots).unwrap();

        assert_eq!(result.assignment(first), Some(BotId(1)));
        // Bot 1 would queue to 1200. Bot 2 walks 27 tiles: ceil(27 / 0.15) = 180
        // travel, finishing at 780. The idle bot wins this time.
        assert_eq!(result.assignment(second), Some(BotId(2)));
        assert_eq!(result.makespan, 780);
    }

    #[test]
    fn ties_break_on_end_time_before_action_id() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Lower id, longer duration. Higher id, shorter duration. One bot.
        let long = net.add(free(&mut id_gen, "long", 100));
        let short = net.add(free(&mut id_gen, "short", 10));
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
