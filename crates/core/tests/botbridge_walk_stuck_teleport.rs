//! **The stuck-walk recovery must not manufacture a permanent stuck.**
//!
//! When a walk leg stops progressing, `on_tick` teleports the character onto
//! the waypoint. `LuaControl.teleport` does not respect collisions
//! (`workspace/factorio-api-docs/runtime-api.json`, Factorio 2.1.17: it returns
//! "`true` if the entity was successfully teleported" and says nothing about
//! refusing an occupied destination), and characters *do* collide. So two bots
//! whose legs went stuck near the same tile were both put on that tile, after
//! which the game's pathfinder correctly reported no path for either of them,
//! and every retry stacked them further.
//!
//! That is exactly what ended `workspace/runs/run-1788341905-92036`: bot 3 was
//! teleported to (-22.5, 17.5), bot 1 was teleported onto it, and
//! `samples.jsonl` shows both frozen at those identical coordinates until the
//! run gave up — `milestone_stuck`, `stuck_silent`, 33 steps planned and none
//! dispatched, with `failed to find player_path() ... the game's pathfinder
//! returned no path` once per iteration.
//!
//! The fix asks `LuaSurface::find_non_colliding_position` for somewhere the
//! character actually fits, checks what `teleport` answers, and records where
//! the bot *landed* rather than where it was aimed.
//!
//! These tests load the real `control.lua` into a Lua 5.4 state and drive
//! `on_tick` against a stub surface that models occupancy the way the game
//! does: a position is free only if no other character is standing on it.
//!
//! What this cannot prove: that Factorio's own
//! `find_non_colliding_position("character", ..)` counts other characters as
//! obstacles. That is the load-bearing assumption and only a live run settles
//! it. See `docs/superpowers/notes/2026-09-02-awaited-research.md`.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick `on_tick` is driven at. Far past the leg timeout the fixtures set,
/// so every walk below is stuck on the tick it is examined.
const TICK: u64 = 500;
/// The waypoint every stuck leg is aiming at, and the tile the run's two bots
/// ended up sharing.
const WAYPOINT: (f64, f64) = (-22.5, 17.5);

