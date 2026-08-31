use factorio_bot_core::factorio::ticks::ActionTicks;
use factorio_bot_planner::{ActionId, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Narrows a game tick to the planner's [`Ticks`].
///
/// `game.tick` is a `MapTick` (uint64); `Ticks` is 32-bit, which covers about
/// 2.3 years of game time. A value that does not fit comes back **absent**
/// rather than wrapped: a wrapped tick would look like a perfectly plausible
/// early-game measurement and quietly corrupt anything aligned to it, whereas
/// `None` is a fact a consumer can act on.
fn narrow(tick: Option<u64>) -> Option<Ticks> {
    tick.and_then(|t| Ticks::try_from(t).ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Running,
    Success,
    Failed,
}

/// One execution attempt of one action — always the **latest** one.
///
/// # Two kinds of tick live here, and they must never be confused
///
/// `planned_start_tick` and `planned_end_tick` are named for what they actually
/// hold. The only writer is `run_into`, which passes `ScheduledStep::start` and
/// `ScheduledStep::end` — numbers the *scheduler* computed from
/// `Action::duration` before anything ran. They are the estimate, not a
/// measurement of it, and `planned_duration()` is that estimate round-tripped
/// through the log.
///
/// `dispatched_tick` and `replied_tick` are the measurement. They are
/// `game.tick` as the game itself reported it — when it received the command
/// and when it reported the outcome — carried back through
/// [`Actuator`](crate::Actuator) as an [`ActionTicks`] and written by
/// [`ExecutionLog::observe`]. Nothing else may write them.
///
/// **The two are kept side by side deliberately.** The drift between the plan
/// and the game is the signal — it is what tells you the scheduler's model of
/// `Action::duration` is wrong, and it is what a consumer aligning captured
/// frames to a plan needs in order to pin a frame to the tick it was actually
/// taken at. Renaming `planned_*` to something that sounds measured, or filling
/// `dispatched_tick`/`replied_tick` in from the schedule when the game did not
/// answer, would destroy exactly that signal while leaving every reading
/// plausible.
///
/// # An absent tick is a value
///
/// `dispatched_tick` and `replied_tick` are `Option` and stay `None` whenever
/// the game did not tell us: an action that failed before it was dispatched, a
/// reply whose stamp could not be parsed, a tick too large for [`Ticks`], or an
/// actuator with no clock at all. `None` is never to be replaced with zero, with
/// the planned tick, or with the previous action's tick. A consumer must be able
/// to say "no reply tick for this action"; a fabricated number is worse than a
/// missing one, because it will be built on.
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
    /// `game.tick` when the game **received** this attempt's command.
    /// `None` if the game never said — see the type docs.
    pub dispatched_tick: Option<Ticks>,
    /// `game.tick` when the game **reported the outcome** of this attempt.
    /// `None` if the game never said — see the type docs.
    pub replied_tick: Option<Ticks>,
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
                dispatched_tick: None,
                replied_tick: None,
                error: None,
            },
        );
    }

    /// Records the game ticks observed for `id`'s current attempt.
    ///
    /// Separate from [`ExecutionLog::succeed`]/[`ExecutionLog::fail`] on
    /// purpose: those take *plan* numbers, this one takes *game* numbers, and
    /// keeping the two writers apart is what makes it impossible to pass a
    /// schedule value here by slipping an argument. It writes nothing but the
    /// two observed fields.
    ///
    /// Only an attempt that already exists is annotated. There is deliberately
    /// no upsert: an observation with no attempt to attach to would be an
    /// observation of nothing, and inventing an attempt for it would put an
    /// action in the log that the run never started.
    ///
    /// Guarded by `has_finished` for the same reason `succeed` is — a duplicate
    /// completion inside one run must not overwrite the timing already
    /// recorded, or which numbers survive would depend on which writer the game
    /// answered first. A retry is unaffected: `start` reopens the attempt (and
    /// clears these fields) before the retry can observe anything.
    pub fn observe(&mut self, id: ActionId, ticks: ActionTicks) {
        if self.has_finished(id) {
            return;
        }
        if let Some(a) = self.attempts.get_mut(&id) {
            a.dispatched_tick = narrow(ticks.dispatched);
            a.replied_tick = narrow(ticks.replied);
        }
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
            dispatched_tick: None,
            replied_tick: None,
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
            dispatched_tick: None,
            replied_tick: None,
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
    /// the estimate compares the estimate with itself. The measured counterpart
    /// is [`ExecutionLog::observed_duration`], which is `None` whenever the game
    /// did not report both ends.
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

    /// Ticks the game actually spent on this attempt, when it reported both
    /// ends.
    ///
    /// `None` — never zero, never the planned duration — if either observation
    /// is missing, because a duration derived from a fabricated endpoint is a
    /// fabricated duration. `None` also if the reply tick precedes the dispatch
    /// tick, which cannot happen in a game whose clock only advances and so
    /// means one of the two numbers is not what it claims to be.
    pub fn observed_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        let (start, end) = (a.dispatched_tick?, a.replied_tick?);
        end.checked_sub(start)
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
    fn an_attempt_with_no_observation_reports_absent_ticks_not_zero_or_the_plan() {
        // The whole point of the pair. An actuator with no game clock, or a
        // dispatch the game never answered, must leave these empty -- and
        // emphatically not fall back to the planned numbers sitting right
        // beside them, which is the failure mode that makes a fabricated
        // measurement look real.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.succeed(id(1), 340);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, None);
        assert_eq!(a.replied_tick, None);
        assert_ne!(a.dispatched_tick, Some(0), "absent is not tick zero");
        assert_ne!(
            a.dispatched_tick,
            Some(a.planned_start_tick),
            "absent must never fall back to the plan"
        );
        assert_eq!(log.observed_duration(id(1)), None);
        assert_eq!(
            log.planned_duration(id(1)),
            Some(240),
            "the estimate is unaffected by there being no measurement"
        );
    }

    #[test]
    fn an_observation_is_recorded_and_is_not_the_planned_tick() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), Some(70_240)));
        log.succeed(id(1), 340);

        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(70_000));
        assert_eq!(a.replied_tick, Some(70_240));
        // Both kinds survive side by side: the drift between them is the
        // signal, so neither may overwrite the other.
        assert_eq!(a.planned_start_tick, 100);
        assert_eq!(a.planned_end_tick, Some(340));
        assert_ne!(a.dispatched_tick, Some(a.planned_start_tick));
        assert_ne!(a.replied_tick, a.planned_end_tick);
        assert_eq!(log.observed_duration(id(1)), Some(240));
    }

    #[test]
    fn half_an_observation_yields_no_observed_duration() {
        // The game acknowledged the dispatch and then never reported an
        // outcome. The dispatch tick is real and is kept; the duration is not
        // derivable and must not be invented from the planned end.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), None));
        log.succeed(id(1), 340);
        assert_eq!(log.attempt(id(1)).unwrap().dispatched_tick, Some(70_000));
        assert_eq!(log.attempt(id(1)).unwrap().replied_tick, None);
        assert_eq!(log.observed_duration(id(1)), None);
    }

    #[test]
    fn a_tick_too_large_for_the_planners_width_is_absent_rather_than_wrapped() {
        // `game.tick` is 64-bit and `Ticks` is 32-bit. A wrapped value would
        // look like an ordinary early-game measurement and silently misplace
        // anything aligned to it, so it is dropped instead.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(
            id(1),
            ActionTicks::new(Some(u64::from(u32::MAX) + 1), Some(5)),
        );
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, None, "dropped, not wrapped to 0");
        assert_eq!(a.replied_tick, Some(5), "the tick that does fit is kept");
    }

    #[test]
    fn observing_an_action_that_was_never_started_records_nothing() {
        // An observation of nothing. Inventing an attempt to hang it on would
        // put an action in the log that the run never dispatched.
        let mut log = ExecutionLog::default();
        log.observe(id(1), ActionTicks::new(Some(1), Some(2)));
        assert!(log.attempt(id(1)).is_none());
        assert_eq!(log.status(id(1)), Status::Pending);
    }

    #[test]
    fn a_retry_observes_afresh_rather_than_inheriting_the_failed_attempts_ticks() {
        // The superseded attempt's measurement belongs to the superseded
        // attempt. Carrying it forward would report the retry as having been
        // dispatched before it was.
        let mut log = ExecutionLog::default();
        log.start(id(1), 0);
        log.observe(id(1), ActionTicks::new(Some(1_000), Some(1_060)));
        log.fail(id(1), 60, "player was busy".to_string());

        log.start(id(1), 500);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(
            a.dispatched_tick, None,
            "a reopened attempt has observed nothing yet"
        );
        assert_eq!(a.replied_tick, None);

        log.observe(id(1), ActionTicks::new(Some(9_000), Some(9_060)));
        log.succeed(id(1), 560);
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(9_000), "the retry's own ticks");
        assert_eq!(a.replied_tick, Some(9_060));
    }

    #[test]
    fn a_second_observation_of_a_finished_attempt_is_ignored() {
        // Same argument as the second-completion guard: which numbers survive
        // must not depend on which duplicate writer the game answered first.
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        log.observe(id(1), ActionTicks::new(Some(70_000), Some(70_240)));
        log.succeed(id(1), 340);
        log.observe(id(1), ActionTicks::new(Some(1), Some(2)));
        let a = log.attempt(id(1)).expect("attempt");
        assert_eq!(a.dispatched_tick, Some(70_000));
        assert_eq!(a.replied_tick, Some(70_240));
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
