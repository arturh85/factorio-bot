//! Runs and observations: `goal.start`/`goal.run` and the `RunValue` userdata
//! they hand back, plus the observation table both it and `goal.run` build.
//!
//! Lifted from the old `Runs`/`RunEntry` handle registry in `mod.rs` -- same
//! `tokio::sync::watch` completion signal, same reason `:wait()` clones the
//! receiver out rather than ever holding a borrow of the run across an
//! `.await` (see the comment on [`RunValue::add_methods`]'s `wait` method).
//! Only the handle indirection goes: a run is now a value a script holds
//! directly, exactly the shift `PlanValue` (`plan.rs`) already made for a
//! plan, and the two compose the same way -- `goal.start` calls
//! [`PlanValue::take_for_run`], which is also what makes "a plan may be run
//! once" true without a registry of its own to police it.

use super::plan::PlanValue;
use super::{goal_error, lock, ActuatorFactory};
use crate::lua_runner::PendingWork;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::tokio::sync::watch;
use factorio_bot_executor::{run_into, Actuator, ExecutionLog, Status};
use factorio_bot_planner::{ActionNetwork, Schedule};
use std::sync::{Arc, Mutex};

/// One failed action, exactly as `goal.run`/`:wait()` last observed it.
///
/// A plain Rust record rather than reading back out of the `actions` table
/// `build_observation` already built: `obs:failures()` is a Lua function
/// stored on the observation, and it may be called any number of times, so it
/// needs its own owned copy of what it reports rather than a borrow of
/// anything that could have moved on.
struct FailureRecord {
    id: u32,
    error: String,
    status: &'static str,
    attempts: u32,
    planned_start: Option<u32>,
    planned_end: Option<u32>,
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Success => "success",
        Status::Failed => "failed",
    }
}

/// Builds the observation table `goal.run`, `:wait()` and `:progress()` all
/// hand back.
///
/// Counted over the **network**, exactly like the old `Progress::of` in
/// `mod.rs`: an action nothing has dispatched yet has no log entry at all, so
/// counting only what the log knows would report one action out of one
/// rather than one out of thirty. `first_error` is the error of the
/// lowest-id failed action -- deterministic because `net.actions()` iterates
/// a `BTreeMap`, ascending by `ActionId`, so the first `Failed` action this
/// loop meets is always the same one on any two runs of the same log.
///
/// `planned_start`/`planned_end`, never `observed_*`: they are
/// `Attempt::planned_start_tick`/`planned_end_tick` travelling out unchanged,
/// numbers the *scheduler* computed before anything ran, not a measurement
/// the executor took (see `factorio_bot_executor::log` for why one is not on
/// offer). `planned_end` stays `nil` on an unfinished attempt, mirroring
/// `Attempt::planned_end_tick: Option<Ticks>` -- it is never defaulted to the
/// start, to zero, or to the makespan.
fn build_observation(
    lua: &Lua,
    net: &ActionNetwork,
    log: &ExecutionLog,
    done: bool,
) -> LuaResult<LuaTable> {
    let actions = lua.create_table()?;
    let mut pending = 0u32;
    let mut running = 0u32;
    let mut success = 0u32;
    let mut failed = 0u32;
    let mut failures: Vec<FailureRecord> = Vec::new();

    for action in net.actions() {
        let id = action.id;
        let status = log.status(id);
        match status {
            Status::Pending => pending += 1,
            Status::Running => running += 1,
            Status::Success => success += 1,
            Status::Failed => failed += 1,
        }

        let attempt = log.attempt(id);
        let attempts = log.attempts(id);
        let planned_start = attempt.map(|a| a.planned_start_tick);
        let planned_end = attempt.and_then(|a| a.planned_end_tick);
        let error = attempt.and_then(|a| a.error.clone());

        let t = lua.create_table()?;
        t.set("status", status_name(status))?;
        t.set("attempts", attempts)?;
        t.set("planned_start", planned_start)?;
        t.set("planned_end", planned_end)?;
        if let Some(error) = &error {
            t.set("error", error.clone())?;
        }
        actions.set(id.0, t)?;

        if status == Status::Failed {
            failures.push(FailureRecord {
                id: id.0,
                error: error.unwrap_or_default(),
                status: status_name(status),
                attempts,
                planned_start,
                planned_end,
            });
        }
    }

    let first_error = failures.first().map(|f| f.error.clone());

    let obs = lua.create_table()?;
    obs.set("done", done)?;
    obs.set("pending", pending)?;
    obs.set("running", running)?;
    obs.set("success", success)?;
    obs.set("failed", failed)?;
    obs.set("first_error", first_error)?;
    obs.set("actions", actions)?;
    obs.set(
        "failures",
        lua.create_function(move |lua, _self: Option<LuaValue>| {
            let out = lua.create_table()?;
            for (i, f) in failures.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", f.id)?;
                t.set("error", f.error.clone())?;
                t.set("status", f.status)?;
                t.set("attempts", f.attempts)?;
                t.set("planned_start", f.planned_start)?;
                t.set("planned_end", f.planned_end)?;
                out.set(i as i64 + 1, t)?;
            }
            Ok(out)
        })?,
    )?;
    Ok(obs)
}

