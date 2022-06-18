use crate::lua_plan_builder::create_lua_plan_builder;
use crate::lua_rcon::create_lua_rcon;
use crate::lua_world::create_lua_world;
use factorio_bot_core::graph::task_graph::TaskGraph;
use factorio_bot_core::paris::{error, info, warn};
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::process::process_control::{
    FactorioInstance, FactorioParams, FactorioStartCondition,
};
use factorio_bot_core::rlua::{Lua, Variadic};
use factorio_bot_core::settings::FactorioSettings;
use factorio_bot_core::tokio::runtime::Runtime;
use factorio_bot_core::{rlua, serde_json};
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use itertools::Itertools;
use miette::{IntoDiagnostic, Result};
use rlua_async::ChunkExt;
use std::fs::read_to_string;
use std::io::{stdout, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Instant;

pub async fn run_lua(
    planner: &mut Planner,
    lua_code: &str,
    filename: Option<&str>,
    bot_count: u8,
    redirect: bool,
) -> Result<(Option<serde_json::Value>, (String, String))> {
    let buffers = redirect_buffers(redirect);
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
        globals.set(
            "print",
            ctx.create_function(|_, strings: Variadic<String>| {
                info!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
                Ok(())
            })?,
        )?;
        globals.set(
            "print_err",
            ctx.create_function(|_, strings: Variadic<String>| {
                error!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
                Ok(())
            })?,
        )?;
        globals.set(
            "print_warn",
            ctx.create_function(|_, strings: Variadic<String>| {
                warn!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
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
    let buffers = buffers_to_string(buffers)?;
    Ok((result, buffers))
}

#[allow(dead_code)]
pub async fn start_factorio_and_plan_graph(
    settings: &FactorioSettings,
    map_exchange_string: Option<String>,
    seed: Option<String>,
    plan_name: &str,
    bot_count: u8,
) -> Result<TaskGraph> {
    let started = Instant::now();
    let instance_name = "plan";
    let params = FactorioParams {
        seed,
        map_exchange_string,
        instance_name: Some(instance_name.to_owned()),
        wait_until: FactorioStartCondition::DiscoveryComplete,
        ..FactorioParams::default()
    };
    let instance = FactorioInstance::start(settings, params)
        .await
        .expect("failed to start");

    // Use asynchronous stdin
    info!("start took <yellow>{:?}</>", started.elapsed());
    let graph = loop {
        let started = Instant::now();
        let mut planner = Planner::new(
            instance.world.clone().expect("world missing").clone(),
            Some(instance.rcon.clone()),
        );
        let lua_path_str = format!("plans/{}.lua", plan_name);
        let lua_path = Path::new(&lua_path_str);
        let lua_path = std::fs::canonicalize(lua_path).into_diagnostic()?;
        if !lua_path.exists() {
            panic!("plan {} not found at {}", plan_name, lua_path_str);
        }
        let lua_code = read_to_string(lua_path).into_diagnostic()?;
        match if let Err(err) = run_lua(
            &mut planner,
            &lua_code,
            Some(&lua_path_str),
            bot_count,
            false,
        )
        .await
        {
            Err(err)
        } else {
            Ok(planner)
        } {
            Ok(_planner) => planner = _planner,
            Err(err) => {
                error!("executation failed: {:?}", err);
                warn!("enter [q] to quit or any other key to restart plan",);
                let input: Option<i32> = std::io::stdin()
                    .bytes()
                    .next()
                    .and_then(|result| result.ok())
                    .map(|byte| byte as i32);

                if let Some(key) = input {
                    if key == 113 {
                        panic!("aborted")
                    }
                }
                info!("started");
                stdout().flush().into_diagnostic()?;
                continue;
            }
        };
        let world = planner.world();
        let graph = planner.graph();
        // for player in world.players.iter() {
        //     info!(
        //         "bot #{} endet up at {} with inventory: {:?}",
        //         player.player_id, player.position, player.main_inventory
        //     );
        // }
        // if let Some(resources) = &world.resources.read() {
        //     for (name, _) in resources {
        //         let patches = world.resource_patches(&name);
        //         for patch in patches {
        //             info!(
        //                 "{} patch at {} size {}",
        //                 patch.name,
        //                 patch.rect.center(),
        //                 patch.elements.len()
        //             );
        //         }
        //     }
        // }

        // let process_start = graph.node_indices().next().unwrap();
        // let process_end = graph.node_indices().last().unwrap();
        // let (weight, _) = graph
        //     .astar(process_start, process_end)
        //     .expect("no path found");
        // info!("shortest path: {}", weight);

        world.entity_graph.connect().unwrap();
        world.flow_graph.update().unwrap();
        // graph.print();
        println!("{}", graph.graphviz_dot());
        println!("{}", world.entity_graph.graphviz_dot());
        println!("{}", world.flow_graph.graphviz_dot());
        info!("planning took <yellow>{:?}</>", started.elapsed());
        warn!("enter [q] to quit or any other key to restart plan",);

        let input: Option<i32> = std::io::stdin()
            .bytes()
            .next()
            .and_then(|result| result.ok())
            .map(|byte| byte as i32);

        if let Some(key) = input {
            if key == 113 {
                break graph;
            }
        }
    };

    instance.stop().expect("failed to kill child");
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::serde_json::json;
    use factorio_bot_core::test_utils::{draw_world, fixture_world};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn test_draw_world() {
        let world = Arc::new(fixture_world());
        draw_world(world, "tests/world.png");
    }

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
    async fn test_mining() {
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (_result, _) = run_lua(
            &mut planner,
            include_str!("../tests/script.lua"),
            Some("../tests/script.lua"),
            1,
            false,
        )
        .await
        .expect("run_lua failed");
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: Build Starter Miner/Furnace" ]
    3 [ label = "Walk to [-36, 36]" ]
    4 [ label = "Place burner-mining-drill at [-36, 36] (North)" ]
    5 [ label = "Place stone-furnace at [-36, 34] (North)" ]
    6 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "51" ]
    3 -> 4 [ label = "1" ]
    4 -> 5 [ label = "1" ]
    5 -> 6 [ label = "0" ]
    6 -> 1 [ label = "0" ]
}
"#,
        );
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

    async fn _graph_test(bot_count: u8, code: &str, expected: &str) {
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (_, _) = run_lua(&mut planner, code, None, bot_count, false)
            .await
            .expect("run_lua failed");

        let graph = planner.graph();
        assert_eq!(graph.graphviz_dot(), expected,);
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
