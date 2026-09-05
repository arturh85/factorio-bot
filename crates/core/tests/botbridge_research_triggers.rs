//! **Does the trigger sweep complete only what the force has earned?**
//!
//! Factorio 2.0 unlocks 32 technologies by *doing* rather than by research
//! (`LuaTechnologyPrototype::research_trigger`). Measured live on 2026-09-05
//! (`docs/superpowers/notes/2026-09-05-research-triggers.md`): the game fires
//! `mine-entity` on its own for a server-side character, a drill and a
//! pumpjack, and `craft-item` for furnace output; it does not fire `craft-item`
//! for a character's hand craft, nor `build-entity` for
//! `surface.create_entity`, which is every placement this mod makes. So
//! `emulate_research_triggers` in `control.lua` completes those two kinds from
//! counters the mod keeps, and the owner's rule is that it may do so only when
//! the act has actually happened -- never a grant.
//!
//! These tests load the real `control.lua` on a stub game and drive the sweep
//! directly. What they can prove: which counters the sweep reads, that a
//! technology whose prerequisites are open waits (the game gates on them --
//! measured), that a build the enemy raised is not ours, and that the switch
//! for measurements really switches the sweep off. What they cannot: the
//! shape Factorio gives `research_trigger` at runtime, which the live capture
//! in the note is for.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// A tick on the sweep's 60-tick beat.
const SWEEP_TICK: u64 = 3600;

/// Enough of Factorio for `control.lua` to load, plus a real (sorted-key)
/// JSON encoder so the emulation event's payload can be asserted on, and a
/// `storage` that already holds one character bot -- the sweep runs only
/// while one exists.
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

    local function to_json(v)
        local t = type(v)
        if t == "table" then
            local keys = {}
            for k in pairs(v) do keys[#keys + 1] = k end
            table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
            local parts = {}
            for _, k in ipairs(keys) do
                parts[#parts + 1] = '"' .. tostring(k) .. '":' .. to_json(v[k])
            end
            return "{" .. table.concat(parts, ",") .. "}"
        elseif t == "string" then
            return '"' .. v .. '"'
        else
            return tostring(v)
        end
    end
    helpers = setmetatable(
        { table_to_json = to_json },
        { __index = function() return noop end })

    storage = { bots = { [1] = {} } }
    prototypes = { item = {}, entity = {} }
"#;

/// A technology in the stub force: its trigger table exactly as the live game
/// spells it, and the names of its prerequisites.
struct Tech {
    name: &'static str,
    trigger: &'static str,
    prerequisites: &'static [&'static str],
}

/// A stub force with `techs`, every one unresearched unless named in
/// `researched`, whose item production statistics hold `produced`
/// (`name = count` pairs, Lua syntax).
fn stub_game(techs: &[Tech], researched: &[&str], produced: &str) -> String {
    let mut out = String::from("local techs = {}\n");
    for tech in techs {
        let done = researched.contains(&tech.name);
        out.push_str(&format!(
            r#"techs["{n}"] = {{ name = "{n}", researched = {done}, enabled = true,
                prerequisites = {{}}, prototype = {{ research_trigger = {t} }} }}
"#,
            n = tech.name,
            t = tech.trigger,
        ));
    }
    // Prerequisites point at the technology tables themselves, as the game's
    // do, so marking one researched is visible through the other.
    for tech in techs {
        for prerequisite in tech.prerequisites {
            out.push_str(&format!(
                r#"techs["{n}"].prerequisites["{p}"] = techs["{p}"]
"#,
                n = tech.name,
                p = prerequisite,
            ));
        }
    }
    out.push_str(&format!(
        r#"
        local force = {{
            name = "player",
            technologies = techs,
            get_item_production_statistics = function(surface)
                return {{ input_counts = {{ {produced} }} }}
            end,
        }}
        game = {{
            tick = {SWEEP_TICK},
            players = {{}},
            forces = {{ player = force }},
            surfaces = {{ {{}} }},
        }}
    "#
    ));
    out
}

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
    lua.load(call).set_name("call").exec().expect("call");
    lua
}

fn researched(lua: &Lua, tech: &str) -> bool {
    lua.load(format!(
        r#"return game.forces.player.technologies["{tech}"].researched"#
    ))
    .eval::<bool>()
    .expect("researched flag")
}

/// Every `research_trigger_emulated` line the mod wrote, payload only.
fn emulated(lua: &Lua) -> Vec<String> {
    lua.globals()
        .get::<mlua::Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .filter_map(|line| {
            line.strip_prefix(&format!("§{SWEEP_TICK}§research_trigger_emulated§"))
                .map(str::to_string)
        })
        .collect()
}

const SWEEP: &str = "emulate_research_triggers(game.tick)";

