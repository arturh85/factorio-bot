//! **Opening the recipe-category gate is not the wall; naming the machine is.**
//!
//! `produced:petroleum-gas:100` refuses today with
//!
//! ```text
//! 5 recipes produce petroleum-gas -- advanced-oil-processing (category
//! oil-processing), basic-oil-processing (category oil-processing),
//! coal-liquefaction (category oil-processing), empty-petroleum-gas-barrel
//! (category crafting-with-fluid), light-oil-cracking (category chemistry) --
//! and none is in a category this planner runs (crafting, smelting).
//! ```
//!
//! That sentence names a *symptom*. This file establishes, by experiment
//! rather than by reading, what actually stands behind it — by editing the
//! live capture's recipe table so that an oil recipe lands in a category the
//! planner already runs, and then asking the planner for it. Nothing in
//! `src/` changes; the world is what moves.
//!
//! # Why an experiment at all
//!
//! The two walls below are strictly *behind* the category gate, so no
//! measurement on an unmodified world can reach them: every path refuses at
//! the gate first. The only way to find out what the planner does with a
//! fluid ingredient, or with a fluid product, is to hand it one inside a
//! category it admits. That is what these tests do, and it is why they are
//! characterisation tests — they pin what the planner *does today*, so that
//! whoever opens the gate for real sees the next wall move rather than
//! discovering it live.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioItemPrototype, FactorioRecipe};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::{BotId, PlanState, expand, registry_for};
use std::sync::Arc;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

const BOTS: [BotId; 4] = [BotId(1), BotId(2), BotId(3), BotId(4)];

/// The category a character's hands run. Spelled here rather than imported so
/// that widening `util::CRAFTING_CATEGORY` does not silently retarget this
/// experiment at whatever the gate became.
const CRAFTING: &str = "crafting";
const SMELTING: &str = "smelting";

fn snapshot() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture parses")
}

fn recipe(snapshot: &WorldSnapshot, name: &str) -> FactorioRecipe {
    snapshot
        .recipes
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("the live capture carries {name}"))
        .clone()
}

/// The fixture terrain with the live capture's recipe and item tables
/// installed over it, after `edit` has had a chance to rewrite the recipes.
///
/// Same construction as `tests/fluid_have.rs`: the fixture's own recipe table
/// is hand-written and contains no fluid, so a claim made against it would be
/// a claim about a table written by the same hands as the code.
fn live_state_with<F>(bots: &[BotId], edit: F) -> PlanState
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
    PlanState::from_world(Arc::new(world), bots)
}

/// Move one recipe into `category`, and **assert it moved**. A rewrite that
/// silently matched nothing would leave the world unmodified and every claim
/// below would be measuring the ordinary category refusal instead.
fn recategorise(recipes: &mut [FactorioRecipe], name: &str, category: &str) {
    let mut hits = 0;
    for r in recipes.iter_mut() {
        if r.name == name {
            assert_ne!(
                r.category, category,
                "{name} is already in {category}; this experiment would prove nothing"
            );
            r.category = category.to_string();
            hits += 1;
        }
    }
    assert_eq!(hits, 1, "exactly one recipe named {name} must be rewritten");
}

/// Enable a recipe outright, so that `recipe_gate` cannot be the thing that
/// refuses. Every oil recipe ships disabled.
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

// ---------------------------------------------------------------------------
// Premises, read off the capture rather than restated from the planner.
// ---------------------------------------------------------------------------

