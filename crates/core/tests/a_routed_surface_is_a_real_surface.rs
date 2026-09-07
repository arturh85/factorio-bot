//! **A surface the parser routed to is a first-class surface, not a bag of
//! nodes.**
//!
//! Routing landed in three pieces across 2026-09-07 -- entities (`b0bb7f53`),
//! deletions, then ground -- and each proved that a record *reaches* the right
//! surface. What none of them asked is whether the surface it reaches is as
//! complete as the default one. Two things were named as making it
//! second-class:
//!
//! 1. `OutputParser::on_init` calls `entity_graph.connect()` and
//!    `flow_graph.update()` **on the default surface only**, so a routed
//!    surface would get nodes and never edges -- and a surface with no edges
//!    reports zero throughput, which is indistinguishable from a surface that
//!    genuinely produces nothing.
//! 2. `daylight` landed on the default surface however the mod named it, and
//!    daylight is the clearest per-surface fact there is:
//!    `solar_power_multiplier` and `ticks_per_day` are what make a solar array
//!    a different size on a different planet.
//!
//! **The first turned out not to be true any more, and this file is where that
//! is pinned rather than asserted in prose.** Two changes landed earlier the
//! same day between the fear and this test:
//!
//! * `EntityGraph::add` wires incrementally through `connect_nodes_near`
//!   (`2026-09-07-edges-that-outlive-tick-zero.md`), and every route into a
//!   surface -- `update_chunk_entities` and `on_some_entity_created` -- goes
//!   through `add`. Measured there on the 39,191-entity world-record base: a
//!   full sweep run on top of an incremental replay found **0** further edges.
//! * `FlowGraph` rebuilds on demand, keyed on the entity graph's `generation`,
//!   and *every* public reader calls `ensure_current` first. A surface whose
//!   `update()` was never called has `built_generation == NEVER_BUILT`, so the
//!   first read builds it.
//!
//! So `on_init` is an eager first walk for the surface that exists when
//! Factorio logs `initial discovery done`, and no longer the only maintenance
//! any surface gets. `a_routed_surface_is_wired_without_on_init` and
//! `on_init_adds_no_edge_a_routed_surface_lacked` are the two halves of that:
//! one says the edges are there, the other says the sweep would have added
//! nothing. Should `add` ever stop wiring, the first fails; should the sweep
//! ever find something the incremental path missed, the second fails.
//!
//! ## Vacuity, twice avoided
//!
//! Both traps this area has already sprung are designed against here.
//! `a_deletion_on_another_surface_leaves_this_one_standing` (2026-09-06)
//! passed because the *other* surface was never occupied at that tile, so a
//! fix that routed correctly and then did nothing satisfied it. And an
//! assertion that a surface has no edges passes trivially if the ingest never
//! ran at all. So **both surfaces here hold a wired chain of their own**, each
//! asserted by a positive count from the same computation, and the daylight
//! tests assert Nauvis's own derived numbers -- 0.7 average, 0.8467
//! accumulators per panel, both reproduced from the curve and neither written
//! down anywhere in the code -- beside the other planet's different ones.

use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::SurfaceId;
use std::sync::Arc;

fn world() -> Arc<FactorioWorld> {
    Arc::new(FactorioWorld::nauvis_only(Arc::new(FactorioSurface::new())))
}

/// A container record in the shape `serialize_entity` emits.
fn chest(name: &str, x: f64, y: f64, surface: &str) -> String {
    format!(
        r#"{{"name":"{name}","entity_type":"container","direction":0,"position":{{"x":{x},"y":{y}}},"bounding_box":{{"left_top":{{"x":{lx},"y":{ly}}},"right_bottom":{{"x":{rx},"y":{ry}}}}},"surface":"{surface}"}}"#,
        lx = x - 0.4,
        ly = y - 0.4,
        rx = x + 0.4,
        ry = y + 0.4,
    )
}

/// An inserter with a pickup and a drop position, which is what draws edges:
/// `connect_node` resolves both through `node_at` and wires
/// `pickup -> inserter -> drop`. Two edges, from prototype-free geometry, so
/// this fixture needs no prototype table.
fn inserter(x: f64, y: f64, pickup: (f64, f64), drop: (f64, f64), surface: &str) -> String {
    format!(
        r#"{{"name":"inserter","entity_type":"inserter","direction":0,"position":{{"x":{x},"y":{y}}},"bounding_box":{{"left_top":{{"x":{lx},"y":{ly}}},"right_bottom":{{"x":{rx},"y":{ry}}}}},"pickup_position":{{"x":{px},"y":{py}}},"drop_position":{{"x":{dx},"y":{dy}}},"surface":"{surface}"}}"#,
        lx = x - 0.4,
        ly = y - 0.4,
        rx = x + 0.4,
        ry = y + 0.4,
        px = pickup.0,
        py = pickup.1,
        dx = drop.0,
        dy = drop.1,
    )
}

