//! **Identity and geometry in bulk, contents on demand, belts never.**
//!
//! `writeout_entities` ships every entity of every chunk as JSON through a
//! line-oriented text protocol on stdout, once per chunk, for the whole map.
//! Measured on a real run (`workspace/server-log.txt`, seed 31337, 1,424
//! chunks): **50,256 entity records, 10.5 MB** -- 40% of the entire server log,
//! and the same again for `tiles`. On the empty ground this project has always
//! measured on that is affordable. On a finished factory it is the world state,
//! item by item, and it is the reason a 6:39:53 world-record save could not be
//! ingested.
//!
//! Three rules, each with a different kind of evidence behind it.
//!
//! ## 1. Do not send what the ingest throws away
//!
//! `EntityGraph::add` (`crates/core/src/graph/entity_graph.rs`) opens its loop
//! with a `continue` for `flying-text`, `fish`, and any entity whose
//! `bounding_box.width()` is zero. Those reach no quad tree, no `minables`, no
//! `threats`, no petgraph node -- nothing at all. **Measured share of the wire:
//! 1,574 records, 2.59% of the bytes**, almost all fish; a finished base adds
//! every remnant, corpse and particle source to the same category.
//!
//! **This is the only "should not be sent" category that is a fact.** Trees are
//! 68.1% of that payload and rocks and cliffs another 3.1%, and they all stay:
//! `blocked_tree` is built from them and `PlanState::walkable_obstacles_within`
//! and `occupant_of` read it, so dropping them would break block siting and
//! belt routing. A smaller payload that breaks siting is a worse outcome than a
//! slow one.
//!
//! ## 2. Contents on demand
//!
//! `serialize_entity` attaches `output_inventory` and `fuel_inventory` to every
//! record. **Nothing reads them off a bulk-ingested entity**: `add` clones the
//! entity into `entity_tree`, but refuses to re-add over an occupied position,
//! so what is stored is a snapshot from the moment the chunk was generated and
//! is permanently stale. The planner's buffer model reads
//! `FactorioSurface::inventories`, filled only by `observe_inventories` from
//! the RCON reply to `inventory_contents_at` -- the on-demand path, which
//! already exists.
//!
//! On our maps this costs **exactly zero bytes across 50,256 records**, because
//! a map of trees and ore has no machine to own an inventory. That is why the
//! change cannot move the offline planning baselines, and it is also why the
//! byte measurement cannot size the benefit: **this rule is justified by the
//! consumer census, not by a number from a fresh map.**
//!
//! The RCON queries keep the full record. Scripts do read `output_inventory`
//! off `find_entities_in_radius` (`scripts/furnace_run.lua`,
//! `two_row_smelter_live.lua`), and that is the right way to ask.
//!
//! ## 3. Belts carry no contents, and that is a DELIBERATE NON-GOAL
//!
//! A belt goes out as name, position, direction and collision box. It has never
//! carried a transport line and it must not start. A yellow belt holds 8 items
//! per tile and a base has thousands of belt tiles, so per-tile item positions
//! are the single most expensive thing that could be added here and the least
//! useful: the planner reasons about connectivity, not about which item is on
//! which lane.
//!
//! The owner's shape for the day flows *are* needed:
//!
//! > "Same for conveyor belts -- we probably only need the direction, and for a
//! >  whole chain maybe what types of items are on the belts. Having all items
//! >  individually would be expensive on a large base."
//!
//! Direction and connectivity per belt; item *types* per chain; never per-tile
//! positions. `a_belt_is_direction_and_geometry_and_nothing_else` below fails
//! if anybody adds one.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game whose surface returns whatever `_entities` holds, and whose
/// `helpers.table_to_json` records the table it was handed instead of encoding
/// it -- so the assertions read the records themselves rather than parsing a
/// string.
const PRELUDE: &str = r#"
    local function auto()
        local t = {}
        setmetatable(t, { __index = function(tbl, k)
            local v = auto(); rawset(tbl, k, v); return v
        end })
        return t
    end
    defines = auto()
    function noop() end
    local function nooptable()
        return setmetatable({}, { __index = function() return noop end })
    end
    script = nooptable()
    remote = { interfaces = {}, call = noop, add_interface = noop }
    commands = nooptable()
    require = function() return {} end

    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end
    rcon = { print = noop }

    _encoded = nil
    helpers = {
        table_to_json = function(t) _encoded = t; return "<encoded>" end,
        write_file = noop,
        remove_path = noop,
    }

    _entities = {}
    _surface = {
        index = 1,
        name = "nauvis",
        find_entity = function() return nil end,
        find_entities = function() return _entities end,
        find_entities_filtered = function() return {} end,
        get_chunks = function() return function() return nil end end,
    }
    storage = { n_clients = 1, p = {}, map_area = { x1 = 0, y1 = 0, x2 = 0, y2 = 0 } }
    game = { tick = 0, players = {}, connected_players = {}, forces = {},
             surfaces = { _surface, nauvis = _surface }, take_screenshot = noop }
    prototypes = { item = {}, entity = {} }

    -- A box of the given half-width, centred on (x, y). `half = 0` is the
    -- zero-width box `EntityGraph::add` discards.
    function box(x, y, half)
        return {
            left_top = { x = x - half, y = y - half },
            right_bottom = { x = x + half, y = y + half },
        }
    end

    -- `contents` non-nil makes the entity own both inventories, the way a
    -- furnace or a chest does. A belt, a tree and a fish own neither: the real
    -- `get_output_inventory()` returns nil for them, and so does this.
    function entity(name, etype, x, y, half, contents)
        return {
            name = name,
            type = etype,
            direction = 0,
            position = { x = x, y = y },
            bounding_box = box(x, y, half),
            surface = _surface,
            get_output_inventory = function()
                if contents == nil then return nil end
                return { get_contents = function() return contents end }
            end,
            get_fuel_inventory = function()
                if contents == nil then return nil end
                return { get_contents = function() return { coal = 3 } end }
            end,
            -- The INPUT inventory, which has no getter of its own:
            -- `serialize_entity` resolves an index out of `defines.inventory`
            -- and calls `get_inventory(index)`. Owned by whatever owns the
            -- other two, on the same terms.
            get_inventory = function()
                if contents == nil then return nil end
                return { get_contents = function() return { ["iron-ore"] = 7 } end }
            end,
        }
    end