/// **`basic-oil-processing` is the simplest of the five, and it is a
/// single-in / single-out recipe.** The multi-output problem
/// (`advanced-oil-processing`'s three fluids) is not what blocks the first
/// rung, and this is the oracle that says so.
#[test]
fn basic_oil_processing_is_one_fluid_in_and_one_fluid_out() {
    let snapshot = snapshot();
    let basic = recipe(&snapshot, "basic-oil-processing");
    assert_eq!(basic.category, "oil-processing");
    let ingredients = basic
        .ingredients
        .as_ref()
        .expect("the capture states its ingredients");
    assert_eq!(
        ingredients
            .iter()
            .map(|i| (i.name.as_str(), i.ingredient_type.as_str(), i.amount))
            .collect::<Vec<_>>(),
        vec![("crude-oil", "fluid", 100)],
    );
    assert_eq!(
        basic
            .products
            .iter()
            .map(|p| (p.name.as_str(), p.product_type.as_str(), p.amount))
            .collect::<Vec<_>>(),
        vec![("petroleum-gas", "fluid", 45)],
    );

    // The control that stops the assertion above from being a claim about
    // every oil recipe: the *advanced* one really does have three products,
    // so "one product" is a fact about this recipe and not about the parser.
    let advanced = recipe(&snapshot, "advanced-oil-processing");
    assert_eq!(advanced.products.len(), 3);
}

/// **Nothing on the wire says which machine runs which recipe category, and
/// `entity_type` cannot stand in for it.**
///
/// This is the wall the category gate is really made of. Widening the gate to
/// `oil-processing` means a method that stands a machine up, and a method has
/// to name one. `smelting` and `crafting` are the two categories whose machine
/// this planner names by hand and gets away with — a `stone-furnace` literal
/// and the character's own hands. For `oil-processing` the honest name is
/// `oil-refinery`, and there is no field in the world model that yields it:
/// the refinery, the chemical plant, the centrifuge and all three assembling
/// machines are one and the same `entity_type`.
///
/// `LuaEntityPrototype::crafting_categories` is the field the game has and the
/// bridge does not send. See the branch note.
#[test]
fn the_world_model_cannot_name_the_machine_for_a_category() {
    let snapshot = snapshot();
    let by_name = |n: &str| {
        snapshot
            .entity_prototypes
            .iter()
            .find(|p| p.name == n)
            .unwrap_or_else(|| panic!("the live capture carries {n}"))
            .clone()
    };
    let refinery = by_name("oil-refinery");
    let chemical = by_name("chemical-plant");
    let assembler = by_name("assembling-machine-1");
    let furnace = by_name("stone-furnace");

    // All three crafting machines are indistinguishable by the only
    // categorical field the wire carries.
    assert_eq!(refinery.entity_type, assembler.entity_type);
    assert_eq!(chemical.entity_type, assembler.entity_type);

    // The control, which is what makes the equality above a finding rather
    // than a claim that `entity_type` is constant: a furnace is a different
    // type, which is exactly why `smelting` was affordable to hard-code and
    // `oil-processing` is not.
    assert_ne!(furnace.entity_type, assembler.entity_type);

    // And every one of them declares a crafting speed, so "it crafts" is
    // knowable and "what it crafts" is not.
    for p in [&refinery, &chemical, &assembler, &furnace] {
        assert!(
            p.crafting_speed.is_some(),
            "{} declares a crafting speed",
            p.name
        );
    }

    // **The tripwire.** Read against the raw JSON rather than the struct,
    // because a struct cannot be asked about a field it does not have: serde
    // drops unknown keys silently, so a capture that *did* carry crafting
    // categories would deserialise identically and this file would keep
    // passing while its premise was gone.
    //
    // The day `LuaEntityPrototype::crafting_categories` crosses the bridge and
    // the capture is regenerated, this goes red — which is the correct
    // outcome, and the message says what to do about it.
    let raw: serde_json::Value =
        serde_json::from_str(WORLD_SNAPSHOT).expect("the capture parses as JSON");
    let offending: Vec<String> = raw["entity_prototypes"]
        .as_array()
        .expect("entity_prototypes is an array")
        .iter()
        .flat_map(|p| p.as_object().into_iter().flat_map(|o| o.keys()))
        .filter(|k| k.contains("crafting_categor"))
        .map(|k| k.to_string())
        .collect();
    assert!(
        offending.is_empty(),
        "the capture now carries {offending:?} -- the wall this file describes is DOWN. \
         A method for `oil-processing` can name its machine from the world instead of \
         from a table, and `products::Categories::planner_runs` can stop being hand-kept."
    );

    // The control that stops the emptiness above being vacuous: the same scan
    // for a key the capture definitely does carry finds it. A zero from a scan
    // that never ran looks exactly like a zero from a scan that did.
    let known: Vec<String> = raw["entity_prototypes"]
        .as_array()
        .expect("entity_prototypes is an array")
        .iter()
        .flat_map(|p| p.as_object().into_iter().flat_map(|o| o.keys()))
        .filter(|k| k.contains("crafting_speed"))
        .map(|k| k.to_string())
        .collect();
    assert_eq!(
        known.len(),
        18,
        "the scan itself works: 18 prototypes in this capture declare a crafting speed"
    );
}

