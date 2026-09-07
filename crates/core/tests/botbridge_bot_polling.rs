//! **What a bot costs in tick rate, and what stopped paying it.**
//!
//! Measured on 2026-09-06 by two-point differencing at 60,000 and 180,000
//! ticks, so ~16 s of server startup cancels exactly and probe overhead lands
//! in the base rather than the slope:
//!
//! ```text
//! 1 bot    5145 tps    194 us/tick
//! 4 bots   4147 tps    241 us/tick
//! 8 bots   3242 tps    308 us/tick
//! fit: 178 us base + 16.3 us per bot per tick
//! ```
//!
//! At eight bots, 130 of the 308 us -- 42% of the tick -- was
//! `poll_character_bot` re-deriving each bot's whole main inventory: an
//! allocating `get_contents()`, a string per stack, a sort and a concat, sixty
//! times a second, to notice changes that happen a handful of times a minute.
//!
//! **130 us against a 16,667 us real-time budget is under 1% at 60 Hz.** This
//! is a throughput change for `--headless --game-speed N`, where the tick
//! budget is whatever the CPU can do. It is not a correctness fix and not a
//! playability fix, and nothing here should be read as one.
//!
//! The change is two mechanisms, and the tests below exist because neither is
//! safe alone:
//!
//! * a **dirty flag**, set by the mod at the places the mod itself moves items,
//!   so a change it caused is written out on the very next tick -- no latency,
//!   no dependence on counts;
//! * a **stagger**, `tick % PERIOD == id % PERIOD`, so a change the mod did
//!   *not* cause is still seen within `PERIOD` ticks. Without it the flag would
//!   have to be complete, and an audit that must be complete is exactly the
//!   kind that silently is not.
//!
//! **These tests cannot re-measure the tick rate: Factorio is not run here.**
//! What they can establish is the semantics -- that nothing stops being
//! observed, and that the bound on how late an unflagged change may be seen is
//! the one the constant states. The tick-rate claim above is the peer
//! session's measurement, quoted, not reproduced.
//!
//! *Written by the same task that wrote the change.* The stub below states
//! what a Factorio inventory looks like -- `get_contents()` answers an array of
//! `{name, count, quality}`, checked against
//! `crates/core/tests/live-2.1.17-inventory-contents-at.json`, which is a real
//! 2.1.17 reply -- and a stub that omitted `quality` would make the mod's
//! signature string agree with itself while the real game disagreed.

use mlua::{Lua, LuaOptions, StdLib};

const CONTROL_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/control.lua"
));
const TYPES_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../mods/BotBridge/types.lua"
));

