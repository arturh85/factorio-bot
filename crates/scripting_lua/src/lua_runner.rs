use crate::wrapper::lua_plan_builder::create_lua_plan_builder;
use crate::wrapper::lua_rcon::create_lua_rcon;
use crate::wrapper::lua_world::create_lua_world;
use factorio_bot_core::paris::{error, info, warn};
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::rlua::{Lua, Variadic};
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_core::{rlua, serde_json};
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use itertools::Itertools;
use miette::Result;
use rlua_async::ChunkExt;
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
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let lua = Lua::new();
    if let Err(err) = lua.context::<_, rlua::Result<()>>(|ctx| {
        let world = create_lua_world(ctx, planner.plan_world.clone())?;
        let plan = create_lua_plan_builder(ctx, planner.graph.clone(), planner.plan_world.clone())?;
        let globals = ctx.globals();
        globals.set("all_bot_count", all_bots.len())?;
        globals.set("all_bots", all_bots)?;
        globals.set("world", world)?;
        let _stdout = stdout.clone();
        globals.set(
            "print",
            ctx.create_function(move |_, strings: Variadic<String>| {
                info!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
                let mut stdout = _stdout.lock();
                *stdout += &strings.iter().join(" ");
                *stdout += "\n";
                Ok(())
            })?,
        )?;
        let _stderr = stderr.clone();
        globals.set(
            "print_err",
            ctx.create_function(move |_, strings: Variadic<String>| {
                error!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
                let mut stderr = _stderr.lock();
                *stderr += "ERROR: ";
                *stderr += &strings.iter().join(" ");
                *stderr += "\n";
                Ok(())
            })?,
        )?;
        let _stdout = stdout.clone();
        globals.set(
            "print_warn",
            ctx.create_function(move |_, strings: Variadic<String>| {
                warn!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
                let mut stdout = _stdout.lock();
                *stdout += "WARN: ";
                *stdout += &strings.iter().join(" ");
                *stdout += "\n";
                Ok(())
            })?,
        )?;
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
    let filename = filename.unwrap_or("unknown").to_owned();
    let result = thread::spawn(move || {
        match lua.context::<_, rlua::Result<Option<serde_json::Value>>>(|ctx| {
            let chunk = ctx.load(&lua_code);
            let rt = Runtime::new().unwrap();
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
    let stdout = stdout.lock().to_owned();
    let stderr = stderr.lock().to_owned();
    let buffers = buffers_to_string(&stdout, &stderr, buffers)?;
    Ok((result, buffers))
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::gantt_mermaid::to_mermaid_gantt;
    use factorio_bot_core::serde_json::json;
    use factorio_bot_core::test_utils::{draw_world, fixture_world};
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
        draw_world(world.clone(), "tests/world_start.png");

        for bot_count in 1..=1 {
            let mut planner = Planner::new(world.clone(), None);
            let (_result, (stdout, stderr)) = run_lua(
                &mut planner,
                include_str!("../tests/script.lua"),
                Some("../tests/script.lua"),
                bot_count,
                false,
            )
            .await
            .expect("run_lua failed");
            let graph = planner.graph();

            fs::write(
                format!(
                    "{}/tests/task_graph-{}.dot",
                    env!("CARGO_MANIFEST_DIR"),
                    bot_count
                ),
                graph.graphviz_dot(),
            )
            .await
            .expect("failed to write");

            let mut bot_ids = Vec::new();
            for i in 1..=bot_count {
                bot_ids.push(i);
            }

            fs::write(
                format!(
                    "{}/tests/task_graph-{}.md",
                    env!("CARGO_MANIFEST_DIR"),
                    bot_count
                ),
                format!(
                    "```mermaid\n{}\n```\n",
                    to_mermaid_gantt(&planner, bot_ids, &format!("{} bots", bot_count))
                ),
            )
            .await
            .expect("failed to write");
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

            draw_world(
                planner.plan_world.clone(),
                &format!(
                    "{}/tests/world_end-{}.png",
                    env!("CARGO_MANIFEST_DIR"),
                    bot_count
                ),
            );
        }
    }

    #[tokio::test]
    async fn test_free_rect_from_center() {
        result_test(
            1,
            r#"
result = world.findFreeResourceRect("iron-ore", 2, 2, {x=0,y=0})
"#,
            json!({
                "leftTop": {"x": -36.0, "y": 36.0},
                "rightBottom": {"x": -34.0, "y": 38.0}
            }),
        )
        .await
    }

    #[tokio::test]
    async fn test_free_rect_from_top() {
        result_test(
            1,
            r#"
result = world.findFreeResourceRect("iron-ore", 2, 2, {x=0,y=-200})
"#,
            json!({
                "leftTop": {"x": -37.0, "y": 35.0},
                "rightBottom": {"x": -35.0, "y": 37.0}
            }),
        )
        .await
    }

    async fn result_test(bot_count: u8, code: &str, expected: serde_json::Value) {
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (result, _) = run_lua(&mut planner, code, None, bot_count, false)
            .await
            .expect("run_lua failed");

        let actual = serde_json::to_string(&result.expect("no result found")).unwrap();
        let expected = serde_json::to_string(&expected).unwrap();

        assert_eq!(actual, expected);
    }
}
