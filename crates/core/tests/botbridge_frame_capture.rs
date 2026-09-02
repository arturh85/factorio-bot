//! **Screenshot cameras are retired: a capture run takes no picture unless
//! somebody asks for one.**
//!
//! `game.take_screenshot` renders *synchronously inside the game loop*, once
//! per camera, every `FRAME_CAPTURE_INTERVAL` ticks. Run
//! `run-1788365280-15443` paid that six times per capture and produced 2,164
//! JPEGs at 1920x1080 for **947 MB**; the same 45 minutes of video is
//! **290 MB** at 700x854, grabbed from a frame the GPU had already drawn.
//! 3.3x the disk and UPS on top of it.
//!
//! What makes this a *switch* rather than a deletion is the last line of
//! `on_frame_capture_tick`: `sample_force(tick)`. Both samplers --
//! `sample_force` on the 300-tick beat and `sample_bots` on the 60-tick one --
//! are gated on `storage.frame_capture ~= nil`, so the capture *session* is
//! what feeds `samples.jsonl`, which is what the research, production and
//! inventory panels read. Not calling `frame_capture_start` at all would take
//! the whole world-state stream out with the screenshots, silently. So the
//! session still starts; it just registers no camera.
//!
//! These tests load the real `mods/BotBridge/control.lua` into a Lua 5.4 state
//! and drive it against a stub game that records every `take_screenshot`, so
//! what is asserted is the mod's own choice of what to render and when.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A capture tick. A multiple of 300 is not required -- `on_frame_capture_tick`
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
    -- way to reach it, which is the point -- it rides on the same capture
    -- session the screenshots did.
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

/// Runs `rcon_frame_capture_start` with a literal Lua argument list, then one
/// capture tick, and returns the paths `take_screenshot` was asked for.
fn capture(lua: &Lua, args: &str) -> Vec<String> {
    lua.load(format!("rcon_frame_capture_start({args})"))
        .set_name("start")
        .exec()
        .expect("frame capture starts");
    tick_capture(lua, TICK);
    shots(lua)
}

/// One capture beat, exactly as `script.on_nth_tick(300, ...)` delivers it.
/// `game.tick` is set as well as `event.tick` because the handler reads the
/// former -- deliberately, so a frame's name comes from the same clock
/// `stamp_tick` reads.
fn tick_capture(lua: &Lua, tick: u64) {
    lua.load(format!(
        "game.tick = {tick}; on_frame_capture_tick({{ tick = {tick} }})"
    ))
    .set_name("capture tick")
    .exec()
    .expect("capture tick");
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
        .load(format!("rcon_frame_capture_start({args})"))
        .set_name("start")
        .exec()
        .expect_err("the call must be refused");
    err.to_string()
}

/// **The decision.** A run that says nothing about cameras gets no camera, and
/// therefore pays no synchronous render inside the game loop.
#[test]
fn a_capture_started_without_asking_for_cameras_takes_no_screenshot() {
    let lua = mod_with_bots(4);
    let paths = capture(&lua, "'run-1'");
    assert!(
        paths.is_empty(),
        "screenshots are retired: the default capture must render nothing. \
         Run run-1788365280-15443 wrote 2164 JPEGs / 947 MB this way, six \
         cameras per capture, each rendered synchronously inside the update \
         loop. Got {paths:?}"
    );
}

/// The negative control for the test above, and the reason this is a switch
/// rather than a deletion: the capture code is correct, only its cost is not
/// worth paying by default.
#[test]
fn asking_for_every_camera_still_captures_exactly_what_it_used_to() {
    let lua = mod_with_bots(4);
    let paths = capture(&lua, "'run-1', true");
    assert_eq!(
        paths,
        vec![
            format!("frames/tick-{TICK:07}-follow.jpg"),
            format!("frames/tick-{TICK:07}-bot-1.jpg"),
            format!("frames/tick-{TICK:07}-bot-2.jpg"),
            format!("frames/tick-{TICK:07}-bot-3.jpg"),
            format!("frames/tick-{TICK:07}-bot-4.jpg"),
            format!("frames/tick-{TICK:07}-area.jpg"),
        ],
        "`2 + one per player`, in registration order, unchanged"
    );
}

/// The reason the argument is a camera list rather than a boolean: the cost is
/// per camera, and one camera is 1/6th of what run-1788365280-15443 paid.
#[test]
fn a_named_subset_captures_only_those_cameras() {
    let lua = mod_with_bots(4);
    let paths = capture(&lua, "'run-1', {'follow'}");
    assert_eq!(
        paths,
        vec![format!("frames/tick-{TICK:07}-follow.jpg")],
        "one camera asked for, one camera rendered"
    );
}

/// A camera id nobody can supply must be refused, not quietly dropped. A
/// silent drop is the same failure as a flag defaulting wrong, only harder to
/// notice: the run looks configured and captures nothing.
#[test]
fn an_unknown_camera_id_is_refused_rather_than_capturing_nothing() {
    let lua = mod_with_bots(2);
    let message = start_error(&lua, "'run-1', {'bot-9'}");
    assert!(
        message.contains("bot-9"),
        "the refusal has to name the id that was not available. Got {message}"
    );
    assert!(
        shots(&lua).is_empty(),
        "and nothing may be captured by a call that was refused"
    );
}

