use std::sync::Arc;

use crate::factorio::ticks::ActionOutcome;
use crate::factorio::world::FactorioWorld;
// use crate::factorio::ws::{
//     FactorioWebSocketServer, PlayerChangedMainInventoryMessage, PlayerChangedPositionMessage,
//     PlayerDistanceChangedMessage, PlayerLeftMessage, ResearchCompletedMessage,
// };
use crate::types::{
    ChunkPosition, FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic,
    FactorioItemPrototype, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, PlayerId, Pos, Position, Rect,
};
use miette::{IntoDiagnostic, Result};

pub struct OutputParser {
    world: Arc<FactorioWorld>,
    // websocket_server: Option<Addr<FactorioWebSocketServer>>,
}

impl OutputParser {
    /// `tick` is the game tick the mod stamped on the line
    /// (`writeout(tick, key, value)`). Most branches have no use for it; the
    /// `action_completed` branch is the exception, and it is the executor's
    /// only source of a *real* completion tick.
    pub fn parse(&mut self, tick: u64, action: &str, rest: &str) -> Result<()> {
        match action {
            "entities" => {
                let colon_pos = match rest.find(':') {
                    Some(pos) => pos,
                    None => {
                        error!("<red>malformed entities line, missing ':'</>: '{}'", rest);
                        return Ok(());
                    }
                };
                let rect: Rect = rest[0..colon_pos].parse()?;
                let pos: Pos = (&rect.left_top).into();
                let _chunk_position: ChunkPosition = (&pos).into();
                let mut entities = &rest[colon_pos + 1..];
                if entities == "{}" {
                    entities = "[]"
                }
                let entities: Vec<FactorioEntity> =
                    serde_json::from_str(entities).into_diagnostic()?;
                self.world.update_chunk_entities(entities)?;
            }
            "tiles" => {
                let colon_pos = match rest.find(':') {
                    Some(pos) => pos,
                    None => {
                        error!("<red>malformed tiles line, missing ':'</>: '{}'", rest);
                        return Ok(());
                    }
                };
                let rect: Rect = rest[0..colon_pos].parse()?;
                let pos: Pos = (&rect.left_top).into();
                let chunk_position: ChunkPosition = (&pos).into();
                let tiles: Vec<FactorioTile> = rest[colon_pos + 1..]
                    .split(',')
                    .enumerate()
                    .filter_map(|(index, tile)| {
                        let parts: Vec<&str> = tile.split(':').collect();
                        let name: String = parts[0].trim().into();
                        let player_collidable =
                            match parts.get(1).and_then(|p| p.parse::<u8>().ok()) {
                                Some(value) => value == 1,
                                None => {
                                    error!("<red>malformed tile, skipping</>: '{}'", tile);
                                    return None;
                                }
                            };
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
                        let tile = FactorioTile {
                            color: match color_name {
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
                            player_collidable,
                            position: Position::new(
                                (chunk_position.x * 32 + (index % 32) as i32) as f64,
                                (chunk_position.y * 32 + (index / 32) as i32) as f64,
                            ),
                        };
                        Some(tile)
                    })
                    .collect();
                self.world.update_chunk_tiles(tiles)?;
            }
            "graphics" => {
                // 0 graphics: spark-explosion*__core__/graphics/empty.png:1:1:0:0:0:0:1|spark-explosion-higher*__core__/graphics/empty.png:1:1:0:0:0:0:1|
                // filename:width:height:shiftx:shifty:xx:yy:scale (see
                // mods/BotBridge/control.lua's writeout_proto_picture_dir),
                // so width is field 1 and height is field 2 -- not both
                // field 1.
                let graphics: Vec<FactorioGraphic> = rest
                    .split('|')
                    .filter_map(|graphic| {
                        let parts: Vec<&str> = graphic.split(':').collect();
                        let parts2: Vec<&str> = match parts.first() {
                            Some(head) => head.split('*').collect(),
                            None => {
                                error!("<red>malformed graphic, skipping</>: '{}'", graphic);
                                return None;
                            }
                        };
                        let (entity_name, image_path) = match (parts2.first(), parts2.get(1)) {
                            (Some(entity_name), Some(image_path)) => {
                                (String::from(*entity_name), String::from(*image_path))
                            }
                            _ => {
                                error!("<red>malformed graphic, skipping</>: '{}'", graphic);
                                return None;
                            }
                        };
                        let width = match parts.get(1).and_then(|p| p.parse().ok()) {
                            Some(width) => width,
                            None => {
                                error!("<red>malformed graphic, skipping</>: '{}'", graphic);
                                return None;
                            }
                        };
                        let height = match parts.get(2).and_then(|p| p.parse().ok()) {
                            Some(height) => height,
                            None => {
                                error!("<red>malformed graphic, skipping</>: '{}'", graphic);
                                return None;
                            }
                        };
                        Some(FactorioGraphic {
                            entity_name,
                            image_path,
                            width,
                            height,
                        })
                    })
                    .collect();
                self.world.update_graphics(graphics)?;
            }
            // Startup data: a Factorio version bump or mod change can drift
            // the schema of any one of these three the same way it can drift
            // the events handled below (see the comment near line 191).
            // `unwrap_or_else(panic!)` here means any single malformed
            // element -- among potentially thousands -- aborts the whole
            // bot process on startup under `panic = "abort"`. Log and skip
            // just that element instead.
            "entity_prototypes" => {
                let entity_prototypes: Vec<FactorioEntityPrototype> = rest
                    .split('$')
                    .filter_map(
                        |entity_prototype| match serde_json::from_str(entity_prototype) {
                            Ok(entity_prototype) => Some(entity_prototype),
                            Err(err) => {
                                error!(
                                    "<red>failed to deserialize entity prototype</>: {:?} '{}'",
                                    err, entity_prototype
                                );
                                None
                            }
                        },
                    )
                    .collect();
                self.world.update_entity_prototypes(entity_prototypes)?;
            }
            "item_prototypes" => {
                let item_prototypes: Vec<FactorioItemPrototype> = rest
                    .split('$')
                    .filter_map(
                        |item_prototype| match serde_json::from_str(item_prototype) {
                            Ok(item_prototype) => Some(item_prototype),
                            Err(err) => {
                                error!(
                                    "<red>failed to deserialize item prototype</>: {:?} '{}'",
                                    err, item_prototype
                                );
                                None
                            }
                        },
                    )
                    .collect();
                self.world.update_item_prototypes(item_prototypes)?;
            }
            "recipes" => {
                let recipes: Vec<FactorioRecipe> = rest
                    .split('$')
                    .filter_map(|recipe| match serde_json::from_str(recipe) {
                        Ok(recipe) => Some(recipe),
                        Err(err) => {
                            error!(
                                "<red>failed to deserialize recipe</>: {:?} '{}'",
                                err, recipe
                            );
                            None
                        }
                    })
                    .collect();
                self.world.update_recipes(recipes)?;
            }
            "action_completed" => {
                if let Some(pos) = rest.find(' ') {
                    let action_status = &rest[0..pos];
                    let rest = &rest[pos + 1..];
                    let action_id: u32 = match rest.find(' ') {
                        Some(pos) => rest[0..pos].parse().into_diagnostic()?,
                        None => rest.parse().into_diagnostic()?,
                    };
                    // An action status this parser doesn't recognize is not
                    // evidence the action succeeded -- it's evidence we
                    // can't tell either way. Recording it as completed
                    // (e.g. defaulting to "ok") would make the executor
                    // believe a running action finished when it may not
                    // have, which is worse than the panic it replaces: the
                    // old panic at least stopped the run rather than let it
                    // continue on a false belief. So skip the insert
                    // entirely and leave the action outstanding -- the same
                    // "log loudly, do less" posture as the unparseable
                    // events above, just applied to "don't record" instead
                    // of "don't apply".
                    let result = match action_status {
                        "ok" => Some("ok"),
                        "fail" => Some(match rest.find(' ') {
                            Some(pos) => &rest[pos + 1..],
                            None => {
                                // A failure with no message is still a
                                // failure -- the run must learn about it,
                                // not lose it to a panic. This is the
                                // shape the reject side of a live run
                                // produces under load, so treat the
                                // missing message as empty rather than
                                // aborting.
                                error!(
                                    "<red>action_completed fail with no message</>: action {}",
                                    action_id
                                );
                                ""
                            }
                        }),
                        _ => {
                            error!(
                                "<red>unexpected action_completed status</>: <bright-blue>{}</> \
                                 for action {}",
                                action_status, action_id
                            );
                            None
                        }
                    };
                    if let Some(result) = result {
                        // The tick comes from the event line, not from any
                        // plan: it is when the game said the action finished.
                        self.world.actions.insert(
                            action_id,
                            ActionOutcome {
                                tick,
                                result: String::from(result),
                            },
                        );
                    }
                }
            }
            "on_script_path_request_finished" => {
                let parts: Vec<&str> = rest.split('#').collect();
                let id: u32 = parts[0].parse().into_diagnostic()?;
                self.world.path_requests.insert(id, String::from(parts[1]));
            }
            "STATIC_DATA_END" => {
                // handled by OutputReader
            }
            "on_player_left_game" => {
                let player_id: PlayerId = rest.parse().into_diagnostic()?;
                self.world.remove_player(player_id)?;
                // if let Some(websocket_server) = self.websocket_server.as_ref() {
                //     websocket_server
                //         .send(PlayerLeftMessage { player_id })
                //         .await?;
                // }
            }
            "on_research_finished" => {
                // if let Some(websocket_server) = self.websocket_server.as_ref() {
                //     websocket_server.send(ResearchCompletedMessage {}).await?;
                // }
            }
            // This parser reads a stream produced by a game whose schema
            // drifts between versions (see crates/core/tests/live_2_1_payloads.rs),
            // and this crate builds with `panic = "abort"` in release. A
            // `panic!` here used to turn one unparseable line -- one future
            // schema change -- into the death of the whole bot process. Log
            // loudly and skip just this event instead: missing one world
            // update is recoverable, aborting a running multi-bot session is
            // not. Do not silently swallow the error either -- an ignored
            // failure with no log would be worse than the panic it replaces.
            "force" => match serde_json::from_str::<FactorioForce>(rest) {
                Ok(force) => self.world.update_force(force)?,
                Err(err) => {
                    error!("<red>failed to deserialize force</>: {:?} '{}'", err, rest);
                }
            },
            "on_some_entity_created" => match serde_json::from_str::<FactorioEntity>(rest) {
                Ok(entity) => self.world.on_some_entity_created(entity)?,
                Err(err) => {
                    error!("<red>failed to deserialize entity</>: {:?} '{}'", err, rest);
                }
            },
            "on_some_entity_updated" => match serde_json::from_str::<FactorioEntity>(rest) {
                Ok(entity) => self.world.on_some_entity_updated(entity)?,
                Err(err) => {
                    error!("<red>failed to deserialize entity</>: {:?} '{}'", err, rest);
                }
            },
            "on_some_entity_deleted" => match serde_json::from_str::<FactorioEntity>(rest) {
                Ok(entity) => self.world.on_some_entity_deleted(entity)?,
                Err(err) => {
                    error!("<red>failed to deserialize entity</>: {:?} '{}'", err, rest);
                }
            },
            "on_player_main_inventory_changed" => {
                let event: PlayerChangedMainInventoryEvent =
                    serde_json::from_str(rest).into_diagnostic()?;
                let _player_id = event.player_id;
                self.world.player_changed_main_inventory(event)?;
                // if let Some(websocket_server) = self.websocket_server.as_ref() {
                //     websocket_server
                //         .send(PlayerChangedMainInventoryMessage {
                //             player: self.world.players.get(&player_id).unwrap().clone(),
                //         })
                //         .await?;
                // }
            }
            "on_player_changed_position" => {
                let event: PlayerChangedPositionEvent =
                    serde_json::from_str(rest).into_diagnostic()?;
                let _player_id = event.player_id;
                self.world.player_changed_position(event)?;
                // if let Some(websocket_server) = self.websocket_server.as_ref() {
                //     websocket_server
                //         .send(PlayerChangedPositionMessage {
                //             player: self.world.players.get(&player_id).unwrap().clone(),
                //         })
                //         .await?;
                // }
            }
            "on_player_changed_distance" => {
                let event: PlayerChangedDistanceEvent =
                    serde_json::from_str(rest).into_diagnostic()?;
                let _player_id = event.player_id;
                self.world.player_changed_distance(event)?;
                // if let Some(websocket_server) = self.websocket_server.as_ref() {
                //     websocket_server
                //         .send(PlayerDistanceChangedMessage {
                //             player: self.world.players.get(&player_id).unwrap().clone(),
                //         })
                //         .await?;
                // }
            }
            "mined_item" => {
                // info!("tick!");
            }
            "tick" => {
                // info!("tick!");
            }
            // A sampler in `mods/BotBridge/control.lua` (`sample_bots` or
            // `sample_force`) caught an error via `pcall` rather than letting
            // it raise into Factorio's tick loop -- see `record_sample_failure`
            // there. That pcall exists so a broken sampler degrades to a
            // missed sample instead of killing the game, but a silently
            // dropped failure would just trade one invisible bug for another,
            // so it is surfaced here instead of being logged as an
            // "unexpected action".
            "sample_error" => {
                error!("<red>BotBridge sampler failure</>: {}", rest);
            }
            _ => {
                error!("<red>unexpected action</>: <bright-blue>{}</>", action);
            }
        };
        Ok(())
    }

    pub fn on_init(&self) -> Result<()> {
        self.world.entity_graph.connect()?;
        self.world.flow_graph.update()?;
        Ok(())
    }

    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        OutputParser {
            // websocket_server,
            world: Arc::new(FactorioWorld::new()),
        }
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.world.clone()
    }
}
