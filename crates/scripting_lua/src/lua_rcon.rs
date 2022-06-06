use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{AreaFilter, PlayerId, Position, RequestEntity};
use rlua::{Context, Table};
use rlua_async::ContextExt;
use std::sync::Arc;

pub fn create_lua_rcon(
    ctx: Context,
    _rcon: Arc<FactorioRcon>,
    _world: Arc<FactorioWorld>,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
    let rcon = _rcon.clone();
    map_table.set(
        "findEntitiesInRadius",
        ctx.create_async_function_mut::<_, _, _, _>(
            move |_ctx,
                  (search_center, radius, name, entity_type): (
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
                        .find_entities_filtered(&filter, name, entity_type)
                        .await
                        .unwrap())
                }
            },
        )?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "print",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, str: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon.as_ref().print(str.as_str()).await.unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "addResearch",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, tech: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon.as_ref().add_research(tech.as_str()).await.unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
    map_table.set(
        "cheatTechnology",
        ctx.create_async_function_mut::<_, _, _, _>(move |_ctx, tech: String| {
            let _rcon = rcon.clone();
            async move {
                _rcon
                    .as_ref()
                    .cheat_technology(tech.as_str())
                    .await
                    .unwrap();
                Ok(())
            }
        })?,
    )?;
    let rcon = _rcon.clone();
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
