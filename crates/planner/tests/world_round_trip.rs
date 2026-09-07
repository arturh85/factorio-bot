//! A plan made from a dumped world is the plan made from the world it was
//! dumped from.
//!
//! This is the property the whole offline-planning workstream rests on. A
//! planner change costs a 20-minute live run to evaluate; dumping a
//! `FactorioSurface` to a file and planning against the file makes that
//! evaluation sub-second. But an offline result is only worth having if it is
//! *the same result* — if the two can differ, nobody can act on the cheap one,
//! and the expensive one has to be run anyway.
//!
//! So this asserts equality of the whole plan, not of a headline number.
//! Makespan alone is a scalar over a hundred actions and would happily agree
//! while the two plans mined different ore, bound different chains to
//! different bots and sent different steps to different hands. What is
//! compared here is every `Action` (id, kind, preconditions, effects,
//! duration, label), every action's chain and every chain's owner, and every
//! `ScheduledStep` with its bot and its start and end tick.
//!
//! # Why the world is not the bare fixture
//!
//! `PlanState::from_world` reads four ledgers that the derived state does not
//! carry: observed inventories, placement refusals, walk refusals and
//! enclosures. None of them was serialized until this change, so a world
//! dumped at t=0 round-tripped perfectly (all four are empty on a fresh map)
//! and a world dumped **mid-run** silently lost them — which is exactly the
//! dump worth taking, since replanning from a milestone is the point.
//!
//! `the_ledgers_are_load_bearing_for_this_fixture` is what keeps the equality
//! above from being vacuous: it strips the four fields back out of the JSON,
//! which is precisely what the old serialization wrote, and shows the plan
//! moves. The buffer is what carries that demonstration — a furnace holding
//! plates lets `Withdraw` claim a goal that would otherwise go to the ore
//! field. The refusal ledgers are sited clear of anything this plan wants, so
//! they are exercised without steering it; they get their plan-level coverage
//! in `refusal_memory.rs` and `unreachable_memory.rs`.

use factorio_bot_core::factorio::world::{
    Bench, Enclosure, FactorioSurface, HOP_DISTANCE, PlacementRefusal, WalkRefusal,
};
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{
    Direction, FactorioEntity, InventoryItemWithQuality, InventoryResponse, Position,
};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{Action, ActionId, BotId, ChainId, PlanState, Schedule, schedule};
use std::sync::Arc;

const BOTS: [BotId; 4] = [BotId(1), BotId(2), BotId(3), BotId(4)];

/// Clear of the fixture's iron patch (centred (-40, 40), 11 tiles across), the
/// same site `tests/buffers.rs` uses and for the same reason.
const FURNACE: Position = Position { x: -34.0, y: 40.0 };

/// Far from every patch, every tree and every bot in the fixture, so the
/// refusal ledgers ride along without steering the plan.
const NOWHERE: Position = Position { x: 200.0, y: 200.0 };

fn goal() -> Goal {
    Goal::Have {
        item: "automation-science-pack".into(),
        count: 10,
        whose: Holder::Anyone,
    }
}

/// Everything a plan is, in a form two of them can be compared by.
#[derive(Debug, PartialEq)]
struct Plan {
    actions: Vec<Action>,
    /// `(action, its chain, that chain's owner)` for every action, in id
    /// order. Chain membership and ownership live on the network rather than
    /// on an `Action`, so comparing `actions` alone would miss a plan that
    /// welded the same work to a different bot.
    chains: Vec<(ActionId, Option<ChainId>, Option<BotId>)>,
    schedule: Schedule,
}

fn plan_of(world: Arc<FactorioSurface>) -> Plan {
    let state = PlanState::from_world(world, &BOTS);
    let net = expand(&[goal()], &state, &registry_for(&BOTS), BotId(1)).expect("the goal expands");
    let schedule = schedule(&net, &state, &BOTS).expect("the network schedules");
    let mut actions: Vec<Action> = net.actions().cloned().collect();
    actions.sort_by_key(|action| action.id);
    let chains = actions
        .iter()
        .map(|action| {
            let chain = net.chain_of(action.id);
            (action.id, chain, chain.and_then(|c| net.owner_of(c)))
        })
        .collect();
    Plan {
        actions,
        chains,
        schedule,
    }
}

/// A world that has been *running*: something has been looked inside, a build
/// has been refused, a walk has been refused and a bot has been found boxed
/// in. All four are ledgers `PlanState::from_world` reads and nothing else
/// reconstructs.
fn world_mid_run() -> FactorioSurface {
    let world = fixture_world();

    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(
            &FURNACE,
            Direction::North,
        ))
        .expect("the furnace is placed");
    world.observe_inventories(vec![InventoryResponse {
        name: "stone-furnace".into(),
        position: FURNACE,
        output_inventory: Box::new(Some(vec![InventoryItemWithQuality {
            name: "iron-plate".into(),
            quality: "normal".into(),
            count: 40,
        }])),
        fuel_inventory: Box::new(None),
    }]);

    world.record_placement_refusal(PlacementRefusal::at_dispatch(
        Some(4242),
        "stone-furnace",
        NOWHERE,
        4,
        vec!["tree-01".to_string()],
        Some("grass-1".to_string()),
    ));
    world.record_walk_refusal(WalkRefusal {
        tick: None,
        player: 3,
        from: Position::new(0., 0.),
        to: NOWHERE,
    });
    world.record_enclosure(Enclosure {
        tick: None,
        player: 1,
        at: Position::new(0., 0.),
        pocket_tiles: 12.5,
        searched_tiles: 48.,
    });
    world.record_bench(Bench {
        tick: None,
        player: 2,
        at: Position::new(0., 0.),
        refused_hops: 4,
        hop_tiles: HOP_DISTANCE,
    });

    world
}

