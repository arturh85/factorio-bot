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
}

impl RunValue {
    /// A snapshot of the run exactly as things stand right now. Never
    /// blocks -- `watch::Receiver::borrow` reads the latest published value
    /// without waiting for a new one, so a run still in flight is reported
    /// as such rather than awaited.
    fn observation(&self, lua: &Lua) -> LuaResult<LuaTable> {
        let done = *self.finished_rx.borrow();
        let log = lock(&self.log);
        build_observation(lua, &self.net, &log, done)
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
            async move { await_completion(&lua, net, log, finished_rx).await }
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
    let (finished_tx, finished_rx) = watch::channel(false);
    let task_log = log.clone();
    let task_net = net.clone();
    let join = factorio_bot_core::tokio::spawn(async move {
        // A run refused outright (`ExecutionError::CircularWait`) dispatched
        // nothing, so the log stays exactly as empty as it started: the
        // observation reports every action pending, which is honest about
        // "nothing ran" even though it does not say why. Surfacing that
        // reason as its own error is future work, not this task's.
        let _ = run_into(&*act, &sched, &task_net, &task_log).await;
        // No receiver is an ordinary outcome, not a failure: it just means
        // nothing (`:wait()`, `PendingWork`'s drain) is waiting on this run.
        let _ = finished_tx.send(true);
    });
    (
        RunValue {
            net,
            log,
            finished_rx,
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
) -> LuaResult<LuaTable> {
    // A panicking run is already recorded in the log; observing its end here
    // is not the place to re-raise, and a dropped sender (the task ended
    // without ever sending `true`) is ignored for the same reason `mod.rs`'s
    // `wait_for_run` ignores it.
    let _ = finished_rx.wait_for(|done| *done).await;
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
    // Checked before anything is spawned, not after -- see `goal.execute`'s
    // own comment on this in `mod.rs` for why an unregistered run must never
    // exist at all rather than exist and be silently killed when `run_lua`
    // drops its tokio runtime.
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
                await_completion(&lua, run.net, run.log, run.finished_rx).await
            }
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::globals::goal::tests::{exec_bounded, lua_with_goal, Failure, StubActuator};
    use crate::globals::goal::value::install_goal_constructors;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// `lua_with_goal` plus the value-based `have`/`researched` `goal.plan`
    /// consumes.
    ///
    /// Mirrors `plan.rs`'s own `lua_with_world`: production does not wire the
    /// value constructors onto `have`/`researched` until the old
    /// handle-returning surface is deleted (a later task), so every module
    /// that wants to build a *value* to hand `goal.plan` installs them
    /// itself, on its own table, rather than changing what the ~20 existing
    /// handle-based tests in `mod.rs` see.
    fn lua_with_goal_values(stub: Arc<dyn Actuator>) -> Lua {
        let lua = lua_with_goal(stub);
        let table: LuaTable = lua.globals().get("goal").expect("goal table");
        install_goal_constructors(&lua, &table).expect("goal values");
        lua
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::First(
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
        let lua = lua_with_goal_values(Arc::new(StubActuator::new(Failure::Never)));
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
}
