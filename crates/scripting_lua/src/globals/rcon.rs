use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::{AreaFilter, PlayerId, Position, RequestEntity};
use std::sync::Arc;

pub fn create_lua_rcon(
    lua: &Lua,
    _rcon: Arc<FactorioRcon>,
    _world: Arc<FactorioWorld>,
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
                let _lua = lua.clone();
                let search_center = Position::new(
                    search_center.get("x").unwrap(),
                    search_center.get("y").unwrap(),
                );
                async move {
                    let filter = AreaFilter::PositionRadius((search_center, Some(radius)));
                    let result = _rcon
                        .as_ref()
                        .find_entities_filtered(&filter, search_name, search_type)
                        .await
                        .unwrap();
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
                _rcon.as_ref().print(message.as_str()).await.unwrap();
                Ok(())
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
                    .unwrap();
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
                    .unwrap();
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
                _rcon.as_ref().cheat_all_technologies().await.unwrap();
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
                        .unwrap();
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
-- @number player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position `types.Position`
-- @number direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
-- @bool only_ghosts only places ghost version of entities
-- @tparam {int} helper_player_ids array of player ids which may help
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
                let _lua = lua.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
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
                        .unwrap();
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
-- @number player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position `types.Position`
-- @number direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
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
                let _lua = lua.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    let result = _rcon
                        .as_ref()
                        .cheat_blueprint(player_id, blueprint, &position, direction, force_build)
                        .await
                        .unwrap();
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
--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('revive_ghost', ...)
-- @number player_id id of player to give the item to
-- @string name name of entity to revive
-- @param position `types.Position`
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
                let _lua = lua.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    let result = _rcon
                        .as_ref()
                        .revive_ghost(player_id, name.as_str(), &position, &_world)
                        .await
                        .unwrap();
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
-- Sends /silent-command remote.call('action_start_walk_waypoints', ...)
-- @number player_id id of player to give the item to
-- @param position `types.Position`
-- @number radius radius
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
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    _rcon
                        .as_ref()
                        .move_player(&_world, player_id, &position, radius)
                        .await
                        .unwrap();
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
-- @number player_id id of player to give the item to
-- @string name name of resource to mine
-- @param position `types.Position`
-- @number count how many to mine
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
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
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
                        .unwrap();
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
-- @number player_id id of plaer
-- @string name name of item to craft
-- @number count how many to craft
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
                        .unwrap();
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
--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @param inventories table list of `types.Position` to check
-- @return {[string]=number,...}
function rcon.inventory_contents_at(inventories)
end
"#,
        ),
    )?;
    map_table.set(
        "inventory_contents_at",
        lua.create_async_function(move |lua, inventories: LuaTable| {
            let _rcon = rcon.clone();
            let _lua = lua.clone();
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
            let request_entities: LuaResult<Vec<RequestEntity>> = request_entities.into_iter().collect();
            async move {
                let res = _rcon
                    .as_ref()
                    .inventory_contents_at(request_entities?)
                    .await
                    .unwrap();
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
-- @number player_id id of plaer
-- @string name name of item to craft
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
                let _lua = lua.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());

                async move {
                    let result = _rcon
                        .as_ref()
                        .place_entity(player_id, name, position, direction, &_world)
                        .await
                        .unwrap();
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
-- @number player_id id of plaer
-- @string entity_name name entity to insert
-- @param position `types.Position` of inventory
-- @string inventory_type which type of inventory to place in
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
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
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
                        .unwrap();
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
-- @number player_id id of plaer
-- @string entity_name name entity to remove
-- @param position  `types.Position` of inventory
-- @string inventory_type which type of inventory to remove from
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
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
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
                        .unwrap();
                    Ok(())
                }
            },
        )?,
    )?;
    Ok(map_table)
}
