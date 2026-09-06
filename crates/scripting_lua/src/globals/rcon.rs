// Reachable from one line of user Lua, and this crate builds with
// `panic = "abort"`, so every panic here is a remote kill of the whole server
// process rather than a failed script. Two distinct classes lived in this
// file: argument parsing (attacker input) and RCON call results (the game
// server simply being unreachable). The lint keeps both out.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::{AreaFilter, PlayerId, Position, RequestEntity};
use std::sync::Arc;

use super::position_from_lua;

/// An RCON call that fails means the game server is unreachable, not that the
/// script was wrong.
///
/// This is the ordinary unhappy path, not an attack: unwrapping it meant the
/// Factorio server dropping its connection mid-script took the bot server down
/// with it. No attacker required, and looking for a security bug would have
/// walked straight past it.
fn rcon_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("rcon: {err}"))
}

pub fn create_lua_rcon(
    lua: &Lua,
    _rcon: Arc<FactorioRcon>,
    _world: Arc<FactorioSurface>,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- RCON interface
-- methods for sending rcon commands to running factorio instance
--
-- @module rcon

local rcon = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return rcon"#))?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_find_entities_in_radius",
        String::from(
            r#"
--- find entities at given position/radius with optional filters
-- Sends /silent-command remote.call('find_entities_filtered', ...)
-- @param search_center `types.Position`
-- @number radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return {`types.FactorioEntity`}
function rcon.find_entities_in_radius(search_center, radius, search_name, search_type)
end
"#,
        ),
    )?;
    map_table.set(
        "find_entities_in_radius",
        lua.create_async_function(
            move |lua,
                  (search_center, radius, search_name, search_type): (
                LuaTable,
                f64,
                Option<String>,
                Option<String>,
            )| {
                let _rcon = rcon.clone();
                let _lua = lua;
                let search_center = position_from_lua(&search_center, "search_center");
                async move {
                    let search_center = search_center?;
                    let filter = AreaFilter::PositionRadius((search_center, Some(radius)));
                    let result = _rcon
                        .as_ref()
                        .find_entities_filtered(&filter, search_name, search_type.map(|t| vec![t]))
                        .await
                        .map_err(rcon_error)?;
                    _lua.to_value(&result)
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_print",
        String::from(
            r#"
--- print given message on the server
-- Sends /c print(message)
-- @string message
function rcon.print(message)
end
"#,
        ),
    )?;
    map_table.set(
        "print",
        lua.create_async_function(move |_lua, message: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon
                    .as_ref()
                    .print(message.as_str())
                    .await
                    .map_err(rcon_error)?;
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_players",
        String::from(
            r#"
--- the players the game says are connected
-- Sends /silent-command remote.call('players')
--
-- **Asks the game, not the cached world.** `world.player(id)` answers from the
-- planner's view, which contains a bot for every id the run was started with
-- whether or not that client ever connected -- so it is the wrong thing to
-- wait on. A client that is still loading is simply absent from this list.
--
-- Only players with a character are reported: a connection that has not
-- finished spawning cannot act, so counting it would mean planning work for a
-- bot that cannot do it.
-- @treturn {number,...} connected player ids, ascending
function rcon.players()
end
    "#,
        ),
    )?;
    {
        let rcon = _rcon.clone();
        map_table.set(
            "players",
            lua.create_async_function(move |_lua, ()| {
                let rcon = rcon.clone();
                async move {
                    let players = rcon
                        .as_ref()
                        .connected_players()
                        .await
                        .map_err(rcon_error)?;
                    let mut ids: Vec<u32> =
                        players.iter().map(|p| u32::from(p.player_id)).collect();
                    ids.sort_unstable();
                    Ok(ids)
                }
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_last_tick",
        String::from(
            r#"
--- the game tick stamped on the most recent reply
-- Sends nothing: every timed `rcon.*` call already comes back stamped with
-- `game.tick` from inside the game, and this reports the last one seen.
--
-- So it is as current as your last *timed* call, and `nil` before there has
-- been one.
--
-- **Only the timed calls advance it**, which is not the same as all of them.
-- The query calls -- `inventory_contents_at`, `find_entities_in_radius`,
-- `player_info` -- consume the reply body as their return value and never take
-- a tick stamp off it, so polling with one of those leaves this frozen at
-- whatever the last action reported. A measurement loop built on it reads a
-- constant and looks like an instantaneous result.
--
-- The calls that do advance it are the ones that act: `mine`, `craft`,
-- `place_entity`, `insert_to_inventory`, `remove_from_inventory`, `move`,
-- `add_research`, `sampling_start`.
-- @treturn number|nil the tick, or nil if the game has not answered yet
function rcon.last_tick()
end
    "#,
        ),
    )?;
    {
        let rcon = _rcon.clone();
        map_table.set(
            "last_tick",
            lua.create_function(move |_lua, ()| Ok(rcon.as_ref().last_tick()))?,
        )?;
    }

    map_table.set(
        "__doc_entry_game_tick",
        String::from(
            r#"
--- asks the game what tick it is **now**
-- Sends /silent-command, and needs no mod: `game.tick` is vanilla, so this
-- keeps working against a save whose BotBridge copy is older than this binary.
--
-- **The difference from `rcon.last_tick` is the whole reason this exists.**
-- That one reports the stamp on the last *timed* call, and only the calls that
-- ACT carry one -- so a loop that waits without dispatching anything reads a
-- frozen number there and concludes that no time passed. This asks, so it
-- advances whether or not anything was dispatched. `supervisor.witness` is
-- built on exactly that: it waits a stated number of ticks while dispatching
-- nothing at all, and it can only say how long it waited because of this.
--
-- Ticks, never seconds. A headless server sharing a machine with graphical
-- clients does not deliver 60 a second: run `run-1788320177-77989` waited the
-- modelled 4032 ticks in wall clock and got ~3599 real ones, then asked a
-- furnace for 20 plates and found 18. Converting a tick budget to seconds and
-- sleeping is a weaker claim than reading this clock.
--
-- `nil` when the game answered nothing readable -- never a zero, which would
-- read as tick zero.
-- @treturn number|nil the current game tick, or nil if the game did not answer
function rcon.game_tick()
end
    "#,
        ),
    )?;
    {
        let rcon = _rcon.clone();
        map_table.set(
            "game_tick",
            lua.create_async_function(move |_lua, ()| {
                let rcon = rcon.clone();
                async move { rcon.as_ref().game_tick().await.map_err(rcon_error) }
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_sampling_start",
        String::from(
            r#"
--- starts the mod's sampling session, truncating any earlier run's samples
-- Sends /silent-command remote.call('sampling_start', run_id)
-- The optional run_id is opaque: it is stamped verbatim onto every sample line
-- as "run":"<run_id>" and interpreted by nothing in the game. Pass one when the
-- samples must later be matched against something produced elsewhere in the
-- same run, so a consumer can check they belong together instead of trusting
-- that tick numbers lining up means they do. Omit it and the lines carry no run
-- key at all -- notably, the previous run's is not inherited.
--
-- This is what makes a run observable: the mod's world-state samplers
-- (research, production and power every 300 ticks, bot inventories every 60)
-- are gated on an active session and write nothing without one.
--
-- Until 2026-09-02 this also drove per-camera screenshots. They are gone --
-- one run wrote 2164 JPEGs / 947 MB of them against 290 MB for the same 45
-- minutes of video, and take_screenshot rendered synchronously inside the game
-- loop. Video is the visual record; see record.start's video option.
-- @string[opt] run_id opaque tag for this session
-- @treturn number the game tick sampling began at
function rcon.sampling_start(run_id)
end
"#,
        ),
    )?;
    map_table.set(
        "sampling_start",
        lua.create_async_function(move |_lua, run_id: Option<String>| {
            let _rcon = rcon.clone();
            async move {
                let tick = _rcon
                    .as_ref()
                    .sampling_start(run_id)
                    .await
                    .map_err(rcon_error)?;
                Ok(tick)
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_sampling_stop",
        String::from(
            r#"
--- stops sampling; samples already written stay on disk
-- Sends /silent-command remote.call('sampling_stop')
-- @treturn number the game tick sampling stopped at
function rcon.sampling_stop()
end
"#,
        ),
    )?;
    map_table.set(
        "sampling_stop",
        lua.create_async_function(move |_lua, ()| {
            let _rcon = rcon.clone();
            async move {
                let tick = _rcon.as_ref().sampling_stop().await.map_err(rcon_error)?;
                Ok(tick)
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_add_research",
        String::from(
            r#"
--- adds research to queue
-- Sends /silent-command remote.call('add_research', technology_name)
--
-- Returns as soon as the technology is QUEUED, not when it is researched.
-- Factorio's add_research answers "did this enter the research queue"; the
-- research itself takes labs, science packs and minutes. Errors are still
-- reported here -- an unknown name, an already-researched technology, unmet
-- prerequisites -- but a successful return says nothing about the technology
-- being available yet. Use goal.researched, which plans a research action the
-- executor waits out, when the next step needs the technology to exist.
-- @string technology_name name of technology to research
function rcon.add_research(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "add_research",
        lua.create_async_function(move |_lua, technology_name: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon
                    .as_ref()
                    .add_research(technology_name.as_str())
                    .await
                    .map_err(rcon_error)?;
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_cheat_technology",
        String::from(
            r#"
--- CHEATs research
-- Sends /silent-command remote.call('cheat_technology', technology_name)
-- @string technology_name name of technology to CHEAT
function rcon.cheat_technology(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "cheat_technology",
        lua.create_async_function(move |_lua, technology_name: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon
                    .as_ref()
                    .cheat_technology(technology_name.as_str())
                    .await
                    .map_err(rcon_error)?;
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_cheat_all_technologies",
        String::from(
            r#"
--- CHEATs all research
-- Sends /silent-command remote.call('cheat_all_technologies')
function rcon.cheat_all_technologies()
end
"#,
        ),
    )?;
    map_table.set(
        "cheat_all_technologies",
        lua.create_async_function(move |_lua, (): ()| {
            let _rcon = rcon.clone();
            async move {
                _rcon
                    .as_ref()
                    .cheat_all_technologies()
                    .await
                    .map_err(rcon_error)?;
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_cheat_item",
        String::from(
            r#"
--- CHEATs given item
-- Sends /silent-command remote.call('cheat_item', ...)
-- @number player_id id of player to give the item to
-- @string name item name
-- @number count how many items to give player
function rcon.cheat_item(player_id, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "cheat_item",
        lua.create_async_function(
            move |_lua, (player_id, name, count): (PlayerId, String, u32)| {
                let _rcon = rcon.clone();
                async move {
                    _rcon
                        .as_ref()
                        .cheat_item(player_id, name.as_str(), count)
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_place_blueprint",
        String::from(
            r#"
--- places a whole blueprint
-- Sends /silent-command remote.call('place_blueprint', ...)
-- @number player_id id of the player doing the placing
-- @string blueprint blueprint string
-- @param position `types.Position`
-- @number direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
-- @bool only_ghosts only places ghost version of entities
-- @tparam {int} helper_player_ids array of player ids which may help
-- @return {`types.FactorioEntity`} the entities the blueprint placed
function rcon.place_blueprint(player_id, blueprint, position, direction, force_build, only_ghosts, helper_player_ids)
end
"#,
        ),
    )?;
    map_table.set(
        "place_blueprint",
        lua.create_async_function(
            move |lua,
                  (
                player_id,
                blueprint,
                position,
                direction,
                force_build,
                only_ghosts,
                helper_player_ids,
            ): (PlayerId, String, LuaTable, u8, bool, bool, Vec<PlayerId>)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let _lua = lua;
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    let result = _rcon
                        .as_ref()
                        .place_blueprint(
                            player_id,
                            blueprint,
                            &position,
                            direction,
                            force_build,
                            only_ghosts,
                            helper_player_ids,
                            &_world,
                        )
                        .await
                        .map_err(rcon_error)?;
                    _lua.to_value(&result)
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_cheat_blueprint",
        String::from(
            r#"
--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('cheat_blueprint', ...)
-- @number player_id id of the player doing the placing
-- @string blueprint blueprint string
-- @param position `types.Position`
-- @number direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
-- @return {`types.FactorioEntity`} the entities the blueprint placed
function rcon.cheat_blueprint(player_id, blueprint, position, direction, force_build)
end
"#,
        ),
    )?;
    map_table.set(
        "cheat_blueprint",
        lua.create_async_function(
            move |lua,
                  (player_id, blueprint, position, direction, force_build): (
                PlayerId,
                String,
                LuaTable,
                u8,
                bool,
            )| {
                let _rcon = rcon.clone();
                let _lua = lua;
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    let result = _rcon
                        .as_ref()
                        .cheat_blueprint(player_id, blueprint, &position, direction, force_build)
                        .await
                        .map_err(rcon_error)?;
                    _lua.to_value(&result)
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_revive_ghost",
        String::from(
            r#"
--- Revives a single ghost entity into the real thing
-- Sends /silent-command remote.call('revive_ghost', ...)
-- @number player_id id of the player doing the reviving
-- @string name name of entity to revive
-- @param position `types.Position` of the ghost
-- @return `types.FactorioEntity` the revived entity
function rcon.revive_ghost(player_id, name, position)
end
"#,
        ),
    )?;
    map_table.set(
        "revive_ghost",
        lua.create_async_function(
            move |lua, (player_id, name, position): (PlayerId, String, LuaTable)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let _lua = lua;
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    let result = _rcon
                        .as_ref()
                        .revive_ghost(player_id, name.as_str(), &position, &_world)
                        .await
                        .map_err(rcon_error)?;
                    _lua.to_value(&result)
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_move",
        String::from(
            r#"
--- Move a player to a different position
-- Blocks until the game reports the walk finished. A goal the bot cannot get
-- within `radius` of is refused before anything is dispatched, rather than
-- walked to somewhere else and reported as success.
-- Sends /silent-command remote.call('action_start_walk_waypoints', ...)
-- @number player_id id of the player to move
-- @param position `types.Position` to walk to
-- @number[opt] radius how close counts as arrived; exact arrival if omitted
function rcon.move(player_id, position, radius)
end
"#,
        ),
    )?;
    map_table.set(
        "move",
        lua.create_async_function(
            move |_lua, (player_id, position, radius): (PlayerId, LuaTable, Option<f64>)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    _rcon
                        .as_ref()
                        .move_player(&_world, player_id, &position, radius)
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_mine",
        String::from(
            r#"
--- Mine a resource with player
-- Sends /silent-command remote.call('action_start_mining', ...)
-- @number player_id id of the player doing the mining
-- @string name name of resource to mine
-- @param position `types.Position` of the resource
-- @number[opt=1] count how many to mine
function rcon.mine(player_id, name, position, count)
end
"#,
        ),
    )?;
    map_table.set(
        "mine",
        lua.create_async_function(
            move |_lua, (player_id, name, position, count): (PlayerId, String, LuaTable, Option<u32>)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    _rcon
                        .as_ref()
                        .player_mine(
                            &_world,
                            player_id,
                            name.as_str(),
                            &position,
                            count.unwrap_or(1),
                        )
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_craft",
        String::from(
            r#"
--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @number player_id id of the player doing the crafting
-- @string name name of item to craft
-- @number[opt=1] count how many to craft
function rcon.craft(player_id, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "craft",
        lua.create_async_function(
            move |_lua, (player_id, name, count): (PlayerId, String, Option<u32>)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                async move {
                    _rcon
                        .as_ref()
                        .player_craft(&_world, player_id, name.as_str(), count.unwrap_or(1))
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_entry_inventory_contents_at",
        String::from(
            r#"
--- Read the inventories of entities at given positions
-- Sends /silent-command remote.call('inventory_contents_at', ...)
-- @param inventories table list of `{name=string, x=number, y=number}` naming
--   an entity and where it stands
-- @return {`types.InventoryResponse`|nil,...} one entry per request, in the
--   order asked; an entry is nil when no such entity was found there
function rcon.inventory_contents_at(inventories)
end
"#,
        ),
    )?;
    map_table.set(
        "inventory_contents_at",
        lua.create_async_function(move |lua, inventories: LuaTable| {
            let _rcon = rcon.clone();
            let _lua = lua;
            let request_entities: Vec<LuaResult<RequestEntity>> = inventories
                .pairs::<u32, LuaTable>()
                .map(|a| {
                    let t: LuaTable = a?.1;
                    let position = Position::new(t.get("x")?, t.get("y")?);
                    Ok(RequestEntity {
                        name: t.get("name")?,
                        position,
                    })
                })
                .collect();
            let request_entities: LuaResult<Vec<RequestEntity>> =
                request_entities.into_iter().collect();
            async move {
                let res = _rcon
                    .as_ref()
                    .inventory_contents_at(request_entities?)
                    .await
                    .map_err(rcon_error)?;
                _lua.to_value(&res)
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_place_entity",
        String::from(
            r#"
--- Places an item by a player
-- Sends /silent-command remote.call('place_entity', ...)
-- @number player_id id of player
-- @string name name of the entity to place
-- @param position  `types.Position`
-- @number direction direction of placed entity
-- @return `types.FactorioEntity`
function rcon.place_entity(player_id, name, position, direction)
end
"#,
        ),
    )?;
    map_table.set(
        "place_entity",
        lua.create_async_function(
            move |lua, (player_id, name, position, direction): (PlayerId, String, LuaTable, u8)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let _lua = lua;
                let position = position_from_lua(&position, "position");

                async move {
                    let position = position?;
                    // `None`: this Lua global has no parameter for an
                    // underground half yet -- see
                    // `FactorioEntity::underground_half`.
                    let result = _rcon
                        .as_ref()
                        .place_entity(player_id, name, position, direction, None, &_world)
                        .await
                        .map_err(rcon_error)?;
                    _lua.to_value(&result)
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_entry_insert_to_inventory",
        String::from(
            r#"
--- Inserts an item into an inventory
-- Sends /silent-command remote.call('insert_to_inventory', ...)
-- @number player_id id of the player reaching into the inventory
-- @string entity_name name entity to insert
-- @param position `types.Position` of inventory
-- @number inventory_type which inventory of the entity to place in, as a
--   `defines.inventory` index -- a number, not a name: it is handed to the
--   game's own `entity.get_inventory(..)` unchanged
-- @string item_name which item to insert
-- @number item_count how many items to insert
function rcon.insert_to_inventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end
"#,
        ),
    )?;
    map_table.set(
        "insert_to_inventory",
        lua.create_async_function(
            move |_lua,
                  (player_id, entity_name, position, inventory_type, item_name, item_count): (
                PlayerId,
                String,
                LuaTable,
                u32,
                String,
                u32,
            )| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    _rcon
                        .as_ref()
                        .insert_to_inventory(
                            player_id,
                            entity_name,
                            position,
                            inventory_type,
                            item_name,
                            item_count,
                            &_world,
                        )
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    let rcon = _rcon;
    let world = _world;
    map_table.set(
        "__doc_entry_remove_from_inventory",
        String::from(
            r#"
--- Removes an item from an inventory
-- Sends /silent-command remote.call('remove_from_inventory', ...)
-- @number player_id id of the player reaching into the inventory
-- @string entity_name name entity to remove
-- @param position  `types.Position` of inventory
-- @number inventory_type which inventory of the entity to remove from, as a
--   `defines.inventory` index -- a number, not a name: it is handed to the
--   game's own `entity.get_inventory(..)` unchanged
-- @string item_name which item to remove
-- @number item_count how many items to remove
function rcon.remove_from_inventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end
"#,
        ),
    )?;
    map_table.set(
        "remove_from_inventory",
        lua.create_async_function(
            move |_lua,
                  (player_id, entity_name, position, inventory_type, item_name, item_count): (
                PlayerId,
                String,
                LuaTable,
                u32,
                String,
                u32,
            )| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position = position_from_lua(&position, "position");
                async move {
                    let position = position?;
                    _rcon
                        .as_ref()
                        .remove_from_inventory(
                            player_id,
                            entity_name,
                            position,
                            inventory_type,
                            item_name,
                            item_count,
                            &_world,
                        )
                        .await
                        .map_err(rcon_error)?;
                    Ok(())
                }
            },
        )?,
    )?;
    Ok(map_table)
}

/// The `-- Sends /silent-command remote.call('X', ...)` line in a doc block is
/// a claim about behaviour, and until now nothing compared it to any.
///
/// `lua_docs::write_lua_docs` renders `rcon.lua` *from* the `__doc_entry_*`
/// strings above, so generation can only ever guarantee that the published
/// file matches the strings — it says nothing about whether the strings match
/// the functions. `inventory_contents_at` shipped with a summary reading
/// "Craft an item with player", a `-- Sends` line naming
/// `action_start_crafting` and a `@return` describing a map of counts, and a
/// green build the entire time.
///
/// Both halves of every check below are read out of source text, so neither
/// side is a list somebody has to remember to update.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    /// This file: the `__doc_entry_*` claims and the bindings they describe.
    const BINDINGS: &str = include_str!("rcon.rs");
    /// The RCON client the bindings delegate to — where the remote-call names
    /// are actually written down, as string literals passed to `remote_call*`.
    const CLIENT: &str = include_str!("../../../core/src/factorio/rcon.rs");
    /// The mod, which decides what names exist to be called at all.
    const MOD_CONTROL: &str = include_str!("../../../../mods/BotBridge/control.lua");

    /// The source up to its own `#[cfg(test)]`.
    ///
    /// Both Rust files are parsed by scanning for markers that this very test
    /// module also writes down — `map_table.set(`, `self.`, `remote_call(` —
    /// and a parser that read its own prose would invent bindings. Cutting at
    /// the test attribute is what keeps the checks looking only at production
    /// code. (The literal here cannot match itself: in the file's bytes it is
    /// a backslash and an `n`, not a newline.)
    fn production_half(source: &str) -> &str {
        source.split("\n#[cfg(test)]").next().unwrap_or(source)
    }

    /// The leading run of identifier characters, which is how every name below
    /// is read once its opening delimiter has been found.
    fn leading_ident(text: &str) -> &str {
        let end = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(text.len());
        &text[..end]
    }

    /// Lines paired with their byte offsets, exactly — `split_inclusive` keeps
    /// the terminator in the slice, so the running offset cannot drift the way
    /// `lines()` plus an assumed `+ 1` does on CRLF.
    fn lines_with_offsets(src: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let mut at = 0usize;
        for raw in src.split_inclusive('\n') {
            out.push((at, raw.trim_end_matches(['\n', '\r'])));
            at += raw.len();
        }
        out
    }

    /// The names `remote.add_interface("botbridge", { ... })` registers.
    ///
    /// Read off the mod rather than listed here: this is the set of calls that
    /// exist, and it is the mod's to decide.
    fn mod_interface_exports() -> BTreeSet<String> {
        let start = MOD_CONTROL
            .find("remote.add_interface(\"botbridge\"")
            .expect("BotBridge's control.lua must register the botbridge interface");
        let open = MOD_CONTROL[start..]
            .find('{')
            .expect("the interface registration must open a table")
            + start;
        let close = MOD_CONTROL[open..]
            .find('}')
            .expect("the interface table must close")
            + open;
        MOD_CONTROL[open + 1..close]
            .lines()
            .filter_map(|line| line.split('=').next())
            .map(str::trim)
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .map(str::to_string)
            .collect()
    }

    /// Every `impl`-level `fn` in the RCON client, sliced from one header to
    /// the next.
    ///
    /// Sliced by header position rather than by matching braces: the client is
    /// full of `format!("{...}")`, and a brace counter that does not know
    /// about string literals runs off the end of the first function that has
    /// one — which, measured while building this, silently merged twenty
    /// functions into one and made every remote call look reachable from every
    /// method. Slicing between headers cannot do that.
    fn client_functions() -> BTreeMap<String, &'static str> {
        let src = production_half(CLIENT);
        let mut headers: Vec<(usize, String)> = Vec::new();
        for (offset, line) in lines_with_offsets(src) {
            let Some(rest) = line.strip_prefix("    ") else {
                continue;
            };
            // Exactly four spaces: anything deeper is nested inside a body.
            if rest.starts_with(' ') {
                continue;
            }
            let rest = rest
                .strip_prefix("pub(crate) ")
                .or_else(|| rest.strip_prefix("pub "))
                .unwrap_or(rest);
            let rest = rest.strip_prefix("async ").unwrap_or(rest);
            let Some(rest) = rest.strip_prefix("fn ") else {
                continue;
            };
            let name = leading_ident(rest);
            if !name.is_empty() {
                headers.push((offset, name.to_string()));
            }
        }
        let mut functions = BTreeMap::new();
        for (index, (offset, name)) in headers.iter().enumerate() {
            let end = headers
                .get(index + 1)
                .map(|(next, _)| *next)
                .unwrap_or(src.len());
            functions.insert(name.clone(), &src[*offset..end]);
        }
        functions
    }

    /// The remote-call names a single client function sends itself.
    fn remote_calls_in(body: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for marker in ["remote_call(", "remote_call_timed(", "remote_call_json("] {
            for (idx, _) in body.match_indices(marker) {
                let after = body[idx + marker.len()..].trim_start();
                let Some(rest) = after.strip_prefix('"') else {
                    continue;
                };
                let name = leading_ident(rest);
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
        names
    }

    /// The sibling methods a single client function calls.
    ///
    /// `move_player` sends nothing itself; it delegates to `move_player_timed`,
    /// which delegates to `action_start_walk_waypoints`, which is where the
    /// name finally appears. Without this edge the check could only see the
    /// handful of methods that send directly.
    fn self_calls_in(body: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for (idx, _) in body.match_indices("self") {
            // `myself.foo()` is not a self-call.
            if body[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                continue;
            }
            let after = body[idx + "self".len()..].trim_start();
            let Some(after) = after.strip_prefix('.') else {
                continue;
            };
            let after = after.trim_start();
            let name = leading_ident(after);
            if name.is_empty() {
                continue;
            }
            if after[name.len()..].trim_start().starts_with('(') {
                names.insert(name.to_string());
            }
        }
        names
    }

    /// Every remote call reachable from one client method, following
    /// `self.other(...)` edges.
    fn reachable_remote_calls(
        root: &str,
        functions: &BTreeMap<String, &'static str>,
    ) -> BTreeSet<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue = vec![root.to_string()];
        let mut calls = BTreeSet::new();
        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(body) = functions.get(&name) else {
                continue;
            };
            calls.extend(remote_calls_in(body));
            for edge in self_calls_in(body) {
                if functions.contains_key(&edge) {
                    queue.push(edge);
                }
            }
        }
        calls
    }

    /// The `map_table.set("KEY", ...)` blocks of this file, keyed and sliced
    /// from one to the next — so `__doc_entry_move` and the `move` binding it
    /// documents are both recoverable by name.
    fn binding_blocks() -> BTreeMap<String, &'static str> {
        let src = production_half(BINDINGS);
        let marker = "map_table.set(";
        let mut starts: Vec<(usize, String)> = Vec::new();
        for (idx, _) in src.match_indices(marker) {
            let after = src[idx + marker.len()..].trim_start();
            let Some(rest) = after.strip_prefix('"') else {
                continue;
            };
            let key = leading_ident(rest);
            if !key.is_empty() {
                starts.push((idx, key.to_string()));
            }
        }
        let mut blocks = BTreeMap::new();
        for (index, (offset, key)) in starts.iter().enumerate() {
            let end = starts
                .get(index + 1)
                .map(|(next, _)| *next)
                .unwrap_or(src.len());
            blocks.insert(key.clone(), &src[*offset..end]);
        }
        blocks
    }

    /// The name a doc block's `-- Sends ... remote.call('X', ...)` line claims.
    fn documented_remote_call(doc: &str) -> Option<String> {
        let marker = "remote.call('";
        let idx = doc.find(marker)?;
        let name = leading_ident(&doc[idx + marker.len()..]);
        (!name.is_empty()).then(|| name.to_string())
    }

    /// The client method a binding block actually invokes. Every binding here
    /// reaches the client the same way: `_rcon.as_ref().method(..)`.
    fn client_method_of(block: &str) -> Option<String> {
        let marker = ".as_ref()";
        let idx = block.find(marker)?;
        let rest = block[idx + marker.len()..].trim_start();
        let rest = rest.strip_prefix('.')?;
        let name = leading_ident(rest.trim_start());
        (!name.is_empty()).then(|| name.to_string())
    }

    /// A remote call the client sends but the mod does not export is a script
    /// error that only shows up against a running game.
    ///
    /// Separate from the doc check below and not subsumed by it: this one
    /// ranges over *every* call the client sends, including the ones no Lua
    /// binding documents, and it is the mod — not the docs — that it holds
    /// them to.
    #[test]
    fn every_remote_call_the_client_sends_is_exported_by_the_mod() {
        let exports = mod_interface_exports();
        assert!(
            exports.len() >= 20,
            "only {} names parsed out of BotBridge's remote interface; the parse \
             broke and this test would then assert nothing",
            exports.len()
        );
        let functions = client_functions();
        let mut sent: BTreeSet<String> = BTreeSet::new();
        for body in functions.values() {
            sent.extend(remote_calls_in(body));
        }
        assert!(
            sent.len() >= 20,
            "only {} remote calls found in the RCON client; the parse broke and \
             this test would then assert nothing",
            sent.len()
        );
        let missing: Vec<&String> = sent.difference(&exports).collect();
        assert!(
            missing.is_empty(),
            "the RCON client sends {missing:?}, which `remote.add_interface(\"botbridge\", ..)` \
             in mods/BotBridge/control.lua does not export -- every such call fails against a \
             running game. Fix the name in `crates/core/src/factorio/rcon.rs`, or export it \
             from the mod's interface table."
        );
    }

    /// A doc block must name a remote call its own binding can actually send.
    ///
    /// This is the check that would have caught `inventory_contents_at`: the
    /// name it advertised, `action_start_crafting`, is a perfectly real export,
    /// so the mod on its own has nothing to say about it. What makes it wrong
    /// is that the binding calls `FactorioRcon::inventory_contents_at`, from
    /// which `action_start_crafting` is not reachable.
    ///
    /// Membership in the reachable set rather than equality with a single
    /// name, because a method legitimately sends several: `move_player` asks
    /// for a path and then walks it. Erring towards permissive is deliberate —
    /// this check should never cry wolf about a doc that is right — and it
    /// still leaves the sets at two to five names out of the mod's twenty-eight.
    #[test]
    fn each_rcon_doc_block_names_the_remote_call_its_binding_reaches() {
        let blocks = binding_blocks();
        let functions = client_functions();
        assert!(
            functions.len() >= 50,
            "only {} functions parsed out of the RCON client; the parse broke and \
             this test would then assert nothing",
            functions.len()
        );

        // Counted whether or not the claim holds. An earlier spelling counted
        // only the blocks that *passed*, which made the floor fire before the
        // diagnostic did: the mutation used to prove this test discriminates
        // reduced the count by one and tripped "the parse broke" instead of
        // naming the wrong call. A health check on the parse must not be
        // reduced by the defect it is standing guard over.
        let mut with_sends = 0usize;
        let mut problems: Vec<String> = Vec::new();
        for (key, doc) in &blocks {
            let Some(name) = key.strip_prefix("__doc_entry_") else {
                continue;
            };
            let Some(block) = blocks.get(name) else {
                problems.push(format!(
                    "  `__doc_entry_{name}` documents `rcon.{name}`, but no \
                     `map_table.set(\"{name}\", ..)` installs it -- add the binding, or drop \
                     the documentation"
                ));
                continue;
            };
            let Some(method) = client_method_of(block) else {
                problems.push(format!(
                    "  `rcon.{name}`'s binding does not reach the client through \
                     `_rcon.as_ref().<method>(..)`, so its `-- Sends` line cannot be checked -- \
                     call the client that way, or this guard has to be taught the new shape"
                ));
                continue;
            };
            let reachable = reachable_remote_calls(&method, &functions);
            let claim = documented_remote_call(doc);
            if claim.is_some() {
                with_sends += 1;
            }
            match claim {
                Some(claimed) if reachable.contains(&claimed) => {}
                Some(claimed) => problems.push(format!(
                    "  `rcon.{name}` says it sends remote.call('{claimed}'), but its binding \
                     calls `FactorioRcon::{method}`, which sends {reachable:?} -- name one of \
                     those in the `-- Sends` line, or point the binding at the method that \
                     sends '{claimed}'"
                )),
                None if !reachable.is_empty() => problems.push(format!(
                    "  `rcon.{name}` has no `-- Sends /silent-command remote.call('..')` line, \
                     but its binding calls `FactorioRcon::{method}`, which sends {reachable:?} \
                     -- say which one in the doc string"
                )),
                None => {}
            }
        }
        assert!(
            problems.is_empty(),
            "{} rcon doc block(s) describe a call their binding does not make:\n{}",
            problems.len(),
            problems.join("\n")
        );
        assert!(
            with_sends >= 15,
            "only {with_sends} doc blocks carried a `-- Sends` line at all; the parse \
             broke and this test would then assert nothing"
        );
    }
}
