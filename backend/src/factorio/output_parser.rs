use std::sync::Arc;

use actix::Addr;

use crate::factorio::world::FactorioWorld;
use crate::factorio::ws::{
    FactorioWebSocketServer, PlayerChangedMainInventoryMessage, PlayerChangedPositionMessage,
    PlayerDistanceChangedMessage, PlayerLeftMessage, ResearchCompletedMessage,
};
use crate::types::{
    ChunkPosition, FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic,
    FactorioItemPrototype, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, Pos, Position, Rect,
};

pub struct OutputParser {
    world: Arc<FactorioWorld>,
    websocket_server: Option<Addr<FactorioWebSocketServer>>,
}

impl OutputParser {
    pub async fn parse(&mut self, _tick: u64, action: &str, rest: &str) -> anyhow::Result<()> {
        match action {
            "entities" => {
                let colon_pos = rest.find(':').unwrap();
                let rect: Rect = rest[0..colon_pos].parse()?;
                let pos: Pos = (&rect.left_top).into();
                let _chunk_position: ChunkPosition = (&pos).into();
                let mut entities = &rest[colon_pos + 1..];
                if entities == "{}" {
                    entities = "[]"
                }
                let entities: Vec<FactorioEntity> = serde_json::from_str(entities).unwrap();
                self.world.update_chunk_entities(entities)?;
            }
            "tiles" => {
                let colon_pos = rest.find(':').unwrap();
                let rect: Rect = rest[0..colon_pos].parse()?;
                let pos: Pos = (&rect.left_top).into();
                let chunk_position: ChunkPosition = (&pos).into();
                let tiles: Vec<FactorioTile> = rest[colon_pos + 1..]
                    .split(',')
                    .enumerate()
                    .map(|(index, tile)| {
                        let parts: Vec<&str> = tile.split(':').collect();
                        let name: String = parts[0].trim().into();
                        let color_name = match name.find('-') {
                            Some(pos) => {
                                if &name[0..pos] == "red" {
                                    match name[pos + 1..].find('-') {
                                        Some(pos2) => &name[pos + 1..pos + pos2 + 1],
                                        None => &name[pos + 1..],
                                    }
                                } else {
                                    &name[0..pos]
                                }
                            }
                            None => &name,
                        };
                        FactorioTile {
                            color: match &color_name[..] {
                                "water" => Some([0u8, 162u8, 232u8, 255u8]),
                                "deepwater" => Some([18u8, 16u8, 254u8, 255u8]),
                                _ => None, // "out" => [0u8, 0u8, 0u8, 255u8],
                                           // "sand" => [255u8, 249u8, 15u8, 255u8],
                                           // "desert" => [255u8, 229u8, 15u8, 255u8],
                                           // "dry" => [255u8, 255u8, 128u8, 255u8],
                                           // "dirt" => [172u8, 255u8, 0u8, 255u8],
                                           // "grass" => [0u8, 255u8, 64u8, 255u8],
                                           // "water" => [0u8, 162u8, 232u8, 255u8],
                                           // "deepwater" => [18u8, 16u8, 254u8, 255u8],
                                           // _ => {
                                           //     warn!(
                                           //         "<red>unhandled tile type</>: <yellow>{}</> to <bright-blue>'{}'</>",
                                           //         name, color_name
                                           //     );
                                           //     [255u8, 0u8, 255u8, 255u8]
                                           // }
                            },
                            name,
                            player_collidable: parts[1].parse::<u8>().unwrap() == 1,
                            position: Position::new(
                                (chunk_position.x * 32 + (index % 32) as i32) as f64,
                                (chunk_position.y * 32 + (index / 32) as i32) as f64,
                            ),
                        }
                    })
                    .collect();
                self.world.update_chunk_tiles(tiles)?;
            }
            "graphics" => {
                // 0 graphics: spark-explosion*__core__/graphics/empty.png:1:1:0:0:0:0:1|spark-explosion-higher*__core__/graphics/empty.png:1:1:0:0:0:0:1|
                let graphics: Vec<FactorioGraphic> = rest
                    .split('|')
                    .map(|graphic| {
                        let parts: Vec<&str> = graphic.split(':').collect();
                        let parts2: Vec<&str> = parts[0].split('*').collect();
                        FactorioGraphic {
                            entity_name: parts2[0].into(),
                            image_path: parts2[1].into(),
                            width: parts[1].parse().unwrap(),
                            height: parts[1].parse().unwrap(),
                        }
                    })
                    .collect();
                self.world.update_graphics(graphics)?;
            }
            "entity_prototypes" => {
                let entity_prototypes: Vec<FactorioEntityPrototype> = rest
                    .split('$')
                    .map(|entity_prototype| {
                        serde_json::from_str(entity_prototype).unwrap_or_else(|err| {
                            panic!(
                                "failed to deserialize entity prototype: {:?} '{}'",
                                err, entity_prototype
                            )
                        })
                    })
                    .collect();
                self.world.update_entity_prototypes(entity_prototypes)?;
            }
            "item_prototypes" => {
                let item_prototypes: Vec<FactorioItemPrototype> = rest
                    .split('$')
                    .map(|item_prototype| {
                        serde_json::from_str(item_prototype).unwrap_or_else(|err| {
                            panic!(
                                "failed to deserialize item prototype: {:?} '{}'",
                                err, item_prototype
                            )
                        })
                    })
                    .collect();
                self.world.update_item_prototypes(item_prototypes)?;
            }
            "recipes" => {
                let recipes: Vec<FactorioRecipe> = rest
                    .split('$')
                    .map(|recipe| {
                        serde_json::from_str(recipe).unwrap_or_else(|err| {
                            panic!("failed to deserialize recipe: {:?} '{}'", err, recipe)
                        })
                    })
                    .collect();
                self.world.update_recipes(recipes)?;
            }
            "action_completed" => {
                if let Some(pos) = rest.find(' ') {
                    let action_status = &rest[0..pos];
                    let rest = &rest[pos + 1..];
                    let action_id: u32 = match rest.find(' ') {
                        Some(pos) => (&rest[0..pos]).parse()?,
                        None => rest.parse()?,
                    };
                    let result = match action_status {
                        "ok" => "ok",
                        "fail" => {
                            let pos = rest.find(' ').unwrap();
                            &rest[pos + 1..]
                        }
                        _ => panic!(format!("unexpected action_completed: {}", action_status)),
                    };
                    self.world.actions.insert(action_id, String::from(result));
                }
            }
            "on_script_path_request_finished" => {
                let parts: Vec<&str> = rest.split('#').collect();
                let id: u32 = parts[0].parse()?;
                self.world.path_requests.insert(id, String::from(parts[1]));
            }
            "STATIC_DATA_END" => {
                // handled by OutputReader
            }
            "on_player_left_game" => {
                let player_id: u32 = rest.parse()?;
                self.world.remove_player(player_id)?;
                if let Some(websocket_server) = self.websocket_server.as_ref() {
                    websocket_server
                        .send(PlayerLeftMessage { player_id })
                        .await?;
                }
            }
            "on_research_finished" => {
                if let Some(websocket_server) = self.websocket_server.as_ref() {
                    websocket_server.send(ResearchCompletedMessage {}).await?;
                }
            }
            "force" => {
                let force: FactorioForce = serde_json::from_str(rest).unwrap_or_else(|err| {
                    panic!("failed to deserialize force: {:?} '{}'", err, rest)
                });
                self.world.update_force(force)?;
            }
            "on_some_entity_created" => {
                let entity: FactorioEntity = serde_json::from_str(rest).unwrap_or_else(|err| {
                    panic!("failed to deserialize entity: {:?} '{}'", err, rest)
                });
                self.world.on_some_entity_created(entity)?;
            }
            "on_some_entity_updated" => {
                let entity: FactorioEntity = serde_json::from_str(rest).unwrap_or_else(|err| {
                    panic!("failed to deserialize entity: {:?} '{}'", err, rest)
                });
                self.world.on_some_entity_updated(entity)?;
            }
            "on_some_entity_deleted" => {
                let entity: FactorioEntity = serde_json::from_str(rest).unwrap_or_else(|err| {
                    panic!("failed to deserialize entity: {:?} '{}'", err, rest)
                });
                self.world.on_some_entity_deleted(entity)?;
            }
            "on_player_main_inventory_changed" => {
                let event: PlayerChangedMainInventoryEvent = serde_json::from_str(rest)?;
                let player_id = event.player_id;
                self.world.player_changed_main_inventory(event)?;
                if let Some(websocket_server) = self.websocket_server.as_ref() {
                    websocket_server
                        .send(PlayerChangedMainInventoryMessage {
                            player: self.world.players.get(&player_id).unwrap().clone(),
                        })
                        .await?;
                }
            }
            "on_player_changed_position" => {
                let event: PlayerChangedPositionEvent = serde_json::from_str(rest)?;
                let player_id = event.player_id;
                self.world.player_changed_position(event)?;
                if let Some(websocket_server) = self.websocket_server.as_ref() {
                    websocket_server
                        .send(PlayerChangedPositionMessage {
                            player: self.world.players.get(&player_id).unwrap().clone(),
                        })
                        .await?;
                }
            }
            "on_player_changed_distance" => {
                let event: PlayerChangedDistanceEvent = serde_json::from_str(rest)?;
                let player_id = event.player_id;
                self.world.player_changed_distance(event)?;
                if let Some(websocket_server) = self.websocket_server.as_ref() {
                    websocket_server
                        .send(PlayerDistanceChangedMessage {
                            player: self.world.players.get(&player_id).unwrap().clone(),
                        })
                        .await?;
                }
            }
            "mined_item" => {
                // info!("tick!");
            }
            "tick" => {
                // info!("tick!");
            }
            _ => {
                error!("<red>unexpected action</>: <bright-blue>{}</>", action);
            }
        };
        Ok(())
    }

    pub fn on_init(&self) -> anyhow::Result<()> {
        self.world.entity_graph.connect()?;
        self.world.flow_graph.update()?;
        Ok(())
    }

    #[allow(clippy::new_without_default)]
    pub fn new(websocket_server: Option<Addr<FactorioWebSocketServer>>) -> Self {
        OutputParser {
            websocket_server,
            world: Arc::new(FactorioWorld::new()),
        }
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.world.clone()
    }
}
