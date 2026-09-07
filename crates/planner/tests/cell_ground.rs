//! A hand-smelt does not pave the ground its own ore cell needs.
//!
//! # The gate
//!
//! `workspace/runs/run-1788497495-79997` — `producing:logistic-science-pack:6`
//! on the real map — halted after six plan epochs with
//!
//! ```text
//! HALTED: stuck -- refused: no room for a iron-ore cell within 12 tiles of
//! the patch: a drill needs to stand on the ore with a furnace two tiles ahead
//! of it standing off it
//! ```
//!
//! having placed **44 stone furnaces at 44 distinct tiles and not one drill**.
//! The refusal names the ore cell; the cause is the furnaces. A cell wants its
//! furnace two tiles ahead of a drill that stands on the ore, so the only
//! ground a cell's furnace can take is the ring of non-ore tiles at the patch
//! edge — and that is exactly where `free_area_near`, searching outward from
//! `nearest_resource_tile`, puts a hand-smelt's furnace, because ore is the one
//! thing a furnace may not stand on. Measured on that run's own world: the iron
//! patch packs 15 cells clean, 7 with its 44 furnaces standing, and 15 -> 7
//! again when a single further plan's 13 hand-smelt furnaces are sited on the
//! clean world. About **0.6 cell sites per hand-smelt furnace**, monotonically,
//! for as many epochs as the run lasts.
//!
//! The fix is one tier, added after the existing search and never before it
//! (`produce::cell_room_to_spare`, `produce::is_cell_furnace_ground`): while
//! the patch can still take `CELL_SITES_RESERVED` cells the smelt sites exactly
//! where it always did, and only once it cannot does it start stepping around
//! cell ground. So the first test here pins the *inertness* — that is what
//! keeps `researched:automation` byte-identical — and the rest pin the fix.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use factorio_bot_planner::action::ActionKind;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::method::produce::{cell_spec, is_cell_furnace_ground, parts, plan_cell};
use factorio_bot_planner::method::util::{
    free_area_near, free_area_near_where, nearest_resource_tile,
};
use factorio_bot_planner::{ActionNetwork, BotId, PlanState};
use std::sync::Arc;

/// Where a bot stands, and therefore which patch every anchor here resolves to.
fn origin() -> Position {
    Position::new(0., 0.)
}

fn bare() -> PlanState {
    PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
}

fn plan_for(state: &PlanState, plates: u32) -> ActionNetwork {
    let bots = [BotId(1)];
    expand(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: plates,
            whose: Holder::Bot(BotId(1)),
            via: None,
        }],
        state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("a hand-smelt for iron plates")
}

fn furnaces_placed(net: &ActionNetwork) -> Vec<Position> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } if entity.name == "stone-furnace" => {
                Some(entity.position.clone())
            }
            _ => None,
        })
        .collect()
}

/// How many iron cells still fit round the patch **at once**.
///
/// Packed on a fork rather than counted site by site: two cells a tile apart do
/// not both fit, so counting them separately would report room that is not
/// there.
fn packable(state: &PlanState) -> usize {
    let spec = cell_spec(state, "iron-plate").expect("the fixture smelts iron");
    let mut trial = state.fork();
    let mut packed = 0;
    while let Ok(cell) = plan_cell(&trial, &origin(), &spec, 1) {
        for entity in parts(&trial, &cell) {
            trial.create_entity(entity);
        }
        packed += 1;
        // The fixture's patch is 10 tiles square; nothing near this is
        // reachable, and the bound keeps a siting bug from hanging the suite.
        assert!(packed <= 60, "the patch cannot hold sixty cells");
    }
    packed
}

/// The fixture with its iron patch worked down to a handful of cell sites.
///
/// Six epochs of the live run got there by paving the patch edge; this gets
/// there in one step by putting the *drill* half of the patch out of reach —
/// a chest on an ore tile is a tile no drill may stand on — and leaving the
/// edge itself wide open. That is the state the fix has to survive: few cell
/// sites left, and the ground a smelt's ring search reaches first is one of
/// them.
///
/// Crowding the *furnace* half instead would prove nothing, and that mistake
/// was made first: block the tiles a cell's furnace wants and the smelt cannot
/// take them either, so the guarded and unguarded searches agree and the test
/// passes with the fix reverted.
fn patch_crowded_to_a_corner() -> PlanState {
    let mut state = bare();
    let anchor =
        nearest_resource_tile(&state, "iron-ore", &origin(), 1).expect("the fixture has iron");
    for x in -45..=-35 {
        for y in 35..=45 {
            let tile = Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
            if calculate_distance(&tile, &anchor) > 4.0 {
                state.create_entity(FactorioEntity {
                    name: "wooden-chest".into(),
                    entity_type: "container".into(),
                    position: tile,
                    ..Default::default()
                });
            }
        }
    }
    state
}

// ---------------------------------------------------------------------------
// Inertness: a patch with room to spare is sited exactly as before
// ---------------------------------------------------------------------------

