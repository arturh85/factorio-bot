use crate::factorio::entity_graph::EntityGraph;
use crate::factorio::flow_graph::FlowGraph;
use crate::types::{
    FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic, FactorioItemPrototype,
    FactorioPlayer, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, Position, Rect,
};
use async_std::sync::Mutex;
use dashmap::DashMap;
use image::RgbaImage;
use rlua::{Context, Table};
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
    ) -> anyhow::Result<()> {
        for entity_prototype in entity_prototypes {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype);
        }
        Ok(())
    }

    pub fn update_item_prototypes(
        &self,
        item_prototypes: Vec<FactorioItemPrototype>,
    ) -> anyhow::Result<()> {
        for item_prototype in item_prototypes {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype);
        }
        Ok(())
    }

    pub fn remove_player(&self, player_id: u32) -> anyhow::Result<()> {
        self.players.remove(&player_id);
        Ok(())
    }

    pub fn player_changed_distance(&self, event: PlayerChangedDistanceEvent) -> anyhow::Result<()> {
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

    pub fn player_changed_position(&self, event: PlayerChangedPositionEvent) -> anyhow::Result<()> {
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

    pub fn update_force(&self, force: FactorioForce) -> anyhow::Result<()> {
        let name = force.name.clone();
        self.forces.insert(name, force);
        Ok(())
    }

    pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> anyhow::Result<()> {
        // TODO: update entity direction
        Ok(())
    }

    pub fn on_some_entity_created(&self, entity: FactorioEntity) -> anyhow::Result<()> {
        self.entity_graph.add(vec![entity], None)?;
        Ok(())
    }

    pub fn on_some_entity_deleted(&self, entity: FactorioEntity) -> anyhow::Result<()> {
        self.entity_graph.remove(&entity)?;
        Ok(())
    }

    pub fn player_changed_main_inventory(
        &self,
        event: PlayerChangedMainInventoryEvent,
    ) -> anyhow::Result<()> {
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

    pub fn update_recipes(&self, recipes: Vec<FactorioRecipe>) -> anyhow::Result<()> {
        for recipe in recipes {
            self.recipes.insert(recipe.name.clone(), recipe);
        }
        Ok(())
    }

    pub fn update_graphics(&self, graphics: Vec<FactorioGraphic>) -> anyhow::Result<()> {
        for graphic in graphics {
            self.graphics.insert(graphic.entity_name.clone(), graphic);
        }
        Ok(())
    }

    pub fn update_chunk_tiles(&self, tiles: Vec<FactorioTile>) -> anyhow::Result<()> {
        self.entity_graph.add_tiles(tiles, None)?; // FIXME: add clear rect from chunk_position
        Ok(())
    }

    #[allow(clippy::map_clone)]
    pub fn update_chunk_entities(&self, entities: Vec<FactorioEntity>) -> anyhow::Result<()> {
        self.entity_graph.add(entities, None)?; // FIXME: add clear rect
        Ok(())
    }

    pub fn import(&mut self, world: Arc<FactorioWorld>) -> anyhow::Result<()> {
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

pub fn create_lua_world(ctx: Context, _world: Arc<FactorioWorld>) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;

    let world = _world.clone();
    map_table.set(
        "recipe",
        ctx.create_function(move |ctx, name: String| match world.recipes.get(&name) {
            Some(recipe) => Ok(rlua_serde::to_value(ctx, recipe.clone())),
            None => Err(rlua::Error::RuntimeError("recipe not found".into())),
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "player",
        ctx.create_function(
            move |ctx, player_id: u32| match world.players.get(&player_id) {
                Some(player) => Ok(rlua_serde::to_value(ctx, player.clone())),
                None => Err(rlua::Error::RuntimeError("player not found".into())),
            },
        )?,
    )?;

    let world = _world.clone();
    map_table.set(
        "findFreeResourceRect",
        ctx.create_function(
            move |_ctx, (ore_name, width, height, near): (String, u32, u32, Table)| {
                let patches = world.entity_graph.resource_patches(ore_name.as_str());
                let near = Position::new(near.get("x").unwrap(), near.get("y").unwrap());
                for patch in patches {
                    let rect = patch.find_free_rect(width, height, &near);
                    if let Some(rect) = rect {
                        return Ok(rect);
                    }
                }
                Ok(Rect::default())
            },
        )?,
    )?;

    let world = _world;
    map_table.set(
        "inventory",
        ctx.create_function(move |_ctx, (player_id, item_name): (u32, String)| {
            match world.players.get(&player_id) {
                Some(player) => match player.main_inventory.get(&item_name) {
                    Some(cnt) => Ok(*cnt),
                    None => Ok(0),
                },
                None => Err(rlua::Error::RuntimeError("player not found".into())),
            }
        })?,
    )?;

    Ok(map_table)
}

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
