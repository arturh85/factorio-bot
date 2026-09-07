//! A refusal must not outlive the thing that caused it.
//!
//! `crates/planner/tests/refusal_memory.rs` pins the other half of this: a
//! site the game refused is a site the next plan must not choose, for the rest
//! of the run. That is right for a tree, a cliff or a building. It is wrong
//! for a bot standing on the tile, which walks away a second later — and
//! because `FactorioSurface::placement_refusals` is never expired, the wrong
//! case was permanent.
//!
//! What that cost is the property `goal.built` exists for. A `BuildBlock`
//! re-derives the entities not yet standing so a second pass finishes a
//! partial build; one fenced-off footprint refuses the whole block:
//!
//! ```text
//! pass 1: done=true failed=1 pending=0
//! replan REFUSES: cannot build burner-mining-drill at tile (-16,-14)
//! ```
//!
//! Two sessions went looking for a tree at that tile. There was no tree
//! (`docs/superpowers/notes/2026-09-06-a-refusal-that-names-what-it-found.md`
//! retires that hypothesis and leaves this one open). The ledger was the
//! answer.
//!
//! # The rule these tests pin, and its three cases
//!
//! `PlacementRefusal::names_only_transient_blockers` reads the evidence the
//! game itself attached to the refusal. A refusal is dropped only when it
//! **named** what it found and every name is a transient; a refusal that named
//! something durable is kept, and — the case that must not drift — a refusal
//! that named *nothing* is kept too, because "the mod appended nothing" is an
//! absence of observation and not an observation of absence. That asymmetry is
//! this repo's house rule (`EntityGraph::resource_fingerprint`,
//! `app/src/api/runMatch.ts`): equal means equal, different means unknown.

use factorio_bot_core::factorio::world::{PlacementRefusal, RefusalSource};
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioPlayer, Position};
use factorio_bot_planner::method::util::free_area_near;
use factorio_bot_planner::{
    ActionKind, BotId, Goal, MethodRegistry, PlanState, PlannerError, Site, default_registry,
    expand,
};
use std::sync::Arc;

/// The furnace site `refusal_memory.rs` uses, kept identical so the two files
/// differ in exactly one thing: what the refusal says it found.
const SITE: (f64, f64) = (-16., -58.);

/// A refusal at [`SITE`] carrying `blockers` and `tile` verbatim — the two
/// fields the mod's `describe_footprint` fills and `note_placement_refusal`
/// reads back.
fn refusal_naming(blockers: &[&str], tile: Option<&str>) -> PlacementRefusal {
    PlacementRefusal {
        tick: Some(6198),
        entity: "stone-furnace".to_string(),
        position: Position::new(SITE.0, SITE.1),
        direction: Some(0),
        source: RefusalSource::Dispatch,
        blockers: blockers.iter().map(|b| (*b).to_string()).collect(),
        tile: tile.map(str::to_string),
    }
}

fn state_with(refusals: &[PlacementRefusal]) -> PlanState {
    let world = fixture_world();
    for refusal in refusals {
        world.record_placement_refusal(refusal.clone());
    }
    PlanState::from_world(Arc::new(world), &[BotId(1)])
}

fn site() -> Position {
    Position::new(SITE.0, SITE.1)
}

/// The control, and it is load-bearing twice over: without it every "the site
/// is available" assertion below would also pass against a fixture that never
/// had anything at this tile to begin with, and every "the site is refused"
/// one would pass against a fixture with a tree on it.
#[test]
fn the_site_is_open_ground_with_no_refusal_at_all() {
    assert!(
        state_with(&[]).is_area_free("stone-furnace", &site()),
        "{SITE:?} is clear in the fixture, so a refusal is the only thing that \
         can make it unavailable and the only thing these tests vary"
    );
}

/// **The defect.** A bot was standing there; the bot has walked away; the
/// planner must offer the site again.
#[test]
fn a_refusal_whose_only_blocker_was_a_character_stops_being_believed() {
    let state = state_with(&[refusal_naming(&["character"], Some("grass-1"))]);
    assert!(
        state.is_area_free("stone-furnace", &site()),
        "the game named a character as the only thing in the footprint; a \
         character walks away, so that refusal says nothing about the ground \
         and must not fence the site off for the rest of the run"
    );
}

/// A ghost is not an obstacle — a real placement consumes the one beneath it,
/// which is why `GHOST_ENTITY_TYPES` keeps ghosts out of `blocked_tree` and
/// why both of `occupant_of`'s entity loops skip them by name. A refusal
/// blaming one is blaming something that does not block.
#[test]
fn a_refusal_whose_only_blockers_were_ghosts_stops_being_believed() {
    let state = state_with(&[refusal_naming(
        &["entity-ghost", "tile-ghost"],
        Some("grass-1"),
    )]);
    assert!(
        state.is_area_free("stone-furnace", &site()),
        "ghosts do not collide with a real build, so a refusal naming only \
         ghosts is not a fact about the ground"
    );
}

