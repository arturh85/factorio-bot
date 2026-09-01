//! What each bot was doing, over time.
//!
//! Derived from the event log's dispatch/settle pairs rather than recorded
//! separately, for the same reason as [`super::splits`]: a derived summary
//! cannot disagree with what it summarises.

use serde::{Deserialize, Serialize};

use super::{Event, EventKind};

/// One thing a bot did, placed on the tick axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Lane {
    pub bot: u32,
    /// The action id. Not unique across a run: ids restart per plan, so this
    /// identifies an entry only together with `bot` and `from_tick`.
    pub id: u32,
    /// What the plan called it, e.g. `mine 4 iron-ore`.
    pub action: String,
    pub from_tick: u64,
    /// `null` for an action that was dispatched and never settled.
    ///
    /// Drawn unterminated rather than dropped: the bot really did start it and
    /// nothing ever came back, which is a state worth seeing. Dropping it would
    /// make a lost action look like one that never happened.
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
                id: *id,
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
                    .find(|l| l.id == *id && l.bot == *bot && l.to_tick.is_none())
                {
                    open.to_tick = Some(event.tick);
                    open.status = Some(status.clone());
                    open.error = error.clone();
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

    fn ev(tick: u64, kind: EventKind) -> Event {
        Event {
            tick,
            wall_ms: tick,
            kind,
        }
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
                },
            ),
        ]);
        assert_eq!(lanes[0].error.as_deref(), Some("no entity to mine"));
    }
}
