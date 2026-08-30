//! The `goal.*` Lua table: declarative goals planned by `factorio-bot-planner`
//! and executed by `factorio-bot-executor`.
//!
//! Reachable from one line of user Lua, and this crate builds with
//! `panic = "abort"`, so every panic here is a remote kill of the whole server
//! process rather than a failed script. The lint keeps both argument parsing
//! and planner/executor failures on the `LuaError` path.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod value;

use crate::lua_runner::PendingWork;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::tokio::sync::watch;
use factorio_bot_core::tokio::task::JoinHandle;
use factorio_bot_executor::{run_into, Actuator, ExecutionLog, RconActuator, Status};
use factorio_bot_planner::{
    expand, graphviz, mermaid_gantt, registry_for, schedule, ActionNetwork, BotId, Goal, Holder,
    PlanState, Schedule, Ticks,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// How a run gets its actuator.
///
/// A factory rather than an `Arc<dyn Actuator>` because building one talks to
/// the game: `RconActuator::new` asks who is connected and reads
/// `defines.inventory`. It is also the seam the tests need — everything above
/// this line needs a live Factorio server, everything below it is this module's
/// own logic, and without the seam `goal.execute` itself can only be tested by
/// reaching around it into `Runs::spawn`, which pins the helper and not the
/// binding a script actually calls.
type ActuatorFactory = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<Arc<dyn Actuator>, String>> + Send>>
        + Send
        + Sync,
>;

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
    /// The goal the network was expanded from, kept so that `goal.schedule` can
    /// expand it again when it is asked for a roster the network was not built
    /// for. See `roster`.
    goal: Goal,
    /// The bots the network was expanded over.
    ///
    /// A network is only schedulable on the roster it was expanded for.
    /// `SplitAcrossBots` hands each bot a share sized against *that bot's own
    /// inventory* — with a roster of four seeded players, each holding one
    /// stone furnace, the four smelting chains each place the furnace their own
    /// bot already carries. Assigning all four to one bot then asks that bot for
    /// four furnaces it never had, and the plan dies on the first `Place` whose
    /// turn came last: "precondition has 1 stone-furnace of action ActionId(0)
    /// does not hold for bot 1". Nothing in the network records who it was
    /// split for, so this does.
    roster: Vec<BotId>,
    /// `None` until scheduled. `goal.gantt` and `goal.execute` both need it and
    /// both say so rather than inventing an empty one.
    schedule: Option<Arc<Schedule>>,
    /// The run handle `goal.execute` produced for this plan, once it has been
    /// executed.
    ///
    /// Set the moment a run is actually spawned, so a second `goal.execute`
    /// on the same handle can be refused instead of silently dispatching
    /// every action again — against a live game that means placing an entity
    /// twice, inserting twice, mining a tile that is already gone. Left
    /// `None` if the actuator itself could not be built (no connected
    /// players, no reply to the defines query): nothing was dispatched in
    /// that case, so a retry after fixing the connection is a legitimate
    /// first execution, not a re-run.
    run: Option<u32>,
}

/// Expanded plans, keyed by the handle Lua holds.
///
/// A registry rather than userdata for the same reason `Runs` is one: a handle
/// is a plain number, so a script can keep it in a table, pass it around and
/// hand it back on a later call without any Rust lifetime travelling with it.
///
/// # Entries are kept, and that is the choice
///
/// Nothing is ever removed. A handle stays valid for as long as the script that
/// made it is running, because the alternative — freeing a plan when it has been
/// scheduled, or a run when it has been waited on — makes the obvious script
/// wrong: `goal.wait(r)` followed by `goal.progress(r)` to read the final counts
/// is the natural way to write it, and freeing on `wait` turns that into an
/// error about an unknown handle.
///
/// The growth bound is one entry per `goal.have`, `goal.researched` and
/// `goal.execute` call **of a single script run**, not of the server's lifetime:
/// `create_lua_goal` is called once per `run_lua`, both registries are owned by
/// the `Arc`s captured in that run's Lua closures, and they are dropped with the
/// interpreter when the script ends. A script that leaks here is a script that
/// loops forever calling the planner, which is already growing far more than a
/// `BTreeMap` entry per iteration.
#[derive(Default)]
struct Plans {
    /// The next handle to hand out. `saturating_add`, so a script cannot wrap it
    /// back to 0 and alias a handle it is still holding; at the ceiling new
    /// plans replace the most recent one instead, which takes four billion calls
    /// in one script to reach.
    next: u32,
    plans: BTreeMap<u32, PlanEntry>,
}