// ---------------------------------------------------------------------------
// The experiments. Each opens the gate in the world and reports the next wall.
// ---------------------------------------------------------------------------

/// **Wall two: a fluid *ingredient* is asked of a bot as if it were an item.**
///
/// `plastic-bar` is coal (item) plus petroleum-gas (fluid) — the simplest
/// recipe in the game with a mixed bill. Moved into `crafting`, the hand-craft
/// method admits it, and the expansion asks a character for 20 petroleum-gas:
/// `method::util::ingredients_of` maps `(name, amount)` and discards the
/// declared type, so the fluid becomes an ordinary `HasItem` at a bot, and
/// `PlanState::available` sums character inventories and answers 0 forever.
///
/// So the request is not refused for being a fluid — it recurses into
/// *obtaining* the fluid, and the fluid `Have` guard catches it there. The
/// wall is real and it is at the bill, not at the gate.
///
/// `substance::split_bill` is the prepared seam for this and has no caller.
#[test]
fn a_fluid_ingredient_recurses_into_obtaining_the_fluid() {
    let state = live_state_with(&BOTS, |recipes| {
        recategorise(recipes, "plastic-bar", CRAFTING);
        enable(recipes, "plastic-bar");
    });
    let goal = Goal::Have {
        item: "plastic-bar".into(),
        count: 2,
        whose: Holder::Anyone,
    };
    let err = expand(&[goal], &state, &registry_for(&BOTS), BotId(1))
        .expect_err("a recipe whose bill contains a fluid cannot be planned");
    let text = format!("{err}");
    // **The exact sentence is the finding.** `20` is `plastic-bar`'s own
    // petroleum-gas amount, straight out of the recipe's bill — so the
    // expansion really did descend through the hand craft and ask a character
    // for the fluid. Nothing shorter proves that: a refusal merely *naming*
    // petroleum-gas is equally explained by the category gate still firing.
    assert!(
        text.contains("have 20 petroleum-gas"),
        "the refusal must quote the recipe's own ingredient amount, which is what shows the \
         bill was walked: {text}"
    );
    // And it is the *fluid* refusal, not the category one. The category
    // refusal would have named `plastic-bar` as the thing with no runnable
    // producer; this names the fluid inside its bill.
    assert!(
        !text.contains("plastic-bar"),
        "plastic-bar itself must no longer be the thing refused -- that is the gate this \
         experiment opened, and if it still fires nothing was measured: {text}"
    );
    // 20, not 5. `SplitAcrossBots` divides a four-bot `Have` by four, and the
    // fluid guard runs before it — so this number is also the assertion that
    // no share was taken of something no bot can hold.
    assert!(
        !text.contains("have 5 petroleum-gas"),
        "and the fluid must not have been divided into per-bot shares: {text}"
    );
}

