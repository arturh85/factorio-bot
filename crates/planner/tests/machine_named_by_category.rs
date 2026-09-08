//! **The wall moved from "no machine" to "a fluid".**
//!
//! Sequel to `oil_category_gate.rs`, which established by experiment that the
//! category gate was never a scope decision — it was the edge of what the wire
//! could name — and that two further walls stood behind it. This file measures
//! the first one falling.
//!
//! `LuaEntityPrototype.crafting_categories` now crosses the bridge, so
//! `method::machine::MachineTable` can say that `oil-processing` is run by an
//! `oil-refinery` and `chemistry` by a `chemical-plant`. `method::fabricate`
//! uses that, and the refusal a caller gets changes accordingly.
//!
//! # How the world is built, and why the field is injected here
//!
//! `crates/core/tests/live-2.1.17-world-snapshot.json` predates the field — it
//! was captured before the mod sent it — so its prototypes all read `None`.
//! The categories injected below are transcribed from
//! `workspace/server/data/base/prototypes/entity/entities.lua`, the game's own
//! data files, and `parameters` is added to the four machines a live 2.1.17
//! game reports it on (measured, and recorded in
//! `docs/superpowers/notes/2026-09-07-a-recipe-the-planner-cannot-run.md`).
//! Every injection asserts it matched, so a capture that gains the field for
//! real fails here rather than silently measuring nothing.
//!
//! **Every test carries its undeclared control**: the same world, the same
//! goal, with the field absent. Without it, "the refusal names a fluid" would
//! be equally explained by a world that refuses everything.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{
    FactorioEntityPrototype, FactorioItemPrototype, FactorioPlayer, FactorioRecipe, Position,
};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::machine::{Machine, MachineRefusal, MachineTable};
use factorio_bot_planner::{BotId, PlanState, expand, registry_for};
use std::sync::Arc;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

const BOTS: [BotId; 4] = [BotId(1), BotId(2), BotId(3), BotId(4)];

/// Vanilla 2.1.17's crafting machines and what each declares, read off
/// `workspace/server/data/base/prototypes/entity/entities.lua`. `parameters`
/// is the runtime-only blueprint pseudo-category, present on a live game and
/// in no data file.
const VANILLA_CATEGORIES: &[(&str, &[&str])] = &[
    ("character", &["crafting", "hand-crafting"]),
    ("stone-furnace", &["smelting"]),
    ("steel-furnace", &["smelting"]),
    ("electric-furnace", &["smelting"]),
    (
        "assembling-machine-1",
        &["crafting", "advanced-crafting", "parameters"],
    ),
    (
        "assembling-machine-2",
        &[
            "crafting",
            "advanced-crafting",
            "crafting-with-fluid",
            "parameters",
        ],
    ),
    (
        "assembling-machine-3",
        &[
            "crafting",
            "advanced-crafting",
            "crafting-with-fluid",
            "parameters",
        ],
    ),
    ("oil-refinery", &["oil-processing", "parameters"]),
    ("chemical-plant", &["chemistry", "parameters"]),
    ("centrifuge", &["centrifuging", "parameters"]),
    ("rocket-silo", &["rocket-building", "parameters"]),
];

fn snapshot() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture parses")
}

/// The live capture's prototype for `name`, which must exist.
fn prototype(snapshot: &WorldSnapshot, name: &str) -> FactorioEntityPrototype {
    snapshot
        .entity_prototypes
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("the live capture carries a {name} prototype"))
        .clone()
}

