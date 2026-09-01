// Reachable from one line of user Lua, and this crate builds with
// `panic = "abort"`, so every panic here kills the whole process.
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! `record.*` -- writing a durable run archive from a script.
//!
//! The supervisor is a Lua script, so the thing that knows what a milestone is
//! lives on the Lua side. This is the surface it records through.
//!
//! Ticks are not passed in by the caller: a script has no way to know
//! `game.tick`, and a made-up tick would poison the one axis every other part
//! of this design indexes on. Each event is stamped with the tick from the most
//! recent reply the game sent us, which is as current as the last command
//! issued and honestly `null` before there has been one.

use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::record::map::{
    EntitySnapshot, MapKind, MapRecord, Placement, divergence_between,
};
use factorio_bot_core::record::{
    ActionFailure, EventKind, FailureKind, PlannedStep, RunRecorder, SatisfiedReason,
};
use factorio_bot_core::types::{AreaFilter, EntityType, PlayerId, Position, Rect};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

type Slot = Arc<Mutex<Option<RunRecorder>>>;

fn record_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("record: {err}"))
}

fn rcon_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("rcon: {err}"))
}

/// Whether an entity the live game reports belongs in a keyframe's `game`
/// array.
///
/// The keyframe compares what the game has against `EntityGraph::snapshot_within`
/// -- the "model" side -- and a divergence list is only a useful signal when
/// both sides describe the same population. `find_entities_filtered` with no
/// type filter returns *everything* in the box: trees, small rocks, ore,
/// characters, items on the ground. None of those are things a bot places or
/// something `EntityGraph` tracks, so left in, they would dominate every
/// keyframe's `game` array with terrain nobody placed and swamp the actual
/// divergences underneath.
///
/// This mirrors `EntityGraph::add` (`crates/core/src/graph/entity_graph.rs`)
/// exactly: the entity types it inserts into `entity_tree`, the two named
/// rocks it also blocks on, and `Resource` (read back out of `resource_tree`
/// by `snapshot_within`, so ore patches are legitimately part of the model
/// side too). Keep the two lists in sync -- a type `add` starts tracking
/// without a matching arm here would show up as a permanent, spurious
/// divergence for every run that touches it.
fn keyframe_relevant(entity_type: &str, name: &str) -> bool {
    matches!(
        EntityType::from_str(entity_type),
        Ok(EntityType::Furnace)
            | Ok(EntityType::Inserter)
            | Ok(EntityType::Boiler)
            | Ok(EntityType::Lab)
            | Ok(EntityType::OffshorePump)
            | Ok(EntityType::MiningDrill)
            | Ok(EntityType::StorageTank)
            | Ok(EntityType::Container)
            | Ok(EntityType::Splitter)
            | Ok(EntityType::TransportBelt)
            | Ok(EntityType::UndergroundBelt)
            | Ok(EntityType::Pipe)
            | Ok(EntityType::PipeToGround)
            | Ok(EntityType::LogisticContainer)
            | Ok(EntityType::AssemblingMachine)
            | Ok(EntityType::Resource)
    ) || name == "rock-big"
        || name == "rock-huge"
}

/// The reverse of `run.rs`'s `entity_snapshot_to_lua`: reads the same shape
/// back off a Lua table. Built field by field, not through a JSON bridge, so
/// there is no `Option::None`-as-truthy trap to guard against on this side
/// either.
fn entity_snapshot_from_lua(t: &LuaTable) -> LuaResult<EntitySnapshot> {
    let name: String = t.get("name")?;
    let position: LuaTable = t.get("position")?;
    let x: f64 = position.get("x")?;
    let y: f64 = position.get("y")?;
    let direction: u8 = t.get("direction")?;
    Ok(EntitySnapshot {
        name,
        position: Position::new(x, y),
        direction,
    })
}

/// The reverse of `run.rs`'s `placement_to_lua`.
fn placement_from_lua(t: &LuaTable) -> LuaResult<Placement> {
    let intent: LuaTable = t.get("intent")?;
    let actual: LuaTable = t.get("actual")?;
    let drift: Option<Vec<String>> = t.get("drift")?;
    Ok(Placement {
        intent: entity_snapshot_from_lua(&intent)?,
        actual: entity_snapshot_from_lua(&actual)?,
        drift,
    })
}

/// Reads one [`PlannedStep`] off a Lua table shaped by `plan_for_record` in
/// `scripts/supervisor.lua` -- `id`, `bot`, `action`, `deps`, `planned_start`,
/// `planned_duration`.
///
/// `deps` is read by type, not by truthiness. A step this crate's own
/// `step_to_lua` (`goal/plan.rs`) never gave a `deps` key at all (a walk) is a
/// real absent key here and reads as an ordinary Lua `nil` -- but the same
/// field on a table built by round-tripping an `Option<Vec<u32>>` through a
/// Rust `serde` bridge elsewhere in this workspace would instead be mlua's
/// null sentinel, light userdata that is truthy. `local x = t.deps or {}`
/// would not substitute the default in that case, so the type is checked
/// explicitly instead of relying on `or`.
fn planned_step_from_lua(t: &LuaTable) -> LuaResult<PlannedStep> {
    let id: u32 = t.get("id")?;
    let bot: u32 = t.get("bot")?;
    let action: String = t.get("action")?;
    let deps: Vec<u32> = match t.get::<LuaValue>("deps")? {
        LuaValue::Table(deps) => deps
            .sequence_values::<u32>()
            .collect::<LuaResult<Vec<_>>>()?,
        _ => Vec::new(),
    };
    let planned_start: u64 = t.get("planned_start")?;
    let planned_duration: u64 = t.get("planned_duration")?;
    Ok(PlannedStep {
        id,
        bot,
        action,
        deps,
        planned_start,
        planned_duration,
    })
}