/// The control for the test above, on the **same** world construction: with
/// the category rewritten and the fluid ingredient removed, the recipe plans.
///
/// Without this, the refusal above is equally explained by "recategorising a
/// recipe breaks it" or "this world plans nothing", and a test that only
/// asserts a failure cannot tell those apart.
#[test]
fn the_same_recipe_without_its_fluid_plans_fine() {
    let state = live_state_with(&BOTS, |recipes| {
        recategorise(recipes, "plastic-bar", CRAFTING);
        enable(recipes, "plastic-bar");
        let bar = recipes
            .iter_mut()
            .find(|r| r.name == "plastic-bar")
            .expect("present");
        let ingredients = bar.ingredients.as_mut().expect("stated");
        let before = ingredients.len();
        ingredients.retain(|i| i.ingredient_type != "fluid");
        assert_eq!(
            (before, ingredients.len()),
            (2, 1),
            "exactly the petroleum-gas must have been dropped, leaving the coal"
        );
    });
    let goal = Goal::Have {
        item: "plastic-bar".into(),
        count: 2,
        whose: Holder::Anyone,
    };
    let net = expand(&[goal], &state, &registry_for(&BOTS), BotId(1))
        .expect("coal-only plastic bars are an ordinary hand craft");
    assert!(
        !net.is_empty(),
        "and the plan has actions in it -- a non-empty network from the same call that \
         produced a plan is what stops this control being vacuous"
    );
}

/// **Wall three, and the trap in it: opening the category makes the *message*
/// worse, because `NoProducer` steps aside the moment a runnable recipe
/// exists.**
///
/// `products::NoProducer` speaks only when every producer is in a category the
/// planner cannot run — that is its tier 2, and it is why
/// `produced:petroleum-gas:100` gets a five-recipe diagnosis today. Move one
/// producer into `smelting` and the diagnosis evaporates: `NoProducer` sees a
/// runnable recipe, declines, no other method claims the goal either (a
/// furnace has no fluidbox and this planner models none), and the driver falls
/// through to its own unactionable sentence.
///
/// ```text
/// no method can satisfy goal: produce 45 petroleum-gas
/// ```
///
/// That is the exact shape `products.rs`'s module doc calls "the defect this
/// exists for" — a silent `None` dressed up as an answer. **Whoever opens the
/// category gate for real will hit this message first**, and it says nothing
/// about the fluid, the furnace, or the fluidbox that is actually missing.
/// Pinned here rather than fixed: today no vanilla product reaches it
/// (`solid-fuel`, `plastic-bar`, `concrete` and `uranium-235` all get the good
/// tier-2 message on an unmodified world), so a fourth tier would be code
/// written for a world that does not exist yet.
#[test]
fn opening_the_category_silences_the_diagnosis_that_named_the_blocker() {
    let goal = || Goal::Produced {
        item: "petroleum-gas".into(),
        count: 45,
        whose: Holder::Anyone,
        unlocks: None,
    };

    // The control first, on the *unmodified* tables: the good message.
    let before = live_state_with(&BOTS, |_| {});
    let diagnosed = format!(
        "{}",
        expand(&[goal()], &before, &registry_for(&BOTS), BotId(1))
            .expect_err("petroleum-gas has no runnable producer on a vanilla world")
    );
    assert!(
        diagnosed.contains("basic-oil-processing") && diagnosed.contains("oil-processing"),
        "unmodified, the refusal names the producers and their category: {diagnosed}"
    );

    // Now open the gate for exactly one of them.
    let after = live_state_with(&BOTS, |recipes| {
        recategorise(recipes, "basic-oil-processing", SMELTING);
        enable(recipes, "basic-oil-processing");
    });
    let bare = format!(
        "{}",
        expand(&[goal()], &after, &registry_for(&BOTS), BotId(1))
            .expect_err("a fluid product still has nowhere to land")
    );
    assert!(
        bare.contains("no method can satisfy goal"),
        "with the category runnable the driver's generic refusal is what a reader gets: {bare}"
    );
    assert!(
        !bare.contains("basic-oil-processing"),
        "and the recipe that would produce it is no longer named at all -- this is the \
         regression in diagnosis, stated as an assertion so it cannot be lost: {bare}"
    );
}
