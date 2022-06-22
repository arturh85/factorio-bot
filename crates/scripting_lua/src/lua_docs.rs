use crate::lua_plan_builder::create_lua_plan_builder;
use crate::lua_rcon::create_lua_rcon;
use crate::lua_world::create_lua_world;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::rlua;
use factorio_bot_core::rlua::{Lua, Table};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub fn write_lua_docs(target_path: PathBuf) -> rlua::Result<()> {
    let lua = Lua::new();
    lua.context::<_, rlua::Result<()>>(|ctx| {
        let world = Arc::new(FactorioWorld::new());
        let rcon = Arc::new(FactorioRcon::new_empty());
        let planner = Planner::new(world, None);
        let world = create_lua_world(ctx, planner.plan_world.clone()).unwrap();
        let plan = create_lua_plan_builder(ctx, planner.graph.clone(), planner.plan_world.clone())
            .unwrap();
        let rcon = create_lua_rcon(ctx, rcon, planner.real_world).unwrap();

        write_lua_doc(target_path.join("world.lua"), &world);
        write_lua_doc(target_path.join("plan.lua"), &plan);
        write_lua_doc(target_path.join("rcon.lua"), &rcon);
        Ok(())
    })?;
    Ok(())
}

fn write_lua_doc(target_path: PathBuf, doc_table: &Table) {
    let mut body = doc_table
        .get::<String, String>("__doc__header".to_string())
        .unwrap()
        .trim()
        .to_string();
    body += "\n\n";

    for result in doc_table.clone().pairs::<String, String>() {
        if let Ok((key, value)) = result.as_ref() {
            if key.starts_with("__doc_fn") {
                body += value.trim();
                body += "\n\n"
            }
        }
    }
    body += doc_table
        .get::<String, String>("__doc__footer".to_string())
        .unwrap()
        .trim();
    body += "\n";

    fs::write(target_path, body).expect("failed to write");
}
