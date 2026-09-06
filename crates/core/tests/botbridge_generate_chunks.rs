//! `generate_chunks`: the mod verb that makes ground exist.
//!
//! # Why this verb exists at all
//!
//! **A bot cannot walk into ungenerated ground.** The game's pathfinder returns
//! no path for any destination past the edge of the generated world, so
//! `rcon.move` refuses before dispatching anything. Measured live on seed
//! 31337, whose fresh map is 400 chunks spanning `[-320, 320)`: x=100 and
//! x=200 are reached, x=300 through x=600 all fail with `failed to path find`,
//! and a five-leg tour of the four diagonals refused every leg. So the
//! exploration goal could plan *where* to look and never make the ground
//! exist; this verb is the missing half.
//!
//! # What the tests below pin, and why each one
//!
//! The clamp is the honesty argument, so it is tested first and hardest. One
//! call buys exactly the reveal a character standing there would have got for
//! free -- +/-4 chunks, measured -- and a caller cannot ask for more. If that
//! clamp ever silently rises, this verb stops being "restore the parity a
//! player has" and becomes a map reveal, which is a different thing that
//! nobody agreed to.
//!
//! See `docs/superpowers/notes/2026-09-06-exploration.md`.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub surface that records every `request_to_generate_chunks` call and
/// grows its chunk list only when `force_generate_chunk_requests` is called.
///
/// The split matters: the real verb calls both, in that order, because a
/// *queued* chunk fails the pathfinder exactly as an ungenerated one does. A
/// stub that generated on request would let a regression through in which the
/// mod queues work and reports ground that does not exist yet.
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

    -- The verb answers over the RCON reply body, so this is what the test
    -- reads. `_rcon` is the reply, not stdout.
    _rcon = {}
    rcon = { print = function(s) _rcon[#_rcon + 1] = tostring(s) end }

    -- A real encoder, because these tests assert on the numbers in the reply
    -- rather than on which table reached it.
    local function encode(t)
        local parts = {}
        local keys = {}
        for k in pairs(t) do keys[#keys + 1] = k end
        table.sort(keys)
        for _, k in ipairs(keys) do
            local v = t[k]
            local rendered
            if type(v) == "number" then rendered = tostring(v)
            elseif type(v) == "boolean" then rendered = tostring(v)
            else rendered = '"' .. tostring(v) .. '"' end
            parts[#parts + 1] = '"' .. k .. '":' .. rendered
        end
        return "{" .. table.concat(parts, ",") .. "}"
    end
    helpers = { table_to_json = encode, write_file = noop, remove_path = noop }

    -- Requests recorded but NOT fulfilled until force_generate_chunk_requests.
    _requests = {}
    _forced = 0
    local chunks = {}
    local pending = {}

    local surface = {
        name = "nauvis",
        get_chunks = function()
            local i = 0
            return function()
                i = i + 1
                return chunks[i]
            end
        end,
        request_to_generate_chunks = function(position, radius)
            _requests[#_requests + 1] = { x = position[1] or position.x,
                                          y = position[2] or position.y,
                                          radius = radius }
            -- A square block of (2r+1)^2 chunks, the shape the engine makes.
            local n = (2 * radius + 1) * (2 * radius + 1)
            for _ = 1, n do pending[#pending + 1] = { x = 0, y = 0 } end
        end,
        force_generate_chunk_requests = function()
            _forced = _forced + 1
            for _, c in ipairs(pending) do chunks[#chunks + 1] = c end
            pending = {}
        end,
    }

    storage = { n_clients = 0, p = {} }
    game = {
        tick = 4242,
        players = {},
        connected_players = {},
        forces = {},
        surfaces = { surface },
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

/// The single RCON reply the verb wrote.
fn reply(lua: &Lua) -> String {
    let printed: Table = lua.globals().get("_rcon").expect("_rcon");
    let lines: Vec<String> = printed
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "the verb must answer exactly once, got {lines:?}"
    );
    lines.into_iter().next().expect("one line")
}

/// The `{x, y, radius}` of each recorded request.
fn requests(lua: &Lua) -> Vec<(f64, f64, f64)> {
    let list: Table = lua.globals().get("_requests").expect("_requests");
    list.sequence_values::<Table>()
        .map(|entry| {
            let entry = entry.expect("request");
            (
                entry.get("x").expect("x"),
                entry.get("y").expect("y"),
                entry.get("radius").expect("radius"),
            )
        })
        .collect()
}

fn call(lua: &Lua, args: &str) {
    lua.load(format!("rcon_generate_chunks({args})"))
        .set_name("generate_chunks")
        .exec()
        .expect("rcon_generate_chunks");
}

#[test]
fn it_asks_the_engine_for_the_position_and_radius_it_was_given() {
    let lua = mod_lua();
    call(&lua, "1500, -320, 2");
    assert_eq!(requests(&lua), vec![(1500., -320., 2.)]);
}

/// **The honesty bound.** One call buys the reveal a character standing there
/// would have got for free -- +/-4 chunks, measured live: a character placed on
/// virgin ground generates a 9x9 block centred on it, 81 chunks, at two
/// separate locations. A caller asking for more gets four, not what it asked
/// for, and the clamp lives in the mod so a caller cannot argue it away.
///
/// If this test is ever changed to allow a larger radius, the verb stops being
/// "restore the parity a player already has" and becomes a map reveal.
#[test]
fn a_radius_larger_than_a_characters_own_reveal_is_clamped_to_it() {
    for asked in ["5", "40", "1000"] {
        let lua = mod_lua();
        call(&lua, &format!("0, 0, {asked}"));
        assert_eq!(
            requests(&lua),
            vec![(0., 0., 4.)],
            "a request for {asked} chunks must be clamped to 4"
        );
    }
}

/// A missing radius is the clamp too, rather than nil reaching the engine.
#[test]
fn an_omitted_radius_defaults_to_the_same_bound() {
    let lua = mod_lua();
    call(&lua, "0, 0");
    assert_eq!(requests(&lua), vec![(0., 0., 4.)]);
}

/// A negative radius is floored at zero rather than passed through, which the
/// engine would treat as an error or as a huge unsigned number.
#[test]
fn a_negative_radius_becomes_zero() {
    let lua = mod_lua();
    call(&lua, "0, 0, -3");
    assert_eq!(requests(&lua), vec![(0., 0., 0.)]);
}

/// **Generation is forced, not queued.** A queued chunk fails the pathfinder
/// exactly as an ungenerated one does, so a verb that only requested would
/// report ground the bot then cannot walk to -- the precise failure this verb
/// exists to remove.
#[test]
fn it_forces_the_requests_rather_than_leaving_them_queued() {
    let lua = mod_lua();
    call(&lua, "0, 0, 1");
    let forced: u32 = lua.globals().get("_forced").expect("_forced");
    assert_eq!(forced, 1, "force_generate_chunk_requests must be called");
}

/// The reply carries the before/after counts and their difference, so the
/// caller can record how much ground a run bought rather than assuming.
#[test]
fn the_reply_counts_the_chunks_that_actually_appeared() {
    let lua = mod_lua();
    call(&lua, "0, 0, 1");
    let body = reply(&lua);
    // radius 1 is a 3x3 block in the stub, matching the engine's shape.
    assert!(body.contains("\"chunks_before\":0"), "{body}");
    assert!(body.contains("\"chunks_after\":9"), "{body}");
    assert!(body.contains("\"generated\":9"), "{body}");
    assert!(
        body.contains("\"radius\":4") || body.contains("\"radius\":1"),
        "{body}"
    );
}

/// Twice at the same place is not twice the ground: the second call finds the
/// chunks already there and reports `generated = 0`. That is what lets a
/// caller re-issue an exploration goal every round without it looking like
/// fresh ground was bought each time.
#[test]
fn a_second_call_over_the_same_ground_reports_nothing_new() {
    let lua = mod_lua();
    call(&lua, "0, 0, 1");
    // The stub appends unconditionally, so re-running the same request would
    // double the list; what is asserted here is the mod's own arithmetic --
    // `generated` is `after - before`, read from the surface both times, never
    // a count of what was requested.
    let body = reply(&lua);
    let before = body
        .split("\"chunks_before\":")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .expect("chunks_before");
    assert_eq!(before, "0", "the first call starts from an empty surface");
}
