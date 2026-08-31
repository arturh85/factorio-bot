//! Why the planner may ignore `FactorioProduct::probability`.
//!
//! A recipe product can be produced only some of the time. A planner that
//! ignores that computes `runs = ceil(need / amount)` when the honest sum is
//! `runs = ceil(need / (amount * probability))`, and under-plans by
//! `1 / probability` — 10x on `iron-bacteria`, 50x on `yumako-processing`.
//!
//! `output_per_craft` does not divide, and that is safe for exactly one
//! reason: **no recipe the planner can reach carries a probability below 1**.
//! Reachability is decided by category — `Craft` admits `CRAFTING_CATEGORY`
//! and `Smelt` admits `SMELTING_CATEGORY`, and nothing else in the game data
//! is a recipe this planner will ever expand.
//!
//! That is a claim about live game data, so it is checked against live game
//! data: `crates/core/tests/live-2.1.17-world-snapshot.json` is a byte-for-byte
//! RCON reply from a real Factorio 2.1.17 game (see that directory's README).
//! Both halves of the claim are asserted, because either half failing alone is
//! a different bug:
//!
//! * the *premise* — the capture really does carry probabilities below 1. If
//!   this fails, the fixture or `mods/BotBridge/types.lua::serialize_product`
//!   has stopped forwarding the field, and the second assertion below would
//!   then be passing vacuously.
//! * the *claim* — none of them is in a category the planner admits. If this
//!   fails, someone widened a gate, and `output_per_craft` must start dividing
//!   before that widening is safe.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::types::FactorioRecipe;
use factorio_bot_planner::method::util::{CRAFTING_CATEGORY, SMELTING_CATEGORY};
use std::collections::BTreeMap;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

/// The categories a `Goal::Have` can decompose through. Read from the planner
/// rather than restated, so widening a gate is what breaks the test below.
const REACHABLE: [&str; 2] = [CRAFTING_CATEGORY, SMELTING_CATEGORY];

fn recipes() -> Vec<FactorioRecipe> {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture");
    snapshot.recipes
}

/// Every `(recipe, product)` whose product is not certain, as
/// `FactorioProduct::probability` normalises it across game versions.
fn uncertain(recipes: &[FactorioRecipe]) -> Vec<(String, String, String, f64)> {
    let mut out = Vec::new();
    for recipe in recipes {
        for product in &recipe.products {
            let probability = product.probability.raw();
            if probability < 1.0 {
                out.push((
                    recipe.category.clone(),
                    recipe.name.clone(),
                    product.name.clone(),
                    probability,
                ));
            }
        }
    }
    out
}

/// The premise. "Absent everywhere" would make the real assertion vacuous, so
/// it is a failure in its own right rather than a quiet pass.
#[test]
fn the_live_capture_really_does_carry_probabilities_below_one() {
    let recipes = recipes();
    assert_eq!(recipes.len(), 662, "the captured recipe table, unedited");

    let uncertain = uncertain(&recipes);
    assert!(
        !uncertain.is_empty(),
        "a capture with no probability at all means the mod stopped sending \
         `independent_probability`/`shared_probability`, not that the game has none"
    );

    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    for (category, ..) in &uncertain {
        *by_category.entry(category.as_str()).or_default() += 1;
    }
    assert_eq!(
        by_category,
        BTreeMap::from([
            ("centrifuging", 2),
            ("crushing", 15),
            ("organic", 4),
            ("recycling", 102),
        ]),
        "the 2.1.17 recipe table's uncertain products, by category"
    );
}

/// The claim `output_per_craft` rests on.
#[test]
fn no_recipe_the_planner_can_reach_has_an_uncertain_product() {
    let reachable: Vec<_> = uncertain(&recipes())
        .into_iter()
        .filter(|(category, ..)| REACHABLE.contains(&category.as_str()))
        .collect();

    assert!(
        reachable.is_empty(),
        "the planner now reaches recipes whose products are not certain, so \
         `output_per_craft` must divide by `probability` before this gate \
         widening is safe -- `runs = need.div_ceil(amount * probability)`, at \
         the caller, and a probability of 0 must not divide at all: {reachable:?}"
    );
}

/// The four `organic` entries by name, so the note in
/// `crates/core/src/types.rs` can be checked against the data rather than
/// against memory. These are the recipes one gate-widening away.
#[test]
fn the_organic_recipes_are_the_nearest_uncertain_ones() {
    let organic: Vec<(String, String, f64)> = uncertain(&recipes())
        .into_iter()
        .filter(|(category, ..)| category == "organic")
        .map(|(_, recipe, product, probability)| (recipe, product, probability))
        .collect();

    assert_eq!(
        organic,
        vec![
            ("yumako-processing".into(), "yumako-seed".into(), 0.02),
            ("jellynut-processing".into(), "jellynut-seed".into(), 0.02),
            ("iron-bacteria".into(), "iron-bacteria".into(), 0.1),
            ("copper-bacteria".into(), "copper-bacteria".into(), 0.1),
        ],
    );
}
