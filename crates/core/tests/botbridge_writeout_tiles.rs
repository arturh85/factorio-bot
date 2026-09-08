//! **`writeout_tiles` flagged every tile walkable, so water was never solid.**
//!
//! The stdout transport is the *only* path that ever fills
//! `EntityGraph`'s tile and blocked trees: `writeout_tiles` ->
//! `output_parser.rs`'s `"tiles"` arm -> `update_chunk_tiles` ->
//! `EntityGraph::add_tiles`, which inserts a blocking box only when
//! `player_collidable` is true. `writeout_tiles` wrote `tile.name .. ":0"` --
//! the flag hardcoded -- under a standing TODO claiming Factorio 2.0 had
//! changed the collision-layer API beyond use.
//!
//! It had not. `LuaTile::collides_with(CollisionLayerID) -> boolean` is right
//! there in `workspace/factorio-api-docs/runtime-api.json` for 2.1.17, and
//! `mods/BotBridge/types.lua`'s `serialize_tile` has been calling it correctly
//! on the RCON path the whole time. What 2.0 changed was the *layer name*:
//! `player-layer` became `player`, and the old spelling does not degrade to
//! `false`, it raises. That is the trap the TODO half-noticed and then papered
//! over on the wrong side.
//!
//! The cost is measurable in the archive rather than inferred: across the five
//! `workspace/*-log.txt` transcripts, **4,440,064 tile records, every one of
//! them flagged `0`**, including 79,717 `water` and 330,346 `deepwater`. No
//! water tile has ever entered `blocked_tree` in any owned run, so
//! `PlanState::is_area_clear` would approve a boiler standing in a lake.
//!
//! See `docs/superpowers/notes/2026-09-02-water-is-solid.md`.

