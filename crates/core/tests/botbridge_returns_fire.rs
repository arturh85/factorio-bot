//! **A character does not auto-fire, and nothing was pressing the trigger.**
//!
//! Measured live on 2026-09-08, seed 31337, four headless character bots
//! (`docs/superpowers/notes/2026-09-08-a-bot-that-shoots-back.md`): every bot
//! spawns holding a `pistol` in `defines.inventory.character_guns` and ten
//! `firearm-magazine` in `character_ammo` — the freeplay kit — and a **single
//! small-biter killed nine of them in a row** while its own health never moved
//! off 15 of 15. `shooting_state` defaults to `not_shooting` and stays there.
//!
//! Under `shooting_enemies` the same encounter ends in ~106 ticks with the bot
//! at full health. So the missing piece is one write per character, and
//! `defend_character_bot` is it.
//!
//! **What these tests are actually about is the GATE, not the shooting.** A
//! permanently-held order is worse than none near a nest: a bot holding the
//! button 10.7 tiles from a `biter-spawner` emptied all ten magazines into it,
//! failed to kill it, and woke it. So the order must go up only when a hostile
//! **unit** is close and come down again the moment none is — and that is what
//! a stub can check, because `find_enemy_units` answers with `type = "unit"`
//! only, which is exactly why it is the call the mod makes.
//!
//! What these tests cannot prove: that an ordered character actually hits
//! anything. That is the live measurement in the note, and no stub can stand in
//! for it.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game just big enough to load `control.lua` and call one function on
/// it. `defines` autovivifies, so `defines.shooting.shooting_enemies` is a
/// stable unique value and the two states can be told apart by identity —
/// which is all the mod does with them.
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
    print = noop
    rcon = { print = noop }
    helpers = { table_to_json = function() return "{}" end,
                write_file = noop, remove_path = noop }
    storage = {}
    game = { tick = 0, players = {}, forces = {}, surfaces = {} }
    prototypes = { item = {}, entity = {} }

    -- The character under test, plus a record of every question it asked the
    -- surface and every order it was given. `writes` is what makes "only on a
    -- change" checkable at all.
    _asked = {}
    _writes = {}
    function make_bot(enemies_near, initial_state)
        local e = {
            valid = true,
            force = "player",
            position = { x = 10.0, y = -4.0 },
        }
        e.surface = {
            find_enemy_units = function(center, radius, force)
                _asked[#_asked + 1] = { x = center.x, y = center.y,
                                        radius = radius, force = force }
                local out = {}
                for i = 1, enemies_near do out[i] = { name = "small-biter" } end
                return out
            end,
        }
        -- `shooting_state` is deliberately NOT a field of the table: it is a
        -- property in Factorio, and holding it in the table would let a plain
        -- assignment update the key in place without ever reaching
        -- `__newindex`, so the write log would stay empty and every test would
        -- pass by seeing nothing.
        local state = initial_state
        setmetatable(e, {
            __index = function(tbl, k)
                if k == "shooting_state" then return state end
                return nil
            end,
            __newindex = function(tbl, k, v)
                if k == "shooting_state" then
                    _writes[#_writes + 1] = v.state
                    state = v
                    return
                end
                rawset(tbl, k, v)
            end,
        })
        return e
    end
"#;

fn load_mod() -> Lua {
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

/// Run `body` and read back one boolean and the recorded write log.
fn drive(body: &str) -> (bool, Vec<String>) {
    let lua = load_mod();
    lua.load(body).set_name("case").exec().expect("case");
    let threatened: bool = lua.globals().get("_result").expect("_result");
    let writes: Vec<String> = lua
        .globals()
        .get::<mlua::Table>("_writes")
        .expect("_writes")
        .sequence_values::<mlua::Value>()
        .map(|v| {
            let v = v.expect("write");
            // `shooting_enemies` and `not_shooting` are distinct autovivified
            // tables; name them by comparing against `defines` itself rather
            // than by any property of the value.
            format!("{v:?}")
        })
        .collect();
    (threatened, writes)
}

/// Which order each write carried, resolved by identity against `defines`.
fn write_names(body: &str) -> Vec<String> {
    let lua = load_mod();
    lua.load(body).set_name("case").exec().expect("case");
    lua.load(
        r#"
        _names = {}
        for i, s in ipairs(_writes) do
            if s == defines.shooting.shooting_enemies then _names[i] = "shooting_enemies"
            elseif s == defines.shooting.not_shooting then _names[i] = "not_shooting"
            else _names[i] = "UNKNOWN" end
        end
    "#,
    )
    .set_name("names")
    .exec()
    .expect("names");
    lua.globals()
        .get::<mlua::Table>("_names")
        .expect("_names")
        .sequence_values::<String>()
        .map(|v| v.expect("name"))
        .collect()
}

#[test]
fn a_hostile_unit_within_the_radius_raises_the_fire_button() {
    let names = write_names(
        r#"
        local bot = make_bot(1, { state = defines.shooting.not_shooting })
        _result = defend_character_bot(bot)
    "#,
    );
    assert_eq!(
        names,
        vec!["shooting_enemies".to_string()],
        "a biter within DEFEND_RADIUS must put the order up; a character that \
         does not hold its fire button does not fire at all, which is how one \
         small-biter killed nine armed bots in a row"
    );
}

#[test]
fn no_hostile_unit_lowers_it_again() {
    let names = write_names(
        r#"
        local bot = make_bot(0, { state = defines.shooting.shooting_enemies })
        _result = defend_character_bot(bot)
    "#,
    );
    assert_eq!(
        names,
        vec!["not_shooting".to_string()],
        "the order must come DOWN when nothing is near. A held order empties \
         its magazines into a spawner it cannot kill and wakes the nest, which \
         is the failure this gate exists to prevent"
    );
}

/// The point of `find_enemy_units` over `find_entities_filtered`: it answers
/// with `type = "unit"` only, so a worm or a spawner cannot open the gate. The
/// stub cannot return a worm from it *by construction*, which is the property
/// being pinned — this test asserts the mod asks that question, at the
/// character's own position, with the documented radius and its own force.
#[test]
fn the_gate_asks_only_about_units_at_the_bots_own_position() {
    let lua = load_mod();
    lua.load(
        r#"
        local bot = make_bot(0, { state = defines.shooting.not_shooting })
        _result = defend_character_bot(bot)
        _q = _asked[1]
        _radius_matches = (_q.radius == DEFEND_RADIUS)
    "#,
    )
    .set_name("case")
    .exec()
    .expect("case");
    let g = lua.globals();
    let q: mlua::Table = g.get("_q").expect("the surface was asked");
    assert_eq!(
        q.get::<f64>("x").expect("x"),
        10.0,
        "the search must be centred on the character, not on anything else"
    );
    assert_eq!(q.get::<f64>("y").expect("y"), -4.0);
    assert_eq!(
        q.get::<String>("force").expect("force"),
        "player",
        "hostility is asked relative to the bot's OWN force, so this does not \
         assume the enemy force is called `enemy`"
    );
    assert!(
        g.get::<bool>("_radius_matches").expect("radius"),
        "the radius passed must be the documented DEFEND_RADIUS and not a \
         second copy of the number"
    );
}

#[test]
fn an_order_already_correct_is_not_rewritten() {
    let (threatened, writes) = drive(
        r#"
        local bot = make_bot(1, { state = defines.shooting.shooting_enemies })
        _result = defend_character_bot(bot)
    "#,
    );
    assert!(threatened, "a biter is near, so the bot is threatened");
    assert!(
        writes.is_empty(),
        "`shooting_state` is a setter that rebuilds the character's input \
         state; re-writing an order that is already correct costs that on \
         every bot on every sweep for nothing. Got {writes:?}"
    );
}

/// A respawned character is a **new entity** and comes back with the order
/// cleared — measured live: `before uid=1 sh=1` became `uid9 sh0`. Nothing in
/// the mod tracks that, and nothing needs to: the sweep reads the entity's
/// current state every time, so the fresh character is re-armed on its next
/// pass without any death having to be noticed.
#[test]
fn a_freshly_respawned_character_is_re_armed_by_the_next_sweep() {
    let names = write_names(
        r#"
        local respawned = make_bot(1, { state = defines.shooting.not_shooting })
        _result = defend_character_bot(respawned)
    "#,
    );
    assert_eq!(
        names,
        vec!["shooting_enemies".to_string()],
        "a bot that has died once must not be permanently defenceless -- it \
         already comes back with no gun and no ammo, and the order dying with \
         it would be the second half of the same hole"
    );
}

#[test]
fn a_dead_or_missing_character_is_not_ordered_about() {
    for fixture in [
        "local bot = make_bot(1, { state = defines.shooting.not_shooting })
         bot.valid = false
         _result = defend_character_bot(bot)",
        "_result = defend_character_bot(nil)",
    ] {
        let (threatened, writes) = drive(fixture);
        assert!(!threatened);
        assert!(
            writes.is_empty(),
            "writing to an invalid entity raises in Factorio, and the sweep \
             runs on every bot every DEFEND_PERIOD ticks -- including in the \
             window between a death and its respawn. Got {writes:?}"
        );
    }
}
