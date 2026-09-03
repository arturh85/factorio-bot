//! **Does setting a recipe reach a verdict, and does it destroy anything on
//! the way?**
//!
//! Stage 2 of the starter factory is an assembling machine *with a recipe on
//! it*. Until this landed there was no `set_recipe` anywhere in the project,
//! so a machine the planner placed, powered and fed produced nothing at all —
//! the placed-but-dead failure the whole of 2026-09-02 went into eliminating
//! in its other forms.
//!
//! Two things about the game's own API make this worth a test file of its own,
//! and both were read out of `workspace/factorio-api-docs/runtime-api.json`
//! (Factorio 2.1.17) rather than assumed:
//!
//! * **`LuaEntity.set_recipe` does not return a success flag.** It returns an
//!   array of `ItemWithQualityCount` — "Any items removed from this entity as
//!   a result of setting the recipe". Discarding it destroys those items,
//!   which is the same defect `remove_item`, `create_entity` and
//!   `player.teleport` each cost this project once. So the verdict has to come
//!   from `get_recipe()` afterwards, and the return value has to be *moved*,
//!   not read.
//! * **`set_recipe` exists only on `AssemblingMachine`.** A stone furnace has
//!   `get_recipe` and no `set_recipe`, so calling it on one raises inside the
//!   remote call — a far worse answer than a sentence.
//!
//! These tests load the real `mods/BotBridge/control.lua` into a Lua 5.4 state
//! on top of a stub game, exactly as `botbridge_craft_action.rs` does.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick the stub game is frozen at.
const TICK: u64 = 61_204;

/// Enough of Factorio's Lua API for `control.lua` to load, plus capture of both
/// output channels: `rcon.print` is the RCON reply body the executor reads as
/// the action's result, `print` is the stdout `writeout` lands on.
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

    helpers = setmetatable(
        { table_to_json = function(t) return '{}' end },
        { __index = function() return noop end })

    storage = {}
"#;

/// See `botbridge_craft_action.rs`: the sandbox that must be used for *user*
/// scripts lives in a crate that depends on this one, and this interpreter only
/// ever runs two files out of this repository.
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

