//! **A save the mod has never seen hangs the host at `start waiting`, for ever,
//! and nothing about the message says why.**
//!
//! Measured on 2026-09-06 against `workspace/any-wr-6-39-53.zip`, a 6:39:53
//! Space Age rocket launch. Factorio itself was healthy the whole time --
//! `workspace/wrload/server/factorio-current.log` shows it migrate 2.0.66 to
//! 2.1.17, host at 4.4 s, autosave twice, and shut down cleanly on SIGTERM at
//! 900 s. The server sat at ~1.15 GB resident and ~25% CPU, which is one core
//! simulating a big base with nothing to do. **Our side never sent it a single
//! RCON command.**
//!
//! ## The chain, and every link is in this repo
//!
//! `read_output` (`crates/core/src/process/output_reader.rs`) blocks on
//! `rx1.recv()` until a stdout line `contains("my_client_id")`. Only then does
//! it connect RCON and call `initialize_server`, which is what sends
//! `whoami("server")`, which is what calls `on_whoami`, which is what builds
//! `client_local_data.initial_discovery` -- the replay that carries a loaded
//! save's already-generated chunks into the world model. **No beacon, no RCON,
//! no discovery, no world.**
//!
//! The only line in the mod that can print that substring is the tick-120 beat
//! in `on_tick`, and it was guarded on `my_client_id ~= nil`. `my_client_id` is
//! a module local assigned in exactly one place: `on_load`.
//!
//! And `on_load` does not run. Quoting the shipped 2.1.17 API docs
//! (`workspace/factorio-api-docs/runtime-api.json`, `LuaBootstrap::on_load`):
//!
//! > This is **only** called for mods that have been part of the save
//! > previously, or for players connecting to a running multiplayer session.
//!
//! A save created without BotBridge gets `on_init` instead -- "only called when
//! a new save game is created or when a save file is loaded that previously
//! didn't contain the mod" -- and `on_init` never touched `my_client_id`. So on
//! any foreign save the local stays `nil` for the whole session and the beacon
//! is silent for ever.
//!
//! ## Why every normal run was fine, which is why nobody found it
//!
//! Our own runs create the map in a *separate* `--create` invocation
//! (`instance_setup.rs`) and then start a *second* process with
//! `--start-server level.zip`. That second process loads a save the mod is
//! already part of, so it takes the `on_load` path and the beacon fires. The
//! bug is reachable only by the one thing this project had never done: hand
//! Factorio a save somebody else made.
//!
//! ## Two changes, because the fault has two halves
//!
//! * `on_init` sets `my_client_id` too, restoring the invariant the rest of the
//!   file assumes -- that it holds a number from the first tick of every
//!   session, however that session began.
//! * The beacon no longer refuses to speak when it is `nil`. A readiness signal
//!   that goes quiet exactly when state is unexpected is the "silence is not
//!   success" shape CLAUDE.md enumerates; printing `my_client_id=nil` still
//!   unblocks the host and still says something true. This is the half that
//!   would have turned a 900-second timeout into a one-line answer.
//!
//! The mod's own comment above `rcon_session_reset` called this print a "debug
//! print" whose value "nothing reads". Nothing in the *mod* reads it. The host
//! reads it, and it is the only thing the host waits for.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game with no freeplay interface, no players and no chunks.
///
/// `storage` is the **empty table the engine hands a mod it has never seen** --
/// not the populated one a save restores. Pre-seeding the fields `on_init` is
/// supposed to write would let a broken `on_init` pass, so the tests that
/// exercise the `on_load` path put their own `storage` in place explicitly.
const PRELUDE: &str = r#"
    local function auto()
        local t = {}
        setmetatable(t, { __index = function(tbl, k)
            local v = auto(); rawset(tbl, k, v); return v
        end })
        return t
    end
    defines = auto()
    defines.controllers = { cutscene = "cutscene", character = "character" }

    function noop() end
    local function nooptable()
        return setmetatable({}, { __index = function() return noop end })
    end
    script = nooptable()
    commands = nooptable()
    require = function() return {} end

    -- `remote.interfaces` must be a real table with no `freeplay` key:
    -- `disable_crashsite` indexes it and takes the "no freeplay interface"
    -- branch, which is what a foreign save's scenario looks like.
    remote = { interfaces = {}, call = noop, add_interface = noop }

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
        name = "nauvis",
        find_entity = function() return nil end,
        find_entities_filtered = function() return {} end,
        get_chunks = function() return function() return nil end end,
    }
    _players = {}
    storage = {}
    game = {
        tick = 0,
        players = _players,
        connected_players = {},
        forces = {},
        surfaces = { _surface, nauvis = _surface },
        take_screenshot = noop,
    }
    prototypes = { item = {}, entity = {} }

    -- The beacon is the only thing under test. Everything the tick handler
    -- would otherwise do on tick 0 -- the static-data dump, the bot poll, the
    -- drill tracker -- is stubbed after control.lua loads, in `tick_to`.
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
    lua.load(
        r#"
        -- Narrow the tick handler to the beat under test. These are the only
        -- other things `on_tick` does before the beacon, and each one wants a
        -- game far richer than this stub.
        poll_character_bots = function() end
        track_mining_drills = function() end
        writeout_initial_stuff = function() end
        "#,
    )
    .set_name("narrow_on_tick")
    .exec()
    .expect("narrow on_tick");
    lua
}

