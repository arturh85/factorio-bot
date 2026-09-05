use std::sync::Arc;

use crate::factorio::ticks::ActionOutcome;
use crate::factorio::world::{
    DeathEvent, FactorioWorld, ResearchTriggerEvent, RespawnEvent, TeleportEvent,
};
// use crate::factorio::ws::{
//     FactorioWebSocketServer, PlayerChangedMainInventoryMessage, PlayerChangedPositionMessage,
//     PlayerDistanceChangedMessage, PlayerLeftMessage, ResearchCompletedMessage,
// };
use crate::types::{
    ChunkPosition, FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic,
    FactorioItemPrototype, FactorioRecipe, FactorioTile, PlayerChangedDistanceEvent,
    PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, PlayerId, Pos, Position, Rect,
};
use miette::{IntoDiagnostic, Result, miette};

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
                // `split_once`, not `split('#').collect()` then `parts[1]`:
                // that indexing panics on a line with no `#` at all, taking
                // the whole output parser down with it, and it truncated a
                // payload that contained a second `#`. The result half is
                // stored verbatim -- it is JSON on success and one of the
                // mod's plain-text verdicts on failure, and it is
                // `sleep_for_path_request_result`'s job to tell those apart.
                let (id, result) = rest.split_once('#').ok_or_else(|| {
                    miette!("on_script_path_request_finished without a '#': {rest}")
                })?;
                let id: u32 = id.parse().into_diagnostic()?;
                self.world.path_requests.insert(id, String::from(result));
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
            // One of `control.lua`'s two remaining `player.teleport` sites
            // fired (the stuck-walk site is gone: a stalled leg fails the walk
            // and Rust asks for a fresh path).
            // Before this arm existed none of them were observable at all:
            // `on_player_changed_position` fires identically for a teleport
            // and a walked step, so a run whose bots teleported repeatedly
            // recorded ordinary-looking walk durations with nothing to say
            // otherwise. Logging is the immediate, always-on visibility; the
            // event is also queued on `self.world` for
            // `crates/scripting_lua`'s `record.teleports()` to drain into
            // `events.jsonl` -- this crate cannot call the recorder directly
            // (it lives in `crates/scripting_lua`, which depends on this
            // crate and not the other way around).
            "teleport" => match serde_json::from_str::<TeleportEvent>(rest) {
                Ok(event) => {
                    warn!(
                        "<yellow>teleport</>: bot {} {} from {} to {} ({:.1} tiles){}",
                        event.player_id,
                        event.reason,
                        event.from,
                        event.to,
                        event.distance,
                        event
                            .action_id
                            .map(|id| format!(", action {id}"))
                            .unwrap_or_default(),
                    );
                    self.world.record_teleport(tick, event);
                }
                Err(err) => {
                    error!(
                        "<red>failed to deserialize teleport</>: {:?} '{}'",
                        err, rest
                    );
                }
            },
            // A stalled character stood on a tile something collides with
            // and is walking to the nearest clear one before its walk is
            // failed (`walk_step_clear_landing` in `mods/BotBridge/control.lua`).
            // Not a teleport -- it is a walk -- so it is logged, not queued
            // as one; the walk's own failure carries the same fact into the
            // record (`then stepped clear to (x/y)`).
            "walk_step_clear" => match serde_json::from_str::<serde_json::Value>(rest) {
                Ok(event) => {
                    warn!(
                        "<yellow>step clear</>: bot {} is pinned at {} ({}), walking to {} before its walk is failed",
                        event["player_id"],
                        event["from"],
                        event["cause"].as_str().unwrap_or("cause unknown"),
                        event["to"],
                    );
                }
                Err(err) => {
                    error!(
                        "<red>failed to deserialize walk_step_clear</>: {:?} '{}'",
                        err, rest
                    );
                }
            },
            // A bot lost its character (`on_player_died` in
            // `mods/BotBridge/control.lua`). Logged here so it is visible
            // while the run happens, and queued on `self.world` for
            // `crates/scripting_lua`'s `record.deaths()` to write into
            // `events.jsonl` -- the same two-crate arrangement `teleport`
            // above explains. The player is deliberately NOT removed from
            // `world.players`: it is still connected and will respawn, and
            // its last position is the only one anything has.
            "player_died" => match serde_json::from_str::<DeathEvent>(rest) {
                Ok(event) => {
                    warn!(
                        "<red>bot {} died</> at tick {}{}{}",
                        event.player_id,
                        tick,
                        event
                            .cause
                            .as_deref()
                            .map(|c| format!(", killed by {c}"))
                            .unwrap_or_default(),
                        event
                            .respawn_in
                            .map(|t| format!(", respawns in {t} ticks"))
                            .unwrap_or_default(),
                    );
                    self.world.record_death(tick, event);
                }
                Err(err) => {
                    error!(
                        "<red>failed to deserialize player_died</>: {:?} '{}'",
                        err, rest
                    );
                }
            },
            "player_respawned" => match serde_json::from_str::<RespawnEvent>(rest) {
                Ok(event) => {
                    info!("bot {} respawned at tick {}", event.player_id, tick);
                    self.world.record_respawn(tick, event);
                }
                Err(err) => {
                    error!(
                        "<red>failed to deserialize player_respawned</>: {:?} '{}'",
                        err, rest
                    );
                }
            },
            "research_trigger_emulated" => {
                match serde_json::from_str::<ResearchTriggerEvent>(rest) {
                    Ok(event) => {
                        info!(
                            "trigger technology {} earned at tick {} ({} {}/{})",
                            event.technology,
                            tick,
                            event.trigger,
                            event.count(),
                            event.needed
                        );
                        self.world.record_research_trigger(tick, event);
                    }
                    Err(err) => {
                        error!(
                            "<red>failed to deserialize research_trigger_emulated</>: {:?} '{}'",
                            err, rest
                        );
                    }
                }
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

    /// Like [`OutputParser::new`], but parsing into a world the caller
    /// already holds a handle to -- so a test can drive a `writeout`-shaped
    /// line through `parse` and then inspect (or hand to another crate) the
    /// exact [`FactorioWorld`] it landed in, e.g. via
    /// [`FactorioWorld::drain_teleports`].
    pub fn with_world(world: Arc<FactorioWorld>) -> Self {
        OutputParser { world }
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.world.clone()
    }
}

