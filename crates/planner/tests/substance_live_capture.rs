//! The fluid classifier, checked against a world nobody on this branch wrote.
//!
//! `crates/planner/src/substance.rs` decides whether a prototype name is an
//! item a character can hold or a fluid that no character can hold. It is an
//! *instrument*, and this repository's own rule for a new instrument is that
//! its first reading is evidence about the instrument, not about the system —
//! so it is validated against something that already knows the answer.
//!
//! That something is `crates/core/tests/live-2.1.17-world-snapshot.json`, a
//! byte-for-byte RCON reply from a real Factorio 2.1.17 game, checked in by an
//! earlier task for `tests/recipe_probability.rs`. **I did not write it and
//! could not have written it to agree with me**, which is the entire point:
//! the unit tests in `substance.rs` were written by the same hand as the code,
//! and a fixture written beside its code agrees with its code
//! (`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`).
//!
//! Both halves are asserted, the way `recipe_probability.rs` does it, because
//! either failing alone means something different:
//!
//! * the **premise** — the capture really does declare fluids, and really does
//!   carry an item table. If this fails, the fixture or
//!   `mods/BotBridge/types.lua` has stopped forwarding a field, and every
//!   assertion below would be passing vacuously.
//! * the **claim** — the classifier finds exactly the fluids the capture
//!   declares, calls every item prototype an item, and reports no disagreement
//!   between its two independent sources of evidence.
//!
//! The last of those is the one that licenses the classifier's tier order.
//! `SubstanceTable::from_parts` lets the item table overrule a recipe's
//! declaration, which is only safe because the two never disagree — measured,
//! not assumed, and re-measured every time this test runs.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_planner::substance::{FLUID_TYPE, Substance, SubstanceTable};
use std::collections::BTreeSet;

const WORLD_SNAPSHOT: &str = include_str!("../../core/tests/live-2.1.17-world-snapshot.json");

fn snapshot() -> WorldSnapshot {
    serde_json::from_str(WORLD_SNAPSHOT).expect("the live capture parses")
}

/// Every name the capture's recipes declare as a fluid, read straight off the
/// JSON fields rather than through the classifier — so this is an oracle and
/// not a restatement of the thing under test.
fn declared_fluids(snapshot: &WorldSnapshot) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for recipe in &snapshot.recipes {
        if let Some(ingredients) = recipe.ingredients.as_ref() {
            for i in ingredients {
                if i.ingredient_type == FLUID_TYPE {
                    out.insert(i.name.clone());
                }
            }
        }
        for p in &recipe.products {
            if p.product_type == FLUID_TYPE {
                out.insert(p.name.clone());
            }
        }
    }
    out
}

fn table_for(snapshot: &WorldSnapshot) -> SubstanceTable {
    SubstanceTable::from_parts(
        snapshot.recipes.iter(),
        snapshot
            .item_prototypes
            .iter()
            .map(|proto| proto.name.as_str()),
    )
}

/// The premise. Without this, everything below could pass on an empty capture.
#[test]
fn the_capture_declares_fluids_and_items() {
    let snapshot = snapshot();
    assert_eq!(snapshot.recipes.len(), 662, "the capture's recipe count");
    assert_eq!(
        snapshot.item_prototypes.len(),
        342,
        "the capture's item prototype count"
    );
    let fluids = declared_fluids(&snapshot);
    assert_eq!(
        fluids.len(),
        21,
        "the capture declares 21 distinct fluids: {fluids:?}"
    );
    // Named, not just counted: a count could survive the set changing wholesale.
    for expected in [
        "crude-oil",
        "petroleum-gas",
        "water",
        "sulfuric-acid",
        "heavy-oil",
        "light-oil",
        "lubricant",
        "steam",
    ] {
        assert!(fluids.contains(expected), "{expected} is declared a fluid");
    }
}

/// The claim, first half: the classifier finds exactly the declared fluids —
/// no more (it does not invent one) and no fewer (it does not miss one).
#[test]
fn the_classifier_finds_exactly_the_declared_fluids() {
    let snapshot = snapshot();
    let expected = declared_fluids(&snapshot);
    let table = table_for(&snapshot);
    let found: BTreeSet<String> = table.fluids().map(str::to_string).collect();
    assert_eq!(found, expected);
}

/// The claim, second half: every one of the 342 item prototypes is classified,
/// and every one of them is an item. A fluid appearing here would mean the
/// game puts fluids in the item table, which would invalidate evidence tier 1.
#[test]
fn every_item_prototype_is_classified_as_an_item() {
    let snapshot = snapshot();
    let table = table_for(&snapshot);
    let mut wrong: Vec<(String, Option<Substance>)> = Vec::new();
    for proto in &snapshot.item_prototypes {
        let said = table.of(&proto.name);
        if said != Some(Substance::Item) {
            wrong.push((proto.name.clone(), said));
        }
    }
    assert!(wrong.is_empty(), "misclassified item prototypes: {wrong:?}");
}

/// The check that licenses the tier order: the two independent sources of
/// evidence — the recipes' declared types and the item prototype table —
/// contradict each other about nothing in a real game's data.
///
/// If this ever fails, the fix is to look at the named data, **not** to change
/// which tier wins.
#[test]
fn the_two_sources_of_evidence_never_disagree() {
    let snapshot = snapshot();
    let found = SubstanceTable::disagreements(
        snapshot.recipes.iter(),
        snapshot
            .item_prototypes
            .iter()
            .map(|proto| proto.name.as_str()),
    );
    assert_eq!(found, vec![], "the capture's two oracles disagree");
}

/// The four names the oil chain turns on, and one control. Spelled out because
/// a set comparison passing tells a reader nothing about whether the names
/// they care about are in it.
#[test]
fn the_battery_chain_is_classified_the_way_the_survey_says() {
    let snapshot = snapshot();
    let table = table_for(&snapshot);
    for fluid in ["crude-oil", "petroleum-gas", "water", "sulfuric-acid"] {
        assert_eq!(
            table.of(fluid),
            Some(Substance::Fluid),
            "{fluid} is a fluid"
        );
        assert!(table.is_fluid(fluid));
    }
    // The survey: everything on the battery chain except these two is a fluid.
    for item in ["sulfur", "battery", "iron-plate", "copper-plate"] {
        assert_eq!(table.of(item), Some(Substance::Item), "{item} is an item");
        assert!(!table.is_fluid(item));
    }
}
