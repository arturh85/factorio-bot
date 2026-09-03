//! **The mod's recording *session*, and the screenshots it no longer takes.**
//!
//! Until 2026-09-02 the 300-tick beat also drove `game.take_screenshot` --
//! once per camera, *synchronously inside the game loop*. Run
//! `run-1788365280-15443` paid that six times per beat and produced 2,164
//! JPEGs at 1920x1080 for **947 MB**; the same 45 minutes of video is
//! **290 MB** at 700x854, grabbed from a frame the GPU had already drawn.
//! 3.3x the disk and UPS on top of it. The cameras are gone.
//!
//! What survived, and why this file did: the last line of
//! `on_sample_force_tick` is `sample_force(tick)`. Both samplers --
//! `sample_force` on the 300-tick beat and `sample_bots` on the 60-tick one --
//! are gated on `storage.sampling ~= nil`, so the *session* is what feeds
//! `samples.jsonl`, which is what the research, production and inventory
//! panels read. Deleting the session along with the screenshots would have
//! taken the whole world-state stream with it, silently.
//!
//! These tests load the real `mods/BotBridge/control.lua` into a Lua 5.4 state
//! and drive it against a stub game that records every `take_screenshot` --
//! so "no screenshot is taken" is asserted against the mod's own source rather
//! than assumed from the diff.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A sample beat. A multiple of 300 is not required -- `on_sample_force_tick`
/// is called directly here, exactly as `script.on_nth_tick` would call it.
const TICK: u64 = 1800;

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
    -- Real enough to hand back the nth-tick handlers the mod registers at
    -- load: the 60-tick bot sampler is a local closure and this is the only
    -- way to reach it, which is the point -- it rides on the same session the
    -- screenshots used to.
    _nth_tick = {}
    script = setmetatable(
        { on_nth_tick = function(n, f) _nth_tick[n] = f end },
        { __index = function() return noop end })
    remote = nooptable()
    commands = nooptable()
    require = function() return {} end

    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }

    -- Enough of a JSON writer to read a sample line back. Key order is not
    -- asserted anywhere; only the presence of a record and its `kind`.
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
    _removed = {}
    helpers = {
        table_to_json = json,
        write_file = function(path, data, append)
            _written[#_written + 1] = { path = path, data = data, append = append == true }
        end,
        remove_path = function(path) _removed[#_removed + 1] = path end,
    }

    storage = {}
    _players = {}
    _shots = {}

    _surface = {
        index = 1,
        find_entities_filtered = function() return {} end,
        find_entity = function() return nil end,
        find_non_colliding_position = function(name, center) return center end,
    }

    function make_player(idx, x, y, connected)
        local p = {
            index = idx,
            name = "bot" .. idx,
            connected = connected ~= false,
            position = { x = x, y = y },
            surface = _surface,
            character = {
                position = { x = x, y = y },
                mining_state = { mining = false },
                get_inventory = function()
                    return { get_contents = function() return {} end }
                end,
            },
            crafting_queue_size = 0,
        }
        _players[idx] = p
        return p
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
        take_screenshot = function(args)
            _shots[#_shots + 1] = args.path
        end,
    }
    prototypes = { item = {}, entity = {} }
"#;

const STUB_TICK_EXTRAS: &str = r#"
    writeout_initial_stuff = function() end
    writeout_recipes = function() end
    writeout_forces = function() end
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

/// Loads the mod and seeds `bots` connected players at distinct positions.
fn mod_with_bots(bots: u32) -> Lua {
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
    for index in 1..=bots {
        lua.load(format!("make_player({index}, {index}.5, 0.5, true)"))
            .set_name("player")
            .exec()
            .expect("player");
    }
    lua.load("game.connected_players = _players")
        .set_name("connected")
        .exec()
        .expect("connected");
    lua
}

/// Runs `rcon_sampling_start` with a literal Lua argument list, then one force
/// beat, and returns the paths `take_screenshot` was asked for -- which must
/// always be none.
fn session(lua: &Lua, args: &str) -> Vec<String> {
    lua.load(format!("rcon_sampling_start({args})"))
        .set_name("start")
        .exec()
        .expect("sampling starts");
    force_beat(lua, TICK);
    shots(lua)
}

/// One force beat, exactly as `script.on_nth_tick(300, ...)` delivers it.
/// `game.tick` is set as well as `event.tick` because the handler reads the
/// former -- deliberately, so a sample's tick comes from the same clock
/// `stamp_tick` reads.
fn force_beat(lua: &Lua, tick: u64) {
    lua.load(format!(
        "game.tick = {tick}; on_sample_force_tick({{ tick = {tick} }})"
    ))
    .set_name("force beat")
    .exec()
    .expect("force beat");
}

fn shots(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<Table>("_shots")
        .expect("_shots")
        .sequence_values::<String>()
        .map(|v| v.expect("path"))
        .collect()
}

/// Every `(path, data)` the mod wrote through `helpers.write_file`.
fn written(lua: &Lua) -> Vec<(String, String)> {
    lua.globals()
        .get::<Table>("_written")
        .expect("_written")
        .sequence_values::<Table>()
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.get::<String>("path").expect("path"),
                entry.get::<String>("data").expect("data"),
            )
        })
        .collect()
}

