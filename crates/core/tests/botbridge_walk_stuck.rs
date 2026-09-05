//! **A stalled walk leg fails the walk. It does not teleport, and it does not
//! re-path.**
//!
//! `control.lua` steers a walking character along waypoints the game's
//! pathfinder chose once, at dispatch time — and the run doing the walking is
//! *building things*, so those waypoints go stale as a matter of course. In
//! `workspace/runs/run-1788344167-58471` bot 1 kept trying to reach an ore tile
//! at `(-23.5, 18.5)` from behind a `stone-furnace` at `(-22.0, 18.0)` that the
//! same run had placed 4,400 ticks earlier; a 2x2 furnace spans x in
//! `[-23, -21]`, squarely across the route.
//!
//! Two recoveries have lived in this branch of the mod and both were wrong, in
//! the same way and for the same reason.
//!
//! - The **teleport** hopped the character onto the next waypoint, converting
//!   an unreachable destination into a reported arrival. The planner never
//!   learned a site was unreachable and kept choosing it.
//! - The **re-path** asked the game for a fresh route. Better, but still built
//!   out of the only thing this file has: `w.waypoints`. So it aimed at the
//!   last node of the path that had just gone stale — not at the goal the
//!   caller asked for — with a radius of 0.5 that the caller never chose and
//!   no judgement of whether a character could stand there. Every retry was
//!   therefore strictly harder than the request that had already failed, and
//!   in run 30's three failed walks arithmetically impossible: each ended
//!   inside a stone furnace, whose clearance needs 0.8984375.
//!
//! The recovery lives in `FactorioRcon::move_player_timed`
//! (`crates/core/src/factorio/rcon.rs`) now, where the goal, the radius and
//! `judge_path`'s standability check already are. What is left here is the
//! honest half: say the leg stopped progressing, say it promptly, and say it in
//! words the retry and the run record can both read.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state and drive
//! `on_tick` and `on_script_path_request_finished` against a stub surface. The
//! stub records every path request, which is how "the mod asks for nothing of
//! its own any more" is checked rather than assumed.

use factorio_bot_core::factorio::rcon::{WalkBlockerKind, walk_blocker, walk_reports_stalled_leg};
use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick `on_tick` is first driven at. Far past the leg timeout the stuck
/// fixtures set, so those walks are stuck on the tick they are first examined.
const TICK: u64 = 500;
/// Where every walk below is going.
const GOAL: (f64, f64) = (-22.5, 17.5);
/// How far from [`GOAL`] the bot stands.
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

    -- Every `request_path` the mod made, in order, and what it asked for. The
    -- point of these tests is that a stalled walk adds nothing to this: the
    -- only path requests left in the mod are the ones an RCON caller asked for.
    _path_requests = {}
    _next_path_handle = 0

    _players = {}

    local function make_surface()
        return {
            request_path = function(args)
                _next_path_handle = _next_path_handle + 1
                _path_requests[#_path_requests + 1] = {
                    id = _next_path_handle,
                    start_x = args.start.x,
                    start_y = args.start.y,
                    goal_x = args.goal.x,
                    goal_y = args.goal.y,
                    radius = args.radius,
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

/// A walk of `waypoints` legs ending at [`GOAL`], with an explicit leg timer.
///
/// `leg_timeout` and `idx_tick` are the whole fixture: `(1, 0)` is a leg that
/// the very first `on_tick` sees as stalled, and a wide timeout is the control
/// for it.
fn walk(waypoints: &[(f64, f64)], leg_timeout: u64, idx_tick: u64) -> String {
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
            idx_tick = {idx_tick},
            leg_timeout = {leg_timeout},
        }} }}
    "#,
        sx = gx + START_OFFSET,
    )
}

/// A walk of `waypoints` legs that is already past its leg timeout.
fn stuck_walk(waypoints: &[(f64, f64)]) -> String {
    walk(waypoints, 1, 0)
}

/// The everyday fixture: two legs, the second of which is the destination.
fn two_leg_walk() -> String {
    let (gx, gy) = GOAL;
    stuck_walk(&[(gx + 3.0, gy), (gx, gy)])
}

