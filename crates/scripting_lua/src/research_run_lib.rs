//! The shipped `research_run.lua`, and a smoke test that drives it against a
//! stub `rcon`/`goal`/`record`.
//!
//! `research_run.lua` is tracked (unlike `workspace/scripts/multibot.lua`,
//! `showcase.lua` and `record_smoke.lua`, which are gitignored scratch it is
//! modelled on) precisely because it is meant to ship: a release build
//! `include_dir!`-embeds `scripts/` at compile time, so an untracked driver
//! would not be there to run. That makes this file the only thing standing
//! between "it works on my workspace" and "it works from a clean checkout" --
//! see `supervisor_lib.rs` for the same argument made about `supervisor.lua`.
//!
//! The constant below is `include_str!` of `scripts/research_run.lua` itself,
//! never a copy, for the same reason `supervisor_lib::SUPERVISOR_LUA` is: a
//! copy drifts, and then the test passes against code that is not what runs.

/// `scripts/research_run.lua`, verbatim.
pub const RESEARCH_RUN_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/research_run.lua"
));

#[cfg(test)]
mod tests {
    use super::RESEARCH_RUN_LUA;
    use crate::supervisor_lib::SUPERVISOR_LUA;
    use factorio_bot_core::mlua::Lua;

    /// The supervisor runs inside the sandbox in production, and so does this
    /// driver -- see `supervisor_lib`'s own note on why `Lua::new` is banned
    /// outside `sandbox` by `clippy.toml`.
    fn sandboxed() -> Lua {
        crate::sandbox::new_sandboxed_lua().expect("sandboxed lua")
    }

    /// Stubs `rcon`, `goal` and `record`, and makes `include("supervisor.lua")`
    /// -- `research_run.lua`'s first line -- load the *real* shipped
    /// `supervisor.lua` via `load` on a text chunk, rather than a hand-rolled
    /// stand-in: this is the file that ships, so a test exercising anything
    /// else would not be testing it.
    ///
    /// `plan_steps` is a scripted sequence of step counts, one per call to
    /// `goal.plan`, exactly as `supervisor_lib::tests::harness` uses it.
    /// `roster` is a Lua table literal `rcon.players()` returns every time it
    /// is asked -- `research_run.lua`'s own wait loop only cares whether it is
    /// non-empty, so a fixed answer is enough to drive both the "bots showed
    /// up" and "no bots ever connected" paths.
    fn harness(plan_steps: &str, roster: &str) -> Lua {
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

            __plan_steps = PLAN_STEPS
            __plan_calls = 0

            rcon = {}
            rcon.players = function() return ROSTER end
            rcon.inventory_contents_at = function() return {} end

            goal = {}
            -- See `supervisor_lib::tests::harness`: the supervisor asks this
            -- before it reports a zero-step plan satisfied.
            __holds = true
            goal.holds = function(_g, _opts) return __holds end
            goal.have = function(item, count) return { kind = "have", item = item, count = count } end
            goal.researched = function(tech) return { kind = "researched", tech = tech } end
            goal.plan = function(_g, _opts)
                __plan_calls = __plan_calls + 1
                local n = __plan_steps[__plan_calls]
                if n == nil then error("stub: no scripted plan #" .. __plan_calls) end
                local steps = {}
                for i = 1, n do
                    steps[i] = {
                        id = i, bot = 1, label = "step " .. i,
                        start = (i - 1) * 10, finish = i * 10, deps = {},
                    }
                end
                return { steps = steps }
            end
            goal.run = function(_plan)
                -- `walks` is shaped like the real `obs.walks`
                -- (`build_observation`, `globals/goal/run.rs`): an array
                -- keyed by nothing, each entry naming its own
                -- `(bot, step_index)`, because a walk has no action id to be
                -- keyed by. One is enough to prove the driver forwards them.
                return { failed = 0, lost = 0, pending = 0, success = 0,
                         running = 0, done = true, actions = {},
                         walks = { { bot = 1, step_index = 0,
                                     to = { x = 4.5, y = -2.5 },
                                     status = "success",
                                     planned_start = 0, planned_end = 60,
                                     dispatched_tick = 1000,
                                     replied_tick = 1180 } } }
            end

            __plan_created_calls = {}
            __milestone_satisfied_calls = {}
            __milestone_stuck_calls = {}
            __finish_calls = {}
            record = {}
            record.start = function() return "run-test" end
            record.milestone_started = function(_index, _name) end
            record.plan_created = function(index, plan)
                table.insert(__plan_created_calls, { index = index, plan = plan })
            end
            record.milestone_satisfied = function(index, iterations, reason)
                table.insert(__milestone_satisfied_calls,
                    { index = index, iterations = iterations, reason = reason })
            end
            record.actions = function(_steps, _actions) return 0 end
            -- Counts *and keeps* what it was handed: the point of the wiring
            -- is that the walks of the batch reach the record, so a stub that
            -- only counted calls would pass against a driver that flushed an
            -- empty table.
            __walks_calls = {}
            record.walks = function(walks)
                table.insert(__walks_calls, walks)
                return #walks
            end
            record.teleports = function() return 0 end
            record.refusals = function() return 0 end
            record.milestone_stuck = function(index, outcome, last_error, best_steps)
                table.insert(__milestone_stuck_calls, { index = index, outcome = outcome,
                    last_error = last_error, best_steps = best_steps })
            end
            record.finish = function(outcome)
                table.insert(__finish_calls, outcome)
                return "run-test"
            end
        "#
        .replace("PLAN_STEPS", plan_steps)
        .replace("ROSTER", roster);
        lua.load(&stub).exec().expect("stub installs");
        lua
    }