fn start_error(lua: &Lua, args: &str) -> String {
    let err = lua
        .load(format!("rcon_sampling_start({args})"))
        .set_name("start")
        .exec()
        .expect_err("the call must be refused");
    err.to_string()
}

/// Sample lines the mod wrote, most recent last.
///
/// The empty truncating write a session opens with is excluded: it is a
/// *clearing* of the file rather than a line in it, and counting it would make
/// "this run wrote no samples" indistinguishable from "this run started".
fn samples(lua: &Lua) -> Vec<String> {
    written(lua)
        .into_iter()
        .filter(|(path, data)| path == "botbridge/samples.jsonl" && !data.is_empty())
        .map(|(_, data)| data)
        .collect()
}

/// **The decision.** The 300-tick beat renders nothing. This is the UPS win,
/// and it is asserted against the mod's own source: the stub records every
/// `take_screenshot`, so a camera reintroduced anywhere on this path fails
/// here rather than showing up as a disk full of JPEGs on the next live run.
#[test]
fn a_sampling_beat_takes_no_screenshot() {
    let lua = mod_with_bots(4);
    assert!(
        session(&lua, "'run-1'").is_empty(),
        "the 300-tick beat must render nothing at all"
    );
    force_beat(&lua, TICK + 300);
    assert!(shots(&lua).is_empty(), "and it must keep rendering nothing");
}

/// Nothing gets captured whatever a caller says, because there is no longer an
/// argument that could ask for it. `rcon_sampling_start` takes a run id and
/// nothing else, so a second positional argument is a caller mistake -- but
/// this pins the outcome rather than the arity: even passing the old
/// `cameras = true` must not produce a render.
#[test]
fn the_retired_camera_argument_cannot_turn_rendering_back_on() {
    let lua = mod_with_bots(2);
    lua.load("rcon_sampling_start('run-1', true)")
        .set_name("start")
        .exec()
        .expect("an extra argument is ignored, not honoured");
    force_beat(&lua, TICK);
    assert!(
        shots(&lua).is_empty(),
        "there is no path back to `take_screenshot` from here. Got {:?}",
        shots(&lua)
    );
}

/// **Why the session outlived the screenshots.** Screenshots were the
/// expensive part; the world-state stream that rides on the same session is
/// not. `sample_force` and `sample_bots` both return early when
/// `storage.sampling` is nil, so a "fix" that stopped starting the session
/// would take research, production, power and bot inventories out of every run
/// with nothing to say it had.
#[test]
fn the_world_state_samplers_run_for_the_whole_session() {
    let lua = mod_with_bots(2);
    session(&lua, "'run-1'");

    assert!(
        samples(&lua)
            .iter()
            .any(|line| line.contains("\"kind\":\"force\"")),
        "the 300-tick beat still has to write a force sample -- it is what the \
         research, production and power panels read. Got {:?}",
        samples(&lua)
    );

    lua.load("_nth_tick[60]({ tick = 60 })")
        .set_name("bot beat")
        .exec()
        .expect("bot sample beat");
    assert!(
        samples(&lua)
            .iter()
            .any(|line| line.contains("\"kind\":\"bots\"")),
        "and the 60-tick beat still has to write bot samples. Got {:?}",
        samples(&lua)
    );
}

