use crate::globals::create_lua_globals;
use crate::globals::goal::create_lua_goal;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::world::create_lua_world;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub fn write_lua_docs(target_path: PathBuf) -> LuaResult<()> {
    let lua = crate::sandbox::new_sandboxed_lua()?;
    let world = Arc::new(FactorioWorld::new());
    let rcon = Arc::new(FactorioRcon::new_empty());
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    let planner = Planner::new(world, None);
    // Doc generation never executes a script, so the sandbox root only has to
    // be a real directory; the bindings are introspected, not called.
    let cwd = target_path.parent().unwrap_or(&target_path).to_path_buf();
    let world_table = create_lua_world(&lua, planner.plan_world.clone(), cwd.clone(), cwd.clone())?;
    let goal_table = create_lua_goal(
        &lua,
        planner.plan_world.clone(),
        planner.real_world.clone(),
        None,
        vec![],
    )?;
    let rcon_table = create_lua_rcon(&lua, rcon, planner.real_world)?;
    let code_by_path: HashMap<String, String> = HashMap::new();
    let code_by_path: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(code_by_path));
    create_lua_globals(
        &lua,
        vec![],
        cwd.clone(),
        cwd,
        stdout,
        stderr,
        code_by_path,
        None,
    )?;

    write_lua_doc(target_path.join("globals.lua"), &lua.globals());
    write_lua_doc(target_path.join("world.lua"), &world_table);
    write_lua_doc(target_path.join("goal.lua"), &goal_table);
    write_lua_doc(target_path.join("rcon.lua"), &rcon_table);
    Ok(())
}

/// Collects a module table's `__doc_entry_*` strings, sorted by key.
///
/// The obvious spelling — `pairs::<String, String>().flatten()` — silently
/// truncated every module, because these tables hold the *bound functions*
/// beside the documentation strings. mlua's `TablePairs::next` opens with
/// `self.key.take()` and restores the cursor only on the success arm
/// (`mlua-0.12.0/src/table.rs:1369-1402`); a conversion error is yielded once
/// and every later call falls into `else { None }`. So the first function
/// value *ended* iteration rather than being skipped, and `.flatten()` turned
/// that into silence — a stopped iterator and an exhausted one look the same.
/// The published docs had been a prefix of the real API for as long as this
/// existed, with a green build the whole time.
///
/// Reading both halves as `LuaValue` is infallible, so nothing can terminate
/// the walk early; a non-string value is simply not documentation.
///
/// Sorting is not cosmetic. Lua 5.4 seeds its string hashes per interpreter
/// state, so `pairs` visits a table in a different order in every `Lua`
/// instance — two generations inside one process disagreed. That made the
/// generated files irreproducible and, worse, made the truncation look like
/// churn rather than a bug.
fn doc_entries(doc_table: &LuaTable) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for pair in doc_table.clone().pairs::<LuaValue, LuaValue>() {
        let Ok((key, value)) = pair else { continue };
        let (Some(key), Some(value)) = (key.as_string(), value.as_string()) else {
            continue;
        };
        let key = key.to_string_lossy();
        if key.starts_with("__doc_entry_") {
            entries.push((key, value.to_string_lossy()));
        }
    }
    entries.sort();
    entries
}

