//! What a previous plan left in a furnace or a chest, and whether the next
//! plan can see it.
//!
//! # The gate
//!
//! Convergence puts materials into a machine as a handover: one bot loads a
//! furnace, another unloads it. If the consumer never arrives and the plan is
//! remade, the planner has to be able to see what is sitting in there --
//! because by then the ore those plates came from is *gone from the ground*.
//! A replan that cannot see the buffer does not merely forget the items, it
//! plans to mine ore that no longer exists.
//!
//! Every test here is about one question: does a `Have` goal that a standing
//! buffer already covers still send a bot to the ore field?

mod common;

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{
    Direction, FactorioEntity, InventoryItemWithQuality, InventoryResponse, Position,
};
use factorio_bot_planner::action::ActionKind;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::{Withdraw, registry_for};
use factorio_bot_planner::{ActionNetwork, BotId, Method, PlanState, schedule};
use std::sync::Arc;

/// Clear of the fixture's iron patch (centred (-40, 40), 11 tiles across) and
/// of everything else in it, so nothing about siting is under test here.
const FURNACE: Position = Position { x: -34.0, y: 40.0 };
const SECOND_FURNACE: Position = Position { x: -30.0, y: 40.0 };

fn items(item: &str, count: u32) -> Vec<InventoryItemWithQuality> {
    vec![InventoryItemWithQuality {
        name: item.into(),
        quality: "normal".into(),
        count,
    }]
}

/// A furnace standing at `at`, holding `stock` of `item` in its result slot.
///
/// The contents arrive the way the production path delivers them: as the reply
/// shape of `inventory_contents_at`, handed to
/// `FactorioWorld::observe_inventories`. Building them any other way would
/// exercise a path no run takes.
fn stock_a_furnace(world: &FactorioWorld, at: Position, item: &str, stock: u32) {
    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(&at, Direction::North))
        .expect("the furnace is placed");
    world.observe_inventories(vec![InventoryResponse {
        name: "stone-furnace".into(),
        position: at,
        output_inventory: Box::new(Some(items(item, stock))),
        fuel_inventory: Box::new(None),
    }]);
}

fn world_with_stocked_furnace(item: &str, stock: u32) -> Arc<FactorioWorld> {
    let world = fixture_world();
    stock_a_furnace(&world, FURNACE, item, stock);
    Arc::new(world)
}

fn plan(world: Arc<FactorioWorld>, goals: &[Goal]) -> (ActionNetwork, PlanState) {
    let bots = [BotId(1)];
    let state = PlanState::from_world(world, &bots);
    let net = expand(goals, &state, &registry_for(&bots), BotId(1)).expect("the goals expand");
    (net, state)
}

fn have(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Bot(BotId(1)),
    }
}

/// Every action in the network, described and sorted -- a plan's shape without
/// its ids or its ordering, which is what these tests are about.
fn kinds(net: &ActionNetwork) -> Vec<String> {
    let mut out: Vec<String> = net
        .actions()
        .map(|a| match &a.kind {
            ActionKind::Mine { item, count, .. } => format!("mine {count} {item}"),
            ActionKind::Chop {
                entity,
                item,
                count,
                ..
            } => format!("chop {count} {entity} for {item}"),
            ActionKind::Craft { item, count } => format!("craft {count} {item}"),
            ActionKind::Place { entity } => format!("place {}", entity.name),
            ActionKind::Insert {
                entity,
                item,
                count,
                ..
            } => format!("insert {count} {item} into {entity}"),
            ActionKind::Remove {
                entity,
                item,
                count,
                pos,
                ..
            } => format!("take {count} {item} from {entity} at {pos}"),
            ActionKind::Research { tech } => format!("research {tech}"),
            ActionKind::SetRecipe { entity, recipe, .. } => {
                format!("set recipe {recipe} on {entity}")
            }
            ActionKind::Evacuate { to } => format!("evacuate to {to}"),
            ActionKind::Survey { to } => format!("survey {to}"),
            ActionKind::StampGhosts { anchor, .. } => format!("stamp ghosts at {anchor}"),
        })
        .collect();
    out.sort();
    out
}

