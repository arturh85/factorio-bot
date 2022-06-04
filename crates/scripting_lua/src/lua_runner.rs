use crate::lua_plan_builder::create_lua_plan_builder;
use crate::lua_rcon::create_lua_rcon;
use crate::lua_world::create_lua_world;
use factorio_bot_core::factorio::rcon::RconSettings;
use factorio_bot_core::graph::task_graph::TaskGraph;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::process::instance_setup::setup_factorio_instance;
use factorio_bot_core::process::process_control::{start_factorio_server, FactorioStartCondition};
use factorio_bot_core::settings::FactorioSettings;
use gag::BufferRedirect;
use itertools::Itertools;
use miette::{IntoDiagnostic, Result};
use rlua::{Lua, Variadic};
use rlua_async::ChunkExt;
use std::fs::read_to_string;
use std::io::{stdout, Read, Write};
use std::path::Path;
use std::time::Instant;
use tokio::runtime::Runtime;

pub fn run_lua(planner: &mut Planner, lua_code: &str, bot_count: u32) -> Result<(String, String)> {
    let mut stdout = BufferRedirect::stdout().into_diagnostic()?;
    let mut stderr = BufferRedirect::stderr().into_diagnostic()?;
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let lua = Lua::new();
    if let Err(err) = lua.context::<_, rlua::Result<()>>(|ctx| {
        let world = create_lua_world(ctx, planner.plan_world.clone())?;
        let plan = create_lua_plan_builder(ctx, planner.graph.clone(), planner.plan_world.clone())?;
        let globals = ctx.globals();
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
        let chunk = ctx.load(lua_code);
        let rt = Runtime::new().unwrap();
        rt.block_on(chunk.exec_async(ctx))?;
        Ok(())
    }) {
        error!("{}", err)
    }
    let mut stdout_str = String::new();
    let mut stderr_str = String::new();
    stdout.read_to_string(&mut stdout_str).into_diagnostic()?;
    stderr.read_to_string(&mut stderr_str).into_diagnostic()?;
    Ok((stdout_str, stderr_str))
}

pub async fn start_factorio_and_plan_graph(
    settings: &FactorioSettings,
    map_exchange_string: Option<String>,
    seed: Option<String>,
    plan_name: &str,
    bot_count: u32,
) -> Result<TaskGraph> {
    let started = Instant::now();
    let instance_name = "plan";
    let rcon_settings = RconSettings::new(settings.rcon_port as u16, &settings.rcon_pass, None);
    setup_factorio_instance(
        &settings.workspace_path,
        &settings.factorio_archive_path,
        &rcon_settings,
        None,
        instance_name,
        true,
        true,
        map_exchange_string,
        seed,
        true,
    )
    .await
    .expect("failed to initially setup instance");

    let (world, rcon, mut child) = start_factorio_server(
        &settings.workspace_path,
        &rcon_settings,
        None,
        instance_name,
        // None,
        false,
        true,
        FactorioStartCondition::DiscoveryComplete,
    )
    .await
    .expect("failed to start");

    // Use asynchronous stdin
    info!("start took <yellow>{:?}</>", started.elapsed());
    let graph = loop {
        let started = Instant::now();
        let mut planner = Planner::new(world.clone(), Some(rcon.clone()));
        let lua_path_str = format!("plans/{}.lua", plan_name);
        let lua_path = Path::new(&lua_path_str);
        let lua_path = std::fs::canonicalize(lua_path).into_diagnostic()?;
        if !lua_path.exists() {
            panic!("plan {} not found at {}", plan_name, lua_path_str);
        }
        let lua_code = read_to_string(lua_path).into_diagnostic()?;
        match std::thread::spawn(move || {
            if let Err(err) = run_lua(&mut planner, &lua_code, bot_count) {
                Err(err)
            } else {
                Ok(planner)
            }
        })
        .join()
        .unwrap()
        {
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
        }
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

    child.kill().expect("failed to kill child");
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::test_utils::{draw_world, fixture_world};
    use serial_test::serial;
    use std::sync::Arc;

    use super::*;

    #[test]
    #[serial]
    fn test_draw_world() {
        let world = Arc::new(fixture_world());
        draw_world(world);
    }
    // ! gag does not work properly with test runners: https://crates.io/crates/gag
    // ! > Won't work in rust test cases.
    // ! > The rust test cases use std::io::set_print to redirect stdout. You can get around this
    // ! > though by using the --nocapture argument when running your tests.
    // #[test]
    // #[serial]
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

    #[test]
    #[serial]
    fn test_mining() {
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        run_lua(
            &mut planner,
            r##"
    plan.groupStart("Mine Stuff")
    for idx,playerId in pairs(all_bots) do
        plan.mine(playerId, {x=idx * 10,y=43}, "rock-huge", 1)
    end
    plan.groupEnd()
        "##,
            2,
        )
        .unwrap();
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: Mine Stuff" ]
    3 [ label = "Walk to [10, 43]" ]
    4 [ label = "Mining rock-huge" ]
    5 [ label = "Walk to [20, 43]" ]
    6 [ label = "Mining rock-huge" ]
    7 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "45" ]
    2 -> 5 [ label = "48" ]
    3 -> 4 [ label = "3" ]
    4 -> 7 [ label = "3" ]
    5 -> 6 [ label = "3" ]
    6 -> 7 [ label = "0" ]
    7 -> 1 [ label = "0" ]
}
"#,
        );
    }
}
