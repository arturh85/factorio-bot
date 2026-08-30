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

/// A force named `{name}` in which `automation` is researched iff `{done}`.
///
/// Deliberately minimal and deliberately *disagreeing* between instances: the
/// point of a multi-force world here is that the forces answer differently, so
/// a planner that consulted the wrong one gives a different answer rather than
/// the same one by luck.
fn one_technology_force(name: &str, done: bool) -> FactorioForce {
    let json = format!(
        r#"
        {{
          "name": "{name}",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {{
            "automation": {{
              "name": "automation",
              "enabled": true,
              "upgrade": false,
              "researched": {done},
              "prerequisites": [],
              "research_unit_ingredients": [
                {{ "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }}
              ],
              "research_unit_count": {count},
              "research_unit_energy": 600.0,
              "order": "a-a",
              "level": 1,
              "valid": true
            }}
          }}
        }}
        "#,
        name = name,
        done = done,
        // The unit count differs with `researched` too, so a test can tell
        // which force a *cost* was read from and not only which was asked
        // about researched-ness.
        count = if done { 99 } else { 10 },
    );
    serde_json::from_str(&json).expect("the generated force must parse")
}

/// `fixture_world()` with several forces, each defining `automation`
/// differently.
///
/// `forces` is a `DashMap`, so this is also the fixture that says whether the
/// planner's answer depends on the hash seed. The names are passed in so a
/// caller can put the researched one first or last alphabetically.
pub(crate) fn world_with_forces(forces: &[(&str, bool)]) -> FactorioWorld {
    let world = fixture_world();
    for (name, done) in forces {
        world
            .update_force(one_technology_force(name, *done))
            .expect("update_force cannot fail for a well-formed force");
    }
    world
}

/// `fixture_world()` with a straight prerequisite chain `chain-0` requires
/// `chain-1` requires ... `chain-{len-1}`.
///
/// Ingredient-free on purpose: the depth an expansion reaches is the sum of the
/// prerequisite recursion and the recipe nesting under each technology's
/// science packs, and this fixture isolates the first by removing the second.
pub(crate) fn world_with_prerequisite_chain(len: u32) -> FactorioWorld {
    let mut technologies = Vec::new();
    for level in 0..len {
        let prerequisites = if level + 1 < len {
            format!(r#"["chain-{}"]"#, level + 1)
        } else {
            "[]".to_string()
        };
        technologies.push(format!(
            r#"
            "chain-{level}": {{
              "name": "chain-{level}",
              "enabled": true,
              "upgrade": false,
              "researched": false,
              "prerequisites": {prerequisites},
              "research_unit_ingredients": [],
              "research_unit_count": 1,
              "research_unit_energy": 60.0,
              "order": "c-{level}",
              "level": 1,
              "valid": true
            }}"#
        ));
    }
    let json = format!(
        r#"
        {{
          "name": "player",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {{ {} }}
        }}
        "#,
        technologies.join(",")
    );
    let world = fixture_world();
    let force: FactorioForce =
        serde_json::from_str(&json).expect("the generated chain force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

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
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    /// The premise the whole fixture rests on. If `fixture_world` ever grows a
    /// force of its own, `world_with_technologies` would be layering on top of
    /// one rather than introducing the only one, and the research tests would
    /// silently start reading someone else's technology table.
    #[test]
    fn the_shared_fixture_world_still_carries_no_forces() {
        assert_eq!(fixture_world().forces.len(), 0);
    }

    /// The seam D1 named: `is_researched` used to answer over *any* force
    /// while costs were read from the alphabetically first one. `alpha` has
    /// not researched automation and `zeta` has, so the two questions have
    /// opposite answers and only a planner that asks one force gets a
    /// consistent pair.
    ///
    /// Written against the *acting* force rather than against either literal
    /// answer: `alpha` sorts first, so the acting force has not researched it,
    /// and the cost read must be `alpha`'s 10 rather than `zeta`'s 99.
    #[test]
    fn a_world_of_disagreeing_forces_is_answered_by_one_of_them() {
        let world = Arc::new(world_with_forces(&[("alpha", false), ("zeta", true)]));
        let state = PlanState::from_world(world, &[BotId(1)]);

        assert!(
            !state.is_researched("automation"),
            "the acting force has not researched it, whatever the other force says"
        );
        assert_eq!(
            state
                .technology("automation")
                .expect("the acting force defines it")
                .research_unit_count,
            10,
            "the cost must come from the same force that answered is_researched"
        );
    }

    /// The other direction, so neither result can be a constant: with the
    /// researched force sorting *first*, both answers flip together.
    #[test]
    fn the_acting_force_decides_both_answers_together() {
        let world = Arc::new(world_with_forces(&[("alpha", true), ("zeta", false)]));
        let state = PlanState::from_world(world, &[BotId(1)]);

        assert!(state.is_researched("automation"));
        assert_eq!(
            state
                .technology("automation")
                .expect("the acting force defines it")
                .research_unit_count,
            99,
        );
    }

    /// `forces` is a `DashMap`, whose iteration order moves with the hash
    /// seed. Choosing the acting force by `min` collapses that order by
    /// construction, so the answer must be identical across freshly built
    /// worlds rather than merely usually the same.
    ///
    /// Six forces so that "first by iteration" and "first by name" are very
    /// unlikely to coincide, and fifty fresh worlds so a seed-dependent
    /// implementation has room to show it. This is the same fixture the two
    /// tests above use, run for a different property.
    #[test]
    fn the_acting_force_does_not_depend_on_map_order() {
        let names = [
            ("mu", true),
            ("zeta", true),
            ("alpha", false),
            ("kappa", true),
            ("beta", true),
            ("omega", true),
        ];
        for _ in 0..50 {
            let world = Arc::new(world_with_forces(&names));
            let state = PlanState::from_world(world, &[BotId(1)]);
            assert!(!state.is_researched("automation"));
            assert_eq!(
                state
                    .technology("automation")
                    .expect("the acting force defines it")
                    .research_unit_count,
                10,
            );
        }
    }

    /// A world with no forces acts for none, and knows no technology. This is
    /// what the shared `fixture_world` is, so it is the state every
    /// non-research test in the crate plans against.
    #[test]
    fn a_world_without_forces_acts_for_no_force() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        assert!(state.technology("automation").is_none());
        assert!(!state.is_researched("automation"));
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
