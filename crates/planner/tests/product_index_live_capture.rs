//! The product index, checked against a world nobody on this branch wrote.
//!
//! `crates/planner/src/products.rs` answers "which recipes produce this
//! product". Its unit tests were written by the same hand as the code, and a
//! fixture written beside its code agrees with its code
//! (`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`), so
//! the claims that matter are re-derived here from
//! `crates/core/tests/live-2.1.17-world-snapshot.json` — a byte-for-byte RCON
//! reply from a real Factorio 2.1.17 game, checked in by an earlier task for
//! `tests/recipe_probability.rs`. **I did not write it and could not have
//! written it to agree with me.**
//!
//! Both halves are asserted, the way `recipe_probability.rs` and
//! `substance_live_capture.rs` do it, because either failing alone means
//! something different:
//!
//! * the **premise** — the capture really is many-to-many, really does have
//!   662 recipes, and really does contain 62 products no recipe is named
//!   after. If this fails, the capture or the mod's serialisation changed and
//!   every claim below would be passing vacuously.
//! * the **claim** — the index reproduces an oracle built straight off the
//!   JSON fields, and the one-to-one property that makes the *existing*
//!   `method::util::recipe_for` accidentally correct really does hold over
//!   `crafting` + `smelting` and nowhere else.
//!
//! That last assertion is the load-bearing one. It is the licence for leaving
//! every existing caller alone, and the alarm that fires the day somebody
//! widens a category gate without moving to the index.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::types::FactorioRecipe;
use factorio_bot_planner::products::{Categories, ProductIndex, ProductRefusal};
use factorio_bot_planner::substance::Substance;
use std::collections::{BTreeMap, BTreeSet};

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

fn snapshot() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture parses")
}

fn index(snapshot: &WorldSnapshot) -> ProductIndex {
    ProductIndex::from_parts(
        snapshot.recipes.iter(),
        snapshot.item_prototypes.iter().map(|i| i.name.as_str()),
    )
}

/// product name → the recipes that list it, read straight off the JSON fields
/// rather than through the index. This is the oracle; the index is the thing
/// under test, and the two are computed by different code.
fn oracle(recipes: &[FactorioRecipe]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for recipe in recipes {
        for product in &recipe.products {
            out.entry(product.name.clone())
                .or_default()
                .insert(recipe.name.clone());
        }
    }
    out
}

/// The premise. Every number here was measured off the capture before the
/// module was written and is restated in `products.rs`'s own doc; if the
/// capture changes, this fails loudly instead of quietly weakening the
/// assertions below.
#[test]
fn the_capture_really_is_many_to_many() {
    let snapshot = snapshot();
    assert_eq!(snapshot.recipes.len(), 662, "recipes in the capture");
    assert_eq!(snapshot.item_prototypes.len(), 342, "item prototypes");

    let by_product = oracle(&snapshot.recipes);
    assert_eq!(by_product.len(), 330, "distinct products");

    let multi_product = snapshot
        .recipes
        .iter()
        .filter(|r| r.products.len() > 1)
        .count();
    assert_eq!(multi_product, 230, "recipes with more than one product");

    let multi_maker = by_product.values().filter(|set| set.len() > 1).count();
    assert_eq!(multi_maker, 155, "products made by more than one recipe");

    let recipe_names: BTreeSet<&str> = snapshot.recipes.iter().map(|r| r.name.as_str()).collect();
    let orphans: Vec<&str> = by_product
        .keys()
        .map(String::as_str)
        .filter(|product| !recipe_names.contains(product))
        .collect();
    assert_eq!(
        orphans.len(),
        62,
        "products with no same-named recipe, which `recipe_for` answers None for: {orphans:?}"
    );
    for expected in [
        "petroleum-gas",
        "crude-oil",
        "heavy-oil",
        "light-oil",
        "steam",
        "water",
        "iron-ore",
        "coal",
        "stone",
        "wood",
        "solid-fuel",
        "uranium-235",
    ] {
        assert!(
            orphans.contains(&expected),
            "{expected} should be one of the 62 products `recipe_for` cannot find"
        );
    }
}

/// The claim, in full: the index and an independently computed oracle agree on
/// every product and every producer.
#[test]
fn the_index_reproduces_the_oracle_exactly() {
    let snapshot = snapshot();
    let index = index(&snapshot);
    let oracle = oracle(&snapshot.recipes);

    assert_eq!(
        index.products().map(str::to_string).collect::<Vec<_>>(),
        oracle.keys().cloned().collect::<Vec<_>>(),
        "the same products, in the same order"
    );
    for (product, makers) in &oracle {
        let indexed: BTreeSet<String> = index
            .recipes_producing(product)
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(&indexed, makers, "producers of {product}");
    }
    assert_eq!(index.len(), snapshot.recipes.len(), "every recipe indexed");
}