/// Enough of the Factorio API for `poll_character_bots` and the RCON entry
/// points that mark a bot dirty.
///
/// `_reads` counts the two things the change is about: `inventory` is one
/// `get_contents()` scan, `queue` is one `crafting_queue` read. A test that
/// asserts on those counts is asserting on the work done, not on a proxy for
/// it.
const STUB: &str = r#"
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
    commands = nooptable()
    require = function() return {} end
    remote = setmetatable({ interfaces = {} }, { __index = function() return noop end })

    _out = {}
    print = function(s) _out[#_out + 1] = tostring(s) end
    _rcon_lines = {}
    rcon = { print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }
    helpers = setmetatable(
        { table_to_json = function(t) return "<json>" end },
        { __index = function() return noop end })

    _reads = { inventory = 0, queue = 0 }
    _inv = {}       -- bot id -> { item name -> count }
    _queue = {}     -- bot id -> array of { recipe = name, count = n } or nil

    _recipes = {
        ["iron-gear-wheel"] = {
            name = "iron-gear-wheel",
            enabled = true,
            products = { { type = "item", name = "iron-gear-wheel", amount = 1 } },
        },
    }
    _force = { name = "player", recipes = _recipes }
    _surface = {
        index = 1,
        find_entity = function() return nil end,
        find_entities_filtered = function() return {} end,
    }

    storage = { p = {}, bots = {}, n_clients = 0 }
    game = {
        tick = 0,
        players = {},
        connected_players = {},
        forces = { player = _force },
        surfaces = { _surface, nauvis = _surface },
    }
    prototypes = { item = {}, entity = {} }

    -- A character bot as the mod sees one. `get_contents()` answers an array of
    -- {name, count, quality} -- the 2.0 shape, confirmed against
    -- live-2.1.17-inventory-contents-at.json -- because the mod builds its
    -- change signature out of all three fields.
    function make_bot(id, x, y)
        _inv[id] = {}
        local ent
        ent = {
            valid = true, name = "character", type = "character", unit_number = id,
            position = { x = x, y = y },
            force = _force,
            surface = _surface,
            crafting_queue_size = 0,
            get_main_inventory = function()
                return {
                    get_contents = function()
                        _reads.inventory = _reads.inventory + 1
                        local out = {}
                        for name, count in pairs(_inv[id]) do
                            out[#out + 1] = { name = name, count = count, quality = "normal" }
                        end
                        return out
                    end,
                    get_item_count = function(name) return _inv[id][name] or 0 end,
                }
            end,
            insert = function(spec)
                _inv[id][spec.name] = (_inv[id][spec.name] or 0) + spec.count
                return spec.count
            end,
        }
        setmetatable(ent, { __index = function(_, key)
            if key == "crafting_queue" then
                _reads.queue = _reads.queue + 1
                return _queue[id]
            end
        end })
        storage.bots[id] = { entity = ent, name = "bot-" .. id }
        storage.p[id] = {}
        return ent
    end

    -- What the game does behind the mod's back: a mining yield arriving, a
    -- craft completing, anything the mod did not perform itself. No flag is
    -- set, on purpose -- this is the case the stagger exists for.
    function game_gives(id, name, count)
        _inv[id][name] = (_inv[id][name] or 0) + count
    end

    function inventory_writeouts()
        local out = {}
        for _, line in ipairs(_out) do
            local tick, id = string.match(line,
                "^\xc2\xa7(%d+)\xc2\xa7on_player_main_inventory_changed\xc2\xa7")
            if tick ~= nil then out[#out + 1] = tonumber(tick) end
        end
        return out
    end
"#;

/// The parts of the mod not under test. `emulate_research_triggers` reads the
/// force's production statistics on a 60-tick beat and has its own tests;
/// `on_player_crafted_item` is the real event handler, replaced here by a
/// recorder so a craft settle is visible as a count.
const AFTER: &str = r#"
    emulate_research_triggers = function() end
    _crafted = {}
    on_player_crafted_item = function(event)
        _crafted[#_crafted + 1] = { tick = event.tick, id = event.player_index }
    end
"#;

fn fresh(bots: u8) -> Lua {
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
    lua.load(STUB).set_name("stub_game").exec().expect("stub");
    lua.load(TYPES_LUA)
        .set_name("types.lua")
        .exec()
        .expect("mod types.lua");
    lua.load(CONTROL_LUA)
        .set_name("control.lua")
        .exec()
        .expect("mod control.lua");
    lua.load(AFTER).set_name("after").exec().expect("overrides");
    for id in 1..=bots {
        lua.load(format!("make_bot({id}, {id}.5, 0.5)"))
            .set_name("make_bot")
            .exec()
            .expect("bot");
    }
    lua
}

fn exec(lua: &Lua, chunk: &str) {
    lua.load(chunk.to_string())
        .set_name("step")
        .exec()
        .unwrap_or_else(|err| panic!("lua: {err}"));
}

fn number(lua: &Lua, expr: &str) -> i64 {
    lua.load(format!("return {expr}"))
        .eval::<i64>()
        .unwrap_or_else(|err| panic!("{expr}: {err}"))
}

/// The ticks at which an `on_player_main_inventory_changed` was written out.
fn inventory_writeout_ticks(lua: &Lua) -> Vec<i64> {
    lua.load("return inventory_writeouts()")
        .eval::<mlua::Table>()
        .expect("writeouts")
        .sequence_values::<i64>()
        .map(|t| t.expect("a tick"))
        .collect()
}

/// The stagger period, read from the mod rather than repeated here -- a test
/// that hard-codes 30 stops testing the constant the moment someone changes it.
fn period(lua: &Lua) -> i64 {
    number(lua, "BOT_INVENTORY_POLL_PERIOD")
}

/// Run one whole period so every bot's `last_inventory` is established, then
/// clear the captured output.
///
/// Without this the first staggered scan of every bot reports its (empty)
/// inventory against a `nil` baseline, and a test counting writeouts would be
/// counting the roster's introduction rather than the change under test.
fn prime(lua: &Lua) {
    let p = period(lua);
    for t in 500..500 + p {
        exec(lua, &format!("poll_character_bots({t})"));
    }
    exec(lua, "_out = {}");
}

/// A tick on which bot `id` is *not* due for its staggered scan, so anything
/// observed there was observed because of the dirty flag and nothing else.
fn tick_that_is_not_bots_turn(lua: &Lua, id: i64) -> i64 {
    let p = period(lua);
    let t = 1000 + id;
    let t = if t % p == id % p { t + 1 } else { t };
    assert_ne!(t % p, id % p, "the chosen tick must not be bot {id}'s turn");
    t
}

/// **The flag's whole point: no latency for a change the mod made itself.**
///
/// Driven through a real RCON entry point rather than by calling
/// `mark_bot_inventory_dirty` directly, because what is under test is the
/// wiring, not the helper.
#[test]
fn a_change_the_mod_caused_is_written_out_on_the_very_next_tick() {
    let lua = fresh(4);
    let t = tick_that_is_not_bots_turn(&lua, 2);

    prime(&lua);

    exec(&lua, "rcon_cheat_item(2, 'iron-plate', 5)");
    exec(&lua, &format!("poll_character_bots({t})"));

    assert_eq!(
        inventory_writeout_ticks(&lua),
        vec![t],
        "bot 2's inventory changed through the mod, so it must be written out \
         on the next tick even though tick {t} is not its turn in the stagger"
    );
}

/// **The stagger's whole point: a change the mod did not cause is still seen,
/// within the bound the constant states.**
///
/// This is the hard case -- a mining yield landing, a craft finishing, anything
/// the game does on its own. No flag is set here. The assertion is on absolute
/// counts and on the bound, not on a relation between two numbers.
#[test]
fn a_change_the_mod_did_not_cause_is_seen_within_the_stagger_bound() {
    let lua = fresh(4);
    let p = period(&lua);

    prime(&lua);

    exec(&lua, "game_gives(3, 'copper-ore', 7)");
    let first = 1000;
    for t in first..first + p {
        exec(&lua, &format!("poll_character_bots({t})"));
    }

    let ticks = inventory_writeout_ticks(&lua);
    assert_eq!(
        ticks.len(),
        1,
        "exactly one writeout: the change is reported once and then the \
         signature matches again. Got {ticks:?}"
    );
    assert!(
        ticks[0] - first < p,
        "a change nobody flagged must still be seen within \
         BOT_INVENTORY_POLL_PERIOD ({p}) ticks of happening; it was seen {} \
         ticks later",
        ticks[0] - first
    );
    assert_eq!(
        ticks[0] % p,
        3 % p,
        "and it is seen on bot 3's own slot in the stagger, not on some other \
         bot's"
    );
}

/// **The property the tick-rate measurement was about: work per tick does not
/// grow with the roster.**
///
/// Eight bots, every one changed behind the mod's back, one period of ticks.
/// Each is scanned exactly once and no two share a tick -- so the per-tick cost
/// is one bot's scan whether the roster is one bot or eight.
#[test]
fn eight_bots_are_each_scanned_once_per_period_and_never_on_the_same_tick() {
    let lua = fresh(8);
    let p = period(&lua);

    prime(&lua);
    exec(&lua, "_reads.inventory = 0");

    for id in 1..=8 {
        exec(&lua, &format!("game_gives({id}, 'iron-ore', {id})"));
    }
    let first = 1000;
    for t in first..first + p {
        exec(&lua, &format!("poll_character_bots({t})"));
    }

    let mut ticks = inventory_writeout_ticks(&lua);
    assert_eq!(
        ticks.len(),
        8,
        "all eight changes are reported, one each. Got {ticks:?}"
    );
    ticks.sort_unstable();
    ticks.dedup();
    assert_eq!(
        ticks.len(),
        8,
        "and on eight different ticks: two bots scanning on one tick is the \
         cost this change exists to remove"
    );

    assert_eq!(
        number(&lua, "_reads.inventory"),
        8,
        "eight bots over {p} ticks cost eight inventory scans, not {}",
        8 * p
    );
}

/// The emission stays change-gated: a roster nobody touches writes out nothing,
/// however many times it is polled.
///
/// Without this the two tests above could pass while the mod emitted a writeout
/// on every staggered scan -- which would be a *correct* world model and a
/// flood of stdout.
#[test]
fn an_unchanged_inventory_is_never_written_out_twice() {
    let lua = fresh(4);
    let p = period(&lua);

    exec(&lua, "game_gives(1, 'wood', 2)");
    prime(&lua);

    for t in 1000..1000 + 3 * p {
        exec(&lua, &format!("poll_character_bots({t})"));
    }
    assert_eq!(
        inventory_writeout_ticks(&lua),
        Vec::<i64>::new(),
        "nothing changed, so nothing is written out"
    );
}

/// An announcement -- a spawn, a respawn, a savepoint resume -- must not wait
/// for the bot's turn. It is how the Rust side learns the roster exists.
#[test]
fn an_announcement_is_immediate_whatever_the_stagger_says() {
    let lua = fresh(4);
    let t = tick_that_is_not_bots_turn(&lua, 4);
    exec(&lua, "game_gives(4, 'stone-furnace', 1)");
    exec(&lua, &format!("announce_character_bot({t}, 4)"));

    assert_eq!(
        inventory_writeout_ticks(&lua),
        vec![t],
        "an announced bot is announced now, not up to a period later"
    );
}

/// **A craft settles on the tick it finishes, with no stagger anywhere near
/// it.** `poll_character_crafts` is what turns a queue drop into
/// `on_player_crafted_item`, and a plan has hundreds of crafts; adding a period
/// to each is the reason batching was rejected.
#[test]
fn a_craft_settles_on_the_tick_the_queue_drops() {
    let lua = fresh(4);
    exec(&lua, "poll_character_bots(500)");

    // The queue only ever becomes non-empty through `begin_crafting`, so the
    // mod is told about it the same way the game would be.
    exec(
        &lua,
        "storage.bots[2].craft_active = true; \
         _queue[2] = { { recipe = 'iron-gear-wheel', count = 2 } }",
    );
    let t = tick_that_is_not_bots_turn(&lua, 2);
    exec(&lua, &format!("poll_character_bots({t})"));
    assert_eq!(
        number(&lua, "#_crafted"),
        0,
        "nothing has finished yet: the queue still holds both"
    );

    exec(
        &lua,
        "_queue[2] = { { recipe = 'iron-gear-wheel', count = 1 } }",
    );
    exec(&lua, &format!("poll_character_bots({})", t + 1));
    assert_eq!(
        number(&lua, "#_crafted"),
        1,
        "one gear left the queue, so exactly one craft settles, on the tick it \
         happened"
    );
    assert_eq!(
        number(&lua, "_crafted[1].tick"),
        t + 1,
        "and it is stamped with that tick, not with a later batch's"
    );
}

/// The other half of the craft path: the queue is not read at all while there
/// is provably nothing in it.
///
/// This is exact rather than approximate. A character bot has no player at a
/// keyboard, so its queue can only become non-empty through `begin_crafting`,
/// which only `rcon_action_start_crafting` calls -- and that sets the flag.
#[test]
fn an_idle_bots_crafting_queue_is_not_read_at_all() {
    let lua = fresh(4);
    exec(&lua, "poll_character_bots(500)");
    exec(&lua, "_reads.queue = 0");

    for t in 1000..1100 {
        exec(&lua, &format!("poll_character_bots({t})"));
    }
    assert_eq!(
        number(&lua, "_reads.queue"),
        0,
        "four idle bots over 100 ticks used to cost 400 whole-queue scans"
    );

    // And the moment one is queued, the reads come back -- otherwise this test
    // would pass just as well against a mod that never polled crafts again.
    exec(
        &lua,
        "storage.bots[1].craft_active = true; \
         _queue[1] = { { recipe = 'iron-gear-wheel', count = 1 } }",
    );
    exec(&lua, "poll_character_bots(1100)");
    assert_eq!(
        number(&lua, "_reads.queue"),
        1,
        "exactly the one bot with a craft in flight is read"
    );
}