/// The fixture terrain with the live capture's tables installed, `edit`
/// applied to the recipes, and the crafting machines optionally told what they
/// craft.
///
/// Same construction as `oil_category_gate.rs`: the fixture's own recipe table
/// is hand-written, so a claim made against it would be a claim about a table
/// written by the same hands as the code.
fn live_state_with<F>(bots: &[BotId], declare_categories: bool, edit: F) -> PlanState
where
    F: FnOnce(&mut Vec<FactorioRecipe>),
{
    let snapshot = snapshot();
    let world = fixture_world();
    let mut recipes: Vec<FactorioRecipe> = snapshot.recipes.clone();
    let items: Vec<FactorioItemPrototype> = snapshot.item_prototypes.clone();
    assert!(
        recipes.len() > 600 && items.len() > 300,
        "the live capture carries a whole game's tables, not a stub: {} recipes, {} items",
        recipes.len(),
        items.len()
    );
    edit(&mut recipes);
    world
        .update_recipes(recipes)
        .expect("installing the recipe table");
    world
        .update_item_prototypes(items)
        .expect("installing the live item-prototype table");

    // The five crafting machines, taken from the capture rather than built:
    // their collision boxes decide where `free_area_near` may put one, and a
    // hand-written box is a fixture fitted to the code.
    let mut installed = 0;
    for (name, categories) in VANILLA_CATEGORIES {
        let mut proto = prototype(&snapshot, name);
        assert_eq!(
            proto.crafting_categories, None,
            "the capture predates the field; if {name} now carries it, this injection is stale \
             and the tests below are measuring the capture rather than the experiment"
        );
        if declare_categories {
            proto.crafting_categories = Some(categories.iter().map(|c| (*c).to_string()).collect());
        }
        world
            .globals
            .entity_prototypes
            .insert(proto.name.clone(), proto);
        installed += 1;
    }
    assert_eq!(installed, VANILLA_CATEGORIES.len());

    // A roster holding the machines, so that the machine's own bill is never
    // what refuses. Standing apart so four bots do not share a tile.
    for (index, bot) in bots.iter().enumerate() {
        let mut inventory = std::collections::HashMap::new();
        inventory.insert("chemical-plant".to_string(), 4u32);
        inventory.insert("centrifuge".to_string(), 4u32);
        inventory.insert("oil-refinery".to_string(), 4u32);
        inventory.insert("iron-plate".to_string(), 200u32);
        inventory.insert("sulfur".to_string(), 200u32);
        inventory.insert("coal".to_string(), 200u32);
        // And the parts of a power plant, for the same reason: a
        // `chemical-plant` draws 210 kW, so since 2026-09-08 `method::fabricate`
        // asks `method::power::ensure_powered` for it and the plan builds a
        // plant when no network stands. Without these the goal refuses with
        // `have 1 offshore-pump`, which is a true statement about this fixture
        // and says nothing about the recipe these tests are about.
        inventory.insert("offshore-pump".to_string(), 4u32);
        inventory.insert("boiler".to_string(), 4u32);
        inventory.insert("steam-engine".to_string(), 4u32);
        inventory.insert("small-electric-pole".to_string(), 200u32);
        inventory.insert("pipe".to_string(), 200u32);
        world.globals.players.insert(
            bot.0,
            FactorioPlayer {
                player_id: bot.0,
                position: Position::new(20.0 + (index as f64) * 6.0, 20.0),
                main_inventory: inventory.into_iter().collect(),
                ..Default::default()
            },
        );
    }
    PlanState::from_world(Arc::new(world), bots)
}

/// Enable a recipe outright, so `recipe_gate` cannot be the thing that
/// refuses. Every oil and chemistry recipe ships disabled.
fn enable(recipes: &mut [FactorioRecipe], name: &str) {
    let mut hits = 0;
    for r in recipes.iter_mut() {
        if r.name == name {
            r.enabled = true;
            hits += 1;
        }
    }
    assert_eq!(hits, 1, "exactly one recipe named {name} must be enabled");
}

/// Strip the fluid ingredients from one recipe, and assert the bill shrank.
///
/// The same shape `oil_category_gate.rs` uses for its control: the world
/// moves, `src/` does not.
fn drop_fluid_ingredients(recipes: &mut [FactorioRecipe], name: &str) {
    let mut hits = 0;
    for r in recipes.iter_mut() {
        if r.name == name {
            let before = r.ingredients.as_ref().map(Vec::len).unwrap_or(0);
            if let Some(ingredients) = r.ingredients.as_mut() {
                ingredients.retain(|i| i.ingredient_type != "fluid");
            }
            let after = r.ingredients.as_ref().map(Vec::len).unwrap_or(0);
            assert!(
                after < before,
                "{name} must have had a fluid ingredient to drop: {before} -> {after}"
            );
            hits += 1;
        }
    }
    assert_eq!(hits, 1);
}

fn plan_error(state: PlanState, goal: Goal) -> String {
    let registry = registry_for(&BOTS);
    let said = goal.to_string();
    match expand(&[goal], &state, &registry, BOTS[0]) {
        Ok(_) => panic!("expected a refusal for {said}"),
        Err(e) => format!("{e:?} :: {e}"),
    }
}