#[cfg(test)]
mod death_tests {
    use super::*;
    use crate::factorio::world::BotLifeEvent;

    /// The two writeouts `on_player_died` / `on_player_respawned`
    /// (`mods/BotBridge/control.lua`) produce, in the exact shape
    /// `helpers.table_to_json` renders them, land on the world's death queue
    /// in the order they happened -- a death and then its respawn, never
    /// reordered by being two kinds -- with the mod's tick, not the parse
    /// order, on each.
    #[test]
    fn a_death_and_its_respawn_reach_the_queue_in_order_with_their_ticks() {
        let mut parser = OutputParser::new();
        parser
            .parse(
                1234,
                "player_died",
                r#"{"cause":"medium-worm-turret","cause_type":"turret","player_id":2,"position":{"x":12.5,"y":-3},"respawn_in":587}"#,
            )
            .expect("a death parses");
        parser
            .parse(
                1900,
                "player_respawned",
                r#"{"player_id":2,"position":{"x":0,"y":0}}"#,
            )
            .expect("a respawn parses");
        let drained = parser.world().drain_deaths();
        assert_eq!(drained.len(), 2);
        let (tick, died) = &drained[0];
        assert_eq!(*tick, 1234);
        match died {
            BotLifeEvent::Died(d) => {
                assert_eq!(d.player_id, 2);
                assert_eq!(d.cause.as_deref(), Some("medium-worm-turret"));
                assert_eq!(d.cause_type.as_deref(), Some("turret"));
                assert_eq!(d.respawn_in, Some(587));
                assert_eq!(
                    d.position.as_ref().map(|p| (p.x(), p.y())),
                    Some((12.5, -3.0))
                );
            }
            other => panic!("expected a death first, got {other:?}"),
        }
        let (tick, back) = &drained[1];
        assert_eq!(*tick, 1900);
        assert!(matches!(back, BotLifeEvent::Respawned(r) if r.player_id == 2));
        assert!(
            parser.world().drain_deaths().is_empty(),
            "a drain takes everything"
        );
    }

    /// A death the mod could not fully describe -- no cause, no readable
    /// respawn timer, no position -- is still a death. Every optional field
    /// is `None`, and `player_id` alone is enough to queue it.
    #[test]
    fn a_death_with_nothing_but_a_player_id_still_parses() {
        let mut parser = OutputParser::new();
        parser
            .parse(7, "player_died", r#"{"player_id":3}"#)
            .expect("a bare death parses");
        let drained = parser.world().drain_deaths();
        assert_eq!(drained.len(), 1);
        match &drained[0].1 {
            BotLifeEvent::Died(d) => {
                assert_eq!(d.player_id, 3);
                assert_eq!(d.cause, None);
                assert_eq!(d.respawn_in, None);
                assert_eq!(d.position, None);
            }
            other => panic!("expected a death, got {other:?}"),
        }
    }

    /// A malformed line is logged and dropped, not raised: the parser task
    /// reads every later line of the run, and one bad death must not take
    /// them all down.
    #[test]
    fn a_malformed_death_line_is_dropped_rather_than_failing_the_parser() {
        let mut parser = OutputParser::new();
        parser
            .parse(7, "player_died", "not json")
            .expect("a malformed death does not raise");
        assert!(parser.world().drain_deaths().is_empty());
    }
}

#[cfg(test)]
mod path_request_tests {
    use super::*;

    /// The result half is stored verbatim, `#` and all. It used to be
    /// `split('#').collect()` with `parts[1]` read out, which kept only up to
    /// the next `#` and panicked outright on a line that had none -- inside the
    /// parser task, taking every later line down with it.
    #[test]
    fn a_result_containing_a_hash_is_stored_whole() {
        let mut parser = OutputParser::new();
        parser
            .parse(1, "on_script_path_request_finished", "7#a#b")
            .expect("a well-formed line parses");
        assert_eq!(
            parser.world().path_requests.get(&7).map(|v| v.clone()),
            Some("a#b".to_string())
        );
    }

    /// The mod's plain-text verdicts travel this way too, and reach
    /// `sleep_for_path_request_result` unaltered -- it is that function's job
    /// to tell them from JSON, not this one's.
    #[test]
    fn a_pathfinder_refusal_is_stored_as_the_mod_wrote_it() {
        let mut parser = OutputParser::new();
        parser
            .parse(
                1,
                "on_script_path_request_finished",
                "9#Error: try again later!",
            )
            .expect("a refusal parses");
        assert_eq!(
            parser.world().path_requests.get(&9).map(|v| v.clone()),
            Some("Error: try again later!".to_string())
        );
    }

    #[test]
    fn a_line_with_no_separator_is_an_error_rather_than_a_panic() {
        let mut parser = OutputParser::new();
        let err = parser
            .parse(1, "on_script_path_request_finished", "7")
            .expect_err("a line with no '#' cannot be read");
        assert!(err.to_string().contains("without a '#'"), "{err}");
    }
}
