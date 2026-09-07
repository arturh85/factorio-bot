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
    -- `remote` stays a nooptable for everything the mod calls on it, but
    -- `add_interface` is CAPTURED: a test that reaches a handler through its
    -- own Lua global proves nothing about whether the mod published it under
    -- that name, and a mutation dropping a registration line was green until
    -- this existed.
    _interfaces = {}
    remote = setmetatable(
        { add_interface = function(name, fns) _interfaces[name] = fns end },
        { __index = function() return noop end }
    )
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
/// (`FactorioSurface::record_surface_chunk_dropped`), which is what turns a
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

/// **The other half of the guard: what the drop above cannot say.**
///
/// Each `surface_chunk_dropped` line is honest about the chunk it refused, and
/// the honest refusals sum to a world model that holds one surface -- which is
/// equally consistent with a save that HAS one surface and with a save whose
/// others were never mentioned. Nothing in this mod read `game.surfaces` until
/// 2026-09-07, so those two readings were the same silence.
///
/// `collect_surfaces` ends it, and it **reports rather than ingests**: the
/// guard is untouched, and the census is three fields per surface with no
/// chunk, entity or tile count anywhere in it.
///
/// The stub is given three surfaces **out of index order**, so an
/// implementation forwarding `pairs()` order would have to be lucky to pass,
/// and one of them is a platform with no planet at all.
#[test]
fn the_census_enumerates_every_surface_in_index_order() {
    let lua = mod_lua();
    lua.load(
        r#"
        local function planet(name) return { name = name } end
        _fulgora = { name = "fulgora", index = 5, planet = planet("fulgora") }
        _platform = { name = "platform-1", index = 3 }
        game.surfaces = {
            _fulgora,
            _platform,
            { name = "nauvis", index = 1, planet = planet("nauvis") },
        }
        _census = helpers.table_to_json(collect_surfaces())
        "#,
    )
    .set_name("census")
    .exec()
    .expect("collect_surfaces");

    let census: String = lua.globals().get("_census").expect("_census");
    // The encoder in PRELUDE sorts object keys, so this is the whole record.
    assert_eq!(
        census,
        r#"{"1":{"index":1,"name":"nauvis","planet":"nauvis"},"2":{"index":3,"name":"platform-1"},"3":{"index":5,"name":"fulgora","planet":"fulgora"}}"#,
        "three surfaces, sorted by index, with the platform carrying no planet \
         -- a real answer about what a platform is, not a gap in the record",
    );
}

