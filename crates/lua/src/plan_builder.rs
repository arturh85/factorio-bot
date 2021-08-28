use dashmap::lock::RwLock;
use factorio_bot_core::factorio::task_graph::{MineTarget, PositionRadius, TaskGraph};
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{
    FactorioEntity, FactorioPlayer, PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent,
    Position,
};
use num_traits::ToPrimitive;
use rlua::{Context, Table};
use std::sync::Arc;

pub struct PlanBuilder {
    graph: Arc<RwLock<TaskGraph>>,
    world: Arc<FactorioWorld>,
}

impl PlanBuilder {
    pub fn new(graph: Arc<RwLock<TaskGraph>>, world: Arc<FactorioWorld>) -> PlanBuilder {
        PlanBuilder { graph, world }
    }

    pub fn mine(
        &self,
        player_id: u32,
        position: Position,
        name: &str,
        count: u32,
    ) -> anyhow::Result<()> {
        let mut graph = self.graph.write();
        let player = self
            .world
            .players
            .get(&player_id)
            .expect("invalid player id");
        let distance = calculate_distance(&player.position, &position).ceil();
        let reach_distance = player.resource_reach_distance as f64;
        if distance > reach_distance {
            graph.add_walk_node(
                player_id,
                distance,
                PositionRadius::from_position(&position, reach_distance),
            );
        }
        let mut mining_time = 5.;
        let mut inventory = player.main_inventory.clone();
        if let Some(prototype) = self.world.entity_prototypes.get(name) {
            if let Some(result) = prototype.mine_result.as_ref() {
                for (mine_name, mine_count) in result {
                    if let Some(inventory_count) = inventory.get(mine_name) {
                        let cnt = *mine_count + *inventory_count;
                        inventory.insert(mine_name.clone(), cnt);
                    } else {
                        inventory.insert(mine_name.clone(), *mine_count);
                    }
                }
                if let Some(time) = prototype.mining_time.as_ref() {
                    mining_time = time.to_f64().unwrap().ceil()
                }
            }
        }
        graph.add_mine_node(
            player_id,
            mining_time,
            MineTarget {
                name: name.into(),
                count,
                position,
            },
        );
        drop(player);
        self.world
            .player_changed_main_inventory(PlayerChangedMainInventoryEvent {
                player_id,
                main_inventory: inventory,
            })?;
        Ok(())
    }

    fn distance(&self, player_id: u32, position: &Position) -> f64 {
        calculate_distance(
            &self.world.players.get(&player_id).unwrap().position,
            position,
        )
        .ceil()
    }

    fn player(&self, player_id: u32) -> FactorioPlayer {
        self.world
            .players
            .get(&player_id)
            .expect("failed to find player")
            .clone()
    }
    // fn inventory(&self, player_id: u32, name: &str) -> u32 {
    //     *self
    //         .player(player_id)
    //         .main_inventory
    //         .get(name)
    //         .unwrap_or(&0)
    // }

    pub fn add_walk(&self, player_id: u32, goal: PositionRadius) -> anyhow::Result<()> {
        let distance = self.distance(player_id, &goal.position);
        let mut graph = self.graph.write();
        self.world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id,
                position: goal.position.clone(),
            })?;
        graph.add_walk_node(player_id, distance, goal);
        Ok(())
    }

    pub fn add_place(&mut self, player_id: u32, entity: FactorioEntity) -> anyhow::Result<()> {
        let player = self.player(player_id);
        let distance = calculate_distance(&player.position, &entity.position);
        let build_distance = player.build_distance as f64;
        if distance > build_distance {
            self.add_walk(
                player_id,
                PositionRadius::from_position(&entity.position, build_distance),
            )?;
        }
        let mut inventory = self.player(player_id).main_inventory;
        let inventory_item_count = *inventory.get(&entity.name).unwrap_or(&0);
        if inventory_item_count < 1 {
            return Err(anyhow!(
                "player #{} does not have {} in inventory",
                player_id,
                &entity.name
            ));
        }
        let mut graph = self.graph.write();
        graph.add_place_node(player_id, 1., entity.clone());
        inventory.insert(entity.name.clone(), inventory_item_count - 1);
        self.world
            .player_changed_main_inventory(PlayerChangedMainInventoryEvent {
                player_id,
                main_inventory: inventory,
            })?;
        self.world.on_some_entity_created(entity)?;
        Ok(())
    }

    pub fn group_start(&self, label: &str) {
        let mut graph = self.graph.write();
        graph.group_start(label);
    }

    pub fn group_end(&self) {
        let mut graph = self.graph.write();
        graph.group_end();
    }
}

pub fn create_lua_plan_builder(
    ctx: Context,
    graph: Arc<RwLock<TaskGraph>>,
    world: Arc<FactorioWorld>,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
    let _plan_builder = Arc::new(PlanBuilder::new(graph, world));

    let plan_builder = _plan_builder.clone();
    map_table.set(
        "mine",
        ctx.create_function(
            move |_ctx, (player_id, position, name, count): (u32, Table, String, u32)| {
                plan_builder
                    .mine(
                        player_id,
                        Position::new(position.get("x").unwrap(), position.get("y").unwrap()),
                        name.as_str(),
                        count,
                    )
                    .unwrap();
                Ok(())
            },
        )?,
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "walk",
        ctx.create_function(
            move |_ctx, (player_id, position, radius): (u32, Table, f64)| {
                plan_builder
                    .add_walk(
                        player_id,
                        PositionRadius::new(
                            position.get("x").unwrap(),
                            position.get("y").unwrap(),
                            radius,
                        ),
                    )
                    .unwrap();
                Ok(())
            },
        )?,
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "groupStart",
        ctx.create_function(move |_ctx, label: String| {
            plan_builder.group_start(label.as_str());
            Ok(())
        })?,
    )?;
    let plan_builder = _plan_builder;
    map_table.set(
        "groupEnd",
        ctx.create_function(move |_ctx, ()| {
            plan_builder.group_end();
            Ok(())
        })?,
    )?;
    Ok(map_table)
}
