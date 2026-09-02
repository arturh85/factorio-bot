//! **Does a research action wait for the research?**
//!
//! `LuaForce.add_research` is documented (`workspace/factorio-api-docs/runtime-api.json`,
//! Factorio 2.1.17) as adding a technology "to the back of the research queue"
//! and returning "whether the technology was successfully added". It says
//! nothing about the technology being finished — that arrives ticks or minutes
//! later as `on_research_finished`.
//!
//! The mod used to answer the RCON call the moment the technology was queued
//! and `on_research_finished` carried no action id, so nothing could join a
//! completion back to the action that asked for it. The executor therefore
//! reported every research a success the instant it was requested, and any
//! step depending on the technology ran against a belief nobody established.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state on top of a
//! stub game, start a research action, and then raise `on_research_finished`
//! at the handler — because the join between the two is Lua that only ever
//! runs inside Factorio, and no reply body reveals it.
//!
//! What this cannot prove: that Factorio raises `on_research_finished` with
//! the shape assumed here, or that a queued technology always finishes. See
//! `docs/superpowers/notes/2026-09-02-awaited-research.md`.

use mlua::{Lua, LuaOptions, StdLib};

// The same two files a debug run loads out of the checkout — see
// `botbridge_placement_material.rs` for why the path is spelled this way.
const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick the stub game is frozen at when the action starts.
const START_TICK: u64 = 64738;
/// The tick `on_research_finished` is raised at. Deliberately far from
/// [`START_TICK`], so a completion stamped with the dispatch tick — the thing
/// the old code effectively claimed — is visible at a glance.
const FINISH_TICK: u64 = 91230;

/// Enough of Factorio's Lua API for `control.lua` to load, plus capture of
/// both output channels: `rcon.print` is the RCON reply body the executor
/// reads as the action's result, `print` is the stdout `writeout` lands on.
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

    -- Everything the mod writes to stdout, in order. `writeout` calls the
    -- global `print`, so this is the same channel `OutputParser` reads.
    _printed = {}
    print = function(s) _printed[#_printed + 1] = tostring(s) end

    -- The RCON reply body under construction.
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }

    helpers = setmetatable(
        { table_to_json = function(t) return '{}' end },
        { __index = function() return noop end })

    -- `storage` is Factorio's per-save table. The mod keeps the research
    -- registry there rather than in a module local, and these tests read it
    -- directly to prove that.
    storage = {}
"#;