/// Run `on_tick` for ticks 0..=`last`, as the game does.
fn tick_to(lua: &Lua, last: u32) {
    lua.load(format!(
        "for t = 0, {last} do game.tick = t; on_tick({{ tick = t }}) end"
    ))
    .set_name("ticks")
    .exec()
    .expect("ticks");
}

fn beacons(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("printed line"))
        .filter(|l| l.contains("my_client_id"))
        .collect()
}

/// **The failure, exactly as the host meets it.** A foreign save takes the
/// `on_init` path, and nothing else. 240 ticks is two full beacon beats.
#[test]
fn a_save_the_mod_was_added_to_still_announces_itself() {
    let lua = fresh_mod();
    lua.load("on_init()")
        .set_name("on_init")
        .exec()
        .expect("on_init");
    tick_to(&lua, 240);

    let seen = beacons(&lua);
    assert!(
        !seen.is_empty(),
        "on a save that never contained BotBridge, Factorio calls on_init and \
         NOT on_load, so `my_client_id` is never assigned and the host's \
         `rx1.recv()` in output_reader.rs waits for a line that will never be \
         printed. That is the 900-second `start waiting` hang on the \
         world-record save. Printed lines were: {:?}",
        lua.globals()
            .get::<Table>("_printed")
            .expect("_printed")
            .len()
            .expect("len")
    );
}

/// The value is real, not merely a substring. A beacon that said
/// `my_client_id=nil` would unblock the host -- which is why the second half of
/// the fix is worth having -- but `on_init` is supposed to have produced a
/// count, and this is what tells the two fixes apart.
#[test]
fn the_beacon_carries_the_client_count_on_init() {
    let lua = fresh_mod();
    lua.load("on_init()")
        .set_name("on_init")
        .exec()
        .expect("on_init");
    tick_to(&lua, 120);

    let seen = beacons(&lua);
    assert_eq!(
        seen.first().map(String::as_str),
        Some("my_client_id=1, who=?"),
        "on_init sets storage.n_clients = 1, so the beacon that follows it \
         must carry 1 -- not `nil`, which would mean only the defence-in-depth \
         half of the fix took"
    );
}

/// The path that always worked keeps working. A save the mod was already part
/// of gets `on_load` and no `on_init`, and `storage` comes back off the save.
#[test]
fn a_save_the_mod_was_already_part_of_still_announces_itself() {
    let lua = fresh_mod();
    lua.load("storage = { n_clients = 3, p = {}, map_area = { x1 = 0, y1 = 0, x2 = 0, y2 = 0 } }")
        .set_name("restore_storage")
        .exec()
        .expect("storage");
    lua.load("on_load()")
        .set_name("on_load")
        .exec()
        .expect("on_load");
    tick_to(&lua, 120);

    assert_eq!(
        beacons(&lua).first().map(String::as_str),
        Some("my_client_id=3, who=?"),
        "the on_load path is how every run this project has ever measured \
         starts; it must still carry the count the save was taken at"
    );
}

/// **The second half of the fix, on its own.** A save that *did* contain the
/// mod but whose `storage` has no `n_clients` -- a save written by a BotBridge
/// older than that field, or any future rename of it -- takes the `on_load`
/// path, so `on_init` never runs and `my_client_id` is `nil` again. The old
/// beacon refused to speak in exactly that state and the host hung with no
/// message; this one says `nil`, which unblocks the handshake and names the
/// surprise. Without this test, restoring the `~= nil` guard passes every
/// other assertion in the file.
#[test]
fn the_beacon_speaks_even_when_it_has_no_number_to_report() {
    let lua = fresh_mod();
    lua.load("storage = { p = {}, map_area = { x1 = 0, y1 = 0, x2 = 0, y2 = 0 } }")
        .set_name("storage_without_n_clients")
        .exec()
        .expect("storage");
    lua.load("on_load()")
        .set_name("on_load")
        .exec()
        .expect("on_load");
    tick_to(&lua, 120);

    let seen = beacons(&lua);
    assert_eq!(
        seen.len(),
        2,
        "two beats in ticks 0..=120, and an unknown client id must silence \
         neither of them"
    );
    assert_eq!(
        seen.first().map(String::as_str),
        Some("my_client_id=nil, who=?"),
        "and it reports the surprise rather than hiding it"
    );
}

/// **The beacon stops once the peer knows its own name**, which is the one
/// thing the `who == \"?\"` guard is for. Removing that guard would spam a line
/// the host greps for on every 120th tick of every run.
#[test]
fn the_beacon_falls_silent_once_the_peer_has_a_name() {
    let lua = fresh_mod();
    lua.load("on_init()")
        .set_name("on_init")
        .exec()
        .expect("on_init");
    tick_to(&lua, 120);
    let before = beacons(&lua).len();
    assert_eq!(
        before, 2,
        "ticks 0..=120 contain two beats, because 0 % 120 == 0 -- the first \
         beacon lands on the very first tick, which is what makes the \
         handshake cheap"
    );

    lua.load("rcon_whoami('server')")
        .set_name("whoami")
        .exec()
        .expect("whoami");
    tick_to(&lua, 480);
    assert_eq!(
        beacons(&lua).len(),
        before,
        "after `whoami` the host is already talking to us and the beacon has \
         nothing left to say; three further beats must add nothing"
    );
}
