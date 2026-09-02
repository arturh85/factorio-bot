//! **Where a bot came to rest is never reported, so the planner sites on top
//! of it.**
//!
//! Factorio raises `on_player_changed_position` once per **tile** a character
//! crosses, not once per position change. That is invisible while a bot is
//! walking — the next tile is a few ticks away — and permanent once it stops:
//! the last event names the tile boundary it crossed, and the character then
//! carries on up to a tile further before halting, with nothing to say so.
//!
//! `workspace/runs/run-1788353986-24634` (run 27) is what that costs. Its
//! `workspace/server-log.txt` has bot 3's last position event at tick 18187:
//!
//! ```text
//! §18187§on_player_changed_position§{"player_id":3,"position":{"y":16.96484375,"x":-23.19140625}}
//! ```
//!
//! and `samples.jsonl` — written by the same mod, on the same server, from
//! `player.character.position` — has bot 3 at `(-23.5078125, 16.203125)` from
//! tick 18240 to the end of the run, motionless. Both points are inside tile
//! `(-24, 16)`, so no further event was ever raised. The Rust world was
//! **0.825 tiles wrong about a parked bot for 13 000 ticks**.
//!
//! `crates/planner`'s `characters` occupancy source reads exactly that
//! position. A stone furnace is 1.398 tiles across and a character 0.398, so
//! the two boxes clear each other at the believed position and overlap at the
//! real one: the planner sited a stone furnace at `[-23, 16]`, the game refused
//! it, and the milestone re-planned onto the same site until it gave up.
//! `crates/planner/tests/placement_occupancy.rs` pins that arithmetic from the
//! other side.
//!
//! The fix reports the character's own position at every point the mod's walker
//! lets a character stop — which is the moment, and the only moment, a resting
//! position becomes a fact. These tests load the real `control.lua` into a Lua
//! 5.4 state and drive `on_tick` against a stub that never moves the character
//! itself, so what is asserted is the mod's own choice of what to say and when.
//!
//! What this cannot prove: that Factorio really raises the event per tile
//! rather than per tick. That is read off run 27's own log, where consecutive
//! events for one walking bot are ~1.0 tiles and ~7 ticks apart.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick `on_tick` is driven at. Not a multiple of 120, which `on_tick`
/// uses for an unrelated heartbeat print.
const TICK: u64 = 501;

/// Bot 3's real resting position in run 27, from `samples.jsonl`.
const RESTED: (f64, f64) = (-23.5078125, 16.203125);

/// The waypoint the walk was aiming at. Deliberately **not** [`RESTED`]: the
/// mod's arrival test is a 0.3-by-0.3 box, so a walk finishes near its
/// waypoint rather than on it, and reporting the waypoint instead of the
/// character would be the same class of lie in a smaller size.
const WAYPOINT: (f64, f64) = (-23.5, 16.0);

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
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }

    -- Enough of a JSON writer for the two records these tests read back, with
    -- the numbers printed exactly rather than rounded -- a sub-tile error is
    -- the whole subject.
    local function num(v)
        if v == math.floor(v) then return string.format("%d", v) end
        return string.format("%.8f", v)
    end
    helpers = {
        table_to_json = function(t)
            if t.position ~= nil then
                return '{"player_id":' .. tostring(t.player_id)
                    .. ',"position":{"x":' .. num(t.position.x)
                    .. ',"y":' .. num(t.position.y) .. '}}'
            end
            return '{"player_id":' .. tostring(t.player_id)
                .. ',"reason":"' .. tostring(t.reason) .. '"}'
        end,
        write_file = noop,
        remove_path = noop,
    }

    storage = {}
    _players = {}

    function make_player(idx, x, y)
        local p = {
            index = idx,
            name = "bot" .. idx,
            connected = true,
            character_running_speed = 0.15,
            walking_state = { walking = false },
            character = { position = { x = x, y = y } },
        }
        p.surface = {
            find_non_colliding_position = function(name, center, radius, precision)
                return { x = center.x, y = center.y }
            end,
            find_entity = function() return nil end,
            find_entities_filtered = function() return {} end,
        }
        p.teleport = function(pos)
            p.character.position = { x = pos.x, y = pos.y }
            return true
        end
        _players[idx] = p
        return p
    end

    game = { tick = 0, players = _players, forces = {}, surfaces = {} }
    prototypes = { item = {}, entity = {} }
"#;

const STUB_TICK_EXTRAS: &str = r#"
    writeout_initial_stuff = function() end
    writeout_recipes = function() end
    writeout_forces = function() end
    on_player_changed_distance = function(e) end
"#;

fn lua_for_mod_source() -> Lua {
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
    lua
}