/// The property that keeps `researched:automation` byte-identical.
///
/// On a bare fixture the iron patch packs twenty cells, far past the reserve,
/// so the smelt takes the first free tile the ring search offers — and that
/// tile *is* cell ground. Withholding it would move red, and red is the pin.
#[test]
fn a_patch_with_room_to_spare_is_sited_exactly_as_before() {
    let state = bare();
    assert!(
        packable(&state) > 6,
        "the bare fixture has cell sites to spare"
    );

    let built = furnaces_placed(&plan_for(&state, 20));
    let [site] = built.as_slice() else {
        panic!("a bare world builds exactly one furnace, not {built:?}");
    };
    assert_eq!(
        *site,
        Position::new(-36., 34.),
        "the first free tile the ring search offers, unchanged by the reserve"
    );
    assert!(
        is_cell_furnace_ground(&state, "iron-ore", site),
        "and it really is cell ground -- the reserve is what declines to \
         withhold it, not the geometry"
    );
}

// ---------------------------------------------------------------------------
// The fix: a crowded patch keeps the sites it has left
// ---------------------------------------------------------------------------

/// The live failure, in one step instead of six epochs.
///
/// With the patch down to the last few sites, the furnace a smelt builds must
/// not be one of them. Before the fix this furnace landed on cell ground and
/// the count fell; the run repeated that until `plan_cell` had nothing left.
#[test]
fn a_crowded_patch_does_not_lose_a_cell_site_to_a_hand_smelt() {
    let before = patch_crowded_to_a_corner();
    let sites_before = packable(&before);
    assert!(
        (1..6).contains(&sites_before),
        "the patch is under the reserve but not dead: {sites_before} sites"
    );

    // What the unguarded search would have taken, and that it really is a cell
    // site -- so the assertion below is about the fix and not about a patch
    // that happened to have none.
    let anchor =
        nearest_resource_tile(&before, "iron-ore", &origin(), 1).expect("the fixture has iron");
    let unguarded =
        free_area_near(&before, &anchor, "stone-furnace").expect("open ground beside the patch");
    assert!(
        is_cell_furnace_ground(&before, "iron-ore", &unguarded),
        "the tile the ring search reaches first is a cell site; without that \
         this test proves nothing"
    );

    let built = furnaces_placed(&plan_for(&before, 20));
    assert!(!built.is_empty(), "the smelt still builds its furnace");
    for site in &built {
        assert!(
            !is_cell_furnace_ground(&before, "iron-ore", site),
            "a smelt on a crowded patch put its furnace at {site}, which is a \
             cell site"
        );
    }
}

/// The smelt still gets a furnace, and the goal still expands.
///
/// A reserve that refused to site anything would turn a spatial problem into a
/// refusal, which is not an improvement: the tier is a *preference*, and the
/// fallback behind it is the unguarded search.
#[test]
fn a_patch_with_no_cell_sites_left_withholds_nothing() {
    // A checkerboard of chests over the patch: no 2x2 drill footprint is free
    // anywhere on it, so no cell fits, while half the ore tiles are still bare
    // and the smelt has something to mine.
    let mut state = bare();
    for x in -45..=-35 {
        for y in 35..=45 {
            if (x + y) % 2i32 == 0 {
                state.create_entity(FactorioEntity {
                    name: "wooden-chest".into(),
                    entity_type: "container".into(),
                    position: Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5),
                    ..Default::default()
                });
            }
        }
    }
    assert_eq!(packable(&state), 0, "no cell fits round this patch at all");

    let built = furnaces_placed(&plan_for(&state, 20));
    assert_eq!(
        built.len(),
        1,
        "the smelt builds its one furnace as it always did"
    );
}

/// `is_cell_furnace_ground` reads the state, not a rule about ore.
///
/// The tile two ahead of a drill site is only cell ground while a cell really
/// fits there. Stand the drill's own ground on and the same tile is ordinary
/// open ground again — which is what stops the reserve withholding ground no
/// cell could use.
#[test]
fn ground_stops_being_cell_ground_when_no_drill_can_face_it() {
    let state = bare();
    let spec = cell_spec(&state, "iron-plate").expect("the fixture smelts iron");
    let cell = plan_cell(&state, &origin(), &spec, 1).expect("a bare patch sites a cell");
    assert!(is_cell_furnace_ground(&state, "iron-ore", &cell.furnace));

    // Everything that could stand on the ore two tiles behind it, blocked.
    let mut blocked = state.fork();
    blocked.create_entity(FactorioEntity::new_stone_furnace(
        &cell.drill,
        Direction::North,
    ));
    assert!(
        !is_cell_furnace_ground(&blocked, "iron-ore", &cell.furnace),
        "with no drill site behind it the tile is ordinary ground"
    );
}

/// The tier is a *search*, not a veto on one tile: the site it settles on is
/// the first non-cell tile in the same ring order the unguarded search uses.
#[test]
fn the_guarded_search_keeps_the_ring_order() {
    let state = patch_crowded_to_a_corner();
    let anchor =
        nearest_resource_tile(&state, "iron-ore", &origin(), 1).expect("the fixture has iron");
    let expected = free_area_near_where(&state, &anchor, "stone-furnace", |candidate| {
        !is_cell_furnace_ground(&state, "iron-ore", candidate)
    })
    .expect("open ground that no cell wants");

    let built = furnaces_placed(&plan_for(&state, 20));
    assert_eq!(
        built.as_slice(),
        [expected],
        "the smelt takes the first tile the guarded ring search offers"
    );
}
