//! **Where do eight character bots stand when they are spawned?**
//!
//! `rcon_spawn_bots` in `mods/BotBridge/control.lua` used to put each character
//! at `find_non_colliding_position("character", spawn, 32, 0.5)`, and the game
//! answered honestly by the collision box: eight characters 0.4 tiles across
//! fit in a two-by-two square, so the first eight-bot run
//! (`run-1788614781-38058`) spawned them at `(0,0) (-0.5,-0.5) (-0.5,0.5)
//! (0.5,-0.5) (0,-0.5) (0,0.5) (-0.5,0) (0.5,0)`. The pathfinder refused the
//! first walk of the three bots at `x = 0` -- `failed to path find` from the
//! spawn itself -- and the executor's walk memory learned their destinations
//! as unreachable.
//!
//! The mod now gives every bot its own tile, two apart, on a spiral in bot-id
//! order, and skips any candidate within a tile of a character already placed.
//! These tests load the real `control.lua` into a Lua 5.4 state over a stub
//! surface that records what was created where, and check the geometry the
//! game is never asked to guarantee: pairwise distance, determinism, and that
//! the spread does not depend on whether the game's search counts characters.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// Enough of Factorio's Lua API for `control.lua` to load and for
/// `rcon_spawn_bots` to run. `counts_characters` decides whether the stub
/// `find_non_colliding_position` refuses a spot another character's box
/// covers (the live game does) or answers the centre it was given (a game
/// that did not would be the worst case for the mod's own spacing).
fn stub(counts_characters: bool) -> String {
    format!(
        r#"
        local function auto()
            local t = {{}}
            setmetatable(t, {{ __index = function(tbl, k)
                local v = auto(); rawset(tbl, k, v); return v
            end }})
            return t
        end
        defines = auto()
        function noop() end
        local function nooptable()
            return setmetatable({{}}, {{ __index = function() return noop end }})
        end
        script = nooptable()
        commands = nooptable()
        require = function() return {{}} end
        print = noop
        -- No freeplay: nothing to insert. `remote.interfaces` has to be a
        -- real table, or indexing it answers a function and the next index
        -- raises.
        remote = setmetatable({{ interfaces = {{}} }}, {{ __index = function() return noop end }})
        helpers = setmetatable(
            {{ table_to_json = function(t) return "<json>" end }},
            {{ __index = function() return noop end }})
        _rcon_lines = {{}}
        rcon = {{ print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }}

        _created = {{}}
        _searched = {{}}
        local function overlaps_a_character(p)
            for _, e in ipairs(_created) do
                if math.abs(e.position.x - p.x) < 0.4 and math.abs(e.position.y - p.y) < 0.4 then
                    return true
                end
            end
            return false
        end
        local surface = {{
            find_non_colliding_position = function(name, center, radius, precision, tile_centre)
                _searched[#_searched + 1] = {{ x = center.x, y = center.y, radius = radius }}
                if {counts} and overlaps_a_character(center) then return nil end
                return {{ x = center.x, y = center.y }}
            end,
            create_entity = function(args)
                local e = {{
                    valid = true, name = "character", type = "character",
                    unit_number = #_created + 1,
                    position = {{ x = args.position.x, y = args.position.y }},
                    insert = noop,
                }}
                _created[#_created + 1] = e
                return e
            end,
        }}
        local force = {{
            name = "player",
            get_spawn_position = function(s) return {{ x = 0.3, y = -0.2 }} end,
        }}
        game = {{
            tick = 1000,
            connected_players = {{}},
            players = {{}},
            surfaces = {{ [1] = surface }},
            forces = {{ player = force }},
        }}
        storage = {{ p = {{}} }}
        prototypes = {{ item = {{}}, entity = {{}} }}
    "#,
        counts = if counts_characters { "true" } else { "false" },
    )
}

/// A joining bot is announced through the per-tick poll, which wants the whole
/// inventory API; none of that is under test.
const AFTER: &str = r#"
    announce_character_bot = function() end
    on_player_changed_distance = function() end
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

fn spawn(counts_characters: bool, count: u8) -> Lua {
    let lua = lua_for_mod_source();
    lua.load(stub(counts_characters))
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
    lua.load(AFTER).set_name("after").exec().expect("overrides");
    lua.load(format!("rcon_spawn_bots({count})"))
        .set_name("spawn")
        .exec()
        .expect("rcon_spawn_bots");
    lua
}

/// Where the characters were created, in creation (bot-id) order.
fn positions(lua: &Lua) -> Vec<(f64, f64)> {
    lua.load("local o = {} for i, e in ipairs(_created) do o[i] = { e.position.x, e.position.y } end return o")
        .eval::<mlua::Table>()
        .expect("_created")
        .sequence_values::<mlua::Table>()
        .map(|t| {
            let t = t.expect("a position");
            (t.get::<f64>(1).unwrap(), t.get::<f64>(2).unwrap())
        })
        .collect()
}

fn chebyshev(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

fn assert_spread(positions: &[(f64, f64)]) {
    assert_eq!(positions.len(), 8, "eight characters, got {positions:?}");
    for (i, a) in positions.iter().enumerate() {
        for b in &positions[i + 1..] {
            assert!(
                chebyshev(*a, *b) >= 1.0,
                "bots must not share a tile: {a:?} and {b:?} are {:.2} apart in {positions:?}",
                chebyshev(*a, *b)
            );
        }
        assert!(
            a.0.fract().abs() == 0.5 && a.1.fract().abs() == 0.5,
            "each bot stands on a tile centre, got {a:?}"
        );
        assert!(
            chebyshev(*a, (0.5, -0.5)) <= 6.0,
            "and within a few tiles of the spawn, got {a:?}"
        );
    }
}

/// **The pile, stated as a test.** Eight spawns are eight tiles, not one.
#[test]
fn eight_bots_spawn_on_eight_different_tiles() {
    assert_spread(&positions(&spawn(true, 8)));
}

/// The spread must not depend on the game counting characters as
/// colliding: a search that answers "right there" for every candidate still
/// gets eight different tiles, because the mod keeps its own list of what it
/// has placed.
#[test]
fn the_spread_holds_even_if_the_games_search_ignores_characters() {
    assert_spread(&positions(&spawn(false, 8)));
}

/// Bot 3 stands where bot 3 stood last run. The spiral is a function of the
/// bot id and nothing else, so two fresh spawns agree exactly.
#[test]
fn spawn_positions_are_deterministic_in_bot_id_order() {
    let first = positions(&spawn(true, 8));
    let second = positions(&spawn(true, 8));
    assert_eq!(first, second);
    assert_eq!(
        first[0],
        (0.5, -0.5),
        "bot 1 takes the spawn's own tile centre, got {first:?}"
    );
}

/// The lattice the spiral walks: the origin, then the eight cells around it,
/// then the sixteen around those -- each ring in a fixed row-major order, so
/// the same id always maps to the same offset.
#[test]
fn the_spiral_visits_each_ring_in_a_fixed_order() {
    let lua = spawn(true, 0);
    let offsets: Vec<(i64, i64)> = lua
        .load(
            "local o = {} for n = 1, 10 do local p = character_spawn_offset(n) o[n] = { p.x, p.y } end return o",
        )
        .eval::<mlua::Table>()
        .expect("offsets")
        .sequence_values::<mlua::Table>()
        .map(|t| {
            let t = t.expect("an offset");
            (t.get::<i64>(1).unwrap(), t.get::<i64>(2).unwrap())
        })
        .collect();
    assert_eq!(
        offsets,
        vec![
            (0, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
            (-2, -2),
        ]
    );
}

/// A second call keeps the characters it has and places only the missing
/// ones -- clear of the survivors, which are in the game's world and not in
/// this call's own bookkeeping.
#[test]
fn a_respawn_places_new_characters_clear_of_the_kept_ones() {
    let lua = spawn(true, 3);
    // Bot 2's character is gone; bots 1 and 3 stay where they were.
    lua.load("storage.bots[2].entity.valid = false; rcon_spawn_bots(4)")
        .set_name("respawn")
        .exec()
        .expect("second spawn");
    let all = positions(&lua);
    assert_eq!(all.len(), 5, "three, then two more, got {all:?}");
    let kept = [all[0], all[2]];
    for new in &all[3..] {
        for old in &kept {
            assert!(
                chebyshev(*new, *old) >= 1.0,
                "a new character {new:?} landed on kept one {old:?}: {all:?}"
            );
        }
    }
    assert!(
        chebyshev(all[3], all[4]) >= 1.0,
        "and the two new ones are apart from each other: {all:?}"
    );
}
