use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::rlua;
use factorio_bot_core::rlua::{Context, Table};
use factorio_bot_core::types::{AreaFilter, PlayerId, Position, RequestEntity};
use rlua_async::ContextExt;
use std::sync::Arc;

pub fn create_lua_rcon(
    ctx: Context,
    _rcon: Arc<FactorioRcon>,
    _world: Arc<FactorioWorld>,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
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
        "__doc_fn_findEntitiesInRadius",
        String::from(
            r#"
--- find entities at given position/radius with optional filters
-- Sends /silent-command remote.call('find_entities_filtered', ...)
-- @param search_center x/y position
-- @int radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return table list of FactorioEntity objects
function rcon.findEntitiesInRadius(search_center, radius, search_name, search_type)
end
"#,
        ),
    )?;
    map_table.set(
        "findEntitiesInRadius",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (search_center, radius, search_name, search_type): (
                Table,
                f64,
                Option<String>,
                Option<String>,
            )| {
                let _rcon = rcon.clone();
                let search_center = Position::new(
                    search_center.get("x").unwrap(),
                    search_center.get("y").unwrap(),
                );
                async move {
                    let filter = AreaFilter::PositionRadius((search_center, Some(radius)));
                    Ok(_rcon
                        .as_ref()
                        .find_entities_filtered(&filter, search_name, search_type)
                        .await
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_fn_print",
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
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, message: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon.as_ref().print(message.as_str()).await.unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_fn_addResearch",
        String::from(
            r#"
--- adds research to queue
-- Sends /silent-command remote.call('add_research', technology_name)
-- @string technology_name name of technology to research
function rcon.addResearch(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "addResearch",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, technology_name: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon.as_ref().add_research(technology_name.as_str()).await.unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_fn_cheatTechnology",
        String::from(
            r#"
--- CHEATs research
-- Sends /silent-command remote.call('cheat_technology', technology_name)
-- @string technology_name name of technology to CHEAT
function rcon.cheatTechnology(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "cheatTechnology",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, technology_name: String| {
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
        "__doc_fn_cheatAllTechnologies",
        String::from(
            r#"
--- CHEATs all research
-- Sends /silent-command remote.call('cheat_all_technologies')
function rcon.cheatAllTechnologies()
end
"#,
        ),
    )?;
    map_table.set(
        "cheatAllTechnologies",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, (): ()| {
            let _rcon = rcon.clone();
            async move {
                _rcon.as_ref().cheat_all_technologies().await.unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_fn_cheatItem",
        String::from(
            r#"
--- CHEATs given item
-- Sends /silent-command remote.call('cheat_item', ...)
-- @int player_id id of player to give the item to
-- @string name item name
-- @int count how many items to give player
function rcon.cheatItem(player_id, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "cheatItem",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, name, count): (PlayerId, String, u32)| {
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
        "__doc_fn_placeBlueprint",
        String::from(
            r#"
--- places a whole blueprint
-- Sends /silent-command remote.call('place_blueprint', ...)
-- @int player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position x/y position
-- @int direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
-- @bool only_ghosts only places ghost version of entities
-- @param helper_player_ids array of player ids which may help
function rcon.placeBlueprint(player_id, blueprint, position, direction, force_build, only_ghosts, helper_player_ids)
end
"#,
        ),
    )?;
    map_table.set(
        "placeBlueprint",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (
                player_id,
                blueprint,
                position,
                direction,
                force_build,
                only_ghosts,
                helper_player_ids,
            ): (PlayerId, String, Table, u8, bool, bool, Vec<PlayerId>)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    Ok(_rcon
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
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "__doc_fn_cheatBlueprint",
        String::from(
            r#"
--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('cheat_blueprint', ...)
-- @int player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position x/y position
-- @int direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
function rcon.cheatBlueprint(player_id, blueprint, position, direction, force_build)
end
"#,
        ),
    )?;
    map_table.set(
        "cheatBlueprint",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (player_id, blueprint, position, direction, force_build): (
                PlayerId,
                String,
                Table,
                u8,
                bool,
            )| {
                let _rcon = rcon.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    Ok(_rcon
                        .as_ref()
                        .cheat_blueprint(player_id, blueprint, &position, direction, force_build)
                        .await
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_fn_reviveGhost",
        String::from(
            r#"
--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('revive_ghost', ...)
-- @int player_id id of player to give the item to
-- @string name name of entity to revive
-- @param position x/y position
function rcon.reviveGhost(player_id, name, position)
end
"#,
        ),
    )?;
    map_table.set(
        "reviveGhost",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, name, position): (PlayerId, String, Table)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());
                async move {
                    Ok(_rcon
                        .as_ref()
                        .revive_ghost(player_id, name.as_str(), &position, &_world)
                        .await
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_fn_move",
        String::from(
            r#"
--- Move a player to a different position
-- Sends /silent-command remote.call('action_start_walk_waypoints', ...)
-- @int player_id id of player to give the item to
-- @param position x/y position
-- @int radius radius
function rcon.move(player_id, position, radius)
end
"#,
        ),
    )?;
    map_table.set(
        "move",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, position, radius): (PlayerId, Table, Option<f64>)| {
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
        "__doc_fn_mine",
        String::from(
            r#"
--- Mine a resource with player
-- Sends /silent-command remote.call('action_start_mining', ...)
-- @int player_id id of player to give the item to
-- @string name name of resource to mine
-- @param position x/y position
-- @int count how many to mine
function rcon.mine(player_id, name, position, count)
end
"#,
        ),
    )?;
    map_table.set(
        "mine",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, name, position, count): (PlayerId, String, Table, Option<u32>)| {
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
        "__doc_fn_craft",
        String::from(
            r#"
--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @int player_id id of plaer
-- @string name name of item to craft
-- @int count how many to craft
function rcon.craft(player_id, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "craft",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, name, count): (PlayerId, String, Option<u32>)| {
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
        "__doc_fn_inventoryContentsAt",
        String::from(
            r#"
--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @param inventories table of x/y positions to check
-- @return table of inventory contents
function rcon.inventoryContentsAt(inventories)
end
"#,
        ),
    )?;
    map_table.set(
        "inventoryContentsAt",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, inventories: Table| {
            let _rcon = rcon.clone();
            let request_entities: Vec<RequestEntity> = inventories
                .pairs::<u32, Table>()
                .into_iter()
                .map(|a| {
                    let t: Table = a.unwrap().1;
                    let position = Position::new(t.get("x").unwrap(), t.get("y").unwrap());
                    RequestEntity {
                        name: t.get("name").unwrap(),
                        position,
                    }
                })
                .collect();
            async move {
                Ok(_rcon
                    .as_ref()
                    .inventory_contents_at(request_entities)
                    .await
                    .unwrap())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_fn_placeEntity",
        String::from(
            r#"
--- Places an item by a player
-- Sends /silent-command remote.call('place_entity', ...)
-- @int player_id id of plaer
-- @string name name of item to craft
-- @param position  x/y position table
-- @int direction direction of placed entity
-- @return FactorioEntity object
function rcon.placeEntity(player_id, name, position, direction)
end
"#,
        ),
    )?;
    map_table.set(
        "placeEntity",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx, (player_id, name, position, direction): (PlayerId, String, Table, u8)| {
                let _rcon = rcon.clone();
                let _world = world.clone();
                let position =
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap());

                async move {
                    Ok(_rcon
                        .as_ref()
                        .place_entity(player_id, name, position, direction, &_world)
                        .await
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    let world = _world.clone();
    map_table.set(
        "__doc_fn_insertToInventory",
        String::from(
            r#"
--- Inserts an item into an inventory
-- Sends /silent-command remote.call('insert_to_inventory', ...)
-- @int player_id id of plaer
-- @string entity_name name entity to insert
-- @param position  x/y position table of inventory
-- @string inventory_type which type of inventory to place in
-- @string item_name which item to insert
-- @int item_count how many items to insert
function rcon.insertToInventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end
"#,
        ),
    )?;
    map_table.set(
        "insertToInventory",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (player_id, entity_name, position, inventory_type, item_name, item_count): (
                PlayerId,
                String,
                Table,
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
        "__doc_fn_removeFromInventory",
        String::from(
            r#"
--- Removes an item from an inventory
-- Sends /silent-command remote.call('remove_from_inventory', ...)
-- @int player_id id of plaer
-- @string entity_name name entity to remove
-- @param position  x/y position table of inventory
-- @string inventory_type which type of inventory to remove from
-- @string item_name which item to remove
-- @int item_count how many items to remove
function rcon.removeFromInventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end
"#,
        ),
    )?;
    map_table.set(
        "removeFromInventory",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (player_id, entity_name, position, inventory_type, item_name, item_count): (
                PlayerId,
                String,
                Table,
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
