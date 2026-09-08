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

use crate::ids::BotId;
use crate::method::ExpansionCtx;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::add_to_rect;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioForce, Position, Rect};
use std::sync::Arc;

/// A world whose iron front can seat a whole roster, and its ore neighbours
/// with it.
///
/// **Why this exists, and why it no longer carries the argument it was built
/// for.** `fixture_world`'s iron patch is 121 tiles, and at the separation two
/// hand-mining characters need (3.99 tiles, from
/// `PlanState::mining_tile_separation`) that is **nine seats**. While a claim
/// was held for the length of an expansion and crowded every other bot out of
/// its neighbourhood, the un-converged four-bot unlock plan spent eight of
/// those nine — one per mining *action* — and the shared fixture could host no
/// multi-bot mining at all beyond what it already did. So convergence was
/// demonstrated on a front sized like a real one instead, and the shared
/// fixture was left exactly as it is, with the makespans `tests/red_science.rs`
/// and `tests/scheduling.rs` pin untouched.
///
/// A claim now names the serial timeline it sits on
/// (`crate::state::ClaimRunner`), and one bot's own claims never conflict, so
/// those eight actions cost **four** seats and the shared fixture hosts the
/// handover perfectly well: `the_unlock_subtree_spreads_on_the_shared_fixture`
/// is the headline now, and this front changes its makespan by 25 ticks.
///
/// Kept as a control. A real Factorio ore field is thousands of tiles and
/// hundreds of seats, and a fixture that only ever seats nine is a poor proxy
/// for one; this is also what would catch a seat model that had quietly
/// started depending on how much ore there is.
///
/// The extra ore is a separate block, clear of every existing patch, rather
/// than an enlargement of one: overlapping `spawn_ore` would put two resource
/// entities on one tile and the doubled amounts would be a second, silent
/// change to the fixture.
pub(crate) fn widen_ore_front(world: FactorioSurface) -> FactorioSurface {
    let mut entities = Vec::new();
    // Clear of the fixture's own iron (centred (-40, 40), 11 tiles across),
    // its copper and coal (y around 0) and its water (40, 40).
    factorio_bot_core::test_utils::spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(30., 24.), &Position::new(-40., 64.)),
        "iron-ore",
    );
    world
        .update_chunk_entities(entities)
        .expect("a fixture world accepts its own ore");
    world
}

/// `world` with a stand of real trees in it, at `positions`.
///
/// **`fixture_world`'s own hundred trees are useless for this.** They are
/// `FactorioEntity::new_tree`, which names every one of them `tree-42`, and
/// the prototype fixture has no `tree-42` -- so they carry no `mine_result`,
/// yield nothing, and `PlanState::minable_sources` steps over them. That is
/// exactly why every existing test is unaffected by chopping: the shared
/// fixture's forest is, to the planner, a hundred obstacles and no wood.
///
/// `tree-01` is a prototype the fixture really has (`mine_result {wood: 4}`,
/// `mining_time` 0.55), so these are trees the planner can read a bill off.
/// Added to the *base* world rather than through `PlanState::create_entity`:
/// `minables` lives in `EntityGraph`, and the overlay only ever hides entities
/// from it, never adds one.
pub(crate) fn with_trees(world: FactorioSurface, positions: &[Position]) -> FactorioSurface {
    let entities: Vec<FactorioEntity> = positions
        .iter()
        .map(|position| FactorioEntity {
            name: "tree-01".into(),
            entity_type: "tree".into(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(0.8, 0.8), position),
            ..Default::default()
        })
        .collect();
    world
        .update_chunk_entities(entities)
        .expect("a fixture world accepts trees");
    world
}

/// A `stone-wall` footprint at its real collision box (`0.578125` tiles).
fn stone_wall(position: &Position) -> FactorioEntity {
    FactorioEntity {
        name: "stone-wall".into(),
        entity_type: "wall".into(),
        position: position.clone(),
        bounding_box: add_to_rect(&Rect::from_wh(0.578125, 0.578125), position),
        ..Default::default()
    }
}

/// A `lab` at its real prototype collision box, `2.3984375` tiles
/// (`crates/core/tests/entity-prototype-fixtures.json`).
///
/// **A 3x3 machine, and the shape the old `connect` fixtures never had.** An
/// odd footprint covers three tiles per axis and therefore sits on a tile
/// *centre*, so a legal `lab` position is a half-integer. There is no
/// `FactorioEntity::new_lab`, so it is written out here; the box is the
/// game's, not a round number chosen to make a test pass.
fn lab(position: &Position) -> FactorioEntity {
    FactorioEntity {
        name: "lab".into(),
        entity_type: "lab".into(),
        position: position.clone(),
        bounding_box: add_to_rect(&Rect::from_wh(2.3984375, 2.3984375), position),
        ..Default::default()
    }
}

