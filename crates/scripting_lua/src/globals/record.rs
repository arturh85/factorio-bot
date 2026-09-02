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
use super::rcon::frame_cameras_from_lua;
use factorio_bot_core::factorio::rcon::FrameCameras;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::record::map::{
    Divergence, EntitySnapshot, MapKind, MapRecord, Placement, bounds_around, divergence_between,
};
use factorio_bot_core::record::video::Resolution;
use factorio_bot_core::record::{
    ActionFailure, EventKind, FailureKind, PlannedStep, RunRecorder, SatisfiedReason, VideoOptions,
    VideoRecorder,
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
/// both sides describe the same population. An unfiltered `find_entities_filtered`
/// would return *everything* in the box: trees, small rocks, ore, characters,
/// items on the ground. None of those are things a bot places or something
/// `EntityGraph` tracks, so left in, they would dominate every keyframe's
/// `game` array with terrain nobody placed and swamp the actual divergences
/// underneath.
///
/// This mirrors `EntityGraph::add` (`crates/core/src/graph/entity_graph.rs`)
/// exactly: the entity types it inserts into `entity_tree`, the two named
/// rocks it also blocks on, and `Resource` (read back out of `resource_tree`
/// by `snapshot_within`, so ore patches are legitimately part of the model
/// side too). Keep the two lists in sync -- a type `add` starts tracking
/// without a matching arm here would show up as a permanent, spurious
/// divergence for every run that touches it.
///
/// [`keyframe_snapshot`] now also sends the type half of this filter to the
/// game via `find_entities_filtered`'s `type` parameter (see
/// `keyframe_relevant_types`), so this function's job today is narrower than
/// it used to be: the game already dropped every tree, fish and unit before
/// the reply left the server. What is left for this function to do is the
/// `simple-entity` tail -- the game's `type` filter cannot itself say "type
/// simple-entity AND name rock-big-or-rock-huge", so `keyframe_relevant_types`
/// asks for all of `simple-entity` (rocks, small and large) and this function
/// still has to pick the two big ones back out by name.
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

/// The `type` filter [`keyframe_snapshot`] sends to `find_entities_filtered`
/// so the game drops trees, fish, units, characters and dropped items before
/// the reply ever crosses RCON, instead of `keyframe_relevant` doing it here
/// after the whole box has already been serialised, sent and parsed.
///
/// Every type [`keyframe_relevant`] admits outright, plus `simple-entity` --
/// the game cannot filter that down to just `rock-big`/`rock-huge` by type
/// alone (its `type` and `name` filters narrow the *same* query rather than
/// offering alternatives), so `simple-entity` is asked for in full and
/// `keyframe_relevant` still does the by-name narrowing on what comes back.
/// That tail is measured to be small: a 473-chunk capture of this map's
/// starting area logged 326 `simple-entity` records against 10,542 `tree` and
/// 2,693 `resource` -- so admitting all of `simple-entity` costs a few hundred
/// records at most, where admitting no type filter at all would have cost
/// tens of thousands.
fn keyframe_relevant_types() -> Vec<String> {
    [
        EntityType::Furnace,
        EntityType::Inserter,
        EntityType::Boiler,
        EntityType::Lab,
        EntityType::OffshorePump,
        EntityType::MiningDrill,
        EntityType::StorageTank,
        EntityType::Container,
        EntityType::Splitter,
        EntityType::TransportBelt,
        EntityType::UndergroundBelt,
        EntityType::Pipe,
        EntityType::PipeToGround,
        EntityType::LogisticContainer,
        EntityType::AssemblingMachine,
        EntityType::Resource,
        EntityType::SimpleEntity,
    ]
    .iter()
    .map(|entity_type| entity_type.to_string())
    .collect()
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
        .find_entities_filtered(
            &AreaFilter::Rect(rect.clone()),
            None,
            Some(keyframe_relevant_types()),
        )
        .await
        .map_err(rcon_error)?;
    // Restricted to what `EntityGraph` models -- see `keyframe_relevant` --
    // so `game` and `model` are comparable populations rather than the
    // unfiltered box (trees, rocks, ore, characters, dropped items) against
    // the curated one. The type half of that restriction already happened on
    // the game's side (`keyframe_relevant_types`, above); this pass is what is
    // left: picking `rock-big`/`rock-huge` out of the `simple-entity`s the
    // game could not narrow any further by type alone.
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

/// Reads `record.start`'s `video` option.
///
/// `video = true` takes the defaults; `video = {resolution = "1080p", fps = 15,
/// client = 1}` overrides them; absent, `nil` and `false` all mean no video,
/// which is what every existing script says by saying nothing.
///
/// **An unknown resolution raises at `record.start()`, rather than falling back
/// to the default.** A run that quietly recorded at the wrong size is worse
/// than one that refused to start, and the refusal happens while somebody is
/// still watching the terminal. Same for a `video` that is neither a boolean
/// nor a table: guessing at `video = "true"` would be guessing at a typo.
fn video_options(options: Option<&LuaTable>) -> LuaResult<Option<VideoOptions>> {
    let Some(table) = options else {
        return Ok(None);
    };
    match table.get::<LuaValue>("video")? {
        LuaValue::Nil | LuaValue::Boolean(false) => Ok(None),
        LuaValue::Boolean(true) => Ok(Some(VideoOptions::default())),
        LuaValue::Table(video) => {
            let mut chosen = VideoOptions::default();
            if let Some(name) = video.get::<Option<String>>("resolution")? {
                chosen.resolution = Resolution::parse(&name).map_err(record_error)?;
            }
            if let Some(fps) = video.get::<Option<u32>>("fps")? {
                chosen.fps = fps;
            }
            if let Some(client) = video.get::<Option<u8>>("client")? {
                chosen.client = client;
            }
            Ok(Some(chosen))
        }
        other => Err(record_error(format!(
            "record.start: video must be a boolean or a table, got {}",
            other.type_name()
        ))),
    }
}

/// Reads `record.start`'s `frames` option.
///
/// **Absent means no screenshot camera**, which is what every existing script
/// says by saying nothing -- and which is the point: screenshots were retired
/// on 2026-09-02 and video is the visual record. The capture session itself is
/// started either way, because the mod's world-state samplers are gated on it;
/// this decides only what is rendered.
///
/// Delegates to the same reader `rcon.frame_capture_start` uses, so the two
/// entry points cannot come to disagree about what `true` or a list means.
fn frame_options(options: Option<&LuaTable>) -> LuaResult<FrameCameras> {
    let Some(table) = options else {
        return Ok(FrameCameras::None);
    };
    frame_cameras_from_lua(Some(table.get::<LuaValue>("frames")?)).map_err(|err| {
        record_error(format!(
            "record.start: {}",
            strip_prefix_once(&err.to_string())
        ))
    })
}

/// mlua renders a `RuntimeError` with its own prefix; this keeps the message
/// readable when it is wrapped a second time.
fn strip_prefix_once(message: &str) -> String {
    message
        .strip_prefix("runtime error: ")
        .unwrap_or(message)
        .to_string()
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
--
-- Video is the visual record. Screenshot cameras are RETIRED and a run
-- captures no frames unless it asks: one run wrote 2164 JPEGs / 947 MB of them
-- against 290 MB for the same 45 minutes of video, and take_screenshot renders
-- synchronously inside the game loop, once per camera, every 300 ticks. The
-- capture session still starts, because the world-state samples (research,
-- production, power, bot inventories) ride on it; only the rendering is off.
--
--     record.start({video = true})                        -- 720p, 15 fps, client 1
--     record.start({video = {resolution = "1080p"}})      -- for a final run worth the size
--     record.start({video = {client = 2, fps = 30}})      -- film a different client
--     record.start({frames = {"follow"}})                 -- one screenshot camera as well
--     record.start({frames = true})                       -- every camera, ~520 MB/hour each
--
-- What frames still buy, when you ask for them: a frame is tick-exact and
-- 1920x1080, named by the game itself, and it can say "nothing was captured
-- here". A video cannot -- it keeps writing the last drawn image while the
-- game stalls, which looks exactly like a game that was running and idle --
-- and its tick comes from a table the host built, interpolated between
-- samples.
--
-- `resolution` is "720p" (the default) or "1080p"; an unknown value raises
-- here rather than recording at the wrong size. `client` is which graphical
-- client's window to film, defaulting to 1 -- the one client every run with a
-- client at all has, so the same option means the same thing on a one-bot run
-- and a four-bot one. The camera is not steered: the video shows whatever that
-- client shows.
--
-- Video failing to start never fails the run. The run continues with frames,
-- and `video.json` records what went wrong.
-- @tparam[opt] table options `{video = ..., frames = ...}`; `frames` is `true`, a list of camera ids, or absent for none
-- @treturn string the run id
-- @raise if a recording is already running, the run directory cannot be created, or the video options are not understood
function record.start(options)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        let world = world.clone();
        let runs_root = runs_root.clone();
        let workspace = workspace.clone();
        let bots: Vec<u32> = all_bots.iter().map(|id| u32::from(*id)).collect();
        // `start` took no arguments until video existed. mlua reads an absent
        // argument as `None`, so every script that calls `record.start()` is
        // unaffected.
        map_table.set(
            "start",
            lua.create_async_function(move |_lua, options: Option<LuaTable>| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let world = world.clone();
                let runs_root = runs_root.clone();
                let workspace = workspace.clone();
                let bots = bots.clone();
                async move {
                    // Read before anything is created: a typo in the options is
                    // the one failure that should cost nothing, and refusing
                    // after minting a run id would leave a run directory behind
                    // for a run that never started.
                    let video = video_options(options.as_ref())?;
                    let frames = frame_options(options.as_ref())?;
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
                        .frame_capture_start(Some(run_id.clone()), frames)
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

                    if let Some(video) = video {
                        // Never fatal: `VideoRecorder::start` returns `Ok` with
                        // `status: failed` and a reason for every capture
                        // failure, and the run goes on with frames. Only an
                        // unwritable video directory is an `Err`, and that is a
                        // workspace that cannot be written to at all.
                        let capture = VideoRecorder::start(
                            &workspace,
                            &run_id,
                            video,
                            rcon.clone(),
                            Some(opened_at),
                        )
                        .await
                        .map_err(record_error)?;
                        recorder.attach_video(capture);
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
-- run's record shows what was planned, not only what happened. `steps` and
-- `makespan` are derived from `plan` itself rather than taken as separate
-- arguments, so neither can disagree with what was actually recorded: `steps`
-- is `plan`'s length and `makespan` the latest
-- `planned_start + planned_duration` across every entry (0 for an empty plan).
--
-- `bots` is the **run's roster**, not the bots the plan happens to name. It
-- was derived from the steps, which made a bot that got no work invisible in
-- the record -- a plan covering one bot out of four recorded `bots: [2]` and
-- read exactly like a run of one bot. "Why did bot 4 do nothing" is a question
-- the record has to be able to answer, and it cannot be asked of a field that
-- omits every bot it is about. The roster comes from the run itself, so a
-- script cannot pass a wrong one.
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
        // The run's roster, closed over rather than read off the plan. A bot
        // that got no step is exactly the bot a reader of this record is
        // asking about, and deriving the field from the steps deleted it.
        // Ascending and distinct, so the field's shape is unchanged.
        let roster: Vec<u32> = all_bots
            .iter()
            .map(|id| u32::from(*id))
            .collect::<BTreeSet<u32>>()
            .into_iter()
            .collect();
        map_table.set(
            "plan_created",
            lua.create_function(move |_lua, (index, plan): (u32, LuaTable)| {
                let mut steps: Vec<PlannedStep> = Vec::new();
                let mut makespan: u64 = 0;
                for step in plan.sequence_values::<LuaTable>() {
                    let planned = planned_step_from_lua(&step?)?;
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
                        bots: roster.clone(),
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
-- An `action_dispatched` needs a dispatch tick from the game: an action the
-- game never acknowledged was never dispatched, and saying otherwise would be
-- an invention.
--
-- An `action_settled` needs only a *verdict*. Every action that reached one --
-- `success`, `failed` or `lost` -- gets exactly one settle line, whether or
-- not the game stamped a reply tick for it, because a `lost` action never has
-- one and used to fall out of the record entirely. When the tick is not the
-- game's, `elapsed_ticks` is null and says so. `pending` and `running` write
-- no settle: no verdict, nothing to report.
--
-- So a settle with no dispatch beside it is possible, and it is a finding
-- rather than a gap: the action ended before the game ever acknowledged it.
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
                    // The settle is keyed on the **verdict**, not on a measured
                    // reply tick.
                    //
                    // It used to be keyed on `replied_tick`, and that made one
                    // whole outcome unrecordable: a `Status::Lost` action is
                    // *defined* by no reply ever arriving, so it never has a
                    // reply tick and so it never got an `action_settled` line.
                    // `run-1788347034-00981` wrote 179 dispatches against 170
                    // settles that way -- nine lost `craft`s, every one of them
                    // a dispatch with nothing after it, while the supervisor
                    // counted one lost action per iteration of milestone 7. The
                    // same gate swallowed a `Status::Failed` whose failure
                    // arrived before the game stamped anything
                    // (`ActionFailure::not_dispatched` carries
                    // `ActionTicks::UNKNOWN`), which left a failed action with
                    // no line anywhere at all.
                    //
                    // `pending` and `running` are not verdicts and write
                    // nothing: an action the run never finished has no outcome
                    // to report, and inventing one is the failure mode this
                    // gate has to avoid now that it no longer waits for a tick.
                    if matches!(status.as_str(), "success" | "failed" | "lost") {
                        // `None` only on a genuine success: a settle this
                        // codebase does not spell `"success"` is a failure of
                        // some kind, even one the classifier cannot name yet,
                        // and `FailureKind::Other` says so instead of leaving
                        // `failure` permanently unwritten the way it was
                        // before this classifier existed.
                        let failure = (status != "success")
                            .then(|| classify_failure(error.as_deref().unwrap_or("")));
                        // The game's own tick when it gave one. When it did
                        // not, the honest stamp is the record's own high-water
                        // mark (see `RunRecorder::not_before`): this verdict
                        // was reached no earlier than everything already
                        // written, including this action's own dispatch a few
                        // lines above. What is never done is reusing the
                        // dispatch tick, which would report a real reply at a
                        // real instant and a duration of zero.
                        //
                        // `elapsed_ticks` is what says the difference out
                        // loud. It is `Some` only when *both* ends were
                        // measured, so a synthesized stamp always carries a
                        // null duration -- "nobody measured this", exactly as
                        // `EventKind::ActionSettled` documents -- and can
                        // never be mistaken for a timed span.
                        let settled_tick = match replied {
                            Some(replied) => replied,
                            None => recorder.not_before(dispatched.unwrap_or(0)),
                        };
                        let elapsed_ticks =
                            dispatched.zip(replied).map(|(d, r)| r.saturating_sub(d));
                        recorder
                            .record(
                                settled_tick,
                                EventKind::ActionSettled {
                                    id,
                                    bot,
                                    status,
                                    elapsed_ticks,
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
                                    tick: settled_tick,
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
        "__doc_entry_refusals",
        String::from(
            r#"
--- flushes the placement refusals the game has handed down since the last flush
-- A refusal is `surface.can_place_entity` saying no -- not the
-- `player_blocks_placement` case, which the RCON layer walks the bot clear of
-- and retries. Each one is remembered for the rest of the run and every later
-- plan sites around the refused footprint, so writing them out is what makes
-- that avoidance readable: without it, a planner that has quietly started
-- preferring distant tiles looks like a planner with a bug. Call it once per
-- loop iteration, alongside `record.actions` and `record.teleports`.
--
-- Two writers fill the ledger and each event says which one it came from.
-- `source = "dispatch"` is a build a bot actually attempted and the game
-- turned down; there is a failed action beside it. `source = "pre_check"` is
-- a site `goal.plan` asked the game about *before* returning the plan, so no
-- action for it was ever created -- and only that kind carries `blockers`
-- and `tile`, naming what was in the way.
-- @treturn number how many refusal events were written
-- @raise if no recording is running
function record.refusals()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        let rcon = rcon.clone();
        map_table.set(
            "refusals",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                for refusal in world.unreported_placement_refusals() {
                    // The refusal's own stamp when the reply carried one.
                    // `not_before` is what keeps a missing stamp from
                    // travelling backwards through the log: it clamps to the
                    // last tick already written rather than inventing a zero.
                    let tick = recorder.not_before(
                        refusal
                            .tick
                            .unwrap_or_else(|| rcon.last_tick().unwrap_or(0)),
                    );
                    recorder
                        .record(
                            tick,
                            EventKind::PlacementRefused {
                                entity: refusal.entity,
                                position: refusal.position,
                                source: refusal.source.as_str().to_string(),
                                blockers: refusal.blockers,
                                tile: refusal.tile,
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
--
-- Stops the video recorder first, if `record.start` was given one. A recording
-- that was never stopped is archived still saying `"status": "recording"`,
-- which the viewer reports as a defect rather than showing as a complete
-- video -- so stopping it here is what makes that status mean what it says.
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
        // Async only because the encoder has to be stopped, and stopping it
        // means awaiting a child process.
        map_table.set(
            "finish",
            lua.create_async_function(move |_lua, outcome: String| {
                let slot = slot.clone();
                let rcon = rcon.clone();
                let workspace = workspace.clone();
                async move {
                    // The slot is a `parking_lot::Mutex`, whose guard is not
                    // `Send` and would deadlock a second caller across the
                    // await below. Taking the recorder out and dropping the
                    // guard in one scope is what keeps the await lock-free --
                    // and taking it is what `finish` did before video existed,
                    // so this is the same handover, just made explicit.
                    let mut recorder = {
                        let mut guard = slot.lock();
                        guard
                            .take()
                            .ok_or_else(|| record_error("no recording is running"))?
                    };
                    let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
                    // Before `finish`, which archives the recording but cannot
                    // stop it. A recording archived while still running is
                    // archived saying `recording`, which is the design's own
                    // proof that nobody stopped it.
                    recorder
                        .stop_video(Some(tick))
                        .await
                        .map_err(record_error)?;
                    recorder
                        .finish(
                            tick,
                            &outcome,
                            Some(&workspace),
                            factorio_bot_core::record::DEFAULT_KEEP,
                        )
                        .map_err(record_error)?;
                    Ok(recorder.run_id().to_string())
                }
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
        recording_lua_for(vec![])
    }

    /// [`recording_lua`] with a stated roster, for the bindings that report one.
    fn recording_lua_for(all_bots: Vec<PlayerId>) -> (Lua, tempfile::TempDir, std::path::PathBuf) {
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
            all_bots,
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

    /// The ticks the log carries, in file order -- what `not_before` is for.
    fn read_event_ticks(dir: &std::path::Path) -> Vec<u64> {
        factorio_bot_core::record::read_events(&dir.join("events.jsonl"))
            .expect("events.jsonl readable")
            .events
            .into_iter()
            .map(|event| event.tick)
            .collect()
    }

    // ------------------------------------------------------------- plan_created

    /// `bots` is the roster the plan was made for, not the bots it used.
    ///
    /// The plan below gives work to 1 and 2 out of a run of four. Derived from
    /// the steps -- which is what this did -- the record said `bots: [1, 2]`
    /// and a live run said `bots: [2]`, which reads exactly like a run of one
    /// bot and makes "why did bot 4 do nothing" unanswerable from the record.
    /// The bots that got nothing are the whole question.
    #[test]
    fn plan_created_reports_the_runs_roster_and_not_the_bots_in_the_plan() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2, 3, 4]);
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
                assert_eq!(
                    *bots,
                    vec![1, 2, 3, 4],
                    "the roster, including the two bots this plan gave no work to"
                );
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

    /// An empty plan still names who was available for it.
    ///
    /// This is the case where deriving from the steps was most obviously
    /// wrong: a milestone that planned nothing recorded `bots: []`, which is
    /// indistinguishable from a run with no bots in it.
    #[test]
    fn plan_created_of_an_empty_plan_is_zero_steps_and_zero_makespan() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2]);
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
                assert_eq!(*bots, vec![1, 2], "the roster, even with nothing planned");
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

    // ------------------------------------------- every attempt settles exactly once

    /// **A dispatch with no settle beside it was the record's largest hole.**
    ///
    /// `run-1788347034-00981` recorded 179 `action_dispatched` lines and 170
    /// `action_settled` lines, and every one of the nine missing settles was a
    /// `craft` the game acknowledged and then never answered — `Status::Lost`.
    /// The settle used to be written only when the observation carried a
    /// `replied_tick`, which a lost action can never have *by definition*: no
    /// reply arrived, so no reply tick was ever stamped. The verdict was
    /// therefore structurally unrecordable, and milestone 7 read as an
    /// unbroken run of successes while the supervisor's own counters said one
    /// action per iteration had been lost.
    #[test]
    fn a_lost_action_settles_even_though_the_game_never_replied() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                local steps = { { id = 13, bot = 1, label = "craft 1 stone-furnace" } }
                local actions = {
                    [13] = {
                        status = "lost",
                        dispatched_tick = 93392,
                        error = "the game reported no readable outcome: no action result received in time",
                    },
                }
                return record.actions(steps, actions)
                "#,
            )
            .eval()
            .expect("record.actions runs");
        assert_eq!(
            written, 2,
            "one dispatch and one settle, not just a dispatch"
        );

        let events = read_events(&run_dir);
        let ticks = read_event_ticks(&run_dir);
        assert!(matches!(
            &events[0],
            EventKind::ActionDispatched { id: 13, bot: 1, .. }
        ));
        match &events[1] {
            EventKind::ActionSettled {
                id,
                bot,
                status,
                elapsed_ticks,
                error,
                failure,
            } => {
                assert_eq!(*id, 13);
                assert_eq!(*bot, 1);
                assert_eq!(
                    status, "lost",
                    "`lost` is not `failed`: the game said nothing, it did not say no"
                );
                assert_eq!(
                    *elapsed_ticks, None,
                    "no reply tick was measured, so no duration may be reported"
                );
                assert!(
                    error
                        .as_deref()
                        .is_some_and(|e| e.contains("no action result received in time")),
                    "the settle carries why the outcome is unknown, got {error:?}"
                );
                assert_eq!(
                    failure.as_ref().map(|f| f.kind),
                    Some(FailureKind::Timeout),
                    "and it is classified, not left null"
                );
            }
            other => panic!("expected action_settled, got {other:?}"),
        }
        assert!(
            ticks[1] >= ticks[0],
            "a synthesized settle stamp is never earlier than its own dispatch, got {ticks:?}"
        );
    }

    /// A failure the game never stamped a tick for still reaches the record.
    ///
    /// `ActionFailure::not_dispatched` carries `ActionTicks::UNKNOWN`, so the
    /// attempt has neither tick. Writing nothing for it left a `Status::Failed`
    /// action with no line anywhere in `events.jsonl` — the same hole as the
    /// lost case, one step earlier. There is deliberately no
    /// `action_dispatched` beside this settle: nothing was dispatched, and
    /// inventing a dispatch would be a fabrication in the other direction.
    #[test]
    fn a_failure_the_game_never_stamped_a_tick_for_still_settles() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                local steps = { { id = 4, bot = 2, label = "insert 5 copper-ore" } }
                local actions = {
                    [4] = { status = "failed", error = "rcon: connection reset" },
                }
                return record.actions(steps, actions)
                "#,
            )
            .eval()
            .expect("record.actions runs");
        assert_eq!(written, 1, "the settle alone");

        let events = read_events(&run_dir);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EventKind::ActionSettled {
                id,
                status,
                elapsed_ticks,
                error,
                ..
            } => {
                assert_eq!(*id, 4);
                assert_eq!(status, "failed");
                assert_eq!(*elapsed_ticks, None);
                assert_eq!(error.as_deref(), Some("rcon: connection reset"));
            }
            other => panic!("expected action_settled, got {other:?}"),
        }
    }

    /// The control: an action the run never attempted must stay absent.
    ///
    /// The fix above keys the settle on the *status* rather than on a measured
    /// reply tick, and the failure mode of that is writing verdicts for work
    /// nobody started. `pending` and `running` are not verdicts, so neither
    /// produces a line.
    #[test]
    fn an_action_with_no_verdict_yet_writes_nothing() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                local steps = {
                    { id = 1, bot = 1, label = "mine 5 iron-ore" },
                    { id = 2, bot = 2, label = "mine 5 iron-ore" },
                }
                local actions = {
                    [1] = { status = "pending" },
                    [2] = { status = "running", dispatched_tick = 40 },
                }
                return record.actions(steps, actions)
                "#,
            )
            .eval()
            .expect("record.actions runs");
        assert_eq!(
            written, 1,
            "the running action's dispatch, and no verdict for either"
        );
        let events = read_events(&run_dir);
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, EventKind::ActionSettled { .. })),
            "no verdict was given, so none may be recorded: {events:?}"
        );
    }

    /// Every dispatch in one batch is matched by exactly one settle, whatever
    /// mixture of verdicts the batch holds.
    ///
    /// This is the invariant the live run broke, stated directly: counting
    /// `action_dispatched` against `action_settled` over a whole run is how the
    /// hole was found, so it is what the test counts.
    #[test]
    fn every_dispatched_action_settles_exactly_once() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            local steps = {
                { id = 0, bot = 1, label = "mine 5 iron-ore" },
                { id = 1, bot = 2, label = "place stone-furnace" },
                { id = 2, bot = 3, label = "craft 1 stone-furnace" },
            }
            local actions = {
                [0] = { status = "success", dispatched_tick = 100, replied_tick = 700 },
                [1] = { status = "failed", dispatched_tick = 710, replied_tick = 710,
                        error = "game rejected the command: can_place_entity said 'no'" },
                [2] = { status = "lost", dispatched_tick = 720,
                        error = "no action result received in time" },
            }
            record.actions(steps, actions)
            "#,
        )
        .exec()
        .expect("record.actions runs");

        let events = read_events(&run_dir);
        let mut dispatched: Vec<u32> = Vec::new();
        let mut settled: Vec<(u32, String)> = Vec::new();
        for event in &events {
            match event {
                EventKind::ActionDispatched { id, .. } => dispatched.push(*id),
                EventKind::ActionSettled { id, status, .. } => {
                    settled.push((*id, status.clone()));
                }
                other => panic!("unexpected event {other:?}"),
            }
        }
        dispatched.sort_unstable();
        settled.sort();
        assert_eq!(dispatched, vec![0, 1, 2]);
        assert_eq!(
            settled,
            vec![
                (0, "success".to_string()),
                (1, "failed".to_string()),
                (2, "lost".to_string()),
            ],
            "and the three verdicts stay three different words"
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

    // ------------------------------------------------------------- refusals

    /// `record.refusals()` writes each refused site once and leaves it on the
    /// world.
    ///
    /// The second half is the point, and is where this differs from
    /// `record.teleports()` beside it: a teleport is an event that needs
    /// writing once, while a refusal is a standing fact `PlanState::from_world`
    /// has to re-read on every plan. Draining it into the record would make
    /// the planner forget the site the moment it was written down.
    #[test]
    fn refusals_are_written_once_and_stay_available_to_the_planner() {
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

        for (tick, x, y) in [(6198u64, -16., -58.), (6204, -19., 51.)] {
            world.record_placement_refusal(
                factorio_bot_core::factorio::world::PlacementRefusal::at_dispatch(
                    Some(tick),
                    "stone-furnace",
                    Position { x, y },
                ),
            );
        }

        let written: u32 = lua
            .load("return record.refusals()")
            .eval()
            .expect("record.refusals() runs");
        assert_eq!(written, 2);

        let events = read_events(&run_dir);
        assert_eq!(events.len(), 2);
        match &events[0] {
            EventKind::PlacementRefused {
                entity,
                position,
                source,
                blockers,
                tile,
            } => {
                assert_eq!(entity, "stone-furnace");
                assert_eq!(position.x, -16.);
                assert_eq!(position.y, -58.);
                assert_eq!(source, "dispatch");
                assert!(
                    blockers.is_empty() && tile.is_none(),
                    "a refusal observed at dispatch has no cause to report: \
                     {blockers:?} / {tile:?}"
                );
            }
            other => panic!("expected placement_refused, got {other:?}"),
        }

        let again: u32 = lua
            .load("return record.refusals()")
            .eval()
            .expect("record.refusals() runs on an empty ledger");
        assert_eq!(again, 0, "each refusal is written exactly once");
        assert_eq!(
            world.placement_refusals().len(),
            2,
            "the planner must still see both sites after they were recorded"
        );
    }

    /// A refusal the pre-flight check learned carries what the game found, and
    /// says it was learned before dispatch.
    ///
    /// The distinction is not cosmetic. A `dispatch` refusal has a failed
    /// action beside it in the same log; a `pre_check` refusal has none, and a
    /// reader who could not tell them apart would go looking for the missing
    /// `action_settled` line. `blockers` and `tile` are the other half: five
    /// runs ended on a refusal that named no cause, and this is the line that
    /// names one.
    #[test]
    fn a_pre_check_refusal_records_its_source_and_what_was_in_the_way() {
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

        world.record_placement_refusal(factorio_bot_core::factorio::world::PlacementRefusal {
            tick: Some(6198),
            entity: "stone-furnace".to_string(),
            position: Position { x: -16., y: -58. },
            source: factorio_bot_core::factorio::world::RefusalSource::PreCheck,
            blockers: vec!["tree-01".to_string(), "tree-02".to_string()],
            tile: Some("grass-3".to_string()),
        });
        let written: u32 = lua
            .load("return record.refusals()")
            .eval()
            .expect("record.refusals() runs");
        assert_eq!(written, 1);

        match &read_events(&run_dir)[0] {
            EventKind::PlacementRefused {
                source,
                blockers,
                tile,
                ..
            } => {
                assert_eq!(source, "pre_check");
                assert_eq!(
                    blockers,
                    &vec!["tree-01".to_string(), "tree-02".to_string()]
                );
                assert_eq!(tile.as_deref(), Some("grass-3"));
            }
            other => panic!("expected placement_refused, got {other:?}"),
        }
    }

    /// A refusal whose reply carried no tick stamp still lands in order.
    ///
    /// `not_before` is what does it: a missing stamp becomes the last tick
    /// already written, never a zero that would sort the line before the run
    /// started.
    #[test]
    fn an_unstamped_refusal_does_not_travel_backwards_in_the_log() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut recorder = RunRecorder::start(tmp.path(), "run-1").expect("recorder starts");
        let run_dir = recorder.dir().to_path_buf();
        recorder
            .record(
                900,
                EventKind::MilestoneStarted {
                    index: 1,
                    goal: "anything".to_string(),
                },
            )
            .expect("a first event");
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

        world.record_placement_refusal(
            factorio_bot_core::factorio::world::PlacementRefusal::at_dispatch(
                None,
                "stone-furnace",
                Position { x: 0., y: 0. },
            ),
        );
        let written: u32 = lua
            .load("return record.refusals()")
            .eval()
            .expect("record.refusals() runs");
        assert_eq!(written, 1);

        let ticks = read_event_ticks(&run_dir);
        assert_eq!(
            ticks,
            vec![900, 900],
            "an unstamped refusal is clamped to the log's own clock"
        );
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

    /// A `{...}` literal, evaluated in a sandboxed Lua, so these tests read
    /// the options exactly as a script writes them rather than as a Rust
    /// author imagines a script writes them.
    fn options(source: &str) -> (Lua, LuaTable) {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        let table = lua
            .load(format!("return {source}"))
            .eval::<LuaTable>()
            .expect("an options table");
        (lua, table)
    }

    /// **The half of "off by default" that lives outside the mod.** The mod
    /// refuses to capture without being asked, and so does this: a script that
    /// says nothing must produce `FrameCameras::None`, not something the mod
    /// then has to talk out of.
    #[test]
    fn saying_nothing_about_frames_means_no_screenshot_camera() {
        assert_eq!(
            frame_options(None).expect("no options is fine"),
            FrameCameras::None,
            "record.start() captures no frames -- screenshots are retired"
        );
        for source in [
            "{}",
            "{frames = false}",
            "{frames = nil}",
            "{video = true}",
            "{other = 1}",
        ] {
            let (_lua, table) = options(source);
            assert_eq!(
                frame_options(Some(&table)).expect(source),
                FrameCameras::None,
                "{source} must register no camera"
            );
        }
    }

    #[test]
    fn frames_true_asks_for_every_camera_and_a_list_asks_for_those() {
        let (_lua, table) = options("{frames = true}");
        assert_eq!(
            frame_options(Some(&table)).expect("true"),
            FrameCameras::All
        );

        let (_lua, table) = options(r#"{frames = {"follow", "area"}}"#);
        assert_eq!(
            frame_options(Some(&table)).expect("a list"),
            FrameCameras::Only(vec!["follow".into(), "area".into()])
        );
    }

    /// Lua truthiness would read `frames = 1` or `frames = "follow"` as "all
    /// of them", which is the expensive direction to guess in: ~520 MB an hour
    /// per camera, on a run that meant to name one.
    #[test]
    fn a_frames_option_that_is_neither_a_flag_nor_a_list_is_refused() {
        for source in [r#"{frames = "follow"}"#, "{frames = 1}"] {
            let (_lua, table) = options(source);
            let err = frame_options(Some(&table)).expect_err(source);
            assert!(
                err.to_string().contains("cameras"),
                "{source}: the refusal has to say what it refused, got {err}"
            );
        }
    }

    #[test]
    fn saying_nothing_about_video_means_no_video() {
        assert!(
            video_options(None).expect("no options is fine").is_none(),
            "record.start() records no video, as every existing script expects"
        );
        for source in ["{}", "{video = false}", "{video = nil}", "{other = 1}"] {
            let (_lua, table) = options(source);
            assert!(
                video_options(Some(&table)).expect(source).is_none(),
                "{source} must not start an encoder"
            );
        }
    }

    #[test]
    fn video_true_takes_the_settled_defaults() {
        let (_lua, table) = options("{video = true}");
        let chosen = video_options(Some(&table))
            .expect("a boolean is understood")
            .expect("video was asked for");
        assert_eq!(
            chosen.resolution.size(),
            (1280, 720),
            "720p is the default; 1080p is the opt-in"
        );
        assert_eq!(chosen.fps, 15);
        assert_eq!(chosen.client, 1);
        assert_eq!(
            chosen.pid, None,
            "the pid is discovered from the workspace, not asked of the script"
        );
    }

    #[test]
    fn a_video_table_overrides_only_what_it_names() {
        let (_lua, table) = options(r#"{video = {resolution = "1080p"}}"#);
        let chosen = video_options(Some(&table))
            .expect("a table is understood")
            .expect("video was asked for");
        assert_eq!(chosen.resolution.size(), (1920, 1080));
        assert_eq!(chosen.fps, 15, "untouched keys keep their defaults");
        assert_eq!(chosen.client, 1);

        let (_lua, table) = options("{video = {client = 3, fps = 30}}");
        let chosen = video_options(Some(&table))
            .expect("a table is understood")
            .expect("video was asked for");
        assert_eq!(
            chosen.client, 3,
            "which client to film is the script's call"
        );
        assert_eq!(chosen.fps, 30);
        assert_eq!(chosen.resolution.size(), (1280, 720));
    }

    /// The decision this refuses to soften: a run that quietly recorded at the
    /// wrong size is worse than one that refused to start.
    #[test]
    fn an_unknown_resolution_refuses_rather_than_falling_back() {
        let (_lua, table) = options(r#"{video = {resolution = "4k"}}"#);
        let err = video_options(Some(&table)).expect_err("4k is not a resolution here");
        let text = err.to_string();
        assert!(text.contains("4k"), "{text}");
        assert!(
            text.contains("720p"),
            "the error says what is allowed: {text}"
        );
    }

    /// `video = "true"` is a typo, and guessing at it would be guessing at
    /// whether the run was supposed to record.
    #[test]
    fn a_video_that_is_neither_a_boolean_nor_a_table_is_refused() {
        for source in [r#"{video = "true"}"#, "{video = 1}"] {
            let (_lua, table) = options(source);
            let err = video_options(Some(&table)).expect_err(source);
            let text = err.to_string();
            assert!(text.contains("boolean or a table"), "{source} -> {text}");
        }
    }

    /// The options are read before the run id is minted, the run directory is
    /// created or the game is called. Proved here by the *message*: with a
    /// recording already running, a bad option still reports the option --
    /// which it could only do by having been read first.
    #[tokio::test]
    async fn a_bad_video_option_is_refused_before_anything_is_created() {
        let (lua, _tmp, _dir) = recording_lua();
        let err = lua
            .load(r#"record.start({video = {resolution = "4k"}})"#)
            .exec_async()
            .await
            .expect_err("4k is not a resolution here");
        let text = err.to_string();
        assert!(text.contains("4k"), "{text}");
        assert!(
            !text.contains("already running"),
            "the option is read before the slot is even looked at: {text}"
        );
    }
}
