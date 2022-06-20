use crate::graph::entity_graph::EntityGraph;
use crate::graph::flow_graph::FlowGraph;
use crate::types::{
    FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic, FactorioItemPrototype,
    FactorioPlayer, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, PlayerId,
};
use dashmap::DashMap;
use image::RgbaImage;
use miette::{IntoDiagnostic, Result};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;
use std::{fmt, fs};
use tokio::sync::Mutex;

pub struct FactorioWorld {
    pub players: DashMap<PlayerId, FactorioPlayer>,
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

impl FactorioWorld {
    pub fn update_entity_prototypes(
        &self,
        entity_prototypes: Vec<FactorioEntityPrototype>,
    ) -> Result<()> {
        for entity_prototype in entity_prototypes {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype);
        }
        Ok(())
    }

    pub fn update_item_prototypes(
        &self,
        item_prototypes: Vec<FactorioItemPrototype>,
    ) -> Result<()> {
        for item_prototype in item_prototypes {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype);
        }
        Ok(())
    }

    pub fn remove_player(&self, player_id: PlayerId) -> Result<()> {
        self.players.remove(&player_id);
        Ok(())
    }

    pub fn player_changed_distance(&self, event: PlayerChangedDistanceEvent) -> Result<()> {
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

    pub fn player_changed_position(&self, event: PlayerChangedPositionEvent) -> Result<()> {
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

    pub fn update_force(&self, force: FactorioForce) -> Result<()> {
        let name = force.name.clone();
        self.forces.insert(name, force);
        Ok(())
    }

    pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> Result<()> {
        // TODO: update entity direction
        Ok(())
    }

    pub fn on_some_entity_created(&self, entity: FactorioEntity) -> Result<()> {
        info!("XXX on_some_entity_created {:?}", &entity);
        self.entity_graph.add(vec![entity], None)?;
        Ok(())
    }

    pub fn on_some_entity_deleted(&self, entity: FactorioEntity) -> Result<()> {
        self.entity_graph.remove(&entity)?;
        Ok(())
    }

    pub fn player_changed_main_inventory(
        &self,
        event: PlayerChangedMainInventoryEvent,
    ) -> Result<()> {
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

    pub fn update_recipes(&self, recipes: Vec<FactorioRecipe>) -> Result<()> {
        for recipe in recipes {
            self.recipes.insert(recipe.name.clone(), recipe);
        }
        Ok(())
    }

    pub fn update_graphics(&self, graphics: Vec<FactorioGraphic>) -> Result<()> {
        for graphic in graphics {
            self.graphics.insert(graphic.entity_name.clone(), graphic);
        }
        Ok(())
    }

    pub fn update_chunk_tiles(&self, tiles: Vec<FactorioTile>) -> Result<()> {
        self.entity_graph.add_tiles(tiles, None)?; // FIXME: add clear rect from chunk_position
        Ok(())
    }

    #[allow(clippy::map_clone)]
    pub fn update_chunk_entities(&self, entities: Vec<FactorioEntity>) -> Result<()> {
        self.entity_graph.add(entities, None)?; // FIXME: add clear rect
        Ok(())
    }

    pub fn import(&mut self, world: Arc<FactorioWorld>) -> Result<()> {
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
        let players: DashMap<PlayerId, FactorioPlayer> = DashMap::new();
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

    pub fn dump(&self, save_path: Option<&str>) -> Result<()> {
        let content = serde_json::to_string_pretty(self).into_diagnostic()?;
        if let Some(save_path) = save_path {
            fs::write(save_path, &content).into_diagnostic()?;
        } else {
            println!("{content}");
        }

        Ok(())
    }
}

unsafe impl Send for FactorioWorld {}
unsafe impl Sync for FactorioWorld {}

impl Serialize for FactorioWorld {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FactorioWorld", 9)?;
        state.serialize_field("players", &self.players)?;
        state.serialize_field("forces", &self.forces)?;
        state.serialize_field("graphics", &self.graphics)?;
        state.serialize_field("recipes", &*self.recipes)?;
        state.serialize_field("entity_prototypes", &*self.entity_prototypes)?;
        state.serialize_field("item_prototypes", &self.item_prototypes)?;
        state.serialize_field("actions", &self.actions)?;
        state.serialize_field("path_requests", &self.path_requests)?;
        state.serialize_field("entity_graph", &*self.entity_graph)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for FactorioWorld {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Players,
            Forces,
            Graphics,
            Recipes,
            EntityPrototypes,
            ItemPrototypes,
            Actions,
            PathRequests,
            EntityGraph,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`secs` or `nanos`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "players" => Ok(Field::Players),
                            "forces" => Ok(Field::Forces),
                            "graphics" => Ok(Field::Graphics),
                            "recipes" => Ok(Field::Recipes),
                            "entity_prototypes" => Ok(Field::EntityPrototypes),
                            "item_prototypes" => Ok(Field::ItemPrototypes),
                            "actions" => Ok(Field::Actions),
                            "path_requests" => Ok(Field::PathRequests),
                            "entity_graph" => Ok(Field::EntityGraph),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct FactorioWorldVisitor;

        impl<'de> Visitor<'de> for FactorioWorldVisitor {
            type Value = FactorioWorld;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FactorioWorld")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut players = None;
                let mut forces = None;
                let mut graphics = None;
                let mut recipes = None;
                let mut entity_prototypes = None;
                let mut item_prototypes = None;
                let mut actions = None;
                let mut path_requests = None;
                let mut entity_graph = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Players => {
                            if players.is_some() {
                                return Err(de::Error::duplicate_field("players"));
                            }
                            players = Some(map.next_value()?);
                        }
                        Field::Forces => {
                            if forces.is_some() {
                                return Err(de::Error::duplicate_field("forces"));
                            }
                            forces = Some(map.next_value()?);
                        }
                        Field::Graphics => {
                            if graphics.is_some() {
                                return Err(de::Error::duplicate_field("graphics"));
                            }
                            graphics = Some(map.next_value()?);
                        }
                        Field::Recipes => {
                            if recipes.is_some() {
                                return Err(de::Error::duplicate_field("recipes"));
                            }
                            recipes = Some(map.next_value()?);
                        }
                        Field::EntityPrototypes => {
                            if entity_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("entity_prototypes"));
                            }
                            entity_prototypes = Some(map.next_value()?);
                        }
                        Field::ItemPrototypes => {
                            if item_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("item_prototypes"));
                            }
                            item_prototypes = Some(map.next_value()?);
                        }
                        Field::Actions => {
                            if actions.is_some() {
                                return Err(de::Error::duplicate_field("actions"));
                            }
                            actions = Some(map.next_value()?);
                        }
                        Field::PathRequests => {
                            if path_requests.is_some() {
                                return Err(de::Error::duplicate_field("path_requests"));
                            }
                            path_requests = Some(map.next_value()?);
                        }
                        Field::EntityGraph => {
                            if entity_graph.is_some() {
                                return Err(de::Error::duplicate_field("entity_graph"));
                            }
                            entity_graph = Some(map.next_value()?);
                        }
                    }
                }
                let players = players.ok_or_else(|| de::Error::missing_field("players"))?;
                let forces = forces.ok_or_else(|| de::Error::missing_field("forces"))?;
                let graphics = graphics.ok_or_else(|| de::Error::missing_field("graphics"))?;
                let recipes = recipes.ok_or_else(|| de::Error::missing_field("recipes"))?;
                let entity_prototypes = entity_prototypes
                    .ok_or_else(|| de::Error::missing_field("entity_prototypes"))?;
                let item_prototypes =
                    item_prototypes.ok_or_else(|| de::Error::missing_field("item_prototypes"))?;
                let actions = actions.ok_or_else(|| de::Error::missing_field("actions"))?;
                let path_requests =
                    path_requests.ok_or_else(|| de::Error::missing_field("path_requests"))?;
                let entity_graph =
                    entity_graph.ok_or_else(|| de::Error::missing_field("entity_graph"))?;

                let entity_graph: Arc<EntityGraph> = Arc::new(entity_graph);
                let flow_graph = Arc::new(FlowGraph::new(entity_graph.clone()));
                Ok(FactorioWorld {
                    players,
                    forces,
                    graphics,
                    recipes: Arc::new(recipes),
                    entity_prototypes: Arc::new(entity_prototypes),
                    item_prototypes,
                    image_cache: Default::default(),
                    actions,
                    path_requests,
                    next_action_id: Default::default(),
                    entity_graph,
                    flow_graph,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "players",
            "forces",
            "graphics",
            "recipes",
            "entity_prototypes",
            "item_prototypes",
            "actions",
            "path_requests",
            "entity_graph",
        ];
        deserializer.deserialize_struct("FactorioWorld", FIELDS, FactorioWorldVisitor)
    }
}

impl Clone for FactorioWorld {
    fn clone(&self) -> Self {
        let entity_prototypes = Arc::new((*self.entity_prototypes).clone());
        let recipes = Arc::new((*self.recipes).clone());
        let entity_graph = Arc::new((*self.entity_graph).clone());
        let _entity_graph = entity_graph.clone();
        FactorioWorld {
            entity_graph,
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
            flow_graph: Arc::new(FlowGraph::new(_entity_graph)),
        }
    }

    fn clone_from(&mut self, _source: &Self) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::redundant_clone)]
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
