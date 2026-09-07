// Reachable from one line of user Lua, and this crate builds with
// `panic = "abort"`, so every panic here is a remote kill of the whole server
// process rather than a failed script. The lint is scoped to this file: the
// sibling `rcon` and `plan` modules carry the same defect and are handled
// under their own tasks.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use factorio_bot_core::draw::draw_world;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::util::blueprint_build_area;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::factorio_blueprint::BlueprintCodec;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::scripts::resolve_write_path;
use factorio_bot_core::serde_json;
use factorio_bot_core::types::{FactorioBlueprintInfo, PlayerId, Rect};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use super::goal::BufferRefresher;
use super::{path_error, position_from_lua, relative_to};
use crate::blocked::blocked_boxes_report;

/// Says what the buffer read before a dump found, or why there was none.
///
/// Deliberately not shared with `goal::plan`'s narration of the same call: the
/// two failures have different consequences and the whole value of the line is
/// that it names one. A plan that could not read its buffers re-mines ore
/// *now*; a dump that could not read them writes a file that looks complete
/// and misleads every offline plan made from it afterwards, possibly days
/// later, with nothing in the file to say so.
async fn narrate_buffer_refresh(refresher: Option<&BufferRefresher>) {
    let Some(refresher) = refresher else {
        // No RCON: a dump taken with no game behind it (`--clients 0`, or the
        // doc generator). There was nothing to ask, so there is nothing to
        // report and no run to mislead.
        return;
    };
    match refresher().await {
        Ok(0) => factorio_bot_core::paris::info!(
            "nothing to read before dumping: the world knows of no furnace or chest yet, so the \
             dump's inventories are empty because the world is, not because nobody looked"
        ),
        Ok(asked) => factorio_bot_core::paris::info!(
            "read the contents of <bright-blue>{}</> buffer(s) into the dump",
            asked
        ),
        Err(ref err) => {
            factorio_bot_core::paris::warn!(
                "<red>could not read what is in this world's buffers</>; dumping anyway, with \
                 whatever contents were already known. Anything standing in a furnace or chest \
                 that this run never looked inside will be missing from the file, and a plan made \
                 from it offline will make those materials again instead of fetching them"
            );
            factorio_bot_core::tracing::warn!(
                "buffer refresh failed before world.dump, writing the dump with the contents \
                 already known: {}",
                err
            );
        }
    }
}

/// See [`crate::globals::create_lua_globals`] for why the sandbox needs both a
/// `scripts_root` (the boundary) and a `script_dir` (what relative paths are
/// resolved against).
///
/// `rcon` is here for exactly one binding: `world.dump` asks the game what is
/// standing in this world's buffers before it writes the file, so that the
/// dump carries container contents rather than whatever the run happened to
/// have asked about already. See the `__doc_entry_dump` text for why that read
/// belongs to the dump rather than to the script calling it. `None` -- no
/// game, or `--clients 0` -- is a legitimate mode and simply skips the read.
pub fn create_lua_world(
    lua: &Lua,
    world: Arc<FactorioSurface>,
    scripts_root: PathBuf,
    script_dir: PathBuf,
    rcon: Option<Arc<FactorioRcon>>,
) -> LuaResult<LuaTable> {
    // Built here rather than inside the binding for the reason
    // `create_lua_goal` builds the same callback: a `Planner` is a context
    // holder over exactly these two `Arc`s, so rebuilding one per call is
    // cheaper than reasoning about the lifetime of one that is kept -- and a
    // callback is what lets the tests below drive the real `dump` binding
    // without an RCON connection, which is the only way to assert *when* the
    // read happens relative to the write.
    let refresher: Option<BufferRefresher> = rcon.map(|rcon| {
        let world = world.clone();
        Arc::new(move || {
            let planner = Planner::new(world.clone(), Some(rcon.clone()));
            Box::pin(async move {
                planner
                    .refresh_buffers()
                    .await
                    .map_err(|err| err.to_string())
            }) as Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>
        }) as BufferRefresher
    });
    create_lua_world_with(lua, world, scripts_root, script_dir, refresher)
}

