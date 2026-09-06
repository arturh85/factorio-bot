//! Prevention, against run `run-1788432181-42528`'s own geometry.
//!
//! # What this reproduces, and what it cannot
//!
//! `crates/core/tests/enclosure_run13.rs` already establishes the detection
//! half of this story against the same run: bots 2 and 3 sat frozen from tick
//! ~48 000 to the end while the power plant was built around them, and the
//! run's own archived entities (furnaces, ore -- `entity_tree`'s admitted
//! types) leave every bot reading as open ground, because the class of
//! obstacle that actually closed on them (trees, water -- `blocked_tree`
//! only) is structurally absent from a keyframe. That file's second test
//! reconstructs the missing ring at the radius the game's own path refusals
//! bound it to (three tiles) and shows the check names bots 2 and 3 and not 1
//! or 4.
//!
//! This file asks the *other* half of the same question, at the *other* end
//! of the timeline: given the same real bot, the same real furnaces and ore,
//! and the same reconstructed ring **missing its last side**, does
//! `crate::enclosure::check` -- the planner's own pre-commit guard -- catch
//! the placement that would have closed it, before it closes? It cannot use
//! `power::plan_plant` end to end (that needs a shoreline this run's fixture
//! was never captured with), so the candidate footprint is the ring's own
//! missing side, standing in for "whatever eight-part shape gets built next
//! to a bot" the same way `crate::enclosure`'s own module doc argues a
//! synthetic footprint may: the check does not care what an obstacle is
//! called, only whether it closes a fill.
//!
//! Bot 3's real position and run's real furnaces come straight out of
//! `run-1788432181-42528-frozen-bots.json`, the same fixture
//! `enclosure_run13.rs` uses -- included from `crates/core/tests/` by a
//! relative path rather than duplicated, since `include_str!` resolves at
//! compile time and crosses no crate boundary.

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::fixture_entity_prototypes;
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioPlayer, Position};
use factorio_bot_planner::enclosure::{EnclosurePrevention, check};
use factorio_bot_planner::ids::BotId;
use factorio_bot_planner::state::PlanState;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Frozen {
    bots: Vec<FrozenBot>,
    entities: Vec<Snapshot>,
}

#[derive(Debug, Deserialize)]
struct FrozenBot {
    id: u32,
    position: Position,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    name: String,
    position: Position,
    direction: u8,
}

fn run_13() -> Frozen {
    serde_json::from_str(include_str!(
        "../../core/tests/run-1788432181-42528-frozen-bots.json"
    ))
    .expect("the run-13 fixture parses")
}

impl Frozen {
    fn at(&self, bot: u32) -> Position {
        self.bots
            .iter()
            .find(|b| b.id == bot)
            .unwrap_or_else(|| panic!("run 13 recorded bot {bot}"))
            .position
            .clone()
    }
}

/// A closed square ring of trees, `radius` tiles out from `around`, in the
/// same spacing `enclosure_run13.rs`'s own `tree_ring` uses: half a tile
/// apart, so nothing a character-sized box could slip through is ever left
/// between two trees.
///
/// Returns the ring split into its four sides (north, east, south, west) by
/// which fixed axis each tile shares with `around`, so a caller can build
/// every side but one.
fn ring_sides(around: &Position, radius: f64) -> [Vec<FactorioEntity>; 4] {
    let mut sides: [Vec<FactorioEntity>; 4] = Default::default();
    let mut offset = -radius;
    while offset <= radius {
        let north = Position::new(around.x() + offset, around.y() - radius);
        let south = Position::new(around.x() + offset, around.y() + radius);
        let west = Position::new(around.x() - radius, around.y() + offset);
        let east = Position::new(around.x() + radius, around.y() + offset);
        sides[0].push(FactorioEntity::new_tree(&north));
        sides[1].push(FactorioEntity::new_tree(&east));
        sides[2].push(FactorioEntity::new_tree(&south));
        sides[3].push(FactorioEntity::new_tree(&west));
        offset += 0.5;
    }
    sides
}

