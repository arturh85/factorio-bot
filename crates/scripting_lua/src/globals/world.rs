// Reachable from one line of user Lua, and this crate builds with
// `panic = "abort"`, so every panic here is a remote kill of the whole server
// process rather than a failed script. The lint is scoped to this file: the
// sibling `rcon` and `plan` modules carry the same defect and are handled
// under their own tasks.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use factorio_bot_core::draw::draw_world;
use factorio_bot_core::factorio::util::blueprint_build_area;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::factorio_blueprint::BlueprintCodec;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::scripts::resolve_write_path;
use factorio_bot_core::serde_json;
use factorio_bot_core::types::{FactorioBlueprintInfo, PlayerId};
use std::path::PathBuf;
use std::sync::Arc;

use super::{path_error, position_from_lua, relative_to};

/// See [`crate::globals::create_lua_globals`] for why the sandbox needs both a
/// `scripts_root` (the boundary) and a `script_dir` (what relative paths are
/// resolved against).
pub fn create_lua_world(
    lua: &Lua,
    _world: Arc<FactorioWorld>,
    scripts_root: PathBuf,
    script_dir: PathBuf,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
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
        "__doc_entry_recipe",
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
        lua.create_function(move |lua, name: String| match world.recipes.get(&name) {
            Some(recipe) => lua.to_value(&*recipe),
            None => Ok(LuaValue::Nil),
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_entry_player",
        String::from(
            r#"
--- lookup player by id
-- The player id will start at 1 and increment.
-- @number player_id id of player
-- @return `types.FactorioPlayer`
function world.player(player_id)
end
"#,
        ),
    )?;
    map_table.set(
        "player",
        lua.create_function(
            move |lua, player_id: PlayerId| match world.players.get(&player_id) {
                Some(player) => lua.to_value(&*player),
                None => Ok(LuaValue::Nil),
            },
        )?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_entry_find_free_resource_rect",
        String::from(
            r#"
--- find non-blocked rectangle with given resource
-- Searches the resource patches of `ore_name`, nearest to `near` first, for a
-- free rectangle of the given size. **Finding no room is not an error:** it is
-- an absence, and it is reported as `nil`. There is no ore, and no size, for
-- which a successful answer is the all-zero rectangle, so `nil` is the only
-- value that cannot be mistaken for a site at the map origin.
-- @string ore_name name of the resource, e.g. "iron-ore"
-- @number width width of the rectangle to fit, in tiles
-- @number height height of the rectangle to fit, in tiles
-- @param near `types.Position` to search outwards from
-- @return `types.Rect` the free rectangle, or nil if no patch of `ore_name`
--   has room for one -- including when there is no such patch at all
function world.find_free_resource_rect(ore_name, width, height, near)
end
"#,
        ),
    )?;
    map_table.set(
        "find_free_resource_rect",
        lua.create_function(
            move |lua, (ore_name, width, height, near): (String, u32, u32, LuaTable)| {
                let patches = world.entity_graph.resource_patches(ore_name.as_str());
                let near = position_from_lua(&near, "near")?;
                for patch in patches {
                    if let Some(rect) = patch.find_free_rect(width, height, &near) {
                        return lua.to_value(&rect);
                    }
                }
                // Not `Rect::default()`. An all-zero rectangle is a valid-looking
                // 0x0 site at the map origin, so a caller who forgot to check
                // built there instead of failing, and the wreckage read as a
                // planner bug rather than a missing patch.
                Ok(LuaValue::Nil)
            },
        )?,
    )?;

    map_table.set(
        "__doc_entry_parse_blueprint",
        String::from(
            r#"
--- Parse blueprint
-- @string blueprint blueprint string
-- @string label name to record on the result; it is carried through
--   unchanged, not read out of the blueprint
-- @return `types.FactorioBlueprintInfo`
function world.parse_blueprint(blueprint, label)
end
"#,
        ),
    )?;
    let world = _world.clone();
    map_table.set(
        "parse_blueprint",
        lua.create_function(move |lua, (blueprint, label): (String, String)| {
            let decoded = BlueprintCodec::decode_string(&blueprint).map_err(|err| {
                LuaError::RuntimeError(format!("failed to parse blueprint: {err}"))
            })?;
            let rect = blueprint_build_area(world.entity_prototypes.clone(), &blueprint);
            let response = FactorioBlueprintInfo {
                rect: rect.clone(),
                label,
                blueprint,
                width: rect.width() as u16,
                height: rect.height() as u16,
                data: serde_json::to_value(decoded).map_err(|err| {
                    LuaError::RuntimeError(format!("failed to serialise blueprint: {err}"))
                })?,
            };
            lua.to_value(&response)
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_entry_find_entities_in_radius",
        String::from(
            r#"
--- find entities at given position/radius with optional filters
-- Answered from the entity graph this process already holds -- no RCON round
-- trip, and so no cost per call. `rcon.find_entities_in_radius` is the one
-- that asks the game.
-- @param search_center `types.Position`
-- @number radius searches in circular radius around search_center
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
        lua.create_function(
            move |_lua,
                  (search_center, radius, search_name, search_type): (
                LuaTable,
                f64,
                Option<String>,
                Option<String>,
            )| {
                let search_center = position_from_lua(&search_center, "search_center")?;
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
        "__doc_entry_draw",
        String::from(
            r#"
--- draw world and save as image at given path
-- Bounded to the scripts directory exactly like `globals.file_write`: the
-- path is relative to the calling script, its parent directory must already
-- exist, an existing symlink at the target is refused, and a path that would
-- leave the scripts directory is refused rather than clamped.
-- @string save_path where to write the image, relative to the scripts directory
function world.draw(save_path)
end
"#,
        ),
    )?;
    let root = scripts_root;
    let dir = script_dir;
    map_table.set(
        "draw",
        lua.create_function(move |_lua, save_path: String| {
            let resolved = resolve_write_path(
                &root,
                &relative_to(&root, &dir, &save_path).map_err(path_error)?,
            )
            .map_err(path_error)?;
            draw_world(world.clone(), &resolved)
                .map_err(|err| LuaError::RuntimeError(format!("{err}")))
        })?,
    )?;

    let world = _world;
    map_table.set(
        "__doc_entry_inventory",
        String::from(
            r#"
--- counts how many of a given item the player has
-- Reads the player's main inventory. An item the player does not carry counts
-- zero; only an unknown player is an error.
-- @number player_id id of player
-- @string item_name name of item
-- @return number how many the player holds
function world.inventory(player_id, item_name)
end
"#,
        ),
    )?;
    map_table.set(
        "inventory",
        lua.create_function(move |_lua, (player_id, item_name): (PlayerId, String)| {
            match world.players.get(&player_id) {
                Some(player) => match player.main_inventory.get(&item_name) {
                    Some(cnt) => Ok(*cnt),
                    None => Ok(0),
                },
                None => Err(LuaError::RuntimeError("player not found".into())),
            }
        })?,
    )?;

    Ok(map_table)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use factorio_bot_core::plan::planner::Planner;
    use factorio_bot_core::types::{
        PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, Position,
    };
    use std::collections::BTreeMap;

    /// Builds the `world` table exactly the way a run does.
    ///
    /// Every binding in this file closes over the one `Arc<FactorioWorld>`
    /// given to `create_lua_world`, and `lua_runner` hands it that Arc once,
    /// before the chunk runs -- so taking the handle here through `Planner` in
    /// the same order is the point, not incidental setup. A test that passed
    /// the world Arc straight in would not be testing what production does.
    fn lua_world_for(world: &Arc<FactorioWorld>) -> (Lua, LuaTable) {
        let mut planner = Planner::new(world.clone(), None);
        planner.initiate_missing_players_with_default_inventory(1);
        // `lua_runner` refreshes and then takes exactly this handle.
        planner.update_plan_world();
        let bound = planner.plan_world.clone();

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let root = std::env::temp_dir();
        let table = create_lua_world(&lua, bound, root.clone(), root).expect("world table");
        (lua, table)
    }

    /// The `world.*` surface must answer from the running game.
    ///
    /// Measured live before the fix: a bot standing at `(-22.29, 35.34)` read
    /// back as `(0, 0)` -- 41.8 tiles and some 6,200 ticks out -- because the
    /// bindings held a deep copy of the world taken before the run started.
    ///
    /// The pre-mutation read is asserted too. Without it a binding that simply
    /// always reported the post-mutation value would pass.
    #[test]
    fn world_player_reports_the_position_the_game_has_now() {
        let world = Arc::new(FactorioWorld::new());
        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: 1,
                position: Position::new(0., 0.),
            })
            .expect("seed");

        let (lua, table) = lua_world_for(&world);
        lua.globals().set("world", table).expect("set global");

        let before: f64 = lua
            .load("return world.player(1).position.x")
            .eval()
            .expect("read before");
        assert_eq!(before, 0., "precondition: the binding starts at the origin");

        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: 1,
                position: Position::new(-22.29, 35.34),
            })
            .expect("move");

        let after: (f64, f64) = lua
            .load("local p = world.player(1).position return p.x, p.y")
            .eval()
            .expect("read after");
        assert_eq!(
            after,
            (-22.29, 35.34),
            "world.player is frozen at the pre-run snapshot"
        );
    }

    /// The other half of the live symptom: the ten iron plates the run had
    /// just smelted read as zero, which is what made a goal that had genuinely
    /// succeeded look like it had produced nothing.
    ///
    /// Asserted separately from the position test rather than folded into it:
    /// `players` is one `DashMap`, so a single read would let either field
    /// stand in for the other, and `world.inventory` is a different binding
    /// with its own captured handle.
    #[test]
    fn world_inventory_reports_the_items_the_run_produced() {
        let world = Arc::new(FactorioWorld::new());
        let (lua, table) = lua_world_for(&world);
        lua.globals().set("world", table).expect("set global");

        let before: u32 = lua
            .load("return world.inventory(1, 'iron-plate')")
            .eval()
            .expect("read before");
        assert_eq!(before, 0, "precondition: the run starts with no plates");

        let mut inventory: BTreeMap<String, u32> = BTreeMap::new();
        inventory.insert("iron-plate".to_owned(), 10);
        world
            .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                1, inventory,
            ))
            .expect("smelt");

        let after: u32 = lua
            .load("return world.inventory(1, 'iron-plate')")
            .eval()
            .expect("read after");
        assert_eq!(
            after, 10,
            "world.inventory is frozen: the plates the run smelted are invisible"
        );
    }
}
