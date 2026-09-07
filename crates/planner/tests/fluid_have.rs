//! **A fluid is never in an inventory, so `Goal::Have` cannot state one.**
//!
//! The defect this pins, in the exact words the CLI printed on
//! `workspace/scripts/map-31337-explored.json` before the guard existed:
//!
//! ```text
//! $ factorio-bot plan --goal have:petroleum-gas:100 --bots 1,2,3,4
//! Error: the goal did not expand: no method can satisfy goal:
//!        have 25 petroleum-gas (a share sized for bot 1)
//! ```
//!
//! Read the parenthesis. The planner divided 100 petroleum-gas into four
//! shares of 25 and asked bot 1 to obtain its share. That is right for iron
//! plates and nonsense for a fluid: no character inventory holds a fluid in
//! any amount, so the share is an artefact of the refusal path rather than a
//! fact about the request, and every number in that sentence describes
//! something that cannot exist.
//!
//! # Why the world here is the live capture and not the fixture
//!
//! `factorio_bot_core::test_utils::fixture_world` has no fluid in it at all —
//! its recipe table is hand-written, and a classifier run over it would be
//! reading a table written by the same hands as the code. So this file builds
//! the fixture and then **overwrites its recipe and item-prototype tables with
//! `crates/core/tests/live-2.1.17-world-snapshot.json`**, a real RCON reply
//! from a Factorio 2.1.17 game checked in by an earlier task. `petroleum-gas`
//! is a fluid here because the game said so, not because a fixture did.
//!
//! # The premise this asserts before it asserts anything else
//!
//! A green test that refuses a fluid proves nothing unless the splitter would
//! otherwise have taken the goal — that is the "the property holds for another
//! reason" trap in
//! `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`. So
//! `the_splitter_claims_this_goal_which_is_why_the_guard_is_needed` asks the
//! production registry directly, through `MethodRegistry::find`, which method
//! would claim `Have { petroleum-gas, 100, Anyone }` at a top-level site. It
//! answers `split-across-bots`. The guard is what stops that from running, and
//! nothing about the fixture is.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioItemPrototype, FactorioRecipe};
use factorio_bot_planner::error::PlannerError;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::GoalSite;
use factorio_bot_planner::products::{Categories, ProductIndex, ProductRefusal};
use factorio_bot_planner::substance::{FLUID_TYPE, FluidRefusal, FluidSource, SubstanceTable};
use factorio_bot_planner::{BotId, PlanState, expand, registry_for};
use std::sync::Arc;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

/// The default roster, which is the roster the defect was found on: four bots
/// is what `--bots 1,2,3,4` asks for and what `SplitAcrossBots` divides by.
const BOTS: [BotId; 4] = [BotId(1), BotId(2), BotId(3), BotId(4)];

/// The fluid the CLI refused, and the count it was asked for.
const FLUID: &str = "petroleum-gas";
const COUNT: u32 = 100;

fn snapshot() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture parses")
}

/// The fixture world with the live capture's recipe and item-prototype tables
/// installed over the top.
///
/// The fixture's *terrain* is kept — ore fields, trees, rocks, water — because
/// the goal under test has to be refused on a world where ordinary goals still
/// plan. Only the two tables the substance classifier reads are replaced, and
/// they are replaced with the game's own.
fn live_state(bots: &[BotId]) -> PlanState {
    let snapshot = snapshot();
    let world = fixture_world();
    let recipes: Vec<FactorioRecipe> = snapshot.recipes.clone();
    let items: Vec<FactorioItemPrototype> = snapshot.item_prototypes.clone();
    assert!(
        recipes.len() > 600 && items.len() > 300,
        "the live capture carries a whole game's tables, not a stub: {} recipes, {} items",
        recipes.len(),
        items.len()
    );
    world
        .update_recipes(recipes)
        .expect("installing the live recipe table");
    world
        .update_item_prototypes(items)
        .expect("installing the live item-prototype table");
    PlanState::from_world(Arc::new(world), bots)
}

fn have(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
        via: None,
    }
}

/// The site a goal the caller handed to `expand` sits at.
fn top_level() -> GoalSite {
    GoalSite {
        top_level: true,
        in_chain: false,
        converging: false,
    }
}

// ---------------------------------------------------------------------------
// Premises. Each of these is about the fixture, not about the guard, and each
// would make the claims below vacuous if it stopped holding.
// ---------------------------------------------------------------------------

