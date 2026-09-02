//! **A stalled walk leg re-paths; it never teleports.**
//!
//! `control.lua` steers a walking character along waypoints the game's
//! pathfinder chose once, at dispatch time — and the run doing the walking is
//! *building things*, so those waypoints go stale as a matter of course. In
//! `workspace/runs/run-1788344167-58471` bot 1 kept trying to reach an ore tile
//! at `(-23.5, 18.5)` from behind a `stone-furnace` at `(-22.0, 18.0)` that the
//! same run had placed 4,400 ticks earlier; a 2x2 furnace spans x in
//! `[-23, -21]`, squarely across the route.
//!
//! The old recovery teleported the character onto the next waypoint. That is
//! worse than no recovery at all: it converts an *unreachable destination* into
//! a *reported arrival*, so the planner never learns a site is unreachable and
//! keeps choosing it. Every later fix — not landing on an occupied tile, not
//! re-aiming at a waypoint the character cannot stand on, a cap on how many
//! times one walk may be rescued — was compensation for that one inversion.
//!
//! What replaces it asks the game the question the teleport was papering over:
//! *is there a path from where this character actually stands to where it is
//! going?* Either there is one, and the walk carries on along it, or there is
//! not, and the walk fails saying `unreachable` — which is the fact the
//! supervisor needs and the teleport destroyed.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state and drive
//! `on_tick` and `on_script_path_request_finished` against a stub surface. The
//! stub records path requests and lets a test answer them the three ways the
//! game can: with a path, with `try_again_later`, or with neither.
//!
//! What this cannot prove: that Factorio's own `request_path` finds a way round
//! an obstruction the previous path went through. That is the load-bearing
//! assumption and only a live run settles it. What it *can* prove, and does, is
//! that the mod asks, that it asks from the right place, and that every answer
//! — including "no" — reaches the caller intact.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick `on_tick` is first driven at. Far past the leg timeout the fixtures
/// set, so every walk below is stuck on the tick it is first examined.
const TICK: u64 = 500;
/// Where every walk below is going.
const GOAL: (f64, f64) = (-22.5, 17.5);
/// How far from [`GOAL`] the stuck bot stands. Six tiles, so a leg aimed
/// straight at the goal times out after `6 / 0.15 * 3 = 120` ticks.
const START_OFFSET: f64 = 6.0;
/// The action id every fixture walk is dispatched under.
const ACTION: u32 = 110;

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
        { table_to_json = function(t) return "<json>" end },
        { __index = function() return noop end })

    storage = {}

    -- Any `player.teleport` the mod attempts. The whole point of these tests is
    -- that this stays empty: the two remaining teleport sites are the ghost and
    -- blueprint ones, and neither is reachable from `on_tick`.
    _teleports = {}

    -- Every `request_path` the mod made, in order, and what it asked for.
    _path_requests = {}
    _next_path_handle = 0
    -- Flipped by a test to make the game refuse to take a request at all.
    _path_request_refuses = false

    _players = {}

    local function make_surface()
        return {
            request_path = function(args)
                if _path_request_refuses then return nil end
                _next_path_handle = _next_path_handle + 1
                _path_requests[#_path_requests + 1] = {
                    id = _next_path_handle,
                    start_x = args.start.x,
                    start_y = args.start.y,
                    goal_x = args.goal.x,
                    goal_y = args.goal.y,
                    radius = args.radius,
                    answered = false,
                }
                return _next_path_handle
            end,
            find_non_colliding_position = function(name, center) return center end,
            find_entity = function() return nil end,
        }
    end

    -- A player standing at (x, y). `position` and `character.position` are the
    -- same table, exactly as they are in the game: a path request reads
    -- `player.position` and the walk follower reads `player.character.position`,
    -- and a stub where those two drift would hide a real mix-up.
    function make_player(idx, x, y)
        local pos = { x = x, y = y }
        local p = {
            index = idx,
            name = "bot" .. idx,
            connected = true,
            force = "player",
            position = pos,
            character_running_speed = 0.15,
            walking_state = { walking = false },
            character = {
                position = pos,
                prototype = { collision_box = {}, collision_mask = {} },
            },
        }
        p.surface = make_surface()
        p.teleport = function(target)
            _teleports[#_teleports + 1] = { player = idx, x = target.x, y = target.y }
            return true
        end
        _players[idx] = p
        return p
    end

    game = { tick = 0, players = _players, forces = {}, surfaces = {} }
    prototypes = { item = {}, entity = {} }
"#;

/// `on_tick` writes the static world data out on its first call and stamps
/// distances; neither is what is under test.
const STUB_TICK_EXTRAS: &str = r#"
    writeout_initial_stuff = function() end
    writeout_recipes = function() end
    writeout_forces = function() end
    on_player_changed_distance = function(e) end
"#;

const INIT_STORAGE: &str = "storage.p = {}\n";

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

/// A loaded `control.lua` with the fixture applied and nothing driven yet.
fn load(setup: &str) -> Lua {
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
    lua.load(setup)
        .set_name("setup")
        .exec()
        .expect("fixture setup");
    lua
}

fn tick(lua: &Lua, at: u64) {
    lua.load(format!("on_tick({{ tick = {at} }})"))
        .set_name("on_tick")
        .exec()
        .expect("on_tick");
}

/// Drives `ticks` consecutive `on_tick` calls from [`TICK`].
///
/// Two is the minimum for any *abort*, because an abort is a two-step move: the
/// branch that gives up nils the current waypoint, and the `dest == nil` arm on
/// the following tick clears `walking` and writes the verdict.
fn run_ticks(lua: &Lua, ticks: u64) {
    for at in TICK..TICK + ticks {
        tick(lua, at);
    }
}

/// A walk of `waypoints` legs ending at [`GOAL`], already past its leg timeout.
///
/// `leg_timeout = 1` with `idx_tick = 0` means the very first `on_tick` sees a
/// stalled leg, which is the only condition every test here starts from.
fn stuck_walk(waypoints: &[(f64, f64)]) -> String {
    let (gx, gy) = GOAL;
    let legs = waypoints
        .iter()
        .map(|(x, y)| format!("{{ x = {x}, y = {y} }}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"{INIT_STORAGE}
        make_player(1, {sx}, {gy})
        storage.p[1] = {{ walking = {{
            idx = 1,
            waypoints = {{ {legs} }},
            action_id = {ACTION},
            idx_tick = 0,
            leg_timeout = 1,
        }} }}
    "#,
        sx = gx + START_OFFSET,
    )
}

/// The everyday fixture: two legs, the second of which is the destination.
fn two_leg_walk() -> String {
    let (gx, gy) = GOAL;
    stuck_walk(&[(gx + 3.0, gy), (gx, gy)])
}

#[derive(Debug, Clone, Copy)]
struct PathRequest {
    id: u32,
    start: (f64, f64),
    goal: (f64, f64),
    radius: f64,
}

fn path_requests(lua: &Lua) -> Vec<PathRequest> {
    lua.globals()
        .get::<mlua::Table>("_path_requests")
        .expect("_path_requests")
        .sequence_values::<mlua::Table>()
        .map(|t| {
            let t = t.expect("a request row");
            PathRequest {
                id: t.get("id").expect("id"),
                start: (
                    t.get("start_x").expect("start_x"),
                    t.get("start_y").expect("start_y"),
                ),
                goal: (
                    t.get("goal_x").expect("goal_x"),
                    t.get("goal_y").expect("goal_y"),
                ),
                radius: t.get("radius").expect("radius"),
            }
        })
        .collect()
}

fn teleports(lua: &Lua) -> usize {
    lua.globals()
        .get::<mlua::Table>("_teleports")
        .expect("_teleports")
        .len()
        .expect("teleport count") as usize
}

fn stdout(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect()
}

fn line_containing(lua: &Lua, needle: &str) -> Option<String> {
    stdout(lua).into_iter().find(|l| l.contains(needle))
}

fn failure(lua: &Lua) -> Option<String> {
    line_containing(lua, &format!("action_completed§fail {ACTION}"))
}

/// The pathfinder answers request `id` with a path through `points`.
fn answer_with_path(lua: &Lua, at: u64, id: u32, points: &[(f64, f64)]) {
    let path = points
        .iter()
        .map(|(x, y)| {
            format!("{{ position = {{ x = {x}, y = {y} }}, needs_destroy_to_reach = false }}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    lua.load(format!(
        "on_script_path_request_finished({{ tick = {at}, id = {id}, path = {{ {path} }} }})"
    ))
    .set_name("path_answer")
    .exec()
    .expect("path answer");
}

/// The pathfinder searched and found nothing.
fn answer_no_path(lua: &Lua, at: u64, id: u32) {
    lua.load(format!(
        "on_script_path_request_finished({{ tick = {at}, id = {id} }})"
    ))
    .set_name("path_answer")
    .exec()
    .expect("path answer");
}

/// The pathfinder's queue was full and never searched at all.
fn answer_try_again(lua: &Lua, at: u64, id: u32) {
    lua.load(format!(
        "on_script_path_request_finished({{ tick = {at}, id = {id}, try_again_later = true }})"
    ))
    .set_name("path_answer")
    .exec()
    .expect("path answer");
}

fn walk_field<T: mlua::FromLua>(lua: &Lua, field: &str) -> Option<T> {
    lua.load(format!(
        "local w = storage.p[1] and storage.p[1].walking; return w and w.{field}"
    ))
    .eval::<Option<T>>()
    .expect("a walk field")
}

fn waypoints(lua: &Lua) -> Vec<(f64, f64)> {
    lua.load(
        "local w = storage.p[1] and storage.p[1].walking
         local out = {}
         if w ~= nil then
             for i, p in ipairs(w.waypoints) do out[i] = { p.x, p.y } end
         end
         return out",
    )
    .eval::<mlua::Table>()
    .expect("waypoints")
    .sequence_values::<mlua::Table>()
    .map(|t| {
        let t = t.expect("a waypoint");
        (t.get::<f64>(1).expect("x"), t.get::<f64>(2).expect("y"))
    })
    .collect()
}

// ---------------------------------------------------------------------------
// asking

/// **The fix, stated as a test.** A stalled leg asks the game for a path; it
/// does not move the character itself.
#[test]
fn a_stalled_leg_asks_for_a_fresh_path_and_never_teleports() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);

    assert_eq!(
        teleports(&lua),
        0,
        "a stalled leg must never move the character by fiat; that is what \
         turned an unreachable destination into a reported arrival"
    );
    let requests = path_requests(&lua);
    assert_eq!(
        requests.len(),
        1,
        "one re-path was asked for, got {requests:?}"
    );
}

/// The request has to start from where the character *is*. Asking from the
/// original start would re-derive the same stale path, which is the entire bug.
#[test]
fn the_re_path_starts_from_where_the_character_actually_stands() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);

    let requests = path_requests(&lua);
    assert_eq!(
        requests[0].start,
        (gx + START_OFFSET, gy),
        "the fresh path must be computed from the character's current position"
    );
}