/// The shared setup behind every `method::connect` fixture: a `fixture_world`
/// with `entities` in its **base** entity graph, and an `ExpansionCtx` over
/// it bound to bot 1.
///
/// Base entities via `update_chunk_entities`, never `PlanState::create_entity`:
/// `connect_steps` reads obstacles from
/// `state.base().entity_graph.blocking_boxes_within`, which never sees the
/// plan overlay, so a machine placed through the overlay would be invisible to
/// the very check these fixtures exist to exercise.
fn connect_ctx(entities: Vec<FactorioEntity>) -> ExpansionCtx {
    let world = fixture_world();
    world
        .update_chunk_entities(entities)
        .expect("a fixture world accepts these entities");
    ExpansionCtx::new(PlanState::from_world(Arc::new(world), &[]), BotId(1))
}

/// **Two real Factorio machine shapes at legal Factorio positions**, which is
/// the whole point of this fixture and what its predecessor did not have.
///
/// * a `stone-furnace` from the **production constructor**
///   (`FactorioEntity::new_stone_furnace`, box 1.8) at `(5.0, 5.0)`. Two tiles
///   per axis is an *even* footprint, so the centre sits on a tile boundary —
///   an integer. `method::util::tile_alignment` states that rule and says
///   getting it wrong has already cost this project a day.
/// * a `lab` (box 2.3984375) at `(12.5, 5.5)`. Three tiles per axis is odd, so
///   its centre is a tile centre — a half-integer.
///
/// The old fixture put a furnace at `(0.5, 0.5)`, where a 2x2 entity cannot
/// legally sit in Factorio. At `(0.5, 0.5)` both boxes — 1.3984375 and 1.8 —
/// cover exactly one cell, making them indistinguishable in this fixture, so
/// the box could never have been the discriminator. The illegal *position*
/// alone was what let `connect_steps` pass while treating both machines as 1x1.
/// The fixture had been built to fit the code. (The production constructor uses
/// 1.8, which does not match the prototype fixture's 1.3984375.)
///
/// Nothing else is in the way: `fixture_world`'s rocks are at `(20, 20)` and
/// `(40, 30)`, its trees around `(-20, -20)`, and its ore patches west of
/// `x = -40`.
pub(crate) fn furnace_and_lab_on_open_ground() -> (ExpansionCtx, FactorioEntity, FactorioEntity) {
    let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
    let lab = lab(&Position::new(12.5, 5.5));
    let ctx = connect_ctx(vec![furnace.clone(), lab.clone()]);
    (ctx, furnace, lab)
}

/// [`furnace_and_lab_on_open_ground`], with a `stone-wall` column at
/// `x = 8.5` spanning `y` from -30 to 30 — past the 24-tile radius of the
/// `enclosure::window` centred on `(5.0, 5.0)` on either side, so every row
/// inside that window has its `x = 8.5` cell blocked and no surface route
/// can cross it. Until 2026-09-09 this was the walled-destination control
/// and `connect_steps` refused on it; now the one-wide wall is exactly what
/// an underground pair is for, and [`furnace_and_lab_behind_a_wide_wall`]
/// is the control that still refuses.
///
/// The column sits strictly between the two machines' own footprints (the
/// furnace ends at `x = 5.9`, the lab starts at `x = 11.3`), so it blocks the
/// route and nothing else — in particular neither machine's chosen perimeter
/// tiles, which are both on their north sides.
pub(crate) fn furnace_and_lab_behind_a_wall() -> (ExpansionCtx, FactorioEntity, FactorioEntity) {
    let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
    let lab = lab(&Position::new(12.5, 5.5));
    let mut entities = vec![furnace.clone(), lab.clone()];
    for y in -30..=30 {
        entities.push(stone_wall(&Position::new(8.5, f64::from(y) + 0.5)));
    }
    (connect_ctx(entities), furnace, lab)
}

