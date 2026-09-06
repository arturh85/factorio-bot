//! **Does a blueprint's ghosts reach the planner's world model?**
//!
//! Until 2026-09-06 they did not, and the gap was invisible from both ends.
//!
//! `rcon_place_blueprint` returned its ghosts in the RCON *reply body*, which
//! the executor reads as the action's result and nothing else ever sees. The
//! world model is fed by `writeout` on stdout, and a script-driven
//! `build_blueprint` raises no `on_built_entity`, so `on_some_entity_created`
//! never fired for a ghost. Measured on a live 9-entity block: **9 ghosts
//! standing in the game, 0 visible to the planner.**
//!
//! The cost was a whole recovery path that could never fire.
//! `method::blueprint::recover_anchor_from_ghosts` is correct code with
//! passing unit tests — and those tests construct ghosts directly in
//! `PlanState`, which is precisely the step the live path never performs. **A
//! reader with nothing to read**, and a fixture supplying what reality does
//! not. The same shape as "a module with no caller is a hypothesis", one level
//! down.
//!
//! So this test exercises the real `control.lua` against a stub game and
//! asserts on the **writeout stream**, because that is the channel the model
//! actually listens to. Asserting on the reply body would have passed all
//! along.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!("../../../mods/BotBridge/control.lua");
const TYPES_LUA: &str = include_str!("../../../mods/BotBridge/types.lua");

const STUB_TICK: i64 = 4242;

/// Captures `print`, which is the only thing `writeout` does, so the test can
/// read the stream the Rust `output_parser` reads.
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

    -- The writeout stream. `writeout` is exactly one `print`, so capturing
    -- print IS capturing what the model is told.
    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end

    -- A real-enough encoder: the test needs to see the ghost's own name in
    -- the stream, which a fixed string would hide.
    helpers = setmetatable({
        table_to_json = function(t)
            if type(t) ~= "table" then return tostring(t) end
            local parts = {}
            for k, v in pairs(t) do
                if type(v) ~= "table" and type(v) ~= "function" then
                    parts[#parts + 1] = '"' .. tostring(k) .. '":"' .. tostring(v) .. '"'
                end
            end
            table.sort(parts)
            return "{" .. table.concat(parts, ",") .. "}"
        end,
    }, { __index = function() return noop end })

    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }
"#;

/// A blueprint build that leaves `n` ghosts standing, none revivable (the
/// bots hold nothing), which is the `only_ghosts` case and the one the
/// recovery path exists for.
fn stub_with_ghosts(n: usize) -> String {
    let ghosts: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{
                    valid = true,
                    name = "entity-ghost",
                    type = "entity-ghost",
                    ghost_name = "transport-belt",
                    ghost_type = "transport-belt",
                    direction = 0,
                    position = {{ x = {x}.5, y = 7.5 }},
                    bounding_box = {{
                        left_top = {{ x = {x}.1, y = 7.1 }},
                        right_bottom = {{ x = {x}.9, y = 7.9 }},
                    }},
                    get_output_inventory = function() return nil end,
                    get_fuel_inventory = function() return nil end,
                    revive = function() return false, nil end,
                }}"#,
                x = 10 + i
            )
        })
        .collect();
    format!(
        r#"
        storage = {{ p = {{ [1] = {{}} }} }}
        local bp_entity = {{
            stack = {{
                import_stack = function(bp) return 0 end,
                build_blueprint = function(args) return {{ {ghosts} }} end,
            }},
            destroy = function() end,
        }}
        local surface = {{ create_entity = function(args) return bp_entity end }}
        local player = {{
            name = "bot1",
            connected = true,
            character = {{}},
            position = {{ x = 0, y = 0 }},
            force = "player",
            surface = surface,
            get_main_inventory = function()
                return {{ get_contents = function() return {{}} end }}
            end,
        }}
        game = {{
            tick = {tick},
            players = {{ player }},
            forces = {{ player = {{ print = noop }} }},
        }}
        prototypes = {{ entity = setmetatable({{}}, {{ __index = function()
            return {{ collision_box = {{
                left_top = {{ x = -0.4, y = -0.4 }},
                right_bottom = {{ x = 0.4, y = 0.4 }},
            }} }}
        end }}) }}
    "#,
        ghosts = ghosts.join(", "),
        tick = STUB_TICK,
    )
}

