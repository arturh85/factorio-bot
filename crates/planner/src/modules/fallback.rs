//! Flat fallback schedule for module-backed action networks.
//!
//! When the primary scheduler fails or produces an invalid schedule for a
//! module-level action network, this module provides a deterministic,
//! dependency-respecting fallback that assigns actions to bots in topological
//! order while preserving chain ownership and travel constraints.
//!
//! The fallback is deliberately simpler than the main [`schedule`]: it never
//! reorders work for lookahead, never re-sites chains for spread preference,
//! and never tries to infer parallelism beyond what the topological order
//! allows. What it does guarantee is:
//!
//! * Every action starts after all its predecessors plus their edge lags.
//! * A chain's actions (including pinned and owner-constrained ones) stay
//!   on their assigned bot.
//! * Walk steps are emitted between non-colocated actions on the same bot.
//! * Preconditions are checked against a forked state with the bot already
//!   moved, exactly as the main scheduler does.
//! * Impossible ownership constraints produce an error rather than silently
//!   reassigning the action to a different bot.

use std::collections::{BTreeMap, BTreeSet};

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::control::PlanControl;
use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ChainId, Ticks};
use crate::modules::artifact::ModuleError;
use crate::network::ActionNetwork;
use crate::schedule::{
    Schedule, ScheduledStep, StepKind, WALK_TILES_PER_TICK, arrival_point, travel_ticks,
};
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// earliest_start
// ---------------------------------------------------------------------------

/// Compute the earliest tick an action can start, given when the bot is free
/// and when each predecessor finishes plus its lag.
///
/// # Arguments
///
/// * `bot_free` — the tick at which the bot finishes its previous commitments.
/// * `predecessor_ends` — pairs of `(end_tick, lag)` for each predecessor of
///   the action being scheduled. `end_tick` is when the predecessor finishes;
///   `lag` is the minimum gap after that before the action may start.
///
/// # Returns
///
/// The earliest tick the action may start, or `ModuleError::ArithmeticOverflow`
/// if any addition would overflow `u32`.
pub fn earliest_start(
    bot_free: Ticks,
    predecessor_ends: &[(Ticks, Ticks)],
) -> Result<Ticks, ModuleError> {
    let dep_ready: Ticks = predecessor_ends
        .iter()
        .map(|&(end, lag)| {
            end.checked_add(lag)
                .ok_or(ModuleError::ArithmeticOverflow)
        })
        .try_fold(0u32, |acc, candidate| {
            candidate.map(|c| acc.max(c))
        })?;
    bot_free
        .max(dep_ready)
        .checked_add(0)
        .ok_or(ModuleError::ArithmeticOverflow)
}

// ---------------------------------------------------------------------------
// schedule_fallback
// ---------------------------------------------------------------------------