/// The other direction, and the reason this is not just "forget refusals". A
/// named durable blocker is exactly what the ledger was built to carry.
#[test]
fn a_refusal_naming_a_standing_entity_is_still_believed() {
    let state = state_with(&[refusal_naming(&["stone-furnace"], Some("grass-1"))]);
    assert!(
        !state.is_area_free("stone-furnace", &site()),
        "a furnace is standing in the footprint and nothing in a plan moves \
         it; this refusal must survive"
    );
}

/// A transient *and* something durable is durable. Expiring on "a character
/// was among the blockers" would drop a real verdict every time a bot happened
/// to be standing next to the obstacle that actually caused it.
#[test]
fn a_transient_alongside_a_durable_blocker_does_not_expire_the_refusal() {
    let state = state_with(&[refusal_naming(&["character", "tree-01"], Some("grass-1"))]);
    assert!(
        !state.is_area_free("stone-furnace", &site()),
        "the game found a tree in the footprint as well as a bot; the tree is \
         still there when the bot leaves"
    );
}

/// Empty blockers **with** a tile: the game scanned the box and found no
/// entity, so the ground itself is the answer. Durable.
#[test]
fn a_refusal_that_found_no_entity_but_named_the_tile_is_still_believed() {
    let state = state_with(&[refusal_naming(&[], Some("water"))]);
    assert!(
        !state.is_area_free("stone-furnace", &site()),
        "an empty blocker list beside a named tile is a real observation -- \
         nothing intersected the footprint, so the ground is what refused"
    );
}

/// Empty blockers **and** no tile: the mod appended nothing, so nobody looked.
/// Unknown, not clear — and every refusal recorded before the mod started
/// naming what it found has exactly this shape, `refusal_memory.rs`'s whole
/// fixture included.
#[test]
fn a_refusal_that_named_nothing_at_all_is_still_believed() {
    let state = state_with(&[refusal_naming(&[], None)]);
    assert!(
        !state.is_area_free("stone-furnace", &site()),
        "no blockers and no tile is the signature of a refusal nothing was \
         asked about; treating that as clear ground would expire every entry \
         written before the mod named anything, on no evidence"
    );
}

/// And the search behaves the same way: an expired refusal costs no detour,
/// where a live one does. Pinned because `is_area_free` alone would not catch
/// a filter that ran after `free_area_near`'s candidate walk.
#[test]
fn the_site_search_returns_the_anchor_again_once_a_refusal_expires() {
    let from = site();

    let durable = state_with(&[refusal_naming(&["tree-01"], Some("grass-1"))]);
    let detoured = free_area_near(&durable, &from, "stone-furnace")
        .expect("the map is not fenced off by one refusal");
    assert_ne!(
        detoured, from,
        "a durable refusal must still push the search off the anchor, or the \
         next assertion is about nothing"
    );

    let expired = state_with(&[refusal_naming(&["character"], Some("grass-1"))]);
    assert_eq!(
        free_area_near(&expired, &from, "stone-furnace").expect("open ground at the anchor"),
        from,
        "once the refusal is retired the anchor is available again, which is \
         what makes a partially-built block completable at its own site"
    );
}

// ---------------------------------------------------------------------------
// The acceptance test: a block that lost a placement to a transient blocker
// must be completable on replan.
// ---------------------------------------------------------------------------