/// A started run: an inspectable value holding exactly what an observation
/// needs.
///
/// `log` is an `Arc<Mutex<ExecutionLog>>`, not an owned `ExecutionLog`,
/// because `run_into` keeps writing into it from the spawned task while
/// `:progress()` reads it mid-run; `net` sizes the observation (see
/// `build_observation`); `finished_rx` is the same `tokio::sync::watch`
/// completion signal the old `RunEntry` used, cloned rather than taken
/// wherever it is awaited so a second `:wait()` still answers instead of
/// hanging (a `watch::Receiver`, unlike the run's own `JoinHandle`, can be
/// awaited by any number of independent clones).
pub(crate) struct RunValue {
    net: Arc<ActionNetwork>,
    log: Arc<Mutex<ExecutionLog>>,
    finished_rx: watch::Receiver<bool>,
    /// Set if `run_into` refused the run outright rather than executing it.
    ///
    /// `ExecutionError` is raised *before a single command reaches the game*
    /// (see its own doc comment), so a run that carries one dispatched
    /// nothing and its log is exactly as empty as it started. Reporting that
    /// as an observation would render an absent fact as a present one: every
    /// action `pending`, `done == true` -- a finished run that never began.
    /// The error is carried here instead and raised by every observation of
    /// this run, so the script hears why nothing happened.
    ///
    /// The one such error today, `CircularWait`, is unreachable through
    /// `goal.plan`: `schedule()` admits an action only once all its network
    /// predecessors are already placed, so every wait edge points forward
    /// along one global order. That is a fact about the current caller, not a
    /// property of this code -- a future scheduler, or a `Schedule` built any
    /// other way, reaches it -- so the shape is correct here regardless.
    start_error: Arc<Mutex<Option<String>>>,
}

impl RunValue {
    /// A snapshot of the run exactly as things stand right now. Never
    /// blocks -- `watch::Receiver::borrow` reads the latest published value
    /// without waiting for a new one, so a run still in flight is reported
    /// as such rather than awaited.
    fn observation(&self, lua: &Lua) -> LuaResult<LuaTable> {
        refuse_a_run_that_never_started(&self.start_error)?;
        let done = *self.finished_rx.borrow();
        let log = lock(&self.log);
        build_observation(lua, &self.net, &log, done)
    }
}

/// Raises if the run was refused before it started; see
/// [`RunValue::start_error`].
fn refuse_a_run_that_never_started(start_error: &Mutex<Option<String>>) -> LuaResult<()> {
    match lock(start_error).as_ref() {
        Some(err) => Err(goal_error(err)),
        None => Ok(()),
    }
}

impl LuaUserData for RunValue {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("progress", |lua, this, ()| this.observation(lua));

        // Everything `this` (a borrow into mlua's own userdata storage) is
        // needed for is cloned out *before* the async block below, and none
        // of the clones are moved back through `this` -- so nothing here
        // holds that borrow across the `.await`. Doing so would make a
        // concurrent `run:progress()` on this same value fail to borrow
        // while `:wait()` was still in flight, the exact hazard the module
        // doc's `wait_for_run` reasoning (lifted from `mod.rs`) warns about,
        // just relocated from a `std::sync::Mutex` guard to mlua's userdata
        // borrow tracking.
        methods.add_async_method("wait", |lua, this, ()| {
            let net = this.net.clone();
            let log = this.log.clone();
            let finished_rx = this.finished_rx.clone();
            let start_error = this.start_error.clone();
            async move { await_completion(&lua, net, log, finished_rx, start_error).await }
        });
    }
}

