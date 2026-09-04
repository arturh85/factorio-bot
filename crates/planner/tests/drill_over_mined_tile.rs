//! A cell must not be built on the ore the same plan sends a bot to hand-mine.
//!
//! # `run-1788455754-92581`
//!
//! The first live run carrying `11fabe43` (`PlaceDrill`). One 244-step plan,
//! `place: 2, mine: 44, fuel: 1` dispatched, and then
//!
//! ```text
//! tick 13294  bot 1  mine 4 iron-ore  target (-33.5, -22.5)
//! game rejected: "could not start mining for 301 ticks:
//!                 expected iron-ore at (-33.5/-22.5), found burner-mining-drill"
//! ```
//!
//! Action 17 of that same plan was `place burner-mining-drill at [-34, -23]`,
//! dispatched at tick 5833 and successful. A 2x2 drill centred there covers
//! the tiles whose centres are `(-34.5, -23.5)`, `(-33.5, -23.5)`,
//! `(-34.5, -22.5)` and `(-33.5, -22.5)` — the last of which is the tile
//! action 31 was sent to. Nothing was wrong with either decision on its own;
//! they were simply made against each other.
//!
//! # Why nothing caught it before
//!
//! Two ledgers, and neither could see the other's commitments.
//!
//! * `PlanState::resource_unclaimed` — what every tile selector in
//!   `method::util` reads — excluded tiles that were claimed, crowded, stood
//!   on by a character, or covered by something in the base world's
//!   *blocking* tree. Not tiles the plan itself had built on: the entity
//!   overlay was simply not one of its sources.
//! * `produce::fit` — what sites a cell — required ore under the drill and
//!   clear ground under the furnace. Ore is the one obstacle
//!   `is_area_clear_of` waives for a drill, and a mining claim was never an
//!   obstacle to a placement at all.
//!
//! Before `11fabe43` nothing ever placed anything on ore during a gathering
//! goal, so the two ledgers had never had to agree.
//!
//! Both directions are fixed and both are tested here, because expansion order
//! decides which commitment is made first and nothing constrains that order.
//! In this run the drill went first: `PlaceDrill::expand` writes the cell into
//! the state and *then* returns the bill subgoals that pay for it, so the ore
//! that bought the drill was selected from under the drill.

use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::{fixture_world, spawn_ore};
use factorio_bot_core::types::{Direction, EntityName, FactorioEntity, Position, Rect};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::produce::{cell_spec, plan_cell};
use factorio_bot_planner::method::util::resource_tiles_for;
use factorio_bot_planner::state::ClaimRunner;
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

/// The run's own iron field, near enough of it to reproduce the siting.
///
/// `spawn_ore` lays tiles on integer positions and `EntityGraph` keys them by
/// the tile they floor into, so a rect from `(-40, -28)` to `(-30, -18)`
/// carries the tile `(-34, -23)` the run's drill stood on and the tile
/// `(-33, -23)` whose centre `(-33.5, -22.5)` the run's mine died on.
const RUN_PATCH: ((f64, f64), (f64, f64)) = ((-40., -28.), (-30., -18.));

/// The world the run planned against, near enough: the shared fixture's
/// prototypes and recipes, plus the run's own iron field. The fixture's own
/// iron sits at `(-40, 40)`, twenty tiles further from the origin than this,
/// so a bot at the origin sites its cell here.
fn run_world(bots: &[BotId]) -> PlanState {
    let world = fixture_world();
    let mut ore = Vec::new();
    spawn_ore(
        &mut ore,
        Rect::new(
            &Position::new(RUN_PATCH.0.0, RUN_PATCH.0.1),
            &Position::new(RUN_PATCH.1.0, RUN_PATCH.1.1),
        ),
        &EntityName::IronOre.to_string(),
    );
    world.update_chunk_entities(ore).expect("deliver the ore");
    PlanState::from_world(Arc::new(world), bots)
}

/// Every tile a `Place` action's entity would cover, over the whole plan.
fn built_tiles(state: &PlanState, net: &ActionNetwork) -> Vec<(String, Position, Rect)> {
    net.actions()
        .filter_map(|action| match &action.kind {
            ActionKind::Place { entity } => {
                let facing = Direction::from_u8(entity.direction)?;
                let area = state.collision_area_facing(&entity.name, &entity.position, facing)?;
                Some((entity.name.clone(), entity.position.clone(), area))
            }
            _ => None,
        })
        .collect()
}

