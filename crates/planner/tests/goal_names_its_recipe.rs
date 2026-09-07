//! **A goal that names its recipe.**
//!
//! Sequel to `machine_named_by_category.rs`, which moved the wall from *"no
//! machine runs this category"* to *"four recipes this planner can run produce
//! petroleum-gas and nothing here can choose between them; ask for a recipe by
//! name rather than for the product"* -- a message asking for a vocabulary
//! that did not exist. `Goal::Have::via` / `Goal::Produced::via` is that
//! vocabulary.
//!
//! # What is measured here
//!
//! * an unqualified goal is **unchanged**, on the same world and the same
//!   binary -- the qualifier is opt-in and moves nothing;
//! * naming `basic-oil-processing` moves the refusal one rung, to the fluid
//!   *ingredient*, which is where the brief predicted it and where the open
//!   owner decision lives;
//! * every way a named recipe can be wrong is refused **by name**, and each
//!   one is a different message: no such recipe, a recipe that does not
//!   produce it, a recipe with no machine, a recipe the hand-craft methods
//!   cannot be told about;
//! * **absent is not a value** -- `via: None` and `via: Some(x)` where `x`
//!   does not exist are different facts and produce different messages, and
//!   the `None` message is the one that existed before.
//!
//! # The world
//!
//! Built exactly as `machine_named_by_category.rs` builds it: the live 2.1.17
//! capture with `crafting_categories` injected from the game's own data files,
//! and every test that needs the contrast carries its **undeclared control**.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{
    FactorioEntityPrototype, FactorioItemPrototype, FactorioPlayer, FactorioRecipe, Position,
};
use factorio_bot_planner::goal::{Goal, Holder};
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
// Absent is not a value
// ---------------------------------------------------------------------------

/// **The opt-in guarantee.** On the world where the qualifier is possible, a
/// goal that does not use it gets the message it got before the qualifier
/// existed -- the ambiguity, naming all four runnable recipes.
///
/// Paired with a non-accidental assertion from the same computation: the four
/// names are not a list this test wrote down, they are exactly the recipes the
/// index reports as producing petroleum-gas in a runnable category, and the
/// fifth producer (`empty-petroleum-gas-barrel`, `crafting-with-fluid`, which
/// is ambiguous between three assembling machines) is absent for that reason.
#[test]
fn a_goal_that_names_no_recipe_gets_the_message_it_always_got() {
    let said = plan_error(
        live_state_with(&BOTS, true, |_| {}),
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: None,
        },
    );
    assert!(
        said.contains("nothing here can choose between them"),
        "{said}"
    );
    assert!(said.contains("ask for a recipe by name"), "{said}");
    for runnable in [
        "advanced-oil-processing",
        "basic-oil-processing",
        "coal-liquefaction",
        "light-oil-cracking",
    ] {
        assert!(said.contains(runnable), "{runnable} missing from {said}");
    }
    assert!(
        !said.contains("empty-petroleum-gas-barrel"),
        "the fifth producer runs in a category no single machine owns, so it is \
         not one of the choices: {said}"
    );
}

/// **`via: None` and `via: Some(a name nothing has)` are different facts.**
///
/// The seventh instance of this conflation is catalogued in this repo; this is
/// the test that stops the eighth. The two messages must not be the same
/// string, and the qualified one must be about the *recipe*, not about the
/// product -- petroleum-gas is produced by five recipes, so "no recipe
/// produces it" would be a lie.
#[test]
fn a_recipe_that_does_not_exist_is_not_the_same_as_no_recipe_named() {
    let goal = |via: Option<&str>| Goal::Produced {
        item: "petroleum-gas".into(),
        count: 100,
        whose: Holder::Anyone,
        unlocks: None,
        via: via.map(str::to_string),
    };
    let absent = plan_error(live_state_with(&BOTS, true, |_| {}), goal(None));
    let missing = plan_error(
        live_state_with(&BOTS, true, |_| {}),
        goal(Some("no-such-recipe")),
    );
    assert_ne!(absent, missing);
    assert!(missing.contains("no-such-recipe"), "{missing}");
    assert!(
        missing.contains("no recipe of that name exists in this world"),
        "{missing}"
    );
    // And it still tells the caller what *does* produce it, so the message is
    // actionable rather than merely correct.
    assert!(missing.contains("basic-oil-processing"), "{missing}");
    assert!(
        !missing.contains("no recipe in this world produces"),
        "the product is makeable; only the name was wrong: {missing}"
    );
}

// ---------------------------------------------------------------------------
// The wall moves one rung
// ---------------------------------------------------------------------------