/// A blueprint string around a hand-written entity list.
///
/// `Goal::Built` takes blueprint TEXT, not a `Blueprint`, and
/// `method::blueprint`'s own `encode_test_blueprint` is `#[cfg(test)]` inside
/// the crate and so unreachable from an integration test. Same four steps:
/// JSON envelope, zlib, base64, version byte.
fn encode_blueprint(entities: &[(&str, f64, f64)]) -> String {
    use base64::Engine;
    use std::io::Write;
    let entity_json: Vec<String> = entities
        .iter()
        .enumerate()
        .map(|(i, (name, x, y))| {
            format!(
                r#"{{"entity_number":{},"name":"{name}","position":{{"x":{x},"y":{y}}},"direction":0}}"#,
                i + 1
            )
        })
        .collect();
    let envelope = format!(
        r#"{{"blueprint":{{"version":1,"entities":[{}]}}}}"#,
        entity_json.join(",")
    );
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(envelope.as_bytes())
        .expect("in-memory zlib write cannot fail");
    let compressed = encoder.finish().expect("in-memory zlib finish cannot fail");
    format!(
        "0{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    )
}

/// The anchor the block is stamped at, well away from the fixture's origin so
/// nothing but the refusal under test is in the footprint.
const BLOCK_ANCHOR: (f64, f64) = (-40., -58.);

/// A world holding one bot with enough reach to build, plus `refusals`.
fn block_state(refusals: &[PlacementRefusal]) -> PlanState {
    let world = fixture_world();
    world.players.insert(
        1,
        FactorioPlayer {
            player_id: 1,
            position: Position::new(0.0, 0.0),
            build_distance: 1000,
            reach_distance: 1000,
            resource_reach_distance: 4.0,
            ..Default::default()
        },
    );
    for refusal in refusals {
        world.record_placement_refusal(refusal.clone());
    }
    PlanState::from_world(Arc::new(world), &[BotId(1)])
}

/// A refusal of `entity` centred exactly on one of the block's own tiles.
fn refusal_on_block_tile(entity: &str, offset: (f64, f64), blockers: &[&str]) -> PlacementRefusal {
    PlacementRefusal {
        tick: Some(6198),
        entity: entity.to_string(),
        position: Position::new(BLOCK_ANCHOR.0 + offset.0, BLOCK_ANCHOR.1 + offset.1),
        direction: Some(0),
        source: RefusalSource::Dispatch,
        blockers: blockers.iter().map(|b| (*b).to_string()).collect(),
        tile: Some("grass-1".to_string()),
    }
}

fn block_goal() -> Goal {
    Goal::Built {
        blueprint: encode_blueprint(&[("stone-furnace", 0.0, 0.0), ("stone-furnace", 3.0, 0.0)]),
        site: Site::At(Position::new(BLOCK_ANCHOR.0, BLOCK_ANCHOR.1)),
    }
}

/// How many `Place` actions a goal expands into, or the error that refused it.
fn placements(state: &PlanState) -> Result<usize, PlannerError> {
    let registry: MethodRegistry = default_registry();
    let net = expand(&[block_goal()], state, &registry, BotId(1))?;
    Ok(net
        .actions()
        .filter(|a| matches!(a.kind, ActionKind::Place { .. }))
        .count())
}

/// The control: with a clear ledger the block plans both furnaces. Without
/// this, "the transient case plans two placements" could be true of a fixture
/// where the block plans two placements no matter what the ledger says.
#[test]
fn the_block_plans_both_placements_on_a_clear_ledger() {
    assert_eq!(
        placements(&block_state(&[])).expect("the anchor is clear ground in the fixture"),
        2,
        "both furnaces must be planned, or neither half of this pair means \
         anything"
    );
}

/// The other control: a *durable* refusal on one of the block's tiles refuses
/// the block. This is the behaviour being preserved, and it is what proves the
/// acceptance test below is about the expiry rule and not about the refusal
/// being ignored altogether.
#[test]
fn a_durable_refusal_on_a_block_tile_still_refuses_the_block() {
    let state = block_state(&[refusal_on_block_tile(
        "stone-furnace",
        (3.0, 0.0),
        &["tree-01"],
    )]);
    match placements(&state) {
        Err(PlannerError::BlockGroundOccupied { occupant, .. }) => assert!(
            occupant.contains("already refused"),
            "the refusal is what must be named, not some other occupant; got {occupant}"
        ),
        other => panic!(
            "a footprint the game refused because of a tree must still refuse \
             the block; got {other:?}"
        ),
    }
}

/// **The acceptance test.** A block lost one placement because a bot was
/// standing on that tile. The bot has walked away. A replan must attempt the
/// placement again — which is the whole property `goal.built` exists for, and
/// which was defeated for the rest of the run before the ledger was read with
/// the evidence in it.
#[test]
fn a_block_refused_by_a_transient_blocker_is_completable_on_replan() {
    let state = block_state(&[refusal_on_block_tile(
        "stone-furnace",
        (3.0, 0.0),
        &["character"],
    )]);
    let planned = placements(&state).unwrap_or_else(|e| {
        panic!(
            "the only thing on this footprint was a bot, which has since \
             walked away; the replan must not refuse the block. Got {e:?}"
        )
    });
    assert_eq!(
        planned, 2,
        "both furnaces must be planned again, the refused one included -- a \
         replan that quietly plans only the other one has not finished the \
         block either"
    );
}

/// And the replan really does aim at the tile that was refused, rather than
/// siting the second furnace somewhere else and reading as complete.
#[test]
fn the_replan_aims_at_the_tile_the_transient_refusal_named() {
    let refused_tile = Position::new(BLOCK_ANCHOR.0 + 3.0, BLOCK_ANCHOR.1);
    let state = block_state(&[refusal_on_block_tile(
        "stone-furnace",
        (3.0, 0.0),
        &["character"],
    )]);
    let registry: MethodRegistry = default_registry();
    let net = expand(&[block_goal()], &state, &registry, BotId(1))
        .expect("a transiently-refused footprint no longer refuses the block");
    assert!(
        net.actions().any(|a| match &a.kind {
            ActionKind::Place { entity } => entity.position == refused_tile,
            _ => false,
        }),
        "a `Built` block is stamped at a fixed anchor, so completing it means \
         building at {refused_tile:?} itself"
    );
}