use mlua::{Lua, LuaOptions, StdLib, Table};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A stub game whose tiles answer `collides_with` the way 2.1.17 does.
///
/// Two things here are load-bearing:
///
/// * `collides_with` **raises** on any layer name that is not a real 2.1
///   collision layer, exactly as the engine does ("Unknown collision-layer
///   name: player-layer"). A stub that returned `false` for a wrong name would
///   let the pre-2.0 spelling pass as "nothing collides", which is the failure
///   this whole file exists to make impossible.
/// * every call is counted, and the layer it was asked about recorded, so a
///   test can assert *what* was asked and not only what came back.
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
    helpers = {
        table_to_json = function() return "{}" end,
        write_file = noop,
        remove_path = noop,
    }

    -- The collision layers a vanilla 2.1 game defines that this mod could
    -- plausibly name. `player-layer` is deliberately absent: it is the 1.1
    -- spelling and naming it must raise.
    _real_layers = { player = true, object = true, water_tile = true,
                     ground_tile = true, is_lower_object = true }

    -- What the map looks like. Water and deepwater collide with `player`;
    -- grass does not. Taken from the live capture in
    -- crates/core/tests/live-2.1.17-tiles.json, which came through the RCON
    -- path and so shows the flags the game really reports.
    _collidable_names = { water = true, deepwater = true, ["out-of-map"] = true }

    _collides_calls = {}

    _tile_names = {}
    function set_tile(x, y, name) _tile_names[x .. "/" .. y] = name end

    -- What is HIDDEN under a tile: landfill laid over a lake keeps pumping
    -- water, which is why `tile_fluid` walks visible -> hidden -> double
    -- hidden at all.
    _hidden_tiles = {}
    function set_hidden(x, y, name) _hidden_tiles[x .. "/" .. y] = name end
    _double_hidden_tiles = {}
    function set_double_hidden(x, y, name)
        _double_hidden_tiles[x .. "/" .. y] = name
    end

    -- Every distinct prototype lookup, so a test can assert the fluid is asked
    -- once per NAME and not once per tile.
    _tile_proto_lookups = {}

    game = {
        tick = 0,
        players = {},
        connected_players = {},
        forces = {},
        surfaces = {},
        take_screenshot = noop,
    }
    storage = { n_clients = 0, p = {} }
    -- `prototypes.tile[name].fluid` is what says which fluid an offshore pump
    -- on this tile would draw -- the prototype-side answer to a question the
    -- runtime `get_fluid_source_fluid` can only answer for a pump that already
    -- stands. Vanilla gives it to `water` and `deepwater` and to nothing else;
    -- a real Space Age game also gives it to `ammoniacal-ocean` and to lava,
    -- which is exactly why the mod sends the fluid's NAME rather than a flag.
    prototypes = {
        item = {},
        entity = {},
        tile = setmetatable({}, { __index = function(_, name)
            _tile_proto_lookups[#_tile_proto_lookups + 1] = tostring(name)
            if name == "water" or name == "deepwater" then
                return { name = name, fluid = { name = "water" } }
            end
            return { name = name }
        end }),
    }

    _surface = {
        -- `ground_header` reads this: since 2026-09-07 a tiles line names the
        -- surface it was read off, so ground can be routed the way entities
        -- already are.
        name = "nauvis",
        -- One call per chunk, not 1024: `writeout_tiles` only pays for the
        -- per-tile hidden-tile walk when the area actually holds a covered
        -- tile.
        count_tiles_filtered = function(filters)
            local n = 0
            if filters.has_hidden_tile then
                for y = filters.area.left_top.y, filters.area.right_bottom.y - 1 do
                    for x = filters.area.left_top.x, filters.area.right_bottom.x - 1 do
                        if _hidden_tiles[x .. "/" .. y] then n = n + 1 end
                    end
                end
            end
            return n
        end,
        get_tile = function(x, y)
            local name = _tile_names[x .. "/" .. y] or "grass-1"
            return {
                name = name,
                position = { x = x, y = y },
                hidden_tile = _hidden_tiles[x .. "/" .. y],
                double_hidden_tile = _double_hidden_tiles[x .. "/" .. y],
                collides_with = function(layer)
                    _collides_calls[#_collides_calls + 1] =
                        { layer = tostring(layer), name = name }
                    if not _real_layers[layer] then
                        error("Unknown collision-layer name: " .. tostring(layer))
                    end
                    return _collidable_names[name] == true
                end,
            }
        end,
    }

    function area(x1, y1, x2, y2)
        return { left_top = { x = x1, y = y1 }, right_bottom = { x = x2, y = y2 } }
    end
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

/// The payload of the one `tiles` record the mod wrote, on the wire form
/// `crates/core/src/process/output_parser.rs` reads (`§<tick>§tiles§<body>`).
fn tiles_record(lua: &Lua) -> String {
    let mut records: Vec<String> = lua
        .globals()
        .get::<Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .filter(|line| line.contains("§tiles§"))
        .collect();
    assert_eq!(
        records.len(),
        1,
        "expected one tiles record, got {records:?}"
    );
    let line = records.pop().expect("one record");
    line.splitn(3, '§')
        .nth(2)
        .expect("§tick§tiles§body")
        .trim_start_matches("tiles§")
        .to_string()
}

/// Every `collides_with` call the mod made, as `(layer, tile name)`.
fn collides_calls(lua: &Lua) -> Vec<(String, String)> {
    lua.globals()
        .get::<Table>("_collides_calls")
        .expect("_collides_calls")
        .sequence_values::<Table>()
        .map(|v| {
            let t = v.expect("call");
            (
                t.get::<String>("layer").expect("layer"),
                t.get::<String>("name").expect("name"),
            )
        })
        .collect()
}

/// **The defect.** Water and deepwater have to reach stdout flagged solid, or
/// they never enter `blocked_tree` and the planner sites buildings in lakes.
#[test]
fn water_and_deepwater_are_written_out_as_solid() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "deepwater")
        set_tile(0, 1, "grass-1")
        set_tile(1, 1, "grass-1")
        writeout_tiles(7, _surface, area(0, 0, 2, 2))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tiles_record(&lua),
        "0,0;2,2;nauvis: water:1:water,deepwater:1:water,grass-1:0:,grass-1:0:",
        "the flag after each tile name is what output_parser.rs turns into \
         FactorioTile::player_collidable, and EntityGraph::add_tiles inserts a \
         blocking box only when it is true. Hardcoding it to 0 -- which every \
         one of the 4,440,064 tile records in workspace/*-log.txt does -- means \
         no lake has ever blocked anything."
    );
}