/// **The deliverable.** Naming `basic-oil-processing` answers the question the
/// old refusal asked, and the next thing missing is named: the recipe's own
/// 100 crude-oil, which is a fluid.
///
/// The `100` is the non-accidental half: it is `basic-oil-processing`'s own
/// ingredient amount, read out of the live capture, so the bill really was
/// walked with the recipe already chosen. A refusal merely *mentioning*
/// crude-oil would be equally explained by the old category message.
#[test]
fn naming_the_recipe_moves_the_refusal_to_the_fluid_ingredient() {
    let said = plan_error(
        live_state_with(&BOTS, true, |r| enable(r, "basic-oil-processing")),
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("basic-oil-processing".into()),
        },
    );
    assert!(
        !said.contains("nothing here can choose between them"),
        "the ambiguity is answered: {said}"
    );
    assert!(
        said.contains("basic-oil-processing runs in oil-refinery"),
        "{said}"
    );
    assert!(said.contains("100 crude-oil"), "{said}");
    assert!(said.contains("which is a fluid"), "{said}");
}

/// The other rung of the same ladder, for the recipe whose *product* is the
/// wall rather than its ingredient. `light-oil-cracking` is chemistry, so the
/// machine is a chemical-plant, and with its fluid ingredients dropped what is
/// left is a fluid product with nowhere to land.
///
/// Its value here is that naming a recipe reaches a **different** wall than
/// the test above: the qualifier chooses which recipe, and the recipe chooses
/// which wall.
#[test]
fn naming_a_different_recipe_for_the_same_product_reaches_a_different_wall() {
    let said = plan_error(
        live_state_with(&BOTS, true, |r| {
            enable(r, "light-oil-cracking");
            drop_fluid_ingredients(r, "light-oil-cracking");
        }),
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("light-oil-cracking".into()),
        },
    );
    assert!(
        said.contains("light-oil-cracking runs in chemical-plant"),
        "{said}"
    );
    assert!(said.contains("nowhere for it to land"), "{said}");
}

// ---------------------------------------------------------------------------
// Every way a named recipe can be wrong
// ---------------------------------------------------------------------------

/// A recipe that exists and makes something else is refused **by name**, and
/// says what it does make.
#[test]
fn a_recipe_that_does_not_produce_the_product_is_refused_by_name() {
    let said = plan_error(
        live_state_with(&BOTS, true, |_| {}),
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("iron-gear-wheel".into()),
        },
    );
    assert!(
        said.contains("iron-gear-wheel does not produce petroleum-gas"),
        "{said}"
    );
    assert!(said.contains("it produces iron-gear-wheel"), "{said}");
}

/// **The machine reason, not the ambiguity one.** `casting-iron-gear-wheel` is
/// `metallurgy`, which no vanilla machine in this capture declares, so naming
/// it must produce the machine table's own words.
///
/// The control that makes it a claim about the *named* recipe: unqualified,
/// the same goal on the same world **plans**, because `iron-gear-wheel` is an
/// ordinary hand craft. So the refusal is caused by the qualifier and by
/// nothing else -- and, critically, the planner did **not** quietly answer the
/// qualified goal with the hand craft.
#[test]
fn a_named_recipe_with_no_machine_gives_the_machine_reason_not_the_ambiguity_one() {
    let goal = |via: Option<&str>| Goal::Have {
        item: "iron-gear-wheel".into(),
        count: 5,
        whose: Holder::Anyone,
        via: via.map(str::to_string),
    };
    let registry = registry_for(&BOTS);
    let state = live_state_with(&BOTS, true, |_| {});
    expand(&[goal(None)], &state, &registry, BOTS[0])
        .expect("the unqualified goal is an ordinary hand craft and plans");

    let said = plan_error(
        live_state_with(&BOTS, true, |_| {}),
        goal(Some("casting-iron-gear-wheel")),
    );
    assert!(said.contains("casting-iron-gear-wheel"), "{said}");
    assert!(said.contains("which does produce it"), "{said}");
    assert!(said.contains("category metallurgy"), "{said}");
    assert!(
        said.contains("no machine in this world model runs recipe category metallurgy"),
        "{said}"
    );
    assert!(
        !said.contains("nothing here can choose between them"),
        "naming a recipe has already answered which one: {said}"
    );
}

/// The same goal on the **undeclared** world, which is every dump this project
/// holds. The machine reason is still the one given, and it is the *other*
/// half of `MachineRefusal::NoMachine` -- "it did not say", not "nothing
/// crafts this".
///
/// Without this, "the refusal names the machine reason" would be equally
/// explained by a world that refuses everything the same way.
#[test]
fn on_a_world_that_never_said_the_machine_reason_says_it_did_not_say() {
    let said = plan_error(
        live_state_with(&BOTS, false, |_| {}),
        Goal::Have {
            item: "iron-gear-wheel".into(),
            count: 5,
            whose: Holder::Anyone,
            via: Some("casting-iron-gear-wheel".into()),
        },
    );
    assert!(said.contains("it did not say"), "{said}");
    assert!(said.contains("predates `crafting_categories`"), "{said}");
}