/// The handles of every `request_path` the mod made, in order.
fn path_requests(lua: &Lua) -> Vec<u32> {
    lua.globals()
        .get::<mlua::Table>("_path_requests")
        .expect("_path_requests")
        .sequence_values::<mlua::Table>()
        .map(|t| t.expect("a request row").get("id").expect("id"))
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

fn lines_containing(lua: &Lua, needle: &str) -> Vec<String> {
    stdout(lua)
        .into_iter()
        .filter(|l| l.contains(needle))
        .collect()
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

fn walk_field<T: mlua::FromLua>(lua: &Lua, field: &str) -> Option<T> {
    lua.load(format!(
        "local w = storage.p[1] and storage.p[1].walking; return w and w.{field}"
    ))
    .eval::<Option<T>>()
    .expect("a walk field")
}

fn walking_state_is_walking(lua: &Lua) -> bool {
    lua.load("return _players[1].walking_state.walking == true")
        .eval::<bool>()
        .expect("walking_state")
}

// ---------------------------------------------------------------------------
// what a stalled leg does now

/// **The change, stated as a test.** A stalled leg fails the walk, and it asks
/// the game for nothing on the way.
///
/// The "asks for nothing" half is the load-bearing one: as long as the mod is
/// still making path requests of its own, it is still answering the question
/// with the weaker of the two available judgements. It has no goal, no radius
/// and no entity graph; Rust has all three.
#[test]
fn a_stalled_leg_fails_the_walk_without_asking_the_game_for_a_path() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 2);

    assert_eq!(
        path_requests(&lua),
        Vec::<u32>::new(),
        "the mod no longer re-paths for itself; the retry lives in \
         move_player_timed"
    );
    assert_eq!(teleports(&lua), 0, "and it certainly does not teleport");
    assert!(
        failure(&lua).is_some(),
        "the walk is failed, got {:?}",
        stdout(&lua)
    );
}

/// And it fails **promptly** — on the two ticks an abort takes, not after a
/// budget of retries.
///
/// This is the property `WALK_REPATH_LIMIT` existed to protect and the one that
/// must not regress: a doomed walk has to be reported in seconds, not after the
/// executor's 360-second `ACTION_RESULT_DEADLINE`. Moving the retry to Rust
/// keeps it because Rust only ever retries a walk the game *answered*.
#[test]
fn the_walk_is_failed_on_the_tick_after_the_leg_times_out() {
    let lua = load(&two_leg_walk());
    tick(&lua, TICK);
    assert_eq!(failure(&lua), None, "the abort nils the waypoint first");
    tick(&lua, TICK + 1);
    assert!(
        failure(&lua).is_some(),
        "and the `dest == nil` arm reports it on the very next tick, got {:?}",
        stdout(&lua)
    );
}

/// The verdict has to be readable by the thing that acts on it.
///
/// `FactorioRcon::move_player_timed` retries a stalled leg and nothing else:
/// [`walk_reports_stalled_leg`] is what tells it apart from a walk failure that
/// is an *answer* about the map, and it matches on this wording because a
/// verdict crosses the RCON boundary as one opaque string. Asserting the real
/// mod's real message against the real classifier is the only thing that keeps
/// the two ends from drifting apart silently — the retry would simply stop
/// happening, and nothing else would notice.
#[test]
fn the_failure_is_worded_so_the_rust_retry_recognises_it() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 2);
    let reported = failure(&lua).expect("a failed walk");

    assert!(
        walk_reports_stalled_leg(&reported),
        "move_player_timed must recognise this as a stall worth re-asking, got \
         {reported}"
    );
}

/// The verdict also names the leg and both positions, because the run record
/// parses them back out.
///
/// `from` is where the character actually stood when the mod gave up and
/// `to` is the waypoint it was steering at, in the mod's own `(x/y)`
/// formatting. `walk_endpoints` (`crates/scripting_lua/src/globals/record.rs`)
/// reads exactly this shape, and it is the one place in the run record that
/// states an observed position of a bot rather than a planned one.
#[test]
fn the_failure_names_the_leg_and_the_two_positions() {
    let (gx, gy) = GOAL;
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 2);
    let reported = failure(&lua).expect("a failed walk");

    assert!(
        reported.contains("leg 1 of 2"),
        "which leg of how many, got {reported}"
    );
    assert!(
        reported.contains(&format!(
            "from ({}/{}) to ({}/{})",
            gx + START_OFFSET,
            gy,
            gx + 3.0,
            gy
        )),
        "the two observed positions, in the mod's own coord() shape, got \
         {reported}"
    );
}

