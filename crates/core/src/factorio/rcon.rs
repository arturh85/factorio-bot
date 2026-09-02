use crate::errors::{
    RconError, RconNoWaterFound, RconOutOfResourceReach, RconPathRequestFailed,
    RconPlayerBlockesAllPlacement, RconPlayerBlockesPlacement, RconPlayerNotFound,
    RconRadiusLimitReached, RconReplyNotJson, RconTimeout, RconUnexpectedEmptyResponse,
    RconUnexpectedOutput, RconWalkFallsShort,
};
use crate::factorio::snapshot::WorldSnapshot;
use crate::factorio::ticks::{ActionTicks, take_tick_stamp};
use crate::factorio::util::{
    add_to_rect, blueprint_build_area, build_entity_path, calculate_distance, hashmap_to_lua,
    map_blocked_tiles, move_pos, move_position, position_to_lua, rect_to_lua, span_rect,
    str_to_lua, value_to_lua, vec_to_lua, vector_add, vector_multiply, vector_normalize,
    vector_substract,
};
use crate::factorio::world::{FactorioWorld, PlacementRefusal, RefusalSource};
use crate::settings::FactorioSettings;
use crate::types::{
    ActionId, AreaFilter, Direction, FactorioEntity, FactorioForce, FactorioPlayer, FactorioTile,
    InventoryResponse, PlayerId, Pos, Position, Rect, RequestEntity,
};
use miette::{Context, IntoDiagnostic, Report, Result, miette};
use parking_lot::RwLock;
use rcon::Connection;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tracing::{info, warn};

const RCON_INTERFACE: &str = "botbridge";

/// How long a dispatched action may go without a verdict before the wait gives
/// up. Unchanged from the literal it replaces; named so the timeout branch is
/// reachable from a test.
const ACTION_RESULT_DEADLINE: Duration = Duration::from_secs(360);

/// The `/silent-command remote.call(...)` text for a BotBridge function.
/// Renders `value` as a Lua string literal safe to paste into a `remote.call`
/// command line.
///
/// [`str_to_lua`] only wraps in quotes, which is fine for the identifiers and
/// prototype names it is used for but not for a value chosen by someone else.
/// A run id is opaque by contract -- this side does not get to say what may be
/// in it -- so a `'`, a backslash or a newline has to survive as data rather
/// than end the literal and let the rest be read as Lua. The `\ddd` escapes
/// cover the two characters that would terminate the command line itself:
/// RCON commands are newline-delimited, so an embedded newline would otherwise
/// split one call into two.
fn lua_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\010"),
            '\r' => out.push_str("\\013"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

/// Which screenshot cameras a capture run registers.
///
/// **[`FrameCameras::None`] is the default, and it is a real answer rather
/// than a degraded one.** Video is the visual record now; the capture session
/// still runs, because the mod's world-state samplers ride on it, but it
/// renders nothing. See [`FactorioRcon::frame_capture_start`] for the numbers.
///
/// There is deliberately no `Default` that resolves to anything else, and no
/// `Option<FrameCameras>` anywhere: a call site that forgets to say what it
/// wants must produce the cheap outcome, not the 947 MB one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FrameCameras {
    /// No camera at all. The session runs; nothing is rendered.
    #[default]
    None,
    /// Every camera the mod can offer: `follow`, `bot-N` per player, `area`.
    /// A bot joining mid-run gets its own camera under this, and only this.
    All,
    /// Exactly these camera ids. The mod **raises** on one it cannot supply,
    /// which `frame_capture_verdict` surfaces as an error -- a list quietly
    /// reduced to nothing would look configured and capture as much as
    /// [`FrameCameras::None`].
    Only(Vec<String>),
}

impl FrameCameras {
    /// The Lua argument the mod reads, or `None` to pass no argument at all --
    /// which the mod already treats as "no cameras", so the default costs
    /// nothing on the wire.
    fn to_lua_literal(&self) -> Option<String> {
        match self {
            FrameCameras::None => None,
            FrameCameras::All => Some("true".to_string()),
            FrameCameras::Only(ids) => Some(format!(
                "{{{}}}",
                ids.iter()
                    .map(|id| lua_string_literal(id))
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        }
    }
}

/// The positional argument list for `frame_capture_start`.
///
/// `nil` is spelled out rather than the first argument being omitted when
/// there is no run id but there are cameras: Lua is positional, and a
/// shortened list would hand the camera selection to `run_id`, where the mod's
/// type check would refuse it. Emitted only when it is actually needed, so the
/// default call is unchanged on the wire from what it always sent.
fn frame_capture_args(run_id: Option<&str>, cameras: &FrameCameras) -> Vec<String> {
    match (run_id, cameras.to_lua_literal()) {
        (Some(run_id), None) => vec![lua_string_literal(run_id)],
        (Some(run_id), Some(cameras)) => vec![lua_string_literal(run_id), cameras],
        (None, None) => vec![],
        (None, Some(cameras)) => vec!["nil".to_string(), cameras],
    }
}

/// Judges the reply to either frame-capture toggle.
///
/// Unlike most `remote_call_timed` callers here, this refuses a reply that
/// still has text in it after the tick stamp is taken off. The mod raises on a
/// camera id it cannot use, on a run id that is not a string, and on wiping a
/// directory it cannot wipe, and the game reports that as "Cannot execute
/// command. Error: ..." in the reply body rather than as a transport failure.
/// Dropping those lines would turn a capture that never started into a silent
/// success, and the first evidence would be an empty frame directory much
/// later.
///
/// A free function taking the reply, rather than a method taking the call's
/// name: the name has to stay a literal in each toggle's own body, because
/// that is where `each_rcon_doc_block_names_the_remote_call_its_binding_reaches`
/// reads it from. Passing the name down to a shared sender hid it from that
/// check, which is exactly the check that catches a doc block promising a
/// `remote.call` the binding does not make.
fn frame_capture_verdict(reply: (Option<Vec<String>>, Option<u64>)) -> Result<Option<u64>> {
    let (lines, tick) = reply;
    if let Some(lines) = lines {
        return Err(RconUnexpectedOutput {
            output: lines.join("\n"),
        }
        .into());
    }
    Ok(tick)
}

/// Judges a reply that should carry nothing.
///
/// Factorio writes `Cannot execute command. Error: ...` into the reply body
/// when the mod raises, and the mod writes its own refusals there with
/// `rcon.print`. A caller that drops the body therefore reports success for
/// every call it ever makes -- which is how `add_research` came to accept a
/// technology that does not exist, and how a run spent 61345 ticks re-issuing
/// an action that did nothing while everything claimed to work.
///
/// This is [`frame_capture_verdict`] without the tick: the same rule, for the
/// calls that have no payload to return.
fn expect_silence(lines: Option<Vec<String>>) -> Result<()> {
    if let Some(lines) = lines {
        return Err(RconUnexpectedOutput {
            output: lines.join("\n"),
        }
        .into());
    }
    Ok(())
}

fn remote_call_command(function_name: &str, args: &[String]) -> String {
    let mut arg_string: String = args.join(", ");
    if !arg_string.is_empty() {
        arg_string = String::from(", ") + &arg_string;
    }
    format!("/silent-command remote.call('{RCON_INTERFACE}', '{function_name}'{arg_string})")
}

/// Splits an RCON reply body into lines, dropping the trailing newline the
/// server always appends. `None` for an empty reply.
fn split_reply(result: &str, silent: bool) -> Option<Vec<String>> {
    if result.is_empty() {
        return None;
    }
    let body = &result[0..result.len() - 1];
    if !silent {
        info!("rcon ⮞ {}", body);
    }
    Some(body.split('\n').map(|str| str.to_owned()).collect())
}

/// How much of an unparseable reply to quote back in the error.
///
/// Long enough for `Cannot execute command. Error: <lua traceback head>` and
/// for a mod complaint to be recognisable; short enough that a truncated 744 kB
/// `world_snapshot` does not arrive in a log line.
pub const REPLY_SNIPPET_LIMIT: usize = 200;

/// The head of `text`, quoted, elided when it was longer, and with the
/// whitespace that would break a one-line message escaped.
///
/// Truncation is on a `char` boundary rather than a byte one: a reply is a
/// game's own text and may well be multi-byte, and slicing it mid-codepoint
/// would panic on the very path whose job is to report a fault.
fn reply_snippet(text: &str) -> String {
    let head: String = text.chars().take(REPLY_SNIPPET_LIMIT).collect();
    let elided = head.chars().count() < text.chars().count();
    let escaped = head.replace('\\', "\\\\").replace('\n', "\\n");
    if elided {
        format!("\"{escaped}\"...")
    } else {
        format!("\"{escaped}\"")
    }
}

/// Deserialises one RCON reply, and says **what arrived** when it cannot.
///
/// Every `serde_json::from_str` on a reply goes through here. The bare
/// `.into_diagnostic()` it replaces produced `expected value at line 1
/// column 1` — serde's message for input that is not JSON at all — from six
/// different call sites, which is a message that identifies neither the call
/// nor the payload. See [`RconReplyNotJson`].
fn parse_reply<T: serde::de::DeserializeOwned>(call: &str, text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|err| {
        RconReplyNotJson {
            call: call.to_string(),
            byte_count: text.len(),
            snippet: reply_snippet(text),
            parser: err.to_string(),
        }
        .into()
    })
}

/// Judges the reply to an `insert_to_inventory` / `remove_from_inventory` RPC.
///
/// # This is where a transfer's `Success` gets its strength
///
/// The mod's two transfer handlers do not report a result; they report
/// **complaints**, and only when something was wrong. Every arithmetic
/// post-condition they check — entity missing, inventory missing, the player
/// holding fewer items than asked (`clamping...`), the inventory taking fewer
/// than offered (`tried to insert N but inserted M`), the player failing to
/// give up or receive what moved (`wtf, ...`) — routes through
/// `complain`, and `complain` is `rcon.print` (`mods/BotBridge/control.lua`),
/// i.e. it writes **into this very reply body**. A handler that moved exactly
/// what was asked prints nothing but its `§tick§` stamp.
///
/// So "the reply is empty once the stamp is off" is not merely "the game did
/// not throw": it is the mod asserting that the requested count is the count
/// that moved. Anything left over is a verdict of failure — including a
/// zero-move, which is the case worth naming, because a zero-move that came
/// back green would be indistinguishable from a completed transfer to anything
/// downstream. See `transfer_success_means_items_moved` below, which drives the
/// real mod source to prove it.
///
/// The two mechanisms this rests on are the mod's complaint path and this
/// function; breaking either silently downgrades every transfer's `Success` to
/// the weak reading.
fn judge_transfer_reply(
    lines: Option<Vec<String>>,
    tick: Option<u64>,
) -> Result<ActionTicks, ActionFailure> {
    if let Some(lines) = lines {
        // The game answered, so it saw the command and judged it: a verdict
        // at a real tick, not a command that never landed.
        return Err(ActionFailure::refused(
            RconError {
                message: format!("{lines:?}"),
            }
            .into(),
            ActionTicks::at(tick),
        ));
    }
    Ok(ActionTicks::at(tick))
}

/// The radius Factorio's `LuaSurface.request_path` uses when none is given.
///
/// Documented as "how close we need to get to the goal. Default 1." The mod
/// forwards a `nil` radius unchanged, so a caller passing `None` is asking for
/// this, not for an exact landing.
/// The mod's wording for a placement the game refused without naming a cause.
///
/// `mods/BotBridge/control.lua` prints
/// `cannot place item '<item>' because surface.can_place_entity said 'no'`
/// when `can_place_entity` says no and the acting player is *not* in the
/// footprint; the other branch prints the `player_blocks_placement` sentinel,
/// which is handled separately and deliberately not remembered (see
/// [`PlacementRefusal`]).
///
/// Matched here, at the one place the game's own line is still a line, rather
/// than downstream against an error string two more layers of wrapping deep.
/// A wording change therefore costs a refusal that is not learned from -- the
/// behaviour before this existed -- and never a site excluded for a reason
/// that was not given.
const CAN_PLACE_REFUSAL: &str = "can_place_entity said 'no'";

/// Remembers `line` as a refused site, if it is one.
///
/// Called on both arms that turn an unrecognised reply into an error: the
/// first attempt's, and the one after the actor has been walked aside. The
/// second matters as much as the first -- a refusal that survives the walk is
/// the strongest evidence there is that the blocker is not the actor.
fn note_placement_refusal(
    world: &Arc<FactorioWorld>,
    tick: Option<u64>,
    line: &str,
    item_name: &str,
    entity_position: &Position,
) {
    if !line.contains(CAN_PLACE_REFUSAL) {
        return;
    }
    // `at_dispatch`, not a literal: the mod's line names no cause and there is
    // nothing left to ask by the time it arrives here, so this path has no
    // blockers and no tile to report. Only the pre-flight check
    // (`FactorioRcon::can_place_entities`) can fill those in.
    let refusal = PlacementRefusal::at_dispatch(tick, item_name, entity_position.clone());
    if world.record_placement_refusal(refusal) {
        warn!(
            "the game refused to build {} at {}; the planner will avoid that footprint \
             for the rest of this run",
            item_name, entity_position
        );
    }
}

/// Joins a `can_place_entities` reply to the queries that produced it and
/// writes the durable refusals into the world's ledger.
///
/// Split out of [`FactorioRcon::can_place_entities`] so the join and the
/// filtering can be driven without a game: everything above this line is
/// transport, everything in it is the judgement.
///
/// The join is **by index and only by index**, which is why a reply of the
/// wrong length is rejected outright rather than zipped to the shorter of the
/// two. A verdict attached to the wrong query would exclude ground the game
/// never refused — a silent, permanent error in the one direction that
/// matters, since the ledger is never expired.
fn accept_verdicts(
    world: &Arc<FactorioWorld>,
    queries: &[PlacementQuery],
    reply: PlacementVerdicts,
) -> Result<Vec<PlacementVerdict>> {
    if reply.sites.len() != queries.len() {
        return Err(miette!(
            "asked the game about {} placements and got {} verdicts back; the reply cannot be \
             joined to the queries by index, so none of it is used",
            queries.len(),
            reply.sites.len()
        ));
    }
    for (query, verdict) in queries.iter().zip(reply.sites.iter()) {
        if !verdict.is_durable_refusal() {
            continue;
        }
        let refusal = PlacementRefusal {
            tick: reply.tick,
            entity: query.item_name.clone(),
            position: query.position.clone(),
            source: RefusalSource::PreCheck,
            blockers: verdict.blockers.clone(),
            tile: verdict.tile.clone(),
        };
        if world.record_placement_refusal(refusal) {
            warn!(
                "the game would refuse to build {} at {} ({}); replanning around that footprint \
                 rather than dispatching it",
                query.item_name,
                query.position,
                describe_blockers(&verdict.blockers, verdict.tile.as_deref()),
            );
        }
    }
    Ok(reply.sites)
}

/// A human-readable cause for a pre-check refusal, for the one warning line
/// this produces.
///
/// Deliberately distinguishes "no entity was in the box" from "we did not
/// look": an empty blocker list on a pre-check refusal is a real observation
/// — nothing intersected the footprint — and it points at the ground itself,
/// which is why the tile is named alongside it.
fn describe_blockers(blockers: &[String], tile: Option<&str>) -> String {
    let what = if blockers.is_empty() {
        "no entity in the footprint".to_string()
    } else {
        format!("blocked by {}", blockers.join(", "))
    };
    match tile {
        Some(tile) => format!("{what}; tile {tile}"),
        None => what,
    }
}

/// One candidate placement to ask the game about before a plan commits to it.
///
/// The three fields are exactly what
/// [`FactorioRcon::place_entity_timed`] would be called with, and that is the
/// point: a pre-check that asked a different question would be worse than no
/// pre-check, because its green would be believed.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementQuery {
    /// The player whose force and surface the check runs against. A `BotId`
    /// *is* a player id, so this is the bot the schedule assigned the step to.
    pub player_id: PlayerId,
    /// The item the bot would be holding.
    pub item_name: String,
    /// The centre the build would be aimed at.
    pub position: Position,
    /// `defines.direction`, as the plan chose it.
    pub direction: u8,
}

/// The game's answer for one [`PlacementQuery`].
///
/// `ok` is the whole verdict; everything else is evidence about a `false`,
/// and is what a refusal observed at dispatch can never carry.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct PlacementVerdict {
    /// Whether `surface.can_place_entity` said yes.
    #[serde(default)]
    pub ok: bool,
    /// Whether a **character** — any character, not just the acting bot — was
    /// inside the tested collision box.
    ///
    /// This is the discriminator that keeps the ledger honest. A character is
    /// the one blocker that moves on its own, `PlanState::from_world` already
    /// re-reads every character from the world on every plan, and at
    /// pre-check time the acting bot has not walked to the site yet — so a
    /// character in the footprint now says nothing about the ground and must
    /// not be remembered as if it did. It is the same distinction the mod
    /// makes at dispatch with `§player_blocks_placement§`, widened from the
    /// acting player to every character because at plan time there is no
    /// acting player standing anywhere yet.
    #[serde(default)]
    pub character: bool,
    /// The distinct names of the entities intersecting the tested box, sorted.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// The tile under the queried centre, when the game reported one.
    #[serde(default)]
    pub tile: Option<String>,
    /// Set when the mod could not run the check at all — an unknown player, or
    /// an item with no `place_result`. Such a site is **not** recorded as a
    /// refusal: nothing was learned about the ground.
    #[serde(default)]
    pub error: Option<String>,
}

impl PlacementVerdict {
    /// Whether this verdict is a fact about the ground worth keeping.
    ///
    /// Three ways to be `false`, and they are different: the game said yes;
    /// the game said no because a character was standing there (transient);
    /// or the mod could not ask at all (nothing observed).
    pub fn is_durable_refusal(&self) -> bool {
        !self.ok && !self.character && self.error.is_none()
    }
}

/// The whole `can_place_entities` reply: one tick, one verdict per query, in
/// the order asked.
#[derive(Debug, Clone, Default, Deserialize)]
struct PlacementVerdicts {
    #[serde(default)]
    tick: Option<u64>,
    #[serde(default)]
    sites: Vec<PlacementVerdict>,
}

const DEFAULT_PATH_RADIUS: f64 = 1.0;

/// How much further than the requested radius a returned path may end from the
/// goal and still count as reaching it.
///
/// Factorio's pathfinder puts every waypoint on a tile centre, so a goal that
/// is not itself a tile centre is up to `sqrt(2)/2 = 0.707` tiles from the
/// nearest waypoint that could serve as the path's end. Rounded up to a whole
/// tile.
///
/// The number is checked against the three legitimate paths of the 2026-08-30
/// live run (`.superpowers/sdd/2026-08-30-goal-values/live-smelt-run.md`),
/// whose endpoints landed 0.707, 1.000 and 2.828 tiles from their goals — the
/// last with an explicit radius of 3 — while that run's silently substituted
/// goal landed **9.30** tiles out. The tolerance separates 1.000 from 9.30
/// with a factor of four to spare in both directions.
const PATH_ENDPOINT_SLACK: f64 = 1.0;

/// Where a walk along `waypoints` will actually leave a player who currently
/// stands at `current`.
///
/// An empty path is not a failure to arrive: the pathfinder returns nothing
/// when there is nowhere to go, and the mod completes such a walk on the next
/// tick without moving. The honest end position in that case is where the
/// player already is — which the caller may not know, hence the `Option`.
fn walk_end_position<'a>(
    waypoints: &'a [Position],
    current: Option<&'a Position>,
) -> Option<&'a Position> {
    waypoints.last().or(current)
}

/// Whether a walk that ends at `end` counts as having arrived at `goal`.
///
/// This is the question [`FactorioRcon::player_path`] deliberately does not
/// answer. That method is best effort: when the goal is unreachable — the
/// classic case being a tile the bot has just built on — it retries against a
/// synthesised goal offset away from the real one by the radius, and returns
/// the path to *that*. The path is real and the walk along it succeeds, so
/// every downstream signal says success while the bot stands somewhere it was
/// never asked to be.
///
/// The tolerance is the radius the caller asked for plus
/// [`PATH_ENDPOINT_SLACK`]; `None` means the caller asked for Factorio's own
/// default of [`DEFAULT_PATH_RADIUS`], not for an exact landing.
fn walk_arrives(goal: &Position, radius: Option<f64>, end: &Position) -> bool {
    calculate_distance(end, goal) <= arrival_tolerance(radius)
}

/// The tolerance [`walk_arrives`] applies, exposed so a failure can report it.
fn arrival_tolerance(radius: Option<f64>) -> f64 {
    radius.unwrap_or(DEFAULT_PATH_RADIUS) + PATH_ENDPOINT_SLACK
}

/// The prototype whose collision box a walk's destination must have room for.
///
/// Read from the world rather than written down as `0.19921875`, because the
/// number is a property of the running game's prototypes and this crate has no
/// business asserting it. When the world does not have it — a world built
/// before `update_entity_prototypes` ran, which every early tick is — the
/// footprint degrades to a point, see [`character_footprint`].
const CHARACTER_PROTOTYPE: &str = "character";

/// How far around a destination to look for an entity that can be *named* in a
/// refusal.
///
/// Only ever used for wording. Whether the destination is blocked is decided by
/// [`EntityGraph::blocking_boxes_within`](crate::graph::entity_graph::EntityGraph::blocking_boxes_within);
/// this radius merely has to be wide enough to reach the centre of a large
/// entity whose box covers the destination, and a miss costs a less specific
/// message and nothing else.
const BLOCKER_NAMING_RADIUS: f64 = 3.0;

/// How far outside the footprint to ask the quad tree for boxes.
///
/// The tree's query is a narrowing pass that already admits boxes which merely
/// come close, so this is belt and braces against a degenerate (zero-area)
/// footprint querying badly; every box it returns is re-tested exactly against
/// the footprint afterwards, so a wider probe cannot widen the verdict.
const BLOCKER_PROBE_MARGIN: f64 = 1.0;

/// What is known about a character standing at a position — **not** whether the
/// ground is clear.
///
/// The asymmetry is the whole point, and it is the same one
/// [`PlacementVerdict::is_durable_refusal`] makes about the game's build
/// refusals: an observation that something *is* there is a fact, while the
/// absence of an observation is not.
/// [`EntityGraph`](crate::graph::entity_graph::EntityGraph) is an in-bounds oracle —
/// it can only answer about entities and tiles it has been told about — so
/// "no box overlaps" covers both "the ground is clear" and "the graph has
/// never seen this ground", and those two are indistinguishable from here.
///
/// Hence two variants and not three: only [`StandingVerdict::Blocked`] may
/// refuse a walk. Everything else is allowed through, including every case
/// where the answer is really "I cannot tell". This guard sits on the path of
/// every walk, so a false positive is worse than the stall it prevents.
#[derive(Debug, Clone, PartialEq)]
enum StandingVerdict {
    /// A collision box the graph has actually seen overlaps the character's
    /// footprint at that position. The string names the obstruction for the
    /// failure message; see [`describe_blocker`].
    Blocked { blocker: String },
    /// Nothing the graph has seen overlaps the footprint. Read as "cannot be
    /// proved blocked", never as "clear".
    NotProvablyBlocked,
}

/// Whether two rectangles overlap in the sense the game collides them: sharing
/// only an edge is not an overlap.
///
/// Strict on all four comparisons, which matters twice. A character whose box
/// abuts a furnace's exactly — `0.69921875 + 0.19921875` from its centre — is
/// standing legally and must not be refused; and a degenerate footprint (zero
/// width and height, the unknown-prototype fallback) reduces this to
/// [`Rect::contains`], i.e. "the point is strictly inside the box", which is
/// the weakest claim that still catches a destination inside a building.
fn boxes_overlap(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x()
        && b.left_top.x() < a.right_bottom.x()
        && a.left_top.y() < b.right_bottom.y()
        && b.left_top.y() < a.right_bottom.y()
}

/// The area a character standing at `at` occupies, as the world's own
/// prototypes describe it.
///
/// With no `character` prototype the footprint collapses to the single point
/// `at`. That is deliberately a *weaker* question — "is this exact point inside
/// a building" instead of "does a character fit here" — because guessing an
/// extent we were never told would refuse walks on an invented number.
fn character_footprint(world: &FactorioWorld, at: &Position) -> Rect {
    match world.entity_prototypes.get(CHARACTER_PROTOTYPE) {
        Some(prototype) => add_to_rect(&prototype.collision_box, at),
        None => Rect::new(at, at),
    }
}

