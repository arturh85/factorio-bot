//! The `goal.*` Lua table: declarative goals planned by `factorio-bot-planner`
//! and executed by `factorio-bot-executor`.
//!
//! Reachable from one line of user Lua, and this crate builds with
//! `panic = "abort"`, so every panic here is a remote kill of the whole server
//! process rather than a failed script. The lint keeps both argument parsing
//! and planner/executor failures on the `LuaError` path.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::tokio::task::JoinHandle;
use factorio_bot_executor::{run_into, Actuator, ExecutionLog, RconActuator, Status};
use factorio_bot_planner::{
    expand, graphviz, mermaid_gantt, registry_for, schedule, ActionNetwork, BotId, Goal, Holder,
    PlanState, Schedule, Ticks,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// A planner or executor failure is the script's problem, not the process's.
fn goal_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("goal: {err}"))
}

/// Poisoning is not a reason to abort: the only thing a panicking holder of one
/// of these locks can leave behind is a partially-updated registry, and every
/// reader below re-checks what it needs anyway.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One expanded plan, plus its schedule once `goal.schedule` has run.
struct PlanEntry {
    net: Arc<ActionNetwork>,
    /// `None` until scheduled. `goal.gantt` and `goal.execute` both need it and
    /// both say so rather than inventing an empty one.
    schedule: Option<Arc<Schedule>>,
}

/// Expanded plans, keyed by the handle Lua holds.
///
/// A registry rather than userdata for the same reason `Runs` is one: a handle
/// is a plain number, so a script can keep it in a table, pass it around and
/// hand it back on a later call without any Rust lifetime travelling with it.
#[derive(Default)]
struct Plans {
    next: u32,
    plans: BTreeMap<u32, PlanEntry>,
}

impl Plans {
    fn insert(&mut self, net: ActionNetwork) -> u32 {
        let handle = self.next;
        self.next += 1;
        self.plans.insert(
            handle,
            PlanEntry {
                net: Arc::new(net),
                schedule: None,
            },
        );
        handle
    }

    fn get(&self, handle: u32) -> LuaResult<&PlanEntry> {
        self.plans
            .get(&handle)
            .ok_or_else(|| goal_error(format!("no plan with handle {handle}")))
    }

    /// The network and schedule of a plan that has been scheduled.
    fn scheduled(&self, handle: u32) -> LuaResult<(Arc<ActionNetwork>, Arc<Schedule>)> {
        let entry = self.get(handle)?;
        let schedule = entry.schedule.clone().ok_or_else(|| {
            goal_error(format!(
                "plan {handle} has not been scheduled; call goal.schedule(plan, bot_count) first"
            ))
        })?;
        Ok((entry.net.clone(), schedule))
    }
}

/// Live runs, keyed by the handle Lua holds.
///
/// A registry rather than userdata because the shared log outlives any single
/// Lua call and must be readable from `goal.progress` on a later call. Bots
/// keep working while the script does something else — that is the point of
/// the handle-based API.
#[derive(Default)]
struct Runs {
    next: u32,
    runs: BTreeMap<u32, RunEntry>,
}

struct RunEntry {
    progress: Arc<Mutex<ExecutionLog>>,
    /// The network the run is against, so `goal.progress` can count every
    /// action rather than only the ones the log has heard about.
    net: Arc<ActionNetwork>,
    /// Taken by the first `goal.wait`; `None` afterwards, because a
    /// `JoinHandle` can only be awaited once. A second `wait` on a finished run
    /// still answers, from the log.
    join: Option<JoinHandle<()>>,
    /// Set if `run_into` refused to start the run at all. Nothing was executed
    /// in that case, so reporting `done` with everything pending would be a lie.
    error: Arc<Mutex<Option<String>>>,
    /// Set once the run's task has returned.
    ///
    /// `done` cannot be derived from the counts. When a bot's action fails the
    /// executor abandons the rest of that bot's slice, and an abandoned action
    /// is never written to the log at all (`run.rs`'s `abandon_rest` only
    /// publishes on the watch channel) — so a finished, failed run still
    /// reports actions as `pending` forever. `done` means "the run is over",
    /// which is the question a polling script is actually asking.
    finished: Arc<AtomicBool>,
}

