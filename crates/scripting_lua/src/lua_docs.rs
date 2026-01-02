use crate::globals::create_lua_globals;
use crate::globals::plan::create_lua_plan_builder;
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
    let lua = Lua::new();
    let world = Arc::new(FactorioWorld::new());
    let rcon = Arc::new(FactorioRcon::new_empty());
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    let planner = Planner::new(world, None);
    let cwd = target_path.parent().expect("failed to find parent");
    let world_table = create_lua_world(&lua, planner.plan_world.clone(), cwd.to_path_buf())?;
    let plan_table =
        create_lua_plan_builder(&lua, planner.graph.clone(), planner.plan_world.clone())?;
    let rcon_table = create_lua_rcon(&lua, rcon, planner.real_world)?;
    let code_by_path: HashMap<String, String> = HashMap::new();
    let code_by_path: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(code_by_path));
    create_lua_globals(
        &lua,
        vec![],
        cwd.to_path_buf(),
        stdout,
        stderr,
        code_by_path,
    )?;

    write_lua_doc(target_path.join("globals.lua"), &lua.globals());
    write_lua_doc(target_path.join("world.lua"), &world_table);
    write_lua_doc(target_path.join("plan.lua"), &plan_table);
    write_lua_doc(target_path.join("rcon.lua"), &rcon_table);
    Ok(())
}

fn write_lua_doc(target_path: PathBuf, doc_table: &LuaTable) {
    let mut body = doc_table
        .get::<String>("__doc__header")
        .unwrap_or_default()
        .trim()
        .to_string();
    body += "\n\n";

    for (key, value) in doc_table.clone().pairs::<String, String>().flatten() {
        if key.starts_with("__doc_entry_") {
            body += value.trim();
            body += "\n\n"
        }
    }
    body += doc_table
        .get::<String>("__doc__footer")
        .unwrap_or_default()
        .trim();
    body += "\n";

    fs::write(target_path, body).expect("failed to write");
}
