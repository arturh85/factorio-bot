//! What each bot was doing, over time.
//!
//! Derived from the event log's dispatch/settle pairs rather than recorded
//! separately, for the same reason as [`super::splits`]: a derived summary
//! cannot disagree with what it summarises.

use serde::{Deserialize, Serialize};

use super::{Event, EventKind};
use crate::types::Position;

/// The label a walk lane carries.
///
/// Composed here because the schedule gives a walk no text of its own, and
/// full precision because a walk destination is routinely a tile centre --
/// `-22.30078125` and `-22.3` are inside different collision boxes, and this
/// string is what a reader compares against a `walk_settled` failure's
/// `destination`.
fn walk_label(to: &Position) -> String {
    format!("walk to ({}, {})", to.x, to.y)
}

/// One thing a bot did, placed on the tick axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Lane {
    pub bot: u32,
    /// The action id, or `null` for a lane that is not an action.
    ///
    /// Not unique across a run when it is present: ids restart per plan, so it
    /// identifies an entry only together with `bot` and `from_tick`.
    ///
    /// `null` means **this lane has no action id**, which today means it is a
    /// walk -- the scheduler emits a walk as its own `StepKind` with no
    /// `ActionId`, and there is nothing to put here. Borrowing the walk's
    /// `step_index` for the field would put a number from a different id space
    /// under a name that means action id, which is the trap
    /// [`super::EventKind::Teleport`]'s `action_id` already documents from the
    /// other direction.
    pub id: Option<u32>,
    /// What the plan called it, e.g. `mine 4 iron-ore`.
    ///
    /// A walk has no label anywhere -- `StepKind::Walk` carries a destination
    /// and a radius and no text -- so its lane's label is composed here, from
    /// the destination the schedule asked for. It is the one string in this
    /// type that the plan did not write.
    pub action: String,
    pub from_tick: u64,
    /// `null` for an action that was dispatched and never settled.
    ///
    /// Drawn unterminated rather than dropped: the bot really did start it and
    /// nothing ever came back, which is a state worth seeing. Dropping it would
    /// make a lost action look like one that never happened.
    //
    // A run interrupted mid-dispatch is now the only way to reach this. An
    // action the executor lost track of settles as `status: "lost"` and
    // terminates its lane like any other verdict -- until 2026-09-02 it did
    // not, because `record.actions` would not write a settle without a game
    // reply tick, so every lost action reached here as an unterminated lane and
    // was indistinguishable from a run that was killed.
    pub to_tick: Option<u64>,
    /// `null` while unterminated, otherwise the verdict the game gave.
    pub status: Option<String>,
    pub error: Option<String>,
}

