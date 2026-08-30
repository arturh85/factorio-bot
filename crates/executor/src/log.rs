use factorio_bot_planner::{ActionId, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Running,
    Success,
    Failed,
}

/// One execution attempt of one action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub status: Status,
    pub started_tick: Ticks,
    pub ended_tick: Option<Ticks>,
    pub error: Option<String>,
}

/// Observed execution state, keyed by action.
///
/// Deliberately separate from `Schedule`: the schedule is an immutable plan
/// value, and estimated-versus-actual is a join over these two, not a mutation
/// of the plan. `BTreeMap` because iteration order is part of the contract —
/// `failed()` returns ids in a stable order so recovery is reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLog {
    attempts: BTreeMap<ActionId, Attempt>,
}

impl ExecutionLog {
    pub fn attempt(&self, id: ActionId) -> Option<&Attempt> {
        self.attempts.get(&id)
    }

    pub fn status(&self, id: ActionId) -> Status {
        self.attempts.get(&id).map_or(Status::Pending, |a| a.status)
    }

    /// Records that an attempt has begun.
    ///
    /// An action that already finished is left exactly as it is. A `Schedule`
    /// naming the same `ActionId` in two steps is malformed input, and
    /// re-opening a finished attempt would destroy the outcome and the
    /// duration already observed for it. First completion wins — see
    /// `succeed`.
    pub fn start(&mut self, id: ActionId, tick: Ticks) {
        if self.has_finished(id) {
            return;
        }
        self.attempts.insert(
            id,
            Attempt {
                status: Status::Running,
                started_tick: tick,
                ended_tick: None,
                error: None,
            },
        );
    }

    fn has_finished(&self, id: ActionId) -> bool {
        self.attempts
            .get(&id)
            .is_some_and(|a| a.ended_tick.is_some())
    }

    /// Records a success. Upserts: if no `start()` was ever recorded for
    /// `id`, an attempt is created rather than the write being dropped, so
    /// completions are never lost from the log — the synthesized
    /// `started_tick` is honest that we never actually observed a start.
    ///
    /// A second completion of an already-finished action is **ignored**, not
    /// asserted against. This used to be a `debug_assert!`, but the only way
    /// to reach it is malformed input — the same `ActionId` in two schedule
    /// steps — and the panic fired inside a `join_all`, unwinding every other
    /// bot along with it. Bad input is not a reason to abort a run that is
    /// otherwise going fine. Ignoring the later write is also the only
    /// order-independent choice available here: overwriting would make the
    /// recorded outcome and duration depend on which writer the game answered
    /// first.
    pub fn succeed(&mut self, id: ActionId, tick: Ticks) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            started_tick: tick,
            ended_tick: None,
            error: None,
        });
        a.status = Status::Success;
        a.ended_tick = Some(tick);
    }

    /// Records a failure. Upserts, and ignores a second completion, for the
    /// same reasons as `succeed()`.
    pub fn fail(&mut self, id: ActionId, tick: Ticks, error: String) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            started_tick: tick,
            ended_tick: None,
            error: None,
        });
        a.status = Status::Failed;
        a.ended_tick = Some(tick);
        a.error = Some(error);
    }

    /// Game ticks the action actually took, once finished.
    pub fn observed_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        a.ended_tick.map(|end| {
            debug_assert!(
                end >= a.started_tick,
                "ended_tick {end} precedes started_tick {}",
                a.started_tick
            );
            end.saturating_sub(a.started_tick)
        })
    }

    /// Failed action ids, in ascending id order.
    pub fn failed(&self) -> Vec<ActionId> {
        self.attempts
            .iter()
            .filter(|(_, a)| a.status == Status::Failed)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_planner::ActionId;

    fn id(n: u32) -> ActionId {
        ActionId(n)
    }

    #[test]
    fn an_unrecorded_action_is_pending() {
        let log = ExecutionLog::default();
        assert_eq!(log.status(id(1)), Status::Pending);
    }

    #[test]
    fn starting_then_finishing_records_a_duration() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        assert_eq!(log.status(id(1)), Status::Running);
        log.succeed(id(1), 340);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.observed_duration(id(1)), Some(240));
    }

    #[test]
    fn a_failure_keeps_its_message() {
        let mut log = ExecutionLog::default();
        log.start(id(7), 10);
        log.fail(id(7), 20, "cannot reach target".to_string());
        assert_eq!(log.status(id(7)), Status::Failed);
        assert_eq!(
            log.attempt(id(7)).and_then(|a| a.error.as_deref()),
            Some("cannot reach target")
        );
    }

    #[test]
    fn failed_actions_are_listed_in_id_order() {
        let mut log = ExecutionLog::default();
        for n in [9u32, 3, 5] {
            log.start(id(n), 0);
            log.fail(id(n), 1, "x".to_string());
        }
        assert_eq!(log.failed(), vec![id(3), id(5), id(9)]);
    }

    #[test]
    fn succeed_without_a_prior_start_still_records_success() {
        let mut log = ExecutionLog::default();
        log.succeed(id(2), 50);
        assert_eq!(log.status(id(2)), Status::Success);
    }

    #[test]
    fn fail_without_a_prior_start_still_records_the_message() {
        let mut log = ExecutionLog::default();
        log.fail(id(3), 15, "never started".to_string());
        assert_eq!(log.status(id(3)), Status::Failed);
        assert_eq!(
            log.attempt(id(3)).and_then(|a| a.error.as_deref()),
            Some("never started")
        );
    }

    #[test]
    fn a_second_completion_leaves_the_first_outcome_and_its_timing_alone() {
        // Reachable only from malformed input (one `ActionId` in two schedule
        // steps). It used to panic, and the panic unwound every other bot in
        // the same `join_all`.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        log.fail(id(1), 900, "a second runner finished it".to_string());
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.observed_duration(id(1)), Some(240));
        assert_eq!(log.attempt(id(1)).and_then(|a| a.error.as_deref()), None);
    }

    #[test]
    fn restarting_a_finished_action_does_not_reopen_it() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        log.start(id(1), 1_000);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.observed_duration(id(1)), Some(240));
    }
}
