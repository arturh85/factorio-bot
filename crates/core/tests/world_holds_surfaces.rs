//! What [`FactorioWorld`] claims about the surfaces it holds.
//!
//! Every test here was falsified against the implementation one break at a
//! time, and the surviving claims are the ones that failed when broken.

use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::types::{FactorioForce, SurfaceId};
use std::sync::Arc;

fn surface() -> Arc<FactorioSurface> {
    Arc::new(FactorioSurface::new())
}

/// A surface that shares `world`'s globals -- the only kind a world accepts.
fn sibling(world: &FactorioWorld) -> Arc<FactorioSurface> {
    Arc::new(FactorioSurface::with_globals(world.globals().clone()))
}

#[test]
fn a_nauvis_only_world_holds_exactly_that_surface_under_that_name() {
    let held = surface();
    let world = FactorioWorld::nauvis_only(held.clone());

    assert_eq!(world.len(), 1);
    assert!(!world.is_empty());
    assert_eq!(
        world.surface_ids().collect::<Vec<_>>(),
        vec![&SurfaceId::nauvis()]
    );
    // The same surface, not an equal one: the world holds a handle, so a
    // caller reaching through it observes what the output parser writes.
    assert!(Arc::ptr_eq(world.nauvis().expect("nauvis"), &held));
    assert!(Arc::ptr_eq(
        world.surface(&SurfaceId::nauvis()).expect("by id"),
        &held
    ));
}

#[test]
fn an_unobserved_surface_is_none_rather_than_an_empty_one() {
    let world = FactorioWorld::nauvis_only(surface());

    // Not `Some(empty surface)`: the mod drops every non-Nauvis chunk, so a
    // surface nobody was told about must read as unknown, never as "there is
    // nothing there".
    assert!(world.surface(&SurfaceId::from("vulcanus")).is_none());
}

#[test]
fn only_surface_answers_while_the_world_holds_one() {
    let held = surface();
    let world = FactorioWorld::nauvis_only(held.clone());

    assert!(Arc::ptr_eq(world.only_surface().expect("one"), &held));
}

#[test]
fn a_world_built_under_another_name_still_answers_only_surface_but_not_nauvis() {
    let held = surface();
    let world = FactorioWorld::new(SurfaceId::from("vulcanus"), held.clone());

    // `only_surface` is the porting seam and does not care what the surface
    // is called; `nauvis()` is for a caller that genuinely means Nauvis, and
    // must not invent one.
    assert!(Arc::ptr_eq(world.only_surface().expect("one"), &held));
    assert!(world.nauvis().is_none());
}

#[test]
fn re_inserting_the_held_surface_replaces_it_and_stays_one_surface() {
    let first = surface();
    let mut world = FactorioWorld::nauvis_only(first.clone());
    let second = sibling(&world);

    world
        .insert_surface(SurfaceId::nauvis(), second.clone())
        .expect("the same surface refreshed is not a second surface");

    assert_eq!(world.len(), 1);
    assert!(Arc::ptr_eq(world.nauvis().expect("nauvis"), &second));
    assert!(!Arc::ptr_eq(world.nauvis().expect("nauvis"), &first));
}

/// **The refusal that used to stand here is gone, and this is what earned
/// its removal.**
///
/// `insert_surface` refused *every* second surface by name
/// (`SurfaceNotYetSeparable`) for as long as recipes, prototypes, forces and
/// the action id counter were fields on `FactorioSurface`: accepting one
/// would have handed the run two copies of the research state. Those fields
/// live in one `GameGlobals` now, shared by `Arc`, so this holds two surfaces
/// and reads the globals **through both of them**.
///
/// Three facts, one per row of the type's own table -- a force's research, an
/// entry of the prototype data, and the action id counter -- each written
/// through one surface and read back through the other.
#[test]
fn two_surfaces_share_one_force_one_recipe_table_and_one_action_id_counter() {
    let nauvis = surface();
    let mut world = FactorioWorld::nauvis_only(nauvis.clone());
    let vulcanus = sibling(&world);
    world
        .insert_surface(SurfaceId::from("vulcanus"), vulcanus.clone())
        .expect("a surface sharing the world's globals is not a second world");

    assert_eq!(world.len(), 2, "both are held");

    // 1. Research. Written through Nauvis, read through Vulcanus.
    let force: FactorioForce = serde_json::from_str(
        r#"{
          "name": "player",
          "force_id": 1,
          "current_research": null,
          "research_progress": null,
          "technologies": {
            "automation": {
              "name": "automation",
              "enabled": true,
              "upgrade": false,
              "researched": true,
              "prerequisites": [],
              "research_unit_ingredients": [],
              "research_unit_count": 10,
              "research_unit_energy": 600.0,
              "order": "a-a",
              "level": 1,
              "valid": true
            }
          }
        }"#,
    )
    .expect("the force fixture parses");
    nauvis.update_force(force).expect("a force update");
    let seen_from_vulcanus = vulcanus
        .globals
        .forces
        .get("player")
        .expect("the same force is visible from the other surface");
    assert!(
        seen_from_vulcanus
            .technologies
            .get("automation")
            .expect("the technology travelled with it")
            .researched,
        "two surfaces disagreeing about what is researched is the bug this \
         whole split exists to prevent",
    );

    // 2. Prototype data. One table, not two.
    assert!(
        Arc::ptr_eq(&nauvis.globals.recipes, &vulcanus.globals.recipes),
        "the recipe table is the same object, not an equal copy",
    );
    assert!(Arc::ptr_eq(
        &nauvis.globals.entity_prototypes,
        &vulcanus.globals.entity_prototypes
    ));

    // 3. The action id space. Minting through one surface must advance the
    //    counter the other reads: two counters would hand two surfaces the
    //    same `action_id`, and the executor keys its completion signal on it.
    let minted = {
        let mut next = nauvis
            .globals
            .next_action_id
            .try_lock()
            .expect("uncontended");
        let id = *next;
        *next += 1;
        id
    };
    let next_from_vulcanus = *vulcanus
        .globals
        .next_action_id
        .try_lock()
        .expect("uncontended");
    assert_eq!(
        next_from_vulcanus,
        minted + 1,
        "the counter advanced for both, because there is only one",
    );

    // And it really is one object, not two that happen to agree so far.
    assert!(Arc::ptr_eq(&nauvis.globals, &vulcanus.globals));
    assert!(Arc::ptr_eq(world.globals(), &vulcanus.globals));
}