/// And it has to aim at the walk's *destination*, not at the leg that stalled.
/// Re-pathing to the next waypoint would route around nothing: the next
/// waypoint is on the far side of whatever is in the way.
#[test]
fn the_re_path_aims_at_the_walks_destination_not_the_stalled_leg() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);

    let requests = path_requests(&lua);
    assert_eq!(
        requests[0].goal, GOAL,
        "the destination is the last waypoint of the walk, not the next one"
    );
}

/// The radius has to be small enough that "the pathfinder found a way" means
/// it found a way *there*. Factorio's default of 1 lets it stop a tile short,
/// and a re-path that ends somewhere else is the reported-arrival lie again.
#[test]
fn the_re_path_asks_for_the_destination_itself() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);

    let radius = path_requests(&lua)[0].radius;
    assert!(
        (0.5..1.0).contains(&radius),
        "0.5 is the smallest radius that does not collapse onto the goal's own \
         tile, and anything from 1 upwards lets the pathfinder stop short; got \
         {radius}"
    );
}

/// The final leg is not special any more. It used to abort with a message that
/// said nothing about *why*, and it is the leg most likely to be blocked by
/// something the run itself built at the destination.
#[test]
fn a_stalled_final_leg_re_paths_too() {
    let lua = load(&stuck_walk(&[GOAL]));
    run_ticks(&lua, 1);

    let requests = path_requests(&lua);
    assert_eq!(
        requests.len(),
        1,
        "the last leg is where the destination itself is blocked; it must ask \
         like any other, got {requests:?}"
    );
    assert_eq!(requests[0].goal, GOAL);
}