    #[test]
    fn a_connected_roster_runs_every_milestone_to_completion() {
        const MILESTONES: u32 = 7;

        // Every milestone the shipped ladder declares, each satisfied on the
        // first plan.
        //
        // MILESTONES tracks `scripts/research_run.lua`'s `goals` table and must
        // be updated with it. Deriving it would be better, but the table is a
        // Lua local the harness cannot see, and a wrong count here fails loudly
        // -- which is how this constant was found when the ladder grew from
        // four rungs to seven.
        // One scripted zero-step plan per milestone, built from the same
        // constant the assertion uses. Hand-writing the list is what broke when
        // the ladder grew: `goal.plan` errors on an unscripted call, so four
        // entries against seven rungs crashed the run rather than failing the
        // count, which is a slower thing to read.
        let plans = format!(
            "{{{}}}",
            std::iter::repeat_n("0", MILESTONES as usize)
                .collect::<Vec<_>>()
                .join(", ")
        );
        let lua = harness(&plans, "{1, 2}");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("research_run.lua runs to completion");
        let g = lua.globals();
        let satisfied: mlua::Table = g.get("__milestone_satisfied_calls").unwrap();
        assert_eq!(
            satisfied.raw_len(),
            MILESTONES as usize,
            "every milestone the ladder declares must be satisfied"
        );
        for i in 1..=MILESTONES {
            let call: mlua::Table = satisfied.get(i).unwrap();
            assert_eq!(
                call.get::<String>("reason").unwrap(),
                "already_satisfied",
                "milestone {i}'s reason must reach record.milestone_satisfied -- \
                 the stub's `goal.holds` says the goal holds, so the record must \
                 say so too rather than the weaker `plan_empty` this used to \
                 report for every zero-step plan alike"
            );
        }
        let finish: mlua::Table = g.get("__finish_calls").unwrap();
        assert_eq!(finish.raw_len(), 1, "record.finish is called exactly once");
        assert_eq!(finish.get::<String>(1).unwrap(), "done");
    }

    #[test]
    fn no_connected_bots_aborts_without_raising_or_recording() {
        let lua = harness("{}", "{}");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("an empty roster must abort cleanly, not raise");
        let finish: mlua::Table = lua.globals().get("__finish_calls").unwrap();
        assert_eq!(
            finish.raw_len(),
            0,
            "record.start/finish are never reached when nobody connected"
        );
    }

