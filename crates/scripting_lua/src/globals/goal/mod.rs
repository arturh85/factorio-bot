//! The `goal.*` Lua table: declarative goals planned by `factorio-bot-planner`
//! and executed by `factorio-bot-executor`.
//!
//! Everything a script holds here is a **value**, never a handle: a goal is a
//! plain Lua table (`goal/value.rs`), a plan is a [`plan::PlanValue`] and a
//! run is a [`run::RunValue`]. The integer-handle registries this module used
//! to keep — `Plans`, `Runs` and the `goal.schedule`/`goal.execute`/
//! `goal.progress`/`goal.wait` bindings that indexed them — are gone: a value
//! can be inspected, printed and passed around, and the rules a registry used
//! to police (a plan may be run once) now live on the value itself.
//!
//! Reachable from one line of user Lua, and this crate builds with
//! `panic = "abort"`, so every panic here is a remote kill of the whole server
//! process rather than a failed script. The lint keeps both argument parsing
//! and planner/executor failures on the `LuaError` path.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod plan;
mod recovery;
mod run;
mod value;

use factorio_bot_core::factorio::rcon::{FactorioRcon, PlacementQuery, PlacementVerdict};
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_executor::{Actuator, RconActuator};
use factorio_bot_planner::{ActionNetwork, BotId, Goal, PlanState, expand, holds, registry_for};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

/// How a run gets its actuator.
///
/// A factory rather than an `Arc<dyn Actuator>` because building one talks to
/// the game: `RconActuator::new` asks who is connected and reads
/// `defines.inventory`. It is also the seam the tests need — everything above
/// this line needs a live Factorio server, everything below it is this module's
/// own logic, and without the seam `goal.start`/`goal.run` themselves can only
/// be tested by reaching around them into `run::spawn`, which pins the helper
/// and not the binding a script actually calls.
type ActuatorFactory = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<Arc<dyn Actuator>, String>> + Send>>
        + Send
        + Sync,
>;

/// How `goal.plan` asks the game whether the sites it just chose are legal.
///
/// # Why this is a separate seam from [`ActuatorFactory`]
///
/// The actuator belongs to a *run*: building one queries the game for the
/// connected roster and `defines.inventory`, and there is no run yet when a
/// plan is being expanded. This is a plain question about the map that needs
/// neither, and giving it its own seam is what lets `goal.plan` stay callable
/// with no run in sight.
///
/// # Why the planner's purity survives this
///
/// It is not called from expansion, and expansion cannot reach it. The
/// sequence is: expand (pure) -> schedule (pure) -> **ask** -> write what the
/// game said into the world's refusal ledger -> expand again (pure, and now
/// reading a world with one more fact in it). Every expansion is still a pure
/// function of a world snapshot and a roster, which is exactly what
/// `expansion_is_deterministic` pins; what changed between two of them is the
/// world, and a changing world is what `PlanState::from_world` is for.
///
/// `None` — no game, or a build with no RCON — means no pre-check, and a plan
/// is returned exactly as it always was. That default is deliberate: an
/// absent checker must never be able to look like a green answer.
pub(crate) type PlacementChecker = Arc<
    dyn Fn(
            Vec<PlacementQuery>,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<PlacementVerdict>, String>> + Send>>
        + Send
        + Sync,
>;

/// A planner or executor failure is the script's problem, not the process's.
fn goal_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("goal: {err}"))
}

/// Poisoning is not a reason to abort: the only thing a panicking holder of one
/// of these locks can leave behind is a partially-updated log, and every reader
/// below re-checks what it needs anyway.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Builds the `goal` table.
///
/// `plan_world` is what goals are planned against; `real_world` is what the
/// executor drives. They are now the same world -- see [`Planner`]'s docs: the
/// hypothetical `plan_world` once held was a deep copy that no surviving code
/// wrote to, and that froze every `world.*` read at the run's start state. The
/// two parameters remain only so the split can be reintroduced deliberately if
/// a simulation surface is ever wanted again.
///
/// [`Planner`]: factorio_bot_core::plan::planner::Planner
/// `bots` is the run's roster, as player ids.
pub fn create_lua_goal(
    lua: &Lua,
    plan_world: Arc<FactorioWorld>,
    real_world: Arc<FactorioWorld>,
    rcon: Option<Arc<FactorioRcon>>,
    bots: Vec<u8>,
) -> LuaResult<LuaTable> {
    // Cloned before the actuator factory takes ownership of `rcon`: the
    // pre-check and the actuator both need it and neither owns the other.
    let probe_rcon = rcon.clone();
    let actuator: ActuatorFactory = Arc::new(move || {
        let rcon = rcon.clone();
        let world = real_world.clone();
        Box::pin(async move {
            let rcon = rcon
                .ok_or_else(|| "no rcon connection; goal.run needs a running game".to_string())?;
            RconActuator::new(rcon, world)
                .await
                .map(|a| Arc::new(a) as Arc<dyn Actuator>)
                .map_err(|err| err.to_string())
        })
    });
    // Captures `plan_world`, not `real_world`: what this writes is read back
    // by `PlanState::from_world`, which is given the planning world. They are
    // the same object today (see this function's own doc comment) and the
    // pre-check would still work if they were separated -- but it would be
    // writing an observation about the ground into a world nobody plans
    // against, which is the one way this could go quietly useless.
    let checker: Option<PlacementChecker> = probe_rcon.map(|rcon| {
        let world = plan_world.clone();
        Arc::new(move |queries: Vec<PlacementQuery>| {
            let rcon = rcon.clone();
            let world = world.clone();
            Box::pin(async move {
                rcon.can_place_entities(&world, &queries)
                    .await
                    .map_err(|err| err.to_string())
            })
                as Pin<Box<dyn Future<Output = Result<Vec<PlacementVerdict>, String>> + Send>>
        }) as PlacementChecker
    });
    create_lua_goal_with(lua, plan_world, actuator, bots, checker)
}

