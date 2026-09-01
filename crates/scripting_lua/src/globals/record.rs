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

use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::record::{EventKind, RunRecorder};
use factorio_bot_core::types::PlayerId;
use std::path::PathBuf;
use std::sync::Arc;

type Slot = Arc<Mutex<Option<RunRecorder>>>;

fn record_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("record: {err}"))
}

fn rcon_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("rcon: {err}"))
}

/// Stamps an event with the game's own clock, as last observed.
fn tick_of(rcon: &factorio_bot_core::factorio::rcon::FactorioRcon) -> u64 {
    rcon.last_tick().unwrap_or(0)
}

fn with_recorder<T>(
    slot: &Slot,
    f: impl FnOnce(&mut RunRecorder) -> std::io::Result<T>,
) -> LuaResult<T> {
    let mut guard = slot.lock();
    let recorder = guard
        .as_mut()
        .ok_or_else(|| record_error("no recording is running -- call record.start() first"))?;
    f(recorder).map_err(record_error)
}

pub fn create_lua_record(
    lua: &Lua,
    rcon: Arc<factorio_bot_core::factorio::rcon::FactorioRcon>,
    scripts_root: PathBuf,
    all_bots: Vec<PlayerId>,
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

    let slot: Slot = Arc::new(Mutex::new(None));

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
                    let mut recorder =
                        RunRecorder::start(&runs_root, run_id.clone()).map_err(record_error)?;
                    recorder
                        .record(
                            tick_of(&rcon),
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

                    // Same id to the mod, so the sidecar it writes names this run.
                    rcon.as_ref()
                        .frame_capture_start(Some(run_id.clone()))
                        .await
                        .map_err(rcon_error)?;
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
                with_recorder(&slot, |recorder| {
                    recorder.record(tick_of(&rcon), EventKind::MilestoneStarted { index, goal })
                })
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
                let tick = tick_of(&rcon);
                with_recorder(&slot, |recorder| {
                    recorder.record(
                        tick,
                        EventKind::MilestoneSatisfied {
                            index,
                            iterations,
                            elapsed_ticks: 0,
                        },
                    )
                })
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
                let tick = tick_of(&rcon);
                with_recorder(&slot, |recorder| {
                    recorder.record(
                        tick,
                        EventKind::MilestoneStuck {
                            index,
                            outcome: outcome.clone(),
                            best_steps: None,
                            last_error: None,
                        },
                    )
                })
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
                let tick = tick_of(&rcon);
                let mut guard = slot.lock();
                let mut recorder = guard
                    .take()
                    .ok_or_else(|| record_error("no recording is running"))?;
                recorder
                    .finish(tick, &outcome, Some(&workspace))
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
