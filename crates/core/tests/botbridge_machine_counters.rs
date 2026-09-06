//! **Per-machine lifetime production, as the mod computes it.**
//!
//! The owner's requirement is one number per machine: how many items it made,
//! ever. Two entirely different mechanisms answer it, and this file pins both
//! against the real `mods/BotBridge/control.lua` rather than against a
//! description of it.
//!
//! * A **crafting machine** (furnace, assembling machine) is counted by the
//!   game: `LuaEntity.products_finished`. That counter is *crafts*, not items,
//!   so the mod multiplies each new craft by the recipe's yield -- a
//!   `copper-cable` assembler that finished 3 crafts made 6 cables, and a
//!   record that summed `products_finished` against `production.made` would be
//!   short by half and look like lost output.
//! * A **mining drill** is counted by nobody. Factorio 2.1.17 declares three
//!   `MiningDrill` members on `LuaEntity` (`mining_area`,
//!   `mining_drill_filter_mode`, `mining_target`), has no `mining_progress`
//!   attribute at all, no drill inventory define and no drill-mined event. The
//!   mod accumulates the fall in the `amount` of **every resource tile in the
//!   drill's `mining_area`**, every tick, and credits a depleted tile's
//!   remainder from `on_resource_depleted`. Watching only the tile the drill
//!   *names* is not enough: it mines a tile and moves that pointer in the same
//!   tick, which measured 130 against 133 actually mined.
//!
//! Every row therefore says **how** its number was obtained -- `game`,
//! `accumulated`, `unavailable` (a producer whose count cannot be had, i.e. a
//! pumpjack on infinite crude oil) or `not-a-producer` (a lab, a boiler, a
//! steam engine, a chest). An inferred number that reads as exact is the
//! failure mode this field exists to prevent.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game with a machine set the test declares, and a
/// `find_entities_filtered` that honours the `type` filter -- which it must,
/// because the mod asks it two different questions: the machine sampler asks
/// for a list of types on the 300-tick beat, and the drill accumulator asks
/// for `mining-drill` alone on every tick.
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
    _nth_tick = {}
    _events = {}
    script = setmetatable(
        { on_nth_tick = function(n, f) _nth_tick[n] = f end,
          on_event = function(id, f) _events[#_events + 1] = f end },
        { __index = function() return noop end })
    remote = nooptable()
    commands = nooptable()
    require = function() return {} end

    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end
    rcon = { print = noop }

    local function json(v)
        local t = type(v)
        if t == "number" then
            if v == math.floor(v) then return string.format("%d", v) end
            return string.format("%.6f", v)
        elseif t == "string" then
            return '"' .. v .. '"'
        elseif t == "boolean" then
            return tostring(v)
        elseif t == "nil" then
            return "null"
        end
        local parts = {}
        if #v > 0 then
            for _, item in ipairs(v) do parts[#parts + 1] = json(item) end
            return "[" .. table.concat(parts, ",") .. "]"
        end
        for k, item in pairs(v) do
            parts[#parts + 1] = '"' .. tostring(k) .. '":' .. json(item)
        end
        return "{" .. table.concat(parts, ",") .. "}"
    end

    _written = {}
    helpers = {
        table_to_json = json,
        write_file = function(path, data, append)
            _written[#_written + 1] = { path = path, data = data }
        end,
        remove_path = noop,
    }

    storage = {}
    _players = {}
    _machines = {}
    -- Resource tiles, answered for `type = "resource"` queries only: the mod
    -- asks for those when it primes a drill's baseline from its mining area.
    _resources = {}

    local function matches(filter, entity)
        if filter == nil then return true end
        if type(filter) == "string" then return entity.type == filter end
        for _, t in pairs(filter) do
            if entity.type == t then return true end
        end
        return false
    end

    -- **Named, because every real surface is.** `machine_key`'s fallback and
    -- `resource_key` are surface-qualified (`surface.name .. "|" .. name@x,y`),
    -- so a stub without a `name` models a world that cannot exist and errors
    -- the moment the mod reads it. The stub carries the field rather than the
    -- mod tolerating its absence: a nil surface in a live game means something
    -- is badly wrong, and a fallback to the unqualified key would silently
    -- restore the cross-surface collision these keys exist to prevent.
    _surface = {
        index = 1,
        name = "nauvis",
        find_entities_filtered = function(args)
            local out = {}
            if args.type == "resource" then
                for _, r in pairs(_resources) do
                    if r.valid then out[#out + 1] = r end
                end
                return out
            end
            for _, entity in pairs(_machines) do
                if entity.valid and matches(args.type, entity) then
                    out[#out + 1] = entity
                end
            end
            return out
        end,
        find_entity = function() return nil end,
        find_non_colliding_position = function(name, center) return center end,
    }

    -- One machine. `fields` overrides anything below it.
    function make_machine(unit, kind, name, fields)
        local entity = {
            unit_number = unit,
            type = kind,
            name = name,
            valid = true,
            position = { x = unit, y = 0 },
            surface = _surface,
            status = nil,
            electric_network_id = nil,
            get_inventory = function() return nil end,
            get_output_inventory = function() return nil end,
            get_fuel_inventory = function() return nil end,
        }
        for k, v in pairs(fields or {}) do entity[k] = v end
        _machines[#_machines + 1] = entity
        return entity
    end

    -- A recipe as the mod reads it: `products` plus a prototype that may name
    -- a main product. `yield` is items per completed craft.
    function make_recipe(name, yield)
        return {
            name = name,
            products = { { name = name, amount = yield } },
            prototype = { main_product = { name = name, amount = yield } },
        }
    end

    -- A resource tile a drill can be pointed at.
    --
    -- **No `unit_number`, deliberately.** A live 2.1.17 server answers nil for
    -- `mining_target.unit_number` while `.amount` reads fine, and the first
    -- version of the accumulator keyed on it and credited nothing at all --
    -- 0 reported for a drill that had just mined 133 iron ore. The fixture
    -- that hid it was this one, with a unit number the game does not give.
    -- A resource is identified by its position, and so is this.
    --
    -- **It carries a `surface`, because a resource entity has one.**
    -- `resource_key` is `surface.name .. "|" .. name@x,y`: iron ore at (10, 10)
    -- on Nauvis and iron ore at (10, 10) on Vulcanus are different tiles, and
    -- the unqualified key called them the same one.
    function make_resource(x, y, name, amount, infinite)
        local r = {
            name = name,
            type = "resource",
            valid = true,
            surface = _surface,
            position = { x = x, y = y },
            amount = amount,
            prototype = { infinite_resource = infinite == true },
        }
        _resources[#_resources + 1] = r
        return r
    end

    _force = {
        name = "player",
        current_research = nil,
        research_progress = 0,
        technologies = {},
        get_item_production_statistics = function()
            return { input_counts = {}, output_counts = {} }
        end,
    }

    game = {
        tick = 0,
        players = _players,
        connected_players = {},
        forces = { player = _force },
        surfaces = { _surface },
        take_screenshot = noop,
    }
    prototypes = { item = {}, entity = {} }
"#;

const STUB_TICK_EXTRAS: &str = r#"
    writeout_initial_stuff = function() end
    writeout_recipes = function() end
    writeout_forces = function() end
    poll_character_bots = function() end
"#;

fn lua_with_mod() -> Lua {
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
        .expect("stub");
    lua.load(TYPES_LUA)
        .set_name("types.lua")
        .exec()
        .expect("mod types.lua");
    lua.load(CONTROL_LUA)
        .set_name("control.lua")
        .exec()
        .expect("mod control.lua");
    lua.load(STUB_TICK_EXTRAS)
        .set_name("after")
        .exec()
        .expect("post-load overrides");
    lua.load("rcon_sampling_start('test-run')")
        .set_name("start")
        .exec()
        .expect("sampling starts");
    lua
}

fn exec(lua: &Lua, chunk: &str) {
    lua.load(chunk).set_name("step").exec().expect("chunk");
}

/// `n` ticks of `on_tick`, which is the only beat the drill accumulator has.
fn ticks(lua: &Lua, from: u64, count: u64) {
    for tick in from..from + count {
        exec(
            lua,
            &format!("game.tick = {tick}; on_tick({{ tick = {tick} }})"),
        );
    }
}

/// The machine map of the row the 300-tick beat wrote, as raw JSON text.
fn machine_sample(lua: &Lua, tick: u64) -> String {
    exec(
        lua,
        &format!("_written = {{}}; game.tick = {tick}; on_sample_force_tick({{ tick = {tick} }})"),
    );
    let written: Table = lua.globals().get("_written").expect("_written");
    written
        .sequence_values::<Table>()
        .map(|e| e.expect("entry").get::<String>("data").expect("data"))
        .find(|data| data.contains("\"kind\":\"machines\""))
        .expect("a machines sample was written")
}

/// One machine's row, as the `key: value` fragments a test can look for.
/// Parsing JSON properly here would test the stub's serialiser, not the mod.
fn row(sample: &str, unit: &str) -> String {
    let key = format!("\"{unit}\":{{");
    let start = sample.find(&key).unwrap_or_else(|| {
        panic!("machine {unit} is missing from the sample: {sample}");
    }) + key.len();
    let mut depth = 1usize;
    let mut end = start;
    for (offset, ch) in sample[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    sample[start..end].to_string()
}

/// A furnace's count comes from the game, and one craft of a 1:1 recipe is one
/// item -- the case the archived runs already confirm, where 27 stone furnaces
/// summed to exactly the 859 plates the force's own statistics reported.
#[test]
fn furnace_counter_comes_from_the_game() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"make_machine(11, "furnace", "stone-furnace", {
            products_finished = 42,
            is_crafting = function() return true end,
            crafting_progress = 0.25,
            get_recipe = function() return make_recipe("iron-plate", 1) end,
        })"#,
    );
    let sample = machine_sample(&lua, 300);
    let furnace = row(&sample, "11");
    assert!(
        furnace.contains("\"produced\":42"),
        "furnace should report 42 items: {furnace}"
    );
    assert!(
        furnace.contains("\"produced_source\":\"game\""),
        "and say the game supplied it: {furnace}"
    );
    assert!(
        furnace.contains("\"products_finished\":42"),
        "the raw craft counter stays: {furnace}"
    );
}

/// **Crafts are not items.** A `copper-cable` craft yields two, so an
/// assembler at 3 crafts has produced 6 -- and both numbers are in the row, so
/// nobody has to know which one they are holding.
#[test]
fn a_craft_of_two_counts_as_two_items() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_asm = make_machine(21, "assembling-machine", "assembling-machine-1", {
            products_finished = 0,
            is_crafting = function() return true end,
            crafting_progress = 0.5,
            get_recipe = function() return make_recipe("copper-cable", 2) end,
        })"#,
    );
    exec(&lua, "_asm.products_finished = 3");
    let first = row(&machine_sample(&lua, 300), "21");
    assert!(
        first.contains("\"produced\":6") && first.contains("\"products_finished\":3"),
        "3 crafts of copper-cable are 6 items: {first}"
    );
    exec(&lua, "_asm.products_finished = 5");
    let second = row(&machine_sample(&lua, 600), "21");
    assert!(
        second.contains("\"produced\":10"),
        "and the count accumulates across beats: {second}"
    );
}

/// The drill accumulator: the resource's `amount` falls, and that fall is the
/// drill's output. Nothing in the game counts this, so the mod does.
#[test]
fn drill_counter_accumulates_from_the_resource() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_ore = make_resource(-42.5, -37.5, "iron-ore", 500, false)
        make_machine(31, "mining-drill", "burner-mining-drill", {
            mining_target = _ore,
            mining_area = { { -43, -38 }, { -41, -36 } },
        })"#,
    );
    ticks(&lua, 1, 2);
    let idle = row(&machine_sample(&lua, 300), "31");
    assert!(
        idle.contains("\"produced\":0") && idle.contains("\"produced_source\":\"accumulated\""),
        "a drill that has mined nothing reports zero, accumulated: {idle}"
    );
    exec(&lua, "_ore.amount = 497");
    ticks(&lua, 3, 1);
    exec(&lua, "_ore.amount = 495");
    ticks(&lua, 4, 1);
    let mined = row(&machine_sample(&lua, 600), "31");
    assert!(
        mined.contains("\"produced\":5"),
        "two falls of 3 and 2 are five items: {mined}"
    );
    assert!(
        mined.contains("\"mining\":\"iron-ore\""),
        "and the row still says what it is mining, which is what names the item: {mined}"
    );
}

/// **A drill works several tiles and switches between them**, and the pointer
/// moves in the same tick as the mine. Two live measurements are behind this
/// case: comparing to the previous *target* read 120 of 133, and reading only
/// the current target read 130 of 133 -- one item per tile, stranded on tiles
/// the drill had turned away from. Watching every tile in the area, which is
/// what the mod does, strands nothing.
#[test]
fn switching_between_tiles_loses_nothing() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_a = make_resource(-42.5, -37.5, "iron-ore", 100, false)
        _b = make_resource(-41.5, -37.5, "iron-ore", 200, false)
        _drill = make_machine(31, "mining-drill", "burner-mining-drill", {
            mining_target = _a,
            mining_area = { { -43, -38 }, { -41, -36 } },
        })"#,
    );
    ticks(&lua, 1, 1);
    // Each step mines one item and moves the pointer in the same breath, which
    // is the live behaviour: the tile that just lost an item is no longer the
    // one the drill names.
    for (tick, chunk) in [
        (2, "_drill.mining_target = _b"),
        (3, "_b.amount = 199; _drill.mining_target = _a"),
        (4, "_a.amount = 99; _drill.mining_target = _b"),
        (5, "_b.amount = 198; _drill.mining_target = _a"),
        (6, "_a.amount = 98"),
    ] {
        exec(&lua, chunk);
        ticks(&lua, tick, 1);
    }
    let drill = row(&machine_sample(&lua, 300), "31");
    assert!(
        drill.contains("\"produced\":4"),
        "two items off each tile is four, however often the pointer moved: {drill}"
    );
}