/// While the answer is outstanding the character holds still and no second
/// request is stacked on the first. A character that keeps walking into
/// whatever stopped it is walking away from the position the new path was
/// computed from.
#[test]
fn a_walk_waiting_on_a_re_path_holds_still_and_asks_only_once() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 30);

    assert_eq!(
        path_requests(&lua).len(),
        1,
        "one outstanding request per walk, however long the answer takes"
    );
    let walking: bool = lua
        .load("return _players[1].walking_state.walking == true")
        .eval()
        .expect("walking_state");
    assert!(
        !walking,
        "the character must not keep steering while it waits"
    );
}

// ---------------------------------------------------------------------------
// answering: a path

/// The answer replaces the remaining route and the walk carries on. Nothing is
/// failed, nothing is teleported, and the leg index restarts at the head of the
/// new path.
#[test]
fn a_fresh_path_replaces_the_route_and_the_walk_carries_on() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_with_path(
        &lua,
        TICK + 1,
        id,
        &[(gx + 4.0, gy + 4.0), (gx + 2.0, gy + 2.0), (gx, gy)],
    );

    assert_eq!(
        waypoints(&lua),
        vec![(gx + 4.0, gy + 4.0), (gx + 2.0, gy + 2.0), (gx, gy)],
        "the whole remaining route is replaced, not patched"
    );
    assert_eq!(walk_field::<u32>(&lua, "idx"), Some(1), "at the head of it");
    assert_eq!(walk_field::<u32>(&lua, "repaths"), Some(1));
    assert_eq!(
        failure(&lua),
        None,
        "a re-path that worked is not a failure"
    );
}