fn run(stub: &str, call: &str) -> Lua {
    #[allow(clippy::disallowed_methods)]
    let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()).expect("lua");
    for (name, src) in [
        ("stub_game", format!("{PRELUDE}{stub}")),
        ("types.lua", TYPES_LUA.to_string()),
        ("control.lua", CONTROL_LUA.to_string()),
        ("call", call.to_string()),
    ] {
        lua.load(&src)
            .set_name(name)
            .exec()
            .unwrap_or_else(|err| panic!("loading {name}: {err}"));
    }
    lua
}

/// Everything `print` received, which is exactly the model's input stream.
fn printed(lua: &Lua) -> Vec<String> {
    lua.load("return _printed").eval().expect("_printed")
}

fn ghost_writeouts(lua: &Lua) -> Vec<String> {
    printed(lua)
        .into_iter()
        .filter(|line| line.contains("on_some_entity_created"))
        .collect()
}

/// The defect, stated as a test: every ghost a blueprint leaves standing is
/// announced on the stream the world model reads.
#[test]
fn every_ghost_a_blueprint_leaves_is_written_out() {
    let lua = run(
        &stub_with_ghosts(9),
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
    );

    let lines = ghost_writeouts(&lua);
    assert_eq!(
        lines.len(),
        9,
        "9 ghosts were built and the model must hear about all 9; got: {lines:#?}"
    );
}

/// The ghost's own identity has to survive, not just its existence.
///
/// `PlanState::ghosts_named_any` matches on `ghost_name` and explicitly never
/// on `name` — every ghost's `name` is the literal `entity-ghost` — so a
/// writeout carrying only `name` would arrive and still recover nothing.
#[test]
fn the_writeout_carries_ghost_name_and_not_only_entity_ghost() {
    let lua = run(
        &stub_with_ghosts(1),
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
    );

    let line = ghost_writeouts(&lua).pop().expect("one ghost writeout");
    assert!(
        line.contains("ghost_name") && line.contains("transport-belt"),
        "the model matches on ghost_name; without it recovery still sees nothing: {line}"
    );
    assert!(
        line.contains("entity-ghost"),
        "and `name` must stay `entity-ghost`, which is what marks it a ghost: {line}"
    );
}

/// It rides the tick, like every other writeout, because the model joins
/// everything on `game.tick`.
#[test]
fn the_writeout_carries_the_tick() {
    let lua = run(
        &stub_with_ghosts(1),
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
    );

    let line = ghost_writeouts(&lua).pop().expect("one ghost writeout");
    assert!(
        line.starts_with(&format!("§{STUB_TICK}§on_some_entity_created§")),
        "writeout format is §tick§key§value: {line}"
    );
}

/// The reply body must keep working: it is what the executor reads as this
/// action's result, and a change to the stream must not disturb it.
///
/// Asserted because the fix adds output next to a channel this repo has
/// already broken once by writing to the wrong one — a debug `rcon.print`
/// inside a handler turns a successful action into a reported failure.
#[test]
fn the_rcon_reply_still_carries_the_result() {
    let lua = run(
        &stub_with_ghosts(2),
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
    );

    let lines: Vec<String> = lua.load("return _rcon_lines").eval().expect("_rcon_lines");
    assert_eq!(lines.len(), 1, "exactly one reply body: {lines:#?}");
    // Not asserting the body's *shape*: this file's `table_to_json` is a stub
    // that cannot encode nested tables, so it renders the result array as
    // `{}`. What matters here is that the handler still reports success rather
    // than the failure string, and that adding a `writeout` did not turn one
    // channel into the other.
    assert_ne!(
        lines[0], "Error: failed to build anything",
        "two ghosts were built, so the reply must not be the failure string"
    );
}

/// A build that leaves nothing standing writes nothing, so an empty stream
/// means "no ghosts" and never "the writeout was skipped".
#[test]
fn a_blueprint_that_builds_nothing_writes_no_ghosts() {
    let lua = run(
        &stub_with_ghosts(0),
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
    );

    assert!(
        ghost_writeouts(&lua).is_empty(),
        "no ghosts were built, so nothing may be announced"
    );
}