/// The name of the layer, asked separately, because getting it wrong is the
/// mistake the removed TODO was written around. 2.0 renamed `player-layer` to
/// `player` and raises on the old spelling.
#[test]
fn the_collision_layer_is_asked_for_by_its_2_0_name() {
    let lua = mod_lua();
    lua.load(r#"set_tile(0, 0, "water") writeout_tiles(0, _surface, area(0, 0, 1, 1))"#)
        .set_name("writeout_tiles")
        .exec()
        .expect("a wrong layer name raises rather than reading false");

    let calls = collides_calls(&lua);
    assert_eq!(
        calls,
        vec![("player".to_string(), "water".to_string())],
        "types.lua's serialize_tile already asks `collides_with('player')`; \
         the two transports have to agree or the same lake is solid over RCON \
         and walkable over stdout"
    );
}

/// The negative control. Ground the character can walk on must stay flagged
/// walkable -- a fix that hardcoded `1` instead of `0` would make every tile
/// on the map a blocking box and refuse every build.
#[test]
fn walkable_ground_is_still_written_out_as_walkable() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "grass-1")
        set_tile(1, 0, "sand-1")
        writeout_tiles(0, _surface, area(0, 0, 2, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tiles_record(&lua),
        "0,0;2,1;nauvis: grass-1:0:,sand-1:0:",
        "grass and sand collide with nothing; flagging them solid would put a \
         blocking box under every tile of the map"
    );
}

/// `writeout_tiles`' own comment calls it SLOW ("beastie can do ~2.8 per
/// tick"), and a chunk is 1024 tiles. Collision is a property of the tile
/// *prototype* and a tile's name names its prototype exactly, so the mod asks
/// the engine once per distinct name rather than once per tile.
#[test]
fn the_collision_flag_is_asked_once_per_tile_prototype_not_once_per_tile() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "water")
        set_tile(0, 1, "grass-1")
        set_tile(1, 1, "grass-1")
        writeout_tiles(0, _surface, area(0, 0, 2, 2))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    let calls = collides_calls(&lua);
    assert_eq!(
        calls,
        vec![
            ("player".to_string(), "water".to_string()),
            ("player".to_string(), "grass-1".to_string()),
        ],
        "four tiles, two prototypes, two crossings of the mod/engine boundary"
    );

    // And the memoised answer still has to be the right one per name.
    assert_eq!(
        tiles_record(&lua),
        "0,0;2,2;nauvis: water:1:water,water:1:water,grass-1:0:,grass-1:0:"
    );
}

