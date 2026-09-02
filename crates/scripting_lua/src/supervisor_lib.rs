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
    ///
    /// Each scheduled step is a real table -- `id`, `bot`, `label`, `start`,
    /// `finish` -- in the shape `PlanValue.steps` (`goal/plan.rs`) actually
    /// hands back, chained `id = i - 1` depends on `id = i - 2` so `deps` has
    /// something to carry, not bare integers: `Sup:step()` now reads these
    /// fields to build `t.plan` for `record.plan_created`, and a stub that
    /// stayed at "steps are just their own index" would pass while the real
    /// field-by-field read it exercises stayed untested.
    fn harness(plan_steps: &str, run_obs: &str) -> Lua {
        let lua = sandboxed();
        let stub = r#"
            __plan_steps = PLAN_STEPS
            __run_obs = RUN_OBS
            __plan_calls = 0
            __run_calls = 0
            __keyframe_calls = 0
            -- Present in every test here, the way `record` is only installed
            -- alongside a live game connection in production: proves the
            -- milestone-boundary call happens when recording is available,
            -- without needing a second harness just for that.
            record = {}
            record.keyframe = function()
                __keyframe_calls = __keyframe_calls + 1
            end
            goal = {}
            -- What `goal.holds` answers. `true` by default, because the
            -- ordinary reason a plan comes back empty is that the goal is
            -- already met; a test that wants the other two answers sets this
            -- to `false` (the planner contradicting the world) or `nil` (a
            -- goal possession cannot settle).
            __holds = true
            __holds_calls = 0
            goal.holds = function(_g, _opts)
                __holds_calls = __holds_calls + 1
                return __holds
            end
            -- The Rust classifier's stand-in (`goal.refusal`, `goal/mod.rs`).
            -- The real one reads a code off the error value; this one answers
            -- for whatever `goal.plan` raised last, so a test can drive both
            -- branches without matching on message text -- which is exactly
            -- what the classifier exists to stop callers doing.
            --
            -- The message is run 31's own
            -- (`workspace/runs/run-1788372605-35170/`), verbatim.
            __refusal_message = "automation needs a lab with 60 kW of electric supply, and the plan can show only 0 kW"
            __last_raise_was_a_refusal = false
            goal.refusal = function(_err)
                if not __last_raise_was_a_refusal then return nil end
                return { code = "planner::research_needs_power",
                         message = __refusal_message }
            end
            goal.plan = function(_g, _opts)
                __plan_calls = __plan_calls + 1
                local n = __plan_steps[__plan_calls]
                if n == "refuse" then
                    __last_raise_was_a_refusal = true
                    error("goal: " .. __refusal_message, 0)
                end
                __last_raise_was_a_refusal = false
                if n == nil then error("stub: no scripted plan #" .. __plan_calls) end
                if n == "raise" then error("stub: unknown item") end
                local steps = {}
                for i = 1, n do
                    steps[i] = {
                        id = i, bot = 1, label = "step " .. i,
                        start = (i - 1) * 10, finish = i * 10,
                        deps = (i > 1) and { i - 1 } or {},
                    }
                end
                return { steps = steps }
            end
            goal.run = function(_plan)
                __run_calls = __run_calls + 1
                local o = __run_obs[__run_calls] or {}
                return { failed = o.failed or 0, lost = o.lost or 0,
                         walks_failed = o.walks_failed or 0,
                         walks_lost = o.walks_lost or 0,
                         pending = o.pending or 0, success = o.success or 0,
                         running = 0, done = true,
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

    /// **A run whose walks failed is not a silent one.**
    ///
    /// `stuck_silent` means "no progress and every run claimed success", and it
    /// carries no error because there is supposed to be none to carry. A walk
    /// failure produces no failed *action*, so the loop used to see
    /// `failed = 0` and call that silence — which is how
    /// `workspace/runs/run-1788341905-92036` was recorded `stuck_silent` with
    /// `last_error: null` while the pathfinder's refusal was logged once per
    /// iteration.
    #[test]
    fn walks_that_failed_are_failures_and_the_halt_carries_their_error() {
        let lua = harness(
            "{10, 10, 10, 10}",
            "{{walks_failed=2, pending=10, first_error='the pathfinder returned no path'}}",
        );
        let (state, _, _) = drive(&lua, "{stall_limit = 3}");
        assert_eq!(
            state, "stuck",
            "a failed walk is a failure; calling it silence hides the only \
             evidence the run produced"
        );
        let report: String = lua.globals().get("__report").expect("__report");
        assert!(
            report.contains("the pathfinder returned no path"),
            "the halt has to carry the reason, got {report}"
        );
    }

    /// The lost-walk half of the same hole. A walk the run lost track of gets
    /// no verdict at all, so it is in neither `failed` nor `walks_failed`; a
    /// teleport spin is precisely that, and `run-1788344167-58471` spent four
    /// executor deadlines on one before anybody could see a reason.
    #[test]
    fn walks_the_run_lost_track_of_are_failures_too() {
        let lua = harness(
            "{10, 10, 10, 10}",
            "{{walks_lost=1, pending=10, first_error='no action result received in time'}}",
        );
        let (state, _, _) = drive(&lua, "{stall_limit = 3}");
        assert_eq!(
            state, "stuck",
            "a walk that never answered is not a run that claimed success"
        );
        let report: String = lua.globals().get("__report").expect("__report");
        assert!(
            report.contains("no action result received in time"),
            "the halt has to carry the reason, got {report}"
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
    fn a_run_reports_its_whole_tally_not_just_failures() {
        // A run that dispatched everything and learned nothing back and a run
        // that dispatched nothing at all both report `failed = 0`. They are
        // completely different events, and `pending` is what separates them --
        // so a caller must not have to guess from the one number.
        let lua = harness("{5, 0}", "{{failed=0, pending=5, success=0}}");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             local seen
             repeat
                 local t = sup:step()
                 if t.action == 'ran' then seen = t end
             until sup:finished()
             __pending, __success, __failed = seen.pending, seen.success, seen.failed",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__pending").unwrap(),
            5,
            "nothing was dispatched"
        );
        assert_eq!(g.get::<i64>("__success").unwrap(), 0);
        assert_eq!(
            g.get::<i64>("__failed").unwrap(),
            0,
            "and `failed` alone would have called that a clean run"
        );
    }

    /// **The four trouble counts reach the driver as four counts.**
    ///
    /// `failed`, `lost`, `walks_failed` and `walks_lost` are four different
    /// facts along two axes -- the game judging the attempt and saying no
    /// versus the game never answering, for actions versus for walks. The
    /// transition used to hand the driver their SUM under the name `failed`,
    /// while `lost` rode along separately, so a run with one lost action and
    /// nothing else wrong printed `failed=1 lost=1` -- which reads as two
    /// problems and is one, counted twice. That line misdirected a live
    /// diagnosis. Nothing named `failed` may carry anything but `obs.failed`.
    #[test]
    fn the_four_trouble_counts_reach_the_driver_separately() {
        let lua = harness("{5, 0}", "{{lost=1, walks_lost=1, pending=3, success=1}}");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             local seen
             repeat
                 local t = sup:step()
                 if t.action == 'ran' then seen = t end
             until sup:finished()
             __failed, __lost = seen.failed, seen.lost
             __walks_failed, __walks_lost = seen.walks_failed, seen.walks_lost",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__failed").unwrap(),
            0,
            "the game refused nothing; `failed` must not absorb the other three"
        );
        assert_eq!(g.get::<i64>("__lost").unwrap(), 1);
        assert_eq!(g.get::<i64>("__walks_failed").unwrap(), 0);
        assert_eq!(
            g.get::<i64>("__walks_lost").unwrap(),
            1,
            "a walk nobody heard back about is its own fact and must be reachable"
        );
    }

    /// The other half: keeping the four apart must not quietly un-do what the
    /// sum was for. A run whose *only* trouble is a failed walk still has to
    /// halt as `stuck` rather than `stuck_silent` (the two tests above pin
    /// that), and the driver must still be able to see the walk counts that
    /// made it so -- which is what this checks from the transition's side.
    #[test]
    fn a_failed_walk_is_visible_on_the_transition_that_reported_it() {
        let lua = harness("{5, 0}", "{{walks_failed=2, pending=5}}");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             local seen
             repeat
                 local t = sup:step()
                 if t.action == 'ran' then seen = t end
             until sup:finished()
             __failed, __walks_failed = seen.failed, seen.walks_failed",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__failed").unwrap(), 0);
        assert_eq!(g.get::<i64>("__walks_failed").unwrap(), 2);
    }

    #[test]
    fn a_keyframe_is_written_once_per_closed_milestone_when_recording() {
        // 10 -> 6 -> 0: one milestone, satisfied on the third plan. `_close`
        // runs exactly once, so exactly one keyframe -- not one per iteration,
        // not one per run, and not zero because there is deliberately no tick
        // timer driving this.
        let lua = harness("{10, 6, 0}", "{}");
        let (state, _plans, _runs) = drive(&lua, "{}");
        assert_eq!(state, "done");
        assert_eq!(
            lua.globals().get::<i64>("__keyframe_calls").unwrap(),
            1,
            "one milestone was closed, so exactly one keyframe"
        );
    }

    #[test]
    fn two_closed_milestones_write_two_keyframes() {
        let lua = harness("{0, 0}", "{}");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a', 'b'}, {})
             repeat sup:step() until sup:finished()
             __state = sup.state",
        )
        .exec()
        .unwrap();
        assert_eq!(lua.globals().get::<String>("__state").unwrap(), "done");
        assert_eq!(lua.globals().get::<i64>("__keyframe_calls").unwrap(), 2);
    }

    #[test]
    fn the_loop_works_when_no_recording_is_installed_at_all() {
        // Every other test in this file installs a `record` stub; this one
        // deliberately does not, because that is the common case in
        // production too -- `record` exists only alongside a live game
        // connection, and most callers of `supervisor` (every planning-only
        // script, most of the harness above until the stub was added) have
        // no such connection. The milestone-boundary keyframe call must be
        // inert then, not a hard failure of the whole loop.
        let lua = sandboxed();
        lua.load(
            r#"
            goal = {}
            -- What `goal.holds` answers. `true` by default, because the
            -- ordinary reason a plan comes back empty is that the goal is
            -- already met; a test that wants the other two answers sets this
            -- to `false` (the planner contradicting the world) or `nil` (a
            -- goal possession cannot settle).
            __holds = true
            __holds_calls = 0
            goal.holds = function(_g, _opts)
                __holds_calls = __holds_calls + 1
                return __holds
            end
            goal.plan = function(_g, _opts) return { steps = {} } end
            goal.run = function(_plan) return { done = true } end
        "#,
        )
        .exec()
        .expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             repeat sup:step() until sup:finished()
             __state = sup.state",
        )
        .exec()
        .expect("the loop must not fail just because nothing is recording");
        assert_eq!(lua.globals().get::<String>("__state").unwrap(), "done");
    }

    #[test]
    fn a_genuine_keyframe_failure_is_surfaced_through_print_err_not_swallowed() {
        // `record.keyframe()` answers "no recording"/"nothing placed yet"
        // with `false`, never an error -- so anything that DOES raise here is
        // a real failure (the game unreachable, a bug in the glue), and
        // `pcall` alone would make that failure look byte-for-byte identical
        // to the ordinary "nothing to do" case: silence, zero keyframes,
        // nothing saying why. The loop must still finish (a keyframe is a
        // nicety, not core control flow), but it must not go quiet about it.
        let lua = sandboxed();
        lua.load(
            r#"
            goal = {}
            -- What `goal.holds` answers. `true` by default, because the
            -- ordinary reason a plan comes back empty is that the goal is
            -- already met; a test that wants the other two answers sets this
            -- to `false` (the planner contradicting the world) or `nil` (a
            -- goal possession cannot settle).
            __holds = true
            __holds_calls = 0
            goal.holds = function(_g, _opts)
                __holds_calls = __holds_calls + 1
                return __holds
            end
            goal.plan = function(_g, _opts) return { steps = {} } end
            goal.run = function(_plan) return { done = true } end
            record = {}
            record.keyframe = function() error("rcon: connection reset") end
            __print_err_calls = {}
            print_err = function(...)
                local args = {...}
                table.insert(__print_err_calls, table.concat(args, " "))
            end
        "#,
        )
        .exec()
        .expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        lua.load(
            "local sup = supervisor.new(supervisor.list {'a'}, {})
             repeat sup:step() until sup:finished()
             __state = sup.state",
        )
        .exec()
        .expect("a keyframe failure must not kill the run");
        assert_eq!(lua.globals().get::<String>("__state").unwrap(), "done");

        let calls: Vec<String> = lua
            .load("return __print_err_calls")
            .eval::<mlua::Table>()
            .unwrap()
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()
            .unwrap();
        assert_eq!(calls.len(), 1, "exactly one milestone was closed");
        assert!(
            calls[0].contains("rcon: connection reset"),
            "the original error must survive into what gets reported, got: {calls:?}"
        );
    }

    // ---- Layer 3: what `t` exposes for a driver's `record.*` calls -------

    /// Drives one milestone whose plan is empty, forwarding `t.reason` the way
    /// `scripts/research_run.lua` does, and hands back what was recorded.
    ///
    /// `Sup:step()` itself never calls `record.*` (see the module comment on
    /// `supervisor.lua`), so playing the driver's part is the only way to see
    /// the reason a run's log would actually carry.
    ///
    /// `None` when nothing was recorded as satisfied at all, which is a real
    /// answer and not a missing one: a milestone that closed some other way
    /// records no `milestone_satisfied` line, and a helper that returned a
    /// string for that case would have to invent one.
    fn reason_for_an_empty_plan(holds: &str) -> Option<String> {
        let lua = harness("{0}", "{}");
        lua.load(
            r#"
            __holds = HOLDS
            __satisfied = {}
            record.milestone_satisfied = function(index, iterations, reason)
                table.insert(__satisfied, { index = index, iterations = iterations, reason = reason })
            end
            local sup = supervisor.new(supervisor.list {"a"}, {})
            repeat
                local t = sup:step()
                if t.action == "satisfied" then
                    record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
                end
            until sup:finished()
            "#
            .replace("HOLDS", holds),
        )
        .exec()
        .expect("driver runs");
        lua.load("return __satisfied[1] and __satisfied[1].reason")
            .eval()
            .expect("the driver's own record calls run")
    }

    /// "Already done" and "the planner produced nothing and cannot say why"
    /// must not collapse into the same recorded line -- that is the whole of
    /// `SatisfiedReason`. The supervisor used to report `plan_empty` for both
    /// because it could not tell them apart; `goal.holds` is what tells them
    /// apart.
    ///
    /// Only one of the three answers is a satisfaction now. `nil` used to be
    /// recorded as `plan_empty` -- a *satisfied* line, carrying the one fact
    /// actually observed -- and that was the defect: `plan_empty` is a
    /// `SatisfiedReason`, so a goal nothing could answer closed the milestone
    /// as done. See
    /// `an_unanswerable_goal_with_an_empty_plan_is_not_a_satisfaction`.
    #[test]
    fn a_milestone_satisfied_by_an_empty_plan_records_which_it_was() {
        assert_eq!(
            reason_for_an_empty_plan("true").as_deref(),
            Some("already_satisfied"),
            "the goal was checked and holds; saying only `plan_empty` here \
             would throw away the fact the check established"
        );
        assert_eq!(
            reason_for_an_empty_plan("nil"),
            None,
            "a goal possession cannot settle was never satisfied by anything, \
             so no satisfaction may be recorded for it"
        );
    }

    /// The failure this whole check exists for: a plan with no steps for a
    /// goal that does not hold. The supervisor must refuse to call that
    /// satisfied -- a run reporting `done` for work it never did is the worst
    /// thing this loop can do.
    ///
    /// It raises rather than halting, because it is a contradiction between
    /// the planner and the world rather than a condition of the world, and
    /// there is nothing a retry could change. `research_run.lua` catches it,
    /// records the milestone as `plan_error` with this text, and closes the
    /// recording -- the same path a `goal.plan` construction error takes.
    #[test]
    fn an_empty_plan_for_an_unheld_goal_is_refused_rather_than_reported_satisfied() {
        let lua = harness("{0}", "{}");
        let err = lua
            .load(
                r#"
                __holds = false
                local sup = supervisor.new(supervisor.list {"a"}, {})
                repeat sup:step() until sup:finished()
                "#,
            )
            .exec()
            .expect_err("the supervisor must not report this satisfied");
        let text = err.to_string();
        assert!(
            text.contains("does not hold"),
            "the refusal must say what is wrong, got {text}"
        );
        assert!(
            !text.contains("stuck"),
            "and must not blame the world for a planner defect, got {text}"
        );
    }

    /// The check is asked once per empty plan, not once per goal kind or once
    /// per run: a milestone that plans real work must not pay for it.
    #[test]
    fn a_milestone_with_work_to_do_is_never_asked_whether_it_already_holds() {
        let lua = harness("{3, 0}", "{}");
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            repeat sup:step() until sup:finished()
            "#,
        )
        .exec()
        .expect("driver runs");
        assert_eq!(
            lua.globals().get::<i64>("__holds_calls").unwrap(),
            1,
            "only the empty re-plan asks"
        );
    }

    #[test]
    fn a_satisfied_transition_still_carries_a_plan_table_even_though_it_is_empty() {
        // `record.plan_created` should still be reachable for the zero-step
        // case -- "the planner ran and returned nothing" is itself worth
        // recording, not only the non-empty case.
        let lua = harness("{0}", "{}");
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            local t = sup:step() -- acquired
            t = sup:step() -- satisfied: the planner returned nothing
            __action = t.action
            __plan_type = type(t.plan)
            __plan_len = #t.plan
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "satisfied");
        assert_eq!(g.get::<String>("__plan_type").unwrap(), "table");
        assert_eq!(g.get::<i64>("__plan_len").unwrap(), 0);
    }

    #[test]
    fn a_planned_transition_exposes_steps_shaped_for_record_plan_created() {
        let lua = harness("{2, 0}", "{}");
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            local t = sup:step() -- acquired
            t = sup:step() -- planned
            __action = t.action
            __plan = t.plan
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "planned");
        let plan: mlua::Table = g.get("__plan").unwrap();
        assert_eq!(plan.raw_len(), 2, "two scheduled steps");
        let first: mlua::Table = plan.get(1).unwrap();
        assert_eq!(first.get::<u32>("id").unwrap(), 1);
        assert_eq!(first.get::<u32>("bot").unwrap(), 1);
        assert_eq!(first.get::<String>("action").unwrap(), "step 1");
        assert_eq!(first.get::<u64>("planned_start").unwrap(), 0);
        assert_eq!(first.get::<u64>("planned_duration").unwrap(), 10);
        let first_deps: mlua::Table = first.get("deps").unwrap();
        assert_eq!(first_deps.raw_len(), 0, "the first step waits on nothing");

        let second: mlua::Table = plan.get(2).unwrap();
        let second_deps: mlua::Table = second.get("deps").unwrap();
        assert_eq!(second_deps.raw_len(), 1);
        assert_eq!(
            second_deps.get::<u32>(1).unwrap(),
            1,
            "step 2 waits on step 1"
        );
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

    // ---- Layer 4: a planner verdict is not a fault ------------------------

    /// **Run 31, and the whole point of this change.**
    ///
    /// `workspace/runs/run-1788372605-35170/` satisfied six milestones in
    /// twelve minutes, asked for `researched("automation")` in a world with no
    /// electric supply, and the planner refused -- correctly, and with the
    /// most informative sentence it has ever produced. The raise then went
    /// straight past this loop: no `_close`, so no history entry, so
    /// `sup:report()` printed milestones 1-6 and **no `milestone 7` line at
    /// all**. The run that produced the best result so far is the one whose
    /// record cannot say what happened to it.
    ///
    /// A verdict about the world closes the milestone instead. The run reaches
    /// a terminal state on its own terms, `record.finish` is handed `stuck`
    /// rather than `crashed`, and the reason is on the line.
    #[test]
    fn a_refusal_closes_the_milestone_instead_of_killing_the_run() {
        let lua = harness("{'refuse'}", "{}");
        let (state, plans, runs) = drive(&lua, "{}");
        assert_eq!(
            state, "stuck",
            "a goal this world cannot reach halts the loop; it does not raise \
             out of it"
        );
        assert_eq!(
            plans, 1,
            "re-planning a refusal asks the same question of the same world \
             and gets the same answer; it must not burn the iteration cap \
             before reporting"
        );
        assert_eq!(runs, 0, "nothing was planned, so nothing can be run");

        let report: String = lua.globals().get("__report").expect("__report");
        assert!(
            report.contains("milestone 1: stuck"),
            "the milestone must appear in the summary at all -- its absence is \
             the defect this closes: {report}"
        );
        assert!(
            report.contains(
                "refused: automation needs a lab with 60 kW of electric supply, \
                 and the plan can show only 0 kW"
            ),
            "and the summary must carry the planner's own reason: {report}"
        );
        assert!(
            !report.contains("last error"),
            "nothing failed: no action was dispatched, so calling the refusal \
             an error would invent one: {report}"
        );
        assert_eq!(
            lua.globals().get::<i64>("__keyframe_calls").unwrap(),
            1,
            "a closed milestone is a closed milestone: it gets its keyframe"
        );
    }

    /// What the driver sees, which is what reaches the run record.
    ///
    /// `Sup:step()` never calls `record.*` itself, so the transition is the
    /// only channel: `scripts/research_run.lua` reads `t.refusal` and hands
    /// its message to `record.milestone_stuck`. The code rides along because a
    /// reader of the record should not have to match on a sentence to know
    /// which refusal this was.
    #[test]
    fn a_refusal_reaches_the_driver_as_a_halt_carrying_the_planners_own_code() {
        let lua = harness("{'refuse'}", "{}");
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            local seen
            repeat
                local t = sup:step()
                if t.action == "halted" then seen = t end
            until sup:finished()
            __action, __state = seen.action, seen.state
            __code = seen.refusal and seen.refusal.code
            __message = seen.refusal and seen.refusal.message
            __steps, __iteration = seen.steps, seen.iteration
            __first_error = sup.first_error
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "halted");
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "planner::research_needs_power",
            "the planner's own code is what makes this recognisable without \
             reading the sentence"
        );
        assert!(
            g.get::<String>("__message")
                .unwrap()
                .contains("60 kW of electric supply"),
            "and the sentence is what makes it readable"
        );
        assert_eq!(g.get::<i64>("__steps").unwrap(), 0);
        assert_eq!(
            g.get::<i64>("__iteration").unwrap(),
            0,
            "no plan was ever produced, so no iteration was spent"
        );
        assert_eq!(
            g.get::<Option<String>>("__first_error").unwrap(),
            None,
            "`first_error` means the first failed *action*, and no action ran; \
             a refusal must not put a sentence there and make the run look \
             like something was dispatched and refused"
        );
    }

    /// The negative control for the classification, from this side.
    ///
    /// The stub's classifier answers `nil` for anything that was not the
    /// scripted refusal -- exactly as the real one answers `nil` for a planner
    /// fault -- and a raise it does not vouch for must still end the run. This
    /// is what keeps the change from being a catch-all: if this test could not
    /// go red, the loop would be swallowing genuine bugs and reporting them as
    /// conditions of the world.
    #[test]
    fn a_raise_the_classifier_does_not_vouch_for_still_ends_the_run() {
        let lua = harness("{'raise'}", "{}");
        let err = lua
            .load(
                "local sup = supervisor.new(supervisor.list {'a'}, {}) \
                   repeat sup:step() until sup:finished()",
            )
            .exec()
            .expect_err("a fault must propagate even with a classifier present");
        assert!(
            format!("{err}").contains("unknown item"),
            "the original error must survive being caught and re-raised, got: {err}"
        );
        assert_eq!(
            lua.globals().get::<i64>("__plan_calls").unwrap(),
            1,
            "a fault must not be retried"
        );
    }

    /// And the control for the classifier being absent altogether.
    ///
    /// A `goal` table with no `goal.refusal` cannot say a raise was a verdict,
    /// and the safe reading of "cannot classify" is *fault*: loud, and out of
    /// the loop. The opposite default would let an old or stubbed binding turn
    /// every planner bug into a quietly recorded world condition. (This is the
    /// same rule `goal.plan`'s placement pre-check follows: an absent checker
    /// must never look like a green answer.)
    #[test]
    fn a_goal_table_with_no_classifier_treats_a_raise_as_a_fault() {
        let lua = sandboxed();
        lua.load(
            r#"
            goal = {}
            goal.holds = function(_g, _opts) return true end
            goal.plan = function(_g, _opts) error("goal: something went wrong", 0) end
            goal.run = function(_plan) return { done = true } end
        "#,
        )
        .exec()
        .expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        let err = lua
            .load(
                "local sup = supervisor.new(supervisor.list {'a'}, {}) \
                   repeat sup:step() until sup:finished()",
            )
            .exec()
            .expect_err("with nothing to classify the raise, it must propagate");
        assert!(
            format!("{err}").contains("something went wrong"),
            "got: {err}"
        );
    }

    /// A refusal that arrives *after* real work must not overwrite the reason
    /// that work gave.
    ///
    /// Milestone 1 plans 5 steps, runs them, one action fails, and the re-plan
    /// is refused. Both facts belong on the record and they are different
    /// facts: `first_error` is the first failed action's own text, and the
    /// refusal is why the milestone closed. Collapsing them would lose
    /// whichever was written second.
    #[test]
    fn a_refusal_after_a_failed_run_keeps_both_reasons_apart() {
        let lua = harness(
            "{5, 'refuse'}",
            "{{failed=1, first_error='no entity to mine'}}",
        );
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            repeat sup:step() until sup:finished()
            __state = sup.state
            __first_error = sup.first_error
            local h = sup:history()[1]
            __outcome = h.outcome
            __refusal = h.refusal and h.refusal.message
            __report = sup:report()
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(g.get::<String>("__outcome").unwrap(), "stuck");
        assert_eq!(
            g.get::<String>("__first_error").unwrap(),
            "no entity to mine",
            "the action that failed keeps its own text"
        );
        assert!(
            g.get::<String>("__refusal")
                .unwrap()
                .contains("60 kW of electric supply"),
            "and the refusal that closed the milestone keeps its own"
        );
        let report: String = g.get("__report").unwrap();
        assert!(
            report.contains("refused: automation needs a lab"),
            "the halt's own reason is what the summary leads with: {report}"
        );
    }

    // ---- Layer 5: a goal nothing can answer is not a goal that is done ----

    /// **D0 of `docs/superpowers/specs/2026-09-03-starter-factory-design.md`.**
    ///
    /// `goal.holds` has three answers and only one of them is a satisfaction.
    /// `nil` -- "no method here can answer this goal at all" -- used to close
    /// the milestone `satisfied` with reason `plan_empty`, which is a
    /// `SatisfiedReason`: the absence of a verdict recorded as success. That
    /// is the failure this project spent a day removing everywhere else, and
    /// it was still wired into the one branch of this loop that had no other
    /// signal.
    ///
    /// It was harmless only because every goal a script could build was a
    /// `have` or a `researched`, both of which `holds` answers. The moment a
    /// goal kind it answers `nil` for (`Produced`, `Producing`) reaches a
    /// method that emits no steps -- for any reason at all -- the milestone
    /// reports a factory built on nothing.
    #[test]
    fn an_unanswerable_goal_with_an_empty_plan_is_not_a_satisfaction() {
        let lua = harness("{0}", "{}");
        lua.load("__holds = nil").exec().expect("stub set");
        let (state, plans, runs) = drive(&lua, "{}");
        assert_eq!(
            state, "stuck",
            "nothing established that this goal is met, so the loop must not \
             say it is"
        );
        assert_eq!(
            plans, 1,
            "re-planning asks the same question of the same world -- nothing \
             ran, so nothing changed -- and must not burn the cap first"
        );
        assert_eq!(runs, 0, "nothing was planned, so nothing can be run");

        let report: String = lua.globals().get("__report").expect("__report");
        assert!(
            report.contains("milestone 1: stuck"),
            "the milestone must appear in the summary as what it was: {report}"
        );
        assert!(
            report.contains("cannot say whether the goal holds"),
            "and must say why it stopped, in the loop's own words: {report}"
        );
        assert!(
            !report.contains("last error"),
            "no action was dispatched, so there is no error to name: {report}"
        );
    }

    /// What the driver sees. `scripts/research_run.lua` reads `t.refusal`
    /// and hands its message to `record.milestone_stuck`, so an unanswerable
    /// halt that carried its reason anywhere else would reach the record with
    /// no reason at all.
    ///
    /// The `code` is what keeps this apart from a planner refusal for a reader
    /// of that record: a planner refusal carries the planner's own miette code
    /// and this carries `supervisor::unanswerable`, which no `PlannerError`
    /// can produce.
    #[test]
    fn an_unanswerable_halt_carries_its_own_code_not_the_planners() {
        let lua = harness("{0}", "{}");
        lua.load(
            r#"
            __holds = nil
            local sup = supervisor.new(supervisor.list {"a"}, {})
            local seen
            repeat
                local t = sup:step()
                if t.action == "halted" then seen = t end
            until sup:finished()
            __action, __state = seen.action, seen.state
            __reason = seen.reason
            __code = seen.refusal and seen.refusal.code
            __message = seen.refusal and seen.refusal.message
            __history_refusal = sup:history()[1].refusal
                and sup:history()[1].refusal.code
            __outcome = sup:history()[1].outcome
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "halted");
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(g.get::<String>("__outcome").unwrap(), "stuck");
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "supervisor::unanswerable",
            "this halt is the supervisor's own verdict, not the planner's, and \
             the code is the only thing that says so without reading the \
             sentence"
        );
        assert_eq!(
            g.get::<String>("__history_refusal").unwrap(),
            "supervisor::unanswerable",
            "and it must be on the history entry too, or `sup:report()` and \
             any caller reading the history lose it"
        );
        assert!(
            g.get::<String>("__message")
                .unwrap()
                .contains("milestone 1"),
            "the sentence names the milestone it is about"
        );
        assert_eq!(
            g.get::<Option<String>>("__reason").unwrap(),
            None,
            "`reason` is a `SatisfiedReason` and nothing here was satisfied; \
             carrying one would put the old `plan_empty` line back on the \
             record through a different field"
        );
    }

    /// The negative control from the other side: `holds` answering `true`
    /// still closes the milestone as satisfied and the loop still moves on.
    ///
    /// Without this, the change could be "always halt on an empty plan", which
    /// would pass every assertion above and break every run that finishes.
    #[test]
    fn an_empty_plan_for_a_goal_that_does_hold_is_still_satisfied() {
        let lua = harness("{0}", "{}");
        let (state, _, runs) = drive(&lua, "{}");
        assert_eq!(
            state, "done",
            "the default stub answers `true`; a met goal closes and the source \
             runs out"
        );
        assert_eq!(runs, 0);
    }

    /// And the control that this costs a milestone with work to do nothing.
    ///
    /// `holds` is `nil` for the whole run, but the first plan has three steps,
    /// so the loop plans, runs, and only the empty re-plan reaches the new
    /// branch. A change that consulted `holds` earlier -- or halted on the
    /// answer rather than on the answer *plus* an empty plan -- would stop this
    /// run before it did anything.
    #[test]
    fn an_unanswerable_goal_still_gets_its_work_done_first() {
        let lua = harness("{3, 0}", "{}");
        lua.load("__holds = nil").exec().expect("stub set");
        let (state, plans, runs) = drive(&lua, "{}");
        assert_eq!(state, "stuck");
        assert_eq!(plans, 2, "the first plan had work in it and was planned");
        assert_eq!(runs, 1, "and it was run");
        assert_eq!(
            lua.globals().get::<i64>("__holds_calls").unwrap(),
            1,
            "only the empty re-plan asks"
        );
    }

    // ---- Layer 6: the witness -- a machine that stands is not one that works

    /// **The stage-1 cell, with the game's own numbers, and a decoy.**
    ///
    /// A burner mining drill at `(-35, 35)` facing north reports
    /// `drop_position = (-35.35, 33.7)`; the stone furnace two tiles north at
    /// `(-35, 33)` has the collision box the prototype gives it,
    /// `±0.69921875`, so its near edge sits at `33.69921875`. **The drop point
    /// is outside that box by 0.00078125 of a tile**, one part in 1280 — so a
    /// witness that asked whether the point is inside the box would answer
    /// "not fed" for the one layout stage 1 exists to build. See
    /// `PlanState::delivers_into`, which decides it the same way.
    ///
    /// `__decoy` is the other half of the trap and is not incidental: stage
    /// 1's bill hand-smelts in stone furnaces of its own, and one of them
    /// holds two plates a *bot* put there. Nothing drops into it, so a witness
    /// that watched every furnace in the area would be counting a bot's
    /// leftovers as machine-made production.
    const CELL_FIXTURE: &str = r#"
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
    "#;

    /// The real supervisor over a stub game.
    ///
    /// `rcon.game_tick` advances one tick per call, which is not an arbitrary
    /// stub: Factorio processes rcon once per tick, so a round trip cannot come
    /// back sooner than the next one. That is the only clock the wait has —
    /// the sandbox has no sleep — and it is why the loop is self-paced rather
    /// than spinning.
    ///
    /// The watched furnace produces nothing by default. A cell that stands and
    /// makes nothing is the failure this whole layer exists for, so it is what
    /// the fixture does unless a test says otherwise.
    fn witness_harness(extra: &str) -> Lua {
        let lua = sandboxed();
        let stub = format!(
            r#"
            {CELL_FIXTURE}
            __tick = 1000
            __tick_step = 1
            __tick_calls = 0
            __find_calls = 0
            __produce_from_tick = nil
            __produce_count = 0
            __world = {{ __drill, __furnace, __decoy }}
            rcon = {{}}
            rcon.game_tick = function()
                __tick_calls = __tick_calls + 1
                __tick = __tick + __tick_step
                return __tick
            end
            rcon.find_entities_in_radius = function(_centre, _radius, name)
                __find_calls = __find_calls + 1
                if __produce_from_tick ~= nil and __tick >= __produce_from_tick then
                    __furnace.output_inventory = {{
                        {{ name = "iron-plate", quality = "normal", count = __produce_count }}
                    }}
                end
                local out = {{}}
                for _, e in ipairs(__world) do
                    if name == nil or e.name == name then out[#out + 1] = e end
                end
                return out
            end
            goal = {{}}
            __plan_calls, __run_calls, __holds_calls = 0, 0, 0
            goal.plan = function(_g, _o)
                __plan_calls = __plan_calls + 1
                return {{ steps = {{}} }}
            end
            goal.run = function(_p) __run_calls = __run_calls + 1; return {{ done = true }} end
            goal.holds = function(_g, _o) __holds_calls = __holds_calls + 1; return true end
            __keyframe_calls = 0
            record = {{}}
            record.keyframe = function() __keyframe_calls = __keyframe_calls + 1 end
        "#
        );
        lua.load(&stub).exec().expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        lua.load(extra).exec().expect("test fixture installs");
        lua
    }

    /// The stage-1 witness spec, as `factory_stage1.lua` writes it.
    const WITNESS_SPEC: &str = r#"supervisor.witness {
        item = "iron-plate", from = "burner-mining-drill", into = "stone-furnace",
        near = { x = 0, y = 0 }, radius = 250,
        at_least = 1, within_ticks = 600, probe_ticks = 60,
    }"#;

    /// Drives one witness milestone to its terminal state and hands back the
    /// last transition it produced, as a driver would see it.
    fn drive_witness(lua: &Lua) {
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list { __spec }, {})
            local seen
            local guard = 0
            repeat
                local t = sup:step()
                if t.action ~= "acquired" and t.action ~= "finished" then seen = t end
                guard = guard + 1
                if guard > 20 then error("a witness milestone did not terminate") end
            until sup:finished()
            __action, __state = seen.action, seen.state
            __code = seen.refusal and seen.refusal.code
            __message = seen.refusal and seen.refusal.message
            __reason = seen.reason
            __w = seen.witness
            __sup_state = sup.state
            __report = sup:report()
            "#,
        )
        .exec()
        .expect("the witness milestone runs to a verdict");
    }

    /// **The property the whole thing rests on: it dispatches nothing.**
    ///
    /// If no bot acted, an item that appeared can only have been made by a
    /// machine — that is the entire argument for why a witness is evidence
    /// where `goal.holds` is only a claim about ground. A witness that planned,
    /// or ran, or even asked the planner a question would forfeit it.
    #[test]
    fn a_witness_dispatches_nothing_at_all() {
        let lua = witness_harness(&format!("__spec = {WITNESS_SPEC}"));
        drive_witness(&lua);
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__plan_calls").unwrap(),
            0,
            "a witness must not cost the planner an expansion, or a ladder with \
             one in it would plan differently from a ladder without"
        );
        assert_eq!(g.get::<i64>("__run_calls").unwrap(), 0, "and nothing ran");
        assert_eq!(
            g.get::<i64>("__holds_calls").unwrap(),
            0,
            "nor is `goal.holds` consulted: the witness asks the game, not the \
             planner, and the planner's answer is the one that cannot see this"
        );
    }

    /// A cell that produces is witnessed, and the wait stops as soon as it has
    /// proved itself rather than running the clock out.
    #[test]
    fn a_cell_whose_output_rises_is_witnessed_and_the_wait_stops_early() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             __produce_from_tick = 1120
             __produce_count = 3"
        ));
        drive_witness(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "satisfied");
        assert_eq!(g.get::<String>("__sup_state").unwrap(), "done");
        let w: mlua::Table = g.get("__w").expect("the observation rides along");
        assert_eq!(w.get::<i64>("before").unwrap(), 0);
        assert_eq!(w.get::<i64>("after").unwrap(), 3);
        assert_eq!(w.get::<i64>("gained").unwrap(), 3);
        assert_eq!(
            w.get::<i64>("watched").unwrap(),
            1,
            "one furnace is fed; the hand-smelt decoy two tiles west is not"
        );
        let elapsed = w.get::<i64>("elapsed_ticks").unwrap();
        assert!(
            (120..600).contains(&elapsed),
            "the wait must end when the cell has proved itself, not when the \
             deadline runs out; elapsed was {elapsed}"
        );
    }

    /// **The failure the witness exists for.** The cell stands — the planner
    /// would say the goal holds — and it makes nothing.
    #[test]
    fn a_cell_that_stands_and_produces_nothing_halts_with_its_own_code() {
        let lua = witness_harness(&format!("__spec = {WITNESS_SPEC}"));
        drive_witness(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "halted");
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "supervisor::not_producing"
        );
        let message = g.get::<String>("__message").unwrap();
        assert!(
            message.contains("stands and produces nothing"),
            "the verdict has to say which of the two it is: {message}"
        );
        assert!(
            message.contains("no bot acted"),
            "and why that is evidence rather than a coincidence: {message}"
        );
        let w: mlua::Table = g.get("__w").unwrap();
        assert!(
            w.get::<i64>("elapsed_ticks").unwrap() >= 600,
            "a dead cell may only be called dead after the whole window"
        );
        let report: String = g.get("__report").unwrap();
        assert!(
            report.contains("milestone 1: stuck") && report.contains("refused:"),
            "and it closes the milestone in the summary like any other verdict: \
             {report}"
        );
    }

    /// **A dead cell and an unbuilt one have different fixes, so they get
    /// different verdicts.** Nothing is fed here — the drill is gone — and
    /// reporting that as "produces nothing" would send the next reader to
    /// check fuel levels for a cell that was never built.
    #[test]
    fn a_cell_that_was_never_built_reads_differently_from_a_dead_one() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             __world = {{ __furnace, __decoy }}"
        ));
        drive_witness(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(g.get::<String>("__code").unwrap(), "supervisor::no_cell");
        let message = g.get::<String>("__message").unwrap();
        assert!(
            message.contains("Nothing was built to watch"),
            "it must name the absence, not the silence: {message}"
        );
        assert!(
            message.contains("says nothing about whether anything produces"),
            "and must refuse the verdict it did not earn: {message}"
        );
        assert!(
            !message.contains("produces nothing"),
            "which is the one sentence it must not be confusable with: {message}"
        );
    }

    /// The decoy, from the other end. Two furnaces stand and one of them holds
    /// plates a bot smelted; only the fed one is watched, so the witness still
    /// says the cell produces nothing.
    #[test]
    fn a_furnace_nothing_drops_into_is_not_watched_however_full_it_is() {
        let lua = witness_harness(&format!("__spec = {WITNESS_SPEC}"));
        drive_witness(&lua);
        let g = lua.globals();
        let w: mlua::Table = g.get("__w").unwrap();
        assert_eq!(
            w.get::<i64>("watched").unwrap(),
            1,
            "two stone furnaces stand, one is fed"
        );
        assert_eq!(
            w.get::<i64>("before").unwrap(),
            0,
            "the decoy's two hand-smelted plates must not be in the baseline — \
             a witness that summed every furnace would call a bot's leftovers \
             production the moment one more appeared anywhere"
        );
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "supervisor::not_producing"
        );
    }

    /// **The 1/1280.** The drop point of the vanilla starter pair is outside
    /// the furnace's collision box by 0.00078125 of a tile, so box containment
    /// would reject the one layout stage 1 builds. A drop point resolves to the
    /// tile it lands in; the furnace covers both of its tiles.
    #[test]
    fn the_drop_point_that_misses_the_box_by_one_part_in_1280_still_feeds_it() {
        let lua = witness_harness("");
        lua.load(
            r#"
            __fed = supervisor.delivers_into(__drill, __furnace)
            -- the measurement, so nobody later "fixes" this into box containment
            __miss = __drill.drop_position.y - __furnace.bounding_box.right_bottom.y
            __decoy_fed = supervisor.delivers_into(__drill, __decoy)
            __backwards = supervisor.delivers_into(__furnace, __drill)
            "#,
        )
        .exec()
        .expect("the pure geometry runs without a game");
        let g = lua.globals();
        assert!(
            g.get::<bool>("__fed").unwrap(),
            "the starter pair feeds; anything that says otherwise has rejected \
             the whole of stage 1"
        );
        assert!(
            (g.get::<f64>("__miss").unwrap() - 0.00078125).abs() < 1e-12,
            "and it does so by 1/1280 of a tile, which is the number that makes \
             tile containment load-bearing rather than a convenience"
        );
        assert!(
            !g.get::<bool>("__decoy_fed").unwrap(),
            "the furnace two tiles west of the fed one is not fed"
        );
        assert!(
            !g.get::<bool>("__backwards").unwrap(),
            "and a furnace reports no drop position at all, so it feeds nothing \
             — the direction of the link is not symmetric"
        );
    }

    /// A witness may not call a cell dead on a wait that did not happen.
    ///
    /// The game's clock stands still — paused, saving, gone — so the window
    /// never elapses. The poll cap ends the loop, and the verdict is that
    /// nothing was established, which is a different thing from "nothing was
    /// produced".
    #[test]
    fn a_clock_that_does_not_advance_is_inconclusive_and_not_a_dead_cell() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             __tick_step = 0"
        ));
        drive_witness(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "supervisor::witness_inconclusive",
            "a wait that did not happen proves nothing in either direction"
        );
        let message = g.get::<String>("__message").unwrap();
        assert!(
            message.contains("clock is not advancing"),
            "and it must name the cause it can actually see: {message}"
        );
        assert!(
            !message.contains("produces nothing"),
            "never the verdict it did not earn: {message}"
        );
    }

    /// The other half of the same rule: a game that will not report a tick at
    /// all cannot be waited in either.
    #[test]
    fn a_game_that_reports_no_tick_is_inconclusive_rather_than_assumed() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             rcon.game_tick = function() return nil end"
        ));
        drive_witness(&lua);
        assert_eq!(
            lua.globals().get::<String>("__code").unwrap(),
            "supervisor::witness_inconclusive"
        );
    }

    /// A witness closes its milestone like any other, so the run's record and
    /// summary carry it. `_close` is the only place a keyframe is written.
    #[test]
    fn a_witness_milestone_is_closed_and_gets_its_keyframe() {
        let lua = witness_harness(&format!("__spec = {WITNESS_SPEC}"));
        drive_witness(&lua);
        assert_eq!(
            lua.globals().get::<i64>("__keyframe_calls").unwrap(),
            1,
            "one milestone was closed, so exactly one keyframe"
        );
    }

    /// A satisfied witness must still be recordable by a driver that only
    /// knows the two `SatisfiedReason` strings `record.milestone_satisfied`
    /// accepts. This pins the one it uses, because a third string would be
    /// refused by name at the Rust boundary and take the run down with it.
    #[test]
    fn a_satisfied_witness_carries_a_reason_the_record_will_accept() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             __produce_from_tick = 1120
             __produce_count = 1"
        ));
        drive_witness(&lua);
        assert_eq!(
            lua.globals().get::<String>("__reason").unwrap(),
            "already_satisfied",
            "`record.milestone_satisfied` accepts only `already_satisfied` and \
             `plan_empty`; anything else raises inside the recorder"
        );
    }

    /// **`within_ticks` has no default and the constructor says so.** It is
    /// the number that decides what a failure means; a library that guessed it
    /// would be handing back a verdict nobody derived.
    #[test]
    fn a_witness_with_no_deadline_is_refused_at_construction() {
        let lua = witness_harness("");
        let err = lua
            .load(
                r#"supervisor.witness { item = "iron-plate", from = "a", into = "b",
                                        near = { x = 0, y = 0 } }"#,
            )
            .exec()
            .expect_err("a witness with no deadline must raise");
        assert!(
            format!("{err}").contains("`within_ticks` is required"),
            "got: {err}"
        );
    }

    /// And the rest of the shape, refused where the mistake is cheap rather
    /// than twenty minutes into a run.
    #[test]
    fn a_witness_missing_the_two_ends_of_its_link_is_refused_at_construction() {
        let lua = witness_harness("");
        for (spec, want) in [
            (
                r#"supervisor.witness { from = "a", into = "b", near = {x=0,y=0}, within_ticks = 1 }"#,
                "`item` must be a string",
            ),
            (
                r#"supervisor.witness { item = "i", into = "b", near = {x=0,y=0}, within_ticks = 1 }"#,
                "`from` must be a string",
            ),
            (
                r#"supervisor.witness { item = "i", from = "a", near = {x=0,y=0}, within_ticks = 1 }"#,
                "`into` must be a string",
            ),
            (
                r#"supervisor.witness { item = "i", from = "a", into = "b", within_ticks = 1 }"#,
                "`near` must be a position",
            ),
        ] {
            let err = lua
                .load(spec)
                .exec()
                .expect_err("an incomplete witness must raise at construction");
            assert!(format!("{err}").contains(want), "wanted {want}, got: {err}");
        }
    }

    /// The counter, on the shapes the bridge actually produces.
    #[test]
    fn count_item_reads_output_inventories_and_survives_the_null_sentinel() {
        let lua = witness_harness("");
        lua.load(
            r#"
            __empty = supervisor.count_item({ { name = "stone-furnace" } }, "iron-plate")
            __mixed = supervisor.count_item({
                { output_inventory = { { name = "iron-plate", count = 4 },
                                       { name = "stone", count = 9 } } },
                { output_inventory = { { name = "iron-plate", count = 2 } } },
            }, "iron-plate")
            "#,
        )
        .exec()
        .expect("the pure counter runs");
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__empty").unwrap(),
            0,
            "an entity with no output inventory contributes nothing, and must \
             not raise: `Option::None` arrives as light userdata, which is \
             truthy, so `inv or {{}}` would hand `ipairs` a sentinel"
        );
        assert_eq!(
            g.get::<i64>("__mixed").unwrap(),
            6,
            "summed across machines, and only the item asked for"
        );
    }

    /// A witness whose game is not there is a **fault**, not a verdict: the
    /// script was built wrong, nothing about the world was established, and
    /// halting quietly would record a condition of the world for a defect in
    /// the run. Same rule `refusal_of` follows for an unclassifiable raise.
    #[test]
    fn a_witness_with_no_game_to_look_at_raises_rather_than_halting() {
        let lua = witness_harness(&format!("__spec = {WITNESS_SPEC}\nrcon = nil"));
        let err = lua
            .load(
                "local sup = supervisor.new(supervisor.list { __spec }, {}) \
                   repeat sup:step() until sup:finished()",
            )
            .exec()
            .expect_err("a witness with no rcon must propagate");
        assert!(
            format!("{err}").contains("no game to witness it in"),
            "got: {err}"
        );
    }

    /// The loop's other milestones are untouched by a witness beside them: a
    /// goal rung still plans and runs, and the witness rung still costs the
    /// planner nothing.
    #[test]
    fn a_witness_beside_a_goal_leaves_the_goals_own_milestone_alone() {
        let lua = witness_harness(&format!(
            "__spec = {WITNESS_SPEC}
             __produce_from_tick = 1120
             __produce_count = 1"
        ));
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list { "a", __spec }, {})
            local outcomes = {}
            repeat
                local t = sup:step()
                if t.action == "satisfied" or t.action == "halted" then
                    outcomes[#outcomes + 1] = t.milestone_index .. ":" .. t.action
                end
            until sup:finished()
            __outcomes = table.concat(outcomes, " ")
            __state = sup.state
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "done");
        assert_eq!(
            g.get::<String>("__outcomes").unwrap(),
            "1:satisfied 2:satisfied"
        );
        assert_eq!(
            g.get::<i64>("__plan_calls").unwrap(),
            1,
            "the goal rung planned once; the witness rung planned not at all"
        );
        assert_eq!(
            g.get::<i64>("__keyframe_calls").unwrap(),
            2,
            "two milestones closed, two keyframes"
        );
    }
}