/// The capture really does call `petroleum-gas` a fluid — read straight off
/// the JSON fields, so this is an oracle rather than a restatement of
/// `SubstanceTable`, which is the thing under test.
#[test]
fn the_capture_itself_declares_petroleum_gas_a_fluid() {
    let snapshot = snapshot();
    let declared_fluid = snapshot.recipes.iter().any(|recipe| {
        recipe
            .products
            .iter()
            .any(|p| p.name == FLUID && p.product_type == FLUID_TYPE)
    });
    assert!(
        declared_fluid,
        "some recipe in the capture must declare {FLUID} a fluid product, or this whole file \
         is testing a name the game never called a fluid"
    );
    assert!(
        !snapshot.item_prototypes.iter().any(|p| p.name == FLUID),
        "{FLUID} must not be in the item-prototype table -- if it were, the game itself would \
         be calling it an item and the guard would be wrong"
    );
    // The control: the classifier's other answer has to be reachable on the
    // same tables, or `is_fluid` could be a constant `true`.
    let table = SubstanceTable::from_parts(
        snapshot.recipes.iter(),
        snapshot.item_prototypes.iter().map(|p| p.name.as_str()),
    );
    assert!(
        table.is_fluid(FLUID),
        "the classifier agrees with the capture"
    );
    assert!(
        !table.is_fluid("iron-plate"),
        "and it does not call every name a fluid"
    );
}

/// **The premise that makes every claim below non-vacuous.**
///
/// Absent the guard, `SplitAcrossBots` is the method that claims this goal —
/// which is exactly how the refusal came to mention a share sized for bot 1.
/// Asked through the production registry, at the site the CLI's goal sits at.
///
/// If this ever stops answering `split-across-bots`, the guard may still be
/// correct but this file no longer proves it is load-bearing.
#[test]
fn the_splitter_claims_this_goal_which_is_why_the_guard_is_needed() {
    let state = live_state(&BOTS);
    let registry = registry_for(&BOTS);
    let claimed = registry
        .find(&have(FLUID, COUNT), &state, top_level())
        .map(|method| method.name());
    assert_eq!(
        claimed,
        Some("split-across-bots"),
        "the roster splitter must be the method that would take a fluid `Have` on this world, \
         or the guard in `expand_goal_body` is not what removes the share from the refusal"
    );
}

/// Ordinary goals still plan on this world, so a refusal below is about the
/// fluid and not about a world nothing can be planned on.
#[test]
fn an_item_goal_still_plans_on_this_world() {
    let state = live_state(&BOTS);
    let net = expand(
        &[have("iron-plate", 8)],
        &state,
        &registry_for(&BOTS),
        BotId(1),
    )
    .expect("a four-bot roster plans eight iron plates on this world");
    assert!(!net.is_empty(), "and the plan has actions in it");
}

// ---------------------------------------------------------------------------
// The claims.
// ---------------------------------------------------------------------------

/// The defect, pinned: the refusal is about the fluid and never about a share.
#[test]
fn a_fluid_have_is_refused_by_shape_and_never_split_into_shares() {
    let state = live_state(&BOTS);
    let err = expand(
        &[have(FLUID, COUNT)],
        &state,
        &registry_for(&BOTS),
        BotId(1),
    )
    .expect_err("a character cannot hold a fluid, so this cannot plan");

    let PlannerError::FluidNotItem(FluidRefusal::NotCarryable {
        fluid,
        count,
        produced_by,
    }) = &err
    else {
        panic!("expected a FluidNotItem refusal, got: {err:?}");
    };

    assert_eq!(
        fluid, FLUID,
        "the refusal names the fluid that was asked for"
    );
    assert_eq!(
        *count, COUNT,
        "and the count the CALLER asked for -- 100, not a quarter of it"
    );

    let message = err.to_string();
    assert!(
        !message.contains("share sized for"),
        "the refusal must never mention a bot's share of a fluid; got: {message}"
    );
    assert!(
        !message.contains("no method can satisfy"),
        "and it must not fall back to the driver's unactionable text; got: {message}"
    );

    // The refusal says where the fluid would have to come from, which is the
    // part a reader can act on. Read off the capture, so this is the game's
    // list and not one written here.
    let FluidSource::Recipes {
        recipes,
        categories,
    } = produced_by
    else {
        panic!("the live capture produces {FLUID} by recipe; got {produced_by:?}");
    };
    assert!(
        recipes.contains(&"basic-oil-processing".to_string()),
        "the producers must include basic-oil-processing; got {recipes:?}"
    );
    assert!(
        categories.contains(&"oil-processing".to_string()),
        "and name the category no machine in this planner runs; got {categories:?}"
    );
}