/// Parses `record.milestone_satisfied`'s third argument.
///
/// Never produces [`SatisfiedReason::Unknown`]: that variant means "recorded
/// before this field existed", a fact about an old file on disk, and is not
/// something a live call can mean to say. A caller passing anything else is
/// refused by name instead of being folded into `Unknown` -- doing that would
/// make every future reader unable to trust that the value ever meant what it
/// says.
fn parse_satisfied_reason(reason: &str) -> LuaResult<SatisfiedReason> {
    match reason {
        "already_satisfied" => Ok(SatisfiedReason::AlreadySatisfied),
        "plan_empty" => Ok(SatisfiedReason::PlanEmpty),
        other => Err(record_error(format!(
            "milestone_satisfied: unknown reason \"{other}\"; expected \"already_satisfied\" or \"plan_empty\""
        ))),
    }
}

/// Classifies a settled action's error text into a coarse [`FailureKind`].
///
/// Matched against the outer `ActuatorError` wording
/// (`crates/executor/src/actuator.rs`) and the inner mod/rcon text it wraps
/// (`crates/core/src/errors.rs`) -- as plain substrings, deliberately, rather
/// than a dependency on either crate's error types: this classifier only
/// needs to read text that already crossed the Lua boundary as a `String`,
/// and a substring match degrades to [`FailureKind::Other`] instead of
/// failing outright when the wording moves. Only the four outer
/// `ActuatorError` formats are pinned by a test on the producing side
/// (`crates/executor/src/actuator.rs`); the inner text is not, so a wording
/// change there is the first thing to check if a failure starts landing in
/// `Other` that used to classify correctly.
fn classify_failure(error: &str) -> ActionFailure {
    let kind = if error.contains("no action result received in time") {
        FailureKind::Timeout
    } else if error.contains("no path to")
        || error.contains("tile arrival tolerance")
        || error.contains("tile resource reach")
    {
        FailureKind::Unreachable
    } else if error.contains("player still blocks placement")
        || error.contains("player blocks placement in all directions")
    {
        FailureKind::Blocked
    } else if error.contains("does not have any") {
        FailureKind::MissingItem
    } else if error.contains("game rejected the command") {
        FailureKind::Rejected
    } else {
        FailureKind::Other
    };
    // The item name, for `MissingItem` only -- the mod's own wording is
    // `cannot place item '<item>' because the player '<name>' does not have
    // any`, so the text between the first pair of single quotes is the item.
    // Anything else is left with no detail rather than a guess: `error`
    // beside this field already carries the whole message for a person to
    // read.
    let detail = (kind == FailureKind::MissingItem)
        .then(|| error.split('\'').nth(1))
        .flatten()
        .map(str::to_string);
    ActionFailure { kind, detail }
}

/// Records a *live* event: stamped with the game's clock, never earlier than
/// something already in the log. See [`RunRecorder::not_before`].
fn record_live(
    slot: &Slot,
    rcon: &factorio_bot_core::factorio::rcon::FactorioRcon,
    kind: EventKind,
) -> LuaResult<()> {
    let mut guard = slot.lock();
    let recorder = guard
        .as_mut()
        .ok_or_else(|| record_error("no recording is running -- call record.start() first"))?;
    let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
    recorder.record(tick, kind).map_err(record_error)
}

pub fn create_lua_record(
    lua: &Lua,
    rcon: Arc<factorio_bot_core::factorio::rcon::FactorioRcon>,
    world: Arc<FactorioWorld>,
    scripts_root: PathBuf,
    all_bots: Vec<PlayerId>,
) -> LuaResult<LuaTable> {
    create_lua_record_with_slot(
        lua,
        rcon,
        world,
        scripts_root,
        all_bots,
        Arc::new(Mutex::new(None)),
    )
}

