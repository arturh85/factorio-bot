//! **The mod dies at tick 0 if anything happens before the first `on_tick`.**
//!
//! ```text
//! control.lua: attempt to index upvalue 'client_local_data' (a nil value)
//!   in on_player_joined_game
//! -> non-recoverable, mod state goes to Failed
//! ```
//!
//! `client_local_data` was declared `= nil` at the top of the file and
//! initialised *inside* `on_tick`. Every other reader of it -- the join
//! handler, the chunk handler, `rcon_whoami` -- therefore depended on a tick
//! having run first, and nothing said so. Found by a `--host` probe
//! (`docs/superpowers/notes/2026-09-02-graphical-host-probe.md`), where the
//! host's own player joins at tick 0, but the precondition is not
//! `--host`-specific: it is *any* handler firing before the first tick, which
//! is a shape, not a configuration.
//!
//! The fix is to initialise it at load rather than to guard each reader. It
//! has a natural empty value -- `{whoami = nil}` is exactly what `on_tick` was
//! assigning -- so the ordering dependency can be removed instead of tolerated,
//! and the next handler added to this file does not inherit the trap. It is
//! per-peer module state and deliberately not in `storage`, so initialising it
//! identically on every peer at load is not a desync risk: it is the same
//! table every peer would have built on its own first tick.
//!
//! Nothing about a pre-tick join is lost by not skipping. `whoami` is nil until
//! `rcon_whoami` sets it, and it was nil on the first tick too, so the one
//! `whoami`-gated branch in the join handler could never have run at tick 0
//! either way. What the handler does unconditionally -- count the client and
//! wait for its inventory -- now runs instead of raising.

use mlua::{Lua, LuaOptions, StdLib, Table};

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
    rcon = { print = noop }
    helpers = {
        table_to_json = function() return "{}" end,
        write_file = noop,
        remove_path = noop,
    }

    _surface = {
        index = 1,
        find_entity = function() return nil end,
        find_entities_filtered = function() return {} end,
        get_chunks = function() return function() return nil end end,
    }
    _shots = {}
    _players = {}
    function make_player(idx, x, y)
        _players[idx] = {
            index = idx,
            name = "bot" .. idx,
            connected = true,
            position = { x = x, y = y },
            character = { position = { x = x, y = y } },
            get_main_inventory = function() return nil end,
            surface = _surface,
        }
    end

    -- `n_clients` is what `on_player_joined_game` increments; a run that has
    -- not ticked yet has still had `on_init`/`on_load` set this up.
    storage = { n_clients = 0, p = {}, map_area = { x1 = 0, y1 = 0, x2 = 0, y2 = 0 } }
    game = {
        tick = 0,
        players = _players,
        connected_players = {},
        forces = {},
        -- Keyed as well as indexed: `on_chunk_generated` compares its event
        -- surface against `game.surfaces['nauvis']` and returns early on a
        -- mismatch, so a stub without the key makes that test pass vacuously.
        surfaces = { _surface, nauvis = _surface },
        take_screenshot = function(args) _shots[#_shots + 1] = args.path end,
    }
    prototypes = { item = {}, entity = {} }
"#;

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
    // Deliberately NO `on_tick` call anywhere in this file.
    lua.load("make_player(1, 0.5, 0.5)")
        .set_name("player")
        .exec()
        .expect("player");
    lua
}

fn run(lua: &Lua, chunk: &str) -> Result<(), mlua::Error> {
    lua.load(chunk.to_string()).set_name("pre-tick").exec()
}

/// **The crash.** A player joining before the first tick took the whole mod to
/// `Failed`, and the message named a nil upvalue rather than the ordering that
/// caused it.
#[test]
fn a_player_may_join_before_the_first_tick() {
    let lua = fresh_mod();
    run(&lua, "on_player_joined_game({ player_index = 1 })").unwrap_or_else(|err| {
        panic!(
            "joining before any tick has run must not raise -- the mod goes to \
             Failed and stays there. Got: {err}"
        )
    });
    assert_eq!(
        lua.globals()
            .get::<Table>("storage")
            .expect("storage")
            .get::<u32>("n_clients")
            .expect("n_clients"),
        1,
        "and the join is still counted: skipping the handler would lose a real \
         player, which is why this is an initialisation fix and not a guard"
    );
}

/// The same trap, reached through the call the host actually makes first.
/// `rcon_whoami` is how a peer learns its own name, and nothing schedules it
/// after a tick.
#[test]
fn whoami_may_be_answered_before_the_first_tick() {
    let lua = fresh_mod();
    run(&lua, "rcon_whoami('client1')")
        .unwrap_or_else(|err| panic!("rcon_whoami before any tick must not raise. Got: {err}"));
}

/// And through chunk generation, which fires while the map is being built --
/// before, during and after the first tick, in that order on a fresh save.
#[test]
fn a_chunk_may_be_generated_before_the_first_tick() {
    let lua = fresh_mod();
    run(
        &lua,
        r#"
        writeout_entities = function() end
        writeout_tiles = function() end
        on_chunk_generated({
            tick = 0,
            area = { left_top = { x = 0, y = 0 }, right_bottom = { x = 32, y = 32 } },
            surface = _surface,
        })
        "#,
    )
    .unwrap_or_else(|err| panic!("on_chunk_generated before any tick must not raise. Got: {err}"));
}

/// The state is real, not merely non-raising.
///
/// This is what rules out the other candidate fix -- a guard in each reader
/// that returns early before the first tick. Under a guard, an identity
/// learned pre-tick is dropped on the floor and the peer spends the run as
/// "?": `client_local_data` is where `whoami` lives, and skipping the write is
/// how you lose it. Here the identity survives, and the one branch that acts
/// on it acts.
#[test]
fn an_identity_learned_before_the_first_tick_is_still_the_peers_identity() {
    let lua = fresh_mod();
    run(&lua, "rcon_whoami('client1')").expect("whoami pre-tick");
    run(&lua, "on_player_joined_game({ player_index = 1 })").expect("join pre-tick");

    let shots = lua
        .globals()
        .get::<Table>("_shots")
        .expect("_shots")
        .len()
        .expect("length");
    assert!(
        shots > 0,
        "client1's join sweep is gated on whoami, so a fix that dropped the \
         pre-tick identity would leave this at 0 while every other assertion \
         here still passed"
    );
}