/// **A tile first seen while the drill is already mining it has lost its first
/// item.** `mining_target` names the tile the drill just took an item from, so
/// that tile's first reading is post-mine and becomes its own baseline. Live
/// this read 130 against 133. The mod primes every tile in the drill's
/// `mining_area` when it starts tracking the drill, so a tile the drill turns
/// to later already has a baseline that predates any mining.
#[test]
fn a_drills_area_is_primed_so_a_new_tile_loses_nothing() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_a = make_resource(-42.5, -37.5, "iron-ore", 100, false)
        _b = make_resource(-41.5, -37.5, "iron-ore", 200, false)
        _drill = make_machine(31, "mining-drill", "burner-mining-drill", {
            mining_target = _a,
            mining_area = { { -43, -38 }, { -41, -36 } },
        })"#,
    );
    ticks(&lua, 1, 1);
    // The drill turns to B having already taken one item from it -- the
    // reading and the mine happen in the same tick, which is the live case.
    exec(&lua, "_b.amount = 199; _drill.mining_target = _b");
    ticks(&lua, 2, 1);
    let drill = row(&machine_sample(&lua, 300), "31");
    assert!(
        drill.contains("\"produced\":1"),
        "the primed baseline of 200 catches the item taken before the first reading: {drill}"
    );
}

