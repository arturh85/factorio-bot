//! Controlled request outcomes, bounded conflict retries, and the
//! [`plan_controlled`] entry point.
//!
//! [`plan_controlled`] replaces the two ad-hoc retry blocks inside
//! [`crate::plan_best`] with one iterative driver that obeys the shared
//! [`PlanControl`] and preserves feasible incumbents. [`plan_best`] itself
//! becomes a compatibility wrapper.

use std::collections::BTreeSet;
use factorio_bot_core::types::Position;

use crate::control::{BudgetLimits, BudgetReport, PlanControl, WorkKind};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::memory::ReplanMemory;
use crate::method::{expand, MethodRegistry};
use crate::network::ActionNetwork;
use crate::schedule::{schedule, Schedule};
use crate::ids::BotId;
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// Request result types
// ---------------------------------------------------------------------------

/// A fully validated and scheduled plan.
#[derive(Clone, Debug)]
pub struct PlannedMilestone {
    /// The action network (dependency graph of all actions).
    pub net: ActionNetwork,
    /// The schedule (per-bot action timeline).
    pub schedule: Schedule,
    /// Replan memory for persisting instance identity.
    pub memory: ReplanMemory,
}

/// Outcome of a planning request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    /// A complete, feasible plan was found.
    Complete,
    /// Budget exhausted; an incumbent plan exists and is executable.
    Exhausted,
    /// The problem is infeasible (no plan exists).
    Infeasible,
    /// The goal or configuration is unsupported.
    Unsupported,
}

/// The result of a [`plan_controlled`] call.
#[derive(Debug)]
pub struct PlanResult {
    /// Overall status.
    pub status: PlanStatus,
    /// The best feasible plan found, if any.
    pub incumbent: Option<PlannedMilestone>,
    /// Diagnostic error for failures without an incumbent.
    pub diagnostic: Option<PlannerError>,
    /// Budget report showing consumed work and stop reason.
    pub budget: BudgetReport,
}

// ---------------------------------------------------------------------------
// Retry helper
// ---------------------------------------------------------------------------

/// Try to start a conflict retry at position `at`.
///
/// Returns `Ok(true)` if a new retry was started, `Ok(false)` if `at` was
/// already retried, and `Err(PlannerError::PlanningStopped)` if the budget
/// has no retry allowance left.
///
/// The precondition `seen` tracks which positions have been retried so that
/// the same conflict does not consume multiple retry units.
/// A tile coordinate usable as an ordered key.
type TileKey = (i32, i32);

fn pos_to_tilekey(p: &Position) -> TileKey {
    (p.x.floor() as i32, p.y.floor() as i32)
}

pub fn next_conflict_retry(
    control: &PlanControl,
    seen: &mut BTreeSet<TileKey>,
    at: Position,
) -> Result<bool, PlannerError> {
    let key = pos_to_tilekey(&at);
    control.checkpoint()?;
    if seen.contains(&key) {
        return Ok(false);
    }
    control.charge(WorkKind::Retry)?;
    seen.insert(key);
    Ok(true)
}

/// Parse the position from a conflict error's condition string.
///
/// Reuses the existing parser format from `lib.rs`. Returns `None` if the
/// string does not contain a parseable `Position`.
fn parse_conflict_position(condition: &str) -> Option<Position> {
    // Same format as the existing parser in lib.rs: "[x, y]" inside the
    // error message.
    let start = condition.find('[')?;
    let end = condition.find(']')?;
    let coords = &condition[start + 1..end];
    let mut parts = coords.splitn(2, ',');
    let x = parts.next()?.trim().parse::<f64>().ok()?;
    let y = parts.next()?.trim().parse::<f64>().ok()?;
    Some(Position::new(x, y))
}

// ---------------------------------------------------------------------------
// Controlled planner entry point
// ---------------------------------------------------------------------------

