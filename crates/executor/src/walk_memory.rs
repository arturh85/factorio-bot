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
use factorio_bot_core::factorio::world::{Enclosure, FactorioWorld, WalkRefusal};
use factorio_bot_core::graph::enclosure::{Escape, SEARCH_RADIUS, escape_from};
use factorio_bot_core::miette::Report;
use factorio_bot_core::tracing::{info, warn};
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
///
/// # Every branch says so out loud
///
/// This module shipped with no logging and nothing in the run record, and the
/// cost was immediate: three consecutive live runs repeated the same refused
/// `(bot, destination)` pairs, and nobody could tell from outside the source
/// whether the ledger had ever been written to. "It did not fire", "it fired
/// and wrote to a world nobody reads" and "it fired, was read, and the
/// scheduler had no second candidate to offer" are three different defects
/// with one appearance, and separating them took a day.
///
/// So every path out of this function emits exactly one `tracing` line naming
/// which one it took. `tracing` and not `paris` because these explain a
/// planner decision to whoever reads the log afterwards, which is the
/// distinction `CLAUDE.md` draws -- **they land on stderr, so a run capture
/// that keeps only stdout will not have them**.
pub fn note_walk_refusal(
    world: &FactorioWorld,
    player: PlayerId,
    from: Option<&Position>,
    to: &Position,
    failure: &ActionFailure,
) -> bool {
    if !pathfinder_found_nothing(&failure.error) {
        info!(
            player,
            to = %to,
            error = %failure.error,
            "walk failure teaches nothing about the map: the pathfinder never \
             answered that there is no way there, so nothing is remembered"
        );
        return false;
    }
    let Some(from) = from else {
        warn!(
            player,
            to = %to,
            "the pathfinder refused this walk but the world does not know where \
             the character was standing, so the refusal cannot be remembered -- \
             recording it without an origin would claim the destination is dead \
             for this bot wherever it stands"
        );
        return false;
    };
    let learned = world.record_walk_refusal(WalkRefusal {
        tick: failure.ticks.dispatched,
        player,
        from: from.clone(),
        to: to.clone(),
    });
    warn!(
        player,
        from = %from,
        to = %to,
        learned,
        "the pathfinder refused this walk; the ledger the next plan reads now \
         holds it (learned=false means this exact question was already in it)"
    );
    note_enclosure(world, player, from, failure.ticks.dispatched);
    learned
}