/// Names the obstruction for a refusal message, best effort.
///
/// `blocking_boxes_within` answers with rectangles and no names — its payload
/// is a bare `is_minable` flag — so the name has to come from the entity tree,
/// which holds only the types that tree tracks. A tree, a rock or a water tile
/// therefore blocks without being named, and the box is reported instead. The
/// query is deliberately unfiltered by name and type: narrowing it is what
/// reintroduced the forest-siting bug that `1b2b2149` was careful to leave
/// alone.
fn describe_blocker(world: &FactorioWorld, footprint: &Rect, blocker: &Rect) -> String {
    let named = world
        .entity_graph
        .find_entities_in_radius(footprint.center(), BLOCKER_NAMING_RADIUS, None, None)
        .into_iter()
        .find(|entity| boxes_overlap(&entity.bounding_box, footprint));
    match named {
        Some(entity) => format!("{} at {}", entity.name, entity.position),
        None => format!(
            "a collision box spanning {} to {}",
            blocker.left_top, blocker.right_bottom
        ),
    }
}

/// Whether the graph can prove a character cannot stand at `at`.
///
/// The one caller is [`judge_path`], for the *last* waypoint of a
/// returned path — which is the walk's non-negotiable destination inside the
/// mod, the position its follower steers at until it arrives or the leg times
/// out. Run 30 (`docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`)
/// had three walks whose last waypoint was inside a stone furnace this same run
/// had built; each burned four re-paths and a leg timeout on a destination that
/// was unsatisfiable by arithmetic, and reported it as terrain.
fn standing_verdict(world: &FactorioWorld, at: &Position) -> StandingVerdict {
    let footprint = character_footprint(world, at);
    let probe = Rect::new(
        &Position::new(
            footprint.left_top.x() - BLOCKER_PROBE_MARGIN,
            footprint.left_top.y() - BLOCKER_PROBE_MARGIN,
        ),
        &Position::new(
            footprint.right_bottom.x() + BLOCKER_PROBE_MARGIN,
            footprint.right_bottom.y() + BLOCKER_PROBE_MARGIN,
        ),
    );
    // The quad tree admits boxes that merely come close (its doc comment says
    // so), so every candidate is re-tested exactly before it may refuse.
    match world
        .entity_graph
        .blocking_boxes_within(&probe)
        .into_iter()
        .find(|blocker| boxes_overlap(blocker, &footprint))
    {
        Some(blocker) => StandingVerdict::Blocked {
            blocker: describe_blocker(world, &footprint, &blocker),
        },
        None => StandingVerdict::NotProvablyBlocked,
    }
}

/// A walk would have ended somewhere a character cannot stand.
///
/// Raised *before* any walk is dispatched, by [`judge_path`], when
/// the last waypoint of the path the pathfinder returned is inside a collision
/// box the entity graph has seen. That waypoint is the walk's destination in
/// the mod, so such a walk cannot finish: the follower steers at a point the
/// character cannot occupy, wedges against the box, and the re-path that
/// follows is unsatisfiable too — `WALK_REPATH_RADIUS` is 0.5 while standing
/// clear of a stone furnace needs 0.8984375.
///
/// Lives here rather than in [`crate::errors`] because it is meaningless away
/// from the one function that raises it, the same arrangement
/// [`crate::scripts::ScriptPathError`] uses.
// False positive from the thiserror/miette derives using struct fields in
// format strings, same as `crate::errors`.
#[allow(unused_assignments)]
#[derive(thiserror::Error, Debug, miette::Diagnostic)]
#[error(
    "the walk to [{goal_x}, {goal_y}] would end at [{end_x}, {end_y}], inside {blocker} — a character cannot stand there, so the walk could only stall"
)]
#[diagnostic(
    code(factorio::rcon::walk_ends_where_nobody_can_stand),
    help(
        "the pathfinder returned a route terminating inside a building; aim beside the target rather than at it — an entity's own position is inside its own collision box"
    )
)]
pub struct RconWalkEndsWhereNobodyCanStand {
    pub goal_x: f64,
    pub goal_y: f64,
    pub end_x: f64,
    pub end_y: f64,
    pub blocker: String,
}

/// Everything [`FactorioRcon::move_player_timed`] decides about a returned path
/// before it dispatches anything.
///
/// A free function, and not inlined into the caller, because both refusals are
/// pure judgements about data the game already handed over: given the
/// waypoints, the goal, the radius and a world, the answer is fixed. Driving
/// them here needs no RCON connection and no running game, which is the only
/// way run 30's actual coordinates can be a test.
///
/// Two questions, in this order:
///
/// 1. **Does the path reach the caller's goal?** [`FactorioRcon::player_path`]
///    is best effort and may have substituted the goal, so a path that lands
///    outside [`arrival_tolerance`] is [`RconWalkFallsShort`]. Unchanged.
/// 2. **Can a character stand where it ends?** Only asked of an actual
///    waypoint. An empty path is not a destination anybody chose — the mod
///    completes such a walk next tick without moving — so the fallback
///    position that question 1 judges is deliberately not fed to question 2:
///    refusing a walk because the bot is *already* standing somewhere the
///    graph calls blocked would be a refusal about the past.
fn judge_path(
    world: &FactorioWorld,
    goal: &Position,
    radius: Option<f64>,
    waypoints: &[Position],
    here: Option<&Position>,
) -> Result<(), ActionFailure> {
    if let Some(end) = walk_end_position(waypoints, here)
        && !walk_arrives(goal, radius, end)
    {
        return Err(ActionFailure::not_dispatched(
            RconWalkFallsShort {
                goal_x: goal.x(),
                goal_y: goal.y(),
                end_x: end.x(),
                end_y: end.y(),
                shortfall: calculate_distance(end, goal),
                tolerance: arrival_tolerance(radius),
            }
            .into(),
        ));
    }

    if let Some(end) = waypoints.last()
        && let StandingVerdict::Blocked { blocker } = standing_verdict(world, end)
    {
        return Err(ActionFailure::not_dispatched(
            RconWalkEndsWhereNobodyCanStand {
                goal_x: goal.x(),
                goal_y: goal.y(),
                end_x: end.x(),
                end_y: end.y(),
                blocker,
            }
            .into(),
        ));
    }

    Ok(())
}

/// One entry of a `LuaSurface.request_path` result, as the mod's
/// `on_script_path_request_finished` now serialises it
/// (`mods/BotBridge/control.lua`).
///
/// `PathfinderWaypoint.needs_destroy_to_reach` — "`true` if the path from the
/// previous waypoint to this one goes through an entity that must be
/// destroyed" — used to be read by the mod and dropped before it ever reached
/// Rust, which is the direct upstream cause of the stuck-teleport bug: the
/// bot walked into the obstruction the pathfinder had already flagged, made
/// no progress, and the game had already said why. `#[serde(flatten)]` keeps
/// the wire shape exactly `{"x":.., "y":.., "needs_destroy_to_reach":..}`, so
/// this is additive over the previous `{"x":.., "y":..}` shape; `default`
/// means a reply from before the mod sent the field — including the
/// `path_request_reply_wakes_the_waiter` regression test's literal JSON —
/// still parses, reading as "no obstruction reported" rather than failing.
#[derive(Debug, Clone, Deserialize)]
struct PathWaypoint {
    #[serde(flatten)]
    position: Position,
    #[serde(default)]
    needs_destroy_to_reach: bool,
}

/// Extracts the positions from a pathfinder result, logging once if any leg
/// requires destroying something to pass.
///
/// This is the one place `needs_destroy_to_reach` is read on the Rust side.
/// Nothing here refuses or reroutes around a blocked leg yet — the walk is
/// still dispatched — so this changes what the caller *knows*, not what it
/// *does*: previously the mod discarded the flag before it ever crossed the
/// wire, and a leg that the pathfinder had already flagged as blocked looked
/// identical to a clear one all the way down to the stuck-teleport it caused.
fn waypoint_positions(waypoints: Vec<PathWaypoint>, context: &str) -> Vec<Position> {
    let blocked = waypoints
        .iter()
        .filter(|w| w.needs_destroy_to_reach)
        .count();
    if blocked > 0 {
        warn!(
            "{}: {} of {} waypoints require destroying something to reach",
            context,
            blocked,
            waypoints.len()
        );
    }
    waypoints.into_iter().map(|w| w.position).collect()
}

/// Whether a player at `player` may mine a resource at `target`.
///
/// `reach` is the player's own `resource_reach_distance`, a `double` read from
/// the game — 2.7 for a plain character, `f64::MAX` for a player with no
/// character at all. It is never assumed to be 3.
///
/// The game enforces this bound silently: a `mining_state` aimed at a resource
/// outside it produces no event, no error and no progress, so a dispatch from
/// out of reach costs the whole action deadline and yields nothing.
fn within_resource_reach(player: &Position, target: &Position, reach: f64) -> bool {
    calculate_distance(player, target) <= reach
}

/// The path radius to request when a walk must end within `bound` of a goal.
///
/// Asking for `bound` itself is what the 2026-08-30 run did, and it left the
/// bot **3.345** tiles from the ore against a reach of 3 — outside by 0.345.
/// "Within R of the goal" is not "within R of the goal once you stop": the
/// path's last waypoint is on a tile centre and the mod's follower stops
/// within a 0.3-by-0.3 box of it, so a walk to radius R comes to rest at up to
/// roughly `R + 1.1`. Aiming at half the bound leaves room for that inside the
/// bound the caller actually has to satisfy.
///
/// Floored at half a tile so the goal never collapses onto the target's own
/// tile, which is the request shape that makes the pathfinder fail outright —
/// and which is exactly what a plan's `AtPosition` target is for a place, an
/// insert or a remove, since those name the entity's own position.
///
/// Public because the executor applies it to the radius a `StepKind::Walk`
/// carries, which is the same question this answers for a mine's corrective
/// walk. One rule, one place.
pub fn approach_radius(bound: f64) -> f64 {
    (bound * 0.5).clamp(0.5, bound.max(0.5))
}

/// The mod's wording for a mine whose target stopped existing part-way through.
///
/// Owned by `mods/BotBridge/control.lua`'s mining watchdog, which reports
/// `ERROR: the target <name> was gone before mining finished -- something else
/// mined it first` when `p[idx].mining` outlives the entity it names. The
/// entity does not have to have been stolen: mining a tile to zero destroys it,
/// and the game may destroy it before `on_player_mined_entity` closes the
/// action, so this is also how an ordinary exhaustion reaches us.
const MINE_TARGET_GONE: &str = "was gone before mining finished";

/// Whether a refused mine's verdict says the target is no longer there.
///
/// Matched on the mod's text because there is nothing else to match on: the
/// verdict reaches Rust as one opaque string in
/// [`crate::errors::RconError`]. Split out as a free function so the wording
/// this depends on is pinned by a test in this crate rather than only by a live
/// run -- if `control.lua` ever rewords it, that test fails here instead of the
/// retirement quietly stopping.
pub fn mine_reports_target_gone(message: &str) -> bool {
    message.contains(MINE_TARGET_GONE)
}

/// The mod's wording for a pathfinder that never searched.
///
/// Owned by `mods/BotBridge/control.lua`'s `on_script_path_request_finished`,
/// which answers `Error: try again later!` when `request_path` reports
/// `try_again_later` -- the request queue was full, so the question was never
/// put. Its sibling, `Error: failed to path find`, means the search happened
/// and there is no way there.
const PATHFINDER_BUSY: &str = "try again later";

/// Whether a refused path request says the queue was full rather than that
/// there is no path.
///
/// Matched on the mod's text because there is nothing else to match on, the
/// same way [`mine_reports_target_gone`] is, and split out as a free function
/// so a reword fails a test in this crate instead of quietly turning every
/// busy pathfinder into an unreachable goal.
pub fn path_request_was_busy(message: &str) -> bool {
    message.contains(PATHFINDER_BUSY)
}

/// [`path_request_was_busy`] against the error a path request actually fails
/// with.
///
/// The downcast is deliberate: a timeout, a dropped connection or a malformed
/// reply are not full queues, and retrying one of those as if it were would
/// hide it behind a delay.
fn is_busy_path_request(err: &Report) -> bool {
    err.downcast_ref::<RconPathRequestFailed>()
        .is_some_and(|failed| path_request_was_busy(&failed.reason))
}

/// Whether the pathfinder actually searched and reported that there is no way
/// there.
///
/// This, and only this, is what the offset-goal fallback in
/// [`FactorioRcon::player_path`] is an answer to. A request the game never
/// accepted, a queue that stayed full, a reply that never came and a reply that
/// would not parse are all "we do not know", and substituting a different goal
/// on the strength of not knowing is how a caller ends up walked somewhere it
/// never asked for.
fn path_search_found_nothing(err: &Report) -> bool {
    err.downcast_ref::<RconPathRequestFailed>()
        .is_some_and(|failed| !path_request_was_busy(&failed.reason))
}

/// How many times one path request is put to a pathfinder that keeps saying its
/// queue is full.
///
/// Three, because the cost of asking again is a few hundred milliseconds and
/// the cost of *not* asking again is a walk refused for a reason that was never
/// about the walk. The alternative this replaces was worse than a plain
/// failure: a full queue fell into the offset-goal search below, which answers
/// a question nobody asked and hands back a path to somewhere else.
const PATH_REQUEST_BUSY_ATTEMPTS: u32 = 3;

/// How long to wait before putting the same question to a full queue again.
const PATH_REQUEST_BUSY_BACKOFF: Duration = Duration::from_millis(200);

/// How far a dispatch got before it failed.
///
/// # The distinction a timeout cannot make on its own
///
/// "No reply arrived" is the same observation whether the game never saw the
/// command or saw it and then went quiet, and those two need opposite
/// renderings downstream: the second is an action whose outcome this run will
/// not learn ([`crate::factorio::ticks::ActionTicks`]'s consumer calls that
/// *lost*), the first is an action that simply did not happen. Classifying by
/// error type alone therefore cannot work, and guessing costs a bot being
/// reported as having lost track of work it never started.
///
/// So the phase is recorded where it is *known* -- at the call site, by which
/// statement was executing -- rather than inferred later from the error.
///
/// # What counts as evidence of a dispatch
///
/// Exactly one thing: **the game itself answered the dispatch RPC.** Every
/// `action_start_*` and every synchronous `remote_call_timed` below returns
/// only after BotBridge's `remote.call` ran inside the game and its reply came
/// back over RCON. That is positive evidence, not an inference from having
/// written bytes to a socket.
///
/// Anything weaker is [`Dispatch::NotDispatched`], including the case where the
/// RCON round trip itself fails after the command may already have been
/// written: we cannot tell a failed connect from a failed read, so we do not
/// claim to. That under-claims -- a command the game ran is reported as one it
/// never saw -- and that is the direction chosen deliberately, because the
/// other one invents an outstanding action out of nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// The game never acknowledged the command. Nothing is outstanding.
    ///
    /// Every failure before the dispatch RPC returned: a path request that
    /// timed out or found nothing, a player the world does not know, the walk a
    /// mine makes first, a dead RCON connection.
    NotDispatched,
    /// The game acknowledged the command and gave a verdict, and the verdict
    /// was no. Something is known, and it is a failure.
    Refused,
    /// The game acknowledged the command and no readable verdict ever arrived.
    ///
    /// The only state that says an action may still be out there. Produced by
    /// exactly one place -- [`FactorioRcon::sleep_for_action_result`] running
    /// out of patience *after* an `action_start_*` returned -- because that is
    /// the only place where both halves of the claim are established.
    NoVerdict,
}

/// A dispatch that failed, plus everything the game had already told us.
///
/// # Two facts that used to be destroyed in this file
///
/// **When.** These methods are shaped `let dispatched = action_start_...?; let
/// replied = sleep_for_action_result(...)?;`, so a `?` on the second threw away
/// the first's tick -- a number the game really produced. The consumer then saw
/// an absent tick that meant "we were handed a measurement and dropped it"
/// sitting beside one that meant "there was nothing to measure". `ticks` ends
/// that: it holds whatever was stamped, and [`ActionTicks::UNKNOWN`] is once
/// again a statement about the game rather than about the plumbing.
///
/// **Whether the game ever saw it.** See [`Dispatch`].
///
/// # Nothing here may be invented
///
/// The `From<Report>` conversion -- the one `?` uses -- is deliberately the one
/// that can supply neither: it yields [`Dispatch::NotDispatched`] and
/// [`ActionTicks::UNKNOWN`]. A pre-dispatch failure therefore stays absent and
/// unclaimed without anyone having to remember, and asserting either fact takes
/// an explicit constructor.
#[derive(Debug)]
pub struct ActionFailure {
    /// What went wrong, unchanged from what it always was.
    pub error: Report,
    /// What the game had stamped by the time it did. `ActionTicks::UNKNOWN`
    /// when the game said nothing -- never a zero, never a plan value.
    pub ticks: ActionTicks,
    /// How far the dispatch got. See [`Dispatch`].
    pub dispatch: Dispatch,
}

impl ActionFailure {
    /// The game never acknowledged the command. Carries no ticks, because
    /// nothing stamped one.
    pub fn not_dispatched(error: Report) -> Self {
        ActionFailure {
            error,
            ticks: ActionTicks::UNKNOWN,
            dispatch: Dispatch::NotDispatched,
        }
    }

    /// The game judged the command and said no, at these ticks.
    pub fn refused(error: Report, ticks: ActionTicks) -> Self {
        ActionFailure {
            error,
            ticks,
            dispatch: Dispatch::Refused,
        }
    }

    /// The game took the command and never gave a readable verdict. `ticks`
    /// carries the dispatch stamp, which is the evidence that there is an
    /// action out there to have lost.
    pub fn no_verdict(error: Report, ticks: ActionTicks) -> Self {
        ActionFailure {
            error,
            ticks,
            dispatch: Dispatch::NoVerdict,
        }
    }

    /// Drops the two facts on purpose, for the callers that never had them --
    /// the untimed wrappers, which return the plain error they always returned.
    ///
    /// Not a `From` impl: miette's blanket `From<E: Diagnostic> for Report`
    /// makes that a coherence question nobody should have to think about, and a
    /// named method reads as the deliberate discard it is.
    pub fn into_report(self) -> Report {
        self.error
    }
}

/// The conversion `?` uses. It can claim nothing, which is what keeps a
/// pre-dispatch failure honest by default -- see [`ActionFailure`].
impl From<Report> for ActionFailure {
    fn from(error: Report) -> Self {
        ActionFailure::not_dispatched(error)
    }
}

impl std::fmt::Display for ActionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for ActionFailure {}

/// The Lua the game runs to report its current tick, for
/// [`FactorioRcon::game_tick`].
///
/// Deliberately not a BotBridge function: `game.tick` and `rcon.print` are both
/// vanilla, so this answers against any save the server will load, including
/// one whose `workspace/mods` copy predates this binary. It reuses the mod's
/// own [`crate::factorio::ticks::TICK_STAMP_PREFIX`] so exactly one parser
/// ([`take_tick_stamp`]) reads a tick off a reply, whoever wrote it.
pub const GAME_TICK_QUERY: &str = "/silent-command rcon.print(\"§tick§\"..game.tick)";

pub struct FactorioRcon {
    pool: Option<bb8::Pool<ConnectionManager>>,
    silent: Arc<RwLock<bool>>,
    /// The tick stamped on the most recent reply that carried one.
    ///
    /// Every `remote_call_timed` reply arrives stamped with `game.tick` from
    /// inside the game, so the game tells us what time it is on every command
    /// we send. Keeping the last one costs nothing and spares us a mod
    /// function whose only job would be to ask a question we are already
    /// being answered.
    ///
    /// `0` means "no stamped reply yet", which is why the accessor returns an
    /// `Option` rather than handing out a tick that never happened.
    last_tick: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg_attr(test, mockall::automock)]
impl FactorioRcon {
    pub async fn new(settings: &RconSettings, silent: Arc<RwLock<bool>>) -> Result<Self> {
        let address = format!(
            "{}:{}",
            settings.host.clone().unwrap_or_else(|| "127.0.0.1".into()),
            settings.port
        );
        let manager = ConnectionManager::new(&address, &settings.pass);
        Ok(FactorioRcon {
            last_tick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pool: Some(
                bb8::Pool::builder()
                    .max_size(15)
                    .build(manager)
                    .await
                    .into_diagnostic()?,
            ),
            silent,
        })
    }

    /// Create a FactorioRcon instance without any connection; every call
    /// through [`FactorioRcon::send`] returns a "not connected" error.
    ///
    /// Why? because LuaRconBuilder requires FactorioRcon which i didnt want to
    /// change to an option. It used to *panic* rather than fail — see `send`.
    pub fn new_empty() -> Self {
        FactorioRcon {
            last_tick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pool: None,
            silent: Arc::new(RwLock::new(true)),
        }
    }

    /// Executes a couple of commands that need to be send to a newly started factorio server
    pub async fn initialize_server(&self) -> Result<()> {
        self.silent_print("").await.expect("failed to silent print");
        self.whoami("server").await.expect("failed to whoami");
        self.send("/silent-command game.surfaces[1].always_day=true")
            .await
            .expect("always day");
        Ok(())
    }

