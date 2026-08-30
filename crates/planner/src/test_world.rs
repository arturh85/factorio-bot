//! Worlds for the planner's own unit tests.
//!
//! `factorio_bot_core::test_utils::fixture_world` carries no forces at all, and
//! technologies live on forces, so nothing about research can be tested against
//! it as it stands. It is also **not ours to change**: `tests/red_science.rs`
//! and `tests/scheduling.rs` pin makespans to its exact contents, and adding an
//! entity, a recipe or a prototype to it would move those numbers for reasons
//! that have nothing to do with research. So this builds *on top of* a fresh
//! `fixture_world` instead — same entities, same recipes, same ore — with one
//! force bolted on afterwards. Every existing fixture is untouched.
//!
//! The technology table below is written out here rather than captured from a
//! real save, for the same reason the recipe fixtures are pinned: a test that
//! asserts a research costs 20 science packs has to be able to point at where
//! the 20 comes from. The shapes are the game's (see `FactorioTechnology`), and
//! `automation` carries the real game's numbers; the rest are chosen to reach
//! specific branches and are commented with which.

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::FactorioForce;

/// One force, `player`, with a small technology tree.
///
/// * `automation` — the real thing: no prerequisites, 10 units of one
///   automation science pack each, `research_unit_energy` 600 (= `time = 10`
///   seconds as the runtime API reports it). The root of every other entry.
/// * `logistics` — one prerequisite, and a *different* unit count from
///   `automation`, so a test that reads the wrong technology's cost fails.
/// * `military` — prerequisite `logistics`, so its prerequisite chain is two
///   deep, and an ingredient `amount` of 2, so `research_ingredients`'
///   multiply is exercised by something other than 1.
/// * `steel-processing` — already `researched`, and a `null` prerequisite list
///   rather than an empty one, which is the other half of
///   `prerequisites: Option<Vec<String>>`.
/// * `mixed-research` — **not a real technology.** Two producible ingredient
///   types are needed to reach the convergence branch, and the recipe fixture
///   carries exactly one science pack, so this asks for a science pack and an
///   iron plate. Nothing else uses it.
/// * `loop-a` / `loop-b` — prerequisites of each other. Game data is a DAG;
///   this pair is here to prove that a cycle in it terminates as an error
///   rather than hanging.
const FIXTURE_FORCE_JSON: &str = r#"
{
  "name": "player",
  "force_id": 1,
  "current_research": null,
  "research_progress": null,
  "technologies": {
    "automation": {
      "name": "automation",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": [],
      "research_unit_ingredients": [
        { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
      ],
      "research_unit_count": 10,
      "research_unit_energy": 600.0,
      "order": "a-a",
      "level": 1,
      "valid": true
    },
    "logistics": {
      "name": "logistics",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": ["automation"],
      "research_unit_ingredients": [
        { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
      ],
      "research_unit_count": 20,
      "research_unit_energy": 900.0,
      "order": "a-b",
      "level": 1,
      "valid": true
    },
    "military": {
      "name": "military",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": ["logistics"],
      "research_unit_ingredients": [
        { "name": "automation-science-pack", "ingredient_type": "item", "amount": 2 }
      ],
      "research_unit_count": 5,
      "research_unit_energy": 900.0,
      "order": "a-c",
      "level": 1,
      "valid": true
    },
    "steel-processing": {
      "name": "steel-processing",
      "enabled": true,
      "upgrade": false,
      "researched": true,
      "prerequisites": null,
      "research_unit_ingredients": [
        { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
      ],
      "research_unit_count": 50,
      "research_unit_energy": 300.0,
      "order": "a-d",
      "level": 1,
      "valid": true
    },
    "mixed-research": {
      "name": "mixed-research",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": [],
      "research_unit_ingredients": [
        { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 },
        { "name": "iron-plate", "ingredient_type": "item", "amount": 3 }
      ],
      "research_unit_count": 2,
      "research_unit_energy": 60.0,
      "order": "a-e",
      "level": 1,
      "valid": true
    },
    "loop-a": {
      "name": "loop-a",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": ["loop-b"],
      "research_unit_ingredients": [],
      "research_unit_count": 1,
      "research_unit_energy": 60.0,
      "order": "z-a",
      "level": 1,
      "valid": true
    },
    "loop-b": {
      "name": "loop-b",
      "enabled": true,
      "upgrade": false,
      "researched": false,
      "prerequisites": ["loop-a"],
      "research_unit_ingredients": [],
      "research_unit_count": 1,
      "research_unit_energy": 60.0,
      "order": "z-b",
      "level": 1,
      "valid": true
    }
  }
}
"#;

/// `fixture_world()` plus the force above. Nothing else differs.
pub(crate) fn world_with_technologies() -> FactorioWorld {
    let world = fixture_world();
    let force: FactorioForce =
        serde_json::from_str(FIXTURE_FORCE_JSON).expect("the fixture force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;

    /// The premise the whole fixture rests on. If `fixture_world` ever grows a
    /// force of its own, `world_with_technologies` would be layering on top of
    /// one rather than introducing the only one, and the research tests would
    /// silently start reading someone else's technology table.
    #[test]
    fn the_shared_fixture_world_still_carries_no_forces() {
        assert_eq!(fixture_world().forces.len(), 0);
    }

    /// Guards the JSON against a rename or a type change in
    /// `FactorioTechnology`: `serde_json::from_str` would fail, and every
    /// research test would fail with a parse message instead of saying that
    /// the fixture no longer matches the type.
    #[test]
    fn the_fixture_force_parses_and_carries_the_expected_technologies() {
        let world = world_with_technologies();
        let force = world.forces.get("player").expect("the player force");
        let names: Vec<&str> = force.technologies.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            vec![
                "automation",
                "logistics",
                "loop-a",
                "loop-b",
                "military",
                "mixed-research",
                "steel-processing",
            ]
        );
        let automation = force
            .technologies
            .get("automation")
            .expect("automation is defined");
        assert_eq!(automation.research_unit_count, 10);
        assert_eq!(automation.research_unit_ingredients.len(), 1);
        assert_eq!(
            automation.research_unit_ingredients[0].name,
            "automation-science-pack"
        );
        assert_eq!(automation.research_unit_ingredients[0].amount, 1);
        assert!(!automation.researched);
        assert!(
            force
                .technologies
                .get("steel-processing")
                .expect("steel-processing is defined")
                .researched
        );
    }
}