/// Refused *before* the wipe, for the same reason the run-id type check is:
/// a call this function is going to reject must not first destroy the previous
/// run's frames.
#[test]
fn a_refused_camera_list_does_not_wipe_the_previous_runs_frames() {
    let lua = mod_with_bots(2);
    let _ = start_error(&lua, "'run-1', {'nonesuch'}");
    let removed: Vec<String> = lua
        .globals()
        .get::<Table>("_removed")
        .expect("_removed")
        .sequence_values::<String>()
        .map(|v| v.expect("path"))
        .collect();
    assert!(
        removed.is_empty(),
        "the wipe must come after validation. Got {removed:?}"
    );
}

/// **The load-bearing negative control.** Screenshots are retired; the sampling
/// beat that rides on the same session is not. `sample_force` and `sample_bots`
/// both return early when `storage.frame_capture` is nil, so a "fix" that
/// stopped starting the session would take research, production, power and bot
/// inventories out of every run with nothing to say it had.
#[test]
fn the_world_state_samplers_still_run_when_no_camera_was_asked_for() {
    let lua = mod_with_bots(2);
    assert!(capture(&lua, "'run-1'").is_empty(), "no camera, as decided");

    let samples: Vec<String> = written(&lua)
        .into_iter()
        .filter(|(path, _)| path == "botbridge/samples.jsonl")
        .map(|(_, data)| data)
        .collect();
    assert!(
        samples
            .iter()
            .any(|line| line.contains("\"kind\":\"force\"")),
        "the 300-tick beat still has to write a force sample -- it is what the \
         research, production and power panels read. Got {samples:?}"
    );

    lua.load("_nth_tick[60]({ tick = 60 })")
        .set_name("bot beat")
        .exec()
        .expect("bot sample beat");
    let samples: Vec<String> = written(&lua)
        .into_iter()
        .filter(|(path, _)| path == "botbridge/samples.jsonl")
        .map(|(_, data)| data)
        .collect();
    assert!(
        samples
            .iter()
            .any(|line| line.contains("\"kind\":\"bots\"")),
        "and the 60-tick beat still has to write bot samples. Got {samples:?}"
    );
}

/// A run that captured nothing must read as *"none were captured"*, never as a
/// missing directory a consumer cannot judge. The sidecar is what says the
/// (empty) directory belongs to this run, which is what makes
/// `archive_frames` walk it and write `index.json: []` rather than skip it as
/// somebody else's leftovers.
#[test]
fn the_run_sidecar_is_written_even_when_no_camera_was_asked_for() {
    let lua = mod_with_bots(2);
    capture(&lua, "'run-1'");
    let sidecar = written(&lua)
        .into_iter()
        .find(|(path, _)| path == "frames/run.json");
    assert_eq!(
        sidecar,
        Some((
            "frames/run.json".to_string(),
            r#"{"run":"run-1"}"#.to_string()
        )),
        "zero frames is an answer; an unclaimed directory is a shrug"
    );
}

/// A bot joining mid-run gets a camera only when the run asked for all of
/// them. `frame_capture_on_player_joined` exists so a late joiner is not
/// invisible -- but on a run that deliberately registered no camera it would
/// switch capture back on, one bot at a time, and nothing would report it.
#[test]
fn a_bot_joining_mid_run_gets_no_camera_when_none_were_asked_for() {
    let lua = mod_with_bots(1);
    capture(&lua, "'run-1'");
    lua.load("make_player(2, 9.5, 0.5, true); frame_capture_on_player_joined(2)")
        .set_name("join")
        .exec()
        .expect("join");
    tick_capture(&lua, TICK + 300);
    assert!(
        shots(&lua).is_empty(),
        "a joiner must not re-arm a capture that was started with no cameras. \
         Got {:?}",
        shots(&lua)
    );
}

/// The other half of the rule above: on an all-cameras run a late joiner still
/// gets its own camera from the tick it arrived, which is the behaviour
/// `frame_capture_on_player_joined` was written for.
#[test]
fn a_bot_joining_an_all_camera_run_still_gets_its_own_camera() {
    let lua = mod_with_bots(1);
    capture(&lua, "'run-1', true");
    lua.load("make_player(2, 9.5, 0.5, true); frame_capture_on_player_joined(2)")
        .set_name("join")
        .exec()
        .expect("join");
    tick_capture(&lua, TICK + 300);
    assert!(
        shots(&lua)
            .iter()
            .any(|path| path == &format!("frames/tick-{:07}-bot-2.jpg", TICK + 300)),
        "Got {:?}",
        shots(&lua)
    );
}

/// `false` is spelled out as well as absent, because a caller threading an
/// option through from somewhere else will produce one or the other and the
/// two have to mean the same thing.
#[test]
fn false_means_the_same_as_saying_nothing() {
    let lua = mod_with_bots(2);
    assert!(capture(&lua, "'run-1', false").is_empty());
}

/// An argument of the wrong type is a caller error, and a caller error that
/// silently captures everything would be the expensive direction to fail in.
#[test]
fn a_cameras_argument_that_is_not_a_list_is_refused() {
    let lua = mod_with_bots(2);
    let message = start_error(&lua, "'run-1', 7");
    assert!(
        message.contains("camera"),
        "the refusal has to say what it refused. Got {message}"
    );
}