"#;

const AREA: &str = "{ left_top = { x = 0, y = 0 }, right_bottom = { x = 32, y = 32 } }";

fn fresh_mod() -> Lua {
    #[allow(
        clippy::disallowed_methods,
        reason = "test-only interpreter for the repo's own mod source; the \
                  sandbox lives in a crate that depends on this one"
    )]
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .expect("test interpreter");
    lua.load(PRELUDE)
        .set_name("stub_game")
        .exec()
        .expect("stub game");
    lua.load(TYPES_LUA)
        .set_name("types.lua")
        .exec()
        .expect("mod types.lua");
    lua.load(CONTROL_LUA)
        .set_name("control.lua")
        .exec()
        .expect("mod control.lua");
    lua
}

/// Populate `_entities`, run one chunk's writeout, and hand back the records
/// that went on the wire.
fn written(lua: &Lua, entities: &str) -> Vec<Table> {
    lua.load(format!("_entities = {{ {entities} }}"))
        .set_name("entities")
        .exec()
        .expect("entities");
    lua.load(format!("writeout_entities(0, _surface, {AREA})"))
        .set_name("writeout")
        .exec()
        .expect("writeout_entities");
    lua.globals()
        .get::<Table>("_encoded")
        .expect("_encoded -- writeout_entities must have called table_to_json")
        .sequence_values::<Table>()
        .map(|v| v.expect("record"))
        .collect()
}

fn names(records: &[Table]) -> Vec<String> {
    records
        .iter()
        .map(|r| r.get::<String>("name").expect("name"))
        .collect()
}

/// **What the ingest discards is never sent, and what it uses always is.**
///
/// The tree is the control and it is the important half: trees are 68.1% of
/// the measured payload and `blocked_tree` is built from them, so a filter that
/// swept them up with the fish would shrink the wire and break block siting.
#[test]
fn the_bulk_writeout_sends_exactly_what_the_ingest_can_use() {
    let lua = fresh_mod();
    let records = written(
        &lua,
        r#"
        entity("tree-03", "tree", 4, 4, 0.4),
        entity("fish", "fish", 5, 5, 0.4),
        entity("flying-text", "flying-text", 6, 6, 0.4),
        entity("explosion", "explosion", 7, 7, 0),
        entity("rock-big", "simple-entity", 8, 8, 0.7),
        entity("cliff", "cliff", 9, 9, 2),
        "#,
    );

    assert_eq!(
        names(&records),
        vec!["tree-03", "rock-big", "cliff"],
        "fish, flying-text and a zero-width box are thrown away by \
         `EntityGraph::add` the instant they arrive, so sending them is pure \
         cost -- while trees, rocks and cliffs all land in `blocked_tree` and \
         the planner sites blocks against them"
    );
}

