//! How much *work* a plan cost, counted rather than timed.
//!
//! # Why a count and not a duration
//!
//! `CLAUDE.md` advertised offline planning as "~4 seconds" for months while the
//! real figure on an explored dump had become 319. Nobody noticed, because a
//! number written in prose has no invalidation: it is a cache with no key, and
//! the only thing that would have falsified it was somebody re-running the
//! command and caring about the answer.
//!
//! A wall-clock assertion cannot replace it — this box ranged from load 1 to
//! load 80 in a single day, and a timing test on a shared machine either fails
//! for reasons that have nothing to do with the change or is loose enough to
//! catch nothing. **A count is deterministic.** The same world and the same
//! goal expand the same goals, flood-fill the same patches and fork the same
//! number of times on an idle box and on a hammered one, so a ceiling on a
//! count is a regression guard a test can actually hold, and a 2x regression
//! in any of these terms shows up as a hard failure with a name attached.
//!
//! # What is counted, and why these five
//!
//! Each is a term the 2026-09-08 planner profile named as dominant, so a
//! change that improves one and quietly doubles another is visible here rather
//! than only in a wall time nobody trusts:
//!
//! * `goals_expanded` — the size of the expansion, the ~72% term;
//! * `resource_patches` — flood fills over charted ore, ~14.5k per crude-oil
//!   plan before this was cached;
//! * `threat_queries` — [`crate::graph::entity_graph::EntityGraph::threats_from`],
//!   17.4M per plan before `ThreatField`;
//! * `preds` — the scheduler's adjacency scan, O(N·E) per round;
//! * `forks` — `PlanState::fork`, 949k per crude-oil schedule.
//!
//! # Purity
//!
//! `crates/planner` promises pure, deterministic planning, and a thread-local
//! counter does not break that promise: nothing reads a count to decide
//! anything, so no plan can differ because of one. It is thread-local rather
//! than shared so that two plans on two threads cannot interleave into a
//! meaningless total — planning is single-threaded today (`plan_best` is a
//! plain `for` over `DrainPolicy::ALL`), and this stays correct if that ever
//! stops being true.
//!
//! # Reading it
//!
//! Use [`measure`], which differences a snapshot around a closure, rather than
//! [`reset`] plus [`snapshot`]: the difference nests, so a caller that measures
//! a whole plan does not have to know whether something inside it also measured
//! a part.

use serde::{Deserialize, Serialize};
use std::cell::Cell;

/// Counts of the planner's dominant repeated operations.
///
/// `Copy`, so the thread-local can be a plain [`Cell`] rather than a
/// `RefCell` — an increment is a `get`, a field bump and a `set`, with no
/// borrow that could panic re-entrantly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCounts {
    /// Calls to `method::expand_goal`, i.e. goals and sub-goals expanded.
    #[serde(default)]
    pub goals_expanded: u64,
    /// Calls to [`crate::graph::entity_graph::EntityGraph::resource_patches`]
    /// that actually ran a flood fill. A cache hit is not counted: the point
    /// of the number is the work done, not the question asked.
    #[serde(default)]
    pub resource_patches: u64,
    /// Calls to [`crate::graph::entity_graph::EntityGraph::threats_from`].
    #[serde(default)]
    pub threat_queries: u64,
    /// Calls to `ActionNetwork::preds`.
    #[serde(default)]
    pub preds: u64,
    /// Calls to `PlanState::fork` made by the scheduler and the expander.
    #[serde(default)]
    pub forks: u64,
}

impl WorkCounts {
    /// `self - earlier`, saturating — what happened between two snapshots.
    ///
    /// Saturating rather than wrapping because a caller that snapshots in the
    /// wrong order should get zero and a wrong-looking report, not a number
    /// near `u64::MAX` that reads as a catastrophic regression.
    #[must_use]
    pub fn since(self, earlier: WorkCounts) -> WorkCounts {
        WorkCounts {
            goals_expanded: self.goals_expanded.saturating_sub(earlier.goals_expanded),
            resource_patches: self
                .resource_patches
                .saturating_sub(earlier.resource_patches),
            threat_queries: self.threat_queries.saturating_sub(earlier.threat_queries),
            preds: self.preds.saturating_sub(earlier.preds),
            forks: self.forks.saturating_sub(earlier.forks),
        }
    }

    /// The counts as lines a person reads, one per line, no trailing newline.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("goals expanded   {}", self.goals_expanded),
            format!("resource patches {}", self.resource_patches),
            format!("threat queries   {}", self.threat_queries),
            format!("preds scans      {}", self.preds),
            format!("state forks      {}", self.forks),
        ]
    }
}

thread_local! {
    static COUNTS: Cell<WorkCounts> = const {
        Cell::new(WorkCounts {
            goals_expanded: 0,
            resource_patches: 0,
            threat_queries: 0,
            preds: 0,
            forks: 0,
        })
    };
}

/// Mutate this thread's counters.
///
/// Kept `#[inline]` and taking a closure so that every call site is one line
/// and the counter set can grow without touching them.
#[inline]
pub fn count(f: impl FnOnce(&mut WorkCounts)) {
    COUNTS.with(|cell| {
        let mut counts = cell.get();
        f(&mut counts);
        cell.set(counts);
    });
}

/// This thread's counters as they stand.
#[must_use]
pub fn snapshot() -> WorkCounts {
    COUNTS.with(Cell::get)
}

/// Zero this thread's counters.
pub fn reset() {
    COUNTS.with(|cell| cell.set(WorkCounts::default()));
}

/// Run `f` and report what it cost, without disturbing any measurement it is
/// nested inside.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, WorkCounts) {
    let before = snapshot();
    let out = f();
    (out, snapshot().since(before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_reports_the_difference_and_nests() {
        reset();
        count(|c| c.forks += 3);
        let (inner, outer) = measure(|| {
            count(|c| c.forks += 1);
            let (_, inner) = measure(|| count(|c| c.forks += 4));
            inner
        });
        // The inner measurement sees only its own work ...
        assert_eq!(inner.forks, 4);
        // ... the outer sees both, and neither sees the 3 that came before.
        assert_eq!(outer.forks, 5);
        assert_eq!(snapshot().forks, 8);
        reset();
        assert_eq!(snapshot(), WorkCounts::default());
    }

    #[test]
    fn a_backwards_difference_saturates_to_zero() {
        let earlier = WorkCounts {
            preds: 10,
            ..WorkCounts::default()
        };
        assert_eq!(WorkCounts::default().since(earlier).preds, 0);
    }
}
