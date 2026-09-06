//! Every blueprint string in `scripts/rcontest.lua` must decode.
//!
//! **This test exists because one of them did not, and nothing noticed.**
//! `MovingBlock` was committed with a single wrong character — byte 147, `F`
//! where the encoder had written `H` — which is enough for zlib to fail with
//! "invalid distance too far back". The blueprint had been *authored* correctly
//! and *run* correctly; the copy that reached this file was retyped by hand and
//! the substitution went in with it.
//!
//! What makes it worth a test rather than a fix is why it survived. There was a
//! decode test for `MovingBlock`, and it passed — because it decoded an inline
//! copy of the string, not the copy in `scripts/rcontest.lua`. **The test and
//! the fixture were two different strings that happened to share a name.** So
//! the fixture every script reads was broken while its own test stayed green,
//! and it was only found when a script actually loaded it and the run panicked.
//!
//! The guard is therefore deliberately shaped: it reads the real file from
//! disk, finds every blueprint-looking assignment in it, and decodes each one.
//! It cannot drift from what the scripts use, because it uses the same bytes.
//!
//! A blueprint string is a version byte, then base64, then zlib, then JSON —
//! so a single altered character usually destroys it rather than perturbing it.
//! That is a good property: corruption is loud, *provided somebody decodes it*.

use factorio_bot_core::blueprint::{decode, BlueprintError};
use std::path::PathBuf;

/// `<repo>/scripts/rcontest.lua`, resolved from this crate's manifest so the
/// test does not depend on the working directory a runner happens to use.
fn rcontest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/core -> crates -> repo root")
        .join("scripts/rcontest.lua")
}

/// Pull every `Name = "0..."` assignment out of the Lua source.
///
/// Matched by shape rather than by an allowlist of names, so a blueprint added
/// to that file is covered the moment it lands, without anyone remembering to
/// extend this test. A fixture nobody decodes is exactly what went wrong.
fn blueprint_assignments(src: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in src.lines() {
        let Some((lhs, rest)) = line.split_once('=') else {
            continue;
        };
        let name = lhs.trim().trim_start_matches("local ").trim();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = rest.trim();
        let Some(body) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = body.find('"') else { continue };
        let text = &body[..end];
        // A blueprint string starts with a version byte and is long; this
        // skips ordinary Lua strings without needing to know their names.
        if text.len() > 40 && text.starts_with('0') {
            found.push((name.to_string(), text.to_string()));
        }
    }
    found
}

#[test]
fn every_blueprint_in_rcontest_lua_decodes() {
    let path = rcontest_path();
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let blueprints = blueprint_assignments(&src);

    // If the extraction ever stops finding anything -- the file moves, or the
    // assignment style changes -- this test would otherwise pass while checking
    // nothing at all, which is the failure it was written to prevent.
    assert!(
        blueprints.len() >= 7,
        "expected at least the seven known fixtures in {}, found {} -- if the \
         file's shape changed, fix this extractor rather than letting the test \
         quietly check nothing",
        path.display(),
        blueprints.len()
    );

    // Two failure modes, and only one is a defect.
    //
    // **Corruption** -- a bad byte, so base64 or zlib gives up -- is always
    // wrong, and is what this test exists to catch.
    //
    // **Unsupported content** is not. The decoder refuses a blueprint carrying
    // recipes, tiles, circuit wiring or module requests BY NAME rather than
    // silently dropping it, because a block that quietly builds two thirds of
    // itself is worse than one that refuses. `StarterScience` carries recipes
    // on its assembling machines and so legitimately does not decode. Asserting
    // "everything decodes" would fail on a fixture behaving exactly as designed
    // -- which it did, on the first run of this test.
    let mut corrupt = Vec::new();
    let mut refused = Vec::new();
    for (name, text) in &blueprints {
        match decode(text) {
            Ok(bp) => assert!(
                !bp.entities.is_empty(),
                "{name} decoded to zero entities, which no fixture here should"
            ),
            Err(BlueprintError::Unsupported(what)) => {
                refused.push(format!("{name} (unsupported: {what})"));
            }
            Err(e) => corrupt.push(format!("{name}: {e:?}")),
        }
    }
    eprintln!("refused by design, not corrupt: {refused:?}");

    assert!(
        corrupt.is_empty(),
        "blueprint fixtures in {} are CORRUPT: {}\n\
         A single wrong character destroys a blueprint string -- MovingBlock \
         once shipped with byte 147 as 'F' instead of 'H'. Copy these \
         programmatically from the encoder; do not retype them.",
        path.display(),
        corrupt.join(", ")
    );
}