const PRELUDE: &str = r#"
    local function auto()
        local t = {}
        setmetatable(t, { __index = function(tbl, k)
            local v = auto(); rawset(tbl, k, v); return v
        end })
        return t
    end
    defines = auto()
    -- `walking_state` is set from `defines.direction[name]`; the auto table
    -- gives each name a distinct value, which is all the mod needs here.
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
        { table_to_json = function(t)
            -- Only the teleport record is read back, and only these fields.
            return "player_id=" .. tostring(t.player_id)
                .. " reason=" .. tostring(t.reason)
                .. " to=" .. tostring(t.to and t.to.x) .. "/" .. tostring(t.to and t.to.y)
          end },
        { __index = function() return noop end })

    storage = {}

    -- Every `teleport` the mod attempted: who, where to, and what the game
    -- answered.
    _teleports = {}
    -- Flipped by a test to make the game refuse.
    _teleport_refuses = false
    -- How far apart two characters have to be to not collide. Factorio's
    -- character collision box is about 0.4 tiles across; anything closer than
    -- this counts as the same tile for these tests.
    local CLEARANCE = 0.5

    _players = {}

    local function occupied(x, y, except)
        for i, p in pairs(_players) do
            if p ~= except and p.character ~= nil then
                local c = p.character.position
                if math.abs(c.x - x) < CLEARANCE and math.abs(c.y - y) < CLEARANCE then
                    return true
                end
            end
        end
        return false
    end

    -- Enough of `LuaSurface::find_non_colliding_position` to be honest about
    -- the one thing that matters: it will not hand back a spot another
    -- character is standing on. Nearest-first, stepping by `precision`.
    local function make_surface(owner)
        return {
            find_non_colliding_position = function(name, center, radius, precision)
                _searches = (_searches or 0) + 1
                if not occupied(center.x, center.y, owner) then
                    return { x = center.x, y = center.y }
                end
                local step = precision
                if step == nil or step < 0.01 then step = 0.01 end
                local r = step
                while r <= radius do
                    for dx = -r, r, step do
                        for dy = -r, r, step do
                            local x, y = center.x + dx, center.y + dy
                            if not occupied(x, y, owner) then
                                return { x = x, y = y }
                            end
                        end
                    end
                    r = r + step
                end
                return nil
            end,
            find_entity = function() return nil end,
        }
    end

    -- A player standing at (x, y). `walking` is filled in by the fixtures that
    -- want a walk in progress.
    function make_player(idx, x, y)
        local p = {
            index = idx,
            name = "bot" .. idx,
            connected = true,
            character_running_speed = 0.15,
            walking_state = { walking = false },
            character = { position = { x = x, y = y } },
        }
        p.surface = make_surface(p)
        p.teleport = function(pos)
            if _teleport_refuses then
                _teleports[#_teleports + 1] = { player = idx, x = pos.x, y = pos.y, ok = false }
                return false
            end
            p.character.position = { x = pos.x, y = pos.y }
            _teleports[#_teleports + 1] = { player = idx, x = pos.x, y = pos.y, ok = true }
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

fn run(setup: &str) -> Lua {
    run_ticks(setup, 2)
}

/// [`run`], driving `ticks` consecutive `on_tick` calls from [`TICK`].
///
/// More than two is how a *spin* becomes visible: a leg that teleports and
/// then keeps aiming at a waypoint it cannot reach times out again 61 ticks
/// later, and again, and again.
fn run_ticks(setup: &str, ticks: u64) -> Lua {
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
    // Two ticks, because an abort is a two-step move: the branch that gives up
    // nils the current waypoint, and the `dest == nil` arm on the *following*
    // tick is what clears `walking` and writes the verdict. That is how the
    // pre-existing last-waypoint abort has always worked and the new arms
    // reuse it rather than inventing a second way to fail. A leg that
    // teleported successfully re-stamps its timer, so the second tick adds no
    // further teleport.
    for tick in TICK..TICK + ticks {
        lua.load(format!("on_tick({{ tick = {tick} }})"))
            .set_name("on_tick")
            .exec()
            .expect("on_tick");
    }
    lua
}

/// A player at `(x, y)` with a stuck two-waypoint walk aiming at [`WAYPOINT`].
///
/// Two waypoints, because the one-waypoint case is the *last* leg, which the
/// mod aborts rather than teleporting — a different branch with its own,
/// already-correct behaviour.
fn stuck_walker(idx: u32, x: f64, y: f64, action_id: u32) -> String {
    let (wx, wy) = WAYPOINT;
    format!(
        r#"
        make_player({idx}, {x}, {y})
        storage.p[{idx}] = {{ walking = {{
            idx = 1,
            waypoints = {{ {{ x = {wx}, y = {wy} }}, {{ x = {wx}, y = {wy2} }} }},
            action_id = {action_id},
            idx_tick = 0,
            leg_timeout = 1,
        }} }}
    "#,
        wy2 = wy + 8.0,
    )
}

/// A player standing still, so it is nothing but an obstacle.
fn bystander(idx: u32, x: f64, y: f64) -> String {
    format!("make_player({idx}, {x}, {y})\nstorage.p[{idx}] = {{}}\n")
}

const INIT_STORAGE: &str = "storage.p = {}\n";

fn teleports(lua: &Lua) -> Vec<(u32, f64, f64, bool)> {
    lua.load(
        r#"
        local out = {}
        for i, t in ipairs(_teleports) do
            out[i] = { t.player, t.x, t.y, t.ok }
        end
        return out
    "#,
    )
    .eval::<mlua::Table>()
    .expect("_teleports")
    .sequence_values::<mlua::Table>()
    .map(|t| {
        let t = t.expect("a teleport row");
        (
            t.get::<u32>(1).expect("player"),
            t.get::<f64>(2).expect("x"),
            t.get::<f64>(3).expect("y"),
            t.get::<bool>(4).expect("ok"),
        )
    })
    .collect()
}

fn stdout(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect()
}

fn position(lua: &Lua, idx: u32) -> (f64, f64) {
    let t: mlua::Table = lua
        .load(format!("return _players[{idx}].character.position"))
        .eval()
        .expect("a character position");
    (t.get("x").expect("x"), t.get("y").expect("y"))
}

fn line_containing(lua: &Lua, needle: &str) -> Option<String> {
    stdout(lua).into_iter().find(|l| l.contains(needle))
}

/// **The bug, stated as a test.** A waypoint another bot is standing on must
/// not be teleported onto. Stacking two characters is what turned a stuck leg
/// into a stuck run: the pathfinder then refuses every later request, so the
/// recovery creates the condition it exists to fix.
#[test]
fn a_stuck_leg_never_lands_on_an_occupied_tile() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}{}{}",
        bystander(3, wx, wy),
        stuck_walker(1, wx + 6.0, wy, 110),
    ));
    let moves = teleports(&lua);
    assert_eq!(moves.len(), 1, "one teleport attempt, got {moves:?}");
    let (player, x, y, ok) = moves[0];
    assert_eq!(player, 1);
    assert!(ok, "the stub game accepted this teleport");
    assert!(
        (x - wx).abs() >= 0.5 || (y - wy).abs() >= 0.5,
        "the bot was put on top of the one already standing at the waypoint, \
         at ({x}, {y})"
    );
    let (bx, by) = position(&lua, 3);
    assert_eq!(
        (bx, by),
        (wx, wy),
        "the bystander did not move; it is the obstacle"
    );
}

