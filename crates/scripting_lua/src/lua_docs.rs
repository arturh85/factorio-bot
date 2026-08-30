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
    /// appears in the rendered file.
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
                "globals.all_bots",
                "globals.Direction = {",
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
                "function goal.execute(",
                "function goal.gantt(",
                "function goal.graphviz(",
                "function goal.have(",
                "function goal.progress(",
                "function goal.researched(",
                "function goal.schedule(",
                "function goal.wait(",
            ],
        ),
    ];

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
            let body = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
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

    /// Lua seeds its string hashes per interpreter state, so `pairs` visits a
    /// table in a different order in different processes. A generated file
    /// whose contents depend on that is not reproducible, and the ordering
    /// alone would have hidden the truncation bug from anyone comparing two
    /// runs. Generating twice in one process is the weaker half of this; the
    /// sort is what makes it hold across processes too.
    #[test]
    fn generated_files_are_ordered_deterministically() {
        let (_a, first) = generate();
        let (_b, second) = generate();
        for (file, _) in EXPECTED {
            assert_eq!(
                fs::read_to_string(first.join(file)).expect("first"),
                fs::read_to_string(second.join(file)).expect("second"),
                "{file} differs between two generations"
            );
        }
    }
}
