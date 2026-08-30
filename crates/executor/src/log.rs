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

/// One execution attempt of one action — always the **latest** one.
///
/// # These ticks are scheduled, not observed
///
/// `planned_start_tick` and `planned_end_tick` are named for what they actually
/// hold. The only writer is `run_into`, which passes `ScheduledStep::start` and
/// `ScheduledStep::end` — numbers the *scheduler* computed from
/// `Action::duration` before anything ran. They are the estimate, not a
/// measurement of it.
///
/// So `planned_duration()` is a plan value round-tripped through the log, and
/// an "estimated versus actual" comparison built on it would compare the
/// estimate with itself and always agree. Real observed timings need the game's
/// tick at dispatch and at reply, and **the executor has no game-clock source**:
/// `Actuator` returns `Result<(), ActuatorError>` with no tick in it, and
/// nothing in this crate reads `game.tick`. Getting one means widening
/// `Actuator` (or a BotBridge change), and that is not this increment.
///
/// Naming them honestly is the point. A vacuous metric that reads like a real
/// one is worse than an absent one, because a caller will build on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub status: Status,
    /// Which attempt this is, counting from 1. Greater than 1 means the action
    /// was dispatched again after a failure or an interruption — see
    /// `ExecutionLog::start`. Saturates rather than wrapping: at `u32::MAX` the
    /// count stops being exact, which is harmless, whereas wrapping to 0 would
    /// make `attempts()` report an action that has run four billion times as
    /// never started, and every retry budget reading it would reset.
    pub number: u32,
    /// The tick the *schedule* placed this attempt's start at. Not observed.
    pub planned_start_tick: Ticks,
    /// The tick the *schedule* placed this attempt's end at, once finished.
    /// Not observed.
    pub planned_end_tick: Option<Ticks>,
    pub error: Option<String>,
}

