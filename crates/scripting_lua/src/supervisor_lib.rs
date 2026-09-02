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
    fn reason_for_an_empty_plan(holds: &str) -> String {
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
        lua.load("return __satisfied[1].reason")
            .eval()
            .expect("reason recorded")
    }

    /// "Already done" and "the planner produced nothing and cannot say why"
    /// must not collapse into the same recorded line -- that is the whole of
    /// `SatisfiedReason`. The supervisor used to report `plan_empty` for both
    /// because it could not tell them apart; `goal.holds` is what tells them
    /// apart, and each of its three answers has to reach the record as itself.
    #[test]
    fn a_milestone_satisfied_by_an_empty_plan_records_which_it_was() {
        assert_eq!(
            reason_for_an_empty_plan("true"),
            "already_satisfied",
            "the goal was checked and holds; saying only `plan_empty` here \
             would throw away the fact the check established"
        );
        assert_eq!(
            reason_for_an_empty_plan("nil"),
            "plan_empty",
            "a goal possession cannot settle leaves the one observed fact: \
             the plan came back with nothing in it"
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
}