fn mined_tiles(net: &ActionNetwork) -> Vec<(Position, String)> {
    net.actions()
        .filter_map(|action| match &action.kind {
            ActionKind::Mine { pos, item, .. } => Some((pos.clone(), item.clone())),
            _ => None,
        })
        .collect()
}

/// Is `tile`'s centre inside `area`? A tile centre is half a tile from every
/// edge of its own square, so an overlap of the two is an interior point.
fn inside(area: &Rect, tile: &Position) -> bool {
    tile.x() > area.left_top.x()
        && tile.x() < area.right_bottom.x()
        && tile.y() > area.left_top.y()
        && tile.y() < area.right_bottom.y()
}

/// The run's plan, in miniature: one bot, fifty iron plates.
///
/// Fifty is the count the `steam-power` trigger asks for and the count
/// `PlaceDrill`'s own gate crosses over at, so this goal builds a cell — and
/// the cell's bill (a drill, a furnace and their coal, from an empty
/// inventory) is what sends the bot to hand-mine iron ore in the first place.
/// Both halves of the collision come out of the one goal, exactly as they did
/// in the run.
fn run_plan(state: &PlanState) -> ActionNetwork {
    let bots = [BotId(1)];
    expand(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: 50,
            whose: Holder::Bot(BotId(1)),
        }],
        state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("fifty plates are worth a cell in this world")
}

#[test]
fn a_plan_never_mines_the_ore_it_stands_a_machine_on() {
    let state = run_world(&[BotId(1)]);
    let net = run_plan(&state);
    let built = built_tiles(&state, &net);
    let mined = mined_tiles(&net);

    // Non-vacuity, both ways: without a drill there is nothing to collide
    // with, and without an iron mine there is nothing to collide.
    assert!(
        built
            .iter()
            .any(|(name, _, _)| name == &EntityName::BurnerMiningDrill.to_string()),
        "this goal builds a cell, or the test is asserting nothing: {built:?}"
    );
    assert!(
        mined.iter().any(|(_, item)| item == "iron-ore"),
        "this goal hand-mines the iron its cell's bill costs: {mined:?}"
    );

    for (name, position, area) in &built {
        for (tile, item) in &mined {
            assert!(
                !inside(area, tile),
                "mine {item} at {tile} is under the {name} this same plan places \
                 at {position} -- the bot would arrive to \
                 'expected {item}, found {name}'"
            );
        }
    }
}

/// The other direction: a tile already promised to a miner is not a drill site.
///
/// `plan_cell` is asked twice against the same world — once clean, and once
/// with a tile of the clean answer's own footprint claimed for mining. The
/// second answer must move off it.
///
/// **The claim is made under a runner, and the runner is left set.** That is
/// not decoration: `PlaceDrill` converges, so its bill and its siting share one
/// chain and one runner, and `is_resource_crowded_for` waives separation
/// between two claims of the *same* runner — one bot runs one action at a time.
/// So inside the chain that builds the cell, `nearest_resource_tile` will
/// happily anchor immediately beside a tile that chain has already claimed, and
/// the ring search sites a 1.8-tile drill straight over it. Claim it with no
/// runner instead and crowding alone pushes the anchor 3.69 tiles clear, which
/// hides the defect behind a rule that is not the one under test.
///
/// Asked of the siting function directly rather than through `expand`, because
/// which of the two directions a given plan takes is decided by expansion
/// order — a test that went through `expand` would be pinning that order.
#[test]
fn a_tile_a_mine_has_claimed_is_not_a_drill_site() {
    let state = run_world(&[BotId(1)]);
    let spec = cell_spec(&state, "iron-plate").expect("iron plate smelts from one ore");
    let from = Position::new(0., 0.);

    let clean = plan_cell(&state, &from, &spec, 1).expect("the run's patch has room for a cell");
    let clean_area = state
        .collision_area_facing("burner-mining-drill", &clean.drill, clean.facing)
        .expect("the fixture has a drill prototype");

    // Every tile the clean siting covers, one run each: which of the four a
    // mining action happens to have taken is not something this rule may
    // depend on.
    let (base_x, base_y) = (clean.drill.x().floor(), clean.drill.y().floor());
    let covered: Vec<Position> = [(-1, -1), (0, -1), (-1, 0), (0, 0)]
        .into_iter()
        .map(|(dx, dy)| Position::new(base_x + f64::from(dx) + 0.5, base_y + f64::from(dy) + 0.5))
        .filter(|tile| inside(&clean_area, tile))
        .collect();
    assert_eq!(
        covered.len(),
        4,
        "a 1.8-tile drill centred on a tile corner covers exactly four tiles: {covered:?}"
    );

    for claimed in covered {
        let mut with_claim = state.fork();
        with_claim.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        with_claim.claim_resource(&claimed);

        let resited = plan_cell(&with_claim, &from, &spec, 1).expect("there is other ore to build on");
        let resited_area = with_claim
            .collision_area_facing("burner-mining-drill", &resited.drill, resited.facing)
            .expect("the fixture has a drill prototype");
        assert!(
            !inside(&resited_area, &claimed),
            "a cell sited at {} still covers {claimed}, which a mining action of \
             the same chain had already claimed",
            resited.drill
        );
    }
}

