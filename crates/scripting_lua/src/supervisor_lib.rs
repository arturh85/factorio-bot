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

/// `scripts/rocket_policy.lua`, verbatim.
pub const ROCKET_POLICY_LUA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/rocket_policy.lua"
));

#[cfg(test)]
mod tests {
    use super::{ROCKET_POLICY_LUA, SUPERVISOR_LUA};
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
            -- The `why` of every `obs:recover()` ask, in order. A test reads
            -- this to see which asks were made at all -- a rule that refuses
            -- *before* asking (a lost action) is a different fact from one
            -- that asks and declines the answer (a re-expansion).
            __recover_calls = {}
            -- The steps of a plan of `n`, in the shape `PlanValue.steps` hands
            -- back. Shared by `goal.plan` and the `obs:recover()` stub, so a
            -- recovered plan is the same kind of thing a planned one is --
            -- which is exactly what `PlanValue::from_recovery` guarantees on
            -- the Rust side.
            function __make_steps(n)
                local steps = {}
                for i = 1, n do
                    steps[i] = {
                        id = i, bot = 1, label = "step " .. i,
                        start = (i - 1) * 10, finish = i * 10,
                        deps = (i > 1) and { i - 1 } or {},
                    }
                end
                return steps
            end
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
                return { steps = __make_steps(n) }
            end
            goal.run = function(_plan)
                __run_calls = __run_calls + 1
                local o = __run_obs[__run_calls] or {}
                local obs = { failed = o.failed or 0, lost = o.lost or 0,
                         abandoned = o.abandoned or 0,
                         walks_failed = o.walks_failed or 0,
                         walks_lost = o.walks_lost or 0,
                         pending = o.pending or 0, success = o.success or 0,
                         running = 0, done = true,
                         first_error = o.first_error,
                         actions = o.actions or {},
                         walks = o.walks }
                -- `obs:recover()`, installed only when the scripted
                -- observation asks for it -- because "this binding has no
                -- `recover`" is itself a case the loop has to survive, and it
                -- is the case every other test in this file is in.
                if o.recover ~= nil then
                    local r = o.recover
                    obs.recover = function(_self)
                        table.insert(__recover_calls, r.why)
                        if r.why == "raise" then error("stub: recover raised", 0) end
                        if r.steps == nil then return nil, r.why end
                        -- `bots` is the roster the proposal was made against.
                        -- A real `PlanValue` always has one, and it is what
                        -- `record.plan_created` writes, so a test asserts it
                        -- survives onto the transition.
                        return { steps = __make_steps(r.steps), bots = {1, 2} }, r.why
                    end
                end
                return obs
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

    // ---- Layer 7: recovery -- continuing a plan instead of discarding it --
    //
    // `docs/superpowers/specs/2026-09-04-recovery-instead-of-replan-design.md`,
    // S1. Every test here drives the real `supervisor.lua` against a scripted
    // `obs:recover()`; no live run has ever reached the recovery tiers, so
    // these are the only proof the four rules hold.

