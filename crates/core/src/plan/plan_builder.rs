use crate::errors::PlayerMissingItem;
use crate::factorio::util::calculate_distance;
use crate::factorio::world::FactorioWorld;
use crate::graph::task_graph::TaskGraph;
use crate::types::{
    FactorioEntity, FactorioPlayer, InventoryItem, InventoryLocation, MineTarget,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, PlayerId, Position,
    PositionRadius,
};
use miette::Result;
use num_traits::ToPrimitive;
use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Clone)]
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
        player_id: PlayerId,
        position: Position,
        name: &str,
        count: u32,
    ) -> Result<()> {
        let player = self.player(player_id);
        let distance = calculate_distance(&player.position, &position).ceil();
        let reach_distance = player.resource_reach_distance as f64;
        if distance > reach_distance {
            self.add_walk(
                player_id,
                PositionRadius::from_position(&position, reach_distance),
            )?;
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
        let mut graph = self.graph.write();
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
        self.world.player_changed_main_inventory(
            PlayerChangedMainInventoryEvent::from_btreemap(player_id, inventory),
        )?;
        Ok(())
    }

    fn distance(&self, player_id: PlayerId, position: &Position) -> f64 {
        calculate_distance(
            &self.world.players.get(&player_id).unwrap().position,
            position,
        )
        .ceil()
    }

    fn player(&self, player_id: PlayerId) -> FactorioPlayer {
        self.world
            .players
            .get(&player_id)
            .expect("failed to find player")
            .clone()
    }
    // fn inventory(&self, player_id: PlayerId, name: &str) -> u32 {
    //     *self
    //         .player(player_id)
    //         .main_inventory
    //         .get(name)
    //         .unwrap_or(&0)
    // }

    pub fn add_walk(&self, player_id: PlayerId, goal: PositionRadius) -> Result<()> {
        let distance = self.distance(player_id, &goal.position);
        self.world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id,
                position: goal.position.clone(),
            })?;
        let mut graph = self.graph.write();
        graph.add_walk_node(player_id, distance, goal);
        Ok(())
    }

    pub fn add_place(&self, player_id: PlayerId, entity: FactorioEntity) -> Result<FactorioEntity> {
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
            return Err(PlayerMissingItem {
                player_id,
                item: entity.name,
            }
            .into());
        }
        let mut graph = self.graph.write();
        graph.add_place_node(player_id, 1., entity.clone());
        inventory.insert(entity.name.clone(), inventory_item_count - 1);
        self.world.player_changed_main_inventory(
            PlayerChangedMainInventoryEvent::from_btreemap(player_id, inventory),
        )?;
        self.world.on_some_entity_created(entity.clone())?;
        Ok(entity)
    }

    pub fn add_insert_into_inventory(
        &self,
        player_id: PlayerId,
        location: InventoryLocation,
        item: InventoryItem,
    ) -> Result<()> {
        let player = self.player(player_id);
        let distance = calculate_distance(&player.position, &location.position);
        let reach_distance = player.reach_distance as f64;
        if distance > reach_distance {
            self.add_walk(
                player_id,
                PositionRadius::from_position(&location.position, reach_distance),
            )?;
        }
        let mut inventory = self.player(player_id).main_inventory;
        let inventory_item_count = *inventory.get(&item.name).unwrap_or(&0);
        if inventory_item_count < item.count {
            return Err(PlayerMissingItem {
                player_id,
                item: item.name,
            }
            .into());
        }
        let mut graph = self.graph.write();
        graph.add_insert_into_inventory_node(player_id, 1., location, item.clone());
        drop(graph);

        // Update player inventory to reflect items inserted into target inventory
        inventory.insert(item.name.clone(), inventory_item_count - item.count);
        self.world.player_changed_main_inventory(
            PlayerChangedMainInventoryEvent::from_btreemap(player_id, inventory),
        )?;
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
