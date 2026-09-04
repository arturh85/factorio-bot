//! **Does a placement that fails still cost the bot its material?**
//!
//! `rcon_place_entity` in `mods/BotBridge/control.lua` used to `remove_item`
//! and then `create_entity`, throwing both return values away. Verified against
//! `workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17, runtime api
//! 6): `LuaSurface::create_entity` returns "the created entity or `nil` if the
//! creation failed", and `LuaControl::remove_item` returns "the number of items
//! that were actually removed". Discarding both gives two failure modes that
//! cancel out in the logs -- a failed placement destroys the material, and a
//! removal that moved nothing builds for free.
//!
//! The same file's `build_blueprint` calls passed `force_build`, which 2.0
//! replaced with `build_mode :: defines.build_mode`. An unknown key in a
//! `takes_table` call is not read, so the flag was silently ignored and the
//! inherited default (`normal`, all-or-nothing) is the *opposite* of what
//! `force_build = true` asked for.
//!
//! Neither of those is visible from Rust: the mod's Lua only ever runs inside
//! Factorio. So these tests load the real `control.lua` into a Lua 5.4 state on
//! top of a stub game that **counts what the handler did to the inventory and
//! to the world**, which is the part no reply body reveals.
//!
//! What this cannot prove: that Factorio itself accepts `build_mode`, or that
//! `forced` behaves as documented in a live game. See
//! `docs/superpowers/notes/2026-09-02-mod-placement-correctness.md`.

use mlua::{Lua, LuaOptions, StdLib};

// The same two files a debug run loads out of the checkout. `repo_mods_path!`
// (crates/core/src/process/instance_setup.rs) is `pub(crate)` and so out of
// reach from an integration test; this resolves the identical
// `CARGO_MANIFEST_DIR`-relative path, so the bytes compiled in here are the
// bytes a debug run compares `workspace/mods` against.
const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// The tick the stub game is frozen at. Any value works; a recognisable one
/// makes a failure message readable.
const STUB_TICK: u64 = 64738;

/// Enough of Factorio's Lua API for `control.lua` to load. Everything the
/// individual tests care about is appended after this.
const PRELUDE: &str = r#"
    -- `defines.*` is read at load time and, for build_mode, compared by
    -- identity below; an auto-vivifying table gives every name a stable,
    -- distinct value without enumerating the real tree.
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
    print = noop

    -- Only `table_to_json` matters: the success reply has to start with `{`
    -- or `place_entity_timed` reads it as a refusal.
    helpers = setmetatable(
        { table_to_json = function(t) return '{"name":"stone-furnace"}' end },
        { __index = function() return noop end })

    -- The RCON reply body under construction.
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }
"#;

/// The workspace forbids building an interpreter outside
/// `scripting_lua::sandbox`, and rightly: that one runs *user* scripts. This
/// one runs two files from this repository with no path by which a caller
/// could substitute another, and `scripting_lua` depends on this crate so the
/// sandbox cannot be reached from here.
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

/// Loads the stub game, the real mod, and then `after` (a chance to override a
/// global the mod defined), and runs `call`.
fn run(stub: &str, after: &str, call: &str) -> Lua {
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
    lua.load(after)
        .set_name("after")
        .exec()
        .expect("post-load overrides");
    lua.load(call)
        .set_name("call")
        .exec()
        .expect("handler call");
    lua
}

fn number(lua: &Lua, name: &str) -> i64 {
    lua.globals().get(name).unwrap_or_else(|err| {
        panic!("reading {name}: {err}");
    })
}

fn rcon_lines(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>("_rcon_lines")
        .expect("_rcon_lines")
        .sequence_values::<String>()
        .map(|v| v.expect("rcon line"))
        .collect()
}

/// The reply as `place_entity_timed` reads it: the tick stamp is lifted off by
/// `take_tick_stamp` and exactly one line has to remain.
fn one_line_reply(lua: &Lua) -> String {
    let lines = rcon_lines(lua);
    assert_eq!(
        lines.len(),
        2,
        "one message plus one tick stamp; `place_entity_timed` rejects any \
         other shape as unexpected output. Got {lines:?}"
    );
    assert_eq!(
        lines[1],
        format!("§tick§{STUB_TICK}"),
        "every exit stamps the tick it was judged at"
    );
    lines[0].clone()
}