/// **The destination is not negotiable.** The Rust side checked the dispatched
/// path's last waypoint against what the caller asked for before any of this
/// began and nothing re-checks it afterwards, so a re-path that ends short must
/// still finish at the same place — otherwise "the walk completed" stops
/// meaning "the bot got there", which is exactly the teleport's inversion.
#[test]
fn a_re_path_that_stops_short_still_ends_at_the_destination() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_with_path(&lua, TICK + 1, id, &[(gx + 2.0, gy), (gx + 1.0, gy)]);

    let route = waypoints(&lua);
    assert_eq!(
        route.last(),
        Some(&GOAL),
        "the walk still has to end where it was going, got {route:?}"
    );
}

/// The control for that: a path that already arrives must not have the
/// destination bolted on twice, which would leave a zero-length final leg.
#[test]
fn a_re_path_that_arrives_does_not_repeat_the_destination() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_with_path(&lua, TICK + 1, id, &[(gx + 2.0, gy), (gx, gy)]);

    assert_eq!(
        waypoints(&lua),
        vec![(gx + 2.0, gy), (gx, gy)],
        "the path already ends at the destination"
    );
}

// ---------------------------------------------------------------------------
// answering: no path

/// **The whole point.** The pathfinder saying "there is no way there" is the
/// fact the teleport destroyed. It has to reach the caller, and it has to say
/// *unreachable* rather than some generic stuck.
#[test]
fn no_path_fails_the_walk_saying_unreachable() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_no_path(&lua, TICK + 1, id);
    tick(&lua, TICK + 2);

    let failed = failure(&lua).expect("a walk with no path must be failed, not left running");
    assert!(
        failed.contains("unreachable"),
        "the reason has to name it, got {failed}"
    );
    assert_eq!(teleports(&lua), 0, "and nothing may be hopped over instead");
}