/// End to end, across the seam: the mod's own stdout line, through the real
/// `OutputParser`, into the real `EntityGraph`.
///
/// The two halves are pinned separately above, but neither one alone says the
/// flag *arrives*. `output_parser.rs` reads `parts[1].parse::<u8>() == 1` and
/// `EntityGraph::add_tiles` inserts into `blocked_tree` only when
/// `player_collidable` is true, so a wire form the parser mis-reads would leave
/// both unit tests green and the lake still walkable.
#[test]
fn the_mods_own_tiles_line_makes_water_block_and_leaves_grass_open() {
    use factorio_bot_core::graph::entity_graph::EntityGraph;
    use factorio_bot_core::process::output_parser::OutputParser;
    use factorio_bot_core::types::{Position, Rect};

    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "grass-1")
        writeout_tiles(11, _surface, area(0, 0, 2, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");
    let body = tiles_record(&lua);

    let mut parser = OutputParser::new();
    parser.parse(11, "tiles", &body).expect("tiles line parses");
    let graph: &EntityGraph = &parser.world().entity_graph;

    // add_tiles stores a tile as the 1x1 box [pos, pos+1), so the water tile at
    // (0, 0) blocks the unit square the game would refuse a build in.
    let over_water = graph.blocking_boxes_within(&Rect::new(
        &Position::new(0.1, 0.1),
        &Position::new(0.9, 0.9),
    ));
    assert_eq!(
        over_water.len(),
        1,
        "a water tile has to reach blocked_tree, or PlanState::is_area_clear \
         approves a boiler standing in the lake. Got {over_water:?}"
    );

    let over_grass = graph.blocking_boxes_within(&Rect::new(
        &Position::new(1.1, 0.1),
        &Position::new(1.9, 0.9),
    ));
    assert!(
        over_grass.is_empty(),
        "and grass must still be open ground, or nothing can be built anywhere. \
         Got {over_grass:?}"
    );
}

/// **The header names the surface it was READ OFF, not a constant.**
///
/// `writeout_tiles` has always been handed a `surface` and, until 2026-09-07,
/// never put it on the wire -- so ground was the last position-keyed writeout
/// that could not say where it came from, and the mod's Nauvis guard in
/// `on_chunk_generated` was the only thing preventing a second surface's tiles
/// from merging into Nauvis by position alone.
///
/// The two halves matter together. A hard-coded `nauvis` would satisfy every
/// other test in this file, all of which run on the default stub; only reading
/// a *different* surface back distinguishes "the field is filled in" from "the
/// field is filled in from the argument". And the same tile bodies are asserted
/// either side, so the surface is the only thing that moved.
#[test]
fn the_tiles_header_names_the_surface_it_was_read_off() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        _surface.name = "vulcanus"
        writeout_tiles(3, _surface, area(0, 0, 1, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tiles_record(&lua),
        "0,0;1,1;vulcanus: water:1:water",
        "the third header field is the surface the tiles were read off, which \
         is what output_parser.rs routes on"
    );

    let lua = mod_lua();
    lua.load(r#"set_tile(0, 0, "water") writeout_tiles(3, _surface, area(0, 0, 1, 1))"#)
        .set_name("writeout_tiles")
        .exec()
        .expect("writeout_tiles");
    assert_eq!(
        tiles_record(&lua),
        "0,0;1,1;nauvis: water:1:water",
        "and the default stub still says nauvis -- the two lines differ in the \
         surface and in nothing else"
    );
}

// ---------------------------------------------------------------------------
// A THIRD FIELD: which fluid an offshore pump on this tile would draw
// ---------------------------------------------------------------------------
//
// A Factorio 2.0 offshore pump takes its fluid from the tile, not from its own
// prototype -- its output fluidbox is unfiltered, measured across all 56 boxes
// in this mod set. So "what does a pump here produce" is a question about
// ground, and `LuaTilePrototype::fluid` is the only thing that answers it
// without a pump already standing.

/// **`grass-1:0:` and `grass-1:0:?` are different facts, and the wire has to
/// keep them apart.**
///
/// This is the whole reason the field is three-valued. An empty third field
/// says *this tile yields nothing* -- which is what lets a planner refuse "put
/// a pump here" on a fact. A `?` says *we could not tell*, which must attribute
/// nothing and refuse nothing. An `Option<String>` on the Rust side, or a
/// single "absent" form here, would merge them and turn every unreadable tile
/// into dry land.
#[test]
fn a_dry_tile_says_so_and_is_not_the_same_as_an_unreadable_one() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "grass-1")
        writeout_tiles(0, _surface, area(0, 0, 2, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tiles_record(&lua),
        "0,0;2,1;nauvis: water:1:water,grass-1:0:",
        "water NAMES the fluid it yields -- not a flag, because Space Age's \
         ammoniacal ocean yields ammonia and Vulcanus' lava yields lava -- and \
         grass says, definitely, that it yields nothing"
    );
}

/// **A lake under landfill still pumps water, and the visible name cannot say
/// so.**
///
/// `LuaEntity::get_fluid_source_fluid` is documented as accounting for "visible
/// tile, hidden tile and double hidden tile"; `tile_fluid` walks the same three
/// in the same order. Reading only the visible `landfill` here would report
/// `dry` -- a confident wrong answer, which is worse than `?`.
#[test]
fn a_covered_lake_still_names_the_fluid_underneath_it() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "landfill")
        set_hidden(0, 0, "water")
        set_tile(1, 0, "landfill")
        set_double_hidden(1, 0, "deepwater")
        set_tile(2, 0, "landfill")
        writeout_tiles(0, _surface, area(0, 0, 3, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tiles_record(&lua),
        "0,0;3,1;nauvis: landfill:0:water,landfill:0:water,landfill:0:",
        "the first tile's hidden water and the second's double-hidden \
         deepwater both surface; the third is landfill over nothing and is \
         honestly dry"
    );
}