/// How the stub machine behaves when `set_recipe` is called on it.
enum Machine {
    /// An assembling machine that takes the recipe and evicts nothing.
    Assembler,
    /// An assembling machine that takes the recipe and evicts `count` of
    /// `item` — what the game does when the old recipe's ingredients no longer
    /// belong in it.
    Evicts { item: &'static str, count: u32 },
    /// An assembling machine that *ignores* the call. Nothing in the game's
    /// documented return value would reveal this: `set_recipe` answers with the
    /// evicted items either way, so only `get_recipe()` afterwards can tell.
    Deaf,
    /// A stone furnace. `set_recipe` is not defined on it at all, so touching
    /// it raises.
    Furnace,
}

/// One player on a force that knows `recipes` (with their `enabled` flags), and
/// one entity of `kind` standing at (12.5, 8.5).
///
/// `player_capacity` is how many items the player's inventory will accept, so a
/// test can drive the case where the evicted items have nowhere to go.
fn stub_game(recipes: &[(&str, bool)], kind: &Machine, player_capacity: u32) -> String {
    let known = recipes
        .iter()
        .map(|(name, enabled)| {
            format!(r#"["{name}"] = {{ name = "{name}", enabled = {enabled} }}"#)
        })
        .collect::<Vec<_>>()
        .join(",\n");
    // Every function here is **dot-called**, not colon-called, because that is
    // how Factorio's own API is used: `entity.set_recipe(recipe)`,
    // `surface.find_entity(name, pos)`, `player.insert{...}`. A stub taking a
    // leading `self` would silently receive the recipe as `self` and `nil` as
    // the name, and every assertion below would then be about the wrong call.
    let (entity_name, entity_type, set_recipe) = match kind {
        Machine::Assembler => (
            "assembling-machine-1",
            "assembling-machine",
            "function(name) _entity.recipe = name; return {} end".to_string(),
        ),
        Machine::Evicts { item, count } => (
            "assembling-machine-1",
            "assembling-machine",
            format!(
                r#"function(name) _entity.recipe = name
                    return {{ {{ name = "{item}", quality = "normal", count = {count} }} }} end"#
            ),
        ),
        Machine::Deaf => (
            "assembling-machine-1",
            "assembling-machine",
            "function(name) return {} end".to_string(),
        ),
        Machine::Furnace => ("stone-furnace", "furnace", "nil".to_string()),
    };
    format!(
        r#"
        _set_recipe_calls = {{}}
        _player_received = {{}}
        _player_room = {player_capacity}
        local force = {{
            recipes = {{ {known} }},
            technologies = {{}},
            add_research = function() return true end,
            print = noop,
        }}
        _entity = {{
            name = "{entity_name}",
            type = "{entity_type}",
            position = {{ x = 12.5, y = 8.5 }},
            recipe = nil,
        }}
        _entity.get_recipe = function()
            if _entity.recipe == nil then return nil end
            return {{ name = _entity.recipe }}
        end
        local raw_set = {set_recipe}
        if raw_set ~= nil then
            _entity.set_recipe = function(name)
                _set_recipe_calls[#_set_recipe_calls + 1] = name
                return raw_set(name)
            end
        end
        local surface = {{
            find_entity = function(name, pos)
                if _entity_missing then return nil end
                if name ~= _entity.name then return nil end
                return _entity
            end,
        }}
        local the_player = {{
            index = 1,
            name = "bot1",
            force = force,
            surface = surface,
            position = {{ x = 12.5, y = 10.5 }},
            print = noop,
        }}
        the_player.insert = function(stack)
            local taken = stack.count
            if taken > _player_room then taken = _player_room end
            _player_room = _player_room - taken
            _player_received[#_player_received + 1] = stack.name .. " x" .. taken
            return taken
        end
        game = {{
            tick = {TICK},
            players = {{ [1] = the_player }},
            forces = {{ player = force }},
            surfaces = {{}},
            connected_players = {{}},
        }}
        prototypes = {{ item = {{}}, entity = {{}}, recipe = {{}} }}
    "#
    )
}

/// Loads the stub game and the real mod, then runs `call`.
fn run(stub: &str, call: &str) -> Lua {
    let lua = lua_for_mod_source();
    lua.load(format!("{PRELUDE}{stub}"))
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
    lua.load(call)
        .set_name("call")
        .exec()
        .expect("handler call");
    lua
}

fn lines(lua: &Lua, global: &str) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>(global)
        .unwrap_or_else(|err| panic!("reading {global}: {err}"))
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect()
}

fn rcon_lines(lua: &Lua) -> Vec<String> {
    lines(lua, "_rcon_lines")
}

/// Every recipe name `LuaEntity.set_recipe` was actually called with.
fn set_recipe_calls(lua: &Lua) -> Vec<String> {
    lines(lua, "_set_recipe_calls")
}

/// Every stack the acting player was handed, as `name xN`.
fn player_received(lua: &Lua) -> Vec<String> {
    lines(lua, "_player_received")
}

/// The recipe the stub machine is left carrying.
fn machine_recipe(lua: &Lua) -> Option<String> {
    lua.load("return _entity.recipe").eval().expect("stub read")
}

fn call(player: u32, entity: &str, recipe: &str) -> String {
    format!(r#"rcon_set_recipe({player}, "{entity}", {{x = 12.5, y = 8.5}}, "{recipe}")"#)
}

/// The plain case. **The reply body *is* the action's result** to
/// `set_recipe_timed`, so a success that answered anything but its tick stamp
/// would be read as a refusal.
#[test]
fn setting_a_recipe_replies_with_only_a_tick_stamp() {
    let lua = run(
        &stub_game(
            &[("automation-science-pack", true)],
            &Machine::Assembler,
            100,
        ),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{TICK}")],
        "a recipe that was set answers with the tick and nothing else"
    );
    assert_eq!(
        machine_recipe(&lua).as_deref(),
        Some("automation-science-pack"),
        "and the machine is really carrying it"
    );
}

/// **A recipe the force has not unlocked is refused by name.**
///
/// `automation-science-pack` is `enabled = false` until its trigger technology
/// fires, and the gate is *force-scoped*: the flag lives on
/// `player.force.recipes[name]`, not on the prototype. A refusal that did not
/// name the recipe would be the confidently-incomplete diagnostic this project
/// has removed twice; setting nothing and reporting success would be worse
/// still, because the machine would then be dead and the plan would believe it
/// was finished.
#[test]
fn a_locked_recipe_is_refused_by_name_and_sets_nothing() {
    let lua = run(
        &stub_game(
            &[("automation-science-pack", false)],
            &Machine::Assembler,
            100,
        ),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal and no stamp, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: ")
            && reply[0].contains("automation-science-pack")
            && reply[0].contains("not enabled"),
        "the refusal names the recipe and why, got {reply:?}"
    );
    assert_eq!(
        set_recipe_calls(&lua),
        Vec::<String>::new(),
        "a locked recipe never reaches the game"
    );
    assert_eq!(machine_recipe(&lua), None);
}

/// A recipe the force has never heard of. Checked before the entity is even
/// looked up, and separately from the locked case: "you spelled it wrong" and
/// "research it first" send a reader to different places.
#[test]
fn an_unknown_recipe_is_refused_by_name() {
    let lua = run(
        &stub_game(&[("iron-gear-wheel", true)], &Machine::Assembler, 100),
        &call(1, "assembling-machine-1", "no-such-recipe"),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec!["Error: no such recipe: no-such-recipe".to_string()]
    );
    assert_eq!(set_recipe_calls(&lua), Vec::<String>::new());
}

/// A player the game does not have. Refused rather than indexed into, which
/// would raise inside the remote call.
#[test]
fn an_unknown_player_is_refused() {
    let lua = run(
        &stub_game(&[("iron-gear-wheel", true)], &Machine::Assembler, 100),
        &call(9, "assembling-machine-1", "iron-gear-wheel"),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec!["Error: no such player: 9".to_string()]
    );
}

/// Nothing standing where the plan believed a machine was. A durable fact about
/// the world, and the message names both the entity and the tile so the plan
/// that aimed there can be found.
#[test]
fn a_missing_machine_is_refused_and_names_the_tile() {
    let lua = run(
        &format!(
            "{}\n_entity_missing = true",
            stub_game(&[("iron-gear-wheel", true)], &Machine::Assembler, 100)
        ),
        &call(1, "assembling-machine-1", "iron-gear-wheel"),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal and no stamp, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: ")
            && reply[0].contains("assembling-machine-1")
            && reply[0].contains("12.5")
            && reply[0].contains("8.5"),
        "the refusal names the entity and where it was looked for, got {reply:?}"
    );
}

/// **A furnace is not an assembling machine**, and `set_recipe` is not defined
/// on one (`runtime-api.json` lists it under the `AssemblingMachine` subclass
/// only). Calling it anyway raises inside the remote call, which reaches the
/// executor as an unreadable reply rather than as a sentence — so the type is
/// checked first, and the refusal says what the thing actually is.
#[test]
fn a_machine_that_is_not_an_assembler_is_refused_by_type() {
    let lua = run(
        &stub_game(&[("iron-plate", true)], &Machine::Furnace, 100),
        &call(1, "stone-furnace", "iron-plate"),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal and no stamp, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: ")
            && reply[0].contains("stone-furnace")
            && reply[0].contains("furnace"),
        "the refusal names the entity and its type, got {reply:?}"
    );
}

/// **The return value is items, and they must not be destroyed.**
///
/// `set_recipe` answers with whatever it evicted from the machine. Dropping
/// that array deletes those items from the game with nothing anywhere saying
/// so — the exact shape of the three discarded return values this project has
/// already paid for. They go to the acting player, who is standing at the
/// machine to operate it.
#[test]
fn items_the_recipe_change_evicts_go_to_the_player() {
    let lua = run(
        &stub_game(
            &[("automation-science-pack", true)],
            &Machine::Evicts {
                item: "iron-gear-wheel",
                count: 7,
            },
            100,
        ),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{TICK}")],
        "the recipe was set, so this is a success"
    );
    assert_eq!(
        player_received(&lua),
        vec!["iron-gear-wheel x7".to_string()],
        "and the evicted items were handed to the bot, not deleted"
    );
}

/// The same eviction, with a bot who cannot carry it. Items that fit nowhere
/// are gone, and an action that silently lost seven gear wheels while
/// reporting success would leave the executor's model of that bot wrong for the
/// rest of the run.
///
/// Refusing is safe *because this verb is idempotent*: the recipe is already
/// set, so a retry evicts nothing and comes back clean. The message says so,
/// or a reader would think the recipe had not been set either.
#[test]
fn items_the_player_cannot_carry_are_reported_as_lost() {
    let lua = run(
        &stub_game(
            &[("automation-science-pack", true)],
            &Machine::Evicts {
                item: "iron-gear-wheel",
                count: 7,
            },
            2,
        ),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal and no stamp, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: ") && reply[0].contains("iron-gear-wheel"),
        "the refusal names what was lost, got {reply:?}"
    );
    assert!(
        reply[0].contains('5'),
        "and how many of them — 7 evicted, 2 carried, got {reply:?}"
    );
    assert!(
        reply[0].contains("automation-science-pack"),
        "and that the recipe itself was set, got {reply:?}"
    );
    assert_eq!(
        player_received(&lua),
        vec!["iron-gear-wheel x2".to_string()],
        "what did fit still went to the bot"
    );
}

/// **The verdict comes from `get_recipe()`, not from the return value.**
///
/// A machine that ignores `set_recipe` still answers it with an (empty) array,
/// so nothing in the documented return value distinguishes success from a
/// silent no-op. Only reading the recipe back does — and without that read this
/// verb would report success on exactly the placed-but-dead machine it exists
/// to prevent.
#[test]
fn a_recipe_that_did_not_take_is_refused() {
    let lua = run(
        &stub_game(&[("automation-science-pack", true)], &Machine::Deaf, 100),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal and no stamp, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: ") && reply[0].contains("automation-science-pack"),
        "the refusal names the recipe that did not take, got {reply:?}"
    );
    assert_eq!(
        set_recipe_calls(&lua),
        vec!["automation-science-pack".to_string()],
        "the game was asked; it just did not do it"
    );
}

/// **Idempotent, which is what makes it safe under `recover.rs`'s tier 1.**
///
/// `Place`, `Insert` and `Remove` are the counter-examples: re-running one
/// builds a second building or moves a second batch. Re-running this lands on
/// the same state, so a tier-1 retry of a `SetRecipe` costs a round trip and
/// nothing else.
#[test]
fn setting_the_same_recipe_twice_succeeds_twice() {
    let one = call(1, "assembling-machine-1", "automation-science-pack");
    let lua = run(
        &stub_game(
            &[("automation-science-pack", true)],
            &Machine::Assembler,
            100,
        ),
        &format!("{one}\n{one}"),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{TICK}"), format!("§tick§{TICK}")],
        "both dispatches succeed"
    );
    assert_eq!(
        machine_recipe(&lua).as_deref(),
        Some("automation-science-pack")
    );
}

/// A refusal is a line in the reply body and **nothing else**. `writeout` is
/// stdout and is not read as a verdict; a `rcon.print` used for narration would
/// turn a success into a reported failure, which is why this file checks the
/// reply body exactly rather than merely looking for a substring in it.
#[test]
fn a_success_writes_no_narration_into_the_reply() {
    let lua = run(
        &stub_game(
            &[("automation-science-pack", true)],
            &Machine::Assembler,
            100,
        ),
        &call(1, "assembling-machine-1", "automation-science-pack"),
    );
    let stdout = lines(&lua, "_printed");
    assert!(
        !stdout.iter().any(|l| l.contains("§tick§")),
        "the tick stamp belongs in the reply, not on stdout, got {stdout:?}"
    );
    assert_eq!(rcon_lines(&lua).len(), 1);
}