fn entities_line(records: &[String]) -> String {
    format!("0,0;32,32:[{}]", records.join(","))
}

fn edge_count(surface: &FactorioSurface) -> usize {
    surface.entity_graph.inner_graph().edge_count()
}

fn node_count(surface: &FactorioSurface) -> usize {
    surface.entity_graph.inner_graph().node_count()
}

/// Three entities on each of two surfaces: chest, inserter, chest, the
/// inserter picking from one and dropping into the other. Both surfaces are
/// genuinely occupied, so "the other surface is not empty" is never the reason
/// a count comes out right.
fn two_wired_surfaces() -> Arc<FactorioWorld> {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());
    parser
        .parse(
            10,
            "entities",
            &entities_line(&[
                chest("iron-chest", 10.5, 10.5, "nauvis"),
                inserter(11.5, 10.5, (10.5, 10.5), (12.5, 10.5), "nauvis"),
                chest("iron-chest", 12.5, 10.5, "nauvis"),
                chest("wooden-chest", 20.5, 20.5, "vulcanus"),
                inserter(21.5, 20.5, (20.5, 20.5), (22.5, 20.5), "vulcanus"),
                chest("wooden-chest", 22.5, 20.5, "vulcanus"),
            ]),
        )
        .expect("the entities line must parse");
    world
}

/// **The routed surface has edges before anything swept it.**
///
/// `on_init` is never called here. The edges exist because `EntityGraph::add`
/// wires what it added, which is the property this asserts: if `add` stopped
/// calling `connect_nodes_near`, a routed surface would be nodes with no edges
/// and this fails.
#[test]
fn a_routed_surface_is_wired_without_on_init() {
    let world = two_wired_surfaces();
    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");

    // Three nodes each, so neither count below is over a surface that only
    // received part of the batch.
    assert_eq!(
        node_count(&nauvis),
        3,
        "nauvis holds its own three entities"
    );
    assert_eq!(
        node_count(&vulcanus),
        3,
        "and vulcanus its own three, at different coordinates"
    );

    // pickup -> inserter and inserter -> drop, on each surface.
    assert_eq!(
        edge_count(&nauvis),
        2,
        "the default surface is wired, as it always was"
    );
    assert_eq!(
        edge_count(&vulcanus),
        2,
        "and so is the routed one -- the incremental wiring in `add` runs \
         wherever `route` sent the batch"
    );
}

/// **The full sweep finds nothing the routed surface lacked** -- which is why
/// `on_init` connecting only the default surface is not the silent hole it
/// looks like.
///
/// This is the small-fixture form of the world-record measurement in
/// `2026-09-07-edges-that-outlive-tick-zero.md` ("edges the sweep still found:
/// 0" over 39,191 entities). It is stated as an equality on both surfaces
/// rather than as "vulcanus is non-zero", so a sweep that started adding edges
/// the incremental path misses fails here rather than passing quietly.
///
/// **The one known divergence runs the other way**: incremental wiring
/// produces one edge *more* than a from-scratch sweep when an underground half
/// is dropped into an existing pair's gap, because this graph only appends.
/// That is named in `a_half_dropped_into_a_tunnel_leaves_the_long_pair_behind`
/// and no fixture here contains an underground belt.
#[test]
fn on_init_adds_no_edge_a_routed_surface_lacked() {
    let world = two_wired_surfaces();
    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");

    let before = (edge_count(&nauvis), edge_count(&vulcanus));

    // Both sweeps by hand: `on_init` runs the default surface's, and the
    // question is what a routed surface would gain if it ran there too.
    nauvis.entity_graph.connect().expect("the nauvis sweep");
    vulcanus.entity_graph.connect().expect("the vulcanus sweep");

    assert_eq!(
        (edge_count(&nauvis), edge_count(&vulcanus)),
        before,
        "a full sweep must find nothing the incremental wiring in `add` missed, \
         on either surface"
    );
    assert_eq!(before, (2, 2), "and both surfaces were wired to begin with");
}