impl Runs {
    /// Spawns the run and returns its handle **without awaiting it**.
    ///
    /// Split out from the Lua binding so it can be driven against a stub
    /// `Actuator`: everything above this line needs a live Factorio server,
    /// and everything below it is the part that can be wrong.
    fn spawn(
        &mut self,
        act: Arc<dyn Actuator>,
        sched: Arc<Schedule>,
        net: Arc<ActionNetwork>,
    ) -> u32 {
        let progress = Arc::new(Mutex::new(ExecutionLog::default()));
        let error = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let task_progress = progress.clone();
        let task_net = net.clone();
        let task_error = error.clone();
        let task_finished = finished.clone();
        let join = factorio_bot_core::tokio::spawn(async move {
            if let Err(err) = run_into(&*act, &sched, &task_net, &task_progress).await {
                *lock(&task_error) = Some(err.to_string());
            }
            task_finished.store(true, Ordering::SeqCst);
        });
        let handle = self.next;
        self.next += 1;
        self.runs.insert(
            handle,
            RunEntry {
                progress,
                net,
                join: Some(join),
                error,
                finished,
            },
        );
        handle
    }

    fn get(&self, handle: u32) -> LuaResult<&RunEntry> {
        self.runs
            .get(&handle)
            .ok_or_else(|| goal_error(format!("no run with handle {handle}")))
    }

    fn snapshot(&self, handle: u32) -> LuaResult<Progress> {
        let entry = self.get(handle)?;
        if let Some(err) = lock(&entry.error).as_ref() {
            return Err(goal_error(err));
        }
        Ok(Progress::of(
            &entry.net,
            &lock(&entry.progress),
            entry.finished.load(Ordering::SeqCst),
        ))
    }
}

/// What `goal.progress` and `goal.wait` hand back to Lua.
#[derive(Debug, Default, PartialEq, Eq)]
struct Progress {
    pending: u32,
    running: u32,
    success: u32,
    failed: u32,
    done: bool,
}

impl Progress {
    /// Counted over the **network**, not over the log: an action nothing has
    /// dispatched yet has no log entry at all, and counting only what the log
    /// knows would report one action succeeded out of one, rather than one out
    /// of thirty. The four counts therefore always partition the plan.
    fn of(net: &ActionNetwork, log: &ExecutionLog, finished: bool) -> Progress {
        let mut progress = Progress {
            done: finished,
            ..Progress::default()
        };
        for action in net.actions() {
            match log.status(action.id) {
                Status::Pending => progress.pending += 1,
                Status::Running => progress.running += 1,
                Status::Success => progress.success += 1,
                Status::Failed => progress.failed += 1,
            }
        }
        progress
    }

    fn to_lua_table(&self, lua: &Lua) -> LuaResult<LuaTable> {
        let table = lua.create_table()?;
        table.set("pending", self.pending)?;
        table.set("running", self.running)?;
        table.set("success", self.success)?;
        table.set("failed", self.failed)?;
        table.set("done", self.done)?;
        Ok(table)
    }
}