/// **The hidden-tile walk costs ONE call per chunk, not 1024.**
///
/// `writeout_tiles` is marked SLOW in its own comment and runs over a 32x32
/// chunk, so a per-tile `hidden_tile` read would add 1024 crossings of the
/// mod/engine boundary to the function least able to afford them.
/// `count_tiles_filtered{has_hidden_tile = true}` answers for the whole area
/// at once, and on ground the game has only just generated the answer is zero.
///
/// Asserted through the prototype-lookup counter rather than by timing: with
/// no covered tile, the fluid is asked once per distinct tile NAME. The
/// control is the same map with one hidden tile, where the per-tile walk does
/// run -- without it this test would pass for a version that never looked at
/// hidden tiles at all.
#[test]
fn the_fluid_is_asked_once_per_tile_prototype_when_nothing_is_covered() {
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "water")
        set_tile(0, 1, "grass-1")
        set_tile(1, 1, "grass-1")
        writeout_tiles(0, _surface, area(0, 0, 2, 2))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    assert_eq!(
        tile_proto_lookups(&lua),
        vec!["water".to_string(), "grass-1".to_string()],
        "four tiles, two prototypes, two crossings"
    );

    // The control: one covered tile, and the walk runs -- so the memoised
    // answer is genuinely gated on the chunk-level question and not simply
    // never reached.
    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "landfill")
        set_hidden(0, 0, "water")
        set_tile(1, 0, "landfill")
        writeout_tiles(0, _surface, area(0, 0, 2, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");
    assert_eq!(
        tiles_record(&lua),
        "0,0;2,1;nauvis: landfill:0:water,landfill:0:",
        "the covered tile's water surfaces only because the chunk-level check \
         found it"
    );
}

/// Every distinct `prototypes.tile[...]` lookup the mod made, in order.
fn tile_proto_lookups(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<Vec<String>>("_tile_proto_lookups")
        .expect("_tile_proto_lookups")
}

/// End to end across the seam, the way `the_mods_own_tiles_line_makes_water_
/// block_and_leaves_grass_open` does for the collision flag: the mod's own
/// stdout line, through the real `OutputParser`, into the real `EntityGraph`,
/// and out of `fluid_at`.
///
/// Neither the mod test nor the parser test alone says the fluid *arrives*.
/// This is the only assertion in the file that would fail if the wire form and
/// the parser disagreed about which `:`-separated field the fluid is in.
#[test]
fn the_mods_own_tiles_line_makes_water_yield_water_and_grass_yield_nothing() {
    use factorio_bot_core::graph::entity_graph::EntityGraph;
    use factorio_bot_core::process::output_parser::OutputParser;
    use factorio_bot_core::types::{Position, TileFluid};

    let lua = mod_lua();
    lua.load(
        r#"
        set_tile(0, 0, "water")
        set_tile(1, 0, "grass-1")
        writeout_tiles(11, _surface, area(0, 0, 2, 1))
        "#,
    )
    .set_name("writeout_tiles")
    .exec()
    .expect("writeout_tiles");

    let body = tiles_record(&lua);
    let mut parser = OutputParser::new();
    parser
        .parse(11, "tiles", &body)
        .expect("the mod's own line parses");
    let graph: &EntityGraph = &parser.world().entity_graph;

    assert_eq!(
        graph.fluid_at(&Position::new(0.5, 0.5)),
        TileFluid::Yields {
            fluid: "water".to_owned()
        },
        "a pump in front of this tile draws water"
    );
    assert_eq!(
        graph.fluid_at(&Position::new(1.5, 0.5)),
        TileFluid::Dry,
        "and this one is charted and definitely yields nothing"
    );
    assert_eq!(
        graph.fluid_at(&Position::new(50.5, 50.5)),
        TileFluid::Unknown,
        "while ground nobody charted is UNKNOWN and never dry -- reading it as \
         dry would let a planner refuse a lake it has simply never walked to"
    );
}

/// **An older sender's two-field tile is `Unknown`, never `Dry`.**
///
/// This is not hypothetical and it is not rare: it is **every archived server
/// log and both world dumps**, 4,440,064 tile records that were written before
/// this field existed. Reading them as dry would assert that no map this
/// project has ever run on has any water on it -- the exact
/// `absent-is-not-a-value` collapse the three-state type exists to prevent.
///
/// The three forms are asserted against one another in a single test on
/// purpose: a version that merged any two of them would still pass two of the
/// three assertions on its own.
#[test]
fn a_two_field_tile_is_unknown_and_a_question_mark_is_too() {
    use factorio_bot_core::process::output_parser::OutputParser;
    use factorio_bot_core::types::{Position, TileFluid};

    let mut parser = OutputParser::new();
    parser
        .parse(
            1,
            "tiles",
            // Three tiles in a row: an old sender's two fields, an explicit
            // "could not tell", and an explicit "yields nothing".
            "0,0;3,1;nauvis: grass-1:0,grass-1:0:?,grass-1:0:",
        )
        .expect("a mixed line parses");
    let graph = &parser.world().entity_graph;

    assert_eq!(
        graph.fluid_at(&Position::new(0.5, 0.5)),
        TileFluid::Unknown,
        "two fields means the sender did not say"
    );
    assert_eq!(
        graph.fluid_at(&Position::new(1.5, 0.5)),
        TileFluid::Unknown,
        "and `?` means the sender looked and could not tell -- the same \
         inertness, reached differently"
    );
    assert_eq!(
        graph.fluid_at(&Position::new(2.5, 0.5)),
        TileFluid::Dry,
        "only an EMPTY third field is the claim that this tile yields nothing"
    );
}