/// A `FactorioWorld` carrying run 13's own furnaces and ore, converted
/// through the same fixture prototypes `enclosure_run13.rs` uses so each gets
/// a real collision box, plus `extra` (a partial reconstructed ring).
fn world_of(entities: &[Snapshot], extra: Vec<FactorioEntity>) -> FactorioWorld {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let world = FactorioWorld::new();
    let proto_vec: Vec<_> = prototypes.iter().map(|v| v.clone()).collect();
    world
        .update_entity_prototypes(proto_vec)
        .expect("prototypes load");

    let mut built: Vec<FactorioEntity> = Vec::with_capacity(entities.len() + extra.len());
    for snapshot in entities {
        let entity = FactorioEntity::from_prototype(
            &snapshot.name,
            snapshot.position.clone(),
            Direction::from_u8(snapshot.direction),
            None,
            None,
            prototypes.clone(),
        )
        .expect("from_prototype does not fail");
        if entity.bounding_box.width() > 0. {
            built.push(entity);
        }
    }
    built.extend(extra);
    world
        .update_chunk_entities(built)
        .expect("the run's entities load");
    world
}

/// `state`'s own players, exactly bot 3 at its real run-13 position -- the
/// one non-participating bystander this scenario is about. Nobody else needs
/// a `characters` entry for `check` to consult: bot 3 alone is close enough
/// to the reconstructed ring to matter.
fn place_bot_3(world: &FactorioWorld, run: &Frozen) {
    world.players.insert(
        3,
        FactorioPlayer {
            player_id: 3,
            position: run.at(3),
            ..Default::default()
        },
    );
}

/// The scenario `crate::enclosure::check` exists for: a bot with open ground
/// on one side of a nearly-closed ring, and a candidate placement that closes
/// the last side.
///
/// Asserts the actual outcome, not merely that nothing panics: either bot 3
/// is named in the returned `Evacuate` list with a real target, or the
/// candidate is refused outright. Both are legitimate answers to "would this
/// trap a bystander" -- see `crate::enclosure::EnclosurePrevention`'s own
/// doc -- and either is strictly better than what run 13 actually did, which
/// was neither: the wall went up and nothing in the plan noticed.
#[test]
fn a_placement_that_closes_the_last_gap_around_bot_3_is_caught() {
    let run = run_13();
    let bot_3 = run.at(3);
    // Centred on the bot itself rather than the midpoint `enclosure_run13.rs`
    // uses between bots 2 and 3: this test is about one bystander and one
    // candidate footprint, not the pair, so the ring is drawn around the one
    // position that has to stay open.
    let sides = ring_sides(&bot_3, 3.);
    let [north, east, south, west] = sides;

    let mut before_ring = Vec::new();
    before_ring.extend(north.clone());
    before_ring.extend(east.clone());
    before_ring.extend(south.clone());
    // `west` deliberately withheld: the one gap a candidate placement is
    // about to close.

    let before_world = world_of(&run.entities, before_ring);
    place_bot_3(&before_world, &run);
    let before_state = PlanState::from_world(Arc::new(before_world), &[BotId(3)]);

    // Sanity check on the premise, not the thing under test: with the gap
    // still open, bot 3 must actually be free. A test that skipped this and
    // went straight to asserting `Evacuate`/`Refuse` could not tell "the
    // check works" from "the ring was closed before `check` ever ran".
    assert_eq!(
        factorio_bot_planner::enclosure::EnclosurePrevention::Clear,
        check(&before_state, &before_state, &bot_3),
        "with the west side of the ring still open, this placement (nothing, \
         checked against itself) must find bot 3 unaffected"
    );

    let mut trial = before_state.fork();
    for tree in &west {
        trial.create_entity(tree.clone());
    }

    match check(&before_state, &trial, &bot_3) {
        EnclosurePrevention::Evacuate(evacuations) => {
            assert_eq!(
                evacuations.len(),
                1,
                "only bot 3 stands near this footprint"
            );
            let evacuation = &evacuations[0];
            assert_eq!(evacuation.bot, BotId(3));
            assert!(
                evacuation.pocket_tiles > 0. && evacuation.pocket_tiles < 36.,
                "bot 3's pocket is inside a 6x6 ring, got {}",
                evacuation.pocket_tiles
            );
            // The escape target is somewhere the west side does not stand --
            // otherwise the "evacuation" would walk the bot into the wall
            // that is trapping it.
            let clear_of_wall = west
                .iter()
                .all(|tree| tree.position.manhattan_distance(&evacuation.to) > 0.5);
            assert!(
                clear_of_wall,
                "the escape target {:?} must not sit on the wall that closed the gap",
                evacuation.to
            );
        }
        EnclosurePrevention::Refuse => {
            // Also acceptable: refusing to site the footprint at all is the
            // other half of the brief's own contract. Nothing further to
            // assert beyond having reached this arm rather than `Clear`.
        }
        EnclosurePrevention::Clear => panic!(
            "closing the last side of a ring around a real, stationary bot must not \
             read as clear -- this is exactly run 13's own failure, reproduced"
        ),
    }
}