/// A surface carrying **its own** globals is still refused, by name.
///
/// This is what is left of `SurfaceNotYetSeparable`, and it is the narrower,
/// sharper version: not "a second surface is impossible" but "this particular
/// surface would bring a second research state with it".
#[test]
fn a_surface_with_its_own_globals_is_refused_by_name() {
    let mut world = FactorioWorld::nauvis_only(surface());

    let refusal = world
        // `surface()`, not `sibling()`: a freshly built surface has globals
        // of its own.
        .insert_surface(SurfaceId::from("vulcanus"), surface())
        .expect_err("its own globals would duplicate the force's research");

    assert_eq!(refusal.held, SurfaceId::nauvis());
    assert_eq!(refusal.offered, SurfaceId::from("vulcanus"));
    // The message names both surfaces, because "cannot add a surface" without
    // saying which is beside which is not a disclosure.
    let said = refusal.to_string();
    assert!(said.contains("vulcanus"), "{said}");
    assert!(said.contains("nauvis"), "{said}");

    // And the refusal is total: the world is unchanged, not half-updated.
    assert_eq!(world.len(), 1);
    assert!(world.surface(&SurfaceId::from("vulcanus")).is_none());
}

/// **A cloned surface is a fork and cannot be put back**, which is the one
/// consequence of the design worth pinning.
///
/// `Clone` deep-copies the globals on purpose: the plan world is speculative
/// and its writes must not reach the live model. That makes a clone's globals
/// a different object, so inserting one would be exactly the duplication the
/// check exists to stop -- and it looks harmless, because the copy starts out
/// equal.
#[test]
fn a_cloned_surface_is_refused_because_a_clone_forks_the_globals() {
    let nauvis = surface();
    let mut world = FactorioWorld::nauvis_only(nauvis.clone());

    let forked = Arc::new((*nauvis).clone());
    assert!(
        !Arc::ptr_eq(&forked.globals, &nauvis.globals),
        "a clone forks the globals -- if this ever shares them, the plan \
         world's imagined research reaches the executor",
    );
    world
        .insert_surface(SurfaceId::from("vulcanus"), forked)
        .expect_err("a fork brings a second copy of the research state");
    assert_eq!(world.len(), 1);
}

/// **A dump has to carry the daylight curve, because an offline plan has no
/// game to ask.**
///
/// `crates/planner`'s solar accessors read the surface, not a prototype, so a
/// `world.dump` that dropped this would make every solar question unanswerable
/// on exactly the basis this project iterates in. Round-tripped through the
/// hand-written `Serialize`/`Deserialize` pair rather than checked in memory:
/// that pair names its fields as strings in four separate places, and a field
/// added to three of them is a silent loss.
#[test]
fn a_surfaces_daylight_survives_a_dump_and_reload() {
    let surface = FactorioSurface::new();
    surface.update_daylight(factorio_bot_core::types::SurfaceDaylight {
        surface: Some(SurfaceId::nauvis()),
        ticks_per_day: Some(25_200),
        dawn: Some(0.75),
        dusk: Some(0.25),
        evening: Some(0.45),
        morning: Some(0.55),
        daytime: Some(0.125),
        solar_power_multiplier: Some(1.0),
        always_day: Some(false),
        freeze_daytime: Some(false),
    });

    let json = serde_json::to_string(&surface).expect("a surface serialises");
    let back: FactorioSurface = serde_json::from_str(&json).expect("and reloads");
    let daylight = back.daylight().expect("the curve came back");
    assert_eq!(daylight.ticks_per_day, Some(25_200));
    assert_eq!(daylight.dusk, Some(0.25));
    assert_eq!(daylight.dawn, Some(0.75));
    assert_eq!(daylight.solar_power_multiplier, Some(1.0));
    assert_eq!(
        daylight.average_solar_fraction(1.0, 0.0),
        Some(0.7),
        "and it still describes a day after the round trip",
    );
}

/// **Every dump this project has archived predates this field, and all of them
/// must stay readable.**
///
/// Two distinct absences collapse to the same answer on purpose: a file with no
/// `daylight` key at all, and one whose sender had no curve to report. Neither
/// is a surface in permanent darkness, and a planner that read either as zero
/// would credit solar at nothing for a reason nobody established — the failure
/// this repo's rule about `None` exists to prevent.
#[test]
fn a_dump_written_before_the_daylight_channel_still_loads() {
    let surface = FactorioSurface::new();
    let json = serde_json::to_string(&surface).expect("a surface serialises");
    let stripped: serde_json::Value = {
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("json");
        value
            .as_object_mut()
            .expect("an object")
            .remove("daylight")
            .expect("the key is there to remove, or this test proves nothing");
        value
    };

    let back: FactorioSurface =
        serde_json::from_value(stripped).expect("an older dump must still load");
    assert!(
        back.daylight().is_none(),
        "absent is unknown, and unknown is not dark",
    );
}