fn write_lua_doc(target_path: PathBuf, doc_table: &LuaTable) {
    let mut body = doc_table
        .get::<String>("__doc__header")
        .unwrap_or_default()
        .trim()
        .to_string();
    body += "\n\n";

    for (_, value) in doc_entries(doc_table) {
        body += value.trim();
        body += "\n\n"
    }
    body += doc_table
        .get::<String>("__doc__footer")
        .unwrap_or_default()
        .trim();
    body += "\n";

    fs::write(target_path, body).expect("failed to write");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binding that carries a `__doc_entry_*` string, spelled the way it
    /// appears in the rendered file, **in the order the generator must emit
    /// them** — sorted by `__doc_entry_*` key, which is why `Direction` leads
    /// `all_bots` (ASCII `D` < `a`) and `direction_opposite` leads
    /// `directions_all` (`_` < `s`).
    ///
    /// This list is deliberately exhaustive rather than a spot check. The
    /// generator's failure mode was *truncation* — it emitted a prefix of each
    /// module and stopped — so a test that asserted one entry per file would
    /// have passed against the broken generator whenever that entry happened
    /// to land first. Requiring all of them means a truncated file always
    /// names something it is missing.
    const EXPECTED: &[(&str, &[&str])] = &[
        (
            "globals.lua",
            &[
                "globals.Direction = {",
                "globals.all_bots",
                "function globals.direction_clockwise(",
                "function globals.direction_opposite(",
                "function globals.directions_all(",
                "function globals.directions_orthogonal(",
                "function globals.file_read(",
                "function globals.file_write(",
                "function globals.include(",
                "function globals.print(",
                "function globals.print_err(",
                "function globals.print_warn(",
            ],
        ),
        (
            "world.lua",
            &[
                "function world.draw(",
                "function world.find_entities_in_radius(",
                "function world.find_free_resource_rect(",
                "function world.inventory(",
                "function world.parse_blueprint(",
                "function world.player(",
                "function world.recipe(",
            ],
        ),
        (
            "rcon.lua",
            &[
                "function rcon.add_research(",
                "function rcon.cheat_all_technologies(",
                "function rcon.cheat_blueprint(",
                "function rcon.cheat_item(",
                "function rcon.cheat_technology(",
                "function rcon.craft(",
                "function rcon.find_entities_in_radius(",
                "function rcon.insert_to_inventory(",
                "function rcon.inventory_contents_at(",
                "function rcon.mine(",
                "function rcon.move(",
                "function rcon.place_blueprint(",
                "function rcon.place_entity(",
                "function rcon.print(",
                "function rcon.remove_from_inventory(",
                "function rcon.revive_ghost(",
            ],
        ),
        (
            "goal.lua",
            &[
                "function goal.all(",
                "function goal.have(",
                "function goal.plan(",
                "function goal.researched(",
                "function goal.run(",
                "function goal.start(",
            ],
        ),
    ];

    /// `goal.lua`, in **both** directions.
    ///
    /// [`every_documented_binding_reaches_the_generated_file`] asserts only
    /// that each listed entry is present, so it catches a function that is
    /// *removed* and stays silent on one that is *added* — the list is then a
    /// mirror of the code rather than a check on it, and it stays green when
    /// a seventh function appears. This compares the set the generator
    /// actually emitted against the set that is meant to exist, so an
    /// undocumented addition fails as loudly as a lost entry.
    ///
    /// Only `goal.lua`: it is the file this change churns, and the other
    /// three carry longer lists that nothing here is renaming.
    #[test]
    fn the_goal_doc_entries_are_exactly_the_goal_surface() {
        use std::collections::BTreeSet;

        let (_dir, target) = generate();
        let body = fs::read_to_string(target.join("goal.lua")).expect("goal.lua");
        let emitted: BTreeSet<&str> = body
            .lines()
            .filter_map(|line| line.strip_prefix("function goal."))
            .filter_map(|rest| rest.split('(').next())
            .collect();
        let expected: BTreeSet<&str> = ["all", "have", "plan", "researched", "run", "start"].into();
        assert_eq!(
            emitted, expected,
            "goal.lua doc entries drifted from the goal table"
        );
    }

    fn generate() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("lua");
        fs::create_dir_all(&target).expect("create target");
        write_lua_docs(target.clone()).expect("write_lua_docs");
        (dir, target)
    }

    /// The generator walks tables that hold the bound *functions* beside the
    /// documentation strings. Nothing in the build reads these files, so a
    /// generator that drops entries produces a green build and empty docs;
    /// this is the only thing that contradicts it.
    #[test]
    fn every_documented_binding_reaches_the_generated_file() {
        let (_dir, target) = generate();
        let mut missing: Vec<String> = Vec::new();
        for (file, entries) in EXPECTED {
            let path = target.join(file);
            let body =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            for entry in *entries {
                if !body.contains(entry) {
                    missing.push(format!("{file} is missing `{entry}`"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} documentation entries never reached the generated files:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    /// The generated files must be a function of the source, not of Lua's
    /// hash seed — so the entries appear in sorted key order, always.
    ///
    /// This asserts the ordering *directly*, by position in the file, because
    /// the obvious test for it cannot be trusted. See
    /// [`generated_files_are_ordered_deterministically`] for what went wrong
    /// with the obvious one; the short version is that comparing two
    /// generations only discriminates when the two interpreters happen to draw
    /// different seeds, which is a property of the allocator and the clock
    /// rather than of anything the test does. Reading positions out of one
    /// file draws no entropy at all: if `entries.sort()` goes, the emitted
    /// order becomes Lua's hash order, and hash order matching sorted order
    /// for twelve keys is 1 in 12! — repeated independently across four files.
    #[test]
    fn entries_are_emitted_in_sorted_key_order() {
        let (_dir, target) = generate();
        for (file, entries) in EXPECTED {
            let body = fs::read_to_string(target.join(file)).expect("read generated file");
            let mut previous: Option<(&str, usize)> = None;
            for entry in *entries {
                let at = body
                    .find(entry)
                    .unwrap_or_else(|| panic!("{file} is missing `{entry}`"));
                if let Some((earlier, earlier_at)) = previous {
                    assert!(
                        earlier_at < at,
                        "{file} emits `{entry}` before `{earlier}`; entries must be \
                         sorted by their `__doc_entry_*` key, and are not when the \
                         generator leaks Lua's per-state hash order into the output"
                    );
                }
                previous = Some((entry, at));
            }
        }
    }

    /// Two generations in one process agree.
    ///
    /// **This test guards weakly and cannot be made to guard strongly.** It is
    /// kept because it states the user-visible property — build twice, get the
    /// same file — and removed reliance on it is exactly what
    /// [`entries_are_emitted_in_sorted_key_order`] is for.
    ///
    /// The trap, which review caught: it only discriminates when the two
    /// interpreters draw *different* seeds. PUC Lua's `luai_makeseed` mixes the
    /// new `lua_State`'s heap address, a stack address and `time(NULL)`. Two
    /// back-to-back generations can agree on all three — a quiet allocator
    /// hands the second state the block the first just freed, and no clock
    /// second is crossed — and then the test passes with the sort deleted. On
    /// the reviewer's machine it did so 30 times out of 30 in isolation while
    /// failing 15 out of 15 under the full crate suite, where neighbouring
    /// tests perturb the allocator; on mine it failed 30 out of 30 in
    /// isolation. Same test, same mutation, opposite verdicts: its
    /// discriminating power was coming from the environment, not from itself.
    ///
    /// Holding the intervening states *alive* is what this can do about it:
    /// the first generation's freed block is occupied when the second
    /// generation allocates, so the addresses cannot coincide. That is a
    /// guarantee about the allocator, not about the clock, which is why it is
    /// the backup assertion and not the primary one.
    #[test]
    fn generated_files_are_ordered_deterministically() {
        let (_a, first) = generate();
        // Kept alive across the second `generate()` on purpose: dropping these
        // would hand the block straight back and restore the coincidence.
        let _occupy_the_freed_states: Vec<Lua> = (0..8)
            .map(|_| crate::sandbox::new_sandboxed_lua().expect("sandboxed lua"))
            .collect();
        let (_b, second) = generate();
        for (file, _) in EXPECTED {
            assert_eq!(
                fs::read_to_string(first.join(file)).expect("first"),
                fs::read_to_string(second.join(file)).expect("second"),
                "{file} differs between two generations"
            );
        }
    }

    /// The sandbox notes that Task 1 bounded are the part of these strings a
    /// reader has to see; assert they are actually in the shipped text rather
    /// than only in the source.
    #[test]
    fn sandbox_notes_are_documented() {
        let (_dir, target) = generate();
        let globals = fs::read_to_string(target.join("globals.lua")).expect("globals.lua");
        let world = fs::read_to_string(target.join("world.lua")).expect("world.lua");
        // Backticked, because the bare names are substrings of ordinary words
        // in this file — `direction` contains "io", `position` contains "os" —
        // and an assertion that passes on `direction` is worse than no
        // assertion, since it reads as coverage.
        for missing in [
            "`io`",
            "`os`",
            "`package`",
            "`require`",
            "`dofile`",
            "`loadfile`",
        ] {
            assert!(
                globals.contains(missing),
                "globals.lua does not mention that {missing} is unavailable"
            );
        }
        for body in [&globals, &world] {
            assert!(
                body.contains("scripts directory"),
                "a bounded binding does not say paths are relative to the scripts directory"
            );
        }
    }
}