/// The census is also askable of a **running** game, which is what a
/// long-played save needs: `rcon_world_snapshot` carries the same list, but on
/// a six-hour base its prototype tables are megabytes and the one question
/// here is three fields per surface.
#[test]
fn the_census_is_askable_over_rcon() {
    let lua = mod_lua();
    lua.load(
        r#"
        _replies = {}
        rcon = { print = function(s) _replies[#_replies + 1] = tostring(s) end }
        game.surfaces = {
            { name = "nauvis", index = 1, planet = { name = "nauvis" } },
            { name = "platform-1", index = 2 },
        }
        -- Through the PUBLISHED interface, not the Lua global: a caller
        -- reaches this as `remote.call('botbridge', 'surfaces')` and nothing
        -- else, so a handler that exists and is not registered is a handler
        -- nobody can ask.
        _interfaces.botbridge.surfaces()
        "#,
    )
    .set_name("rcon_surfaces")
    .exec()
    .expect("rcon_surfaces");

    let replies: Table = lua.globals().get("_replies").expect("_replies");
    let replies: Vec<String> = replies
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect();
    assert_eq!(replies.len(), 1, "one reply, the whole census");
    assert_eq!(
        replies[0],
        r#"{"1":{"index":1,"name":"nauvis","planet":"nauvis"},"2":{"index":2,"name":"platform-1"}}"#,
    );

    // **And it stays out of the executor's channel.** `rcon.print` inside a
    // function the executor calls lands in the action's result body; this one
    // is never called that way, and the assertion that it is registered under
    // its own name is what keeps that true if somebody rewires it.
    assert!(
        !printed(&lua).iter().any(|line| line.contains('\u{a7}')),
        "an RCON query is not a writeout -- nothing here belongs in the record",
    );
}

/// **The census rides on `world_snapshot` too, and today Rust drops it.**
///
/// `WorldSnapshot` has no field for it yet: the landing site is
/// `WorldSnapshot.surfaces`, which did not exist when this test was written:
/// the key was tolerated and thrown away, and the test's name said so. It now
/// **lands**, and this is the seam between the two transports -- one JSON
/// shape, read by the RCON half here and by the stdout half in
/// `surface_census_lands.rs`.
#[test]
fn a_snapshot_carrying_the_census_lands_it_rather_than_dropping_it() {
    use factorio_bot_core::factorio::snapshot::WorldSnapshot;

    let with_census = r#"{
        "entity_prototypes": [], "item_prototypes": [], "recipes": [],
        "forces": [], "daylight": null,
        "surfaces": [
            {"index": 1, "name": "nauvis", "planet": "nauvis"},
            {"index": 2, "name": "platform-1"}
        ]
    }"#;
    let snapshot: WorldSnapshot = serde_json::from_str(with_census).expect("the census parses");
    // Non-accidental control: the parse really produced a snapshot, so the
    // assertion below is about the census and not about an empty success.
    assert!(snapshot.recipes.is_empty());
    assert!(snapshot.entity_prototypes.is_empty());

    let census = snapshot
        .surfaces
        .expect("the census is carried, not dropped");
    assert_eq!(census.len(), 2);
    assert_eq!(census[0].name, "nauvis");
    assert_eq!(census[0].planet.as_deref(), Some("nauvis"));
    assert_eq!(census[1].name, "platform-1");
    assert_eq!(
        census[1].planet, None,
        "a platform is a surface that is not a planet -- a real answer, not a gap",
    );
}

/// **The stdout half, which had to land in the same commit as its parser
/// arm.** `output_parser.rs` logs `unexpected action: <key>` as an *error* for
/// a writeout key it has no arm for, so a mod emitting the census before Rust
/// could receive it would have put a red line in every run that looks like a
/// defect and is not. `control.lua` carried a comment saying exactly that at
/// the point this function now occupies.
///
/// Asserted through `writeout_initial_stuff`, not by calling
/// `writeout_surfaces` directly: a function nobody calls emits nothing, and
/// that is the failure this pins.
#[test]
fn the_census_is_written_out_at_init_under_its_own_key() {
    let lua = mod_lua();
    lua.load(
        r#"
        game.surfaces = {
            { name = "nauvis", index = 1, planet = { name = "nauvis" } },
            { name = "platform-1", index = 2 },
        }
        writeout_surfaces()
        "#,
    )
    .set_name("writeout_surfaces")
    .exec()
    .expect("writeout_surfaces");

    let lines = writeouts(&lua, "surfaces");
    assert_eq!(lines.len(), 1, "one line carrying the whole census");
    assert_eq!(
        lines[0],
        r#"{"1":{"index":1,"name":"nauvis","planet":"nauvis"},"2":{"index":2,"name":"platform-1"}}"#,
        "the same record `collect_surfaces` gives the RCON half -- one shape, two transports",
    );
}

/// `writeout_initial_stuff` is what a run actually calls, and a
/// `writeout_surfaces` that exists but is never called emits nothing.
#[test]
fn writeout_initial_stuff_emits_the_census() {
    let lua = mod_lua();
    lua.load(
        r#"
        game.surfaces = { { name = "nauvis", index = 1, planet = { name = "nauvis" } } }
        -- Everything else `writeout_initial_stuff` reaches for is stubbed out;
        -- what is under test is that the census is among the lines it emits.
        writeout_pictures = noop
        writeout_entity_prototypes = noop
        writeout_item_prototypes = noop
        writeout_recipes = noop
        writeout_forces = noop
        writeout_daylight = noop
        writeout_initial_stuff()
        "#,
    )
    .set_name("writeout_initial_stuff")
    .exec()
    .expect("writeout_initial_stuff");

    assert_eq!(
        writeouts(&lua, "surfaces").len(),
        1,
        "the census is emitted at init, not merely definable",
    );
    // Non-accidental control from the same call: the envelope really ran, so
    // the assertion above is about the census and not about a silent no-op.
    assert_eq!(
        writeouts(&lua, "STATIC_DATA_END").len(),
        1,
        "the init envelope itself ran",
    );
}