/// Asks whether the character can reach open ground at all, and remembers the
/// answer when it cannot.
///
/// # Why here
///
/// This is the cheapest useful trigger. A refused path is the moment the game
/// itself has already said something is wrong about this spot -- and by the
/// time it reaches this function, five searches from it have found nothing (the
/// goal plus `player_path`'s four rotated offsets). Running the fill per
/// *dispatch* would ask a question nobody has a reason to ask, thousands of
/// times a run; running it on a timer would ask it of bots nothing is wrong
/// with. This asks it exactly when there is a reason to.
///
/// The conjunction is also what keeps a false positive out of the archive. A
/// report needs the pathfinder to refuse a route from here **and** an
/// independent fill over the occupancy model to close, and the model is
/// incomplete only in the direction that opens the fill up (uncharted ground
/// holds no entities and so reads as free).
///
/// # Why the answer is deliberately not acted on
///
/// Nothing reads this ledger except the record. Evacuating a bot before a build
/// and mining a way out are both worth doing and neither can be designed
/// against a run archive that has never once said when this happens -- which is
/// the entire finding of `run-1788432181-42528`.
///
/// # Why all three answers are logged, and separately
///
/// [`Escape`] has three variants and the module that defines it is emphatic
/// that they must never be collapsed: "there is a way out within the window"
/// and "I could not tell" are different facts. That distinction is the whole
/// value of the log line here. When this detector stays silent through a run
/// with two frozen bots, "the fill found an escape" and "the window fell
/// outside the occupancy model" point at completely different follow-ups, and
/// a single "no enclosure" line would have said neither.
///
/// [`escape_from`] is called directly rather than through
/// `enclosure_at`, which folds `Open` and `Unknown` into one `None` --
/// exactly the collapse this needs to avoid.
fn note_enclosure(world: &FactorioWorld, player: PlayerId, from: &Position, tick: Option<u64>) {
    match escape_from(&world.entity_graph, from) {
        Escape::Enclosed { pocket_tiles } => {
            let fresh = world.record_enclosure(Enclosure {
                tick,
                player,
                at: from.clone(),
                pocket_tiles,
                searched_tiles: SEARCH_RADIUS,
            });
            warn!(
                player,
                at = %from,
                pocket_tiles,
                searched_tiles = SEARCH_RADIUS,
                fresh,
                "WALLED IN: this character cannot reach open ground -- every \
                 point it can walk to is inside this pocket"
            );
        }
        Escape::Open => info!(
            player,
            at = %from,
            searched_tiles = SEARCH_RADIUS,
            "the walk was refused but the character is not boxed in: the free \
             region around it reaches the edge of the searched window, so what \
             is unreachable is the destination, not the bot"
        ),
        Escape::Unknown(why) => info!(
            player,
            at = %from,
            why = ?why,
            "the escape search declined to answer, which is not the same as \
             finding a way out"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::errors::RconTimeout;
    use factorio_bot_core::factorio::ticks::ActionTicks;
    use factorio_bot_core::types::FactorioEntity;

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

    /// Seals `around` inside a ring of trees, exactly the way
    /// `crates/core`'s `enclosure_bounds` tests do, so a refusal raised from
    /// inside it has something real to find.
    fn wall_in(world: &FactorioWorld, around: &Position) {
        let mut ring = Vec::new();
        let mut offset = -3.0;
        while offset <= 3.0 {
            for (x, y) in [
                (around.x() + offset, around.y() - 3.0),
                (around.x() + offset, around.y() + 3.0),
                (around.x() - 3.0, around.y() + offset),
                (around.x() + 3.0, around.y() + offset),
            ] {
                ring.push(FactorioEntity::new_tree(&Position::new(x, y)));
            }
            offset += 0.5;
        }
        world.entity_graph.add(ring, None).expect("the ring loads");
    }

    /// The refusal and the diagnosis are written together, from one call.
    ///
    /// This is the wiring the whole change is for: `run-1788432181-42528`
    /// produced nineteen of these refusals and the record still could not say
    /// that the bot was unable to move at all.
    #[test]
    fn a_refusal_from_inside_a_wall_also_names_the_enclosure() {
        let world = FactorioWorld::new();
        wall_in(&world, &here());
        assert!(note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(NO_PATH)
        ));
        let found = world.enclosures();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].player, 3);
        assert_eq!(found[0].at, here());
        assert!(found[0].pocket_tiles > 0.);
    }

    /// A refusal from open ground names no enclosure -- the two are separate
    /// judgements and only their conjunction is a finding.
    #[test]
    fn a_refusal_from_open_ground_names_no_enclosure() {
        let world = FactorioWorld::new();
        assert!(note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(NO_PATH)
        ));
        assert!(
            world.enclosures().is_empty(),
            "the destination was unreachable; the bot was not"
        );
    }

    /// A walled-in bot whose failure taught us nothing still teaches nothing.
    ///
    /// The check hangs off the refusal that *established* something. A full
    /// request queue means the game never searched, so there is no reason to
    /// believe anything is wrong here and no report is made -- even though the
    /// fill would have found the ring.
    #[test]
    fn a_busy_queue_names_no_enclosure_even_inside_a_wall() {
        let world = FactorioWorld::new();
        wall_in(&world, &here());
        assert!(!note_walk_refusal(
            &world,
            3,
            Some(&here()),
            &there(),
            &path_failure(BUSY)
        ));
        assert!(world.enclosures().is_empty());
    }
}