/// Enough of the API for `rcon_place_entity` to run all the way to the
/// create/charge pair, counting what it did on the way.
///
/// `create_ok` decides whether `create_entity` hands back an entity;
/// `removes` is what `remove_item` reports it actually took.
fn stub_place(create_ok: bool, removes: i64) -> String {
    format!(
        r#"
        _created = 0
        _destroyed = 0
        _charged = 0
        _charged_count = 0

        local entity = {{
            name = "stone-furnace",
            destroy = function() _destroyed = _destroyed + 1 end,
        }}
        local surface = {{
            can_place_entity = function(args) return true end,
            create_entity = function(args)
                _created = _created + 1
                if {create_ok} then return entity end
                return nil
            end,
            find_entity = function(name, pos) return nil end,
        }}
        local player = {{
            name = "bot1",
            connected = true,
            character = {{}},
            position = {{ x = 38.3046875, y = 16.4765625 }},
            force = "player",
            surface = surface,
            get_item_count = function(name) return 1 end,
            remove_item = function(items)
                _charged = _charged + 1
                _charged_count = items.count
                return {removes}
            end,
        }}
        prototypes = {{ item = {{
            ["stone-furnace"] = {{ place_result = {{
                name = "stone-furnace",
                collision_box = {{
                    left_top = {{ x = -0.9, y = -0.9 }},
                    right_bottom = {{ x = 0.9, y = 0.9 }},
                }},
            }} }},
        }} }}
        game = {{
            tick = {tick},
            players = {{ player }},
            forces = {{ player = {{ print = noop }} }},
        }}
    "#,
        create_ok = if create_ok { "true" } else { "false" },
        removes = removes,
        tick = STUB_TICK,
    )
}

/// `serialize_entity` wants a whole `LuaEntity`; what is under test here is the
/// inventory arithmetic, and the serialisers have their own suite in
/// `botbridge_serialisers.rs`.
const STUB_SERIALISE: &str = "serialize_entity = function(e) return {} end";

const PLACE_FURNACE: &str = r#"rcon_place_entity(1, "stone-furnace", {38, 16}, 0)"#;

/// **The bug, stated as a test: a placement the game refuses must not eat the
/// item.**
///
/// `create_entity` returning nil is the dominant failure mode in this
/// project's runs, so this was the well travelled path. Nothing may be charged
/// on it.
#[test]
fn a_failed_creation_charges_the_player_nothing() {
    let lua = run(&stub_place(false, 1), STUB_SERIALISE, PLACE_FURNACE);
    assert_eq!(number(&lua, "_created"), 1, "the handler did try to build");
    assert_eq!(
        number(&lua, "_charged"),
        0,
        "the item must still be in the bot's inventory: nothing was built"
    );
    let line = one_line_reply(&lua);
    assert!(
        line.contains("surface.create_entity returned nil"),
        "the caller is told the creation failed, got {line:?}"
    );
}

/// The other half of the same pair. `remove_item` reporting zero means the
/// affordability check and the spend disagreed, and the entity standing there
/// was never paid for -- so it is undone rather than kept.
#[test]
fn a_charge_that_takes_nothing_undoes_the_build() {
    let lua = run(&stub_place(true, 0), STUB_SERIALISE, PLACE_FURNACE);
    assert_eq!(number(&lua, "_created"), 1);
    assert_eq!(number(&lua, "_charged"), 1, "the charge was attempted");
    assert_eq!(
        number(&lua, "_destroyed"),
        1,
        "an entity nobody paid for must not be left standing"
    );
}

