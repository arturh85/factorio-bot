//! The shipped `supervisor.lua`, and the tests that hold it to its spec.
//!
//! The supervisor is a Lua library rather than Rust so its policy stays
//! readable and editable without a rebuild. That would normally make the most
//! decision-dense component the least tested part of a workspace with a
//! thousand Rust tests; it does not, because the tests below drive the real
//! shipped file against a stub `goal` table.
//!
//! The constant is `include_str!` of `scripts/supervisor.lua` itself, never a
//! copy. A copy drifts, and then the tests pass against code that is not what
//! runs.
//!
//! See `docs/superpowers/specs/2026-08-31-supervisor-loop-design.md`.

/// `scripts/supervisor.lua`, verbatim.
pub const SUPERVISOR_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/supervisor.lua"
));

#[cfg(test)]
mod tests {
    use super::SUPERVISOR_LUA;
    use factorio_bot_core::mlua::Lua;

    /// The supervisor runs inside the sandbox in production, so it is tested
    /// there too. A permissive `Lua::new` would let a script that reaches for
    /// `os` or `io` pass here and fail in the only environment that matters --
    /// which is why `clippy.toml` bans building one outside `sandbox`.
    fn sandboxed() -> Lua {
        crate::sandbox::new_sandboxed_lua().expect("sandboxed lua")
    }

    /// A Lua state holding the real supervisor plus a stub `goal` table whose
    /// `plan` returns a scripted sequence of step counts and whose `run`
    /// returns scripted observations.
    ///
    /// `plan_steps` entries are step counts; the string `"raise"` makes
    /// `goal.plan` raise, standing in for an unknown item or technology.
    fn harness(plan_steps: &str, run_obs: &str) -> Lua {
        let lua = sandboxed();
        let stub = r#"
            __plan_steps = PLAN_STEPS
            __run_obs = RUN_OBS
            __plan_calls = 0
            __run_calls = 0
            goal = {}
            goal.plan = function(_g, _opts)
                __plan_calls = __plan_calls + 1
                local n = __plan_steps[__plan_calls]
                if n == nil then error("stub: no scripted plan #" .. __plan_calls) end
                if n == "raise" then error("stub: unknown item") end
                local steps = {}
                for i = 1, n do steps[i] = i end
                return { steps = steps }
            end
            goal.run = function(_plan)
                __run_calls = __run_calls + 1
                local o = __run_obs[__run_calls] or {}
                return { failed = o.failed or 0, lost = o.lost or 0,
                         first_error = o.first_error }
            end
        "#
        .replace("PLAN_STEPS", plan_steps)
        .replace("RUN_OBS", run_obs);
        lua.load(&stub).exec().expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        lua
    }

    /// Drive to a terminal state and report `(state, plan_calls, run_calls)`.
    fn drive(lua: &Lua, opts: &str) -> (String, i64, i64) {
        let driver = r#"
            local sup = supervisor.new(supervisor.list {"a"}, OPTS)
            local guard = 0
            repeat
                sup:step()
                guard = guard + 1
                if guard > 500 then error("did not terminate") end
            until sup:finished()
            __state = sup.state
            __report = sup:report()
            __history_len = #sup:history()
        "#
        .replace("OPTS", opts);
        lua.load(&driver).exec().expect("driver runs");
        let g = lua.globals();
        (
            g.get::<String>("__state").unwrap(),
            g.get::<i64>("__plan_calls").unwrap(),
            g.get::<i64>("__run_calls").unwrap(),
        )
    }

    // ---- Layer 1: the tracker, plain integers, no goals or plans ----------

