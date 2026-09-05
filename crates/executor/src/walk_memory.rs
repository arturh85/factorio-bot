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
use factorio_bot_core::factorio::rcon::{ActionFailure, FactorioRcon, path_request_was_busy};
use factorio_bot_core::factorio::world::{
    Bench, BenchRelease, Enclosure, FactorioWorld, HOP_DISTANCE, WalkRefusal,
};
use factorio_bot_core::graph::enclosure::{Escape, SEARCH_RADIUS, escape_from};
use factorio_bot_core::miette::Report;
use factorio_bot_core::tracing::{info, warn};
use factorio_bot_core::types::{PlayerId, Position};
use std::sync::Arc;

/// What a mobility probe established about a character -- see
/// [`judge_mobility`].
#[derive(Debug, Clone, PartialEq)]
pub enum Mobility {
    /// At least one short hop was pathed: the character can leave its tile,
    /// so whatever was unreachable, it was not the character.
    Free {
        /// The first hop target the game agreed to route to.
        hop: Position,
    },
    /// Every hop asked for came back `failed to path find`: the character
    /// cannot leave its own tile.
    BoxedIn {
        /// How many hops were asked for and refused.
        refused_hops: u8,
    },
    /// The probe did not establish either: no hop was pathed, and at least
    /// one answer was not the pathfinder saying no -- a full queue that
    /// outlasted its retries, a timeout, a dropped connection.
    Unknown,
}

/// Turns the game's answers to a hop probe into a verdict on the character.
///
/// Pure, so it can be tested without a game -- the asking is
/// `FactorioRcon::probe_player_hops`, the judging is here, and the two are
/// separated for the reason this module's header gives.
///
/// # The three answers are not symmetrical
///
/// * One pathed hop is enough for [`Mobility::Free`]. The claim it makes is
///   only "the character can leave its tile", and one route proves that.
/// * [`Mobility::BoxedIn`] needs *every* hop to be a definitive
///   `failed to path find` ([`pathfinder_found_nothing`]). Benching a bot
///   takes it out of every plan until something lifts the bench, so the
///   evidence has to be the game's searched-and-found-nothing for each
///   direction, not a queue that was full in one of them.
/// * Anything else is [`Mobility::Unknown`], and unknown changes nothing:
///   the walk failed, the walk ledger holds the pair, and the next plan
///   proceeds as it did before this probe existed.
///
/// An empty probe is `Unknown` for the same reason: no question was asked.
pub fn judge_mobility(hops: &[(Position, Result<(), Report>)]) -> Mobility {
    if let Some((hop, _)) = hops.iter().find(|(_, answer)| answer.is_ok()) {
        return Mobility::Free { hop: hop.clone() };
    }
    if hops.is_empty() {
        return Mobility::Unknown;
    }
    let all_refused = hops
        .iter()
        .all(|(_, answer)| answer.as_ref().err().is_some_and(pathfinder_found_nothing));
    if all_refused {
        Mobility::BoxedIn {
            refused_hops: u8::try_from(hops.len()).unwrap_or(u8::MAX),
        }
    } else {
        Mobility::Unknown
    }
}

/// Asks the game whether `player` can leave the spot it stands on, and
/// judges the answer. See [`judge_mobility`] for the verdicts and
/// `FactorioRcon::probe_player_hops` for the question.
pub async fn probe_mobility(
    rcon: &FactorioRcon,
    world: &Arc<FactorioWorld>,
    player: PlayerId,
    at: &Position,
) -> Mobility {
    judge_mobility(&rcon.probe_player_hops(world, player, at).await)
}

/// Writes a mobility verdict into the world's bench ledger. Returns whether
/// the ledger changed.
///
/// [`Mobility::BoxedIn`] benches the player at `at`; [`Mobility::Free`]
/// lifts a bench it had, as [`BenchRelease::Probed`]; [`Mobility::Unknown`]
/// touches nothing. Every branch logs, for the reason [`note_walk_refusal`]
/// gives: this ledger decides what the next plan may ask of a bot, and a
/// decision that reaches no log is a decision nobody can audit.
pub fn note_mobility(
    world: &FactorioWorld,
    player: PlayerId,
    at: &Position,
    tick: Option<u64>,
    verdict: &Mobility,
) -> bool {
    match verdict {
        Mobility::BoxedIn { refused_hops } => {
            let changed = world.record_bench(Bench {
                tick,
                player,
                at: at.clone(),
                refused_hops: *refused_hops,
                hop_tiles: HOP_DISTANCE,
            });
            warn!(
                player,
                at = %at,
                refused_hops,
                hop_tiles = HOP_DISTANCE,
                changed,
                "BENCHED: the game refused every short hop from where this character \
                 stands, so it cannot leave its own tile -- the next plan gives it no \
                 step that needs it to walk, until a walk succeeds or a re-probe finds \
                 a way out"
            );
            changed
        }
        Mobility::Free { hop } => {
            let released = world.release_bench(player, tick, BenchRelease::Probed);
            if released {
                warn!(
                    player,
                    at = %at,
                    hop = %hop,
                    "RELEASED: the game will now path this character to a short hop, so \
                     its bench is lifted and the next plan may send it"
                );
            } else {
                info!(
                    player,
                    at = %at,
                    hop = %hop,
                    "the game paths this character to a short hop from where it stands, \
                     so what was unreachable is the destination, not the bot"
                );
            }
            released
        }
        Mobility::Unknown => {
            info!(
                player,
                at = %at,
                "the mobility probe established nothing -- no hop was pathed and not \
                 every answer was the pathfinder saying no -- so the bench ledger is \
                 left as it was"
            );
            false
        }
    }
}