    #[test]
    fn only_a_planning_attempt_with_steps_reaches_record_plan_created() {
        // Milestone 1 plans three times -- 2 steps, then 3, then 0 (the
        // empty re-plan that signals satisfaction) -- before the other three
        // milestones are each satisfied on their first (already-empty)
        // attempt. Six planning attempts in total, but `record.plan_created`
        // must be reached only for the two that returned real steps: the
        // empty re-plan that merely confirms satisfaction must never reach
        // it, because a consumer taking the LAST `plan_created` per
        // milestone would otherwise see an empty DAG for a milestone that
        // actually ran 3 steps.
        let lua = harness("{2, 3, 0, 0, 0, 0}", "{1}");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("research_run.lua runs to completion");
        let calls: mlua::Table = lua.globals().get("__plan_created_calls").unwrap();
        assert_eq!(
            calls.raw_len(),
            2,
            "only the two non-empty planning attempts reach record.plan_created"
        );
        let last: mlua::Table = calls.get(2).unwrap();
        assert_eq!(
            last.get::<u32>("index").unwrap(),
            1,
            "milestone 1 is the only one that ever plans a non-empty step"
        );
        let plan: mlua::Table = last.get("plan").unwrap();
        assert_eq!(
            plan.raw_len(),
            3,
            "the LAST plan_created for a satisfied milestone must be its real DAG, \
             not the empty re-plan that closed it"
        );
    }

    #[test]
    fn a_raise_during_planning_is_recorded_with_the_callers_error_text() {
        // An empty scripted plan list means `goal.plan`'s very first call
        // finds nothing to return and raises -- exactly the "construction
        // error" `supervisor.lua` deliberately leaves unwrapped (see its own
        // comment on `Sup:step`'s "planning" branch). `research_run.lua`'s
        // outer `pcall` must catch it, pass the caught text through to
        // `record.milestone_stuck` as `last_error`, and still close the
        // recording -- a run that raises is the one most worth being able to
        // read afterwards.
        let lua = harness("{}", "{1}");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("the raise must be caught, not propagate out of the script");

        let stuck: mlua::Table = lua.globals().get("__milestone_stuck_calls").unwrap();
        assert_eq!(stuck.raw_len(), 1, "exactly one stuck milestone recorded");
        let call: mlua::Table = stuck.get(1).unwrap();
        assert_eq!(call.get::<u32>("index").unwrap(), 1);
        assert_eq!(call.get::<String>("outcome").unwrap(), "plan_error");
        let last_error: String = call.get("last_error").unwrap();
        assert!(
            last_error.contains("no scripted plan"),
            "the pcall's own caught text must reach the record verbatim: {last_error}"
        );

        let finish: mlua::Table = lua.globals().get("__finish_calls").unwrap();
        assert_eq!(
            finish.raw_len(),
            1,
            "record.finish is still called after a raise"
        );
        assert_eq!(finish.get::<String>(1).unwrap(), "crashed");
    }

    /// **The batch's walks reach the record.**
    ///
    /// The driver flushes actions, teleports and refusals once per "ran"
    /// transition; walks are the fourth, and were missing entirely until
    /// 2026-09-02 -- a run could fail three walks and leave one `last_error`
    /// string in its record, with the other two only in
    /// `workspace/server-log.txt`, which the next run overwrites.
    ///
    /// A walk is not an action and is not in `t.actions`: it has no action id,
    /// so it travels as its own array and gets its own call. This asserts the
    /// call happened *and* that it carried the walk, because a driver that
    /// flushed an empty table would satisfy a call count and record nothing.
    #[test]
    fn the_batchs_walks_are_flushed_to_the_record_on_every_ran_transition() {
        // Milestone 1 plans two steps, runs them, then re-plans empty; the
        // remaining six rungs are satisfied on their first attempt. Exactly
        // one "ran" transition, so exactly one walk flush.
        let lua = harness("{2, 0, 0, 0, 0, 0, 0, 0}", "{1, 2}");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("research_run.lua runs to completion");
        let calls: mlua::Table = lua.globals().get("__walks_calls").unwrap();
        assert_eq!(
            calls.raw_len(),
            1,
            "one executed plan, one flush of its walks"
        );
        let walks: mlua::Table = calls.get(1).unwrap();
        assert_eq!(
            walks.raw_len(),
            1,
            "and the flush carried the observation's walk, not an empty table"
        );
        let walk: mlua::Table = walks.get(1).unwrap();
        assert_eq!(walk.get::<u32>("bot").unwrap(), 1);
        assert_eq!(
            walk.get::<u32>("step_index").unwrap(),
            0,
            "`(bot, step_index)` is the only thing that names a walk, so both \
             halves have to survive the hand-off"
        );
    }

    // ---- the roster the run is built with --------------------------------

