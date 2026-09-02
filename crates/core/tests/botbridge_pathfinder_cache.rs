//! **`PathfinderFlags.cache` defaults to true, and we never set it.**
//!
//! The shipped 2.1.17 docs say a cached path *"might fail to respond to changes
//! in the environment"*. We are the thing changing the environment: bots place
//! furnaces, drills and belts mid-run, on the tiles they are about to walk
//! across.
//!
//! Measured in run 30: **12 of 75 returned paths contain a waypoint strictly
//! inside a furnace we had placed**, every one of them flagged
//! `needs_destroy_to_reach: false` — the game telling us it believes the tile
//! is clear. A bot then walks into a building that is there, and the walk
//! stalls.
//!
//! **This is an experiment as much as a fix.** Stale caching is the leading
//! explanation for that ratio, not an established one. Setting `cache = false`
//! is what tests it: on the next run with `FACTORIO_BOT_REFRESH_MODS=1` the
//! 12/75 should go to **zero**. If it does not, the explanation is wrong and
//! the real cause is somewhere else — say so rather than quietly keeping the
//! flag because it seemed sensible. See
//! `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`.
//!
//! What this test can and cannot prove: it pins that the mod *asks* for an
//! uncached search, read off the argument table the mod hands
//! `LuaSurface::request_path`. Whether the game then returns a path free of
//! entities we placed is a property of a live run, and only a live run can
//! answer it.

use mlua::{Lua, LuaOptions, StdLib, Table, Value};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

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
    helpers = {
        table_to_json = function() return "{}" end,
        write_file = noop,
        remove_path = noop,
    }

    -- Every request the mod makes, verbatim, so the assertions read the table
    -- the game would have received rather than a summary of it.
    _requests = {}
    _surface = {
        index = 1,
        request_path = function(args)
            _requests[#_requests + 1] = args
            return 7
        end,
        find_entity = function() return nil end,
        find_entities_filtered = function() return {} end,
    }

    _players = {
        [1] = {
            index = 1,
            name = "bot1",
            connected = true,
            position = { x = 0.5, y = 0.5 },
            force = { name = "player" },
            surface = _surface,
            character = {
                position = { x = 0.5, y = 0.5 },
                prototype = {
                    collision_box = { { -0.2, -0.2 }, { 0.2, 0.2 } },
                    collision_mask = { layers = { player = true } },
                },
            },
        },
    }

    storage = { n_clients = 1, p = {} }
    game = {
        tick = 0,
        players = _players,
        connected_players = _players,
        forces = { player = { name = "player" } },
        surfaces = { _surface, nauvis = _surface },
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

/// The `pathfind_flags` of the single request the mod made.
fn flags_of_one_request(lua: &Lua) -> Table {
    let requests: Table = lua.globals().get("_requests").expect("_requests");
    assert_eq!(
        requests.len().expect("length"),
        1,
        "exactly one path request is expected here"
    );
    let request: Table = requests.get(1).expect("the request");
    request.get("pathfind_flags").expect("pathfind_flags")
}

/// **The defect.** `cache` is absent, so the game applies its default of
/// `true` and may answer from a search made before we built anything.
#[test]
fn a_walk_path_request_asks_for_an_uncached_search() {
    let lua = mod_lua();
    lua.load("request_player_path(game.players[1], { x = 30, y = 0 }, 2)")
        .set_name("request")
        .exec()
        .expect("request_player_path");

    let flags = flags_of_one_request(&lua);
    let cache: Value = flags.get("cache").expect("cache");
    assert_eq!(
        cache,
        Value::Boolean(false),
        "PathfinderFlags.cache defaults to TRUE when absent, and the 2.1 docs \
         say a cached path 'might fail to respond to changes in the \
         environment' -- we are what changes it. Run 30 returned 12 of 75 \
         paths with a waypoint strictly inside a furnace we had placed, each \
         flagged needs_destroy_to_reach=false. Got {cache:?}"
    );
}

/// The flags that were already there have to survive: this is one added key,
/// not a rewritten table. `prefer_straight_paths` in particular is what keeps
/// a walk from wandering, and dropping it would change every path in the run
/// while the ratio under test happened to improve.
#[test]
fn the_flags_that_were_already_there_are_unchanged() {
    let lua = mod_lua();
    lua.load("request_player_path(game.players[1], { x = 30, y = 0 }, 2)")
        .set_name("request")
        .exec()
        .expect("request_player_path");

    let flags = flags_of_one_request(&lua);
    assert_eq!(
        flags
            .get::<bool>("allow_destroy_friendly_entities")
            .expect("allow_destroy_friendly_entities"),
        false
    );
    assert_eq!(
        flags
            .get::<bool>("prefer_straight_paths")
            .expect("prefer_straight_paths"),
        true
    );
}

/// The character-specific parts of the request are what make this a *walk*
/// rather than a generic search, and they are read here so a future edit to
/// this table cannot quietly drop one while the cache assertion still passes.
#[test]
fn the_request_is_still_a_characters_own_path() {
    let lua = mod_lua();
    lua.load("request_player_path(game.players[1], { x = 30, y = 0 }, 2)")
        .set_name("request")
        .exec()
        .expect("request_player_path");

    let requests: Table = lua.globals().get("_requests").expect("_requests");
    let request: Table = requests.get(1).expect("the request");
    assert!(
        request.get::<Value>("bounding_box").expect("bounding_box") != Value::Nil,
        "the character's collision box, not a default one"
    );
    assert!(
        request
            .get::<Value>("collision_mask")
            .expect("collision_mask")
            != Value::Nil,
        "the character's collision mask"
    );
    assert!(
        request
            .get::<Value>("entity_to_ignore")
            .expect("entity_to_ignore")
            != Value::Nil,
        "the character itself, or it paths around where it is standing"
    );
}
