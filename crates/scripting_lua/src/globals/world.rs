use factorio_bot_core::factorio::util::blueprint_build_area;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::factorio_blueprint::BlueprintCodec;
use factorio_bot_core::rlua::{Context, Table};
use factorio_bot_core::rlua_serde;
use factorio_bot_core::test_utils::draw_world;
use factorio_bot_core::types::{FactorioBlueprintInfo, PlayerId, Position, Rect};
use factorio_bot_core::{rlua, serde_json};
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
-- @return `types.FactorioRecipe`
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
-- @return `types.FactorioPlayer`
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
-- @param near `types.Position`
-- @return `types.FactorioPlayer`
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

    map_table.set(
        "__doc_fn_parse_blueprint",
        String::from(
            r#"
--- Parse blueprint
-- @string blueprint blueprint string
-- @return `types.FactorioBlueprintInfo`
function world.parse_blueprint(...)
end
"#,
        ),
    )?;
    let world = _world.clone();
    map_table.set(
        "parse_blueprint",
        ctx.create_function(move |ctx, (blueprint, label): (String, String)| {
            let decoded =
                BlueprintCodec::decode_string(&blueprint).expect("failed to parse blueprint");
            let rect = blueprint_build_area(world.entity_prototypes.clone(), &blueprint);
            let response = FactorioBlueprintInfo {
                rect: rect.clone(),
                label: label.clone(),
                blueprint: blueprint.clone(),
                width: rect.width() as u16,
                height: rect.height() as u16,
                data: serde_json::to_value(decoded).unwrap(),
            };
            Ok(rlua_serde::to_value(ctx, response))
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_fn_find_entities_in_radius",
        String::from(
            r#"
--- find entities at given position/radius with optional filters
-- Sends 
-- @param search_center `types.Position` 
-- @int radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return {`types.FactorioEntity`}
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
-- @return {`types.FactorioEntity`}
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
