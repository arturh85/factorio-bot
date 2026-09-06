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
        blueprints.len() >= 5,
        "expected at least the five known fixtures in {}, found {} -- if the \
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
