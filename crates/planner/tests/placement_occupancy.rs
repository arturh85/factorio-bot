//! A character standing on the ground is an obstacle, and the planner has to
//! know it — whether or not the character is one of the plan's own bots.
//!
//! Two runs, one message, and the second one is why the roster is not carved
//! out of the first one's fix.
//!
//! # `run-1788319014-01846`
//!
//! Rung 4 (`research automation`) reached 92-step plans and then failed five
//! placements in a row with
//!
//! ```text
//! cannot place item 'stone-furnace' because surface.can_place_entity said 'no'
//! ```
//!
//! `samples.jsonl` has bot 2 at `(-18.2421875, 51.28125)`, motionless from
//! tick 7200 to the end of the run, inside both refused footprints. The
//! keyframe in `map.jsonl` shows nothing on either target tile in the game's
//! entities or the model's — characters are filtered out of both sides — so
//! the obstruction could only be one of the classes a keyframe cannot show,
//! and the samples name it.
//!
//! BotBridge's own message corroborates the identity: `rcon_place_entity`
//! answers `§player_blocks_placement§` when the *acting* player is inside the
//! footprint and the generic text otherwise. Every refusal in both runs was
//! generic — the builder was clear, someone else was not.
//!
//! # `run-1788322836-81715`
//!
//! Same message, same call, one run's fix in place and useless. That fix held
//! only the characters the roster did **not** name, and this run's roster was
//! the whole connected game (`rcon.players()` returned `[2, 3, 4]`), so the
//! filter emptied it of everything that mattered. Rung 4 planned 114 steps and
//! put every one of them on bot 2. Bot 3 was standing at
//! `(-15.328125, -58.2890625)` where rung 2's copper had left it; the planner
//! sited a stone furnace at `[-16, -58]`, over bot 3 by a third of a tile, and
//! the game refused it three times before the milestone gave up.
//!
//! Being on the roster is not a promise that the plan will move you.
//!
//! The numbers below are those runs', unchanged.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{PlayerChangedPositionEvent, Position};
use factorio_bot_planner::method::util::free_area_near;
use factorio_bot_planner::{BotId, PlanState};
use std::sync::Arc;

/// Where bot 2 was parked for the last 14 000 ticks of `run-1788319014-01846`.
const PARKED: (f64, f64) = (-18.2421875, 51.28125);

/// The two sites that run's rung 4 kept choosing, and the game kept refusing.
const REFUSED_SITES: [(f64, f64); 2] = [(-19., 51.), (-18., 51.)];

/// Where bot 3 stood for the whole of `run-1788322836-81715`'s rung 4, having
/// finished rung 2's copper there and been given no work since.
const PARKED_ROSTER_BOT: (f64, f64) = (-15.328125, -58.2890625);

/// The site that run's rung 4 chose three times, and the game refused three
/// times. Its box spans `[-16.8, -15.2] x [-58.8, -57.2]`.
const REFUSED_ROSTER_SITE: (f64, f64) = (-16., -58.);

/// `fixture_world` has no players at all, so a parked bot is exactly the one
/// character in the world and nothing else can account for a refusal.
fn state_with_parked_bot(parked: &[(u8, (f64, f64))], roster: &[BotId]) -> PlanState {
    let world = fixture_world();
    for (player_id, (x, y)) in parked {
        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: *player_id,
                position: Position::new(*x, *y),
            })
            .expect("park a player");
    }
    PlanState::from_world(Arc::new(world), roster)
}

/// The control: with nobody standing there, every site in play is open ground.
///
/// Without this the regressions below would pass against a fixture that
/// happened to have a tree on the tile, and would say nothing about characters
/// at all.
#[test]
fn the_refused_sites_are_clear_when_no_one_is_standing_on_them() {
    let state = state_with_parked_bot(&[], &[BotId(1)]);
    for (x, y) in REFUSED_SITES.iter().copied().chain([REFUSED_ROSTER_SITE]) {
        assert!(
            state.is_area_free("stone-furnace", &Position::new(x, y)),
            "[{x}, {y}] has nothing on it in the fixture"
        );
    }
}

