//! What [`FactorioWorld`] claims about the surfaces it holds.
//!
//! Every test here was falsified against the implementation one break at a
//! time, and the surviving claims are the ones that failed when broken.

use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::types::SurfaceId;
use std::sync::Arc;

fn surface() -> Arc<FactorioSurface> {
    Arc::new(FactorioSurface::new())
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
    let second = surface();
    let mut world = FactorioWorld::nauvis_only(first.clone());

    world
        .insert_surface(SurfaceId::nauvis(), second.clone())
        .expect("the same surface refreshed is not a second surface");

    assert_eq!(world.len(), 1);
    assert!(Arc::ptr_eq(world.nauvis().expect("nauvis"), &second));
    assert!(!Arc::ptr_eq(world.nauvis().expect("nauvis"), &first));
}

#[test]
fn a_second_surface_is_refused_by_name_rather_than_forking_the_research_state() {
    let mut world = FactorioWorld::nauvis_only(surface());

    let refusal = world
        .insert_surface(SurfaceId::from("vulcanus"), surface())
        .expect_err("a second surface would duplicate the force's research");

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