/// **The load-bearing measurement.** Within the two categories this planner
/// runs, the product-to-recipe map is one-to-one, total and name-preserving —
/// which is the entire reason `method::util::recipe_for` has never produced a
/// wrong plan, and the reason introducing this index moves no baseline.
///
/// Widening `CRAFTING_CATEGORY` or `SMELTING_CATEGORY`, or running on a world
/// where `crafting-with-fluid`, `metallurgy` or `recycling` is reachable,
/// breaks all three properties at once. This test is where that is noticed.
#[test]
fn recipe_for_is_accidentally_exact_over_crafting_and_smelting_only() {
    let snapshot = snapshot();
    let index = index(&snapshot);
    let runs = Categories::planner_brings();

    let runnable: Vec<&FactorioRecipe> = snapshot
        .recipes
        .iter()
        .filter(|r| runs.admits(&r.category))
        .collect();
    let by_product = oracle(
        &runnable
            .iter()
            .map(|r| (*r).clone())
            .collect::<Vec<FactorioRecipe>>(),
    );
    assert_eq!(
        by_product.len(),
        194,
        "products of crafting+smelting recipes"
    );

    for (product, makers) in &by_product {
        assert_eq!(
            makers.len(),
            1,
            "{product} is made by more than one recipe this planner runs: {makers:?}"
        );
        assert!(
            makers.contains(product),
            "{product} is made by {makers:?}, none of which is named after it"
        );
        // And the index agrees, resolving to that one recipe with no refusal.
        let chosen = index
            .sole_recipe_producing(product, &runs)
            .unwrap_or_else(|e| panic!("{product}: {e}"));
        assert_eq!(&chosen.name, product);
    }
    // No runnable recipe has *more* than one product. One has none at all:
    // `recipe-unknown`, a `crafting` recipe the game ships as a placeholder,
    // found by this oracle and not by me. The index never lists it under any
    // product, which is right -- and `recipe_for("recipe-unknown")` would hand
    // it back, which is a second, smaller face of the same name-keyed defect.
    let productless: Vec<&str> = runnable
        .iter()
        .filter(|r| r.products.is_empty())
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(productless, vec!["recipe-unknown"]);
    for recipe in &runnable {
        assert!(
            recipe.products.len() <= 1,
            "{} has more than one product",
            recipe.name
        );
    }
}

/// The refusal a fluid goal gets, against the real capture: five producers,
/// named, with the categories that are the actual missing prerequisite.
#[test]
fn petroleum_gas_refuses_by_naming_all_five_of_its_recipes() {
    let snapshot = snapshot();
    let index = index(&snapshot);

    let err = index
        .sole_recipe_producing("petroleum-gas", &Categories::planner_brings())
        .expect_err("no crafting or smelting recipe makes petroleum-gas");
    let ProductRefusal::NoRunnableCategory {
        candidates,
        substance,
        ..
    } = &err
    else {
        panic!("expected tier 2, got {err:?}");
    };
    assert_eq!(
        candidates
            .iter()
            .map(|c| (c.recipe.as_str(), c.category.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("advanced-oil-processing", "oil-processing"),
            ("basic-oil-processing", "oil-processing"),
            ("coal-liquefaction", "oil-processing"),
            ("empty-petroleum-gas-barrel", "crafting-with-fluid"),
            ("light-oil-cracking", "chemistry"),
        ],
        "every producer in the capture, in name order"
    );
    assert_eq!(*substance, Some(Substance::Fluid));

    let text = err.to_string();
    assert!(text.contains("5 recipes produce petroleum-gas"), "{text}");
    assert!(text.contains("no character inventory can hold"), "{text}");
}

/// The multi-product case the old one-to-one table cannot hold: one recipe,
/// three fluid products, all three indexed to it with the amounts the game
/// data states (`workspace/server/data/base/prototypes/recipe.lua`: heavy-oil
/// 25, light-oil 45, petroleum-gas 55).
#[test]
fn advanced_oil_processing_is_indexed_under_all_three_of_its_products() {
    let snapshot = snapshot();
    let index = index(&snapshot);

    let recipe = index
        .recipe("advanced-oil-processing")
        .expect("the capture has it");
    assert_eq!(recipe.products.len(), 3, "the premise: three products");

    for (product, amount) in [("heavy-oil", 25), ("light-oil", 45), ("petroleum-gas", 55)] {
        assert!(
            index
                .recipes_producing(product)
                .iter()
                .any(|r| r.name == "advanced-oil-processing"),
            "{product} is not indexed to advanced-oil-processing"
        );
        let stated = recipe
            .products
            .iter()
            .find(|p| p.name == product)
            .map(|p| p.amount);
        assert_eq!(stated, Some(amount), "{product} per run");
    }
}

/// Tier 1 against the real capture: `iron-ore` is an item nothing makes, and
/// the refusal says to mine it rather than saying nothing.
#[test]
fn iron_ore_is_told_to_come_out_of_the_ground() {
    let snapshot = snapshot();
    let index = index(&snapshot);

    // The premise: some recipes *do* mention iron-ore as a product on this
    // capture, so this has to be asked with the planner's own categories
    // rather than `any`, and the tier below is 2 rather than 1. The list is
    // the oracle's, not mine -- I expected the two asteroid-crushing recipes
    // and the capture also has three `recycling` ones, which is exactly the
    // kind of correction an oracle is for.
    let crushers = index.recipes_producing("iron-ore");
    assert_eq!(
        crushers.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec![
            "advanced-metallic-asteroid-crushing",
            "concrete-recycling",
            "hazard-concrete-recycling",
            "iron-ore-recycling",
            "metallic-asteroid-crushing",
        ],
        "space-age crushing and recycling -- on Nauvis at t=0, all unreachable"
    );

    let err = index
        .sole_recipe_producing("iron-ore", &Categories::planner_brings())
        .expect_err("no crafting or smelting recipe makes iron ore");
    assert!(
        matches!(err, ProductRefusal::NoRunnableCategory { .. }),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(text.contains("crushing"), "{text}");

    // And a name the capture has never heard of is tier 1 with no substance,
    // which is the honest "check the spelling" answer.
    let unknown = index
        .sole_recipe_producing("unobtainium", &Categories::any())
        .expect_err("nothing makes it");
    assert_eq!(
        unknown,
        ProductRefusal::NotProduced {
            product: "unobtainium".to_string(),
            substance: None,
        }
    );
}