/// `run-1788319014-01846`: a bot outside the roster occupies the ground it
/// stands on.
#[test]
fn a_parked_bot_outside_the_roster_blocks_a_placement() {
    let state = state_with_parked_bot(&[(2, PARKED)], &[BotId(1)]);
    for (x, y) in REFUSED_SITES {
        assert!(
            !state.is_area_free("stone-furnace", &Position::new(x, y)),
            "a stone furnace at [{x}, {y}] covers bot 2 at {PARKED:?}; \
             the game refused this five times in run-1788319014-01846"
        );
    }
}

/// `run-1788322836-81715`: and so does a bot **inside** it.
///
/// This is the assertion that used to run the other way. A roster bot is a bot
/// the plan *may* move, not one it will: which bots a plan moves is not settled
/// until `schedule` has assigned the work, and this rung assigned all 114 steps
/// to bot 2. Bot 3 was as immovable as any stranger.
#[test]
fn a_parked_bot_inside_the_roster_blocks_a_placement_too() {
    let state = state_with_parked_bot(&[(3, PARKED_ROSTER_BOT)], &[BotId(2), BotId(3), BotId(4)]);
    let (x, y) = REFUSED_ROSTER_SITE;
    assert!(
        !state.is_area_free("stone-furnace", &Position::new(x, y)),
        "a stone furnace at [{x}, {y}] covers bot 3 at {PARKED_ROSTER_BOT:?}; \
         bot 3 is on the roster and the plan still gave it nothing to do, so \
         nothing was ever going to move it"
    );
}

