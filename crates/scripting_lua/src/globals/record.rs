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

use super::position_from_lua;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::record::map::{
    Divergence, EntitySnapshot, MapKind, MapRecord, Placement, bounds_around, divergence_between,
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

/// Queries the live game and the world model within `bounds` and reports
/// where they diverge.
///
/// Shared by `record.keyframe()` (bounds from placements so far) and the
/// keyframe `record.start()` writes at the run's very own opening (bounds
/// from the bots' positions) -- factored out so the two can never disagree
/// about what counts as "relevant" or how a divergence is computed, which
/// they would if each grew its own copy of this logic.
async fn keyframe_snapshot(
    rcon: &factorio_bot_core::factorio::rcon::FactorioRcon,
    world: &FactorioWorld,
    bounds: &factorio_bot_core::record::map::Bounds,
) -> LuaResult<(Vec<EntitySnapshot>, Vec<EntitySnapshot>, Vec<Divergence>)> {
    let rect = Rect::new(
        &Position::new(bounds.left, bounds.top),
        &Position::new(bounds.right, bounds.bottom),
    );
    let game_entities = rcon
        .find_entities_filtered(&AreaFilter::Rect(rect.clone()), None, None)
        .await
        .map_err(rcon_error)?;
    // Restricted to what `EntityGraph` models -- see `keyframe_relevant` --
    // so `game` and `model` are comparable populations rather than the
    // unfiltered box (trees, rocks, ore, characters, dropped items) against
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
    Ok((game, model, divergence))
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

/// Reads the two counts and the item out of a partial transfer, or `None` if
/// this error is not one.
///
/// BotBridge does not report transfer *results*; it reports complaints, and a
/// transfer that moved less than asked complains in one of three wordings
/// (`mods/BotBridge/control.lua`, `rcon_insert_to_inventory` /
/// `rcon_remove_from_inventory`):
///
/// ```text
/// tried to remove 20 iron-plate but removed 18
/// tried to insert 20x iron-plate but inserted 18
/// cannot insert 20x iron-plate, because player #1 only has 18. clamping...
/// ```
///
/// All three reach here wrapped in `game rejected the command: Unexpected
/// Response: [...]`, which is why this is matched on the inner wording. The
/// first two are checked before the third because they describe what the
/// transfer *did*, while the clamp line describes what it decided to attempt;
/// a run that clamped and then fell short again prints both, and the transfer
/// line is the one whose numbers are the outcome.
///
/// Parsed rather than pattern-matched wholesale so a wording change costs a
/// `None` -- the failure then classifies as [`FailureKind::Rejected`] and the
/// whole message is still in `error` for a person -- rather than a wrong
/// number. Nothing here invents a count: both must parse as integers or this
/// declines.
fn partial_transfer_detail(error: &str) -> Option<String> {
    fn digits(text: &str) -> Option<u64> {
        let head: &str = text.split(|c: char| !c.is_ascii_digit()).next()?;
        head.parse().ok()
    }
    fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str)> {
        text.split_once(open)?.1.split_once(close)
    }

    // `tried to remove <asked> <item> but removed <moved>`
    if let Some((head, tail)) = between(error, "tried to remove ", " but removed ")
        && let Some((asked, item)) = head.split_once(' ')
        && let (Some(asked), Some(moved)) = (digits(asked), digits(tail))
    {
        return Some(format!("moved {moved} of {asked} {item}"));
    }
    // `tried to insert <asked>x <item> but inserted <moved>`
    if let Some((head, tail)) = between(error, "tried to insert ", " but inserted ")
        && let Some((asked, item)) = head.split_once("x ")
        && let (Some(asked), Some(moved)) = (digits(asked), digits(tail))
    {
        return Some(format!("moved {moved} of {asked} {item}"));
    }
    // `cannot insert <asked>x <item>, because player #<id> only has <moved>.
    //  clamping...` -- the insert did happen, at the clamped count.
    if let Some((head, tail)) = between(error, "cannot insert ", ", because ")
        && tail.contains("clamping")
        && let Some((asked, item)) = head.split_once("x ")
        && let Some(have) = tail.split_once("only has ")
        && let (Some(asked), Some(moved)) = (digits(asked), digits(have.1))
    {
        return Some(format!("moved {moved} of {asked} {item}"));
    }
    None
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
///
/// [`FailureKind::PartialTransfer`] is checked *before*
/// [`FailureKind::Rejected`] and must stay there: a partial transfer arrives
/// wrapped in `game rejected the command`, so testing the outer wording first
/// would swallow every one of them into `Rejected` and throw the counts away.
fn classify_failure(error: &str) -> ActionFailure {
    // Checked ahead of the coarse kinds because it produces its own detail and
    // its text satisfies `Rejected`'s match as well.
    if let Some(detail) = partial_transfer_detail(error) {
        return ActionFailure {
            kind: FailureKind::PartialTransfer,
            detail: Some(detail),
        };
    }
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
    // Shared by `record.keyframe()` and `record.finish()`: both ingest
    // samples out of this same workspace, alongside the frame/keyframe work
    // each already does at exactly these two moments.
    let workspace = scripts_root
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| record_error("scripts directory has no parent workspace"))?;

    map_table.set(
        "__doc_entry_start",
        String::from(
            r#"
--- starts recording a run
-- Mints a run id, creates `<workspace>/runs/<id>/`, and starts frame capture
-- with that same id -- one call, so the log and the frames cannot disagree
-- about which run they belong to.
--
-- Also writes an opening keyframe to `map.jsonl`, bounded by the connected
-- bots' own positions (plus a margin) rather than by placements, since
-- nothing has been placed yet -- so a run that crashes before its first
-- placement still leaves a map behind. Writes nothing if no bots are
-- connected yet.
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
        let world = world.clone();
        let runs_root = runs_root.clone();
        let bots: Vec<u32> = all_bots.iter().map(|id| u32::from(*id)).collect();
        map_table.set(
            "start",
            lua.create_async_function(move |_lua, ()| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let world = world.clone();
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
                    let opened_at = opened_at.unwrap_or_else(|| rcon.last_tick().unwrap_or(0));

                    let mut recorder =
                        RunRecorder::start(&runs_root, run_id.clone()).map_err(record_error)?;
                    recorder
                        .record(
                            opened_at,
                            EventKind::RunStarted {
                                run_id: run_id.clone(),
                                bots,
                                seed: None,
                                factorio: None,
                                git: None,
                            },
                        )
                        .map_err(record_error)?;

                    // A keyframe at the run's own opening, not only at
                    // milestone boundaries. `record.keyframe()`'s bounds come
                    // from placements so far, and at this instant nothing has
                    // been placed -- that source is empty for the entire run
                    // if it dies before its first placement, which is exactly
                    // the run whose map a diagnosis most wants to open. Bot
                    // positions are the one thing that reliably exists this
                    // early: every connected bot has a character with a real
                    // position, and `bounds_around` widens the box around
                    // them by the same 16-tile margin `record.keyframe()`
                    // uses, so the two keyframes stay directly comparable.
                    //
                    // An empty roster (a script that did not wait for one)
                    // writes no keyframe here, exactly like `record.keyframe()`
                    // writing nothing when nothing has been placed -- both are
                    // "no bounds to draw yet", not a failure. A genuine
                    // failure to reach the game or read the model is left to
                    // raise: `frame_capture_start` two calls above already
                    // proved the game is reachable, so a failure past that
                    // point is a real defect, not a mundane timing gap, and
                    // must not be swallowed into a run that silently starts
                    // with no opening keyframe and no explanation why.
                    let players = rcon
                        .as_ref()
                        .connected_players()
                        .await
                        .map_err(rcon_error)?;
                    if let Some(bounds) =
                        bounds_around(players.into_iter().map(|p| p.position), 16.0)
                    {
                        let (game, model, divergence) =
                            keyframe_snapshot(&rcon, &world, &bounds).await?;
                        recorder
                            .record_map(MapRecord {
                                tick: opened_at,
                                kind: MapKind::Keyframe {
                                    bounds,
                                    game,
                                    model,
                                    divergence,
                                },
                            })
                            .map_err(record_error)?;
                    }

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
--
-- The dispatch also carries `target`, when the action has one: the tile or
-- position the plan sent the bot to, straight from the plan rather than
-- anything the game reported back. `nil` for a `craft`/`research` action,
-- which act nowhere in particular -- never for a `mine`/`place`/`insert`/
-- `remove` action just because nothing was observed about it yet.
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
                    // `nil` here means "this action has no target" -- `build_observation`
                    // (`crates/scripting_lua/src/globals/goal/run.rs`) only omits the key
                    // for `craft`/`research`, which really do act on no location, never
                    // for a `mine`/`place`/`insert`/`remove` it merely has not observed
                    // anything about yet. It is the planner's intent, not the game's
                    // answer -- see `ActionKind::target_position`'s doc for why. Read
                    // field-by-field via `position_from_lua`, not `lua.to_value`/
                    // `from_value`, so a resource tile's half-tile centre passes through
                    // exactly as the plan set it rather than being rounded by a serde
                    // bridge on the way.
                    let target: Option<LuaTable> = observed.get("target")?;
                    let target = target
                        .as_ref()
                        .map(|t| position_from_lua(t, "target"))
                        .transpose()?;

                    if let Some(dispatched) = dispatched {
                        recorder
                            .record(
                                dispatched,
                                EventKind::ActionDispatched {
                                    id,
                                    bot,
                                    action: label,
                                    target,
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
        "__doc_entry_teleports",
        String::from(
            r#"
--- flushes any `player.teleport` calls the mod has reported since the last flush
-- `control.lua` teleports a bot in three places -- a stuck walk leg timing
-- out, and two sites that move a bot clear of a ghost's or a blueprint's
-- bounding box before reviving it -- and none of them are otherwise visible
-- here: `on_player_changed_position` fires identically for a teleport and an
-- ordinary walked step. Each is queued as it is parsed with the real game
-- tick it happened at, so calling this late does not blur *when* a teleport
-- occurred, only when it gets written. Call it once per loop iteration
-- (alongside `record.actions`) so nothing is left unflushed for long.
-- @treturn number how many teleport events were written
-- @raise if no recording is running
function record.teleports()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        map_table.set(
            "teleports",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                for (tick, event) in world.drain_teleports() {
                    let tick = recorder.not_before(tick);
                    recorder
                        .record(
                            tick,
                            EventKind::Teleport {
                                bot: u32::from(event.player_id),
                                reason: event.reason,
                                from: event.from,
                                to: event.to,
                                distance: event.distance,
                                action_id: event.action_id,
                            },
                        )
                        .map_err(record_error)?;
                    written += 1;
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
-- Also ingests whatever the mod has written to `samples.jsonl` since the last
-- time this (or `record.finish()`) ran, the same way `record.finish()` does --
-- this is the other of the two moments the design calls for, so a run killed
-- before it ever reaches `record.finish()` still keeps everything through its
-- last closed milestone instead of losing the whole sample stream.
--
-- Returns `false`, and writes no keyframe, for either of two unremarkable
-- cases: no recording is running, or one is but nothing has been placed yet
-- (a keyframe over a box nothing has ever occupied is not a fact worth
-- recording) -- sample ingestion still runs in the second case, since it does
-- not depend on anything having been placed. Both are expected outcomes of
-- calling this from a place that does not know whether recording is active,
-- so both are a return value, not an error -- a caller that wants to tell
-- them apart still can, since only the second follows a successful
-- `record.start()`. Anything else going wrong (the game cannot be reached, a
-- malformed response, an unrecognised sample schema) still raises, because
-- that is not a "nothing to do here" outcome and must not look like one.
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
        let workspace = workspace.clone();
        map_table.set(
            "keyframe",
            lua.create_async_function(move |_lua, ()| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let world = world.clone();
                let workspace = workspace.clone();
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
                        let mut guard = slot.lock();
                        let Some(recorder) = guard.as_mut() else {
                            return Ok(false);
                        };
                        // Ingested here, ahead of the placed-bounds check
                        // below: unlike a keyframe, a sample line does not
                        // depend on anything having been placed, so a
                        // milestone with no placements yet (a pure-research
                        // one, say) still gets this boundary's samples
                        // archived even though it writes no keyframe.
                        recorder
                            .ingest_samples(Some(&workspace))
                            .map_err(record_error)?;
                        recorder.placed_bounds(16.0)
                    };
                    let Some(bounds) = bounds else {
                        return Ok(false);
                    };

                    let (game, model, divergence) =
                        keyframe_snapshot(&rcon, &world, &bounds).await?;

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
-- run are left where they are. Also catches up on any samples the mod wrote
-- since the last `record.keyframe()` call (or all of them, if this run never
-- reached one) -- the same incremental ingestion `record.keyframe()` runs at
-- every milestone boundary, so nothing is read or archived twice.
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
        let workspace = workspace.clone();
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
    use factorio_bot_core::factorio::world::TeleportEvent;
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
    fn a_partial_removal_carries_both_counts_rather_than_reading_as_a_rejection() {
        // The failure that stuck rung 4 of `run-1788320177-77989`. A rejection
        // means nothing moved; this means 18 plates are in the bot's hands and
        // 2 are not, and the two must not share a kind. The counts are the
        // point: "it failed" cannot be replanned against.
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: Unexpected Response: [\"tried to remove 20 iron-plate but removed 18\"]""#
            ),
            Some(ActionFailure {
                kind: FailureKind::PartialTransfer,
                detail: Some("moved 18 of 20 iron-plate".into()),
            })
        );
    }

    #[test]
    fn a_partial_insert_is_read_the_same_way_from_the_other_wording() {
        // `rcon_insert_to_inventory`'s complaint puts an `x` after the count
        // where `rcon_remove_from_inventory`'s does not, which is exactly the
        // sort of difference a single substring match gets wrong.
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: Unexpected Response: [\"tried to insert 50x coal but inserted 12\"]""#
            ),
            Some(ActionFailure {
                kind: FailureKind::PartialTransfer,
                detail: Some("moved 12 of 50 coal".into()),
            })
        );
    }

    #[test]
    fn a_clamped_insert_is_a_partial_transfer_too() {
        // The mod clamps to what the player actually holds and then inserts
        // that. Items moved, just not as many as asked -- the same fact, said
        // in the mod's third wording.
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: Unexpected Response: [\"cannot insert 20x iron-ore, because player #1 only has 18. clamping...\"]""#
            ),
            Some(ActionFailure {
                kind: FailureKind::PartialTransfer,
                detail: Some("moved 18 of 20 iron-ore".into()),
            })
        );
    }

    #[test]
    fn a_transfer_complaint_with_no_readable_counts_falls_back_to_rejected() {
        // The classifier parses rather than pattern-matching, so a wording it
        // cannot read must cost a coarser kind and nothing else -- never an
        // invented number. The whole message is still in `error` beside this.
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: Unexpected Response: [\"tried to remove some iron-plate but removed fewer\"]""#
            ),
            Some(ActionFailure {
                kind: FailureKind::Rejected,
                detail: None,
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

    // ------------------------------------------------ action_dispatched target

    #[test]
    fn a_target_on_the_observed_action_reaches_the_dispatched_event() {
        // Drives the real `record.actions` path end to end: the Lua script
        // shapes `observation.actions` exactly the way `build_observation`
        // (`crates/scripting_lua/src/globals/goal/run.rs`) does, and this
        // checks what actually lands in `events.jsonl`, not just that some
        // conversion function agrees with itself.
        //
        // The position is a resource tile's real centre -- `(-40.5, -48.5)`,
        // never `(-41, -49)` -- so a regression that floors it anywhere on
        // the way from Lua to `EventKind::ActionDispatched` fails this test
        // rather than passing it by coincidence.
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            local steps = { { id = 1, bot = 2, label = "mine iron-ore" } }
            local actions = {
                [1] = {
                    status = "success",
                    dispatched_tick = 10,
                    target = { x = -40.5, y = -48.5 },
                },
            }
            record.actions(steps, actions)
            "#,
        )
        .exec()
        .expect("record.actions runs");

        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::ActionDispatched { target, .. } => {
                assert_eq!(*target, Some(Position::new(-40.5, -48.5)));
            }
            other => panic!("expected action_dispatched, got {other:?}"),
        }
    }

    #[test]
    fn an_observed_action_with_no_target_key_records_none_not_a_guess() {
        // `build_observation` never sets a `target` key for `craft`/
        // `research` -- this is that omission's shape on the Lua side, and it
        // must read back as `None`, never a zeroed or guessed position.
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            local steps = { { id = 1, bot = 2, label = "craft iron-gear-wheel" } }
            local actions = {
                [1] = { status = "success", dispatched_tick = 10 },
            }
            record.actions(steps, actions)
            "#,
        )
        .exec()
        .expect("record.actions runs");

        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::ActionDispatched { target, .. } => assert_eq!(*target, None),
            other => panic!("expected action_dispatched, got {other:?}"),
        }
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

    // ------------------------------------------------------------- teleports

    /// Drives the actual mod->core->Lua path, not just `record.teleports()`
    /// in isolation: a `writeout`-shaped line goes through
    /// `factorio_bot_core::process::output_parser::OutputParser` -- the same
    /// parser that reads BotBridge's real stdout -- into a `FactorioWorld`
    /// shared with the recording Lua sandbox, and only then is
    /// `record.teleports()` asked to drain it. A test that only exercised
    /// `record.teleports()` against a hand-built queue would leave the
    /// parser hop -- the actual gap this closes -- completely unverified.
    #[test]
    fn teleport_writeout_reaches_events_jsonl_through_the_real_parser() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let tmp = tempfile::tempdir().expect("tempdir");
        let recorder = RunRecorder::start(tmp.path(), "run-1").expect("recorder starts");
        let run_dir = recorder.dir().to_path_buf();
        let slot: Slot = Arc::new(Mutex::new(Some(recorder)));
        let world = Arc::new(FactorioWorld::new());

        let table = create_lua_record_with_slot(
            &lua,
            Arc::new(FactorioRcon::new_empty()),
            world.clone(),
            tmp.path().join("scripts"),
            vec![],
            slot,
        )
        .expect("record table");
        lua.globals().set("record", table).expect("install");

        let mut parser = factorio_bot_core::process::output_parser::OutputParser::with_world(world);
        parser
            .parse(
                12_345,
                "teleport",
                r#"{"player_id":1,"reason":"walk_stuck","from":{"x":1.0,"y":2.0},"to":{"x":41.0,"y":2.0},"distance":40.0,"action_id":7}"#,
            )
            .expect("teleport writeout parses");

        let written: u32 = lua
            .load("return record.teleports()")
            .eval()
            .expect("record.teleports() runs");
        assert_eq!(written, 1);

        let events = read_events(&run_dir);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EventKind::Teleport {
                bot,
                reason,
                from,
                to,
                distance,
                action_id,
            } => {
                assert_eq!(*bot, 1);
                assert_eq!(reason, "walk_stuck");
                assert_eq!(from.x, 1.0);
                assert_eq!(from.y, 2.0);
                assert_eq!(to.x, 41.0);
                assert_eq!(to.y, 2.0);
                assert_eq!(*distance, 40.0, "the mod's own distance, not recomputed");
                assert_eq!(
                    *action_id,
                    Some(7),
                    "the stuck-walk site carries an action id"
                );
            }
            other => panic!("expected teleport, got {other:?}"),
        }
    }

    /// `recording_lua()` builds its own `FactorioWorld` internally and does
    /// not hand it back, so this test can't push onto its queue -- it builds
    /// the same wiring `create_lua_record` does, just keeping the world
    /// around so it can call `record_teleport` directly. Complements
    /// `teleport_writeout_reaches_events_jsonl_through_the_real_parser`
    /// above (which is the one test that must go through the real parser)
    /// with what that test doesn't cover: reason/action_id fidelity across
    /// all three mod sites, write order, and that a second drain reports
    /// zero once the queue is actually empty.
    #[test]
    fn teleports_distinguishes_all_three_mod_sites_and_drains_the_queue() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let tmp = tempfile::tempdir().expect("tempdir");
        let recorder = RunRecorder::start(tmp.path(), "run-1").expect("recorder starts");
        let run_dir = recorder.dir().to_path_buf();
        let slot: Slot = Arc::new(Mutex::new(Some(recorder)));
        let world = Arc::new(FactorioWorld::new());

        let table = create_lua_record_with_slot(
            &lua,
            Arc::new(FactorioRcon::new_empty()),
            world.clone(),
            tmp.path().join("scripts"),
            vec![],
            slot,
        )
        .expect("record table");
        lua.globals().set("record", table).expect("install");

        world.record_teleport(
            100,
            TeleportEvent {
                player_id: 1,
                reason: "walk_stuck".to_string(),
                from: Position { x: 0.0, y: 0.0 },
                to: Position { x: 10.0, y: 0.0 },
                distance: 10.0,
                action_id: Some(3),
            },
        );
        world.record_teleport(
            101,
            TeleportEvent {
                player_id: 1,
                reason: "revive_ghost_blocked".to_string(),
                from: Position { x: 10.0, y: 0.0 },
                to: Position { x: 11.0, y: 1.0 },
                distance: 1.414,
                action_id: None,
            },
        );
        world.record_teleport(
            102,
            TeleportEvent {
                player_id: 2,
                reason: "place_blueprint_blocked".to_string(),
                from: Position { x: 5.0, y: 5.0 },
                to: Position { x: 6.0, y: 6.0 },
                distance: 1.414,
                action_id: None,
            },
        );

        let written: u32 = lua
            .load("return record.teleports()")
            .eval()
            .expect("record.teleports() drains all three");
        assert_eq!(written, 3);

        let events = read_events(&run_dir);
        let reasons: Vec<String> = events
            .iter()
            .map(|e| match e {
                EventKind::Teleport { reason, .. } => reason.clone(),
                other => panic!("expected teleport, got {other:?}"),
            })
            .collect();
        assert_eq!(
            reasons,
            vec![
                "walk_stuck",
                "revive_ghost_blocked",
                "place_blueprint_blocked"
            ],
            "written in the order they were queued"
        );

        match &events[1] {
            EventKind::Teleport { action_id, bot, .. } => {
                assert_eq!(
                    *action_id, None,
                    "the ghost/blueprint sites have no dispatched action to attach to"
                );
                assert_eq!(*bot, 1);
            }
            other => panic!("expected teleport, got {other:?}"),
        }
        match &events[2] {
            EventKind::Teleport { bot, .. } => assert_eq!(*bot, 2),
            other => panic!("expected teleport, got {other:?}"),
        }

        let written_again: u32 = lua
            .load("return record.teleports()")
            .eval()
            .expect("record.teleports() runs on an empty queue");
        assert_eq!(
            written_again, 0,
            "the queue was actually drained, not just read"
        );
    }
}