/// The same footprint, asked about a position far from any bot, finds
/// nothing -- the false-positive guard `enclosure_run13.rs` pins for
/// detection, restated for prevention. A check that fired on every placement
/// regardless of who was nearby would be worse than the bug it exists to
/// catch.
#[test]
fn the_same_footprint_far_from_any_bot_is_clear() {
    let run = run_13();
    let bot_3 = run.at(3);
    let sides = ring_sides(&bot_3, 3.);
    let [north, east, south, _west] = sides;

    let mut before_ring = Vec::new();
    before_ring.extend(north);
    before_ring.extend(east);
    before_ring.extend(south);

    let before_world = world_of(&run.entities, before_ring);
    place_bot_3(&before_world, &run);
    let before_state = PlanState::from_world(Arc::new(before_world), &[BotId(3)]);

    let far_away = Position::new(bot_3.x() + 500., bot_3.y() + 500.);
    let mut trial = before_state.fork();
    trial.create_entity(FactorioEntity::new_tree(&far_away));

    assert_eq!(
        EnclosurePrevention::Clear,
        check(&before_state, &trial, &far_away),
        "a tree five hundred tiles from bot 3 cannot be what traps it"
    );
}

// ---------------------------------------------------------------------------
// The window, against the shape it is guarding
// ---------------------------------------------------------------------------

/// The pump of the plant `method::power` sites on `fixture_world`'s lake, and
/// the tile that plant's **second** steam engine stands on when it is sized
/// against 1,000 kW. Both read off `plan_plant_for` itself -- see
/// `enclosure::plant_reach::a_second_engine_takes_a_plant_past_the_pad_that_used_to_bound_the_window`,
/// which pins the same plant's reach at 14.008 tiles from this pump against
/// the 12 the retired `FOOTPRINT_PAD` admitted.
const PLANT_PUMP: (f64, f64) = (39.5, 37.5);
const SECOND_ENGINE: (f64, f64) = (36.5, 26.5);

/// A bystander 41.1 tiles from that pump.
///
/// Chosen against three inequalities, none of them read off the code:
///
/// * **further than 36 tiles from the pump**, which is where
///   `SEARCH_RADIUS + FOOTPRINT_PAD` stopped looking, so the old selection
///   never examined it;
/// * **near enough that the second engine lands in its own search window** --
///   the engine's grown box covers the tile centres `(35.5, 24.5)`,
///   `(36.5, 24.5)` and `(37.5, 24.5)`, which are on the bottom edge of the
///   48x48 window anchored at `(-4, -23)` for a bot standing here;
/// * and the fixed check examines it, because it asks whether the parts
///   `trial` holds touch *this bot's* window rather than whether the bot is
///   inside a radius padded by a guess at how big a plant is.
const BYSTANDER: (f64, f64) = (20.5, 1.0);

/// The three tile centres the second engine's collision box covers on that
/// window's own boundary -- the gap the pen below is built around.
const GAP: [(f64, f64); 3] = [(35.5, 24.5), (36.5, 24.5), (37.5, 24.5)];