/// [`furnace_and_lab_behind_a_wall`]'s wall, five columns wide: `x = 6.5`
/// through `10.5`, every row from -30 to 30. The cells between the two
/// machines are `x 6..=10` (the furnace ends at cell 5, the lab starts at
/// 11), so the wall fills them all, and crossing it needs a pair six apart
/// -- one more than the fixture's `underground-belt` prototype allows
/// (`max_underground_distance = 5`). The wide-wall control: `connect_steps`
/// must refuse **by span**, naming the number, not by a bare `NoRoute`.
pub(crate) fn furnace_and_lab_behind_a_wide_wall() -> (ExpansionCtx, FactorioEntity, FactorioEntity)
{
    let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
    let lab = lab(&Position::new(12.5, 5.5));
    let mut entities = vec![furnace.clone(), lab.clone()];
    for x in 6..=10 {
        for y in -30..=30 {
            entities.push(stone_wall(&Position::new(
                f64::from(x) + 0.5,
                f64::from(y) + 0.5,
            )));
        }
    }
    (connect_ctx(entities), furnace, lab)
}

/// [`furnace_and_lab_on_open_ground`] with one `tree-42` standing on the row
/// the belt would otherwise run straight along.
///
/// **The zero-half-box fixture.** The tree is at `(9.5, 3.05)`, so its
/// `0.8`-tile box spans `y 2.65..3.45` and covers **no tile centre at all**
/// (the neighbouring centres are at `y = 2.5` and `y = 3.5`). Rasterised with
/// the zero half-box `connect_steps` used to pass, it blocks nothing and the
/// belt is routed straight through it; rasterised with the belt's own
/// `0.4` half-box it blocks the two cells it really leaves no room on, and
/// the route detours. Trees and rocks sit at arbitrary sub-tile positions in
/// a real game, which is why this is the obstacle that exposes it and a
/// grid-aligned building is not.
pub(crate) fn furnace_and_lab_with_a_tree() -> (ExpansionCtx, FactorioEntity, FactorioEntity) {
    let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
    let lab = lab(&Position::new(12.5, 5.5));
    let entities = vec![
        furnace.clone(),
        lab.clone(),
        FactorioEntity::new_tree(&Position::new(9.5, 3.05)),
    ];
    (connect_ctx(entities), furnace, lab)
}

/// Two `stone-furnace`s close enough that the second's only unwalled
/// perimeter pair is the pair the first one already claimed.
///
/// The window origin is `(-19, -19)`. The first furnace at `(5.0, 5.0)`
/// covers cells `x 23..=24, y 23..=24`; its first free perimeter pair is
/// North at `x = 23`, so it claims the inserter cell `(23, 22)` = `(4.5, 3.5)`
/// and the belt cell `(23, 21)` = `(4.5, 2.5)`. The second furnace at
/// `(5.0, 1.0)` covers cells `x 23..=24, y 19..=20`, and its own South
/// perimeter at `x = 23` wants exactly those two cells back.
///
/// Seven `stone-wall`s close every other pair: its North side (`(4.5, -0.5)`
/// and `(5.5, -0.5)`), its East side (`(6.5, 0.5)`, `(6.5, 1.5)`), its West
/// side (`(3.5, 0.5)`, `(3.5, 1.5)`) and the other South candidate
/// (`(5.5, 2.5)`). Each wall's `0.578` box, grown by the belt half-box, still
/// covers exactly the one cell it is centred on, so none of them reaches the
/// first furnace's claimed pair.
///
/// So: with the two ends aware of each other, the second furnace's search
/// finds every candidate walled or claimed and `connect_steps` refuses.
/// Without that awareness it finds `(23, 21)` and `(23, 22)` "free" — they
/// are open ground, and only the first furnace's own derivation knows
/// otherwise — and emits a second `Place` for tiles the first furnace's belt
/// and inserter already took.
pub(crate) fn furnaces_sharing_a_perimeter() -> (ExpansionCtx, FactorioEntity, FactorioEntity) {
    let first = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
    let second = FactorioEntity::new_stone_furnace(&Position::new(5.0, 1.0), Direction::North);
    let mut entities = vec![first.clone(), second.clone()];
    for wall in [
        (4.5, -0.5),
        (5.5, -0.5),
        (6.5, 0.5),
        (6.5, 1.5),
        (3.5, 0.5),
        (3.5, 1.5),
        (5.5, 2.5),
    ] {
        entities.push(stone_wall(&Position::new(wall.0, wall.1)));
    }
    (connect_ctx(entities), first, second)
}

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

