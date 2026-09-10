//! Shared deterministic planning budgets, sticky stop reasons, and
//! phase counters.
//!
//! A [`PlanControl`] belongs to a top-level request and is shared by
//! expansion, placement, routing, scheduling, policy alternatives, and
//! recovery retries. Nested calls cannot reset it.
//!
//! All operations are deterministic given the same work limits and
//! cancellation check: wall time never enters the budget decision.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::PlannerError;

// ---------------------------------------------------------------------------
// Work kinds
// ---------------------------------------------------------------------------

/// A category of planning work that can be bounded by the shared budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkKind {
    /// Expanding a goal into its method calls and sub-goals.
    Goal,
    /// Searching for a feasible placement site.
    Site,
    /// Expanding a routing-node search.
    RouteNode,
    /// Checking action/bot feasibility during scheduling.
    Assignment,
    /// Evaluating a lookahead pair during scheduling.
    LookaheadPair,
    /// Scanning the graph (enclosure, route checks, etc.).
    GraphScan,
    /// A conflict retry (tile conflict, re-siting, etc.).
    Retry,
}

/// Phase of planning for observation and measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanPhase {
    Expansion,
    Placement,
    Routing,
    Scheduling,
    Recovery,
}

// ---------------------------------------------------------------------------
// Budget limits and reports
// ---------------------------------------------------------------------------

/// Limits on how many units of each [`WorkKind`] may be consumed.
///
/// A missing maximum means unbounded for that counter; zero means no
/// operation of that kind is permitted.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetLimits {
    pub maxima: BTreeMap<WorkKind, u64>,
}

/// Why a planning request stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// A work-kind counter hit its maximum.
    Limit(WorkKind),
    /// An external cancellation check returned true.
    Cancelled,
}
impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Limit(kind) => write!(f, "work limit reached for {:?}", kind),
            StopReason::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Snapshot of what has been consumed and whether the control has stopped.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetReport {
    /// Units consumed per work kind.
    pub used: BTreeMap<WorkKind, u64>,
    /// Why the control stopped, if it did.
    pub stopped: Option<StopReason>,
}

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ControlState {
    limits: BudgetLimits,
    report: BudgetReport,
    // Cancellation closure. Debug is implemented manually so this field does
    // not force `dyn Fn() -> bool` to be Debug.
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl std::fmt::Debug for ControlState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlState")
            .field("limits", &self.limits)
            .field("report", &self.report)
            .field("cancelled", &self.cancelled.is_some())
            .finish()
    }
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            limits: BudgetLimits::default(),
            report: BudgetReport::default(),
            cancelled: None,
        }
    }
}

impl ControlState {
    /// Charge one unit of `kind`. Returns an error if the budget is already
    /// exhausted, was already stopped, or this charge would exceed the
    /// maximum.
    fn charge(&mut self, kind: WorkKind) -> Result<(), PlannerError> {
        // If already stopped, return the existing reason.
        if let Some(ref reason) = self.report.stopped {
            let msg = match reason {
                StopReason::Limit(k) => format!("work limit reached for {:?}", k),
                StopReason::Cancelled => "cancelled".to_string(),
            };
            return Err(PlannerError::PlanningStopped { reason: msg });
        }

        let max = self.limits.maxima.get(&kind).copied();
        let current = self.report.used.get(&kind).copied().unwrap_or(0);

        // Reject if the limit would be exceeded.
        if let Some(max_val) = max {
            if current >= max_val {
                let reason = StopReason::Limit(kind);
                self.report.stopped = Some(reason);
                return Err(PlannerError::PlanningStopped {
                    reason: format!("work limit reached for {:?}", kind),
                });
            }
        }

        // Increment (insertion only on success).
        let entry = self.report.used.entry(kind).or_insert(0);
        *entry = entry.checked_add(1).ok_or_else(|| {
            let reason = StopReason::Limit(kind);
            self.report.stopped = Some(reason);
            PlannerError::PlanningStopped {
                reason: format!("work limit reached for {:?}", kind),
            }
        })?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PlanControl
// ---------------------------------------------------------------------------

/// Shared planning budget and cancellation latch.
///
/// Clone shares the underlying `Arc<Mutex<ControlState>>`, so a cloned
/// control sees the same budget and stop reason as the original.
#[derive(Clone)]
pub struct PlanControl {
    inner: Arc<Mutex<ControlState>>,
}

impl std::fmt::Debug for PlanControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format only the report and limits; skip the cancellation closure.
        if let Ok(state) = self.inner.lock() {
            f.debug_struct("PlanControl")
                .field("limits", &state.limits)
                .field("report", &state.report)
                .field("cancelled", &state.cancelled.is_some())
                .finish()
        } else {
            f.debug_struct("PlanControl")
                .field("inner", &"<locked>")
                .finish()
        }
    }
}