/// Spawns `sched` against `act`, returning the run immediately and the
/// task's own `JoinHandle` so the caller can register it into
/// [`PendingWork`] -- the same split `Runs::spawn` used to make in `mod.rs`,
/// and for the same reason: a stub `Actuator` can drive this directly in
/// tests, with no live game and no Lua involved at all.
fn spawn(
    act: Arc<dyn Actuator>,
    sched: Arc<Schedule>,
    net: Arc<ActionNetwork>,
) -> (RunValue, factorio_bot_core::tokio::task::JoinHandle<()>) {
    let log = Arc::new(Mutex::new(ExecutionLog::default()));
    let start_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (finished_tx, finished_rx) = watch::channel(false);
    let task_log = log.clone();
    let task_net = net.clone();
    let task_error = start_error.clone();
    let join = factorio_bot_core::tokio::spawn(async move {
        // A run refused outright dispatched nothing, so the log stays exactly
        // as empty as it started. Recording *why* is what keeps the
        // observation from reading as a finished run with everything still
        // pending -- see [`RunValue::start_error`].
        if let Err(err) = run_into(&*act, &sched, &task_net, &task_log).await {
            *lock(&task_error) = Some(err.to_string());
        }
        // No receiver is an ordinary outcome, not a failure: it just means
        // nothing (`:wait()`, `PendingWork`'s drain) is waiting on this run.
        let _ = finished_tx.send(true);
    });
    (
        RunValue {
            net,
            log,
            finished_rx,
            start_error,
        },
        join,
    )
}

/// Awaits the run's completion signal, then builds its observation.
///
/// Shared by `goal.run` and `RunValue::wait`'s method so the two can never
/// disagree about what "the run is over" means. The receiver is taken by
/// value and awaited here, never borrowed from a live `RunValue` across the
/// wait -- both callers clone it out first, for the reasons on
/// [`RunValue::add_methods`].
async fn await_completion(
    lua: &Lua,
    net: Arc<ActionNetwork>,
    log: Arc<Mutex<ExecutionLog>>,
    mut finished_rx: watch::Receiver<bool>,
    start_error: Arc<Mutex<Option<String>>>,
) -> LuaResult<LuaTable> {
    // A panicking run is already recorded in the log; observing its end here
    // is not the place to re-raise, and a dropped sender (the task ended
    // without ever sending `true`) is ignored for the same reason.
    let _ = finished_rx.wait_for(|done| *done).await;
    // Checked *after* the wait, not before: the task writes it as it ends, so
    // a check before the signal could read `None` on a run that was about to
    // report a refusal.
    refuse_a_run_that_never_started(&start_error)?;
    let log = lock(&log);
    build_observation(lua, &net, &log, true)
}

/// `goal.start`'s body, shared with `goal.run` so the two can never disagree
/// about what "starting" means: `goal.run` calls this and then
/// [`await_completion`], rather than re-driving `run_into` itself.
///
/// `taken` is computed by the caller, synchronously, from
/// [`PlanValue::take_for_run`] before this function -- and the `async`
/// closure that calls it -- ever exist, so no borrow of the plan's userdata
/// survives into the returned future either.
async fn start_impl(
    lua: &Lua,
    taken: LuaResult<(Arc<ActionNetwork>, Arc<Schedule>)>,
    actuator: &ActuatorFactory,
) -> LuaResult<RunValue> {
    let (net, sched) = taken?;
    // Checked before anything is spawned, not after: registration is what
    // stops a fire-and-forget `goal.start` from being killed mid-plan when
    // `run_lua` drops its tokio runtime (see `PendingWork` in
    // `lua_runner.rs`). A caller that omits `set_app_data` must not be able
    // to lose a run silently, so the loud failure has to come before the run
    // exists -- an unregistered run must never exist at all rather than
    // exist and be quietly killed.
    let pending = lua.app_data_ref::<PendingWork>().ok_or_else(|| {
        goal_error(
            "goal.start: no PendingWork registered for this Lua state; refusing to \
             start a run that could be silently killed when the interpreter's \
             runtime is dropped (internal error, not a script bug)",
        )
    })?;
    let pending = pending.clone();
    // Building the actuator is awaited; running the schedule is not -- an
    // actuator that cannot be built at all (no connected players, no reply
    // to the defines query) is a setup error the script should hear about at
    // the call, not a run that silently never happened.
    let act = actuator().await.map_err(goal_error)?;
    let (run, join) = spawn(act, sched, net);
    pending.register(join);
    Ok(run)
}