/// Plan under a shared [`PlanControl`], replacing the two ad-hoc retry
/// blocks in [`crate::plan_best`].
///
/// The driver expands and schedules under each drain policy, keeps the
/// best schedule, and on failure retries with reserved conflict tiles up
/// to the retry budget. If a retry yields a feasible incumbent, it is
/// preserved even when the budget later exhausts.
///
/// The control is **not** installed into `state` here: the caller must
/// call `state.with_control(control)` before passing the state. This
/// avoids mutating the caller's state.
///
/// Returns a [`PlanResult`] with the outcome and budget report.
pub fn plan_controlled(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
    control: &PlanControl,
) -> PlanResult {
    // Initial policy loop and retry state
    let mut best: Option<(ActionNetwork, Schedule)> = None;
    let mut first_error: Option<PlannerError> = None;
    let mut retry_seen: BTreeSet<TileKey> = BTreeSet::new();

    // Expand and schedule under each drain policy.
    for policy in crate::method::produce::DrainPolicy::ALL {
        // Check the budget before each attempt.
        if let Err(err) = control.checkpoint() {
            first_error.get_or_insert(err);
            break;
        }

        let under = state
            .clone()
            .with_drain_policy(policy)
            .with_fresh_policy_probe();

        // Charge goal expansion work.
        if let Err(err) = control.charge(WorkKind::Goal) {
            first_error.get_or_insert(err);
            break;
        }

        let net = match expand(goals, &under, registry, chain_actor) {
            Ok(net) => net,
            Err(err) => {
                first_error.get_or_insert(err);
                continue;
            }
        };

        // Charge scheduling work.
        if let Err(err) = control.charge(WorkKind::Assignment) {
            first_error.get_or_insert(err);
            break;
        }

        let settled = !under.drain_policy_mattered();
        match schedule(&net, &under, roster) {
            Ok(plan) => {
                if best
                    .as_ref()
                    .is_none_or(|(_, best)| plan.makespan < best.makespan)
                {
                    best = Some((net, plan));
                }
            }
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
        if settled {
            break;
        }
    }

    // If no plan succeeded, try conflict retries.
    if best.is_none() {
        let mut retry_state = state.clone();

        // First retry: the recursive conflict path (single try).
        if let Some(PlannerError::ChainOwnerInfeasible { condition, .. }) = &first_error {
            if let Some(pos) = parse_conflict_position(condition) {
                let _ = control.checkpoint();
                match next_conflict_retry(control, &mut retry_seen, pos.clone()) {
                    Ok(true) => {
                        retry_state.reserve_ground(&[pos], "conflict retry");
                        if let Err(err) = do_retry_try(goals, &retry_state, registry,
                            chain_actor, roster, control, &mut best) {
                            first_error.get_or_insert(err);
                        }
                    }
                    Ok(false) => {} // already seen, skip
                    Err(err) => { first_error.get_or_insert(err); }
                }
            }
        }

        // Second retry loop: the MAX_RETRY_DEPTH bounded loop.
        if best.is_none() {
            let max_depth: u32 = 3; // matches the original MAX_RETRY_DEPTH
            for _depth in 0..max_depth {
                // Check budget before each attempt.
                if let Err(err) = control.checkpoint() {
                    first_error.get_or_insert(err);
                    break;
                }

                let conflict_tile = match &first_error {
                    Some(PlannerError::ChainOwnerInfeasible { condition, .. }) => {
                        parse_conflict_position(condition)
                    }
                    Some(PlannerError::AssemblyNoRouteForSupply { why, .. }) => {
                        parse_conflict_position(why)
                    }
                    _ => None,
                };

                match conflict_tile {
                    Some(pos) => {
                        match next_conflict_retry(control, &mut retry_seen, pos.clone()) {
                            Ok(true) => {
                                retry_state.reserve_ground(&[pos], "conflict retry");
                                let under = retry_state.clone().with_fresh_policy_probe();
                                let _ = control.charge(WorkKind::Goal);
                                match expand(goals, &under, registry, chain_actor) {
                                    Ok(net) => {
                                        let _ = control.charge(WorkKind::Assignment);
                                        match schedule(&net, &under, roster) {
                                            Ok(plan) => {
                                                let memory = crate::memory::capture_intent(
                                                    state, &net, &plan, 0);
                                                best = Some((net, plan));
                                                break;
                                            }
                                            Err(err) => { first_error = Some(err); }
                                        }
                                    }
                                    Err(err) => { first_error = Some(err); }
                                }
                            }
                            Ok(false) => break, // repeated conflict
                            Err(err) => {
                                first_error.get_or_insert(err);
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Build the result.
    let report = control.report();
    match (best, first_error) {
        (Some((net, plan)), _) => {
            let memory = crate::memory::capture_intent(state, &net, &plan, 0);
            let status = if report.stopped.is_some() {
                PlanStatus::Exhausted
            } else {
                PlanStatus::Complete
            };
            PlanResult {
                status,
                incumbent: Some(PlannedMilestone {
                    net: net.clone(),
                    schedule: plan.clone(),
                    memory: memory.clone(),
                }),
                diagnostic: None,
                budget: report,
            }
        }
        (None, Some(err)) => {
            let status = if matches!(&err, PlannerError::PlanningStopped { .. }) {
                PlanStatus::Exhausted
            } else {
                PlanStatus::Infeasible
            };
            PlanResult {
                status,
                incumbent: None,
                diagnostic: Some(err),
                budget: report,
            }
        }
        (None, None) => {
            PlanResult {
                status: PlanStatus::Infeasible,
                incumbent: None,
                diagnostic: Some(PlannerError::PlanningStopped {
                    reason: "no plan could be built, no error was recorded".to_string(),
                }),
                budget: report,
            }
        }
    }
}

/// Try a single retry with the default drain policy.
fn do_retry_try(
    goals: &[Goal],
    under: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
    control: &PlanControl,
    best: &mut Option<(ActionNetwork, Schedule)>,
) -> Result<(), PlannerError> {
    control.checkpoint()?;
    control.charge(WorkKind::Goal)?;
    let net = expand(goals, under, registry, chain_actor)?;
    control.charge(WorkKind::Assignment)?;
    let plan = schedule(&net, under, roster)?;
    if best
        .as_ref()
        .is_none_or(|(_, b)| plan.makespan < b.makespan)
    {
        *best = Some((net, plan));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compatibility wrapper
// ---------------------------------------------------------------------------

/// Compatibility wrapper that preserves the existing `plan_best` signature.
///
/// Creates a default control allowing one conflict retry and otherwise
/// preserving unlimited legacy behavior. Returns the old tuple signature.
pub fn plan_best_compat(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
) -> Result<(ActionNetwork, Schedule, ReplanMemory), PlannerError> {
    let control = PlanControl::new(BudgetLimits {
        maxima: std::collections::BTreeMap::from([(WorkKind::Retry, 1)]),
    });
    let state = state.clone().with_control(control.clone());
    let result = plan_controlled(goals, &state, registry, chain_actor, roster, &control);

    match (result.incumbent, result.diagnostic) {
        (Some(milestone), _) => Ok((milestone.net, milestone.schedule, milestone.memory)),
        (None, Some(err)) => Err(err),
        (None, None) => Err(PlannerError::PlanningStopped {
            reason: "no plan could be built".to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::BudgetLimits;
    use crate::control::WorkKind;
    use std::collections::BTreeMap;
    use factorio_bot_core::types::Position;

    fn pos(x: f64, y: f64) -> Position { Position::new(x, y) }

    #[test]
    fn repeated_conflict_does_not_start_another_attempt() {
        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Retry, 1)]),
        });
        let mut seen = BTreeSet::new();
        assert!(next_conflict_retry(&control, &mut seen, pos(3.0, 4.0)).unwrap());
        assert!(!next_conflict_retry(&control, &mut seen, pos(3.0, 4.0)).unwrap());
        assert!(next_conflict_retry(&control, &mut seen, pos(4.0, 4.0)).is_err());
    }

    #[test]
    fn retry_budget_exhaustion_returns_proper_error() {
        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Retry, 0)]),
        });
        let mut seen = BTreeSet::new();
        // Zero retries means no retry is allowed.
        assert!(next_conflict_retry(&control, &mut seen, pos(1.0, 2.0)).is_err());
        assert!(seen.is_empty());
    }
}