/// `SmeltingBlock`'s geometry is not arbitrary — it is lifted from
/// `FurnaceLine`, which stood 176 of 179 entities correctly in a live game.
///
/// Pinned here because the value of that block is precisely that its offsets
/// are *proven* rather than authored: a furnace is 2x2 and sits on integer
/// coordinates while its inserters sit on half-integers, and getting that
/// parity wrong yields a layout that places perfectly and does nothing. If
/// someone "tidies" these numbers, this fails.
#[test]
fn the_smelting_block_keeps_the_geometry_it_inherited_from_furnaceline() {
    let src = std::fs::read_to_string(rcontest_path()).expect("rcontest.lua readable");
    let (_, text) = blueprint_assignments(&src)
        .into_iter()
        .find(|(name, _)| name == "SmeltingBlock")
        .expect("SmeltingBlock is in rcontest.lua");
    let bp = decode(&text).expect("SmeltingBlock decodes");

    let mut seen: Vec<(String, f64, f64, u8)> = bp
        .entities
        .iter()
        .map(|e| (e.name.clone(), e.offset.x(), e.offset.y(), e.direction))
        .collect();
    seen.sort_by(|a, b| a.3.cmp(&b.3).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)));

    // Both inserters are direction 0: the input picks from the north (the
    // source chest) and drops south into the furnace; the output picks from
    // the north (the furnace itself) and drops south into the sink. That is
    // FurnaceLine's own arrangement, and an inserter's direction names the
    // side it PICKS UP from.
    let want: Vec<(&str, f64, f64, u8)> = vec![
        ("iron-chest", 5.5, 0.5, 0),
        ("burner-inserter", 5.5, 1.5, 0),
        ("stone-furnace", 5.0, 3.0, 0),
        ("burner-inserter", 5.5, 4.5, 0),
        ("iron-chest", 5.5, 5.5, 0),
    ];
    assert_eq!(seen.len(), want.len(), "entity count changed: {seen:?}");
    for (name, x, y, dir) in &want {
        assert!(
            seen.iter().any(|(n, sx, sy, sd)| n == name
                && (sx - x).abs() < 1e-9
                && (sy - y).abs() < 1e-9
                && sd == dir),
            "missing {name} at ({x}, {y}) facing {dir}; got {seen:?}"
        );
    }
}


/// `TJunctionSmelter` merges ore and coal onto one belt the way Factorio does.
///
/// Two mechanics decide whether this block works, and neither is visible in a
/// picture of it — a layout with either one backwards places 100% correctly and
/// moves nothing useful:
///
/// 1. **An inserter drops on the belt's FAR lane.** The ore loader therefore
///    sits north of the main belt, so ore lands on the south lane.
/// 2. **A belt running into the SIDE of another sideloads onto its NEAR lane.**
///    The coal branch therefore comes from the north, putting coal on the north
///    lane.
///
/// Get either wrong and both commodities land on one lane, where coal crowds
/// ore out. That is measured, not supposed: the single-chest predecessor to
/// this block drained 50 coal down to 17 over 2,500 ticks while its ore never
/// moved off 99, and produced one plate — from the single ore that escaped
/// before the coal took over. Lane separation is the mechanism, not tidiness.
///
/// The prototype list is pinned for the same reason as the geometry: every name
/// must be enabled on a fresh force, because the only moment this block is
/// interesting is before any research exists.
#[test]
fn the_t_junction_smelter_separates_its_lanes_and_needs_no_research() {
    let src = std::fs::read_to_string(rcontest_path()).expect("rcontest.lua readable");
    let (_, text) = blueprint_assignments(&src)
        .into_iter()
        .find(|(name, _)| name == "TJunctionSmelter")
        .expect("TJunctionSmelter is in rcontest.lua");
    let bp = decode(&text).expect("TJunctionSmelter decodes");

    // Verified against `crates/core/tests/live-2.1.17-world-snapshot.json`:
    // each of these recipes carries `enabled: true` on an unresearched force.
    const BUILDABLE_AT_T0: &[&str] = &[
        "iron-chest",
        "burner-inserter",
        "transport-belt",
        "stone-furnace",
    ];
    for e in &bp.entities {
        assert!(
            BUILDABLE_AT_T0.contains(&e.name.as_str()),
            "{} needs research; this block must stand on a fresh force",
            e.name
        );
    }

    let at = |n: &str, x: f64, y: f64| {
        bp.entities.iter().find(|e| {
            e.name == n && (e.offset.x() - x).abs() < 1e-9 && (e.offset.y() - y).abs() < 1e-9
        })
    };

    // The main belt runs east along y = 0.5 and must be unbroken from the ore
    // loader's drop tile through the last takeoff, or the lanes never arrive.
    for i in 0..8 {
        let x = 3.5 + f64::from(i);
        assert_eq!(
            at("transport-belt", x, 0.5).map(|e| e.direction),
            Some(4),
            "main belt must run east at x={x}"
        );
    }

    // Ore enters from the NORTH, so far-lane insertion puts it on the SOUTH lane.
    assert_eq!(
        at("burner-inserter", 3.5, -0.5).map(|e| e.direction),
        Some(0),
        "the ore loader must pick from the chest to its north"
    );
    assert!(
        at("iron-chest", 3.5, -1.5).is_some(),
        "ore chest north of its loader"
    );

    // The coal branch runs SOUTH into the side of the main belt. Its last tile
    // must sit directly above a main-belt tile: that adjacency IS the T
    // junction, and a one-tile gap turns the merge into a belt that just ends.
    for y in [-1.5f64, -0.5] {
        assert_eq!(
            at("transport-belt", 5.5, y).map(|e| e.direction),
            Some(8),
            "coal branch must run south at y={y}"
        );
    }
    assert!(
        at("transport-belt", 5.5, 0.5).is_some(),
        "the coal branch must terminate against a main-belt tile, or it is not a T junction"
    );

    // Takeoffs sit SOUTH of the belt — so they draw the far (coal) lane first
    // and fall back to the near (ore) lane once a furnace's fuel slot is full —
    // and drop into the furnace directly below.
    for (x, fx) in [(7.5f64, 7.0f64), (8.5, 9.0)] {
        assert_eq!(
            at("burner-inserter", x, 1.5).map(|e| e.direction),
            Some(0),
            "takeoff at x={x} must pick from the belt to its north"
        );
        // A 2x2 furnace centred at fx spans fx-1 .. fx+1; the arm drops at (x, 2.5).
        assert!(
            at("stone-furnace", fx, 3.0).is_some() && (x - fx).abs() < 1.0,
            "takeoff at x={x} must drop inside the furnace at x={fx}"
        );
    }
}