/// Execution state, keyed by action.
///
/// Deliberately separate from `Schedule`: the schedule is an immutable plan
/// value, and progress is a join over the two rather than a mutation of the
/// plan. `BTreeMap` because iteration order is part of the contract —
/// `failed()` returns ids in a stable order so recovery is reproducible.
///
/// Only the latest attempt of each action is kept; `Attempt::number` says how
/// many there have been. See `start` for why a count rather than a history.
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

    /// How many times `id` has been started. Zero if never.
    ///
    /// Greater than one means `recover` proposed the action again after a
    /// failure and the caller ran it. A caller that wants to stop retrying a
    /// hopeless action reads this — `recover` itself is pure and keeps no
    /// memory across rounds, so this counter is the only record that a retry
    /// happened at all.
    pub fn attempts(&self, id: ActionId) -> u32 {
        self.attempts.get(&id).map_or(0, |a| a.number)
    }

    /// Records that an attempt has begun.
    ///
    /// Two cases hide behind "the attempt already finished", and they want
    /// opposite answers:
    ///
    /// - **A duplicate dispatch.** One `ActionId` in two schedule steps, or two
    ///   bots handed the same action. The action was meant to happen once, and
    ///   re-opening the attempt would destroy the outcome already recorded for
    ///   it. First completion wins.
    /// - **A retry.** `recover` returns the failed action in its tier-1
    ///   proposal *by design*, so the caller runs it again. Refusing the write
    ///   here is what made a successful retry unrecordable: the attempt stayed
    ///   `Failed` forever, `failed()` kept naming it, and the caller kept being
    ///   handed the same proposal — an unbounded retry loop issuing real
    ///   commands to a live server.
    ///
    /// The status separates them, and it separates them exactly. `run_into`
    /// drops duplicate ids from a schedule before dispatching (`planned_steps`),
    /// so within one run an action is started once; a second `start` on a
    /// `Success` attempt can therefore only be a duplicate reaching the log by
    /// some other route, and re-running succeeded work is a bug either way. A
    /// `start` on a `Failed` attempt has no such reading: nothing re-dispatches
    /// a failure inside one run, so it is a retry.
    ///
    /// Hence: **a retry supersedes a `Failed` attempt; a `Success` attempt is
    /// left exactly as it is.** The superseding attempt inherits `number + 1`
    /// so the fact of the retry outlives the attempt it replaced.
    ///
    /// A count, not a `Vec<Attempt>` history. The question anyone actually has
    /// here is "was this retried, and how often" — a retry budget, a stuck-action
    /// check — and a history answers it while forcing every reader (`status`,
    /// `planned_duration`, `failed`, and every consumer of the serialized log)
    /// to first answer "which attempt?", turning one field into a decision at
    /// each of them. The cost is real and is stated rather than hidden: the
    /// superseded attempt's error message is lost. That is tolerable because the
    /// caller that chose to retry had that message in hand — `recover` surfaced
    /// it in the round that produced the retry — and a `Vec` is the obvious
    /// upgrade if diagnosing across attempts ever becomes the job.
    pub fn start(&mut self, id: ActionId, tick: Ticks) {
        let number = match self.attempts.get(&id) {
            // A duplicate dispatch of work that already succeeded.
            Some(a) if a.status == Status::Success => return,
            // A retry of a failure: supersede it, and remember it happened.
            Some(a) if a.status == Status::Failed => a.number.saturating_add(1),
            // A retry of an *interrupted* attempt. `Running` means the last run
            // died between dispatch and reply, so starting it again is a retry
            // exactly like the `Failed` case — and the case a retry budget is
            // most needed for, since an interrupted action produces no verdict
            // to escalate on. Counting it flat was a hole: three interrupted
            // runs left `attempts() == 1`, so no budget built on this counter
            // could ever trip.
            //
            // Nothing double-starts a `Running` attempt inside one run:
            // `planned_steps` drops duplicate ids before dispatch and each
            // surviving step calls `start` once. So a second `start` on a
            // `Running` attempt is always a later run picking the action up
            // again.
            Some(a) => a.number.saturating_add(1),
            None => 1,
        };
        self.attempts.insert(
            id,
            Attempt {
                status: Status::Running,
                number,
                planned_start_tick: tick,
                planned_end_tick: None,
                error: None,
            },
        );
    }

    /// Whether this attempt has already reached an outcome. A superseding
    /// `start` clears it, which is what lets a retry record its own.
    fn has_finished(&self, id: ActionId) -> bool {
        self.attempts
            .get(&id)
            .is_some_and(|a| a.planned_end_tick.is_some())
    }

    /// Records a success. Upserts: if no `start()` was ever recorded for
    /// `id`, an attempt is created rather than the write being dropped, so
    /// completions are never lost from the log — the synthesized
    /// `planned_start_tick` is honest that we never observed a start either.
    ///
    /// A second completion of an already-finished attempt is **ignored**, not
    /// asserted against. This used to be a `debug_assert!`, but the only way
    /// to reach it is malformed input — the same `ActionId` in two schedule
    /// steps — and the panic fired inside a `join_all`, unwinding every other
    /// bot along with it. Bad input is not a reason to abort a run that is
    /// otherwise going fine. Ignoring the later write is also the only
    /// order-independent choice available here: overwriting would make the
    /// recorded outcome and duration depend on which writer the game answered
    /// first.
    ///
    /// This guard is about *concurrent duplicates within one run*, which is why
    /// it is untouched by the retry rule above: a retry re-opens the attempt in
    /// `start` first, so by the time it completes there is nothing finished to
    /// ignore.
    pub fn succeed(&mut self, id: ActionId, tick: Ticks) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            number: 1,
            planned_start_tick: tick,
            planned_end_tick: None,
            error: None,
        });
        a.status = Status::Success;
        a.planned_end_tick = Some(tick);
    }

    /// Records a failure. Upserts, and ignores a second completion, for the
    /// same reasons as `succeed()`.
    pub fn fail(&mut self, id: ActionId, tick: Ticks, error: String) {
        if self.has_finished(id) {
            return;
        }
        let a = self.attempts.entry(id).or_insert_with(|| Attempt {
            status: Status::Running,
            number: 1,
            planned_start_tick: tick,
            planned_end_tick: None,
            error: None,
        });
        a.status = Status::Failed;
        a.planned_end_tick = Some(tick);
        a.error = Some(error);
    }

    /// Ticks the schedule allotted this attempt, once finished.
    ///
    /// **Not a measurement.** Both endpoints come from the schedule, so this is
    /// `Action::duration` travelling back out of the log, and comparing it to
    /// the estimate compares the estimate with itself. See `Attempt` for why
    /// there is no observed duration to offer instead, and what it would take
    /// to have one.
    pub fn planned_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        a.planned_end_tick.map(|end| {
            debug_assert!(
                end >= a.planned_start_tick,
                "planned_end_tick {end} precedes planned_start_tick {}",
                a.planned_start_tick
            );
            end.saturating_sub(a.planned_start_tick)
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
        assert_eq!(log.planned_duration(id(1)), Some(240));
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
        assert_eq!(log.planned_duration(id(1)), Some(240));
        assert_eq!(log.attempt(id(1)).and_then(|a| a.error.as_deref()), None);
    }

    #[test]
    fn re_dispatching_a_succeeded_action_does_not_reopen_it() {
        // A duplicate dispatch, not a retry: re-running succeeded work is a
        // bug, and reopening the attempt would destroy the outcome recorded
        // for it.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        log.start(id(1), 1_000);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.planned_duration(id(1)), Some(240));
        assert_eq!(log.attempts(id(1)), 1, "a refused start is not an attempt");
    }

    #[test]
    fn a_retry_after_a_failure_supersedes_it_and_can_record_success() {
        // `recover` returns failed actions in its tier-1 proposal by design.
        // While `start` refused to reopen a finished attempt, the retry's
        // command reached the game but its outcome could never be written:
        // the action stayed `Failed`, `failed()` kept naming it, and the
        // caller kept being handed the same proposal — an unbounded retry
        // loop against a live server.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.fail(id(1), 60, "player was busy".to_string());
        assert_eq!(log.failed(), vec![id(1)]);

        log.start(id(1), 500);
        assert_eq!(
            log.status(id(1)),
            Status::Running,
            "a retry must reopen the attempt, or its outcome is unrecordable"
        );
        log.succeed(id(1), 560);

        assert_eq!(log.status(id(1)), Status::Success);
        assert!(
            log.failed().is_empty(),
            "a succeeded retry must stop being reported as a failure"
        );
        assert_eq!(
            log.planned_duration(id(1)),
            Some(60),
            "the retry's own span"
        );
        assert_eq!(
            log.attempt(id(1)).and_then(|a| a.error.as_deref()),
            None,
            "the superseded failure's message does not linger on a success"
        );
    }

    #[test]
    fn an_interrupted_attempt_is_counted_when_it_is_picked_up_again() {
        // D5. `Running` means the previous run died between dispatch and reply.
        // Starting the action again is a retry exactly like the `Failed` case,
        // and it is the case a retry budget is *most* for, because an
        // interrupted action never produces a verdict to escalate on. While
        // this counted flat, three interrupted runs left `attempts() == 1` and
        // no budget reading the counter could ever trip.
        let mut log = ExecutionLog::default();
        for round in 0..3 {
            log.start(id(1), 100 * round);
            assert_eq!(log.status(id(1)), Status::Running, "never finished");
        }
        assert_eq!(log.attempts(id(1)), 3);
    }

    #[test]
    fn the_attempt_counter_saturates_rather_than_wrapping_to_never_started() {
        // D6. `number + 1` panicked in debug and wrapped in release, and a
        // wrapped counter reports an action that has run four billion times as
        // never started — resetting every retry budget reading it.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.fail(id(1), 1, "x".to_string());
        // Reach the ceiling without four billion round trips.
        if let Some(a) = log.attempts.get_mut(&id(1)) {
            a.number = u32::MAX;
        }
        log.start(id(1), 2);
        assert_eq!(log.attempts(id(1)), u32::MAX, "saturated, not wrapped");
        assert_ne!(log.attempts(id(1)), 0, "a wrap would read as never started");
    }

    #[test]
    fn a_retry_is_counted_so_a_caller_can_tell_one_attempt_from_three() {
        // The only record that a retry happened: `recover` is pure and keeps
        // no memory across rounds, so a caller's retry budget has nothing else
        // to read.
        let mut log = ExecutionLog::default();
        assert_eq!(log.attempts(id(1)), 0, "never started");
        log.start(id(1), 0);
        assert_eq!(log.attempts(id(1)), 1);
        for round in 1..3 {
            log.fail(id(1), 60 * round, "still busy".to_string());
            log.start(id(1), 100 * round);
        }
        assert_eq!(log.attempts(id(1)), 3);
        assert_eq!(log.status(id(1)), Status::Running);
    }
}
