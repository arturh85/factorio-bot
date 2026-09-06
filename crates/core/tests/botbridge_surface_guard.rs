//! **The one `if` that keeps this project single-surface, and the line that
//! finally admits it.**
//!
//! `on_chunk_generated` in `mods/BotBridge/control.lua` returns early for any
//! surface but Nauvis. The guard is deliberate and must stay: `EntityGraph`,
//! `PlanState` and the mod's own `resource_key` are keyed by position alone,
//! so a Vulcanus chunk would merge into Nauvis with **no error anywhere** --
//! wrong ore amounts, an arbitrary `entity_at`, a `resource_fingerprint` that
//! loses the overlapping tiles from its census entirely.
//!
//! What was wrong was not the guard but its silence. It used to
//! `print("unknown surface")`, and a bare `print` is not a `writeout`: it
//! carries no `§tick§key§` envelope, so `output_parser.rs` never saw it. The
//! drop reached the server log and **no record artefact at all**, which means a
//! run that discarded a whole planet's chunks was byte-identical, in every
//! artefact anyone reads, to a run that never left home. This repo has a name
//! for that shape -- "silence is not success" -- and this was its fifth
//! instance.
//!
//! **Space Age is enabled in this workspace** (`workspace/mods/mod-list.json`
//! has `space-age`, `quality`, `elevated-rails` and `recycler` all enabled), so
//! every run this project has ever measured was a Space Age run that happened
//! never to leave Nauvis. The second surface is one rocket away, not a future
//! modding decision.
//!
//! Every test below runs the **real** `control.lua` against a stub game, so it
//! pins the mod's behaviour rather than a description of it. See
//! `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game with **two** surfaces, which is the whole point: a fixture with
/// only Nauvis could not tell the guard from an unconditional pass, and a
/// fixture with only Vulcanus could not tell the guard from a mod that writes
/// nothing at all. Both surfaces answer `get_tile` and
/// `find_entities_filtered`, so the accepted path really does produce output.
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
    rcon = { print = noop }

    -- A real encoder with sorted keys and nested tables, because the
    -- assertions below read the JSON the mod actually emits. A stub encoder
    -- that could not nest would have hidden the `left_top` object entirely --
    -- that exact failure was found in this repo on 2026-09-06.
    local function encode(v)
        local t = type(v)
        if t == "number" then
            if v == math.floor(v) then return string.format("%d", v) end
            return tostring(v)
        elseif t == "string" then return '"' .. v .. '"'
        elseif t == "boolean" then return tostring(v)
        elseif t == "nil" then return "null" end
        local keys = {}
        for k in pairs(v) do keys[#keys + 1] = k end
        table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
        local parts = {}
        for _, k in ipairs(keys) do
            parts[#parts + 1] = '"' .. tostring(k) .. '":' .. encode(v[k])
        end
        return "{" .. table.concat(parts, ",") .. "}"
    end
    helpers = { table_to_json = encode, write_file = noop, remove_path = noop }

    local function make_surface(name)
        return {
            name = name,
            index = 1,
            get_tile = function(x, y)
                return { name = "grass-1", collides_with = function() return false end }
            end,
            find_entities = function() return {} end,
            find_entities_filtered = function() return {} end,
            find_entity = function() return nil end,
        }
    end

    _nauvis = make_surface("nauvis")
    _vulcanus = make_surface("vulcanus")

    storage = {
        n_clients = 0,
        p = {},
        map_area = { x1 = 0, y1 = 0, x2 = 0, y2 = 0 },
    }
    client_local_data = { whoami = "server" }
    game = {
        tick = 4242,
        players = {},
        connected_players = {},
        forces = {},
        surfaces = { _nauvis, nauvis = _nauvis, vulcanus = _vulcanus },
        take_screenshot = noop,
    }
    prototypes = { item = {}, entity = {} }
"#;

fn mod_lua() -> Lua {
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

/// Fires `on_chunk_generated` for one 32x32 chunk on the named surface.
fn generate_chunk(lua: &Lua, surface_global: &str, left_top_x: i32, left_top_y: i32) {
    lua.load(format!(
        "on_chunk_generated({{ tick = 4242, surface = {surface_global}, area = {{ \
         left_top = {{ x = {left_top_x}, y = {left_top_y} }}, \
         right_bottom = {{ x = {}, y = {} }} }} }})",
        left_top_x + 32,
        left_top_y + 32
    ))
    .set_name("on_chunk_generated")
    .exec()
    .expect("on_chunk_generated");
}

/// Every line the mod printed to stdout.
fn printed(lua: &Lua) -> Vec<String> {
    let list: Table = lua.globals().get("_printed").expect("_printed");
    list.sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect()
}

/// The bodies of every `writeout` line whose key is `key`.
///
/// `writeout` is `print("§"..tick.."§"..key.."§"..value)`, and *that envelope
/// is the whole subject of this file*: `output_parser.rs` keys on the sentinel,
/// so a line without it reaches no artefact. Splitting on it here rather than
/// substring-matching means a regression back to a bare `print` fails, which
/// substring-matching on the key alone would not catch.
fn writeouts(lua: &Lua, key: &str) -> Vec<String> {
    printed(lua)
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.split('\u{a7}');
            // "" before the leading separator, then tick, then key, then value.
            let _empty = parts.next()?;
            let _tick = parts.next()?;
            let this_key = parts.next()?;
            if this_key != key {
                return None;
            }
            Some(parts.collect::<Vec<_>>().join("\u{a7}"))
        })
        .collect()
}

#[test]
fn a_chunk_on_another_surface_is_dropped_and_the_drop_is_written_out() {
    let lua = mod_lua();
    generate_chunk(&lua, "_vulcanus", -32, 64);

    let dropped = writeouts(&lua, "surface_chunk_dropped");
    assert_eq!(
        dropped,
        vec![r#"{"left_top":{"x":-32,"y":64},"surface":"vulcanus"}"#.to_string()],
        "the drop must reach stdout inside a writeout envelope, naming the \
         surface and the chunk's top-left TILE; all lines: {:?}",
        printed(&lua)
    );
}

/// The other half, and the reason the fixture carries two surfaces: the guard
/// has to be what stops the chunk, not the stub being unable to produce
/// anything. Vulcanus writes **no** `tiles` and **no** `entities`; Nauvis
/// writes both.
#[test]
fn nothing_from_another_surface_reaches_the_world_model() {
    let lua = mod_lua();
    generate_chunk(&lua, "_vulcanus", -32, 64);
    assert_eq!(
        writeouts(&lua, "tiles").len(),
        0,
        "a dropped chunk must contribute no tiles: {:?}",
        printed(&lua)
    );
    assert_eq!(
        writeouts(&lua, "entities").len(),
        0,
        "a dropped chunk must contribute no entities: {:?}",
        printed(&lua)
    );

    let nauvis = mod_lua();
    generate_chunk(&nauvis, "_nauvis", -32, 64);
    assert_eq!(
        writeouts(&nauvis, "tiles").len(),
        1,
        "the same chunk on Nauvis MUST write tiles -- otherwise the test above \
         proves only that the stub is inert: {:?}",
        printed(&nauvis)
    );
    assert_eq!(
        writeouts(&nauvis, "entities").len(),
        1,
        "and entities: {:?}",
        printed(&nauvis)
    );
    assert_eq!(
        writeouts(&nauvis, "surface_chunk_dropped").len(),
        0,
        "and Nauvis is never reported as dropped: {:?}",
        printed(&nauvis)
    );
}

/// **One line per dropped chunk, on purpose.**
///
/// The mod keeps no counter, because a counter would have to live in `storage`
/// to survive a save/load, and a mod-side tally that resets on load is worse
/// than none. Folding happens in Rust
/// (`FactorioWorld::record_surface_chunk_dropped`), which is what turns a
/// generated planet's tens of thousands of chunks into one row per surface.
#[test]
fn every_dropped_chunk_reports_itself() {
    let lua = mod_lua();
    generate_chunk(&lua, "_vulcanus", 0, 0);
    generate_chunk(&lua, "_vulcanus", 32, 0);
    generate_chunk(&lua, "_vulcanus", 0, 32);
    assert_eq!(
        writeouts(&lua, "surface_chunk_dropped").len(),
        3,
        "three chunks, three lines: {:?}",
        printed(&lua)
    );
}