/// Installs `goal.start` and `goal.run` on `table`.
pub(crate) fn install_goal_run(
    lua: &Lua,
    table: &LuaTable,
    actuator: ActuatorFactory,
) -> LuaResult<()> {
    let start_actuator = actuator.clone();
    table.set(
        "start",
        lua.create_async_function(move |lua, plan: LuaUserDataRef<PlanValue>| {
            // `take_for_run` is the whole synchronous part of this call: it
            // is what flips a plan from unrun to running, and doing it here
            // -- not inside the future below -- is what keeps `plan`'s
            // borrow from ever crossing an `.await`.
            let taken = plan.take_for_run();
            let actuator = start_actuator.clone();
            async move { start_impl(&lua, taken, &actuator).await }
        })?,
    )?;

    table.set(
        "run",
        lua.create_async_function(move |lua, plan: LuaUserDataRef<PlanValue>| {
            let taken = plan.take_for_run();
            let actuator = actuator.clone();
            async move {
                let run = start_impl(&lua, taken, &actuator).await?;
                await_completion(&lua, run.net, run.log, run.finished_rx, run.start_error).await
            }
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::globals::goal::create_lua_goal_with;
    use crate::globals::goal::tests::{
        exec_bounded, factory, lua_with_goal, mining_plan, science_plan, seeded_world_for, Failure,
        StubActuator,
    };
    use factorio_bot_core::tokio::sync::mpsc;
    use factorio_bot_core::types::Position;
    use factorio_bot_planner::{
        Action, ActionId, ActionKind, BotId, ScheduledStep, StepKind, Ticks,
    };
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    // Every test here drives the table `create_lua_goal_with` really
    // returns. Until the handle surface was deleted this module had to
    // install the value constructors onto that table itself, because
    // production still bound `have`/`researched` to the handle-returning
    // closures; now it does not, and the helper that did it is gone with
    // them -- which is the point of the deletion.

    /// Reads the four counts and `done` off an observation table.
    fn counts(obs: &LuaTable) -> (u32, u32, u32, u32, bool) {
        (
            obs.get("pending").expect("pending"),
            obs.get("running").expect("running"),
            obs.get("success").expect("success"),
            obs.get("failed").expect("failed"),
            obs.get("done").expect("done"),
        )
    }

    /// A `Lua` for reading observations built below the Lua seam. These
    /// tests drive [`spawn`] directly, so they need an interpreter only to
    /// hold the table [`build_observation`] writes into.
    fn observing_lua() -> Lua {
        crate::sandbox::new_sandboxed_lua().expect("sandbox")
    }

    /// Runs `code`, failing rather than hanging if it does not finish, and
    /// returning the error `code` raised.
    ///
    /// `exec_bounded` (reused from `mod.rs`) is the success-path counterpart;
    /// this is its mirror for the tests here that assert a script must
    /// fail -- `exec_bounded` itself `.expect`s success, so it cannot be used
    /// for them.
    async fn exec_bounded_err(lua: &Lua, code: &str) -> String {
        let outcome = factorio_bot_core::tokio::time::timeout(
            Duration::from_secs(10),
            lua.load(code).exec_async(),
        )
        .await;
        match outcome {
            Err(_) => panic!("the script did not finish within 10s"),
            Ok(Ok(())) => panic!("the script was expected to fail but succeeded"),
            Ok(Err(err)) => err.to_string(),
        }
    }

    #[tokio::test]
    async fn run_reports_a_finished_observation() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local p = goal.plan(goal.have("iron-ore", 2))
            local obs = goal.run(p)
            assert(obs.done, "a returned run is finished")
            assert(obs.failed == 0, "nothing failed, got " .. obs.failed)
            assert(obs.success > 0, "something succeeded")
            assert(obs.pending == 0 and obs.running == 0, "nothing left outstanding")
            assert(obs.first_error == nil, "no error on a clean run")
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn an_observation_carries_per_action_outcomes_keyed_by_step_id() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local p = goal.plan(goal.have("iron-ore", 2))
            local obs = goal.run(p)
            local checked = 0
            for _, s in ipairs(p.steps) do
                if s.kind ~= "walk" then
                    local a = obs.actions[s.id]
                    assert(a, "no outcome for action " .. tostring(s.id))
                    assert(a.status == "success", "status for " .. s.id .. ": " .. a.status)
                    assert(a.attempts == 1, "one attempt")
                    assert(type(a.planned_start) == "number", "planned_start")
                    assert(type(a.planned_end) == "number", "planned_end once finished")
                    checked = checked + 1
                end
            end
            assert(checked > 0, "the plan had action steps to check")
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn tick_fields_are_named_planned_not_observed() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local obs = goal.run(goal.plan(goal.have("iron-ore", 2)))
            for id, a in pairs(obs.actions) do
                assert(a.planned_start ~= nil, "planned_start")
                assert(a.observed_start == nil, "these are estimates, not measurements")
                assert(a.observed_end == nil, "these are estimates, not measurements")
            end
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn failures_are_reported_with_their_errors() {
        // `Failure::First` fails the first action dispatched anywhere and
        // succeeds thereafter.
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::First(
            AtomicBool::new(false),
        ))));
        exec_bounded(
            &lua,
            r#"
            local obs = goal.run(goal.plan(goal.have("iron-ore", 2)))
            assert(obs.failed >= 1, "the stub failed an action, got " .. obs.failed)
            assert(type(obs.first_error) == "string", "first_error is set")
            assert(#obs.first_error > 0, "first_error is not empty")
            local fs = obs:failures()
            assert(#fs == obs.failed, "failures() agrees with the count")
            assert(type(fs[1].error) == "string" and #fs[1].error > 0, "each failure carries its error")
            assert(fs[1].id ~= nil, "each failure names its action")
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn start_is_non_blocking_and_progress_reads_it() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local run = goal.start(goal.plan(goal.have("iron-ore", 2)))
            -- Returning at all is the assertion: a blocking start could not
            -- reach this line before the run finished.
            local snap = run:progress()
            assert(type(snap.done) == "boolean", "progress answers with an observation")
            assert(type(snap.success) == "number", "and it carries counts")
            local obs = run:wait()
            assert(obs.done, "wait returns only once the run is over")
            assert(obs.failed == 0, "clean run")
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn wait_is_idempotent() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local run = goal.start(goal.plan(goal.have("iron-ore", 2)))
            local a = run:wait()
            local b = run:wait()
            assert(a.done and b.done, "both waits return a finished observation")
            assert(a.success == b.success, "and they agree")
        "#,
        )
        .await;
    }

    #[tokio::test]
    async fn running_one_plan_twice_raises() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        let err = exec_bounded_err(
            &lua,
            r#"
            local p = goal.plan(goal.have("iron-ore", 2))
            goal.run(p)
            goal.run(p)
        "#,
        )
        .await;
        assert!(err.contains("already"), "{err}");
    }

    #[tokio::test]
    async fn goal_run_equals_start_then_wait() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local direct = goal.run(goal.plan(goal.have("iron-ore", 2)))
            local staged = goal.start(goal.plan(goal.have("iron-ore", 2))):wait()
            assert(direct.done == staged.done, "done")
            assert(direct.success == staged.success, "success")
            assert(direct.failed == staged.failed, "failed")
        "#,
        )
        .await;
    }

    // ------------------------------------------------------------- counting
    //
    // These four drive [`spawn`] and [`await_completion`] directly rather
    // than through `goal.run`. They are about what an observation *counts*,
    // and the plans they count over (a four-bot red-science plan; a two-bot
    // mining plan) are built from the planner directly so the numbers under
    // test are not also a function of whatever `goal.plan` happens to choose.
    // Ported from the handle-era tests of the same names, which drove
    // `Runs::spawn`/`wait_for_run` in `mod.rs`.

    // `start_paused` because the executor honours the schedule's lag edges in
    // real wall-clock time (`executor/run.rs`), and red science smelts: the
    // run takes ~29 real seconds otherwise. Auto-advance is safe here
    // precisely because this test has no gate -- every task is either working
    // or waiting on a timer, so the clock only jumps when nothing can make
    // progress.
    #[tokio::test(start_paused = true)]
    async fn a_run_that_abandons_work_is_done_while_actions_are_still_pending() {
        // The test for the central design decision: `done` is a flag set when
        // the run's task returns, and it CANNOT be derived from the counts.
        //
        // When a bot's action fails, `executor/run.rs`'s `abandon_rest`
        // releases the waiters on the watch channel and writes nothing to the
        // log, so the rest of that bot's slice is never dispatched and stays
        // `Pending` forever. A finished run therefore legitimately reports
        // pending work, and `pending == 0 && running == 0` would call it
        // unfinished for good.
        let (net, sched) = science_plan();
        let total = net.len() as u32;
        let (run, _join) = spawn(
            Arc::new(StubActuator::new(Failure::First(AtomicBool::new(false)))),
            sched,
            net,
        );
        let lua = observing_lua();
        let obs = await_completion(&lua, run.net, run.log, run.finished_rx, run.start_error)
            .await
            .expect("the run finished");
        let (pending, running, success, failed, done) = counts(&obs);
        assert!(done, "the run's task has returned, so it is done");
        assert_eq!(failed, 1, "one action was refused");
        assert!(
            pending > 0,
            "the failing bot's remaining slice was abandoned undispatched, so it must \
             still count as pending -- this is the case that makes `done` a flag \
             rather than `pending == 0`"
        );
        assert!(success > 0, "the other bots carried on");
        assert_eq!(
            pending + running + success + failed,
            total,
            "the counts must partition the plan's actions"
        );
    }

    #[tokio::test]
    async fn an_action_in_flight_counts_as_running_not_as_pending() {
        // `Running` is a status of its own, and folding it into `pending`
        // would make a live run indistinguishable from one that has not
        // started. Observed by signal rather than by sleeping: the stub
        // reports each dispatch and then blocks on a gate the test never
        // opens.
        let (net, sched) = mining_plan();
        let total = net.len() as u32;
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let (_gate_tx, gate_rx) = watch::channel(false);
        let stub = StubActuator {
            entered: Some(entered_tx),
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };

        let (run, _join) = spawn(Arc::new(stub), sched, net);
        entered_rx.recv().await.expect("an action was dispatched");

        let lua = observing_lua();
        let obs = run.observation(&lua).expect("observation");
        let (pending, running, success, failed, done) = counts(&obs);
        assert!(running > 0, "a dispatched action is running, not pending");
        assert_eq!(success, 0, "the gate is shut");
        assert!(!done, "the run is still going");
        assert_eq!(
            pending + running + success + failed,
            total,
            "the counts must partition the plan's actions"
        );
    }

    #[tokio::test]
    async fn a_finished_run_counts_every_action_as_succeeded() {
        let (net, sched) = mining_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let (run, _join) = spawn(Arc::new(StubActuator::new(Failure::Never)), sched, net);
        let lua = observing_lua();
        let obs = await_completion(&lua, run.net, run.log, run.finished_rx, run.start_error)
            .await
            .expect("the run finished");
        let (pending, running, success, failed, done) = counts(&obs);
        assert!(done, "wait must return a finished run");
        assert_eq!(success, total, "every action of the plan succeeded");
        assert_eq!(
            pending + running + success + failed,
            total,
            "the counts must partition the plan's actions"
        );
    }

    #[tokio::test]
    async fn a_run_whose_actions_are_rejected_finishes_with_failures_not_successes() {
        // The counterpart that keeps the test above honest: with only the
        // success path exercised, `build_observation` could map every status
        // to `success` and both would still pass.
        let (net, sched) = mining_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let (run, _join) = spawn(Arc::new(StubActuator::new(Failure::Always)), sched, net);
        let lua = observing_lua();
        let obs = await_completion(&lua, run.net, run.log, run.finished_rx, run.start_error)
            .await
            .expect("the run finished");
        let (pending, running, success, failed, done) = counts(&obs);
        assert!(done, "a failed run is still a finished run");
        assert_eq!(success, 0, "nothing succeeded");
        assert_eq!(failed, total, "every action was dispatched and rejected");
        assert_eq!(
            pending + running + success + failed,
            total,
            "the counts must partition the plan's actions"
        );
    }

    // ------------------------------------------- a run that never started

    /// A network of two actions whose schedule runs a network edge backwards,
    /// which `run_into` refuses with `ExecutionError::CircularWait`: the
    /// network says 1 must precede 0, and `1 -> 0` alone is perfectly
    /// acyclic, but one bot is scheduled to run 0 first and so waits at its
    /// own first step for something only its second step could run.
    ///
    /// Built by hand because `goal.plan` cannot produce it: `schedule()`
    /// admits an action only once every network predecessor is placed, so
    /// every wait edge it emits points forward along one global order. That
    /// makes the refusal unreachable through today's caller and says nothing
    /// about tomorrow's -- an observation that renders "nothing ran" as
    /// "everything finished" is wrong on its own terms.
    fn circular_wait_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
        let mut net = ActionNetwork::new();
        for (id, item) in [(ActionId(0), "iron-ore"), (ActionId(1), "copper-ore")] {
            net.add(Action {
                id,
                kind: ActionKind::Mine {
                    pos: Position::new(10., 10.),
                    item: item.into(),
                    count: 1,
                },
                pre: vec![],
                eff: vec![],
                duration: 60,
                pinned: None,
                label: format!("mine 1 {item}"),
            });
        }
        net.link(ActionId(1), ActionId(0), 0);
        let step = |action: ActionId, start: Ticks, end: Ticks| ScheduledStep {
            what: StepKind::Act {
                action,
                label: format!("act {action:?}"),
            },
            bot: BotId(1),
            start,
            end,
        };
        let sched = Schedule {
            steps: vec![step(ActionId(0), 0, 60), step(ActionId(1), 60, 120)],
            makespan: 120,
        };
        (Arc::new(net), Arc::new(sched))
    }

    #[tokio::test]
    async fn a_run_refused_before_it_started_raises_instead_of_reporting_done() {
        let (net, sched) = circular_wait_plan();
        let (run, _join) = spawn(Arc::new(StubActuator::new(Failure::Never)), sched, net);
        let lua = observing_lua();

        // Waiting on it must raise rather than hand back an observation. The
        // shape being closed here is the one that would otherwise come back:
        // `done = true` with every action `pending`, i.e. a finished run that
        // never began.
        let err = await_completion(
            &lua,
            run.net.clone(),
            run.log.clone(),
            run.finished_rx.clone(),
            run.start_error.clone(),
        )
        .await
        .expect_err("a run that was refused has nothing to report");
        assert!(
            err.to_string().contains("circular wait"),
            "the error must say why nothing ran: {err}"
        );
        // And polling it says the same thing, rather than the two disagreeing
        // about whether there is a run to observe at all.
        let err = run
            .observation(&lua)
            .expect_err("a refused run has no progress either");
        assert!(err.to_string().contains("circular wait"), "{err}");
    }

    #[tokio::test]
    async fn a_run_refused_before_it_started_reaches_the_script() {
        // The same refusal through the binding a script calls, since that is
        // the path the error has to survive: `goal.run` awaits the run and so
        // must re-raise rather than swallow.
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        let (net, sched) = circular_wait_plan();
        let plan = PlanValue::new(net, sched, vec![BotId(1)]);
        lua.globals()
            .set("p", lua.create_userdata(plan).expect("userdata"))
            .expect("set p");

        let err = exec_bounded_err(&lua, "goal.run(p)").await;
        assert!(
            err.contains("circular wait"),
            "the script must hear why nothing ran: {err}"
        );
    }

    // ------------------------------------------------------- the bindings

    #[tokio::test]
    async fn start_returns_before_the_run_has_finished() {
        // Driven through the binding a script actually calls, not through
        // `spawn` underneath it: a `goal.start` rewritten as
        // spawn-then-await passes every test that reaches around it.
        //
        // The gate is never opened, so nothing this run dispatches can
        // finish. A binding that waits therefore cannot return at all, and
        // `exec_bounded` reports that as a failure rather than a hang.
        let (_gate_tx, gate_rx) = watch::channel(false);
        let stub = StubActuator {
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };
        let lua = lua_with_goal(Arc::new(stub));

        exec_bounded(
            &lua,
            r#"
            local run = goal.start(goal.plan(goal.have("iron-ore", 20)))
            local s = run:progress()
            assert(not s.done, "goal.start must return before the run has finished")
            assert(s.success + s.failed == 0, "the gate is shut: nothing can complete")
            assert(s.pending + s.running > 0, "the plan's actions must all still be outstanding")
            "#,
        )
        .await;
    }

    /// `goal.start` must not read a missing `PendingWork` as "nothing to
    /// register into, carry on anyway": in production only `run_lua` installs
    /// one, and a future caller that forgets to must not be able to start a
    /// run nothing will keep alive when its runtime is dropped.
    ///
    /// Proven to discriminate by the assertion at the end: not just that the
    /// call errors, but that nothing was ever dispatched -- i.e. no run was
    /// started at all, not merely a run whose value came back unusable.
    #[tokio::test]
    async fn start_without_pending_work_installed_fails_loudly_instead_of_losing_the_run() {
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let stub = StubActuator {
            entered: Some(entered_tx),
            ..StubActuator::new(Failure::Never)
        };
        // Built directly rather than through `lua_with_goal`: that helper
        // installs `PendingWork` the way `run_lua` does, and this test is
        // specifically about the caller that forgets to.
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(Arc::new(stub)),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err =
            exec_bounded_err(&lua, r#"goal.start(goal.plan(goal.have("iron-ore", 20)))"#).await;
        assert!(
            err.contains("PendingWork"),
            "the error should name what is missing: {err}"
        );

        let dispatched =
            factorio_bot_core::tokio::time::timeout(Duration::from_millis(200), entered_rx.recv())
                .await;
        assert!(
            dispatched.is_err(),
            "no action should ever have been dispatched: goal.start must refuse \
             before a run is spawned, not spawn one it then fails to register"
        );
    }

    /// A script that calls `goal.start` and never waits must still have its
    /// run finish, because `goal.start` registers the run's task into
    /// [`PendingWork`] -- the seam `run_lua` drains before dropping the
    /// runtime that owns it. A test that also waited would prove nothing
    /// about this: the wait would carry the run to completion on its own
    /// regardless of whether anything was registered.
    ///
    /// The gate keeps every dispatched action blocked, so a `pending.drain()`
    /// that returned before the gate opened could only mean the run's
    /// `JoinHandle` was never registered -- proving the drain is genuinely
    /// awaiting the registered task, not merely that it returns eventually.
    #[tokio::test]
    async fn a_fire_and_forget_start_registers_its_run_so_it_still_finishes() {
        let (gate_tx, gate_rx) = watch::channel(false);
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let stub = StubActuator {
            entered: Some(entered_tx),
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };
        let lua = lua_with_goal(Arc::new(stub));
        let pending = PendingWork::default();
        lua.set_app_data(pending.clone());

        // No wait anywhere in this script -- `r` is left global so the test
        // can still read progress on it afterwards.
        exec_bounded(
            &lua,
            r#"r = goal.start(goal.plan(goal.have("iron-ore", 20)))"#,
        )
        .await;

        // The chunk has already returned. Confirm the run has actually
        // started (and is now blocked on the shut gate) before reasoning
        // about whether draining waits for it.
        entered_rx.recv().await.expect("an action was dispatched");
        let progress: LuaTable = lua.load("return r:progress()").eval().expect("progress");
        let done: bool = progress.get("done").expect("done");
        assert!(
            !done,
            "the run must not have finished yet: the gate is shut"
        );

        let mut drain_task = factorio_bot_core::tokio::spawn({
            let pending = pending.clone();
            async move { pending.drain().await }
        });
        let premature =
            factorio_bot_core::tokio::time::timeout(Duration::from_millis(200), &mut drain_task)
                .await;
        assert!(
            premature.is_err(),
            "pending.drain() returned while the run was still gated -- goal.start \
             did not register a handle for it to await"
        );

        gate_tx.send(true).expect("gate has a receiver");
        let failures = factorio_bot_core::tokio::time::timeout(Duration::from_secs(5), drain_task)
            .await
            .expect("drain did not finish after the gate opened")
            .expect("the drain task itself panicked");
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");

        let progress: LuaTable = lua.load("return r:progress()").eval().expect("progress");
        let done: bool = progress.get("done").expect("done");
        let success: u32 = progress.get("success").expect("success");
        assert!(done, "the run must have finished once drain() returned");
        assert!(success > 0, "the run must have actually executed something");
    }

    #[tokio::test]
    async fn a_progress_after_a_wait_reports_the_same_run() {
        // `:wait()` really does block until the run is over, and `:progress()`
        // afterwards reports the same thing rather than a fresh, emptier view.
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local run = goal.start(goal.plan(goal.have("iron-ore", 20)))
            local w = run:wait()
            local s = run:progress()
            assert(w.done and s.done, "wait must return a finished run")
            assert(w.success > 0, "the run must have executed something")
            assert(w.success == s.success, "a poll after a wait reports the same run")
            "#,
        )
        .await;
    }
}
