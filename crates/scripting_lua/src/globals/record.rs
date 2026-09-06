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
use factorio_bot_core::factorio::world::{BenchChange, BotLifeEvent, FactorioWorld};
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::paris::{info, warn};
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::process::instance_setup::{installed_factorio_version, read_map_gen_seed};
use factorio_bot_core::record::map::{
    Divergence, EntitySnapshot, MapKind, MapRecord, Placement, bounds_around, divergence_between,
};
use factorio_bot_core::record::run_mode;
use factorio_bot_core::record::savepoint;
use factorio_bot_core::record::video::Resolution;
use factorio_bot_core::record::{
    ActionFailure, Delivery, EventKind, FailureKind, PlannedStep, Provenance, RunRecorder,
    SatisfiedReason, VideoOptions, VideoRecorder, WalkFailure, WalkFailureKind, git_provenance,
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
///
/// `electric-pole`, `generator` and `solar-panel` joined both lists on
/// 2026-09-02, the day `EntityGraph::add` started tracking them. They are the
/// worked example of the paragraph above: the model side gained them
/// immediately (`snapshot_within` reads `entity_tree`), and had this filter
/// not moved with it, every keyframe taken anywhere near a power plant would
/// have carried one permanent `only_in: "model"` entry per pole -- the same
/// class of noise the `overlaps_bounds` fix was written to remove. See
/// `docs/superpowers/notes/2026-09-02-building-power.md`.
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
            | Ok(EntityType::ElectricPole)
            | Ok(EntityType::Generator)
            | Ok(EntityType::SolarPanel)
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
        EntityType::ElectricPole,
        EntityType::Generator,
        EntityType::SolarPanel,
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
    // One parser, and it lives in the executor: `recover` reads the same
    // wording to refuse re-dispatching a diverged transfer
    // (`crates/executor/src/divergence.rs`), and two copies of the mod's
    // three sentences would drift the way `classify_walk_failure` once did.
    let d = factorio_bot_executor::divergence(error)?;
    Some(format!("moved {} of {} {}", d.moved, d.asked, d.item))
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
    } else if error.contains(NO_CHARACTER_WORDING) {
        // Ahead of `Rejected`, which its outer wrapper (`game rejected the
        // command: Unexpected Response: Error: player 2 has no character:
        // ...`) would otherwise claim. The mod prints this sentence from
        // `get_player` and from every entry point that reads
        // `game.players[id]` directly, and `on_player_died` uses it as the
        // verdict for the walk, mine or craft in flight at the moment of
        // death. A test below generates it from the mod's own code rather
        // than from a string typed here, so the two cannot drift apart
        // silently -- which is how nineteen of twenty walk failures once
        // archived as `other`.
        FailureKind::NoCharacter
    } else if error.contains("no path to")
        || error.contains("tile arrival tolerance")
        || error.contains("tile resource reach")
    {
        FailureKind::Unreachable
    } else if error.contains("player still blocks placement")
        || error.contains("player blocks placement in all directions")
        // The third member of the same family, and the one that reaches a
        // record only after `place_entity_timed` has re-issued the placement
        // `FOOTPRINT_CLEAR_ATTEMPTS` times and the blocker still has not moved
        // (`crates/core/src/factorio/rcon.rs`). Without this arm it lands in
        // `Rejected` -- indistinguishable in a query from a chest that was
        // full or an ore that was gone -- which is how `run-1788481380-80843`
        // recorded `kind: "rejected", detail: null` for the one failure that
        // cost it the whole plan.
        || error.contains("a character is standing in the footprint")
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
    let detail = match kind {
        FailureKind::MissingItem => error.split('\'').nth(1).map(str::to_string),
        FailureKind::NoCharacter => no_character_detail(error),
        _ => None,
    };
    ActionFailure { kind, detail }
}

/// The substring every character-less refusal from the mod carries, on both
/// of its roads: the reply-body `Error: player <n> has no character: <why>`
/// and the `on_player_died` verdict `ERROR: player <n> has no character: died
/// at tick <t> ...`. Defined once because two classifiers match it.
const NO_CHARACTER_WORDING: &str = "has no character";

/// The `<why>` clause after `has no character: `, for
/// [`FailureKind::NoCharacter`]'s detail -- `dead, respawns in 587 ticks`,
/// `in a cutscene`, or `died at tick 1234 killed by medium-worm-turret,
/// respawns in 587 ticks`.
///
/// Cut at the first quote, bracket or newline, because the sentence arrives
/// inside whatever wrapper the road it took added: `action_start_mining`'s
/// refusal is the reply lines' `Debug` rendering, `["Error: ..."]`, and the
/// closing `"]` is not part of the reason.
fn no_character_detail(error: &str) -> Option<String> {
    let (_, tail) = error.split_once(&format!("{NO_CHARACTER_WORDING}: "))?;
    let end = tail.find(['"', ']', '\n']).unwrap_or(tail.len());
    let detail = tail[..end].trim().trim_end_matches(['.', ',']);
    (!detail.is_empty()).then(|| detail.to_string())
}

/// One `(x/y)` pair out of the mod's own `coord()` formatting.
///
/// Declines rather than guessing: either both halves parse as numbers or this
/// returns `None` and the whole message is still in `error` for a person to
/// read. Nothing here rounds -- a walk destination is routinely a tile centre
/// like `-22.30078125`, and a position read back at lower precision would land
/// in a different collision box than the one the walk actually failed against.
fn parse_coord(text: &str) -> Option<Position> {
    let (x, y) = text.split_once('/')?;
    Some(Position::new(
        x.trim().parse().ok()?,
        y.trim().parse().ok()?,
    ))
}

/// The two **observed** positions BotBridge names when it gives up on a walk:
/// `... made no progress for <t> ticks from (<x>/<y>) to (<x>/<y>)`, and --
/// from a build whose mod still re-pathed -- `... found no path from (<x>/<y>)
/// to (<x>/<y>)`.
///
/// The first is the character's real position at the instant the mod gave up
/// (`player.character.position`, not an inference from the last tile boundary
/// crossed), and the second is the destination the mod was actually steering
/// to -- the waypoint of the path *the game returned*, which is not necessarily
/// the `to` the schedule asked for. Both were reconstructed by hand from
/// `workspace/server-log.txt` to diagnose run 30; this is what puts them in the
/// run directory instead.
///
/// Two accepted prefixes rather than one, because the mod's re-path was
/// retired: a stalled leg now fails immediately and Rust retries it
/// (`move_player_timed`, crates/core/src/factorio/rcon.rs). Keeping the older
/// prefix keeps the archived runs readable, which is the whole job of this
/// file.
const WALK_ENDPOINT_PREFIXES: [&str; 2] = ["found no path from ", "no progress for "];

fn walk_endpoints(error: &str) -> (Option<Position>, Option<Position>) {
    let Some(tail) = WALK_ENDPOINT_PREFIXES
        .iter()
        .find_map(|prefix| error.split_once(prefix).map(|(_, tail)| tail))
    else {
        return (None, None);
    };
    // The stalled wording puts the tick count between its prefix and the first
    // coordinate; the no-path wording puts nothing there. Skipping to the `(`
    // reads both without a second parser.
    let Some((_, tail)) = tail.split_once('(') else {
        return (None, None);
    };
    let Some((from, rest)) = tail.split_once(')') else {
        return (None, None);
    };
    let destination = rest
        .strip_prefix(" to (")
        .and_then(|t| t.split_once(')'))
        .and_then(|(coord, _)| parse_coord(coord));
    (parse_coord(from), destination)
}

/// Classifies a settled walk's error text into a [`WalkFailure`].
///
/// Matched as plain substrings against BotBridge's `w.stuck` wordings
/// (`mods/BotBridge/control.lua`) and the outer `ActuatorError` that wraps
/// them, for the same reason [`classify_failure`] does it that way: this only
/// ever sees a `String` that already crossed the Lua boundary, and a substring
/// match degrades to [`WalkFailureKind::Other`] when a wording moves instead of
/// failing outright.
///
/// **The order of the arms is load-bearing between the first two.** A lost walk
/// is wrapped in `the game reported no readable outcome`, and nothing about the
/// walk itself is known -- so the timeout is tested before any wording that
/// would claim knowledge the run does not have.
fn classify_walk_failure(error: &str) -> WalkFailure {
    let kind = if error.contains("no action result received in time")
        || error.contains("no readable outcome")
    {
        WalkFailureKind::Timeout
    } else if error.contains(NO_CHARACTER_WORDING) {
        // Before every pathfinder arm, and it has to be: a dead bot's walk
        // fails at the path request, and `player_path` hands that refusal
        // back wrapped in words the `NoPath` arm below would match. Nothing
        // was searched, so nothing about the map is claimed.
        WalkFailureKind::NoCharacter
    } else if error.contains("try again later") {
        // **Ordering, and it is load-bearing.** `RconPathRequestFailed` renders
        // as `the game's pathfinder returned no path: <the mod's own words>`
        // for *both* of the mod's answers, so the wrapper's text alone cannot
        // tell them apart and the inner wording has to be read first. Matched
        // above the arm below rather than merged into the busy arm further
        // down, because that arm is reached only when this one has already
        // declined the string.
        //
        // What it means here is narrower than the two archived wordings below:
        // `FactorioRcon::player_path_attempt` retries a full queue with
        // backoff, so a `try again later` that survives to a record is one
        // where *every* retry was refused. Still nothing learned about the
        // map, which is the whole distinction.
        WalkFailureKind::PathfinderBusy
    } else if error.contains("a character cannot stand there") {
        // Rust's own `RconWalkEndsWhereNobodyCanStand`: the game returned a
        // route, `judge_path` found its last waypoint inside a collision box
        // the graph knows, and nothing was dispatched. Above the `NoPath` arm
        // because it is the opposite fact -- a path *was* found, and the aim
        // was the problem. `run-1788608011-14361`'s refusal into a
        // neighbouring rock was archived as `other` until this arm existed.
        WalkFailureKind::DestinationBlocked
    } else if error.contains("the character is boxed in") {
        // `RconActuator::walk`'s own wording, appended to a `found no path`
        // refusal when the mobility probe that followed it
        // (`walk_memory::judge_mobility`) found every short hop refused too.
        // Above the `NoPath` arm because the string still carries that arm's
        // wording -- the walk *was* refused -- and the finding is the
        // opposite of what `NoPath` is careful to claim: the bot, not the
        // destination, is what could not be reached from.
        WalkFailureKind::BoxedIn
    } else if error.contains("the destination is unreachable")
        || error.contains("found no path")
        || error.contains("returned no path")
        || error.contains("the best one found ends")
    {
        // The pathfinder searched. This is the fact the stuck-walk teleport
        // used to destroy by hopping over it.
        //
        // Four wordings, from three eras and two sides. The first two are the
        // mod's, from the build that re-pathed for itself. The third is
        // `RconPathRequestFailed` (crates/core/src/errors.rs) carrying the
        // mod's bare `Error: failed to path find` out of a *pre-dispatch* path
        // request, which is what this build produces most: nineteen of the
        // twenty failed walks in `run-1788432181-42528` read that way, and
        // every one of them was classified `other` until this arm learned the
        // wording -- a run whose dominant failure the record could not name.
        // The fourth is Rust's own `RconWalkFallsShort`, which is how an
        // unreachable goal reads now that the retry lives in
        // `move_player_timed`: the fresh path request comes back
        // `failed to path find`, the offset-goal fallback finds somewhere
        // *near* it, and `judge_path` refuses that for landing outside the
        // caller's tolerance. Same searched-and-there-is-no-way-there fact,
        // raised one layer up.
        WalkFailureKind::NoPath
    } else if error.contains("refused a re-path request")
        || error.contains("did not answer a re-path")
    {
        // It never searched: `try again later` on a full request queue, or a
        // request accepted and never answered. Nothing was learned about the
        // destination, which is the whole reason this is not `NoPath`.
        //
        // The mod no longer produces either wording. Kept because the archived
        // runs do, and this classifier is read against them. The *variant* is
        // not archive-only any more, though: the `try again later` arm above
        // is a live producer of it, so `WalkFailureKind::PathfinderBusy`'s own
        // "Archive only" note (crates/core/src/record/mod.rs) now describes
        // these two wordings rather than the kind.
        WalkFailureKind::PathfinderBusy
    } else if error.contains("re-paths on one walk") {
        WalkFailureKind::RepathLimit
    } else if error.contains("made no progress")
        || error.contains("aborted before reaching last waypoint")
    {
        // A leg stopped progressing. `made no progress` is the current mod's
        // wording and reaches the record only after `move_player_timed` has
        // already spent its whole retry budget on fresh paths, so it means the
        // walking itself is stuck rather than that the map is.
        WalkFailureKind::Stalled
    } else {
        WalkFailureKind::Other
    };
    let (from, destination) = walk_endpoints(error);
    WalkFailure {
        kind,
        from,
        destination,
    }
}