/// A failed walk stops walking. Leaving `walking_state` set is how an earlier
/// version of this branch left a character marching in a straight line forever
/// while reporting success every tick.
#[test]
fn a_failed_walk_stops_the_character() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 2);

    assert!(
        !walking_state_is_walking(&lua),
        "the character is no longer being steered"
    );
    assert_eq!(
        walk_field::<mlua::Value>(&lua, "idx"),
        None,
        "and the walk is off the player's storage entirely"
    );
}

/// The final leg is not special. It used to abort here with a message that said
/// nothing about *why*, and it is the leg most likely to be blocked by
/// something the run itself built at the destination.
#[test]
fn a_stalled_final_leg_fails_the_same_way() {
    let lua = load(&stuck_walk(&[GOAL]));
    run_ticks(&lua, 2);

    let reported = failure(&lua).expect("a failed walk");
    assert!(walk_reports_stalled_leg(&reported), "got {reported}");
    assert!(reported.contains("leg 1 of 1"), "got {reported}");
}

/// The verdict is written once. A walk that reported failure and then kept
/// reporting anything is the same class of bug as a second reply under one
/// action id.
#[test]
fn the_failure_is_reported_exactly_once() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 6);

    assert_eq!(
        lines_containing(&lua, &format!("action_completed§fail {ACTION}")).len(),
        1,
        "got {:?}",
        stdout(&lua)
    );
}

/// The control, and it is not a formality: a leg still inside its timeout is
/// walked, not failed. A stall check that fires on every leg would turn every
/// walk in the run into a retry.
#[test]
fn a_leg_still_within_its_timeout_keeps_walking() {
    let (gx, gy) = GOAL;
    let lua = load(&walk(&[(gx + 3.0, gy), (gx, gy)], 600, TICK - 1));
    run_ticks(&lua, 2);

    assert_eq!(failure(&lua), None, "nothing has stalled yet");
    assert!(
        walking_state_is_walking(&lua),
        "and the character is being steered towards its waypoint"
    );
}

// ---------------------------------------------------------------------------
// the seam with the RCON path requests

/// Every path answer belongs to a Rust caller again.
///
/// The mod used to ask for paths of its own and had to filter those answers out
/// of `on_script_path_request_finished`, or they would have left an entry in
/// `world.path_requests` (`sleep_for_path_request_result`,
/// `crates/core/src/factorio/rcon.rs`) keyed by a handle nobody was waiting on,
/// which nothing ever removes. With the re-path gone the filter is gone, and
/// this is the test that says so: an answer that arrives *while a walk is
/// stalled* is still written out, because it cannot be the walk's.
#[test]
fn a_path_answer_is_written_out_even_while_a_walk_is_stalled() {
    let lua = load(&two_leg_walk());
    run_ticks(&lua, 2);
    answer_with_path(&lua, TICK + 2, 9999, &[GOAL]);

    let line = line_containing(&lua, "§on_script_path_request_finished§")
        .expect("an RCON caller is waiting on this one");
    assert!(
        line.contains("9999#"),
        "the reply is keyed by the handle, got {line}"
    );
}

// ---------------------------------------------------------------------------
// the follower driving a character that moves
//
// Everything above holds the character still and asks what the follower says
// about it. These tests move the character the way the game would -- 0.15
// tiles per tick in the direction `walking_state` names -- and ask what the
// follower does over a whole leg. They exist because of run 9
// (`workspace/runs/run-1788576604-65414`): bot 1 walked 1.04 of a 1.42-tile
// leg in 7 ticks, the game then held it still for 54 ticks on open dirt with
// nothing within twelve tiles, and the follower reported "made no progress for
// 61 ticks" -- the whole time since the leg began -- from a position a reader
// took for the leg's origin. The clock is a progress clock now, and the
// message says where on the leg the character stopped and what the follower
// was doing when it did.