/// The unoccupied case still lands exactly on the waypoint. Without this, a
/// search that always nudged would pass the test above while quietly making
/// every ordinary recovery less accurate.
#[test]
fn a_stuck_leg_with_a_clear_waypoint_lands_on_it() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}{}",
        stuck_walker(1, wx + 6.0, wy, 110)
    ));
    let moves = teleports(&lua);
    assert_eq!(moves.len(), 1, "one teleport attempt, got {moves:?}");
    assert_eq!(
        (moves[0].1, moves[0].2),
        (wx, wy),
        "nothing is in the way, so the recovery is unchanged"
    );
}

/// **Two bots stuck on the same leg in the same tick.** `on_tick` walks the
/// players one at a time and a teleport takes effect immediately, so the
/// second bot's search sees the first one already standing there. This is the
/// case the run record shows going wrong, and it is handled by the ordering
/// rather than by any extra bookkeeping.
#[test]
fn two_bots_stuck_in_one_tick_do_not_land_on_each_other() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}{}{}",
        stuck_walker(1, wx + 6.0, wy, 110),
        stuck_walker(2, wx + 6.0, wy + 1.0, 111),
    ));
    let moves = teleports(&lua);
    assert_eq!(moves.len(), 2, "both bots were stuck, got {moves:?}");
    let (a, b) = (position(&lua, 1), position(&lua, 2));
    assert!(
        (a.0 - b.0).abs() >= 0.5 || (a.1 - b.1).abs() >= 0.5,
        "two bots were sent to one destination in one tick: {a:?} and {b:?}"
    );
}

/// Nowhere free means no teleport at all. Stacking is worse than failing:
/// failing costs this walk, stacking costs both bots for the rest of the run.
#[test]
fn a_stuck_leg_with_nowhere_free_gives_up_instead_of_stacking() {
    let (wx, wy) = WAYPOINT;
    // Ring the waypoint so the search cannot find anything within its radius.
    let mut fixture = format!("{INIT_STORAGE}{}", stuck_walker(1, wx + 20.0, wy, 110));
    let mut idx = 10;
    let mut dx = -6.0;
    while dx <= 6.0 {
        let mut dy = -6.0;
        while dy <= 6.0 {
            fixture.push_str(&bystander(idx, wx + dx, wy + dy));
            idx += 1;
            dy += 0.5;
        }
        dx += 0.5;
    }
    let lua = run(&fixture);
    assert!(
        teleports(&lua).is_empty(),
        "there was nowhere to go, so nothing may have been teleported"
    );
    let failure = line_containing(&lua, "action_completed§fail 110")
        .expect("the walk has to be failed rather than left running");
    assert!(
        failure.contains("no free"),
        "the failure has to say why, got {failure}"
    );
}

