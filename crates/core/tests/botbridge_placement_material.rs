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
const PARKED_BOT: &str = r#"
    { name = "character", type = "character", bounding_box = {
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
        local surface = {{
            can_place_entity = function(args) return false end,
            create_entity = function(args) error("must not build a refused site") end,
            find_entity = function(name, pos) return nil end,
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
            name = "bot4",
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
        game = {{
            tick = {tick},
            players = {{ player }},
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