/// **What the caller sees has to stay sortable.**
///
/// `crates/core/src/factorio/rcon.rs` reads this reply three ways: JSON is a
/// success, `§player_blocks_placement§` makes it walk the bot aside and retry,
/// and `can_place_entity said 'no'` is remembered by `note_placement_refusal`
/// as a durable fact about the *ground* -- fencing the planner out of that
/// footprint for the rest of the run. An empty hand is none of those: the site
/// may be perfectly fine.
#[test]
fn an_unpaid_placement_reads_as_a_material_refusal_not_a_site_refusal() {
    let lua = run(&stub_place(true, 0), STUB_SERIALISE, PLACE_FURNACE);
    let line = one_line_reply(&lua);
    assert!(
        line.contains("cannot place item 'stone-furnace'") && line.contains("removed nothing"),
        "the refusal must name the item and say the spend took nothing, got {line:?}"
    );
    assert!(
        !line.contains("can_place_entity said 'no'"),
        "this is not a verdict about the ground; matching that wording would \
         fence the planner out of a site the game never refused. Got {line:?}"
    );
    assert_ne!(
        line, "§player_blocks_placement§",
        "nor is it the sentinel that makes the RCON layer walk and retry"
    );
    assert!(
        !line.starts_with('{'),
        "and it must not be read as a successful placement"
    );
}

/// The ordinary path, kept honest: exactly one item leaves the inventory,
/// nothing is destroyed, and the reply is the serialised entity.
#[test]
fn a_successful_placement_charges_exactly_one_item() {
    let lua = run(&stub_place(true, 1), STUB_SERIALISE, PLACE_FURNACE);
    assert_eq!(number(&lua, "_charged"), 1);
    assert_eq!(number(&lua, "_charged_count"), 1);
    assert_eq!(
        number(&lua, "_destroyed"),
        0,
        "a paid-for build stays standing"
    );
    let line = one_line_reply(&lua);
    assert!(
        line.starts_with('{'),
        "a success is the entity as json, got {line:?}"
    );
}

/// Enough of the API for both blueprint handlers to reach `build_blueprint`,
/// recording the argument table it was handed.
///
/// The blueprint builds no ghosts, so both handlers stop at "failed to build
/// anything" -- everything past the call is out of scope here.
fn stub_blueprint() -> String {
    format!(
        r#"
        _build_args = nil
        storage = {{ p = {{ [1] = {{}} }} }}

        local bp_entity = {{
            stack = {{
                import_stack = function(bp) return 0 end,
                build_blueprint = function(args) _build_args = args; return {{}} end,
            }},
            destroy = function() end,
        }}
        local surface = {{
            create_entity = function(args) return bp_entity end,
        }}
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
    "#,
        tick = STUB_TICK,
    )
}

fn asked_build_mode(call: &str) -> (bool, bool, bool) {
    let lua = run(&stub_blueprint(), "", call);
    let is = |expr: &str| -> bool {
        lua.load(expr)
            .eval::<bool>()
            .unwrap_or_else(|err| panic!("evaluating {expr}: {err}"))
    };
    (
        is("return _build_args.force_build ~= nil"),
        is("return _build_args.build_mode == defines.build_mode.forced"),
        is("return _build_args.build_mode == defines.build_mode.normal"),
    )
}

/// **A flag the game does not read is worse than no flag at all.**
///
/// 1.1's `force_build = true` meant "build everything you can". 2.x reads
/// `build_mode`, so the boolean went nowhere and the *default* applied --
/// `normal`, all-or-nothing, the opposite request. Both blueprint handlers are
/// driven here because they had the same line and must not drift apart.
#[test]
fn force_build_reaches_the_game_as_defines_build_mode_forced() {
    for call in [
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, true, true, {})"#,
        r#"rcon_cheat_blueprint(1, "bp", 0, 0, 0, true)"#,
    ] {
        let (has_force_build, forced, normal) = asked_build_mode(call);
        assert!(
            !has_force_build,
            "`force_build` is not a parameter of LuaItemCommon::build_blueprint \
             in 2.1.17; sending it is sending nothing ({call})"
        );
        assert!(
            forced && !normal,
            "force_build = true must arrive as defines.build_mode.forced ({call})"
        );
    }
}

/// And the other way round, so the translation is a translation and not a
/// constant.
#[test]
fn no_force_build_reaches_the_game_as_defines_build_mode_normal() {
    for call in [
        r#"rcon_place_blueprint(1, "bp", 0, 0, 0, false, true, {})"#,
        r#"rcon_cheat_blueprint(1, "bp", 0, 0, 0, false)"#,
    ] {
        let (has_force_build, forced, normal) = asked_build_mode(call);
        assert!(!has_force_build, "still no `force_build` key ({call})");
        assert!(
            normal && !forced,
            "force_build = false must arrive as defines.build_mode.normal ({call})"
        );
    }
}

