use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::io::{stdout, Read, Write};
use std::path::Path;
use std::time::Instant;

use crate::factorio::instance_setup::setup_factorio_instance;
use async_std::sync::Arc;
use dashmap::lock::RwLock;
use gag::BufferRedirect;
use rlua::Lua;
// use crate::factorio::plan_builder::create_lua_plan_builder;
use crate::factorio::process_control::{start_factorio_server, FactorioStartCondition};
use crate::factorio::rcon::{create_lua_rcon, FactorioRcon, RconSettings};
use crate::factorio::task_graph::TaskGraph;
use crate::factorio::world::{create_lua_world, FactorioWorld};
// use crate::factorio::ws::FactorioWebSocketServer;
use crate::factorio::plan_builder::create_lua_plan_builder;
use crate::settings::AppSettings;
use crate::types::{EntityName, PlayerChangedMainInventoryEvent};
use rlua_async::ChunkExt;

pub struct Planner {
    #[allow(dead_code)]
    rcon: Option<Arc<FactorioRcon>>,
    real_world: Arc<FactorioWorld>,
    plan_world: Arc<FactorioWorld>,
    graph: Arc<RwLock<TaskGraph>>,
}

impl Planner {
    pub fn new(world: Arc<FactorioWorld>, rcon: Option<Arc<FactorioRcon>>) -> Planner {
        let plan_world = (*world).clone();
        Planner {
            graph: Arc::new(RwLock::new(TaskGraph::new())),
            rcon,
            real_world: world,
            plan_world: Arc::new(plan_world),
        }
    }

    pub fn reset(&mut self) {
        let plan_world = (*self.real_world).clone();
        self.plan_world = Arc::new(plan_world);
        self.graph = Arc::new(RwLock::new(TaskGraph::new()));
    }

    pub fn plan(&mut self, lua_code: String, bot_count: u32) -> anyhow::Result<(String, String)> {
        let mut stdout = BufferRedirect::stdout()?;
        let mut stderr = BufferRedirect::stderr()?;
        let all_bots = self.initiate_missing_players_with_default_inventory(bot_count);
        self.plan_world = Arc::new((*self.real_world).clone());
        let lua = Lua::new();
        lua.context::<_, rlua::Result<()>>(|ctx| {
            let world = create_lua_world(ctx, self.plan_world.clone())?;
            let plan = create_lua_plan_builder(ctx, self.graph.clone(), self.plan_world.clone())?;
            let globals = ctx.globals();
            globals.set("all_bots", all_bots)?;
            globals.set("world", world)?;
            globals.set("plan", plan)?;
            if let Some(rcon) = self.rcon.as_ref() {
                let rcon = create_lua_rcon(ctx, rcon.clone(), self.real_world.clone())?;
                globals.set("rcon", rcon)?;
            }
            let chunk = ctx.load(&lua_code);
            async_std::task::block_on(chunk.exec_async(ctx))?;
            Ok(())
        })?;
        let mut stdout_str = String::new();
        let mut stderr_str = String::new();
        stdout.read_to_string(&mut stdout_str).unwrap();
        stderr.read_to_string(&mut stderr_str).unwrap();
        Ok((stdout_str, stderr_str))
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.plan_world.clone()
    }
    pub fn graph(&self) -> TaskGraph {
        self.graph.read().clone()
    }

    fn initiate_missing_players_with_default_inventory(&mut self, bot_count: u32) -> Vec<u32> {
        let mut player_ids: Vec<u32> = vec![];
        for player_id in 1u32..=bot_count {
            player_ids.push(player_id);
            // initialize missing players with default inventory
            if self.real_world.players.get(&player_id).is_none() {
                let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
                main_inventory.insert(EntityName::Wood.to_string(), 1);
                main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
                main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
                self.plan_world
                    .player_changed_main_inventory(PlayerChangedMainInventoryEvent {
                        player_id,
                        main_inventory: Box::new(main_inventory.clone()),
                    })
                    .expect("failed to set player inventory");
            }
        }
        player_ids
    }
}

pub async fn start_factorio_and_plan_graph(
    settings: AppSettings,
    map_exchange_string: Option<String>,
    seed: Option<String>,
    plan_name: &str,
    bot_count: u32,
) -> anyhow::Result<TaskGraph> {
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
        info!("foo 1");
        let mut planner = Planner::new(world.clone(), Some(rcon.clone()));
        info!("foo 2");
        let lua_path_str = format!("plans/{}.lua", plan_name);
        let lua_path = Path::new(&lua_path_str);
        let lua_path = std::fs::canonicalize(lua_path)?;
        if !lua_path.exists() {
            anyhow::bail!("plan {} not found at {}", plan_name, lua_path_str);
        }
        let lua_code = read_to_string(lua_path)?;
        info!("foo 3");
        match std::thread::spawn(move || {
            info!("foo 4");
            if let Err(err) = planner.plan(lua_code, bot_count) {
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
                        anyhow::bail!("aborted")
                    }
                }
                info!("started");
                stdout().flush()?;
                continue;
            }
        }
        info!("foo 5");
        let world = planner.world();
        info!("foo 6");
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

// pub async fn execute_node(node: NodeIndex<u32>) -> JoinHandle<NodeIndex<u32>> {}

pub fn execute_plan(
    _world: Arc<FactorioWorld>,
    _rcon: Arc<FactorioRcon>,
    // _websocket_server: Option<Addr<FactorioWebSocketServer>>,
    plan: TaskGraph,
) {
    // let queue = TaskQueue::<NodeIndex>::from_registry();
    // let _worker = TaskWorker::<NodeIndex, TaskResult>::new();

    let root = plan.node_indices().next().unwrap();

    let pointer = root;
    let _tick = 0;
    loop {
        // if let Some(websocket_server) = websocket_server.as_ref() {
        //     websocket_server
        //         .send(TaskStarted {
        //             node_id: pointer.index(),
        //             tick,
        //         })
        //         .await?;
        // }

        // let incoming = plan.edges_directed(pointer, petgraph::Direction::Incoming);
        // for edge in incoming {
        //     let target = edge.target();
        // }
        let outgoing = plan.edges_directed(pointer, petgraph::Direction::Outgoing);
        for _edge in outgoing {
            // queue.do_send(Push::new(edge.target()));
        }

        // let foo = worker.next().await;

        // let task = plan.node_weight_mut(pointer).unwrap();
        // if task.data.is_some() {
        //     queue.do_send(Push::new(pointer))
        // }
    }
}

#[cfg(test)]
mod tests {
    use crate::factorio::tests::{draw_world, fixture_world};

    use super::*;

    #[test]
    fn test_planner() {
        let world = Arc::new(fixture_world());
        draw_world(world.clone());
        let mut planner = Planner::new(world, None);
        planner
            .plan(
                r##"
    plan.groupStart("Mine Stuff")
    for idx,playerId in pairs(all_bots) do
        plan.mine(playerId, {x=idx * 10,y=43}, "rock-huge", 1)
    end
    plan.groupEnd()
        "##
                .into(),
                0,
            )
            .unwrap();
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: Mine Stuff" ]
    3 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "0" ]
    3 -> 1 [ label = "0" ]
}
"#,
        );
    }
}
