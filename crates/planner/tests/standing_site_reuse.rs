//! A replan finishes what its earlier plans started, rather than building it
//! again beside them.
//!
//! # The run
//!
//! `workspace/headless-a/runs/run-1788608648-56109` -- `producing:logistic-
//! science-pack:6`, seed 31337, four character bots -- replanned four times
//! after unrelated placement failures, and each plan sited its own power and
//! its own labs and its own cells as if the world were empty. Its last
//! keyframe held **3 offshore pumps, 2 boilers, 1 steam engine, 4 labs and 6
//! assembling machines** for a goal wanting one plant, one lab and four
//! machines, and its fifth plan halted with *"no shoreline within 10 tiles of
//! it has room for a pump, a boiler, a steam engine and the pipes between
//! them"*: the shore was full of the run's own half-built plants.
//!
//! Three mechanisms, each pinned here against the crate's public API so that
//! this file compiles against the code that had the defect and fails there:
//!
//! * a pump, pipes and boiler with no engine were **passed over** by
//!   `power::supply_for` -- no pole with generation behind it means no network
//!   to adopt, and `plan_plant` sites only on ground its whole layout can take
//!   -- so the next plan built a second plant up the shore;
//! * two assembling machines with nothing else of their cell were passed over
//!   by `assemble::plan_cells` for the same reason, so the next plan sited a
//!   whole new cell beside them;
//! * and those dead machines were **charged 75 kW each** on the network they
//!   stood in, so by the fourth plan a 900 kW engine with nothing running on it
//!   read 145 kW of headroom, and the cell that wanted 189 got a third plant.
//!
//! The unit tests in `method/power.rs`, `method/assemble.rs`, `method/have.rs`
//! and `state.rs` cover each tier in place; these are the same claims from
//! outside, and the ones that were run against the pre-fix tree.

use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioRecipe, Position};
use factorio_bot_planner::goal::Goal;
use factorio_bot_planner::method::assemble::{Role, assembly_spec, plan_cell};
use factorio_bot_planner::method::power::{BOILER, ENGINE, PIPE, POLE, PUMP, plan_plant};
use factorio_bot_planner::{ActionKind, ActionNetwork, BotId, PlanState, expand, registry_for};
use std::sync::Arc;

const PACK: &str = "automation-science-pack";
const MACHINE: &str = "assembling-machine-1";

/// `fixture_world()` plus the `assembling-machine-1` recipe the 1.1 capture
/// lacks -- the live 2.1.17 one, exactly as `red_science_cell.rs` adds it.
fn world() -> factorio_bot_core::factorio::world::FactorioSurface {
    let world = fixture_world();
    let recipe: FactorioRecipe = serde_json::from_str(
        r#"{
          "name": "assembling-machine-1",
          "valid": true,
          "enabled": true,
          "category": "crafting",
          "ingredients": [
            { "name": "iron-plate", "ingredient_type": "item", "amount": 9 },
            { "name": "iron-gear-wheel", "ingredient_type": "item", "amount": 5 },
            { "name": "electronic-circuit", "ingredient_type": "item", "amount": 3 }
          ],
          "products": [
            { "name": "assembling-machine-1", "product_type": "item", "amount": 1, "probability": 1.0 }
          ],
          "hidden": false,
          "energy": 0.5,
          "order": "a[items]-a[assembling-machine-1]",
          "group": "production",
          "subgroup": "production-machine"
        }"#,
    )
    .expect("the assembling machine recipe parses");
    world
        .update_recipes(vec![recipe])
        .expect("update_recipes cannot fail for a well-formed recipe");
    world
}

fn bare(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(world()), bots);
    for bot in bots {
        state.gain(*bot, "wood", 2);
    }
    state
}

/// An entity of `name` at `position`, typed off the prototype the way the
/// planner's own placements are.
fn entity(
    state: &PlanState,
    name: &str,
    position: Position,
    direction: Direction,
) -> FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    FactorioEntity {
        name: name.to_string(),
        entity_type,
        position,
        direction: factorio_bot_core::num_traits::ToPrimitive::to_u8(&direction).unwrap_or(0),
        ..Default::default()
    }
}

fn placed(net: &ActionNetwork, name: &str) -> Vec<Position> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } if entity.name == name => Some(entity.position.clone()),
            _ => None,
        })
        .collect()
}