/// Round-trips a world the way a dump does: to a JSON string and back.
fn dumped(world: &FactorioSurface) -> FactorioSurface {
    let json = serde_json::to_string(world).expect("a world serialises");
    serde_json::from_str(&json).expect("and comes back")
}

/// The headline: identical in, identical out.
#[test]
fn a_plan_from_a_dumped_world_is_the_plan_from_the_live_one() {
    let live = Arc::new(world_mid_run());
    let loaded = Arc::new(dumped(&live));

    let from_live = plan_of(live);
    let from_loaded = plan_of(loaded);

    // Named separately before the whole-plan comparison, because a failure
    // here should say *what* diverged rather than print two hundred actions.
    assert_eq!(
        from_loaded.schedule.makespan, from_live.schedule.makespan,
        "the makespan moved"
    );
    assert_eq!(
        from_loaded.actions.len(),
        from_live.actions.len(),
        "a different number of actions"
    );
    for (loaded, live) in from_loaded.actions.iter().zip(&from_live.actions) {
        assert_eq!(
            loaded.label, live.label,
            "action {} is a different act",
            live.id.0
        );
        assert_eq!(
            loaded, live,
            "action {} differs beyond its label",
            live.id.0
        );
    }
    assert_eq!(from_loaded.chains, from_live.chains, "the chains moved");
    assert_eq!(
        from_loaded.schedule.steps, from_live.schedule.steps,
        "the schedule moved"
    );
    assert_eq!(from_loaded, from_live);
}

/// A plan worth comparing at all: this fixture and goal produce a real plan,
/// not an empty one that two of anything would agree about.
#[test]
fn the_plan_being_compared_is_a_real_one() {
    let plan = plan_of(Arc::new(world_mid_run()));
    assert!(
        plan.actions.len() > 10,
        "only {} actions -- the equality above would be near-vacuous",
        plan.actions.len()
    );
    assert!(plan.schedule.makespan > 0);
    assert!(
        plan.chains.iter().any(|(_, chain, _)| chain.is_some()),
        "no chains at all, so chain equality proves nothing"
    );
}

/// What the gap cost, reproduced.
///
/// Stripping the four ledger fields out of the JSON is exactly what the old
/// serialization wrote. The world that comes back is the same map with the
/// same entities — the furnace is still standing, because entities *are*
/// serialized — and it plans differently, because nothing remembers what is
/// inside it. That is the silent loss: no error, no missing field, a plausible
/// plan that mines ore it did not have to.
#[test]
fn the_ledgers_are_load_bearing_for_this_fixture() {
    let live = world_mid_run();
    let mut value: serde_json::Value =
        serde_json::to_value(&live).expect("a world serialises to a value");
    let object = value.as_object_mut().expect("a struct is a map");
    for gone in [
        "inventories",
        "placement_refusals",
        "walk_refusals",
        "enclosures",
        "benches",
    ] {
        assert!(object.remove(gone).is_some(), "{gone} was written");
    }
    let forgetful: FactorioSurface =
        serde_json::from_value(value).expect("an older dump is still readable");

    let with = plan_of(Arc::new(live));
    let without = plan_of(Arc::new(forgetful));
    assert_ne!(
        with, without,
        "the ledgers changed nothing here, so the equality test above proves \
         nothing about them -- give the fixture something that depends on one"
    );
}

/// The ledgers reach the plan state, not merely the world.
///
/// The equality above would still pass if `from_world` had stopped reading a
/// ledger entirely, so this asks the loaded world the same questions
/// `from_world` asks it.
#[test]
fn a_loaded_world_answers_the_four_questions_from_world_asks() {
    let live = world_mid_run();
    let loaded = dumped(&live);

    assert_eq!(
        loaded.observed_inventories(),
        live.observed_inventories(),
        "buffers"
    );
    assert_eq!(
        loaded.placement_refusals(),
        live.placement_refusals(),
        "refused sites"
    );
    assert_eq!(
        loaded.walk_refusals(),
        live.walk_refusals(),
        "refused walks"
    );
    assert_eq!(loaded.enclosures(), live.enclosures(), "enclosures");
    assert_eq!(loaded.benches(), live.benches(), "benches");

    let state = PlanState::from_world(Arc::new(loaded), &BOTS);
    assert!(
        !state
            .buffers_holding(&Position::new(0., 0.), "iron-plate")
            .is_empty(),
        "the furnace's plates did not survive into the plan state"
    );
}