/// A walk for `player` succeeded, which is proof enough that it can move:
/// lifts its bench if it had one. Returns whether it did.
///
/// This is how a bot pushed clear by something the plan did not ask for --
/// another bot's step-aside, a recovery teleport, a mined-away wall -- gets
/// back into the roster without waiting for a re-probe.
pub fn note_walk_succeeded(world: &FactorioWorld, player: PlayerId, tick: Option<u64>) -> bool {
    let released = world.release_bench(player, tick, BenchRelease::Walked);
    if released {
        warn!(
            player,
            "RELEASED: a walk for this benched character succeeded, so its bench is \
             lifted and the next plan may send it"
        );
    }
    released
}

/// Re-asks the game about every benched character, from where it stands
/// now, and lifts the bench of each one it will path. Returns the players
/// released.
///
/// Called before a plan is made (`crates/scripting_lua`'s buffer refresher),
/// because that is the moment the answer matters: a bench is what keeps the
/// planner from re-sending a bot the game refused, and the only party that
/// can say the refusal has ended is the game. A bot whose position the world
/// has lost is probed from where it was benched, which is the last place it
/// was known to be.
pub async fn reprobe_benched(rcon: &FactorioRcon, world: &Arc<FactorioWorld>) -> Vec<PlayerId> {
    let mut released = Vec::new();
    for bench in world.benches() {
        let at = world
            .players
            .get(&bench.player)
            .map(|player| player.position.clone())
            .unwrap_or_else(|| bench.at.clone());
        let verdict = probe_mobility(rcon, world, bench.player, &at).await;
        if matches!(verdict, Mobility::Free { .. })
            && note_mobility(world, bench.player, &at, rcon.last_tick(), &verdict)
        {
            released.push(bench.player);
        }
    }
    released
}

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
        // Carefully *not* "the character is not boxed in". This fill reasons
        // over a model that holds no characters and seeds from a tile centre;
        // `run-1788614781-38058`'s bot 6 stood overlapping a furnace, this
        // arm said the destination was the problem, and the game refused
        // every path from the spot. Whether the character itself can move is
        // the mobility probe's answer (`note_mobility`), not this one's.
        Escape::Open => info!(
            player,
            at = %from,
            searched_tiles = SEARCH_RADIUS,
            "the occupancy model finds no enclosure here: the free region around \
             the tile reaches the edge of the searched window. That says nothing \
             about whether the character itself can leave its tile -- the game's \
             hop probe answers that"
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

    // -----------------------------------------------------------------------
    // The mobility probe: the game's verdict on the character, judged here.
    // -----------------------------------------------------------------------

    fn refused(reason: &str) -> Report {
        RconPathRequestFailed {
            reason: format!("Error: {reason}"),
        }
        .into()
    }

    /// Four hops, each with an answer, as `probe_player_hops` returns them.
    fn hops(answers: [Result<(), Report>; 4]) -> Vec<(Position, Result<(), Report>)> {
        factorio_bot_core::factorio::world::hop_targets(&here())
            .into_iter()
            .zip(answers)
            .collect()
    }

    /// Bot 6 of `run-1788614781-38058`, as the game would have answered had
    /// anyone asked: no route three tiles in any direction.
    #[test]
    fn every_hop_refused_is_boxed_in() {
        let verdict = judge_mobility(&hops([
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
        ]));
        assert_eq!(verdict, Mobility::BoxedIn { refused_hops: 4 });
    }

    /// One route is enough: the claim is only "it can leave its tile".
    #[test]
    fn one_pathed_hop_is_free_and_names_the_hop() {
        let targets = factorio_bot_core::factorio::world::hop_targets(&here());
        let verdict = judge_mobility(&hops([
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
            Ok(()),
            Err(refused(NO_PATH)),
        ]));
        assert_eq!(
            verdict,
            Mobility::Free {
                hop: targets[2].clone()
            }
        );
    }

    /// A full queue in one direction is not the pathfinder saying no, and a
    /// bench needs the game's no in every direction.
    #[test]
    fn a_busy_answer_among_refusals_is_unknown_not_boxed_in() {
        let verdict = judge_mobility(&hops([
            Err(refused(NO_PATH)),
            Err(refused(BUSY)),
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
        ]));
        assert_eq!(verdict, Mobility::Unknown);
    }

    /// Nor is a timeout, which is not the pathfinder answering at all.
    #[test]
    fn a_timeout_among_refusals_is_unknown() {
        let verdict = judge_mobility(&hops([
            Err(refused(NO_PATH)),
            Err(refused(NO_PATH)),
            Err(RconTimeout {}.into()),
            Err(refused(NO_PATH)),
        ]));
        assert_eq!(verdict, Mobility::Unknown);
    }

    /// No question asked, no answer claimed.
    #[test]
    fn an_empty_probe_is_unknown() {
        assert_eq!(judge_mobility(&[]), Mobility::Unknown);
    }

    /// The bench: a boxed-in verdict is written where the next plan reads.
    #[test]
    fn a_boxed_in_verdict_benches_the_bot_where_it_stands() {
        let world = FactorioWorld::new();
        let verdict = Mobility::BoxedIn { refused_hops: 4 };
        assert!(note_mobility(&world, 6, &here(), None, &verdict));
        let benches = world.benches();
        assert_eq!(benches.len(), 1);
        assert_eq!(benches[0].player, 6);
        assert_eq!(benches[0].at, here());
        assert_eq!(benches[0].refused_hops, 4);
        assert_eq!(benches[0].hop_tiles, HOP_DISTANCE);
        assert!(world.is_benched(6));
        // The same verdict from the same spot is the same bench, not a
        // second row for the record.
        assert!(!note_mobility(&world, 6, &here(), None, &verdict));
        assert_eq!(world.benches().len(), 1);
    }

    /// The record sees the transition once, and the ledger keeps the state.
    #[test]
    fn a_bench_is_queued_for_the_record_once() {
        use factorio_bot_core::factorio::world::BenchChange;
        let world = FactorioWorld::new();
        let verdict = Mobility::BoxedIn { refused_hops: 4 };
        note_mobility(&world, 6, &here(), Some(26_953), &verdict);
        note_mobility(&world, 6, &here(), Some(30_587), &verdict);
        let changes = world.drain_bench_changes();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], BenchChange::Benched(bench) if bench.player == 6));
        assert!(
            world.drain_bench_changes().is_empty(),
            "drained means drained"
        );
        assert!(
            world.is_benched(6),
            "draining the events does not lift the bench"
        );
    }

    /// Free from where it stands lifts the bench, and says why.
    #[test]
    fn a_free_verdict_releases_a_benched_bot() {
        use factorio_bot_core::factorio::world::{BenchChange, BenchRelease};
        let world = FactorioWorld::new();
        note_mobility(
            &world,
            6,
            &here(),
            None,
            &Mobility::BoxedIn { refused_hops: 4 },
        );
        world.drain_bench_changes();
        let free = Mobility::Free { hop: there() };
        assert!(note_mobility(&world, 6, &here(), Some(40_000), &free));
        assert!(!world.is_benched(6));
        let changes = world.drain_bench_changes();
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            BenchChange::Released {
                player: 6,
                why: BenchRelease::Probed,
                tick: Some(40_000),
                ..
            }
        ));
        // Free for a bot that was never benched changes nothing.
        assert!(!note_mobility(&world, 3, &here(), None, &free));
        assert!(world.drain_bench_changes().is_empty());
    }

    /// Unknown is not a verdict, and a bench survives it.
    #[test]
    fn an_unknown_verdict_leaves_the_bench_as_it_was() {
        let world = FactorioWorld::new();
        note_mobility(
            &world,
            6,
            &here(),
            None,
            &Mobility::BoxedIn { refused_hops: 4 },
        );
        assert!(!note_mobility(&world, 6, &here(), None, &Mobility::Unknown));
        assert!(world.is_benched(6));
        assert!(!note_mobility(&world, 3, &here(), None, &Mobility::Unknown));
        assert!(!world.is_benched(3));
    }

    /// A walk that succeeded is the plainest proof of movement there is.
    #[test]
    fn a_successful_walk_releases_the_bench() {
        use factorio_bot_core::factorio::world::{BenchChange, BenchRelease};
        let world = FactorioWorld::new();
        note_mobility(
            &world,
            6,
            &here(),
            None,
            &Mobility::BoxedIn { refused_hops: 4 },
        );
        world.drain_bench_changes();
        assert!(note_walk_succeeded(&world, 6, Some(41_000)));
        assert!(!world.is_benched(6));
        let changes = world.drain_bench_changes();
        assert!(matches!(
            &changes[..],
            [BenchChange::Released {
                player: 6,
                why: BenchRelease::Walked,
                ..
            }]
        ));
        assert!(
            !note_walk_succeeded(&world, 1, Some(41_000)),
            "a bot that was never benched has nothing to release"
        );
    }

    /// A bench earned somewhere else is about somewhere else: the bot moved
    /// between the two, so the new spot replaces the old and the record is
    /// told where it is stuck now.
    #[test]
    fn a_bench_earned_elsewhere_replaces_the_old_one() {
        let world = FactorioWorld::new();
        let verdict = Mobility::BoxedIn { refused_hops: 4 };
        note_mobility(&world, 6, &here(), None, &verdict);
        assert!(note_mobility(&world, 6, &there(), None, &verdict));
        let benches = world.benches();
        assert_eq!(benches.len(), 1, "one bench per player");
        assert_eq!(benches[0].at, there());
        assert_eq!(world.drain_bench_changes().len(), 2);
    }
}