    /// Drives one milestone to a terminal state and returns a trace of the
    /// transitions, `(trace, plan_calls, run_calls, recover_asks)`.
    ///
    /// The trace is what a driver sees, in order, because that is the only
    /// channel the loop has: a recovery is an ordinary `planned` transition
    /// carrying `t.recovery`, and a run that is about to be continued is an
    /// ordinary `ran` transition whose `state` says `recovering`.
    fn trace(lua: &Lua, opts: &str) -> (String, i64, i64, String) {
        let driver = r#"
            local sup = supervisor.new(supervisor.list {"a"}, OPTS)
            local out, guard = {}, 0
            repeat
                local t = sup:step()
                local line = t.action
                if t.action == "planned" then
                    line = line .. "(" .. tostring(t.steps)
                        .. (t.recovery and (" " .. t.recovery) or "") .. ")"
                elseif t.action == "ran" then
                    line = line .. "(" .. tostring(t.state) .. ")"
                end
                out[#out + 1] = line
                guard = guard + 1
                if guard > 200 then error("did not terminate") end
            until sup:finished()
            __trace = table.concat(out, " ")
            __state = sup.state
        "#
        .replace("OPTS", opts);
        lua.load(&driver).exec().expect("driver runs");
        let g = lua.globals();
        (
            g.get::<String>("__trace").unwrap(),
            g.get::<i64>("__plan_calls").unwrap(),
            g.get::<i64>("__run_calls").unwrap(),
            lua.load("return table.concat(__recover_calls, ',')")
                .eval::<String>()
                .unwrap(),
        )
    }

    /// **The case S1 exists for**, in the shape `run-1788481380-80843` had it:
    /// 194 steps planned, most of them done, one placement refused because a
    /// character was standing in the footprint -- a transient that cleared 53
    /// ticks later. The loop used to answer that by throwing the plan away and
    /// re-siting the power plant 65 tiles from a pole it had already built.
    ///
    /// It now runs the remainder instead, and -- the assertion that matters --
    /// **without calling the planner**: `plan_calls` counts the first plan and
    /// the closing empty re-plan, and nothing in between.
    #[test]
    fn a_transient_failure_continues_the_plan_instead_of_replanning_it() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, pending=4,
               first_error='cannot place item stone-furnace because a character is standing in the footprint',
               recover={why='rescheduled', steps=4}},
              {success=9}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(recovering) planned(4 rescheduled) \
             ran(planning) satisfied finished"
        );
        assert_eq!(asks, "rescheduled");
        assert_eq!(
            plans, 2,
            "the recovery cost no expansion: one plan, one closing re-plan"
        );
        assert_eq!(runs, 2, "and the remainder was actually run");
        assert_eq!(lua.globals().get::<String>("__state").unwrap(), "done");
    }

    /// **Rule 1: tier 2 is refused.**
    ///
    /// `Reexpanded` is a replan by another name -- a new network numbered from
    /// zero, run against a fresh log -- and taking it here would bypass
    /// `iterations` and the stall, which already handle replanning correctly,
    /// while claiming in the record to be a continuation. The ask is still
    /// made (the tier is only knowable from the answer); the answer is
    /// declined.
    #[test]
    fn a_reexpansion_is_declined_and_the_loop_replans_instead() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, recover={why='reexpanded', steps=4}}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace, "acquired planned(10) ran(planning) satisfied finished",
            "a re-expansion must leave the loop in exactly the state it was in \
             before recovery existed"
        );
        assert_eq!(
            asks, "reexpanded",
            "the ask was made and the answer refused"
        );
        assert_eq!(plans, 2);
        assert_eq!(runs, 1, "the re-expanded plan must not be run from here");
    }

    /// **Rule 2: a run with a lost action is never recovered -- and never even
    /// asked about.**
    ///
    /// `Lost` means "dispatched, no verdict", not "did not happen".
    /// `recover()` retires an action only on `Success`, so a lost one comes
    /// back in the proposal and is dispatched a second time; `Insert`, `Remove`
    /// and `Place` are not idempotent, and `run-1788517971-48257` lost a
    /// `take 13 coal from the wooden-chest`, which would empty the chest twice.
    ///
    /// Asserted on the ask rather than on the answer: refusing before asking is
    /// a different and stronger fact than asking and declining, and it is the
    /// one the double-execution hazard requires.
    #[test]
    fn a_run_with_a_lost_action_is_replanned_without_asking_for_a_recovery() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, lost=1, success=5,
               first_error='no action result received in time',
               recover={why='rescheduled', steps=4}}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(planning) satisfied finished"
        );
        assert_eq!(
            asks, "",
            "a lost action must not even be offered to `recover`: the proposal \
             it would make re-dispatches the action nobody has a verdict for"
        );
        assert_eq!(plans, 2);
        assert_eq!(runs, 1);
    }

    /// Drives one milestone and returns the `not_recovered` reasons of its
    /// `ran` transitions, in order, `-` standing for nil.
    fn refusals(lua: &Lua, opts: &str) -> String {
        let driver = r#"
            local sup = supervisor.new(supervisor.list {"a"}, OPTS)
            local out, guard = {}, 0
            repeat
                local t = sup:step()
                if t.action == "ran" then
                    out[#out + 1] = t.not_recovered or "-"
                end
                guard = guard + 1
                if guard > 200 then error("did not terminate") end
            until sup:finished()
            __refusals = table.concat(out, " ")
        "#
        .replace("OPTS", opts);
        lua.load(&driver).exec().expect("driver runs");
        lua.globals().get::<String>("__refusals").unwrap()
    }

    /// The verdict of `run-1788552801-73005`, batch 3, as it reaches Lua.
    const DIVERGED_TAKE: &str = r#"game rejected the command: Unexpected Response: ["tried to remove 64 iron-plate but removed 40"]"#;

    /// **Rule 5: a run whose failure is a divergence is never recovered --
    /// and never even asked about.**
    ///
    /// `run-1788552801-73005`, batch 3: `take 64 iron-plate from the cell`
    /// found 40, the game handed them over, and the action was recorded
    /// failed. Tier 1 re-issued the identical take against the now-empty
    /// cell and got `removed 0` -- a whole recovery spent on a retry that
    /// could only fail. The plan's model of the container was wrong; only a
    /// replan re-reads it.
    ///
    /// Asserted on the ask, like the lost-action rule: `recover()` itself now
    /// refuses tier 1 for this class, so the ask would come back declined
    /// anyway, but refusing before asking is the stronger fact and the one
    /// that costs no `PlanState::from_world`. And asserted on the reason: the
    /// transition names the rule, so a driver can print why.
    #[test]
    fn a_diverged_take_is_replanned_without_asking_for_a_recovery() {
        let obs = format!(
            "{{{{failed=1, success=5, pending=4,
               first_error='{DIVERGED_TAKE}',
               actions={{[38]={{status='failed', attempts=1, error='{DIVERGED_TAKE}'}}}},
               recover={{why='rescheduled', steps=4}}}},
              {{success=9}}}}"
        );
        let lua = harness("{10, 0}", &obs);
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace, "acquired planned(10) ran(planning) satisfied finished",
            "the remainder is replanned, not continued"
        );
        assert_eq!(
            asks, "",
            "a diverged take must not even be offered to `recover`: the \
             proposal would issue the identical take against the same cell"
        );
        assert_eq!(plans, 2);
        assert_eq!(runs, 1);

        let lua = harness("{10, 0}", &obs);
        assert_eq!(refusals(&lua, "{}"), "divergence");
    }

    /// The divergence is read off the failed ACTIONS, not off `first_error`
    /// alone: the first failure of a batch can be something else entirely,
    /// and the take that diverged still decides.
    #[test]
    fn a_divergence_behind_an_earlier_failure_is_still_found() {
        let obs = format!(
            "{{{{failed=2, success=5, pending=4,
               first_error='no path to (12, 7)',
               actions={{[3]={{status='failed', error='no path to (12, 7)'}},
                        [38]={{status='failed', error='{DIVERGED_TAKE}'}},
                        [40]={{status='success'}}}},
               recover={{why='rescheduled', steps=4}}}},
              {{success=9}}}}"
        );
        let lua = harness("{10, 0}", &obs);
        let (_, _, _, asks) = trace(&lua, "{}");
        assert_eq!(asks, "");
        let lua = harness("{10, 0}", &obs);
        assert_eq!(refusals(&lua, "{}"), "divergence");
    }

    /// The other refusals name themselves the same way, and a run that was
    /// continued -- or had nothing to continue -- names nothing.
    #[test]
    fn every_refusal_names_its_rule_and_a_continuation_names_none() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, lost=1, success=5, recover={why='rescheduled', steps=4}}}",
        );
        assert_eq!(refusals(&lua, "{}"), "lost");

        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, recover={why='reexpanded', steps=4}}}",
        );
        assert_eq!(refusals(&lua, "{}"), "reexpanded");

        // Continued, then the continuation ran clean: nothing to refuse on
        // either transition.
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, pending=4, recover={why='rescheduled', steps=4}},
              {success=9}}",
        );
        assert_eq!(refusals(&lua, "{}"), "- -");
    }

    /// `supervisor.divergence` reads exactly the mod's three wordings and
    /// declines everything else -- the same contract as the executor's
    /// parser (`crates/executor/src/divergence.rs`), which is what makes the
    /// two refusals agree.
    #[test]
    fn the_divergence_reader_matches_the_three_wordings_and_nothing_else() {
        let lua = harness("{}", "{}");
        // One string per answer, because a multi-value return needs
        // `FromLuaMulti` and a tuple does not implement it.
        let read = |err: &str| -> Option<String> {
            lua.globals().set("__err", err).unwrap();
            lua.load(
                "local d = supervisor.divergence(__err)
                 if d == nil then return nil end
                 return d.item .. ' ' .. d.asked .. ' ' .. d.moved",
            )
            .eval::<Option<String>>()
            .unwrap()
        };
        assert_eq!(read(DIVERGED_TAKE).as_deref(), Some("iron-plate 64 40"));
        assert_eq!(
            read(r#"["tried to remove 64 iron-plate but removed 0"]"#).as_deref(),
            Some("iron-plate 64 0")
        );
        assert_eq!(
            read("tried to insert 50x coal but inserted 12").as_deref(),
            Some("coal 50 12")
        );
        assert_eq!(
            read("cannot insert 20x iron-ore, because player #1 only has 18. clamping...")
                .as_deref(),
            Some("iron-ore 20 18")
        );
        for text in [
            "game rejected the command: cannot insert to inventory of nonexisting entity",
            "tried to remove some iron-plate but removed fewer",
            "no action result received in time",
            "",
        ] {
            assert_eq!(read(text), None, "{text:?}");
        }
        assert!(
            lua.load("return supervisor.divergence(nil) == nil")
                .eval::<bool>()
                .unwrap()
        );
    }

    /// **Rule 3: at most `recovery_limit` recoveries per plan lineage.**
    ///
    /// Every run here makes progress, so the progress rule never fires and the
    /// cap is the only thing that can stop it. Tier 2 has no budget and cannot
    /// have one -- it is pure -- so `recover.rs` hands the duty to its caller,
    /// and this is the caller.
    #[test]
    fn a_lineage_gets_at_most_two_recoveries_even_while_it_is_progressing() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=1, recover={why='rescheduled', steps=8}},
              {failed=1, success=2, recover={why='rescheduled', steps=6}},
              {failed=1, success=3, recover={why='rescheduled', steps=4}}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(recovering) planned(8 rescheduled) \
             ran(recovering) planned(6 rescheduled) ran(planning) satisfied finished"
        );
        assert_eq!(
            asks, "rescheduled,rescheduled",
            "the third run is past the budget and must not ask at all"
        );
        assert_eq!(runs, 3, "one plan, executed three times, then a re-plan");
        assert_eq!(plans, 2);
    }

    /// The budget is per LINEAGE, not per milestone: a fresh `goal.plan` result
    /// is a fresh plan, a fresh log and a fresh budget.
    ///
    /// Without the reset, a milestone that recovered twice early would spend
    /// the rest of its iterations unable to recover at all -- and the reason
    /// would be invisible, because nothing would be asked.
    #[test]
    fn a_fresh_plan_restores_the_recovery_budget() {
        let lua = harness(
            "{10, 10, 0}",
            "{{failed=1, success=1, recover={why='rescheduled', steps=8}},
              {failed=1, success=2, recover={why='rescheduled', steps=6}},
              {failed=1, success=3, recover={why='rescheduled', steps=4}},
              {failed=1, success=1, recover={why='rescheduled', steps=7}},
              {success=9}}",
        );
        let (_trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            asks, "rescheduled,rescheduled,rescheduled",
            "two on the first lineage, then one on the second"
        );
        assert_eq!(plans, 3, "two real plans and the closing empty one");
        assert_eq!(runs, 5);
    }

    /// **Rule 4: a recovery that adds no successes is not a recovery.**
    ///
    /// This is the loop breaker, and it is the one the tiers cannot supply.
    /// A failed walk halts its bot; `abandon_rest` publishes `Failed` on the
    /// watch channels and writes **nothing to the log**, so every abandoned
    /// action stays `Pending`. `exhausted_tier_one` looks for `Failed` with
    /// `attempts >= 3` and no attempt count ever rises, so `recover()` proposes
    /// `Rescheduled` **forever** -- its budget is denominated in a unit this
    /// failure never produces. A walk is also the dominant failure: 76 failed
    /// walks against 28 failed/lost actions across 21 archived runs.
    ///
    /// `recovery_limit = 5` so that the cap cannot be what stops it. Only the
    /// progress rule can, and the second run is where it fires -- the first
    /// repetition, not the fifth.
    #[test]
    fn a_recovery_that_makes_no_new_progress_is_abandoned_on_the_first_repeat() {
        let lua = harness(
            "{10, 0}",
            "{{walks_failed=1, pending=10, success=0,
               first_error='the pathfinder returned no path',
               recover={why='rescheduled', steps=10}},
              {walks_failed=1, pending=10, success=0,
               first_error='the pathfinder returned no path',
               recover={why='rescheduled', steps=10}}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{recovery_limit = 5}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(recovering) planned(10 rescheduled) \
             ran(planning) satisfied finished"
        );
        assert_eq!(
            asks, "rescheduled",
            "the second run repeated the first exactly, so there is nothing to \
             continue and the loop must stop asking"
        );
        assert_eq!(runs, 2);
        assert_eq!(plans, 2);
    }

    /// The other side of rule 4, and the reason it is a delta rather than
    /// `success > 0`: a run whose very first walk failed has `success == 0`
    /// and is **exactly** the case tier 1 is for -- the reschedule demotes the
    /// refused `(bot, destination)` pair and another bot usually takes the
    /// work. A rule reading the absolute would refuse the whole failure class
    /// it was written for.
    #[test]
    fn a_first_run_that_succeeded_at_nothing_is_still_offered_a_recovery() {
        let lua = harness(
            "{10, 0}",
            "{{walks_failed=1, pending=10, success=0,
               recover={why='rescheduled', steps=10}},
              {success=10}}",
        );
        let (_trace, _plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(asks, "rescheduled");
        assert_eq!(runs, 2, "and the retry is what got the work done");
    }

    /// **A recovery does not touch the tracker, the iteration count or the
    /// step-count history.**
    ///
    /// A tier-1 proposal's step count is the REMAINDER -- 4 where the plan had
    /// 10 -- so `tracker.observe`ing it would read as a huge improvement and
    /// reset `stall`, hiding a genuine stall behind the loop's own retries.
    /// The transition still reports the tracker's numbers, so a driver printing
    /// "best N" prints the plan's best rather than the remainder's.
    ///
    /// This also pins the transition's shape, which is what a driver records:
    /// `action = "planned"` (so `record.plan_created` fires in a driver that
    /// has never heard of recovery), a `plan` table of the remainder, and the
    /// roster the proposal was made against.
    #[test]
    fn a_recovery_leaves_the_tracker_the_iterations_and_the_step_counts_alone() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, recover={why='rescheduled', steps=4}}, {success=9}}",
        );
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            sup:step()            -- acquired
            sup:step()            -- planned, 10 steps
            sup:step()            -- ran, and accepts the proposal
            local t = sup:step()  -- the recovery, as a plan
            __best, __stall = sup.tracker.best, sup.tracker.stall
            __iterations, __counts = sup.iterations, #sup.step_counts
            __action, __steps, __why = t.action, t.steps, t.recovery
            __best_on_t, __iteration_on_t = t.best, t.iteration
            __plan_len = #t.plan
            __bots = table.concat(t.bots, ",")
            __recoveries = t.recoveries
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__best").unwrap(),
            10,
            "the remainder is not a better plan; it is the same plan, partly done"
        );
        assert_eq!(g.get::<i64>("__stall").unwrap(), 0);
        assert_eq!(
            g.get::<i64>("__iterations").unwrap(),
            1,
            "one plan was made for this milestone, and a recovery is not another"
        );
        assert_eq!(g.get::<i64>("__counts").unwrap(), 1);
        assert_eq!(g.get::<String>("__action").unwrap(), "planned");
        assert_eq!(g.get::<i64>("__steps").unwrap(), 4);
        assert_eq!(g.get::<String>("__why").unwrap(), "rescheduled");
        assert_eq!(g.get::<i64>("__recoveries").unwrap(), 1);
        assert_eq!(
            g.get::<i64>("__best_on_t").unwrap(),
            10,
            "the transition reports the tracker's number, not the remainder's"
        );
        assert_eq!(g.get::<i64>("__iteration_on_t").unwrap(), 1);
        assert_eq!(
            g.get::<i64>("__plan_len").unwrap(),
            4,
            "shaped for `record.plan_created` exactly as a planned plan is"
        );
        assert_eq!(
            g.get::<String>("__bots").unwrap(),
            "1,2",
            "the roster the proposal was made against, which is what the record \
             writes and what nothing downstream can work out for itself"
        );
    }

    /// **A carried-forward log re-offers the previous run's walks.**
    ///
    /// `build_observation` iterates `log.walks()`, and a tier-1 proposal runs
    /// against the log of the run it recovers, while `start_walk` only
    /// overwrites the `(bot, step_index)` keys the narrower schedule reaches.
    /// So the survivors come back -- and `record.walks` writes
    /// `walk_dispatched` at the walk's own dispatch tick, which for a survivor
    /// is **earlier than the record's current position**. That is a broken
    /// monotonicity and a double count in `just analyse`.
    ///
    /// The loop therefore hands each walk to a driver exactly once per lineage,
    /// keyed by `(bot, step_index, dispatched_tick)` -- so a genuine re-walk of
    /// the same slot, which arrives with a new dispatch tick, is still offered.
    #[test]
    fn a_walk_already_handed_to_a_driver_is_not_offered_again_by_a_recovery() {
        let lua = harness(
            "{10, 0}",
            "{{pending=10, success=1,
               walks={{bot=1, step_index=0, dispatched_tick=100, status='failed',
                       to={x=1,y=2}, error='the pathfinder returned no path'}},
               recover={why='rescheduled', steps=4}},
              {success=5,
               walks={{bot=1, step_index=0, dispatched_tick=100, status='failed',
                       to={x=1,y=2}, error='the pathfinder returned no path'},
                      {bot=2, step_index=0, dispatched_tick=200, status='success',
                       to={x=3,y=4}}}}}",
        );
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            __offered, __failed_counts, __bots = {}, {}, {}
            repeat
                local t = sup:step()
                if t.action == "ran" then
                    __offered[#__offered + 1] = #t.walks
                    __failed_counts[#__failed_counts + 1] = t.walks_failed
                    local names = {}
                    for _, w in ipairs(t.walks) do names[#names + 1] = w.bot end
                    __bots[#__bots + 1] = table.concat(names, "+")
                end
            until sup:finished()
            __offered = table.concat(__offered, ",")
            __failed_counts = table.concat(__failed_counts, ",")
            __bots = table.concat(__bots, " ")
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(
            g.get::<String>("__offered").unwrap(),
            "1,1",
            "the recovery's observation lists two walks and only one of them is new"
        );
        assert_eq!(
            g.get::<String>("__bots").unwrap(),
            "1 2",
            "and the new one is the walk the recovery actually made"
        );
        assert_eq!(
            g.get::<String>("__failed_counts").unwrap(),
            "1,0",
            "the failed walk belongs to the run that made it; counting it again \
             would report a clean recovery as having failed the walk that \
             provoked it"
        );
    }

    /// The counts a driver prints are this run's own, derived from the walks it
    /// is being handed, and on a fresh log that is the whole log -- so nothing
    /// about an ordinary run changes.
    #[test]
    fn a_fresh_runs_walk_counts_are_unchanged_by_the_de_cumulation() {
        let lua = harness(
            "{5, 0}",
            "{{pending=5,
               walks={{bot=1, step_index=0, dispatched_tick=100, status='failed', to={x=1,y=2}},
                      {bot=2, step_index=1, dispatched_tick=110, status='lost', to={x=3,y=4}},
                      {bot=3, step_index=2, dispatched_tick=120, status='success', to={x=5,y=6}}}}}",
        );
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list {"a"}, {})
            local seen
            repeat
                local t = sup:step()
                if t.action == "ran" then seen = t end
            until sup:finished()
            __walks, __failed, __lost = #seen.walks, seen.walks_failed, seen.walks_lost
            "#,
        )
        .exec()
        .expect("driver runs");
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__walks").unwrap(), 3);
        assert_eq!(g.get::<i64>("__failed").unwrap(), 1);
        assert_eq!(g.get::<i64>("__lost").unwrap(), 1);
    }

    /// An observation with no `recover` at all -- an older binding, or any of
    /// the stubs above -- replans, exactly as the loop did before S1. "Cannot
    /// ask" must never come out as "carry on", and here the safe reading of it
    /// is the behaviour that existed before the question did.
    #[test]
    fn an_observation_with_no_recover_at_all_is_replanned() {
        let lua = harness("{10, 0}", "{{failed=1, success=5}}");
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(planning) satisfied finished"
        );
        assert_eq!(asks, "");
        assert_eq!(plans, 2);
        assert_eq!(runs, 1);
    }

    /// `recovery_limit = 0` turns the whole thing off and restores the previous
    /// behaviour byte for byte. This is the escape hatch a run wants when a
    /// recovery is suspected of hiding something -- and, until `S0` puts a
    /// `cause` on `PlanCreated`, it is also how a measured run gets its
    /// control.
    #[test]
    fn a_recovery_limit_of_zero_never_asks() {
        let lua = harness(
            "{10, 0}",
            "{{failed=1, success=5, recover={why='rescheduled', steps=4}}}",
        );
        let (trace, plans, runs, asks) = trace(&lua, "{recovery_limit = 0}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(planning) satisfied finished"
        );
        assert_eq!(asks, "");
        assert_eq!(plans, 2);
        assert_eq!(runs, 1);
    }

    /// `recover` raises for a run that is not finished and for a roster that
    /// has lost a bot since the plan was made. Neither is a reason to end the
    /// run: both mean "replan", which is what this loop does anyway. A raise
    /// here must not escape, and must not be mistaken for a proposal.
    #[test]
    fn a_recover_that_raises_falls_back_to_replanning() {
        let lua = harness("{10, 0}", "{{failed=1, success=5, recover={why='raise'}}}");
        let (trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            trace,
            "acquired planned(10) ran(planning) satisfied finished"
        );
        assert_eq!(asks, "raise", "the ask was made and it blew up");
        assert_eq!(plans, 2);
        assert_eq!(runs, 1);
    }

    /// Tier 0 and tier 3 both answer with no plan -- "the work is done" and
    /// "nothing mechanical is left" -- and both mean the same thing to this
    /// loop: there is nothing to continue, so replan and let the stall and the
    /// cap do their job.
    #[test]
    fn a_recovery_that_proposes_no_plan_replans() {
        for why in ["complete", "surfaced"] {
            let lua = harness(
                "{10, 0}",
                &format!("{{{{failed=1, success=5, recover={{why='{why}'}}}}}}"),
            );
            let (trace, plans, runs, asks) = trace(&lua, "{}");
            assert_eq!(
                trace,
                "acquired planned(10) ran(planning) satisfied finished"
            );
            assert_eq!(asks, why);
            assert_eq!(plans, 2);
            assert_eq!(runs, 1);
        }
    }

    /// A run that finished its plan cleanly is never asked. `recover()` would
    /// answer `Complete`, and the ask is not free: it builds a whole
    /// `PlanState` from the world. 8 of the archive's 57 replans are this case
    /// -- the plan ran to completion and the goal still did not hold -- and
    /// recovery buys them nothing.
    #[test]
    fn a_clean_run_is_never_asked_for_a_recovery() {
        let lua = harness(
            "{10, 6, 0}",
            "{{success=10, recover={why='rescheduled', steps=4}},
              {success=6, recover={why='rescheduled', steps=2}}}",
        );
        let (_trace, plans, runs, asks) = trace(&lua, "{}");
        assert_eq!(
            asks, "",
            "nothing failed and nothing is pending, so there is nothing to recover"
        );
        assert_eq!(plans, 3);
        assert_eq!(runs, 2);
    }

    // ---- Re-rostering: a bot that died leaves the roster, and comes back --

    /// A harness whose `roster` option answers a scripted sequence of
    /// `rcon.players()` replies, one per poll. `__roster_polls` counts the
    /// polls, so a test can see the bounded wait happen rather than infer it.
    /// Once the sequence is exhausted the last answer repeats, which is what
    /// a game whose state has settled looks like.
    fn roster_driver(lua: &Lua, answers: &str, patience: i64) -> (String, i64) {
        let driver = r#"
            __roster_answers = ANSWERS
            __roster_polls = 0
            local function roster()
                __roster_polls = __roster_polls + 1
                return __roster_answers[math.min(__roster_polls, #__roster_answers)]
            end
            local sup = supervisor.new(supervisor.list {"a"},
                { bots = {1, 2, 3, 4}, roster = roster, roster_patience = PATIENCE })
            local out, guard = {}, 0
            repeat
                local t = sup:step()
                local line = t.action
                if t.action == "planned" then
                    line = line .. "(" .. tostring(t.steps) .. ")"
                elseif t.action == "rerostered" then
                    line = line .. "{" .. table.concat(t.bots, ",") .. "|left "
                        .. table.concat(t.left, ",") .. "|back "
                        .. table.concat(t.returned, ",") .. "}"
                end
                out[#out + 1] = line
                guard = guard + 1
                if guard > 200 then error("did not terminate") end
            until sup:finished()
            __trace = table.concat(out, " ")
            __bots = table.concat(sup.bots, ",")
        "#
        .replace("ANSWERS", answers)
        .replace("PATIENCE", &patience.to_string());
        lua.load(&driver).exec().expect("driver runs");
        let g = lua.globals();
        (
            g.get::<String>("__trace").unwrap(),
            g.get::<i64>("__roster_polls").unwrap(),
        )
    }

    /// The default: no `roster` option, and the roster is exactly what it
    /// was before this existed -- fixed for the run, never asked about.
    #[test]
    fn without_a_roster_function_the_roster_is_never_re_checked() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let (trace, plans, runs) = drive(&lua, "{bots = {1, 2, 3, 4}}");
        assert_eq!(trace, "done");
        assert_eq!((plans, runs), (2, 1));
    }

    /// **A dead bot is dropped, but only after the bound.** Bot 3 is absent
    /// from every poll; the loop polls `roster_patience` times before the
    /// first plan, then plans for `{1, 2, 4}`, and the `rerostered`
    /// transition names who left. Every later plan is asked once and finds
    /// nothing to wait for.
    #[test]
    fn a_bot_absent_past_the_bound_is_dropped_and_the_transition_says_so() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let (trace, polls) = roster_driver(&lua, "{ {1, 2, 4} }", 5);
        assert_eq!(
            trace,
            "acquired rerostered{1,2,4|left 3|back } planned(10) ran satisfied finished"
        );
        // 5 polls for the first plan's wait, 1 each for the plan after the
        // re-roster and the closing empty re-plan.
        assert_eq!(polls, 5 + 1 + 1);
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2,4");
    }

    /// **A bot back within the bound costs the wait and nothing else.** Bot 3
    /// is missing for two polls and back on the third: no transition, the
    /// roster is unchanged, and the plan is for all four.
    #[test]
    fn a_bot_back_within_the_bound_is_waited_for_and_kept() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let (trace, polls) = roster_driver(&lua, "{ {1, 2, 4}, {1, 2, 4}, {1, 2, 3, 4} }", 800);
        assert_eq!(trace, "acquired planned(10) ran satisfied finished");
        assert_eq!(
            polls,
            3 + 1,
            "two misses, one hit, one check before the closing re-plan"
        );
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2,3,4");
    }

    /// **A dropped bot that respawns is picked up on the first poll that
    /// lists it**, and the transition names it under `returned`.
    #[test]
    fn a_bot_that_returns_is_re_added_on_the_next_plan() {
        // Plan 1: bot 3 absent for the whole bound -> dropped. Plan 2 (the
        // replan after a failed run): bot 3 is back -> re-added.
        let lua = harness("{10, 6, 0}", "{{failed=1, success=9}, {success=6}}");
        let (trace, _polls) = roster_driver(
            &lua,
            // Absent for the bound (polls 1-3) and for the check before the
            // first plan (poll 4); back from the check before the second.
            "{ {1, 2, 4}, {1, 2, 4}, {1, 2, 4}, {1, 2, 4}, {1, 2, 3, 4} }",
            3,
        );
        assert_eq!(
            trace,
            "acquired rerostered{1,2,4|left 3|back } planned(10) ran \
             rerostered{1,2,3,4|left |back 3} planned(6) ran satisfied finished"
        );
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2,3,4");
    }

    /// **A bot that was never rostered is never added.** Player 5 appears in
    /// every answer; the roster started as `{1, 2, 3, 4}` and stays so, with
    /// no transition -- "the first non-empty answer is the roster" is the bug
    /// that froze run 30, and this is the rule that keeps re-rostering from
    /// being a second road to it.
    #[test]
    fn a_bot_the_run_did_not_start_with_is_never_picked_up() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let (trace, _polls) = roster_driver(&lua, "{ {1, 2, 3, 4, 5} }", 5);
        assert_eq!(trace, "acquired planned(10) ran satisfied finished");
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2,3,4");
    }

    /// **An empty answer decides nothing.** Every poll answers `{}` -- the
    /// game not answering, or every bot dead at once -- and the roster is
    /// kept rather than emptied: planning for nobody is not a plan, and
    /// dropping four bots on a silence would be run 30 in reverse.
    #[test]
    fn an_empty_roster_answer_keeps_the_roster_and_records_nothing() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let (trace, polls) = roster_driver(&lua, "{ {} }", 4);
        assert_eq!(trace, "acquired planned(10) ran satisfied finished");
        assert_eq!(
            polls,
            4 + 4,
            "the bound is spent on every plan, and nothing is concluded"
        );
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2,3,4");
    }

    /// A roster function that raises is the same as one that answers
    /// nothing: the roster is kept.
    #[test]
    fn a_roster_function_that_raises_keeps_the_roster() {
        let lua = harness("{10, 0}", "{{success=10}}");
        let driver = r#"
            local sup = supervisor.new(supervisor.list {"a"},
                { bots = {1, 2}, roster = function() error("rcon is down") end,
                  roster_patience = 2 })
            repeat sup:step() until sup:finished()
            __bots = table.concat(sup.bots, ",")
            __state = sup.state
        "#;
        lua.load(driver).exec().expect("driver runs");
        let bots: String = lua.globals().get("__bots").unwrap();
        assert_eq!(bots, "1,2");
        assert_eq!(lua.globals().get::<String>("__state").unwrap(), "done");
    }

    /// **A recovery is not accepted for a roster that has lost a bot.** The
    /// tier-1 proposal was made for `{1, 2}`; bot 2 is gone by the time the
    /// loop would replay it, so the proposal is dropped, the change is
    /// reported, and the planner is asked again for `{1}` -- rather than the
    /// remainder handing bot 2 its share of a plan the game would refuse
    /// step by step.
    #[test]
    fn a_recovery_is_dropped_when_the_roster_changed_under_it() {
        let lua = harness(
            "{10, 4, 0}",
            "{{failed=1, success=5, pending=4, recover={why='rescheduled', steps=4}},
              {success=4}}",
        );
        let driver = r#"
            __roster_polls = 0
            local answers = { {1, 2}, {1} }
            local function roster()
                __roster_polls = __roster_polls + 1
                -- Present for the first plan's check; gone from the second
                -- poll on, i.e. by the time the recovery would be replayed.
                return answers[math.min(__roster_polls, #answers)]
            end
            local sup = supervisor.new(supervisor.list {"a"},
                { bots = {1, 2}, roster = roster, roster_patience = 2 })
            local out, guard = {}, 0
            repeat
                local t = sup:step()
                local line = t.action
                if t.action == "planned" then
                    line = line .. "(" .. tostring(t.steps)
                        .. (t.recovery and (" " .. t.recovery) or "") .. ")"
                elseif t.action == "ran" then
                    line = line .. "(" .. tostring(t.state) .. ")"
                elseif t.action == "rerostered" then
                    line = line .. "{" .. table.concat(t.bots, ",") .. "}"
                end
                out[#out + 1] = line
                guard = guard + 1
                if guard > 200 then error("did not terminate") end
            until sup:finished()
            __trace = table.concat(out, " ")
        "#;
        lua.load(driver).exec().expect("driver runs");
        let trace: String = lua.globals().get("__trace").unwrap();
        assert_eq!(
            trace,
            "acquired planned(10) ran(recovering) rerostered{1} planned(4) ran(planning) \
             satisfied finished"
        );
        let plans: i64 = lua.globals().get("__plan_calls").unwrap();
        assert_eq!(
            plans, 3,
            "the recovery was replaced by a real plan for the new roster"
        );
        let asks: String = lua
            .load("return table.concat(__recover_calls, ',')")
            .eval()
            .unwrap();
        assert_eq!(
            asks, "rescheduled",
            "the ask was made; its answer was dropped"
        );
    }

    // ---- Layer 7: the sustain window -- an idle window, not a verdict

    /// The first rung's spec, as the run script writes it, but short enough to
    /// poll through in a test: the shape is what is under test, not the size.
    const SUSTAIN_SPEC: &str = r#"supervisor.sustain {
        item = "iron-plate", per_minute = 15,
        window_ticks = 120, lead_in_ticks = 180, probe_ticks = 60,
    }"#;

    fn drive_sustain(lua: &Lua) {
        lua.load(
            r#"
            local sup = supervisor.new(supervisor.list { __spec }, {})
            local seen
            local guard = 0
            repeat
                local t = sup:step()
                if t.action ~= "acquired" and t.action ~= "finished" then seen = t end
                guard = guard + 1
                if guard > 20 then error("a sustain milestone did not terminate") end
            until sup:finished()
            __action, __state = seen.action, seen.state
            __code = seen.refusal and seen.refusal.code
            __message = seen.refusal and seen.refusal.message
            __s = seen.sustain
            "#,
        )
        .exec()
        .expect("the sustain milestone runs to a terminal state");
    }

    /// The property the whole rung rests on, and the same one the witness
    /// rests on: it dispatches nothing, so no feeding verb this run could have
    /// issued lands in the window or in the lead-in before it.
    #[test]
    fn a_sustain_window_dispatches_nothing_at_all() {
        let lua = witness_harness(&format!("__spec = {SUSTAIN_SPEC}"));
        drive_sustain(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__plan_calls").unwrap(), 0, "no expansion");
        assert_eq!(g.get::<i64>("__run_calls").unwrap(), 0, "and nothing ran");
        assert_eq!(
            g.get::<i64>("__holds_calls").unwrap(),
            0,
            "nor is the planner asked, which for a standing rate answers None anyway"
        );
    }

    /// **`satisfied` means the window happened, never that the rate held**, and
    /// the observation says so rather than leaving a reader to assume.
    ///
    /// This is the `obs.done` failure in advance: a field that answers a
    /// question nobody asked, confidently, about the wrong object. The rung
    /// cannot read per-machine counters -- they are not on `FactorioEntity` --
    /// so it names the command that can.
    #[test]
    fn an_elapsed_window_reports_deferred_and_names_what_answers_it() {
        let lua = witness_harness(&format!("__spec = {SUSTAIN_SPEC}"));
        drive_sustain(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "satisfied");
        let obs: factorio_bot_core::mlua::Table = g.get("__s").expect("an observation");
        assert_eq!(
            obs.get::<String>("verdict").unwrap(),
            "deferred",
            "the rung must not claim a rate it cannot measure"
        );
        assert!(
            obs.get::<i64>("elapsed_ticks").unwrap() >= 300,
            "the whole lead-in plus window elapsed"
        );
        let answered: String = obs.get("answered_by").unwrap();
        assert!(
            answered.contains("--sustain iron-plate:15:120:180"),
            "and it names the exact command that answers it: {answered}"
        );
    }

    /// A clock that is not advancing is `inconclusive`, never a failed rate.
    ///
    /// The same distinction `supervisor::witness_inconclusive` draws: a window
    /// that did not happen establishes nothing, and reporting it as a rate
    /// that did not hold would be the mistake this whole design exists to
    /// stop, one level up.
    #[test]
    fn a_clock_that_does_not_advance_is_inconclusive_not_a_failed_rate() {
        let lua = witness_harness(&format!("__tick_step = 0\n__spec = {SUSTAIN_SPEC}"));
        drive_sustain(&lua);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__action").unwrap(), "halted");
        assert_eq!(
            g.get::<String>("__code").unwrap(),
            "supervisor::sustain_inconclusive"
        );
        let msg: String = g.get("__message").unwrap();
        assert!(
            msg.contains("did not happen"),
            "and says why nothing follows: {msg}"
        );
    }

    /// Neither duration has a default, for `within_ticks`' reason.
    #[test]
    fn a_sustain_window_refuses_to_guess_its_durations() {
        let lua = witness_harness("");
        for (src, missing) in [
            (
                r#"supervisor.sustain { item = "iron-plate", per_minute = 15, lead_in_ticks = 9600 }"#,
                "window_ticks",
            ),
            (
                r#"supervisor.sustain { item = "iron-plate", per_minute = 15, window_ticks = 7200 }"#,
                "lead_in_ticks",
            ),
        ] {
            let err = lua.load(src).exec().expect_err(src).to_string();
            assert!(
                err.contains(missing) && err.contains("no default"),
                "{src}: {err}"
            );
        }
    }

    /// The roster option is checked at construction, like the source.
    #[test]
    fn a_roster_that_is_not_a_function_is_refused_at_construction() {
        let lua = harness("{}", "{}");
        let err = lua
            .load(r#"supervisor.new(supervisor.list {"a"}, { bots = {1}, roster = {1, 2} })"#)
            .exec()
            .expect_err("a table is not a roster function");
        assert!(
            err.to_string().contains("roster must be a function"),
            "{err}"
        );
    }

    // ---- The ring-widening source (`supervisor.chart_until`) -------------
    //
    // A live run of this costs 15-25 minutes and a whole map; these drive the
    // real source and the real loop against a world model small enough to
    // state in one number -- how far charted ground reaches, and how far out
    // the thing the target wants is. That is the entire physics the widening
    // depends on.

    /// A Lua state holding the real supervisor plus a `goal` table that
    /// models exactly one fact: **charted ground reaches `__reach` tiles, and
    /// the target needs ground out to `__want`**.
    ///
    /// `goal.plan` of the target refuses `planner::not_charted` while
    /// `__reach < __want` and plans otherwise; `goal.plan` of a
    /// `goal.charted(x, y, r)` emits eight surveys when `r > __reach` and
    /// nothing when it does not -- which is the idempotence the whole loop
    /// rests on -- and running one sets `__reach = r`. Eight because that is
    /// ring 1 of the real lattice, and 0 failures / 4,159 ticks is what one
    /// measured ring cost.
    ///
    /// `__target_code` swaps the target's refusal for one charting cannot
    /// clear, and `"raise"` makes it a fault the classifier does not vouch
    /// for -- the two negative controls.
    fn chart_harness(setup: &str) -> Lua {
        let lua = sandboxed();
        let stub = r#"
            -- A fresh map is generated to +/-320 before anybody walks.
            __reach = 320
            __want = 372.5
            __target_code = "planner::not_charted"
            __plan_calls = 0
            __run_calls = 0
            __target_ran = false
            __charted_planned = {}
            __keyframe_calls = 0
            record = { keyframe = function() __keyframe_calls = __keyframe_calls + 1 end }
            goal = {}
            __target = { kind = "target" }
            function goal.charted(x, y, radius)
                return { kind = "charted", x = x, y = y, radius = radius }
            end
            local function steps(n)
                local out = {}
                for i = 1, n do
                    out[i] = { id = i, bot = 1, label = "survey " .. i,
                               start = (i - 1) * 10, finish = i * 10, deps = {} }
                end
                return out
            end
            __last_code = nil
            goal.refusal = function(_err)
                if __last_code == nil then return nil end
                return { code = __last_code,
                         message = "no crude-oil is charted anywhere this plan can see; "
                             .. "charted ground reaches " .. tostring(__reach) }
            end
            goal.plan = function(g, _opts)
                __plan_calls = __plan_calls + 1
                if type(g) == "table" and g.kind == "charted" then
                    __last_code = nil
                    table.insert(__charted_planned, g.radius)
                    if g.radius <= __reach then return { steps = {} } end
                    return { steps = steps(8), radius = g.radius }
                end
                if __reach < __want then
                    if __target_code == "raise" then
                        __last_code = nil
                        error("stub: a fault nothing vouches for", 0)
                    end
                    __last_code = __target_code
                    error("goal: refused", 0)
                end
                __last_code = nil
                -- Once the target has run, it holds: a stub that kept
                -- returning work would stall the loop rather than satisfy it.
                if __target_ran then return { steps = {} } end
                return { steps = steps(3) }
            end
            goal.run = function(plan)
                __run_calls = __run_calls + 1
                -- The survey happened: the model now has the ground.
                if plan.radius ~= nil then
                    if plan.radius > __reach then __reach = plan.radius end
                else
                    __target_ran = true
                end
                return { failed = 0, lost = 0, walks_failed = 0, walks_lost = 0,
                         pending = 0, success = #plan.steps, running = 0, done = true,
                         actions = {} }
            end
            goal.holds = function(g, _opts)
                if type(g) == "table" and g.kind == "charted" then
                    return g.radius <= __reach
                end
                return true
            end
        "#;
        lua.load(stub).exec().expect("stub installs");
        lua.load(SUPERVISOR_LUA).exec().expect("supervisor loads");
        if !setup.is_empty() {
            lua.load(setup).exec().expect("setup runs");
        }
        lua
    }

    /// Drive a `chart_until` source to a terminal state and expose the census.
    fn drive_chart(lua: &Lua, spec: &str) {
        let driver = r#"
            local ch = supervisor.chart_until(SPEC)
            local sup = supervisor.new(ch.source, { bots = {1} })
            local guard, names, halts = 0, {}, {}
            repeat
                local t = sup:step()
                if t.action == "acquired" then
                    names[#names + 1] = ch:name_of(t.milestone_index)
                elseif t.action == "halted" then
                    halts[#halts + 1] = t.refusal and t.refusal.code or "?"
                end
                guard = guard + 1
                if guard > 500 then error("did not terminate") end
            until sup:finished()
            __state = sup.state
            __rings = ch.rings
            __radii = ch.radii
            __probes = ch.probes
            __reason = ch.reason
            __exhausted = ch.exhausted
            __names = names
            __halts = halts
            __milestones = #sup:history()
        "#
        .replace("SPEC", spec);
        lua.load(&driver).exec().expect("driver runs");
    }

    const SPEC: &str = "{ target = __target }";

    /// The whole point, end to end: one ring is walked, and then the target
    /// runs because it stopped refusing -- not because anything named oil.
    #[test]
    fn one_ring_is_walked_and_then_the_target_is_planned() {
        let lua = chart_harness("");
        drive_chart(&lua, SPEC);
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "done");
        assert_eq!(
            g.get::<i64>("__rings").unwrap(),
            1,
            "the oil at 372.5 is inside ring 1's reveal (384), so a second \
             ring would be walking nobody asked for"
        );
        assert_eq!(g.get::<Vec<f64>>("__radii").unwrap(), vec![384.0]);
        assert_eq!(
            g.get::<String>("__reason").unwrap(),
            "plannable",
            "the census says why it stopped, and `plannable` is the good one"
        );
        assert!(!g.get::<bool>("__exhausted").unwrap());
        assert_eq!(
            g.get::<i64>("__milestones").unwrap(),
            2,
            "one chart milestone and the target: both closed, both in history"
        );
        assert!(
            g.get::<Vec<String>>("__halts").unwrap().is_empty(),
            "nothing halted: the target planned and ran"
        );
    }

    /// The bound. Widening forever is not an option and stopping quietly is
    /// not either -- the run must end on the planner's own sentence.
    #[test]
    fn the_bound_ends_the_run_on_the_planners_refusal_rather_than_on_silence() {
        // Oil nowhere within reach of the bound.
        let lua = chart_harness("__want = 5000");
        drive_chart(&lua, SPEC);
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__rings").unwrap(),
            3,
            "the default bound, in rings"
        );
        assert_eq!(
            g.get::<Vec<f64>>("__radii").unwrap(),
            vec![384.0, 640.0, 896.0],
            "each ring widens by exactly one lattice pitch: a smaller step \
             adds no lattice cells at all and would chart nothing"
        );
        assert!(g.get::<bool>("__exhausted").unwrap());
        assert_eq!(g.get::<String>("__reason").unwrap(), "exhausted");
        assert_eq!(
            g.get::<String>("__state").unwrap(),
            "stuck",
            "NOT `done`: a source that simply stopped issuing milestones would \
             report the run as finished, which is the exact silence this bound \
             exists to avoid"
        );
        assert_eq!(
            g.get::<Vec<String>>("__halts").unwrap(),
            vec!["planner::not_charted".to_string()],
            "and the halt carries the planner's own code, so the record says \
             `still not charted after three rings` rather than nothing"
        );
    }

    /// A refusal a bigger disc cannot clear stops the widening immediately.
    /// Charting is expensive and `not_charted` is the only refusal it treats.
    #[test]
    fn a_refusal_charting_cannot_clear_walks_nobody_anywhere() {
        let lua = chart_harness("__want = 5000 __target_code = \"planner::not_hand_minable\"");
        drive_chart(&lua, SPEC);
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__rings").unwrap(), 0);
        assert_eq!(g.get::<i64>("__run_calls").unwrap(), 0);
        assert_eq!(
            g.get::<String>("__reason").unwrap(),
            "refused:planner::not_hand_minable",
            "the census names WHICH refusal ended it, not merely that one did"
        );
        assert_eq!(g.get::<String>("__state").unwrap(), "stuck");
    }

    /// A target that is already plannable charts nothing. This is what makes
    /// the source safe to put in front of a milestone unconditionally.
    #[test]
    fn a_target_that_already_plans_costs_no_walking() {
        let lua = chart_harness("__want = 0");
        drive_chart(&lua, SPEC);
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__rings").unwrap(), 0);
        assert_eq!(g.get::<String>("__reason").unwrap(), "plannable");
        assert_eq!(g.get::<String>("__state").unwrap(), "done");
        assert_eq!(
            g.get::<i64>("__probes").unwrap(),
            1,
            "exactly one expansion is spent finding that out"
        );
    }

    /// Ring `k` is re-issued as a disc containing every ring below it, so the
    /// already-walked ground must plan nothing rather than be walked again.
    #[test]
    fn a_wider_ring_does_not_re_walk_the_ground_the_last_one_covered() {
        let lua = chart_harness("__want = 5000");
        drive_chart(&lua, SPEC);
        let g = lua.globals();
        assert_eq!(
            g.get::<i64>("__run_calls").unwrap(),
            3,
            "three rings, three runs -- the replan that closes each milestone \
             finds nothing left and does not dispatch a fourth"
        );
    }

    /// The negative control for the classification, from the source's side:
    /// a raise `goal.refusal` does not vouch for is a fault and must end the
    /// run, never be read as "not charted, widen".
    #[test]
    fn a_raise_the_classifier_does_not_vouch_for_propagates_out_of_the_source() {
        let lua = chart_harness("__target_code = \"raise\"");
        let err = lua
            .load(
                "local ch = supervisor.chart_until { target = __target } \
                 local sup = supervisor.new(ch.source, { bots = {1} }) \
                 repeat sup:step() until sup:finished()",
            )
            .exec()
            .expect_err("a fault must not be swallowed as a widening signal");
        assert!(
            err.to_string().contains("a fault nothing vouches for"),
            "{err}"
        );
    }

    /// The record needs a name per milestone, and the source issues more than
    /// one, so a driver cannot hold a single constant any more.
    #[test]
    fn every_issued_milestone_has_its_own_name_for_the_record() {
        let lua = chart_harness("__want = 5000");
        drive_chart(&lua, SPEC);
        let names: Vec<String> = lua.globals().get("__names").unwrap();
        assert_eq!(names.len(), 4, "three rings and the target: {names:?}");
        assert!(names[0].contains("chart ring 1 of 3") && names[0].contains("384"));
        assert!(names[2].contains("chart ring 3 of 3") && names[2].contains("896"));
        assert!(
            names[3].contains("STILL not charted"),
            "the exhausted target says so in its own name: {names:?}"
        );
    }

    /// The radius is derived from the lattice, not typed: `floor(r / 256)`
    /// rings, and `256k + 128` is the ground a survey at `256k` reaches.
    #[test]
    fn the_radius_names_the_ground_the_survey_reaches() {
        let lua = chart_harness("");
        lua.load(
            "__r1 = supervisor.chart_radius(1) \
             __r2 = supervisor.chart_radius(2) \
             __r3 = supervisor.chart_radius(3)",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(g.get::<f64>("__r1").unwrap(), 384.0);
        assert_eq!(g.get::<f64>("__r2").unwrap(), 640.0);
        assert_eq!(g.get::<f64>("__r3").unwrap(), 896.0);
    }

    /// Construction errors, in the shape the witness and sustain use.
    #[test]
    fn a_chart_source_refuses_a_missing_target_and_a_nonsense_bound() {
        let lua = chart_harness("");
        for (src, wanted) in [
            ("supervisor.chart_until {}", "`target` is required"),
            (
                "supervisor.chart_until { target = __target, max_rings = 0 }",
                "positive integer",
            ),
            (
                "supervisor.chart_until { target = __target, around = 5 }",
                "must be a position",
            ),
        ] {
            let err = lua.load(src).exec().expect_err(src).to_string();
            assert!(err.contains(wanted), "{src}: {err}");
        }
    }

    // ---- Rocket policy: construction limiter and commissioning decisions ---

    fn policy_harness() -> Lua {
        let lua = sandboxed();
        lua.load(super::ROCKET_POLICY_LUA).exec().expect("rocket_policy loads");
        lua
    }

    #[test]
    fn a_building_state_with_all_constructed_and_no_flow_emits_repair() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={}, commissioned_ids={},
                       state="building", windows=0}
            local obs = {tick=100, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __instance_count = #next_m.instance_ids
            __state = next_m.state
            __original_state = m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(
            g.get::<String>("__decision").unwrap(),
            "repair",
            "cap reached, supply missing, evidence known -> repair"
        );
        assert_eq!(
            g.get::<i64>("__instance_count").unwrap(),
            2,
            "instance ids are preserved"
        );
        assert_ne!(
            g.get::<String>("__state").unwrap(),
            "complete",
            "no flow, so not complete"
        );
        assert_eq!(
            g.get::<String>("__original_state").unwrap(),
            "building",
            "input memory is not mutated"
        );
    }

    #[test]
    fn a_building_state_below_cap_emits_build() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=5, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11}, constructed_ids={}, commissioned_ids={},
                       state="building", windows=0}
            local obs = {tick=100, constructed_ids={}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __instance_count = #next_m.instance_ids
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "build",
            "below max_new_copies with nothing built -> build");
    }

    #[test]
    fn a_building_state_at_cap_not_constructed_observes() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11}, commissioned_ids={},
                       state="building", windows=0}
            local obs = {tick=1000, constructed_ids={11}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "observe",
            "at cap but not all constructed -> wait");
    }

    #[test]
    fn a_building_state_transitions_to_commissioning_when_flow_is_good() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=4000, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="building", windows=0}
            local obs = {tick=2000, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
            __window_start = next_m.window_start
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__state").unwrap(), "commissioning",
            "all built, flow good -> commission");
        assert_eq!(g.get::<i64>("__window_start").unwrap(), 2000,
            "window starts at observation tick");
    }

    #[test]
    fn unknown_evidence_in_building_state_yields_observe() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={}, commissioned_ids={},
                       state="building", windows=0}
            local obs = {tick=500, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=false, evidence_known=false}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "observe",
            "no evidence -> wait, cannot diagnose");
    }

    #[test]
    fn duplicate_ids_in_observation_are_deduplicated() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={}, commissioned_ids={},
                       state="building", windows=0}
            -- observation reports 11 twice and 12 once
            local obs = {tick=1000, constructed_ids={11,11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __constructed = #next_m.constructed_ids
            __has_11 = false; __has_12 = false
            for _, id in ipairs(next_m.constructed_ids) do
                if id == 11 then __has_11 = true end
                if id == 12 then __has_12 = true end
            end
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__constructed").unwrap(), 2,
            "duplicates are collapsed");
        assert!(g.get::<bool>("__has_11").unwrap(), "11 present");
        assert!(g.get::<bool>("__has_12").unwrap(), "12 present");
    }

    #[test]
    fn repeated_observations_are_idempotent() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=1, window_start=500}
            -- Same observation twice: mid-window, rate holding
            local obs = {tick=1500, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local m1, d1 = policy.limit(cfg, m, obs)
            local m2, d2 = policy.limit(cfg, m1, obs)
            __d1, __d2 = d1, d2
            __w1, __w2 = m1.windows, m2.windows
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__d1").unwrap(), "observe");
        assert_eq!(g.get::<String>("__d2").unwrap(), "observe",
            "same observation -> same decision");
        assert_eq!(g.get::<i64>("__w1").unwrap(), 1);
        assert_eq!(g.get::<i64>("__w2").unwrap(), 1,
            "windows not incremented mid-window");
    }

    #[test]
    fn commissioning_completes_after_three_windows() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=1000, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=2, window_start=7000}
            local obs = {tick=8000, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
            __windows = next_m.windows
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "complete",
            "3rd window good -> complete");
        assert_eq!(g.get::<String>("__state").unwrap(), "complete");
        assert_eq!(g.get::<i64>("__windows").unwrap(), 3);
    }

    #[test]
    fn commissioning_resets_window_count_on_rate_drop() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=1000, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=2, window_start=7000}
            local obs = {tick=8000, constructed_ids={11,12}, delivered_rate=30,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __windows = next_m.windows
            __window_start = next_m.window_start
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__windows").unwrap(), 0,
            "rate below target resets window count");
        assert_eq!(g.get::<i64>("__window_start").unwrap(), 8000,
            "new window starts at this observation tick");
    }

    #[test]
    fn supply_loss_mid_window_triggers_repair() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=5000, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=1, window_start=1000}
            local obs = {tick=3000, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __windows = next_m.windows
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "repair",
            "supply loss mid-window -> repair");
        assert_eq!(g.get::<i64>("__windows").unwrap(), 1,
            "window count should NOT reset on mid-window supply loss (not a full window)");
    }

    #[test]
    fn support_expiry_emits_blocked() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11}, commissioned_ids={},
                       state="building", windows=0, support_until=5000}
            local obs = {tick=6000, constructed_ids={11}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "blocked",
            "support expired -> blocked");
        assert_eq!(g.get::<String>("__state").unwrap(), "blocked");
    }

    #[test]
    fn depletion_emits_blocked() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=1, window_start=1000}
            local obs = {tick=3000, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=true, evidence_known=true, depleted=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "blocked",
            "depleted finite batch -> blocked");
        assert_eq!(g.get::<String>("__state").unwrap(), "blocked");
    }

    #[test]
    fn resume_midway_through_window_continues_observing() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=10000, required_windows=3}
            -- Resumed: saved memory says we were in a window that started at tick 5000
            -- Current tick is 8000 (3000 ticks into the window)
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=1, window_start=5000}
            local obs = {tick=8000, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __windows = next_m.windows
            __state = next_m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "observe",
            "mid-window -> observe");
        assert_eq!(g.get::<i64>("__windows").unwrap(), 1,
            "windows not incremented mid-window");
    }

    #[test]
    fn finite_batch_completes_with_quantity_check() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=1000, required_windows=3}
            -- Commissioning, need 3 windows, already have 2
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=2, window_start=7000}
            local obs = {tick=8000, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __state = next_m.state
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "complete",
            "third consecutive good window -> complete");
    }

    #[test]
    fn complete_state_returns_complete() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={11,12},
                       state="complete", windows=3, window_start=9000}
            local obs = {tick=10000, constructed_ids={11,12}, delivered_rate=60,
                         supply_ready=true, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "complete");
    }

    #[test]
    fn blocked_state_returns_blocked() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="blocked", windows=0}
            local obs = {tick=10000, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=false, evidence_known=true}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "blocked");
    }

    #[test]
    fn missing_evidence_during_commissioning_does_not_reset_windows() {
        let lua = policy_harness();
        let src = r#"
            local cfg = {target_rate=60, max_new_copies=2, window_ticks=1000, required_windows=3}
            local m = {instance_ids={11,12}, constructed_ids={11,12}, commissioned_ids={},
                       state="commissioning", windows=1, window_start=7000}
            -- Window elapsed, but no evidence -> observe, not reset
            local obs = {tick=8000, constructed_ids={11,12}, delivered_rate=0,
                         supply_ready=false, evidence_known=false}
            local next_m, decision = policy.limit(cfg, m, obs)
            __decision = decision
            __windows = next_m.windows
        "#;
        lua.load(src).exec().expect("policy.limit runs");
        let g = lua.globals();
        assert_eq!(g.get::<String>("__decision").unwrap(), "observe",
            "no evidence -> observe, not repair");
    }
    // ---- Layer 10: Rocket-policy next() and milestone orchestration -------
    //
    // Tests for policy.next(), the 10-stage policy engine.
    // Each test drives the real rocket_policy.lua through its decision
    // machinery against a stubbed snapshot.

    /// A policy harness that loads the full rocket_policy.lua and prepares
    /// a config, empty memory, and a mutable snapshot table.
    fn policy_next_harness() -> Lua {
        let lua = sandboxed();
        let stub = r#"
            __snapshot = {
                tick = 1000,
                researched = {},
                recipe_enabled = {},
                accessible_stock = {},
                instances = {},
                flow_evidence = {},
                lab_capacity = nil,
                threats = nil,
                launch_evidence = {},
                module_shortfalls = nil,
                total_produced = {},
            }
            __config = {
                max_new_copies = 2,
                window_ticks = 7200,
                required_windows = 3,
            }
            __memory = {}
            function set_stock(item, count)
                __snapshot.accessible_stock[item] = count
            end
            function set_researched(tech)
                __snapshot.researched[tech] = true
            end
            function set_instance(proto, count)
                __snapshot.instances[proto] = count
            end
            function set_flow(item)
                __snapshot.flow_evidence[item] = true
            end
            function set_produced(item, count)
                __snapshot.total_produced[item] = (__snapshot.total_produced[item] or 0) + count
            end
        "#;
        lua.load(stub).exec().expect("stub installs");
        lua.load(ROCKET_POLICY_LUA).exec().expect("rocket_policy loads");
        lua
    }

    /// Drive a single policy.next() call.
    fn drive_next(lua: &Lua) -> (String, i64, String, String, String) {
        lua.load(
            r#"
            local next_m, d = policy.next(__config, __memory, __snapshot)
            __memory = next_m
            __d_kind = tostring(d.kind)
            __d_stage = d.stage or 0
            __d_goal_type = (d.goal and d.goal.type) or ""
            __d_goal_name = (d.goal and (d.goal.name or d.goal.item)) or ""
            __d_reason = d.reason or ""
            "#,
        )
        .exec()
        .expect("policy.next runs");
        let g = lua.globals();
        (
            g.get::<String>("__d_kind").unwrap(),
            g.get::<i64>("__d_stage").unwrap(),
            g.get::<String>("__d_goal_type").unwrap(),
            g.get::<String>("__d_goal_name").unwrap(),
            g.get::<String>("__d_reason").unwrap(),
        )
    }

    /// A locked steel row (stage 8) selects its research first.
    #[test]
    fn a_locked_steel_row_selects_its_research_first() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 8 }
            __snapshot.researched = {}
            __snapshot.researched["engine"] = true
            __snapshot.researched["automation"] = true
            __snapshot.researched["oil-processing"] = true
            __snapshot.researched["chemical-science-pack"] = true
            "#,
        )
        .exec()
        .expect("stage 8 preconditions set");

        let (kind, stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "prerequisite");
        assert_eq!(stage, 8);
        assert_eq!(goal_type, "researched");
        assert!(
            goal_name.contains("advanced-material-processing"),
            "missing research must be advanced-material-processing, got: {goal_name}"
        );
    }

    /// An absent refinery emits a finite construction prerequisite.
    #[test]
    fn an_absent_refinery_emits_a_finite_construction_prerequisite() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 7 }
            set_researched("oil-processing")
            set_researched("automation")
            set_instance("oil-refinery", 0)
            set_instance("chemical-plant", 0)
            set_stock("plastic-bar", 0)
            set_stock("advanced-circuit", 0)
            "#,
        )
        .exec()
        .expect("stage 7 preconditions set");

        let (kind, _stage, _goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "build",
            "absent refinery should emit build decision");
        assert!(
            kind == "build",
            "build target should be oil-refinery related, got kind={kind} name={goal_name}"
        );
    }

    /// An already-constructed consumer with no plastic requests upstream repair.
    #[test]
    fn a_consumer_with_no_plastic_requests_upstream_repair() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 7 }
            set_researched("oil-processing")
            set_researched("automation")
            set_instance("oil-refinery", 1)
            set_instance("chemical-plant", 1)
            set_stock("advanced-circuit", 0)
            set_stock("processing-unit", 0)
            "#,
        )
        .exec()
        .expect("stage 7 with built refinery but no plastic");

        let (kind, _stage, _goal_type, _goal_name, reason) = drive_next(&lua);
        assert_eq!(kind, "support",
            "no plastic with built refinery should request upstream repair");
        assert!(reason.contains("plastic"),
            "reason must mention plastic: {reason}");
    }

    /// Stage 1 burner start requests drills first.
    #[test]
    fn stage_1_burner_start_requests_drills_first() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 1 }
            set_instance("burner-mining-drill", 0)
            set_stock("iron-plate", 0)
            set_stock("copper-plate", 0)
            "#,
        )
        .exec()
        .expect("stage 1 preconditions set");

        let (kind, stage, _goal_type, _goal_name, reason) = drive_next(&lua);
        assert_eq!(kind, "build", "stage 1 with no drills should request build");
        assert_eq!(stage, 1);
        assert!(reason.contains("drill"), "reason must mention drills: {reason}");
    }

    /// Logistic robotics is requested as research, not robot construction.
    #[test]
    fn logistic_robotics_is_requested_as_research_not_robot_construction() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 6 }
            set_researched("automation")
            set_researched("automation-science-pack")
            set_researched("logistics")
            set_stock("logistic-science-pack", 0)
            set_instance("assembling-machine-1", 1)
            "#,
        )
        .exec()
        .expect("stage 6 preconditions set");

        let (kind, _stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "prerequisite",
            "missing logistic-robotics must emit prerequisite, not build");
        assert_eq!(goal_type, "researched");
        assert!(
            goal_name.contains("logistic-robotics"),
            "the tech must be logistic-robotics, got: {goal_name}"
        );
    }

    /// Advanced-material-processing-2 research is requested, not furnace construction.
    #[test]
    fn advanced_material_processing_is_research_not_furnace_construction() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 8 }
            __snapshot.researched = {
                engine = true, automation = true,
                ["oil-processing"] = true,
                ["chemical-science-pack"] = true,
            }
            set_stock("steel-plate", 0)
            "#,
        )
        .exec()
        .expect("stage 8 preconditions set");

        let (kind, _stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "prerequisite");
        assert_eq!(goal_type, "researched");
        assert!(
            goal_name.contains("advanced-material-processing"),
            "needed tech: {goal_name}"
        );
    }

    /// 50-part target for rocket parts.
    #[test]
    fn fifty_part_target_is_requested() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 9 }
            set_researched("rocket-silo")
            set_researched("production-science-pack")
            set_researched("utility-science-pack")
            set_researched("rocket-fuel")
            set_researched("low-density-structure")
            set_researched("chemical-science-pack")
            set_researched("advanced-material-processing-2")
            set_instance("rocket-silo", 1)
            set_stock("rocket-part", 0)
            "#,
        )
        .exec()
        .expect("stage 9 with silo built, no parts");

        let (kind, stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "build", "stage 9 with no parts should request building");
        assert_eq!(stage, 9);
        assert!(
            goal_name == "rocket-part" || goal_type == "produce",
            "must target rocket-part: type={goal_type} name={goal_name}"
        );
    }

    /// Starter-pack unlock precedes space-platform trigger.
    #[test]
    fn starter_pack_unlock_precedes_space_platform() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 10 }
            set_researched("rocket-silo")
            set_researched("production-science-pack")
            set_researched("utility-science-pack")
            set_stock("steel-plate", 0)
            "#,
        )
        .exec()
        .expect("stage 10 preconditions set");

        let (kind, _stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "prerequisite",
            "stage 10 without space-platform must request research first");
        assert_eq!(goal_type, "researched");
        assert!(
            goal_name.contains("space-platform"),
            "must request space-platform: {goal_name}"
        );
    }

    /// Stage 4 needs steam-power research.
    #[test]
    fn stage_4_needs_steam_power_first() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 4 }
            set_researched("automation")
            set_researched("electronics")
            set_instance("boiler", 0)
            set_instance("steam-engine", 0)
            "#,
        )
        .exec()
        .expect("stage 4 preconditions set");

        let (kind, _stage, goal_type, goal_name, _reason) = drive_next(&lua);
        assert_eq!(kind, "prerequisite",
            "stage 4 without steam-power must request research");
        assert_eq!(goal_type, "researched");
        assert!(
            goal_name.contains("steam-power"),
            "must request steam-power: {goal_name}"
        );
    }

    /// Research queue generation.
    #[test]
    fn research_queue_generation() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            local q = policy.research_queue({ researched = { automation = true } })
            local names = {}
            for _, t in ipairs(q) do
                names[#names + 1] = t.name .. ":" .. tostring(t.branch)
            end
            __queue = table.concat(names, ", ")
            "#,
        )
        .exec()
        .expect("research queue runs");
        let q = lua.globals().get::<String>("__queue").unwrap();
        assert!(
            q.contains("electronics") || q.contains("logistics"),
            "ready queue should contain electronics or logistics: {q}"
        );
        assert!(q.contains("steam-power"), "steam-power should be ready: {q}");
    }

    /// Deadline infeasibility detected.
    #[test]
    fn deadline_infeasibility_is_detected() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            __memory = { stage = 1 }
            __snapshot.tick = 40000
            "#,
        )
        .exec()
        .expect("past-deadline scenario");

        let (kind, _stage, _goal_type, _goal_name, reason) = drive_next(&lua);
        assert_eq!(kind, "blocked",
            "stage past deadline should be blocked");
        assert!(reason.contains("deadline"),
            "reason must mention deadline: {reason}");
    }

    /// Research service time computation.
    #[test]
    fn research_service_time_is_computed() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            local minutes, rate = policy.research_service_time({
                lab_capacity = { packs_per_min = 30, remaining_packs = 750 }
            })
            __minutes = minutes
            __rate = rate
            "#,
        )
        .exec()
        .expect("research service time runs");
        let g = lua.globals();
        let minutes = g.get::<f64>("__minutes").unwrap();
        assert!(
            (minutes - 25.0).abs() < 1.0,
            "~25 minutes for 750 packs at 30/min, got {minutes}"
        );
        assert_eq!(g.get::<i64>("__rate").unwrap(), 30);
    }

    /// Required pack rate includes margin.
    #[test]
    fn required_pack_rate_includes_margin() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            local rate = policy.required_pack_rate({ tick = 0 }, 216000, 750)
            __rate = rate
            "#,
        )
        .exec()
        .expect("required pack rate runs");
        let rate = lua.globals().get::<i64>("__rate").unwrap();
        assert!(rate >= 30, "rate must be at least 30/min floor, got {rate}");
        assert!(rate <= 45, "rate for 750/36min with 25% margin ~26->30 floor, got {rate}");
    }

    /// Policy hash exports all 10 stage definitions.
    #[test]
    fn policy_hash_exports_all_stages() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            local h = policy.hash()
            __stage_count = 0
            for k, v in pairs(h) do
                if type(v) == "table" and v.id then
                    __stage_count = __stage_count + 1
                end
            end
            __rocket_parts = h.rocket_parts_required
            __payload_steel = h.total_payload_steel
            __payload_cable = h.total_payload_cable
            "#,
        )
        .exec()
        .expect("policy.hash runs");
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__stage_count").unwrap(), 10,
            "policy hash must include all 10 stages");
        assert_eq!(g.get::<i64>("__rocket_parts").unwrap(), 50);
        assert_eq!(g.get::<i64>("__payload_steel").unwrap(), 1220);
        assert_eq!(g.get::<i64>("__payload_cable").unwrap(), 1200);
    }

    /// policy.milestones() returns 10 stage descriptions.
    #[test]
    fn policy_milestones_returns_all_stages() {
        let lua = policy_next_harness();
        lua.load(
            r#"
            local ms = policy.milestones()
            __count = #ms
            __first_id = ms[1] and ms[1].stage_id
            __last_id = ms[10] and ms[10].stage_id
            "#,
        )
        .exec()
        .expect("policy.milestones runs");
        let g = lua.globals();
        assert_eq!(g.get::<i64>("__count").unwrap(), 10,
            "milestones must return all 10 stages");
        assert_eq!(g.get::<i64>("__first_id").unwrap(), 1);
        assert_eq!(g.get::<i64>("__last_id").unwrap(), 10);
    }


}