/// Reads `record.plan_created`'s optional roster argument.
///
/// Three answers, and they are three different things:
///
/// - **Absent or `nil`** -> `None`. Nobody said which roster the plan was made
///   for, so the record says so. It does NOT fall back to the run's own
///   roster: that fallback is precisely the bug this argument exists to fix,
///   and a plausible-looking substitute is worse than a null, because a reader
///   cannot tell it apart from a stated fact.
/// - **A list of positive integers** -> `Some`, deduplicated and ascending, so
///   the field's shape does not depend on the order the caller happened to
///   build its table in.
/// - **Anything else** -> refused. A roster this cannot read is a construction
///   error in the caller, and recording `[]` for it would put "this plan was
///   made for no bots" into the one part of the archive that stays trustworthy
///   when outcomes do not. `mlua`'s null sentinel is light userdata and
///   therefore truthy, so the match is on the *value*, never on truthiness.
fn roster_from_lua(bots: Option<LuaValue>) -> LuaResult<Option<Vec<u32>>> {
    let value = match bots {
        None | Some(LuaValue::Nil) => return Ok(None),
        Some(value) => value,
    };
    let LuaValue::Table(table) = value else {
        return Err(record_error(format!(
            "record.plan_created: bots must be a table of bot ids, got a {}",
            value.type_name()
        )));
    };
    let mut roster = BTreeSet::new();
    for id in table.sequence_values::<LuaValue>() {
        let id = id?;
        let id = id
            .as_integer()
            .filter(|id| *id > 0)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(|| {
                record_error("record.plan_created: bots must be a list of positive bot ids")
            })?;
        roster.insert(id);
    }
    if roster.is_empty() {
        return Err(record_error(
            "record.plan_created: bots is empty; a plan is always made for at least one bot",
        ));
    }
    Ok(Some(roster.into_iter().collect()))
}