/// A `daylight` writeout body, in the shape `serialize_surface_daylight`
/// emits. `surface: None` is an older mod, which is what every archived run
/// contains.
fn daylight_line(surface: Option<&str>, ticks_per_day: u32, solar_power_multiplier: f64) -> String {
    let surface = match surface {
        Some(s) => format!(r#""surface":"{s}","#),
        None => String::new(),
    };
    format!(
        r#"{{{surface}"ticks_per_day":{ticks_per_day},"dawn":0.75,"dusk":0.25,"evening":0.45,"morning":0.55,"daytime":0.0,"solar_power_multiplier":{solar_power_multiplier},"always_day":false,"freeze_daytime":false}}"#
    )
}

/// What `PlanState::accumulators_per_panel` computes, restated here from the
/// same two inputs so this test measures the daylight and not the planner.
/// A vanilla solar panel makes 1,000 J/tick at noon with endpoints 1 and 0;
/// a vanilla accumulator's buffer is 5 MJ.
fn accumulators_per_panel(daylight: &factorio_bot_core::types::SurfaceDaylight) -> f64 {
    let deficit = daylight
        .night_deficit_fraction(1.0, 0.0)
        .expect("a curve that describes a day");
    let ticks_per_day = f64::from(daylight.ticks_per_day.expect("a day length"));
    deficit * 1_000.0 * ticks_per_day / 5_000_000.0
}

/// **Two surfaces carry two curves, and the numbers derived from them
/// differ.**
///
/// Nauvis's are asserted at their own known values -- 0.7 average and 0.8467
/// accumulators per panel, the two vanilla ratios the derivation reproduces --
/// so this is not merely "the two are not equal", which a routing bug that put
/// both curves on one surface and then corrupted one would also satisfy.
#[test]
fn two_surfaces_carry_different_daylight_and_different_solar_numbers() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(10, "daylight", &daylight_line(Some("nauvis"), 25_200, 1.0))
        .expect("the nauvis daylight line must parse");
    // A brighter, shorter-dayed planet. The values are chosen to move both
    // terms at once: the multiplier scales the average, and the day length
    // moves only the accumulator ratio.
    parser
        .parse(
            10,
            "daylight",
            &daylight_line(Some("vulcanus"), 12_600, 2.5),
        )
        .expect("the vulcanus daylight line must parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");

    let n = nauvis.daylight().expect("nauvis reported a curve");
    let v = vulcanus
        .daylight()
        .expect("vulcanus reported its own curve");

    assert_eq!(
        n.surface.as_ref().map(SurfaceId::as_str),
        Some("nauvis"),
        "the curve on nauvis is the one that named nauvis"
    );
    assert_eq!(
        v.surface.as_ref().map(SurfaceId::as_str),
        Some("vulcanus"),
        "and the curve on vulcanus the one that named vulcanus -- had the \
         second line landed on the default surface it would have replaced the \
         first, since `update_daylight` replaces rather than merges"
    );

    // The panel average: 0.7 of nameplate on vanilla Nauvis, and the other
    // planet's own figure from the same integral.
    let n_avg = n.average_solar_fraction(1.0, 0.0).expect("a nauvis day");
    let v_avg = v.average_solar_fraction(1.0, 0.0).expect("a vulcanus day");
    assert!(
        (n_avg - 0.7).abs() < 1e-9,
        "vanilla Nauvis averages 0.7 of nameplate, got {n_avg}"
    );
    assert!(
        (v_avg - 1.75).abs() < 1e-9,
        "the brighter planet averages 2.5x that, got {v_avg}"
    );

    // The accumulator bank: 0.8467 per panel on Nauvis at the live 25,200-tick
    // day, and a different number on a planet with a different day.
    let n_acc = accumulators_per_panel(&n);
    let v_acc = accumulators_per_panel(&v);
    assert!(
        (n_acc - 0.8467).abs() < 5e-4,
        "vanilla Nauvis needs 0.8467 accumulators per panel, got {n_acc}"
    );
    assert!(
        (v_acc - n_acc).abs() > 0.1,
        "and the other planet a materially different bank, got {v_acc} against {n_acc}"
    );
}

/// **A curve that names no surface is the sender not saying, and goes to the
/// default surface** -- the same rule every other routed writeout follows, and
/// the reading every archived run needs, because they all predate the field.
///
/// Paired with the other half, which is what makes it non-vacuous: the routed
/// surface must keep the curve it *was* given rather than being overwritten by
/// the unnamed one.
#[test]
fn a_curve_that_names_no_surface_goes_to_the_default_one() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            10,
            "daylight",
            &daylight_line(Some("vulcanus"), 12_600, 2.5),
        )
        .expect("the vulcanus daylight line must parse");
    parser
        .parse(10, "daylight", &daylight_line(None, 25_200, 1.0))
        .expect("an unnamed daylight line must parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must still be held");

    assert_eq!(
        nauvis.daylight().and_then(|d| d.ticks_per_day),
        Some(25_200),
        "an unnamed curve lands on the default surface"
    );
    assert_eq!(
        vulcanus.daylight().and_then(|d| d.ticks_per_day),
        Some(12_600),
        "and leaves the routed surface's own curve alone"
    );
    assert_eq!(
        nauvis.daylight().and_then(|d| d.surface),
        None,
        "the record still says nobody named a surface -- the parser does not \
         fill the field in on the sender's behalf"
    );
}