/// `on_some_entity_created` writes the entity out through `serialize_entity`,
/// which reads inventories, bounding boxes and a dozen other things a stub
/// entity does not have; the writeout is not what these tests are about.
const SERIALIZE_STUB: &str = "serialize_entity = function(ent) return { name = ent.name } end";

const ELECTRONICS: Tech = Tech {
    name: "electronics",
    trigger: r#"{ type = "craft-item", item = { name = "copper-plate" }, count = 10 }"#,
    prerequisites: &[],
};
const STEAM_POWER: Tech = Tech {
    name: "steam-power",
    trigger: r#"{ type = "craft-item", item = { name = "iron-plate" }, count = 50 }"#,
    prerequisites: &[],
};
const AUTOMATION_SCIENCE_PACK: Tech = Tech {
    name: "automation-science-pack",
    trigger: r#"{ type = "craft-item", item = { name = "lab" }, count = 1 }"#,
    prerequisites: &["electronics", "steam-power"],
};
/// The live spelling: `entity`, singular, a name filter.
const SPACE_PLATFORM: Tech = Tech {
    name: "space-platform",
    trigger: "nil",
    prerequisites: &[],
};
const SPACE_SCIENCE_PACK: Tech = Tech {
    name: "space-science-pack",
    trigger: r#"{ type = "build-entity", entity = { name = "asteroid-collector" } }"#,
    prerequisites: &["space-platform"],
};
const OIL_GATHERING: Tech = Tech {
    name: "oil-gathering",
    trigger: "nil",
    prerequisites: &[],
};
const OIL_PROCESSING: Tech = Tech {
    name: "oil-processing",
    trigger: r#"{ type = "mine-entity", entities = { "crude-oil" } }"#,
    prerequisites: &["oil-gathering"],
};