/// [`create_lua_world`] with the buffer read supplied rather than built from
/// RCON.
///
/// The only caller in production is [`create_lua_world`]; the tests use it to
/// drive the real bindings -- `world.dump` included -- against a stub.
pub(crate) fn create_lua_world_with(
    lua: &Lua,
    _world: Arc<FactorioSurface>,
    scripts_root: PathBuf,
    script_dir: PathBuf,
    buffer_refresher: Option<BufferRefresher>,
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
        "__doc_entry_blocked_boxes",
        String::from(
            r#"
--- what the world model believes blocks building over a rectangle
-- Reads the entity graph's *blocked* tree, which is a different structure from
-- the one `world.find_entities_in_radius` reads. That one holds a whitelist of
-- entity types -- furnaces, inserters, belts, containers, the two big rocks --
-- because it models a factory; trees, small rocks, cliffs, units and water
-- never enter it. The blocked tree holds every collision box the model has
-- been told about, which is what a build refusal is decided against.
--
-- **The boxes are anonymous, and this does not invent a name for them.** The
-- tree stores one bit per box, `minable` (the entity's type was `tree` or
-- `simple-entity`), and no name at all, so each box comes back reading
-- "a box, minable, source unknown". A refusal that recited "a tree, cliff,
-- rock or unit" named four things it had not read, and named a tree at a tile
-- the game said held nothing but ore.
--
-- **Read `coverage` before `boxes`.** An empty `boxes` means the ground is
-- clear only when `coverage` is `"charted"`. `"unknown"` means no tile of the
-- rectangle has ever been written out to the model, so the empty list says
-- nothing whatever about the ground; `"partial"` means some of it was;
-- `"outside_model"` means the rectangle leaves the region the index covers at
-- all.
--
-- Every field is always present and `boxes` is always a table, so
-- `for _, b in ipairs(result.boxes)` is safe with no guard. Nothing here is
-- ever nil.
--
-- Answered from the model this process already holds -- no RCON round trip.
-- Diff it against `rcon.find_entities_filtered` over the same rectangle to see
-- where the model and the game disagree; `scripts/blocked_diff.lua` does
-- exactly that.
--
-- The rectangle is widened to whole tiles and the widened one is returned as
-- `area`. Asking about more than 16384 tiles (128x128) is an error, as is a
-- rectangle with a swapped corner.
-- @param left_top `types.Position` top-left corner of the rectangle
-- @param right_bottom `types.Position` bottom-right corner of the rectangle
-- @return `types.BlockedBoxReport`
function world.blocked_boxes(left_top, right_bottom)
end
"#,
        ),
    )?;
    map_table.set(
        "blocked_boxes",
        lua.create_function(move |lua, (left_top, right_bottom): (LuaTable, LuaTable)| {
            let left_top = position_from_lua(&left_top, "left_top")?;
            let right_bottom = position_from_lua(&right_bottom, "right_bottom")?;
            let area = Rect::new(&left_top, &right_bottom);
            let report = blocked_boxes_report(&world.entity_graph, &area)
                .map_err(|err| LuaError::RuntimeError(err.to_string()))?;
            lua.to_value(&report)
        })?,
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
    let (draw_root, draw_dir) = (root.clone(), dir.clone());
    map_table.set(
        "draw",
        lua.create_function(move |_lua, save_path: String| {
            let resolved = resolve_write_path(
                &draw_root,
                &relative_to(&draw_root, &draw_dir, &save_path).map_err(path_error)?,
            )
            .map_err(path_error)?;
            draw_world(world.clone(), &resolved)
                .map_err(|err| LuaError::RuntimeError(format!("{err}")))
        })?,
    )?;

    let world = _world.clone();
    map_table.set(
        "__doc_entry_dump",
        String::from(
            r#"
--- write the whole world to a JSON file, for planning against it later
-- What comes out is the input to offline planning:
-- `factorio-bot plan --world <file> --goal <spec>` reads it back and runs the
-- same `expand()` and `schedule()` a run does, with no Factorio, no RCON and
-- no workspace -- which turns evaluating a planner change from a twenty-minute
-- run into a fraction of a second. The plan made from the file is the plan
-- that would have been made here; `crates/planner/tests/world_round_trip.rs`
-- pins that.
--
-- **Call it where the interesting state is.** A dump taken at setup is a
-- fresh map, which is worth having; a dump taken after a milestone also
-- carries every site the game refused a build at, every walk it refused a
-- route to, and what is inside every furnace and chest -- which is what makes
-- replanning from that milestone mean anything.
--
-- **It asks the game what is in those containers first**, one RCON round trip
-- naming the furnaces and chests the world knows about, exactly as `goal.plan`
-- does immediately before it plans. That is a side effect, and it is a
-- deliberate one: container contents are pulled, never pushed (Factorio raises
-- no event for "a chest's contents changed"), so a dump that only wrote back
-- what somebody had already happened to ask about would write `inventories:
-- []` for every script that has not planned -- and an offline
-- `factorio-bot plan --world <that file>` would then plan as if every furnace
-- and chest in the world were empty, mining ore again to remake materials that
-- are standing in a container ten tiles away. The file gives no sign of this;
-- it looks complete. Making the read the caller's job trades a surprising
-- side effect for a silent trap, and the trap is worse. With no game behind
-- the script (`--clients 0`) there is nothing to ask and nothing is asked.
--
-- The read is still only a read: what lands in the file is a reading taken
-- moments before the write, so it is as stale as any other pull. If it fails,
-- the dump is written anyway, with a warning -- an older reading in a file
-- somebody is watching is worth more than no file at all.
--
-- Bounded to the scripts directory exactly like `world.draw` and
-- `globals.file_write`: the path is relative to the calling script, its parent
-- directory must already exist, an existing symlink at the target is refused,
-- and a path that would leave the scripts directory is refused rather than
-- clamped.
-- @string save_path where to write the dump, relative to the scripts directory
function world.dump(save_path)
end
"#,
        ),
    )?;
    map_table.set(
        "dump",
        lua.create_async_function(move |_lua, save_path: String| {
            // Resolved before the read, not after: a path that leaves the
            // scripts root is refused without having touched the game, so a
            // bad call costs nothing and the boundary stays the first thing
            // this binding checks.
            let resolved = relative_to(&root, &dir, &save_path)
                .map_err(path_error)
                .and_then(|relative| resolve_write_path(&root, &relative).map_err(path_error));
            let world = world.clone();
            let refresher = buffer_refresher.clone();
            async move {
                let resolved = resolved?;
                narrate_buffer_refresh(refresher.as_ref()).await;
                world
                    .dump_to(&resolved)
                    .map_err(|err| LuaError::RuntimeError(format!("{err}")))
            }
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
    use factorio_bot_core::types::{
        PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent, Position,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Builds the `world` table exactly the way a run does.
    ///
    /// Every binding in this file closes over the one `Arc<FactorioSurface>`
    /// given to `create_lua_world`, and `lua_runner` hands it that Arc once,
    /// before the chunk runs -- so taking the handle here through `Planner` in
    /// the same order is the point, not incidental setup. A test that passed
    /// the world Arc straight in would not be testing what production does.
    fn lua_world_for(world: &Arc<FactorioSurface>) -> (Lua, LuaTable) {
        let mut planner = Planner::new(world.clone(), None);
        planner.initiate_missing_players_with_default_inventory(1);
        // `lua_runner` refreshes and then takes exactly this handle.
        planner.update_plan_world();
        let bound = planner.plan_world.clone();

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let root = std::env::temp_dir();
        let table = create_lua_world(&lua, bound, root.clone(), root, None).expect("world table");
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
    /// A [`BufferRefresher`] that counts its calls and answers `outcome`.
    ///
    /// It also records the world's inventories at the moment it was called, so
    /// a test can assert the read happened *before* the write rather than
    /// merely that it happened. A refresher that ran after `dump_to` would
    /// count the same and leave the file exactly as wrong as no refresher at
    /// all.
    fn counting_refresher(outcome: Result<usize, String>) -> (BufferRefresher, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let refresher: BufferRefresher = Arc::new(move || {
            let outcome = outcome.clone();
            let seen = seen.clone();
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                outcome
            }) as Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>
        });
        (refresher, calls)
    }

    /// Builds the `world` table over `world` with the given buffer read.
    fn dumping_world(
        lua: &Lua,
        world: &Arc<FactorioSurface>,
        root: &std::path::Path,
        refresher: Option<BufferRefresher>,
    ) -> LuaTable {
        let mut planner = Planner::new(world.clone(), None);
        planner.initiate_missing_players_with_default_inventory(1);
        planner.update_plan_world();
        create_lua_world_with(
            lua,
            planner.plan_world.clone(),
            root.to_path_buf(),
            root.to_path_buf(),
            refresher,
        )
        .expect("world table")
    }

    /// A furnace holding forty plates, standing where a run left it.
    fn furnace_holding(world: &FactorioSurface, at: &Position, count: u32) {
        use factorio_bot_core::types::{Direction, FactorioEntity, InventoryResponse};
        world
            .on_some_entity_created(FactorioEntity::new_stone_furnace(at, Direction::North))
            .expect("the furnace is placed");
        world.observe_inventories(vec![InventoryResponse {
            name: "stone-furnace".into(),
            position: at.clone(),
            output_inventory: Box::new(Some(vec![
                factorio_bot_core::types::InventoryItemWithQuality {
                    name: "iron-plate".into(),
                    quality: "normal".into(),
                    count,
                },
            ])),
            fuel_inventory: Box::new(None),
            input_inventory: Box::new(None),
        }]);
    }

    /// A dump is the world *now*, and it carries what the planner reads.
    ///
    /// The reason a dump exists is offline planning, and the reason it has to
    /// be taken mid-run is that a fresh map's ledgers are all empty -- so the
    /// two things worth asserting are that the file is live (not the pre-run
    /// snapshot the other tests in this module were written for) and that an
    /// observed inventory reaches it. `crates/planner/tests/world_round_trip.rs`
    /// takes it from there and pins that the plan is unchanged.
    #[tokio::test]
    async fn world_dump_writes_the_world_the_run_has_now() {
        use factorio_bot_core::serde_json;

        let world = Arc::new(FactorioSurface::new());
        let root = tempfile::tempdir().expect("a scripts root");
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = dumping_world(&lua, &world, root.path(), None);
        lua.globals().set("world", table).expect("set global");

        // After the bindings were built, exactly as a run does it.
        let at = Position::new(-34., 40.);
        furnace_holding(&world, &at, 40);

        lua.load("world.dump('world.json')")
            .exec_async()
            .await
            .expect("dumps");

        let written = std::fs::read_to_string(root.path().join("world.json")).expect("a file");
        let back: FactorioSurface = serde_json::from_str(&written).expect("a readable world");
        assert_eq!(
            back.observed_inventories(),
            world.observed_inventories(),
            "the dump did not carry what the run had looked inside"
        );
        assert!(
            back.entity_graph.entity_at(&at).is_some(),
            "the dump did not carry the furnace the run placed"
        );
    }

    /// **The dump asks the game what is in the buffers, and asks before it
    /// writes.**
    ///
    /// The defect this closes: nothing on the dump path ever called
    /// `Planner::refresh_buffers`, so `inventories` came out `[]` for every
    /// script that had not also planned -- and the whole `Withdraw` half of
    /// the planner was therefore unreachable from
    /// `factorio-bot plan --world <dump>`, the loop this project evaluates
    /// planner changes with. A file that says `[]` is indistinguishable from a
    /// world with empty chests, which is why the failure was invisible.
    ///
    /// The refresher here writes the contents when it runs, so the assertion
    /// is about *ordering*: a read issued after `dump_to` would leave the file
    /// as empty as no read at all.
    #[tokio::test]
    async fn world_dump_reads_the_buffers_before_it_writes() {
        use factorio_bot_core::serde_json;

        let world = Arc::new(FactorioSurface::new());
        let root = tempfile::tempdir().expect("a scripts root");
        let at = Position::new(-34., 40.);

        let filling = world.clone();
        let at_fill = at.clone();
        let refresher: BufferRefresher = Arc::new(move || {
            let world = filling.clone();
            let at = at_fill.clone();
            Box::pin(async move {
                furnace_holding(&world, &at, 40);
                Ok(1usize)
            }) as Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>
        });

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = dumping_world(&lua, &world, root.path(), Some(refresher));
        lua.globals().set("world", table).expect("set global");

        assert!(
            world.observed_inventories().is_empty(),
            "precondition: nothing has looked in anything yet"
        );

        lua.load("world.dump('world.json')")
            .exec_async()
            .await
            .expect("dumps");

        let written = std::fs::read_to_string(root.path().join("world.json")).expect("a file");
        let back: FactorioSurface = serde_json::from_str(&written).expect("a readable world");
        let carried = back.observed_inventories();
        assert_eq!(
            carried.len(),
            1,
            "the dump was written before the buffers were read, so it carries none of them"
        );
        assert_eq!(
            carried[0].1.output.get("iron-plate"),
            Some(&40),
            "the dump carried a reading, but not the contents in it: {carried:?}"
        );
    }

    /// A dump with no game behind it asks nothing and still writes.
    ///
    /// `--clients 0` and the doc generator both build this table with no RCON.
    /// Making the read mandatory would turn the offline modes into failures;
    /// the empty `inventories` they produce is honest, because there is no
    /// game holding anything.
    #[tokio::test]
    async fn world_dump_without_a_game_writes_anyway() {
        let world = Arc::new(FactorioSurface::new());
        let root = tempfile::tempdir().expect("a scripts root");
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = dumping_world(&lua, &world, root.path(), None);
        lua.globals().set("world", table).expect("set global");

        lua.load("world.dump('world.json')")
            .exec_async()
            .await
            .expect("dumps with no rcon");
        assert!(
            root.path().join("world.json").exists(),
            "no file was written"
        );
    }

    /// A buffer read that fails does not cost the caller the dump.
    ///
    /// The contents in the file are then whatever was already known, which is
    /// the same staleness every pull has, one round trip further out. Losing
    /// the whole file instead would be strictly worse: a dump is usually taken
    /// at a milestone that cannot be reproduced without repeating the run.
    #[tokio::test]
    async fn world_dump_survives_a_failed_buffer_read() {
        let world = Arc::new(FactorioSurface::new());
        let root = tempfile::tempdir().expect("a scripts root");
        let (refresher, calls) = counting_refresher(Err("rcon went away".to_string()));
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = dumping_world(&lua, &world, root.path(), Some(refresher));
        lua.globals().set("world", table).expect("set global");

        lua.load("world.dump('world.json')")
            .exec_async()
            .await
            .expect("a failed buffer read is not a failed dump");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the read was not attempted"
        );
        assert!(
            root.path().join("world.json").exists(),
            "no file was written"
        );
    }

    /// The same boundary `world.draw` and `globals.file_write` keep.
    ///
    /// The script-execution endpoint is unauthenticated, so this is what
    /// stands between an HTTP caller and the host filesystem; a binding that
    /// writes a whole world is a bigger lever than most.
    ///
    /// The refusal is also asserted to cost **no** RCON round trip: an
    /// unauthenticated caller must not be able to make the host talk to the
    /// game by naming a path it was never going to be allowed to write.
    #[tokio::test]
    async fn world_dump_refuses_to_leave_the_scripts_directory() {
        let world = Arc::new(FactorioSurface::new());
        let root = tempfile::tempdir().expect("a scripts root");
        let (refresher, calls) = counting_refresher(Ok(3));
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = dumping_world(&lua, &world, root.path(), Some(refresher));
        lua.globals().set("world", table).expect("set global");

        let escaped = lua.load("world.dump('../escaped.json')").exec_async().await;
        assert!(escaped.is_err(), "a path leaving the root was accepted");
        assert!(
            !root
                .path()
                .parent()
                .expect("a parent")
                .join("escaped.json")
                .exists(),
            "the refusal still wrote the file"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a refused path still made the host query the game"
        );
    }
}
