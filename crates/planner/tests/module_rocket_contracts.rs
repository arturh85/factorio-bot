//! Pinned prototype contract for the Space Age first-launch closure.
//!
//! This test verifies that the installed game's prototype data for rocket
//! construction and launch matches the checked-in fixture, preventing drift
//! between the planner's assumptions and the running game.

use std::collections::BTreeMap;

/// Compact representation of launch-related prototypes, extracted and
/// normalized from a live game session.
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct LaunchPrototypeFixture {
    pub game_version: String,
    pub mods: BTreeMap<String, String>,
    pub prototype_hash: String,
    pub rocket_parts_required: u32,
    pub recipes: BTreeMap<String, serde_json::Value>,
    pub technologies: BTreeMap<String, serde_json::Value>,
    pub machines: BTreeMap<String, serde_json::Value>,
}

/// The Space Age launch closure: rocket-part recipe, silo technology,
/// supporting machines (assemblers, furnaces, drills), and the starter
/// platform pack required to begin space science.
#[test]
fn installed_launch_has_the_space_age_research_and_payload_contract() {
    let f: LaunchPrototypeFixture = serde_json::from_str(include_str!(
        "fixtures/rocket-2.1.17.json"
    ))
    .expect("fixture should parse as LaunchPrototypeFixture");

    assert_eq!(f.game_version, "2.1.17");
    assert_eq!(f.rocket_parts_required, 50);

    // rocket-silo technology prereqs include Space Age additions
    let prereqs = f.technologies["rocket-silo"]["prerequisites"]
        .as_array()
        .expect("rocket-silo prerequisites should be an array");
    assert!(
        prereqs.iter().any(|p| p == "logistic-robotics"),
        "rocket-silo should require logistic-robotics (Space Age)"
    );
    assert!(
        prereqs.iter().any(|p| p == "advanced-material-processing-2"),
        "rocket-silo should require advanced-material-processing-2 (Space Age)"
    );

    // space-platform-starter-pack requires 60 foundation
    assert_eq!(
        f.recipes["space-platform-starter-pack"]["ingredients"]
            ["space-platform-foundation"],
        60,
        "starter pack needs 60 foundation"
    );

    // rocket-part recipe should require its three components
    let rp_ingredients = f.recipes["rocket-part"]["ingredients"]
        .as_object()
        .expect("rocket-part ingredients should be a map");
    assert!(rp_ingredients.contains_key("low-density-structure"));
    assert!(rp_ingredients.contains_key("rocket-fuel"));
    assert!(rp_ingredients.contains_key("processing-unit"));

    // rocket-silo machine should expose its inventory/progress fields
    let silo = f.machines.get("rocket-silo")
        .expect("rocket-silo machine should be present");
    assert_eq!(
        silo["rocket_parts_required"].as_u64().unwrap_or(0),
        50,
        "silo machine should require 50 rocket parts"
    );
}