/// **A machine's identity travels; its contents do not.**
#[test]
fn a_machine_is_sent_without_its_inventories() {
    let lua = fresh_mod();
    let records = written(
        &lua,
        r#"entity("stone-furnace", "furnace", 4, 4, 1, { ["iron-plate"] = 42 })"#,
    );

    assert_eq!(names(&records), vec!["stone-furnace"]);
    let furnace = &records[0];
    assert!(
        furnace
            .get::<Option<Table>>("output_inventory")
            .expect("get")
            .is_none(),
        "42 iron plates have no business on the wire: nothing reads a \
         bulk-ingested entity's inventory, and `inventory_contents_at` is how \
         a caller asks about the few it cares about"
    );
    assert!(
        furnace
            .get::<Option<Table>>("fuel_inventory")
            .expect("get")
            .is_none(),
        "the fuel slot goes the same way, and for the same reason"
    );
    assert!(
        furnace
            .get::<Option<Table>>("input_inventory")
            .expect("get")
            .is_none(),
        "and so does the ore it is holding: `omit_inventories` covers all \
         three, or the bulk path grows the whole world's item positions again"
    );
    assert!(
        furnace
            .get::<Option<Table>>("bounding_box")
            .expect("get")
            .is_some(),
        "geometry stays -- this is a contents change, not a geometry one, and \
         `blocked_tree` is built from exactly this box"
    );
    assert_eq!(
        furnace.get::<String>("entity_type").expect("entity_type"),
        "furnace",
        "and so does identity"
    );
}

/// **The query path is untouched.** Scripts read `output_inventory` off
/// `rcon.find_entities_in_radius` to count plates; that is the supported way to
/// ask, and a change aimed at the bulk path must not break it.
#[test]
fn an_rcon_query_still_answers_with_the_contents() {
    let lua = fresh_mod();
    lua.load(
        r#"
        _one = serialize_entity(entity("stone-furnace", "furnace", 4, 4, 1,
                                       { ["iron-plate"] = 42 }))
        "#,
    )
    .set_name("query")
    .exec()
    .expect("serialize_entity");

    let record = lua.globals().get::<Table>("_one").expect("_one");
    assert_eq!(
        record
            .get::<Table>("output_inventory")
            .expect("output_inventory -- an RCON query must still carry it")
            .get::<u32>("iron-plate")
            .expect("iron-plate"),
        42,
        "`serialize_entity` called with no options is the query path, and it \
         is unchanged: only `writeout_entities` passes `omit_inventories`"
    );
    assert!(
        record
            .get::<Option<Table>>("fuel_inventory")
            .expect("get")
            .is_some(),
        "both halves, or a script counting fuel breaks instead"
    );
    assert_eq!(
        record
            .get::<Table>("input_inventory")
            .expect("input_inventory -- the query path carries it too")
            .get::<u32>("iron-ore")
            .expect("iron-ore"),
        7,
        "all three, now: what the furnace holds is exactly the reading that \
         separates `it has ore and is not smelting` from `no ore arrived`"
    );
}

/// **A belt carries no contents, and this test exists to keep it that way.**
///
/// A yellow belt holds 8 items per tile and a base has thousands of belt tiles.
/// Nothing in the serialiser has ever read a transport line and nothing should
/// start: when flows are needed the shape is direction and connectivity per
/// belt and item *types* per chain, never per-tile positions.
#[test]
fn a_belt_is_direction_and_geometry_and_nothing_else() {
    let lua = fresh_mod();
    let records = written(
        &lua,
        r#"entity("transport-belt", "transport-belt", 4, 4, 0.4)"#,
    );

    assert_eq!(names(&records), vec!["transport-belt"]);
    let belt = &records[0];
    let mut keys: Vec<String> = belt
        .clone()
        .pairs::<String, mlua::Value>()
        .map(|kv| kv.expect("pair").0)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "bounding_box".to_string(),
            "direction".to_string(),
            "entity_type".to_string(),
            "name".to_string(),
            "position".to_string(),
            "surface".to_string(),
        ],
        "a belt's whole record. If this list has grown a transport line, a \
         lane, an item list or a per-tile anything, that is the change this \
         test exists to refuse -- see the module docs for the shape flows \
         should take instead"
    );
}
