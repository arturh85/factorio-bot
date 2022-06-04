#[cfg_attr(test, mockall_double::double)]
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::graph::task_graph::TaskGraph;
use crate::types::{EntityName, PlayerChangedMainInventoryEvent};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Planner {
    #[allow(dead_code)]
    pub rcon: Option<Arc<FactorioRcon>>,
    pub real_world: Arc<FactorioWorld>,
    pub plan_world: Arc<FactorioWorld>,
    pub graph: Arc<RwLock<TaskGraph>>,
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
    pub fn update_plan_world(&mut self) {
        self.plan_world = Arc::new((*self.real_world).clone());
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.plan_world.clone()
    }
    pub fn graph(&self) -> TaskGraph {
        self.graph.read().clone()
    }

    pub fn initiate_missing_players_with_default_inventory(&mut self, bot_count: u32) -> Vec<u32> {
        let mut player_ids: Vec<u32> = vec![];
        for player_id in 1u32..=bot_count {
            player_ids.push(player_id);
            // initialize missing players with default inventory
            if self.real_world.players.get(&player_id).is_none() {
                let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
                main_inventory.insert(EntityName::Wood.to_string(), 1);
                main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
                main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
                self.real_world
                    .player_changed_main_inventory(PlayerChangedMainInventoryEvent {
                        player_id,
                        main_inventory: main_inventory.clone(),
                    })
                    .expect("failed to set player inventory");
            }
        }
        player_ids
    }
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