/// `teleport` returns whether it happened, and that return used to be
/// discarded — the same discarded-return-value class as the placement fix. A
/// walk whose recovery did not move must not carry on as if it had.
#[test]
fn a_teleport_the_game_refuses_fails_the_walk() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}_teleport_refuses = true\n{}",
        stuck_walker(1, wx + 6.0, wy, 110)
    ));
    let moves = teleports(&lua);
    assert_eq!(moves.len(), 1, "it did try, got {moves:?}");
    assert!(!moves[0].3, "and the game said no");
    assert_eq!(
        position(&lua, 1),
        (wx + 6.0, wy),
        "a refused teleport leaves the bot where it was"
    );
    let failure = line_containing(&lua, "action_completed§fail 110")
        .expect("a walk whose recovery did not happen must not stay running");
    assert!(
        failure.contains("refused"),
        "the failure has to name the refusal, got {failure}"
    );
}

/// **The record must name where the bot landed, not where it was sent.**
///
/// `teleport_writeout` was called with the waypoint, before the teleport, so
/// an adjusted or failed landing left the record asserting something untrue.
/// In the run that prompted this the two happened to coincide, so the record
/// was accurate by luck.
#[test]
fn the_record_names_where_the_bot_landed() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}{}{}",
        bystander(3, wx, wy),
        stuck_walker(1, wx + 6.0, wy, 110),
    ));
    let moves = teleports(&lua);
    let (_, x, y, _) = moves[0];
    let record = line_containing(&lua, "§teleport§").expect("a teleport record");
    // Parsed rather than string-matched: Lua and Rust format the same float
    // differently ("-23.0" against "-23"), and the claim here is about the
    // number, not about the spelling.
    let (rx, ry) = record
        .split_once("to=")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(a, b)| {
            (
                a.trim().parse::<f64>().expect("recorded x"),
                b.trim().parse::<f64>().expect("recorded y"),
            )
        })
        .unwrap_or_else(|| panic!("no to= in {record}"));
    assert_eq!(
        (rx, ry),
        (x, y),
        "the record has to carry the landing, got {record}"
    );
    assert_ne!(
        (rx, ry),
        (wx, wy),
        "and must not carry the waypoint it was adjusted away from, got {record}"
    );
}

/// Nothing above may have quietly broken the branch that was already right:
/// a stuck *final* leg aborts the walk and never teleports.
#[test]
fn a_stuck_last_leg_still_aborts_without_teleporting() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        r#"{INIT_STORAGE}
        make_player(1, {x}, {wy})
        storage.p[1] = {{ walking = {{
            idx = 1,
            waypoints = {{ {{ x = {wx}, y = {wy} }} }},
            action_id = 110,
            idx_tick = 0,
            leg_timeout = 1,
        }} }}
    "#,
        x = wx + 6.0,
    ));
    assert!(
        teleports(&lua).is_empty(),
        "the last leg is aborted, never teleported"
    );
}

/// A line of `count` waypoints one tile apart, every one of them occupied, so
/// each leg's teleport has to be adjusted off the line.
///
/// One tile apart on purpose: `walk_leg_timeout_ticks` floors at 60 ticks, so
/// short legs keep the whole fixture inside a few hundred ticks.
fn blocked_corridor(count: u32, action_id: u32) -> String {
    let (wx, wy) = WAYPOINT;
    let mut out = String::new();
    let mut waypoints = Vec::new();
    for i in 0..count {
        let x = wx - f64::from(i);
        out.push_str(&bystander(20 + i, x, wy));
        waypoints.push(format!("{{ x = {x}, y = {wy} }}"));
    }
    out.push_str(&format!(
        r#"
        make_player(1, {start}, {wy})
        storage.p[1] = {{ walking = {{
            idx = 1,
            waypoints = {{ {waypoints} }},
            action_id = {action_id},
            idx_tick = 0,
            leg_timeout = 1,
        }} }}
    "#,
        start = wx + 6.0,
        waypoints = waypoints.join(", "),
    ));
    out
}

