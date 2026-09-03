//! The shipped `scripts/factory_stage1.lua`, and the tests that hold its two
//! rungs to what they claim.
//!
//! Same argument as `supervisor_lib` and `research_run_lib`: this file ships --
//! a release build `include_dir!`-embeds `scripts/` at compile time -- so the
//! constant below is `include_str!` of the real script, never a copy, and the
//! tests drive that script against stub `rcon`/`goal`/`record` tables.
//!
//! What is worth testing here is not the loop (that is `supervisor_lib`'s job)
//! but the **ladder**: that the run has a witness rung at all, that it comes
//! after the `Producing` rung, and that a cell which stands and makes nothing
//! takes the run down as `stuck` rather than being reported built. A witness
//! nobody runs is theatre, and this is what says it is run.

/// `scripts/factory_stage1.lua`, verbatim.
pub const FACTORY_STAGE1_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/factory_stage1.lua"
));

#[cfg(test)]
mod tests {
    use super::FACTORY_STAGE1_LUA;
    use crate::supervisor_lib::SUPERVISOR_LUA;
    use factorio_bot_core::mlua::Lua;

    fn sandboxed() -> Lua {
        crate::sandbox::new_sandboxed_lua().expect("sandboxed lua")
    }

    /// Stubs the whole surface `factory_stage1.lua` touches, and makes its
    /// first line -- `include("supervisor.lua")` -- load the **real** shipped
    /// supervisor, exactly as `research_run_lib` does.
    ///
    /// The world is the stage-1 cell with the game's own numbers: a burner
    /// drill at `(-35, 35)` whose reported `drop_position` is `(-35.35, 33.7)`,
    /// the stone furnace two tiles north whose collision box ends at
    /// `33.69921875`, and a hand-smelt furnace nothing drops into holding two
    /// plates a bot put there.
    ///
    /// `produce_from_tick` is `"nil"` for a cell that stands and makes nothing.
    fn harness(produce_from_tick: &str, produce_count: u32) -> Lua {
        let lua = sandboxed();
        lua.globals()
            .set("__supervisor_src", SUPERVISOR_LUA)
            .expect("set supervisor source");
        let stub = r#"
            include = function(name)
                if name ~= "supervisor.lua" then
                    error("stub include: unrecognised " .. tostring(name))
                end
                local chunk, err = load(__supervisor_src, "supervisor.lua")
                if chunk == nil then error("stub include: " .. tostring(err)) end
                return chunk()
            end

            __drill = {
                name = "burner-mining-drill", position = {x = -35, y = 35}, direction = 0,
                drop_position = {x = -35.35, y = 33.7},
                bounding_box = { left_top = {x = -35.9, y = 34.1},
                                 right_bottom = {x = -34.1, y = 35.9} },
            }
            __furnace = {
                name = "stone-furnace", position = {x = -35, y = 33}, direction = 0,
                bounding_box = { left_top = {x = -35.69921875, y = 32.30078125},
                                 right_bottom = {x = -34.30078125, y = 33.69921875} },
                output_inventory = {},
            }
            __decoy = {
                name = "stone-furnace", position = {x = -37, y = 33}, direction = 0,
                bounding_box = { left_top = {x = -37.69921875, y = 32.30078125},
                                 right_bottom = {x = -36.30078125, y = 33.69921875} },
                output_inventory = { { name = "iron-plate", quality = "normal", count = 2 } },
            }
            __world = { __drill, __furnace, __decoy }
            __produce_from_tick = PRODUCE_FROM
            __produce_count = PRODUCE_COUNT

            __tick = 1000
            rcon = {}
            rcon.players = function() return {1, 2} end
            rcon.inventory_contents_at = function() return {} end
            -- One tick per round trip: Factorio processes rcon once a tick, so
            -- that is the floor a real poll loop is paced by.
            rcon.game_tick = function() __tick = __tick + 1; return __tick end
            rcon.find_entities_in_radius = function(_c, _r, name)
                if __produce_from_tick ~= nil and __tick >= __produce_from_tick then
                    __furnace.output_inventory = {
                        { name = "iron-plate", quality = "normal", count = __produce_count }
                    }
                end
                local out = {}
                for _, e in ipairs(__world) do
                    if name == nil or e.name == name then out[#out + 1] = e end
                end
                return out
            end

            goal = {}
            __plan_calls = 0
            __holds = true
            goal.holds = function(_g, _o) return __holds end
            goal.producing = function(item, per_minute)
                return { kind = "producing", item = item, per_minute = per_minute }
            end
            goal.plan = function(_g, _o) __plan_calls = __plan_calls + 1; return { steps = {} } end
            goal.run = function(_p) return { done = true, actions = {}, walks = {} } end

            __started = {}
            __satisfied = {}
            __stuck = {}
            __finish = {}
            __printed = {}
            print = function(...)
                local args = {...}
                for i = 1, #args do args[i] = tostring(args[i]) end
                table.insert(__printed, table.concat(args, " "))
            end
            record = {}
            record.start = function() return "run-test" end
            record.milestone_started = function(index, name)
                table.insert(__started, { index = index, name = name })
            end
            record.plan_created = function() end
            record.milestone_satisfied = function(index, iterations, reason)
                if reason ~= "already_satisfied" and reason ~= "plan_empty" then
                    error("milestone_satisfied: unknown reason \"" .. tostring(reason) .. "\"")
                end
                table.insert(__satisfied, { index = index, reason = reason })
            end
            record.milestone_stuck = function(index, outcome, last_error, best)
                table.insert(__stuck, { index = index, outcome = outcome,
                                        last_error = last_error, best = best })
            end
            record.actions = function() return 0 end
            record.walks = function() return 0 end
            record.teleports = function() return 0 end
            record.refusals = function() return 0 end
            record.enclosures = function() return 0 end
            record.keyframe = function() end
            record.finish = function(outcome)
                table.insert(__finish, outcome)
                return "run-test"
            end
        "#
        .replace("PRODUCE_FROM", produce_from_tick)
        .replace("PRODUCE_COUNT", &produce_count.to_string());
        lua.load(&stub).exec().expect("stub installs");
        lua
    }

    fn strings(lua: &Lua, global: &str) -> Vec<String> {
        lua.load(format!("return {global}"))
            .eval::<mlua::Table>()
            .expect("table")
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()
            .expect("strings")
    }

    /// **The ladder has a witness in it, and it comes second.**
    ///
    /// The whole of stage 1's "done when" is `Producing{iron-plate, 15}` plans,
    /// builds, *and* a witness with the bots idle shows the furnace's output
    /// rising. A run that stops after the first is a run that proved a machine
    /// stands. This is the test that says the second rung is really wired --
    /// without it, `supervisor.witness` would be a library nothing calls.
    #[test]
    fn the_ladder_witnesses_the_cell_it_just_built() {
        let lua = harness("1120", 3);
        lua.load(FACTORY_STAGE1_LUA)
            .exec()
            .expect("factory_stage1.lua runs to completion");

        let started: mlua::Table = lua.globals().get("__started").unwrap();
        assert_eq!(started.raw_len(), 2, "two rungs: build, then witness");
        let second: mlua::Table = started.get(2).unwrap();
        let name: String = second.get("name").unwrap();
        assert!(
            name.contains("witness"),
            "the second rung must say what it is, because the record's own \
             `SatisfiedReason` cannot: {name}"
        );

        let satisfied: mlua::Table = lua.globals().get("__satisfied").unwrap();
        assert_eq!(satisfied.raw_len(), 2, "both rungs closed satisfied");
        assert_eq!(
            lua.globals().get::<i64>("__plan_calls").unwrap(),
            1,
            "the build rung planned once; the witness rung planned not at all"
        );

        let printed = strings(&lua, "__printed");
        let line = printed
            .iter()
            .find(|l| l.contains("WITNESSED"))
            .unwrap_or_else(|| panic!("the observation must be printed: {printed:?}"));
        assert!(
            line.contains("0 -> 3") && line.contains("wanted 1"),
            "and it must print the numbers, not a verdict word: {line}"
        );
        assert_eq!(
            lua.globals()
                .get::<mlua::Table>("__finish")
                .unwrap()
                .get::<String>(1)
                .unwrap(),
            "done"
        );
    }

    /// **A cell that stands and makes nothing takes the run down.**
    ///
    /// This is the whole point: without the witness rung this run reported
    /// `done` for a factory whose only evidence was that two machines were
    /// placed. With it, the same world ends `stuck`, and the reason on the
    /// record names the cell rather than blaming a plan.
    #[test]
    fn a_cell_that_produces_nothing_ends_the_run_stuck_rather_than_done() {
        let lua = harness("nil", 0);
        lua.load(FACTORY_STAGE1_LUA)
            .exec()
            .expect("factory_stage1.lua runs to completion");

        let satisfied: mlua::Table = lua.globals().get("__satisfied").unwrap();
        assert_eq!(
            satisfied.raw_len(),
            1,
            "the build rung is satisfied -- the cell really does stand -- and \
             the witness rung is not"
        );
        let stuck: mlua::Table = lua.globals().get("__stuck").unwrap();
        assert_eq!(stuck.raw_len(), 1, "exactly one stuck milestone");
        let call: mlua::Table = stuck.get(1).unwrap();
        assert_eq!(call.get::<u32>("index").unwrap(), 2);
        assert_eq!(call.get::<String>("outcome").unwrap(), "stuck");
        let why: String = call.get("last_error").unwrap();
        assert!(
            why.contains("stands and produces nothing"),
            "the record must carry the witness's own sentence: {why}"
        );
        assert_eq!(
            lua.globals()
                .get::<mlua::Table>("__finish")
                .unwrap()
                .get::<String>(1)
                .unwrap(),
            "stuck",
            "and the run must not finish `done` for a factory that makes nothing"
        );
    }

    /// The negative control for the one above. Without it, "always halt on the
    /// witness rung" would satisfy every assertion there and break every run
    /// that works -- which is the same shape of mistake as reporting an empty
    /// plan satisfied.
    #[test]
    fn a_run_with_no_bots_never_reaches_the_witness_at_all() {
        let lua = harness("nil", 0);
        lua.load("rcon.players = function() return {} end")
            .exec()
            .expect("stub set");
        lua.load(FACTORY_STAGE1_LUA)
            .exec()
            .expect("an empty roster aborts cleanly, not by raising");
        assert_eq!(
            lua.globals()
                .get::<mlua::Table>("__finish")
                .unwrap()
                .raw_len(),
            0,
            "nothing is recorded when nobody connected"
        );
    }
}