/// And the search moves on rather than returning the site anyway.
///
/// `free_area_near` is what sites a furnace. It walks outward in rings and
/// returns the first candidate `is_area_free` accepts, so the assertion that
/// matters is that what it hands back is somewhere the character is not.
#[test]
fn the_site_search_walks_past_a_parked_bot() {
    let state = state_with_parked_bot(&[(2, PARKED)], &[BotId(1)]);
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

/// The roster case reaches the same search, and it too walks past.
///
/// The cost of blocking on a roster bot is one ring of this search, which is
/// the whole argument for taking the conservative direction: the other one
/// costs a milestone.
#[test]
fn the_site_search_walks_past_a_parked_roster_bot() {
    let state = state_with_parked_bot(&[(3, PARKED_ROSTER_BOT)], &[BotId(2), BotId(3), BotId(4)]);
    let from = Position::new(REFUSED_ROSTER_SITE.0, REFUSED_ROSTER_SITE.1);
    let found = free_area_near(&state, &from, "stone-furnace").expect("open ground nearby");
    assert!(
        state.is_area_free("stone-furnace", &found),
        "the search must not return a site it would itself refuse"
    );
    assert!(
        found != Position::new(REFUSED_ROSTER_SITE.0, REFUSED_ROSTER_SITE.1),
        "{REFUSED_ROSTER_SITE:?} is occupied and must not be chosen"
    );
}

/// Only the ground the character actually covers is refused.
///
/// A character's collision box is ±0.19921875, so the block is a little over a
/// third of a tile — not a tile, and not a furnace-sized exclusion zone. One
/// tile further out in either direction is open.
#[test]
fn the_block_is_the_character_box_and_no_larger() {
    let state = state_with_parked_bot(&[(2, PARKED)], &[BotId(1)]);
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

// ---------------------------------------------------------------------------
// `run-1788353986-24634` (run 27): the model was right and the input was
// stale, which looks exactly the same from the plan.
// ---------------------------------------------------------------------------

/// The last position run 27's server ever reported for bot 3, at tick 18187.
///
/// From `workspace/server-log.txt`:
///
/// ```text
/// §18187§on_player_changed_position§{"player_id":3,"position":{"y":16.96484375,"x":-23.19140625}}
/// ```
///
/// Nothing followed it. Factorio raises `on_player_changed_position` once per
/// **tile** crossed, so a character that stops part-way into a tile is last
/// heard from at the boundary it entered by.
const RUN27_REPORTED: (f64, f64) = (-23.19140625, 16.96484375);

/// Where bot 3 actually stood, from run 27's own `samples.jsonl`, unchanged
/// from tick 18240 to the end of the run.
///
/// Both points are inside tile `(-24, 16)`. The gap between them is 0.825
/// tiles, and it is permanent: a parked character raises no further events, so
/// nothing corrects it.
const RUN27_RESTED: (f64, f64) = (-23.5078125, 16.203125);

/// The site milestone 6 chose, dispatched, and had refused — three plans in a
/// row, at ticks 22713, 27281 and 31329.
const RUN27_SITE: (f64, f64) = (-23., 16.);

/// **The whole of cause seven in one assertion.**
///
/// A stone furnace's collision box is ±0.69921875 and a character's is
/// ±0.19921875 (both read off run 27's own `entity_prototypes` writeout, not
/// from memory). Centred on `RUN27_SITE` the furnace spans
/// `[-23.699, -22.301] x [15.301, 16.699]`.
///
/// * At `RUN27_REPORTED` the character spans `y [16.766, 17.164]` — clear of
///   the furnace by 0.067 tiles, so the planner sited there and was **right to,
///   given what it was told**.
/// * At `RUN27_RESTED` it spans `y [16.004, 16.402]` — squarely inside, which
///   is why `can_place_entity` said no every time.
///
/// So the planner's `characters` occupancy source was never the problem: the
/// six sources are a model of the world, and this one was being fed a position
/// the world had grown out of. The fix is upstream, in `mods/BotBridge`'s
/// walker reporting where a character comes to rest
/// (`crates/core/tests/botbridge_rest_position.rs`), and this test is the
/// arithmetic that says why 0.825 tiles was enough to cost a milestone.
#[test]
fn run27s_reported_position_clears_the_site_and_its_real_one_does_not() {
    let believed = state_with_parked_bot(&[(3, RUN27_REPORTED)], &[BotId(1), BotId(3)]);
    let (x, y) = RUN27_SITE;
    assert!(
        believed.is_area_free("stone-furnace", &Position::new(x, y)),
        "with bot 3 believed at {RUN27_REPORTED:?} the site is open, and the \
         planner choosing it is correct reasoning from a stale fact. If this \
         ever fails the diagnosis of run 27 is wrong and the note should be \
         reopened"
    );

    let truth = state_with_parked_bot(&[(3, RUN27_RESTED)], &[BotId(1), BotId(3)]);
    assert!(
        !truth.is_area_free("stone-furnace", &Position::new(x, y)),
        "with bot 3 where it actually stood, the same site is blocked — so \
         nothing in the planner needed changing, only what it is told"
    );
}

/// And with the truth in hand the search moves on, so an accurate position is
/// the whole fix at this end: no re-siting loop, no refusal ledger entry, no
/// iteration spent.
#[test]
fn the_site_search_walks_past_run27s_parked_bot_once_it_is_told_the_truth() {
    let state = state_with_parked_bot(&[(3, RUN27_RESTED)], &[BotId(1), BotId(3)]);
    let from = Position::new(RUN27_SITE.0, RUN27_SITE.1);
    let found = free_area_near(&state, &from, "stone-furnace").expect("open ground nearby");
    assert!(
        found != Position::new(RUN27_SITE.0, RUN27_SITE.1),
        "the site the game refused three times must not be chosen a fourth"
    );
    assert!(
        state.is_area_free("stone-furnace", &found),
        "the search must not return a site it would itself refuse"
    );
}

/// The planner is pure and deterministic, and a position arriving later must
/// not make it less so: the same world built twice must choose the same site.
///
/// Cheap to assert and worth asserting here specifically, because the fix this
/// file documents makes character positions change **more often** than they
/// used to — a resting position now arrives as its own event. Ordering is the
/// thing that would break first if `characters` were ever collected straight
/// off the `players` `DashMap` instead of into a `BTreeMap`.
#[test]
fn the_same_world_sites_the_same_furnace_twice() {
    let from = Position::new(RUN27_SITE.0, RUN27_SITE.1);
    let bots = [BotId(1), BotId(3)];
    let first = free_area_near(
        &state_with_parked_bot(&[(3, RUN27_RESTED), (1, PARKED)], &bots),
        &from,
        "stone-furnace",
    );
    let second = free_area_near(
        &state_with_parked_bot(&[(1, PARKED), (3, RUN27_RESTED)], &bots),
        &from,
        "stone-furnace",
    );
    assert_eq!(
        first, second,
        "identical inputs must give byte-identical plans, whatever order the \
         positions were learned in"
    );
    assert!(first.is_some(), "the fixture has open ground near the site");
}