/// Where the character starts for every walk below: 0.05 tiles off the centre
/// of its own tile, which is where a real walk starts -- and the pathfinder's
/// first waypoint is that tile centre, so the first leg is over before the
/// character has moved. Then a 1.42-tile diagonal, which is what most legs of
/// a real path are (the median leg in run 11 was exactly `sqrt(2)`).
const START: (f64, f64) = (10.45, 10.45);
const OWN_TILE: (f64, f64) = (10.5, 10.5);
const NEXT_TILE: (f64, f64) = (11.5, 9.5);

/// A stub character that walks: after each tick, `step_character(idx, speed)`
/// moves it `speed` tiles along the direction `walking_state` names, exactly
/// as the game would. The probe's two surface calls are stubbed to "open
/// ground", so a stall here reads `nothing findable` rather than
/// `probe failed`.
const MOVING_STUB: &str = r#"
    local dir_names = {}
    for _, name in ipairs({ "north", "northeast", "east", "southeast",
                            "south", "southwest", "west", "northwest" }) do
        dir_names[defines.direction[name]] = name
    end
    local dir_vec = {
        north = { 0, -1 }, northeast = { 1, -1 }, east = { 1, 0 }, southeast = { 1, 1 },
        south = { 0, 1 }, southwest = { -1, 1 }, west = { -1, 0 }, northwest = { -1, -1 },
    }
    function step_character(idx, speed)
        local p = _players[idx]
        local ws = p.walking_state
        if ws == nil or not ws.walking then return false end
        local v = dir_vec[dir_names[ws.direction]]
        local len = math.sqrt(v[1] * v[1] + v[2] * v[2])
        p.position.x = p.position.x + v[1] / len * speed
        p.position.y = p.position.y + v[2] / len * speed
        return true
    end
    function open_ground(idx)
        local s = _players[idx].surface
        s.find_entities_filtered = function() return {} end
        s.get_tile = function() return { name = "dirt-3", valid = true } end
    end
"#;

