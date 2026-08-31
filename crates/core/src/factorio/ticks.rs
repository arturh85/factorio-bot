//! Game ticks the mod reports, as distinct from ticks a schedule predicted.
//!
//! Everything here carries a number the *game* produced. Nothing here may ever
//! be filled in from a plan: a field named for a measurement must hold a
//! measurement, and a fabricated one is worse than an absent one because a
//! consumer will build on it. `Option::None` is the representation of "the game
//! did not tell us", and it is a value in its own right — never a reason to
//! substitute a default.

use serde::{Deserialize, Serialize};

/// The sentinel BotBridge prefixes its tick line with.
///
/// Chosen so it cannot collide with a payload: `§` is not producible by
/// `helpers.table_to_json` (which escapes non-ASCII) and appears in no message
/// `complain` builds. `mods/BotBridge/control.lua`'s `stamp_tick` writes it;
/// [`take_tick_stamp`] is the only thing that reads it.
pub const TICK_STAMP_PREFIX: &str = "§tick§";

/// Splits an RCON reply into its payload and the tick BotBridge stamped on it.
///
/// The stamp has to be removed *before* the payload is judged, because every
/// caller judges the payload by shape: an action start treats any remaining
/// line as an error message, and `place_entity` demands exactly one JSON
/// document. Leaving the stamp in would turn every success into a failure —
/// the same trap as debugging the mod with a bare `rcon.print`.
///
/// A reply consisting only of the stamp comes back as `None`, so a response
/// that was empty before the mod started stamping is still empty here.
/// An unparseable stamp yields `None` for the tick rather than a zero: we know
/// the game answered, we do not know when.
pub fn take_tick_stamp(lines: Option<Vec<String>>) -> (Option<Vec<String>>, Option<u64>) {
    let Some(lines) = lines else {
        return (None, None);
    };
    let mut tick = None;
    let mut rest: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        match line.strip_prefix(TICK_STAMP_PREFIX) {
            // Last stamp wins: a reply carries at most one, and if a future
            // handler ever stamps twice the later one is the closer of the two
            // to when the game finished with the command.
            Some(n) => tick = n.trim().parse::<u64>().ok(),
            None => rest.push(line),
        }
    }
    if rest.is_empty() {
        (None, tick)
    } else {
        (Some(rest), tick)
    }
}

/// What the game reported for a dispatched action, and when.
///
/// The `tick` is the one the mod stamped on its `action_completed` event
/// (`writeout(tick, "action_completed", ...)`), which it has always carried and
/// which the parser used to discard. `result` is `"ok"` or the failure message,
/// exactly as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// `game.tick` at the moment the game reported the outcome.
    pub tick: u64,
    /// `"ok"`, or the reason the action failed.
    pub result: String,
}

impl ActionOutcome {
    pub fn is_ok(&self) -> bool {
        self.result == "ok"
    }
}

/// The two game ticks one dispatched action can be observed at.
///
/// Both are `Option` and both mean exactly what they say:
///
/// - `dispatched` — `game.tick` when the game *received* the command, read off
///   the stamp BotBridge puts on the RPC response.
/// - `replied` — `game.tick` when the game *reported the outcome*. For an
///   asynchronous action (walk, mine, craft) that is a later tick, carried on
///   the mod's `action_completed` event; for a synchronous one (place, insert,
///   remove, research) the command runs and finishes inside one tick, so it is
///   genuinely the same number as `dispatched` rather than a copy standing in
///   for a measurement nobody took.
///
/// `None` is not a failure to be papered over. A command that never reached the
/// game, a reply whose stamp did not parse, or an actuator with no clock at all
/// all leave the field empty, and every consumer must be able to say "no tick
/// for this one" instead of reading a zero as tick zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTicks {
    pub dispatched: Option<u64>,
    pub replied: Option<u64>,
}

impl ActionTicks {
    /// Nothing observed. The honest answer for an actuator with no game clock.
    pub const UNKNOWN: ActionTicks = ActionTicks {
        dispatched: None,
        replied: None,
    };

    /// A command the game ran to completion within the tick it received it.
    pub fn at(tick: Option<u64>) -> Self {
        ActionTicks {
            dispatched: tick,
            replied: tick,
        }
    }

    pub fn new(dispatched: Option<u64>, replied: Option<u64>) -> Self {
        ActionTicks {
            dispatched,
            replied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_only_reply_reads_as_empty_with_a_tick() {
        // The shape every action start now produces on success. It has to stay
        // indistinguishable from the old empty reply, or every dispatch that
        // used to succeed starts reporting the stamp as an error message.
        let (rest, tick) = take_tick_stamp(Some(vec!["§tick§4211".to_string()]));
        assert_eq!(rest, None);
        assert_eq!(tick, Some(4211));
    }

    #[test]
    fn a_payload_survives_the_stamp_being_taken_off_it() {
        let (rest, tick) = take_tick_stamp(Some(vec![
            r#"{"name":"stone-furnace"}"#.to_string(),
            "§tick§99".to_string(),
        ]));
        assert_eq!(rest, Some(vec![r#"{"name":"stone-furnace"}"#.to_string()]));
        assert_eq!(tick, Some(99));
    }

    #[test]
    fn an_error_message_is_still_an_error_message() {
        let (rest, tick) = take_tick_stamp(Some(vec![
            "Error: no entity to mine".to_string(),
            "§tick§7".to_string(),
        ]));
        assert_eq!(rest, Some(vec!["Error: no entity to mine".to_string()]));
        assert_eq!(tick, Some(7));
    }

    #[test]
    fn an_unstamped_reply_is_untouched_and_has_no_tick() {
        // An older mod, or one of the many RPCs that do not stamp.
        let (rest, tick) = take_tick_stamp(Some(vec!["whatever".to_string()]));
        assert_eq!(rest, Some(vec!["whatever".to_string()]));
        assert_eq!(tick, None, "absent, not zero");
    }

    #[test]
    fn an_unparseable_stamp_is_absent_rather_than_zero() {
        // We know the game answered; we do not know when. Zero would read as
        // tick zero -- the very start of the map -- and a consumer aligning
        // frames to it would place the action before the game began.
        let (rest, tick) = take_tick_stamp(Some(vec!["§tick§not-a-number".to_string()]));
        assert_eq!(rest, None);
        assert_eq!(tick, None);
    }

    #[test]
    fn an_empty_reply_stays_empty() {
        assert_eq!(take_tick_stamp(None), (None, None));
    }

    #[test]
    fn unknown_ticks_are_absent_on_both_ends() {
        assert_eq!(ActionTicks::UNKNOWN.dispatched, None);
        assert_eq!(ActionTicks::UNKNOWN.replied, None);
        assert_eq!(ActionTicks::default(), ActionTicks::UNKNOWN);
    }

    #[test]
    fn a_same_tick_action_reports_the_one_tick_on_both_ends() {
        let t = ActionTicks::at(Some(1234));
        assert_eq!(t.dispatched, Some(1234));
        assert_eq!(t.replied, Some(1234));
        // And an absent tick stays absent on both ends rather than becoming 0.
        assert_eq!(ActionTicks::at(None), ActionTicks::UNKNOWN);
    }
}
