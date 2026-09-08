//! **A dead bot must not be reported as a bot that never connected.**
//!
//! `character_missing_reason` in `mods/BotBridge/control.lua` was written for
//! exactly this: Factorio keeps a dead player's `LuaPlayer` and drops its
//! character, so "no character" had been answering `not connected`, and
//! neither the record nor `just analyse` could say a bot had died at all. Its
//! own doc comment says so.
//!
//! **The fix landed on the paths that call `no_character_error` directly and
//! missed `get_player`**, which is the entry point most of the mod's verbs go
//! through -- and which asked `connected` *first*. For a character bot that is
//! the same fact twice: `CHARACTER_PROXY_OWN.connected` is
//! `entity ~= nil and entity.valid`, and a dead bot's `entity` is nil. So in
//! headless mode -- the mode this project iterates in -- the `character`
//! branch was unreachable and every dead bot answered `not connected`.
//!
//! Found in `run-1788833726-34821`, the first run in this project's history in
//! which a bot died. Three bots died. The two failures that went through
//! `get_player` were archived as
//!
//! ```text
//! game rejected the command: Unexpected Response: Error: player 1 not connected
//! game rejected the command: Unexpected Response: Error: player 2 not connected
//! ```
//!
//! which `classify_walk_failure` files as `WalkFailureKind::Other`. Those were
//! the walks that ended bot 1's and bot 2's slices, so the last thing the
//! record says about either bot is a connect stall -- a completely different
//! diagnosis -- over a bot the game had named the killer of one statement
//! earlier.
//!
//! This is the `occupant_of` mistake in another file: a cheap check ahead of
//! the specific one masks it, and the reader goes looking for whatever the
//! message named.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state over a stub
//! game, exactly as `botbridge_character_spawn.rs` does.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// Enough of Factorio for `control.lua` to load and for `rcon_spawn_bots` to
/// put one character bot in `storage.bots`.
const STUB: &str = r#"
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
    commands = nooptable()
    require = function() return {} end
    print = noop
    remote = setmetatable({ interfaces = {} }, { __index = function() return noop end })
    helpers = setmetatable(
        { table_to_json = function(t) return "<json>" end },
        { __index = function() return noop end })
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }

    _created = {}
    local surface = {
        find_non_colliding_position = function(name, center, radius, precision, tile_centre)
            return { x = center.x, y = center.y }
        end,
        create_entity = function(args)
            local e = {
                valid = true, name = "character", type = "character",
                unit_number = #_created + 1,
                position = { x = args.position.x, y = args.position.y },
                insert = noop,
            }
            _created[#_created + 1] = e
            return e
        end,
    }
    local force = {
        name = "player",
        get_spawn_position = function(s) return { x = 0.5, y = 0.5 } end,
    }
    game = {
        tick = 30000,
        connected_players = {},
        players = {},
        surfaces = { [1] = surface },
        forces = { player = force },
    }
    storage = { p = {} }
    prototypes = { item = {}, entity = {} }
"#;

const AFTER: &str = r#"
    announce_character_bot = function() end
    on_player_changed_distance = function() end
"#;

/// One character bot, spawned and alive.
fn one_bot() -> Lua {
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
    lua.load(STUB).set_name("stub_game").exec().expect("stub");
    lua.load(TYPES_LUA)
        .set_name("types.lua")
        .exec()
        .expect("mod types.lua");
    lua.load(CONTROL_LUA)
        .set_name("control.lua")
        .exec()
        .expect("mod control.lua");
    lua.load(AFTER).set_name("after").exec().expect("overrides");
    lua.load("rcon_spawn_bots(1)")
        .set_name("spawn")
        .exec()
        .expect("rcon_spawn_bots");
    lua
}

/// Everything `rcon.print` was handed, joined. The mod's refusals are single
/// lines, so this is the whole reply body the executor would read.
fn replies(lua: &Lua) -> String {
    lua.load("return table.concat(_rcon_lines, ' | ')")
        .eval::<String>()
        .expect("_rcon_lines")
}

fn clear_replies(lua: &Lua) {
    lua.load("_rcon_lines = {}").exec().expect("clear");
}

/// The precondition every other test here rests on: the stub really did
/// produce a live character bot, and `get_player` hands it back.
///
/// Without this, a test asserting "no `not connected`" would pass against a
/// mod that refused everything for some unrelated reason.
#[test]
fn a_living_character_bot_is_handed_back() {
    let lua = one_bot();
    // `rcon_spawn_bots` answers with its own roster JSON; only what
    // `get_player` says is under test here.
    clear_replies(&lua);
    let got: bool = lua
        .load("return get_player(1) ~= nil")
        .eval()
        .expect("get_player");
    assert!(got, "a live bot resolves; replies were: {}", replies(&lua));
    assert_eq!(replies(&lua), "", "a live bot is refused for nothing");
}

/// **The regression.** Kill the character the way the game does -- the entity
/// goes, `respawn_at` is set -- and ask `get_player` why there is no handle.
#[test]
fn a_dead_character_bot_is_refused_as_dead_and_not_as_disconnected() {
    let lua = one_bot();
    lua.load("storage.bots[1].entity = nil; storage.bots[1].respawn_at = game.tick + 600")
        .exec()
        .expect("kill the bot");
    clear_replies(&lua);

    let got: bool = lua
        .load("return get_player(1) ~= nil")
        .eval()
        .expect("get_player");
    assert!(!got, "a dead bot has no handle to hand back");

    let said = replies(&lua);
    assert!(
        said.contains("has no character"),
        "the substring `classify_failure` keys on is missing from {said:?}"
    );
    assert!(
        said.contains("respawns in 600 ticks"),
        "the respawn timer `classify_failure` reads out is missing from {said:?}"
    );
    assert!(
        !said.contains("not connected"),
        "a dead bot reported as a connect stall -- the whole defect -- in {said:?}"
    );
}

/// The other half, and the reason this is a reorder rather than a deletion: a
/// **connected client** that merely left keeps its character (Factorio does
/// not remove it on disconnect), so it falls past the `character` branch and
/// is still refused as `not connected`.
///
/// Without this the fix would be untestably one-sided -- any reordering
/// satisfies the test above, including deleting the `connected` check
/// outright, which would then report a genuinely absent client as some
/// controller confusion.
#[test]
fn a_disconnected_client_that_still_has_a_character_is_refused_as_disconnected() {
    let lua = one_bot();
    // A player, not a character bot: `bot_handle` returns `game.players[id]`
    // for an id `storage.bots` does not hold.
    lua.load(
        "storage.p[7] = {}
         game.players[7] = { connected = false, character = { valid = true }, index = 7 }",
    )
    .exec()
    .expect("an offline client");
    clear_replies(&lua);

    let got: bool = lua
        .load("return get_player(7) ~= nil")
        .eval()
        .expect("get_player");
    assert!(!got, "an offline client is still refused");

    let said = replies(&lua);
    assert!(
        said.contains("not connected"),
        "an offline client with a character is exactly the `not connected` case, got {said:?}"
    );
    assert!(
        !said.contains("has no character"),
        "it has one; saying otherwise trades one wrong diagnosis for another: {said:?}"
    );
}

/// A **dead client** -- connected, character gone -- was already handled
/// correctly, and must stay that way. This is the case the ordering used to be
/// right for, and the one that made the defect invisible to anyone testing
/// with graphical clients rather than headless character bots.
#[test]
fn a_dead_client_is_refused_as_dead() {
    let lua = one_bot();
    lua.load(
        "storage.p[7] = {}
         game.players[7] = { connected = true, character = nil, index = 7,
                             ticks_to_respawn = 412 }",
    )
    .exec()
    .expect("a dead client");
    clear_replies(&lua);

    let got: bool = lua
        .load("return get_player(7) ~= nil")
        .eval()
        .expect("get_player");
    assert!(!got);

    let said = replies(&lua);
    assert!(
        said.contains("has no character") && said.contains("respawns in 412 ticks"),
        "a dead client's refusal lost its reason: {said:?}"
    );
}