    /// Sends raw command to factorio server
    pub async fn send(&self, command: &str) -> Result<Option<Vec<String>>> {
        let silent = *self.silent.read();
        if !silent {
            info!("rcon ⮜ {}", command);
        }
        // let started = Instant::now();
        // Every rcon call funnels through here, including the `rcon.*` Lua
        // bindings. `new_empty()` builds a poolless handle by design, and this
        // used to `unwrap()` it — so "not connected" aborted the process
        // instead of returning, and under `panic = "abort"` that takes the
        // whole server with it rather than failing the one call.
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| miette!("rcon is not connected"))?;
        let mut conn = pool.get().await.into_diagnostic()?;
        let result = conn
            .cmd(&String::from(command).add("\n"))
            .await
            .into_diagnostic()?;
        drop(conn);
        // info!("send took {} ms", started.elapsed().as_millis());
        Ok(split_reply(&result, silent))
    }

    /// Calls a lua function exported by BotBridge
    async fn remote_call(
        &self,
        function_name: &str,
        args: Vec<String>,
    ) -> Result<Option<Vec<String>>> {
        self.send(&remote_call_command(function_name, &args)).await
    }

    /// [`FactorioRcon::remote_call`], with the tick BotBridge stamped on the
    /// reply taken off it.
    ///
    /// The stamp has to come off before the payload is judged: every caller
    /// below judges the reply by shape -- an action start treats any remaining
    /// line as an error, `place_entity` demands exactly one JSON document --
    /// and a stamp left in would turn every success into a failure. See
    /// [`crate::factorio::ticks::take_tick_stamp`].
    async fn remote_call_timed(
        &self,
        function_name: &str,
        args: Vec<String>,
    ) -> Result<(Option<Vec<String>>, Option<u64>)> {
        let (lines, tick) = take_tick_stamp(self.remote_call(function_name, args).await?);
        if let Some(tick) = tick {
            self.last_tick
                .store(tick, std::sync::atomic::Ordering::Relaxed);
        }
        Ok((lines, tick))
    }

    /// The game tick stamped on the most recent reply that carried one.
    ///
    /// This is *observed*, not queried: it is as current as the last command
    /// sent, and `None` before any stamped reply has arrived. A caller that
    /// needs the tick to be exactly now must send something first -- which is
    /// the honest shape, because no value here can be fresher than our last
    /// word from the game.
    pub fn last_tick(&self) -> Option<u64> {
        match self.last_tick.load(std::sync::atomic::Ordering::Relaxed) {
            0 => None,
            tick => Some(tick),
        }
    }

    /// Ask the game what tick it is **now**, rather than reading the stamp off
    /// whatever was last sent ([`FactorioRcon::last_tick`]).
    ///
    /// # Why anything needs this
    ///
    /// A tick is the game's own clock and it does not run at 60 a second just
    /// because it is meant to. A headless server sharing a machine with
    /// graphical clients, or one taking screenshots, delivers fewer: run
    /// `run-1788320177-77989` asked a furnace for 20 plates after waiting the
    /// modelled 4032 ticks *in wall clock* and got 18, because only ~3599
    /// ticks had actually elapsed. Anything that means "wait for the machine
    /// to do N ticks of work" has to read this clock; converting ticks to
    /// seconds and sleeping is a different, weaker claim.
    ///
    /// Uses BotBridge's own `§tick§` stamp format
    /// ([`crate::factorio::ticks::TICK_STAMP_PREFIX`]) but not BotBridge
    /// itself: `game.tick` needs no mod, so this keeps working against a save
    /// whose mod copy is older than this binary. `None` when the game answered
    /// nothing readable -- never a zero, which would read as tick zero.
    pub async fn game_tick(&self) -> Result<Option<u64>> {
        let (_, tick) = take_tick_stamp(self.send(GAME_TICK_QUERY).await?);
        if let Some(tick) = tick {
            self.last_tick
                .store(tick, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(tick)
    }

    /// Calls a BotBridge function whose reply must be one *complete* JSON
    /// document, and returns that document's text.
    ///
    /// Why this is not [`FactorioRcon::remote_call`] followed by
    /// `serde_json::from_str`: the pooled connection has to still be in hand
    /// when the reply is judged.
    ///
    /// This client builds its connections with `enable_factorio_quirks(true)`,
    /// which makes the `rcon` crate use `receive_single_packet_response` — it
    /// reads exactly *one* packet per command. A reply the server split across
    /// packets therefore leaves its remainder sitting unread in the socket, and
    /// nothing in the `rcon` crate reports that: `cmd` returns the first
    /// packet's body as a plain `Ok`. Left alone, the connection goes back into
    /// the `bb8` pool still holding that tail, and the *next* command on it
    /// reads someone else's reply — permanent cross-talk from one oversized
    /// read.
    ///
    /// The only in-band evidence of a short read is that the JSON does not
    /// parse, so the completeness check happens here, while
    /// [`RconConnection::mark_desynced`] can still reach the connection and
    /// keep [`ConnectionManager::has_broken`] from handing it out again. The
    /// error carries the byte count, because "expected `,` at line 1 column
    /// 393025" names the parser and a size names the cause.
    async fn remote_call_json(&self, function_name: &str, args: Vec<String>) -> Result<String> {
        let silent = *self.silent.read();
        let command = remote_call_command(function_name, &args);
        if !silent {
            info!("rcon ⮜ {}", command);
        }
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| miette!("rcon is not connected"))?;
        let mut conn = pool.get().await.into_diagnostic()?;
        let result = match conn.cmd(&command.clone().add("\n")).await {
            Ok(result) => result,
            Err(err) => {
                // The read failed part-way through a reply that was already
                // being written, so the socket's position is unknown.
                conn.mark_desynced();
                return Err(err).into_diagnostic();
            }
        };
        let Some(mut lines) = split_reply(&result, silent) else {
            return Err(RconUnexpectedEmptyResponse {}.into());
        };
        let json = lines.pop().ok_or(RconUnexpectedEmptyResponse {})?;
        // A server whose BotBridge predates this call answers with the game's
        // own "Cannot execute command. Error: No such function: ..." rather
        // than with JSON. That is a complete reply, just not the one asked
        // for, so the connection stays usable; reporting the text is the
        // difference between "expected value at line 1 column 1" and a message
        // naming the cause.
        if !json.starts_with('{') && !json.starts_with('[') {
            return Err(RconUnexpectedOutput { output: json }.into());
        }
        // `IgnoredAny` walks the document without building it: this asks only
        // "did the JSON end", which is precisely the truncation question, and
        // leaves the typed parse to the caller.
        if let Err(err) = serde_json::from_str::<serde::de::IgnoredAny>(&json) {
            conn.mark_desynced();
            return Err(err).into_diagnostic().wrap_err(format!(
                "the {} reply is not a complete JSON document: {} bytes arrived and the document \
                 does not end. This client reads one RCON packet per command, so a reply larger \
                 than the server puts in a single packet is truncated here. The connection has \
                 been dropped from the pool rather than returned to it holding the remainder.",
                function_name,
                json.len()
            ));
        }
        Ok(json)
    }

    /// Take a screenshot -> but where?
    pub async fn screenshot(&self, width: i16, height: i16, depth: i8) -> Result<()> {
        self.send(&format!("/screenshot {} {} {}", width, height, depth))
            .await?;
        Ok(())
    }

    /// Turns the mod's tick-driven frame capture on, and reports the game tick
    /// it took effect at.
    ///
    /// The cadence deliberately does *not* live here. A frame's filename
    /// carries the tick it was taken at, and only the game knows that: an RCON
    /// command arrives whenever it arrives, so a caller driving the cadence
    /// from out here could only name frames after its own loop counter -- and
    /// a counter cannot fail to produce a contiguous, plausible sequence, even
    /// when the game dropped frames. This is a toggle, not a shutter.
    ///
    /// The returned tick is the game's own (BotBridge's `stamp_tick`), so a
    /// caller can check that every frame it later reads was taken at or after
    /// the moment capture began, rather than trusting that it was.
    ///
    /// `run_id` tags the run with an opaque identifier the mod echoes into
    /// `frames/run.json` and never interprets. Pass one when the frames will
    /// have to be matched against something produced elsewhere in the same run
    /// -- a replay document, say -- so a consumer can check the two came from
    /// the same capture instead of trusting that ticks lining up means they
    /// did. Pass `None` when nothing needs correlating: the mod then writes no
    /// sidecar at all, and a consumer that finds none knows it cannot tell,
    /// which is the honest answer. It never inherits the previous run's id.
    ///
    /// # What gets captured, and what it costs
    ///
    /// **Nothing, unless `cameras` asks for something.** Screenshots were
    /// retired as a default on 2026-09-02 in favour of video: run
    /// `run-1788365280-15443` wrote 2,164 JPEGs / **947 MB** of them against
    /// **290 MB** for the same 45 minutes of video, and `game.take_screenshot`
    /// renders synchronously inside the game loop, once per camera per
    /// capture, where the video grabber reads a frame the GPU already drew.
    ///
    /// This call is still made on every recorded run, and that is deliberate:
    /// the mod's `sample_force` and `sample_bots` beats are both gated on the
    /// capture *session*, so `samples.jsonl` -- research, production, power,
    /// bot inventories -- exists only while one is running. The screenshots
    /// are the part whose cost is not worth paying; the session is not.
    ///
    /// With [`FrameCameras::All`] the mod registers `2 + one per bot` cameras:
    /// `follow` (player 1), `bot-<player_index>` for every player the game
    /// knows of, and `area`, which frames the bounding box of all connected
    /// bots. A camera whose bot is not connected writes **no file** for that
    /// tick rather than a substitute, so a bot that joined late has no frames
    /// from before it joined.
    ///
    /// **0.72 MB per frame**, measured at JPEG quality 85 and 1920x1080. One
    /// frame per camera every 300 ticks is 12 a minute, so **one camera costs
    /// ~520 MB an hour and the three cameras of a one-bot run cost ~1.56 GB an
    /// hour** — and each further bot adds a camera, hence another ~520 MB an
    /// hour. It is wiped per run and never accumulates across runs, but a long
    /// run with several bots fills a disk. The number is here so that whoever
    /// turns this back on reads it before doing so rather than afterwards.
    ///
    /// Taken by value rather than as `Option<&str>` because this `impl` is
    /// `#[automock]`ed and mockall cannot elide a lifetime inside a generic.
    pub async fn frame_capture_start(
        &self,
        run_id: Option<String>,
        cameras: FrameCameras,
    ) -> Result<Option<u64>> {
        let args = frame_capture_args(run_id.as_deref(), &cameras);
        frame_capture_verdict(self.remote_call_timed("frame_capture_start", args).await?)
    }

    /// Turns the mod's frame capture off, reporting the game tick it stopped
    /// at. Frames already written stay on disk.
    pub async fn frame_capture_stop(&self) -> Result<Option<u64>> {
        frame_capture_verdict(self.remote_call_timed("frame_capture_stop", vec![]).await?)
    }

    /// Print given message to all Clients as Chat Message from Server loudly using /c
    pub async fn print(&self, message: &str) -> Result<()> {
        self.send(&format!("/c print({})", str_to_lua(message)))
            .await?;
        Ok(())
    }

    /// Print given message to all Clients as Chat Message from Server silenty using /silent-command
    pub async fn silent_print(&self, str: &str) -> Result<()> {
        self.send(&format!("/silent-command print({})", str_to_lua(str)))
            .await?;
        Ok(())
    }

    /// Save the current game on server
    pub async fn server_save(&self) -> Result<()> {
        self.send("/server-save").await?;
        Ok(())
    }

    /// Starts initial discovery process for "server"
    pub async fn whoami(&self, name: &str) -> Result<()> {
        expect_silence(self.remote_call("whoami", vec![str_to_lua(name)]).await?)
    }

    /// The mod's `players` reply as one JSON string, or `None` when nobody is
    /// connected.
    ///
    /// Lua's `helpers.table_to_json({})` yields `"{}"` rather than `"[]"` for
    /// an empty table, so an empty result arrives as an object. That is not an
    /// error; it means nobody is connected. Shared by the two accessors below
    /// so they cannot disagree about what an empty reply means.
    async fn connected_players_json(&self) -> Result<Option<String>> {
        let Some(lines) = self.remote_call("players", vec![]).await? else {
            return Ok(None);
        };
        let json_str = lines.join("");
        if json_str == "{}" || json_str.is_empty() {
            return Ok(None);
        }
        Ok(Some(json_str))
    }

    /// Every connected player that has a character, as the mod reports them.
    pub async fn connected_players(&self) -> Result<Vec<FactorioPlayer>> {
        let Some(json_str) = self.connected_players_json().await? else {
            return Ok(vec![]);
        };
        serde_json::from_str::<Vec<FactorioPlayer>>(&json_str)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to parse players: {json_str}"))
    }

    /// Returns the number of connected players (with characters)
    ///
    /// Counts the reply's elements without interpreting them: this drives the
    /// startup wait for clients to connect, and a player object we cannot fully
    /// deserialize is still a connected player. A malformed reply counts as
    /// zero, as it always has, rather than failing the wait loop.
    pub async fn connected_player_count(&self) -> Result<usize> {
        let Some(json_str) = self.connected_players_json().await? else {
            return Ok(0);
        };
        match serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            Ok(players) => Ok(players.len()),
            Err(e) => {
                info!("rcon players parse error: {} for: {}", e, json_str);
                Ok(0)
            }
        }
    }

    /// Adds research to the queue, **without waiting for it to finish**.
    ///
    /// The queue-only path, for the Lua binding and the REST endpoint: a
    /// script that wants to start a research and carry on should not block for
    /// minutes. Anything that needs the technology to *exist* afterwards wants
    /// [`FactorioRcon::research_timed`].
    pub async fn add_research(&self, technology_name: &str) -> Result<()> {
        self.add_research_timed(technology_name)
            .await
            .map(|_| ())
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::add_research`], reporting the game tick it ran at.
    ///
    /// **Both ends of the returned [`ActionTicks`] are the tick the technology
    /// was queued at, and that is all this measures.** Queueing is synchronous
    /// -- the command runs and returns inside one tick -- but the *research*
    /// is not: it takes labs, science packs and minutes, and finishes on
    /// `on_research_finished` long after this has returned. Treating this
    /// method's success as "the technology exists now" is the defect
    /// [`FactorioRcon::research_timed`] was added to fix; it reported rung 7 of
    /// the milestone ladder complete the instant the research was requested.
    pub async fn add_research_timed(
        &self,
        technology_name: &str,
    ) -> Result<ActionTicks, ActionFailure> {
        let (lines, tick) = self
            .remote_call_timed("add_research", vec![str_to_lua(technology_name)])
            .await?;
        let ran_at = ActionTicks::at(tick);
        // Any surviving line is the game refusing. Factorio writes
        // "Cannot execute command. Error: ..." into the reply body when the mod
        // raises, and the mod writes its own refusal there for the cases that
        // do not raise at all.
        //
        // This was `_lines` until 2026-09-01, discarded by name, so *every*
        // call reported success -- including researching a technology that does
        // not exist. A run spent 61345 ticks re-planning `research electronics`
        // five times, was told success five times, and the supervisor halted it
        // as `stuck_silent`: no progress while everything claimed to work.
        if let Some(lines) = lines {
            return Err(ActionFailure::refused(
                RconUnexpectedOutput {
                    output: lines.join("\n"),
                }
                .into(),
                ran_at,
            ));
        }
        Ok(ran_at)
    }

    /// Queues `technology_name` **and waits for the game to finish researching
    /// it**.
    ///
    /// The durative counterpart of [`FactorioRcon::add_research_timed`], and
    /// the one the executor uses. `LuaForce.add_research` answers "did this
    /// enter the queue", never "is this researched"; the completion arrives
    /// ticks or minutes later as `on_research_finished`, which carried no
    /// action id until the mod started remembering which ids asked for which
    /// technology (`research_actions`, `mods/BotBridge/control.lua`).
    ///
    /// Shaped exactly like [`FactorioRcon::player_craft_timed`], for the same
    /// reason: in both, the *game* does the durative work and announces the end
    /// of it, so the mod needs no `on_tick` follower and nothing here polls the
    /// game. The wait is on the push channel -- the mod's `action_completed`
    /// writeout, parsed into `world.actions` -- so the reply tick is the game's
    /// own `game.tick` at the moment the research finished.
    ///
    /// The two ticks it returns are therefore genuinely different numbers: when
    /// the technology was queued, and when it was done.
    ///
    /// # The deadline this is subject to
    ///
    /// `ACTION_RESULT_DEADLINE` is 360 wall-clock seconds. Research is the one
    /// action kind whose real duration is set by the factory rather than by the
    /// bot -- lab count, science supply, speed modules -- so it is also the one
    /// most able to outlast that deadline honestly. A research that does so is
    /// reported [`Dispatch::NoVerdict`], which is the correct claim (the game
    /// took the command and we stopped listening) but is not the same as a
    /// failure.
    pub async fn research_timed(
        &self,
        world: &Arc<FactorioWorld>,
        technology_name: &str,
    ) -> Result<ActionTicks, ActionFailure> {
        let mut next_action_id = world.as_ref().next_action_id.lock().await;
        let action_id: ActionId = *next_action_id;
        *next_action_id = (*next_action_id + 1) % 1000;
        drop(next_action_id);
        let (lines, dispatched) = self
            .remote_call_timed(
                "action_start_research",
                vec![action_id.to_string(), str_to_lua(technology_name)],
            )
            .await?;
        // A refusal is answered in the reply body and is the end of it -- the
        // mod registers nothing for a technology it would not queue, so there
        // is no completion coming and nothing to wait for. Classified
        // `Refused` rather than left to `?`: the game did see this command and
        // did judge it, and that is a stronger claim than `NotDispatched`.
        if let Some(lines) = lines {
            return Err(ActionFailure::refused(
                RconUnexpectedOutput {
                    output: lines.join("\n"),
                }
                .into(),
                ActionTicks::at(dispatched),
            ));
        }
        self.sleep_for_action_result(world, action_id, dispatched)
            .await
    }

    /// Cheats in an Item in given quantity to given player
    pub async fn cheat_item(
        &self,
        player_id: PlayerId,
        item_name: &str,
        item_count: u32,
    ) -> Result<()> {
        expect_silence(
            self.remote_call(
                "cheat_item",
                vec![
                    player_id.to_string(),
                    str_to_lua(item_name),
                    item_count.to_string(),
                ],
            )
            .await?,
        )
    }

    pub async fn cheat_technology(&self, technology_name: &str) -> Result<()> {
        expect_silence(
            self.remote_call("cheat_technology", vec![str_to_lua(technology_name)])
                .await?,
        )
    }

    pub async fn cheat_all_technologies(&self) -> Result<()> {
        expect_silence(self.remote_call("cheat_all_technologies", vec![]).await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn place_blueprint(
        &self,
        player_id: PlayerId,
        blueprint: String,
        position: &Position,
        direction: u8,
        force_build: bool,
        only_ghosts: bool,
        inventory_player_ids: Vec<u8>,
        world: &Arc<FactorioWorld>,
    ) -> Result<Vec<FactorioEntity>> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(RconPlayerNotFound { player_id }.into());
        }
        let player = player.unwrap();
        let distance = calculate_distance(&player.position, position);
        let build_distance = player.build_distance as f64;
        drop(player); // wow, without this factorio (?) freezes (!)
        if distance > build_distance {
            warn!("too far away, moving first!");
            self.move_player(world, player_id, position, Some(build_distance))
                .await?;
        }
        // TODO: move inventory players close too

        let build_area = blueprint_build_area(world.entity_prototypes.clone(), &blueprint);
        let width_2 = build_area.width() / 2.0;
        let height_2 = build_area.height() / 2.0;
        let build_area = Rect {
            left_top: Position::new(position.x() - width_2, position.y() - height_2),
            right_bottom: Position::new(position.x() + width_2, position.y() + height_2),
        };
        let build_area_entities = self
            .find_entities_filtered(&AreaFilter::Rect(build_area.clone()), None, None)
            .await?;

        for entity in build_area_entities {
            if entity.name != "character"
                && entity.entity_type != "resource"
                && build_area.contains(&entity.position)
            {
                warn!(
                    "mining entity in build area: {} @ {}/{}",
                    entity.name,
                    entity.position.x(),
                    entity.position.y()
                );
                self.player_mine(world, player_id, &entity.name, &entity.position, 1)
                    .await?;
            }
        }
        let inventory_player_ids: Vec<String> = inventory_player_ids
            .iter()
            .map(|player_id| player_id.to_string())
            .collect();
        let lines = self
            .remote_call(
                "place_blueprint",
                vec![
                    player_id.to_string(),
                    str_to_lua(&blueprint),
                    position.x().to_string(),
                    position.y().to_string(),
                    direction.to_string(),
                    force_build.to_string(),
                    only_ghosts.to_string(),
                    vec_to_lua(inventory_player_ids),
                ],
            )
            .await?;
        if lines.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let mut json = lines.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        if json.starts_with('[') {
            Ok(parse_reply("place_blueprint", &json)?)
        } else {
            Err(RconError { message: json }.into())
        }
    }

    pub async fn revive_ghost(
        &self,
        player_id: PlayerId,
        name: &str,
        position: &Position,
        world: &Arc<FactorioWorld>,
    ) -> Result<FactorioEntity> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(RconPlayerNotFound { player_id }.into());
        }
        let player = player.unwrap();
        let build_distance = player.build_distance as f64;
        let distance = calculate_distance(&player.position, position);
        drop(player); // wow, without this factorio (?) freezes (!)
        if distance > build_distance {
            warn!("too far away, moving first!");
            self.move_player(world, player_id, position, Some(build_distance))
                .await?;
        }
        let lines = self
            .remote_call(
                "revive_ghost",
                vec![
                    player_id.to_string(),
                    str_to_lua(name),
                    position.x().to_string(),
                    position.y().to_string(),
                ],
            )
            .await?;
        if lines.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let json = lines.unwrap().pop().unwrap();
        if json.starts_with('{') {
            Ok(parse_reply("revive_ghost", &json)?)
        } else {
            Err(RconError { message: json }.into())
        }
    }

    pub async fn cheat_blueprint(
        &self,
        player_id: PlayerId,
        blueprint: String,
        position: &Position,
        direction: u8,
        force_build: bool,
    ) -> Result<Vec<FactorioEntity>> {
        let lines = self
            .remote_call(
                "cheat_blueprint",
                vec![
                    player_id.to_string(),
                    str_to_lua(&blueprint),
                    position.x().to_string(),
                    position.y().to_string(),
                    direction.to_string(),
                    force_build.to_string(),
                ],
            )
            .await?;
        if lines.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let mut json = lines.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        parse_reply("cheat_blueprint", &json)
    }

    pub async fn store_map_data(&self, key: &str, value: Value) -> Result<()> {
        expect_silence(
            self.remote_call(
                "store_map_data",
                vec![str_to_lua(key), value_to_lua(&value)],
            )
            .await?,
        )
    }

    pub async fn retrieve_map_data(&self, key: &str) -> Result<Option<Value>> {
        let lines = self
            .remote_call("retrieve_map_data", vec![str_to_lua(key)])
            .await?;
        if lines.is_none() {
            return Ok(None);
        }
        let mut json = lines.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        Ok(Some(parse_reply("retrieve_map_data", &json)?))
    }

    /// Waits for the game's verdict on a dispatched action and returns the
    /// **game tick it arrived at**.
    ///
    /// The tick is not new information the game had to be asked for: the mod
    /// has always stamped it on its `action_completed` event. It used to be
    /// dropped by `OutputParser`, which is precisely why the executor had no
    /// game clock and had to describe its timings as planned rather than
    /// observed.
    async fn sleep_for_action_result(
        &self,
        world: &Arc<FactorioWorld>,
        action_id: ActionId,
        dispatched: Option<u64>,
    ) -> Result<ActionTicks, ActionFailure> {
        self.sleep_for_action_result_until(world, action_id, dispatched, ACTION_RESULT_DEADLINE)
            .await
    }

    /// [`FactorioRcon::sleep_for_action_result`] with the deadline named, so a
    /// test can reach the timeout branch without waiting six minutes for it.
    /// Nothing else about the two differs.
    async fn sleep_for_action_result_until(
        &self,
        world: &Arc<FactorioWorld>,
        action_id: ActionId,
        dispatched: Option<u64>,
        deadline: Duration,
    ) -> Result<ActionTicks, ActionFailure> {
        let wait_start = Instant::now();
        loop {
            sleep(Duration::from_millis(50)).await;
            // Take the reply in one operation. Looking it up with `get` and
            // then calling `remove` holds the shard's read guard across a call
            // that needs the same shard's write guard, which self-deadlocks the
            // whole task on the first tick the reply is actually there.
            if let Some((_, outcome)) = world.actions.remove(&action_id) {
                let ticks = ActionTicks::new(dispatched, Some(outcome.tick));
                if outcome.is_ok() {
                    return Ok(ticks);
                }
                // A verdict, and it was no. Both stamps are real -- the game
                // told us when it took the command and when it gave up on it --
                // and the failure carries them for the same reason the success
                // does.
                return Err(ActionFailure::refused(
                    RconError {
                        message: outcome.result,
                    }
                    .into(),
                    ticks,
                ));
            }
            if wait_start.elapsed() > deadline {
                // The one place in this file entitled to say an action may
                // still be out there: `action_start_*` already returned, so the
                // game acknowledged this command, and no verdict followed.
                // `replied` stays absent because nothing replied.
                return Err(ActionFailure::no_verdict(
                    RconTimeout {}.into(),
                    ActionTicks::new(dispatched, None),
                ));
            }
        }
    }

    async fn sleep_for_path_request_result(
        &self,
        world: &Arc<FactorioWorld>,
        request_id: u32,
    ) -> Result<Vec<PathWaypoint>> {
        let wait_start = Instant::now();
        loop {
            sleep(Duration::from_millis(50)).await;
            // Take the reply in one operation -- see sleep_for_action_result.
            if let Some((_, mut result)) = world.path_requests.remove(&request_id) {
                if result == "{}" {
                    result = String::from("[]");
                }
                // The mod fills this slot with *either* a JSON array of
                // waypoints or one of its own plain-text verdicts —
                // `Error: failed to path find`, `Error: try again later!`
                // (`on_script_path_request_finished`, control.lua). Both used
                // to go straight to `serde_json`, so a pathfinder that had
                // answered clearly came back as `expected value at line 1
                // column 1` and the answer was thrown away. That string is
                // what halted the 2026-09-02 research run, reported against a
                // `place` whose blocked-placement recovery walks the bot out
                // of its own build site.
                if !result.starts_with('[') {
                    return Err(RconPathRequestFailed { reason: result }.into());
                }
                return parse_reply("async_request_path", &result);
            }
            if wait_start.elapsed() > Duration::from_secs(60) {
                return Err(RconTimeout {}.into());
            }
        }
    }

    pub async fn move_player(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<()> {
        self.move_player_timed(world, player_id, goal, radius)
            .await
            .map(|_| ())
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::move_player`], reporting the game ticks it was observed
    /// at: the tick the game accepted the waypoints, and the tick it reported
    /// the walk finished at.
    ///
    /// # A walk that cannot arrive is refused rather than walked
    ///
    /// [`FactorioRcon::player_path`] is best effort: when the goal itself is
    /// unreachable it retries against a *synthesised* goal offset away from the
    /// real one by the radius, and returns the path to that. The walk along
    /// such a path completes normally, so every signal downstream — the mod's
    /// `action_completed`, the actuator, the execution log — says success while
    /// the bot stands somewhere it was never asked to be. In the 2026-08-30
    /// live run that was 9.30 tiles out, because the goal was the tile the bot
    /// had just put a furnace on, and nothing downstream could tell.
    ///
    /// So the path is checked against the goal the *caller* asked for, before
    /// anything is dispatched, and a path that does not reach it is
    /// [`Dispatch::NotDispatched`] with [`RconWalkFallsShort`]. That is the
    /// honest classification and it uses no new signal: the walk genuinely did
    /// not happen, nothing is outstanding, and the executor renders it
    /// `Failed` rather than `Lost`. Checking here rather than inside
    /// `player_path` keeps that method usable as the "get near" primitive its
    /// other callers want, and checking *before* the dispatch means the bot is
    /// not marched across the map for nothing.
    ///
    /// # A walk that ends inside a building is refused too
    ///
    /// Arriving is not the only way a path can be useless. The last waypoint is
    /// the walk's destination *in the mod*, and the pathfinder will happily
    /// return one inside a building we ourselves built: 12 of run 30's 75
    /// returned paths had a waypoint strictly inside a stone furnace, each
    /// flagged `needs_destroy_to_reach: false`. All three of that run's failed
    /// walks are exactly the ones whose last waypoint was one of those, and
    /// each cost four re-paths and a leg timeout before reporting the terrain
    /// as unreachable — while the re-path could not have succeeded, since
    /// `WALK_REPATH_RADIUS` is 0.5 and standing clear of a stone furnace needs
    /// 0.8984375.
    ///
    /// So the destination is also checked for standability, against the entity
    /// graph, and refused as [`RconWalkEndsWhereNobodyCanStand`] naming the
    /// obstruction. Only a *provable* overlap refuses: see
    /// [`StandingVerdict`] for why "cannot tell" has to be allowed through.
    pub async fn move_player_timed(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<ActionTicks, ActionFailure> {
        let mut next_action_id = world.as_ref().next_action_id.lock().await;
        let action_id: ActionId = *next_action_id;
        *next_action_id = (*next_action_id + 1) % 1000;
        drop(next_action_id);

        // Everything up to and including `action_start_walk_waypoints` is the
        // pre-dispatch phase, and every `?` here goes through
        // `From<Report> for ActionFailure` -- so a path request that times out
        // comes out `NotDispatched`, with no tick and no claim that anything is
        // outstanding. That is the case a timeout alone cannot tell from the
        // one below.
        let waypoints = self.player_path(world, player_id, goal, radius).await?;

        // The pre-dispatch judgement: does this path reach the goal, and can a
        // character stand where it ends. `player_path` may have substituted the
        // goal, so the path is judged against what the caller asked for. An
        // unknown player position with an empty path leaves nothing to judge,
        // and an unjudgeable walk is dispatched rather than refused on a guess.
        let here = world.players.get(&player_id).map(|p| p.position.clone());
        judge_path(world, goal, radius, &waypoints, here.as_ref())?;

        let dispatched = self
            .action_start_walk_waypoints(action_id, player_id, waypoints)
            .await?;
        self.sleep_for_action_result(world, action_id, dispatched)
            .await
    }

    pub async fn player_mine(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        name: &str,
        position: &Position,
        count: u32,
    ) -> Result<()> {
        self.player_mine_timed(world, player_id, name, position, count)
            .await
            .map(|_| ())
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::player_mine`], reporting the game ticks it was observed
    /// at.
    ///
    /// If the player has to walk to the resource first, the ticks reported on
    /// **success** are the *mining* action's own -- the walk is a separate
    /// dispatch with separate ticks, and folding the two together would make
    /// the mine look like it started when the bot set off.
    ///
    /// # A corrective walk that fails keeps what the game stamped on it
    ///
    /// On the failure path there are no mining ticks to displace, and the walk
    /// is dispatched via [`FactorioRcon::move_player_timed`] rather than
    /// [`FactorioRcon::move_player`] so its [`ActionFailure`] arrives here
    /// intact. `move_player` ends in `map_err(ActionFailure::into_report)`,
    /// which keeps only the message: the [`Dispatch`] phase and the
    /// [`crate::factorio::ticks::ActionTicks`] are dropped, and the `?` here
    /// then rebuilt the failure through `From<Report>` as
    /// [`Dispatch::NotDispatched`] with [`ActionTicks::UNKNOWN`].
    ///
    /// Both halves of that were wrong for a walk the game had acknowledged and
    /// then gone quiet on. `NotDispatched` claims nothing is outstanding while
    /// the bot is still walking somewhere the plan does not know about; and an
    /// untimed failure is written to the run record as *nothing at all* --
    /// `record.actions` skips any action with neither tick. Run 9
    /// (`workspace/runs/run-1788310810-27811`) has two plans that each took a
    /// full `ACTION_RESULT_DEADLINE` and contain no event explaining where the
    /// six minutes went, because this is where they went.
    ///
    /// The blast radius is only the silent case: everything raised before
    /// `action_start_walk_waypoints` returns is still `NotDispatched` and
    /// still renders `Rejected`.
    ///
    /// # The reach is re-checked after that walk, and the mine is refused if it
    /// still is not met
    ///
    /// The game enforces `resource_reach_distance` **silently**: a
    /// `mining_state` aimed at a resource outside it produces no event, no
    /// error and no progress. In the 2026-08-30 live run the corrective walk
    /// left the bot 3.345 tiles from the ore against a reach of 3, the mine was
    /// dispatched regardless, and the action sat unanswered for 21,400 ticks
    /// until the 360-second deadline declared it lost.
    ///
    /// "Within `radius` of the goal" is not "within `radius` of the goal once
    /// you stop" — the path's last waypoint is on a tile centre and the mod's
    /// follower halts within a 0.3-by-0.3 box of it — so the walk now aims at
    /// [`approach_radius`], comfortably inside the reach, and where it
    /// actually ended is re-measured afterwards. Falling short is
    /// [`Dispatch::NotDispatched`] with [`RconOutOfResourceReach`]: nothing was
    /// sent, so nothing is outstanding, and a six-minute silence becomes an
    /// immediate refusal the executor can retry.
    pub async fn player_mine_timed(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        name: &str,
        position: &Position,
        count: u32,
    ) -> Result<ActionTicks, ActionFailure> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(ActionFailure::not_dispatched(
                RconPlayerNotFound { player_id }.into(),
            ));
        }
        let player = player.unwrap();
        let mut next_action_id = world.as_ref().next_action_id.lock().await;
        let action_id: ActionId = *next_action_id;
        *next_action_id = (*next_action_id + 1) % 1000;
        drop(next_action_id);
        // The reach is the value the game reported for this player -- 2.7 for a
        // plain character, `f64::MAX` for one without a character. Never 3.
        let resource_reach_distance = player.resource_reach_distance;
        let here = player.position.clone();
        drop(player); // wow, without this factorio (?) freezes (!)
        if !within_resource_reach(&here, position, resource_reach_distance) {
            warn!("too far away, moving first!");
            self.move_player_timed(
                world,
                player_id,
                position,
                Some(approach_radius(resource_reach_distance)),
            )
            .await?;
            // Where the walk *ended*, not where it was aimed. The two differ by
            // about a tile, and that difference is the whole of this fault.
            let landed = world
                .players
                .get(&player_id)
                .map(|p| p.position.clone())
                .ok_or_else(|| {
                    ActionFailure::not_dispatched(RconPlayerNotFound { player_id }.into())
                })?;
            if !within_resource_reach(&landed, position, resource_reach_distance) {
                return Err(ActionFailure::not_dispatched(
                    RconOutOfResourceReach {
                        target_x: position.x(),
                        target_y: position.y(),
                        distance: calculate_distance(&landed, position),
                        reach: resource_reach_distance,
                    }
                    .into(),
                ));
            }
        }
        // The walk a mine may make first is a *different* dispatch with its own
        // action id, so a failure in it leaves this mine un-dispatched -- which
        // is what the `?` says.
        let dispatched = self
            .action_start_mining(action_id, player_id, name, position, count)
            .await?;
        let outcome = self
            .sleep_for_action_result(world, action_id, dispatched)
            .await;
        // Retire the tile the moment the game says it is done for. Nothing else
        // does: the mod reports a resource's `amount` when its chunk is written
        // out and never again, and it emits no depletion event at all, so
        // without this the planner keeps choosing a tile a bot already mined
        // dry and the bot walks back to nothing. See
        // `EntityGraph::resource_mined`.
        match &outcome {
            Ok(_) => {
                world.entity_graph.resource_mined(name, position, count);
            }
            Err(failure) if mine_reports_target_gone(&failure.error.to_string()) => {
                world.entity_graph.retire_resource(name, position);
            }
            Err(_) => {}
        }
        outcome
    }

    pub async fn player_craft(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        recipe: &str,
        count: u32,
    ) -> Result<()> {
        self.player_craft_timed(world, player_id, recipe, count)
            .await
            .map(|_| ())
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::player_craft`], reporting the game ticks it was observed
    /// at.
    ///
    /// **Durative.** This returns when the game raises
    /// `on_player_crafted_item` for the last craft the request asked for, not
    /// when the crafts are queued. The join is `(player, recipe)` — all the
    /// event carries — and lives in `storage.craft_actions`
    /// (`mods/BotBridge/control.lua`); the two ticks it returns are therefore
    /// genuinely different numbers, the queue tick and the finish tick.
    ///
    /// A craft the game cancels settles as a failure on
    /// `on_player_cancelled_crafting`, and a craft the game will not start is
    /// answered in the reply body, below — neither costs the
    /// `ACTION_RESULT_DEADLINE`. What still can is a craft the game accepts and
    /// then neither finishes nor cancels, which is [`Dispatch::NoVerdict`]: the
    /// correct claim, and not the same as a failure.
    pub async fn player_craft_timed(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        recipe: &str,
        count: u32,
    ) -> Result<ActionTicks, ActionFailure> {
        let mut next_action_id = world.as_ref().next_action_id.lock().await;
        let action_id: ActionId = *next_action_id;
        *next_action_id = (*next_action_id + 1) % 1000;
        drop(next_action_id);
        let (lines, dispatched) = self
            .remote_call_timed(
                "action_start_crafting",
                vec![
                    action_id.to_string(),
                    player_id.to_string(),
                    str_to_lua(recipe),
                    count.to_string(),
                ],
            )
            .await?;
        // A refusal is answered in the reply body and is the end of it -- the
        // mod registers nothing for a craft it would not start, so there is no
        // completion coming and nothing to wait for. Classified `Refused`
        // rather than left to `?` on `action_start_crafting`: the game did see
        // this command and did judge it, and that is a stronger claim than
        // `NotDispatched`. It also keeps the dispatch tick, which the weaker
        // path throws away. Same shape as [`FactorioRcon::research_timed`].
        if let Some(lines) = lines {
            return Err(ActionFailure::refused(
                RconUnexpectedOutput {
                    output: lines.join("\n"),
                }
                .into(),
                ActionTicks::at(dispatched),
            ));
        }
        self.sleep_for_action_result(world, action_id, dispatched)
            .await
    }

    pub async fn inventory_contents_at(
        &self,
        entities: Vec<RequestEntity>,
    ) -> Result<Vec<Option<InventoryResponse>>> {
        let positions: Vec<String> = entities
            .into_iter()
            .map(|entity| {
                let mut map: HashMap<String, String> = HashMap::new();
                map.insert(String::from("name"), str_to_lua(&entity.name));
                map.insert(
                    String::from("position"),
                    vec_to_lua(vec![
                        entity.position.x.to_string(),
                        entity.position.y.to_string(),
                    ]),
                );
                hashmap_to_lua(map)
            })
            .collect();

        let lines = self
            .remote_call("inventory_contents_at", vec![vec_to_lua(positions)])
            .await?;
        if lines.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let mut json = lines.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        parse_reply("inventory_contents_at", &json)
    }

    /// The bulk static world data — prototypes, recipes and the player force —
    /// in one reply.
    ///
    /// The RCON counterpart of the mod's `writeout_*` functions, which only a
    /// server this process spawned can be read from. Both transports share the
    /// `collect_*` functions in `control.lua`, so this returns the same records
    /// the stdout path parses.
    ///
    /// One call rather than a paged protocol. Measured on Factorio 2.1.17 with
    /// Space Age on a fresh freeplay map, the whole reply is 744 kB
    /// (209 kB of entity prototypes, 49 kB of item prototypes, 138 kB for the
    /// player force's technologies, 345 kB of recipes); a probe
    /// of `rcon.print(string.rep('x', n))` against the same server returned
    /// 16 MB in a single RCON packet, so the payload still has more than an
    /// order of magnitude of headroom.
    ///
    /// It very nearly doubled when `collect_recipes` stopped filtering on
    /// `enabled` — 393 kB to 744 kB, almost all of it the 639 disabled recipes
    /// a fresh map has — and that was measured rather than assumed, because a
    /// truncated payload that silently dropped recipes would be a worse bug
    /// than the missing-recipe one the change fixes. Note that this crate
    /// configures the `rcon`
    /// connection with `enable_factorio_quirks(true)`, which reads exactly one
    /// packet per command — a chunked reply would need multi-packet reads that
    /// this client does not do. [`FactorioRcon::remote_call_json`] is what
    /// makes that limit *loud* instead of silent if a bigger base, a wider
    /// radius or more mods ever push a reply past it.
    pub async fn world_snapshot(&self) -> Result<WorldSnapshot> {
        let json = self.remote_call_json("world_snapshot", vec![]).await?;
        serde_json::from_str(json.as_str())
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "failed to parse the world_snapshot reply ({} bytes)",
                    json.len()
                )
            })
    }

    pub async fn player_force(&self) -> Result<FactorioForce> {
        let lines = self.remote_call("player_force", vec![]).await?;
        if lines.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let json = lines.unwrap().pop().unwrap();
        parse_reply("player_force", &json)
    }

    /// Asks the game whether each of `queries` could be built, **before** any
    /// of them is dispatched, and remembers the ones it refuses.
    ///
    /// # One round trip, whatever the plan's size
    ///
    /// Every query goes out in a single `remote.call`, and the answer is one
    /// JSON document. A 103-step plan with six placements costs one call, not
    /// six; the reply is a handful of bytes per site, orders of magnitude
    /// short of the single-packet limit [`FactorioRcon::remote_call_json`]
    /// guards.
    ///
    /// # What it records, and what it deliberately does not
    ///
    /// A verdict that [`PlacementVerdict::is_durable_refusal`] lands in the
    /// world's refusal ledger exactly as a dispatch-time refusal does, so
    /// `PlanState::from_world` excludes the footprint on the very next
    /// expansion and `record.refusals()` writes it out. A refusal with a
    /// character in the footprint, and a site the mod could not judge, are
    /// **not** recorded — see [`PlacementVerdict::character`].
    ///
    /// # What a green answer is not
    ///
    /// It is not a guarantee. It is the game's answer at the tick it was
    /// asked, and the plan runs afterwards: a bot can walk into the footprint,
    /// a biter can wander in, and an earlier step of the same plan can put
    /// something there. The dispatch-time refusal path is still the backstop
    /// and is unchanged.
    pub async fn can_place_entities(
        &self,
        world: &Arc<FactorioWorld>,
        queries: &[PlacementQuery],
    ) -> Result<Vec<PlacementVerdict>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let sites: Vec<String> = queries
            .iter()
            .map(|q| {
                format!(
                    "{{ player = {}, item = {}, position = {}, direction = {} }}",
                    q.player_id,
                    str_to_lua(&q.item_name),
                    vec_to_lua(vec![q.position.x.to_string(), q.position.y.to_string()]),
                    q.direction,
                )
            })
            .collect();
        let json = self
            .remote_call_json("can_place_entities", vec![vec_to_lua(sites)])
            .await?;
        let reply: PlacementVerdicts = serde_json::from_str(&json)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to parse the can_place_entities reply: {json}"))?;
        accept_verdicts(world, queries, reply)
    }

    pub async fn place_entity(
        &self,
        player_id: PlayerId,
        item_name: String,
        entity_position: Position,
        direction: u8,
        world: &Arc<FactorioWorld>,
    ) -> Result<FactorioEntity> {
        self.place_entity_timed(player_id, item_name, entity_position, direction, world)
            .await
            .map(|(entity, _ticks)| entity)
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::place_entity`], reporting the game tick it ran at
    /// alongside the entity it created.
    ///
    /// Placement is synchronous -- `surface.create_entity` returns within the
    /// tick the command was received -- so both ends of [`ActionTicks`] are the
    /// same number. On the `§player_blocks_placement§` path the placement is
    /// retried after a walk, and the tick reported is the *retry's*, because
    /// that is the dispatch that actually placed the entity.
    pub async fn place_entity_timed(
        &self,
        player_id: PlayerId,
        item_name: String,
        entity_position: Position,
        direction: u8,
        world: &Arc<FactorioWorld>,
    ) -> Result<(FactorioEntity, ActionTicks), ActionFailure> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(ActionFailure::not_dispatched(
                RconPlayerNotFound { player_id }.into(),
            ));
        }
        let player = player.unwrap();
        let player_position = player.position.clone();
        let build_distance = player.build_distance as f64;
        drop(player); // wow, without this factorio (?) freezes (!)
        let distance = calculate_distance(&player_position, &entity_position);
        if distance > build_distance {
            warn!("too far away, moving first!");
            self.move_player(world, player_id, &entity_position, Some(build_distance))
                .await?;
        }
        let (lines, tick) = self
            .remote_call_timed(
                "place_entity",
                vec![
                    player_id.to_string(),
                    str_to_lua(&item_name),
                    position_to_lua(&entity_position),
                    direction.to_string(),
                ],
            )
            .await?;
        // Past this point the game has answered the RPC, so every failure below
        // is a verdict the game gave and carries the tick it gave it at. On the
        // blocked-and-retry path that stays true: `refused_at` is re-bound to
        // the retry's stamp, which is the dispatch the outcome belongs to.
        let refused_at = ActionTicks::at(tick);
        if let Some(lines) = lines {
            if lines.len() != 1 {
                Err(ActionFailure::refused(
                    RconUnexpectedOutput {
                        output: lines.join("\n"),
                    }
                    .into(),
                    refused_at,
                ))
            } else {
                let line = &lines[0];
                // `starts_with`, not a grapheme index. The old form built a
                // grapheme vector and read `chars[0]`, which panics on the
                // empty line an empty reply body splits into -- a panic inside
                // a run's dispatch task, on the failure path, where a returned
                // error is what the executor is waiting for.
                if line.starts_with('{') {
                    Ok((
                        parse_reply("place_entity", line)
                            .map_err(|e| ActionFailure::refused(e, refused_at))?,
                        ActionTicks::at(tick),
                    ))
                } else if &line[..] == "§player_blocks_placement§" {
                    // The eight compass points. This was `0..8u8` on the
                    // Factorio 1.x scale, where those were all eight
                    // directions; on the 2.x scale `0..8` is only half a
                    // circle, so it has to be named rather than counted.
                    for test_direction in Direction::compass() {
                        let Some(test_position) =
                            move_position(&player_position, test_direction, 5.0)
                        else {
                            continue;
                        };
                        if self
                            .is_area_empty(&AreaFilter::PositionRadius((
                                test_position.clone(),
                                Some(2.0),
                            )))
                            .await
                            .map_err(|e| ActionFailure::refused(e, refused_at))?
                        {
                            self.move_player(world, player_id, &test_position, Some(1.0))
                                .await
                                .map_err(|e| ActionFailure::refused(e, refused_at))?;
                            let (lines, tick) = self
                                .remote_call_timed(
                                    "place_entity",
                                    vec![
                                        player_id.to_string(),
                                        str_to_lua(&item_name),
                                        position_to_lua(&entity_position),
                                        direction.to_string(),
                                    ],
                                )
                                .await?;
                            let refused_at = ActionTicks::at(tick);
                            return if let Some(lines) = lines {
                                if lines.len() != 1 {
                                    return Err(ActionFailure::refused(
                                        RconUnexpectedOutput {
                                            output: lines.join("\n"),
                                        }
                                        .into(),
                                        refused_at,
                                    ));
                                }
                                let line = &lines[0];
                                if line.starts_with('{') {
                                    Ok((
                                        parse_reply("place_entity", line)
                                            .map_err(|e| ActionFailure::refused(e, refused_at))?,
                                        ActionTicks::at(tick),
                                    ))
                                } else if &line[..] == "§player_blocks_placement§" {
                                    Err(ActionFailure::refused(
                                        RconPlayerBlockesPlacement {}.into(),
                                        refused_at,
                                    ))
                                } else {
                                    note_placement_refusal(
                                        world,
                                        tick,
                                        line,
                                        &item_name,
                                        &entity_position,
                                    );
                                    Err(ActionFailure::refused(
                                        RconError {
                                            message: line.clone(),
                                        }
                                        .into(),
                                        refused_at,
                                    ))
                                }
                            } else {
                                Err(ActionFailure::refused(
                                    RconUnexpectedEmptyResponse {}.into(),
                                    refused_at,
                                ))
                            };
                        }
                    }
                    Err(ActionFailure::refused(
                        RconPlayerBlockesAllPlacement {}.into(),
                        refused_at,
                    ))
                } else {
                    note_placement_refusal(world, tick, line, &item_name, &entity_position);
                    Err(ActionFailure::refused(
                        RconError {
                            message: line.clone(),
                        }
                        .into(),
                        refused_at,
                    ))
                }
            }
        } else {
            Err(ActionFailure::refused(
                RconUnexpectedEmptyResponse {}.into(),
                refused_at,
            ))
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_to_inventory(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioWorld>,
    ) -> Result<()> {
        self.insert_to_inventory_timed(
            player_id,
            entity_name,
            entity_position,
            inventory_type,
            item_name,
            item_count,
            world,
        )
        .await
        .map(|_| ())
        .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::insert_to_inventory`], reporting the game tick it ran
    /// at. Synchronous, so both ends of [`ActionTicks`] are that tick.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_to_inventory_timed(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioWorld>,
    ) -> Result<ActionTicks, ActionFailure> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(ActionFailure::not_dispatched(
                RconPlayerNotFound { player_id }.into(),
            ));
        }
        let player = player.unwrap();
        let reach_distance = player.reach_distance as f64;
        let distance = calculate_distance(&player.position, &entity_position);
        drop(player); // wow, without this factorio (?) freezes (!)
        if distance > reach_distance {
            warn!("too far away, moving first!");
            self.move_player(world, player_id, &entity_position, Some(reach_distance))
                .await?;
        }

        let player_id = player_id.to_string();
        let mut items: HashMap<String, String> = HashMap::new();
        items.insert(String::from("name"), str_to_lua(&item_name));
        items.insert(String::from("count"), item_count.to_string());
        let (lines, tick) = self
            .remote_call_timed(
                "insert_to_inventory",
                vec![
                    player_id,
                    str_to_lua(&entity_name),
                    position_to_lua(&entity_position),
                    inventory_type.to_string(),
                    hashmap_to_lua(items),
                ],
            )
            .await?;
        judge_transfer_reply(lines, tick)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn remove_from_inventory(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioWorld>,
    ) -> Result<()> {
        self.remove_from_inventory_timed(
            player_id,
            entity_name,
            entity_position,
            inventory_type,
            item_name,
            item_count,
            world,
        )
        .await
        .map(|_| ())
        .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::remove_from_inventory`], reporting the game tick it ran
    /// at. Synchronous, so both ends of [`ActionTicks`] are that tick.
    #[allow(clippy::too_many_arguments)]
    pub async fn remove_from_inventory_timed(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioWorld>,
    ) -> Result<ActionTicks, ActionFailure> {
        let player = world.players.get(&player_id);
        if player.is_none() {
            return Err(ActionFailure::not_dispatched(
                RconPlayerNotFound { player_id }.into(),
            ));
        }
        let player = player.unwrap();
        let reach_distance = player.reach_distance as f64;
        let distance = calculate_distance(&player.position, &entity_position);
        drop(player); // wow, without this factorio (?) freezes (!)
        if distance > reach_distance {
            warn!("too far away, moving first!");
            self.move_player(world, player_id, &entity_position, Some(reach_distance))
                .await?;
        }
        let player_id = player_id.to_string();
        let mut items: HashMap<String, String> = HashMap::new();
        items.insert(String::from("name"), str_to_lua(&item_name));
        items.insert(String::from("count"), item_count.to_string());
        let (lines, tick) = self
            .remote_call_timed(
                "remove_from_inventory",
                vec![
                    player_id,
                    str_to_lua(&entity_name),
                    position_to_lua(&entity_position),
                    inventory_type.to_string(),
                    hashmap_to_lua(items),
                ],
            )
            .await?;
        judge_transfer_reply(lines, tick)
    }

    pub async fn is_area_empty(&self, area_filter: &AreaFilter) -> Result<bool> {
        let entities = self.find_entities_filtered(area_filter, None, None).await?;
        if !entities.is_empty() {
            return Ok(false);
        }
        let tiles = self.find_tiles_filtered(area_filter, None).await?;
        for tile in tiles {
            if tile.player_collidable {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // https://lua-api.factorio.com/latest/LuaSurface.html#LuaSurface.find_entities_filtered
    /*
       Table with the following fields:
       area :: BoundingBox (optional)
       position :: Position (optional)
       radius :: double (optional): If given with position, will return all entities within the radius of the position.
       name :: string or array of string (optional)
       type :: string or array of string (optional)
       ghost_name :: string or array of string (optional)
       ghost_type :: string or array of string (optional)
       direction :: defines.direction or array of defines.direction (optional)
       collision_mask :: CollisionMaskLayer or array of CollisionMaskLayer (optional)
       force :: ForceSpecification or array of ForceSpecification (optional)
       to_be_upgraded :: boolean (optional)
       limit :: uint (optional)
       invert :: boolean (optional): If the filters should be inverted. These filters are: name, type, ghost_name, ghost_type, direction, collision_mask, force.
    */

    /// `search_type` is a list because the game's own `type` filter is:
    /// passing one type discards everything else at the source, and the game
    /// happily accepts several at once (`type :: string or array of string`).
    /// [`crate::factorio::snapshot::attach_world`] still passes `None` for
    /// this on purpose -- it feeds `EntityGraph::add`, whose `blocked_tree`
    /// keys placement refusals off *every* collidable entity including
    /// trees, so narrowing this query would silently reintroduce furnaces
    /// sited inside forests. A caller that only reads a curated subset back
    /// out (`EntityGraph::snapshot_within`'s whitelist, say) is the one that
    /// should narrow here -- see `keyframe_relevant_types` in
    /// `crates/scripting_lua/src/globals/record.rs`.
    pub async fn find_entities_filtered(
        &self,
        area_filter: &AreaFilter,
        search_name: Option<String>,
        search_type: Option<Vec<String>>,
    ) -> Result<Vec<FactorioEntity>> {
        let mut args: HashMap<String, String> = HashMap::new();
        match area_filter {
            AreaFilter::Rect(area) => {
                args.insert(String::from("area"), rect_to_lua(area));
            }
            AreaFilter::PositionRadius((position, radius)) => {
                args.insert(String::from("position"), position_to_lua(position));
                if let Some(radius) = radius {
                    if radius > &3000.0 {
                        return Err(RconRadiusLimitReached { limit: 3000 }.into());
                    }
                    args.insert(String::from("radius"), radius.to_string());
                }
            }
        }
        if let Some(name) = search_name {
            args.insert(String::from("name"), str_to_lua(&name));
        }
        if let Some(entity_types) = search_type {
            let quoted: Vec<String> = entity_types.iter().map(|t| str_to_lua(t)).collect();
            args.insert(String::from("type"), vec_to_lua(quoted));
        }
        // `remote_call_json` rather than `remote_call`: this reply is
        // unbounded by construction (an area query, not a count), and a
        // short read here previously failed only at `serde_json::from_str`,
        // leaving the connection in the pool still holding the unread
        // remainder -- see `remote_call_json`'s own doc comment for what that
        // costs the *next* command on the same connection.
        let mut json = self
            .remote_call_json("find_entities_filtered", vec![hashmap_to_lua(args)])
            .await?;
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        parse_reply("find_entities_filtered", &json)
    }

    pub async fn parse_map_exchange_string(
        &self,
        name: &str,
        map_exchange_string: &str,
    ) -> Result<()> {
        let result = self
            .remote_call(
                "parse_map_exchange_string",
                vec![str_to_lua(name), str_to_lua(map_exchange_string)],
            )
            .await?;
        if let Some(result) = result {
            return Err(RconError {
                message: result.join("\n"),
            }
            .into());
        }
        Ok(())
    }
    pub async fn find_tiles_filtered(
        &self,
        area_filter: &AreaFilter,
        name: Option<String>,
    ) -> Result<Vec<FactorioTile>> {
        let mut args: HashMap<String, String> = HashMap::new();
        match area_filter {
            AreaFilter::Rect(area) => {
                args.insert(String::from("area"), rect_to_lua(area));
            }
            AreaFilter::PositionRadius((position, radius)) => {
                args.insert(String::from("position"), position_to_lua(position));
                if let Some(radius) = radius {
                    if radius > &3000.0 {
                        return Err(RconRadiusLimitReached { limit: 3000 }.into());
                    }
                    args.insert(String::from("radius"), radius.to_string());
                }
            }
        }
        if let Some(name) = name {
            args.insert(String::from("name"), str_to_lua(&name));
        }
        // See the matching comment on `find_entities_filtered`: this reply is
        // also unbounded by construction, so a short read must be detected
        // rather than merely fail to parse.
        let mut json = self
            .remote_call_json("find_tiles_filtered", vec![hashmap_to_lua(args)])
            .await?;
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        parse_reply("find_tiles_filtered", &json)
    }

    async fn async_request_player_path(
        &self,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<u32> {
        let radius = match radius {
            Some(radius) => radius.to_string(),
            None => String::from("nil"),
        };
        let result = self
            .remote_call(
                "async_request_player_path",
                vec![player_id.to_string(), position_to_lua(goal), radius],
            )
            .await?;
        if result.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let result = result.unwrap().pop().unwrap();
        match result.parse() {
            Ok(result) => Ok(result),
            Err(_) => Err(RconError { message: result }.into()),
        }
    }

    async fn async_request_path(
        &self,
        start: &Position,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<u32> {
        let radius = match radius {
            Some(radius) => radius.to_string(),
            None => String::from("nil"),
        };
        let result = self
            .remote_call(
                "async_request_path",
                vec![position_to_lua(start), position_to_lua(goal), radius],
            )
            .await?;
        if result.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let result = result.unwrap().pop().unwrap();
        match result.parse() {
            Ok(result) => Ok(result),
            Err(_) => Err(RconError { message: result }.into()),
        }
    }

    // https://lua-api.factorio.com/latest/LuaSurface.html#LuaSurface.request_path
    /*
       bounding_box :: BoundingBox
       collision_mask :: CollisionMask or array of string
       start :: Position
       goal :: Position
       force :: LuaForce or string
       radius :: double (optional): How close we need to get to the goal. Default 1.
       pathfind_flags :: PathFindFlags (optional): Flags to affect the pathfinder.
       can_open_gates :: boolean (optional): If the path request can open gates. Default false.
       path_resolution_modifier :: int (optional): The resolution modifier of the pathing. Defaults to 0.
       entity_to_ignore :: LuaEntity (optional): If given, the pathfind will ignore collisions with this entity.
    */
    /// One character path request, put again while the game says its queue was
    /// full.
    ///
    /// **`try again later` is not `failed to path find`.** The first says the
    /// request was never searched, which is worth repeating; the second says it
    /// was searched and there is no way there, which is not. Everything above
    /// this line used to treat both the same.
    async fn player_path_attempt(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<Vec<PathWaypoint>> {
        let mut attempts_left = PATH_REQUEST_BUSY_ATTEMPTS;
        loop {
            let id = self
                .async_request_player_path(player_id, goal, radius)
                .await?;
            match self.sleep_for_path_request_result(world, id).await {
                Err(err) if attempts_left > 1 && is_busy_path_request(&err) => {
                    attempts_left -= 1;
                    warn!(
                        "the pathfinder queue was full for #{}, asking again ({} left)",
                        player_id, attempts_left
                    );
                    sleep(PATH_REQUEST_BUSY_BACKOFF).await;
                }
                other => return other,
            }
        }
    }

    /// [`FactorioRcon::player_path_attempt`] for a path between two positions.
    async fn path_attempt(
        &self,
        world: &Arc<FactorioWorld>,
        start: &Position,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<Vec<PathWaypoint>> {
        let mut attempts_left = PATH_REQUEST_BUSY_ATTEMPTS;
        loop {
            let id = self.async_request_path(start, goal, radius).await?;
            match self.sleep_for_path_request_result(world, id).await {
                Err(err) if attempts_left > 1 && is_busy_path_request(&err) => {
                    attempts_left -= 1;
                    warn!(
                        "the pathfinder queue was full for a path from {}/{}, asking again ({} left)",
                        start.x(),
                        start.y(),
                        attempts_left
                    );
                    sleep(PATH_REQUEST_BUSY_BACKOFF).await;
                }
                other => return other,
            }
        }
    }

    pub async fn player_path(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<Vec<Position>> {
        let context = format!("player_path for #{player_id}");
        match self
            .player_path_attempt(world, player_id, goal, radius)
            .await
        {
            Ok(path) => Ok(waypoint_positions(path, &context)),
            // The offset-goal fallback below answers exactly one question:
            // "this goal cannot be reached, is anywhere near it?". Every other
            // failure -- a full queue, a request the game never took, a reply
            // that never came -- is "we do not know", and substituting a goal
            // on the strength of not knowing is how a walk that was fine ends
            // up refused for falling short somewhere it never asked to be.
            Err(err) if !path_search_found_nothing(&err) => Err(err),
            Err(err) => {
                warn!(
                    "failed to find player_path() for #{} to {}/{}: {:?}",
                    player_id,
                    goal.x(),
                    goal.y(),
                    err
                );
                let Some(player) = world.players.get(&player_id) else {
                    // Nowhere to search *from*. The fallback needs the
                    // player's position to pick a direction, and inventing one
                    // would aim the substituted goal at random.
                    return Err(err);
                };
                let mut direction = vector_normalize(&vector_substract(&player.position, goal));
                drop(player);
                for _ in 0..4 {
                    // direction = goal - player.position
                    // newGoal = goal + direciton.normalize() * radius
                    let new_goal =
                        vector_add(goal, &vector_multiply(&direction, radius.unwrap_or(10.0)));

                    let id = self
                        .async_request_player_path(player_id, &new_goal, radius)
                        .await?;
                    if let Ok(result) = self.sleep_for_path_request_result(world, id).await {
                        return Ok(waypoint_positions(result, &context));
                    }
                    direction = direction.rotate_clockwise();
                }
                Err(err)
            }
        }
    }

    pub async fn path(
        &self,
        world: &Arc<FactorioWorld>,
        start: &Position,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<Vec<Position>> {
        let context = format!("path from {}/{}", start.x(), start.y());
        match self.path_attempt(world, start, goal, radius).await {
            Ok(path) => Ok(waypoint_positions(path, &context)),
            Err(err) if !path_search_found_nothing(&err) => Err(err),
            Err(err) => {
                warn!(
                    "failed to find path() from {}/{} to {}/{}: {:?}",
                    start.x(),
                    start.y(),
                    goal.x(),
                    goal.y(),
                    err
                );
                let mut direction = vector_normalize(&vector_substract(start, goal));
                for _ in 0..4 {
                    // direction = goal - player.position
                    // newGoal = goal + direciton.normalize() * radius
                    let new_goal =
                        vector_add(goal, &vector_multiply(&direction, radius.unwrap_or(10.0)));

                    let id = self.async_request_path(start, &new_goal, radius).await?;
                    if let Ok(result) = self.sleep_for_path_request_result(world, id).await {
                        return Ok(waypoint_positions(result, &context));
                    }
                    direction = direction.rotate_clockwise();
                }
                Err(err)
            }
        }
    }

    pub async fn action_start_walk_waypoints(
        &self,
        action_id: ActionId,
        player_id: PlayerId,
        waypoints: Vec<Position>,
    ) -> Result<Option<u64>> {
        // set_waypoints(action_id, player_id, waypoints)
        let action_id = action_id.to_string();
        let player_id = player_id.to_string();
        let waypoints = waypoints
            .iter()
            .map(position_to_lua)
            .collect::<Vec<String>>()
            .join(", ");
        // The tick stamp is taken off first, so the "any reply at all is an
        // error" rule below still means what it always meant.
        let (result, tick) = self
            .remote_call_timed(
                "action_start_walk_waypoints",
                vec![action_id, player_id, format!("{{ {} }}", waypoints)],
            )
            .await?;
        if let Some(result) = result {
            return Err(RconError {
                message: result.join("\n"),
            }
            .into());
        }
        Ok(tick)
    }

    pub async fn action_start_mining(
        &self,
        action_id: ActionId,
        player_id: PlayerId,
        name: &str,
        position: &Position,
        count: u32,
    ) -> Result<Option<u64>> {
        let action_id = action_id.to_string();
        let player_id = player_id.to_string();
        let (result, tick) = self
            .remote_call_timed(
                "action_start_mining",
                vec![
                    action_id,
                    player_id,
                    str_to_lua(name),
                    position_to_lua(position),
                    count.to_string(),
                ],
            )
            .await?;
        if let Some(result) = result {
            return Err(RconError {
                message: format!("{:?}", result),
            }
            .into());
        }
        Ok(tick)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn plan_path(
        &self,
        world: &Arc<FactorioWorld>,
        entity_name: &str,
        entity_type: &str,
        underground_entity_name: &str,
        underground_entity_type: &str,
        underground_max: u8,
        from_position: &Position,
        to_position: &Position,
        to_direction: Direction,
    ) -> Result<Vec<FactorioEntity>> {
        let build_rect = span_rect(from_position, to_position, 20.0);
        let entities = self
            .find_entities_filtered(&AreaFilter::Rect(build_rect.clone()), None, None)
            .await?;
        let tiles = self
            .find_tiles_filtered(&AreaFilter::Rect(build_rect), Some("water".into()))
            .await?;

        build_entity_path(
            world.entity_prototypes.clone(),
            entity_name,
            entity_type,
            underground_entity_name,
            underground_entity_type,
            underground_max,
            from_position,
            to_position,
            to_direction,
            entities,
            tiles,
        )
    }

    pub async fn find_offshore_pump_placement_options(
        &self,
        world: &Arc<FactorioWorld>,
        search_center: Position,
        pump_direction: Direction,
    ) -> Result<Vec<Pos>> {
        for radius in 3..10 {
            let tiles = self
                .find_tiles_filtered(
                    &AreaFilter::PositionRadius((
                        search_center.clone(),
                        Some((radius * 100) as f64),
                    )),
                    Some("water".into()),
                )
                .await?;
            if tiles.is_empty() {
                continue;
            }
            let mapped = map_blocked_tiles(
                world.entity_prototypes.clone(),
                &vec![],
                &tiles.iter().collect(),
            );
            return Ok(tiles
                .iter()
                .filter(|tile| {
                    let pos = (&tile.position).into();
                    // `pump_direction` is a cardinal, so all three of these
                    // resolve; a half-diagonal names no tile and cannot be
                    // judged, which is not a shoreline we may build on.
                    let (Some(ahead), Some(right), Some(left)) = (
                        move_pos(&pos, pump_direction, 1),
                        move_pos(&pos, pump_direction.clockwise(), 1),
                        move_pos(&pos, pump_direction.clockwise().opposite(), 1),
                    ) else {
                        return false;
                    };
                    if mapped.contains_key(&ahead) {
                        return false;
                    }
                    if !mapped.contains_key(&right) {
                        return false;
                    }
                    if !mapped.contains_key(&left) {
                        return false;
                    }
                    true
                })
                .map(|tile| (&tile.position).into())
                .collect());
        }
        Err(RconNoWaterFound {}.into())
    }
}

unsafe impl Send for FactorioRcon {}

unsafe impl Sync for FactorioRcon {}

pub struct ConnectionManager {
    address: String,
    pass: String,
}

unsafe impl Sync for ConnectionManager {}

impl ConnectionManager {
    pub fn new<S: Into<String>>(address: S, pass: S) -> Self {
        ConnectionManager {
            address: address.into(),
            pass: pass.into(),
        }
    }
}

/// A pooled RCON connection, plus whether it is still known to be in sync with
/// the server.
///
/// `bb8` decides whether to keep a connection by asking
/// [`ConnectionManager::has_broken`], and that answer has to be *stored*
/// somewhere per connection. `rcon::Connection` has no room for it and is not
/// ours to change, so the pool holds this wrapper instead of the bare
/// connection.
pub struct RconConnection {
    conn: Connection<TcpStream>,
    /// Set once this connection may be holding an unread remainder of a reply.
    ///
    /// Never cleared: there is no way to resynchronise a stream whose position
    /// is unknown, because the client cannot tell a leftover tail from the next
    /// legitimate reply. Once set, the connection is dropped rather than
    /// reused.
    desynced: bool,
}

impl RconConnection {
    /// Runs one command, marking the connection desynced if the read fails
    /// part-way.
    pub async fn cmd(&mut self, command: &str) -> rcon::Result<String> {
        let result = self.conn.cmd(command).await;
        if result.is_err() {
            self.desynced = true;
        }
        result
    }

    /// Declares this connection no longer trustworthy — see
    /// [`FactorioRcon::remote_call_json`], the one caller that can detect a
    /// short read.
    pub fn mark_desynced(&mut self) {
        self.desynced = true;
    }
}

impl bb8::ManageConnection for ConnectionManager {
    type Connection = RconConnection;
    type Error = rcon::Error;

    fn connect(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::Connection, Self::Error>> + Send {
        let address = self.address.clone();
        let pass = self.pass.clone();
        async move {
            Ok(RconConnection {
                conn: Connection::builder()
                    .enable_factorio_quirks(true)
                    .connect(&address, &pass)
                    .await?,
                desynced: false,
            })
        }
    }

    /// Checked when the pool hands a connection out. It stays a local test:
    /// the honest liveness check is a round trip to the server, and paying one
    /// per checkout would double the cost of every RCON call to catch a case
    /// `cmd` already surfaces as an error. What it does catch is a desynced
    /// connection that somehow survived [`Self::has_broken`] — belt as well as
    /// braces, since the cost of reusing one is silent cross-talk rather than a
    /// failure.
    async fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        if conn.desynced {
            return Err(rcon::Error::Io(std::io::Error::other(
                "rcon connection desynced by a truncated reply",
            )));
        }
        Ok(())
    }

    /// Asked when a connection is returned to the pool. This used to be a flat
    /// `false`, which meant a connection that had failed mid-reply went back
    /// into rotation still holding the remainder in its socket, and every later
    /// command on it read someone else's tail.
    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.desynced
    }
}

pub struct RconSettings {
    pub port: u16,
    pub pass: String,
    pub host: Option<String>,
}

impl RconSettings {
    pub fn new_from_config(
        settings: &FactorioSettings,
        server_host: Option<String>,
    ) -> RconSettings {
        RconSettings {
            port: settings.rcon_port,
            pass: settings.rcon_pass.to_string(),
            host: server_host,
        }
    }
    pub fn new(rcon_port: u16, rcon_pass: &str, server_host: Option<String>) -> RconSettings {
        RconSettings {
            port: rcon_port,
            pass: rcon_pass.to_owned(),
            host: server_host,
        }
    }
}

/// Regression tests for the "reply arrives, executor never wakes" hang.
///
/// `sleep_for_path_request_result` and `sleep_for_action_result` poll a
/// [`DashMap`](dashmap::DashMap) on the shared [`FactorioWorld`] for a reply the
/// stdout [`crate::process::output_parser::OutputParser`] inserts. They used to
/// do that as `if let Some(x) = map.get(&id) { ...; map.remove(&id); }`, which
/// holds the shard's read guard across a call that wants the same shard's write
/// guard -- a self-deadlock, on the very first tick where the reply is present.
/// It looks exactly like "the reply was received and then nothing happened":
/// the process sleeps, burns no CPU, and never issues another RCON command.
///
/// These tests deliver the reply the way the parser does and require the waiter
/// to come back. They run the waiter on its own thread and wait on a channel,
/// because a deadlocked future can never be cancelled -- `tokio::time::timeout`
/// would hang with it instead of failing the test.
#[cfg(test)]
mod wait_for_reply_tests {
    use super::*;
    use crate::factorio::ticks::ActionOutcome;
    use crate::factorio::world::FactorioWorld;
    use std::sync::mpsc;

    fn quiet_rcon() -> FactorioRcon {
        FactorioRcon::new_empty()
    }

    /// Starts `f` on a dedicated thread and hands back its result channel, so
    /// the test can deliver the reply *while* the waiter is polling. A
    /// `recv_timeout` that expires means the waiter never came back -- for
    /// these tests, that it deadlocked.
    fn start<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx
    }

    const DEADLINE: Duration = Duration::from_secs(20);

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn path_request_reply_wakes_the_waiter() {
        let world = Arc::new(FactorioWorld::new());
        let waiter_world = world.clone();
        let waited =
            start(move || block_on(quiet_rcon().sleep_for_path_request_result(&waiter_world, 1)));
        // The waiter polls every 50ms; deliver the reply the way the output
        // parser does, once it is certainly polling.
        std::thread::sleep(Duration::from_millis(200));
        world.path_requests.insert(
            1,
            r#"[{"x":0.0,"y":0.0},{"x":-15.5,"y":-35.5}]"#.to_string(),
        );

        let waited = waited.recv_timeout(DEADLINE).expect(
            "sleep_for_path_request_result never returned after the reply was delivered \
             (deadlocked holding a DashMap read guard across remove())",
        );
        let path = waited.expect("the delivered path should have parsed");
        assert_eq!(path.len(), 2, "got {path:?}");
        assert!(
            world.path_requests.get(&1).is_none(),
            "the consumed reply should have been removed from the world"
        );
    }

    /// The failure this whole change exists for.
    ///
    /// `on_script_path_request_finished` writes plain text into the same slot a
    /// successful request fills with JSON. Feeding that to `serde_json` gave
    /// `expected value at line 1 column 1`, which names neither the pathfinder
    /// nor its verdict, and is what the 2026-09-02 research run halted on.
    #[test]
    fn a_pathfinder_refusal_is_reported_as_one_and_not_as_a_json_syntax_error() {
        for reason in ["Error: failed to path find", "Error: try again later!"] {
            let world = Arc::new(FactorioWorld::new());
            let waiter_world = world.clone();
            let waited = start(move || {
                block_on(quiet_rcon().sleep_for_path_request_result(&waiter_world, 3))
            });
            std::thread::sleep(Duration::from_millis(200));
            world.path_requests.insert(3, reason.to_string());

            let err = waited
                .recv_timeout(DEADLINE)
                .expect("the waiter never returned")
                .expect_err("a pathfinder refusal is not a path");
            let text = err.to_string();
            assert!(
                text.contains(reason),
                "the mod's own words must survive; got {text:?}"
            );
            assert!(
                !text.contains("expected value at line 1"),
                "a pathfinder verdict must not be reported as a JSON syntax error; got {text:?}"
            );
        }
    }

    /// And a reply that really is malformed JSON says what arrived.
    #[test]
    fn a_malformed_path_reply_quotes_what_it_received() {
        let world = Arc::new(FactorioWorld::new());
        let waiter_world = world.clone();
        let waited =
            start(move || block_on(quiet_rcon().sleep_for_path_request_result(&waiter_world, 4)));
        std::thread::sleep(Duration::from_millis(200));
        world.path_requests.insert(4, "[{\"x\":0.0,".to_string());

        let err = waited
            .recv_timeout(DEADLINE)
            .expect("the waiter never returned")
            .expect_err("a truncated document is not a path");
        let text = err.to_string();
        assert!(
            text.contains("async_request_path"),
            "the call has to name itself; got {text:?}"
        );
        assert!(
            text.contains("x\":0.0,"),
            "the offending text has to be quoted back; got {text:?}"
        );
    }

    #[test]
    fn action_result_reply_wakes_the_waiter() {
        let world = Arc::new(FactorioWorld::new());
        let waiter_world = world.clone();
        let waited = start(move || {
            block_on(quiet_rcon().sleep_for_action_result(&waiter_world, 7, Some(4200)))
        });
        std::thread::sleep(Duration::from_millis(200));
        world.actions.insert(
            7,
            ActionOutcome {
                tick: 4242,
                result: "ok".to_string(),
            },
        );

        let ticks = waited
            .recv_timeout(DEADLINE)
            .expect(
                "sleep_for_action_result never returned after the reply was delivered \
                 (deadlocked holding a DashMap read guard across remove())",
            )
            .expect("an \"ok\" action result should succeed");
        assert_eq!(
            ticks.replied,
            Some(4242),
            "the completion tick must be the game's, not a plan value"
        );
        assert_eq!(
            ticks.dispatched,
            Some(4200),
            "the dispatch stamp must be carried through, not recomputed"
        );
        assert!(
            world.actions.get(&7).is_none(),
            "the consumed reply should have been removed from the world"
        );
    }

    #[test]
    fn failed_action_result_is_reported_and_consumed() {
        let world = Arc::new(FactorioWorld::new());
        let waiter_world = world.clone();
        let waited = start(move || {
            block_on(quiet_rcon().sleep_for_action_result(&waiter_world, 8, Some(4200)))
        });
        std::thread::sleep(Duration::from_millis(200));
        world.actions.insert(
            8,
            ActionOutcome {
                tick: 11,
                result: "target is out of reach".to_string(),
            },
        );

        let err = waited
            .recv_timeout(DEADLINE)
            .expect("sleep_for_action_result never returned after the failure was delivered")
            .expect_err("a non-ok action result should be an error");
        assert!(
            format!("{err:?}").contains("target is out of reach"),
            "error should carry the game's message, got {err:?}"
        );
        assert_eq!(
            err.dispatch,
            Dispatch::Refused,
            "the game judged this one; that is a verdict, not a lost action"
        );
        assert_eq!(
            err.ticks,
            ActionTicks::new(Some(4200), Some(11)),
            "a refused dispatch keeps both stamps the game produced"
        );
        // Action ids are reused (mod 1000). A failure left behind in the map
        // makes the next action with that id fail instantly.
        assert!(
            world.actions.get(&8).is_none(),
            "a failed reply must also be consumed, or it poisons the reused action id"
        );
    }
}

/// What a failed dispatch is allowed to claim.
///
/// Two facts, and for each of them both directions, because replacing one false
/// statement with its mirror image is not progress:
///
/// - a dispatch the game stamped and then failed keeps the stamp, **and** one
///   that failed before any stamp still reports none;
/// - a timeout *after* the game acknowledged the command is
///   [`Dispatch::NoVerdict`], **and** a timeout before it is not.
#[cfg(test)]
mod dispatch_evidence_tests {
    use super::*;
    use crate::factorio::ticks::ActionOutcome;
    use crate::factorio::world::FactorioWorld;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Fact 2, the direction that would overclaim. `RconTimeout` is also what
    /// a path request produces, and a path request runs before anything is
    /// dispatched -- so the *same error* must come out `NotDispatched` when it
    /// is raised in that phase. This is the conversion `?` uses at every
    /// pre-dispatch call site, exercised with the very error that makes the
    /// distinction load-bearing.
    #[test]
    fn a_timeout_raised_before_the_dispatch_claims_no_dispatch_and_no_tick() {
        let failure: ActionFailure = Report::from(RconTimeout {}).into();
        assert_eq!(
            failure.dispatch,
            Dispatch::NotDispatched,
            "a timeout is not evidence of a dispatch; a path request times out too"
        );
        assert_eq!(
            failure.ticks,
            ActionTicks::UNKNOWN,
            "nothing stamped this, so nothing may be reported"
        );
    }

    /// Fact 1, the direction that would fabricate: a failure raised before the
    /// game saw anything must report no tick, through the real production path
    /// rather than a hand-built value. A poolless [`FactorioRcon::new_empty`]
    /// fails inside `player_path`, which is exactly the pre-dispatch phase.
    #[test]
    fn move_player_timed_reports_nothing_when_it_fails_before_dispatching() {
        let world = Arc::new(FactorioWorld::new());
        let failure = block_on(FactorioRcon::new_empty().move_player_timed(
            &world,
            1,
            &Position::new(10.0, 10.0),
            None,
        ))
        .expect_err("a disconnected rcon cannot request a path");
        assert_eq!(
            failure.ticks,
            ActionTicks::UNKNOWN,
            "the game never saw this, so there is no measurement to report"
        );
        assert_eq!(failure.ticks.dispatched, None, "absent is not tick zero");
        assert_eq!(
            failure.dispatch,
            Dispatch::NotDispatched,
            "nothing was dispatched, so nothing can be outstanding"
        );
    }

    /// Fact 2, the direction that needs the new state. The wait is entered only
    /// after `action_start_*` returned, so running out of patience here means
    /// the game took the command and never answered -- the one case that may
    /// say an action is still out there.
    #[test]
    fn a_dispatched_action_that_never_answers_has_no_verdict() {
        let world = Arc::new(FactorioWorld::new());
        let failure = block_on(FactorioRcon::new_empty().sleep_for_action_result_until(
            &world,
            3,
            Some(7_777),
            Duration::from_millis(120),
        ))
        .expect_err("no reply was ever delivered");
        assert_eq!(
            failure.dispatch,
            Dispatch::NoVerdict,
            "the game acknowledged this command and then went quiet"
        );
        assert_eq!(
            failure.ticks.dispatched,
            Some(7_777),
            "the dispatch stamp is the evidence that there is an action to have lost"
        );
        assert_eq!(
            failure.ticks.replied, None,
            "nothing replied, so no reply tick may be invented"
        );
    }

    /// The case that rules out using `ticks.dispatched.is_some()` as the test
    /// for "was it dispatched". A stamp the mod wrote but nothing could parse
    /// leaves the tick absent while the dispatch still happened, and the two
    /// facts are recorded separately precisely so this comes out right.
    #[test]
    fn a_dispatch_the_game_did_not_stamp_is_still_a_dispatch() {
        let world = Arc::new(FactorioWorld::new());
        let failure = block_on(FactorioRcon::new_empty().sleep_for_action_result_until(
            &world,
            4,
            None,
            Duration::from_millis(120),
        ))
        .expect_err("no reply was ever delivered");
        assert_eq!(
            failure.dispatch,
            Dispatch::NoVerdict,
            "an unparseable stamp does not un-dispatch the command"
        );
        assert_eq!(failure.ticks, ActionTicks::UNKNOWN);
    }

    /// Fact 1, the direction that used to drop a measurement: the game stamped
    /// the dispatch, then reported failure, and both numbers survive the error
    /// path.
    #[test]
    fn a_refused_dispatch_keeps_the_stamps_the_game_produced() {
        let world = Arc::new(FactorioWorld::new());
        world.actions.insert(
            5,
            ActionOutcome {
                tick: 4_270,
                result: "out of reach".to_string(),
            },
        );
        let failure = block_on(FactorioRcon::new_empty().sleep_for_action_result_until(
            &world,
            5,
            Some(4_211),
            Duration::from_secs(5),
        ))
        .expect_err("a non-ok outcome is a failure");
        assert_eq!(
            failure.ticks,
            ActionTicks::new(Some(4_211), Some(4_270)),
            "a failed dispatch carries the same measurement a successful one would"
        );
        assert_eq!(
            failure.dispatch,
            Dispatch::Refused,
            "the game gave a verdict; nothing is outstanding"
        );
    }
}

/// Positioning: what a walk and a mine are allowed to claim.
///
/// Every number in here is a measurement from the live
/// `goal.have("iron-plate", 10)` run recorded in
/// `.superpowers/sdd/2026-08-30-goal-values/live-smelt-run.md`, read off the
/// mod's own `on_script_path_request_finished` and
/// `on_player_changed_position` writeouts. Both faults and both non-faults of
/// that run are here, so each guard is pinned from both sides by the same
/// afternoon's evidence rather than by invented geometry.
#[cfg(test)]
mod positioning_tests {
    use super::*;

    /// Walk s0 of the run. Goal `(-21, 37)`, no radius; the pathfinder's answer
    /// ended at `(-20.5, 37.5)`, 0.707 tiles away, and the bot really did
    /// arrive. **The direction that must keep working:** a tile-centre
    /// endpoint beside a non-tile-centre goal is arrival, not a shortfall.
    #[test]
    fn a_path_that_ends_beside_the_goal_has_arrived() {
        let goal = Position::new(-21.0, 37.0);
        let end = Position::new(-20.5, 37.5);
        assert!(
            walk_arrives(&goal, None, &end),
            "0.707 tiles is the tile-centre quantisation of the goal, not a failure to arrive"
        );
    }

    /// Walk s2 of the run, the widest legitimate endpoint with no radius:
    /// goal `(-71.5, -36.5)`, path ended at `(-70.5, -36.5)`, exactly 1.0 tiles
    /// away — Factorio's own default radius. It must not be rejected.
    #[test]
    fn a_path_that_stops_at_factorios_default_radius_has_arrived() {
        assert!(
            walk_arrives(
                &Position::new(-71.5, -36.5),
                None,
                &Position::new(-70.5, -36.5)
            ),
            "`None` asks for request_path's default radius of 1, so ending 1.0 away is arrival"
        );
    }

    /// The mine's pre-walk: goal `(-22.5, 36.5)` with an explicit radius of 3,
    /// path ended at `(-24.5, 34.5)`, 2.828 tiles away — inside the radius that
    /// was asked for. A radius the caller supplied must widen the tolerance.
    #[test]
    fn an_explicit_radius_widens_what_counts_as_arrival() {
        let goal = Position::new(-22.5, 36.5);
        let end = Position::new(-24.5, 34.5);
        assert!(
            walk_arrives(&goal, Some(3.0), &end),
            "2.828 is inside the radius of 3 the caller asked for"
        );
        assert!(
            !walk_arrives(&goal, None, &end),
            "the very same endpoint is a shortfall when the caller asked for the default radius"
        );
    }

    /// **Fault 1, the direction that used to lie.** Walk s4 asked for
    /// `(-21, 37)` — the tile the bot's own furnace had just been placed on.
    /// The pathfinder answered `Error: failed to path find`, `player_path`
    /// silently retried against a goal offset by 10 tiles, and the path it
    /// returned ends at `(-26.5, 29.5)`: **9.30 tiles** from where the plan put
    /// the bot. That walk was reported as a success.
    #[test]
    fn the_substituted_goal_of_the_live_run_is_not_an_arrival() {
        let goal = Position::new(-21.0, 37.0);
        let end = Position::new(-26.5, 29.5);
        assert!(
            (calculate_distance(&end, &goal) - 9.3).abs() < 0.01,
            "the run's own numbers: 9.30 tiles short"
        );
        assert!(
            !walk_arrives(&goal, None, &end),
            "a path to a synthesised goal 10 tiles away is not a path to the goal"
        );
    }

    /// An empty path is not a shortfall. The pathfinder returns nothing when
    /// there is nowhere to go and the mod completes such a walk on the next
    /// tick without moving, so the honest end position is where the player
    /// already stands.
    #[test]
    fn an_empty_path_ends_where_the_player_already_is() {
        let here = Position::new(3.0, 4.0);
        assert_eq!(walk_end_position(&[], Some(&here)), Some(&here));
        assert!(walk_arrives(&Position::new(3.2, 4.1), None, &here));
        assert!(!walk_arrives(&Position::new(30.0, 40.0), None, &here));
        assert_eq!(
            walk_end_position(&[], None),
            None,
            "with no waypoints and no known position there is nothing to check against"
        );
    }

    /// The last waypoint is the end of the walk, not the first or the nearest.
    #[test]
    fn the_end_of_a_path_is_its_last_waypoint() {
        let path = vec![
            Position::new(0.0, 0.0),
            Position::new(5.5, 5.5),
            Position::new(-20.5, 37.5),
        ];
        let here = Position::new(0.0, 0.0);
        assert_eq!(
            walk_end_position(&path, Some(&here)),
            Some(&Position::new(-20.5, 37.5)),
            "a known path overrides the player's current position"
        );
    }

    /// **Fault 2, the direction that used to hang.** After its one corrective
    /// walk the bot came to rest at `(-24.902344, 34.171875)` with the ore at
    /// `(-22.5, 36.5)`: 3.345 tiles, against a `resource_reach_distance` of 3.
    /// The mine was dispatched anyway and the game refused it silently for
    /// 21,400 ticks.
    #[test]
    fn the_live_runs_mine_was_dispatched_from_outside_reach() {
        let player = Position::new(-24.90234375, 34.171875);
        let ore = Position::new(-22.5, 36.5);
        assert!(
            (calculate_distance(&player, &ore) - 3.345).abs() < 0.001,
            "the run's own numbers: 3.345 tiles"
        );
        assert!(
            !within_resource_reach(&player, &ore, 3.0),
            "0.345 tiles outside the reach the mod reported"
        );
        assert!(
            !within_resource_reach(&player, &ore, 2.7),
            "and 0.645 outside the 2.7 a real character actually has"
        );
    }

    /// **The direction that must keep working.** The run's *other* mine — one
    /// coal at `(-71.5, -36.5)`, dispatched with the bot at
    /// `(-70.222656, -36.277344)`, 1.297 tiles away — succeeded in the game and
    /// must still be dispatched. Without this half the fix is just "never
    /// mine".
    #[test]
    fn the_live_runs_successful_mine_is_still_within_reach() {
        let player = Position::new(-70.22265625, -36.27734375);
        let coal = Position::new(-71.5, -36.5);
        assert!(
            (calculate_distance(&player, &coal) - 1.297).abs() < 0.001,
            "the run's own numbers: 1.297 tiles"
        );
        assert!(
            within_resource_reach(&player, &coal, 3.0),
            "this mine ran and returned one coal; it must still be dispatched"
        );
        assert!(
            within_resource_reach(&player, &coal, 2.7),
            "and it is inside a real character's 2.7 as well"
        );
    }

    /// The reach is whatever the game says it is — never a hard-coded 3. A
    /// player with no character reports `f64::MAX`, and nothing is out of that
    /// player's reach.
    #[test]
    fn reach_is_read_from_the_player_and_not_assumed() {
        let player = Position::new(0.0, 0.0);
        let far = Position::new(1_000.0, 1_000.0);
        assert!(!within_resource_reach(&player, &far, 3.0));
        assert!(
            within_resource_reach(&player, &far, f64::MAX),
            "a player with no character has unbounded resource reach"
        );
        // The boundary itself is inclusive: the game's check is `<=`.
        assert!(within_resource_reach(
            &player,
            &Position::new(2.7, 0.0),
            2.7
        ));
        assert!(!within_resource_reach(
            &player,
            &Position::new(2.7001, 0.0),
            2.7
        ));
    }

    /// A mine's corrective walk must aim comfortably *inside* the reach, since
    /// where a walk is aimed and where it comes to rest differ by roughly a
    /// tile. Aiming at the reach itself is what put the run 0.345 outside it.
    #[test]
    fn a_mines_corrective_walk_aims_inside_the_reach() {
        for reach in [2.7_f64, 3.0, 4.0, 10.0] {
            let radius = approach_radius(reach);
            assert!(
                radius < reach,
                "aiming at the reach itself is the fault; got {radius} for reach {reach}"
            );
            assert!(
                radius + 1.2 <= reach || reach < 2.4,
                "the walk must leave room for the ~1.1 tiles between a path's end and \
                 where the follower stops; got {radius} for reach {reach}"
            );
        }
        assert!(
            approach_radius(0.1) >= 0.5,
            "a radius that collapses onto the resource's own tile is the request shape \
             that makes the pathfinder fail outright"
        );
        assert!(
            approach_radius(f64::MAX).is_finite(),
            "an uncharactered player's unbounded reach must not become an infinite radius"
        );
    }

    /// The property a plan's walk needs, which the mine's needs did not cover.
    ///
    /// A `StepKind::Walk` names the thing to get near — for a place, insert or
    /// remove that is the entity's own tile — and a bound of `build_distance`
    /// or `reach_distance`, both 10 in the game's defaults. The request must
    /// never be for the tile itself, whatever the bound: a radius of zero is
    /// what makes `request_path` fail outright on an occupied goal, which is
    /// the whole fault this exists to prevent.
    #[test]
    fn an_approach_never_asks_for_the_targets_own_tile() {
        for bound in [0.0_f64, 0.4, 1.0, 2.7, 10.0] {
            let radius = approach_radius(bound);
            assert!(
                radius >= 0.5,
                "a request of {radius} for bound {bound} collapses onto the goal tile"
            );
        }
        // And the halving is a halving, not a floor that swallows every bound:
        // the game's default build and reach distance is 10, and asking for 0.5
        // there would walk the bot onto the furnace just as surely.
        assert_eq!(approach_radius(10.0), 5.0);
    }

    /// The wording `mine_reports_target_gone` matches is the mod's, not ours.
    ///
    /// This is a cross-language contract with nothing but a string on either
    /// side, so the mod's own source is read rather than described: if
    /// `control.lua`'s mining watchdog is reworded, this fails here -- loudly,
    /// and in the crate that depends on it -- instead of the retirement quietly
    /// never firing again and mined-out tiles staying in the model for another
    /// run. `repo_mods_path!` is the same checkout the drift check compares a
    /// workspace against, so the guard and the run cannot end up talking about
    /// different files.
    #[test]
    fn a_vanished_mine_target_is_recognised_from_the_mods_own_wording() {
        use crate::process::instance_setup::repo_mods_path;
        const CONTROL_LUA: &str = include_str!(repo_mods_path!("/BotBridge/control.lua"));
        assert!(
            CONTROL_LUA.contains(MINE_TARGET_GONE),
            "the mod no longer says {MINE_TARGET_GONE:?}, so nothing retires a mined-out tile"
        );

        // And the phrase survives the wrapping `sleep_for_action_result` puts
        // a refusal through, which is the only form the caller ever sees.
        let verdict = "ERROR: the target iron-ore was gone before mining finished -- \
                       something else mined it first";
        let refusal: miette::Report = RconError {
            message: verdict.to_string(),
        }
        .into();
        assert!(
            mine_reports_target_gone(&refusal.to_string()),
            "the mod's own verdict must be recognised, got: {refusal}"
        );

        // The negative control: an out-of-reach refusal must not retire a tile
        // that is still full of ore.
        let out_of_reach: miette::Report = RconOutOfResourceReach {
            target_x: -40.5,
            target_y: -48.5,
            distance: 3.345,
            reach: 2.7,
        }
        .into();
        assert!(!mine_reports_target_gone(&out_of_reach.to_string()));
    }

    /// **The pathfinder's two failures are not the same failure**, and the
    /// wording that tells them apart is the mod's, not ours.
    ///
    /// `try again later` means the request queue was full and the search never
    /// happened; `failed to path find` means it searched and found nothing.
    /// Read out of `control.lua` for the same reason the mining verdict is: a
    /// cross-language contract with nothing but a string on either side, and a
    /// reword that went unnoticed would turn every busy pathfinder into an
    /// unreachable goal.
    #[test]
    fn a_busy_pathfinder_is_recognised_from_the_mods_own_wording() {
        use crate::process::instance_setup::repo_mods_path;
        const CONTROL_LUA: &str = include_str!(repo_mods_path!("/BotBridge/control.lua"));
        assert!(
            CONTROL_LUA.contains(PATHFINDER_BUSY),
            "the mod no longer says {PATHFINDER_BUSY:?}, so a full queue now reads \
             as an unreachable goal"
        );
        assert!(path_request_was_busy("Error: try again later!"));
        assert!(
            !path_request_was_busy("Error: failed to path find"),
            "a search that found nothing is an answer, not a full queue"
        );
    }

    /// And the same distinction survives the error type it travels in, which is
    /// the only form `player_path` ever sees it in.
    #[test]
    fn a_full_queue_and_a_missing_path_are_told_apart_as_errors() {
        let busy: Report = RconPathRequestFailed {
            reason: "Error: try again later!".to_string(),
        }
        .into();
        let missing: Report = RconPathRequestFailed {
            reason: "Error: failed to path find".to_string(),
        }
        .into();
        assert!(is_busy_path_request(&busy));
        assert!(
            !is_busy_path_request(&missing),
            "substituting a different goal for this one would answer a question \
             nobody asked"
        );
        // A failure that is not a path failure at all -- a timeout, say -- is
        // not a full queue either, and must not be retried as one.
        let timeout: Report = RconTimeout {}.into();
        assert!(!is_busy_path_request(&timeout));

        // And the offset-goal fallback is reserved for the one failure that is
        // actually an answer. Letting a full queue or a silence through it
        // hands the caller a path to a goal it never asked for -- the same
        // reported-arrival shape the stuck-walk teleport had.
        assert!(path_search_found_nothing(&missing));
        assert!(!path_search_found_nothing(&busy));
        assert!(
            !path_search_found_nothing(&timeout),
            "a reply that never came is not a search that found nothing"
        );
    }
}

/// The guarantee a green transfer row rests on, driven through the real mod.
///
/// # Why this test loads `control.lua` instead of describing what it does
///
/// The property under test — *`Success` on an `insert` or a `remove` means the
/// items moved* — is not implemented anywhere. It is what happens when two
/// independent mechanisms line up: the mod's `complain` writes into the RCON
/// **reply body** (it is `rcon.print`, not just `print`), and
/// [`judge_transfer_reply`] treats any surviving line in that body as a
/// verdict of failure. Neither half knows about the other, so a test that
/// hand-writes the reply body it expects the mod to produce would keep passing
/// after the mod stopped producing it — it would be testing its own fixture.
///
/// So the mod's own `control.lua` is loaded into a real Lua 5.4 interpreter
/// against a stubbed Factorio API, the real handler is called on an inventory
/// that yields fewer items than asked, and whatever it prints to the RCON
/// interface is fed through the real [`split_reply`] → [`take_tick_stamp`] →
/// [`judge_transfer_reply`] chain. The only invented part is the game itself.
///
/// What that still cannot see is listed in
/// `.superpowers/sdd/2026-08-30-goal-values/transfer-guarantee.md`. The one
/// that matters is that this reads the *checkout's*
/// `mods/BotBridge/control.lua`, while a run loads `workspace/mods`, a copy
/// that wins over the checkout once it exists and is never refreshed. A run
/// whose copy has drifted is running other code, and no amount of green here
/// would say otherwise.
///
/// That gap is not closed here, because it cannot be: whether a particular
/// machine's `workspace/mods` has drifted is a fact about that machine, not
/// about this source tree, and CI has no workspace at all. Asserting it as a
/// test would either fail on a fresh checkout or pass vacuously. So it is
/// reported where it exists instead -- at run time, on the "Using mods
/// directory" line, which now carries the verdict of comparing the copy in
/// use against this checkout (see `process::instance_setup`, and
/// `asset_sync::warn_if_stale` for the release build's equivalent against the
/// embedded snapshot). The bytes below and the directory that check compares
/// against both come from `repo_mods_path!`, so the guard and the run-time
/// report cannot end up talking about different files.
/// What an unparseable reply says about itself.
#[cfg(test)]
mod reply_snippet_tests {
    use super::*;

    #[test]
    fn a_short_reply_is_quoted_whole() {
        assert_eq!(
            reply_snippet("Cannot execute command. Error: boom"),
            "\"Cannot execute command. Error: boom\""
        );
    }

    #[test]
    fn a_long_reply_is_elided_rather_than_logged_whole() {
        let huge = "x".repeat(REPLY_SNIPPET_LIMIT * 10);
        let snippet = reply_snippet(&huge);
        assert!(snippet.ends_with("\"..."), "{snippet}");
        assert!(
            snippet.chars().count() < REPLY_SNIPPET_LIMIT + 10,
            "a snippet is a snippet: {} chars",
            snippet.chars().count()
        );
    }

    /// Truncation is by `char`, not by byte: a game's text may be multi-byte,
    /// and slicing mid-codepoint would panic on the reporting path itself.
    #[test]
    fn a_multibyte_reply_does_not_panic_on_truncation() {
        let huge = "§".repeat(REPLY_SNIPPET_LIMIT * 2);
        let snippet = reply_snippet(&huge);
        assert!(snippet.starts_with("\"§"), "{snippet}");
    }

    /// The whole point: the message names the call and quotes the payload.
    #[test]
    fn an_unparseable_reply_names_its_call_and_quotes_what_arrived() {
        let err = parse_reply::<Vec<Position>>("find_entities_filtered", "Error: no such function")
            .expect_err("plain text is not JSON");
        let text = err.to_string();
        assert!(text.contains("find_entities_filtered"), "{text}");
        assert!(text.contains("Error: no such function"), "{text}");
        assert!(
            text.contains("expected value at line 1 column 1"),
            "serde's own message stays, it is just no longer all there is: {text}"
        );
    }
}

/// What the pre-flight check writes into the refusal ledger, and what it
/// deliberately refuses to write.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod placement_precheck_tests {
    use super::*;

    fn query(item: &str, x: f64, y: f64) -> PlacementQuery {
        PlacementQuery {
            player_id: 1,
            item_name: item.to_string(),
            position: Position::new(x, y),
            direction: 0,
        }
    }

    fn verdict(ok: bool, character: bool, error: Option<&str>) -> PlacementVerdict {
        PlacementVerdict {
            ok,
            character,
            blockers: if ok {
                Vec::new()
            } else {
                vec!["tree-01".into()]
            },
            tile: if ok { None } else { Some("grass-1".into()) },
            error: error.map(str::to_string),
        }
    }

    /// The whole filter, in one batch: four sites, exactly one of which is a
    /// fact about the ground.
    ///
    /// The three that are not are not near-misses -- they are the three ways
    /// a `false` can arrive without meaning "this ground is unbuildable", and
    /// each was reasoned about separately. A green is not a refusal; a
    /// character is transient and is already modelled from the world on every
    /// plan; and a question the mod could not ask is not an answer.
    #[test]
    fn only_a_refusal_with_no_character_and_no_error_reaches_the_ledger() {
        let world = Arc::new(FactorioWorld::new());
        let queries = vec![
            query("stone-furnace", 1., 1.),
            query("stone-furnace", 2., 2.),
            query("stone-furnace", 3., 3.),
            query("stone-furnace", 4., 4.),
        ];
        let reply = PlacementVerdicts {
            tick: Some(6198),
            sites: vec![
                verdict(true, false, None),
                verdict(false, true, None),
                verdict(false, false, Some("item 'x' has no place_result")),
                verdict(false, false, None),
            ],
        };
        let verdicts = accept_verdicts(&world, &queries, reply).expect("the lengths agree");
        assert_eq!(
            verdicts.len(),
            4,
            "every verdict is handed back to the caller"
        );

        let learned = world.placement_refusals();
        assert_eq!(
            learned.len(),
            1,
            "only the fourth site is a fact about the ground: {learned:?}"
        );
        assert_eq!(learned[0].position, Position::new(4., 4.));
        assert_eq!(learned[0].tick, Some(6198));
        assert_eq!(learned[0].source, RefusalSource::PreCheck);
        assert_eq!(learned[0].blockers, vec!["tree-01".to_string()]);
        assert_eq!(learned[0].tile.as_deref(), Some("grass-1"));
    }

    /// A reply that cannot be joined to its queries is refused whole.
    ///
    /// Zipping to the shorter of the two would attach a verdict to the wrong
    /// query, and a refusal is never expired -- so a mis-join would exclude
    /// ground nobody refused for the rest of the run. Nothing is recorded.
    #[test]
    fn a_reply_of_the_wrong_length_is_rejected_and_records_nothing() {
        let world = Arc::new(FactorioWorld::new());
        let queries = vec![
            query("stone-furnace", 1., 1.),
            query("stone-furnace", 2., 2.),
        ];
        let reply = PlacementVerdicts {
            tick: Some(6198),
            sites: vec![verdict(false, false, None)],
        };
        let err = accept_verdicts(&world, &queries, reply).expect_err("one verdict for two sites");
        let text = format!("{err:?}");
        assert!(text.contains('2') && text.contains('1'), "{text}");
        assert!(
            world.placement_refusals().is_empty(),
            "a reply that cannot be joined teaches nothing at all"
        );
    }

    /// A pre-check refusal and a dispatch refusal at the same site are one
    /// entry, and the first one recorded is the one kept.
    #[test]
    fn a_site_already_in_the_ledger_is_not_recorded_twice() {
        let world = Arc::new(FactorioWorld::new());
        let queries = vec![query("stone-furnace", 1., 1.)];
        let reply = PlacementVerdicts {
            tick: Some(10),
            sites: vec![verdict(false, false, None)],
        };
        accept_verdicts(&world, &queries, reply).expect("recorded");
        world.record_placement_refusal(PlacementRefusal::at_dispatch(
            Some(20),
            "stone-furnace",
            Position::new(1., 1.),
        ));
        let learned = world.placement_refusals();
        assert_eq!(learned.len(), 1);
        assert_eq!(
            learned[0].source,
            RefusalSource::PreCheck,
            "the first recording wins, which is the one carrying the evidence"
        );
    }

    /// The one warning line has to say what the game found -- and has to
    /// distinguish "nothing was in the footprint" from "we did not look".
    #[test]
    fn the_cause_reads_differently_when_nothing_was_in_the_footprint() {
        assert_eq!(
            describe_blockers(&["tree-01".into(), "cliff".into()], Some("grass-1")),
            "blocked by tree-01, cliff; tile grass-1"
        );
        assert_eq!(
            describe_blockers(&[], Some("water")),
            "no entity in the footprint; tile water",
            "an empty list on a pre-check refusal points at the ground itself"
        );
        assert_eq!(describe_blockers(&[], None), "no entity in the footprint");
    }
}

#[cfg(test)]
mod game_tick_query_tests {
    use super::*;
    use crate::factorio::ticks::{TICK_STAMP_PREFIX, take_tick_stamp};

    /// The query has to be readable by the one parser that reads ticks, and it
    /// has to be answerable without BotBridge -- otherwise a save whose
    /// `workspace/mods` copy predates this binary silently loses its clock and
    /// every lag wait quietly reverts to the wall-clock guess this replaced.
    #[test]
    fn the_query_is_vanilla_lua_stamped_in_the_format_take_tick_stamp_reads() {
        assert!(GAME_TICK_QUERY.starts_with("/silent-command "));
        assert!(GAME_TICK_QUERY.contains("game.tick"));
        assert!(GAME_TICK_QUERY.contains(TICK_STAMP_PREFIX));
        assert!(
            !GAME_TICK_QUERY.contains("remote.call"),
            "the point is that this needs no mod"
        );
        assert!(!GAME_TICK_QUERY.contains('\n'));
    }

    /// A round trip: the reply the query's own Lua would print must come back
    /// out of the parser as that tick and no payload. Written against the
    /// format rather than against a live game, which is the half that can
    /// break silently.
    #[test]
    fn the_reply_the_query_produces_parses_back_to_the_tick() {
        let printed = format!("{TICK_STAMP_PREFIX}64738\n");
        let (payload, tick) = take_tick_stamp(split_reply(&printed, true));
        assert_eq!(tick, Some(64738));
        assert_eq!(
            payload, None,
            "the stamp is the whole reply; anything left over would be judged as an error"
        );
    }
}

#[cfg(test)]
mod transfer_guarantee_tests {
    use super::*;
    use crate::factorio::ticks::take_tick_stamp;
    use mlua::{Lua, LuaOptions, StdLib};

    use crate::process::instance_setup::repo_mods_path;

    // Derived from the same compile-time constant the run-time drift check
    // uses, so the bytes this test compiled in and the directory a run is
    // compared against cannot come apart -- see `repo_mods_path`.
    const CONTROL_LUA: &str = include_str!(repo_mods_path!("/BotBridge/control.lua"));
    const TYPES_LUA: &str = include_str!(repo_mods_path!("/BotBridge/types.lua"));

    /// The tick the stub game is frozen at. Any value works; a recognisable one
    /// makes a failure message readable.
    const STUB_TICK: u64 = 64738;

    /// Enough of Factorio's Lua API for `control.lua` to load and for the two
    /// transfer handlers to run. Everything here is a stub *except* the two
    /// numbers the handlers do arithmetic on: `_held`, what the player has, and
    /// `_moves`, what the target inventory will actually accept or give up.
    fn stub_game(held: i64, moves: i64) -> String {
        format!(
            r#"
            -- `defines.events.on_tick` and friends are read at load time; any
            -- distinct value will do, so grow them on demand.
            local function auto()
                local t = {{}}
                setmetatable(t, {{ __index = function(tbl, k)
                    local v = auto(); rawset(tbl, k, v); return v
                end }})
                return t
            end
            defines = auto()
            local function noop() end
            local function nooptable()
                return setmetatable({{}}, {{ __index = function() return noop end }})
            end
            script = nooptable()
            remote = nooptable()
            commands = nooptable()
            helpers = nooptable()
            require = function() return {{}} end
            print = noop

            -- The RCON reply body under construction. `complain` and
            -- `stamp_tick` both land here, which is the whole point.
            _rcon_lines = {{}}
            rcon = {{ print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }}

            local held = {held}
            local moves = {moves}
            local inventory = {{
                -- What the furnace hands over (remove) or takes (insert).
                remove = function(items) return moves end,
                insert = function(items) return moves end,
            }}
            local entity = {{ get_inventory = function(t) return inventory end }}
            local player = {{
                surface = {{ find_entity = function(name, pos) return entity end }},
                get_item_count = function(name) return held end,
                -- The player end of the move always cooperates, so a complaint
                -- can only come from the count arithmetic itself.
                insert = function(items) return items.count end,
                remove_item = function(items) return items.count end,
            }}
            game = {{
                tick = {tick},
                players = {{ player }},
                forces = {{ player = {{ print = noop }} }},
            }}
        "#,
            held = held,
            moves = moves,
            tick = STUB_TICK,
        )
    }

    /// Runs one real transfer handler and returns the verdict the production
    /// chain reaches for the reply it produced.
    ///
    /// `held` is what the bot carries, `moves` is what the target inventory
    /// really accepts or yields, and `asked` is the count in the command.
    /// Loads `stub`, then the repo's own `types.lua` and `control.lua`, runs
    /// `call`, and hands back the RCON reply body the mod printed.
    fn lua_for_mod_source() -> Lua {
        // The workspace forbids building an interpreter outside
        // `scripting_lua::sandbox`, and rightly: that one runs *user* scripts.
        // This one runs a single file from this repository, `control.lua`, with
        // no path by which a caller could substitute another, and
        // `scripting_lua` depends on this crate so the sandbox cannot be
        // reached from here. The library set is the sandbox's minus
        // `coroutine`, so this is not a widening of what a Lua chunk can do.
        #[allow(
            clippy::disallowed_methods,
            reason = "test-only interpreter for the repo's own mod source; the \
                      sandbox lives in a crate that depends on this one"
        )]
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )
        .expect("test interpreter");
        lua
    }

    fn run_handler(stub: String, call: &str) -> Vec<String> {
        let lua = lua_for_mod_source();
        lua.load(stub)
            .set_name("stub_game")
            .exec()
            .expect("stub game");
        lua.load(TYPES_LUA)
            .set_name("types.lua")
            .exec()
            .expect("mod types.lua");
        lua.load(CONTROL_LUA)
            .set_name("control.lua")
            .exec()
            .expect("mod control.lua");
        lua.load(call)
            .set_name("call")
            .exec()
            .expect("handler call");

        lua.globals()
            .get::<mlua::Table>("_rcon_lines")
            .expect("_rcon_lines")
            .sequence_values::<String>()
            .map(|v| v.expect("rcon line"))
            .collect()
    }

    /// The reply body an RCON server would hand back for `printed`: the lines
    /// plus the trailing newline `split_reply` is written to strip.
    fn reply_body(printed: &[String]) -> String {
        if printed.is_empty() {
            String::new()
        } else {
            printed.join("\n") + "\n"
        }
    }

    fn transfer(call: &str, held: i64, moves: i64) -> (Result<ActionTicks, ActionFailure>, String) {
        let printed = run_handler(stub_game(held, moves), call);
        let body = reply_body(&printed);
        let (lines, tick) = take_tick_stamp(split_reply(&body, true));
        (judge_transfer_reply(lines, tick), printed.join("\n"))
    }

    /// Enough of the API for `rcon_place_entity` to reach its refusal branches.
    /// `can_place` decides which one: `false` with the player inside the
    /// footprint is the `§player_blocks_placement§` case, and `held` at zero is
    /// the "does not have any" case.
    fn stub_place(can_place: bool, held: i64) -> String {
        format!(
            r#"
            local function auto()
                local t = {{}}
                setmetatable(t, {{ __index = function(tbl, k)
                    local v = auto(); rawset(tbl, k, v); return v
                end }})
                return t
            end
            defines = auto()
            local function noop() end
            local function nooptable()
                return setmetatable({{}}, {{ __index = function() return noop end }})
            end
            script = nooptable()
            remote = nooptable()
            commands = nooptable()
            helpers = nooptable()
            require = function() return {{}} end
            print = noop

            _rcon_lines = {{}}
            rcon = {{ print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }}

            local surface = {{
                can_place_entity = function(args) return {can_place} end,
                create_entity = function(args) return nil end,
                find_entity = function(name, pos) return nil end,
            }}
            local player = {{
                name = "bot1",
                -- Standing dead centre of the tile it is about to build on:
                -- the live 2026-09-02 case.
                position = {{ x = 38.3046875, y = 16.4765625 }},
                force = "player",
                surface = surface,
                get_item_count = function(name) return {held} end,
                remove_item = function(items) return items.count end,
            }}
            prototypes = {{ item = {{
                ["stone-furnace"] = {{ place_result = {{
                    name = "stone-furnace",
                    collision_box = {{
                        left_top = {{ x = -0.9, y = -0.9 }},
                        right_bottom = {{ x = 0.9, y = 0.9 }},
                    }},
                }} }},
            }} }}
            game = {{
                tick = {tick},
                players = {{ player }},
                forces = {{ player = {{ print = noop }} }},
            }}
        "#,
            can_place = if can_place { "true" } else { "false" },
            held = held,
            tick = STUB_TICK,
        )
    }

    const PLACE_FURNACE: &str = r#"rcon_place_entity(1, "stone-furnace", {38, 16}, 0)"#;

    /// A refused placement is still a placement the game *judged*, so it has to
    /// carry the tick it judged it at.
    ///
    /// Without the stamp the executor records the failure with no ticks at all,
    /// and `record.actions` writes no `action_dispatched` and no
    /// `action_settled` for it -- which is why the 2026-09-02 run's stuck
    /// milestone has a 95-step plan, an error, and not one event naming the
    /// step that produced it.
    #[test]
    fn a_refused_placement_still_stamps_the_tick_it_was_refused_at() {
        for (can_place, held, expected) in [
            (false, 1, "§player_blocks_placement§"),
            (true, 0, "does not have any"),
        ] {
            let printed = run_handler(stub_place(can_place, held), PLACE_FURNACE);
            let body = reply_body(&printed);
            let (lines, tick) = take_tick_stamp(split_reply(&body, true));
            assert_eq!(
                tick,
                Some(STUB_TICK),
                "a refusal the game reached must carry its tick; it printed {printed:?}"
            );
            let lines = lines.expect("the refusal itself must survive the stamp being taken off");
            assert_eq!(
                lines.len(),
                1,
                "`place_entity_timed` reads a one-line reply; got {lines:?}"
            );
            assert!(
                lines[0].contains(expected),
                "expected {expected:?} in {lines:?}"
            );
        }
    }

    /// Enough of the API for `rcon_can_place_entities` to run, plus a record
    /// of **every argument table `can_place_entity` was asked with**.
    ///
    /// `_asked` is the point of this stub. The pre-flight check is only worth
    /// anything if it asks the game the same question the real placement
    /// asks, and the two traps there are silent: `build_check_type` defaults
    /// to `ghost_revive` rather than `manual` (a ghost check validates far
    /// less than it looks like it does -- the same family of mistake as
    /// `only_ghosts = true` on a blueprint), and `force` defaults to
    /// `"neutral"`. A pre-check that fell into either would hand back a green
    /// that means nothing.
    ///
    /// `entities` is what `find_entities_filtered` reports inside the
    /// footprint, as `(name, type)` pairs.
    fn stub_can_place(can_place: bool, entities: &[(&str, &str)], tile: &str) -> String {
        let found: String = entities
            .iter()
            .map(|(name, kind)| format!("{{ name = \"{name}\", type = \"{kind}\" }}, "))
            .collect();
        format!(
            r#"
            local function auto()
                local t = {{}}
                setmetatable(t, {{ __index = function(tbl, k)
                    local v = auto(); rawset(tbl, k, v); return v
                end }})
                return t
            end
            defines = auto()
            defines.build_check_type.manual = "MANUAL"
            defines.build_check_type.ghost_revive = "GHOST_REVIVE"
            local function noop() end
            local function nooptable()
                return setmetatable({{}}, {{ __index = function() return noop end }})
            end
            script = nooptable()
            remote = nooptable()
            commands = nooptable()
            require = function() return {{}} end
            print = noop

            _rcon_lines = {{}}
            rcon = {{ print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end }}
            -- Every argument table `can_place_entity` is asked with, in order.
            _asked = {{}}

            -- Enough of a json writer for the assertions below: the reply is
            -- only ever booleans, numbers, strings and arrays of strings.
            helpers = setmetatable(
                {{ table_to_json = function(v) return _json(v) end }},
                {{ __index = function() return noop end }})
            function _json(v)
                local t = type(v)
                if t == "nil" then return "null" end
                if t == "boolean" or t == "number" then return tostring(v) end
                if t == "string" then return '"' .. v .. '"' end
                if #v > 0 then
                    local parts = {{}}
                    for _, item in ipairs(v) do parts[#parts + 1] = _json(item) end
                    return "[" .. table.concat(parts, ",") .. "]"
                end
                local keys = {{}}
                for k in pairs(v) do keys[#keys + 1] = k end
                table.sort(keys)
                local parts = {{}}
                for _, k in ipairs(keys) do
                    parts[#parts + 1] = '"' .. k .. '":' .. _json(v[k])
                end
                return "{{" .. table.concat(parts, ",") .. "}}"
            end

            local found = {{ {found} }}
            local surface = {{
                can_place_entity = function(args)
                    _asked[#_asked + 1] = args
                    return {can_place}
                end,
                create_entity = function(args) return nil end,
                find_entity = function(name, pos) return nil end,
                find_entities_filtered = function(args) return found end,
                get_tile = function(x, y) return {{ valid = true, name = "{tile}" }} end,
            }}
            local player = {{
                name = "bot1",
                position = {{ x = 0.5, y = 0.5 }},
                force = "player",
                surface = surface,
                get_item_count = function(name) return 1 end,
                remove_item = function(items) return items.count end,
            }}
            prototypes = {{ item = {{
                ["stone-furnace"] = {{ place_result = {{
                    name = "stone-furnace",
                    collision_box = {{
                        left_top = {{ x = -0.7, y = -0.7 }},
                        right_bottom = {{ x = 0.7, y = 0.7 }},
                    }},
                }} }},
            }} }}
            game = {{
                tick = {tick},
                players = {{ player }},
                forces = {{ player = {{ print = noop }} }},
            }}
        "#,
            can_place = if can_place { "true" } else { "false" },
            found = found,
            tile = tile,
            tick = STUB_TICK,
        )
    }

    const CHECK_FURNACE: &str = r#"rcon_can_place_entities({
        { player = 1, item = "stone-furnace", position = {38, 16}, direction = 0 } })"#;

    /// Runs the real handler and parses its reply the way production does.
    fn check(can_place: bool, entities: &[(&str, &str)], tile: &str) -> PlacementVerdicts {
        let printed = run_handler(stub_can_place(can_place, entities, tile), CHECK_FURNACE);
        assert_eq!(printed.len(), 1, "one json document, got {printed:?}");
        serde_json::from_str(&printed[0])
            .unwrap_or_else(|err| panic!("the reply must parse: {err} in {:?}", printed[0]))
    }

    /// **The question has to be the same question.**
    ///
    /// A pre-check asking a *different* `can_place_entity` than the real
    /// placement is worse than no pre-check, because its green would be
    /// believed. Both call sites go through `placement_check_args`, and this
    /// drives both handlers against the same stub and compares the argument
    /// tables the game was actually handed.
    ///
    /// The `build_check_type` assertion is not decoration. Verified against
    /// `workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17,
    /// runtime api 6): the parameter is optional and **defaults to
    /// `ghost_revive`**, so omitting it asks about reviving a ghost rather
    /// than about a player building by hand. That is the same trap as
    /// `only_ghosts = true` — ghosts do not collide, so the check validates
    /// far less than it appears to.
    #[test]
    fn the_pre_check_asks_can_place_entity_exactly_what_a_real_placement_asks() {
        fn asked(call: &str) -> Vec<(String, String, String, String)> {
            let lua = lua_for_mod_source();
            lua.load(stub_can_place(true, &[], "grass-1"))
                .set_name("stub_game")
                .exec()
                .expect("stub game");
            lua.load(TYPES_LUA)
                .set_name("types.lua")
                .exec()
                .expect("mod types.lua");
            lua.load(CONTROL_LUA)
                .set_name("control.lua")
                .exec()
                .expect("mod control.lua");
            lua.load(call)
                .set_name("call")
                .exec()
                .expect("handler call");
            lua.globals()
                .get::<mlua::Table>("_asked")
                .expect("_asked")
                .sequence_values::<mlua::Table>()
                .map(|args| {
                    let args = args.expect("an argument table");
                    (
                        args.get::<String>("name").expect("name"),
                        format!(
                            "{:?}",
                            args.get::<mlua::Value>("position").expect("position")
                        ),
                        args.get::<String>("force").expect("force"),
                        args.get::<String>("build_check_type").expect(
                            "build_check_type is not optional here: the game's default \
                                     is ghost_revive, which is a different question",
                        ),
                    )
                })
                .collect()
        }

        let placed = asked(PLACE_FURNACE);
        let checked = asked(CHECK_FURNACE);
        assert_eq!(placed.len(), 1, "the placement asks exactly once");
        assert_eq!(checked.len(), 1, "the pre-check asks exactly once");
        assert_eq!(
            placed[0].0, checked[0].0,
            "both must name the same entity prototype"
        );
        assert_eq!(
            placed[0].2, checked[0].2,
            "both must ask as the acting player's force, not the default neutral"
        );
        assert_eq!(
            placed[0].3, "MANUAL",
            "the real placement must ask the manual build check"
        );
        assert_eq!(
            checked[0].3, "MANUAL",
            "and so must the pre-check: the game's default is ghost_revive, which \
             validates far less than it looks like it does"
        );
    }

    /// A site the game allows comes back green and says nothing else.
    #[test]
    fn an_allowed_site_is_green_and_carries_no_cause() {
        let reply = check(true, &[("tree-01", "tree")], "grass-1");
        assert_eq!(reply.tick, Some(STUB_TICK));
        assert_eq!(reply.sites.len(), 1);
        let verdict = &reply.sites[0];
        assert!(verdict.ok);
        assert!(!verdict.is_durable_refusal(), "green is not a refusal");
        assert!(
            verdict.blockers.is_empty() && verdict.tile.is_none(),
            "nothing is looked up for a site that is fine: {verdict:?}"
        );
    }

    /// **The answer five runs did not have.** A refused site names what is
    /// standing in the footprint the game just tested, and the tile under it.
    #[test]
    fn a_refused_site_names_what_is_in_the_way() {
        let reply = check(
            false,
            &[("tree-02", "tree"), ("tree-01", "tree")],
            "grass-3",
        );
        let verdict = &reply.sites[0];
        assert!(!verdict.ok);
        assert!(verdict.is_durable_refusal());
        assert!(!verdict.character, "no character was in the box");
        assert_eq!(
            verdict.blockers,
            vec!["tree-01".to_string(), "tree-02".to_string()],
            "distinct names, sorted, so two runs report the same thing"
        );
        assert_eq!(verdict.tile.as_deref(), Some("grass-3"));
    }

    /// The one blocker that must **not** be learned as a fact about the
    /// ground.
    ///
    /// A character moves on its own, `PlanState::from_world` re-reads every
    /// character from the world on every plan, and at pre-check time the
    /// acting bot has not walked to the site yet -- so a character in the
    /// footprint now says nothing about whether the site is buildable when
    /// the plan gets there. The mod makes the same distinction at dispatch
    /// with `§player_blocks_placement§`; this widens it from the acting
    /// player to every character, for the reason above.
    #[test]
    fn a_character_in_the_footprint_is_reported_but_is_not_a_durable_refusal() {
        let reply = check(false, &[("character", "character")], "grass-1");
        let verdict = &reply.sites[0];
        assert!(!verdict.ok, "the game did refuse it");
        assert!(verdict.character);
        assert!(
            !verdict.is_durable_refusal(),
            "a bot standing there is not a fact about the ground"
        );
    }

    /// An item that cannot be built at all is reported as an error, not as a
    /// refusal: nothing was learned about the ground.
    #[test]
    fn an_item_with_no_place_result_is_an_error_not_a_refusal() {
        let printed = run_handler(
            stub_can_place(true, &[], "grass-1"),
            r#"rcon_can_place_entities({
                { player = 1, item = "iron-plate", position = {38, 16}, direction = 0 } })"#,
        );
        let reply: PlacementVerdicts =
            serde_json::from_str(&printed[0]).expect("the reply must parse");
        let verdict = &reply.sites[0];
        assert!(!verdict.ok);
        assert!(verdict.error.is_some(), "{verdict:?}");
        assert!(
            !verdict.is_durable_refusal(),
            "a question the mod could not ask is not an answer about the ground"
        );
    }

    /// The batch is answered in the order it was asked, which is the only
    /// thing the join by index can rest on.
    #[test]
    fn a_batch_comes_back_in_the_order_it_was_asked() {
        let printed = run_handler(
            stub_can_place(true, &[], "grass-1"),
            r#"rcon_can_place_entities({
                { player = 1, item = "stone-furnace", position = {1, 1}, direction = 0 },
                { player = 9, item = "stone-furnace", position = {2, 2}, direction = 0 },
                { player = 1, item = "stone-furnace", position = {3, 3}, direction = 0 } })"#,
        );
        let reply: PlacementVerdicts =
            serde_json::from_str(&printed[0]).expect("the reply must parse");
        assert_eq!(reply.sites.len(), 3, "one verdict per query");
        assert!(reply.sites[0].ok);
        assert!(
            reply.sites[1].error.is_some(),
            "player 9 does not exist in the stub, and the slot still has to be filled \
             or every later verdict joins to the wrong query"
        );
        assert!(reply.sites[2].ok);
    }

    const REMOVE_TEN: &str = r#"rcon_remove_from_inventory(
        1, "stone-furnace", {x=-21.0, y=37.0}, 5, {name="iron-plate", count=10})"#;
    const INSERT_TEN: &str = r#"rcon_insert_to_inventory(
        1, "stone-furnace", {x=-21.0, y=37.0}, 2, {name="iron-ore", count=10})"#;

    /// **The guarantee.** A `remove` that moved nothing must not come back
    /// green.
    ///
    /// This is the reading a replay view depends on: a green transfer row says
    /// items moved, not merely that the game did not refuse the command. The
    /// furnace here is asked for 10 plates and yields 0 — the exact shape of
    /// "the smelt had not finished yet" — and the run must call that `Failed`.
    #[test]
    fn a_remove_that_moved_nothing_is_a_failure() {
        let (verdict, printed) = transfer(REMOVE_TEN, 0, 0);
        let failure = verdict.expect_err(
            "a remove that moved 0 of 10 reported success -- \
             every green transfer row in the replay is now unfalsifiable",
        );
        assert!(
            matches!(failure.dispatch, Dispatch::Refused),
            "the game judged this one, so it is a refusal rather than an undelivered \
             dispatch: got {:?}",
            failure.dispatch
        );
        // The tick survives the failure: the game stamped it before complaining.
        assert_eq!(failure.ticks, ActionTicks::at(Some(STUB_TICK)));
        // And the reason has to be the shortfall itself. Without this the test
        // would still pass if the handler failed for some unrelated reason --
        // a missing entity, a stub that did not load -- which is exactly the
        // false green this exists to prevent.
        assert!(
            printed.contains("but removed 0"),
            "the mod must complain about the shortfall *into the reply body*; \
             it printed {printed:?}"
        );
    }

    /// The same for the other half of the pair, on its clamp path.
    ///
    /// An `insert` of items the bot does not hold clamps the count to zero and
    /// moves nothing. The complaint is emitted *before* the clamp, so this
    /// still fails rather than passing silently.
    #[test]
    fn an_insert_that_moved_nothing_is_a_failure() {
        let (verdict, printed) = transfer(INSERT_TEN, 0, 0);
        let failure = verdict.expect_err("an insert that moved 0 of 10 reported success");
        assert!(matches!(failure.dispatch, Dispatch::Refused));
        assert!(
            printed.contains("only has 0"),
            "the clamp must complain into the reply body; it printed {printed:?}"
        );
    }

    /// A partial move is a failure too — `Success` asserts the *full* count.
    ///
    /// The complaint is asserted **whole**, not by fragment, because a second
    /// reader now depends on its shape: `classify_failure`
    /// (`crates/scripting_lua/src/globals/record.rs`) parses both counts and
    /// the item name out of this exact wording to build a
    /// `FailureKind::PartialTransfer`, so that a run's record says *18 of 20
    /// moved* rather than only *rejected*. Those two live in different crates
    /// and cannot check each other; this assertion is the pin. If it fails
    /// because the mod's wording moved, the classifier's own tests are the
    /// other half to update.
    #[test]
    fn a_remove_that_moved_some_but_not_all_is_a_failure() {
        let (verdict, printed) = transfer(REMOVE_TEN, 0, 7);
        verdict.expect_err("a remove that moved 7 of 10 reported success");
        assert_eq!(
            printed,
            format!("tried to remove 10 iron-plate but removed 7\n§tick§{STUB_TICK}"),
            "the shortfall wording is parsed by `classify_failure` in \
             crates/scripting_lua; both counts and the item must stay where it \
             looks for them"
        );
    }

    /// The insert side's clamp wording, pinned for the same reader and the
    /// same reason. It states the two counts differently — `20x` rather than
    /// `20`, and the moved count after `only has` — which is exactly why the
    /// classifier parses three shapes rather than one.
    #[test]
    fn a_clamped_insert_states_both_counts_in_the_wording_the_record_parses() {
        let (verdict, printed) = transfer(INSERT_TEN, 6, 6);
        verdict.expect_err("an insert clamped from 10 to 6 reported success");
        assert_eq!(
            printed,
            format!(
                "cannot insert 10x iron-ore, because player #1 only has 6. clamping...\n\
                 §tick§{STUB_TICK}"
            ),
            "the clamp wording is parsed by `classify_failure` in \
             crates/scripting_lua"
        );
    }

    /// The discriminator. Without this the three tests above would all pass
    /// against a mod that complained about everything, or a judgement that
    /// refused every transfer.
    #[test]
    fn a_transfer_that_moved_everything_asked_for_succeeds() {
        for (call, held, moves) in [(REMOVE_TEN, 0, 10), (INSERT_TEN, 10, 10)] {
            let (verdict, printed) = transfer(call, held, moves);
            assert_eq!(
                printed,
                format!("§tick§{STUB_TICK}"),
                "a complete transfer prints its stamp and nothing else"
            );
            let ticks = verdict.expect("a complete transfer must succeed");
            assert_eq!(ticks, ActionTicks::at(Some(STUB_TICK)));
        }
    }

    /// An action the mod fails must say something. `tostring(nil)` is "nil",
    /// and "nil" is what the executor renders as the game's whole verdict --
    /// `game rejected the command: Unexpected Response: nil`, which is what a
    /// 2026-09-02 run reported for a mine two bots raced for.
    ///
    /// `print` is redirected into the same buffer the RCON stub uses, because
    /// `writeout` -- the stdout channel `action_failed` writes on -- is `print`.
    #[test]
    fn a_failed_action_always_carries_words() {
        let printed = run_handler(
            stub_place(true, 1),
            r#"
            print = function(s) _rcon_lines[#_rcon_lines + 1] = tostring(s) end
            action_failed(64738, 7)
            action_failed(64738, 8, "ERROR: too far too mine")
            "#,
        );
        assert_eq!(printed.len(), 2, "got {printed:?}");
        assert!(
            !printed[0].ends_with(" nil"),
            "a missing reason must not reach the executor as the word \"nil\"; got {printed:?}"
        );
        assert!(printed[0].contains("without saying why"), "got {printed:?}");
        assert!(
            printed[1].ends_with("fail 8 ERROR: too far too mine"),
            "a real reason must survive unchanged; got {printed:?}"
        );
    }

    /// A run id is opaque by contract, so the quoting has to survive a value
    /// this side did not choose. Without escaping, a `'` closes the literal
    /// and everything after it is read as Lua by the game.
    #[test]
    fn a_run_id_containing_a_quote_stays_one_lua_string() {
        assert_eq!(lua_string_literal("job-7"), "'job-7'");
        assert_eq!(
            lua_string_literal("a'); game.print('pwned"),
            "'a\\'); game.print(\\'pwned'"
        );
        assert_eq!(lua_string_literal("back\\slash"), "'back\\\\slash'");
    }

    /// RCON commands are newline-delimited: an unescaped newline would end the
    /// command and leave the remainder to be run as the next one.
    #[test]
    fn a_run_id_containing_a_newline_does_not_end_the_command() {
        let literal = lua_string_literal("one\ntwo\r");
        assert!(!literal.contains('\n'), "{literal:?} still holds a newline");
        assert!(!literal.contains('\r'), "{literal:?} still holds a return");
        assert_eq!(literal, "'one\\010two\\013'");
    }
}

/// Which refusals are remembered, and which are deliberately not.
///
/// The mod answers a failed `can_place_entity` two different ways, and the
/// difference is the whole basis for the belief horizon: the
/// `player_blocks_placement` sentinel names its cause and is already handled
/// by walking the actor aside and retrying, while the generic wording names
/// nothing, which is exactly why it has to be remembered rather than
/// explained away. `place_entity_timed` needs a live connection, so these
/// drive the discriminator directly with the lines the game really sends.
#[cfg(test)]
mod placement_refusal_tests {
    use super::*;
    use crate::factorio::world::FactorioWorld;

    fn world() -> Arc<FactorioWorld> {
        Arc::new(FactorioWorld::new())
    }

    /// The line four runs died on.
    const GENERIC: &str =
        "cannot place item 'stone-furnace' because surface.can_place_entity said 'no'";

    #[test]
    fn the_generic_refusal_is_remembered_with_its_site() {
        let world = world();
        let at = Position::new(-16., -58.);
        note_placement_refusal(&world, Some(6198), GENERIC, "stone-furnace", &at);
        let refusals = world.placement_refusals();
        assert_eq!(refusals.len(), 1, "got {refusals:?}");
        assert_eq!(refusals[0].entity, "stone-furnace");
        assert_eq!(refusals[0].position, at);
        assert_eq!(refusals[0].tick, Some(6198));
    }

    /// The other half of the same branch. A player-blocked refusal is about a
    /// character that will move; remembering it would fence the planner out
    /// of ground with nothing wrong with it.
    #[test]
    fn a_player_blocked_refusal_is_not_remembered() {
        let world = world();
        note_placement_refusal(
            &world,
            Some(1),
            "§player_blocks_placement§",
            "stone-furnace",
            &Position::new(-16., -58.),
        );
        assert!(
            world.placement_refusals().is_empty(),
            "the mod named the cause and the RCON layer retries it; that is \
             not a fact about the ground"
        );
    }

    /// Nor is every other complaint that arrives on the same arm. An empty
    /// hand and a nil place_result are refusals of the *command*, not of the
    /// site, and neither says anything about the tile.
    #[test]
    fn refusals_that_are_not_about_the_site_are_not_remembered() {
        let world = world();
        for line in [
            "cannot place item 'stone-furnace' because the player 'bot' does not have any",
            "cannot place item 'stone-furnace' because place_result is nil",
            "ERROR: something else entirely",
        ] {
            note_placement_refusal(&world, None, line, "stone-furnace", &Position::new(0., 0.));
        }
        assert!(
            world.placement_refusals().is_empty(),
            "only `can_place_entity said 'no'` is a verdict about the ground"
        );
    }

    /// A run that is refused the same site three times learns it once, and a
    /// record is told about it once — while the planner keeps seeing it on
    /// every plan, which is the difference between this ledger and the
    /// teleport queue beside it.
    #[test]
    fn a_repeated_refusal_is_reported_once_and_kept_forever() {
        let world = world();
        let at = Position::new(-16., -58.);
        for tick in [6198, 6204, 6209] {
            note_placement_refusal(&world, Some(tick), GENERIC, "stone-furnace", &at);
        }
        assert_eq!(world.placement_refusals().len(), 1);
        assert_eq!(world.unreported_placement_refusals().len(), 1);
        assert!(
            world.unreported_placement_refusals().is_empty(),
            "a second flush writes nothing"
        );
        assert_eq!(
            world.placement_refusals().len(),
            1,
            "reporting must not take the site away from the planner"
        );
    }
}

/// **Screenshots are retired, and the wire has to say so by saying nothing.**
///
/// The cost of getting the default wrong here is asymmetric: a run that
/// captures when it should not writes 947 MB and takes the render inside the
/// game loop, while a run that does not capture when it should loses pictures
/// nobody had asked for. So the tests below pin the cheap outcome as the one
/// an unconfigured call produces, at the last point before the command leaves
/// this process.
#[cfg(test)]
mod frame_camera_tests {
    use super::{FrameCameras, frame_capture_args};

    /// A call that says nothing about cameras must send exactly the argument
    /// list it always sent -- one run id and no more -- so the mod's own
    /// default is what decides, and there is only one place to get it wrong.
    #[test]
    fn the_default_asks_for_nothing_and_adds_nothing_to_the_wire() {
        assert_eq!(FrameCameras::default(), FrameCameras::None);
        assert_eq!(
            frame_capture_args(Some("run-1"), &FrameCameras::None),
            vec!["'run-1'".to_string()]
        );
    }

    #[test]
    fn every_camera_is_the_literal_true_the_mod_reads() {
        assert_eq!(
            frame_capture_args(Some("run-1"), &FrameCameras::All),
            vec!["'run-1'".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn a_subset_is_a_lua_list_of_quoted_ids() {
        assert_eq!(
            frame_capture_args(
                Some("run-1"),
                &FrameCameras::Only(vec!["follow".into(), "bot-1".into()])
            ),
            vec!["'run-1'".to_string(), "{'follow','bot-1'}".to_string()]
        );
    }

    /// The positional hazard. Without the placeholder the camera list lands in
    /// `run_id`, where the mod raises "run id must be a string" -- a refusal
    /// naming the wrong argument, for a call that was correct.
    #[test]
    fn an_untagged_capture_with_cameras_keeps_the_run_id_slot_open() {
        assert_eq!(
            frame_capture_args(None, &FrameCameras::All),
            vec!["nil".to_string(), "true".to_string()]
        );
        assert!(
            frame_capture_args(None, &FrameCameras::None).is_empty(),
            "and an untagged capture with no cameras still sends no argument \
             at all -- the mod distinguishes an absent id from any value"
        );
    }
}

/// A walk must not be dispatched at a destination nobody can stand on — and
/// must still be dispatched everywhere we cannot prove that.
///
/// The data is run 30's, `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`:
/// three failed walks, three destinations, each inside a stone furnace the same
/// run had built, with the collision boxes read from this crate's own prototype
/// fixtures — which carry the game's real `0.19921875` character and
/// `0.69921875` stone furnace half-extents, so the arithmetic in these tests is
/// the arithmetic the run did.
///
/// Half the module is about the other direction. This guard sits on the path of
/// **every** walk, so each refusing test is paired with one that pins what must
/// not be refused: a graph that has seen nothing, a footprint that merely
/// touches, a world with no character prototype.
#[cfg(test)]
mod walk_destination_tests {
    use super::*;
    use crate::test_utils::fixture_entity_prototypes;
    use crate::types::FactorioEntityPrototype;

    /// A world holding the fixture prototypes and whatever entities the test
    /// gives it, which is what `FactorioWorld::new` plus
    /// `update_chunk_entities` gets us: `entity_prototypes` is shared with the
    /// entity graph, so the furnaces below get the game's own collision box.
    fn world_with(furnaces: &[Position]) -> Arc<FactorioWorld> {
        let world = FactorioWorld::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        let entities = furnaces
            .iter()
            .map(|position| {
                FactorioEntity::from_prototype(
                    "stone-furnace",
                    position.clone(),
                    None,
                    None,
                    None,
                    world.entity_prototypes.clone(),
                )
                .expect("the fixture has a stone-furnace prototype")
            })
            .collect();
        world.update_chunk_entities(entities).unwrap();
        Arc::new(world)
    }

    fn refusal(result: Result<(), ActionFailure>) -> ActionFailure {
        result.expect_err("this destination is inside a furnace, so the walk cannot arrive")
    }

    /// Run 30's walk 72, tick 81,661, verbatim. The mine's corrective walk
    /// toward the copper at `(-23.5, 18.5)` got a path ending 1.2309 tiles
    /// short of it — comfortably inside the 2.35 tolerance
    /// `approach_radius(2.7)` implies, so the arrival check passes and passed
    /// then — but that endpoint is inside the stone furnace the same run built
    /// at `(-22, 18)` at tick 33,342.
    #[test]
    fn run_30_walk_72_is_refused_before_dispatch_and_names_the_furnace() {
        let world = world_with(&[Position::new(-22., 18.)]);
        let goal = Position::new(-23.5, 18.5);
        let radius = Some(approach_radius(2.7));
        let end = Position::new(-22.30078125, 18.22265625);

        assert!(
            walk_arrives(&goal, radius, &end),
            "the arrival check passes this path -- that is why it was dispatched"
        );

        let failure = refusal(judge_path(&world, &goal, radius, &[end], None));
        let message = format!("{:?}", failure.error);
        assert!(
            message.contains("stone-furnace") && message.contains("[-22, 18]"),
            "the refusal must name the obstruction and where it is, not blame \
             the terrain: {message}"
        );
        assert!(
            message.contains("-22.3") && message.contains("18.22"),
            "and name the destination it refused: {message}"
        );
        assert_eq!(
            failure.dispatch,
            Dispatch::NotDispatched,
            "the walk never happened, so nothing is outstanding"
        );
        assert_eq!(
            failure.ticks,
            ActionTicks::UNKNOWN,
            "the game never saw this, so there is no measurement to report"
        );
    }

    /// Run 30's walk 96, tick 89,002. Its furnace at `(-22, 24)` was placed at
    /// tick 87,493, 1,509 ticks before the walk that ended inside it: the graph
    /// knew, and nobody asked.
    #[test]
    fn run_30_walk_96_is_refused_before_dispatch() {
        let world = world_with(&[Position::new(-22., 24.)]);
        let goal = Position::new(-22.5, 22.5);
        let end = Position::new(-22.2890625, 23.3359375);

        assert!(
            walk_arrives(&goal, None, &end),
            "this path arrives, by the only check that used to exist"
        );
        let message = format!(
            "{:?}",
            refusal(judge_path(&world, &goal, None, &[end], None)).error
        );
        assert!(
            message.contains("stone-furnace") && message.contains("[-22, 24]"),
            "{message}"
        );
    }

    /// Run 30's walk 179, tick 165,964 — the cleanest of the three. Its goal is
    /// the furnace's *own position*, which is what a `take from the furnace`
    /// step's `AtPosition` target is, so the endpoint 0.707 tiles away is both
    /// a perfectly good arrival and a place no character can be.
    #[test]
    fn run_30_walk_179_is_refused_even_though_it_lands_where_it_was_asked_to() {
        let world = world_with(&[Position::new(-19., 20.)]);
        let goal = Position::new(-19., 20.);
        let end = Position::new(-19.5, 19.5);

        assert!(walk_arrives(&goal, None, &end), "0.707 is well inside 2.0");
        let message = format!(
            "{:?}",
            refusal(judge_path(&world, &goal, None, &[end], None)).error
        );
        assert!(
            message.contains("stone-furnace") && message.contains("[-19, 20]"),
            "{message}"
        );
    }

    /// The regression that would matter. `blocking_boxes_within` is an
    /// **in-bounds** oracle: an empty answer means "nothing I have seen is
    /// there", never "the ground is clear". Run 30's own walk 72 destination,
    /// against a graph that has not been told about the furnace, must be walked
    /// -- refusing here would ground every bot on ground we have not surveyed,
    /// which is worse than the stall this guard prevents.
    #[test]
    fn a_destination_the_graph_has_never_seen_is_walked() {
        let world = world_with(&[]);
        let end = Position::new(-22.30078125, 18.22265625);
        assert_eq!(
            standing_verdict(&world, &end),
            StandingVerdict::NotProvablyBlocked,
            "an unseen obstruction is not an observed one"
        );
        assert!(
            judge_path(
                &world,
                &Position::new(-23.5, 18.5),
                Some(approach_radius(2.7)),
                &[end],
                None
            )
            .is_ok(),
            "cannot tell must not refuse"
        );
    }

    /// The false positive that would matter next: a destination beside the
    /// furnace, on the tile centre the pathfinder actually aims at, with the
    /// furnace right there in the graph.
    #[test]
    fn a_destination_next_to_a_known_furnace_is_walked() {
        let world = world_with(&[Position::new(-22., 18.)]);
        assert_eq!(
            standing_verdict(&world, &Position::new(-23.5, 18.5)),
            StandingVerdict::NotProvablyBlocked,
            "a tile away from the box is not inside it"
        );
    }

    /// The exact arithmetic the note turns on, as a boundary test.
    /// `0.69921875 + 0.19921875 = 0.8984375` is where a character stands clear
    /// of a stone furnace; its box then *touches* the furnace's and touching is
    /// not colliding. One 1/256th nearer is an overlap. Both directions, one
    /// position unit apart.
    #[test]
    fn a_footprint_that_only_touches_the_furnace_is_walked_and_one_unit_nearer_is_not() {
        let world = world_with(&[Position::new(-22., 18.)]);
        assert_eq!(
            standing_verdict(&world, &Position::new(-22.8984375, 18.)),
            StandingVerdict::NotProvablyBlocked,
            "sharing an edge is legal standing room and must not be refused"
        );
        assert!(
            matches!(
                standing_verdict(&world, &Position::new(-22.89453125, 18.)),
                StandingVerdict::Blocked { .. }
            ),
            "1/256 nearer and the boxes genuinely overlap"
        );
    }

    /// With no `character` prototype the footprint collapses to a point, which
    /// is a weaker question and must stay weaker: run 30's endpoint is still
    /// strictly inside the furnace and is still refused, while a position where
    /// only a character's *extent* would overlap is no longer provably blocked
    /// and is walked.
    #[test]
    fn without_a_character_prototype_the_question_narrows_rather_than_guesses() {
        let world = world_with(&[Position::new(-22., 18.)]);
        world.entity_prototypes.remove(CHARACTER_PROTOTYPE);

        assert!(
            matches!(
                standing_verdict(&world, &Position::new(-22.30078125, 18.22265625)),
                StandingVerdict::Blocked { .. }
            ),
            "a point strictly inside a building is blocked whatever stands on it"
        );
        assert_eq!(
            standing_verdict(&world, &Position::new(-22.89453125, 18.)),
            StandingVerdict::NotProvablyBlocked,
            "an extent we were never told is not one to invent"
        );
    }

    /// An empty path is not a destination anybody chose. The mod completes such
    /// a walk on the next tick without moving, and the bot's current position
    /// is judged by the arrival check alone -- so a bot standing somewhere the
    /// graph calls blocked (a furnace built on top of it, a stale box) is not
    /// refused a walk it never asked for.
    #[test]
    fn an_empty_path_is_not_judged_for_standing_room() {
        let world = world_with(&[Position::new(-22., 18.)]);
        let here = Position::new(-22.30078125, 18.22265625);
        assert!(
            matches!(
                standing_verdict(&world, &here),
                StandingVerdict::Blocked { .. }
            ),
            "the position itself is inside the furnace"
        );
        assert!(
            judge_path(&world, &here, None, &[], Some(&here)).is_ok(),
            "but there is no walk here to refuse"
        );
    }

    /// The arrival check keeps its precedence and its wording: a path that
    /// falls short is still `RconWalkFallsShort`, not the new refusal, even
    /// when its endpoint is also inside a furnace.
    #[test]
    fn a_path_that_falls_short_is_still_reported_as_falling_short() {
        let world = world_with(&[Position::new(-22., 18.)]);
        let failure = refusal(judge_path(
            &world,
            &Position::new(40., 40.),
            None,
            &[Position::new(-22.30078125, 18.22265625)],
            None,
        ));
        let message = format!("{:?}", failure.error);
        assert!(
            message.contains("no path to"),
            "a walk that does not arrive is refused for not arriving: {message}"
        );
    }

    /// Not every blocker has a name to give. `blocked_tree` holds water tiles
    /// and trees, which the entity tree never sees, so the refusal falls back
    /// to the box it did find rather than inventing a name or -- worse --
    /// declining to refuse.
    #[test]
    fn an_unnameable_blocker_is_still_refused_and_reported_as_a_box() {
        let world = FactorioWorld::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        world
            .update_chunk_entities(vec![FactorioEntity::new_tree(&Position::new(3., 4.))])
            .unwrap();
        let world = Arc::new(world);

        let verdict = standing_verdict(&world, &Position::new(3., 4.));
        match verdict {
            StandingVerdict::Blocked { blocker } => assert!(
                blocker.contains("collision box spanning"),
                "an unnamed blocker reports its box: {blocker}"
            ),
            other => panic!("a tree is a blocker: {other:?}"),
        }
    }
}