// ---------------------------------------------------------------------------
// A refused placement: is the refusal about the *ground*, or about a bot that
// is standing there and will walk away?
// ---------------------------------------------------------------------------

/// The site run 24 (`run-1788347034-00981`) was refused at, and the two bot
/// positions the run's `samples.jsonl` recorded eleven ticks before the
/// dispatch. Real coordinates rather than invented ones, so a reader can put
/// the test beside the record.
const SITE: &str = "{-21, 24}";
/// Bot 4, the acting bot: nowhere near the footprint.
const ACTOR_AWAY: (f64, f64) = (-16.39, 20.46);
/// Bot 3, parked where servicing its own furnace at `[-23, 24]` left it.
/// A character's collision box is `±0.2` by `±0.3`, so this box is
/// `[-21.67, -21.27] x [23.43, 24.03]` — inside the furnace footprint
/// `[-21.9, -20.1] x [23.1, 24.9]` by a third of a tile in x.
///
/// `player` is the index the stub's `game.players` answers to, which is how
/// the mod gets from a character entity to something it can walk. A character
/// with no player behind it is [`PARKED_STRANGER`].
const PARKED_BOT: &str = r#"
    { name = "character", type = "character", player_index = 3,
      position = { x = -21.47, y = 23.73 },
      bounding_box = {
        left_top = { x = -21.67, y = 23.43 },
        right_bottom = { x = -21.27, y = 24.03 } } }
"#;
/// The same character with nothing behind it — no player, so nothing to ask.
/// Physically identical, and the mod must neither move it nor raise.
const PARKED_STRANGER: &str = r#"
    { name = "character", type = "character",
      position = { x = -21.47, y = 23.73 },
      bounding_box = {
        left_top = { x = -21.67, y = 23.43 },
        right_bottom = { x = -21.27, y = 24.03 } } }
"#;
/// Something that is *not* going to walk away, in the same footprint.
const TREE: &str = r#"
    { name = "tree-01", type = "tree", bounding_box = {
        left_top = { x = -21.6, y = 23.6 },
        right_bottom = { x = -21.2, y = 24.0 } } }
"#;