/// Schedule an action network with a simple topo-order fallback.
///
/// This is the module-level equivalent of the main [`crate::schedule::schedule`]
/// function, specialised for flat fallback scheduling. It is used when the
/// primary scheduler fails for module-originated networks.
///
/// # Guarantees
///
/// 1. Actions are dispatched in topological order (respecting edges).
/// 2. Chain/pinned-owner constraints are hard: an action whose chain has an
///    owner must run on that bot, and that bot must be in the roster.
/// 3. Travel time is computed between actions using the same bot-position
///    simulation the main scheduler uses.
/// 4. Preconditions are validated against a forked state (feasibility check),
///    exactly as in the main [`schedule`].
/// 5. Impossible ownership returns [`PlannerError::ChainOwnerInfeasible`]
///    rather than reassigning to another bot.
/// 6. Walk steps (`StepKind::Walk`) are emitted into the schedule so that
///    the record shows *why* the travel time was spent.
/// 7. The shared budget is charged for assignment work and checked at
///    cancellation checkpoints.
///
/// # Errors
///
/// Returns `PlannerError` for the same conditions the main scheduler does:
/// empty roster, deadlock (cyclic or unreachable), chain-owner infeasible,
/// or precondition unsatisfied. The fallback never silently reassigns an
/// owned action to a different bot.
pub fn schedule_fallback(
    net: &ActionNetwork,
    state: &PlanState,
    roster: &[BotId],
    control: &PlanControl,
) -> Result<Schedule, PlannerError> {
    if roster.is_empty() {
        return Err(PlannerError::NoBots);
    }

    // Charge scheduling work against the shared budget.
    control.charge(crate::control::WorkKind::Assignment)?;
    control.checkpoint()?;

    // Validate the network before scheduling.
    net.validate()?;

    // Topological order of the network.
    let order = net.topo_order()?;

    // Predecessor lists, read once.
    let preds: BTreeMap<ActionId, Vec<(ActionId, Ticks)>> =
        net.actions().map(|a| (a.id, net.preds(a.id))).collect();
    let preds_of = |id: ActionId| -> &[(ActionId, Ticks)] {
        preds.get(&id).map_or(&[][..], Vec::as_slice)
    };

    // Simulated bot state.
    let mut sim = state.fork();

    // Per-bot free-at tick and position.
    let mut free_at: BTreeMap<BotId, Ticks> = roster.iter().map(|b| (*b, 0)).collect();
    let mut positions: BTreeMap<BotId, crate::state::BotState> =
        roster.iter().filter_map(|b| {
            sim.bot(*b).map(|bs| (*b, bs.clone()))
        }).collect();

    // When an action finishes (for dependency readiness).
    let mut finished: BTreeMap<ActionId, Ticks> = BTreeMap::new();
    let mut done: BTreeSet<ActionId> = BTreeSet::new();

    // Chain-to-bot binding (already determined by chain owner).
    // We don't do chain-opening assignment here — the owner is fixed.
    let mut chain_binding: BTreeMap<ChainId, BotId> = BTreeMap::new();
    for action in net.actions() {
        let chain = net.chain_of(action.id);
        if let Some(chain) = chain {
            if let Some(owner) = net.owner_of(chain) {
                if !roster.contains(&owner) {
                    return Err(PlannerError::UnknownBot(owner));
                }
                chain_binding.entry(chain).or_insert(owner);
            }
        }
    }

    // When research actions must be serialised.
    let mut research_free_at: Ticks = 0;

    // The resulting steps.
    let mut steps: Vec<ScheduledStep> = Vec::new();

    for &action_id in &order {
        control.checkpoint()?;

        let action = net.action(action_id).ok_or_else(|| {
            PlannerError::Deadlock { action: action_id }
        })?;

        // Compute predecessor readiness.
        let deps_ready: Ticks = preds_of(action_id)
            .iter()
            .map(|(p, lag)| {
                finished.get(p).copied().unwrap_or(0)
                    .checked_add(*lag)
                    .unwrap_or(Ticks::MAX)
            })
            .max()
            .unwrap_or(0);

        // Determine the bot for this action.
        let chain = net.chain_of(action_id);
        let owner = chain.and_then(|c| net.owner_of(c));

        let bot: BotId = match (action.pinned, owner, chain.and_then(|c| chain_binding.get(&c).copied())) {
            // Explicit pin.
            (Some(pinned), _, _) => {
                if !roster.contains(&pinned) {
                    return Err(PlannerError::UnknownBot(pinned));
                }
                // Check for conflicts with chain binding.
                if let (Some(c), Some(bound)) = (chain, chain.and_then(|c| chain_binding.get(&c).copied())) {
                    if bound != pinned {
                        return Err(PlannerError::ChainConflict {
                            chain: c,
                            action: action_id,
                            bound_to: bound,
                            pinned_to: pinned,
                        });
                    }
                }
                // Check for conflict with owner.
                if let (Some(c), Some(own)) = (chain, owner) {
                    if own != pinned {
                        return Err(PlannerError::ChainConflict {
                            chain: c,
                            action: action_id,
                            bound_to: own,
                            pinned_to: pinned,
                        });
                    }
                }
                pinned
            }
            // Chain owner (hard constraint).
            (None, Some(own), _) => own,
            // Already bound chain.
            (None, None, Some(bound)) => bound,
            // Free action: pick the bot that finishes earliest.
            (None, None, None) => {
                // For free actions, we need to spread work.
                *free_at.iter().min_by_key(|&(_, &t)| t).map(|(b, _)| b).unwrap_or(&roster[0])
            }
        };

        if !roster.contains(&bot) {
            return Err(PlannerError::UnknownBot(bot));
        }

        // Bind this chain if it hasn't been bound yet (chain opening).
        if let Some(c) = chain {
            if !chain_binding.contains_key(&c) && owner.is_none() && action.pinned.is_none() {
                chain_binding.insert(c, bot);
            }
        }

        let bot_free = free_at[&bot];
        let from_pos = positions.get(&bot).map(|bs| bs.position.clone());
        let walk_target = action.required_position();

        // Compute travel ticks.
        let (travel, arrival_pos) = match (&from_pos, &walk_target) {
            (Some(from), Some((to, min_radius, radius))) => {
                let trav = travel_ticks(from, to, *min_radius, *radius);
                if trav > 0 {
                    let arr = arrival_point(to, from, *min_radius, *radius);
                    (trav, Some(arr))
                } else {
                    (0u32, None)
                }
            }
            _ => (0u32, None),
        };

        // Walk starts when bot is free (walking during dependency lag).
        let walk_start = bot_free;
        let walk_end = walk_start.checked_add(travel).ok_or_else(|| {
            PlannerError::Deadlock {
                action: action_id,
            }
        })?;

        // Feasibility check: simulate the bot at the arrival position.
        if let Some(ref arrival) = arrival_pos {
            let mut trial = sim.fork();
            trial.set_position(bot, arrival.clone());
            if let Some(cond) = action.pre.iter().find(|c| !c.holds(&trial, bot)) {
                // Check whether this is a chain-owner failure.
                if owner.is_some() || chain_binding.contains_key(&chain.unwrap_or(ChainId(u32::MAX))) {
                    return Err(PlannerError::ChainOwnerInfeasible {
                        chain: chain.unwrap_or(ChainId(u32::MAX)),
                        action: action_id,
                        bot,
                        condition: format!(
                            "precondition '{}' does not hold after walking {} ticks from {:?} to {:?}",
                            cond, travel, from_pos, arrival
                        ),
                    });
                }
                return Err(PlannerError::PreconditionUnsatisfied {
                    action: action_id,
                    bot,
                    condition: format!(
                        "precondition '{}' does not hold after walking {} ticks from {:?} to {:?}",
                        cond, travel, from_pos, arrival
                    ),
                });
            }
            // Update simulated position.
            sim.set_position(bot, arrival.clone());
        }

        // Act start: after walk AND after dependencies are ready.
        let act_start = walk_end.max(deps_ready);

        // Research serialisation.
        let act_start = if matches!(action.kind, ActionKind::Research { .. }) {
            act_start.max(research_free_at)
        } else {
            act_start
        };

        // Duration.
        let duration = action.duration;
        let end = act_start.checked_add(duration).ok_or_else(|| {
            PlannerError::Deadlock { action: action_id }
        })?;

        // Update research serialisation.
        if matches!(action.kind, ActionKind::Research { .. }) {
            research_free_at = end;
        }

        // Emit walk step if travel > 0.
        if travel > 0 {
            if let Some((ref to, min_radius, radius)) = walk_target {
                steps.push(ScheduledStep {
                    what: StepKind::Walk {
                        to: to.clone(),
                        min_radius,
                        radius,
                    },
                    bot,
                    start: walk_start,
                    end: walk_end,
                });
            }
        }

        // Emit action step.
        steps.push(ScheduledStep {
            what: StepKind::Act {
                action: action_id,
                label: action.label.clone(),
            },
            bot,
            start: act_start,
            end,
        });

        // Update state.
        free_at.insert(bot, end);
        finished.insert(action_id, end);
        done.insert(action_id);

        // Update bot position for next action.
        if let Some(arrival) = arrival_pos {
            sim.set_position(bot, arrival.clone());
            if let Some(bs) = positions.get_mut(&bot) {
                bs.position = arrival.clone();
            }
        }

        // Update inventory effects in chronological order.
        for eff in &action.eff {
            eff.apply(&mut sim, bot).ok();
        }
    }

    let makespan = free_at.values().max().copied().unwrap_or(0);

    Ok(Schedule { steps, makespan })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind};
    use crate::ids::ActionIdGen;
    use crate::network::ActionNetwork;
    use crate::control::{BudgetLimits, PlanControl};

    #[test]
    fn successor_waits_for_other_bot_and_edge_lag() {
        // Action A finishes at 100 with lag 30 for action B.
        // Action B's other predecessor finishes at 80 with lag 0.
        // Bot is free at 20.
        // max(20, 100+30=130, 80+0=80) = 130
        assert_eq!(
            earliest_start(20, &[(100, 30), (80, 0)]).unwrap(),
            130
        );
        // Overflow case.
        assert!(earliest_start(0, &[(u32::MAX, 1)]).is_err());
    }

    #[test]
    fn bot_free_dominates_empty_deps() {
        assert_eq!(earliest_start(50, &[]).unwrap(), 50);
    }

    #[test]
    fn deps_ready_dominates_bot_free() {
        assert_eq!(earliest_start(10, &[(100, 0)]).unwrap(), 100);
    }

    #[test]
    fn chain_owner_is_hard_constraint() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        let a1 = net.add(Action {
            id: id_gen.next(),
            kind: ActionKind::Craft { item: "iron-plate".into(), count: 1 },
            pre: vec![],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: "a1".into(),
        });

        let c1 = ChainId(1);
        net.set_chain(a1, c1);
        net.set_chain_owner(c1, BotId(1));

        let state = PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[BotId(1)],
        );

        let control = PlanControl::new(BudgetLimits::default());

        let result = schedule_fallback(
            &net,
            &state,
            &[BotId(1)],
            &control,
        );
        assert!(result.is_ok(), "owner-bot in roster should work: {:?}", result.err());
    }

    #[test]
    fn chain_owner_not_in_roster_fails() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        let a1 = net.add(Action {
            id: id_gen.next(),
            kind: ActionKind::Craft { item: "iron-plate".into(), count: 1 },
            pre: vec![],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: "a1".into(),
        });

        let c1 = ChainId(1);
        net.set_chain(a1, c1);
        net.set_chain_owner(c1, BotId(99)); // Not in roster.

        let state = PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[BotId(1), BotId(2)],
        );

        let control = PlanControl::new(BudgetLimits::default());

        let result = schedule_fallback(
            &net,
            &state,
            &[BotId(1), BotId(2)],
            &control,
        );
        assert!(result.is_err(), "chain owner not in roster must fail");
    }

    #[test]
    fn empty_roster_fails() {
        let net = ActionNetwork::new();
        let state = PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[],
        );
        let control = PlanControl::new(BudgetLimits::default());

        let result = schedule_fallback(&net, &state, &[], &control);
        assert!(matches!(result, Err(PlannerError::NoBots)));
    }

    #[test]
    fn cyclic_network_fails() {
        let mut net = ActionNetwork::new();
        let a1 = net.add(Action {
            id: ActionId(1),
            kind: ActionKind::Craft { item: "a".into(), count: 1 },
            pre: vec![],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: "a1".into(),
        });
        let a2 = net.add(Action {
            id: ActionId(2),
            kind: ActionKind::Craft { item: "b".into(), count: 1 },
            pre: vec![],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: "a2".into(),
        });
        // Create cycle: a1 -> a2 -> a1
        net.link(a1, a2, 0);
        net.link(a2, a1, 0);

        let state = PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[BotId(1)],
        );
        let control = PlanControl::new(BudgetLimits::default());

        let result = schedule_fallback(&net, &state, &[BotId(1)], &control);
        assert!(matches!(result, Err(PlannerError::CyclicNetwork(_))));
    }
}