/// Builds the `goal` table.
///
/// `plan_world` is what goals are planned against (the same hypothetical world
/// `plan.*` and `world.*` see); `real_world` is what the executor drives.
/// `bots` is the run's roster, as player ids.
pub fn create_lua_goal(
    lua: &Lua,
    plan_world: Arc<FactorioWorld>,
    real_world: Arc<FactorioWorld>,
    rcon: Option<Arc<FactorioRcon>>,
    bots: Vec<u8>,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Goals
-- Declarative goals: say what you want, not how to get it. A goal is expanded
-- into an action network by the planner, assigned to bots by the scheduler,
-- and then executed against the running game.
--
-- Every function that takes a `plan` takes the number returned by `goal.have`
-- or `goal.researched`, and every function that takes a `run` takes the number
-- returned by `goal.execute`.
--
-- @module goal

local goal = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return goal"#))?;

    let roster: Vec<BotId> = bots.into_iter().map(BotId).collect();
    let plans: Arc<Mutex<Plans>> = Arc::new(Mutex::new(Plans::default()));
    let runs: Arc<Mutex<Runs>> = Arc::new(Mutex::new(Runs::default()));

    // `goal.have`
    let _plans = plans.clone();
    let world = plan_world.clone();
    let bots = roster.clone();
    map_table.set(
        "__doc_entry_have",
        String::from(
            r#"
--- plans for the bots to end up holding items
-- Expands the goal into an action network. Any bot may contribute: the count
-- is satisfied by the sum across the whole roster, which is what lets the
-- planner split the work.
-- @string item_name name of the item, e.g. "iron-plate"
-- @number count how many are wanted
-- @treturn number a plan handle
function goal.have(item_name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "have",
        lua.create_function(move |_lua, (item_name, count): (String, u32)| {
            let net = expand_goal(
                Goal::Have {
                    item: item_name,
                    count,
                    whose: Holder::Anyone,
                },
                &world,
                &bots,
            )?;
            Ok(lock(&_plans).insert(net))
        })?,
    )?;

    // `goal.researched`
    let _plans = plans.clone();
    let world = plan_world.clone();
    let bots = roster.clone();
    map_table.set(
        "__doc_entry_researched",
        String::from(
            r#"
--- plans for a technology to be researched
-- @string technology_name name of the technology, e.g. "automation"
-- @treturn number a plan handle
function goal.researched(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "researched",
        lua.create_function(move |_lua, technology_name: String| {
            let net = expand_goal(Goal::Researched(technology_name), &world, &bots)?;
            Ok(lock(&_plans).insert(net))
        })?,
    )?;

    // `goal.schedule`
    let _plans = plans.clone();
    let world = plan_world.clone();
    map_table.set(
        "__doc_entry_schedule",
        String::from(
            r#"
--- assigns a plan's actions to bots over time
-- Stores the schedule on the plan handle, which `goal.gantt` and
-- `goal.execute` both need. Scheduling the same plan again replaces it.
-- @number plan_handle handle returned by `goal.have` or `goal.researched`
-- @number bot_count how many bots to spread the work over
-- @treturn number the makespan in ticks
function goal.schedule(plan_handle, bot_count)
end
"#,
        ),
    )?;
    map_table.set(
        "schedule",
        lua.create_function(move |_lua, (plan_handle, bot_count): (u32, u8)| {
            let bots: Vec<BotId> = (1..=bot_count).map(BotId).collect();
            let state = PlanState::from_world(world.clone(), &bots);
            let mut plans = lock(&_plans);
            let net = plans.get(plan_handle)?.net.clone();
            let scheduled = schedule(&net, &state, &bots).map_err(goal_error)?;
            let makespan: Ticks = scheduled.makespan;
            if let Some(entry) = plans.plans.get_mut(&plan_handle) {
                entry.schedule = Some(Arc::new(scheduled));
            }
            Ok(makespan)
        })?,
    )?;

    // `goal.graphviz`
    let _plans = plans.clone();
    map_table.set(
        "__doc_entry_graphviz",
        String::from(
            r#"
--- renders a plan's action network as graphviz source
-- Nodes are coloured by the chain they belong to. Does not need the plan to
-- have been scheduled.
-- @number plan_handle handle returned by `goal.have` or `goal.researched`
-- @treturn string graphviz source
function goal.graphviz(plan_handle)
end
"#,
        ),
    )?;
    map_table.set(
        "graphviz",
        lua.create_function(move |_lua, plan_handle: u32| {
            let plans = lock(&_plans);
            Ok(graphviz(&plans.get(plan_handle)?.net))
        })?,
    )?;

    // `goal.gantt`
    let _plans = plans.clone();
    map_table.set(
        "__doc_entry_gantt",
        String::from(
            r#"
--- renders a scheduled plan as a mermaid gantt chart, one section per bot
-- @number plan_handle handle returned by `goal.have` or `goal.researched`
-- @string title title of the chart
-- @treturn string mermaid gantt source for the scheduled plan
-- @raise error if the plan has not been scheduled
function goal.gantt(plan_handle, title)
end
"#,
        ),
    )?;
    map_table.set(
        "gantt",
        lua.create_function(move |_lua, (plan_handle, title): (u32, String)| {
            let plans = lock(&_plans);
            let (_net, scheduled) = plans.scheduled(plan_handle)?;
            Ok(mermaid_gantt(&scheduled, &title))
        })?,
    )?;

    // `goal.execute`
    let _plans = plans.clone();
    let _runs = runs.clone();
    map_table.set(
        "__doc_entry_execute",
        String::from(
            r#"
--- starts executing a scheduled plan and returns immediately
-- The bots keep working while the script does something else; poll with
-- `goal.progress` or block with `goal.wait`. The run is bound to the script's
-- lifetime, so a script that exits without waiting abandons it.
-- @number plan_handle handle returned by `goal.have` or `goal.researched`
-- @treturn number a run handle
-- @raise error if the plan has not been scheduled, or no game is connected
function goal.execute(plan_handle)
end
"#,
        ),
    )?;
    map_table.set(
        "execute",
        lua.create_async_function(move |_lua, plan_handle: u32| {
            let plans = _plans.clone();
            let runs = _runs.clone();
            let rcon = rcon.clone();
            let real_world = real_world.clone();
            async move {
                let (net, scheduled) = lock(&plans).scheduled(plan_handle)?;
                let rcon = rcon.ok_or_else(|| {
                    goal_error("no rcon connection; goal.execute needs a running game")
                })?;
                // Awaited, not spawned: an actuator that cannot be built at all
                // — no connected players, no reply to the defines query — is a
                // setup error the script should hear about at the call, not a
                // run that silently never happened.
                let actuator = RconActuator::new(rcon, real_world)
                    .await
                    .map_err(goal_error)?;
                Ok(lock(&runs).spawn(Arc::new(actuator), scheduled, net))
            }
        })?,
    )?;

    // `goal.progress`
    let _runs = runs.clone();
    map_table.set(
        "__doc_entry_progress",
        String::from(
            r#"
--- reads a live snapshot of a run
-- Counts every action of the plan, so the four counts always sum to the number
-- of actions in it. `done` means the run is over, which is not the same as
-- every action having an outcome: when a bot's action fails the rest of its
-- work is abandoned and stays `pending`.
-- @number run_handle handle returned by `goal.execute`
-- @treturn table {pending=n, running=n, success=n, failed=n, done=bool}
function goal.progress(run_handle)
end
"#,
        ),
    )?;
    map_table.set(
        "progress",
        lua.create_function(move |lua, run_handle: u32| {
            lock(&_runs).snapshot(run_handle)?.to_lua_table(lua)
        })?,
    )?;

    // `goal.wait`
    let _runs = runs;
    map_table.set(
        "__doc_entry_wait",
        String::from(
            r#"
--- blocks until a run finishes
-- @number run_handle handle returned by `goal.execute`
-- @treturn table the same table as `goal.progress`, with done=true
function goal.wait(run_handle)
end
"#,
        ),
    )?;
    map_table.set(
        "wait",
        lua.create_async_function(move |lua, run_handle: u32| {
            let runs = _runs.clone();
            async move { wait_for_run(&runs, run_handle).await?.to_lua_table(&lua) }
        })?,
    )?;

    Ok(map_table)
}

/// Expands one goal against the run's roster.
///
/// The roster is the bots the script was started with — the same set `plan.*`
/// addresses — because `SplitAcrossBots` needs to know who exists before it can
/// split anything. `goal.schedule`'s `bot_count` chooses how many bots the
/// resulting actions are *assigned* to, which is a later and separate decision.
fn expand_goal(goal: Goal, world: &Arc<FactorioWorld>, bots: &[BotId]) -> LuaResult<ActionNetwork> {
    let chain_actor = *bots
        .first()
        .ok_or_else(|| goal_error("no bots in this run; goals need at least one"))?;
    let state = PlanState::from_world(world.clone(), bots);
    expand(&[goal], &state, &registry_for(bots), chain_actor).map_err(goal_error)
}

/// Awaits a run's task, then reports on it.
///
/// The `JoinHandle` is taken out of the registry before the await rather than
/// held across it: the guard is a plain `std::sync::Mutex`, and holding one
/// across an await point is what turns a second `goal.progress` on another run
/// into a deadlock.
async fn wait_for_run(runs: &Mutex<Runs>, handle: u32) -> LuaResult<Progress> {
    let join = {
        let mut guard = lock(runs);
        // Fail now if the handle is unknown, rather than after the await.
        guard.get(handle)?;
        guard
            .runs
            .get_mut(&handle)
            .and_then(|entry| entry.join.take())
    };
    if let Some(join) = join {
        // A panicking run is already recorded in the log; joining it is not the
        // place to re-raise, and aborting the process here would be worse.
        let _ = join.await;
    }
    lock(runs).snapshot(handle)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use factorio_bot_executor::ActuatorError;
    use factorio_bot_planner::InventorySlot;
    use std::time::Duration;

    /// An actuator that takes a measurable amount of time and then reports the
    /// same verdict for everything.
    ///
    /// The delay is what makes "returned without waiting" observable: if
    /// `Runs::spawn` ran the schedule to completion before returning, the very
    /// next `snapshot` would already show the run finished.
    struct StubActuator {
        delay: Duration,
        /// Applies to the action methods only. Walking always succeeds: a bot
        /// whose *walk* fails has the rest of its slice abandoned without ever
        /// being dispatched, so every action would stay `Pending` and the
        /// failure counting this stub exists to exercise would never run.
        fails: bool,
    }

    impl StubActuator {
        async fn act(&self) -> Result<(), ActuatorError> {
            factorio_bot_core::tokio::time::sleep(self.delay).await;
            if self.fails {
                return Err(ActuatorError::Rejected("stub refuses".to_string()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl Actuator for StubActuator {
        async fn walk(&self, _bot: BotId, _to: Position) -> Result<(), ActuatorError> {
            factorio_bot_core::tokio::time::sleep(self.delay).await;
            Ok(())
        }
        async fn mine(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.act().await
        }
        async fn craft(
            &self,
            _bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.act().await
        }
        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<(), ActuatorError> {
            self.act().await
        }
        async fn insert(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.act().await
        }
        async fn remove(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.act().await
        }
        async fn research(&self, _tech: &str) -> Result<(), ActuatorError> {
            self.act().await
        }
    }

    /// A small real plan: mining iron ore with two bots. Real rather than
    /// hand-built so the counts below are counts of something the planner
    /// actually produces, and multi-action so "the counts partition the plan"
    /// is not the same statement as "one action has one status".
    fn fixture_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
        let bots = [BotId(1), BotId(2)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 20,
                whose: Holder::Anyone,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("the fixture world can be mined");
        assert!(
            net.len() > 1,
            "the tests below want a plan with more than one action, got {}",
            net.len()
        );
        let scheduled = schedule(&net, &state, &bots).expect("schedulable");
        (Arc::new(net), Arc::new(scheduled))
    }

    fn stub(delay_ms: u64, fails: bool) -> Arc<dyn Actuator> {
        Arc::new(StubActuator {
            delay: Duration::from_millis(delay_ms),
            fails,
        })
    }

    #[tokio::test]
    async fn execute_returns_a_handle_without_waiting_for_the_run() {
        let (net, scheduled) = fixture_plan();
        let total = net.len() as u32;
        // Without this the two count assertions below would both hold
        // vacuously on an empty plan.
        assert!(total > 0, "the fixture plan must contain actions");

        let runs = Mutex::new(Runs::default());
        let handle = lock(&runs).spawn(stub(150, false), scheduled, net);

        // Nothing has been dispatched yet, so every action is still pending.
        // A blocking `spawn` would have every action `success` here instead.
        let snapshot = lock(&runs).snapshot(handle).expect("known handle");
        assert_eq!(
            snapshot,
            Progress {
                pending: total,
                running: 0,
                success: 0,
                failed: 0,
                done: false,
            },
            "goal.execute must return before the run has made progress"
        );
    }

    #[tokio::test]
    async fn wait_returns_a_done_snapshot_counting_every_action_as_succeeded() {
        let (net, scheduled) = fixture_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let runs = Mutex::new(Runs::default());
        let handle = lock(&runs).spawn(stub(0, false), scheduled, net);

        let snapshot = wait_for_run(&runs, handle).await.expect("known handle");
        assert!(
            snapshot.done,
            "wait must return a finished run: {snapshot:?}"
        );
        assert_eq!(
            snapshot.success, total,
            "every action of the plan succeeded: {snapshot:?}"
        );
        assert_eq!(
            snapshot.pending + snapshot.running + snapshot.success + snapshot.failed,
            total,
            "the counts must partition the plan's actions: {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn a_run_whose_actions_are_rejected_finishes_with_failures_not_successes() {
        // The counterpart that keeps the test above honest: with only the
        // success path exercised, `Progress::of` could map every status to
        // `success` and both would still pass.
        let (net, scheduled) = fixture_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let runs = Mutex::new(Runs::default());
        let handle = lock(&runs).spawn(stub(0, true), scheduled, net);

        let snapshot = wait_for_run(&runs, handle).await.expect("known handle");
        assert!(snapshot.done, "a failed run is still a finished run");
        assert_eq!(snapshot.success, 0, "nothing succeeded: {snapshot:?}");
        assert_eq!(
            snapshot.failed, total,
            "every action was dispatched and rejected: {snapshot:?}"
        );
        assert_eq!(
            snapshot.pending + snapshot.running + snapshot.success + snapshot.failed,
            total,
            "the counts must partition the plan's actions: {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn waiting_twice_answers_twice() {
        // The `JoinHandle` is taken by the first wait; a second one must still
        // report rather than error or hang.
        let (net, scheduled) = fixture_plan();
        let runs = Mutex::new(Runs::default());
        let handle = lock(&runs).spawn(stub(0, false), scheduled, net);

        let first = wait_for_run(&runs, handle).await.expect("known handle");
        // Without this the equality below would hold on two empty snapshots.
        assert!(
            first.done && first.success > 0,
            "the first wait must report a finished run: {first:?}"
        );
        let second = wait_for_run(&runs, handle).await.expect("known handle");
        assert_eq!(first, second, "a second wait must report the same run");
    }

    #[tokio::test]
    async fn an_unknown_run_handle_is_an_error_not_a_panic() {
        let runs = Mutex::new(Runs::default());
        assert!(lock(&runs).snapshot(7).is_err());
        assert!(wait_for_run(&runs, 7).await.is_err());
    }

    /// Runs `tests/goal_script.lua` through the real interpreter, which is the
    /// only thing that proves the table is actually installed as a global and
    /// that every binding survives the sandbox.
    ///
    /// The sandbox root is a temp directory: the script touches no files, and
    /// rooting it in the repository would put it beside the write-only
    /// artifacts `test_script` regenerates.
    #[tokio::test]
    async fn the_goal_script_fixture_runs() {
        use factorio_bot_core::plan::planner::Planner;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let mut planner = Planner::new(Arc::new(fixture_world()), None);
        crate::lua_runner::run_lua(
            &mut planner,
            include_str!("../../tests/goal_script.lua"),
            None,
            &root,
            4,
            false,
        )
        .await
        .expect("goal_script.lua failed");
    }
}