/// The gate, stated as a test.
///
/// Ten iron plates are sitting in a furnace the last plan filled. Asked for
/// ten iron plates, the roster must walk over and take them -- not mine ten
/// iron ore, which is what it does when the buffer is invisible.
#[test]
fn plates_standing_in_a_furnace_are_taken_rather_than_smelted_again() {
    let (net, _) = plan(
        world_with_stocked_furnace("iron-plate", 10),
        &[have("iron-plate", 10)],
    );
    assert_eq!(
        kinds(&net),
        vec![format!(
            "take 10 iron-plate from stone-furnace at {FURNACE}"
        )],
        "the whole plan is one walk and one take"
    );
}

/// A buffer that covers part of the bill is emptied, and the rest is made.
///
/// The common case of the gate, not the tidy one: a handover interrupted
/// halfway leaves *some* of the plates in the furnace. Refusing to take four
/// because four is not ten would strand them exactly as thoroughly as not
/// seeing them at all.
#[test]
fn a_partly_full_buffer_is_emptied_and_the_rest_is_made() {
    let (net, _) = plan(
        world_with_stocked_furnace("iron-plate", 4),
        &[have("iron-plate", 10)],
    );
    let plan = kinds(&net);
    assert!(
        plan.contains(&format!(
            "take 4 iron-plate from stone-furnace at {FURNACE}"
        )),
        "the four that exist are taken: {plan:?}"
    );
    assert!(
        plan.contains(&"mine 6 iron-ore".to_string()),
        "and only the missing six are mined: {plan:?}"
    );
    assert!(
        !plan.contains(&"mine 10 iron-ore".to_string()),
        "the four in the furnace must not be mined again: {plan:?}"
    );
}

/// The shortfall arithmetic reaches the remainder, and is not applied twice.
///
/// `Withdraw` states its leftover subgoal with the goal's own `count`, because
/// `run_steps` has already simulated the take into the inventory by then and
/// `shortfall` recomputes the difference itself. Handing it `remaining`
/// instead would subtract twice -- four taken from a bill of ten would mine
/// two. This is the test that says which.
#[test]
fn the_remainder_is_the_difference_and_not_the_difference_twice() {
    let (net, _) = plan(
        world_with_stocked_furnace("iron-plate", 4),
        &[have("iron-plate", 10)],
    );
    let ore: u32 = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { item, count, .. } if item == "iron-ore" => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(ore, 6, "ten wanted, four in hand, six to mine");
}