/// Reads one of `record.roster_changed`'s bot lists: a table of positive
/// integers, deduplicated and ascending. Unlike [`roster_from_lua`] an empty
/// table is a legitimate answer -- "nobody left" is a thing this event says --
/// and `nil` is refused rather than read as empty, because a caller that
/// forgot an argument should hear about it.
fn bot_list_from_lua(name: &str, value: LuaValue) -> LuaResult<Vec<u32>> {
    let LuaValue::Table(table) = value else {
        return Err(record_error(format!(
            "record.roster_changed: {name} must be a table of bot ids, got a {}",
            value.type_name()
        )));
    };
    let mut ids = BTreeSet::new();
    for id in table.sequence_values::<LuaValue>() {
        let id = id?;
        let id = id
            .as_integer()
            .filter(|id| *id > 0)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(|| {
                record_error(format!(
                    "record.roster_changed: {name} must be a list of positive bot ids"
                ))
            })?;
        ids.insert(id);
    }
    Ok(ids.into_iter().collect())
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

/// The run recorder, published into the Lua state's app data so a binding
/// *below* the Lua seam can write a live event without a script asking it to.
///
/// The same seam [`crate::lua_runner::ReplaySink`] uses, for the same reason:
/// the thing that needs it is `goal.start`'s batch heartbeat, and threading a
/// recorder through `create_lua_goal` would put it in the signature of every
/// binding that has nothing to do with recording.
///
/// **Absence is an ordinary state, not an error.** `record` is installed only
/// alongside `rcon` (see `lua_runner.rs`), so a planning-only interpreter has
/// no recorder at all and a heartbeat that cannot find one simply does not
/// write. That is the same reading as `ReplaySink`'s: nobody is listening.
///
/// It holds the `rcon` handle as well as the slot because a live event's tick
/// is `FactorioRcon::last_tick` -- an event stamped by anything else would be
/// stamping the record's one shared axis with a number nobody observed.
#[derive(Clone)]
pub struct LiveRecord {
    slot: Slot,
    rcon: Arc<factorio_bot_core::factorio::rcon::FactorioRcon>,
}

impl LiveRecord {
    /// Writes `kind` now, stamped with the game's clock.
    ///
    /// Returns whether it was written. `false` means no recording is running
    /// -- a script that never called `record.start()` -- which is a state a
    /// caller may ignore, unlike an I/O failure, which is logged here because
    /// the caller (a background heartbeat) has nobody to report it to.
    pub fn record(&self, kind: EventKind) -> bool {
        let mut guard = self.slot.lock();
        let Some(recorder) = guard.as_mut() else {
            return false;
        };
        let tick = recorder.not_before(self.rcon.last_tick().unwrap_or(0));
        if let Err(err) = recorder.record(tick, kind) {
            factorio_bot_core::tracing::error!(
                error = %err,
                "failed to write a live event to the run record"
            );
            return false;
        }
        true
    }
}

/// Records an event at a tick the caller observed itself -- still never
/// earlier than something already in the log. See [`RunRecorder::not_before`].
fn record_at(slot: &Slot, tick: u64, kind: EventKind) -> LuaResult<()> {
    let mut guard = slot.lock();
    let recorder = guard
        .as_mut()
        .ok_or_else(|| record_error("no recording is running -- call record.start() first"))?;
    let tick = recorder.not_before(tick);
    recorder.record(tick, kind).map_err(record_error)
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

/// Writes the world out as this milestone's savepoint, and records what
/// happened either way.
///
/// # Why this hangs off `milestone_satisfied` and not off the supervisor
///
/// `record.milestone_satisfied` is the single place every driver script closes
/// a milestone through -- `research_run.lua`, `factory_stage1.lua` and
/// `factory_stage2.lua` all reach it from the supervisor's `"satisfied"`
/// transition. Hanging the savepoint off the Lua side instead would mean three
/// copies of it, and a fourth script would silently have none.
///
/// # It never fails the run
///
/// A savepoint is a souvenir of a milestone, not part of reaching one. Every
/// failure is recorded as [`EventKind::SavepointFailed`] and narrated, and the
/// run continues -- but it *is* recorded, because a milestone with no
/// savepoint and a milestone nobody tried to save look identical from the
/// archive otherwise, and this project has already been bitten four times by
/// checks that reported nothing while broken.
///
/// # Every milestone, not just the last
///
/// The milestone worth resuming *before* is the one that has never succeeded,
/// so the interesting savepoint is the one taken at the milestone before it.
/// Retention is [`factorio_bot_core::record::retention::reap`]'s, inherited by
/// living inside the run directory.
async fn take_savepoint(
    slot: &Slot,
    rcon: &factorio_bot_core::factorio::rcon::FactorioRcon,
    workspace: &std::path::Path,
    milestone_index: u32,
) {
    // The run directory and id are read and the lock released before anything
    // is awaited: `slot` is a `parking_lot::Mutex`, which is not held across
    // an await point anywhere in this file and must not start being.
    let Some((run_dir, run_id)) = ({
        let guard = slot.lock();
        guard
            .as_ref()
            .map(|recorder| (recorder.dir().to_path_buf(), recorder.run_id().to_string()))
    }) else {
        // No recording is running, so there is no run directory to put a
        // savepoint in and nothing that would ever read one. `record_live`
        // above has already refused for the same reason.
        return;
    };

    let instance = workspace.join("server");
    let request = savepoint::CaptureRequest {
        instance_dir: &instance,
        mod_dir: &savepoint::bridge_mod_dir(workspace),
        run_dir: &run_dir,
        run_id: &run_id,
        milestone_index,
        timeout: savepoint::SAVE_TIMEOUT,
    };
    let outcome = savepoint::capture(rcon, request).await;
    let event = match outcome {
        Ok((savepoint, elapsed)) => {
            info!(
                "savepoint for milestone <bright-blue>{}</>: {} ({} bytes, {} ms)",
                milestone_index,
                savepoint.file,
                savepoint.bytes,
                elapsed.as_millis()
            );
            EventKind::SavepointWritten {
                milestone_index,
                file: format!("{}/{}", savepoint::SAVEPOINTS_DIR, savepoint.file),
                bytes: savepoint.bytes,
                wrote_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            }
        }
        Err(err) => {
            warn!(
                "no savepoint for milestone <bright-blue>{}</>: {}",
                milestone_index, err
            );
            EventKind::SavepointFailed {
                milestone_index,
                error: err.to_string(),
            }
        }
    };
    // Recorded through the same path as everything else, and a failure to
    // record it is itself narrated rather than swallowed: this function
    // returns nothing, so a dropped `Result` here would be the silence the
    // event exists to prevent.
    if let Err(err) = record_live(slot, rcon, event) {
        warn!("could not record the savepoint outcome: {}", err);
    }
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
/// (`sampling_start`) that fails against `FactorioRcon::new_empty()`.
fn create_lua_record_with_slot(
    lua: &Lua,
    rcon: Arc<factorio_bot_core::factorio::rcon::FactorioRcon>,
    world: Arc<FactorioWorld>,
    scripts_root: PathBuf,
    all_bots: Vec<PlayerId>,
    slot: Slot,
) -> LuaResult<LuaTable> {
    // Published before any binding is installed, so a run started on the
    // script's very first line already has a recorder to beat into. It shares
    // the slot rather than copying anything out of it: `record.start()` fills
    // that slot later, and a heartbeat holding a snapshot taken now would be
    // holding the `None` it was created with forever.
    lua.set_app_data(LiveRecord {
        slot: slot.clone(),
        rcon: rcon.clone(),
    });
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Run recording
-- Writes a durable archive of a run: an append-only event log, the world-state
-- samples taken while it ran, and speedrun-style splits derived from its
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
    // samples out of this same workspace, alongside the keyframe work each
    // already does at exactly these two moments.
    let workspace = scripts_root
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| record_error("scripts directory has no parent workspace"))?;

    map_table.set(
        "__doc_entry_start",
        String::from(
            r#"
--- starts recording a run
-- Mints a run id, creates `<workspace>/runs/<id>/`, and starts the mod's
-- sampling session with that same id -- one call, so the log and the samples
-- cannot disagree about which run they belong to.
--
-- Also writes an opening keyframe to `map.jsonl`, bounded by the connected
-- bots' own positions (plus a margin) rather than by placements, since
-- nothing has been placed yet -- so a run that crashes before its first
-- placement still leaves a map behind. Writes nothing if no bots are
-- connected yet.
--
-- Video is the visual record, and it is opt-in. Per-camera screenshots were
-- retired on 2026-09-02 and removed: one run wrote 2164 JPEGs / 947 MB of them
-- against 290 MB for the same 45 minutes of video, and take_screenshot
-- rendered synchronously inside the game loop, once per camera, every 300
-- ticks.
--
--     record.start()                                      -- log, map and samples only
--     record.start({video = true})                        -- 720p, 15 fps, client 1
--     record.start({video = {resolution = "1080p"}})      -- for a final run worth the size
--     record.start({video = {client = 2, fps = 30}})      -- film a different client
--
-- What video cannot do, and what the event log and `map.jsonl` are for: a
-- video keeps writing the last drawn image while the game stalls, which looks
-- exactly like a game that was running and idle, and its tick comes from a
-- table the host built, interpolated between samples. The text artefacts are
-- the tick-exact record.
--
-- `resolution` is "720p" (the default) or "1080p"; an unknown value raises
-- here rather than recording at the wrong size. `client` is which graphical
-- client's window to film, defaulting to 1 -- the one client every run with a
-- client at all has, so the same option means the same thing on a one-bot run
-- and a four-bot one. The camera is not steered: the video shows whatever that
-- client shows.
--
-- Video failing to start never fails the run. The run continues without it,
-- and `video.json` records what went wrong.
-- @tparam[opt] table options `{video = ...}`
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
                    if slot.lock().is_some() {
                        return Err(record_error(
                            "a recording is already running -- call record.finish() first",
                        ));
                    }
                    let run_id = mint_run_id();

                    // The sampling session first, and its reply is where the
                    // opening tick comes from. Recording `run_started` before
                    // any command has been sent would stamp it with a tick
                    // nobody has observed -- the first live run opened at tick
                    // 0 while every later event was near 59000, which is a
                    // fabricated number that looks like data.
                    let opened_at = rcon
                        .as_ref()
                        .sampling_start(Some(run_id.clone()))
                        .await
                        .map_err(rcon_error)?;
                    let opened_at = opened_at.unwrap_or_else(|| rcon.last_tick().unwrap_or(0));

                    let mut recorder =
                        RunRecorder::start(&runs_root, run_id.clone()).map_err(record_error)?;
                    // Before the first event, so the very first `record()` can
                    // already pull samples across. A sampling session was just
                    // opened two calls up, so this run *is* being sampled --
                    // and the recorder's own tick-interval ingest is what keeps
                    // the archive current between milestone boundaries, which
                    // are as far apart as a milestone is long.
                    recorder.watch_samples(workspace.as_path());
                    // The other half of the free-vision disclosure. The world
                    // model is what `on_chunk_generated` feeds, and it holds
                    // ground no bot has been near; `watch_samples` above is
                    // where the "how far did a bot actually get" half comes
                    // from. Turned on here rather than left to a supervisor
                    // script to ask for, because a run that quietly omitted it
                    // would be a run whose number carries an undisclosed
                    // asterisk -- see `EventKind::VisionMeasured`.
                    recorder.watch_model(world.entity_graph.clone());

                    // Provenance, written before the first event and before
                    // anything below can fail.
                    //
                    // `seed`, `factorio` and `git` are three fields that have
                    // existed on `run_started` since it was introduced and were
                    // hardcoded `None` here the whole time -- all 24 archived
                    // runs carry `seed: null, factorio: null, git: null`, so no
                    // archived run can be attributed to a map or to a commit.
                    // This is where that stops.
                    //
                    // The server instance directory is `<workspace>/server`,
                    // the default `instance_name` every CLI path uses
                    // (`process_control.rs`). A workspace set up under another
                    // name reads as "seed unknown", which is true of it.
                    let instance = workspace.join("server");
                    let run_mode = run_mode::read_run_mode(&instance);
                    let seed = read_map_gen_seed(&instance);
                    let factorio = installed_factorio_version(&workspace.join("data"));
                    // The working tree of the process's own directory. See
                    // `GitProvenance`: this is where the code is *now*, which
                    // is what built the binary only if nobody has moved since,
                    // and `dirty` is what stops that being read as an identity.
                    let git = std::env::current_dir()
                        .ok()
                        .and_then(|cwd| git_provenance(&cwd));
                    let provenance = Provenance {
                        schema: Provenance::SCHEMA,
                        run_id: run_id.clone(),
                        started_unix: recorder.started_unix(),
                        started_tick: opened_at,
                        seed: seed.clone(),
                        map_exchange_string: std::fs::read_to_string(
                            instance.join("map-exchange-string.txt"),
                        )
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                        map: world.entity_graph.resource_fingerprint(),
                        factorio: factorio.clone(),
                        git: git.clone(),
                        profile: if cfg!(debug_assertions) {
                            "debug".to_string()
                        } else {
                            "release".to_string()
                        },
                        roster_requested: bots.clone(),
                        workspace: workspace
                            .canonicalize()
                            .ok()
                            .map(|p| p.to_string_lossy().into_owned()),
                        // Read from the instance rather than passed in: which
                        // world the server loaded is a fact about the server,
                        // known to whatever started it and to nothing in
                        // between -- exactly like `map-gen-seed.txt` two
                        // fields up, and read the same way. The marker is
                        // *deleted* on every non-resuming start, so this is
                        // null because this run is fresh and never because a
                        // previous run's answer was left lying about.
                        //
                        // `Some(_)` here is what makes `--compare` refuse to
                        // measure this run against a fresh-world one: it
                        // starts on accumulated world state and its timings
                        // are not the same quantity.
                        //
                        // Carries the same caveat as `seed` above, and for the
                        // same reason: it describes `<workspace>/server`. A
                        // run attached to somebody else's server with
                        // `--connect` is not using that instance at all, so a
                        // marker left there by a local resumed run would be
                        // read as this run's. Nothing clears it in that case,
                        // because clearing it would erase a true fact about an
                        // instance this process did not start.
                        resumed_from: savepoint::read_resume_marker(&instance)
                            .map(|marker| marker.label),
                        // Same road, same caveats: written by the process that
                        // started `<workspace>/server`, absent for a build
                        // that predates it or a server this process did not
                        // start. See `crates/core/src/record/run_mode.rs`.
                        bot_mode: run_mode.as_ref().map(|m| m.bot_mode.as_str().to_string()),
                        game_speed: run_mode.as_ref().map(|m| m.game_speed),
                    };
                    // Never fatal. A run that cannot write its provenance is
                    // still a run worth recording, and the reader's rule is
                    // that an absent file means "unknown" -- which is exactly
                    // what this failure leaves behind, honestly.
                    if let Err(error) = recorder.record_provenance(&provenance) {
                        factorio_bot_core::tracing::warn!(
                            %error,
                            "could not write provenance.json; this run will not be \
                             attributable to a map or a commit"
                        );
                    }

                    recorder
                        .record(
                            opened_at,
                            EventKind::RunStarted {
                                run_id: run_id.clone(),
                                bots,
                                // The event carries the seed as a number
                                // because it always has. A seed that does not
                                // parse stays `None` here and survives in full
                                // in `provenance.json`, rather than being
                                // rounded into something that looks like data.
                                seed: seed.as_deref().and_then(|s| s.parse::<u64>().ok()),
                                factorio,
                                git: git.map(|g| {
                                    if g.dirty {
                                        format!("{}-dirty", g.commit)
                                    } else {
                                        g.commit
                                    }
                                }),
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
                    // raise: `sampling_start` two calls above already
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

                    // Video is filmed from a client's window, and a headless
                    // run has none. Every live script asks for video, so this
                    // is skipped with a warning rather than refused -- the
                    // same "never fatal" rule the capture itself follows --
                    // but said here by name, rather than left to
                    // `VideoRecorder::start` to report "no window found",
                    // which reads as a capture fault.
                    let headless = run_mode.as_ref().map(|m| m.bot_mode)
                        == Some(run_mode::BotMode::Characters);
                    if headless && video.is_some() {
                        factorio_bot_core::tracing::warn!(
                            "video requested, but this run is --headless and has no client \
                             window to film; recording without video"
                        );
                    }
                    let video = if headless { None } else { video };
                    if let Some(video) = video {
                        // Never fatal: `VideoRecorder::start` returns `Ok` with
                        // `status: failed` and a reason for every capture
                        // failure, and the run goes on without it. Only an
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
--- records that a milestone was reached, and saves the world it was reached in
-- Writes `runs/<run>/savepoints/milestone-<index>.zip`, which a later run can
-- start from with `--resume-from <run>:<index>` instead of spending twenty
-- minutes re-deriving the same world. Every milestone gets one, because the
-- milestone worth starting *before* is the one that has never succeeded.
--
-- Saving is asynchronous inside Factorio, so this waits for the file to be
-- complete before returning -- ordinarily well under a second. If it cannot be
-- saved the run carries on regardless and the record says why
-- (`savepoint_failed`); a savepoint is a souvenir of a milestone, not part of
-- reaching one.
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
        let workspace = workspace.clone();
        map_table.set(
            "milestone_satisfied",
            lua.create_async_function(
                move |_lua, (index, iterations, reason): (u32, u32, String)| {
                    let slot = slot.clone();
                    let rcon = rcon.clone();
                    let workspace = workspace.clone();
                    async move {
                        let reason = parse_satisfied_reason(&reason)?;
                        record_live(
                            &slot,
                            &rcon,
                            EventKind::MilestoneSatisfied {
                                index,
                                iterations,
                                reason,
                            },
                        )?;
                        take_savepoint(&slot, &rcon, &workspace, index).await;
                        Ok(())
                    }
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
-- `plan.tick`, when the table carries one, is the tick the event is stamped
-- with: the tick `goal.plan` handed the plan back at (`PlanValue.tick`).
-- Without it the event takes the last RCON reply's tick, which on a headless
-- run is the run's start -- the plan then reads as made before the seconds of
-- planning that went into it.
--
-- `bots` is the **roster the plan was expanded against** -- every bot the
-- planner was allowed to give work to -- and it is the third argument because
-- nothing else knows it. Pass `plan.bots`.
--
-- Two wrong answers have been recorded in this field already, in opposite
-- directions. Derived from the steps, it made a bot that got no work invisible:
-- a plan covering one bot out of four recorded `bots: [2]` and read exactly
-- like a run of one bot, so "why did bot 4 do nothing" could not be asked of
-- the record at all. Taken from the run's process roster instead, it lies the
-- other way whenever a script plans with `goal.plan{bots = ...}`: run 30
-- recorded `bots: [1, 2]` for a plan made for `[2]` alone, claiming a bot had
-- been offered work it was never offered.
--
-- Omit it and the field is recorded as `nil` -- present and null, never
-- absent, and never quietly replaced by the run's roster. A plan whose roster
-- nobody stated is a plan whose roster the record does not know, and the
-- ambient value is a plausible-looking substitute for that rather than a
-- weaker version of it. A `bots` that is not a list of positive bot ids is
-- refused outright, for the same reason `record.milestone_satisfied` refuses a
-- reason it does not recognise.
-- @number index the milestone's position in the run, from 1
-- @tparam table plan an array of step tables
-- @tparam[opt] table bots the roster the plan was made for, i.e. `plan.bots`
-- @raise if no recording is running, or if `bots` is not a list of bot ids
function record.plan_created(index, plan, bots)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "plan_created",
            lua.create_function(
                move |_lua, (index, plan, bots): (u32, LuaTable, Option<LuaValue>)| {
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
                    // `plan.tick` is the tick `goal.plan` handed the plan
                    // back at (`PlanValue.tick`, carried onto the shaped
                    // table by `supervisor.lua`). Without it the event was
                    // stamped with the last RCON reply's tick, which on a
                    // headless run is the run's *start*: `plan_created`
                    // read 415 while the game stood at 2,252, and the
                    // planning time it hid was the whole speed tax.
                    let made_at: Option<u64> = plan.get("tick")?;
                    let kind = EventKind::PlanCreated {
                        milestone_index: index,
                        steps: step_count,
                        makespan,
                        bots: roster_from_lua(bots)?,
                        plan: steps,
                    };
                    match made_at {
                        Some(tick) => record_at(&slot, tick, kind),
                        None => record_live(&slot, &rcon, kind),
                    }
                },
            )?,
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
                    // What this step put INTO something, read off the PLAN
                    // rather than off the label. `step_to_lua` publishes
                    // `kind = "insert"` with `item`, `count`, `entity` and
                    // `slot` for every `ActionKind::Insert`, which is every
                    // hand delivery including fuel; nothing else here is one.
                    //
                    // Read defensively -- a caller may hand `record.actions`
                    // a hand-built step table, and a missing field is a step
                    // with no delivery rather than an error.
                    let delivery = if step.get::<Option<String>>("kind")?.as_deref()
                        == Some("insert")
                    {
                        match (
                            step.get::<Option<String>>("item")?,
                            step.get::<Option<u32>>("count")?,
                            step.get::<Option<String>>("entity")?,
                            step.get::<Option<String>>("slot")?,
                        ) {
                            (Some(item), Some(count), Some(entity), Some(slot)) => Some(Delivery {
                                item,
                                count,
                                entity,
                                slot,
                            }),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    if let Some(dispatched) = dispatched {
                        recorder
                            .record(
                                dispatched,
                                EventKind::ActionDispatched {
                                    id,
                                    bot,
                                    action: label,
                                    target,
                                    delivery,
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
        "__doc_entry_walks",
        String::from(
            r#"
--- records where each bot walked during one executed plan
-- Takes `observation.walks` -- the array `record.actions` has no room for,
-- because a walk is not an action: the scheduler emits it as its own step
-- with no action id, so `(bot, step_index)` is the only thing that names one.
--
-- Walking is most of the wall clock in these plans, and until this existed no
-- walk reached `events.jsonl` at all: a run whose walks failed left one
-- `last_error` string in the record and everything else only in the server
-- log, which the next run overwrites.
--
-- Two events per walk, on exactly the same terms as `record.actions` writes
-- its two. A `walk_dispatched` needs a dispatch tick from the game, because a
-- walk the game never acknowledged was never dispatched. A `walk_settled`
-- needs only a verdict -- `success`, `failed` or `lost` -- and every walk that
-- reached one gets exactly one settle, whether or not a reply tick was ever
-- stamped: a lost walk never has one, which is precisely why it must not be
-- what the settle is gated on. `pending` and `running` write nothing.
--
-- `failed` and `lost` stay apart. The game refusing a walk and the game never
-- answering are different facts with different fixes, and the classified
-- `failure` keeps the distinction the pathfinder itself makes: a request queue
-- that would not take the search (`pathfinder_busy`, worth repeating) against a
-- search that came back empty (`no_path`, a fact about the destination). Where
-- the mod named positions, the failure carries them -- observed ones, taken at
-- the instant it gave up.
--
-- Call it once per loop iteration, alongside `record.actions`.
-- @tparam table walks `observation.walks`
-- @treturn number how many events were written
-- @raise if no recording is running
function record.walks(walks)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        map_table.set(
            "walks",
            lua.create_function(move |_lua, walks: LuaTable| {
                let mut written = 0u32;
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;

                for walk in walks.sequence_values::<LuaTable>() {
                    let walk = walk?;
                    let bot: u32 = walk.get("bot")?;
                    let step_index: u32 = walk.get("step_index")?;
                    // Read field-by-field rather than through a serde bridge,
                    // for the reason `record.actions` reads `target` that way:
                    // a walk destination is routinely a tile centre and must
                    // pass through exactly as the schedule set it.
                    let to: LuaTable = walk.get("to")?;
                    let to = position_from_lua(&to, "to")?;
                    let status: String = walk
                        .get::<Option<String>>("status")?
                        .unwrap_or_else(|| "unknown".into());
                    // A walk's planned span is known the moment the step
                    // exists -- unlike an action's, which is discovered by
                    // finishing -- so both ends are always present. The
                    // fallbacks are for a caller that passes neither, and
                    // produce a zero-length prediction rather than a
                    // fabricated one.
                    let planned_start: u64 = walk.get::<Option<u64>>("planned_start")?.unwrap_or(0);
                    let planned_end: u64 = walk
                        .get::<Option<u64>>("planned_end")?
                        .unwrap_or(planned_start);
                    let dispatched: Option<u64> = walk.get("dispatched_tick")?;
                    let replied: Option<u64> = walk.get("replied_tick")?;
                    let error: Option<String> = walk.get("error")?;

                    if let Some(dispatched) = dispatched {
                        recorder
                            .record(
                                dispatched,
                                EventKind::WalkDispatched {
                                    bot,
                                    step_index,
                                    to: to.clone(),
                                    planned_start,
                                    planned_duration: planned_end.saturating_sub(planned_start),
                                },
                            )
                            .map_err(record_error)?;
                        written += 1;
                    }
                    // Keyed on the **verdict**, never on a measured reply
                    // tick. `record.actions` reached that rule the expensive
                    // way -- its settle used to live inside `if let
                    // Some(replied)`, which made `status: "lost"` structurally
                    // unrecordable, since a lost attempt is *defined* by no
                    // reply ever arriving. This is built with that already
                    // known, and `a_lost_walk_settles_even_though_the_game_never_replied`
                    // pins it so it cannot be rebuilt.
                    if matches!(status.as_str(), "success" | "failed" | "lost") {
                        // `None` only on a genuine success. Anything else is a
                        // failure of some kind, even one the classifier cannot
                        // name, and `WalkFailureKind::Other` says so rather
                        // than leaving the field null.
                        let failure = (status != "success")
                            .then(|| classify_walk_failure(error.as_deref().unwrap_or("")));
                        // The game's own tick when it gave one; otherwise the
                        // record's high-water mark, which is the honest "no
                        // earlier than everything already written" -- never
                        // the dispatch tick, which would report a real
                        // instant and a duration of zero.
                        let settled_tick = match replied {
                            Some(replied) => replied,
                            None => recorder.not_before(dispatched.unwrap_or(0)),
                        };
                        // `Some` only when both ends were measured, so a
                        // synthesized stamp always carries a null duration and
                        // can never be mistaken for a timed span.
                        let elapsed_ticks =
                            dispatched.zip(replied).map(|(d, r)| r.saturating_sub(d));
                        recorder
                            .record(
                                settled_tick,
                                EventKind::WalkSettled {
                                    bot,
                                    step_index,
                                    to,
                                    status,
                                    elapsed_ticks,
                                    error,
                                    failure,
                                },
                            )
                            .map_err(record_error)?;
                        written += 1;
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
                                direction: refusal.direction,
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
        "__doc_entry_enclosures",
        String::from(
            r#"
--- flushes the bots found walled in, and those walked clear of it, since the last flush
-- A bot is "walled in" when a flood fill over the occupancy model, started
-- where it stands, closes without reaching open ground -- every placement
-- around it was individually legal and the *set* formed a wall. The fill is
-- run on the game's own pathfinding grid (one cell per tile, the character's
-- box tested at each tile's centre), because that is the grid that decides
-- whether a walk is refused: a crack a character fits through but the
-- pathfinder will not use is a wall. The check runs when the game's
-- pathfinder has already refused that bot a route from that spot, so each
-- `bot_enclosed` event sits beside a failed walk and explains it.
--
-- The same fill runs *before* every placement, from where the placing bot
-- stands, with the footprint added: when it would close, the executor walks
-- the bot to the nearest tile that stays open first and this writes a
-- `bot_stepped_aside` event saying so -- the enclosure that did not happen,
-- and the reason a `place` was preceded by a walk the plan has no step for.
--
-- It also flushes the *game's* verdict on who can move. When the pathfinder
-- refuses a walk, the executor asks it for a short hop in each of four
-- directions from the character; every hop refused means the character
-- cannot leave its own tile, and it is **benched** -- a `bot_benched` event
-- here, and no step that needs it to walk in any later plan, until a walk
-- for it succeeds or a re-probe before a plan finds a way out
-- (`bot_released`, with `why`). This one *does* change a plan, and is the
-- answer to `run-1788614781-38058`, where the fill above said "open" for a
-- bot standing in a furnace and seven plans re-sent it the same walk.
--
-- Call it once per loop iteration, alongside `record.actions`,
-- `record.teleports` and `record.refusals`. The fill's own events change
-- no plan. These exist because `run-1788432181-42528` ran its whole budget with two of
-- four bots frozen for 77% of it and no artefact said so, and because
-- `run-1788552801-73005` walled its own bot 1 in with a placement and this
-- function, on the finer grid it then used, wrote nothing.
--
-- Each `bot_enclosed` event carries `searched_tiles`, the radius the fill was
-- allowed. An enclosure wider than that window produces no event, so no
-- events is not evidence that no bot was walled in.
-- @treturn number how many events were written
-- @raise if no recording is running
function record.enclosures()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        let rcon = rcon.clone();
        map_table.set(
            "enclosures",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                for found in world.unreported_enclosures() {
                    // The observation's own stamp when it had one, which it
                    // ordinarily does not: a path request that finds nothing is
                    // refused before the walk is dispatched and so is never
                    // stamped. `not_before` clamps to the last tick already
                    // written rather than inventing a zero, exactly as
                    // `record.refusals` does.
                    let tick = recorder
                        .not_before(found.tick.unwrap_or_else(|| rcon.last_tick().unwrap_or(0)));
                    recorder
                        .record(
                            tick,
                            EventKind::BotEnclosed {
                                bot: u32::from(found.player),
                                position: found.at,
                                pocket_tiles: found.pocket_tiles,
                                searched_tiles: found.searched_tiles,
                            },
                        )
                        .map_err(record_error)?;
                    written += 1;
                }
                for step in world.drain_step_asides() {
                    // Stamped by the step-aside walk's own reply when the game
                    // gave one; otherwise the record's high-water mark, as
                    // above.
                    let tick = recorder
                        .not_before(step.tick.unwrap_or_else(|| rcon.last_tick().unwrap_or(0)));
                    recorder
                        .record(
                            tick,
                            EventKind::BotSteppedAside {
                                bot: u32::from(step.player),
                                from: step.from,
                                to: step.to,
                                placing: step.placing,
                                site: step.site,
                                reason: step.reason.as_str().to_string(),
                                pocket_tiles: step.pocket_tiles,
                            },
                        )
                        .map_err(record_error)?;
                    // Written, but NOT counted: every driver prints this
                    // call's answer as "WALLED IN: N bot(s) can no longer
                    // reach open ground", and a step-aside is the enclosure
                    // that did not happen. run-1788569499-05724 printed
                    // `WALLED IN: 1` for a bot that had just walked clear.
                }
                for change in world.drain_bench_changes() {
                    // The game's verdict on who can move, and its reversal.
                    // A bench is stamped by the refused walk that earned it,
                    // which ordinarily is nothing (refused before dispatch);
                    // a release by the walk or the probe that lifted it.
                    // Counted like an enclosure: a benched bot is a bot that
                    // can no longer reach open ground, by the game's own
                    // word, and the driver's "WALLED IN: N" line is the one
                    // place a person watching the run sees it. A release is
                    // not counted, for the reason a step-aside is not.
                    let (tick, event, counted) = match change {
                        BenchChange::Benched(bench) => (
                            bench.tick,
                            EventKind::BotBenched {
                                bot: u32::from(bench.player),
                                position: bench.at,
                                refused_hops: u32::from(bench.refused_hops),
                                hop_tiles: bench.hop_tiles,
                            },
                            true,
                        ),
                        BenchChange::Released {
                            tick,
                            player,
                            at,
                            why,
                        } => (
                            tick,
                            EventKind::BotReleased {
                                bot: u32::from(player),
                                position: at,
                                why: why.as_str().to_string(),
                            },
                            false,
                        ),
                    };
                    let tick =
                        recorder.not_before(tick.unwrap_or_else(|| rcon.last_tick().unwrap_or(0)));
                    recorder.record(tick, event).map_err(record_error)?;
                    if counted {
                        written += 1;
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
        "__doc_entry_deaths",
        String::from(
            r#"
--- flushes the bot deaths and respawns the mod has reported since the last flush
-- A bot that dies keeps its player and loses its character; the game
-- respawns one after `ticks_to_respawn` (600 by default). In between, every
-- action for it is refused with `player <n> has no character: ...`, which the
-- record classifies as `no_character`. This is the event that says WHY those
-- refusals happened: `bot_died` with the tick, where it stood, what killed it
-- when the game names a cause, and the respawn timer; then `bot_respawned`
-- when the character is back.
--
-- Call it once per loop iteration, alongside `record.actions`,
-- `record.teleports`, `record.refusals` and `record.enclosures`, and once
-- more after the loop. Each event is queued with the game tick it happened
-- at, so calling late blurs when it is written, not when it happened.
--
-- Nothing acts on these. The roster reacts through `supervisor.new`'s
-- `roster` option, which asks `rcon.players()` before every plan; see
-- `record.roster_changed` for what it writes when the answer changed.
-- @treturn number how many death and respawn events were written
-- @raise if no recording is running
function record.deaths()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        map_table.set(
            "deaths",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                for (tick, event) in world.drain_deaths() {
                    let tick = recorder.not_before(tick);
                    let kind = match event {
                        BotLifeEvent::Died(death) => EventKind::BotDied {
                            bot: u32::from(death.player_id),
                            position: death.position,
                            cause: death.cause,
                            cause_type: death.cause_type,
                            respawn_in: death.respawn_in,
                        },
                        BotLifeEvent::Respawned(back) => EventKind::BotRespawned {
                            bot: u32::from(back.player_id),
                            position: back.position,
                        },
                    };
                    recorder.record(tick, kind).map_err(record_error)?;
                    written += 1;
                }
                Ok(written)
            })?,
        )?;
    }

    map_table.set(
        "__doc_entry_research_triggers",
        String::from(
            r#"
--- flushes the trigger technologies the mod has completed on a headless run
-- Factorio 2.0 unlocks some technologies by *doing* -- craft a lab, smelt
-- 50 iron plates -- and the game fires those from a player's actions. A
-- server-side character has no player: the game still fires `mine-entity`
-- and a furnace's `craft-item`, but never a hand craft's, and never
-- `build-entity`. The mod's sweep completes those from its own counters once
-- the act has happened and every prerequisite is researched, and each
-- completion is queued as a `research_trigger_emulated` event naming the
-- technology, the trigger kind, what was counted and how many against how
-- many were needed. This flushes that queue into `events.jsonl`.
--
-- Call it once per loop iteration, alongside `record.actions` and
-- `record.deaths`, and once more after the loop. A run with real players
-- writes none of these: the game fires every trigger itself.
-- @treturn number how many events were written
-- @raise if no recording is running
function record.research_triggers()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        map_table.set(
            "research_triggers",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                for (tick, event) in world.drain_research_triggers() {
                    let tick = recorder.not_before(tick);
                    let count = event.count();
                    recorder
                        .record(
                            tick,
                            EventKind::ResearchTriggerEmulated {
                                technology: event.technology,
                                trigger: event.trigger,
                                item: event.item,
                                entity: event.entity,
                                needed: event.needed,
                                count,
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
        "__doc_entry_surface_chunks_dropped",
        String::from(
            r#"
--- flushes the chunks the mod discarded for being on another surface
-- This project supports exactly one surface. `on_chunk_generated` in the
-- BotBridge mod returns early for any surface but Nauvis, because the world
-- model keys entities, resources and tiles by position alone and a second
-- surface's chunk would merge into Nauvis with no error anywhere. Space Age
-- is enabled here, so that second surface is one rocket away.
--
-- The drop used to reach only the server log. This flushes it into
-- `events.jsonl` as one `surface_chunk_dropped` row per surface, carrying how
-- many chunks were discarded since the last call and one example chunk. A run
-- that writes none of these never met a second surface, which is every run so
-- far; a run that writes one planned against an incomplete world.
--
-- Call it beside `record.actions` and `record.deaths`.
-- @treturn number how many events were written
-- @raise if no recording is running
function record.surface_chunks_dropped()
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let world = world.clone();
        map_table.set(
            "surface_chunks_dropped",
            lua.create_function(move |_lua, ()| {
                let mut guard = slot.lock();
                let recorder = guard.as_mut().ok_or_else(|| {
                    record_error("no recording is running -- call record.start() first")
                })?;
                let mut written = 0u32;
                // **By first tick, not by name.** `RunRecorder::not_before`
                // keeps `events.jsonl` non-decreasing in tick, so a row
                // written out of order is silently stamped with the previous
                // row's tick -- writing `gleba` (t=602) before `vulcanus`
                // (t=600) made vulcanus's row read 602 and lost the fact that
                // it was seen first. The map is keyed by name because that is
                // the identity; the *order* has to be chronological.
                let mut rows: Vec<_> = world.drain_surface_chunk_drops().into_iter().collect();
                rows.sort_by(|(a_name, a), (b_name, b)| {
                    a.first_tick.cmp(&b.first_tick).then(a_name.cmp(b_name))
                });
                for (surface, drops) in rows {
                    let tick = recorder.not_before(drops.first_tick);
                    recorder
                        .record(
                            tick,
                            EventKind::SurfaceChunkDropped {
                                surface: surface.to_string(),
                                chunks: drops.chunks,
                                first_left_top_x: drops.first_left_top.x,
                                first_left_top_y: drops.first_left_top.y,
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
        "__doc_entry_roster_changed",
        String::from(
            r#"
--- records that the supervisor changed the roster it plans for
-- Written from the supervisor's `rerostered` transition: a bot that was in
-- the roster and has had no character for longer than the bounded respawn
-- wait has been dropped (`left`), or one dropped earlier has come back
-- (`returned`). `bots` is the roster from here on. Every later
-- `plan_created.bots` will show the new roster; this is the line that says
-- why it changed.
--
-- The lists are read as bot ids and written ascending; `left` and `returned`
-- may be empty but not both, and `bots` may not be empty -- a roster of
-- nobody is not something this loop ever plans for, so recording one would be
-- recording a bug.
-- @tparam {number,...} bots the roster from now on
-- @tparam {number,...} left bots dropped from the roster
-- @tparam {number,...} returned bots picked back up
-- @string reason the supervisor's own sentence for why
-- @raise if no recording is running, or the lists do not describe a change
function record.roster_changed(bots, left, returned, reason)
end
    "#,
        ),
    )?;
    {
        let slot = slot.clone();
        let rcon = rcon.clone();
        map_table.set(
            "roster_changed",
            lua.create_function(
                move |_lua,
                      (bots, left, returned, reason): (
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    String,
                )| {
                    let bots = bot_list_from_lua("bots", bots)?;
                    let left = bot_list_from_lua("left", left)?;
                    let returned = bot_list_from_lua("returned", returned)?;
                    if bots.is_empty() {
                        return Err(record_error(
                            "record.roster_changed: bots is empty; the supervisor never plans for nobody",
                        ));
                    }
                    if left.is_empty() && returned.is_empty() {
                        return Err(record_error(
                            "record.roster_changed: neither left nor returned names a bot, so nothing changed",
                        ));
                    }
                    record_live(
                        &slot,
                        &rcon,
                        EventKind::RosterChanged {
                            bots,
                            left,
                            returned,
                            reason,
                        },
                    )
                },
            )?,
        )?;
    }

    map_table.set(
        "__doc_entry_finish",
        String::from(
            r#"
--- closes the recording
-- Writes the manifest and the derived splits, and copies this run's video out
-- of the workspace before the next run overwrites it. A recording belonging to
-- another run is left where it is. Also catches up on any samples the mod
-- wrote since the last `record.keyframe()` call (or all of them, if this run
-- never reached one) -- the same incremental ingestion `record.keyframe()`
-- runs at every milestone boundary, so nothing is read or archived twice.
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

// Deliberately at the bottom of the file, below every `.set("name", ...)`
// above. `doc_guard`'s `production_half` cuts a source at its FIRST
// `\n#[cfg(test)]` and scans only what precedes it, so a test-only item placed
// higher up hides every binding registration under it and
// `every_doc_block_matches_the_closure_its_binding_is_installed_with` reports
// twelve bindings as uninstalled. That failure names the bindings and says
// nothing about the `#[cfg(test)]` that caused it.
#[cfg(test)]
impl LiveRecord {
    /// A handle over an already-open recorder.
    ///
    /// For the tests of the bindings *below* the Lua seam that write through
    /// this: they need somewhere real for an event to land, and they cannot
    /// get there through `record.start()`, which always makes a live RCON call
    /// (`sampling_start`) that no test has a game to answer.
    pub(crate) fn for_tests(recorder: RunRecorder) -> Self {
        LiveRecord {
            slot: Arc::new(Mutex::new(Some(recorder))),
            rcon: Arc::new(factorio_bot_core::factorio::rcon::FactorioRcon::new_empty()),
        }
    }
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

    #[test]
    fn a_live_record_with_no_recording_running_declines_rather_than_failing() {
        // The state every planning-only interpreter is in, and the state a run
        // started before `record.start()` is in. A heartbeat that treated this
        // as an error would turn "nobody is recording" into a failed run.
        let live = LiveRecord {
            slot: Arc::new(Mutex::new(None)),
            rcon: Arc::new(FactorioRcon::new_empty()),
        };
        assert!(
            !live.record(EventKind::RunFinished {
                outcome: "test".to_string(),
                elapsed_ticks: 0,
            }),
            "a live record with an empty slot writes nothing and says so"
        );
    }

    #[test]
    fn a_live_record_writes_through_the_same_slot_the_bindings_use() {
        // The seam's whole point: `record.start()` fills the slot later, so a
        // handle taken now must see the recorder that arrives afterwards --
        // which it does only because it holds the slot rather than a snapshot
        // of what was in it.
        let tmp = tempfile::tempdir().expect("tempdir");
        let recorder = RunRecorder::start(tmp.path(), "run-live").expect("recorder starts");
        let run_dir = recorder.dir().to_path_buf();
        let slot: Slot = Arc::new(Mutex::new(None));
        let live = LiveRecord {
            slot: slot.clone(),
            rcon: Arc::new(FactorioRcon::new_empty()),
        };
        assert!(!live.record(EventKind::RunFinished {
            outcome: "too early".to_string(),
            elapsed_ticks: 0,
        }));
        *slot.lock() = Some(recorder);
        assert!(live.record(EventKind::RunFinished {
            outcome: "recorded".to_string(),
            elapsed_ticks: 7,
        }));
        let text = std::fs::read_to_string(run_dir.join("events.jsonl")).expect("events");
        assert!(
            text.contains("\"outcome\":\"recorded\"") && !text.contains("too early"),
            "only the event written once a recorder was in the slot is on disk: {text}"
        );
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

    /// **`plan.tick` is the event's tick.** Without it the event takes the
    /// last RCON reply's tick, which on a headless run is the run's start --
    /// `plan_created` read 415 while the game stood at 2,252, and the
    /// planning time that hid was the whole speed tax. The stub RCON here
    /// has never answered, so the only way this event can read 4,180 is
    /// through the field.
    #[test]
    fn plan_created_is_stamped_with_the_tick_the_plan_was_made_at() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2]);
        lua.load(
            r#"
            local plan = {
                { id = 1, bot = 1, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
            }
            plan.tick = 4180
            record.plan_created(1, plan, { 1, 2 })
            "#,
        )
        .exec()
        .expect("plan_created runs");
        assert_eq!(read_event_ticks(&run_dir), vec![4180]);
        match &read_events(&run_dir)[0] {
            EventKind::PlanCreated { steps, .. } => assert_eq!(*steps, 1),
            other => panic!("expected plan_created, got {other:?}"),
        }
    }

    /// Without the field the event falls back to the last observed tick, as
    /// it always did -- and never to a tick earlier than the log already has.
    #[test]
    fn plan_created_without_a_tick_is_stamped_live() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2]);
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 1, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
            }, { 1, 2 })
            "#,
        )
        .exec()
        .expect("plan_created runs");
        let ticks = read_event_ticks(&run_dir);
        assert_eq!(ticks.len(), 1);
        assert!(
            ticks[0] < 4180,
            "no plan.tick, so not the planning clock's stamp: {ticks:?}"
        );
    }

    /// `bots` is the roster the plan was made for, not the bots it used.
    ///
    /// The plan below gives work to 1 and 2 out of a roster of four. Derived
    /// from the steps -- which is what this did -- the record said
    /// `bots: [1, 2]` and a live run said `bots: [2]`, which reads exactly
    /// like a run of one bot and makes "why did bot 4 do nothing"
    /// unanswerable from the record. The bots that got nothing are the whole
    /// question.
    ///
    /// The roster is now the caller's to state, and this passes one that
    /// differs from the binding's ambient roster in *both* directions -- it
    /// omits a process bot and includes one the process never had -- so
    /// neither a fallback to the ambient value nor an intersection with it
    /// could pass.
    #[test]
    fn plan_created_reports_the_runs_roster_and_not_the_bots_in_the_plan() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2, 3, 9]);
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 2, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
                { id = 2, bot = 1, action = "smelt 10 iron-plate", deps = { 1 },
                  planned_start = 300, planned_duration = 600 },
            }, { 4, 1, 2, 3 })
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
                    Some(vec![1, 2, 3, 4]),
                    "the roster the plan was made for, ascending, including the \
                     two bots this plan gave no work to"
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
        lua.load("record.plan_created(3, {}, { 1, 2 })")
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
                assert_eq!(
                    *bots,
                    Some(vec![1, 2]),
                    "the roster, even with nothing planned"
                );
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
            .filter_map(|e| match e {
                EventKind::MilestoneSatisfied { reason, .. } => Some(*reason),
                _ => None,
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

    /// Closing a milestone now also saves the world, and there is no game here
    /// to save it -- so this is the failure path, which is the one that
    /// matters: the milestone must still be recorded, the script must still
    /// run, and the record must say a savepoint was attempted and lost.
    ///
    /// A savepoint is a souvenir of a milestone, not part of reaching one. If
    /// this ever starts raising, every run loses its milestones the first time
    /// a disk fills up.
    #[test]
    fn a_savepoint_that_cannot_be_taken_is_recorded_and_does_not_fail_the_milestone() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(r#"record.milestone_satisfied(1, 0, "already_satisfied")"#)
            .exec()
            .expect("a milestone closes even when its savepoint cannot be written");

        let events = read_events(&run_dir);
        assert!(
            matches!(events.first(), Some(EventKind::MilestoneSatisfied { .. })),
            "the milestone is recorded first, and unconditionally: {events:?}"
        );
        match events.get(1) {
            Some(EventKind::SavepointFailed {
                milestone_index,
                error,
            }) => {
                assert_eq!(*milestone_index, 1);
                assert!(
                    !error.is_empty(),
                    "a failure with no reason is the silence this event exists to break"
                );
            }
            other => panic!("expected savepoint_failed beside the milestone, got {other:?}"),
        }
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

    /// The exact line `run-1788481380-80843` recorded as
    /// `kind: "rejected", detail: null`, which put the failure that cost the
    /// run its plan in the same bucket as a full chest and an absent ore.
    ///
    /// It reaches a record at all only when `place_entity_timed`'s retries have
    /// run out -- the blocker never moved -- and that is precisely the case a
    /// reader needs to be able to find.
    #[test]
    fn a_character_in_the_footprint_is_blocked_not_merely_rejected() {
        assert_eq!(
            recorded_failure(
                "failed",
                r#""game rejected the command: Unexpected Response: cannot place item 'stone-furnace' because a character is standing in the footprint""#
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

    /// A hand delivery carries its numbers, not just its prose.
    ///
    /// The quantity was always in the label — *"fuel the stone-furnace with 23
    /// coal"* — and reading it back means parsing sentences. The **hand-credit
    /// mass balance** (`tools/run_analysis.py`) has to price every delivery a
    /// run made, and reports `unknown` rather than a verdict when it cannot,
    /// so the numbers travel as fields. A `fuel` action is an `insert` into
    /// the fuel slot: `step.kind` says `insert` and `step.slot` says which
    /// inventory, exactly as `goal.plan` publishes them.
    #[test]
    fn an_insert_records_what_it_delivered() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                local steps = {
                    {
                        id = 2, bot = 1, kind = "insert",
                        label = "fuel the stone-furnace with 23 coal",
                        entity = "stone-furnace", slot = "fuel",
                        item = "coal", count = 23,
                    },
                }
                local actions = {
                    [2] = { status = "success", dispatched_tick = 4520, replied_tick = 4520 },
                }
                return record.actions(steps, actions)
                "#,
            )
            .eval()
            .expect("record.actions runs");
        assert_eq!(written, 2);
        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::ActionDispatched { delivery, .. } => assert_eq!(
                delivery.as_ref(),
                Some(&Delivery {
                    item: "coal".into(),
                    count: 23,
                    entity: "stone-furnace".into(),
                    slot: "fuel".into(),
                }),
                "the delivery travels as numbers, not as a sentence to re-parse"
            ),
            other => panic!("expected action_dispatched, got {other:?}"),
        }
    }

    /// Everything that is not an insert delivers nothing, and says so.
    ///
    /// A `take` moves material OUT of a machine and into a bot, which credits
    /// no production; a `craft` and a `place` touch no machine inventory at
    /// all. Recording a delivery for any of them would put credit in the
    /// balance that the roster never handed over.
    #[test]
    fn a_take_or_a_craft_records_no_delivery() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                local steps = {
                    {
                        id = 1, bot = 1, kind = "remove",
                        label = "take 50 iron-plate from the stone-furnace",
                        entity = "stone-furnace", slot = "furnace_result",
                        item = "iron-plate", count = 50,
                    },
                    { id = 2, bot = 1, kind = "craft", label = "craft 1 stone-furnace",
                      item = "stone-furnace", count = 1 },
                }
                local actions = {
                    [1] = { status = "success", dispatched_tick = 10, replied_tick = 10 },
                    [2] = { status = "success", dispatched_tick = 20, replied_tick = 20 },
                }
                return record.actions(steps, actions)
                "#,
            )
            .eval()
            .expect("record.actions runs");
        assert_eq!(written, 4);
        for event in read_events(&run_dir) {
            if let EventKind::ActionDispatched { delivery, .. } = event {
                assert_eq!(delivery, None, "only an insert delivers");
            }
        }
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
            // The electric network, admitted to `EntityGraph::add` on
            // 2026-09-02. These three are exactly the case this test's doc
            // warns about: a type `add` tracks and this filter does not is a
            // permanent `only_in: "model"` divergence on every keyframe that
            // sees a pole.
            "electric-pole",
            "generator",
            "solar-panel",
        ] {
            assert!(
                keyframe_relevant(entity_type, "some-entity"),
                "{entity_type} is one of EntityGraph::add's tracked types"
            );
        }
        // And the game-side half of the filter has to ask for them, or they
        // are dropped before the reply leaves the server and diverge just the
        // same, in the other direction.
        let asked_for = keyframe_relevant_types();
        for entity_type in ["electric-pole", "generator", "solar-panel"] {
            assert!(
                asked_for.iter().any(|t| t == entity_type),
                "the game-side type filter must ask for {entity_type} too, got {asked_for:?}"
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
                    0,
                    Vec::new(),
                    None,
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
                direction,
                source,
                blockers,
                tile,
            } => {
                assert_eq!(entity, "stone-furnace");
                assert_eq!(position.x, -16.);
                assert_eq!(position.y, -58.);
                assert_eq!(*direction, Some(0));
                assert_eq!(source, "dispatch");
                assert!(
                    blockers.is_empty() && tile.is_none(),
                    "a refusal whose reply named nothing reports nothing: \
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
            direction: Some(4),
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
                0,
                Vec::new(),
                None,
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

    // -------------------------------------------------------- bot deaths

    /// `mods/BotBridge/control.lua`, verbatim, so the wording tests below
    /// generate the sentence from the mod's own code instead of from a string
    /// typed here. A copy would pass against wording the mod no longer
    /// prints -- which is how nineteen of twenty walk failures in
    /// `run-1788432181-42528` came to be archived as `other`.
    const BOTBRIDGE_CONTROL_LUA: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../mods/BotBridge/control.lua"
    ));

    /// Enough of the Factorio runtime for `control.lua` to load and for its
    /// player-facing helpers to run: `defines` (any unknown key answers with
    /// its own name, which is what event and inventory ids are used for at
    /// load time), a `script`/`remote` that register nothing, a
    /// `helpers.table_to_json` that renders sorted keys, and a `print`/`rcon`
    /// that capture. Everything is captured into `__printed`.
    const FACTORIO_STUB: &str = r#"
        local any = setmetatable({}, { __index = function(_, k) return k end })
        defines = {
            events = any,
            controllers = { character = 1, god = 2, cutscene = 3, ghost = 4,
                            spectator = 5, editor = 6, remote = 7 },
            inventory = any, direction = any, build_mode = any,
            chunk_generated_status = any, entity_status = any,
            rail_direction = any, wire_type = any, riding = any,
        }
        script = { on_init = function() end, on_load = function() end,
                   on_event = function() end, on_nth_tick = function() end,
                   on_configuration_changed = function() end }
        remote = { add_interface = function() end, interfaces = {}, call = function() end }
        helpers = {}
        helpers.table_to_json = function(t)
            local keys = {}
            for k in pairs(t) do keys[#keys + 1] = tostring(k) end
            table.sort(keys)
            local parts = {}
            for _, k in ipairs(keys) do
                local v = t[k]
                if type(v) == "table" then v = helpers.table_to_json(v)
                elseif type(v) == "string" then v = '"' .. v .. '"'
                else v = tostring(v) end
                parts[#parts + 1] = '"' .. k .. '":' .. v
            end
            return "{" .. table.concat(parts, ",") .. "}"
        end
        __printed = {}
        print = function(...) __printed[#__printed + 1] = table.concat({...}, " ") end
        rcon = { print = function(s) __printed[#__printed + 1] = "RCON:" .. s end }
        prototypes = { item = any, entity = any }
        storage = { p = {} }
        require = function() end
        game = { players = {}, forces = { player = { print = function() end } },
                 surfaces = {}, tick = 0, connected_players = {} }
    "#;

    /// The real mod, loaded into the sandbox under the stub above.
    fn mod_lua() -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.load(FACTORIO_STUB).exec().expect("stub installs");
        lua.load(BOTBRIDGE_CONTROL_LUA)
            .exec()
            .expect("mods/BotBridge/control.lua loads under the stub");
        lua
    }

    /// **The wording is pinned from both ends.** The mod's `get_player`
    /// prints it for a dead bot; `classify_failure` reads it back as
    /// `NoCharacter` with the respawn timer in the detail. The sentence is
    /// produced by the mod's own code here, not typed twice.
    #[test]
    fn the_mods_own_no_character_refusal_classifies_as_no_character() {
        let lua = mod_lua();
        let refusal: String = lua
            .load(
                r#"
                local dead = { connected = true, character = nil, ticks_to_respawn = 587,
                               controller_type = defines.controllers.ghost, index = 2 }
                game.players[2] = dead
                storage.p[2] = {}
                assert(get_player(2) == nil, "a dead player is refused")
                return __printed[#__printed]
                "#,
            )
            .eval()
            .expect("get_player runs");
        assert_eq!(
            refusal, "RCON:Error: player 2 has no character: dead, respawns in 587 ticks",
            "the exact reply body a dead bot's dispatch gets"
        );
        // As it reaches the record: wrapped by `RconError` and then by
        // `ActuatorError::Rejected`, the way a refused `action_start_*`
        // travels.
        let as_recorded = format!(
            "game rejected the command: Unexpected Response: {}",
            refusal.trim_start_matches("RCON:")
        );
        assert_eq!(
            classify_failure(&as_recorded),
            ActionFailure {
                kind: FailureKind::NoCharacter,
                detail: Some("dead, respawns in 587 ticks".to_string()),
            }
        );
        // And on the mining road, where the reply lines arrive `Debug`-rendered.
        let mining = format!(
            "game rejected the command: Unexpected Response: [\"{}\"]",
            refusal.trim_start_matches("RCON:")
        );
        assert_eq!(
            classify_failure(&mining).detail.as_deref(),
            Some("dead, respawns in 587 ticks"),
            "the closing quote and bracket are the wrapper's, not the reason's"
        );
    }

    /// The other two causes of a missing character keep their own detail, so
    /// a reader can tell a respawn wait from the crash-site cutscene.
    #[test]
    fn a_cutscene_and_an_unknown_controller_are_named_in_the_detail() {
        let lua = mod_lua();
        let (cutscene, god): (String, String) = lua
            .load(
                r#"
                local cut = { connected = true, character = nil,
                              controller_type = defines.controllers.cutscene }
                local god = { connected = true, character = nil,
                              controller_type = defines.controllers.god }
                return no_character_error(1, cut), no_character_error(3, god)
                "#,
            )
            .eval()
            .expect("no_character_error runs");
        assert_eq!(cutscene, "Error: player 1 has no character: in a cutscene");
        assert_eq!(god, "Error: player 3 has no character: controller 'god'");
        assert_eq!(
            classify_failure(&cutscene).detail.as_deref(),
            Some("in a cutscene")
        );
        assert_eq!(classify_failure(&god).kind, FailureKind::NoCharacter);
        let alive: bool = lua
            .load("return character_missing_reason({ character = {} }) == nil")
            .eval()
            .expect("runs");
        assert!(alive, "a player with a character has no missing reason");
    }

    /// `on_player_died` fails the walk, the mine and every craft in flight
    /// with a verdict carrying the same wording, at the death tick, and
    /// writes the `player_died` line `OutputParser` reads. Both halves are
    /// checked against the classifiers here, and the writeout's shape is
    /// the one `death_tests` in `output_parser.rs` parses.
    #[test]
    fn a_death_fails_everything_in_flight_with_the_no_character_wording() {
        let lua = mod_lua();
        let printed: Vec<String> = lua
            .load(
                r#"
                local dead = { connected = true, character = nil, ticks_to_respawn = 587,
                               controller_type = defines.controllers.ghost,
                               position = { x = 12.5, y = -3 }, index = 2 }
                game.players[2] = dead
                storage.p[2] = { walking = { action_id = 41 }, mining = { action_id = 42 } }
                storage.craft_actions = { [2] = { ["iron-gear-wheel"] = { { id = 43, remaining = 2 } } } }
                __printed = {}
                on_player_died({ tick = 1234, player_index = 2,
                                 cause = { valid = true, name = "medium-worm-turret", type = "turret" } })
                assert(storage.p[2].walking == nil, "the walk is cleared")
                assert(storage.p[2].mining == nil, "the mine is cleared")
                assert(storage.craft_actions[2] == nil, "the craft bucket is cleared")
                return __printed
                "#,
            )
            .eval()
            .expect("on_player_died runs");
        assert_eq!(
            printed[0],
            r#"§1234§player_died§{"cause":"medium-worm-turret","cause_type":"turret","player_id":2,"position":{"x":12.5,"y":-3},"respawn_in":587}"#
        );
        let verdict = "ERROR: player 2 has no character: died at tick 1234 killed by medium-worm-turret, respawns in 587 ticks";
        assert_eq!(printed[1], verdict, "the narration line");
        assert_eq!(
            printed[2],
            format!("§1234§action_completed§fail 41 {verdict}")
        );
        assert_eq!(
            printed[3],
            format!("§1234§action_completed§fail 42 {verdict}")
        );
        assert_eq!(
            printed[4],
            format!("§1234§action_completed§fail 43 {verdict}")
        );
        assert_eq!(printed.len(), 5);
        // A walk failed this way is `no_character`, never `no_path`: the
        // wording says nothing about the map.
        assert_eq!(
            classify_walk_failure(&format!("game rejected the command: {verdict}")).kind,
            WalkFailureKind::NoCharacter
        );
        assert_eq!(
            classify_failure(verdict),
            ActionFailure {
                kind: FailureKind::NoCharacter,
                detail: Some(
                    "died at tick 1234 killed by medium-worm-turret, respawns in 587 ticks"
                        .to_string()
                ),
            }
        );
    }

    /// A pre-dispatch refusal now names where the character stood and what
    /// goal the game was asked for, in the wording `walk_endpoints` reads.
    ///
    /// The exact line `run-1788552801-73005` archived three times, from one
    /// spot, as `no_path` from nowhere to nowhere -- plus the suffix
    /// `RconActuator::walk` appends when it holds the character's position.
    /// The numbers are the run's own: bot 1 at `(34.41796875, -4.62890625)`,
    /// the first refused goal `(43.5, -0.5)`.
    #[test]
    fn a_pre_dispatch_refusal_carries_the_origin_and_the_goal_it_asked_for() {
        let archived = "game rejected the command: the game's pathfinder returned no path: \
                        Error: failed to path find";
        let bare = classify_walk_failure(archived);
        assert_eq!(bare.kind, WalkFailureKind::NoPath);
        assert_eq!(
            (bare.from, bare.destination),
            (None, None),
            "the archived shape"
        );

        let now =
            format!("{archived} -- found no path from (34.41796875/-4.62890625) to (43.5/-0.5)");
        let failure = classify_walk_failure(&now);
        assert_eq!(failure.kind, WalkFailureKind::NoPath);
        assert_eq!(failure.from, Some(Position::new(34.41796875, -4.62890625)));
        assert_eq!(failure.destination, Some(Position::new(43.5, -0.5)));
    }

    /// A dead bot's walk fails at the path request, and `player_path` hands
    /// that back inside wording the `NoPath` arm matches. The
    /// `has no character` arm is tested first, so the record says the bot
    /// had no character rather than that the destination is unreachable.
    #[test]
    fn a_walk_refused_for_no_character_is_not_read_as_no_path() {
        let error = "game rejected the command: the game's pathfinder returned no path: \
                     Error: player 2 has no character: dead, respawns in 587 ticks";
        let failure = classify_walk_failure(error);
        assert_eq!(failure.kind, WalkFailureKind::NoCharacter);
        assert_eq!(failure.from, None);
        assert_eq!(failure.destination, None);
    }

    /// `run-1788608011-14361`, bot 4, step 16, verbatim: `judge_path` refused
    /// the route because its last waypoint was inside the neighbouring
    /// big-rock's box. It was archived as `other`, which is the one answer a
    /// classifier must not give to the most-refused walk of the day.
    #[test]
    fn a_walk_refused_for_ending_inside_a_box_is_destination_blocked() {
        let error = "game rejected the command: the walk to \
                     [-18.56129845185668, 22.239337181155513] would end at \
                     [-18.55859375, 22.23828125], inside a collision box spanning \
                     [-18.56, 20.6] to [-16.56, 22.5] — a character cannot stand \
                     there, so the walk could only stall";
        let failure = classify_walk_failure(error);
        assert_eq!(failure.kind, WalkFailureKind::DestinationBlocked);
        assert_eq!(failure.from, None);
        assert_eq!(failure.destination, None);
    }

    /// `RconActuator::walk`'s wording for a walk the mobility probe found the
    /// bot could not have made from anywhere: the `found no path` refusal is
    /// still in the string, and the bench is the finding.
    #[test]
    fn a_walk_refused_from_a_boxed_in_character_is_boxed_in_not_no_path() {
        let error = "game rejected the command: the game's pathfinder returned no path: \
                     Error: failed to path find -- found no path from \
                     (-5.203125/-29.09765625) to (12.5/-33.5) -- and the character is \
                     boxed in: the game refused all 4 short hops of 3 tiles from where it \
                     stands, so it is benched until it can move";
        let failure = classify_walk_failure(error);
        assert_eq!(failure.kind, WalkFailureKind::BoxedIn);
        assert_eq!(
            failure.from.map(|p| (p.x(), p.y())),
            Some((-5.203125, -29.09765625)),
            "the endpoints are still read off the same wording"
        );
        assert_eq!(
            failure.destination.map(|p| (p.x(), p.y())),
            Some((12.5, -33.5))
        );
    }

    /// A bench and its release reach `events.jsonl` through
    /// `record.enclosures()`, the way an enclosure does -- and the bench is
    /// counted in the answer while the release is not, for the same reason a
    /// step-aside is not.
    #[test]
    fn a_bench_and_its_release_reach_events_jsonl() {
        use factorio_bot_core::factorio::world::{Bench, BenchRelease, HOP_DISTANCE};
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

        let at = Position::new(-5.203125, -29.09765625);
        world.record_bench(Bench {
            tick: Some(26_953),
            player: 6,
            at: at.clone(),
            refused_hops: 4,
            hop_tiles: HOP_DISTANCE,
        });
        let written: u32 = lua
            .load("return record.enclosures()")
            .eval()
            .expect("record.enclosures() runs");
        assert_eq!(
            written, 1,
            "a benched bot is a bot that cannot reach open ground"
        );

        world.release_bench(6, Some(40_000), BenchRelease::Walked);
        let written: u32 = lua
            .load("return record.enclosures()")
            .eval()
            .expect("record.enclosures() runs again");
        assert_eq!(written, 0, "a release is written but not counted");

        let events = read_events(&run_dir);
        assert_eq!(events.len(), 2, "{events:?}");
        match &events[0] {
            EventKind::BotBenched {
                bot,
                position,
                refused_hops,
                hop_tiles,
            } => {
                assert_eq!(*bot, 6);
                assert_eq!(position, &at);
                assert_eq!(*refused_hops, 4);
                assert_eq!(*hop_tiles, HOP_DISTANCE);
            }
            other => panic!("expected bot_benched, got {other:?}"),
        }
        match &events[1] {
            EventKind::BotReleased { bot, position, why } => {
                assert_eq!(*bot, 6);
                assert_eq!(position, &at);
                assert_eq!(why, "walked");
            }
            other => panic!("expected bot_released, got {other:?}"),
        }
        assert_eq!(read_event_ticks(&run_dir), vec![26_953, 40_000]);
    }

    /// The mod's `surface_chunk_dropped` lines through the real parser into
    /// `events.jsonl`.
    ///
    /// **The whole point of this road is that it used to end nowhere.** The
    /// mod printed a bare `"unknown surface"`, which carries no `§tick§key§`
    /// envelope, so the drop reached the server log and no artefact: a run
    /// that discarded a whole planet's chunks was indistinguishable, in every
    /// file anyone reads, from one that never left Nauvis. Four chunks in,
    /// two rows out -- one per surface, folded, with the first chunk and the
    /// first tick of each.
    #[test]
    fn a_dropped_surface_reaches_events_jsonl_through_the_real_parser() {
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
        for (tick, surface, x, y) in [
            (600_u64, "vulcanus", -32, 64),
            (601, "vulcanus", 0, 0),
            (602, "gleba", 96, -96),
        ] {
            parser
                .parse(
                    tick,
                    "surface_chunk_dropped",
                    &format!(r#"{{"left_top":{{"x":{x},"y":{y}}},"surface":"{surface}"}}"#),
                )
                .expect("drop line parses");
        }

        let written: u32 = lua
            .load("return record.surface_chunks_dropped()")
            .eval()
            .expect("record.surface_chunks_dropped() runs");
        assert_eq!(written, 2, "two surfaces, not three chunks");
        assert_eq!(
            read_events(&run_dir),
            vec![
                EventKind::SurfaceChunkDropped {
                    surface: "vulcanus".into(),
                    chunks: 2,
                    first_left_top_x: -32.0,
                    first_left_top_y: 64.0,
                },
                EventKind::SurfaceChunkDropped {
                    surface: "gleba".into(),
                    chunks: 1,
                    first_left_top_x: 96.0,
                    first_left_top_y: -96.0,
                },
            ],
            "chronological, not alphabetical -- vulcanus was seen first"
        );
        // Each row is stamped with the tick its surface first appeared, not
        // the tick of the flush. This assertion is the one that caught the
        // ordering: written name-first, gleba (602) went out ahead of
        // vulcanus (600) and `not_before` silently restamped vulcanus 602.
        assert_eq!(read_event_ticks(&run_dir), vec![600, 602]);
        let again: u32 = lua
            .load("return record.surface_chunks_dropped()")
            .eval()
            .expect("runs on an empty tally");
        assert_eq!(again, 0, "the tally was drained");
    }

    /// The mod's `research_trigger_emulated` line through the real parser
    /// into `events.jsonl`, both spellings of the count: `produced` for a
    /// `craft-item` trigger, `built` for a `build-entity` one.
    #[test]
    fn an_emulated_trigger_reaches_events_jsonl_through_the_real_parser() {
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
                22_680,
                "research_trigger_emulated",
                r#"{"technology":"automation-science-pack","trigger":"craft-item","item":"lab","needed":1,"produced":1}"#,
            )
            .expect("craft-item line parses");
        parser
            .parse(
                30_000,
                "research_trigger_emulated",
                r#"{"technology":"space-science-pack","trigger":"build-entity","entity":"asteroid-collector","needed":1,"built":2}"#,
            )
            .expect("build-entity line parses");

        let written: u32 = lua
            .load("return record.research_triggers()")
            .eval()
            .expect("record.research_triggers() runs");
        assert_eq!(written, 2);
        let events = read_events(&run_dir);
        assert_eq!(
            events,
            vec![
                EventKind::ResearchTriggerEmulated {
                    technology: "automation-science-pack".into(),
                    trigger: "craft-item".into(),
                    item: Some("lab".into()),
                    entity: None,
                    needed: 1,
                    count: 1,
                },
                EventKind::ResearchTriggerEmulated {
                    technology: "space-science-pack".into(),
                    trigger: "build-entity".into(),
                    item: None,
                    entity: Some("asteroid-collector".into()),
                    needed: 1,
                    count: 2,
                },
            ]
        );
        assert_eq!(read_event_ticks(&run_dir), vec![22_680, 30_000]);
        let again: u32 = lua
            .load("return record.research_triggers()")
            .eval()
            .expect("record.research_triggers() runs on an empty queue");
        assert_eq!(again, 0);
    }

    /// Drives the real mod->core->Lua road for a death, as
    /// `teleport_writeout_reaches_events_jsonl_through_the_real_parser`
    /// does for a teleport: the mod's line through `OutputParser` into a
    /// world the recording sandbox shares, then `record.deaths()`.
    #[test]
    fn a_death_and_respawn_reach_events_jsonl_through_the_real_parser() {
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
                1234,
                "player_died",
                r#"{"cause":"medium-worm-turret","cause_type":"turret","player_id":2,"position":{"x":12.5,"y":-3},"respawn_in":587}"#,
            )
            .expect("death parses");
        parser
            .parse(
                1900,
                "player_respawned",
                r#"{"player_id":2,"position":{"x":0,"y":0}}"#,
            )
            .expect("respawn parses");

        let written: u32 = lua
            .load("return record.deaths()")
            .eval()
            .expect("record.deaths() runs");
        assert_eq!(written, 2);
        let events = read_events(&run_dir);
        assert_eq!(events.len(), 2);
        match &events[0] {
            EventKind::BotDied {
                bot,
                position,
                cause,
                cause_type,
                respawn_in,
            } => {
                assert_eq!(*bot, 2);
                assert_eq!(
                    position.as_ref().map(|p| (p.x(), p.y())),
                    Some((12.5, -3.0))
                );
                assert_eq!(cause.as_deref(), Some("medium-worm-turret"));
                assert_eq!(cause_type.as_deref(), Some("turret"));
                assert_eq!(*respawn_in, Some(587));
            }
            other => panic!("expected bot_died, got {other:?}"),
        }
        match &events[1] {
            EventKind::BotRespawned { bot, position } => {
                assert_eq!(*bot, 2);
                assert_eq!(position.as_ref().map(|p| (p.x(), p.y())), Some((0.0, 0.0)));
            }
            other => panic!("expected bot_respawned, got {other:?}"),
        }
        assert_eq!(read_event_ticks(&run_dir), vec![1234, 1900]);
        let again: u32 = lua
            .load("return record.deaths()")
            .eval()
            .expect("record.deaths() runs on an empty queue");
        assert_eq!(again, 0);
    }

    // ---------------------------------------------------- roster_changed

    #[test]
    fn a_roster_change_is_written_with_its_lists_sorted_and_deduplicated() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"record.roster_changed({3, 1, 1}, {2}, {}, "bot(s) 2 had no character for 800 roster check(s)")"#,
        )
        .exec()
        .expect("record.roster_changed runs");
        let events = read_events(&run_dir);
        assert_eq!(
            events,
            vec![EventKind::RosterChanged {
                bots: vec![1, 3],
                left: vec![2],
                returned: vec![],
                reason: "bot(s) 2 had no character for 800 roster check(s)".to_string(),
            }]
        );
    }

    /// A "change" that names nobody in `left` or `returned` is refused, and
    /// so is a roster of nobody: both would put a line into the record that
    /// says something happened when the supervisor never does either.
    #[test]
    fn a_roster_change_that_changes_nothing_is_refused() {
        let (lua, _tmp, run_dir) = recording_lua();
        let err = lua
            .load(r#"record.roster_changed({1, 2}, {}, {}, "nothing")"#)
            .exec()
            .expect_err("no change is refused");
        assert!(err.to_string().contains("nothing changed"), "{err}");
        let err = lua
            .load(r#"record.roster_changed({}, {1}, {}, "everyone left")"#)
            .exec()
            .expect_err("an empty roster is refused");
        assert!(err.to_string().contains("bots is empty"), "{err}");
        let err = lua
            .load(r#"record.roster_changed({1}, nil, {}, "forgot")"#)
            .exec()
            .expect_err("a nil list is refused rather than read as empty");
        assert!(err.to_string().contains("must be a table"), "{err}");
        assert!(read_events(&run_dir).is_empty());
    }

    // --------------------------------------------------------------- walks

    /// **A walk reached the record for the first time here.**
    ///
    /// `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md` diagnosed
    /// three failed walks in run 30 that left no line in `events.jsonl` at
    /// all: the only trace in the run directory was one `last_error` string,
    /// and the other two existed solely in `workspace/server-log.txt`, which
    /// the next run overwrites. Walking is most of a run's wall clock, so
    /// this was the largest thing the record could not see.
    ///
    /// Asserted against the raw JSONL rather than the parsed enum on purpose:
    /// this is the test that went red before any of the Rust types existed.
    #[test]
    fn a_walk_reaches_the_event_log() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                return record.walks({
                    { bot = 2, step_index = 4, to = { x = -23.5, y = 18.5 },
                      status = "success", planned_start = 0, planned_end = 240,
                      dispatched_tick = 81000, replied_tick = 81400 },
                })
                "#,
            )
            .eval()
            .expect("record.walks runs");
        assert_eq!(written, 2, "one dispatch and one settle");
        let text = std::fs::read_to_string(run_dir.join("events.jsonl")).expect("events.jsonl");
        assert!(
            text.contains(r#""kind":"walk_settled""#),
            "the walk has to be in the log, got {text}"
        );
    }

    /// **A lost walk settles, and it settles as `lost`.**
    ///
    /// A walk the executor stopped waiting for never has a reply tick -- that
    /// is what "lost" means -- so a settle gated on one is a settle that can
    /// never be written for it. That gate is exactly the bug `record.actions`
    /// had (`run-1788347034-00981`: 179 dispatches, 170 settles, nine lost
    /// crafts with a dispatch and nothing after it), and this pins that the
    /// walk half was never built with it.
    ///
    /// `lost` is also not `failed`. The game refusing a walk and the game
    /// never answering are different facts: only the first is a verdict, and
    /// only the second leaves a bot that may still be walking.
    #[test]
    fn a_lost_walk_settles_even_though_the_game_never_replied() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                return record.walks({
                    { bot = 2, step_index = 3, to = { x = -22.5, y = 21.5 },
                      status = "lost", planned_start = 0, planned_end = 300,
                      dispatched_tick = 105028,
                      error = "the game reported no readable outcome: no action result received in time" },
                })
                "#,
            )
            .eval()
            .expect("record.walks runs");
        assert_eq!(
            written, 2,
            "one dispatch and one settle, not just a dispatch"
        );

        let events = read_events(&run_dir);
        let ticks = read_event_ticks(&run_dir);
        match &events[1] {
            EventKind::WalkSettled {
                bot,
                step_index,
                status,
                elapsed_ticks,
                failure,
                ..
            } => {
                assert_eq!((*bot, *step_index), (2, 3));
                assert_eq!(status, "lost", "the game said nothing; it did not say no");
                assert_ne!(status, "failed");
                assert_eq!(
                    *elapsed_ticks, None,
                    "no reply tick was measured, so no duration may be claimed"
                );
                assert_eq!(
                    failure.as_ref().map(|f| f.kind),
                    Some(WalkFailureKind::Timeout),
                    "classified as a timeout, which says nothing about the walk itself"
                );
            }
            other => panic!("expected walk_settled, got {other:?}"),
        }
        assert!(
            ticks[1] >= ticks[0],
            "a synthesized settle stamp is never earlier than its own dispatch, got {ticks:?}"
        );
    }

    /// **`no_path` and `pathfinder_busy` are not the same failure.**
    ///
    /// `FactorioRcon::player_path_attempt` branches on exactly this and so must
    /// the record: `try again later` means the request queue was full and
    /// nothing was searched, so repeating the walk is the right move; `failed
    /// to path find` means the pathfinder searched and came back empty, which
    /// is a fact about the destination that repeating will not change.
    /// Collapsing them into one "the walk failed" leaves the reader to guess
    /// which of two opposite responses applies.
    ///
    /// Most of these wordings are **historical**: they come from the build
    /// whose mod re-pathed for itself, and the archived runs are full of them.
    /// The two a current run can produce are the last two.
    #[test]
    fn a_queue_that_would_not_search_is_not_a_search_that_found_nothing() {
        let cases = [
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 the destination is unreachable: the game's pathfinder found no path from \
                 (6.90234375/30.09765625) to (-22.30078125/18.22265625)",
                WalkFailureKind::NoPath,
            ),
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 the game refused a re-path request",
                WalkFailureKind::PathfinderBusy,
            ),
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 the pathfinder did not answer a re-path within 600 ticks",
                WalkFailureKind::PathfinderBusy,
            ),
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 gave up after 4 re-paths on one walk",
                WalkFailureKind::RepathLimit,
            ),
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 aborted before reaching last waypoint",
                WalkFailureKind::Stalled,
            ),
            (
                "no path to [-22.5, 17.5] — the best one found ends [9.301] tiles away at \
                 [-13.5, 15.5], outside the [1.000] tile arrival tolerance",
                WalkFailureKind::NoPath,
            ),
            (
                "game rejected the command: Unexpected Response: ERROR: stuck while walking, \
                 leg 3 of 12 made no progress for 187 ticks from (6.90234375/30.09765625) to \
                 (-22.30078125/18.22265625)",
                WalkFailureKind::Stalled,
            ),
            // The wording nineteen of `run-1788432181-42528`'s twenty failed
            // walks arrived with, and the one this build produces most: a
            // pre-dispatch path request that searched and found nothing,
            // wrapped by `RconPathRequestFailed` (crates/core/src/errors.rs).
            (
                "game rejected the command: the game's pathfinder returned no path: \
                 Error: failed to path find",
                WalkFailureKind::NoPath,
            ),
            // Same wrapper, opposite fact. `player_path_attempt` retries a
            // full queue and only re-raises it when every retry was refused,
            // so this does reach a record -- and it must not be read as a
            // statement about the map.
            (
                "game rejected the command: the game's pathfinder returned no path: \
                 Error: try again later!",
                WalkFailureKind::PathfinderBusy,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(classify_walk_failure(error).kind, expected, "{error}");
        }
    }

    /// The positions a `no_path` failure names are **observed**, and they are
    /// not the destination the schedule asked for.
    ///
    /// Run 30's walk 72 asked to stand near `(-23.5, 18.5)` and died steering
    /// at `(-22.30078125, 18.22265625)` -- a point strictly inside the
    /// collision box of a stone furnace the same run had placed. Recording
    /// only the schedule's `to` would hide exactly the fact that had to be
    /// reconstructed by hand from a server log the next run overwrites.
    #[test]
    fn a_no_path_failure_carries_the_two_positions_the_game_named() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.walks({
                { bot = 2, step_index = 7, to = { x = -23.5, y = 18.5 },
                  status = "failed", planned_start = 0, planned_end = 240,
                  dispatched_tick = 81381, replied_tick = 81661,
                  error = "game rejected the command: Unexpected Response: ERROR: stuck while walking, the destination is unreachable: the game's pathfinder found no path from (6.90234375/30.09765625) to (-22.30078125/18.22265625)" },
            })
            "#,
        )
        .exec()
        .expect("record.walks runs");

        let events = read_events(&run_dir);
        match &events[1] {
            EventKind::WalkSettled {
                to,
                elapsed_ticks,
                failure,
                ..
            } => {
                let failure = failure.as_ref().expect("a failed walk is classified");
                assert_eq!(failure.kind, WalkFailureKind::NoPath);
                assert_eq!(
                    failure.from,
                    Some(Position::new(6.902_343_75, 30.097_656_25)),
                    "where the character actually stood when the mod gave up"
                );
                assert_eq!(
                    failure.destination,
                    Some(Position::new(-22.300_781_25, 18.222_656_25)),
                    "the waypoint the walk was really steering at, exact to the \
                     position unit -- a rounded copy lands in a different \
                     collision box than the one it failed against"
                );
                assert_ne!(
                    failure.destination.as_ref(),
                    Some(to),
                    "and it is NOT what the schedule asked for; that difference \
                     is the finding"
                );
                assert_eq!(*elapsed_ticks, Some(280));
            }
            other => panic!("expected walk_settled, got {other:?}"),
        }
    }

    /// The current mod's stall wording names the same two positions, and they
    /// have to survive the same way.
    ///
    /// This is the wording a *new* run produces: the mod's re-path is gone, so
    /// `found no path` above can only come out of the archive now, and a
    /// stalled leg is what a walk that has already exhausted
    /// `move_player_timed`'s retry budget looks like. Losing `from` and
    /// `destination` here would quietly retire the one place the run record
    /// states a real observed position of a bot.
    #[test]
    fn a_stalled_walk_carries_the_two_positions_the_mod_named() {
        let stalled = "game rejected the command: Unexpected Response: ERROR: stuck while \
                       walking, leg 3 of 12 made no progress for 187 ticks from \
                       (6.90234375/30.09765625) to (-22.30078125/18.22265625)";
        let failure = classify_walk_failure(stalled);
        assert_eq!(failure.kind, WalkFailureKind::Stalled);
        assert_eq!(
            failure.from,
            Some(Position::new(6.902_343_75, 30.097_656_25)),
            "where the character actually stood when the mod gave up"
        );
        assert_eq!(
            failure.destination,
            Some(Position::new(-22.300_781_25, 18.222_656_25)),
            "the waypoint the walk was really steering at"
        );
    }

    /// The wording the mod produces since its stall clock measures progress:
    /// the leg's length and origin follow `moved`, and a steering observation
    /// follows the tile. `from` is still the stall position and `to` the
    /// waypoint -- the two extra coordinates in the sentence must not be the
    /// ones read here, or the archive's "where the character stood" column
    /// would silently become "where the leg began".
    #[test]
    fn a_stalled_walk_with_the_leg_and_steering_clauses_still_reads_the_same_two_positions() {
        let stalled = "game rejected the command: Unexpected Response: ERROR: stuck while \
                       walking, leg 2 of 86 made no progress for 54 ticks from \
                       (-44.8125/74.71484375) to (-44.5/74.5), moved 1.04 tiles of a \
                       1.42-tile leg that began at (-45.55078125/75.453125), blocked at \
                       (-44.062/74.715) by nothing findable on tile 'dirt-3', steering \
                       east at 0.150 tiles/tick, walking_state read back walking=true";
        let failure = classify_walk_failure(stalled);
        assert_eq!(failure.kind, WalkFailureKind::Stalled);
        assert_eq!(failure.from, Some(Position::new(-44.8125, 74.71484375)));
        assert_eq!(failure.destination, Some(Position::new(-44.5, 74.5)));
    }

    /// A walk that arrived carries no failure, and a wording nothing
    /// recognises still carries one.
    ///
    /// `Other` rather than `None`: a settle this codebase does not spell
    /// `success` went wrong somehow, and leaving the field null would make an
    /// unclassifiable failure indistinguishable from an arrival in any query
    /// that groups by it.
    #[test]
    fn a_successful_walk_has_no_failure_and_an_unknown_wording_still_has_one() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.walks({
                { bot = 1, step_index = 0, to = { x = 1, y = 2 }, status = "success",
                  planned_start = 0, planned_end = 60,
                  dispatched_tick = 100, replied_tick = 160 },
                { bot = 1, step_index = 1, to = { x = 3, y = 4 }, status = "failed",
                  planned_start = 60, planned_end = 120,
                  dispatched_tick = 200, replied_tick = 260,
                  error = "something nobody has seen before" },
            })
            "#,
        )
        .exec()
        .expect("record.walks runs");

        let events = read_events(&run_dir);
        let failures: Vec<Option<WalkFailureKind>> = events
            .iter()
            .filter_map(|e| match e {
                EventKind::WalkSettled { failure, .. } => Some(failure.as_ref().map(|f| f.kind)),
                _ => None,
            })
            .collect();
        assert_eq!(
            failures,
            vec![None, Some(WalkFailureKind::Other)],
            "success carries none; an unrecognised failure carries `other`"
        );
    }

    /// A walk still in flight is not a verdict and writes no settle.
    ///
    /// The same rule `record.actions` applies to `pending`/`running` actions,
    /// and for the same reason: a walk the run never finished has no outcome
    /// to report, and inventing one is the failure mode that gating on the
    /// verdict rather than on a tick has to avoid.
    #[test]
    fn a_walk_still_running_writes_a_dispatch_and_no_settle() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                return record.walks({
                    { bot = 1, step_index = 0, to = { x = 1, y = 2 },
                      status = "running", planned_start = 0, planned_end = 60,
                      dispatched_tick = 100 },
                })
                "#,
            )
            .eval()
            .expect("record.walks runs");
        assert_eq!(written, 1);
        let events = read_events(&run_dir);
        assert!(matches!(&events[0], EventKind::WalkDispatched { .. }));
        assert_eq!(events.len(), 1, "no verdict, no settle: {events:?}");
    }

    /// A walk the game never acknowledged gets no dispatch line, and still
    /// settles.
    ///
    /// The mirror of `record.actions`' rule: inventing a dispatch tick for a
    /// walk the game never saw would be a fabrication, but dropping the
    /// verdict as well is how a failure comes to exist nowhere at all. The
    /// settle then names its own destination, which is the reason `to` is
    /// repeated on it -- without it this line would name a bot and an index
    /// and nothing else.
    #[test]
    fn a_walk_the_game_never_stamped_a_tick_for_still_settles() {
        let (lua, _tmp, run_dir) = recording_lua();
        let written: u32 = lua
            .load(
                r#"
                return record.walks({
                    { bot = 4, step_index = 2, to = { x = -19.5, y = 19.5 },
                      status = "failed", planned_start = 0, planned_end = 60,
                      error = "game rejected the command: rcon connection lost" },
                })
                "#,
            )
            .eval()
            .expect("record.walks runs");
        assert_eq!(written, 1, "a settle, and deliberately no dispatch");
        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::WalkSettled { to, status, .. } => {
                assert_eq!(status, "failed");
                assert_eq!(
                    *to,
                    Position::new(-19.5, 19.5),
                    "the settle names where the walk was going even with no \
                     dispatch line beside it"
                );
            }
            other => panic!("expected walk_settled, got {other:?}"),
        }
    }

    /// The dispatch carries the scheduler's prediction, as a **duration**.
    ///
    /// `planned_start` and `planned_end` are plan-relative ticks; the record
    /// keeps the start and turns the pair into a duration, exactly as
    /// `PlannedStep` does for an action. A walk's prediction is in the record
    /// nowhere else -- `plan_created` carries only steps that have an action
    /// id -- so without this a measured walk has nothing to be compared
    /// against, which is the whole point of measuring it.
    #[test]
    fn the_dispatch_carries_the_planned_span_as_a_duration() {
        let (lua, _tmp, run_dir) = recording_lua();
        lua.load(
            r#"
            record.walks({
                { bot = 1, step_index = 0, to = { x = 1, y = 2 }, status = "success",
                  planned_start = 300, planned_end = 540,
                  dispatched_tick = 1000, replied_tick = 1900 },
            })
            "#,
        )
        .exec()
        .expect("record.walks runs");
        let events = read_events(&run_dir);
        match &events[0] {
            EventKind::WalkDispatched {
                planned_start,
                planned_duration,
                ..
            } => {
                assert_eq!(*planned_start, 300);
                assert_eq!(*planned_duration, 240, "540 - 300, not 540");
            }
            other => panic!("expected walk_dispatched, got {other:?}"),
        }
        let ticks = read_event_ticks(&run_dir);
        assert_eq!(
            ticks,
            vec![1000, 1900],
            "the events themselves sit on the game's clock, not the plan's"
        );
    }

    /// **The roster a plan was made for is the caller's fact, not the
    /// binding's.**
    ///
    /// `plan_created.bots` used to be derived from the plan's own steps, which
    /// deleted exactly the bots a reader is asking about; it was then changed
    /// to the roster `create_lua_record` closes over, and that is wrong in a
    /// different direction. A script that plans with `goal.plan{bots = ...}`
    /// -- which `research_run.lua` does, from `rcon.players()` -- expands
    /// against *that* roster, and the binding's ambient one is neither it nor
    /// the step bots. Run 30 recorded `bots: [1, 2]` for a plan made for `[2]`
    /// alone, because bot 1 was invisible to `rcon.players()` during
    /// freeplay's crash-site cutscene: the record claimed a bot had been
    /// offered work it was never offered.
    ///
    /// That matters more here than in most fields. `plan_created` is written
    /// at planning time and carries the whole DAG, which makes it the part of
    /// an archived run that stays trustworthy even where outcomes do not -- a
    /// lie inside the reliable record is worse than one inside a suspect one.
    #[test]
    fn plan_created_reports_the_roster_the_plan_was_expanded_against() {
        // The *process* roster is four bots; the plan was made for one.
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2, 3, 4]);
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 2, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
            }, { 2 })
            "#,
        )
        .exec()
        .expect("plan_created runs");

        match &read_events(&run_dir)[0] {
            EventKind::PlanCreated { bots, .. } => assert_eq!(
                *bots,
                Some(vec![2]),
                "the roster `goal.plan` was given, not the four the process was \
                 started with"
            ),
            other => panic!("expected plan_created, got {other:?}"),
        }
    }

    /// A caller that does not say which roster gets a null, not a guess.
    ///
    /// Present-and-null, the same shape `run_started.seed` uses: a key that is
    /// always there says "we looked", where a plausible-looking substitute
    /// says something nobody established. The ambient roster is exactly such a
    /// substitute, which is why it is no longer the fallback.
    #[test]
    fn plan_created_with_no_roster_records_null_rather_than_the_ambient_one() {
        let (lua, _tmp, run_dir) = recording_lua_for(vec![1, 2, 3, 4]);
        lua.load(
            r#"
            record.plan_created(1, {
                { id = 1, bot = 2, action = "mine 10 iron-ore", deps = {},
                  planned_start = 0, planned_duration = 300 },
            })
            "#,
        )
        .exec()
        .expect("plan_created runs");

        match &read_events(&run_dir)[0] {
            EventKind::PlanCreated { bots, .. } => assert_eq!(
                *bots, None,
                "nobody said which roster, so the record must not name one"
            ),
            other => panic!("expected plan_created, got {other:?}"),
        }
        let text = std::fs::read_to_string(run_dir.join("events.jsonl")).expect("events.jsonl");
        assert!(
            text.contains(r#""bots":null"#),
            "present-and-null, never absent: {text}"
        );
    }

    /// A roster that is not a list of bot ids is refused, rather than written
    /// as an empty one.
    ///
    /// The same rule `record.milestone_satisfied` applies to an unrecognised
    /// reason: a value this binding cannot read is a construction error in the
    /// caller, and quietly recording `[]` for it would put "this plan was made
    /// for no bots" into the one record that is supposed to be trustworthy.
    #[test]
    fn plan_created_refuses_a_roster_it_cannot_read() {
        for roster in ["\"1,2\"", "{}", "{ 0 }", "{ \"two\" }"] {
            let (lua, _tmp, _run_dir) = recording_lua_for(vec![1, 2]);
            let err = lua
                .load(format!(
                    r#"record.plan_created(1, {{ {{ id = 1, bot = 2, action = "a", deps = {{}},
                        planned_start = 0, planned_duration = 1 }} }}, {roster})"#
                ))
                .exec()
                .expect_err(roster);
            assert!(
                err.to_string().contains("bots"),
                "the refusal names the argument: {err}"
            );
        }
    }
}