// ---------------------------------------------------------------------------
// The machine is nameable
// ---------------------------------------------------------------------------

/// The headline. A world that says what its machines craft can name the
/// machine for a category, and one that does not says *that* instead.
#[test]
fn declaring_what_a_machine_crafts_names_the_machine_for_its_category() {
    let declared = live_state_with(&BOTS, true, |_| {});
    let table = MachineTable::from_state(&declared);
    assert!(table.world_declares_categories());
    assert_eq!(
        table.machine_for("oil-processing"),
        Ok(Machine::Entity("oil-refinery".into()))
    );
    assert_eq!(
        table.machine_for("chemistry"),
        Ok(Machine::Entity("chemical-plant".into()))
    );
    assert_eq!(
        table.machine_for("centrifuging"),
        Ok(Machine::Entity("centrifuge".into()))
    );

    // The control that makes the claim non-accidental: the SAME world with
    // the field absent -- which is every dump this project has ever taken --
    // refuses, and its refusal says "did not say", not "nothing crafts this".
    let silent = live_state_with(&BOTS, false, |_| {});
    let table = MachineTable::from_state(&silent);
    assert!(!table.world_declares_categories());
    assert_eq!(
        table.machine_for("oil-processing"),
        Err(MachineRefusal::NoMachine {
            category: "oil-processing".into(),
            world_says: false,
        })
    );
    assert!(
        table
            .machine_for("oil-processing")
            .unwrap_err()
            .to_string()
            .contains("it did not say")
    );
}

/// The ambiguous categories are real vanilla ones, and since 2026-09-08 they
/// are **answered by the cost preference rather than refused** -- not silently:
/// `assembling-machine-1` is named because it is the cheapest of the three to
/// obtain on this world, priced transitively from its recipes down to the
/// charted ground.
///
/// The refusal is still what a tie gets, and that half lives in
/// `method::machine`'s own tests, where a fixture can make two machines cost
/// the same. On a live capture they never do.
#[test]
fn a_category_several_vanilla_machines_declare_names_the_cheapest() {
    let declared = live_state_with(&BOTS, true, |_| {});
    let table = MachineTable::from_state(&declared);
    assert_eq!(
        table.machine_for("advanced-crafting"),
        Ok(Machine::Entity("assembling-machine-1".into()))
    );
    assert!(
        table
            .runnable_categories()
            .contains(&"advanced-crafting".to_string())
    );
    // The control: this is a choice among three, not a category only one
    // machine declares.
    assert_eq!(
        table
            .machine_for("crafting-with-fluid")
            .expect("two machines declare it and one is cheaper"),
        Machine::Entity("assembling-machine-2".into())
    );
}

// ---------------------------------------------------------------------------
// The wall moved
// ---------------------------------------------------------------------------

/// **The deliverable.** `plastic-bar` is coal (an item) plus 20 petroleum-gas
/// (a fluid) in a `chemical-plant`. Before the field, the refusal blamed the
/// category. After it, the machine is named and the refusal blames the fluid,
/// by name and by amount.
///
/// 20 is the finding: it is the recipe's own ingredient amount, so the bill
/// really was walked with the machine already chosen. A refusal merely
/// *mentioning* petroleum-gas would be equally explained by the old gate.
#[test]
fn a_fluid_ingredient_is_refused_with_the_machine_named() {
    let goal = || Goal::Have {
        item: "plastic-bar".into(),
        count: 2,
        whose: Holder::Anyone,
        via: None,
    };

    let before = plan_error(
        live_state_with(&BOTS, false, |r| enable(r, "plastic-bar")),
        goal(),
    );
    assert!(
        before.contains("none is in a category this planner runs"),
        "the undeclared world blames the category: {before}"
    );
    assert!(
        !before.contains("chemical-plant"),
        "and cannot name a machine: {before}"
    );

    let after = plan_error(
        live_state_with(&BOTS, true, |r| enable(r, "plastic-bar")),
        goal(),
    );
    assert!(after.contains("chemical-plant"), "{after}");
    assert!(after.contains("category chemistry"), "{after}");
    assert!(after.contains("20 petroleum-gas"), "{after}");
    assert!(after.contains("a fluid, so it arrives by pipe"), "{after}");
    // Since 2026-09-07 the missing thing is a standing SOURCE, not an action
    // that carries a fluid: a fluid ingredient is satisfied by connectivity.
    assert!(after.contains("can be shown to supply"), "{after}");
    // And it says where the fluid would have to come from, which is what
    // `substance::FluidSource` is for.
    assert!(after.contains("basic-oil-processing"), "{after}");
}