/// A failed walk stops walking. Leaving `walking_state` set is how an earlier
/// version of this branch left a character marching in a straight line forever
/// while reporting success every tick.
#[test]
fn an_unreachable_walk_stops_the_character() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_no_path(&lua, TICK + 1, id);
    tick(&lua, TICK + 2);

    let still_walking: bool = lua
        .load("return storage.p[1].walking ~= nil or _players[1].walking_state.walking == true")
        .eval()
        .expect("walk state");
    assert!(
        !still_walking,
        "the walk is over; nothing may still be steering"
    );
}

// ---------------------------------------------------------------------------
// answering: try again later

/// **The two pathfinder failures are not the same failure.** `try again later`
/// means the request queue was full and the question was never asked. Treating
/// it as unreachable would fail walks that were fine.
#[test]
fn a_busy_pathfinder_is_asked_again_rather_than_called_unreachable() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_try_again(&lua, TICK + 1, id);

    let requests = path_requests(&lua);
    assert_eq!(
        requests.len(),
        2,
        "a full queue is asked again, got {requests:?}"
    );
    assert_eq!(failure(&lua), None, "and it is not a failure");
    assert_eq!(
        requests[1].goal, GOAL,
        "the retry asks the same question, not a substituted one"
    );
}

/// And retrying a busy queue must not spend the re-path budget: nothing was
/// searched, so nothing was learned.
#[test]
fn a_busy_pathfinder_does_not_spend_the_re_path_budget() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    for i in 0..6 {
        let id = path_requests(&lua).last().expect("a request").id;
        answer_try_again(&lua, TICK + 1 + i, id);
    }
    // Ticked afterwards on purpose: a walk that was failed sets its verdict on
    // one tick and writes it on the next, so a test that never ticks again
    // cannot tell "still alive" from "failed and not yet reported".
    tick(&lua, TICK + 8);

    assert_eq!(
        walk_field::<u32>(&lua, "repaths").unwrap_or(0),
        0,
        "a queue that was full never answered the question"
    );
    assert_eq!(failure(&lua), None, "so the walk is still alive");
}

/// What bounds that retrying is a tick budget, not a retry count, and it is not
/// restamped by a retry — otherwise a permanently saturated pathfinder would
/// keep one walk alive forever.
#[test]
fn a_pathfinder_that_never_answers_ends_the_walk() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 320);

    assert_eq!(
        path_requests(&lua).len(),
        1,
        "no answer came, so nothing was re-asked"
    );
    let failed = failure(&lua).expect("a walk cannot wait on the pathfinder forever");
    assert!(
        failed.contains("did not answer"),
        "the reason has to name the silence, got {failed}"
    );
}

// ---------------------------------------------------------------------------
// bounding

/// Answers every outstanding request with a path straight to [`GOAL`], driving
/// `ticks` ticks. The stub never moves the character, so every adopted path
/// stalls again: this is the runaway the bound exists for.
fn run_answering_every_request(lua: &Lua, ticks: u64) {
    let mut answered = 0usize;
    for at in TICK..TICK + ticks {
        tick(lua, at);
        let requests = path_requests(lua);
        while answered < requests.len() {
            answer_with_path(lua, at, requests[answered].id, &[GOAL]);
            answered += 1;
        }
    }
}

/// **A bound, so a walk that re-paths forever costs seconds rather than the
/// executor's whole 360-second deadline.** A re-path that keeps succeeding and
/// never arrives has to terminate, and it has to terminate into a named
/// failure the supervisor can replan against.
#[test]
fn a_walk_that_keeps_re_pathing_gives_up_at_the_limit() {
    let lua = load(&two_leg_walk());
    run_answering_every_request(&lua, 800);

    let requests = path_requests(&lua);
    assert_eq!(
        requests.len(),
        4,
        "the bound is 4 re-paths for one walk, got {requests:?}"
    );
    let failed = failure(&lua).expect("hitting the bound has to end the walk");
    assert!(
        failed.contains("4 re-paths"),
        "the failure has to name the bound, got {failed}"
    );
}

