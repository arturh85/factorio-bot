//! Speedrun-style splits, derived from the event log.
//!
//! Derived rather than recorded independently, so a split cannot disagree with
//! the log it summarises. The caller materialises the result into
//! `splits.json` so the viewer and any comparison tool do not each re-derive
//! it.
//!
//! Splits are measured in *ticks*. Wall time is in the log but is never what
//! two runs are compared on: a headless server and a graphical client with
//! three cameras do not run at the same speed, so a wall-time comparison
//! silently compares hardware instead of runs.

use serde::{Deserialize, Serialize};

use super::{Event, EventKind};

/// One milestone's timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Split {
    pub index: u32,
    pub goal: String,
    pub started_tick: u64,
    /// The tick the milestone closed, or `null` when the run ended without
    /// closing it. Present-and-null, never absent.
    pub ended_tick: Option<u64>,
    /// `satisfied`, `stuck`, `stuck_silent`, `exhausted`, or `unfinished` for a
    /// milestone that was still open when the log ended.
    pub outcome: String,
    /// `ended_tick - started_tick`, or `null` while unfinished. Materialised
    /// rather than left to every reader to subtract, because a reader that
    /// subtracts a null gets zero and zero is a plausible-looking duration.
    pub elapsed_ticks: Option<u64>,
}

/// The outcome of a milestone still open when the log ended.
///
/// A crashed run is not an error to be refused -- it is the run most worth
/// looking at -- so its open milestone is reported as unfinished rather than
/// dropped or silently closed at the last tick seen.
pub const UNFINISHED: &str = "unfinished";

/// Turns an event log into splits, in the order the milestones started.
pub fn derive_splits(events: &[Event]) -> Vec<Split> {
    let mut splits: Vec<Split> = Vec::new();

    for event in events {
        match &event.kind {
            EventKind::MilestoneStarted { index, goal } => {
                splits.push(Split {
                    index: *index,
                    goal: goal.clone(),
                    started_tick: event.tick,
                    ended_tick: None,
                    outcome: UNFINISHED.to_string(),
                    elapsed_ticks: None,
                });
            }
            EventKind::MilestoneSatisfied { index, .. } => {
                close(&mut splits, *index, event.tick, "satisfied");
            }
            EventKind::MilestoneStuck { index, outcome, .. } => {
                close(&mut splits, *index, event.tick, outcome);
            }
            _ => {}
        }
    }
    splits
}

/// Closes the most recently opened split with this index.
///
/// Most recent, not first: a milestone index can legitimately repeat across a
/// run, and closing the earliest would attribute a late milestone's time to an
/// early one -- which produces splits that look plausible and are wrong.
fn close(splits: &mut [Split], index: u32, tick: u64, outcome: &str) {
    if let Some(split) = splits
        .iter_mut()
        .rev()
        .find(|s| s.index == index && s.ended_tick.is_none())
    {
        split.ended_tick = Some(tick);
        split.outcome = outcome.to_string();
        split.elapsed_ticks = Some(tick.saturating_sub(split.started_tick));
    }
}

#[cfg(test)]
mod tests {
    use super::super::SatisfiedReason;
    use super::*;

    fn ev(tick: u64, kind: EventKind) -> Event {
        Event {
            tick,
            wall_ms: tick,
            kind,
        }
    }

    fn started(index: u32, goal: &str, tick: u64) -> Event {
        ev(
            tick,
            EventKind::MilestoneStarted {
                index,
                goal: goal.into(),
            },
        )
    }

    fn satisfied(index: u32, tick: u64) -> Event {
        ev(
            tick,
            EventKind::MilestoneSatisfied {
                index,
                iterations: 1,
                reason: SatisfiedReason::AlreadySatisfied,
            },
        )
    }

    #[test]
    fn a_satisfied_milestone_gets_its_elapsed_ticks() {
        let splits = derive_splits(&[started(0, "a", 100), satisfied(0, 400)]);
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].started_tick, 100);
        assert_eq!(splits[0].ended_tick, Some(400));
        assert_eq!(splits[0].elapsed_ticks, Some(300));
        assert_eq!(splits[0].outcome, "satisfied");
    }

    #[test]
    fn milestones_keep_the_order_they_started_in() {
        let splits = derive_splits(&[
            started(0, "a", 10),
            satisfied(0, 20),
            started(1, "b", 30),
            satisfied(1, 90),
        ]);
        assert_eq!(
            splits.iter().map(|s| s.goal.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(splits[1].elapsed_ticks, Some(60));
    }

    #[test]
    fn a_stuck_milestone_carries_its_own_outcome() {
        let splits = derive_splits(&[
            started(0, "a", 10),
            ev(
                50,
                EventKind::MilestoneStuck {
                    index: 0,
                    outcome: "stuck_silent".into(),
                    best_steps: Some(42),
                    last_error: None,
                },
            ),
        ]);
        assert_eq!(splits[0].outcome, "stuck_silent");
        assert_eq!(splits[0].elapsed_ticks, Some(40));
    }

    #[test]
    fn a_run_that_never_finished_leaves_its_milestone_unfinished() {
        // The log just stops -- the crashed run, which is the one worth reading.
        let splits = derive_splits(&[started(0, "a", 10), satisfied(0, 20), started(1, "b", 30)]);
        assert_eq!(splits.len(), 2);
        assert_eq!(splits[1].outcome, UNFINISHED);
        assert_eq!(splits[1].ended_tick, None);
        assert_eq!(
            splits[1].elapsed_ticks, None,
            "an unfinished split must not report a duration; zero would look plausible"
        );
    }

    #[test]
    fn a_repeated_index_closes_the_most_recent_opening() {
        // Index 0 twice. Closing the earliest would credit the late milestone's
        // time to the early one and produce splits that look fine and are wrong.
        let splits = derive_splits(&[
            started(0, "first", 10),
            satisfied(0, 20),
            started(0, "second", 100),
            satisfied(0, 500),
        ]);
        assert_eq!(splits.len(), 2);
        assert_eq!(splits[0].elapsed_ticks, Some(10));
        assert_eq!(splits[1].elapsed_ticks, Some(400));
    }

    #[test]
    fn a_close_with_no_matching_open_is_ignored_rather_than_inventing_a_split() {
        let splits = derive_splits(&[satisfied(7, 100)]);
        assert!(
            splits.is_empty(),
            "a milestone that never started has no timing to report"
        );
    }

    #[test]
    fn unrelated_events_do_not_affect_splits() {
        let splits = derive_splits(&[
            started(0, "a", 10),
            ev(
                15,
                EventKind::Frame {
                    bot: 1,
                    camera: "c".into(),
                    file: "f.jpg".into(),
                },
            ),
            satisfied(0, 20),
        ]);
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].elapsed_ticks, Some(10));
    }
}
