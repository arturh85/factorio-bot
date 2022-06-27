use crate::globals::create_lua_globals;
use crate::globals::plan::create_lua_plan_builder;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::world::create_lua_world;
use factorio_bot_core::paris::error;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::rlua::Lua;
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_core::{rlua, serde_json};
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use miette::Result;
use rlua_async::ChunkExt;
use std::path::Path;
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
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let lua = Lua::new();
    if let Err(err) = lua.context::<_, rlua::Result<()>>(|ctx| {
        let world = create_lua_world(ctx, planner.plan_world.clone(), cwd.to_path_buf())?;
        let plan = create_lua_plan_builder(ctx, planner.graph.clone(), planner.plan_world.clone())?;
        create_lua_globals(ctx, all_bots, cwd.clone(), stdout.clone(), stderr.clone())?;

        let globals = ctx.globals();
        globals.set("world", world)?;
        globals.set("plan", plan)?;
        if let Some(rcon) = planner.rcon.as_ref() {
            let rcon = create_lua_rcon(ctx, rcon.clone(), planner.real_world.clone())?;
            globals.set("rcon", rcon)?;
        }
        Ok(())
    }) {
        error!("error setting up context: {}", err)
    }

    let lua_code = lua_code.to_owned();
    let result = thread::spawn(move || {
        match lua.context::<_, rlua::Result<Option<serde_json::Value>>>(|ctx| {
            let chunk = ctx.load(&lua_code);
            let rt: Runtime = Runtime::new().unwrap();
            rt.block_on(chunk.exec_async(ctx))?;
            let result: Option<rlua::Value> = ctx.globals().get("result")?;

            Ok(result.map(|r| rlua_serde::from_value(r).unwrap()))
        }) {
            Ok(result) => Ok(result),
            Err(err) => Err(crate::error::to_lua_error(err, &filename, &lua_code)),
        }
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

    // #[test]
    // fn test_dumped_world() {
    // use factorio_bot_core::factorio::world::FactorioWorld;
    // use factorio_bot_core::serde_json;
    //     let world: FactorioWorld =
    //         serde_json::from_str(include_str!("../tests/dump.json")).unwrap();
    //     let world = Arc::new(world);
    //     draw_world(world, "tests/dump.png");
    // }

    // ! gag does not work properly with test runners: https://crates.io/crates/gag
    // ! > Won't work in rust test cases.
    // ! > The rust test cases use std::io::set_print to redirect stdout. You can get around this
    // ! > though by using the --nocapture argument when running your tests.
    // #[test]
    // fn test_logging_world() {
    //     let world = Arc::new(fixture_world());
    //     let mut planner = Planner::new(world, None);
    //     let (stdout, _) = planner
    //         .plan(
    //             r##"
    // print("teststring");
    // "##,
    //             0,
    //         )
    //         .unwrap();
    //     assert!(stdout.contains("teststring"));
    // }

    #[tokio::test]
    async fn test_script() {
        let world = Arc::new(fixture_world());
        // draw_world(world.clone(), "tests/world_start.png");

        let tests_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("script.lua");

        for bot_count in 1..=1 {
            let mut planner = Planner::new(world.clone(), None);
            let (_result, (stdout, stderr)) = run_lua(
                &mut planner,
                include_str!("../tests/script.lua"),
                Some(tests_path.to_str().unwrap()),
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
