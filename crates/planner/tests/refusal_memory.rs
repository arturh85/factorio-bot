//! A site the game refused is a site the next plan must not choose.
//!
//! Four runs died on one message:
//!
//! ```text
//! cannot place item 'stone-furnace' because surface.can_place_entity said 'no'
//! ```
//!
//! and three separate causes were found and fixed behind it — a forest whose
//! trees were in a tree `is_area_clear` did not read, a non-roster character
//! standing in the footprint, and a roster character doing the same
//! (`docs/superpowers/notes/2026-09-02-placement-refusal{,-2,-3}.md`). Each fix
//! was right and each uncovered the next cause, which is the evidence that the
//! planner's model of the ground cannot be made to predict `can_place_entity`
//! by adding one more source at a time.
//!
//! What every one of those runs also did was **re-choose the tile the game had
//! just refused** — twice in one milestone in `run-1788323755-24892` — because
//! a refusal was information nothing carried from the run that earned it to the
//! plan that came next. These tests pin the carrier: a refusal recorded on the
//! world excludes the footprint the game tested, for the rest of the run, from
//! every later `PlanState`.
//!
//! The coordinates are the real runs', unchanged.

use factorio_bot_core::factorio::world::{FactorioSurface, PlacementRefusal};
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{PlayerChangedPositionEvent, Position};
use factorio_bot_planner::method::util::free_area_near;
use factorio_bot_planner::{BotId, PlanState};
use std::sync::Arc;

/// The site `run-1788322836-81715`'s rung 4 chose three times and
/// `run-1788323755-24892` chose again. A stone furnace centred here spans
/// `[-16.8, -15.2] x [-58.8, -57.2]`.
const REFUSED_SITE: (f64, f64) = (-16., -58.);

/// A furnace is 1.398 tiles across, so a candidate one tile away still covers
/// most of the refused box: `[-17.8, -16.2]` overlaps `[-16.8, -15.2]`.
const OVERLAPPING_NEIGHBOUR: (f64, f64) = (-17., -58.);

/// Two tiles away the boxes are disjoint — `[-18.8, -17.2]` against
/// `[-16.8, -15.2]` — so this is the nearest site the refusal says nothing
/// about, and the one the search should reach.
const CLEAR_NEIGHBOUR: (f64, f64) = (-18., -58.);

fn refused(entity: &str, (x, y): (f64, f64)) -> PlacementRefusal {
    PlacementRefusal::at_dispatch(Some(6198), entity, Position::new(x, y), 0, Vec::new(), None)
}

/// A fixture world with `refusals` already recorded, the way a run's
/// `FactorioWorld` carries them from one plan to the next.
fn world_with(refusals: &[PlacementRefusal]) -> Arc<FactorioSurface> {
    let world = fixture_world();
    for refusal in refusals {
        world.record_placement_refusal(refusal.clone());
    }
    Arc::new(world)
}

fn state_with(refusals: &[PlacementRefusal]) -> PlanState {
    PlanState::from_world(world_with(refusals), &[BotId(1)])
}

/// The control. Without it every assertion below would also pass against a
/// fixture that happened to have a tree on the tile.
#[test]
fn the_site_is_open_ground_until_the_game_says_otherwise() {
    let state = state_with(&[]);
    for site in [REFUSED_SITE, OVERLAPPING_NEIGHBOUR, CLEAR_NEIGHBOUR] {
        assert!(
            state.is_area_free("stone-furnace", &Position::new(site.0, site.1)),
            "{site:?} has nothing on it in the fixture, so a refusal is the \
             only thing that can make it unavailable"
        );
    }
}

/// The defect, from the planner's side: the same site, twice.
#[test]
fn a_refused_site_is_not_offered_to_the_next_plan() {
    let state = state_with(&[refused("stone-furnace", REFUSED_SITE)]);
    assert!(
        !state.is_area_free(
            "stone-furnace",
            &Position::new(REFUSED_SITE.0, REFUSED_SITE.1)
        ),
        "the game refused a stone furnace at {REFUSED_SITE:?}; a plan built \
         after that must not offer the site again"
    );
}

/// And the search that sites a furnace walks past it, rather than the refusal
/// merely making `free_area_near` fail.
///
/// This is the whole behavioural claim: the refusal narrows the *site*, not
/// the search. Nothing about the ring order or the radius changes; one set of
/// candidates simply stops being available and the existing outward walk finds
/// the next.
#[test]
fn the_site_search_walks_past_a_refused_site() {
    let from = Position::new(REFUSED_SITE.0, REFUSED_SITE.1);
    let before = free_area_near(&state_with(&[]), &from, "stone-furnace")
        .expect("open ground at the anchor itself");
    assert_eq!(
        before, from,
        "with nothing refused the search returns the anchor, which is what \
         makes the next assertion about the refusal and not about the fixture"
    );

    let state = state_with(&[refused("stone-furnace", REFUSED_SITE)]);
    let found = free_area_near(&state, &from, "stone-furnace").expect("the map is not fenced off");
    assert_ne!(
        found, from,
        "the refused site must not come back out of the search"
    );
    assert!(
        state.is_area_free("stone-furnace", &found),
        "the search must not return a site it would itself refuse"
    );
}

