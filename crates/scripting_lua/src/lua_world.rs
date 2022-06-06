use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{PlayerId, Position, Rect};
use rlua::{Context, Table};
use std::sync::Arc;

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
            move |ctx, player_id: PlayerId| match world.players.get(&player_id) {
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
                info!("entity graph {:?}", *world.entity_graph.inner_graph());
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
        ctx.create_function(move |_ctx, (player_id, item_name): (PlayerId, String)| {
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