/// The gate is the session, not the tick. A beat that arrives before anybody
/// started one writes nothing -- which is what keeps a run nobody asked to
/// record from leaving a stream behind.
#[test]
fn a_beat_outside_a_session_writes_nothing() {
    let lua = mod_with_bots(2);
    force_beat(&lua, TICK);
    lua.load("_nth_tick[60]({ tick = 60 })")
        .set_name("bot beat")
        .exec()
        .expect("bot sample beat");
    assert!(
        samples(&lua).is_empty(),
        "no session, no samples. Got {:?}",
        samples(&lua)
    );
}

/// Stopping is the same gate from the other side: samples already written stay
/// on disk, and no new one is added.
#[test]
fn stopping_the_session_stops_the_samples() {
    let lua = mod_with_bots(2);
    session(&lua, "'run-1'");
    let before = samples(&lua).len();
    lua.load("rcon_sampling_stop()")
        .set_name("stop")
        .exec()
        .expect("sampling stops");
    force_beat(&lua, TICK + 300);
    assert_eq!(
        samples(&lua).len(),
        before,
        "a stopped session must write nothing further"
    );
}

/// Every line carries the session's run id, which is how Rust tells one run's
/// samples from a leftover file's. The id is opaque to the mod: it is echoed,
/// never parsed.
#[test]
fn every_sample_line_carries_the_session_run_id() {
    let lua = mod_with_bots(1);
    session(&lua, "'run-1'");
    assert!(
        samples(&lua)
            .iter()
            .all(|line| line.contains("\"run\":\"run-1\"")),
        "Got {:?}",
        samples(&lua)
    );
}

/// An untagged session writes no `run` key at all, rather than an empty string
/// or a null. Rust reads an absent key as "cannot tell" and falls back to
/// tick-range filtering; a value that looked like an id would defeat that.
#[test]
fn an_untagged_session_writes_no_run_key() {
    let lua = mod_with_bots(1);
    session(&lua, "");
    assert!(
        samples(&lua).iter().all(|line| !line.contains("\"run\"")),
        "Got {:?}",
        samples(&lua)
    );
}

/// Starting truncates the sample file, so one run cannot inherit the previous
/// run's stream. `append = false` is the whole of it, and it is asserted
/// because the flag is easy to get backwards and the symptom -- a run that
/// reports the last run's production -- looks like a Rust bug.
#[test]
fn starting_a_session_truncates_the_previous_runs_samples() {
    let lua = mod_with_bots(1);
    lua.load("rcon_sampling_start('run-1')")
        .set_name("start")
        .exec()
        .expect("sampling starts");
    let truncation = lua
        .load(
            r#"
            for _, entry in ipairs(_written) do
                if entry.path == "botbridge/samples.jsonl" then
                    return entry.data == "" and not entry.append
                end
            end
            return false
        "#,
        )
        .set_name("truncation")
        .eval::<bool>()
        .expect("truncation check");
    assert!(
        truncation,
        "the first write of a session must be an empty, non-appending one"
    );
}

/// The type check is refused *before* anything is written, so a call the mod
/// is going to reject cannot first destroy the previous run's samples.
#[test]
fn a_run_id_that_is_not_a_string_is_refused_before_anything_is_written() {
    let lua = mod_with_bots(1);
    let err = start_error(&lua, "42");
    assert!(
        err.contains("sampling run id must be a string"),
        "Got {err}"
    );
    assert!(
        !written(&lua)
            .iter()
            .any(|(path, _)| path == "botbridge/samples.jsonl"),
        "a refused start must not have touched the sample file at all. Got {:?}",
        written(&lua)
    );
}