/// **The last item of a depleted tile is invisible to the accumulator** -- the
/// mine that takes a tile from 1 to 0 destroys it, and no `amount` can be read
/// afterwards. `on_resource_depleted` credits it, which is the difference
/// between an exact count and one that is systematically one short per tile.
#[test]
fn a_depleted_tile_credits_its_last_item() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_ore = make_resource(-42.5, -37.5, "iron-ore", 2, false)
        make_machine(31, "mining-drill", "burner-mining-drill", {
            mining_target = _ore,
            mining_area = { { -43, -38 }, { -41, -36 } },
        })"#,
    );
    ticks(&lua, 1, 1);
    exec(&lua, "_ore.amount = 1");
    ticks(&lua, 2, 1);
    exec(
        &lua,
        "_ore.amount = 0; on_resource_depleted({ entity = _ore })",
    );
    let row = row(&machine_sample(&lua, 300), "31");
    assert!(
        row.contains("\"produced\":2"),
        "one seen fall plus the depleting mine is both items: {row}"
    );
}

/// **Two drills on one tile both see the same fall.** Their numbers are then
/// upper bounds and their sum double-counts -- so the row says so rather than
/// reading as exact.
#[test]
fn a_shared_tile_is_flagged_not_hidden() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_ore = make_resource(-42.5, -37.5, "iron-ore", 500, false)
        make_machine(31, "mining-drill", "electric-mining-drill",
            { mining_target = _ore, mining_area = { { -43, -38 }, { -41, -36 } }, })
        make_machine(32, "mining-drill", "electric-mining-drill",
            { mining_target = _ore, mining_area = { { -43, -38 }, { -41, -36 } }, })"#,
    );
    ticks(&lua, 1, 1);
    exec(&lua, "_ore.amount = 498");
    ticks(&lua, 2, 1);
    let sample = machine_sample(&lua, 300);
    for unit in ["31", "32"] {
        let drill = row(&sample, unit);
        assert!(
            drill.contains("\"produced\":2"),
            "each drill credits the whole fall: {drill}"
        );
        assert!(
            drill.contains("\"produced_shared\":true"),
            "and says the credit is shared with another drill: {drill}"
        );
    }
}