/// A tree on every boundary tile centre of the 48x48 window whose lowest cell
/// centre is `(low_x, low_y)`, except the centres in `gap`.
///
/// The window corner and the gap are passed as literals by each caller rather
/// than computed from `crate::enclosure`'s own constants: a pen derived from
/// the window function would agree with whatever that function did, which is
/// the failure mode `docs/superpowers/notes/2026-09-06-fixtures-agree-with-/// their-code.md` is about.
fn pen(low_x: f64, low_y: f64, gap: &[(f64, f64)]) -> Vec<FactorioEntity> {
    let (high_x, high_y) = (low_x + 47., low_y + 47.);
    let mut walls = Vec::new();
    let mut x = low_x;
    while x <= high_x {
        let mut y = low_y;
        while y <= high_y {
            let on_edge = x == low_x || x == high_x || y == low_y || y == high_y;
            if on_edge && !gap.contains(&(x, y)) {
                walls.push(FactorioEntity::new_tree(&Position::new(x, y)));
            }
            y += 1.;
        }
        x += 1.;
    }
    // Four far trees so the window is inside the region `blocked_tree`
    // models: an escape search that runs off the model answers `Unknown`,
    // which this test must not be able to mistake for a catch.
    for corner in [(0., -60.), (80., -60.), (0., 60.), (80., 60.)] {
        walls.push(FactorioEntity::new_tree(&Position::new(corner.0, corner.1)));
    }
    walls
}

/// A world holding only `entities`, with the fixture prototypes so the steam
/// engine below gets its real collision box.
fn bare_world(entities: Vec<FactorioEntity>) -> FactorioWorld {
    let prototypes = Arc::new(fixture_entity_prototypes());
    let world = FactorioWorld::new();
    world
        .update_entity_prototypes(prototypes.iter().map(|v| v.clone()).collect())
        .expect("prototypes load");
    world
        .update_chunk_entities(entities)
        .expect("the pen loads");
    world
}

/// A bot that only the **second** steam engine of a two-engine plant would
/// wall in is examined.
///
/// # The defect this pins
///
/// `check` used to select bots with `SEARCH_RADIUS + FOOTPRINT_PAD` = 36
/// tiles from the placement's origin, and `FOOTPRINT_PAD` was sized by hand
/// against a plant with **one** steam engine. `method::power::plan_plant` now
/// sizes the engine row from demand up to `MAX_ENGINES_PER_BOILER`, and the
/// second engine stands five tiles further out: 14.008 tiles from the pump
/// against the 12 the pad allowed. A bot in the 36-to-50-tile band was never
/// asked, so **the placement passed prevention and the bot was walled in with
/// nothing reported** -- the exact failure `enclosure::check` exists to
/// prevent, and silent.
///
/// # Why the pen is this big, which is the defect's own shape
///
/// The bystander has to be more than 36 tiles from the *pump* and within a
/// window's reach of the *engine*, and the engine is only 14 tiles from the
/// pump -- so the bot is necessarily more than twenty tiles from the wall
/// that closes on it, and the pocket is necessarily large. That is not the
/// test being contrived; it is what the missed band physically contains. A
/// bot in a four-tile pocket beside a plant was always inside the old window.
///
/// # What it asserts
///
/// Three calls, so that a green result cannot come from the geometry being
/// wrong in a way that happens to look like a catch:
///
/// 1. checked against **itself**, nothing changes and nobody is named;
/// 2. checked with the **bot** as the origin -- which every window, old or
///    new, examines -- the enclosure is real and is refused;
/// 3. checked with the **pump** as the origin, which is what
///    `plan_plant_for` and `complete_plant` actually pass, the same answer
///    must come back. Before the fix this was `Clear`.
#[test]
fn a_bot_only_the_second_engine_would_wall_in_is_examined() {
    let bystander = Position::new(BYSTANDER.0, BYSTANDER.1);
    let pump = Position::new(PLANT_PUMP.0, PLANT_PUMP.1);

    let world = bare_world(pen(-3.5, -22.5, &GAP));
    world.players.insert(
        3,
        FactorioPlayer {
            player_id: 3,
            position: bystander.clone(),
            ..Default::default()
        },
    );
    let before = PlanState::from_world(Arc::new(world), &[BotId(3)]);

    assert_eq!(
        EnclosurePrevention::Clear,
        check(&before, &before, &pump),
        "a placement of nothing traps nobody: the pen still has its gap"
    );

    let engine = FactorioEntity::from_prototype(
        "steam-engine",
        Position::new(SECOND_ENGINE.0, SECOND_ENGINE.1),
        Some(Direction::North),
        None,
        None,
        Arc::new(fixture_entity_prototypes()),
    )
    .expect("the fixture describes a steam-engine");
    let mut trial = before.fork();
    trial.create_entity(engine);

    assert_eq!(
        EnclosurePrevention::Refuse,
        check(&before, &trial, &bystander),
        "asked about the bot's own tile, the second engine plainly seals it in: \
         the pen's only gap is the three tiles that engine covers, and with them \
         blocked nothing in the window still reaches open ground"
    );

    assert_eq!(
        EnclosurePrevention::Refuse,
        check(&before, &trial, &pump),
        "and the answer must not depend on the origin being the bot: the pump \
         is what `plan_plant_for` passes, and this bot is 41.1 tiles from it -- \
         outside the 36 the hand-sized `FOOTPRINT_PAD` admitted, which is how \
         this came back `Clear` and built the wall"
    );
}