/// The same failure one plan later — and the control that says it cannot
/// happen, which is why the fix above is only about the plan's own overlay.
///
/// A claim lives in a `PlanState` and every plan starts with an empty one, so
/// the *next* milestone meets a built cell as a fact of the world rather than
/// as a commitment of its own. That case was already covered and it is worth
/// knowing *by which* of the graph's trees, because the obvious answer is the
/// wrong one: `EntityGraph::add` files a drill in `entity_tree`, which
/// `resource_tile_blocked` does not read at all — but it also files every
/// entity with a non-zero collision box except resources and rails into
/// `blocked_tree`, which it does read. So a drill that really got built has
/// always been out of selection, and only the drill this plan has merely
/// *decided* on was missing. This test is what says so; without it the second
/// source would have been added to `resource_tile_blocked` on the strength of
/// an argument that sounds right and is not.
#[test]
fn a_drill_the_last_plan_built_was_already_out_of_selection() {
    let from = Position::new(0., 0.);
    let clean = run_world(&[BotId(1)]);
    let spec = cell_spec(&clean, "iron-plate").expect("iron plate smelts from one ore");
    let cell = plan_cell(&clean, &from, &spec, 1).expect("room for a cell");
    let area = clean
        .collision_area_facing("burner-mining-drill", &cell.drill, cell.facing)
        .expect("the fixture has a drill prototype");

    // The control: with nothing standing there, selection does reach under
    // the footprint, so the assertion below is about the drill and not about
    // the walk order.
    let reachable_clean = resource_tiles_for(&clean, "iron-ore", &cell.drill, 4)
        .into_iter()
        .filter(|(tile, _)| inside(&area, tile))
        .count();
    assert!(
        reachable_clean > 0,
        "the tiles under {} are ordinary iron before anything stands on them",
        cell.drill
    );

    let world = fixture_world();
    let mut entities = Vec::new();
    spawn_ore(
        &mut entities,
        Rect::new(
            &Position::new(RUN_PATCH.0.0, RUN_PATCH.0.1),
            &Position::new(RUN_PATCH.1.0, RUN_PATCH.1.1),
        ),
        &EntityName::IronOre.to_string(),
    );
    entities.push(FactorioEntity::new_burner_mining_drill(
        &cell.drill,
        cell.facing,
    ));
    world
        .update_chunk_entities(entities)
        .expect("deliver the ore and the drill that was built on it");
    let built = PlanState::from_world(Arc::new(world), &[BotId(1)]);

    let under: Vec<Position> = resource_tiles_for(&built, "iron-ore", &cell.drill, 4)
        .into_iter()
        .map(|(tile, _)| tile)
        .filter(|tile| inside(&area, tile))
        .collect();
    assert!(
        under.is_empty(),
        "selection offered {under:?}, which is under the burner-mining-drill \
         standing at {}",
        cell.drill
    );
}
