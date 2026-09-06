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
