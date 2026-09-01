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
use factorio_bot_core::record::{EventKind, RunRecorder, SatisfiedReason};
use factorio_bot_core::types::{AreaFilter, PlayerId, Position, Rect};
use std::path::PathBuf;
use std::sync::Arc;

type Slot = Arc<Mutex<Option<RunRecorder>>>;

fn record_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("record: {err}"))
}

fn rcon_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("rcon: {err}"))
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
-- @raise if no recording is running
function record.milestone_satisfied(index, iterations)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "milestone_satisfied",
            lua.create_function(move |_lua, (index, iterations): (u32, u32)| {
                record_live(
                    &slot,
                    &rcon,
                    EventKind::MilestoneSatisfied {
                        index,
                        iterations,
                        // The supervisor does not yet tell this binding
                        // *why* -- that is `record.milestone_satisfied`'s
                        // third argument, added once the caller in
                        // `supervisor.lua` has the answer. Until then this
                        // is the honest value: not a guess at either real
                        // reason.
                        reason: SatisfiedReason::Unknown,
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
-- @number index the milestone's position in the run, from 1
-- @string outcome why it was abandoned
-- @raise if no recording is running
function record.milestone_stuck(index, outcome)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "milestone_stuck",
            lua.create_function(move |_lua, (index, outcome): (u32, String)| {
                record_live(
                    &slot,
                    &rcon,
                    EventKind::MilestoneStuck {
                        index,
                        outcome,
                        best_steps: None,
                        last_error: None,
                    },
                )
            })?,
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
                        recorder
                            .record(
                                replied,
                                EventKind::ActionSettled {
                                    id,
                                    bot,
                                    status,
                                    elapsed_ticks: dispatched.map(|d| replied.saturating_sub(d)),
                                    error,
                                    // The observation this joins against
                                    // carries only a status string and an
                                    // error string today -- no classified
                                    // kind yet, so there is nothing honest to
                                    // put here but `None`.
                                    failure: None,
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
                    let game: Vec<EntitySnapshot> = game_entities
                        .into_iter()
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
}