/// [`create_lua_record`]'s real body, parameterised on the recorder slot.
///
/// Production always starts empty (`create_lua_record` supplies that). Tests
/// take the other path: seeding `slot` with a `RunRecorder` built directly
/// (plain file I/O, no RCON involved) reaches `record.actions`/`record.
/// keyframe` without going through `record.start()` -- which cannot succeed
/// in a test, since it always makes a real RCON call
/// (`frame_capture_start`) that fails against `FactorioRcon::new_empty()`.
fn create_lua_record_with_slot(
    lua: &Lua,
    rcon: Arc<factorio_bot_core::factorio::rcon::FactorioRcon>,
    world: Arc<FactorioWorld>,
    scripts_root: PathBuf,
    all_bots: Vec<PlayerId>,
    slot: Slot,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Run recording
-- Writes a durable archive of a run: an append-only event log, the frames
-- captured while it ran, and speedrun-style splits derived from its
-- milestones. See `docs/superpowers/specs/2026-09-01-run-recording-and-replay-design.md`.
--
-- Events are stamped with the game's own tick, taken from the most recent
-- reply the game sent -- scripts never supply one, because a script cannot
-- know `game.tick` and a fabricated tick would corrupt the axis the whole
-- replay is indexed on.
--
-- @module record

local record = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return record"#))?;

    // `scripts_root` is `<workspace>/scripts` by construction (see
    // `factorio_bot_core::scripts`), so the workspace is its parent. Deriving it
    // beats threading a second path through every caller when one is a strict
    // function of the other -- but it is a real coupling, so it is stated here
    // rather than left to be rediscovered.
    let runs_root = scripts_root
        .parent()
        .map(|workspace| workspace.join("runs"))
        .ok_or_else(|| record_error("scripts directory has no parent workspace"))?;

    map_table.set(
        "__doc_entry_start",
        String::from(
            r#"
--- starts recording a run
-- Mints a run id, creates `<workspace>/runs/<id>/`, and starts frame capture
-- with that same id -- one call, so the log and the frames cannot disagree
-- about which run they belong to.
-- @treturn string the run id
-- @raise if a recording is already running, or the run directory cannot be created
function record.start()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        let runs_root = runs_root.clone();
        let bots: Vec<u32> = all_bots.iter().map(|id| u32::from(*id)).collect();
        map_table.set(
            "start",
            lua.create_async_function(move |_lua, ()| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let runs_root = runs_root.clone();
                let bots = bots.clone();
                async move {
                    if slot.lock().is_some() {
                        return Err(record_error(
                            "a recording is already running -- call record.finish() first",
                        ));
                    }
                    let run_id = mint_run_id();

                    // Frame capture first, and its reply is where the opening
                    // tick comes from. Recording `run_started` before any
                    // command has been sent would stamp it with a tick nobody
                    // has observed -- the first live run opened at tick 0 while
                    // every later event was near 59000, which is a fabricated
                    // number that looks like data.
                    let opened_at = rcon
                        .as_ref()
                        .frame_capture_start(Some(run_id.clone()))
                        .await
                        .map_err(rcon_error)?;

                    let mut recorder =
                        RunRecorder::start(&runs_root, run_id.clone()).map_err(record_error)?;
                    recorder
                        .record(
                            opened_at.unwrap_or_else(|| rcon.last_tick().unwrap_or(0)),
                            EventKind::RunStarted {
                                run_id: run_id.clone(),
                                bots,
                                seed: None,
                                factorio: None,
                                git: None,
                            },
                        )
                        .map_err(record_error)?;
                    *slot.lock() = Some(recorder);
                    Ok(run_id)
                }
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_milestone_started",
        String::from(
            r#"
--- records that a milestone has begun
-- @number index the milestone's position in the run, from 1
-- @string goal a human-readable description of what is being pursued
-- @raise if no recording is running
function record.milestone_started(index, goal)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "milestone_started",
            lua.create_function(move |_lua, (index, goal): (u32, String)| {
                record_live(&slot, &rcon, EventKind::MilestoneStarted { index, goal })
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_milestone_satisfied",
        String::from(
            r#"
--- records that a milestone was reached
-- @number index the milestone's position in the run, from 1
-- @number iterations how many plan/run cycles it took
-- @string reason why no further work was needed: `"already_satisfied"` (the
--   world already met the goal before planning was attempted) or
--   `"plan_empty"` (the planner produced no steps)
-- @raise if no recording is running, or `reason` is not one of the above
function record.milestone_satisfied(index, iterations, reason)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "milestone_satisfied",
            lua.create_function(
                move |_lua, (index, iterations, reason): (u32, u32, String)| {
                    let reason = parse_satisfied_reason(&reason)?;
                    record_live(
                        &slot,
                        &rcon,
                        EventKind::MilestoneSatisfied {
                            index,
                            iterations,
                            reason,
                        },
                    )
                },
            )?,
        )?;
    }

    map_table.set(
        "__doc_entry_plan_created",
        String::from(
            r#"
--- records the plan the scheduler produced for a milestone
-- Takes an array of step tables -- `id`, `bot`, `action`, `deps` (an array of
-- step ids this one waits on), `planned_start`, `planned_duration` -- so a
-- run's record shows what was planned, not only what happened. `steps`,
-- `makespan` and `bots` are derived from `plan` itself rather than taken as
-- separate arguments, so nothing here can disagree with what was actually
-- recorded: `steps` is `plan`'s length, `makespan` the latest
-- `planned_start + planned_duration` across every entry (0 for an empty
-- plan), and `bots` the distinct bot ids it names, ascending.
-- @number index the milestone's position in the run, from 1
-- @tparam table plan an array of step tables
-- @raise if no recording is running
function record.plan_created(index, plan)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "plan_created",
            lua.create_function(move |_lua, (index, plan): (u32, LuaTable)| {
                let mut steps: Vec<PlannedStep> = Vec::new();
                let mut bots: BTreeSet<u32> = BTreeSet::new();
                let mut makespan: u64 = 0;
                for step in plan.sequence_values::<LuaTable>() {
                    let planned = planned_step_from_lua(&step?)?;
                    bots.insert(planned.bot);
                    makespan = makespan.max(
                        planned
                            .planned_start
                            .saturating_add(planned.planned_duration),
                    );
                    steps.push(planned);
                }
                let step_count = u32::try_from(steps.len()).unwrap_or(u32::MAX);
                record_live(
                    &slot,
                    &rcon,
                    EventKind::PlanCreated {
                        milestone_index: index,
                        steps: step_count,
                        makespan,
                        bots: bots.into_iter().collect(),
                        plan: steps,
                    },
                )
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_milestone_stuck",
        String::from(
            r#"
--- records that a milestone was abandoned
-- `outcome` is the supervisor's own verdict and is carried through unchanged:
-- `stuck` (no progress, with failures), `stuck_silent` (no progress and every
-- run reported success, so something is lying) or `exhausted` (the iteration
-- cap tripped while progress was still being made).
--
-- `last_error` and `best_steps` are optional and default to `nil`, meaning
-- "not known" rather than "nothing happened" -- pass whatever the caller
-- actually has (e.g. the text a `pcall` around the driving loop caught) so a
-- stuck milestone's record carries the reason, not just the verdict.
-- @number index the milestone's position in the run, from 1
-- @string outcome why it was abandoned
-- @string[opt] last_error the most recent error text, if one is known
-- @number[opt] best_steps the fewest steps any plan for this milestone reached
-- @raise if no recording is running
function record.milestone_stuck(index, outcome, last_error, best_steps)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "milestone_stuck",
            lua.create_function(
                move |_lua,
                      (index, outcome, last_error, best_steps): (
                    u32,
                    String,
                    Option<String>,
                    Option<u32>,
                )| {
                    record_live(
                        &slot,
                        &rcon,
                        EventKind::MilestoneStuck {
                            index,
                            outcome,
                            best_steps,
                            last_error,
                        },
                    )
                },
            )?,
        )?;
    }

    map_table.set(
        "__doc_entry_actions",
        String::from(
            r#"
--- records what each bot did during one executed plan
-- Takes `plan.steps` and `observation.actions` and joins them by action id:
-- the plan knows which bot an action belongs to and what it is called, the
-- observation knows when the game dispatched it and what verdict it reached.
-- Neither half carries both.
--
-- Ticks come from the game, so an action the game never reported a dispatch
-- for is not recorded -- it cannot be placed in time, and placing it anywhere
-- would be an invention. An action that *was* dispatched and never settled is
-- recorded as the dispatch alone, which is a real state and one worth seeing.
-- @tparam table steps `plan.steps`
-- @tparam table actions `observation.actions`
-- @treturn number how many events were written
-- @raise if no recording is running
function record.actions(steps, actions)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        map_table.set(
            "actions",
            lua.create_function(move |_lua, (steps, actions): (LuaTable, LuaTable)| {
                let mut written = 0u32;
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;

                for step in steps.sequence_values::<LuaTable>() {
                    let step = step?;
                    let Some(id) = step.get::<Option<u32>>("id")? else {
                        continue; // a walk: no action id, no lane entry here
                    };
                    let bot: u32 = step.get("bot")?;
                    let label: String = step.get::<Option<String>>("label")?.unwrap_or_default();
                    let Some(observed) = actions.get::<Option<LuaTable>>(id)? else {
                        continue;
                    };
                    let status: String = observed
                        .get::<Option<String>>("status")?
                        .unwrap_or_else(|| "unknown".into());
                    let dispatched: Option<u64> = observed.get("dispatched_tick")?;
                    let replied: Option<u64> = observed.get("replied_tick")?;
                    let error: Option<String> = observed.get("error")?;
                    let placed: Option<LuaTable> = observed.get("placed")?;

                    if let Some(dispatched) = dispatched {
                        recorder
                            .record(
                                dispatched,
                                EventKind::ActionDispatched {
                                    id,
                                    bot,
                                    action: label,
                                    target: None,
                                },
                            )
                            .map_err(record_error)?;
                        written += 1;
                    }
                    if let Some(replied) = replied {
                        // `None` only on a genuine success: a settle this
                        // codebase does not spell `"success"` is a failure of
                        // some kind, even one the classifier cannot name yet,
                        // and `FailureKind::Other` says so instead of leaving
                        // `failure` permanently unwritten the way it was
                        // before this classifier existed.
                        let failure = (status != "success")
                            .then(|| classify_failure(error.as_deref().unwrap_or("")));
                        recorder
                            .record(
                                replied,
                                EventKind::ActionSettled {
                                    id,
                                    bot,
                                    status,
                                    elapsed_ticks: dispatched.map(|d| replied.saturating_sub(d)),
                                    error,
                                    failure,
                                },
                            )
                            .map_err(record_error)?;
                        written += 1;

                        // `placed` rides on the same settle tick as the
                        // `ActionSettled` line above: a placement is only
                        // known once the actuator has drained it at settle
                        // (see `Attempt::placed`), so there is no earlier
                        // real tick to stamp it with.
                        if let Some(placed) = placed {
                            let placement = placement_from_lua(&placed)?;
                            recorder
                                .record_map(MapRecord {
                                    tick: replied,
                                    kind: MapKind::Placed {
                                        bot,
                                        intent: placement.intent,
                                        actual: placement.actual,
                                        drift: placement.drift,
                                    },
                                })
                                .map_err(record_error)?;
                        }
                    }
                }
                Ok(written)
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_keyframe",
        String::from(
            r#"
--- writes a keyframe: what the game and our belief about it agree on
-- Bounds the box at the bounding box of every entity placed so far this run,
-- plus a 16-tile margin, then asks the live game and the world model for
-- everything inside it and records where they diverge. Call this at
-- milestone boundaries -- there is deliberately no tick timer driving it.
--
-- Returns `false`, and writes nothing, for either of two unremarkable cases:
-- no recording is running, or one is but nothing has been placed yet (a
-- keyframe over a box nothing has ever occupied is not a fact worth
-- recording). Both are expected outcomes of calling this from a place that
-- does not know whether recording is active, so both are a return value, not
-- an error -- a caller that wants to tell them apart still can, since only
-- the second follows a successful `record.start()`. Anything else going
-- wrong (the game cannot be reached, a malformed response) still raises,
-- because that is not a "nothing to do here" outcome and must not look like
-- one.
-- @treturn boolean whether a keyframe was written
-- @raise if the attempt to read the game or the world model itself fails
function record.keyframe()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        let world = world.clone();
        map_table.set(
            "keyframe",
            lua.create_async_function(move |_lua, ()| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let world = world.clone();
                async move {
                    // The bounds are read and released before the `.await`
                    // below: nothing here needs the lock held across it, and
                    // an `await` under a `parking_lot::Mutex` guard is a
                    // deadlock waiting for a yield.
                    //
                    // "No recording is running" is not an error here, unlike
                    // every other `record.*` function: this is the one call
                    // meant to be made from a place (a milestone boundary)
                    // that does not know whether a recording is active, and
                    // that not knowing must not turn into a hard failure of
                    // whatever loop called it. `false` carries the same fact
                    // an error would, just as a value instead of a raise.
                    let bounds = {
                        let guard = slot.lock();
                        match guard.as_ref() {
                            Some(recorder) => recorder.placed_bounds(16.0),
                            None => return Ok(false),
                        }
                    };
                    let Some(bounds) = bounds else {
                        return Ok(false);
                    };

                    let rect = Rect::new(
                        &Position::new(bounds.left, bounds.top),
                        &Position::new(bounds.right, bounds.bottom),
                    );
                    let game_entities = rcon
                        .find_entities_filtered(&AreaFilter::Rect(rect.clone()), None, None)
                        .await
                        .map_err(rcon_error)?;
                    // Restricted to what `EntityGraph` models -- see
                    // `keyframe_relevant` -- so `game` and `model` are
                    // comparable populations rather than the unfiltered box
                    // (trees, rocks, ore, characters, dropped items) against
                    // the curated one.
                    let game: Vec<EntitySnapshot> = game_entities
                        .into_iter()
                        .filter(|e| keyframe_relevant(&e.entity_type, &e.name))
                        .map(|e| EntitySnapshot {
                            name: e.name,
                            position: e.position,
                            direction: e.direction,
                        })
                        .collect();
                    let model = world.entity_graph.snapshot_within(&rect);
                    let divergence = divergence_between(&game, &model);

                    let mut guard = slot.lock();
                    // Same non-error treatment as the first check, for the
                    // same reason: a `record.finish()` racing this call is
                    // "no recording any more", not a bug in this function.
                    let Some(recorder) = guard.as_mut() else {
                        return Ok(false);
                    };
                    let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
                    recorder
                        .record_map(MapRecord {
                            tick,
                            kind: MapKind::Keyframe {
                                bounds,
                                game,
                                model,
                                divergence,
                            },
                        })
                        .map_err(record_error)?;
                    Ok(true)
                }
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_finish",
        String::from(
            r#"
--- closes the recording
-- Writes the manifest and the derived splits, and copies this run's frames out
-- of the workspace before the next run wipes them. Frames belonging to another
-- run are left where they are.
-- @string outcome how the run ended, e.g. "done" or "stuck"
-- @treturn string the run id
-- @raise if no recording is running
function record.finish(outcome)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        let workspace = runs_root
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| record_error("runs directory has no parent workspace"))?;
        map_table.set(
            "finish",
            lua.create_function(move |_lua, outcome: String| {
                let mut guard = slot.lock();
                let mut recorder = guard
                    .take()
                    .ok_or_else(|| record_error("no recording is running"))?;
                let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
                recorder
                    .finish(
                        tick,
                        &outcome,
                        Some(&workspace),
                        factorio_bot_core::record::DEFAULT_KEEP,
                    )
                    .map_err(record_error)?;
                Ok(recorder.run_id().to_string())
            })?,
        )?;
    }

    Ok(map_table)
}

/// A run id that sorts chronologically and does not collide within a second.
///
/// Seconds give the ordering, sub-second nanos give the distinctness. No
/// randomness: two runs cannot start in the same nanosecond in one process, and
/// a random salt would make the id unreproducible for no gain here.
fn mint_run_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("run-{}-{:05}", now.as_secs(), now.subsec_nanos() % 100_000)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use factorio_bot_core::factorio::rcon::FactorioRcon;
    use factorio_bot_core::record::RunRecorder;
    use factorio_bot_core::record::map::MapRecord;

    /// A sandboxed Lua with `record` installed, already recording into a
    /// throwaway directory -- reaching that state without a live game, by
    /// seeding the slot directly rather than going through `record.start()`
    /// (which cannot succeed here: it always makes a real RCON call).
    ///
    /// Returns the temp dir (kept alive for the caller) and the recorder's
    /// own directory, so a test can read `map.jsonl` back off disk.
    fn recording_lua() -> (Lua, tempfile::TempDir, std::path::PathBuf) {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let tmp = tempfile::tempdir().expect("tempdir");
        let recorder = RunRecorder::start(tmp.path(), "run-1").expect("recorder starts");
        let run_dir = recorder.dir().to_path_buf();
        let slot: Slot = Arc::new(Mutex::new(Some(recorder)));

        let table = create_lua_record_with_slot(
            &lua,
            Arc::new(FactorioRcon::new_empty()),
            Arc::new(FactorioWorld::new()),
            tmp.path().join("scripts"),
            vec![],
            slot,
        )
        .expect("record table");
        lua.globals().set("record", table).expect("install");
        (lua, tmp, run_dir)
    }

    fn read_map(dir: &std::path::Path) -> Vec<MapRecord> {
        std::fs::read_to_string(dir.join("map.jsonl"))
            .map(|text| {
                text.lines()
                    .map(|line| {
                        factorio_bot_core::serde_json::from_str(line).expect("valid map record")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn read_events(dir: &std::path::Path) -> Vec<EventKind> {
        factorio_bot_core::record::read_events(&dir.join("events.jsonl"))
            .expect("events.jsonl readable")
            .events
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }

    // ------------------------------------------------------------- plan_created

    #[test]
    fn plan_created_derives_steps_makespan_and_bots_from_the_plan_itself() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 2, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
                { id = 2, bot = 1, action = "smelt 10 iron-plate", deps = { 1 },
                  planned_start = 300, planned_duration = 600 },
            })
            "#,
        )
        .exec()
        .expect("plan_created runs");

        let events = read_events(&run_dir);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EventKind::PlanCreated {
                milestone_index,
                steps,
                makespan,
                bots,
                plan,
            } => {
                assert_eq!(*milestone_index, 1);
                assert_eq!(*steps, 2);
                assert_eq!(
                    *makespan, 900,
                    "the latest planned_start + planned_duration"
                );
                assert_eq!(*bots, vec![1, 2], "distinct bots, ascending");
                assert_eq!(plan[0].id, 1);
                assert_eq!(plan[0].deps, Vec::<u32>::new());
                assert_eq!(plan[1].deps, vec![1]);
                assert_eq!(plan[1].action, "smelt 10 iron-plate");
            }
            other => panic!("expected plan_created, got {other:?}"),
        }
    }

    #[test]
    fn plan_created_treats_a_step_with_no_deps_key_as_an_empty_list() {
        // `deps` is read by type, not by truthiness -- a step that never had
        // the key set at all (a plain Lua table missing a key, not a
        // Rust->Lua `Option::None`) must not raise, and must not silently
        // become truthy either.
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 1, action = "walk", planned_start = 0, planned_duration = 10 },
            })
            "#,
        )
        .exec()
        .expect("plan_created runs without a deps key");

        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::PlanCreated { plan, .. } => assert_eq!(plan[0].deps, Vec::<u32>::new()),
            other => panic!("expected plan_created, got {other:?}"),
        }
    }

    #[test]
    fn plan_created_of_an_empty_plan_is_zero_steps_and_zero_makespan() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load("record.plan_created(3, {})")
            .exec()
            .expect("plan_created runs");
        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::PlanCreated {
                milestone_index,
                steps,
                makespan,
                bots,
                plan,
            } => {
                assert_eq!(*milestone_index, 3);
                assert_eq!(*steps, 0);
                assert_eq!(*makespan, 0);
                assert!(bots.is_empty());
                assert!(plan.is_empty());
            }
            other => panic!("expected plan_created, got {other:?}"),
        }
    }

    // ------------------------------------------------------- milestone_satisfied

    #[test]
    fn milestone_satisfied_accepts_both_documented_reasons() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.milestone_satisfied(1, 0, "already_satisfied")
            record.milestone_satisfied(2, 3, "plan_empty")
            "#,
        )
        .exec()
        .expect("both reasons are accepted");

        let events = read_events(&run_dir);
        let reasons: Vec<SatisfiedReason> = events
            .iter()
            .map(|e| match e {
                EventKind::MilestoneSatisfied { reason, .. } => *reason,
                other => panic!("expected milestone_satisfied, got {other:?}"),
            })
            .collect();
        assert_eq!(
            reasons,
            vec![
                SatisfiedReason::AlreadySatisfied,
                SatisfiedReason::PlanEmpty,
            ]
        );
    }

    #[test]
    fn milestone_satisfied_refuses_an_unrecognised_reason_rather_than_guessing_unknown() {
        // `SatisfiedReason::Unknown` means "recorded before this field
        // existed" -- a live caller passing garbage must be told so, not
        // quietly folded into that meaning.
        let (lua, _tmp, _run_dir) = recording_lua();
        let err = lua
            .load(r#"record.milestone_satisfied(1, 0, "who_knows")"#)
            .exec()
            .expect_err("an unrecognised reason must raise");
        let message = err.to_string();
        assert!(message.contains("who_knows"), "{message}");
    }

    // ----------------------------------------------------------- milestone_stuck

    #[test]
    fn milestone_stuck_records_the_error_text_and_best_steps_it_was_given() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(r#"record.milestone_stuck(1, "stuck", "boom: something broke", 7)"#)
            .exec()
            .expect("milestone_stuck runs");

        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::MilestoneStuck {
                index,
                outcome,
                last_error,
                best_steps,
            } => {
                assert_eq!(*index, 1);
                assert_eq!(outcome, "stuck");
                assert_eq!(last_error.as_deref(), Some("boom: something broke"));
                assert_eq!(*best_steps, Some(7));
            }
            other => panic!("expected milestone_stuck, got {other:?}"),
        }
    }

    #[test]
    fn milestone_stuck_with_nothing_passed_writes_nulls_not_a_guess() {
        // A caller that does not know the error text or the best step count
        // must still be able to say so -- and null must keep meaning "not
        // known", not silently become "nothing happened" (`Some` of some
        // default) or fail outright for omitting an optional argument.
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(r#"record.milestone_stuck(2, "exhausted")"#)
            .exec()
            .expect("milestone_stuck runs with only the required arguments");

        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::MilestoneStuck {
                index,
                outcome,
                last_error,
                best_steps,
            } => {
                assert_eq!(*index, 2);
                assert_eq!(outcome, "exhausted");
                assert_eq!(*last_error, None);
                assert_eq!(*best_steps, None);
            }
            other => panic!("expected milestone_stuck, got {other:?}"),
        }
    }

    // -------------------------------------------------------- failure classification

    /// `record.actions`' shared step script, parameterised on `status` and
    /// `error` so each classification case is one call rather than a fresh
    /// literal.
    fn settled_step_script(status: &str, error_lua: &str) -> String {
        format!(
            r#"
            local steps = {{ {{ id = 1, bot = 1, label = "act" }} }}
            local actions = {{
                [1] = {{ status = "{status}", dispatched_tick = 10, replied_tick = 20,
                         error = {error_lua} }},
            }}
            record.actions(steps, actions)
            "#
        )
    }

    fn recorded_failure(status: &str, error_lua: &str) -> Option<ActionFailure> {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(settled_step_script(status, error_lua))
            .exec()
            .expect("record.actions runs");
        let events = read_events(&run_dir);
        // events[0] is the dispatch; the settle -- and its `failure` -- is
        // the second line `record.actions` writes for one action with both
        // ticks present.
        match &events[1] {
            EventKind::ActionSettled { failure, .. } => failure.clone(),
            other => panic!("expected action_settled, got {other:?}"),
        }
    }

    #[test]
    fn a_successful_settle_carries_no_failure() {
        assert_eq!(recorded_failure("success", "nil"), None);
    }

    #[test]
    fn an_unclassified_failure_falls_back_to_other_rather_than_none() {
        assert_eq!(
            recorded_failure("failed", r#""something nobody has seen before""#),
            Some(ActionFailure {
                kind: FailureKind::Other,
                detail: None,
            })
        );
    }

    #[test]
    fn a_timeout_is_classified_from_the_no_verdict_wording() {
        assert_eq!(
            recorded_failure(
                "failed",
                r#""the game reported no readable outcome: no action result received in time""#
            ),
            Some(ActionFailure {
                kind: FailureKind::Timeout,
                detail: None,
            })
        );
    }

    #[test]
    fn a_blocked_placement_is_classified_from_the_mods_own_wording() {
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: player still blocks placement""#
            ),
            Some(ActionFailure {
                kind: FailureKind::Blocked,
                detail: None,
            })
        );
    }

    #[test]
    fn a_missing_item_is_classified_and_names_the_item_in_detail() {
        assert_eq!(
            recorded_failure(
                "failed",
                r#""cannot place item 'iron-plate' because the player 'bot1' does not have any""#
            ),
            Some(ActionFailure {
                kind: FailureKind::MissingItem,
                detail: Some("iron-plate".into()),
            })
        );
    }

    #[test]
    fn an_otherwise_unclassified_rejection_is_still_rejected() {
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: cannot insert to inventory of nonexisting entity""#
            ),
            Some(ActionFailure {
                kind: FailureKind::Rejected,
                detail: None,
            })
        );
    }

    /// The shape `goal.run`'s `build_observation` produces for one action,
    /// standing in for the real thing so this test does not need a whole
    /// executor run to prove `record.actions` consumes it correctly. `drift`
    /// is a Lua expression spliced in verbatim, so a caller can pass `"nil"`
    /// or a table literal.
    fn placed_step_script(
        id: u32,
        bot: u32,
        dispatched: u64,
        replied: u64,
        drift_lua: &str,
    ) -> String {
        format!(
            r#"
            local steps = {{ {{ id = {id}, bot = {bot}, label = "place" }} }}
            local actions = {{
                [{id}] = {{
                    status = "success",
                    dispatched_tick = {dispatched},
                    replied_tick = {replied},
                    placed = {{
                        intent = {{ name = "stone-furnace",
                                    position = {{ x = -12.0, y = 8.0 }}, direction = 0 }},
                        actual = {{ name = "stone-furnace",
                                    position = {{ x = -12.0, y = 8.5 }}, direction = 0 }},
                        drift = {drift_lua},
                    }},
                }},
            }}
            return record.actions(steps, actions)
            "#
        )
    }

    #[test]
    fn a_placement_observed_by_the_executor_writes_a_placed_line_with_drift() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(placed_step_script(7, 3, 100, 120, r#"{ "position" }"#))
            .eval()
            .expect("record.actions runs");
        assert_eq!(written, 2, "one dispatched event and one settled event");

        let lines = read_map(&run_dir);
        assert_eq!(lines.len(), 1, "exactly one placed line");
        assert_eq!(
            lines[0].tick, 120,
            "stamped with the settle tick, not the dispatch tick"
        );
        match &lines[0].kind {
            MapKind::Placed {
                bot,
                intent,
                actual,
                drift,
            } => {
                assert_eq!(*bot, 3);
                assert_eq!(intent.position, Position::new(-12.0, 8.0));
                assert_eq!(actual.position, Position::new(-12.0, 8.5));
                assert_eq!(*drift, Some(vec!["position".to_string()]));
            }
            other => panic!("expected placed, got {other:?}"),
        }
    }

    #[test]
    fn a_placement_the_game_honoured_exactly_writes_drift_as_none_not_an_empty_list() {
        // `None` and `Some(vec![])` are not the same fact -- one says "the
        // game never disagreed with this placement" and the other would say
        // "the game agreed on every field it was asked to compare", which is
        // subtly different and not what `drift_between` ever produces. The
        // wire format must keep them apart too.
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(placed_step_script(1, 9, 200, 240, "nil"))
            .exec()
            .expect("record.actions runs");

        let lines = read_map(&run_dir);
        assert_eq!(lines.len(), 1);
        match &lines[0].kind {
            MapKind::Placed { drift, .. } => assert_eq!(*drift, None),
            other => panic!("expected placed, got {other:?}"),
        }

        // And the line on disk really is `null`, not `[]`: parsing back
        // through the same type is not enough to catch a writer that
        // serialised the wrong Rust value into the right shape.
        let raw = std::fs::read_to_string(run_dir.join("map.jsonl")).unwrap();
        assert!(
            raw.contains(r#""drift":null"#),
            "expected a literal null, got: {raw}"
        );
    }

    #[test]
    fn an_action_with_no_placed_field_writes_no_map_line() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            local steps = { { id = 1, bot = 1, label = "mine" } }
            local actions = {
                [1] = { status = "success", dispatched_tick = 10, replied_tick = 20 },
            }
            record.actions(steps, actions)
            "#,
        )
        .exec()
        .expect("record.actions runs");

        assert!(
            read_map(&run_dir).is_empty(),
            "an action that placed nothing must not appear in map.jsonl"
        );
    }

    #[test]
    fn keyframe_relevant_admits_only_what_the_entity_graph_models() {
        // Built structures the bots actually place, one per `entity_tree`
        // arm in `EntityGraph::add`.
        for entity_type in [
            "furnace",
            "inserter",
            "boiler",
            "lab",
            "offshore-pump",
            "mining-drill",
            "storage-tank",
            "container",
            "splitter",
            "transport-belt",
            "underground-belt",
            "pipe",
            "pipe-to-ground",
            "logistic-container",
            "assembling-machine",
        ] {
            assert!(
                keyframe_relevant(entity_type, "some-entity"),
                "{entity_type} is one of EntityGraph::add's tracked types"
            );
        }
        // Ore, tracked via `resource_tree` and surfaced by `snapshot_within`.
        assert!(keyframe_relevant("resource", "iron-ore"));
        // The two named rocks `add` also blocks on, regardless of type.
        assert!(keyframe_relevant("simple-entity", "rock-big"));
        assert!(keyframe_relevant("simple-entity", "rock-huge"));

        // Terrain nobody placed and nothing in EntityGraph tracks: trees,
        // small rocks, the player character, items dropped on the ground.
        assert!(!keyframe_relevant("tree", "tree-01"));
        assert!(!keyframe_relevant("simple-entity", "rock-small"));
        assert!(!keyframe_relevant("character", "character"));
        assert!(!keyframe_relevant("item-entity", "item-on-ground"));
    }
}
