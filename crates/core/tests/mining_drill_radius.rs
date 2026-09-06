//! `mining_drill_radius`: how far a drill reaches beyond the tile it stands on.
//!
//! A drill's collision box cannot answer this, and without an answer the
//! planner cannot express "a drill mines a tile it does not stand on". That
//! one gap made two unrelated behaviours needlessly conservative: ore-aware
//! siting asked whether ore lay under a drill's own footprint because that was
//! all the model offered, and the will-not-bury placement rule could not tell a
//! belt over ore a drill can still reach from a belt over ore nobody can mine.
//!
//! # What is and is not verified here
//!
//! The **footprints** below are read out of `live-2.1.17-world-snapshot.json`,
//! a real RCON capture, and are checked on every run.
//!
//! The **radii** are double-sourced, by two paths that share nothing:
//!
//! 1. a live 2.1.17 RCON probe of `LuaEntityPrototype::mining_drill_radius`;
//! 2. the game's own prototype definitions, verified here by reading
//!    `workspace/server/data/base/prototypes/entity/mining-drill.lua`, where
//!    the field is spelled **`resource_searching_radius`** -- 2.49, 0.99 and
//!    0.49 on lines 1757, 1853 and 1900.
//!
//! **The field has two names**, which is worth knowing before concluding it
//! does not exist: `resource_searching_radius` at data stage,
//! `mining_drill_radius` at runtime. Grepping the data files for the runtime
//! name finds nothing at all.
//!
//! What is still *not* reproduced here is the radius arriving over the wire:
//! the committed capture predates the field and carries `null` for all 1,028
//! prototypes. So the wire assertions below are **dormant now and live on the
//! next recapture**: a snapshot carrying the field must agree with the table,
//! and one carrying it for some drills and not others fails rather than being
//! averaged over.
//!
//! Dormancy is stated out loud because a test that silently checks nothing is
//! this repo's most expensive recurring defect -- see
//! `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`. The
//! coverage test at the bottom is what stops this one from becoming that: it
//! fails if the drills disappear from the capture, so "nothing was checked"
//! cannot pass quietly.

use factorio_bot_core::types::FactorioEntityPrototype;

const SNAPSHOT: &str = include_str!("live-2.1.17-world-snapshot.json");

/// Name, expected footprint width, expected `mining_drill_radius`.
///
/// The widths are `collision_box` width, which is the box's own extent and not
/// the tile count: 1.40 occupies 2x2 tiles and 2.70 occupies 3x3.
const DRILLS: &[(&str, f64, f64)] = &[
    // 2 x 0.99 = 1.98 -> 2 tiles, exactly the 2x2 its box occupies.
    ("burner-mining-drill", 1.398_437_5, 0.99),
    // 2 x 2.49 = 4.98 -> 5 tiles against a 3x3: one tile on every side.
    ("electric-mining-drill", 2.695_312_5, 2.49),
    // A pumpjack IS a mining-drill, and it searches ONE tile: 2 x 0.49 = 0.98.
    // Its 3x3 footprint says nothing about that -- it must be centred on the
    // well itself, which is a far tighter siting constraint than either drill.
    ("pumpjack", 2.398_437_5, 0.49),
];

fn prototypes() -> Vec<FactorioEntityPrototype> {
    let value: serde_json::Value = serde_json::from_str(SNAPSHOT).expect("snapshot is JSON");
    serde_json::from_value(value["entity_prototypes"].clone()).expect("prototypes deserialise")
}

fn find(name: &str) -> FactorioEntityPrototype {
    prototypes()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} missing from the live capture"))
}

/// The footprints, checked against the real capture on every run.
///
/// This is the half of the table that does not depend on anybody's memory.
#[test]
fn the_capture_agrees_with_the_footprints_in_the_table() {
    for (name, width, _) in DRILLS {
        let proto = find(name);
        let got = proto.collision_box.right_bottom.x - proto.collision_box.left_top.x;
        assert!(
            (got - width).abs() < 1e-6,
            "{name}: capture says {got}, table says {width}"
        );
    }
}

/// A burner drill's mining area is its own footprint; an electric drill's is a
/// full tile ring beyond it. This is the claim the field exists to support.
///
/// **Compared in TILES, not in collision-box extents, and that distinction is
/// not cosmetic -- the first version of this test asserted the box comparison
/// and failed.** A burner drill's radius of 0.99 *does* exceed its collision
/// half-width of 0.699, so on the box measure it looks like it reaches beyond
/// itself. It does not: 2 x 0.99 = 1.98, which is 2 tiles, exactly the 2x2 its
/// 1.40 box occupies. Factorio sizes a 2x2 entity's box slightly under 2 so
/// that neighbours do not touch, and comparing a radius against that shaved
/// number measures the shaving.
///
/// | drill | box | tiles | 2 x radius | mining tiles |
/// |---|---|---|---|---|
/// | burner | 1.40 | 2 | 1.98 | **2** -- same |
/// | electric | 2.70 | 3 | 4.98 | **5** -- a ring beyond |
fn tiles(extent: f64) -> u32 {
    extent.ceil() as u32
}

