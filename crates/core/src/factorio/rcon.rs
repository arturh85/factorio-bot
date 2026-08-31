use crate::errors::{
    RconError, RconNoWaterFound, RconOutOfResourceReach, RconPlayerBlockesAllPlacement,
    RconPlayerBlockesPlacement, RconPlayerNotFound, RconRadiusLimitReached, RconTimeout,
    RconUnexpectedEmptyResponse, RconUnexpectedOutput, RconWalkFallsShort,
};
use crate::factorio::snapshot::WorldSnapshot;
use crate::factorio::ticks::{ActionTicks, take_tick_stamp};
use crate::factorio::util::{
    blueprint_build_area, build_entity_path, calculate_distance, hashmap_to_lua, map_blocked_tiles,
    move_pos, move_position, position_to_lua, rect_to_lua, span_rect, str_to_lua, value_to_lua,
    vec_to_lua, vector_add, vector_multiply, vector_normalize, vector_substract,
};
use crate::factorio::world::FactorioWorld;
use crate::settings::FactorioSettings;
use crate::types::{
    ActionId, AreaFilter, Direction, FactorioEntity, FactorioForce, FactorioPlayer, FactorioTile,
    InventoryResponse, PlayerId, Pos, Position, Rect, RequestEntity,
};
use miette::{Context, IntoDiagnostic, Report, Result, miette};
use paris::info;
use parking_lot::RwLock;
use rcon::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::sleep;
use unicode_segmentation::UnicodeSegmentation;

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
        info!("<cyan>rcon</>  ⮞ <green>{}</>", body);
    }
    Some(body.split('\n').map(|str| str.to_owned()).collect())
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