/// Two goals cannot both spend the same plates.
///
/// The shared-intermediate defect, in the buffer ledger. Five plates sit in
/// one furnace and two goals each want five; if the overlay were not
/// decremented as each `Remove` is emitted, both would plan a take of five and
/// the second would come back empty at the game.
#[test]
fn two_goals_cannot_both_spend_the_same_plates() {
    let (net, _) = plan(
        world_with_stocked_furnace("iron-plate", 5),
        &[Goal::All(vec![
            Goal::Have {
                item: "iron-plate".into(),
                count: 5,
                whose: Holder::Bot(BotId(1)),
            },
            Goal::Have {
                item: "iron-plate".into(),
                count: 10,
                whose: Holder::Bot(BotId(1)),
            },
        ])],
    );
    // Withdrawals from *that furnace*, and only withdrawals — the ones
    // carrying `Effect::BufferLose`, which is what spends the overlay.
    //
    // **Not every remove at that tile**, which is what this counted until
    // `Smelt` learned to adopt a furnace that already stands. The second goal
    // legitimately smelts its own shortfall, and it may now do so *in this
    // furnace*: it loads ore and takes five plates it made itself. Ten plates
    // leave the tile and both takes are honest, because ten plates were there
    // to take — five left over and five smelted. Counting removes by tile
    // could not tell those apart, and the invariant was never about the tile.
    let taken: u32 = net
        .actions()
        .filter(|a| {
            a.eff.iter().any(|e| {
                matches!(e, factorio_bot_planner::action::Effect::BufferLose { pos, .. }
                    if pos.x == FURNACE.x && pos.y == FURNACE.y)
            })
        })
        .filter_map(|a| match &a.kind {
            ActionKind::Remove { item, count, .. } if item == "iron-plate" => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(
        taken, 5,
        "the furnace holds five plates and the plan may take five, however many goals want them"
    );
    // The positive half: the second goal is not silently dropped either. It
    // wants ten, five of which the first goal's withdrawal already put in the
    // bot's hands, so five are made.
    let ore: u32 = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Mine { item, count, .. } if item == "iron-ore" => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(ore, 5, "the five the buffer could not cover are mined");
}

/// Nearest first, and the whole of both buffers when the bill needs both.
#[test]
fn two_buffers_are_drained_nearest_first() {
    let world = fixture_world();
    stock_a_furnace(&world, SECOND_FURNACE, "iron-plate", 3);
    stock_a_furnace(&world, FURNACE, "iron-plate", 3);
    let (net, _) = plan(Arc::new(world), &[have("iron-plate", 6)]);

    let takes: Vec<(f64, u32)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Remove {
                item, count, pos, ..
            } if item == "iron-plate" => Some((pos.x, *count)),
            _ => None,
        })
        .collect();
    assert_eq!(takes.len(), 2, "both furnaces are emptied: {takes:?}");
    assert_eq!(takes.iter().map(|(_, n)| n).sum::<u32>(), 6);
    // Bot 1 starts at the origin in this fixture, so the furnace at x = -30 is
    // the nearer of the two and its take is emitted first. Emission order is
    // what fixes `ActionId` allocation, so this is a determinism assertion as
    // much as a routing one.
    assert_eq!(
        takes[0].0, SECOND_FURNACE.x,
        "the nearer furnace is emptied first: {takes:?}"
    );
}

/// Same world in, same plan out -- twice, and byte for byte.
#[test]
fn the_same_world_plans_the_same_withdrawal_twice() {
    let world = fixture_world();
    stock_a_furnace(&world, SECOND_FURNACE, "iron-plate", 3);
    stock_a_furnace(&world, FURNACE, "iron-plate", 4);
    let world = Arc::new(world);

    let render = |net: &ActionNetwork| {
        net.actions()
            .map(|a| format!("{:?} {} {:?} {:?}", a.id, a.label, a.pre, a.eff))
            .collect::<Vec<_>>()
    };
    let (first, _) = plan(world.clone(), &[have("iron-plate", 10)]);
    let first = render(&first);
    for _ in 0..10 {
        let (again, _) = plan(world.clone(), &[have("iron-plate", 10)]);
        assert_eq!(
            first,
            render(&again),
            "a plan is a function of its world and nothing else"
        );
    }
}