#[test]
fn only_the_electric_drill_reaches_beyond_its_own_tiles() {
    let (_, burner_box, burner_radius) = DRILLS[0];
    let (_, electric_box, electric_radius) = DRILLS[1];

    // Absolute counts, not only relations. An earlier version asserted just
    // the relations and PASSED under a deliberately broken `tiles()` (floor
    // instead of ceil), because floor happens to preserve both comparisons on
    // these particular numbers. A relation between two wrong numbers is not
    // evidence, and the falsification is what showed it.
    assert_eq!(tiles(burner_box), 2, "a burner drill stands on 2x2");
    assert_eq!(tiles(2.0 * burner_radius), 2, "and works 2x2");
    assert_eq!(tiles(electric_box), 3, "an electric drill stands on 3x3");
    assert_eq!(tiles(2.0 * electric_radius), 5, "and works 5x5");

    assert_eq!(
        tiles(2.0 * burner_radius),
        tiles(burner_box),
        "a burner drill's mining area must be exactly its own footprint"
    );
    assert!(
        tiles(2.0 * electric_radius) > tiles(electric_box),
        "an electric drill must work more tiles than it stands on: {} vs {}",
        tiles(2.0 * electric_radius),
        tiles(electric_box)
    );
    assert_eq!(
        tiles(2.0 * electric_radius),
        tiles(electric_box) + 2,
        "the electric drill's reach is one tile on every side, so two per axis"
    );
}

/// A pumpjack is a `mining-drill` too, and its reach is the tightest of all.
///
/// 2 x 0.49 = 0.98, which is **one tile**, against a 3x3 footprint. So a
/// pumpjack must be centred on the crude-oil well itself: unlike an electric
/// drill, standing next to the resource is not enough, and unlike a burner
/// drill its footprint is no guide at all -- it covers 9 tiles and works 1.
///
/// This is the constraint that sites a pumpjack, and reading its 3x3 box as
/// its working area would place it on 8 tiles of ground that yield nothing.
#[test]
fn a_pumpjack_works_exactly_one_tile_under_a_three_by_three_footprint() {
    let (_, pumpjack_box, pumpjack_radius) = DRILLS[2];

    assert_eq!(
        tiles(2.0 * pumpjack_radius),
        1,
        "a pumpjack searches a single tile"
    );
    assert_eq!(tiles(pumpjack_box), 3, "and stands on nine");
    assert!(
        tiles(2.0 * pumpjack_radius) < tiles(pumpjack_box),
        "so its footprint overstates its reach, which is the trap"
    );
}

/// Dormant until a snapshot is recaptured with the field, then a real check.
///
/// Written this way so a recapture cannot quietly disagree with the table, and
/// so a *partially* populated capture -- one drill with a radius and one
/// without, which would mean the mod shipped half the change -- fails rather
/// than being averaged over.
#[test]
fn a_recaptured_snapshot_must_agree_with_the_measured_radii() {
    let present: Vec<bool> = DRILLS
        .iter()
        .map(|(name, _, _)| find(name).mining_drill_radius.is_some())
        .collect();

    assert!(
        present.iter().all(|p| *p) || present.iter().all(|p| !*p),
        "the capture carries mining_drill_radius for some drills and not others: {present:?}"
    );

    if present[0] {
        for (name, _, radius) in DRILLS {
            let got = find(name).mining_drill_radius.expect("checked present");
            assert!(
                (got - radius).abs() < 1e-6,
                "{name}: recaptured radius {got} disagrees with the measured {radius}. \
                 Either the game changed or the table is wrong -- do not just update the table"
            );
        }
    }
}

/// `None` must mean *unknown reach*, never *no reach*.
///
/// A caller that reads a missing radius as zero would conclude that an
/// electric drill mines only its own footprint, which is the exact
/// conservatism this field exists to remove -- and it would do it silently, on
/// every snapshot written before today.
#[test]
fn an_absent_radius_is_unknown_and_not_zero() {
    let proto = find("electric-mining-drill");
    assert!(
        proto.mining_drill_radius.is_none_or(|r| r > 0.0),
        "a present radius must be positive; absence means unknown"
    );
}

/// Stops the dormant test above from passing by finding nothing at all.
#[test]
fn the_drills_are_actually_in_the_capture() {
    let all = prototypes();
    assert!(
        all.len() > 500,
        "capture looks truncated: {} entries",
        all.len()
    );
    for (name, _, _) in DRILLS {
        assert!(
            all.iter().any(|p| p.name == *name),
            "{name} is not in the capture, so the radius tests check nothing"
        );
    }
}
