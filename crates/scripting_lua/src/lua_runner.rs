use crate::globals::create_lua_globals;
use crate::globals::plan::create_lua_plan_builder;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::world::create_lua_world;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::mlua::LuaSerdeExt;
use factorio_bot_core::paris::error;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_core::{mlua, serde_json};
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use miette::{IntoDiagnostic, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

pub async fn run_lua(
    planner: &mut Planner,
    lua_code: &str,
    filename: Option<&str>,
    bot_count: u8,
    redirect: bool,
) -> Result<(Option<serde_json::Value>, (String, String))> {
    let buffers = redirect_buffers(redirect);
    let stdout: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let filename = filename.unwrap_or("unknown.lua").to_owned();
    let cwd = Path::new(&filename)
        .parent()
        .expect("failed to find cwd")
        .canonicalize()
        .expect("failed to canonicalize");
    let mut code_by_path: HashMap<String, String> = HashMap::new();
    code_by_path.insert(filename.clone(), lua_code.to_owned());
    let code_by_path: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(code_by_path));
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let lua_code = lua_code.to_owned();
    let filename = filename.to_owned();

    let plan_world = planner.plan_world.clone();
    let graph = planner.graph.clone();
    let real_world = planner.real_world.clone();
    let rcon = planner.rcon.clone();
    let cwd_buf = cwd.to_path_buf();

    let thread_stdout = stdout.clone();
    let thread_stderr = stderr.clone();

    let result = thread::spawn(move || {
        let lua = Lua::new();
        let _code_by_path = code_by_path.clone();
        let world = create_lua_world(&lua, plan_world.clone(), cwd_buf).unwrap();
        let plan = create_lua_plan_builder(&lua, graph, plan_world).unwrap();
        create_lua_globals(
            &lua,
            all_bots,
            cwd.clone(),
            thread_stdout,
            thread_stderr,
            _code_by_path,
        ).unwrap();

        let globals = lua.globals();
        globals.set("world", world).unwrap();
        globals.set("plan", plan).unwrap();
        if let Some(rcon) = rcon.as_ref() {
            let rcon = create_lua_rcon(&lua, rcon.clone(), real_world.clone()).unwrap();
            globals.set("rcon", rcon).unwrap();
        }

        let rt: Runtime = Runtime::new().unwrap();
        rt.block_on(async {
            let chunk = lua.load(&lua_code).set_name(&filename);
            match chunk.exec_async().await {
                Ok(_) => {
                    let result: Option<LuaValue> = lua.globals().get("result").ok();
                    Ok(result.map(|r| lua.from_value(r).unwrap()))
                }
                Err(err) => {
                    let code_by_path = code_by_path.lock().clone();
                    Err(crate::error::to_lua_error(err, &code_by_path))
                }
            }
        })
    })
    .join()
    .unwrap()?;
    let stdout: String = stdout.lock().to_owned();
    let stderr: String = stderr.lock().to_owned();
    let buffers = buffers_to_string(&stdout, &stderr, buffers)?;
    Ok((result, buffers))
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::serde_json::json;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;
    use tokio::fs;

    use super::*;

    #[tokio::test]
    async fn test_script() {
        let world = Arc::new(fixture_world());
        // draw_world(world.clone(), "tests/world_start.png");

        let tests_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("script.lua");
        let tests_path = tests_path.to_str().unwrap();

        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_str().unwrap().to_owned();
        let relative_path = tests_path.replace(&cwd, "");
        let relative_path = &relative_path[1..];

        for bot_count in 1..=2 {
            let mut planner = Planner::new(world.clone(), None);
            let (_result, (stdout, stderr)) = run_lua(
                &mut planner,
                include_str!("../tests/script.lua"),
                Some(relative_path),
                bot_count,
                false,
            )
            .await
            .expect("run_lua failed");

            fs::write(
                format!(
                    "{}/tests/stdout-{}.txt",
                    env!("CARGO_MANIFEST_DIR"),
                    bot_count
                ),
                stdout,
            )
            .await
            .expect("failed to write");

            if !stderr.is_empty() {
                fs::write(
                    format!(
                        "{}/tests/stderr-{}.txt",
                        env!("CARGO_MANIFEST_DIR"),
                        bot_count
                    ),
                    stderr,
                )
                .await
                .expect("failed to write");
            }
        }
    }

    #[tokio::test]
    async fn test_free_rect_from_center() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=0})
"#,
            json!({
                "left_top": {"x": -36.0, "y": 36.0},
                "right_bottom": {"x": -34.0, "y": 38.0}
            }),
        )
        .await
    }

    #[tokio::test]
    async fn test_free_rect_from_top() {
        result_test(
            1,
            r#"
result = world.find_free_resource_rect("iron-ore", 2, 2, {x=0,y=-200})
"#,
            json!({
                "left_top": {"x": -37.0, "y": 35.0},
                "right_bottom": {"x": -35.0, "y": 37.0}
            }),
        )
        .await
    }

    async fn result_test(bot_count: u8, code: &str, expected: serde_json::Value) {
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (result, _) = run_lua(&mut planner, code, Some("./test.lua"), bot_count, false)
            .await
            .expect("run_lua failed");

        let actual = serde_json::to_string(&result.expect("no result found")).unwrap();
        let expected = serde_json::to_string(&expected).unwrap();

        assert_eq!(actual, expected);
    }
}
