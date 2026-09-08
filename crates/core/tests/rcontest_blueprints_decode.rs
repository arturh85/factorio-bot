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

use factorio_bot_core::blueprint::{BlueprintError, decode};
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
    seen.sort_by(|a, b| {
        a.3.cmp(&b.3)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
    });

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

/// **The fixtures that declare a grid keep their pitch through decode.**
///
/// Until 2026-09-08 `snap-to-grid` and `absolute-snapping` were allowlisted and
/// silently discarded, and `position-relative-to-grid` was not allowlisted at
/// all — so a blueprint carrying it failed the decode by name while the other
/// two were accepted and dropped.
///
/// The pitch is the load-bearing field: it is the author's own statement of how
/// far apart two copies of the block sit. Nothing else in a blueprint says it —
/// a bounding box over the entities measures what was *drawn*, and the two
/// differ wherever a design leaves deliberate space beside itself. `MinerLine`
/// is the clearest case: its entities span 5 tiles of x and its declared pitch
/// is 7, which is the room its 3x3 drills actually need.
#[test]
fn the_fixtures_that_declare_a_grid_keep_it() {
    let src = std::fs::read_to_string(rcontest_path()).expect("rcontest.lua readable");
    let mut with_grid: Vec<(String, f64, f64, bool)> = Vec::new();
    let mut without = 0usize;
    for (name, text) in blueprint_assignments(&src) {
        let Ok(bp) = decode(&text) else { continue };
        match bp.grid {
            Some(g) => with_grid.push((name, g.pitch.x(), g.pitch.y(), g.absolute)),
            None => without += 1,
        }
    }
    // By NAME: an f64 is not `Ord`, and the pitches are the values under test
    // rather than the key.
    with_grid.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        with_grid,
        vec![
            ("FurnaceLine".to_string(), 29.0, 11.0, true),
            ("MinerLine".to_string(), 7.0, 21.0, true),
            ("SmeltRow24".to_string(), 24.0, 14.0, false),
        ],
        "the declared pitches, read back off the real fixtures. StarterScience \
         declares snap-to-grid 6x11 in its JSON and is absent here on purpose: \
         it carries RECIPES, which this decoder refuses by name, so it never \
         reaches the grid at all. A raw JSON scan sees four; the decoder sees \
         three, and the decoder is what the planner gets. SmeltRow24 is the \
         one with RELATIVE snapping: its period is a claim about how far apart \
         two copies sit, which is all `Site::Beside` reads, and absolute \
         snapping would additionally pin it to a global lattice — a claim \
         nothing here has grounds to make"
    );
    assert!(
        without >= 10,
        "and most fixtures declare nothing, which must stay distinguishable \
         from declaring a zero pitch; got {without}"
    );
}


/// **`SmeltRow24`'s output arms must take plates OUT of the furnaces.**
///
/// An inserter's `direction` names the side it PICKS UP from, so the arm north
/// of the north furnace row -- between that row and the output belt -- must
/// face **south**, into the furnace. Facing north it runs backwards, lifting
/// plates off the output belt and pushing them into the machine that made them.
///
/// Asserted through the real decoder rather than off the raw JSON, because the
/// decoder applies `blueprint_direction`, and that migration is exactly what
/// this test exists to keep honest. A blueprint declaring a pre-2.0 `version`
/// has its directions DOUBLED, since 1.x's eight-value `defines.direction`
/// sits on 2.x's even values. The migration is correct. What it cannot check
/// is whether the version stamp tells the truth, and a blueprint stamped 1.x
/// while carrying 2.x values is silently rotated by it: `east` (4) becomes
/// `south` (8), and `south` (8) becomes `north` (0).
///
/// Not hypothetical -- it is the bug this test was written to catch, present
/// since the fixture was first authored in `4d597c1c`. `SmeltRow24` carried
/// 2.x values under a **1.1** stamp, so every south-facing arm decoded as
/// north-facing: the northern half of the block ran inside out while the
/// southern half was correct. Nothing else could see it. The entity count is
/// unchanged, every arm still lands on a legal tile beside a real machine,
/// `blueprint_power` never reads direction, and a live run reads
/// `waiting_for_source_items` -- which is also what an unfed block reads.
#[test]
fn the_saturating_modules_output_arms_face_the_furnace_they_empty() {
    let src = std::fs::read_to_string(rcontest_path()).expect("rcontest.lua readable");
    let (_, text) = blueprint_assignments(&src)
        .into_iter()
        .find(|(n, _)| n == "SmeltRow24")
        .expect("SmeltRow24 in rcontest.lua");
    let bp = decode(&text).expect("SmeltRow24 decodes");

    // The north furnace row is centred on y = -2.0 and spans y[-3, -1]; the
    // output belt above it is at y = -4.5. The arms between them sit at
    // y = -3.5 and must pick up from the SOUTH.
    let north: Vec<_> = bp
        .entities
        .iter()
        .filter(|e| e.name == "inserter" && (e.offset.y() + 3.5).abs() < 1e-9)
        .collect();
    assert_eq!(north.len(), 12, "one output arm per furnace in the north row");
    for arm in &north {
        assert_eq!(
            arm.direction, 8,
            "the arm at ({}, {}) must face SOUTH into the furnace it empties; \
             facing north it lifts plates off the output belt and feeds them \
             back into the machine. Blueprint version stamp is {} -- if that is \
             pre-2.0, the migration has doubled every direction in the block",
            arm.offset.x(),
            arm.offset.y(),
            bp.version
        );
    }

    // The matching row on the south side must face NORTH, for the same reason.
    // Asserting both is what makes this a test about FLOW rather than about
    // one constant: a uniform rotation moves them together, so two rows that
    // end up pointing the same way is the tell.
    let south: Vec<_> = bp
        .entities
        .iter()
        .filter(|e| e.name == "inserter" && (e.offset.y() - 4.5).abs() < 1e-9)
        .collect();
    assert_eq!(south.len(), 12, "one per furnace in the south row");
    for arm in &south {
        assert_eq!(arm.direction, 0, "the south row's output arms face NORTH");
    }
    assert_ne!(
        north[0].direction, south[0].direction,
        "the two output rows empty their furnaces in OPPOSITE directions; if \
         they agree, the whole block has been rotated by the version migration"
    );
}