    /// **Freeplay hides player 1 for 750 ticks, and this script used to
    /// believe the gap.**
    ///
    /// `crash_site.create_cutscene` is gated on `player_index == 1`, and a
    /// player in a cutscene has no `character` -- which is exactly what
    /// `rcon_players()` filters on. So for 12.5 seconds after joining, player
    /// 1 alone is invisible to `rcon.players()`. `wait_for_roster` returned
    /// the first non-empty answer and froze it for the whole run: in run 30
    /// (`workspace/runs/run-1788365280-15443/`) that answer was `[2]`, and
    /// bots 1, 3 and 4 stood still for 162,158 ticks while the record showed
    /// four connected clients. Four of nineteen archived runs did this.
    ///
    /// The run's roster is not a mystery -- `all_bots` is what it was started
    /// with -- so the wait is for *that set*, not for anybody at all. Here the
    /// first two answers are missing bot 1, exactly as the cutscene makes
    /// them, and the roster the milestones are planned for must still be all
    /// four.
    #[test]
    fn the_roster_waits_for_every_bot_the_run_was_started_with() {
        let lua = harness("{}", "{}");
        lua.load(
            r#"
            all_bots = {1, 2, 3, 4}
            __players_calls = 0
            rcon.players = function()
                __players_calls = __players_calls + 1
                -- The cutscene: bot 1 has no character yet, so the mod does
                -- not report it.
                if __players_calls < 3 then return {2, 3, 4} end
                return {1, 2, 3, 4}
            end
            __planned_with = nil
            goal.plan = function(_g, opts)
                __planned_with = opts and opts.bots
                return { steps = {} }
            end
        "#,
        )
        .exec()
        .expect("stub installs");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("research_run.lua runs");

        let planned: mlua::Table = lua
            .globals()
            .get("__planned_with")
            .expect("the milestones must have been planned for some roster");
        let bots: Vec<u32> = planned
            .sequence_values::<u32>()
            .collect::<mlua::Result<_>>()
            .unwrap();
        assert_eq!(
            bots,
            vec![1, 2, 3, 4],
            "the run must plan for every bot it was started with; taking the \
             first answer costs three of four bots for the whole run"
        );
    }

    /// The other side of the same decision: a client that genuinely never
    /// arrives must not cost the run.
    ///
    /// Refusing to start without the full roster would throw away a run whose
    /// fourth client failed to launch -- and that case is real, which is why
    /// `Planner::roster` reports only the clients that connected. So the wait
    /// gives up, **names who is missing**, and proceeds with whoever is there.
    /// Silently freezing a short roster is what produced run 30; saying so is
    /// not.
    #[test]
    fn a_bot_that_never_appears_is_named_and_the_run_proceeds_without_it() {
        let lua = harness("{}", "{}");
        lua.load(
            r#"
            all_bots = {1, 2, 3, 4}
            rcon.players = function() return {1, 2, 4} end
            __printed = {}
            print = function(...)
                local args = {...}
                table.insert(__printed, table.concat(args, " "))
            end
            __planned_with = nil
            goal.plan = function(_g, opts)
                __planned_with = opts and opts.bots
                return { steps = {} }
            end
        "#,
        )
        .exec()
        .expect("stub installs");
        lua.load(RESEARCH_RUN_LUA)
            .exec()
            .expect("a missing client must not abort a run three bots can do");

        let planned: mlua::Table = lua.globals().get("__planned_with").expect("planned");
        let bots: Vec<u32> = planned
            .sequence_values::<u32>()
            .collect::<mlua::Result<_>>()
            .unwrap();
        assert_eq!(bots, vec![1, 2, 4], "the run proceeds with who showed up");

        let printed: Vec<String> = lua
            .load("return __printed")
            .eval::<mlua::Table>()
            .unwrap()
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()
            .unwrap();
        let warning = printed
            .iter()
            .find(|line| line.contains("WARNING"))
            .unwrap_or_else(|| panic!("the short roster must be said out loud: {printed:?}"));
        assert!(
            warning.contains("3") && warning.contains("4 bot"),
            "the warning must say how many of how many: {warning}"
        );
        assert!(
            warning.contains("never appeared: 3"),
            "and it must name who is missing, or the next reader is back to \
             counting dispatches per bot: {warning}"
        );

        let finish: mlua::Table = lua.globals().get("__finish_calls").unwrap();
        assert_eq!(finish.raw_len(), 1, "the run still records and finishes");
    }
}