fn run_ticks(setup: &str, ticks: u64) -> Lua {
    let lua = lua_for_mod_source();
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
    lua.load(STUB_TICK_EXTRAS)
        .set_name("after")
        .exec()
        .expect("post-load overrides");
    lua.load(setup)
        .set_name("setup")
        .exec()
        .expect("fixture setup");
    for tick in TICK..TICK + ticks {
        lua.load(format!("on_tick({{ tick = {tick} }})"))
            .set_name("on_tick")
            .exec()
            .expect("on_tick");
    }
    lua
}

fn stdout(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect()
}

/// The `on_player_changed_position` writeouts the mod produced, in order.
///
/// Matched on the wire form the Rust output reader parses
/// (`crates/core/src/process/output_reader.rs`), not on some looser shape:
/// a record that does not go out under that key reaches
/// `FactorioWorld::player_changed_position` never.
fn position_writeouts(lua: &Lua) -> Vec<String> {
    stdout(lua)
        .into_iter()
        .filter(|line| line.contains("§on_player_changed_position§"))
        .collect()
}

/// A walk with one waypoint the character is already standing within arrival
/// tolerance of, so the very next tick retires it.
fn arriving_walk(idx: u32, at: (f64, f64), action_id: u32) -> String {
    let (wx, wy) = WAYPOINT;
    format!(
        r#"
        storage.p = {{}}
        make_player({idx}, {x}, {y})
        storage.p[{idx}] = {{ walking = {{
            idx = 1,
            waypoints = {{ {{ x = {wx}, y = {wy} }} }},
            action_id = {action_id},
            idx_tick = {TICK},
            leg_timeout = 600,
        }} }}
    "#,
        x = at.0,
        y = at.1,
    )
}

/// **The defect.** A walk that finishes has to say where the character
/// actually stopped, because nothing else ever will.
#[test]
fn a_walk_that_arrives_reports_where_the_character_came_to_rest() {
    let lua = run_ticks(&arriving_walk(3, RESTED, 42), 1);
    let lines = position_writeouts(&lua);
    assert_eq!(
        lines.len(),
        1,
        "arriving is the moment a resting position becomes a fact, and the \
         game will not raise another event for a character that has stopped. \
         Got {lines:?}"
    );
    assert_eq!(
        lines[0],
        format!(
            "§{TICK}§on_player_changed_position§{{\"player_id\":3,\"position\":{{\"x\":{:.8},\"y\":{:.8}}}}}",
            RESTED.0, RESTED.1
        ),
        "the record must be the character's own position at the tick it \
         stopped. Bot 3's was {RESTED:?} in run 27 while the Rust world still \
         held (-23.19140625, 16.96484375), the tile boundary it had crossed \
         53 ticks earlier -- 0.825 tiles of silent error, which is what let a \
         1.398-tile furnace footprint be sited on top of it"
    );
}

/// The control that keeps the fix from becoming a per-tick flood: a walk still
/// under way says nothing. The game is already raising an event per tile for a
/// character that is moving, and a second source for the same fact would cost
/// a stdout line per bot per tick for no new information.
#[test]
fn a_walk_still_under_way_reports_nothing() {
    let far_from_the_waypoint = (-30.0, 16.0);
    let lua = run_ticks(&arriving_walk(3, far_from_the_waypoint, 42), 2);
    assert!(
        position_writeouts(&lua).is_empty(),
        "a moving character raises the game's own events; only a stop is \
         unobservable. Got {:?}",
        position_writeouts(&lua)
    );
}

/// A walk the mod gives up on stops the character exactly as an arrival does,
/// so it owes the same record. This is the arm run 27's bots did **not** take,
/// and leaving it out would fix the common case and keep the bug for the
/// interesting one.
#[test]
fn a_walk_that_is_abandoned_reports_where_the_character_stopped() {
    let (wx, wy) = WAYPOINT;
    let setup = format!(
        r#"
        storage.p = {{}}
        make_player(3, {x}, {y})
        storage.p[3] = {{ walking = {{
            idx = 1,
            waypoints = {{ {{ x = {wx}, y = {wy} }} }},
            action_id = 42,
            idx_tick = 0,
            leg_timeout = 1,
            stuck = "ERROR: stuck while walking, aborted before reaching last waypoint",
            }} }}
        storage.p[3].walking.waypoints[1] = nil
    "#,
        x = RESTED.0,
        y = RESTED.1,
    );
    let lua = run_ticks(&setup, 1);
    let lines = position_writeouts(&lua);
    assert_eq!(
        lines.len(),
        1,
        "a character the walker has given up on is standing still just as \
         firmly as one that arrived. Got {lines:?}"
    );
    assert!(
        lines[0].contains(&format!("\"x\":{:.8}", RESTED.0)),
        "and the record is still the character's own position. Got {:?}",
        lines[0]
    );
}
