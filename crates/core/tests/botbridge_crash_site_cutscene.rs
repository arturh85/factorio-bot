//! **Freeplay hid bot 1 for 750 ticks, and three of four bots did nothing for
//! a whole run.**
//!
//! `workspace/data/base/script/freeplay/freeplay.lua`'s `on_player_created`
//! ends with `crash_site.create_cutscene(player, {-5, -4})`, reached only on
//! the first player (`storage.init_ran` gates the block, and
//! `crash-site.lua`'s handlers gate the exit on `player_index == 1`). The
//! waypoints total 450 + 150 + 150 = **750 ticks, 12.5 seconds**, and while a
//! cutscene is running `LuaPlayer::character` is nil -- the character is
//! parked in `cutscene_character`.
//!
//! `rcon_players()` filters on `player.connected and player.character`, so for
//! those 12.5 seconds **player 1 does not exist as far as any caller is
//! concerned, and only ever player 1.** In run 30 the script's roster poll
//! landed in that window, got `[2]`, and froze it for the rest of the run:
//! bots 1, 3 and 4 sat at spawn for 162,158 ticks with zero dispatches. Four
//! of the nineteen archived runs with samples carry the same signature.
//!
//! **The second defect is the one that will outlive the first.** A bot in the
//! cutscene is observationally identical to the invented phantom bot
//! `Planner::initiate_missing_players_with_default_inventory` seeds: `(0, 0)`
//! -- the spawn point -- holding `{burner-mining-drill, stone-furnace, wood}`,
//! which is what a healthy freeplay *first* player holds, because
//! `on_player_created` grants `created_items` to everyone and then removes the
//! crash debris (`iron-plate 8`) from player 1 alone. Neither position nor
//! inventory can tell the two apart, and that ambiguity has already produced
//! one wrong diagnosis.
//!
//! Two changes, because they fix two different things:
//!
//! * `set_disable_crashsite` at `on_init` stops the block ever running, so
//!   player 1 keeps its eight iron plates and stops *looking* like a phantom.
//! * `exit_cutscene()` on join ends a cutscene that is already running, which
//!   is the case `set_disable_crashsite` cannot reach: a save created before
//!   this change, or any run where the remote call did not take.
//!
//! See `docs/superpowers/notes/2026-09-02-bot-one-idle.md`.