/// A `crafting` recipe the hand-craft methods **would** pick is honoured, and
/// the plan is byte-identical to the unqualified one.
///
/// This is the no-op case, and it is the one that proves the qualifier is a
/// qualifier rather than a second planner: same actions, same labels, same
/// order.
#[test]
fn a_named_recipe_that_is_the_planners_own_pick_changes_nothing() {
    let registry = registry_for(&BOTS);
    let plan = |via: Option<&str>| {
        let state = live_state_with(&BOTS, true, |_| {});
        let net = expand(
            &[Goal::Have {
                item: "iron-gear-wheel".into(),
                count: 5,
                whose: Holder::Anyone,
                via: via.map(str::to_string),
            }],
            &state,
            &registry,
            BOTS[0],
        )
        .expect("an ordinary hand craft plans either way");
        net.actions()
            .map(|a| format!("{:?} {}", a.id, a.label))
            .collect::<Vec<_>>()
    };
    let unqualified = plan(None);
    assert!(
        unqualified.len() > 1,
        "an empty plan would make this test vacuous: {unqualified:?}"
    );
    assert_eq!(unqualified, plan(Some("iron-gear-wheel")));
}

/// A `crafting` recipe the hand-craft methods **cannot** be told about is
/// refused rather than silently answered with the one they would run.
///
/// # This case does not exist in vanilla, and that was measured
///
/// Over the live 2.1.17 capture, no product made by a `crafting` or `smelting`
/// recipe lacks a same-named one, and none has two -- so `method::util::
/// recipe_for`'s recipe-name-keyed lookup is accidentally exact there (the
/// `products` module doc states the same measurement). The case is a modded
/// one, so the world is edited to create it: a second `crafting` recipe named
/// `alt-gear` that also produces `iron-gear-wheel`.
#[test]
fn a_crafting_recipe_the_hand_methods_cannot_be_told_about_is_refused() {
    let said = plan_error(
        live_state_with(&BOTS, true, |recipes| {
            let gear = recipes
                .iter()
                .find(|r| r.name == "iron-gear-wheel")
                .expect("the capture has iron-gear-wheel")
                .clone();
            let mut alt = gear.clone();
            alt.name = "alt-gear".to_string();
            assert_eq!(alt.category, "crafting");
            assert!(
                alt.products.iter().any(|p| p.name == "iron-gear-wheel"),
                "the injected recipe must really produce the product"
            );
            recipes.push(alt);
        }),
        Goal::Have {
            item: "iron-gear-wheel".into(),
            count: 5,
            whose: Holder::Anyone,
            via: Some("alt-gear".into()),
        },
    );
    assert!(said.contains("alt-gear"), "{said}");
    assert!(
        said.contains("choose their recipe by product name"),
        "{said}"
    );
    assert!(
        said.contains("they would run iron-gear-wheel instead"),
        "the message names what would have happened silently: {said}"
    );
}

// ---------------------------------------------------------------------------
// It refuses before anything is planned
// ---------------------------------------------------------------------------

/// **The guard runs before any method is asked**, which is the whole reason it
/// is not a `Method::refusal`.
///
/// `have:iron-plate:200` is a goal several methods claim eagerly, and on this
/// world the roster already holds 200 -- so `AlreadySatisfied` would claim it
/// and hand back an empty plan without any refusal ever being consulted. With
/// a recipe named that cannot produce it, the goal is refused instead.
#[test]
fn a_goal_a_method_would_claim_immediately_is_still_refused_for_its_recipe() {
    let registry = registry_for(&BOTS);
    let state = live_state_with(&BOTS, true, |_| {});
    let net = expand(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: 200,
            whose: Holder::Anyone,
            via: None,
        }],
        &state,
        &registry,
        BOTS[0],
    )
    .expect("the roster already holds 200 iron plates, so this is satisfied");
    assert_eq!(
        net.actions().count(),
        0,
        "the control: unqualified, this goal is claimed and needs no work"
    );

    let said = plan_error(
        live_state_with(&BOTS, true, |_| {}),
        Goal::Have {
            item: "iron-plate".into(),
            count: 200,
            whose: Holder::Anyone,
            via: Some("iron-gear-wheel".into()),
        },
    );
    assert!(
        said.contains("iron-gear-wheel does not produce iron-plate"),
        "a satisfied goal is still refused when its recipe cannot answer it: {said}"
    );
}