impl Plans {
    fn insert(&mut self, net: ActionNetwork, goal: Goal, roster: Vec<BotId>) -> u32 {
        let handle = self.next;
        self.next = self.next.saturating_add(1);
        self.plans.insert(
            handle,
            PlanEntry {
                net: Arc::new(net),
                goal,
                roster,
                schedule: None,
                run: None,
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

    /// Records that `handle` has been dispatched as `run`, so a later
    /// `goal.execute` on the same handle can be refused.
    fn mark_executed(&mut self, handle: u32, run: u32) {
        if let Some(entry) = self.plans.get_mut(&handle) {
            entry.run = Some(run);
        }
    }
}

/// Live runs, keyed by the handle Lua holds.
///
/// A registry rather than userdata because the shared log outlives any single
/// Lua call and must be readable from `goal.progress` on a later call. Bots
/// keep working while the script does something else — that is the point of
/// the handle-based API.
///
/// Entries are kept for the life of the script run, for the reasons on [`Plans`].
#[derive(Default)]
struct Runs {
    /// See [`Plans::next`]: saturating, not wrapping.
    next: u32,
    runs: BTreeMap<u32, RunEntry>,
}

struct RunEntry {
    progress: Arc<Mutex<ExecutionLog>>,
    /// The network the run is against, so `goal.progress` can count every
    /// action rather than only the ones the log has heard about.
    net: Arc<ActionNetwork>,
    /// Flips to `true` once the run's task returns. A `watch::Receiver`
    /// rather than the task's own `JoinHandle`, because the `JoinHandle`
    /// itself is handed to `goal.execute`'s caller for registration into
    /// [`PendingWork`] — a `JoinHandle` can only be awaited by one owner, but
    /// `goal.wait`/`goal.progress` need to observe completion independently
    /// of that, and possibly more than once (a second `wait` on a finished
    /// run still answers).
    finished_rx: watch::Receiver<bool>,
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
    /// Spawns the run and returns its handle **without awaiting it**, plus the
    /// task's own `JoinHandle` so the caller can register it into
    /// [`PendingWork`].
    ///
    /// Split out from the Lua binding so it can be driven against a stub
    /// `Actuator`: everything above this line needs a live Factorio server,
    /// and everything below it is the part that can be wrong.
    fn spawn(
        &mut self,
        act: Arc<dyn Actuator>,
        sched: Arc<Schedule>,
        net: Arc<ActionNetwork>,
    ) -> (u32, JoinHandle<()>) {
        let progress = Arc::new(Mutex::new(ExecutionLog::default()));
        let error = Arc::new(Mutex::new(None));
        let finished = Arc::new(AtomicBool::new(false));
        let (finished_tx, finished_rx) = watch::channel(false);
        let task_progress = progress.clone();
        let task_net = net.clone();
        let task_error = error.clone();
        let task_finished = finished.clone();
        let join = factorio_bot_core::tokio::spawn(async move {
            if let Err(err) = run_into(&*act, &sched, &task_net, &task_progress).await {
                *lock(&task_error) = Some(err.to_string());
            }
            task_finished.store(true, Ordering::SeqCst);
            // No receiver is an ordinary outcome, not a failure: it just means
            // nothing (goal.wait, PendingWork's drain) is waiting on this run.
            let _ = finished_tx.send(true);
        });
        let handle = self.next;
        self.next = self.next.saturating_add(1);
        self.runs.insert(
            handle,
            RunEntry {
                progress,
                net,
                finished_rx,
                error,
                finished,
            },
        );
        (handle, join)
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
/// `world.*` sees); `real_world` is what the executor drives.
/// `bots` is the run's roster, as player ids.
pub fn create_lua_goal(
    lua: &Lua,
    plan_world: Arc<FactorioWorld>,
    real_world: Arc<FactorioWorld>,
    rcon: Option<Arc<FactorioRcon>>,
    bots: Vec<u8>,
) -> LuaResult<LuaTable> {
    let actuator: ActuatorFactory = Arc::new(move || {
        let rcon = rcon.clone();
        let world = real_world.clone();
        Box::pin(async move {
            let rcon = rcon.ok_or_else(|| {
                "no rcon connection; goal.execute needs a running game".to_string()
            })?;
            RconActuator::new(rcon, world)
                .await
                .map(|a| Arc::new(a) as Arc<dyn Actuator>)
                .map_err(|err| err.to_string())
        })
    });
    create_lua_goal_with(lua, plan_world, actuator, bots)
}

/// [`create_lua_goal`] with the actuator supplied rather than built from RCON.
///
/// The only caller in production is `create_lua_goal`; the tests use it to drive
/// the real bindings — `goal.execute` included — against a stub.
pub(crate) fn create_lua_goal_with(
    lua: &Lua,
    plan_world: Arc<FactorioWorld>,
    actuator: ActuatorFactory,
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
            let goal = Goal::Have {
                item: item_name,
                count,
                whose: Holder::Anyone,
            };
            let net = expand_goal(goal.clone(), &world, &bots)?;
            Ok(lock(&_plans).insert(net, goal, bots.clone()))
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
--
-- Decomposes into the technology's prerequisites, researched first and
-- recursively, then the science packs it costs -- its per-unit ingredients
-- times its unit count, produced by the same methods `goal.have` uses -- then
-- the research itself. A technology the force has already researched, or that
-- an earlier goal in the same plan already researched, costs nothing and adds
-- no actions.
--
-- The handle it returns is an ordinary plan handle: pass it to
-- `goal.schedule`, `goal.graphviz`, `goal.gantt` and `goal.execute` like any
-- other.
-- @string technology_name name of the technology, e.g. "automation"
-- @treturn number a plan handle
-- @raise if no force in this world defines `technology_name`, if the packs it
--   costs cannot be produced in this world, or if its prerequisites cycle
function goal.researched(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "researched",
        lua.create_function(move |_lua, technology_name: String| {
            let goal = Goal::Researched(technology_name);
            let net = expand_goal(goal.clone(), &world, &bots)?;
            Ok(lock(&_plans).insert(net, goal, bots.clone()))
        })?,
    )?;

    // `goal.schedule`
    let _plans = plans.clone();
    let world = plan_world.clone();
    let bots = roster.clone();
    map_table.set(
        "__doc_entry_schedule",
        String::from(
            r#"
--- assigns a plan's actions to bots over time
-- Stores the schedule on the plan handle, which `goal.gantt` and
-- `goal.execute` both need. Scheduling the same plan again replaces it.
-- @number plan_handle handle returned by `goal.have` or `goal.researched`
-- @number bot_count how many of the run's bots to spread the work over; they
--   are taken from the front of the run's roster, and asking for more bots than
--   the run has is an error rather than an invented bot
--
-- Scheduling over fewer bots than the run has re-expands the goal for exactly
-- those bots first: a plan split across four bots spends four bots' starting
-- items, and cannot be run by one of them. The plan handle then names the
-- re-expanded network, so `goal.graphviz`, `goal.gantt` and `goal.execute` all
-- describe the plan that was actually scheduled.
-- @treturn number the makespan in ticks
function goal.schedule(plan_handle, bot_count)
end
"#,
        ),
    )?;
    map_table.set(
        "schedule",
        lua.create_function(move |_lua, (plan_handle, bot_count): (u32, u8)| {
            // Derived from the run's roster, never counted from one: a `BotId`
            // is a Factorio player id, and the run's players are whatever
            // `initiate_missing_players_with_default_inventory` handed us. A
            // literal `(1..=bot_count)` here would be a second, independent
            // claim about who exists, which is precisely the mismatch that had
            // the executor driving the wrong player.
            let bots: Vec<BotId> = bots.iter().copied().take(bot_count as usize).collect();
            if bots.len() < bot_count as usize {
                return Err(goal_error(format!(
                    "this run has {} bot(s); cannot schedule over {bot_count}",
                    bots.len()
                )));
            }
            let state = PlanState::from_world(world.clone(), &bots);
            refuse_unknown_bots(&state)?;
            let mut plans = lock(&_plans);
            let entry = plans.get(plan_handle)?;
            // A network is only schedulable on the roster it was expanded for
            // (see `PlanEntry::roster`). When the caller asks for fewer bots
            // than `goal.have` split over, the plan they hold was built to
            // spend items that the bots they named do not have, so the goal is
            // expanded again for the bots that will actually run it. Same
            // roster, same network: the common call re-expands nothing and the
            // makespans it produces are unchanged.
            let re_expand = entry.roster != bots;
            let net = if re_expand {
                Arc::new(expand_goal(entry.goal.clone(), &world, &bots)?)
            } else {
                entry.net.clone()
            };
            let scheduled = schedule(&net, &state, &bots).map_err(goal_error)?;
            let makespan: Ticks = scheduled.makespan;
            if let Some(entry) = plans.plans.get_mut(&plan_handle) {
                if re_expand {
                    // The schedule names action ids, so the network it names
                    // has to be the one the handle carries from here on —
                    // `goal.execute` looks both up through the same handle.
                    entry.net = net;
                    entry.roster = bots;
                }
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
-- `goal.progress` or block with `goal.wait`. "Returns immediately" is about
-- this call, not about the script as a whole: a run that is still going when
-- the script ends is not abandoned. The script's *own* end blocks until every
-- run it started this way has finished, so a script that never calls
-- `goal.wait` still has its bots run to completion -- it just finds out how
-- they went one call later than a script that waited would.
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
        lua.create_async_function(move |lua, plan_handle: u32| {
            let plans = _plans.clone();
            let runs = _runs.clone();
            let actuator = actuator.clone();
            async move {
                let (net, scheduled) = {
                    let guard = lock(&plans);
                    // Refused before anything else, including the schedule
                    // lookup below: a plan that already has a run is not
                    // something a second `goal.execute` should even parse as
                    // "not yet scheduled" if scheduling were somehow undone --
                    // the run having happened is the more specific fact.
                    // Re-dispatching a plan's actions against a live game
                    // means placing an entity twice, inserting twice, mining a
                    // tile that is already gone, so this project's standing
                    // preference (fail loudly on ambiguity rather than
                    // silently proceed) points at refusing outright, not at
                    // guessing that the caller wanted the first run's handle
                    // back or a genuine restart. A script that means to run
                    // the same goal again builds and schedules a fresh plan
                    // handle for it, which is unambiguous.
                    if let Some(run) = guard.get(plan_handle)?.run {
                        return Err(goal_error(format!(
                            "plan {plan_handle} was already executed as run {run}; \
                             goal.execute refuses to dispatch its actions a second time \
                             -- build and schedule a new plan if you mean to run it again"
                        )));
                    }
                    guard.scheduled(plan_handle)?
                };
                // Checked before anything is spawned, not after: registration
                // is what stops a fire-and-forget `goal.execute` from being
                // killed mid-plan when `run_lua` drops its tokio runtime (see
                // `PendingWork` in `lua_runner.rs`). A caller that omits
                // `set_app_data` — today, only a binding built outside
                // `run_lua`, e.g. a future doc-generation or REPL path that
                // forgets it — must not be able to lose a run silently; the
                // loud failure has to come before the run exists, or an
                // unregistered run is started and orphaned regardless of what
                // this returns. This module's own tests build the table this
                // way on purpose (to test the bindings below the game seam)
                // and install `PendingWork` themselves when they mean to
                // exercise `goal.execute`.
                let pending = lua.app_data_ref::<PendingWork>().ok_or_else(|| {
                    goal_error(
                        "goal.execute: no PendingWork registered for this Lua state; \
                         refusing to start a run that could be silently killed when \
                         the interpreter's runtime is dropped (internal error, not a \
                         script bug)",
                    )
                })?;
                let pending = pending.clone();
                // Building the actuator is awaited; running the schedule is not.
                // An actuator that cannot be built at all — no connected
                // players, no reply to the defines query — is a setup error the
                // script should hear about at the call, not a run that silently
                // never happened.
                let actuator = actuator().await.map_err(goal_error)?;
                let (handle, join) = lock(&runs).spawn(actuator, scheduled, net);
                // Recorded only now that a run genuinely exists: an actuator
                // that failed to build above dispatched nothing, so that path
                // must not be remembered as an execution a retry could be
                // refused against.
                lock(&plans).mark_executed(plan_handle, handle);
                pending.register(join);
                Ok(handle)
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
-- @raise if the handle is unknown, or if the run refused to start at all — a
-- run whose schedule implied a circular wait raises here on every call, since
-- there is no progress to report on something that never began
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
-- `done` is true on return, but the counts may still show pending actions: a
-- bot that hits a failure abandons the rest of its work without dispatching it.
-- @number run_handle handle returned by `goal.execute`
-- @treturn table the same table as `goal.progress`, with done=true
-- @raise on the same conditions as `goal.progress`
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

/// Refuses to plan or schedule against a bot `PlanState::from_world` had to
/// fabricate.
///
/// `PlanState::from_world` hands any roster id the world has no player for a
/// `BotState::default()`: an empty inventory and guessed reach distances
/// (`build_distance`/`reach_distance` 10.0, `resource_reach_distance` 3.0).
/// Planning against that is not harmless — it schedules cleanly and then
/// fails at execution against limits that were never real. Naming the bot
/// here, before either the planner or the scheduler ever sees the fabricated
/// state, turns that into an error a script can act on immediately.
///
/// This cannot fire on the ordinary path: `run_lua` always calls
/// `Planner::initiate_missing_players_with_default_inventory` for every id in
/// the run's roster before a `PlanState` is ever built from that world (see
/// `lua_runner.rs`), so every id `goal.have`/`goal.researched`/`goal.schedule`
/// pass to `PlanState::from_world` already has a real player and
/// `unknown_bots()` comes back empty. It only fires when a roster names a
/// player the world has never heard of at all, which is exactly the
/// fabrication this closes.
fn refuse_unknown_bots(state: &PlanState) -> LuaResult<()> {
    let unknown = state.unknown_bots();
    if unknown.is_empty() {
        return Ok(());
    }
    let names = unknown
        .iter()
        .map(|bot| bot.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(goal_error(format!(
        "bot(s) {names} are not connected players in this world; refusing to plan \
         against a fabricated inventory and guessed reach distances"
    )))
}

/// Expands one goal against a roster.
///
/// `SplitAcrossBots` needs to know who exists before it can split anything, so
/// `goal.have` expands against the bots the script was started with. That is a
/// default, **not** a separate decision from assignment: the split sizes each
/// share against the holdings of the bot it names, so the network it produces
/// only makes sense on that same roster. When `goal.schedule` is given fewer
/// bots it calls this again for those bots, rather than assigning a four-bot
/// plan to one bot and failing on a precondition about a furnace three other
/// bots were carrying.
fn expand_goal(goal: Goal, world: &Arc<FactorioWorld>, bots: &[BotId]) -> LuaResult<ActionNetwork> {
    let chain_actor = *bots
        .first()
        .ok_or_else(|| goal_error("no bots in this run; goals need at least one"))?;
    let state = PlanState::from_world(world.clone(), bots);
    refuse_unknown_bots(&state)?;
    // No rewriting of the planner's own errors. There used to be one here,
    // because `registry_for` held no method for `Goal::Researched` and every
    // research goal came back as `NoApplicableMethod` — "no method can satisfy
    // goal: research automation" — which reads as "that technology is
    // unreachable in this world" for what was really an unbuilt feature. The
    // feature is built (`method::have::Researched`), and the planner now
    // distinguishes the cases itself: a technology no force defines comes back
    // as `UnknownTechnology`, naming it.
    expand(
        std::slice::from_ref(&goal),
        &state,
        &registry_for(bots),
        chain_actor,
    )
    .map_err(goal_error)
}

/// Awaits a run's task, then reports on it.
///
/// The `watch::Receiver` is cloned out of the registry before the await
/// rather than held across it: the guard is a plain `std::sync::Mutex`, and
/// holding one across an await point is what turns a second `goal.progress`
/// on another run into a deadlock. Cloning (rather than taking, the way the
/// run's `JoinHandle` itself is taken exactly once by `PendingWork`) is what
/// lets a second `wait` on the same handle answer instead of hanging: unlike
/// a `JoinHandle`, a `watch::Receiver` can be awaited by any number of
/// independent clones.
///
/// An unknown handle needs no check of its own: it yields no receiver, so
/// nothing is awaited, and the `snapshot` at the end is what reports it. An
/// earlier version guarded the lookup twice; the redundant guard was removed
/// after a mutation of it changed no observable behaviour, which is the only
/// honest verdict available on code that cannot be made to matter.
async fn wait_for_run(runs: &Mutex<Runs>, handle: u32) -> LuaResult<Progress> {
    let finished_rx = lock(runs)
        .get(handle)
        .ok()
        .map(|entry| entry.finished_rx.clone());
    if let Some(mut finished_rx) = finished_rx {
        // A panicking run is already recorded in the log; observing its end
        // here is not the place to re-raise, and aborting the process here
        // would be worse. A dropped sender (the task ended without ever
        // sending `true`) reports as an error we also ignore, for the same
        // reason.
        let _ = finished_rx.wait_for(|done| *done).await;
    }
    lock(runs).snapshot(handle)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::tokio::sync::{mpsc, watch};
    use factorio_bot_core::types::Position;
    use factorio_bot_executor::ActuatorError;
    use factorio_bot_planner::InventorySlot;
    use std::time::Duration;

    /// When the stub refuses an action.
    enum Failure {
        Never,
        Always,
        /// Only the first action dispatched anywhere. That is what produces an
        /// *abandoned* tail: one bot stops at its first step while the others
        /// finish theirs, so the run ends with actions that were never
        /// dispatched and so never reached the log at all.
        First(AtomicBool),
    }

    /// An actuator that never touches a game.
    ///
    /// `walk` always succeeds and is never gated: a bot whose *walk* fails has
    /// the rest of its slice abandoned before a single action is dispatched, so
    /// everything would stay `Pending` and the counting these tests exist to
    /// exercise would never run.
    struct StubActuator {
        delay: Duration,
        fails: Failure,
        /// Signalled as each action is dispatched, so a test can observe a run
        /// mid-flight without sleeping and hoping.
        entered: Option<mpsc::UnboundedSender<()>>,
        /// Actions block here until the test sets it to `true`. A gate that is
        /// never opened is how "did `goal.execute` return without waiting?"
        /// becomes a question with a definite answer.
        gate: Option<watch::Receiver<bool>>,
    }

    impl StubActuator {
        fn new(fails: Failure) -> Self {
            StubActuator {
                delay: Duration::ZERO,
                fails,
                entered: None,
                gate: None,
            }
        }

        async fn act(&self) -> Result<(), ActuatorError> {
            if let Some(entered) = &self.entered {
                let _ = entered.send(());
            }
            if let Some(gate) = &self.gate {
                let mut gate = gate.clone();
                while !*gate.borrow_and_update() {
                    if gate.changed().await.is_err() {
                        break;
                    }
                }
            }
            if !self.delay.is_zero() {
                factorio_bot_core::tokio::time::sleep(self.delay).await;
            }
            let refuse = match &self.fails {
                Failure::Never => false,
                Failure::Always => true,
                Failure::First(spent) => !spent.swap(true, Ordering::SeqCst),
            };
            if refuse {
                return Err(ActuatorError::Rejected("stub refuses".to_string()));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl Actuator for StubActuator {
        async fn walk(&self, _bot: BotId, _to: Position) -> Result<(), ActuatorError> {
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

    /// Wraps a stub as the factory `create_lua_goal_with` takes.
    fn factory(stub: Arc<dyn Actuator>) -> ActuatorFactory {
        Arc::new(move || {
            let stub = stub.clone();
            Box::pin(async move { Ok(stub) })
        })
    }

    /// Two bots mining iron ore: one action each, so no bot has a tail to
    /// abandon. Small on purpose, for the tests that only need *an* action.
    fn mining_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
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
            "the tests below want more than one action, got {}",
            net.len()
        );
        let scheduled = schedule(&net, &state, &bots).expect("schedulable");
        (Arc::new(net), Arc::new(scheduled))
    }

    /// Ten red science across four bots: the plan from
    /// `planner/tests/red_science.rs`, and long enough per bot that a bot which
    /// fails its first action leaves a genuine abandoned tail behind it.
    ///
    /// `mining_plan` cannot do this. One action per bot means the failure *is*
    /// the whole slice, nothing is left to abandon, and every action still ends
    /// up with a status — which is exactly why `done` derived from the counts
    /// survived against it.
    fn science_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        for bot in bots {
            state.gain(bot, "stone-furnace", 2);
        }
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 10,
                whose: Holder::Anyone,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("ten red science expands");
        assert!(
            net.len() > 10,
            "this plan must be long enough for a bot to have a tail to abandon, got {}",
            net.len()
        );
        let scheduled = schedule(&net, &state, &bots).expect("schedulable");
        (Arc::new(net), Arc::new(scheduled))
    }

    // ---------------------------------------------------------------- counting

    // `start_paused` because the executor honours the schedule's lag edges in
    // real wall-clock time (`run.rs:329`), and red science smelts: the run takes
    // ~29 real seconds otherwise. Auto-advance is safe here precisely because
    // this test has no gate — every task is either working or waiting on a
    // timer, so the clock only jumps when nothing can make progress.
    #[tokio::test(start_paused = true)]
    async fn a_run_that_abandons_work_is_done_while_actions_are_still_pending() {
        // The test for the central design decision: `done` is a flag set when
        // the run's task returns, and it CANNOT be derived from the counts.
        //
        // When a bot's action fails, `run.rs`'s `abandon_rest` releases the
        // waiters on the watch channel and writes nothing to the log, so the
        // rest of that bot's slice is never dispatched and stays `Pending`
        // forever. A finished run therefore legitimately reports pending work,
        // and `pending == 0 && running == 0` would call it unfinished for good.
        let (net, scheduled) = science_plan();
        let total = net.len() as u32;
        let runs = Mutex::new(Runs::default());
        let (handle, _join) = lock(&runs).spawn(
            Arc::new(StubActuator::new(Failure::First(AtomicBool::new(false)))),
            scheduled,
            net,
        );

        let snapshot = wait_for_run(&runs, handle).await.expect("known handle");
        assert!(
            snapshot.done,
            "the run's task has returned, so it is done: {snapshot:?}"
        );
        assert_eq!(snapshot.failed, 1, "one action was refused: {snapshot:?}");
        assert!(
            snapshot.pending > 0,
            "the failing bot's remaining slice was abandoned undispatched, so it must \
             still count as pending -- this is the case that makes `done` a flag \
             rather than `pending == 0`: {snapshot:?}"
        );
        assert!(
            snapshot.success > 0,
            "the other bots carried on: {snapshot:?}"
        );
        assert_eq!(
            snapshot.pending + snapshot.running + snapshot.success + snapshot.failed,
            total,
            "the counts must partition the plan's actions: {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn an_action_in_flight_counts_as_running_not_as_pending() {
        // `Running` is a status of its own, and folding it into `pending` would
        // make a live run indistinguishable from one that has not started.
        // Observed by signal rather than by sleeping: the stub reports each
        // dispatch and then blocks on a gate the test never opens.
        let (net, scheduled) = mining_plan();
        let total = net.len() as u32;
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let (_gate_tx, gate_rx) = watch::channel(false);
        let stub = StubActuator {
            entered: Some(entered_tx),
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };

        let runs = Mutex::new(Runs::default());
        let (handle, _join) = lock(&runs).spawn(Arc::new(stub), scheduled, net);

        entered_rx.recv().await.expect("an action was dispatched");
        let snapshot = lock(&runs).snapshot(handle).expect("known handle");
        assert!(
            snapshot.running > 0,
            "a dispatched action is running, not pending: {snapshot:?}"
        );
        assert_eq!(snapshot.success, 0, "the gate is shut: {snapshot:?}");
        assert!(!snapshot.done, "the run is still going: {snapshot:?}");
        assert_eq!(
            snapshot.pending + snapshot.running + snapshot.success + snapshot.failed,
            total,
            "the counts must partition the plan's actions: {snapshot:?}"
        );
    }

    #[tokio::test]
    async fn wait_returns_a_done_snapshot_counting_every_action_as_succeeded() {
        let (net, scheduled) = mining_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let runs = Mutex::new(Runs::default());
        let (handle, _join) =
            lock(&runs).spawn(Arc::new(StubActuator::new(Failure::Never)), scheduled, net);

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
        let (net, scheduled) = mining_plan();
        let total = net.len() as u32;
        assert!(total > 0, "the fixture plan must contain actions");

        let runs = Mutex::new(Runs::default());
        let (handle, _join) =
            lock(&runs).spawn(Arc::new(StubActuator::new(Failure::Always)), scheduled, net);

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
        // The completion signal is a cloned `watch::Receiver`, not a taken
        // `JoinHandle`; a second wait must still report rather than error or
        // hang.
        let (net, scheduled) = mining_plan();
        let runs = Mutex::new(Runs::default());
        let (handle, _join) =
            lock(&runs).spawn(Arc::new(StubActuator::new(Failure::Never)), scheduled, net);

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

    // ------------------------------------------------------------- the bindings

    /// A world whose players are seeded exactly the way a run seeds them, for
    /// an arbitrary roster of player ids.
    ///
    /// `Planner::initiate_missing_players_with_default_inventory` (used by
    /// `seeded_world` below) only ever seeds `1..=bot_count`, so it cannot
    /// stand in for a roster that deliberately names ids outside that range
    /// (`every_bot_the_bindings_dispatch_to_is_a_player_the_actuator_can_drive`'s
    /// `[3, 4]` case exists precisely to do that). This gives each id in
    /// `roster` the same inventory production seeding gives, directly through
    /// the same `FactorioWorld` event the real seeding uses, so
    /// `PlanState::unknown_bots()` comes back empty for it — the property
    /// `goal.have`, `goal.researched` and `goal.schedule` now require of every
    /// bot they are asked to plan for. A bare `fixture_world()` never seeds
    /// any player at all, so every bot named against it is "unknown" by that
    /// same definition; tests that exercise the bindings above the refusal
    /// need this instead.
    fn seeded_world_for(roster: &[u8]) -> Arc<FactorioWorld> {
        let world = fixture_world();
        seed_players(&world, roster);
        Arc::new(world)
    }

    /// Gives each id in `roster` the same inventory production seeding
    /// gives, directly on an already-built `world` -- the piece
    /// `seeded_world_for` cannot offer on its own when a test also needs to
    /// set up something else on the same world first (e.g. a research
    /// force), since `fixture_world()` cannot be seeded twice into two
    /// different `FactorioWorld` values and then merged.
    fn seed_players(world: &FactorioWorld, roster: &[u8]) {
        use factorio_bot_core::types::{EntityName, PlayerChangedMainInventoryEvent};

        for &player_id in roster {
            let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
            main_inventory.insert(EntityName::Wood.to_string(), 1);
            main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
            main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
            world
                .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                    player_id,
                    main_inventory,
                ))
                .expect("seed player");
        }
    }

    /// Installs the real `goal` table, backed by `stub`, into a sandboxed
    /// interpreter — the same one user scripts get.
    ///
    /// Installs a `PendingWork`, as `run_lua` always does in production,
    /// so that `goal.execute` is free to run: most of the tests below are
    /// about what happens *after* a run starts, not about the app-data
    /// check itself. The one test for that check
    /// (`execute_without_pending_work_installed_fails_loudly_instead_of_silently_losing_the_run`)
    /// builds its own `Lua` without this helper so it can leave `PendingWork`
    /// out on purpose.
    ///
    /// Seeded via `seeded_world_for`, not a bare `fixture_world()`: these
    /// tests are about run/progress/execute mechanics, not about the unknown-
    /// bot refusal, so bots 1 and 2 need to be real players or `goal.have`
    /// refuses before any of that mechanics is ever reached.
    fn lua_with_goal(stub: Arc<dyn Actuator>) -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table =
            create_lua_goal_with(&lua, seeded_world_for(&[1, 2]), factory(stub), vec![1, 2])
                .expect("goal table");
        lua.globals().set("goal", table).expect("install");
        lua
    }

    /// Runs `code`, failing rather than hanging if it does not finish.
    ///
    /// The timeout is the point. A `goal.execute` that waited for its run would
    /// block here forever behind the shut gate, and a test that hangs on
    /// regression is not a guard — it reads as a slow suite. This turns it into
    /// a named assertion failure.
    async fn exec_bounded(lua: &Lua, code: &str) {
        let outcome = factorio_bot_core::tokio::time::timeout(
            Duration::from_secs(10),
            lua.load(code).exec_async(),
        )
        .await;
        match outcome {
            Err(_) => panic!("the script did not finish within 10s: goal.execute blocked"),
            Ok(result) => result.expect("the script failed"),
        }
    }

    #[tokio::test]
    async fn the_execute_binding_returns_a_run_handle_without_waiting_for_the_run() {
        // Driven through the binding a script actually calls, not through
        // `Runs::spawn` underneath it: a `goal.execute` rewritten as
        // spawn-then-await passes every test that reaches around it.
        //
        // The gate is never opened, so nothing this run dispatches can finish.
        // A binding that waits therefore cannot return at all, and `exec_bounded`
        // reports that as a failure.
        let (_gate_tx, gate_rx) = watch::channel(false);
        let stub = StubActuator {
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };
        let lua = lua_with_goal(Arc::new(stub));

        exec_bounded(
            &lua,
            r#"
            local p = goal.have("iron-ore", 20)
            goal.schedule(p, 2)
            local r = goal.execute(p)
            local s = goal.progress(r)
            result = {
                pending = s.pending, running = s.running,
                success = s.success, failed = s.failed, done = s.done,
            }
            "#,
        )
        .await;

        let result: LuaTable = lua.globals().get("result").expect("result");
        let done: bool = result.get("done").expect("done");
        let success: u32 = result.get("success").expect("success");
        let failed: u32 = result.get("failed").expect("failed");
        let pending: u32 = result.get("pending").expect("pending");
        let running: u32 = result.get("running").expect("running");
        assert!(
            !done,
            "goal.execute must return before the run has finished"
        );
        assert_eq!(
            success + failed,
            0,
            "the gate is shut: nothing can complete"
        );
        assert!(
            pending + running > 0,
            "the plan's actions must all still be outstanding"
        );
    }

    /// `lua_with_goal` builds the table exactly as this module's own tests
    /// want it -- without installing `PendingWork` -- to drive the bindings
    /// below the game seam. `goal.execute` must not read that as "nothing to
    /// register into, carry on anyway": in production only `run_lua` installs
    /// `PendingWork`, and a future caller that forgets to must not be able to
    /// start a run nothing will keep alive when its runtime is dropped.
    ///
    /// Proven to discriminate by the assertion at the end: not just that the
    /// call errors, but that nothing was ever dispatched -- i.e. no run was
    /// started at all, not merely a run whose handle came back unusable.
    #[tokio::test]
    async fn execute_without_pending_work_installed_fails_loudly_instead_of_silently_losing_the_run(
    ) {
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let stub = StubActuator {
            entered: Some(entered_tx),
            ..StubActuator::new(Failure::Never)
        };
        // Built directly rather than through `lua_with_goal`: that helper
        // installs `PendingWork` the way `run_lua` does, and this test is
        // specifically about the caller that forgets to. Seeded via
        // `seeded_world_for`, not a bare `fixture_world()`, so `goal.have`
        // reaches the `PendingWork` check this test is about instead of
        // refusing earlier for bots 1 and 2 being unknown.
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(Arc::new(stub)),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(
                r#"
                local p = goal.have("iron-ore", 20)
                goal.schedule(p, 2)
                goal.execute(p)
                "#,
            )
            .exec_async()
            .await
            .expect_err("goal.execute must refuse to run without PendingWork installed");
        let message = err.to_string();
        assert!(
            message.contains("PendingWork"),
            "the error should name what is missing: {message}"
        );

        let dispatched =
            factorio_bot_core::tokio::time::timeout(Duration::from_millis(200), entered_rx.recv())
                .await;
        assert!(
            dispatched.is_err(),
            "no action should ever have been dispatched: goal.execute must refuse \
             before a run is spawned, not spawn one it then fails to register"
        );
    }

    /// The regression this whole change closes: a script that calls
    /// `goal.execute` and never calls `goal.wait` must still have its run
    /// finish, because `goal.execute` registers the run's task into
    /// [`PendingWork`] — the seam `run_lua` drains before dropping the
    /// runtime that owns it. A test that also called `goal.wait` would prove
    /// nothing about this: the wait would carry the run to completion on its
    /// own regardless of whether `goal.execute` registered anything.
    ///
    /// This drives `create_lua_goal_with`'s real binding under a real
    /// sandboxed interpreter (`lua_with_goal` / `exec_bounded`, the same
    /// harness the test above uses) and installs a real `PendingWork` as
    /// `run_lua` does, rather than reaching around the binding into
    /// `Runs::spawn`. The gate keeps every dispatched action blocked, so a
    /// `pending.drain()` that returned before the gate opened could only mean
    /// the run's `JoinHandle` was never actually registered — proving the
    /// drain is genuinely awaiting the registered task, not merely that it
    /// returns eventually.
    #[tokio::test]
    async fn a_fire_and_forget_execute_registers_its_run_so_it_still_finishes() {
        let (gate_tx, gate_rx) = watch::channel(false);
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
        let stub = StubActuator {
            entered: Some(entered_tx),
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };
        let lua = lua_with_goal(Arc::new(stub));
        let pending = crate::lua_runner::PendingWork::default();
        lua.set_app_data(pending.clone());

        // No `goal.wait` anywhere in this script -- `r` is left global so the
        // test can still read progress on it afterwards.
        exec_bounded(
            &lua,
            r#"
            local p = goal.have("iron-ore", 20)
            goal.schedule(p, 2)
            r = goal.execute(p)
            "#,
        )
        .await;

        // The chunk has already returned. Confirm the run has actually
        // started (and is now blocked on the shut gate) before reasoning
        // about whether draining waits for it.
        entered_rx.recv().await.expect("an action was dispatched");
        let progress: LuaTable = lua
            .load("return goal.progress(r)")
            .eval()
            .expect("progress");
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
            "pending.drain() returned while the run was still gated -- goal.execute \
             did not register a handle for it to await"
        );

        gate_tx.send(true).expect("gate has a receiver");
        let failures = factorio_bot_core::tokio::time::timeout(Duration::from_secs(5), drain_task)
            .await
            .expect("drain did not finish after the gate opened")
            .expect("the drain task itself panicked");
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");

        let progress: LuaTable = lua
            .load("return goal.progress(r)")
            .eval()
            .expect("progress");
        let done: bool = progress.get("done").expect("done");
        let success: u32 = progress.get("success").expect("success");
        assert!(done, "the run must have finished once drain() returned");
        assert!(success > 0, "the run must have actually executed something");
    }

    #[tokio::test]
    async fn the_progress_and_wait_bindings_agree_on_a_finished_run() {
        // The other half of the binding: with the gate open, `goal.wait` really
        // does block until the run is over and `goal.progress` afterwards
        // reports the same thing.
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local p = goal.have("iron-ore", 20)
            goal.schedule(p, 2)
            local r = goal.execute(p)
            local w = goal.wait(r)
            local s = goal.progress(r)
            result = { waited = w.success, polled = s.success, done = w.done and s.done }
            "#,
        )
        .await;

        let result: LuaTable = lua.globals().get("result").expect("result");
        let waited: u32 = result.get("waited").expect("waited");
        let polled: u32 = result.get("polled").expect("polled");
        let done: bool = result.get("done").expect("done");
        assert!(done, "goal.wait must return a finished run");
        assert!(waited > 0, "the run must have executed something");
        assert_eq!(waited, polled, "a poll after a wait reports the same run");
    }

    // --------------------------------------------- the scheduler/executor seam

    /// Records the bot named by every command it is asked to perform.
    #[derive(Default)]
    struct RecordingActuator {
        bots: std::sync::Mutex<Vec<BotId>>,
    }

    impl RecordingActuator {
        fn note(&self, bot: BotId) -> Result<(), ActuatorError> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().push(bot);
            Ok(())
        }
        fn recorded(&self) -> Vec<BotId> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Actuator for RecordingActuator {
        async fn walk(&self, bot: BotId, _to: Position) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn mine(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn craft(&self, bot: BotId, _recipe: &str, _count: u32) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn place(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn insert(
            &self,
            bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn remove(
            &self,
            bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<(), ActuatorError> {
            self.note(bot)
        }
        async fn research(&self, _tech: &str) -> Result<(), ActuatorError> {
            Ok(())
        }
    }

    /// The seam between the half of the stack that *assigns* work and the half
    /// that *performs* it.
    ///
    /// When this was broken both halves were internally consistent: the
    /// schedule numbered bots `1..=n` and the actuator numbered them `0..n-1`,
    /// and each had a passing test against its own convention. So this test
    /// states neither. It gives the bindings a roster of player ids — the same
    /// thing `Planner::initiate_missing_players_with_default_inventory` returns
    /// to `run_lua` — drives the real `goal.*` bindings all the way through
    /// `goal.execute`, and then asks the *executor's own* resolution function
    /// whether each bot it was handed names a player that is in the game.
    ///
    /// The `[3, 4]` roster is the discriminating one, and it fails against
    /// either side's old convention: a schedule built from `1..=bot_count`
    /// dispatches to players 1 and 2, who are not in the game, and an actuator
    /// that renumbers the connected players `0..n-1` has no bot 3 or 4.
    #[tokio::test]
    async fn every_bot_the_bindings_dispatch_to_is_a_player_the_actuator_can_drive() {
        use factorio_bot_core::types::PlayerId;
        use std::collections::BTreeSet;

        for roster in [vec![1u8], vec![1, 2], vec![3, 4], vec![1, 2, 3, 4]] {
            let rec = Arc::new(RecordingActuator::default());
            let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
            lua.set_app_data(crate::lua_runner::PendingWork::default());
            // Seeded via `seeded_world_for`, not a bare `fixture_world()`:
            // `[3, 4]` is exactly the roster `seeded_world` (which only ever
            // seeds `1..=bot_count`) cannot produce, and this test's whole
            // point is that roster, so every id in it needs to be a real
            // player or `goal.have` refuses before the seam below is ever
            // exercised.
            let table = create_lua_goal_with(
                &lua,
                seeded_world_for(&roster),
                factory(rec.clone()),
                roster.clone(),
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            exec_bounded(
                &lua,
                &format!(
                    r#"
                    local p = goal.have("iron-ore", 20)
                    goal.schedule(p, {})
                    goal.wait(goal.execute(p))
                    "#,
                    roster.len()
                ),
            )
            .await;

            let dispatched = rec.recorded();
            assert!(
                !dispatched.is_empty(),
                "roster {roster:?}: nothing was dispatched, so the loop below would \
                 hold for any numbering at all"
            );
            // The game has exactly the run's players in it — that is what the
            // roster means. `RconActuator::new` builds this same set from
            // `connected_players()`.
            let connected: BTreeSet<PlayerId> = roster.iter().copied().collect();
            for bot in &dispatched {
                let player = RconActuator::resolve_player(&connected, *bot)
                    .unwrap_or_else(|err| panic!("roster {roster:?}: {err}"));
                assert!(
                    roster.contains(&player),
                    "roster {roster:?}: {bot} drove player {player}, who is not in this run"
                );
            }
        }
    }

    /// A world whose players are seeded exactly the way a run seeds them.
    ///
    /// `Planner::initiate_missing_players_with_default_inventory` gives each
    /// bot **one** stone furnace, one burner mining drill and one piece of
    /// wood, and `lua_runner` calls it and then `update_plan_world` before the
    /// bindings ever see the world. Handing every bot everything the plan needs
    /// is what let this bug live under 400-odd green tests, so the seeding is
    /// taken from the production call rather than written out here, and the
    /// assertion below states the property the test depends on.
    fn seeded_world(bot_count: u8) -> Arc<FactorioWorld> {
        use factorio_bot_core::plan::planner::Planner;

        let mut planner = Planner::new(Arc::new(fixture_world()), None);
        let roster = planner.initiate_missing_players_with_default_inventory(bot_count);
        planner.update_plan_world();
        let world = planner.world();
        for id in roster {
            let player = world.players.get(&id).expect("the run seeded this player");
            assert_eq!(
                player.main_inventory.get("stone-furnace").copied(),
                Some(1),
                "bot {id} must hold exactly one furnace, or this test cannot reach the bug"
            );
        }
        world
    }

    /// A plan split across the run's roster is scheduled over fewer bots.
    ///
    /// `goal.have` expands against every bot the run has, and `SplitAcrossBots`
    /// gives each of them a chain that spends *its own* starting stone furnace.
    /// `goal.schedule(p, 1)` then asks one bot to run all of them, which asks
    /// that bot for as many furnaces as the roster had between them. Live, with
    /// `scripts/goal_smoke.lua`, that was:
    ///
    /// ```text
    /// goal: precondition has 1 stone-furnace of action ActionId(0) does not hold for bot 1
    /// ```
    ///
    /// identically for `--bots 2` and `--bots 4`, while `--bots 1` — the only
    /// roster where expansion and assignment agree — succeeded.
    ///
    /// Rosters of 2 and 4 are both here because the split's arithmetic differs
    /// between them: a shortfall of five over two bots is 3 + 2, over four bots
    /// 2 + 1 + 1 + 1, and only the second lets a share of one reach the chain
    /// that a share of one is supposed to open.
    #[tokio::test]
    async fn a_plan_split_across_the_roster_schedules_over_fewer_bots() {
        for bot_count in [1u8, 2, 4] {
            let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
            lua.set_app_data(crate::lua_runner::PendingWork::default());
            let table = create_lua_goal_with(
                &lua,
                seeded_world(bot_count),
                factory(Arc::new(StubActuator::new(Failure::Never))),
                (1..=bot_count).collect(),
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            // The smoke script's own shape: expand over the whole run, then
            // schedule over one bot, then render what was scheduled.
            exec_bounded(
                &lua,
                r#"
                local p = goal.have("iron-plate", 5)
                local makespan = goal.schedule(p, 1)
                assert(makespan > 0, "a real plan takes a positive number of ticks")
                assert(#goal.gantt(p, "smoke") > 0, "the gantt must describe what was scheduled")
                "#,
            )
            .await;
        }
    }

    /// One force with one technology, added on top of the shared fixture
    /// world.
    ///
    /// `fixture_world` carries no forces, and technologies live on forces, so
    /// `goal.researched` has nothing to plan against it. The fixture is shared
    /// with the planner's own pinned makespan tests and must not grow a force
    /// of its own, so this adds one here, for this test only. The numbers are
    /// the game's own for `automation`: 10 units of one automation science
    /// pack each, 600 ticks per unit.
    const RESEARCH_FORCE_JSON: &str = r#"
    {
      "name": "player",
      "force_id": 1,
      "current_research": null,
      "research_progress": null,
      "technologies": {
        "automation": {
          "name": "automation",
          "enabled": true,
          "upgrade": false,
          "researched": false,
          "prerequisites": [],
          "research_unit_ingredients": [
            { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
          ],
          "research_unit_count": 10,
          "research_unit_energy": 600.0,
          "order": "a-a",
          "level": 1,
          "valid": true
        }
      }
    }
    "#;

    /// Runs `tests/goal_script.lua` through the real interpreter and the real
    /// `run_lua` harness, which is the only thing that proves the table is
    /// installed as a global under a live sandbox.
    ///
    /// The sandbox root is a temp directory: the script touches no files, and
    /// rooting it in the repository would put it beside the write-only
    /// artifacts `test_script` regenerates.
    #[tokio::test]
    async fn the_goal_script_fixture_runs() {
        use factorio_bot_core::plan::planner::Planner;
        use factorio_bot_core::types::FactorioForce;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = fixture_world();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        let mut planner = Planner::new(Arc::new(world), None);
        crate::lua_runner::run_lua(
            &mut planner,
            include_str!("../../../tests/goal_script.lua"),
            None,
            &root,
            4,
            None,
        )
        .await
        .expect("goal_script.lua failed");
    }

    // ---------------------------------------------- the unknown-bot refusal

    /// `goal.have` refuses a roster naming a bot the world has no player for,
    /// rather than silently planning it with `BotState::default()`'s empty
    /// inventory and guessed reach distances.
    ///
    /// Bot 1 is seeded and bot 2 is not, so this discriminates from a check
    /// that only ever looks at "is the roster empty" or similar: a roster
    /// with one real bot in it must still be refused for the other, unnamed
    /// one, and the error must name it.
    #[tokio::test]
    async fn goal_have_refuses_a_roster_naming_a_bot_the_world_does_not_have() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1]),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.have("iron-ore", 20)"#)
            .eval_async::<u32>()
            .await
            .expect_err("bot 2 is not a player in this world");
        let message = err.to_string();
        // Not just "does the call error": a fabricated bot's empty inventory
        // also trips unrelated planner errors (e.g. a mismatched starting
        // inventory across bots) that happen to name "bot 2" too, so the
        // assertion has to be on this refusal's own wording, not merely on
        // the bot id appearing somewhere in the message.
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the unknown bot: {message}"
        );
    }

    /// `goal.researched` goes through the same `expand_goal` path as
    /// `goal.have`, so the same refusal must fire for it, against a world
    /// that actually has a technology to research (otherwise the planner's
    /// own `UnknownTechnology` error would fire first and this would prove
    /// nothing about the unknown-bot check specifically).
    #[tokio::test]
    async fn goal_researched_refuses_a_roster_naming_a_bot_the_world_does_not_have() {
        use factorio_bot_core::types::FactorioForce;

        let world = fixture_world();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        // Bot 1 only; the roster below also names bot 2, which this world
        // never seeds.
        seed_players(&world, &[1]);
        let world = Arc::new(world);

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world,
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.researched("automation")"#)
            .eval_async::<u32>()
            .await
            .expect_err("bot 2 is not a player in this world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the unknown bot: {message}"
        );
    }

    /// `goal.schedule` has its own `PlanState::from_world` call, separate
    /// from the one `goal.have` used to expand the plan, and must consult
    /// `unknown_bots()` there too rather than trusting that `goal.have`
    /// already checked.
    ///
    /// Proven to exercise `goal.schedule`'s own check, not a leftover from
    /// `goal.have`'s: both bots are real players when the plan is made (so
    /// `goal.have` succeeds), and bot 2 is only removed from the world
    /// afterwards -- the same `FactorioWorld` the run's `goal` table already
    /// closed over, so `goal.schedule`'s later `PlanState::from_world` call
    /// sees the world as it is *now*, not as it was when the plan was
    /// expanded.
    #[tokio::test]
    async fn goal_schedule_refuses_when_a_bot_becomes_unknown_after_the_plan_was_made() {
        let world = seeded_world_for(&[1, 2]);
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world.clone(),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let p: u32 = lua
            .load(r#"return goal.have("iron-ore", 20)"#)
            .eval_async()
            .await
            .expect("both bots are real players; goal.have must succeed");

        world.remove_player(2).expect("remove_player");

        lua.globals().set("p", p).expect("set p");
        let err = lua
            .load("return goal.schedule(p, 2)")
            .eval_async::<u32>()
            .await
            .expect_err("bot 2 was just removed from the world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the now-unknown bot: {message}"
        );
    }

    // ------------------------------------------- refusing a second execute

    /// The regression this closes: calling `goal.execute` twice on the same
    /// plan handle used to spawn a second, independent run against the same
    /// schedule, dispatching every action again -- against a live game that
    /// means placing an entity twice, inserting twice, mining an
    /// already-mined tile. A plan that already produced a run must refuse a
    /// second one instead.
    ///
    /// Proven to discriminate on dispatch, not just on the error: `rec`
    /// records every command any run performs, so if the refused second
    /// `goal.execute` had actually spawned a run anyway, the count taken
    /// after it would be higher than the count taken after the first
    /// `goal.wait`.
    #[tokio::test]
    async fn a_second_execute_on_the_same_plan_is_refused_and_dispatches_nothing_again() {
        let rec = Arc::new(RecordingActuator::default());
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(rec.clone()),
            vec![1, 2],
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(
            &lua,
            r#"
            p = goal.have("iron-ore", 20)
            goal.schedule(p, 2)
            r1 = goal.execute(p)
            goal.wait(r1)
            "#,
        )
        .await;
        let after_first = rec.recorded().len();
        assert!(
            after_first > 0,
            "the first run must have dispatched something"
        );

        exec_bounded(
            &lua,
            r#"
            local ok, err = pcall(goal.execute, p)
            result = { ok = ok, err = tostring(err) }
            "#,
        )
        .await;
        let result: LuaTable = lua.globals().get("result").expect("result");
        let ok: bool = result.get("ok").expect("ok");
        let err: String = result.get("err").expect("err");
        assert!(!ok, "a second execute on the same plan must be refused");
        assert!(
            err.contains("already executed"),
            "the error should say the plan already ran: {err}"
        );
        assert_eq!(
            rec.recorded().len(),
            after_first,
            "the refused second execute must not have dispatched anything"
        );
    }
}