/// Put a small electric pole and a steam engine into `state`'s overlay, so a
/// research planned against it has somewhere powered to put a lab.
///
/// **Why the overlay and not the world.** `EntityGraph::add`
/// (`crates/core/src/graph/entity_graph.rs`) only inserts a whitelist of
/// entity types into its entity tree, and `electric-pole` and `generator` are
/// not on it: an entity added through `update_chunk_entities` would block
/// placements and still be unreadable by name, so
/// `PlanState::electric_supply_kw` would score it zero. Entities placed
/// *by a plan* go through `PlanState::create_entity` and are visible at once,
/// which is the path this imitates — and the path a future power-building
/// method will really take. Widening that whitelist is a one-line change in a
/// crate this work does not own; see
/// `docs/superpowers/notes/2026-09-02-research-needs-power.md`.
///
/// The geometry, all of it deliberate:
///
/// * The pole sits at `(10.5, 10.5)`, clear of the fixture's ore (all of it
///   west), its rocks (`(20, 20)` and `(40, 30)`), its trees (`(-20, -20)`)
///   and its water (`(40, 40)`), so nothing about smelting or mining moves.
/// * A small pole's supply area is 5x5, so it covers `[8, 13]` on both axes.
/// * The steam engine at `(12.5, 10.5)` is 2.5 x 4.7 tiles, so its box spans
///   `x [11.25, 13.75]` and overlaps that supply area — **which is the whole
///   test**: a generator the pole does not cover contributes nothing, and an
///   earlier draft of this fixture put the engine at `(14.5, 10.5)`, a quarter
///   of a tile outside, and read 0 kW. Coverage is not capacity in both
///   directions.
/// * The lab the search then sites at `(8.5, 8.5)` is 2.4 tiles across, so it
///   clears the engine by 1.5 tiles and the pole by 0.65 — nowhere near the
///   2-tile furnace spacing whose `0.1015625` of slack per side pressed the
///   character against a box in 18 of 18 walk stalls in run 30.
pub(crate) fn with_steam_power(state: &mut PlanState) {
    for (name, position) in [
        ("small-electric-pole", Position::new(10.5, 10.5)),
        ("steam-engine", Position::new(12.5, 10.5)),
    ] {
        state.create_entity(FactorioEntity {
            name: name.into(),
            position,
            ..Default::default()
        });
    }
}

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
/// caller can put the researched one first or last alphabetically — and, since
/// the acting force is looked up by name, so a caller can build the world run
/// 30 actually had: `enemy`, `neutral` and `player`, with only `player`
/// holding the technology.
pub(crate) fn world_with_forces(forces: &[(&str, bool)]) -> FactorioSurface {
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
pub(crate) fn world_with_prerequisite_chain(len: u32) -> FactorioSurface {
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

/// `fixture_world()` with `recipe` switched off and a force whose technologies
/// unlock it.
///
/// **This is the shape the shared fixture cannot express, and that is exactly
/// why it hid the defect this fixture exists for.** Every recipe in
/// `fixture_world` is `enabled: true`, so a planner that could only see enabled
/// recipes passed its whole suite while the live game — where
/// `automation-science-pack` is disabled until its technology is researched —
/// had no such recipe at all. A green run against an all-enabled fixture proves
/// nothing about locked recipes, so tests about them have to build one.
///
/// `unlockers` are technology names that will each carry `recipe` in their
/// `unlocked_recipes`; passing more than one is how the several-unlockers
/// tie-break is tested, and passing none is how the "disabled and nothing turns
/// it on" case is. Each is prerequisite-free and pack-free, so the fixture
/// isolates the unlock question from research cost and prerequisite depth.
pub(crate) fn world_with_locked_recipe(recipe: &str, unlockers: &[&str]) -> FactorioSurface {
    locked_recipe_world(recipe, unlockers, false)
}

/// The same fixture, with every unlocker already researched **in the world**.
///
/// The distinction from calling `PlanState::set_researched` on the fixture
/// above is the point: `set_researched` writes the plan's *overlay*, which
/// means "an action in this plan will do it", whereas this says the force
/// finished it before planning began. `recipe_gate` answers those two
/// differently on purpose (`PlannedResearch` versus `Open`), so a test that
/// wants the second cannot get there through the first.
pub(crate) fn world_with_researched_unlocker(recipe: &str, unlockers: &[&str]) -> FactorioSurface {
    locked_recipe_world(recipe, unlockers, true)
}

fn locked_recipe_world(recipe: &str, unlockers: &[&str], researched: bool) -> FactorioSurface {
    let world = fixture_world();

    let mut locked = world
        .globals
        .recipes
        .get(recipe)
        .unwrap_or_else(|| panic!("the shared fixture must define the {recipe} recipe"))
        .clone();
    assert!(
        locked.enabled,
        "{recipe} is already disabled in the shared fixture; this fixture would then be \
         asserting nothing"
    );
    locked.enabled = false;
    world
        .update_recipes(vec![locked])
        .expect("update_recipes cannot fail for a well-formed recipe");

    let technologies: Vec<String> = unlockers
        .iter()
        .map(|tech| {
            format!(
                r#"
                "{tech}": {{
                  "name": "{tech}",
                  "enabled": true,
                  "upgrade": false,
                  "researched": {researched},
                  "prerequisites": [],
                  "research_unit_ingredients": [],
                  "research_unit_count": 1,
                  "research_unit_energy": 60.0,
                  "order": "u-{tech}",
                  "level": 1,
                  "valid": true,
                  "unlocked_recipes": ["{recipe}"]
                }}"#
            )
        })
        .collect();
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
    let force: FactorioForce =
        serde_json::from_str(&json).expect("the generated unlock force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// A world carrying exactly one technology, `tech`, which is unlocked by a
/// `research_trigger` rather than by science packs.
///
/// `trigger_json` is the trigger object verbatim, so a test can state the
/// shape the game sends rather than a Rust value the parser has already
/// blessed. The pack fields are all zero/empty, which is what the live game
/// really reports for a trigger technology — that is the whole reason the
/// planner used to cost these at nothing.
///
/// `unlocks` names a recipe the technology turns on, which is *disabled* in the
/// resulting world. Passing the same item the trigger asks to craft builds the
/// self-unlocking cycle that shipped 2.1.17 really contains: `foundry` is
/// triggered by crafting a foundry and is the only technology unlocking the
/// foundry recipe.
pub(crate) fn world_with_trigger(
    tech: &str,
    trigger_json: &str,
    locked_recipe: Option<&str>,
    unlocked_by: Option<&str>,
) -> FactorioSurface {
    let world = fixture_world();

    if let Some(recipe) = locked_recipe {
        let mut locked = world
            .globals
            .recipes
            .get(recipe)
            .unwrap_or_else(|| panic!("the shared fixture must define the {recipe} recipe"))
            .clone();
        assert!(
            locked.enabled,
            "{recipe} is already disabled in the shared fixture; this fixture would then be \
             asserting nothing"
        );
        locked.enabled = false;
        world
            .update_recipes(vec![locked])
            .expect("update_recipes cannot fail for a well-formed recipe");
    }
    // Who unlocks the locked recipe: this technology itself (the self-unlocking
    // cycle) or a separate one, which is the ordinary case and the control for
    // it. Both have to be expressible, or a guard that refused every locked
    // trigger item would look correct.
    let unlocker = unlocked_by.unwrap_or(tech);
    let own_recipes = match locked_recipe {
        Some(recipe) if unlocker == tech => format!(r#"["{recipe}"]"#),
        _ => "[]".to_string(),
    };
    let other_technology = match (locked_recipe, unlocked_by) {
        (Some(recipe), Some(other)) => format!(
            r#",
            "{other}": {{
              "name": "{other}",
              "enabled": true,
              "upgrade": false,
              "researched": false,
              "prerequisites": [],
              "research_unit_ingredients": [],
              "research_unit_count": 1,
              "research_unit_energy": 60.0,
              "order": "u-{other}",
              "level": 1,
              "valid": true,
              "unlocked_recipes": ["{recipe}"]
            }}"#
        ),
        _ => String::new(),
    };

    let json = format!(
        r#"
        {{
          "name": "player",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {{
            "{tech}": {{
              "name": "{tech}",
              "enabled": true,
              "upgrade": false,
              "researched": false,
              "prerequisites": [],
              "research_unit_ingredients": [],
              "research_unit_count": 0,
              "research_unit_energy": 0.0,
              "order": "t-{tech}",
              "level": 1,
              "valid": true,
              "unlocked_recipes": {own_recipes},
              "research_trigger": {trigger_json}
            }}{other_technology}
          }}
        }}
        "#
    );
    let force: FactorioForce =
        serde_json::from_str(&json).expect("the generated trigger force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// How the pumpjack recipe stands in [`world_with_oil`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PumpjackRecipe {
    /// No recipe at all -- the shared fixture's own state.
    Absent,
    /// Present and disabled, unlocked by `oil-gathering`, which is or is not
    /// researched in the world.
    LockedBy { researched: bool },
}

/// What [`world_with_oil`] builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OilFixture {
    /// Twelve charted crude-oil wells, as a workspace resumed from a
    /// savepoint holds them (the provenance of `run-1788538389-09170`); or
    /// none, as a fresh map does.
    pub wells: bool,
    /// Whether the prototypes carry the fields the game decides mining by:
    /// crude oil's `resource_category` (`basic-fluid`), the pumpjack's
    /// `resource_categories` (`[basic-fluid]`) and the character's
    /// (`[basic-solid]`). The shared fixture is an older capture with none of
    /// them, which is its own case.
    pub categories: bool,
    pub pumpjack: PumpjackRecipe,
    /// Whether `oil-processing` lists `oil-gathering` as a prerequisite, as
    /// the shipped tree does. Off by default so a plan for it holds only the
    /// trigger's own subtree.
    pub prerequisite: bool,
}

/// The oil ladder's last rung: `oil-processing`, a `mine-entity` trigger
/// naming `crude-oil`, exactly as `data/base/prototypes/technology.lua`
/// writes it -- plus whatever of the world around it `fixture` asks for.
pub(crate) fn world_with_oil(fixture: OilFixture) -> FactorioSurface {
    use factorio_bot_core::types::{Direction, FactorioRecipe};

    let world = fixture_world();
    if fixture.wells {
        let wells: Vec<FactorioEntity> = (0..12)
            .map(|i| {
                FactorioEntity::new_resource(
                    &Position::new(20.5 + 4. * f64::from(i), 20.5),
                    Direction::North,
                    "crude-oil",
                )
            })
            .collect();
        world
            .update_chunk_entities(wells)
            .expect("a chunk of wells");
    }
    if fixture.categories {
        world
            .globals
            .entity_prototypes
            .get_mut("crude-oil")
            .expect("the fixture has a crude-oil prototype")
            .resource_category = Some("basic-fluid".into());
        world
            .globals
            .entity_prototypes
            .get_mut("pumpjack")
            .expect("the fixture has a pumpjack prototype")
            .resource_categories = Some(vec!["basic-fluid".into()]);
        world
            .globals
            .entity_prototypes
            .get_mut("character")
            .expect("the fixture has a character prototype")
            .resource_categories = Some(vec!["basic-solid".into()]);
    }
    let gathering = match fixture.pumpjack {
        PumpjackRecipe::Absent => String::new(),
        PumpjackRecipe::LockedBy { researched } => {
            let recipe: FactorioRecipe = serde_json::from_str(
                r#"{
                  "name": "pumpjack", "valid": true, "enabled": false, "category": "crafting",
                  "ingredients": [
                    { "name": "steel-plate", "ingredient_type": "item", "amount": 5 },
                    { "name": "iron-gear-wheel", "ingredient_type": "item", "amount": 10 },
                    { "name": "electronic-circuit", "ingredient_type": "item", "amount": 5 },
                    { "name": "pipe", "ingredient_type": "item", "amount": 10 }
                  ],
                  "products": [
                    { "name": "pumpjack", "product_type": "item", "amount": 1, "probability": 1.0 }
                  ],
                  "hidden": false, "energy": 5.0, "order": "b-b", "group": "production",
                  "subgroup": "extraction-machine"
                }"#,
            )
            .expect("the pumpjack recipe parses");
            world
                .update_recipes(vec![recipe])
                .expect("update_recipes cannot fail for a well-formed recipe");
            format!(
                r#",
                "oil-gathering": {{
                  "name": "oil-gathering",
                  "enabled": true,
                  "upgrade": false,
                  "researched": {researched},
                  "prerequisites": [],
                  "research_unit_ingredients": [],
                  "research_unit_count": 1,
                  "research_unit_energy": 60.0,
                  "order": "e-a",
                  "level": 1,
                  "valid": true,
                  "unlocked_recipes": ["pumpjack"]
                }}"#
            )
        }
    };
    let prerequisites = if fixture.prerequisite {
        r#"["oil-gathering"]"#
    } else {
        "[]"
    };
    let json = format!(
        r#"
        {{
          "name": "player",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {{
            "oil-processing": {{
              "name": "oil-processing",
              "enabled": true,
              "upgrade": false,
              "researched": false,
              "prerequisites": {prerequisites},
              "research_unit_ingredients": [],
              "research_unit_count": 1,
              "research_unit_energy": 0.0,
              "order": "e-b",
              "level": 1,
              "valid": true,
              "unlocked_recipes": ["oil-refinery", "chemical-plant", "basic-oil-processing"],
              "research_trigger": {{ "type": "mine-entity", "entities": ["crude-oil"] }}
            }}{gathering}
          }}
        }}
        "#
    );
    let force: FactorioForce = serde_json::from_str(&json).expect("the generated oil force parses");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// A `craft-item` trigger technology sitting as the **prerequisite** of an
/// ordinary pack-costed one — the shape the live game presents and no other
/// fixture here does.
///
/// `world_with_trigger` builds a trigger technology with no prerequisites, so
/// a plan for it contains exactly one subtree and the scheduler has nothing to
/// choose between. Vanilla 2.0 puts `steam-power` ("craft 50 iron plates")
/// *under* `automation`, so planning `automation` produces a trigger's
/// production subtree **and** a science-pack subtree competing for the same
/// roster. That competition is what exposed the defect: the pack bill is a
/// `Have { .. Share }` and welds into one chain, the trigger's production was
/// a `Produced { .. Share }` and welded into none, so the scheduler mined the
/// ore onto one bot and asked another to load the furnace.
///
/// The numbers are the real ones for both technologies.
pub(crate) fn world_with_trigger_prerequisite() -> FactorioSurface {
    let json = r#"
    {
      "name": "player",
      "force_id": 1,
      "current_research": null,
      "research_progress": null,
      "technologies": {
        "steam-power": {
          "name": "steam-power",
          "enabled": true,
          "upgrade": false,
          "researched": false,
          "prerequisites": [],
          "research_unit_ingredients": [],
          "research_unit_count": 0,
          "research_unit_energy": 0.0,
          "order": "a-0",
          "level": 1,
          "valid": true,
          "research_trigger": { "type": "craft-item", "item": "iron-plate", "count": 50 }
        },
        "automation": {
          "name": "automation",
          "enabled": true,
          "upgrade": false,
          "researched": false,
          "prerequisites": ["steam-power"],
          "research_unit_ingredients": [
            { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
          ],
          "research_unit_count": 10,
          "research_unit_energy": 600.0,
          "order": "a-a",
          "level": 1,
          "valid": true
        }
      }
    }
    "#;
    let world = fixture_world();
    let force: FactorioForce =
        serde_json::from_str(json).expect("the trigger-prerequisite force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// `fixture_world()` with one technology, `long-research`, costing `units`
/// units of one automation science pack each at `unit_ticks` per unit and
/// needing nothing first.
///
/// The shape of the real game's `logistic-science-pack` (75 × 300) without
/// its prerequisites, so a test about *how* a long research is planned --
/// dealt across the roster, in how many labs -- is not also a test about
/// `automation` and the trigger under it.
pub(crate) fn world_with_long_research(units: u64, unit_ticks: f64) -> FactorioSurface {
    let json = format!(
        r#"
        {{
          "name": "player",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {{
            "long-research": {{
              "name": "long-research",
              "enabled": true,
              "upgrade": false,
              "researched": false,
              "prerequisites": [],
              "research_unit_ingredients": [
                {{ "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }}
              ],
              "research_unit_count": {units},
              "research_unit_energy": {unit_ticks:?},
              "order": "l-a",
              "level": 1,
              "valid": true
            }}
          }}
        }}
        "#
    );
    let world = fixture_world();
    let force: FactorioForce =
        serde_json::from_str(&json).expect("the long-research force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// `fixture_world()` plus the force above. Nothing else differs.
pub(crate) fn world_with_technologies() -> FactorioSurface {
    let world = fixture_world();
    let force: FactorioForce =
        serde_json::from_str(FIXTURE_FORCE_JSON).expect("the fixture force must parse");
    world
        .update_force(force)
        .expect("update_force cannot fail for a well-formed force");
    world
}

/// [`world_with_technologies`] with the lake drained.
///
/// `fixture_world`'s only tiles are a 4x4 block of `water` centred on
/// (40, 40), which since `9ca7229a` is the terrain a plant is sited against
/// and since `fa8dabf3` is solid ground nothing may be built on. A world with
/// no tiles at all is therefore the *control* for every water question: it is
/// what a map with no lake in reach looks like, and it is also exactly what a
/// world attached from a snapshot looks like, since `attach_world` fetches no
/// tiles.
///
/// Built by copying rather than by removing: `update_chunk_tiles` is additive
/// and `EntityGraph` has no "forget the terrain" call, so draining a lake
/// after the fact is not a thing this can ask for.
///
/// **The ore does not come across either**, because resources live in
/// `resource_tree` and `EntityGraph` exposes no query that hands them back.
/// That is harmless for what this world is used for and worth stating anyway:
/// `Researched` chooses where the research will happen *before* it emits the
/// science-pack bill — deliberately, so that a research with no power refuses
/// without first planning the mining of packs nothing would consume — so the
/// water refusal is reached before an empty map could produce a different one.
/// A test using this world should assert the refusal **by variant**, so that
/// the missing ore cannot pass for the missing water.
pub(crate) fn world_with_technologies_and_no_water() -> FactorioSurface {
    let wet = world_with_technologies();
    let dry = FactorioSurface::new();
    dry.update_entity_prototypes(
        wet.globals
            .entity_prototypes
            .iter()
            .map(|e| e.value().clone())
            .collect(),
    )
    .expect("prototypes copy");
    dry.update_item_prototypes(
        wet.globals
            .item_prototypes
            .iter()
            .map(|e| e.value().clone())
            .collect(),
    )
    .expect("item prototypes copy");
    dry.update_recipes(
        wet.globals
            .recipes
            .iter()
            .map(|e| e.value().clone())
            .collect(),
    )
    .expect("recipes copy");
    for force in wet.globals.forces.iter() {
        dry.update_force(force.value().clone()).expect("force copy");
    }
    dry
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
        assert_eq!(fixture_world().globals.forces.len(), 0);
    }

    /// The seam D1 named: `is_researched` used to answer over *any* force
    /// while costs were read from the alphabetically first one. `player` has
    /// not researched automation and `zeta` has, so the two questions have
    /// opposite answers and only a planner that asks one force gets a
    /// consistent pair.
    ///
    /// Written against the *acting* force rather than against either literal
    /// answer: the acting force is `player`, which has not researched it, and
    /// the cost read must be `player`'s 10 rather than `zeta`'s 99.
    #[test]
    fn a_world_of_disagreeing_forces_is_answered_by_one_of_them() {
        let world = Arc::new(world_with_forces(&[("player", false), ("zeta", true)]));
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

    /// The other direction, so neither result can be a constant: flip which
    /// force has the technology and both answers flip together.
    #[test]
    fn the_acting_force_decides_both_answers_together() {
        let world = Arc::new(world_with_forces(&[("player", true), ("zeta", false)]));
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

    /// **The run-30 shape, and the reason the acting force is now named rather
    /// than sorted for.**
    ///
    /// `writeout_forces` (`mods/BotBridge/control.lua`) emits all of
    /// `game.forces`, so a world gains `enemy` and `neutral` at the first
    /// research completion of every run — tick 26,449 of run 30. `enemy` sorts
    /// before `player`, so the old `forces.keys().min()` selected a force that
    /// never researches anything, and all five of milestone 7's plans
    /// re-derived a technology the `player` force had finished 50,000 ticks
    /// earlier.
    ///
    /// The two other forces disagree with `player` on both questions, so a
    /// planner that consults either gives the opposite answer to both.
    #[test]
    fn the_other_forces_in_the_world_do_not_get_a_vote() {
        let world = Arc::new(world_with_forces(&[
            ("enemy", false),
            ("neutral", false),
            ("player", true),
        ]));
        let state = PlanState::from_world(world, &[BotId(1)]);

        assert!(
            state.is_researched("automation"),
            "the player force has it; enemy and neutral sorting first is not a vote"
        );
        assert_eq!(
            state
                .technology("automation")
                .expect("the acting force defines it")
                .research_unit_count,
            99,
            "and the cost comes from the same force"
        );
    }

    /// A world with forces but none of them `player` acts for none.
    ///
    /// The honest reading of a world this planner is not in: refusing to plan
    /// beats planning for somebody else, which is precisely what the sort did.
    #[test]
    fn a_world_without_the_player_force_acts_for_no_force() {
        let world = Arc::new(world_with_forces(&[("enemy", true), ("neutral", true)]));
        let state = PlanState::from_world(world, &[BotId(1)]);

        assert!(state.technology("automation").is_none());
        assert!(!state.is_researched("automation"));
    }

    /// `forces` is a `DashMap`, whose iteration order moves with the hash
    /// seed. Looking the acting force up by name collapses that order by
    /// construction, so the answer must be identical across freshly built
    /// worlds rather than merely usually the same.
    ///
    /// Six forces so that "first by iteration" and "the one we want" are very
    /// unlikely to coincide, and fifty fresh worlds so a seed-dependent
    /// implementation has room to show it. This is the same fixture the tests
    /// above use, run for a different property.
    #[test]
    fn the_acting_force_does_not_depend_on_map_order() {
        let names = [
            ("mu", true),
            ("zeta", true),
            ("player", false),
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
        let force = world
            .globals
            .forces
            .get("player")
            .expect("the player force");
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