pub struct FactorioRcon {
    pool: Option<bb8::Pool<ConnectionManager>>,
    silent: Arc<RwLock<bool>>,
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
            info!("<cyan>rcon</>  ⮜ <green>{}</>", command);
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
        Ok(take_tick_stamp(
            self.remote_call(function_name, args).await?,
        ))
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
            info!("<cyan>rcon</>  ⮜ <green>{}</>", command);
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
    /// The mod registers `2 + one per bot` cameras: `follow` (player 1),
    /// `bot-<player_index>` for every player the game knows of, and `area`,
    /// which frames the bounding box of all connected bots. A camera whose bot
    /// is not connected writes **no file** for that tick rather than a
    /// substitute, so a bot that joined late has no frames from before it
    /// joined.
    ///
    /// **0.72 MB per frame**, measured at JPEG quality 85 and 1920x1080. One
    /// frame per camera every 300 ticks is 12 a minute, so **one camera costs
    /// ~520 MB an hour and the three cameras of a one-bot run cost ~1.56 GB an
    /// hour** — and each further bot adds a camera, hence another ~520 MB an
    /// hour. It is wiped per run and never accumulates across runs, but a long
    /// run with several bots fills a disk. The number is here so that whoever
    /// adds a fourth camera reads it before adding it rather than afterwards.
    ///
    /// Taken by value rather than as `Option<&str>` because this `impl` is
    /// `#[automock]`ed and mockall cannot elide a lifetime inside a generic.
    pub async fn frame_capture_start(&self, run_id: Option<String>) -> Result<Option<u64>> {
        let args = match run_id.as_deref() {
            Some(run_id) => vec![lua_string_literal(run_id)],
            None => vec![],
        };
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
        self.remote_call("whoami", vec![str_to_lua(name)]).await?;
        Ok(())
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

    /// Adds research to the queue
    pub async fn add_research(&self, technology_name: &str) -> Result<()> {
        self.add_research_timed(technology_name)
            .await
            .map(|_| ())
            .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::add_research`], reporting the game tick it ran at.
    ///
    /// Research is queued synchronously: the command runs and returns inside
    /// one tick, so both ends of [`ActionTicks`] are that tick. That is a
    /// measurement, not a duplicated estimate -- the game really did receive
    /// and finish with the command in the same tick.
    pub async fn add_research_timed(
        &self,
        technology_name: &str,
    ) -> Result<ActionTicks, ActionFailure> {
        let (_lines, tick) = self
            .remote_call_timed("add_research", vec![str_to_lua(technology_name)])
            .await?;
        Ok(ActionTicks::at(tick))
    }

    /// Cheats in an Item in given quantity to given player
    pub async fn cheat_item(
        &self,
        player_id: PlayerId,
        item_name: &str,
        item_count: u32,
    ) -> Result<()> {
        self.remote_call(
            "cheat_item",
            vec![
                player_id.to_string(),
                str_to_lua(item_name),
                item_count.to_string(),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn cheat_technology(&self, technology_name: &str) -> Result<()> {
        self.remote_call("cheat_technology", vec![str_to_lua(technology_name)])
            .await?;
        Ok(())
    }

    pub async fn cheat_all_technologies(&self) -> Result<()> {
        self.remote_call("cheat_all_technologies", vec![]).await?;
        Ok(())
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
        if &json[0..1] == "[" {
            Ok(serde_json::from_str(json.as_str()).into_diagnostic()?)
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
        if &json[0..1] == "{" {
            Ok(serde_json::from_str(json.as_str()).into_diagnostic()?)
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
        serde_json::from_str(json.as_str()).into_diagnostic()
    }

    pub async fn store_map_data(&self, key: &str, value: Value) -> Result<()> {
        self.remote_call(
            "store_map_data",
            vec![str_to_lua(key), value_to_lua(&value)],
        )
        .await?;
        Ok(())
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
        Ok(Some(serde_json::from_str(json.as_str()).into_diagnostic()?))
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
    ) -> Result<Vec<Position>> {
        let wait_start = Instant::now();
        loop {
            sleep(Duration::from_millis(50)).await;
            // Take the reply in one operation -- see sleep_for_action_result.
            if let Some((_, mut result)) = world.path_requests.remove(&request_id) {
                if result == "{}" {
                    result = String::from("[]");
                }
                return serde_json::from_str(result.as_str()).into_diagnostic();
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

        // The arrival check. `player_path` may have substituted the goal, so
        // the path is judged against what the caller asked for. An unknown
        // player position with an empty path leaves nothing to judge, and an
        // unjudgeable walk is dispatched rather than refused on a guess.
        let here = world.players.get(&player_id).map(|p| p.position.clone());
        if let Some(end) = walk_end_position(&waypoints, here.as_ref())
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
    /// If the player has to walk to the resource first, the ticks reported are
    /// the *mining* action's own -- the walk is a separate dispatch with
    /// separate ticks, and folding the two together would make the mine look
    /// like it started when the bot set off.
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
            self.move_player(
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
        self.sleep_for_action_result(world, action_id, dispatched)
            .await
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
        let dispatched = self
            .action_start_crafting(action_id, player_id, recipe, count)
            .await?;
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
        serde_json::from_str(json.as_str()).into_diagnostic()
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
        serde_json::from_str(json.as_str()).into_diagnostic()
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
                let chars =
                    UnicodeSegmentation::graphemes(line.as_str(), true).collect::<Vec<&str>>();
                if chars[0] == "{" {
                    Ok((serde_json::from_str(line).unwrap(), ActionTicks::at(tick)))
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
                                let chars = UnicodeSegmentation::graphemes(line.as_str(), true)
                                    .collect::<Vec<&str>>();
                                if chars[0] == "{" {
                                    Ok((serde_json::from_str(line).unwrap(), ActionTicks::at(tick)))
                                } else if &line[..] == "§player_blocks_placement§" {
                                    Err(ActionFailure::refused(
                                        RconPlayerBlockesPlacement {}.into(),
                                        refused_at,
                                    ))
                                } else {
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

    pub async fn find_entities_filtered(
        &self,
        area_filter: &AreaFilter,
        search_name: Option<String>,
        search_type: Option<String>,
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
        if let Some(entity_type) = search_type {
            args.insert(String::from("type"), str_to_lua(&entity_type));
        }
        let result = self
            .remote_call("find_entities_filtered", vec![hashmap_to_lua(args)])
            .await?;
        if result.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let mut json = result.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        serde_json::from_str(json.as_str()).into_diagnostic()
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
        let result = self
            .remote_call("find_tiles_filtered", vec![hashmap_to_lua(args)])
            .await?;
        if result.is_none() {
            return Err(RconUnexpectedEmptyResponse {}.into());
        }
        let mut json = result.unwrap().pop().unwrap();
        // empty objects/arrays are the same in lua
        if json == "{}" {
            json = String::from("[]");
        }
        serde_json::from_str(json.as_str()).into_diagnostic()
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
    pub async fn player_path(
        &self,
        world: &Arc<FactorioWorld>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<Vec<Position>> {
        let id = self
            .async_request_player_path(player_id, goal, radius)
            .await?;
        match self.sleep_for_path_request_result(world, id).await {
            Ok(path) => Ok(path),
            Err(err) => {
                warn!(
                    "failed to find player_path() for #{} to {}/{}: {:?}",
                    player_id,
                    goal.x(),
                    goal.y(),
                    err
                );
                let player = world.players.get(&player_id).unwrap();
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
                        return Ok(result);
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
        let id = self.async_request_path(start, goal, radius).await?;
        match self.sleep_for_path_request_result(world, id).await {
            Ok(path) => Ok(path),
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
                        return Ok(result);
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

    pub async fn action_start_crafting(
        &self,
        action_id: ActionId,
        player_id: PlayerId,
        recipe: &str,
        count: u32,
    ) -> Result<Option<u64>> {
        let action_id = action_id.to_string();
        let player_id = player_id.to_string();
        let (result, tick) = self
            .remote_call_timed(
                "action_start_crafting",
                vec![action_id, player_id, str_to_lua(recipe), count.to_string()],
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
    fn transfer(call: &str, held: i64, moves: i64) -> (Result<ActionTicks, ActionFailure>, String) {
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
        lua.load(stub_game(held, moves))
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

        let printed: Vec<String> = lua
            .globals()
            .get::<mlua::Table>("_rcon_lines")
            .expect("_rcon_lines")
            .sequence_values::<String>()
            .map(|v| v.expect("rcon line"))
            .collect();

        // The RCON server hands back exactly this: the printed lines, plus the
        // trailing newline `split_reply` is written to strip.
        let body = if printed.is_empty() {
            String::new()
        } else {
            printed.join("\n") + "\n"
        };
        let (lines, tick) = take_tick_stamp(split_reply(&body, true));
        (judge_transfer_reply(lines, tick), printed.join("\n"))
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
    #[test]
    fn a_remove_that_moved_some_but_not_all_is_a_failure() {
        let (verdict, printed) = transfer(REMOVE_TEN, 0, 7);
        verdict.expect_err("a remove that moved 7 of 10 reported success");
        assert!(printed.contains("but removed 7"), "printed {printed:?}");
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