/// The other half of the same bill: with its fluid gone, the identical recipe
/// in the identical machine **plans**. Without this the refusal above is
/// equally explained by "a chemistry recipe cannot be planned at all".
#[test]
fn the_same_recipe_without_its_fluid_plans_in_the_named_machine() {
    let state = live_state_with(&BOTS, true, |r| {
        enable(r, "plastic-bar");
        drop_fluid_ingredients(r, "plastic-bar");
    });
    let registry = registry_for(&BOTS);
    let goal = Goal::Have {
        item: "plastic-bar".into(),
        count: 2,
        whose: Holder::Anyone,
        via: None,
    };
    let net = expand(&[goal], &state, &registry, BOTS[0])
        .expect("an all-item chemistry recipe plans once the machine is nameable");
    let labels: Vec<String> = net.actions().map(|a| a.label.clone()).collect();
    assert!(!labels.is_empty(), "a plan with no actions proves nothing");
    assert!(
        labels.iter().any(|l| l.contains("place chemical-plant")),
        "the machine is stood up: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("set chemical-plant to plastic-bar")),
        "and told what to run: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.starts_with("take 1 plastic-bar from the chemical-plant")),
        "and emptied -- one each, because the four-bot splitter dealt the goal \
         out and each machine runs one craft of a recipe that yields two: {labels:?}"
    );
}

