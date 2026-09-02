//! A mining action never asks a tile for more ore than the tile holds.
//!
//! Run `workspace/runs/run-1788334911-41961` closed five rungs and stuck on
//! rung 6 with
//!
//! ```text
//! ERROR: the target iron-ore was gone before mining finished -- something else mined it first
//! ```
//!
//! Nothing else mined it. Every plan in that run was scheduled onto **one**
//! bot (`bots: [2]`, all fifteen `plan_created` events), and its six iron
//! mines went to six *different* tiles — `[-35.5,-56.5]`, `[-34.5,-56.5]`,
//! `[-35.5,-57.5]`, `[-34.5,-57.5]`, `[-34.5,-58.5]`, `[-33.5,-59.5]`. Five of
//! them failed, after delivering 15, 6, 14, 10 and 2 ore against asks of 22,
//! 7, 50, 36 and 26 (120 ticks per ore; the elapsed ticks divide out exactly).
//! The bot mined each tile dry and the mod reported the vanished target with
//! the only wording it has.
//!
//! The cause is the one `docs/superpowers/notes/2026-09-02-tile-reservation.md`
//! predicted in its own concerns section: `DEFAULT_RESOURCE_PER_TILE` is 500,
//! the game says 13, and the planner had never read what the game said. These
//! tests are written against the plan, because a single action asking one tile
//! for fifty ore is not wrong anywhere inside `resource_tiles_for` — it is
//! wrong about the map.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::util::{
    resource_seats, resource_supply_at_least, resource_tiles_for,
};
use factorio_bot_planner::state::DEFAULT_RESOURCE_PER_TILE;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

/// What a live iron tile near a well-used spawn actually holds.
///
/// Not invented: `crates/core/tests/live-2.1.17-entities-resources.json` — a
/// capture off a running 2.1.17 game — reports `iron-ore` at `amount: 13`, and
/// the run above mined 15, 14, 10, 6 and 2 out of five different tiles.
const REPORTED: u32 = 13;

/// The fixture world, with every iron tile re-delivered carrying the amount
/// the game would have reported for it.
///
/// Re-delivery rather than a bespoke world so the *only* difference from
/// `fixture_world()` is the reported amount: same tiles, same patch, same
/// prototypes, same bots. It also goes through the same `EntityGraph::add`
/// path a second chunk writeout takes, which is how a real amount arrives.
fn world_with_iron_holding(bots: &[BotId], amount: u32) -> PlanState {
    let world = fixture_world();
    let ore: Vec<FactorioEntity> = world
        .entity_graph
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .map(|tile| {
            let mut entity = FactorioEntity::new_resource(&tile, Direction::North, "iron-ore");
            entity.amount = Some(amount);
            entity
        })
        .collect();
    assert!(!ore.is_empty(), "the fixture carries an iron field");
    world
        .update_chunk_entities(ore)
        .expect("re-delivering ore with amounts");
    PlanState::from_world(Arc::new(world), bots)
}

/// The untouched fixture: ore whose amount nobody ever reported.
fn world_without_amounts(bots: &[BotId]) -> PlanState {
    PlanState::from_world(Arc::new(fixture_world()), bots)
}

fn gather(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
    }
}

