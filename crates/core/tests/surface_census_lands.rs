//! **The census reaches Rust**, on both transports, and keeps the one
//! distinction it exists for.
//!
//! The mod has been able to enumerate `game.surfaces` since `95c183be`
//! (`remote.call('botbridge', 'surfaces')`, plus a `surfaces` field on
//! `world_snapshot`) and **nothing in Rust could receive it**. That mattered
//! because the alternative to a census is a silence: `on_chunk_generated`
//! drops every non-Nauvis chunk and names the surface it dropped, which is
//! honest per chunk and cannot sum, so "this save has one surface" and "we
//! never looked" produced identical world models. The world-record base was
//! read as 39,237 entities on `nauvis`; it has **ten surfaces**, five of them
//! space platforms, and until the census there was no way to say whether that
//! number was a fact about the save or about us.
//!
//! Three landing sites, all pinned here:
//!
//! 1. `WorldSnapshot.surfaces` -- the RCON/`--connect` transport.
//! 2. `GameGlobals.surfaces` -- the home, read back through
//!    `FactorioWorld::surface_census`. Game-global for the same reason a
//!    research state is, and the aggregate `output_parser` and `snapshot` can
//!    actually reach.
//! 3. The `"surfaces"` arm in `output_parser.rs` -- the stdout transport,
//!    which had to ship with the mod's `writeout_surfaces` because the parser
//!    logs an *error* for a writeout key it has no arm for.
//!
//! **Absent stays distinct from empty throughout.** `None` is *nobody
//! enumerated*; an empty list would claim a running game has no surfaces,
//! which cannot happen. And a surface with `planet: None` is a *platform*, a
//! real answer -- half of the world-record save's surfaces take it.
//!
//! This ingests nothing. The Nauvis guard stays, `game.surfaces[1]` in the
//! chunk replay stays, and the world model still holds one surface. See
//! `docs/superpowers/notes/2026-09-07-the-census-lands.md`.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::FactorioSurfaceInfo;
use std::sync::Arc;

/// The world-record save's own census, five planets and five platforms, as the
/// mod's `collect_surfaces` renders it: sorted by index, a platform carrying
/// no `planet` key at all.
const WR_CENSUS: &str = r#"[
    {"name":"nauvis","index":1,"planet":"nauvis"},
    {"name":"platform-1","index":2},
    {"name":"platform-2","index":3},
    {"name":"platform-3","index":4},
    {"name":"gleba","index":5,"planet":"gleba"},
    {"name":"vulcanus","index":6,"planet":"vulcanus"},
    {"name":"platform-4","index":7},
    {"name":"platform-5","index":8},
    {"name":"fulgora","index":9,"planet":"fulgora"},
    {"name":"aquilo","index":10,"planet":"aquilo"}
]"#;

fn census(rows: &[(&str, u32, Option<&str>)]) -> Vec<FactorioSurfaceInfo> {
    rows.iter()
        .map(|(name, index, planet)| FactorioSurfaceInfo {
            name: (*name).into(),
            index: *index,
            planet: planet.map(str::to_owned),
        })
        .collect()
}

/// **The stdout arm, which is why this commit also touches `control.lua`.**
///
/// `output_parser.rs` logs `unexpected action: <key>` as an *error* for a
/// writeout key it has no arm for, so `writeout_surfaces` and this arm could
/// not ship apart without putting a red line in every run.
#[test]
fn the_stdout_transport_lands_the_census_on_the_world() {
    let mut parser = OutputParser::new();
    parser
        .parse(0, "surfaces", WR_CENSUS)
        .expect("the census parses");

    let landed = parser
        .world()
        .surface_census()
        .expect("the parser landed it -- absent here would mean no arm ran");
    assert_eq!(landed.len(), 10, "five planets and five space platforms");
    assert_eq!(landed[0].name, "nauvis");
    assert_eq!(landed[9].name, "aquilo");
    assert_eq!(
        landed.iter().filter(|s| s.planet.is_none()).count(),
        5,
        "half the save's surfaces are platforms, so `planet: None` is a live \
         case rather than a defensive one",
    );
    assert_eq!(
        landed[4].planet.as_deref(),
        Some("gleba"),
        "a planet names itself, and it is read from `LuaSurface.planet` rather \
         than copied from the surface's own name",
    );
}