/// A one-bot roster never splits anything, and the refusal is the same.
///
/// This separates "the guard fires" from "the splitter was avoided": with one
/// bot there is no share to remove, so a passing assertion here is about the
/// goal's shape alone.
#[test]
fn a_fluid_have_is_refused_for_a_single_bot_too() {
    let bots = [BotId(1)];
    let state = live_state(&bots);
    let err = expand(
        &[have(FLUID, COUNT)],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect_err("one bot cannot hold a fluid either");
    assert!(
        matches!(err, PlannerError::FluidNotItem(_)),
        "expected FluidNotItem, got: {err:?}"
    );
}

/// The guard discriminates. An item asked for in the same shape, on the same
/// world, with the same roster, is not refused as a fluid.
///
/// Without this a guard that refused *every* `Have` would pass the test above.
#[test]
fn an_item_have_is_never_refused_as_a_fluid() {
    let state = live_state(&BOTS);
    for item in ["iron-plate", "iron-ore", "coal", "transport-belt"] {
        match expand(&[have(item, COUNT)], &state, &registry_for(&BOTS), BotId(1)) {
            Ok(_) => {}
            Err(err) => assert!(
                !matches!(err, PlannerError::FluidNotItem(_)),
                "{item} is an item and must never be refused as a fluid; got: {err:?}"
            ),
        }
    }
}

/// A `Goal::Produced` about a fluid is **not** caught by the guard, and gets
/// the product refusal instead.
///
/// Deliberate: "cause 100 petroleum-gas to come into existence" is a
/// meaningful request an oil refinery answers, and only `Have` makes a claim
/// about an inventory. What answers it is `products::NoProducer`, whose first
/// production caller is `registry_for`.
#[test]
fn a_fluid_produced_goal_names_the_recipes_instead_of_the_missing_method() {
    let state = live_state(&BOTS);
    let goal = Goal::Produced {
        item: FLUID.into(),
        count: COUNT,
        whose: Holder::Anyone,
        unlocks: None,
        via: None,
    };
    let err = expand(&[goal], &state, &registry_for(&BOTS), BotId(1))
        .expect_err("no machine in this planner runs oil-processing");
    let message = err.to_string();
    assert!(
        matches!(err, PlannerError::ProductNotMakeable(_)),
        "expected ProductNotMakeable, got: {err:?}"
    );
    assert!(
        message.contains("oil-processing"),
        "the refusal must name the category no machine here runs; got: {message}"
    );
}

// ---------------------------------------------------------------------------
// The second face of the same defect: the product lookup.
// ---------------------------------------------------------------------------

/// `recipe_for`'s keyed-by-recipe-name lookup answers nothing for a fluid, and
/// the product index answers. Both halves asserted, because a claim that the
/// index "now answers" is worth nothing without the demonstration that the
/// other lookup does not.
#[test]
fn the_product_index_answers_for_a_fluid_where_a_recipe_name_lookup_cannot() {
    let snapshot = snapshot();
    // The lookup `method::util::recipe_for` performs, spelled out here so this
    // test does not depend on that function staying private or public.
    assert!(
        !snapshot.recipes.iter().any(|r| r.name == FLUID),
        "there is no recipe called {FLUID}, which is why a recipe-name lookup answers None"
    );

    let index = ProductIndex::from_parts(
        snapshot.recipes.iter(),
        snapshot.item_prototypes.iter().map(|p| p.name.as_str()),
    );
    let producers = index.recipes_producing(FLUID);
    assert!(
        producers.len() >= 2,
        "the index finds every producer, not one: got {:?}",
        producers.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // And it refuses by name rather than silently, naming the wall.
    let refusal = index
        .sole_recipe_producing(FLUID, &Categories::planner_brings())
        .expect_err("no crafting or smelting recipe produces a fluid");
    let ProductRefusal::NoRunnableCategory { candidates, .. } = &refusal else {
        panic!("expected NoRunnableCategory, got {refusal:?}");
    };
    assert!(
        candidates.iter().any(|c| c.category == "oil-processing"),
        "the refusal names the category; got {candidates:?}"
    );
}