/// An infinite resource never falls below its minimum yield, so a pumpjack's
/// output cannot be counted this way at all. Reporting `0` would read as
/// "produced nothing"; `unavailable` is the honest answer.
#[test]
fn an_infinite_resource_reports_unavailable() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"_oil = make_resource(-42.5, -37.5, "crude-oil", 30000, true)
        make_machine(41, "mining-drill", "pumpjack",
            { mining_target = _oil, mining_area = { { -43, -38 }, { -41, -36 } }, })"#,
    );
    ticks(&lua, 1, 2);
    let drill = row(&machine_sample(&lua, 300), "41");
    assert!(
        drill.contains("\"produced_source\":\"unavailable\""),
        "an infinite target cannot be counted: {drill}"
    );
    assert!(
        !drill.contains("\"produced\":"),
        "and no count is invented for it: {drill}"
    );
}

/// A lab, a boiler, a steam engine and a chest make no item, ever. The row
/// says that in a field: an absent key cannot tell "makes nothing" from "the
/// counter is missing", and the whole point of `produced_source` is that
/// nobody reads an inferred number as an exact one.
#[test]
fn non_producers_say_so_rather_than_omitting_the_field() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"make_machine(51, "lab", "lab", {})
        make_machine(52, "boiler", "boiler", {})
        make_machine(53, "generator", "steam-engine", {})
        make_machine(54, "container", "wooden-chest", {})"#,
    );
    let sample = machine_sample(&lua, 300);
    for unit in ["51", "52", "53", "54"] {
        let machine = row(&sample, unit);
        assert!(
            machine.contains("\"produced_source\":\"not-a-producer\""),
            "machine {unit} should declare that it makes no items: {machine}"
        );
        assert!(
            !machine.contains("\"produced\":"),
            "and carry no count: {machine}"
        );
    }
}

/// Nothing accumulates outside a recording session. The drill beat returns on
/// its first line when `storage.sampling` is nil, which is also what keeps it
/// off the tick budget of a run that is not being recorded.
#[test]
fn no_session_no_accumulation() {
    let lua = lua_with_mod();
    exec(
        &lua,
        r#"rcon_sampling_stop()
        _ore = make_resource(-42.5, -37.5, "iron-ore", 500, false)
        make_machine(31, "mining-drill", "burner-mining-drill",
            { mining_target = _ore, mining_area = { { -43, -38 }, { -41, -36 } }, })"#,
    );
    ticks(&lua, 1, 1);
    exec(&lua, "_ore.amount = 400");
    ticks(&lua, 2, 1);
    exec(&lua, "rcon_sampling_start('test-run')");
    let drill = row(&machine_sample(&lua, 300), "31");
    assert!(
        drill.contains("\"produced\":0"),
        "100 ore mined while nobody recorded is not this run's output: {drill}"
    );
}