use mlua::{Lua, LuaOptions, StdLib, Table, Value};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game with a freeplay remote interface and players who can be in a
/// cutscene.
///
/// `exit_cutscene` **raises** when the player is not in a cutscene, because
/// the real one does: the 2.1.17 runtime API says "Errors if not in a
/// cutscene", and freeplay's own `skip_crash_site_cutscene` guards on
/// `controller_type` before calling it. A stub that quietly did nothing would
/// let an unguarded call pass here and abort a live run.
const PRELUDE: &str = r#"
    local function auto()
        local t = {}
        setmetatable(t, { __index = function(tbl, k)
            local v = auto(); rawset(tbl, k, v); return v
        end })
        return t
    end
    defines = auto()
    -- Distinct, comparable controller ids: the mod has to compare against
    -- `defines.controllers.cutscene` and nothing else.
    defines.controllers = { cutscene = "cutscene", character = "character", god = "god" }
    defines.inventory = { character_main = 1 }

    function noop() end
    local function nooptable()
        return setmetatable({}, { __index = function() return noop end })
    end
    script = nooptable()
    commands = nooptable()
    require = function() return {} end

    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end
    _rcon_printed = {}
    rcon = { print = function(s) _rcon_printed[#_rcon_printed + 1] = tostring(s) end }
    helpers = {
        -- Enough of an encoder to tell which players came back.
        table_to_json = function(t)
            local parts = {}
            for _, v in ipairs(t) do parts[#parts + 1] = tostring(v.player_id) end
            return "[" .. table.concat(parts, ",") .. "]"
        end,
        write_file = noop,
        remove_path = noop,
    }

    _remote_calls = {}
    remote = {
        interfaces = { freeplay = { set_disable_crashsite = true } },
        add_interface = noop,
        call = function(iface, fn, ...)
            if remote.interfaces[iface] == nil then
                error("Remote interface " .. tostring(iface) .. " does not exist")
            end
            _remote_calls[#_remote_calls + 1] =
                { interface = tostring(iface), fn = tostring(fn), arg = ... }
        end,
    }

    _exit_cutscene_calls = {}

    local function make_character(idx)
        return {
            position = { x = idx, y = 0 },
            valid = true,
            destructible = true,
            character_running_speed = 0.15,
            get_main_inventory = function() return { get_contents = function() return {} end } end,
        }
    end

    -- A player mid-cutscene: connected, but `character` is nil and the
    -- character it will get back is parked in `cutscene_character`. This is
    -- exactly the shape freeplay leaves player 1 in for 750 ticks.
    function make_player(idx, in_cutscene)
        local character = make_character(idx)
        local player = {
            index = idx,
            name = "bot" .. idx,
            connected = true,
            force = { name = "player" },
            position = { x = idx, y = 0 },
            character = character,
            cutscene_character = nil,
            controller_type = defines.controllers.character,
            get_main_inventory = function()
                return { get_contents = function() return {} end }
            end,
            get_inventory = function()
                return { get_contents = function() return {} end }
            end,
            print = noop,
            gui = { screen = {} },
        }
        if in_cutscene then
            player.character = nil
            player.cutscene_character = character
            player.controller_type = defines.controllers.cutscene
        end
        player.exit_cutscene = function()
            if player.controller_type ~= defines.controllers.cutscene then
                error("Error in exit_cutscene: player is not in a cutscene")
            end
            _exit_cutscene_calls[#_exit_cutscene_calls + 1] = idx
            player.controller_type = defines.controllers.character
            player.character = player.cutscene_character
            player.cutscene_character = nil
        end
        return player
    end

    storage = { n_clients = 0, p = {} }
    game = {
        tick = 0,
        players = {},
        connected_players = {},
        forces = {},
        surfaces = {},
        take_screenshot = noop,
        is_multiplayer = function() return true end,
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
    // The handlers `on_player_joined_game` calls that have nothing to do with
    // cutscenes and would need a whole stub surface of their own.
    lua.load(
        r#"
        wait_for_player_inventory = function() end
        frame_capture_on_player_joined = function() end
        "#,
    )
    .set_name("stubs")
    .exec()
    .expect("handler stubs");
    lua
}

/// Player indices `exit_cutscene` was called for, in order.
fn exit_cutscene_calls(lua: &Lua) -> Vec<i64> {
    lua.globals()
        .get::<Table>("_exit_cutscene_calls")
        .expect("_exit_cutscene_calls")
        .sequence_values::<i64>()
        .map(|v| v.expect("index"))
        .collect()
}

/// Every `remote.call` the mod made, as `(interface, function, argument)`.
fn remote_calls(lua: &Lua) -> Vec<(String, String, bool)> {
    lua.globals()
        .get::<Table>("_remote_calls")
        .expect("_remote_calls")
        .sequence_values::<Table>()
        .map(|v| {
            let t = v.expect("call");
            (
                t.get::<String>("interface").expect("interface"),
                t.get::<String>("fn").expect("fn"),
                matches!(t.get::<Value>("arg").expect("arg"), Value::Boolean(true)),
            )
        })
        .collect()
}

fn rcon_reply(lua: &Lua) -> String {
    let replies: Vec<String> = lua
        .globals()
        .get::<Table>("_rcon_printed")
        .expect("_rcon_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect();
    replies.last().expect("one rcon reply").clone()
}

/// **The mechanism.** Not a regression test -- it cannot go red, because
/// nothing here proposes to change the filter. It pins *why* the cutscene
/// matters: a player without a character is not a player as far as any caller
/// of `rcon.players()` is concerned, and that is the whole of the defect.
#[test]
fn a_player_in_a_cutscene_is_invisible_to_rcon_players() {
    let lua = mod_lua();
    lua.load(
        r#"
        game.players = { [1] = make_player(1, true), [2] = make_player(2, false) }
        rcon_players()
        "#,
    )
    .set_name("rcon_players")
    .exec()
    .expect("rcon_players");

    assert_eq!(
        rcon_reply(&lua),
        "[2]",
        "player 1 is connected and alive and still does not appear -- \
         `player.connected and player.character` is false for the 750 ticks \
         the crash-site cutscene runs, and only ever for player 1"
    );
}

/// **The fix.** Joining ends the cutscene, so the roster the script freezes on
/// its first poll is the whole roster.
#[test]
fn joining_the_game_ends_the_crash_site_cutscene() {
    let lua = mod_lua();
    lua.load(
        r#"
        game.players = { [1] = make_player(1, true), [2] = make_player(2, false) }
        on_player_joined_game({ player_index = 1 })
        rcon_players()
        "#,
    )
    .set_name("join")
    .exec()
    .expect("on_player_joined_game");

    assert_eq!(
        exit_cutscene_calls(&lua),
        vec![1],
        "the joining player is in a cutscene and has to be taken out of it"
    );
    assert_eq!(
        rcon_reply(&lua),
        "[1,2]",
        "and the point of doing so: the player is visible to the roster poll \
         immediately instead of 750 ticks later"
    );
}

/// The negative control, and it is a real hazard rather than a formality:
/// `LuaPlayer::exit_cutscene` **errors if not in a cutscene**, this crate
/// builds with `panic = "abort"`, and an unguarded call would fire on every
/// join of every bot on every run.
#[test]
fn a_player_who_is_not_in_a_cutscene_is_not_asked_to_leave_one() {
    let lua = mod_lua();
    lua.load(
        r#"
        game.players = { [1] = make_player(1, false), [2] = make_player(2, false) }
        on_player_joined_game({ player_index = 2 })
        "#,
    )
    .set_name("join")
    .exec()
    .expect("joining without a cutscene must not raise");

    assert_eq!(
        exit_cutscene_calls(&lua),
        Vec::<i64>::new(),
        "nobody was in a cutscene, so nobody may be taken out of one"
    );
}

/// The other half, and the one that removes the phantom ambiguity.
///
/// `set_disable_crashsite` has to be set **before** the first
/// `on_player_created`, and freeplay says so in place: *"This is so that other
/// mods and scripts have a chance to do remote calls before we do things like
/// charting the starting area, creating the crash site"*. `on_init` runs at
/// map creation; the first player is created when a client connects, tens of
/// seconds later.
#[test]
fn the_crash_site_is_disabled_before_any_player_exists() {
    let lua = mod_lua();
    lua.load("on_init()")
        .set_name("on_init")
        .exec()
        .expect("on_init");

    assert_eq!(
        remote_calls(&lua),
        vec![(
            "freeplay".to_string(),
            "set_disable_crashsite".to_string(),
            true
        )],
        "without this, freeplay takes player 1's eight iron plates into the \
         debris and leaves it at (0, 0) holding exactly what the invented \
         phantom bot holds -- indistinguishable in every record we keep"
    );
}

/// And it must not be load-bearing. A scenario with no freeplay interface --
/// or a Factorio that renamed it -- has to leave the mod working, since
/// `remote.call` on a missing interface raises and this crate aborts on panic.
#[test]
fn a_game_without_the_freeplay_interface_still_initialises() {
    let lua = mod_lua();
    lua.load("remote.interfaces = {} on_init()")
        .set_name("on_init")
        .exec()
        .expect("a game with no freeplay interface must still initialise");

    assert_eq!(
        remote_calls(&lua),
        Vec::new(),
        "nothing to call, and nothing called"
    );
    assert!(
        lua.globals()
            .get::<Table>("storage")
            .expect("storage")
            .contains_key("n_clients")
            .expect("n_clients"),
        "and the rest of on_init still ran"
    );
}
