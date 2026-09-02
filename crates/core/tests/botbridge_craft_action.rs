//! **Does a craft action reach a verdict?**
//!
//! Run `run-1788347034-00981` dispatched 195 actions and settled 184. All
//! eleven that never reached a verdict were `craft`, and
//! `workspace/runs/run-1788347034-00981/samples.jsonl` shows every one of them
//! *physically happening*: bot 1's stone count fell and a `stone-furnace`
//! appeared 43 ticks after the dispatch at tick 50177, and all four bots'
//! `automation-science-pack` counts rose to exactly the requested numbers after
//! the dispatch at 71783. The game did the work; the mod did not join it back
//! to the action, so each cost the executor a full `ACTION_RESULT_DEADLINE` —
//! 360 wall-clock seconds — before being written off.
//!
//! The join used to be a *positional* match against a module-local list:
//! `queue[1].recipe == event.recipe.name`, popping the head. Two properties of
//! that shape are what these tests exist to remove.
//!
//!  * **It head-of-line blocks, permanently.** A crafted item that does not
//!    match the head is ignored and the head stays. So one entry that will
//!    never be crafted stops *every later craft for that player, forever* — no
//!    timeout, no error, no log line. Every route into that state is silent.
//!  * **Nothing ever removed an entry except a matching craft.** A
//!    `begin_crafting` that started fewer crafts than asked left the surplus
//!    entries behind (and the action was refused, so nobody was even waiting on
//!    them), and a cancelled craft left all of them behind.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state on top of a
//! stub game and raise the crafting events at the handlers, because the join
//! between the two is Lua that only ever runs inside Factorio and no reply body
//! reveals it. They follow `botbridge_research_action.rs`, which fixed the same
//! defect for research.
//!
//! What this cannot prove: that Factorio raises `on_player_crafted_item` once
//! per craft with the shape assumed here, or which of the desync routes above
//! the recorded run actually took — the run's mod stdout was not retained. See
//! `docs/superpowers/notes/2026-09-02-crafts-that-never-report.md`.

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

/// The tick the stub game is frozen at when an action starts.
const START_TICK: u64 = 50177;
/// The tick the crafting events are raised at. Deliberately far from
/// [`START_TICK`] so a completion stamped with the dispatch tick is visible at
/// a glance. It is the tick the run's stalled stone furnace actually appeared.
const CRAFT_TICK: u64 = 50220;

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

/// Two players on a force that knows `recipes`, whose `begin_crafting` starts
/// `started` crafts however many were asked for, and records every request.
///
/// `started` is the whole point of one of these tests: `LuaControl.begin_crafting`
/// is documented (`workspace/factorio-api-docs/runtime-api.json`, Factorio
/// 2.1.17) as returning "the count that was actually started crafting", which
/// is not always the count asked for.
fn stub_game(recipes: &[&str], started: Option<u32>) -> String {
    let known = recipes
        .iter()
        .map(|name| format!(r#"["{name}"] = {{ name = "{name}", enabled = true }}"#))
        .collect::<Vec<_>>()
        .join(",\n");
    let started = match started {
        Some(n) => n.to_string(),
        None => "spec.count".to_string(),
    };
    format!(
        r#"
        _started = {{}}
        local force = {{
            recipes = {{ {known} }},
            technologies = {{}},
            add_research = function() return true end,
            print = noop,
        }}
        local function player(idx)
            return {{
                index = idx,
                name = "bot" .. idx,
                force = force,
                begin_crafting = function(spec)
                    _started[#_started + 1] = spec.recipe .. " x" .. spec.count
                    return {started}
                end,
                print = noop,
            }}
        end
        game = {{
            tick = {START_TICK},
            players = {{ [1] = player(1), [2] = player(2) }},
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

/// Every `action_completed` line the mod wrote, in order. Both verdicts travel
/// on this key — `ok <id>` and `fail <id> <reason>`.
fn completions(lua: &Lua) -> Vec<String> {
    lines(lua, "_printed")
        .into_iter()
        .filter(|line| line.contains("§action_completed§"))
        .collect()
}

/// What `begin_crafting` was asked for, in order.
fn started(lua: &Lua) -> Vec<String> {
    lines(lua, "_started")
}

/// Raise `on_player_crafted_item` for one craft of `recipe` by `player`.
fn crafted(player: u32, recipe: &str) -> String {
    format!(
        r#"on_player_crafted_item({{ tick = {CRAFT_TICK}, player_index = {player},
            recipe = {{ name = "{recipe}", products = {{}} }},
            item_stack = {{ name = "{recipe}", count = 1 }} }})"#
    )
}

/// Raise `on_player_cancelled_crafting` for `count` cancelled crafts of
/// `recipe` by `player`.
fn cancelled(player: u32, recipe: &str, count: u32) -> String {
    format!(
        r#"on_player_cancelled_crafting({{ tick = {CRAFT_TICK}, player_index = {player},
            recipe = {{ name = "{recipe}", products = {{}} }},
            cancel_count = {count}, items = {{}} }})"#
    )
}

fn start(action_id: u32, player: u32, recipe: &str, count: u32) -> String {
    format!(r#"rcon_action_start_crafting({action_id}, {player}, "{recipe}", {count})"#)
}

/// **The dispatch reply says only "the game took this", never "it is done".**
///
/// The reply body *is* the action's result to `player_craft_timed`, so a craft
/// that answered anything else here would be read as a refusal.
#[test]
fn starting_a_craft_replies_with_only_a_tick_stamp() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &start(7, 1, "stone-furnace", 1),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec![format!("§tick§{START_TICK}")],
        "the dispatch reply carries the tick and nothing else"
    );
    assert_eq!(
        completions(&lua),
        Vec::<String>::new(),
        "the craft is queued, not finished; nothing may be settled yet"
    );
}

