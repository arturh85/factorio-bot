//! Turning a refused walk into something the next plan can read.
//!
//! # The gap this closes
//!
//! `mods/BotBridge/control.lua` used to *teleport* a character that could not
//! walk somewhere, and the comment where that teleport was says what it cost:
//! "The planner never learned a site was unreachable, so it kept choosing it,
//! and the one piece of information the system needed -- 'there is no way
//! there' -- was destroyed at the exact moment the game had offered it." The
//! teleport is gone and the walk now reports honestly, but reporting is not
//! remembering. Run `run-1788432181-42528` sent bot 3 at the same ore tile on
//! five separate plans, from the one spot it had been standing at since tick
//! 53 700, and was refused before dispatch every time.
//!
//! This module is the other end of that loop: the executor writes the refusal
//! into [`FactorioWorld`], and `crates/planner`'s `PlanState::from_world`
//! reads it back on the next plan. Neither crate can see the other; the world
//! is what both can see, exactly as it is for placement refusals.
//!
//! # Why it lives beside the actuator rather than inside it
//!
//! The judgement -- *is this the refusal that means something* -- is a pure
//! function of an error, and belongs somewhere it can be tested without a
//! game, a socket or a mod. The actuator's job is only to call it with the two
//! positions it alone holds.

use factorio_bot_core::errors::RconPathRequestFailed;
use factorio_bot_core::factorio::rcon::{ActionFailure, path_request_was_busy};
use factorio_bot_core::factorio::world::{FactorioWorld, WalkRefusal};
use factorio_bot_core::miette::Report;
use factorio_bot_core::types::{PlayerId, Position};

/// Whether the pathfinder searched and reported that there is no way there.
///
/// The distinction the mod is careful about and this must not blur: `try again
/// later` means the request queue was full and the question was never asked,
/// so nothing was learned and nothing may be remembered; `failed to path find`
/// means it searched and found nothing.
///
/// The downcast is what makes that structural rather than textual. A timeout,
/// a dropped connection or a reply that would not parse are all "we do not
/// know", and remembering one of those as an unreachable destination would
/// fence a bot off ground on the strength of a network error.
///
/// `crates/core` has the same two lines privately, as
/// `rcon::path_search_found_nothing`, where they gate the offset-goal
/// fallback. They are re-expressed here rather than shared because that one is
/// not public; both are built on the one public primitive that owns the
/// wording, [`path_request_was_busy`], so a reword still lands in one place.
///
/// By the time an error reaches this function the fallback has already run:
/// `FactorioRcon::player_path` answers a genuine no-path by asking again for
/// four goals rotated around the target, and only re-raises the original error
/// when all of those fail too. So this returning `true` means five searches
/// from that spot found nothing.
pub fn pathfinder_found_nothing(error: &Report) -> bool {
    error
        .downcast_ref::<RconPathRequestFailed>()
        .is_some_and(|failed| !path_request_was_busy(&failed.reason))
}

/// Remembers a walk the pathfinder refused, when there is something to
/// remember. Returns whether the world learned something new.
///
/// Three things have to be true, and each missing one is a different kind of
/// silence:
///
/// * the failure is a definitive no-path ([`pathfinder_found_nothing`]) --
///   otherwise nothing was established;
/// * the world knows where the character was standing -- the fact is about a
///   *pair* of points, and a refusal with no origin would be read as "this
///   destination is dead for this bot wherever it stands", which is not what
///   the game said;
/// * `to` is the destination the *plan* named, not the offset goal
///   `approach_annulus` derived from it, so the planner can compare it against
///   the site it chose without redoing that arithmetic.
///
/// The tick is whatever the game stamped, which for this refusal is nothing at
/// all: the path request is answered before the walk is dispatched. `None` is
/// recorded rather than a zero, for the reason [`WalkRefusal::tick`] gives.
pub fn note_walk_refusal(
    world: &FactorioWorld,
    player: PlayerId,
    from: Option<&Position>,
    to: &Position,
    failure: &ActionFailure,
) -> bool {
    if !pathfinder_found_nothing(&failure.error) {
        return false;
    }
    let Some(from) = from else {
        return false;
    };
    world.record_walk_refusal(WalkRefusal {
        tick: failure.ticks.dispatched,
        player,
        from: from.clone(),
        to: to.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::errors::RconTimeout;
    use factorio_bot_core::factorio::ticks::ActionTicks;

    /// The exact wording run `run-1788432181-42528` failed nineteen walks with.
    const NO_PATH: &str = "failed to path find";
    /// And the one that means the question was never asked.
    const BUSY: &str = "try again later!";

    fn path_failure(reason: &str) -> ActionFailure {
        ActionFailure::not_dispatched(
            RconPathRequestFailed {
                reason: format!("Error: {reason}"),
            }
            .into(),
        )
    }

    fn here() -> Position {
        Position::new(-56.2578125, 14.74609375)
    }

    fn there() -> Position {
        Position::new(-54.5, -12.5)
    }

    #[test]
    fn a_search_that_found_nothing_is_remembered() {
        let world = FactorioWorld::new();
        assert!(note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(NO_PATH)
        ));
        let known = world.walk_refusals();
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].player, 3);
        assert_eq!(known[0].from, here());
        assert_eq!(known[0].to, there());
        assert_eq!(
            known[0].tick, None,
            "a path request is refused before the walk is dispatched, so the \
             game never stamped one"
        );
    }

    /// The nuance the mod's own doc insists on: a full queue taught us nothing.
    #[test]
    fn a_full_request_queue_is_not_evidence_about_the_map() {
        let world = FactorioWorld::new();
        assert!(!note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(BUSY)
        ));
        assert!(world.walk_refusals().is_empty());
    }

    /// Nor is anything that is not the pathfinder answering.
    #[test]
    fn a_failure_that_is_not_the_pathfinder_teaches_nothing() {
        let world = FactorioWorld::new();
        let timeout = ActionFailure::no_verdict(RconTimeout {}.into(), ActionTicks::at(Some(700)));
        assert!(!note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &timeout
        ));
        assert!(
            world.walk_refusals().is_empty(),
            "a walk whose outcome nobody knows says nothing about the route"
        );
    }

    /// A refusal with no origin is not a narrower fact, it is a different one.
    #[test]
    fn a_refusal_with_nowhere_to_have_started_from_is_dropped() {
        let world = FactorioWorld::new();
        assert!(!note_walk_refusal(
            &world,
            3,
            None,
            &there(),
            &path_failure(NO_PATH)
        ));
        assert!(
            world.walk_refusals().is_empty(),
            "recording it without `from` would claim the destination is dead \
             for this bot wherever it stands, which the game never said"
        );
    }

    #[test]
    fn the_same_refusal_twice_teaches_nothing_the_second_time() {
        let world = FactorioWorld::new();
        assert!(note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(NO_PATH)
        ));
        assert!(!note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(NO_PATH)
        ));
        assert_eq!(world.walk_refusals().len(), 1);
    }
}