/// A walk dispatched through the mod's own `start_walk_waypoints`, so the leg
/// origin, the clock and the first-tick arrival are the real ones.
fn dispatched_walk(start: (f64, f64), waypoints: &[(f64, f64)]) -> String {
    let legs = waypoints
        .iter()
        .map(|(x, y)| format!("{{ {x}, {y} }}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"{INIT_STORAGE}
        {MOVING_STUB}
        make_player(1, {sx}, {sy})
        open_ground(1)
        game.tick = {dispatch}
        assert(start_walk_waypoints({ACTION}, 1, {{ {legs} }}) == true)
    "#,
        sx = start.0,
        sy = start.1,
        dispatch = TICK - 1,
    )
}

/// One tick of the game as the follower sees it: `on_tick`, then the character
/// moves by what the follower set -- or does not, when `moves` says so.
fn drive(lua: &Lua, at: u64, moves: bool) {
    tick(lua, at);
    if moves {
        lua.load("step_character(1, 0.15)")
            .exec()
            .expect("step_character");
    }
}

fn success(lua: &Lua) -> Option<String> {
    line_containing(lua, &format!("action_completed§ok {ACTION}"))
}

/// The first waypoint of a real path is the centre of the tile the character
/// stands on, and the character is already inside its box. That leg is done
/// on the first tick, and the next waypoint is steered at on that same tick:
/// no tick is spent, no clock is started on a leg that never needed walking.
#[test]
fn a_waypoint_the_character_already_stands_inside_is_done_on_the_first_tick() {
    let lua = load(&dispatched_walk(START, &[OWN_TILE, NEXT_TILE]));
    drive(&lua, TICK, true);

    assert_eq!(walk_field::<u64>(&lua, "idx"), Some(2), "leg 1 is over");
    assert!(walking_state_is_walking(&lua), "and leg 2 is being steered");
    assert_eq!(failure(&lua), None);

    for at in TICK + 1..TICK + 20 {
        drive(&lua, at, true);
    }
    assert!(
        success(&lua).is_some(),
        "a 1.42-tile leg at full speed is over in about ten ticks, got {:?}",
        stdout(&lua)
    );
    assert_eq!(failure(&lua), None);
}

/// A leg walked slowly is not a stall. The character here advances on one tick
/// in four -- what a script-walked character does in a game whose tick rate
/// has collapsed -- so a 5-tile leg takes about 136 ticks. The old clock,
/// 3x the straight-line time floored at 60, gave this leg 100 ticks from the
/// tick it began and failed the walk with "made no progress" while the
/// character was visibly progressing. A progress clock restarts every tick
/// the character gets closer.
#[test]
fn a_leg_walked_at_a_quarter_of_full_speed_is_not_a_stall() {
    let lua = load(&dispatched_walk(START, &[OWN_TILE, (15.5, 10.5)]));
    for (i, at) in (TICK..TICK + 170).enumerate() {
        drive(&lua, at, i % 4 == 0);
    }
    assert_eq!(
        failure(&lua),
        None,
        "a slow leg is walked, not failed, got {:?}",
        stdout(&lua)
    );
    assert!(
        success(&lua).is_some(),
        "and it arrives, got {:?}",
        stdout(&lua)
    );
}

/// Run 9's stall, exactly: seven ticks of walking, then the game holds the
/// character still. The verdict counts from the last tick the leg got closer
/// -- 61, not 68 -- and says where on the leg the character is, where the leg
/// began, what the follower was steering, and what the engine's own
/// `walking_state` read back at that instant.
#[test]
fn a_character_the_game_holds_still_is_reported_with_the_ticks_since_it_last_moved() {
    let lua = load(&dispatched_walk(START, &[OWN_TILE, NEXT_TILE]));
    // Tick 0 ends leg 1 and steers leg 2; the character moves on ticks 0..6.
    for at in TICK..TICK + 7 {
        drive(&lua, at, true);
    }
    // Held still from here. The last tick that saw the distance shrink is
    // TICK + 7 (it reads the position tick 6's step produced), so the clock
    // runs out on TICK + 68 and the verdict is written on TICK + 69.
    for at in TICK + 7..TICK + 68 {
        drive(&lua, at, false);
        assert_eq!(failure(&lua), None, "not yet, at tick {at}");
    }
    drive(&lua, TICK + 68, false);
    drive(&lua, TICK + 69, false);
    let reported = failure(&lua).expect("the walk is failed");

    assert!(walk_reports_stalled_leg(&reported), "got {reported}");
    assert!(
        reported.contains("leg 2 of 2 made no progress for 61 ticks from ("),
        "ticks since the last progress, not since the leg began, got {reported}"
    );
    assert!(
        reported.contains("moved 1.05 tiles of a 1.42-tile leg that began at (10.45/10.45)"),
        "where on the leg it stopped, and the leg's own origin, got {reported}"
    );
    assert!(
        reported.contains("by nothing findable on tile 'dirt-3'"),
        "the probe still answers, got {reported}"
    );
    assert!(
        reported
            .contains(", steering east at 0.150 tiles/tick, walking_state read back walking=true"),
        "what the follower was doing and what the engine held, got {reported}"
    );

    let blocker = walk_blocker(&reported).expect("the Rust side reads it");
    assert_eq!(blocker.kind, WalkBlockerKind::Nothing);
    assert_eq!(blocker.moved_tiles, Some(1.05));
    assert_eq!(blocker.leg_tiles, Some(1.42));
    assert_eq!(blocker.engine_walking, Some(true));
}

/// An offset of exactly 0.3 used to be neither inside the arrival box
/// (`< 0.3`) nor steered (`> 0.3`): the character stood there until the clock
/// ran out. The steer is the complement of the box now.
#[test]
fn an_offset_of_exactly_three_tenths_is_steered_not_stranded() {
    // 0.5 - 0.2 is exactly the double nearest 0.3 (10.5 - 10.2 is not), which
    // is what makes this reachable at all; the fixture checks it rather than
    // assuming it.
    let lua = load(&format!(
        "{}\nassert(({} - {}) == 0.3, 'the fixture needs an exact 0.3')",
        dispatched_walk((0.2, 10.5), &[(0.5, 10.5)]),
        0.5,
        0.2
    ));
    drive(&lua, TICK, true);
    assert!(
        walking_state_is_walking(&lua) || success(&lua).is_some(),
        "0.3 away is steered, got {:?}",
        stdout(&lua)
    );
    drive(&lua, TICK + 1, true);
    assert!(
        success(&lua).is_some(),
        "and arrives, got {:?}",
        stdout(&lua)
    );
    assert_eq!(failure(&lua), None);
}