/// Every precondition of a withdrawal holds at the moment its action starts.
///
/// The `BufferHas` precondition is not decoration: it is checked by
/// `schedule`, so a plan that took more out of a buffer than the buffer holds
/// would fail here rather than at the game. Replayed in *time* order, which is
/// what would expose two takes with no ordering edge between them overlapping.
#[test]
fn a_withdrawal_replays_with_every_precondition_holding() {
    let bots = [BotId(1)];
    let world = fixture_world();
    stock_a_furnace(&world, SECOND_FURNACE, "iron-plate", 3);
    stock_a_furnace(&world, FURNACE, "iron-plate", 4);
    let state = PlanState::from_world(Arc::new(world), &bots);
    let net = expand(
        &[have("iron-plate", 10)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("the goal expands");
    let result = schedule(&net, &state, &bots).expect("the plan schedules");
    common::assert_preconditions_hold_over_time(&net, &state, &result);
}

// ---------------------------------------------------------------------------
// Negative controls: what a buffer must *not* be believed for
// ---------------------------------------------------------------------------

/// Possession is not production.
///
/// A `craft-item` trigger fires on the act of producing, so a `Goal::Produced`
/// satisfied by taking finished items out of a furnace would plan a technology
/// that never unlocks. `Goal::Produced`'s own doc already says possession is
/// not production; this is the method that would have broken that promise.
#[test]
fn a_produced_goal_is_never_satisfied_by_a_withdrawal() {
    let world = world_with_stocked_furnace("iron-plate", 10);
    let bots = [BotId(1)];
    let state = PlanState::from_world(world, &bots);
    let produced = Goal::Produced {
        item: "iron-plate".into(),
        count: 10,
        whose: Holder::Bot(BotId(1)),
        unlocks: None,
    };
    assert!(
        !Withdraw.applicable(&produced, &state),
        "a withdrawal produces nothing, and a trigger would never fire"
    );
    // The positive half of the control, so this cannot pass on a `Withdraw`
    // that simply never claims anything.
    assert!(
        Withdraw.applicable(&have("iron-plate", 10), &state),
        "the same items, wanted rather than produced, are a withdrawal"
    );
}

/// A reading whose entity is gone is not believed.
///
/// The map is keyed by tile, and a tile can be cleared. Planning a `Remove`
/// against an entity the game will not find wastes a bot's walk and reports a
/// failure with no cause in it.
#[test]
fn a_reading_with_no_entity_behind_it_is_not_believed() {
    let world = fixture_world();
    // Contents, and no furnace: exactly what a stale reading looks like after
    // somebody mined the thing it was read from.
    world.observe_inventories(vec![InventoryResponse {
        name: "stone-furnace".into(),
        position: FURNACE,
        output_inventory: Box::new(Some(items("iron-plate", 10))),
        fuel_inventory: Box::new(None),
    }]);
    let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
    assert!(
        !state.has_buffers(),
        "a reading with nothing standing behind it is not a buffer"
    );
}

/// A reading taken from something other than what is standing there now is not
/// believed either.
///
/// The narrower half of the same problem: the tile is occupied, so
/// `Condition::EntityAt` would pass, and only the name check catches it.
#[test]
fn a_reading_for_a_different_entity_is_not_believed() {
    let world = fixture_world();
    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(
            &FURNACE,
            Direction::North,
        ))
        .expect("a furnace stands there");
    world.observe_inventories(vec![InventoryResponse {
        name: "iron-chest".into(),
        position: FURNACE,
        output_inventory: Box::new(Some(items("iron-plate", 10))),
        fuel_inventory: Box::new(None),
    }]);
    let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
    assert!(
        !state.has_buffers(),
        "a chest's contents are not a furnace's, whatever tile they share"
    );
}

/// Fuel is not a buffer.
///
/// Coal in a burning furnace is a machine's consumable. Taking it out stalls
/// the furnace the plan may still be waiting on, and what is recoverable is a
/// partly-burnt slot rather than a count anybody planned.
#[test]
fn coal_in_a_furnaces_fuel_slot_is_not_withdrawn() {
    let world = fixture_world();
    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(
            &FURNACE,
            Direction::North,
        ))
        .expect("a furnace stands there");
    world.observe_inventories(vec![InventoryResponse {
        name: "stone-furnace".into(),
        position: FURNACE,
        output_inventory: Box::new(None),
        fuel_inventory: Box::new(Some(items("coal", 20))),
    }]);
    let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
    assert!(
        !state.has_buffers(),
        "only the output reading makes a buffer"
    );
}

/// The inertness proof, stated directly rather than inferred from the pins.
///
/// `Withdraw` is registered ahead of `SharedSmelt`, `Smelt`, `HandCraft` and
/// `Mine`, which would be a large change to every plan if it ever claimed a
/// goal it should not. It cannot, on any world nobody has pulled container
/// contents into -- which is every fixture in this crate and every run before
/// its first refresh. That is why the makespans in `red_science.rs`,
/// `scheduling.rs`, `smelt_roots.rs`, `seeded_roster.rs` and
/// `split_capacity.rs` did not move.
#[test]
fn a_world_nobody_has_read_contents_from_has_no_buffers_at_all() {
    let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
    assert!(!state.has_buffers());
    for item in ["iron-plate", "iron-ore", "coal", "stone-furnace"] {
        assert!(
            !Withdraw.applicable(&have(item, 10), &state),
            "{item}: nothing to withdraw on a world with no readings"
        );
    }
}