/// Every `Mine` action as `(tile, count)`, in a stable order.
fn mining(net: &ActionNetwork) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { pos, count, .. } => Some((format!("{pos}"), *count)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// One tile of the fixture's iron field, chosen the same way every time.
fn an_iron_tile(state: &PlanState) -> Position {
    let mut tiles: Vec<Position> = state
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    tiles.into_iter().next().expect("the field has tiles")
}

/// The defect, at the smallest scale it exists at: a tile offers what the game
/// says is in it.
#[test]
fn a_tile_offers_the_amount_the_game_reported() {
    let bots = [BotId(1)];
    let reported = world_with_iron_holding(&bots, REPORTED);
    let tile = an_iron_tile(&reported);

    assert_eq!(reported.resource_available(&tile, "iron-ore"), REPORTED);
    assert_eq!(reported.resource_unclaimed(&tile, "iron-ore"), REPORTED);
}

/// The control that keeps the test above from passing for the wrong reason,
/// and the fallback's whole remaining job: a tile nobody reported an amount
/// for still reads as [`DEFAULT_RESOURCE_PER_TILE`].
///
/// Every hand-built fixture in this workspace is in that state, which is why
/// no existing expectation moved.
#[test]
fn a_tile_with_no_reported_amount_falls_back_to_the_modelled_one() {
    let bots = [BotId(1)];
    let silent = world_without_amounts(&bots);
    let tile = an_iron_tile(&silent);

    assert_eq!(
        silent.resource_available(&tile, "iron-ore"),
        DEFAULT_RESOURCE_PER_TILE
    );
}

/// The run's shape. One bot, one goal larger than one tile, and no action that
/// asks a tile for more than it holds.
///
/// Before this, expansion emitted exactly one `mine 50 iron-ore` against the
/// single nearest tile — which is the run's action id 13, verbatim — and the
/// bot mined that tile dry at 14 and failed.
#[test]
fn one_bot_never_asks_one_tile_for_more_ore_than_it_holds() {
    let bots = [BotId(1)];
    let state = world_with_iron_holding(&bots, REPORTED);
    let net = expand(
        &[gather("iron-ore", 50)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("a patch this size can supply fifty ore");

    let mines = mining(&net);
    assert!(
        mines.len() >= 4,
        "fifty ore out of {REPORTED}-ore tiles needs at least four of them, got {mines:?}"
    );
    for (tile, count) in &mines {
        assert!(
            *count <= REPORTED,
            "{tile} was asked for {count} and holds {REPORTED}"
        );
    }
    let total: u32 = mines.iter().map(|(_, count)| *count).sum();
    assert_eq!(total, 50, "the whole goal is still planned");

    let mut tiles: Vec<&String> = mines.iter().map(|(tile, _)| tile).collect();
    tiles.sort();
    let distinct = tiles.len();
    tiles.dedup();
    assert_eq!(distinct, tiles.len(), "one tile, one mining action");
}

/// The falsification, side by side: the same goal on the same map differs only
/// in what the game said about the ore.
///
/// This is the assertion that would have failed before the fix and the one
/// that pins what the fix actually changed — not "more actions", but actions
/// sized to the ground.
#[test]
fn the_same_goal_is_one_action_when_nobody_reports_an_amount() {
    let bots = [BotId(1)];
    let silent = expand(
        &[gather("iron-ore", 50)],
        &world_without_amounts(&bots),
        &registry_for(&bots),
        BotId(1),
    )
    .expect("expands");

    assert_eq!(
        mining(&silent).len(),
        1,
        "a 500-ore tile covers the whole goal, so nothing is split"
    );
}

/// The commitment does not outlive its plan, so the patch stays usable.
///
/// A plan claims the tiles it mines, and that claim is what keeps two of *its*
/// actions off one tile. It must not survive into the next plan: the next plan
/// reads a fresh `PlanState` from the world, and the tiles it can see are the
/// tiles the world still has. If claims leaked, a roster would fence itself
/// out of the patch it is mining after one iteration — which is worse than the
/// bug, because it fails at plan time with `NoApplicableMethod` on a map that
/// visibly has ore.
#[test]
fn a_second_plan_sees_the_whole_patch_again() {
    let bots = [BotId(1)];
    let world = Arc::new({
        let world = fixture_world();
        let ore: Vec<FactorioEntity> = world
            .entity_graph
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .map(|tile| {
                let mut entity = FactorioEntity::new_resource(&tile, Direction::North, "iron-ore");
                entity.amount = Some(REPORTED);
                entity
            })
            .collect();
        world.update_chunk_entities(ore).expect("re-delivering ore");
        world
    });

    let first = PlanState::from_world(world.clone(), &bots);
    let planned = mining(
        &expand(
            &[gather("iron-ore", 50)],
            &first,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("expands"),
    );
    assert!(!planned.is_empty());

    let second = PlanState::from_world(world, &bots);
    for (tile, _) in &planned {
        // Every tile the first plan committed to is on offer again, at its
        // full reported amount: nothing the first plan decided is remembered,
        // and nothing the first plan decided pretends to be a fact about the
        // ground.
        let position = second
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .find(|p| format!("{p}") == *tile)
            .expect("the tile is still in the world");
        assert_eq!(
            second.resource_unclaimed(&position, "iron-ore"),
            REPORTED,
            "{tile} must be offered to the next plan"
        );
    }
    assert_eq!(
        mining(
            &expand(
                &[gather("iron-ore", 50)],
                &second,
                &registry_for(&bots),
                BotId(1),
            )
            .expect("the patch is still usable")
        ),
        planned,
        "the same world plans the same way twice"
    );
}

/// `resource_seats` must never promise a bot a tile `resource_tiles_for` then
/// refuses to hand out, and reading real amounts must not change that.
///
/// The four selectors all read `resource_unclaimed`, which now resolves a
/// tile's capacity through one substitution site. Seats counts *tiles*, so it
/// is unchanged by how much each holds; selection counts *ore*, so it is not.
/// The agreement that matters is that the capacity seats implies is capacity
/// selection can actually deliver.
#[test]
fn seats_and_selection_agree_on_reported_amounts() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let reported = world_with_iron_holding(&bots, REPORTED);
    let silent = world_without_amounts(&bots);
    let from = Position::new(0., 0.);

    let seats = resource_seats(&reported, "iron-ore", 100);
    assert!(seats > 1, "the fixture's iron field seats several miners");
    assert_eq!(
        seats,
        resource_seats(&silent, "iron-ore", 100),
        "how many miners fit is a question about tiles, not about their contents"
    );

    let tiles = resource_tiles_for(&reported, "iron-ore", &from, seats * REPORTED);
    assert!(
        !tiles.is_empty(),
        "selection must be able to fulfil what seats promises"
    );
    assert_eq!(
        tiles.iter().map(|(_, take)| *take).sum::<u32>(),
        seats * REPORTED
    );
    for (tile, take) in &tiles {
        assert!(
            *take <= REPORTED,
            "{tile} was offered {take} and holds {REPORTED}"
        );
    }

    // And the applicability test reads the same ledger: a patch of `n` tiles
    // holding `REPORTED` each supplies exactly that and refuses one more.
    let tile_count = reported
        .resource_patches("iron-ore")
        .into_iter()
        .map(|patch| patch.elements.len() as u32)
        .sum::<u32>();
    assert!(resource_supply_at_least(
        &reported,
        "iron-ore",
        tile_count * REPORTED
    ));
    assert!(
        !resource_supply_at_least(&reported, "iron-ore", tile_count * REPORTED + 1),
        "the patch does not hold more than the game says it holds"
    );
}

/// Reading amounts leaves expansion deterministic.
///
/// `expansion_is_deterministic` compares action *labels*, which now carry the
/// count but not the tile; this compares the tiles and the takes across two
/// expansions of the same goal against the same world.
#[test]
fn capacity_aware_tile_assignment_is_deterministic() {
    let bots = [BotId(1), BotId(2)];
    let a = world_with_iron_holding(&bots, REPORTED);
    let b = world_with_iron_holding(&bots, REPORTED);
    let plan = |state: &PlanState| {
        mining(
            &expand(
                &[gather("iron-ore", 60)],
                state,
                &registry_for(&bots),
                BotId(1),
            )
            .expect("expands"),
        )
    };
    assert_eq!(plan(&a), plan(&b));
}