/// **The regression, stated as a test.**
///
/// `find_non_colliding_position` answers with somewhere the character *fits*,
/// which is not the waypoint whenever the waypoint is inside something.
/// Arrival is judged against the waypoint with a 0.3 box, so a leg that
/// re-aims at it after landing further away than that can never complete: it
/// times out again 61 ticks later, the deterministic search returns the
/// identical spot, and it spins.
///
/// `run-1788344167-58471` did exactly that — 1414 teleports, every one of them
/// bot 1 to `(-22.0, 19.0)`, 87,766 ticks across four dispatches, against a
/// stone furnace the run had placed at `(-22, 18)` itself.
#[test]
fn a_leg_is_never_teleported_more_than_once() {
    let (wx, wy) = WAYPOINT;
    let lua = run_ticks(
        &format!(
            "{INIT_STORAGE}{}{}",
            bystander(3, wx, wy),
            stuck_walker(1, wx + 6.0, wy, 110),
        ),
        400,
    );
    let moves = teleports(&lua);
    assert_eq!(
        moves.len(),
        1,
        "the leg is spent once it has been teleported; re-aiming at a waypoint \
         the character cannot stand on is a spin, got {} attempts",
        moves.len()
    );
}

/// The other half of that: the leg must actually *advance*, not merely stop
/// teleporting. Advancing is what the old collision-blind code achieved by
/// teleporting onto the waypoint illegally, and it is what makes one teleport
/// per leg true by construction — which is also why no memory of
/// already-tried destinations is needed.
#[test]
fn a_teleport_advances_the_leg_it_could_not_finish() {
    let (wx, wy) = WAYPOINT;
    let lua = run(&format!(
        "{INIT_STORAGE}{}{}",
        bystander(3, wx, wy),
        stuck_walker(1, wx + 6.0, wy, 110),
    ));
    let idx: u32 = lua
        .load("return storage.p[1].walking and storage.p[1].walking.idx or 0")
        .eval()
        .expect("the walk's leg index");
    assert_eq!(
        idx, 2,
        "the teleport ended leg 1, so the follower must be aiming at waypoint 2"
    );
}

/// **A cap, so a spin costs seconds rather than the executor's whole
/// deadline.**
///
/// Advancing bounds this branch to one teleport per *leg*, but a long enough
/// route could still hop its whole length. The cap turns that into a named
/// failure the supervisor can replan against.
#[test]
fn a_walk_that_keeps_needing_teleports_gives_up_at_the_cap() {
    let lua = run_ticks(&format!("{INIT_STORAGE}{}", blocked_corridor(14, 110)), 900);
    let moves = teleports(&lua);
    assert_eq!(
        moves.len(),
        8,
        "the cap is 8 teleports for one walk, got {moves:?}"
    );
    let failure = line_containing(&lua, "action_completed§fail 110")
        .expect("hitting the cap has to end the walk, not quietly stop helping");
    assert!(
        failure.contains("8 teleports"),
        "the failure has to name the cap, got {failure}"
    );
}

/// The control: a walk that needs one teleport is nowhere near the cap and
/// must not be failed by it.
#[test]
fn one_teleport_is_not_the_cap() {
    let (wx, wy) = WAYPOINT;
    let lua = run_ticks(
        &format!(
            "{INIT_STORAGE}{}{}",
            bystander(3, wx, wy),
            stuck_walker(1, wx + 6.0, wy, 110),
        ),
        400,
    );
    let failure = line_containing(&lua, "action_completed§fail 110").expect("this walk does end");
    assert!(
        !failure.contains("teleports"),
        "one teleport is a working recovery, not an exhausted budget, got {failure}"
    );
}