/// The plain case, and the one the run failed at: one craft, one crafted item,
/// one verdict, stamped with the tick the game finished it at.
#[test]
fn a_finished_craft_completes_the_action_that_asked_for_it() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &format!(
            "{}\n{}",
            start(7, 1, "stone-furnace", 1),
            crafted(1, "stone-furnace")
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 7")],
        "the action settles on the game's own signal"
    );
}

/// A count of N settles once, on the Nth craft — not on the first, and not N
/// times.
#[test]
fn a_multi_craft_settles_once_on_the_last_one() {
    let lua = run(
        &stub_game(&["automation-science-pack"], None),
        &format!(
            "{}\n{}\n{}\n{}",
            start(0, 1, "automation-science-pack", 3),
            crafted(1, "automation-science-pack"),
            crafted(1, "automation-science-pack"),
            crafted(1, "automation-science-pack"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 0")],
        "three crafts were asked for and three happened: exactly one verdict"
    );
}

/// **The head-of-line test, and the whole reason this file exists.**
///
/// One craft that never happens must not silence the next one. Under the
/// positional match this was the permanent failure: a `lab` entry sitting at
/// the head made every later `stone-furnace` event a non-match, so it was
/// ignored and the head never moved — every subsequent craft for that bot cost
/// a full `ACTION_RESULT_DEADLINE` and never reached a verdict.
#[test]
fn a_craft_that_never_finishes_does_not_block_the_next_one() {
    let lua = run(
        &stub_game(&["lab", "stone-furnace"], None),
        &format!(
            "{}\n{}\n{}",
            start(18, 1, "lab", 1),
            start(7, 1, "stone-furnace", 1),
            crafted(1, "stone-furnace"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 7")],
        "the stone furnace the game actually made settles its own action, \
         whatever is still outstanding for another recipe"
    );
}

/// An intermediate the game crafts on its own way to the requested item is not
/// the requested item. `begin_crafting` queues intermediates when ingredients
/// are missing, and each raises `on_player_crafted_item` under its own recipe
/// name.
#[test]
fn an_intermediate_craft_settles_nothing() {
    let lua = run(
        &stub_game(&["lab", "iron-gear-wheel"], None),
        &format!(
            "{}\n{}\n{}",
            start(18, 1, "lab", 1),
            crafted(1, "iron-gear-wheel"),
            crafted(1, "lab"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 18")],
        "the gear wheel is on the way to the lab, not the lab"
    );
}

/// The join is `(player_index, recipe.name)` because that is all
/// `on_player_crafted_item` carries. One bot crafting must not settle another
/// bot's action for the same recipe.
#[test]
fn a_craft_settles_only_the_player_that_asked_for_it() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &format!(
            "{}\n{}\n{}",
            start(7, 1, "stone-furnace", 1),
            start(8, 2, "stone-furnace", 1),
            crafted(2, "stone-furnace"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 8")],
        "bot 2's furnace is bot 2's, and bot 1 is still waiting"
    );
}

/// Two actions asking the same player for the same recipe both settle, in the
/// order they were asked for — the game's crafting queue is FIFO, so the
/// earlier request's crafts come out first.
#[test]
fn two_actions_on_one_recipe_both_settle_in_order() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &format!(
            "{}\n{}\n{}\n{}\n{}",
            start(7, 1, "stone-furnace", 2),
            start(9, 1, "stone-furnace", 1),
            crafted(1, "stone-furnace"),
            crafted(1, "stone-furnace"),
            crafted(1, "stone-furnace"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![
            format!("§{CRAFT_TICK}§action_completed§ok 7"),
            format!("§{CRAFT_TICK}§action_completed§ok 9"),
        ],
        "the two-craft request settles on its second craft, the one-craft \
         request on the third"
    );
}

/// The registry has to outlive a save/load, which a module local does not:
/// `on_load` rebuilds nothing, so a craft that spans a save would never settle.
/// `storage` is the only table Factorio persists.
#[test]
fn the_registry_lives_in_storage() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &start(7, 1, "stone-furnace", 1),
    );
    let waiting: u32 = lua
        .load("return storage.craft_actions[1][\"stone-furnace\"][1].id")
        .eval()
        .expect("storage.craft_actions is where the waiting ids live");
    assert_eq!(
        waiting, 7,
        "the action that asked is registered under storage"
    );
}

/// **A craft the game will not start must fail at once**, not six minutes
/// later. `begin_crafting` returning 0 is the game's refusal, and the reply
/// body is where `player_craft_timed` reads a refusal.
#[test]
fn a_craft_the_game_will_not_start_is_refused_in_the_reply() {
    let lua = run(
        &stub_game(&["stone-furnace"], Some(0)),
        &start(7, 1, "stone-furnace", 1),
    );
    let reply = rcon_lines(&lua);
    assert_eq!(
        reply.len(),
        1,
        "one refusal line and no tick stamp, got {reply:?}"
    );
    assert!(
        reply[0].starts_with("Error: ")
            && reply[0].contains("stone-furnace")
            && reply[0].contains("started 0"),
        "the refusal says what the game actually started, got {reply:?}"
    );
}

/// **A partial start registers nothing.**
///
/// This is the desync route that used to be silent: `begin_crafting` starting
/// fewer crafts than asked complained (so the action was refused and nobody was
/// waiting) *and then pushed all `count` entries onto the list anyway*. The
/// surplus never drained and blocked every later craft for that bot.
#[test]
fn a_partially_started_craft_registers_nothing() {
    let lua = run(
        &stub_game(&["stone-furnace"], Some(1)),
        &format!(
            "{}\n{}\n{}\n{}",
            start(7, 1, "stone-furnace", 3),
            start(9, 1, "stone-furnace", 1),
            crafted(1, "stone-furnace"),
            crafted(1, "stone-furnace"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 9")],
        "the refused request left nothing behind, so the next one settles on \
         its own craft"
    );
    let empty: bool = lua
        .load("return storage.craft_actions[1] == nil or next(storage.craft_actions[1]) == nil")
        .eval()
        .expect("storage read");
    assert!(empty, "no waiter outlives the crafts it was counting");
}

/// A recipe the force does not have is refused before `begin_crafting` is
/// reached — the call raises on an unknown recipe, and a raise inside the
/// remote call is a far worse answer than a sentence.
#[test]
fn an_unknown_recipe_is_refused_and_starts_nothing() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &start(7, 1, "no-such-recipe", 1),
    );
    assert_eq!(
        rcon_lines(&lua),
        vec!["Error: no such recipe: no-such-recipe".to_string()]
    );
    assert_eq!(
        started(&lua),
        Vec::<String>::new(),
        "an unknown name never reaches begin_crafting"
    );
}

/// **A cancelled craft settles, and it settles as a failure.**
///
/// `on_player_cancelled_crafting` is the game saying those crafts will not
/// happen. Before this it was unhandled: the entries stayed, the action waited
/// out the deadline, and the stale entries blocked everything after it.
#[test]
fn a_cancelled_craft_fails_the_action_at_once() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &format!(
            "{}\n{}",
            start(7, 1, "stone-furnace", 1),
            cancelled(1, "stone-furnace", 1),
        ),
    );
    let settled = completions(&lua);
    assert_eq!(settled.len(), 1, "one verdict, got {settled:?}");
    assert!(
        settled[0].starts_with(&format!("§{CRAFT_TICK}§action_completed§fail 7 ")),
        "a cancellation is a failure, not a success, got {settled:?}"
    );
    assert!(
        settled[0].contains("cancel"),
        "and it says the game cancelled it, got {settled:?}"
    );
}

/// A craft that was half done when the rest was cancelled did not deliver what
/// was asked for. Reporting success on the partial yield would tell the
/// executor a bot holds items it does not have.
#[test]
fn a_partially_completed_craft_that_is_then_cancelled_fails() {
    let lua = run(
        &stub_game(&["automation-science-pack"], None),
        &format!(
            "{}\n{}\n{}",
            start(0, 1, "automation-science-pack", 3),
            crafted(1, "automation-science-pack"),
            cancelled(1, "automation-science-pack", 2),
        ),
    );
    let settled = completions(&lua);
    assert_eq!(settled.len(), 1, "one verdict, got {settled:?}");
    assert!(
        settled[0].starts_with(&format!("§{CRAFT_TICK}§action_completed§fail 0 ")),
        "two of the three never happened, so the action failed, got {settled:?}"
    );
}

/// A cancellation takes from the back of the queue: the game crafts in the
/// order it was asked, so the requests nearest completion are the earliest and
/// the ones that vanish are the latest.
#[test]
fn a_cancellation_fails_the_most_recent_request_first() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &format!(
            "{}\n{}\n{}\n{}",
            start(7, 1, "stone-furnace", 1),
            start(9, 1, "stone-furnace", 1),
            cancelled(1, "stone-furnace", 1),
            crafted(1, "stone-furnace"),
        ),
    );
    let settled = completions(&lua);
    assert_eq!(settled.len(), 2, "both requests answer, got {settled:?}");
    assert!(
        settled[0].starts_with(&format!("§{CRAFT_TICK}§action_completed§fail 9 ")),
        "the last request in is the one the cancellation took, got {settled:?}"
    );
    assert_eq!(
        settled[1],
        format!("§{CRAFT_TICK}§action_completed§ok 7"),
        "and the first request still settles on the craft that did happen"
    );
}

/// A cancellation for a recipe nobody is waiting on — an intermediate, or a
/// craft this run never asked for — settles nothing and consumes nothing.
#[test]
fn a_cancellation_of_another_recipe_settles_nothing() {
    let lua = run(
        &stub_game(&["stone-furnace", "iron-gear-wheel"], None),
        &format!(
            "{}\n{}\n{}",
            start(7, 1, "stone-furnace", 1),
            cancelled(1, "iron-gear-wheel", 4),
            crafted(1, "stone-furnace"),
        ),
    );
    assert_eq!(
        completions(&lua),
        vec![format!("§{CRAFT_TICK}§action_completed§ok 7")],
        "somebody else's cancellation is not this action's verdict"
    );
}

/// A crafted item nobody asked for — a human at the keyboard, or a leftover
/// craft from a request that was already refused — settles nothing and must not
/// raise.
#[test]
fn an_unrequested_craft_settles_nothing() {
    let lua = run(
        &stub_game(&["stone-furnace"], None),
        &crafted(1, "stone-furnace"),
    );
    assert_eq!(completions(&lua), Vec::<String>::new());
}