/// [`create_lua_goal`] with the actuator supplied rather than built from RCON.
///
/// The only caller in production is `create_lua_goal`; the tests use it to drive
/// the real bindings — `goal.start`/`goal.run` included — against a stub.
pub(crate) fn create_lua_goal_with(
    lua: &Lua,
    plan_world: Arc<FactorioWorld>,
    actuator: ActuatorFactory,
    bots: Vec<u8>,
    placement_checker: Option<PlacementChecker>,
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
-- Nothing here is a handle. `goal.have`, `goal.researched` and `goal.all`
-- build **goal values**: ordinary Lua tables you can read (`g.item`,
-- `g.count`), print and pass around. `goal.plan` turns one into a
-- **PlanValue**, which carries the schedule it was given and answers questions
-- about it (`plan.makespan`, `plan.bots`, `plan.steps`, `plan:count{...}`,
-- `plan:find{...}`, `plan:for_bot(id)`, `plan:graphviz()`,
-- `plan:gantt(title)`). `goal.start` and `goal.run` execute a plan and hand
-- back a **RunValue** / an observation table rather than a number to look up
-- later. A plan may be executed at most once.
--
-- A run that did not finish its plan is not the end of it: `obs:recover()`
-- proposes the next plan to run -- or `nil`, when the work is done or nothing
-- mechanical is left -- so a script can write its own retry loop, with its own
-- budget, out of the same `goal.run` it already uses. See `goal.run`.
--
-- @module goal

local goal = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return goal"#))?;

    let roster: Vec<BotId> = bots.into_iter().map(BotId).collect();

    // `goal.have` / `goal.researched` / `goal.all`: the goal-value
    // constructors. Pure — they touch neither the world nor the planner, so
    // an unknown item is not an error here; it is one at `goal.plan`, which
    // is the first call that has a world to check it against.
    map_table.set(
        "__doc_entry_have",
        String::from(
            r#"
--- builds a goal value: the bots end up holding items
-- Pure: nothing is planned, expanded or checked against the world here, so an
-- item name no recipe produces raises at `goal.plan`, not at this call. The
-- table it returns is an ordinary Lua table (`{ kind = "have", item = ...,
-- count = ..., bot = ... }`) that you can inspect and `tostring`.
--
-- Any bot may contribute by default: the count is satisfied by the sum across
-- the whole roster, which is what lets the planner split the work. Pass
-- `{ bot = id }` to demand one named bot hold them instead.
-- @string item_name name of the item, e.g. "iron-plate"
-- @number count how many are wanted; an integer >= 1
-- @tparam[opt] table opts `{ bot = id }` to pin the goal to one bot
-- @treturn table a goal value
-- @raise if the item name is empty, or the count is not an integer >= 1
function goal.have(item_name, count, opts)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_researched",
        String::from(
            r#"
--- builds a goal value: a technology is researched
--
-- Pure, like `goal.have`: a technology no force defines raises at
-- `goal.plan`, which is the first call with a world to check it against.
--
-- Planning it decomposes into the technology's prerequisites, researched
-- first and recursively, then the science packs it costs -- its per-unit
-- ingredients times its unit count, produced by the same methods `goal.have`
-- uses -- then the research itself. A technology the force has already
-- researched, or that an earlier goal in the same plan already researched,
-- costs nothing and adds no actions.
-- @string technology_name name of the technology, e.g. "automation"
-- @treturn table a goal value
-- @raise if the technology name is empty
function goal.researched(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_all",
        String::from(
            r#"
--- builds a goal value: every sub-goal, in one plan
-- The sub-goals are planned together rather than one after another, so work
-- shared between them is done once.
-- @tparam table goals a list of goal values
-- @treturn table a goal value
-- @raise if the list is empty, or holds anything that is not a goal value
function goal.all(goals)
end
"#,
        ),
    )?;
    value::install_goal_constructors(lua, &map_table)?;

    // `goal.plan`
    map_table.set(
        "__doc_entry_plan",
        String::from(
            r#"
--- expands a goal value and schedules it against one roster, in one call
-- Consumes a goal value: a table with a `kind` field, such as
-- `{ kind = "have", item = "iron-plate", count = 8 }`, built by `goal.have`,
-- `goal.researched` or `goal.all`. Expansion and scheduling always share the
-- same roster -- `SplitAcrossBots` sizes each bot's share against that bot's
-- own holdings, so a network expanded for four bots only ever makes sense
-- scheduled on those same four; this call is what makes the mismatch
-- unrepresentable.
-- @tparam table goal a goal value
-- @tparam[opt] table opts `{ bots = { ... } }` -- bot ids, not a count;
--   defaults to every bot in this run
--
-- Every entry of `plan.steps` carries `bot`, `start`, `finish` and `kind`.
-- `finish`, never `end`: `end` is a Lua keyword, so `step.end` does not parse.
-- A `kind == "walk"` step also carries `to` -- a `types.Position` -- and
-- `radius`, the tolerance that walk's own precondition asked for. `radius` is
-- what tells "stand on this tile" from "stand near it", which for a place,
-- insert or remove is the difference between a reachable request and the
-- entity's own tile; a walk that only has to get close carries a non-zero one.
-- Both `radius` and the other scalar fields are accepted by `plan:count{...}`
-- and `plan:find{...}`; `to` and `pos` are not, being tables rather than
-- comparable values.
--
-- **This asks the game before it hands the plan back.** Once the sites are
-- chosen, one RCON call puts every placement in the plan to
-- `surface.can_place_entity`; any the game would refuse is remembered for the
-- rest of the run (`record.refusals()` writes them out) and the goal is
-- expanded again around them, up to twice. So a plan that would have died on
-- `can_place_entity said 'no'` mid-run is re-sited before anything is
-- dispatched, at a cost of one round trip for a plan with nothing wrong with
-- it and none at all for a plan that places nothing.
--
-- A green answer is **not a guarantee**. It is what the game said at the tick
-- it was asked, and the plan runs afterwards: a bot can walk into the
-- footprint before the build happens. A build can still be refused at
-- dispatch, exactly as before.
-- @treturn PlanValue the expanded, scheduled plan
-- @raise if the goal names an unknown item or technology, if any bot in
--   `opts.bots` is not a connected player, or if `opts.bots` is empty
function goal.plan(goal, opts)
end
"#,
        ),
    )?;
    plan::install_goal_plan(
        lua,
        &map_table,
        plan_world.clone(),
        roster.clone(),
        placement_checker,
    )?;

    // `goal.holds`
    map_table.set(
        "__doc_entry_holds",
        String::from(
            r#"
--- answers whether a goal already holds, without planning it
-- The question satisfaction really is. A loop that runs a goal to completion
-- has to decide when it is done, and until this existed the only signal was
-- "the plan came back empty" -- which is a fact about the planner, not about
-- the goal. The two agree today, and `goal.plan` is still what you call to
-- find out *how* to get there; this is how you find out whether you already
-- are.
--
-- Reads the same world snapshot `goal.plan` does, for the same roster, so a
-- `have` goal with no `bot` is satisfied by the sum across the roster and one
-- with a `bot` is satisfied only by that bot's own inventory.
--
-- Three answers, not two. `nil` means the goal names something possession
-- cannot settle -- a production, which is an event and not a state -- so a
-- caller must not read it as "no": treating `nil` as unfinished re-runs work
-- that may be done, and treating it as finished is the lie this call exists
-- to prevent. No constructor on this table builds such a goal today, so today
-- every answer is a boolean; the third case is what a caller must not have
-- assumed away by the time one does.
-- @tparam table goal a goal value
-- @tparam[opt] table opts `{ bots = { ... } }` -- bot ids, not a count;
--   defaults to every bot in this run
-- @treturn boolean|nil true, false, or nil when the goal cannot be answered
--   by looking at the world
-- @raise if any bot in `opts.bots` is not a connected player, or if
--   `opts.bots` is empty
function goal.holds(goal, opts)
end
"#,
        ),
    )?;
    install_goal_holds(lua, &map_table, plan_world.clone(), roster.clone())?;

    // `goal.start` / `goal.run`
    map_table.set(
        "__doc_entry_start",
        String::from(
            r#"
--- starts executing a plan value and returns immediately
-- Consumes the plan: a `PlanValue` may only be taken for a run once, and a
-- second `goal.start`/`goal.run` on the same plan raises rather than
-- dispatching its actions again. The bots keep working while the script does
-- something else; poll with `run:progress()` or block with `run:wait()`.
--
-- "Returns immediately" is about this call, not about the script as a whole:
-- a run that is still going when the script ends is not abandoned. The
-- script's *own* end blocks until every run it started has finished, so a
-- script that never waits still has its bots run to completion -- it just
-- finds out how they went one call later than a script that waited would.
--
-- Either observation a `RunValue` gives back -- `run:progress()` mid-run,
-- `run:wait()` at the end -- carries `:recover()`, but only a finished one
-- will answer it; see `goal.run` for what it proposes.
-- @tparam PlanValue plan a plan returned by `goal.plan`
-- @treturn RunValue the run, immediately -- before it has finished
-- @raise if the plan was already taken for a run, or no game is connected
function goal.start(plan)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_run",
        String::from(
            r#"
--- starts executing a plan value and blocks until it finishes
-- Exactly `goal.start(plan):wait()`.
-- @tparam PlanValue plan a plan returned by `goal.plan`
-- @treturn table an observation: `{ done, pending, running, success, failed,
--   lost, walks_failed, first_error, actions, walks, failures, recover }` --
--   see `RunValue`'s own `:wait()` for the shape.
--
--   `walks_failed` counts walks the game refused. A walk has no action id, so
--   it is in none of the counts above; when one fails the rest of that bot's
--   slice is abandoned, so a run whose walking failed reports `failed = 0`
--   with everything `pending`. `first_error` falls back to the first failed
--   walk's error when no action failed, so such a run is never silent.
--
--   `lost` counts what was dispatched and never accounted for: the game
--   answered with no readable outcome, or the run ended still waiting. Those
--   actions carry `status == "lost"`, and they are deliberately neither
--   `running` (nothing is in flight) nor `failed` (no verdict was ever
--   given), so a display that draws `running` as a busy bot does not draw one
--   for work nobody is watching. `obs:failures()` does not list them.
--
--   `obs:recover()` answers what to do about a run that did not finish its
--   plan, as `next_plan, why`. `why` is one of `"rescheduled"` (the plan
--   still fits the world, so here it is again minus what already succeeded),
--   `"reexpanded"` (the world no longer affords that approach, so the same
--   goal was planned afresh), `"complete"` (every action succeeded; there is
--   nothing to run) or `"surfaced"` (nothing mechanical is left to try). The
--   first two come with a plan to hand straight back to `goal.run`; the last
--   two come with `nil`, which is what a retry loop stops on:
--
--       local obs, tries = goal.run(plan), 0
--       while obs.failed > 0 and tries < 3 do
--         local next_plan = obs:recover()
--         if next_plan == nil then break end
--         obs = goal.run(next_plan)
--         tries = tries + 1
--       end
--
--   Nothing is retried unless a script asks, and the budget above is the
--   script's on purpose: a re-expansion is proposed from the world alone, so
--   a world that has not moved gets the same proposal again, and a loop with
--   no bound of its own can spend a very long time on a goal that cannot be
--   reached. The proposal is a plan like any other -- inspect it
--   (`plan.steps`, `plan:count{...}`) or drop it.
--
--   A recovered plan already carries the history it must be run against, so
--   there is no log to pass and none to get wrong: `obs.actions[id].attempts`
--   keeps counting across a `"rescheduled"` retry, and starts again at 1
--   after a `"reexpanded"` one, whose ids number a different plan from zero.
--   Recovering a run that has not finished raises instead of answering: a
--   proposal made mid-flight would re-dispatch whatever the bots are doing
--   right now, so wait (`run:wait()`, or `goal.run`, which waits) first.
--
--   `walks` is an array of `{ bot, step_index, to, status,
--   planned_start, planned_end, dispatched_tick, replied_tick }`, one per walk
--   step the run dispatched, ordered by bot and then step index; a walk has no
--   action id, so `(bot, step_index)` is what names it. As on an action,
--   `dispatched_tick`/`replied_tick` are `game.tick` as the game reported it
--   and are `nil` -- never zero, never the planned value -- when it did not.
-- @raise on the same conditions as `goal.start`, and if the run was refused
--   before it started at all (a schedule implying a circular wait): nothing
--   ran, so there is no observation to report
function goal.run(plan)
end
"#,
        ),
    )?;
    run::install_goal_run(lua, &map_table, actuator)?;

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
/// `lua_runner.rs`), so every id `goal.plan` passes to `PlanState::from_world`
/// already has a real player and `unknown_bots()` comes back empty. It only
/// fires when a roster names a player the world has never heard of at all,
/// which is exactly the fabrication this closes.
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

/// Installs `goal.holds` on `table`.
///
/// The one call on this surface that answers a question *about the world*
/// rather than producing a plan, and it exists because nothing else could.
/// `scripts/supervisor.lua` treats an empty plan as satisfaction and says so in
/// a paragraph explaining that it has no way to check: within today's method
/// registry an empty plan really does mean the goal held, but that is an
/// internal invariant of `crates/planner`, not a contract this surface
/// exposed, and asserting satisfaction on it was a guess. It is now checkable —
/// and the planner pins the agreement itself (`an_empty_expansion_and_a_held_
/// goal_agree`), so the Lua side may rely on it rather than assume it.
///
/// No scheduling and no expansion: this is a read of the same world snapshot
/// `goal.plan` reads, through the same roster resolution and the same refusal
/// of bots the world has never heard of.
fn install_goal_holds(
    lua: &Lua,
    table: &LuaTable,
    world: Arc<FactorioWorld>,
    default_roster: Vec<BotId>,
) -> LuaResult<()> {
    table.set(
        "holds",
        lua.create_function(move |_lua, (g, opts): (LuaTable, Option<LuaTable>)| {
            let goal = value::goal_from_lua(&g)?;
            let roster = plan::resolve_roster(opts.as_ref(), &default_roster)?;
            let state = PlanState::from_world(world.clone(), &roster);
            refuse_unknown_bots(&state)?;
            // `Option<bool>` reaches Lua as a boolean or `nil` -- the three
            // answers the planner gives, unflattened. Collapsing the third
            // into `false` here would put the guess back one layer down.
            Ok(holds(&goal, &state))
        })?,
    )?;
    Ok(())
}

/// Expands one goal against a roster.
///
/// `SplitAcrossBots` needs to know who exists before it can split anything, so
/// the roster is not optional. That is not a separate decision from
/// assignment: the split sizes each share against the holdings of the bot it
/// names, so the network it produces only makes sense scheduled on that same
/// roster. `goal.plan` passes one roster to this and to `schedule`, which is
/// what makes the old expand-here-assign-there mismatch unrepresentable.
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::tokio::sync::{mpsc, watch};
    use factorio_bot_core::types::Position;
    use factorio_bot_executor::{ActionTicks, ActuatorError, ActuatorFailure};
    use factorio_bot_planner::{Holder, InventorySlot, Schedule, schedule};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    /// When the stub refuses an action.
    ///
    /// `pub(crate)`: `goal::plan`'s and `goal::run`'s own tests reuse this and
    /// `StubActuator` rather than duplicating a second stub actuator.
    pub(crate) enum Failure {
        Never,
        Always,
        /// Only the first action dispatched anywhere. That is what produces an
        /// *abandoned* tail: one bot stops at its first step while the others
        /// finish theirs, so the run ends with actions that were never
        /// dispatched and so never reached the log at all.
        First(AtomicBool),
        /// Every action comes back with no readable verdict — the game
        /// answered and the answer said nothing usable. Not a refusal: it is
        /// what an unreadable `action_completed` looks like from here, and it
        /// is the only way to reach `Status::Lost` from a script.
        WithoutVerdict,
    }

    /// An actuator that never touches a game.
    ///
    /// `walk` succeeds by default and is never gated: a bot whose *walk* fails
    /// has the rest of its slice abandoned before a single action is
    /// dispatched, so everything would stay `Pending` and the counting most of
    /// these tests exist to exercise would never run. `with_failing_walks` is
    /// for the tests that want exactly that shape — it is what a run whose
    /// pathfinder refuses looks like, and it is the shape that reached
    /// `stuck_silent` with no error to show for it.
    pub(crate) struct StubActuator {
        pub(crate) delay: Duration,
        pub(crate) fails: Failure,
        /// Every walk is refused, before any action is dispatched.
        pub(crate) fail_walks: bool,
        /// Signalled as each action is dispatched, so a test can observe a run
        /// mid-flight without sleeping and hoping.
        pub(crate) entered: Option<mpsc::UnboundedSender<()>>,
        /// Actions block here until the test sets it to `true`. A gate that is
        /// never opened is how "did `goal.start` return without waiting?"
        /// becomes a question with a definite answer.
        pub(crate) gate: Option<watch::Receiver<bool>>,
        /// A stand-in game clock, or `None` for an actuator that has none.
        ///
        /// `None` is the default because it is the honest default: a stub is
        /// not a game and has nothing to observe, so it reports
        /// [`ActionTicks::UNKNOWN`] and every test built on it exercises the
        /// absent-tick path all the way out to Lua. `with_clock` is for the
        /// tests that need to prove the ticks a run reports came from the
        /// actuator and not from the schedule.
        pub(crate) clock: Option<Arc<AtomicU64>>,
    }

    /// Where [`StubActuator::with_clock`] starts counting.
    ///
    /// Far beyond any tick these fixtures schedule, so "is this number the
    /// plan's or the actuator's?" has an answer that does not depend on
    /// knowing the schedule.
    pub(crate) const STUB_CLOCK_BASE: u64 = 500_000;

    impl StubActuator {
        pub(crate) fn new(fails: Failure) -> Self {
            StubActuator {
                delay: Duration::ZERO,
                fails,
                fail_walks: false,
                entered: None,
                gate: None,
                clock: None,
            }
        }

        /// Refuses every walk. The run then dispatches nothing at all, which
        /// is the case `workspace/runs/run-1788341905-92036` hit: 33 steps
        /// planned, none dispatched, `failed = 0` for every action, and the
        /// pathfinder's refusal reaching no record.
        pub(crate) fn with_failing_walks(mut self) -> Self {
            self.fail_walks = true;
            self
        }

        /// Gives the stub a monotonic clock, so its dispatches report ticks
        /// that advance the way a real game's would.
        pub(crate) fn with_clock(mut self) -> Self {
            self.clock = Some(Arc::new(AtomicU64::new(STUB_CLOCK_BASE)));
            self
        }

        /// Two ticks per dispatch, the second strictly after the first, and
        /// every dispatch after the one before it — the shape a real run
        /// produces. `None` when the stub has no clock.
        fn tick(&self) -> ActionTicks {
            match &self.clock {
                None => ActionTicks::UNKNOWN,
                Some(clock) => {
                    let dispatched = clock.fetch_add(7, Ordering::SeqCst);
                    let replied = clock.fetch_add(3, Ordering::SeqCst);
                    ActionTicks::new(Some(dispatched), Some(replied))
                }
            }
        }

        async fn act(&self) -> Result<ActionTicks, ActuatorFailure> {
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
            if let Failure::WithoutVerdict = &self.fails {
                return Err(ActuatorError::NoVerdict(
                    "stub answers with a status nothing can read".to_string(),
                )
                .into());
            }
            let refuse = match &self.fails {
                Failure::Never | Failure::WithoutVerdict => false,
                Failure::Always => true,
                Failure::First(spent) => !spent.swap(true, Ordering::SeqCst),
            };
            if refuse {
                return Err(ActuatorError::Rejected("stub refuses".to_string()).into());
            }
            Ok(self.tick())
        }
    }

    #[async_trait]
    impl Actuator for StubActuator {
        async fn walk(
            &self,
            _bot: BotId,
            _to: Position,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            if self.fail_walks {
                // The live wording, so a test asserting the error text is
                // asserting something a run could actually produce.
                return Err(ActuatorError::Rejected(
                    "the game's pathfinder returned no path".to_string(),
                )
                .into());
            }
            Ok(self.tick())
        }
        async fn mine(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn craft(
            &self,
            _bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<ActionTicks, ActuatorFailure> {
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
        ) -> Result<ActionTicks, ActuatorFailure> {
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
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn research(&self, _tech: &str) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
    }

    /// Wraps a stub as the factory `create_lua_goal_with` takes.
    pub(crate) fn factory(stub: Arc<dyn Actuator>) -> ActuatorFactory {
        Arc::new(move || {
            let stub = stub.clone();
            Box::pin(async move { Ok(stub) })
        })
    }

    /// Two bots mining iron ore: one action each, so no bot has a tail to
    /// abandon. Small on purpose, for the tests that only need *an* action.
    ///
    /// `pub(crate)` for `goal::run`'s tests, which drive `run::spawn` against
    /// it directly — the counting they assert on is below the Lua seam, and
    /// building the same network through `goal.plan` would put the planner's
    /// choices between the test and the thing it is testing.
    pub(crate) fn mining_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
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
    pub(crate) fn science_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
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
    /// `goal.plan` now requires of every bot it is asked to plan for. A bare
    /// `fixture_world()` never seeds any player at all, so every bot named
    /// against it is "unknown" by that same definition; tests that exercise
    /// the bindings above the refusal need this instead.
    pub(crate) fn seeded_world_for(roster: &[u8]) -> Arc<FactorioWorld> {
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

    /// A [`PlanOrigin`] for a plan that no goal produced.
    ///
    /// The tests that need one build their network and schedule by hand, so
    /// there is no goal they came from and no recovery they could ask for: an
    /// origin is on a `PlanValue` for the benefit of `obs:recover()`, which
    /// none of them reach. The goal below is a stand-in and says so; the
    /// roster is the part that has to be right, since `plan.bots` reports it.
    pub(crate) fn test_origin(roster: &[BotId]) -> Arc<plan::PlanOrigin> {
        let ids: Vec<u8> = roster.iter().map(|bot| bot.0).collect();
        Arc::new(plan::PlanOrigin {
            goal: Goal::Have {
                item: "iron-ore".into(),
                count: 1,
                whose: Holder::Anyone,
            },
            world: seeded_world_for(&ids),
            roster: roster.to_vec(),
        })
    }

    /// Installs the real `goal` table, backed by `stub`, into a sandboxed
    /// interpreter — the same one user scripts get.
    ///
    /// Installs a `PendingWork`, as `run_lua` always does in production,
    /// so that `goal.start` is free to run: most of the tests below are
    /// about what happens *after* a run starts, not about the app-data
    /// check itself. The one test for that check
    /// (`goal::run`'s `start_without_pending_work_installed_fails_loudly...`)
    /// builds its own `Lua` without this helper so it can leave `PendingWork`
    /// out on purpose.
    ///
    /// Seeded via `seeded_world_for`, not a bare `fixture_world()`: these
    /// tests are about plan/run mechanics, not about the unknown-bot refusal,
    /// so bots 1 and 2 need to be real players or `goal.plan` refuses before
    /// any of that mechanics is ever reached.
    pub(crate) fn lua_with_goal(stub: Arc<dyn Actuator>) -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(stub),
            vec![1, 2],
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");
        lua
    }

    /// How long a script gets before it is called a hang.
    pub(crate) const EXEC_BOUND: Duration = Duration::from_secs(10);

    /// A wall-clock bound that survives a hang which never yields.
    ///
    /// This exists because `tokio::time::timeout` **cannot** bound the hang
    /// these tests are most likely to hit. Every test here is a
    /// `#[tokio::test]`, i.e. a *current-thread* runtime, and a timer on a
    /// current-thread runtime is only polled when the wrapped future yields.
    /// `goal.plan` and `goal.have` are synchronous `create_function`s whose
    /// bodies (`expand` + `schedule`) are pure CPU work with no await point,
    /// so a hang inside one never returns control to the runtime and the
    /// timer never runs. That is not a theory: a probe of
    /// `while true do i = i + 1 end` under the old guard sat at 99.7% CPU with
    /// the test thread in state `R` and the 10s timeout never fired —
    /// the same signature as the run that wedged for 8h52m at ~665% CPU.
    ///
    /// So the bound is enforced from a *separate OS thread*, which the
    /// spinning one cannot starve, and which reports by name.
    ///
    /// **Why it exits the process rather than failing one test.** The obvious
    /// nicer design — run the script on a thread the test abandons on timeout
    /// — is not available: `mlua::Lua` is `!Send` without mlua's `send`
    /// feature (`Lua` holds an `XRc<ReentrantMutex<RawLua>>`, and
    /// `unsafe impl Send for RawLua` is `#[cfg(feature = "send")]`), so
    /// neither the `Lua` nor a `&Lua` can cross a thread boundary at all.
    /// Enabling `send` would force `Send` bounds on every production
    /// `create_function` closure and app-data value, which is a change to
    /// shipped code in service of a test helper. Since the wedged thread can
    /// be neither unwound nor abandoned, the watchdog ends the process
    /// instead. Under `cargo nextest` — the runner this workspace's baseline
    /// uses — each test is its own process, so that *is* a single named test
    /// failure with no collateral. Under plain `cargo test` it aborts the run
    /// with the message below, which is still a bounded, named, loud failure
    /// rather than a suite that hangs until a human notices.
    pub(crate) struct Watchdog {
        /// `true` once the guarded work finished. Paired with a `Condvar` so
        /// disarming wakes the watchdog immediately instead of leaving one
        /// sleeping thread per guarded call.
        finished: Arc<(Mutex<bool>, std::sync::Condvar)>,
    }

    impl Watchdog {
        /// Arms a bound of `limit` over whatever runs before this is dropped.
        fn arm(what: &str, limit: Duration) -> Self {
            let finished = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let watched = Arc::clone(&finished);
            let what = what.trim().to_owned();
            let thread = std::thread::current();
            let test = thread.name().unwrap_or("<unnamed>").to_owned();
            std::thread::Builder::new()
                .name("exec-bounded-watchdog".to_owned())
                .spawn(move || {
                    let (done, wake) = &*watched;
                    let (done, timeout) = wake
                        .wait_timeout_while(lock(done), limit, |done| !*done)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if *done || !timeout.timed_out() {
                        return;
                    }
                    drop(done);
                    // Not a `panic!`: this thread is not the failing one, and
                    // a panic here would only unwind the watchdog. And not
                    // `eprintln!` either -- that routes through libtest's
                    // per-thread output capture, which `std::thread::spawn`
                    // inherits, and the captured buffer is never printed
                    // because the process ends before libtest reports. Writing
                    // to the `Stderr` handle bypasses the capture, so the
                    // message survives with or without `--nocapture`.
                    use std::io::Write;
                    let message = format!(
                        "\nexec_bounded: the script did not finish within {limit:?}.\n  \
                         test:   {test}\n  \
                         script: {what}\n  \
                         This is a hang that never yields, so tokio::time::timeout could not \
                         fire on it and the test thread can be neither unwound nor abandoned \
                         (mlua::Lua is !Send). Exiting the test process with 101 so this reads \
                         as a named failure rather than a wedged suite.\n"
                    );
                    let mut err = std::io::stderr();
                    let _ = err.write_all(message.as_bytes());
                    let _ = err.flush();
                    std::process::exit(101);
                })
                .expect("spawn the exec_bounded watchdog");
            Self { finished }
        }
    }

    impl Drop for Watchdog {
        fn drop(&mut self) {
            let (done, wake) = &*self.finished;
            *lock(done) = true;
            wake.notify_all();
        }
    }

    /// Runs `work` under both bounds, returning `None` if it timed out.
    ///
    /// Two bounds, because they catch different failures. The `tokio` timeout
    /// is the one that can report an *awaiting* hang as an ordinary panic,
    /// leaving the rest of the suite to run, so it is given the shorter
    /// deadline and wins whenever it can fire at all. The watchdog is the
    /// backstop for the case it structurally cannot cover.
    pub(crate) async fn bounded<F: std::future::Future>(
        limit: Duration,
        what: &str,
        work: F,
    ) -> Option<F::Output> {
        let _watchdog = Watchdog::arm(what, limit * 2);
        factorio_bot_core::tokio::time::timeout(limit, work)
            .await
            .ok()
    }

    /// Runs `code`, failing rather than hanging if it does not finish.
    ///
    /// The bound is the point. A `goal.start` that waited for its run would
    /// block here forever behind the shut gate, and a test that hangs on
    /// regression is not a guard — it reads as a slow suite. This turns it into
    /// a named assertion failure. See [`Watchdog`] for the half of that which
    /// a timeout alone cannot do.
    pub(crate) async fn exec_bounded(lua: &Lua, code: &str) {
        exec_bounded_within(EXEC_BOUND, lua, code).await;
    }

    /// Runs `code`, failing rather than hanging if it does not finish, and
    /// returning the error `code` raised.
    ///
    /// [`exec_bounded`] is the success-path counterpart; this is its mirror for
    /// the tests that assert a script must fail -- `exec_bounded` itself
    /// `.expect`s success, so it cannot be used for them. It shares
    /// [`bounded`], so it shares the watchdog: the bound here also covers a
    /// hang that never yields. See [`Watchdog`].
    pub(crate) async fn exec_bounded_err(lua: &Lua, code: &str) -> String {
        match bounded(EXEC_BOUND, code, lua.load(code).exec_async()).await {
            None => panic!("the script did not finish within {EXEC_BOUND:?}"),
            Some(Ok(())) => panic!("the script was expected to fail but succeeded"),
            Some(Err(err)) => err.to_string(),
        }
    }

    /// [`exec_bounded`] with the deadline named, for the tests *about* the
    /// bound — they must not wait [`EXEC_BOUND`] to observe it.
    pub(crate) async fn exec_bounded_within(limit: Duration, lua: &Lua, code: &str) {
        match bounded(limit, code, lua.load(code).exec_async()).await {
            None => panic!("the script did not finish within {limit:?}: goal.start blocked"),
            Some(result) => result.expect("the script failed"),
        }
    }

    // -------------------------------------------------- the bound's own proof

    /// The bound a script gets in [`the_bound_fires_on_a_script_that_never_yields`].
    ///
    /// Short on purpose: the guard's real deadline is [`EXEC_BOUND`], and a
    /// test of the guard must not cost that. The watchdog fires at twice this.
    const PROBE_BOUND: Duration = Duration::from_millis(400);

    /// The probe's full path in this test binary, as libtest's `--exact`
    /// wants it. A rename that does not update this makes the parent test
    /// fail (no test matched), not silently pass.
    const PROBE: &str = "globals::goal::tests::the_watchdog_probe_that_never_yields";

    /// Wedges on purpose, and is meant to.
    ///
    /// `#[ignore]`, so no ordinary run selects it;
    /// [`the_bound_fires_on_a_script_that_never_yields`] runs it as a child
    /// process and asserts on how it died. The loop is Lua rather than a
    /// `sleep`: an awaiting hang is the case the old guard already handled,
    /// and testing only that is precisely why this defect survived. This one
    /// never returns to the runtime at all, so nothing but the watchdog can
    /// end it.
    #[tokio::test]
    #[ignore = "wedges on purpose; driven as a child process by the_bound_fires_on_a_script_that_never_yields"]
    async fn the_watchdog_probe_that_never_yields() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded_within(PROBE_BOUND, &lua, "local i = 0 while true do i = i + 1 end").await;
        unreachable!("the watchdog must have ended this process");
    }

    /// The guard bounds a hang that never yields — proven, not asserted.
    ///
    /// Necessarily a child process: the whole point is that the guard cannot
    /// hand control back to a wedged test, so there is no in-process
    /// observation to make. What is checked is the contract callers rely on —
    /// it ends, it ends *within the bound*, it ends unsuccessfully, and it
    /// says which script and which test.
    ///
    /// This test bounds its own wait externally (`try_wait` against a
    /// deadline, then `kill`) rather than through the mechanism under test,
    /// so a regression in the watchdog shows up here as a failure and never
    /// as a second hang.
    #[test]
    fn the_bound_fires_on_a_script_that_never_yields() {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::Instant;

        let exe = std::env::current_exe().expect("this test binary's own path");
        let mut child = Command::new(exe)
            // No `--test-threads=1`: at concurrency 1 libtest runs the test on
            // `main` rather than on a thread named after it, and the name is
            // what puts the test in the watchdog's message.
            .args([PROBE, "--exact", "--ignored", "--nocapture"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the probe");

        // Generous next to the watchdog's 800ms, tight next to "forever":
        // this is the assertion that the bound exists at all.
        let deadline = Instant::now() + Duration::from_secs(20);
        let started = Instant::now();
        let status = loop {
            match child.try_wait().expect("poll the probe") {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "the probe was still spinning after 20s: the bound did not fire. \
                         This is the pre-fix behaviour -- tokio::time::timeout cannot \
                         preempt a future that never yields."
                    );
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let elapsed = started.elapsed();

        let mut stderr = String::new();
        child
            .stderr
            .take()
            .expect("piped stderr")
            .read_to_string(&mut stderr)
            .expect("read the probe's stderr");

        assert!(
            !status.success(),
            "a wedged script must fail the probe, not pass it: {status}\n{stderr}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the bound must fire near {:?}, not merely eventually: took {elapsed:?}",
            PROBE_BOUND * 2
        );
        for expected in [
            "exec_bounded: the script did not finish within",
            "while true do i = i + 1 end",
            PROBE,
        ] {
            assert!(
                stderr.contains(expected),
                "the failure must name {expected:?} so the next occurrence is diagnosable, \
                 not just loud. Got:\n{stderr}"
            );
        }
    }

    // ------------------------------------------------------------- the surface

    /// The whole surface, as a **set**.
    ///
    /// Deliberately not a loop over an expected-name list: that shape is a
    /// mirror of the code — it stays green when a seventh function appears,
    /// which is exactly the drift it is supposed to catch. Comparing the set
    /// of callables both ways fails on a missing function *and* on an extra
    /// one.
    #[test]
    fn the_goal_table_offers_exactly_the_new_surface() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            local expected = { have=true, researched=true, all=true,
                               plan=true, run=true, start=true, holds=true }
            local actual = {}
            for k, v in pairs(goal) do
                -- the __doc__ keys are strings consumed by the doc generator
                if type(v) == "function" then actual[k] = true end
            end
            for name in pairs(expected) do
                assert(actual[name], "missing from the goal table: " .. name)
            end
            for name in pairs(actual) do
                assert(expected[name], "unexpected function on the goal table: " .. name)
            end
            -- Named explicitly as well, so the failure message says *which* old
            -- name survived rather than only that the set differs.
            for _, gone in ipairs{"schedule","graphviz","gantt","execute","progress","wait"} do
                assert(goal[gone] == nil, gone .. " must be gone, not merely deprecated")
            end
        "#,
        )
        .exec()
        .expect("script");
    }

    /// `goal.holds` answers about the world, and answers per holder.
    ///
    /// `lua_with_goal`'s roster is bots 1 and 2, each seeded the way a run
    /// seeds them: one furnace, one drill, one wood. So two furnaces exist
    /// between them and one exists on each -- the case that makes "whose"
    /// load-bearing rather than decorative, and the same shape as the live
    /// milestone that closed satisfied on a starting inventory nobody had
    /// smelted.
    #[test]
    fn holds_answers_satisfaction_without_planning() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            assert(goal.holds(goal.have("stone-furnace", 2)) == true,
                   "two furnaces exist across the roster")
            assert(goal.holds(goal.have("stone-furnace", 3)) == false,
                   "a third does not")
            assert(goal.holds(goal.have("stone-furnace", 2, { bot = 1 })) == false,
                   "and bot 1 holds only one of them, which is the whole point")
            assert(goal.holds(goal.have("stone-furnace", 1, { bot = 1 })) == true)
            assert(goal.holds(goal.all { goal.have("wood", 2),
                                         goal.have("stone-furnace", 3) }) == false,
                   "a bundle holds only when every member does")
            assert(goal.holds(goal.have("stone-furnace", 2), { bots = { 1 } }) == false,
                   "and the roster asked about is the roster answered for")
        "#,
        )
        .exec()
        .expect("script");
    }

    /// The agreement `supervisor.lua` used to assume: where the plan is empty,
    /// `goal.holds` says the goal is met, and where it is not, it does not.
    /// Pinned through the bindings as well as inside the planner, because it
    /// is the Lua side that acts on it.
    #[test]
    fn an_empty_plan_and_a_held_goal_agree_through_the_bindings() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            for _, n in ipairs{1, 2, 3, 8} do
                local g = goal.have("stone-furnace", n)
                local empty = #goal.plan(g).steps == 0
                assert(empty == (goal.holds(g) == true),
                       "plan emptiness and satisfaction disagree at " .. n)
            end
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn holds_refuses_a_roster_the_world_does_not_have() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        let err = lua
            .load(r#"goal.holds(goal.have("wood", 1), { bots = { 9 } })"#)
            .exec()
            .expect_err("bot 9 is not a connected player");
        assert!(
            err.to_string().contains("not connected players"),
            "the refusal must name the cause, got {err}"
        );
    }

    /// The whole surface, composed, through the table a script really gets.
    ///
    /// Every test the value-based surface grew before this one built its own
    /// table and installed the constructors onto it by hand, because the
    /// production table still carried the handle-based `have`/`researched`.
    /// So each part was proven against a parallel wiring and the composition
    /// against none. This is the one test that holds `create_lua_goal_with`'s
    /// own table to the whole chain: goal value -> plan -> run -> the refusal
    /// of a spent plan.
    #[tokio::test]
    async fn the_production_goal_table_composes_end_to_end() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local g    = goal.all { goal.have("iron-ore", 2), goal.have("coal", 1) }
            local p    = goal.plan(g, { bots = { 1 } })
            local obs  = goal.run(p)
            assert(#p.steps > 0, "the composed surface produced a plan")
            assert(p:count { kind = "mine" } > 0, "and it contains real work")
            assert(obs.done and obs.failed == 0, "and the run completed cleanly")
            -- `tostring` because a Rust-raised error arrives in Lua as an
            -- error *object* (mlua userdata carrying the `LuaError`), not as
            -- a string: `:find` on it raises "attempt to index a userdata
            -- value" and would report a spent-plan refusal as a surface
            -- defect. Every other pcall assertion in this crate spells it the
            -- same way.
            local ok, err = pcall(goal.run, p)
            assert(not ok, "a spent plan must refuse a second run")
            assert(tostring(err):find("already"), "and say so: " .. tostring(err))
        "#,
        )
        .await;
    }

    // --------------------------------------------- the scheduler/executor seam

    /// Records the bot named by every command it is asked to perform.
    #[derive(Default)]
    struct RecordingActuator {
        bots: std::sync::Mutex<Vec<BotId>>,
    }

    impl RecordingActuator {
        /// No game clock: this actuator records who was dispatched to, not
        /// when, so it reports [`ActionTicks::UNKNOWN`].
        fn note(&self, bot: BotId) -> Result<ActionTicks, ActuatorFailure> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().push(bot);
            Ok(ActionTicks::UNKNOWN)
        }
        fn recorded(&self) -> Vec<BotId> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Actuator for RecordingActuator {
        async fn walk(
            &self,
            bot: BotId,
            _to: Position,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn mine(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn craft(
            &self,
            bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn place(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
        ) -> Result<ActionTicks, ActuatorFailure> {
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
        ) -> Result<ActionTicks, ActuatorFailure> {
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
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn research(&self, _tech: &str) -> Result<ActionTicks, ActuatorFailure> {
            Ok(ActionTicks::UNKNOWN)
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
    /// `goal.run`, and then asks the *executor's own* resolution function
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
            // player or `goal.plan` refuses before the seam below is ever
            // exercised.
            let table = create_lua_goal_with(
                &lua,
                seeded_world_for(&roster),
                factory(rec.clone()),
                roster.clone(),
                None,
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            exec_bounded(
                &lua,
                r#"
                goal.run(goal.plan(goal.have("iron-ore", 20)))
                "#,
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

    /// A plan for a run of several bots, made for one of them.
    ///
    /// Under the old handle surface this was the split that could not be
    /// expressed safely: `goal.have` expanded against every bot the run had,
    /// `SplitAcrossBots` gave each of them a chain spending *its own* starting
    /// stone furnace, and `goal.schedule(p, 1)` then asked one bot to run all
    /// of them — asking that bot for as many furnaces as the roster had
    /// between them. Live, with `scripts/goal_smoke.lua`, that was:
    ///
    /// ```text
    /// goal: precondition has 1 stone-furnace of action ActionId(0) does not hold for bot 1
    /// ```
    ///
    /// identically for `--bots 2` and `--bots 4`, while `--bots 1` — the only
    /// roster where expansion and assignment agree — succeeded. `goal.plan`
    /// takes one roster for both, so the mismatch cannot be written any more;
    /// this test is what proves the surviving call really does expand for the
    /// bots it schedules for, against the *production* seeding rather than a
    /// hand-built world.
    ///
    /// Rosters of 2 and 4 are both here because the split's arithmetic differs
    /// between them: a shortfall of five over two bots is 3 + 2, over four bots
    /// 2 + 1 + 1 + 1, and only the second lets a share of one reach the chain
    /// that a share of one is supposed to open.
    #[tokio::test]
    async fn a_plan_made_for_one_bot_of_a_larger_run_is_schedulable() {
        for bot_count in [1u8, 2, 4] {
            let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
            lua.set_app_data(crate::lua_runner::PendingWork::default());
            let table = create_lua_goal_with(
                &lua,
                seeded_world(bot_count),
                factory(Arc::new(StubActuator::new(Failure::Never))),
                (1..=bot_count).collect(),
                None,
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            // The smoke script's own shape: plan for one bot out of the run,
            // then render what was scheduled.
            exec_bounded(
                &lua,
                r#"
                local p = goal.plan(goal.have("iron-plate", 5), { bots = { 1 } })
                assert(p.makespan > 0, "a real plan takes a positive number of ticks")
                for _, s in ipairs(p.steps) do assert(s.bot == 1, "every step on bot 1") end
                assert(#p:gantt("smoke") > 0, "the gantt must describe what was scheduled")
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
        // The fixture has no Factorio behind it, so the world has no players
        // and `Planner::roster` would hand the script nobody -- correctly, for
        // a live run. Seed them the way the planning-only mode does
        // (`--clients 0`, in `cli/lua.rs`), which is the production caller this
        // fixture stands in for.
        planner.initiate_missing_players_with_default_inventory(4);
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

    /// `goal.plan` refuses the run's *default* roster when it names a bot the
    /// world has no player for, rather than silently planning it with
    /// `BotState::default()`'s empty inventory and guessed reach distances.
    ///
    /// The default roster, specifically: `plan.rs`'s
    /// `an_unknown_bot_raises_and_names_it` covers an explicit `opts.bots`,
    /// and a check that only looked at what the caller passed would leave
    /// this path — the one every ordinary script takes — unguarded.
    ///
    /// Bot 1 is seeded and bot 2 is not, so this also discriminates from a
    /// check that only ever looks at "is the roster empty" or similar: a
    /// roster with one real bot in it must still be refused for the other,
    /// unnamed one, and the error must name it.
    #[tokio::test]
    async fn goal_plan_refuses_a_default_roster_naming_a_bot_the_world_does_not_have() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1]),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.plan(goal.have("iron-ore", 20))"#)
            .eval_async::<LuaValue>()
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

    /// A `researched` goal goes through the same `expand_goal` path, so the
    /// same refusal must fire for it, against a world that actually has a
    /// technology to research (otherwise the planner's own
    /// `UnknownTechnology` error would fire first and this would prove
    /// nothing about the unknown-bot check specifically).
    #[tokio::test]
    async fn goal_plan_refuses_an_unknown_bot_for_a_research_goal_too() {
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
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.plan(goal.researched("automation"))"#)
            .eval_async::<LuaValue>()
            .await
            .expect_err("bot 2 is not a player in this world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the unknown bot: {message}"
        );
    }

    /// The refusal reads the world as it is **now**, not as it was when the
    /// table was built.
    ///
    /// A goal value is pure and can be built long before it is planned, so
    /// the check cannot be hoisted to `create_lua_goal_with` or memoised on
    /// the roster: both bots are real players when the goal value is made,
    /// and bot 2 is only removed from the world afterwards -- from the same
    /// `FactorioWorld` the run's `goal` table already closed over, so
    /// `goal.plan`'s `PlanState::from_world` call is the thing that has to
    /// notice.
    #[tokio::test]
    async fn goal_plan_refuses_a_bot_removed_after_the_goal_value_was_built() {
        let world = seeded_world_for(&[1, 2]);
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world.clone(),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(&lua, r#"g = goal.have("iron-ore", 20)"#).await;
        // Proof the goal value itself is fine while both bots are real: this
        // same call is what must fail after the removal below.
        exec_bounded(&lua, "goal.plan(g)").await;

        world.remove_player(2).expect("remove_player");

        let err = lua
            .load("return goal.plan(g)")
            .eval_async::<LuaValue>()
            .await
            .expect_err("bot 2 was just removed from the world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the now-unknown bot: {message}"
        );
    }

    // ------------------------------------------- refusing a second run

    /// The regression this closes: running the same plan twice used to spawn
    /// a second, independent run against the same schedule, dispatching every
    /// action again -- against a live game that means placing an entity
    /// twice, inserting twice, mining an already-mined tile.
    ///
    /// `goal::run`'s `running_one_plan_twice_raises` covers the error text.
    /// This covers the part an error message cannot: `rec` records every
    /// command any run performs, so a refused second `goal.run` that had
    /// nevertheless spawned its run would show up as a higher count after it
    /// than before.
    #[tokio::test]
    async fn a_second_run_on_the_same_plan_dispatches_nothing_again() {
        let rec = Arc::new(RecordingActuator::default());
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(rec.clone()),
            vec![1, 2],
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(
            &lua,
            r#"
            p = goal.plan(goal.have("iron-ore", 20))
            goal.run(p)
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
            local ok, err = pcall(goal.run, p)
            result = { ok = ok, err = tostring(err) }
            "#,
        )
        .await;
        let result: LuaTable = lua.globals().get("result").expect("result");
        let ok: bool = result.get("ok").expect("ok");
        let err: String = result.get("err").expect("err");
        assert!(!ok, "a second run on the same plan must be refused");
        assert!(
            err.contains("already"),
            "the error should say the plan already ran: {err}"
        );
        assert_eq!(
            rec.recorded().len(),
            after_first,
            "the refused second run must not have dispatched anything"
        );
    }
}
