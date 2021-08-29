use crate::factorio::entity_graph::EntityGraph;
use crate::factorio::flow_graph::FlowGraph;
use crate::types::{
    FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic, FactorioItemPrototype,
    FactorioPlayer, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent,
};
use async_std::sync::Mutex;
use dashmap::DashMap;
use image::RgbaImage;
use miette::DiagnosticResult;
use std::sync::Arc;

pub struct FactorioWorld {
    pub players: DashMap<u32, FactorioPlayer>,
    pub forces: DashMap<String, FactorioForce>,
    pub graphics: DashMap<String, FactorioGraphic>,
    pub recipes: Arc<DashMap<String, FactorioRecipe>>,
    pub entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    pub item_prototypes: DashMap<String, FactorioItemPrototype>,
    pub image_cache: DashMap<String, Box<RgbaImage>>,
    pub actions: DashMap<u32, String>,
    pub path_requests: DashMap<u32, String>,
    pub next_action_id: Mutex<u32>,

    pub entity_graph: Arc<EntityGraph>,
    pub flow_graph: Arc<FlowGraph>,
}

impl Clone for FactorioWorld {
    fn clone(&self) -> Self {
        let entity_prototypes = Arc::new((*self.entity_prototypes).clone());
        let recipes = Arc::new((*self.recipes).clone());
        let entity_graph = Arc::new(EntityGraph::new(entity_prototypes.clone(), recipes.clone()));

        FactorioWorld {
            entity_graph: entity_graph.clone(),
            recipes,
            entity_prototypes,
            players: self.players.clone(),
            forces: self.forces.clone(),
            graphics: self.graphics.clone(),
            item_prototypes: self.item_prototypes.clone(),
            image_cache: self.image_cache.clone(),
            actions: self.actions.clone(),
            path_requests: self.path_requests.clone(),
            next_action_id: Mutex::new(0),
            flow_graph: Arc::new(FlowGraph::new(entity_graph)),
        }
    }

    fn clone_from(&mut self, _source: &Self) {
        unimplemented!()
    }
}