/// A stub whose `can_place_entity` refuses, so what is under test is the
/// branch that decides *what kind* of refusal to report.
///
/// `occupants` is Lua source for the list the surface holds;
/// `find_entities_filtered` intersects them against the area it is given,
/// exactly as the real one does, and records that area so the test can pin
/// which box was asked about.
fn stub_refused_place(player_position: (f64, f64), occupants: &str) -> String {
    format!(
        r#"
        _filter_area = "not called"
        local occupants = {{ {occupants} }}
        -- Flipped by a test to make every spot look occupied.
        _nowhere_to_stand = false
        _searched_from = {{}}
        local surface = {{
            can_place_entity = function(args) return false end,
            create_entity = function(args) error("must not build a refused site") end,
            find_entity = function(name, pos) return nil end,
            find_non_colliding_position = function(name, center, radius, precision)
                _searched_from[#_searched_from + 1] = {{ x = center.x, y = center.y }}
                if _nowhere_to_stand then return nil end
                return {{ x = center.x, y = center.y }}
            end,
            find_entities_filtered = function(args)
                local a = args.area
                _filter_area = string.format("%.2f,%.2f,%.2f,%.2f",
                    a.left_top.x, a.left_top.y, a.right_bottom.x, a.right_bottom.y)
                local hits = {{}}
                for _, e in ipairs(occupants) do
                    if args.type ~= nil and e.type ~= args.type then goto continue end
                    local b = e.bounding_box
                    if b.left_top.x <= a.right_bottom.x and b.right_bottom.x >= a.left_top.x
                        and b.left_top.y <= a.right_bottom.y and b.right_bottom.y >= a.left_top.y then
                        hits[#hits + 1] = e
                    end
                    ::continue::
                end
                return hits
            end,
        }}
        local player = {{
            index = 1,
            name = "bot4",
            connected = true,
            character = {{}},
            position = {{ x = {px}, y = {py} }},
            force = "player",
            surface = surface,
            get_item_count = function(name) return 1 end,
            remove_item = function(items) error("must not charge for a refused site") end,
        }}
        prototypes = {{ item = {{
            ["stone-furnace"] = {{ place_result = {{
                name = "stone-furnace",
                collision_box = {{
                    left_top = {{ x = -0.9, y = -0.9 }},
                    right_bottom = {{ x = 0.9, y = 0.9 }},
                }},
            }} }},
        }} }}
        -- The bots the mod could ask to move. Keyed by index, as
        -- `game.players` is, and cross-linked to the occupant list above so a
        -- character the footprint scan returns leads to something walkable.
        storage = {{ p = {{ [1] = {{}} }} }}
        local players = {{ player }}
        for _, e in ipairs(occupants) do
            if e.player_index ~= nil then
                local blocker = {{
                    index = e.player_index,
                    name = "bot" .. e.player_index,
                    connected = true,
                    character_running_speed = 0.15,
                    walking_state = {{ walking = false }},
                    character = {{ position = e.position }},
                    surface = surface,
                }}
                blocker.position = e.position
                e.player = blocker
                players[e.player_index] = blocker
                storage.p[e.player_index] = {{}}
            end
        end
        game = {{
            tick = {tick},
            players = players,
            forces = {{ player = {{ print = noop }} }},
        }}
    "#,
        px = player_position.0,
        py = player_position.1,
        tick = STUB_TICK,
    )
}

fn refuse(player_position: (f64, f64), occupants: &str) -> Lua {
    run(
        &stub_refused_place(player_position, occupants),
        STUB_SERIALISE,
        &format!(r#"rcon_place_entity(1, "stone-furnace", {SITE}, 0)"#),
    )
}

fn string_global(lua: &Lua, name: &str) -> String {
    lua.globals()
        .get(name)
        .unwrap_or_else(|err| panic!("reading {name}: {err}"))
}

/// **The defect run 24 ended on, and the reason it cost a site rather than a
/// retry.**
///
/// `can_place_entity` collides with characters like anything else, and the
/// mod's three-way branch used to recognise only the *acting* player
/// (`§player_blocks_placement§`). Any other bot standing in the footprint fell
/// through to the generic `can_place_entity said 'no'` — which is the exact
/// substring `note_placement_refusal` (`crates/core/src/factorio/rcon.rs`)
/// matches. So a bot parked for a few thousand ticks put open ground into
/// `FactorioWorld::placement_refusals`, which is never expired.
///
/// The ledger entry is the smaller half of the cost. `recover`'s tier 1
/// reschedules the same network, which is *precisely* the recovery a blocker
/// that walks away needs — and tier 1 is skipped when the failed `Place` sits
/// on a refused footprint (`refused_by_the_game`, gated on
/// `PlanState::is_site_refused`). Recording this refusal therefore disabled
/// the one recovery tier that fits it.
///
/// This is the same distinction the pre-check path already draws with
/// `rec.character` / [`PlacementVerdict::is_durable_refusal`]: **any**
/// character, not just the acting one, is a transient rather than a fact about
/// the ground. Two call sites, one concept.
#[test]
fn another_bot_in_the_footprint_is_a_transient_not_a_verdict_about_the_ground() {
    let lua = refuse(ACTOR_AWAY, PARKED_BOT);
    let line = one_line_reply(&lua);
    assert!(
        !line.contains("can_place_entity said 'no'"),
        "a bot standing here says nothing about the ground. Matching that \
         wording fences the planner out of a legal site for the rest of the \
         run AND suppresses the tier-1 reschedule that would have worked. \
         Got {line:?}"
    );
    assert_ne!(
        line, "§player_blocks_placement§",
        "that sentinel makes the RCON layer walk the ACTING bot around eight \
         compass points; the acting bot is not the blocker here, so the walk \
         cannot help"
    );
    assert!(
        line.contains("cannot place item 'stone-furnace'") && line.contains("character"),
        "the refusal must name the item and say a character is in the way, so \
         a reader of the run log can tell it from a ground verdict. Got {line:?}"
    );
    assert_eq!(
        string_global(&lua, "_filter_area"),
        "-21.90,23.10,-20.10,24.90",
        "the box scanned must be the raw collision box the game just tested, \
         not the floor/ceil-expanded one the acting-player check uses — the \
         same choice `rcon_can_place_entities` makes and for the same reason"
    );
}

/// The control that keeps the fix from swallowing the real thing: an empty
/// footprint is still a verdict about the ground, and still enters the ledger.
#[test]
fn a_refusal_with_no_character_in_the_footprint_is_still_about_the_ground() {
    let line = one_line_reply(&refuse(ACTOR_AWAY, ""));
    assert!(
        line.contains("cannot place item 'stone-furnace'")
            && line.contains("can_place_entity said 'no'"),
        "with nothing standing there the game's refusal is the durable fact \
         the refusal memory exists to keep. Got {line:?}"
    );
}

/// A tree is not a transient. Only `type == \"character\"` walks away on its
/// own, so anything else must keep the wording the planner learns from.
#[test]
fn a_tree_in_the_footprint_is_a_verdict_about_the_ground() {
    let line = one_line_reply(&refuse(ACTOR_AWAY, TREE));
    assert!(
        line.contains("can_place_entity said 'no'"),
        "forest siting is one of the five causes already paid for; a tree must \
         still fence the planner off this footprint. Got {line:?}"
    );
}

/// The acting player keeps its existing, better recovery, and keeps it even
/// when another character is in the box too. The branch order is deliberate:
/// walking the actor aside and retrying beats a reschedule, so it is tested
/// first.
#[test]
fn the_acting_player_still_gets_the_walk_aside_sentinel() {
    let actor_in_the_box = (-21.0, 24.0);
    assert_eq!(
        one_line_reply(&refuse(actor_in_the_box, PARKED_BOT)),
        "§player_blocks_placement§",
        "the acting bot in its own footprint is the case the RCON layer can \
         fix by walking it, and that must win over the reschedule path"
    );
}

// ---------------------------------------------------------------------------
// Cause seven: the transient that never ends.
// ---------------------------------------------------------------------------

/// What the mod did to the bots it found in the footprint, as
/// `player index -> the waypoint it was sent to`, or an empty map if it asked
/// nobody to move.
fn asked_to_move(lua: &Lua) -> Vec<(u32, (f64, f64))> {
    lua.load(
        r#"
        local out = {}
        for idx, p in pairs(storage.p) do
            if p.walking ~= nil and p.walking.step_aside then
                local w = p.walking.waypoints[1]
                out[#out + 1] = { idx, w.x, w.y }
            end
        end
        table.sort(out, function(a, b) return a[1] < b[1] end)
        return out
    "#,
    )
    .eval::<mlua::Table>()
    .expect("storage.p")
    .sequence_values::<mlua::Table>()
    .map(|row| {
        let row = row.expect("a row");
        (
            row.get::<u32>(1).expect("player index"),
            (
                row.get::<f64>(2).expect("waypoint x"),
                row.get::<f64>(3).expect("waypoint y"),
            ),
        )
    })
    .collect()
}

/// The footprint under test, `[-21, 24]` expanded by a stone furnace's
/// `±0.9` box: what `can_place_entity` just judged.
const FOOTPRINT: (f64, f64, f64, f64) = (-21.9, 23.1, -20.1, 24.9);

fn inside_footprint((x, y): (f64, f64)) -> bool {
    let (l, t, r, b) = FOOTPRINT;
    x > l && x < r && y > t && y < b
}

/// **The defect: an idle bot in the footprint is a permanent blocker, and
/// nothing in the system ever asks it to move.**
///
/// `537adf30` made this a transient rather than a verdict about the ground,
/// which was right and is not enough. A transient is only transient if
/// something ends it, and a bot that parked there after servicing its own
/// furnace has no reason to leave: `run-1788353986-24634` (run 27) has bot 3
/// motionless at `(-23.5078125, 16.203125)` from tick 18240 to the end of the
/// run while milestone 6 re-planned onto that ground three times and gave up —
/// `stuck after 6 iteration(s)`, `success=0 failed=1 lost=0 pending=30`.
///
/// The mod already walks the **acting** player out of its own footprint
/// (`§player_blocks_placement§`, handled in `place_entity_timed`). This widens
/// that from "the bot I happen to be holding" to "any bot I can reach", which
/// is the same widening `537adf30` made to the *classification* — one concept,
/// now drawn the same way by both halves of the same branch.
#[test]
fn a_parked_bot_in_the_footprint_is_asked_to_walk_out() {
    let lua = refuse(ACTOR_AWAY, PARKED_BOT);
    let moved = asked_to_move(&lua);
    assert_eq!(
        moved.len(),
        1,
        "the one bot standing in the footprint has to be asked to leave; \
         nothing else in the system will ever ask it. Got {moved:?}"
    );
    let (idx, waypoint) = moved[0];
    assert_eq!(idx, 3, "and it is the bot that is actually standing there");
    assert!(
        !inside_footprint(waypoint),
        "a step aside that lands back inside the footprint is a walk that \
         costs time and changes nothing. {waypoint:?} is inside {FOOTPRINT:?}"
    );
}

/// The reply is exactly what it was, because the classification was already
/// right and must not be reopened.
///
/// Two things at once. The wording stays outside the `can_place_entity said
/// 'no'` family that `note_placement_refusal` learns from — a character still
/// says nothing about the ground. And **nothing extra reaches the RCON reply
/// body**: `place_entity_timed` reads that body as the action's whole verdict,
/// so a step-aside that printed anything would turn this refusal into
/// `Unexpected Response` and lose the message that names the cause.
#[test]
fn asking_a_blocker_to_move_does_not_change_what_the_reply_says() {
    let lua = refuse(ACTOR_AWAY, PARKED_BOT);
    let line = one_line_reply(&lua);
    assert_eq!(
        line, "cannot place item 'stone-furnace' because a character is standing in the footprint",
        "the transient/refusal distinction landed in 537adf30 is not this \
         change's to move"
    );
}

/// **A bot the mod is already driving is left strictly alone.**
///
/// This is the one way the fix could cost more than it saves.
/// `storage.p[idx].walking` is how `on_tick` steers a bot through the
/// waypoints an executor action is waiting on; overwriting it would strand
/// that action until the 360-second `ACTION_RESULT_DEADLINE` calls it lost,
/// which is a far worse outcome than the refusal being fixed.
///
/// It is also unnecessary: a bot that is walking is *going* to leave. Only a
/// bot with nothing to do is a permanent blocker, and "the mod is not steering
/// it" is exactly that condition, decided inside a single RCON command and so
/// not racing anything.
#[test]
fn a_blocker_the_mod_is_already_walking_is_not_touched() {
    let lua = run(
        &stub_refused_place(ACTOR_AWAY, PARKED_BOT),
        &format!(
            "{STUB_SERIALISE}\n\
             storage.p[3].walking = {{ idx = 1, action_id = 77, idx_tick = 5, \
             leg_timeout = 60, waypoints = {{ {{ x = 9.5, y = 9.5 }} }} }}\n"
        ),
        &format!(r#"rcon_place_entity(1, "stone-furnace", {SITE}, 0)"#),
    );
    assert!(
        asked_to_move(&lua).is_empty(),
        "a walk in progress must not be replaced"
    );
    let survived: u32 = lua
        .load("return storage.p[3].walking.action_id")
        .eval()
        .expect("the original walk");
    assert_eq!(
        survived, 77,
        "the executor's own walk has to reach on_tick untouched; clobbering \
         it strands the action it belongs to"
    );
}

/// And neither is one that is mining. Same reason: the mod is steering it, an
/// action is waiting on it, and it will move when it is done.
#[test]
fn a_blocker_that_is_mining_is_not_touched() {
    let lua = run(
        &stub_refused_place(ACTOR_AWAY, PARKED_BOT),
        &format!("{STUB_SERIALISE}\nstorage.p[3].mining = {{ action_id = 88, left = 3 }}\n"),
        &format!(r#"rcon_place_entity(1, "stone-furnace", {SITE}, 0)"#),
    );
    assert!(
        asked_to_move(&lua).is_empty(),
        "a mining bot is busy, will finish, and is not a permanent blocker"
    );
}

/// A tree does not walk. The step-aside must reach only the class of blocker
/// that can act on the request, or it is a walk dispatched at scenery.
#[test]
fn a_tree_in_the_footprint_is_asked_nothing() {
    let lua = refuse(ACTOR_AWAY, TREE);
    assert!(
        asked_to_move(&lua).is_empty(),
        "only a character can be asked to move"
    );
}

/// A character with no player behind it cannot be walked — `walking_state` is
/// a `LuaControl` property and the mod's walker steers players. It must be
/// skipped silently rather than raising inside an RCON handler, where a raise
/// costs the caller its reply.
#[test]
fn a_character_with_no_player_is_asked_nothing_and_does_not_raise() {
    let lua = refuse(ACTOR_AWAY, PARKED_STRANGER);
    assert!(
        asked_to_move(&lua).is_empty(),
        "there is nothing to ask; the refusal still reports the character"
    );
    assert_eq!(
        one_line_reply(&lua),
        "cannot place item 'stone-furnace' because a character is standing in the footprint",
        "and the reply is unaffected by there being nobody to move"
    );
}

/// The acting player keeps its own, better recovery and this path stays out of
/// it. `§player_blocks_placement§` walks the actor around eight compass points
/// and **retries the placement**, which beats a step-aside plus a failed action
/// — so the branch order must not change, and no bot is asked to move on it.
#[test]
fn the_acting_player_branch_asks_nobody_to_step_aside() {
    let actor_in_the_box = (-21.0, 24.0);
    let lua = refuse(actor_in_the_box, PARKED_BOT);
    assert_eq!(one_line_reply(&lua), "§player_blocks_placement§");
    assert!(
        asked_to_move(&lua).is_empty(),
        "the RCON layer is about to walk the actor and retry; a second, \
         competing walk dispatched here would fight it"
    );
}

/// Nowhere to stand means no walk. `find_non_colliding_position` answering
/// `nil` is the game saying the character does not fit anywhere near the
/// target, and dispatching a walk to a spot it cannot occupy buys a leg
/// timeout and a stuck-abort instead of an answer.
#[test]
fn a_blocker_with_nowhere_to_stand_is_not_sent_walking() {
    let lua = run(
        &stub_refused_place(ACTOR_AWAY, PARKED_BOT),
        &format!("{STUB_SERIALISE}\n_nowhere_to_stand = true\n"),
        &format!(r#"rcon_place_entity(1, "stone-furnace", {SITE}, 0)"#),
    );
    assert!(
        asked_to_move(&lua).is_empty(),
        "a walk to a spot the character cannot occupy is worse than no walk"
    );
    assert_eq!(
        one_line_reply(&lua),
        "cannot place item 'stone-furnace' because a character is standing in the footprint",
        "and the refusal is reported the same way either way"
    );
}

/// The spot asked for is outside the footprint before the game is consulted,
/// so a `find_non_colliding_position` that answers "yes, right there" cannot
/// hand back somewhere still in the way.
///
/// It is asked for the **nearest** exit, which for run 24's parked bot is
/// westward: its centre is 0.43 tiles from the western edge against 1.37 from
/// the eastern, and a step aside that crosses the whole footprint is a longer
/// walk to no better place.
#[test]
fn the_step_aside_aims_out_of_the_nearest_edge() {
    let lua = refuse(ACTOR_AWAY, PARKED_BOT);
    let searched: Vec<(f64, f64)> = lua
        .load("local o = {} for i, p in ipairs(_searched_from) do o[i] = { p.x, p.y } end return o")
        .eval::<mlua::Table>()
        .expect("_searched_from")
        .sequence_values::<mlua::Table>()
        .map(|t| {
            let t = t.expect("a search");
            (t.get::<f64>(1).unwrap(), t.get::<f64>(2).unwrap())
        })
        .collect();
    assert_eq!(searched.len(), 1, "one bot, one question. Got {searched:?}");
    let (x, y) = searched[0];
    assert!(
        !inside_footprint((x, y)),
        "the target handed to the game must already be clear of {FOOTPRINT:?}; \
         got ({x}, {y})"
    );
    assert!(
        x < FOOTPRINT.0 && (y - 23.73).abs() < 1e-9,
        "the nearest edge is the western one (0.43 tiles away against 1.37 \
         eastward), and stepping sideways off the exit axis is extra walking \
         for nothing. Got ({x}, {y})"
    );
}