/// The bulk world writeouts `on_research_finished` fires before it settles
/// anything. Stubbed out, and recorded, so the ordering test can see them.
const STUB_WORLD_WRITEOUTS: &str = r#"
    writeout_recipes = function() _printed[#_printed + 1] = "recipes" end
    writeout_forces = function() _printed[#_printed + 1] = "forces" end
    on_player_changed_distance = function(e) _printed[#_printed + 1] = "distance" end
"#;

/// See `botbridge_placement_material.rs`: the sandbox that must be used for
/// *user* scripts lives in a crate that depends on this one, and this
/// interpreter only ever runs two files out of this repository.
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

/// A stub force whose technologies are `known`, with `add_research` answering
/// `adds` and recording what it was asked for.
fn stub_game(known: &[&str], adds: bool) -> String {
    let technologies = known
        .iter()
        .map(|name| {
            format!(
                r#"["{name}"] = {{ name = "{name}", researched = false, enabled = true,
                    prerequisites = {{}},
                    prototype = {{ research_trigger = nil }} }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        r#"
        _queued = {{}}
        local force = {{
            technologies = {{ {technologies} }},
            add_research = function(name)
                _queued[#_queued + 1] = name
                return {adds}
            end,
            print = noop,
        }}
        game = {{
            tick = {START_TICK},
            players = {{}},
            forces = {{ player = force }},
            surfaces = {{}},
        }}
        prototypes = {{ item = {{}}, entity = {{}} }}
    "#,
        adds = if adds { "true" } else { "false" },
    )
}

/// Loads the stub game, the real mod, the world-writeout stubs, and then runs
/// `call`.
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
    lua.load(STUB_WORLD_WRITEOUTS)
        .set_name("after")
        .exec()
        .expect("post-load overrides");
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

fn stdout(lua: &Lua) -> Vec<String> {
    lines(lua, "_printed")
}

fn rcon_lines(lua: &Lua) -> Vec<String> {
    lines(lua, "_rcon_lines")
}

/// Every `action_completed` line the mod wrote, in order.
fn completions(lua: &Lua) -> Vec<String> {
    stdout(lua)
        .into_iter()
        .filter(|line| line.contains("§action_completed§"))
        .collect()
}

/// Raise `on_research_finished` for `tech` at [`FINISH_TICK`].
fn finish(tech: &str) -> String {
    format!(
        r#"on_research_finished({{ tick = {FINISH_TICK},
            research = {{ name = "{tech}" }} }})"#
    )
}

const START_AUTOMATION: &str = r#"rcon_action_start_research(42, "automation")"#;

/// **The dispatch reply says only "the game took this", never "it is done".**
///
/// The reply body *is* the action's result to `research_timed`, so a research
/// that answered anything else here would be read as a refusal. One tick stamp
/// and nothing more is the whole contract.
#[test]
fn starting_a_research_action_replies_with_only_a_tick_stamp() {
    let lua = run(&stub_game(&["automation"], true), START_AUTOMATION);
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{START_TICK}")],
        "the dispatch reply carries the tick and nothing else"
    );
    assert_eq!(
        completions(&lua),
        Vec::<String>::new(),
        "the technology is queued, not researched; nothing may be settled yet"
    );
}

/// **The bug, stated as a test: the completion has to reach the action.**
///
/// `on_research_finished` is the game's own signal that the technology is
/// finished. Until it carried the action id through, the executor had no way
/// to learn a research had completed and reported success at queue time
/// instead.
#[test]
fn a_finished_research_completes_the_action_that_asked_for_it() {
    let lua = run(
        &stub_game(&["automation"], true),
        &format!("{START_AUTOMATION}\n{}", finish("automation")),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{FINISH_TICK}§action_completed§ok 42")],
        "the action settles on the game's signal, stamped with the tick the \
         research actually finished at"
    );
}

/// The registry has to outlive a save/load, which a module local does not:
/// `on_load` rebuilds nothing, so a research that spans a save would never
/// settle. `storage` is the only table Factorio persists. Crafting made the
/// same mistake and was moved to `storage.craft_actions` for the same reason —
/// see `botbridge_craft_action.rs`.
#[test]
fn the_registry_lives_in_storage() {
    let lua = run(&stub_game(&["automation"], true), START_AUTOMATION);
    let waiting: mlua::Table = lua
        .load("return storage.research_actions[\"automation\"]")
        .eval()
        .expect("storage.research_actions is where the waiting ids live");
    assert_eq!(
        waiting.sequence_values::<u32>().collect::<Vec<_>>().len(),
        1,
        "the one action that asked for automation is waiting on it"
    );
}

/// The join is by technology name because that is the only thing
/// `on_research_finished` carries. A different technology finishing — the
/// normal case, since `add_research` appends to a queue that may already hold
/// others — must settle nothing, and must not consume the entry.
#[test]
fn another_technology_finishing_settles_nothing() {
    let lua = run(
        &stub_game(&["automation", "logistics"], true),
        &format!(
            "{START_AUTOMATION}\n{}\n{}",
            finish("logistics"),
            finish("automation")
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{FINISH_TICK}§action_completed§ok 42")],
        "logistics finishing is not automation finishing, and must not have \
         consumed the action waiting for automation"
    );
}

/// Two actions asking for the same technology both have to settle. A registry
/// holding one id per technology would silently drop the first, and it would
/// then cost the executor its whole `ACTION_RESULT_DEADLINE` before anyone
/// noticed.
#[test]
fn two_actions_waiting_on_one_technology_both_settle() {
    let lua = run(
        &stub_game(&["automation"], true),
        &format!(
            "{START_AUTOMATION}\nrcon_action_start_research(43, \"automation\")\n{}",
            finish("automation")
        ),
    );
    let settled = completions(&lua);
    assert_eq!(settled.len(), 2, "both actions settle, got {settled:?}");
    assert!(
        settled.contains(&format!("§{FINISH_TICK}§action_completed§ok 42")),
        "the first action settles, got {settled:?}"
    );
    assert!(
        settled.contains(&format!("§{FINISH_TICK}§action_completed§ok 43")),
        "the second action settles, got {settled:?}"
    );
}

/// A refusal is answered in the reply body and is the end of it. Leaving the
/// id registered would settle a later, unrelated research as this action's
/// success — the exact overclaim this change exists to remove.
#[test]
fn a_refused_research_leaves_nothing_waiting() {
    let lua = run(
        &stub_game(&["automation"], false),
        &format!("{START_AUTOMATION}\n{}", finish("automation")),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(reply.len(), 1, "one refusal line, got {reply:?}");
    assert!(
        reply[0].starts_with("Error: cannot research automation"),
        "the refusal still says which of the reasons it was, got {reply:?}"
    );
    assert_eq!(
        completions(&lua),
        Vec::<String>::new(),
        "a refused action must never be settled by somebody else's research"
    );
}

/// An unknown name is refused before anything is queued or registered.
#[test]
fn an_unknown_technology_is_refused_and_registers_nothing() {
    let lua = run(
        &stub_game(&["automation"], true),
        r#"rcon_action_start_research(42, "no-such-tech")"#,
    );
    assert_eq!(
        rcon_lines(&lua),
        vec!["Error: no such technology: no-such-tech".to_string()]
    );
    let queued: mlua::Table = lua.load("return _queued").eval().expect("_queued");
    assert_eq!(
        queued.sequence_values::<String>().count(),
        0,
        "an unknown name never reaches add_research"
    );
    let waiting: mlua::Value = lua
        .load("return storage.research_actions")
        .eval()
        .expect("storage read");
    assert!(
        matches!(waiting, mlua::Value::Nil)
            || lua
                .load("return next(storage.research_actions) == nil")
                .eval::<bool>()
                .expect("emptiness"),
        "nothing is registered for a technology that does not exist"
    );
}

/// **The completion is written last, after the recipes and forces the research
/// unlocked.**
///
/// Stdout is ordered, so the executor learns the new recipes *before* it is
/// told the action succeeded. The other order would let a plan step react to a
/// technology the Rust-side world does not know about yet.
#[test]
fn the_completion_follows_the_world_data_it_unlocked() {
    let lua = run(
        &stub_game(&["automation"], true),
        &format!("{START_AUTOMATION}\n{}", finish("automation")),
    );
    let out = stdout(&lua);
    let recipes = out.iter().position(|l| l == "recipes").expect("recipes");
    let forces = out.iter().position(|l| l == "forces").expect("forces");
    let settled = out
        .iter()
        .position(|l| l.contains("§action_completed§"))
        .expect("a completion");
    assert!(
        recipes < settled && forces < settled,
        "the unlocked world data has to be on the wire before the success \
         that depends on it, got {out:?}"
    );
}

/// The unawaited entry point is still there for the Lua binding and the REST
/// endpoint, and it registers nothing: nobody is waiting, so nothing may be
/// settled on its behalf.
#[test]
fn the_unawaited_entry_point_registers_no_action() {
    let lua = run(
        &stub_game(&["automation"], true),
        &format!(
            "rcon_add_research(\"automation\")\n{}",
            finish("automation")
        ),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{START_TICK}")],
        "queueing still answers with just the tick"
    );
    assert_eq!(
        completions(&lua),
        Vec::<String>::new(),
        "no action id was given, so no action can be completed"
    );
}