/// A bystander on the **diagonal** from the plant, 42.8 tiles away.
///
/// The second half of the same selection defect, and older than the engine
/// row. The searched window is a **square** 48 tiles across and
/// `PlanState::characters_near` filters a **circle**, so a bot on the
/// diagonal is nearer the footprint on each axis than its straight-line
/// distance says: up to `sqrt(2)` times nearer. The old form added a pad to
/// `SEARCH_RADIUS` and compared that to a straight-line distance directly,
/// with no such factor anywhere in it.
///
/// This bot stands 42.8 tiles from the pump -- further than
/// `WINDOW_REACH + 14.008` = 39.5, which is what a radius that measured the
/// footprint correctly and still forgot the square would allow. The second
/// engine's collision box covers four tile centres on this window's own
/// boundary, and they are the pen's only gap. It is here so that a future
/// change back to any radius-from-the-origin selection has to face the
/// diagonal as well as the size.
const DIAGONAL_BYSTANDER: (f64, f64) = (13.5, 3.5);

/// Those four boundary centres, for the window whose lowest cell centre is
/// `(-10.5, -20.5)`: the engine's grown box spans x `35.05..37.95` and y
/// `23.95..29.05`, and this window's highest centres are x 36.5 and y 26.5.
const DIAGONAL_GAP: [(f64, f64); 4] = [(36.5, 24.5), (36.5, 25.5), (36.5, 26.5), (35.5, 26.5)];

#[test]
fn a_bot_on_the_diagonal_is_examined_too() {
    let bystander = Position::new(DIAGONAL_BYSTANDER.0, DIAGONAL_BYSTANDER.1);
    let pump = Position::new(PLANT_PUMP.0, PLANT_PUMP.1);

    let world = bare_world(pen(-10.5, -20.5, &DIAGONAL_GAP));
    world.players.insert(
        3,
        FactorioPlayer {
            player_id: 3,
            position: bystander.clone(),
            ..Default::default()
        },
    );
    let before = PlanState::from_world(Arc::new(world), &[BotId(3)]);

    assert_eq!(
        EnclosurePrevention::Clear,
        check(&before, &before, &pump),
        "the pen still has its gap"
    );

    let engine = FactorioEntity::from_prototype(
        "steam-engine",
        Position::new(SECOND_ENGINE.0, SECOND_ENGINE.1),
        Some(Direction::North),
        None,
        None,
        Arc::new(fixture_entity_prototypes()),
    )
    .expect("the fixture describes a steam-engine");
    let mut trial = before.fork();
    trial.create_entity(engine);

    assert_eq!(
        EnclosurePrevention::Refuse,
        check(&before, &trial, &bystander),
        "asked about the bot's own tile, the engine seals the pen's only gap"
    );

    assert_eq!(
        EnclosurePrevention::Refuse,
        check(&before, &trial, &pump),
        "and asked about the pump 42.8 tiles away on the diagonal, the same: a \
         bound that measured a circle against a square window would stop at \
         39.5 and never look"
    );
}