/// The control: one re-path is a working recovery, not an exhausted budget.
#[test]
fn one_re_path_is_not_the_limit() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_with_path(&lua, TICK + 1, id, &[(gx, gy)]);
    tick(&lua, TICK + 2);

    assert_eq!(
        failure(&lua),
        None,
        "one re-path is nowhere near the bound and must not fail the walk"
    );
}

// ---------------------------------------------------------------------------
// the seam with the RCON path requests

/// The mod now asks for paths of its own, and those answers are consumed here.
/// Writing them out anyway would leave an entry in `world.path_requests`
/// (`sleep_for_path_request_result`, `crates/core/src/factorio/rcon.rs`) keyed
/// by a handle nobody is waiting on, which nothing ever removes.
#[test]
fn a_re_path_answer_is_not_written_out_to_the_rust_side() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    answer_with_path(&lua, TICK + 1, id, &[GOAL]);

    assert_eq!(
        line_containing(&lua, "§on_script_path_request_finished§"),
        None,
        "this answer was the mod's own; nobody on the Rust side is waiting"
    );
}

/// And the answers that *are* someone's — every `player_path` the executor
/// asks for — still get written out exactly as before.
#[test]
fn an_rcon_path_answer_is_still_written_out() {
    let lua = load(&two_leg_walk());
    answer_with_path(&lua, TICK, 9999, &[GOAL]);

    let line = line_containing(&lua, "§on_script_path_request_finished§")
        .expect("an RCON caller is waiting on this one");
    assert!(
        line.contains("9999#"),
        "the reply is keyed by the handle, got {line}"
    );
}

/// An answer that arrives after its walk has already been settled is dropped.
/// Adopting it would restart a walk whose verdict has been reported, which is
/// the same class of bug as a second reply under one action id.
#[test]
fn an_answer_for_a_walk_that_already_ended_is_dropped() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 1);
    let id = path_requests(&lua)[0].id;
    // The walk is settled and the bot is dispatched somewhere else entirely --
    // which is what actually happens when the pending budget runs out and the
    // supervisor replans. Merely clearing `walking` would not be enough of a
    // fixture: the answer has to be offered a *live* walk it does not belong
    // to, because that is the walk it would hijack.
    lua.load(
        "storage.p[1].walking = { idx = 1,
             waypoints = { { x = 99, y = 99 } },
             action_id = 111, idx_tick = 400, leg_timeout = 6000 }",
    )
    .exec()
    .expect("a later, unrelated walk");
    answer_with_path(&lua, TICK + 1, id, &[GOAL]);

    assert_eq!(
        waypoints(&lua),
        vec![(99.0, 99.0)],
        "an answer belonging to a settled walk must not steer the next one"
    );
    assert_eq!(
        walk_field::<u32>(&lua, "action_id"),
        Some(111),
        "and must not report anything under the settled walk's action id"
    );
    assert_eq!(
        line_containing(&lua, "§on_script_path_request_finished§"),
        None,
        "it was still our request, so it is still not the Rust side's answer"
    );
}

/// A game that will not take the request at all is a failure, not a silence.
/// Without this the walk would sit at its stalled leg re-asking every tick.
#[test]
fn a_request_the_game_refuses_fails_the_walk() {
    let lua = load(&format!(
        "{}\n_path_request_refuses = true\n",
        two_leg_walk()
    ));
    run_ticks(&lua, 2);

    assert!(path_requests(&lua).is_empty(), "the game took nothing");
    let failed = failure(&lua).expect("a walk that cannot even ask must not keep asking");
    assert!(
        failed.contains("refused"),
        "the failure has to name the refusal, got {failed}"
    );
}
