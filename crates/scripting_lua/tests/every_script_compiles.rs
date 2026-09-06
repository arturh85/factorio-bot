//! Every `scripts/*.lua` in the repo must compile as Lua 5.4.
//!
//! **This exists because one of them did not, and the test written to guard
//! that very file did not notice.** `scripts/rcontest.lua` shipped for a day
//! with no comma after the `MovingBlock` entry, so its table constructor never
//! closed:
//!
//! ```text
//! scripts/rcontest.lua:20: '}' expected (to close '{' at line 1) near 'SmeltingBlock'
//! ```
//!
//! The file was already covered by
//! `crates/core/tests/rcontest_blueprints_decode.rs`, which was itself written
//! after a corrupt blueprint survived because its test decoded an inline copy
//! rather than the file. That test does read the real file — but it finds
//! blueprints with a **regex over the text** and decodes those. A regex over
//! broken Lua matches exactly as well as a regex over working Lua, so the
//! guard stayed green while the fixture it guards could not be loaded at all.
//!
//! The lesson generalises past that one file: reading a source file as *text*
//! tells you nothing about whether it is *valid*. So this compiles every
//! script with the same interpreter that runs them.
//!
//! Compiling is not executing. `load` resolves no globals, so a script calling
//! `rcon`, `world` or `goal` compiles fine here without a sandbox, a game, or
//! any host functions — this checks syntax, which is precisely the class of
//! defect that a text-scanning test cannot see.

use mlua::Lua;
use std::path::PathBuf;

fn scripts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/scripting_lua -> crates -> repo root")
        .join("scripts")
}

#[test]
fn every_script_in_the_repo_compiles_as_lua() {
    let dir = scripts_dir();
    let lua = Lua::new();

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lua"))
        .collect();
    paths.sort();

    // If the directory moves or empties, this test would otherwise pass while
    // checking nothing -- the same failure it was written to prevent, one
    // level further out.
    assert!(
        paths.len() >= 30,
        "expected the repo's script library in {}, found {} files -- if the \
         layout changed, fix this test rather than letting it quietly check \
         nothing",
        dir.display(),
        paths.len()
    );

    let mut broken = Vec::new();
    for path in &paths {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        // `into_function` compiles without running. A script that calls host
        // functions this crate has not installed still compiles.
        if let Err(e) = lua.load(&src).set_name(name).into_function() {
            broken.push(format!("{name}: {e}"));
        }
    }

    assert!(
        broken.is_empty(),
        "these scripts do not compile:\n  {}\n\nA script that does not parse \
         fails at the moment a run loads it, not at build time -- and a test \
         that scans its text with a regex cannot tell the difference.",
        broken.join("\n  ")
    );
}