/// **All rows or none, deliberately unlike `entity_prototypes`** directly
/// above it in the parser, which `filter_map`s a bad row away.
///
/// Missing one prototype is *degraded*. A census missing one row is wrong in
/// the exact direction the field exists to prevent: it under-reports the
/// surfaces a save has while reading as a complete answer.
#[test]
fn a_census_with_one_bad_row_stays_absent_rather_than_arriving_short() {
    let mut parser = OutputParser::new();
    parser
        .parse(
            0,
            "surfaces",
            r#"[{"name":"nauvis","index":1},{"name":"gleba","index":"five"}]"#,
        )
        .expect("a malformed census is logged, not fatal");

    assert_eq!(
        parser.world().surface_census(),
        None,
        "a census that cannot be parsed stays *nobody enumerated*; arriving \
         with one of two surfaces would be a complete-looking wrong answer",
    );

    // Non-accidental control: the same parser on the same surface lands a
    // well-formed census, so the `None` above is a refusal and not a parser
    // that never ran.
    parser
        .parse(0, "surfaces", r#"[{"name":"nauvis","index":1}]"#)
        .expect("a good census parses");
    assert_eq!(parser.world().surface_census().map(|c| c.len()), Some(1));
}

/// The RCON transport, and the **non-erasure rule** `daylight` established: a
/// sender that says nothing does not overwrite what a newer one said, or
/// "this build is old" silently becomes "nobody ever looked".
#[test]
fn a_snapshot_that_says_nothing_about_surfaces_does_not_erase_the_census() {
    let surface = FactorioSurface::new();
    surface.update_surface_census(census(&[("nauvis", 1, Some("nauvis"))]));

    let snapshot = WorldSnapshot::default();
    assert!(
        snapshot.surfaces.is_none(),
        "an older BotBridge sends nothing here",
    );
    surface.apply_snapshot(snapshot).expect("apply");

    assert_eq!(
        surface.surface_census().map(|c| c.len()),
        Some(1),
        "the census the world already had survives a silent snapshot",
    );
}

/// And the other half of the same rule: a snapshot that *does* carry one
/// installs it. Without this, non-erasure would pass on a field that never
/// lands at all.
#[test]
fn a_snapshot_carrying_a_census_installs_it() {
    let surface = FactorioSurface::new();
    assert_eq!(surface.surface_census(), None, "nobody has looked yet");

    let snapshot = WorldSnapshot {
        surfaces: Some(census(&[
            ("nauvis", 1, Some("nauvis")),
            ("platform-1", 2, None),
        ])),
        ..WorldSnapshot::default()
    };
    surface.apply_snapshot(snapshot).expect("apply");

    let landed = surface.surface_census().expect("installed");
    assert_eq!(landed.len(), 2);
    assert_eq!(landed[1].planet, None, "the platform stays a platform");
}

/// **The census is game-global, so it is read from the aggregate.**
///
/// `FactorioWorld` is constructed in one place and reached only through
/// `FactorioInstance`, while `output_parser` and `snapshot` hold a
/// `FactorioSurface` -- so the census is written through a surface and read
/// back through the world, and those are the same value because every surface
/// of one world shares its `Arc<GameGlobals>`.
#[test]
fn a_census_written_through_a_surface_is_read_back_through_the_world() {
    let surface = Arc::new(FactorioSurface::new());
    let world = FactorioWorld::nauvis_only(surface.clone());
    surface.update_surface_census(census(&[
        ("nauvis", 1, Some("nauvis")),
        ("platform-1", 2, None),
    ]));

    assert_eq!(
        world.surface_census().map(|c| c.len()),
        Some(2),
        "the world reads the globals its surface wrote",
    );
    // **The two questions are different, and the difference is the finding.**
    // The census says what the *save* has; `len()` says what this world model
    // *holds*, which is one because the mod's Nauvis guard drops the rest.
    assert_eq!(
        world.len(),
        1,
        "the model still holds one surface -- this change records, it does not \
         ingest",
    );
}

/// A dump carries it, and a dump written before the field loads as
/// *nobody enumerated* rather than as an empty census.
///
/// An offline plan has no game to enumerate, so a dump that dropped the census
/// would make the question unanswerable rather than merely stale -- the same
/// argument `daylight` makes one field along.
#[test]
fn the_census_round_trips_through_a_dump_and_an_old_dump_reads_as_absent() {
    let surface = FactorioSurface::new();
    surface.update_surface_census(census(&[
        ("nauvis", 1, Some("nauvis")),
        ("platform-1", 2, None),
    ]));

    let json = serde_json::to_string(&surface).expect("serialise");
    let reloaded: FactorioSurface = serde_json::from_str(&json).expect("deserialise");
    let landed = reloaded.surface_census().expect("carried through the dump");
    assert_eq!(landed.len(), 2);
    assert_eq!(landed[1].name, "platform-1");
    assert_eq!(landed[1].planet, None);

    // Every archived dump predates the field. Absent must stay absent, not
    // become "this save has no surfaces".
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("value");
    assert!(
        value
            .as_object_mut()
            .expect("object")
            .remove("surfaces")
            .is_some(),
        "the key was there to remove -- otherwise this test proves nothing",
    );
    let old: FactorioSurface =
        serde_json::from_value(value).expect("a dump with no census still loads");
    assert_eq!(
        old.surface_census(),
        None,
        "an old dump means *nobody enumerated*, never an empty game",
    );
}

/// The distinction the whole field exists for, stated as an assertion rather
/// than a paragraph: **a census that was never taken is not a world with no
/// surfaces.**
#[test]
fn absent_and_empty_are_different_values() {
    let never_asked = FactorioSurface::new();
    assert_eq!(never_asked.surface_census(), None);

    let asked = FactorioSurface::new();
    asked.update_surface_census(Vec::new());
    assert_eq!(
        asked.surface_census(),
        Some(Vec::new()),
        "an empty census is a distinct value from an absent one -- it cannot \
         come from a running game, and folding the two together is exactly the \
         silence this field ends",
    );
    assert_ne!(never_asked.surface_census(), asked.surface_census());
}