fn red_cell(state: &PlanState, bots: &[BotId]) -> ActionNetwork {
    expand(
        &[Goal::Producing {
            item: PACK.into(),
            per_minute: 6,
        }],
        state,
        &registry_for(bots),
        BotId(1),
    )
    .expect("the fixture can plan a red science cell")
}

/// **Plan 2 of the run.** The pump at `[46.5, -8.5]`, three pipes and the
/// boiler at `[45, -5.5]` were down; the engine was not. The next plan places
/// exactly the engine, where the standing pump's own layout puts it, and a
/// pole for it -- and no second pump or boiler.
#[test]
fn a_half_built_plant_is_finished_rather_than_replaced() {
    let bots = [BotId(1)];
    let mut s = bare(&bots);
    let plant = plan_plant(&s, &Position::new(40., 40.)).expect("the fixture has a lake");
    for part in plant
        .parts
        .iter()
        .filter(|part| [PUMP, PIPE, BOILER].contains(&part.name))
    {
        s.create_entity(entity(&s, part.name, part.position.clone(), part.direction));
    }
    let net = red_cell(&s, &bots);
    for name in [PUMP, BOILER, PIPE] {
        assert_eq!(
            placed(&net, name),
            Vec::<Position>::new(),
            "{name} stands already"
        );
    }
    assert_eq!(
        placed(&net, ENGINE),
        vec![plant.engine.clone()],
        "exactly one engine, on the standing plant's own layout"
    );
    assert!(
        placed(&net, POLE).contains(&plant.pole),
        "and the pole that carries it: {:?}",
        placed(&net, POLE)
    );
}

/// **Plan 3 of the run.** Two assembling machines stood with nothing else of
/// their cell; the next plan placed four more. A cell whose machines stand is
/// finished around them: no machine is placed, both recipes are set on the
/// standing ones.
#[test]
fn a_cell_whose_machines_stand_is_finished_around_them() {
    let bots = [BotId(1)];
    let mut s = bare(&bots);
    for (name, position) in [
        (POLE, Position::new(10.5, 10.5)),
        (ENGINE, Position::new(12.5, 10.5)),
        (BOILER, Position::new(12.5, 14.5)),
    ] {
        s.create_entity(entity(&s, name, position, Direction::North));
    }
    let spec = assembly_spec(&s, PACK).expect("red science is a cell");
    let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec).expect("room beside the plant");
    let mut machines: Vec<Position> = Vec::new();
    for role in [Role::Intermediate, Role::Product] {
        let part = cell.at(role).expect("a cell has two machines");
        s.create_entity(entity(&s, MACHINE, part.position.clone(), part.direction));
        machines.push(part.position.clone());
    }
    let net = red_cell(&s, &bots);
    assert_eq!(
        placed(&net, MACHINE),
        Vec::<Position>::new(),
        "the machines stand and are not placed again"
    );
    let mut recipes: Vec<Position> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::SetRecipe { pos, .. } => Some(pos.clone()),
            _ => None,
        })
        .collect();
    let sort =
        |v: &mut Vec<Position>| v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sort(&mut recipes);
    sort(&mut machines);
    assert_eq!(recipes, machines, "both standing machines get their recipe");
}

/// **Plan 4 of the run.** Five recipe-less machines from two abandoned cells
/// were charged 375 kW of the one engine's 900. A crafting machine with no
/// recipe never crafts and is charged nothing; the moment a recipe goes on
/// it, it is.
#[test]
fn a_crafting_machine_with_no_recipe_draws_nothing_from_the_ledger() {
    let bots = [BotId(1)];
    let mut s = bare(&bots);
    for (name, position) in [
        (POLE, Position::new(10.5, 10.5)),
        (ENGINE, Position::new(12.5, 10.5)),
    ] {
        s.create_entity(entity(&s, name, position, Direction::North));
    }
    let dead = Position::new(11.5, 8.5);
    s.create_entity(entity(&s, MACHINE, dead.clone(), Direction::North));
    let area = s
        .collision_area(MACHINE, &dead)
        .expect("the fixture sizes a machine");
    assert_eq!(
        s.electric_supply_kw(&area),
        900.0,
        "the premise: the machine is on the network"
    );
    assert_eq!(
        s.electric_demand_kw(&area, None),
        0.0,
        "a machine with no recipe is not a load"
    );
    s.set_recipe(&dead, "iron-gear-wheel")
        .expect("the machine stands");
    assert_eq!(
        s.electric_demand_kw(&area, None),
        75.0,
        "and a machine with one is"
    );
}