/// **Several fluid products are no longer refused on their count** -- each one
/// gets its own buffer, its own output fluidbox and its own pipe run.
///
/// # What this test used to assert, and why that assertion was a proxy
///
/// It required the words `3 fluids` and `stalls`, which were
/// `FabricateRefusal::ManyFluidProducts` -- a refusal on the *number* of fluid
/// products. The number was never the thing that goes wrong. What goes wrong is
/// a product with nowhere to go, and "nowhere to go" is answerable: a buffer
/// this plan builds is somewhere, exactly as it already was for one product.
/// So the count refusal was replaced by arithmetic on sinks, and this test now
/// pins the capability rather than the wall.
///
/// # The one thing that must not regress
///
/// The three runs must stay on **three separate pipe segments**. A Factorio
/// pipe segment holds one fluid, so two of the refinery's output boxes joined
/// into one segment is a refinery that jams and then produces none of its three
/// fluids, having placed every entity correctly.
///
/// Its fluid *ingredients* are dropped so the rig is exercised from the product
/// side alone; with them the plan refuses one rung earlier, on a crude source
/// this fixture has no wellhead for.
#[test]
fn several_fluid_products_each_get_their_own_sink_and_their_own_pipe() {
    let mut state = live_state_with(&BOTS, true, |r| {
        enable(r, "advanced-oil-processing");
        drop_fluid_ingredients(r, "advanced-oil-processing");
        // **The co-products need somewhere to go, and this fixture has no
        // technology table**, so every recipe it does not enable is
        // `RecipeGate::Unobtainable` -- a recipe no research can ever reach.
        // Enabling the two terminal consumers is what a game that has
        // researched `oil-processing` looks like; without them the rig is
        // refused as `NoDisposal`, correctly, and this test would be asserting
        // geometry it never reaches.
        //
        // The petroleum-gas is what the goal asks for and needs none.
        enable(r, "solid-fuel-from-heavy-oil");
        enable(r, "solid-fuel-from-light-oil");
    });
    // The rig's parts are put in a hand rather than crafted: this test is
    // about the *geometry* of three sinks, and a `storage-tank`'s own bill
    // wants steel this fixture cannot smelt. Obtaining them is
    // `method::have`'s problem and is tested where that lives.
    for bot in BOTS {
        for (item, count) in [("oil-refinery", 1u32), ("storage-tank", 3), ("pipe", 200)] {
            state.gain(bot, item, count);
        }
    }
    let registry = registry_for(&BOTS);
    let goal = Goal::Produced {
        item: "petroleum-gas".into(),
        count: 55,
        whose: Holder::Anyone,
        unlocks: None,
        via: Some("advanced-oil-processing".into()),
    };
    let net = expand(&[goal], &state, &registry, BOTS[0])
        .expect("three fluid products are three sinks, not a refusal");
    let labels: Vec<String> = net.actions().map(|a| a.label.clone()).collect();
    assert!(
        labels.iter().any(|l| l.contains("place oil-refinery")),
        "the machine is stood up: {labels:?}"
    );
    // One buffer per fluid product.
    let tanks = labels
        .iter()
        .filter(|l| l.contains("place storage-tank"))
        .count();
    assert_eq!(tanks, 3, "one buffer per fluid product: {labels:?}");
    // And a run for each, each naming its own fluid.
    for fluid in ["heavy-oil", "light-oil", "petroleum-gas"] {
        assert!(
            labels
                .iter()
                .any(|l| l.contains("place pipe") && l.contains(&format!("carry {fluid} "))),
            "a pipe run carries {fluid}: {labels:?}"
        );
    }

    // **And a chemical plant per co-product, chosen off the LIVE recipe
    // table.** This is the part no hand-written fixture could establish: the
    // candidates here are the game's own -- heavy-oil is also consumed by
    // `coal-liquefaction`, `lubricant`, `heavy-oil-cracking` and
    // `heavy-oil-barrel`, and light-oil by `light-oil-cracking`,
    // `rocket-fuel`, `superconductor` and `light-oil-barrel` -- and the
    // terminal-consumer rule has to pick the two `solid-fuel-from-*` out of
    // that set. The cracking recipes are not merely outranked, they are not
    // candidates: a fluid product would need placing in turn.
    for recipe in ["solid-fuel-from-heavy-oil", "solid-fuel-from-light-oil"] {
        assert!(
            labels
                .iter()
                .any(|l| l == &format!("set chemical-plant to {recipe}")),
            "the {recipe} disposal plant is stood up and told what to run: {labels:?}"
        );
    }
    // **And it is fed from that co-product's own buffer, not from thin air.**
    // The tank is placed by the refinery's rig moments earlier and adopted by
    // the plant's, which is the whole reason `pipe::sources_of` had to learn
    // to attribute a buffer this plan built.
    for fluid in ["heavy-oil", "light-oil"] {
        assert!(
            labels.iter().any(|l| l.contains("place pipe")
                && l.contains(&format!("carry {fluid} between the chemical-plant"))),
            "the {fluid} plant is piped from the buffer that caught it: {labels:?}"
        );
    }
    assert!(
        !labels.iter().any(|l| l.contains("cracking")),
        "cracking is fluid-to-fluid and is never disposal: {labels:?}"
    );
    // The petroleum-gas is what the goal asked for; it lands in its buffer and
    // gets no consumer.
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("solid-fuel-from-petroleum-gas")),
        "the asked-for product is not disposed of: {labels:?}"
    );
}

/// The rung the whole ladder is aimed at. `produced:petroleum-gas` does not
/// plan and was never going to -- `basic-oil-processing` is fluid in and fluid
/// out. What changes is *what it says*: the old refusal blamed the category
/// (which sent a reader looking for a machine), the new one blames the fact
/// that four runnable recipes now produce it and nothing chooses.
///
/// **This is the wall moving, not falling**, and pinning it is the point.
#[test]
fn petroleum_gas_refuses_for_a_different_reason_once_the_machine_is_nameable() {
    let goal = || Goal::Produced {
        item: "petroleum-gas".into(),
        count: 100,
        whose: Holder::Anyone,
        unlocks: None,
        via: None,
    };
    let before = plan_error(live_state_with(&BOTS, false, |_| {}), goal());
    assert!(
        before.contains("none is in a category this planner runs"),
        "{before}"
    );

    let after = plan_error(live_state_with(&BOTS, true, |_| {}), goal());
    assert!(
        !after.contains("none is in a category this planner runs"),
        "the category is no longer the blocker: {after}"
    );
    assert!(after.contains("basic-oil-processing"), "{after}");
    assert!(after.contains("advanced-oil-processing"), "{after}");
    // Ambiguity, not absence: the machine is nameable for three of the four
    // and the planner refuses to pick a recipe rather than guessing.
    assert!(after.contains("nothing here can choose"), "{after}");
}