/// `craft-item`, machine-made: the force's production statistics earn it.
#[test]
fn ten_smelted_copper_plates_earn_electronics() {
    let lua = run(
        &stub_game(&[ELECTRONICS], &[], r#"["copper-plate"] = 10"#),
        SWEEP,
    );
    assert!(researched(&lua, "electronics"));
    assert_eq!(
        emulated(&lua),
        vec![
            r#"{"item":"copper-plate","needed":10,"produced":10,"technology":"electronics","trigger":"craft-item"}"#
        ],
        "the event names the counter that earned it"
    );
}

#[test]
fn nine_copper_plates_earn_nothing() {
    let lua = run(
        &stub_game(&[ELECTRONICS], &[], r#"["copper-plate"] = 9"#),
        SWEEP,
    );
    assert!(!researched(&lua, "electronics"));
    assert!(emulated(&lua).is_empty());
}

/// `craft-item`, hand-made: a lab never appears in production statistics
/// (measured), so the sweep reads the hand-craft tally for it.
#[test]
fn a_hand_crafted_lab_earns_automation_science_pack_once_its_prerequisites_are_researched() {
    let lua = run(
        &stub_game(
            &[ELECTRONICS, STEAM_POWER, AUTOMATION_SCIENCE_PACK],
            &["electronics", "steam-power"],
            "",
        ),
        &format!("storage.crafted_tally = {{ lab = 1 }}\n{SWEEP}"),
    );
    assert!(researched(&lua, "automation-science-pack"));
    assert_eq!(
        emulated(&lua),
        vec![
            r#"{"item":"lab","needed":1,"produced":1,"technology":"automation-science-pack","trigger":"craft-item"}"#
        ]
    );
}

/// **Prerequisites gate the trigger.** The game does this (a rock mined with
/// `planet-discovery-vulcanus` open earned nothing; the same rock with it
/// researched did), so a sweep that ignored them could hand out
/// `automation-science-pack` before the plate triggers that unlock it --
/// a technology this run did not earn in the order the game requires.
#[test]
fn a_hand_crafted_lab_waits_while_a_prerequisite_is_open() {
    let lua = run(
        &stub_game(
            &[ELECTRONICS, STEAM_POWER, AUTOMATION_SCIENCE_PACK],
            &["electronics"],
            "",
        ),
        &format!("storage.crafted_tally = {{ lab = 1 }}\n{SWEEP}"),
    );
    assert!(
        !researched(&lua, "automation-science-pack"),
        "steam-power is still open"
    );
    assert!(emulated(&lua).is_empty());
}

/// The act is remembered across the gate: the same tally earns the technology
/// on the first sweep after the last prerequisite closes, as the game's own
/// counters did for `heating-tower`.
#[test]
fn the_act_before_the_prerequisite_still_counts_after_it() {
    let lua = run(
        &stub_game(
            &[ELECTRONICS, STEAM_POWER, AUTOMATION_SCIENCE_PACK],
            &["electronics"],
            "",
        ),
        &format!(
            r#"storage.crafted_tally = {{ lab = 1 }}
            {SWEEP}
            game.forces.player.technologies["steam-power"].researched = true
            {SWEEP}"#
        ),
    );
    assert!(researched(&lua, "automation-science-pack"));
}

/// `build-entity`: the entity this force built, announced through
/// `on_some_entity_created` -- the point every placement this mod makes
/// passes through -- earns it.
#[test]
fn building_an_asteroid_collector_earns_space_science_pack() {
    let lua = run(
        &stub_game(
            &[SPACE_PLATFORM, SPACE_SCIENCE_PACK],
            &["space-platform"],
            "",
        ),
        &format!(
            r#"{SERIALIZE_STUB}
            on_some_entity_created({{ tick = game.tick, entity = {{
                valid = true, name = "asteroid-collector", type = "asteroid-collector",
                force = game.forces.player, position = {{ x = 1, y = 2 }} }} }})
            {SWEEP}"#
        ),
    );
    assert!(researched(&lua, "space-science-pack"));
    assert_eq!(
        emulated(&lua),
        vec![
            r#"{"built":1,"entity":"asteroid-collector","needed":1,"technology":"space-science-pack","trigger":"build-entity"}"#
        ]
    );
}

#[test]
fn nothing_built_earns_no_build_entity_trigger() {
    let lua = run(
        &stub_game(
            &[SPACE_PLATFORM, SPACE_SCIENCE_PACK],
            &["space-platform"],
            "",
        ),
        SWEEP,
    );
    assert!(!researched(&lua, "space-science-pack"));
    assert!(emulated(&lua).is_empty());
}

/// `on_biter_base_built` reaches the same handler. A spawner the enemy raised
/// is not something we built, and neither is a collector on another force.
#[test]
fn an_entity_another_force_built_does_not_count() {
    let lua = run(
        &stub_game(
            &[SPACE_PLATFORM, SPACE_SCIENCE_PACK],
            &["space-platform"],
            "",
        ),
        &format!(
            r#"{SERIALIZE_STUB}
            on_some_entity_created({{ tick = game.tick, entity = {{
                valid = true, name = "asteroid-collector", type = "asteroid-collector",
                force = {{ name = "enemy" }}, position = {{ x = 1, y = 2 }} }} }})
            {SWEEP}"#
        ),
    );
    assert!(!researched(&lua, "space-science-pack"));
    assert_eq!(
        lua.load("return storage.built_tally['asteroid-collector']")
            .eval::<Option<u32>>()
            .expect("tally"),
        None
    );
}

/// `mine-entity` is the game's own to fire, and it does so without a player
/// (measured for a character, a burner drill and a pumpjack). The sweep must
/// leave it alone even when the statistics say the well has flowed, or a
/// headless run would complete it twice -- or first.
#[test]
fn a_mine_entity_trigger_is_left_to_the_game() {
    let lua = run(
        &stub_game(
            &[OIL_GATHERING, OIL_PROCESSING],
            &["oil-gathering"],
            r#"["crude-oil"] = 150"#,
        ),
        SWEEP,
    );
    assert!(!researched(&lua, "oil-processing"));
    assert!(emulated(&lua).is_empty());
}

/// The switch for measurements: off, the sweep completes nothing and says so
/// in the record; on again, the same counters earn the technology.
#[test]
fn the_emulation_switch_stops_the_sweep_and_is_recorded() {
    let lua = run(
        &stub_game(&[ELECTRONICS], &[], r#"["copper-plate"] = 10"#),
        &format!(
            r#"rcon_set_research_trigger_emulation(false)
            {SWEEP}
            _off = game.forces.player.technologies["electronics"].researched
            rcon_set_research_trigger_emulation(true)
            {SWEEP}"#
        ),
    );
    assert!(
        !lua.globals().get::<bool>("_off").expect("_off"),
        "nothing completes while the switch is off"
    );
    assert!(researched(&lua, "electronics"));
    let printed: Vec<String> = lua
        .globals()
        .get::<mlua::Table>("_printed")
        .expect("_printed")
        .sequence_values::<String>()
        .map(|v| v.expect("line"))
        .collect();
    assert!(
        printed.contains(&format!(
            r#"§{SWEEP_TICK}§research_trigger_emulation§{{"enabled":false}}"#
        )),
        "the switch-off is in the record: {printed:?}"
    );
}

/// With real players the game fires every trigger itself; the sweep exists
/// only for character-bot worlds.
#[test]
fn the_sweep_does_nothing_without_character_bots() {
    let lua = run(
        &stub_game(&[ELECTRONICS], &[], r#"["copper-plate"] = 10"#),
        &format!("storage.bots = {{}}\n{SWEEP}"),
    );
    assert!(!researched(&lua, "electronics"));
}