    fn observe(seq: &[i64]) -> (Option<i64>, i64) {
        let lua = harness("{}", "{}");
        let src = format!(
            "local t = supervisor.tracker.new()
             for _, n in ipairs({{{}}}) do t = supervisor.tracker.observe(t, n) end
             __best, __stall = t.best, t.stall",
            seq.iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        lua.load(&src).exec().unwrap();
        let g = lua.globals();
        (
            g.get::<Option<i64>>("__best").unwrap(),
            g.get::<i64>("__stall").unwrap(),
        )
    }

    #[test]
    fn a_new_best_resets_the_stall() {
        assert_eq!(observe(&[10, 10, 6]), (Some(6), 0));
    }

    #[test]
    fn an_equal_step_count_is_not_an_improvement() {
        assert_eq!(observe(&[10, 10]), (Some(10), 1));
    }

    #[test]
    fn an_increase_is_not_an_improvement() {
        assert_eq!(observe(&[10, 12]), (Some(10), 1));
    }

    #[test]
    fn the_stall_counts_consecutive_non_improvements() {
        assert_eq!(observe(&[10, 10, 10, 10]), (Some(10), 3));
    }

    // ---- Layer 2: the step machine against the stub -----------------------

    #[test]
    fn a_satisfied_milestone_never_runs_anything() {
        let lua = harness("{0}", "{}");
        let (state, plans, runs) = drive(&lua, "{}");
        assert_eq!(state, "done");
        assert_eq!(plans, 1);
        assert_eq!(runs, 0, "an empty plan must not be executed");
    }

    #[test]
    fn a_converging_milestone_runs_once_per_non_empty_plan() {
        // 10 -> run -> 6 -> run -> 0 -> satisfied
        let lua = harness("{10, 6, 0}", "{}");
        let (state, plans, runs) = drive(&lua, "{}");
        assert_eq!(state, "done");
        assert_eq!(plans, 3);
        assert_eq!(runs, 2);
    }

    #[test]
    fn no_progress_halts_at_the_stall_limit_without_a_further_run() {
        // best set on plan 1; stall reaches 3 on plan 4, which must not run.
        let lua = harness("{10, 10, 10, 10}", "{{failed=1, first_error='boom'}}");
        let (state, plans, runs) = drive(&lua, "{stall_limit = 3}");
        assert_eq!(state, "stuck");
        assert_eq!(plans, 4);
        assert_eq!(
            runs, 3,
            "the plan that trips the stall must not be executed"
        );
    }

    #[test]
    fn no_progress_with_every_run_succeeding_is_reported_as_silent() {
        let lua = harness("{10, 10, 10, 10}", "{}");
        let (state, _, _) = drive(&lua, "{stall_limit = 3}");
        assert_eq!(
            state, "stuck_silent",
            "no failures anywhere means something claimed success it did not deliver"
        );
    }

    #[test]
    fn the_cap_is_exhausted_not_stuck_even_though_every_run_succeeded() {
        // Improves by one every iteration, so the stall never fires and the cap
        // does. Progress was happening; calling that `stuck_silent` would
        // accuse the executor of lying about work it actually did.
        let lua = harness("{10, 9, 8, 7, 6, 5}", "{}");
        let (state, _, runs) = drive(&lua, "{max_iterations = 5}");
        assert_eq!(state, "exhausted");
        assert_eq!(runs, 4, "the plan that trips the cap must not be executed");
    }

    #[test]
    fn a_construction_error_propagates_and_is_not_retried() {
        let lua = harness("{'raise'}", "{}");
        let err = lua
            .load(
                "local sup = supervisor.new(supervisor.list {'a'}, {}) \
                   repeat sup:step() until sup:finished()",
            )
            .exec()
            .expect_err("a plan that raises must propagate");
        assert!(
            format!("{err}").contains("unknown item"),
            "the original error must survive, got: {err}"
        );
        assert_eq!(
            lua.globals().get::<i64>("__plan_calls").unwrap(),
            1,
            "a construction error must not be retried"
        );
    }

    #[test]
    fn the_source_is_asked_again_after_a_milestone_is_satisfied() {
        let lua = harness("{0, 0}", "{}");
        lua.load(
            "local asked = 0
             local sup = supervisor.new(function(_h)
                 asked = asked + 1
                 if asked <= 2 then return 'g' end
                 return nil
             end, {})
             repeat sup:step() until sup:finished()
             __asked, __state, __closed = asked, sup.state, #sup:history()",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__asked").unwrap(), 3, "two goals, then nil");
        assert_eq!(g.get::<String>("__state").unwrap(), "done");
        assert_eq!(g.get::<i64>("__closed").unwrap(), 2);
    }

    #[test]
    fn a_failed_run_is_recorded_and_replanned_rather_than_halting() {
        // Fails once, then converges. A world condition is the normal trigger.
        let lua = harness(
            "{10, 6, 0}",
            "{{failed=2, first_error='no entity to mine'}}",
        );
        let (state, _, runs) = drive(&lua, "{}");
        assert_eq!(state, "done");
        assert_eq!(runs, 2);
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             __ignored = 0",
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn a_bad_source_is_refused_at_construction() {
        let lua = harness("{}", "{}");
        let err = lua
            .load("supervisor.new({'not', 'a', 'function'}, {})")
            .exec()
            .expect_err("a non-function source must raise");
        assert!(
            format!("{err}").contains("must be a function"),
            "got: {err}"
        );
    }
}