/// The refusal covers the box the game tested, not the tile at its centre.
///
/// The game was asked whether a 1.398-tile box centred at `REFUSED_SITE` fits
/// and said no. Nothing in that answer says *where* in the box the blocker is,
/// so a candidate whose own box overlaps it is a candidate that may well cover
/// the same unknown obstacle. Excluding only the exact tile would let the
/// planner shuffle one tile at a time into the same refusal, spending an
/// iteration per step against a stall limit of three.
#[test]
fn a_refusal_excludes_the_footprint_the_game_tested() {
    let state = state_with(&[refused("stone-furnace", REFUSED_SITE)]);
    assert!(
        !state.is_area_free(
            "stone-furnace",
            &Position::new(OVERLAPPING_NEIGHBOUR.0, OVERLAPPING_NEIGHBOUR.1)
        ),
        "a furnace at {OVERLAPPING_NEIGHBOUR:?} still covers most of the box \
         the game refused, so it is not evidence of anything different"
    );
}

/// And no further. The exclusion is bounded by the refused box: the first
/// candidate whose own box clears it is available again.
///
/// This is the counterweight to believing a refusal for the whole run. The
/// belief is durable, but it is small — a refused furnace costs the planner a
/// two-tile neighbourhood out of `free_area_near`'s 625-candidate window, and
/// only ever ground the game itself turned down.
#[test]
fn the_exclusion_stops_at_the_edge_of_the_refused_box() {
    let state = state_with(&[refused("stone-furnace", REFUSED_SITE)]);
    assert!(
        state.is_area_free(
            "stone-furnace",
            &Position::new(CLEAR_NEIGHBOUR.0, CLEAR_NEIGHBOUR.1)
        ),
        "{CLEAR_NEIGHBOUR:?}'s box is disjoint from the refused one; the game \
         said nothing about it and neither should we"
    );
}

/// A refusal about one entity is not a refusal about the map.
///
/// Only the ground under the refused footprint is excluded; everywhere else
/// the world is unchanged, which is what keeps a run's accumulated refusals
/// from fencing the planner off a legitimately buildable map.
#[test]
fn a_refusal_leaves_the_rest_of_the_map_alone() {
    let state = state_with(&[refused("stone-furnace", REFUSED_SITE)]);
    let far = Position::new(0., 0.);
    assert!(
        state.is_area_free("stone-furnace", &far),
        "the origin is nowhere near {REFUSED_SITE:?} and must stay buildable"
    );
    assert_eq!(
        state.refused_footprints().len(),
        1,
        "one refusal, one excluded footprint"
    );
}

/// Refusals accumulate, and each one excludes its own box.
#[test]
fn every_refusal_is_kept_not_just_the_last() {
    let state = state_with(&[
        refused("stone-furnace", REFUSED_SITE),
        refused("stone-furnace", CLEAR_NEIGHBOUR),
    ]);
    assert_eq!(state.refused_footprints().len(), 2);
    for site in [REFUSED_SITE, CLEAR_NEIGHBOUR] {
        assert!(
            !state.is_area_free("stone-furnace", &Position::new(site.0, site.1)),
            "{site:?} was refused and stays refused"
        );
    }
}

/// The same site refused twice is one exclusion, not two.
#[test]
fn the_same_site_refused_twice_is_remembered_once() {
    let world = world_with(&[
        refused("stone-furnace", REFUSED_SITE),
        refused("stone-furnace", REFUSED_SITE),
    ]);
    assert_eq!(
        world.placement_refusals().len(),
        1,
        "the run refused this site three times in a row; the ledger holds one"
    );
}

/// Two plans built from the same world see the same exclusions, in the same
/// order, whatever order the game happened to refuse them in.
///
/// The planner is pure and its determinism is pinned by
/// `expansion_is_deterministic`; a memory that made two plans differ on
/// identical inputs would break that guarantee rather than extend it. The
/// footprints are sorted by geometry in `from_world` precisely so arrival
/// order cannot leak in.
#[test]
fn refusals_reach_the_planner_in_a_deterministic_order() {
    let forwards = state_with(&[
        refused("stone-furnace", REFUSED_SITE),
        refused("stone-furnace", CLEAR_NEIGHBOUR),
    ]);
    let backwards = state_with(&[
        refused("stone-furnace", CLEAR_NEIGHBOUR),
        refused("stone-furnace", REFUSED_SITE),
    ]);
    assert_eq!(
        forwards.refused_footprints(),
        backwards.refused_footprints(),
        "the order the game refused two sites in must not reach the plan"
    );
    let again = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
    assert!(
        again.refused_footprints().is_empty(),
        "a world that has been refused nothing excludes nothing"
    );
}

/// A refusal is remembered even when a character explains it.
///
/// Deliberate, and worth an assertion because it looks like a case for
/// forgetting: if a bot was standing there, the tile frees up when it walks
/// away. The refusal message names no cause — that is the whole difficulty —
/// so "a character was near it" is a coincidence the planner cannot promote
/// to an explanation. The `characters` source already handles the case it
/// really is one, from the world, on every plan; this source exists for the
/// cases it is not.
#[test]
fn a_refusal_is_kept_even_with_a_character_nearby() {
    let world = fixture_world();
    world
        .player_changed_position(PlayerChangedPositionEvent {
            player_id: 3,
            position: Position::new(-15.328125, -58.2890625),
        })
        .expect("park a player");
    world.record_placement_refusal(refused("stone-furnace", REFUSED_SITE));
    let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
    assert_eq!(state.refused_footprints().len(), 1);
    assert!(
        !state.is_area_free(
            "stone-furnace",
            &Position::new(OVERLAPPING_NEIGHBOUR.0, OVERLAPPING_NEIGHBOUR.1)
        ),
        "the neighbour is clear of bot 3's 0.4-tile box but not of the box \
         the game refused, and only one of those two is a verdict"
    );
}
