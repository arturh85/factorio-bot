//! **`writeout_forces` emitted every force in `game.forces`, and the planner
//! then planned as `enemy`.**
//!
//! `crates/planner/src/state.rs` picks its acting force with
//! `forces.keys().min()`, which over `{enemy, neutral, player}` returns
//! `enemy`. In run 30 that happened at tick 26,449 -- the first
//! `on_research_finished`, which is where `writeout_forces` runs -- and covered
//! milestones 6 and 7 entirely. At tick 53,485 the `player` force had
//! `automation-science-pack` researched and `enemy` did not, so all five
//! milestone-7 plans re-derived the trigger technology and planned a **second
//! lab for a technology the force already had**. That milestone burned 85,030
//! ticks and ended the run `stuck`.
//!
//! **This is half the fix, and the other half is not here.** Naming the force
//! instead of sorting for it belongs in `crates/planner`, and it has to work
//! against a world that still reports three forces, because every archived run
//! record contains exactly that. The two changes are independent and neither
//! waits on the other.
//!
//! Checked before removing data somebody might depend on: **nothing reads the
//! non-player forces.** The only readers of `FactorioSurface::forces` are the
//! planner (which wants `player` and currently sorts for it) and
//! `crates/executor/src/rcon_actuator.rs`, which already names `"player"` and
//! documents this very emission as the reason it must not sort. And the RCON
//! transport already does what this change makes the stdout transport do:
//! `WorldSnapshot::forces` is documented as "exactly the force the bots act
//! for (`player`)", for exactly these reasons. The two transports are supposed
//! to agree about shape; this is one of them catching up.
//!
//! It also saves ~287 kB of stdout per research completion, which is not the
//! point but is not nothing.
//!
//! See `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The three forces a vanilla game has, in the order `pairs` is least likely
/// to hand them back, so nothing here can accidentally depend on iteration
/// order.
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
    remote = nooptable()
    commands = nooptable()
    require = function() return {} end

    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end
    rcon = { print = noop }

    -- Only the force's own name is needed to tell the records apart, and a
    -- real encoder here would be asserting on serialize_force rather than on
    -- which forces reach stdout.
    helpers = {
        table_to_json = function(t) return '{"name":"' .. tostring(t.name) .. '"}' end,
        write_file = noop,
        remove_path = noop,
    }

    local function make_force(name)
        return {
            name = name,
            index = 1,
            research_progress = 0,
            manual_mining_speed_modifier = 0,
            technologies = {},
            recipes = {},
        }
    end

    storage = { n_clients = 0, p = {} }
    game = {
        tick = 0,
        players = {},
        connected_players = {},
        forces = {
            enemy = make_force("enemy"),
            neutral = make_force("neutral"),
            player = make_force("player"),
        },
        surfaces = {},
        take_screenshot = noop,
    }
    prototypes = { item = {}, entity = {} }
"#;

fn mod_lua() -> Lua {
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

/// The `force` records the mod wrote to stdout, in order.
///
/// Matched on the wire form `crates/core/src/process/output_parser.rs:327`
/// reads (`§<tick>§force§<json>`): a record that does not go out under that
/// key never becomes a `FactorioForce`.
fn force_records(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .filter(|line| line.contains("§force§"))
        .collect()
}

/// **The defect.** Three forces reach the world; the planner sorts and gets
/// `enemy`.
#[test]
fn only_the_force_the_bots_act_for_is_written_out() {
    let lua = mod_lua();
    lua.load("writeout_forces()")
        .set_name("writeout")
        .exec()
        .expect("writeout_forces");

    let records = force_records(&lua);
    assert_eq!(
        records.len(),
        1,
        "game.forces holds enemy, neutral and player; emitting all three puts \
         them all in FactorioSurface::forces, where the planner's \
         `forces.keys().min()` returns `enemy`. Run 30 planned as enemy from \
         tick 26,449 and re-planned a lab for a technology `player` had \
         already researched. Got {records:?}"
    );
    assert!(
        records[0].contains(r#"{"name":"player"}"#),
        "and it must be `player` -- the force control.lua hardcodes in \
         collect_recipes, collect_player_force and start_research. Got {:?}",
        records[0]
    );
}

/// Reached through the handler that actually runs it in a live run. This is
/// the tick at which run 30's world gained `enemy`: the first research to
/// finish.
#[test]
fn a_finished_research_writes_out_one_force_not_three() {
    let lua = mod_lua();
    lua.load(
        r#"
        writeout_recipes = function() end
        on_player_changed_distance = function() end
        settle_research_actions = function() end
        on_research_finished({ tick = 26449, research = { name = "automation" } })
        "#,
    )
    .set_name("research finished")
    .exec()
    .expect("on_research_finished");

    let records = force_records(&lua);
    assert_eq!(records.len(), 1, "got {records:?}");
    assert!(records[0].contains(r#""name":"player""#), "got {records:?}");
}

/// And through the other call site, the one-off static dump at the start of a
/// run. Both have to agree, or the world holds three forces from startup and
/// the research path is beside the point.
#[test]
fn the_opening_static_dump_writes_out_one_force_not_three() {
    let lua = mod_lua();
    lua.load(
        r#"
        writeout_pictures = function() end
        writeout_entity_prototypes = function() end
        writeout_item_prototypes = function() end
        writeout_recipes = function() end
        writeout_initial_stuff()
        "#,
    )
    .set_name("initial")
    .exec()
    .expect("writeout_initial_stuff");

    let records = force_records(&lua);
    assert_eq!(records.len(), 1, "got {records:?}");
    assert!(records[0].contains(r#""name":"player""#), "got {records:?}");
}