impl PlanControl {
    /// Create a new control with the given work limits and no cancellation
    /// check.
    pub fn new(limits: BudgetLimits) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ControlState {
                limits,
                report: BudgetReport::default(),
                cancelled: None,
            })),
        }
    }

    /// Create a new control with work limits and an external cancellation
    /// check. The closure is called outside the mutex lock when
    /// [`checkpoint`] is invoked; if it returns `true`, the control latches
    /// as cancelled.
    pub fn with_cancel(
        limits: BudgetLimits,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ControlState {
                limits,
                report: BudgetReport::default(),
                cancelled: Some(cancelled),
            })),
        }
    }

    /// Charge one unit of work for `kind`.
    ///
    /// Returns `PlanningStopped` if the kind has reached its limit, the
    /// control has already stopped for any reason, or a u64 counter would
    /// overflow.
    pub fn charge(&self, kind: WorkKind) -> Result<(), PlannerError> {
        let mut state = self.inner.lock().unwrap();
        state.charge(kind)
    }

    /// Check for external cancellation and return `PlanningStopped` if the
    /// cancellation closure fires.
    ///
    /// Also returns `PlanningStopped` if the control already stopped for
    /// another reason.
    ///
    /// The cancellation closure is called *outside* the mutex lock so that
    /// a long-running check does not block other access.
    pub fn checkpoint(&self) -> Result<(), PlannerError> {
        // Fast path: already stopped? Check under the lock.
        {
            let state = self.inner.lock().unwrap();
            if state.report.stopped.is_some() {
                let reason = state.report.stopped.as_ref().unwrap();
                let msg = match reason {
                    StopReason::Limit(k) => format!("work limit reached for {:?}", k),
                    StopReason::Cancelled => "cancelled".to_string(),
                };
                return Err(PlannerError::PlanningStopped { reason: msg });
            }
        }

        // Check cancellation outside the lock.
        let cancelled = {
            let state = self.inner.lock().unwrap();
            state
                .cancelled
                .as_ref()
                .map(|check| check())
                .unwrap_or(false)
        };

        if cancelled {
            let mut state = self.inner.lock().unwrap();
            let reason = StopReason::Cancelled;
            state.report.stopped = Some(reason);
            return Err(PlannerError::PlanningStopped {
                reason: "cancelled".to_string(),
            });
        }

        Ok(())
    }

    /// Produce a snapshot of the current budget state.
    pub fn report(&self) -> BudgetReport {
        let state = self.inner.lock().unwrap();
        state.report.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_cannot_reset_the_budget() {
        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Retry, 1)]),
        });
        assert!(control.charge(WorkKind::Retry).is_ok());
        assert!(control.clone().charge(WorkKind::Retry).is_err());
        assert_eq!(control.report().used[&WorkKind::Retry], 1);
        assert_eq!(
            control.report().stopped,
            Some(StopReason::Limit(WorkKind::Retry))
        );
        assert!(control.checkpoint().is_err());
    }

    #[test]
    fn zero_allowance_rejects_immediately() {
        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Goal, 0)]),
        });
        assert!(control.charge(WorkKind::Goal).is_err());
        let report = control.report();
        // Zero means "no operation of this kind is permitted"; the charge
        // is rejected without being counted.
        assert_eq!(report.used.get(&WorkKind::Goal), None);
        assert_eq!(report.stopped, Some(StopReason::Limit(WorkKind::Goal)));
    }

    #[test]
    fn cancellation_latches_permanently() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancelled.clone();
        let control = PlanControl::with_cancel(
            BudgetLimits::default(),
            Arc::new(move || flag.load(std::sync::atomic::Ordering::SeqCst)),
        );

        // Not cancelled yet.
        assert!(control.checkpoint().is_ok());

        // Cancel and check.
        cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(control.checkpoint().is_err());
        assert_eq!(control.report().stopped, Some(StopReason::Cancelled));

        // Even if we reset the flag, the latch stays cancelled.
        cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(control.checkpoint().is_err());
        assert!(control.charge(WorkKind::Goal).is_err());
    }

    #[test]
    fn independent_top_level_controls() {
        let a = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Site, 1)]),
        });
        let b = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::RouteNode, 5)]),
        });

        assert!(a.charge(WorkKind::Site).is_ok());
        assert!(b.charge(WorkKind::RouteNode).is_ok());

        // a should be exhausted for Site (1 used of 1 max).
        assert!(a.charge(WorkKind::Site).is_err());
        // b should have 4 remaining for RouteNode.
        assert!(b.charge(WorkKind::RouteNode).is_ok());
        assert!(b.charge(WorkKind::RouteNode).is_ok());
        assert!(b.charge(WorkKind::RouteNode).is_ok());
        assert!(b.charge(WorkKind::RouteNode).is_ok());
        // 5th more is the 6th charge.
        assert!(b.charge(WorkKind::RouteNode).is_err());

        // a is stopped because Site hit its limit; even unbounded kinds
        // are blocked once the control latches.
        assert!(a.charge(WorkKind::Goal).is_err());
    }

    #[test]
    fn exact_maximum_charges_and_then_stops() {
        // Allow exactly 2 Retry charges.
        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Retry, 2)]),
        });

        assert!(control.charge(WorkKind::Retry).is_ok());
        assert!(control.report().stopped.is_none());

        assert!(control.charge(WorkKind::Retry).is_ok());
        assert!(control.report().stopped.is_none());

        // Third charge should stop with Limit(Retry).
        assert!(control.charge(WorkKind::Retry).is_err());
        assert_eq!(
            control.report().stopped,
            Some(StopReason::Limit(WorkKind::Retry))
        );
        // Only the two successful charges are counted.
        assert_eq!(control.report().used[&WorkKind::Retry], 2);
    }
}
