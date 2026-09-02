//! A character standing on the ground is an obstacle, and the planner has to
//! know it.
//!
//! Run `run-1788319014-01846`, rung 4 (`research automation`), reached 92-step
//! plans and then failed five placements in a row with
//!
//! ```text
//! cannot place item 'stone-furnace' because surface.can_place_entity said 'no'
//! ```
//!
//! The record says which character. The rung sized its split down to one bot
//! (`bots: [1]`), so bots 2 and 3 were left standing wherever rung 2 had last
//! sent them; `samples.jsonl` has bot 2 at `(-18.2421875, 51.28125)`,
//! motionless from tick 7200 to the end of the run. The keyframe in
//! `map.jsonl` shows nothing on the target tile in either the game's entities
//! or the model's — characters are filtered out of both sides — so the
//! obstruction could only be one of the classes a keyframe cannot show, and
//! the samples name it.
//!
//! BotBridge's own message corroborates the identity: `rcon_place_entity`
//! answers `§player_blocks_placement§` when the *acting* player is inside the
//! footprint and the generic text otherwise. Every one of the five refusals
//! was generic — bot 1 was clear, someone else was not.
//!
//! The numbers below are that run's, unchanged.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{PlayerChangedPositionEvent, Position};
use factorio_bot_planner::method::util::free_area_near;
use factorio_bot_planner::{BotId, PlanState};
use std::sync::Arc;

/// Where bot 2 was parked for the last 14 000 ticks of the run.
const PARKED: (f64, f64) = (-18.2421875, 51.28125);

/// The two sites rung 4 kept choosing, and the game kept refusing.
const REFUSED_SITES: [(f64, f64); 2] = [(-19., 51.), (-18., 51.)];

fn state_with_parked_bot(parked: Option<(f64, f64)>, roster: &[BotId]) -> PlanState {
    let world = fixture_world();
    if let Some((x, y)) = parked {
        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: 2,
                position: Position::new(x, y),
            })
            .expect("park a player");
    }
    PlanState::from_world(Arc::new(world), roster)
}

/// The control: with nobody standing there, both sites are open ground.
///
/// Without this the regression below would pass against a fixture that
/// happened to have a tree on the tile, and would say nothing about
/// characters at all.
#[test]
fn the_refused_sites_are_clear_when_no_one_is_standing_on_them() {
    let state = state_with_parked_bot(None, &[BotId(1)]);
    for (x, y) in REFUSED_SITES {
        assert!(
            state.is_area_free("stone-furnace", &Position::new(x, y)),
            "[{x}, {y}] has nothing on it in the fixture"
        );
    }
}

/// The regression: a bot the plan cannot move occupies the ground it stands on.
#[test]
fn a_parked_bot_outside_the_roster_blocks_a_placement() {
    let state = state_with_parked_bot(Some(PARKED), &[BotId(1)]);
    for (x, y) in REFUSED_SITES {
        assert!(
            !state.is_area_free("stone-furnace", &Position::new(x, y)),
            "a stone furnace at [{x}, {y}] covers bot 2 at {PARKED:?}; \
             the game refused this five times in run-1788319014-01846"
        );
    }
}

/// And the search moves on rather than returning the site anyway.
///
/// `free_area_near` is what sites a furnace. It walks outward in rings and
/// returns the first candidate `is_area_free` accepts, so the assertion that
/// matters is that what it hands back is somewhere the character is not.
#[test]
fn the_site_search_walks_past_a_parked_bot() {
    let state = state_with_parked_bot(Some(PARKED), &[BotId(1)]);
    let from = Position::new(REFUSED_SITES[1].0, REFUSED_SITES[1].1);
    let found = free_area_near(&state, &from, "stone-furnace").expect("open ground nearby");
    assert!(
        state.is_area_free("stone-furnace", &found),
        "the search must not return a site it would itself refuse"
    );
    for (x, y) in REFUSED_SITES {
        assert!(
            found != Position::new(x, y),
            "[{x}, {y}] is occupied and must not be chosen"
        );
    }
}

/// A bot **in** the roster does not fence itself out of its own site.
///
/// `base.players` holds where a roster bot *started*; the plan moves it, and
/// the acting bot is kept out of its own footprint by `Condition::AtPosition`'s
/// `min_radius` instead. Treating a roster bot's starting position as a
/// permanent obstacle would refuse a placement the bot is about to walk away
/// from — the opposite mistake, and just as silent.
#[test]
fn a_bot_in_the_roster_does_not_block_its_own_placement() {
    let state = state_with_parked_bot(Some(PARKED), &[BotId(1), BotId(2)]);
    for (x, y) in REFUSED_SITES {
        assert!(
            state.is_area_free("stone-furnace", &Position::new(x, y)),
            "bot 2 is in the roster, so the plan can move it off [{x}, {y}]"
        );
    }
}

/// Only the ground the character actually covers is refused.
///
/// A character's collision box is ±0.19921875, so the block is a little over a
/// third of a tile — not a tile, and not a furnace-sized exclusion zone. One
/// tile further out in either direction is open.
#[test]
fn the_block_is_the_character_box_and_no_larger() {
    let state = state_with_parked_bot(Some(PARKED), &[BotId(1)]);
    for site in [
        Position::new(-21., 51.),
        Position::new(-16., 51.),
        Position::new(-18., 48.),
        Position::new(-18., 54.),
    ] {
        assert!(
            state.is_area_free("stone-furnace", &site),
            "{site:?} is clear of the character at {PARKED:?}"
        );
    }
}