impl FactorioWorld {
    pub fn update_entity_prototypes(
        &self,
        entity_prototypes: Vec<FactorioEntityPrototype>,
    ) -> DiagnosticResult<()> {
        for entity_prototype in entity_prototypes {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype);
        }
        Ok(())
    }

    pub fn update_item_prototypes(
        &self,
        item_prototypes: Vec<FactorioItemPrototype>,
    ) -> DiagnosticResult<()> {
        for item_prototype in item_prototypes {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype);
        }
        Ok(())
    }

    pub fn remove_player(&self, player_id: u32) -> DiagnosticResult<()> {
        self.players.remove(&player_id);
        Ok(())
    }

    pub fn player_changed_distance(
        &self,
        event: PlayerChangedDistanceEvent,
    ) -> DiagnosticResult<()> {
        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: existing_player.position.clone(),
                main_inventory: existing_player.main_inventory.clone(),
                build_distance: event.build_distance,
                reach_distance: event.reach_distance,
                drop_item_distance: event.drop_item_distance,
                item_pickup_distance: event.item_pickup_distance,
                loot_pickup_distance: event.loot_pickup_distance,
                resource_reach_distance: event.resource_reach_distance,
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                build_distance: event.build_distance,
                reach_distance: event.reach_distance,
                drop_item_distance: event.drop_item_distance,
                item_pickup_distance: event.item_pickup_distance,
                loot_pickup_distance: event.loot_pickup_distance,
                resource_reach_distance: event.resource_reach_distance,
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    pub fn player_changed_position(
        &self,
        event: PlayerChangedPositionEvent,
    ) -> DiagnosticResult<()> {
        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: event.position,
                main_inventory: existing_player.main_inventory.clone(),
                build_distance: existing_player.build_distance,
                reach_distance: existing_player.reach_distance,
                drop_item_distance: existing_player.drop_item_distance,
                item_pickup_distance: existing_player.item_pickup_distance,
                loot_pickup_distance: existing_player.loot_pickup_distance,
                resource_reach_distance: existing_player.resource_reach_distance,
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                position: event.position,
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    pub fn update_force(&self, force: FactorioForce) -> DiagnosticResult<()> {
        let name = force.name.clone();
        self.forces.insert(name, force);
        Ok(())
    }

    pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> DiagnosticResult<()> {
        // TODO: update entity direction
        Ok(())
    }

    pub fn on_some_entity_created(&self, entity: FactorioEntity) -> DiagnosticResult<()> {
        info!("XXX on_some_entity_created {:?}", &entity);
        self.entity_graph.add(vec![entity], None)?;
        Ok(())
    }

    pub fn on_some_entity_deleted(&self, entity: FactorioEntity) -> DiagnosticResult<()> {
        self.entity_graph.remove(&entity)?;
        Ok(())
    }

    pub fn player_changed_main_inventory(
        &self,
        event: PlayerChangedMainInventoryEvent,
    ) -> DiagnosticResult<()> {
        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: existing_player.position.clone(),
                main_inventory: event.main_inventory,
                build_distance: existing_player.build_distance,
                reach_distance: existing_player.reach_distance,
                drop_item_distance: existing_player.drop_item_distance,
                item_pickup_distance: existing_player.item_pickup_distance,
                loot_pickup_distance: existing_player.loot_pickup_distance,
                resource_reach_distance: existing_player.resource_reach_distance,
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                main_inventory: event.main_inventory.clone(),
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    pub fn update_recipes(&self, recipes: Vec<FactorioRecipe>) -> DiagnosticResult<()> {
        for recipe in recipes {
            self.recipes.insert(recipe.name.clone(), recipe);
        }
        Ok(())
    }

    pub fn update_graphics(&self, graphics: Vec<FactorioGraphic>) -> DiagnosticResult<()> {
        for graphic in graphics {
            self.graphics.insert(graphic.entity_name.clone(), graphic);
        }
        Ok(())
    }

    pub fn update_chunk_tiles(&self, tiles: Vec<FactorioTile>) -> DiagnosticResult<()> {
        self.entity_graph.add_tiles(tiles, None)?; // FIXME: add clear rect from chunk_position
        Ok(())
    }

    #[allow(clippy::map_clone)]
    pub fn update_chunk_entities(&self, entities: Vec<FactorioEntity>) -> DiagnosticResult<()> {
        self.entity_graph.add(entities, None)?; // FIXME: add clear rect
        Ok(())
    }

    pub fn import(&mut self, world: Arc<FactorioWorld>) -> DiagnosticResult<()> {
        for player in world.players.iter() {
            self.players.insert(player.player_id, player.clone());
        }
        for entity_prototype in world.entity_prototypes.iter() {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype.clone());
        }
        for item_prototype in world.item_prototypes.iter() {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype.clone());
        }
        for recipe in world.recipes.iter() {
            self.recipes.insert(recipe.name.clone(), recipe.clone());
        }
        for force in world.forces.iter() {
            self.forces.insert(force.name.clone(), force.clone());
        }
        self.entity_graph.connect()?;
        Ok(())
    }

    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let forces: DashMap<String, FactorioForce> = DashMap::new();
        let players: DashMap<u32, FactorioPlayer> = DashMap::new();
        let graphics: DashMap<String, FactorioGraphic> = DashMap::new();
        let image_cache: DashMap<String, Box<RgbaImage>> = DashMap::new();
        let item_prototypes: DashMap<String, FactorioItemPrototype> = DashMap::new();
        let recipes: Arc<DashMap<String, FactorioRecipe>> = Arc::new(DashMap::new());
        let entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>> =
            Arc::new(DashMap::new());
        let entity_graph = Arc::new(EntityGraph::new(entity_prototypes.clone(), recipes.clone()));
        let flow_graph = Arc::new(FlowGraph::new(entity_graph.clone()));
        FactorioWorld {
            image_cache,
            players,
            graphics,
            recipes,
            forces,
            entity_prototypes,
            item_prototypes,
            actions: DashMap::new(),
            path_requests: DashMap::new(),
            next_action_id: Mutex::new(1),
            entity_graph,
            flow_graph,
        }
    }
}

unsafe impl Send for FactorioWorld {}
unsafe impl Sync for FactorioWorld {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_boundaries_0() {
        let world = FactorioWorld {
            players: Default::default(),
            forces: Default::default(),
            graphics: Default::default(),
            recipes: Arc::new(Default::default()),
            entity_prototypes: Arc::new(Default::default()),
            item_prototypes: Default::default(),
            image_cache: Default::default(),
            actions: Default::default(),
            path_requests: Default::default(),
            next_action_id: Default::default(),
            entity_graph: Arc::new(EntityGraph::new(
                Arc::new(DashMap::new()),
                Arc::new(DashMap::new()),
            )),
            flow_graph: Arc::new(FlowGraph::new(Arc::new(EntityGraph::new(
                Arc::new(DashMap::new()),
                Arc::new(DashMap::new()),
            )))),
        };

        let _cloned = world.clone();
    }
}
