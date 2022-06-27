use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::rlua;
use factorio_bot_core::rlua::{Context, Table};
use factorio_bot_core::rlua_serde;
use factorio_bot_core::test_utils::draw_world;
use factorio_bot_core::types::{PlayerId, Position, Rect};
use std::path::PathBuf;
use std::sync::Arc;

pub fn create_lua_world(
    ctx: Context,
    _world: Arc<FactorioWorld>,
    cwd: PathBuf,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Factorio World
-- Internal representation of a Factorio world.
--
-- @module world

local world = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return world"#))?;

    let world = _world.clone();
    map_table.set(
        "__doc_fn_recipe",
        String::from(
            r#"
--- lookup recipe by name
-- The name as defined by https://wiki.factorio.com/Materials_and_recipes
-- @string name name of item to craft
-- @return table FactorioRecipe object
function world.recipe(name)
end
"#,
        ),
    )?;
    map_table.set(
        "recipe",
        ctx.create_function(move |ctx, name: String| match world.recipes.get(&name) {
            Some(recipe) => rlua_serde::to_value(ctx, recipe.clone()),
            None => Ok(rlua::Value::Nil),
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_fn_player",
        String::from(
            r#"
--- lookup player by id
-- The player id will start at 1 and increment.
-- @int player_id id of player
-- @return table FactorioPlayer object
function world.player(player_id)
end
"#,
        ),
    )?;
    map_table.set(
        "player",
        ctx.create_function(
            move |ctx, player_id: PlayerId| match world.players.get(&player_id) {
                Some(player) => rlua_serde::to_value(ctx, player.clone()),
                None => Ok(rlua::Value::Nil),
            },
        )?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_fn_find_free_resource_rect",
        String::from(
            r#"
--- find non-blocked rectangle with given resource
-- The ...
-- @string ore_name name of item to craft
-- @int width name of item to craft
-- @int height name of item to craft
-- @param near x/y table
-- @return table FactorioPlayer object
function world.find_free_resource_rect(ore_name, width, height, near)
end
"#,
        ),
    )?;
    map_table.set(
        "find_free_resource_rect",
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

    let world = _world.clone();
    map_table.set(
        "__doc_fn_find_entities_in_radius",
        String::from(
            r#"
--- find entities at given position/radius with optional filters
-- Sends 
-- @param search_center x/y position table 
-- @int radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return table list of FactorioEntity objects
function world.find_entities_in_radius(search_center, radius, search_name, search_type)
end
"#,
        ),
    )?;
    map_table.set(
        "find_entities_in_radius",
        ctx.create_function(
            move |_ctx,
                  (search_center, radius, search_name, search_type): (
                Table,
                f64,
                Option<String>,
                Option<String>,
            )| {
                let search_center = Position::new(
                    search_center.get("x").unwrap(),
                    search_center.get("y").unwrap(),
                );
                let entities = world.entity_graph.find_entities_in_radius(
                    search_center,
                    radius,
                    search_name,
                    search_type,
                );
                Ok(entities)
            },
        )?,
    )?;
    let world = _world.clone();
    map_table.set(
        "__doc_fn_draw",
        String::from(
            r#"
--- draw world and save as image at given path
-- Sends 
-- @string save_path save path 
function world.draw(save_path)
end
"#,
        ),
    )?;
    map_table.set(
        "draw",
        ctx.create_function(move |_ctx, save_path: String| {
            draw_world(world.clone(), cwd.clone(), &save_path);
            Ok(())
        })?,
    )?;

    let world = _world;
    map_table.set(
        "__doc_fn_inventory",
        String::from(
            r#"
--- counts how many of a given item the player has
-- The ...
-- @int player_id id of player
-- @string item_name name of item
-- @return table list of FactorioEntity objects
function world.inventory(player_id, item_name)
end
"#,
        ),
    )?;
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