/// Turns an event log into per-bot lanes, in dispatch order.
pub fn derive_lanes(events: &[Event]) -> Vec<Lane> {
    let mut lanes: Vec<Lane> = Vec::new();

    for event in events {
        match &event.kind {
            EventKind::ActionDispatched {
                id, bot, action, ..
            } => lanes.push(Lane {
                bot: *bot,
                id: Some(*id),
                action: action.clone(),
                from_tick: event.tick,
                to_tick: None,
                status: None,
                error: None,
            }),
            EventKind::ActionSettled {
                id,
                bot,
                status,
                error,
                ..
            } => {
                // Most recently opened, not the first: action ids restart with
                // every plan, so a run has several id 0s. Closing the earliest
                // would attribute a late action's duration to an early one and
                // produce lanes that look entirely reasonable.
                if let Some(open) = lanes
                    .iter_mut()
                    .rev()
                    .find(|l| l.id == Some(*id) && l.bot == *bot && l.to_tick.is_none())
                {
                    open.to_tick = Some(event.tick);
                    open.status = Some(status.clone());
                    open.error = error.clone();
                }
            }
            // A walk opens a lane exactly as an action does. Without this the
            // viewer answered "what was this bot doing at tick T" with nothing
            // for the largest stretch of every run: walking is most of the
            // wall clock, and none of it was drawn.
            EventKind::WalkDispatched { bot, to, .. } => lanes.push(Lane {
                bot: *bot,
                // Not the `step_index`. It is a real identifier for the walk,
                // but it is not an action id, and this field is named for one.
                id: None,
                action: walk_label(to),
                from_tick: event.tick,
                to_tick: None,
                status: None,
                error: None,
            }),
            EventKind::WalkSettled {
                bot,
                to,
                status,
                error,
                ..
            } => {
                // Matched on `id.is_none()` and the bot, not on `step_index`,
                // which this type does not carry -- and it does not need to:
                // `run_bot_signalled` walks ONE bot's steps in schedule order,
                // so a bot has at most one walk in flight and there is only
                // ever one open walk lane of its to close. Most recently
                // opened, for the reason the action arm gives.
                if let Some(open) = lanes
                    .iter_mut()
                    .rev()
                    .find(|l| l.id.is_none() && l.bot == *bot && l.to_tick.is_none())
                {
                    open.to_tick = Some(event.tick);
                    open.status = Some(status.clone());
                    open.error = error.clone();
                } else {
                    // A settle with no dispatch beside it. An action's is
                    // dropped -- there is no span, and `events.jsonl` carries
                    // the settle anyway for anyone who wants it. A walk's is
                    // kept, as a zero-length lane at its own tick, because a
                    // walk the game never acknowledged has no other line in
                    // any view: dropping it puts the failure back exactly
                    // where it was before walks were recorded at all, which
                    // is nowhere.
                    lanes.push(Lane {
                        bot: *bot,
                        id: None,
                        action: walk_label(to),
                        from_tick: event.tick,
                        to_tick: Some(event.tick),
                        status: Some(status.clone()),
                        error: error.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    lanes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Position;

    fn ev(tick: u64, kind: EventKind) -> Event {
        Event { tick, kind }
    }

    fn dispatched(id: u32, bot: u32, action: &str, tick: u64) -> Event {
        ev(
            tick,
            EventKind::ActionDispatched {
                id,
                bot,
                action: action.into(),
                target: None,
            },
        )
    }

    fn settled(id: u32, bot: u32, status: &str, tick: u64) -> Event {
        ev(
            tick,
            EventKind::ActionSettled {
                id,
                bot,
                status: status.into(),
                elapsed_ticks: None,
                error: None,
                failure: None,
            },
        )
    }

    #[test]
    fn a_dispatch_and_settle_pair_become_one_span() {
        let lanes = derive_lanes(&[
            dispatched(0, 1, "mine 4 iron-ore", 100),
            settled(0, 1, "success", 500),
        ]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].from_tick, 100);
        assert_eq!(lanes[0].to_tick, Some(500));
        assert_eq!(lanes[0].status.as_deref(), Some("success"));
        assert_eq!(lanes[0].action, "mine 4 iron-ore");
    }

    #[test]
    fn a_repeated_id_closes_the_most_recent_dispatch() {
        // Action ids restart with every plan, so one run has several id 0s.
        let lanes = derive_lanes(&[
            dispatched(0, 1, "mine iron", 100),
            settled(0, 1, "success", 200),
            dispatched(0, 1, "mine copper", 300),
            settled(0, 1, "success", 900),
        ]);
        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0].to_tick, Some(200));
        assert_eq!(
            lanes[1].to_tick,
            Some(900),
            "not attributed to the first id 0"
        );
    }

    #[test]
    fn a_dispatch_that_never_settled_is_kept_unterminated() {
        // Dropping it would make a lost action look like one that never
        // happened -- the opposite of what the log is for.
        let lanes = derive_lanes(&[dispatched(3, 2, "craft", 100)]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].to_tick, None);
        assert_eq!(lanes[0].status, None);
    }

    /// A lost action closes its lane, and closes it as `lost`.
    ///
    /// Not as `failed`, and not by being left open. The two mean opposite
    /// things to whoever reads the lane -- the game said no, versus we stopped
    /// hearing about it -- and an open lane says a third thing again (the run
    /// was interrupted here). `run-1788347034-00981` drew all nine of its lost
    /// crafts as open lanes because no settle was ever written for them.
    #[test]
    fn a_lost_action_closes_its_lane_as_lost_and_not_as_failed() {
        let lanes = derive_lanes(&[
            dispatched(13, 1, "craft 1 stone-furnace", 93_392),
            ev(
                122_037,
                EventKind::ActionSettled {
                    id: 13,
                    bot: 1,
                    status: "lost".into(),
                    // Null because nobody measured it: a lost action has no
                    // reply tick to subtract the dispatch from.
                    elapsed_ticks: None,
                    error: Some("no action result received in time".into()),
                    failure: None,
                },
            ),
        ]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].to_tick, Some(122_037));
        assert_eq!(lanes[0].status.as_deref(), Some("lost"));
        assert_ne!(
            lanes[0].status.as_deref(),
            Some("failed"),
            "the game never judged this, so the lane must not say it did"
        );
        assert_eq!(
            lanes[0].error.as_deref(),
            Some("no action result received in time"),
            "and why the outcome is unknown travels with it"
        );
    }

    #[test]
    fn a_settle_never_crosses_to_another_bot() {
        let lanes = derive_lanes(&[
            dispatched(0, 1, "a", 100),
            dispatched(0, 2, "b", 110),
            settled(0, 2, "success", 400),
        ]);
        assert_eq!(lanes[0].to_tick, None, "bot 1 is still running");
        assert_eq!(lanes[1].to_tick, Some(400));
    }

    #[test]
    fn a_settle_with_no_dispatch_is_ignored_rather_than_inventing_a_span() {
        assert!(derive_lanes(&[settled(7, 1, "success", 100)]).is_empty());
    }

    #[test]
    fn a_failure_carries_its_error() {
        let lanes = derive_lanes(&[
            dispatched(0, 1, "mine", 100),
            ev(
                200,
                EventKind::ActionSettled {
                    id: 0,
                    bot: 1,
                    status: "failed".into(),
                    elapsed_ticks: Some(100),
                    error: Some("no entity to mine".into()),
                    failure: None,
                },
            ),
        ]);
        assert_eq!(lanes[0].error.as_deref(), Some("no entity to mine"));
    }

    // ---------------------------------------------------------------- walks

    fn walk_dispatched(bot: u32, step_index: u32, to: (f64, f64), tick: u64) -> Event {
        ev(
            tick,
            EventKind::WalkDispatched {
                bot,
                step_index,
                to: Position::new(to.0, to.1),
                planned_start: 0,
                planned_duration: 240,
            },
        )
    }

    fn walk_settled(bot: u32, step_index: u32, to: (f64, f64), status: &str, tick: u64) -> Event {
        ev(
            tick,
            EventKind::WalkSettled {
                bot,
                step_index,
                to: Position::new(to.0, to.1),
                status: status.into(),
                elapsed_ticks: None,
                error: None,
                failure: None,
            },
        )
    }

    /// **A walking bot is not an idle bot.**
    ///
    /// Lanes are what the viewer asks "what was this bot doing at tick T", and
    /// walking is most of a run's wall clock. With only action spans in here
    /// the answer during every walk was nothing at all -- the largest stretch
    /// of a run drawn as a gap.
    #[test]
    fn a_walk_becomes_a_lane_so_a_walking_bot_is_not_drawn_idle() {
        let lanes = derive_lanes(&[
            walk_dispatched(2, 4, (-23.5, 18.5), 81_381),
            walk_settled(2, 4, (-23.5, 18.5), "failed", 81_661),
        ]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].bot, 2);
        assert_eq!(lanes[0].from_tick, 81_381);
        assert_eq!(lanes[0].to_tick, Some(81_661));
        assert_eq!(lanes[0].status.as_deref(), Some("failed"));
        assert_eq!(
            lanes[0].id, None,
            "a walk has no action id, and borrowing one would be a lie about \
             which id space it lives in"
        );
        assert!(
            lanes[0].action.contains("-23.5"),
            "the label names where the bot was going, got {}",
            lanes[0].action
        );
    }

    /// A walk and an action never close each other.
    ///
    /// Both are keyed by bot, and the walk's key is `(bot, step_index)` while
    /// the action's is `(bot, id)` -- two different id spaces that happen to
    /// hold small integers. Closing across them is exactly the confident
    /// nonsense `EventKind::Teleport`'s `action_id` warns about.
    #[test]
    fn a_walk_lane_and_an_action_lane_do_not_close_each_other() {
        let lanes = derive_lanes(&[
            walk_dispatched(1, 0, (5.0, 5.0), 100),
            dispatched(0, 1, "mine 4 iron-ore", 110),
            settled(0, 1, "success", 400),
        ]);
        assert_eq!(lanes.len(), 2);
        assert_eq!(
            lanes[0].to_tick, None,
            "the action's settle must not close the walk"
        );
        assert_eq!(lanes[1].to_tick, Some(400));

        let lanes = derive_lanes(&[
            dispatched(0, 1, "mine 4 iron-ore", 100),
            walk_dispatched(1, 0, (5.0, 5.0), 110),
            walk_settled(1, 0, (5.0, 5.0), "success", 400),
        ]);
        assert_eq!(
            lanes[0].to_tick, None,
            "and the walk's settle must not close the action"
        );
        assert_eq!(lanes[1].to_tick, Some(400));
    }

    /// A settle with no dispatch still draws, because a walk the game never
    /// acknowledged is exactly the one worth seeing.
    ///
    /// An action's settle with no dispatch is dropped here -- there is no span
    /// to draw and the action log carries it anyway. A walk has no other line
    /// anywhere: dropping it would put the failure back where it was before
    /// this record existed, which is nowhere.
    #[test]
    fn a_walk_that_was_never_dispatched_still_draws_at_its_settle() {
        let lanes = derive_lanes(&[walk_settled(3, 1, (-19.5, 19.5), "failed", 165_964)]);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].from_tick, 165_964);
        assert_eq!(lanes[0].to_tick, Some(165_964));
        assert_eq!(lanes[0].status.as_deref(), Some("failed"));
    }

    /// A lost walk closes its lane as `lost`, never as `failed` and never by
    /// being left open -- the rule
    /// `a_lost_action_closes_its_lane_as_lost_and_not_as_failed` states for
    /// actions, applied to the half of the schedule that had no lanes at all.
    #[test]
    fn a_lost_walk_closes_its_lane_as_lost() {
        let lanes = derive_lanes(&[
            walk_dispatched(2, 3, (-22.5, 21.5), 105_028),
            walk_settled(2, 3, (-22.5, 21.5), "lost", 126_628),
        ]);
        assert_eq!(lanes[0].to_tick, Some(126_628));
        assert_eq!(lanes[0].status.as_deref(), Some("lost"));
        assert_ne!(lanes[0].status.as_deref(), Some("failed"));
    }
}
