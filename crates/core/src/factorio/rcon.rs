use crate::blueprint::UndergroundHalf;
use crate::errors::{
    RconError, RconNoWaterFound, RconOutOfResourceReach, RconPathRequestFailed,
    RconPlayerBlockesAllPlacement, RconPlayerBlockesPlacement, RconPlayerNotFound,
    RconRadiusLimitReached, RconReplyNotJson, RconTimeout, RconUnexpectedEmptyResponse,
    RconUnexpectedOutput, RconWalkFallsShort,
};
use crate::factorio::snapshot::{GeneratedChunks, WorldSnapshot};
use crate::factorio::ticks::{ActionTicks, take_tick_stamp};
use crate::factorio::util::{
    add_to_rect, blueprint_build_area, build_entity_path, calculate_distance, hashmap_to_lua,
    map_blocked_tiles, move_pos, move_position, position_to_lua, rect_to_lua, span_rect,
    str_to_lua, value_to_lua, vec_to_lua, vector_add, vector_multiply, vector_normalize,
    vector_substract,
};
use crate::factorio::world::{
    FactorioSurface, HOP_RADIUS, PlacementRefusal, RefusalSource, hop_targets,
};
use crate::graph::entity_graph::ResourceDepletion;
use crate::settings::FactorioSettings;
use crate::types::{
    ActionId, AreaFilter, Direction, FactorioEntity, FactorioForce, FactorioPlayer, FactorioTile,
    InventoryResponse, PlayerId, Pos, Position, Rect, RequestEntity,
};
use miette::{Context, IntoDiagnostic, Report, Result, miette};
use parking_lot::RwLock;
use rcon::Connection;
use serde::{Deserialize, Serialize};
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

/// The positional argument list for `sampling_start`.
///
/// An absent run id sends *no argument*, rather than a `nil` placeholder: the
/// mod distinguishes "no id" from any value, and this is the one place that
/// distinction is turned into bytes.
fn sampling_args(run_id: Option<&str>) -> Vec<String> {
    run_id
        .map(|run_id| vec![lua_string_literal(run_id)])
        .unwrap_or_default()
}

/// Judges the reply to either sampling toggle.
///
/// Unlike most `remote_call_timed` callers here, this refuses a reply that
/// still has text in it after the tick stamp is taken off. The mod raises on a
/// run id that is not a string, and the game reports that as "Cannot execute
/// command. Error: ..." in the reply body rather than as a transport failure.
/// Dropping those lines would turn a session that never started into a silent
/// success, and the first evidence would be an empty `samples.jsonl` much
/// later.
///
/// A free function taking the reply, rather than a method taking the call's
/// name: the name has to stay a literal in each toggle's own body, because
/// that is where `each_rcon_doc_block_names_the_remote_call_its_binding_reaches`
/// reads it from. Passing the name down to a shared sender hid it from that
/// check, which is exactly the check that catches a doc block promising a
/// `remote.call` the binding does not make.
fn sampling_verdict(reply: (Option<Vec<String>>, Option<u64>)) -> Result<Option<u64>> {
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
/// This is [`sampling_verdict`] without the tick: the same rule, for the
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

/// Says so, loudly, when a successful placement moved a character.
///
/// `rcon_place_entity` (`mods/BotBridge/control.lua`, `push_characters_out_of`)
/// adds `pushed_out: [{bot, from, to}]` to the entity it answers with whenever
/// the built entity's real bounding box held a character -- the game's own
/// push-out for a player, done for a server-side character, which the game
/// leaves inside the building with every later path request refused. The
/// entity parse ignores the field, so this is where the executor's log learns
/// of it; the record learns of it through the mod's `teleport` writeout with
/// reason `placement_pushed_out`. Absent `to` means the character fit nowhere
/// within reach and was left where it stood.
fn note_pushed_out(player_id: PlayerId, line: &str) {
    if !line.contains("\"pushed_out\"") {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(pushed) = value.get("pushed_out").and_then(|v| v.as_array()) else {
        return;
    };
    for push in pushed {
        match push.get("to") {
            Some(to) if !to.is_null() => warn!(
                "#{} built {} over bot {} at {}; the character was moved out to {}",
                player_id, value["name"], push["bot"], push["from"], to
            ),
            _ => warn!(
                "#{} built {} over bot {} at {}, and the character fit nowhere within reach: it is still inside",
                player_id, value["name"], push["bot"], push["from"]
            ),
        }
    }
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
/// downstream. See `transfer_guarantee_tests` below -- and inside it
/// `a_remove_that_moved_nothing_is_a_failure`, `an_insert_that_moved_nothing_is_a_failure`
/// and their discriminator `a_transfer_that_moved_everything_asked_for_succeeds`
/// -- which drive the real mod source to prove it.
///
/// The two mechanisms this rests on are the mod's complaint path and this
/// function; breaking either silently downgrades every transfer's `Success` to
/// the weak reading.
///
/// # The one complaint that is not a failure
///
/// There are two ways an insert moves less than it was asked to, and they are
/// **opposite outcomes**:
///
/// - **The source was short.** The bot did not hold what the plan believed it
///   held, so the delivery did not happen and the plan's model of the world is
///   wrong. The mod clamps and says so (`... only has N. clamping...`). A real
///   failure, and it stays one.
/// - **The destination was full.** The bot held everything, offered it, and the
///   inventory had no room for the rest. Nobody can make that inventory hold
///   more of that item; the remainder stays with the bot, which is where it
///   belongs. Nothing is wrong with the world and re-issuing the command would
///   change nothing.
///
/// Collapsing the second into the first cost `run-1788432181-42528` its
/// furthest-ever run at tick 211399: `boiler_coal` sizes a top-up from demand
/// alone, because nothing in `FactorioWorld` reports a fuel level, so it asked
/// for 17 coal into a boiler already holding 47. A fuel slot is one stack, the
/// boiler took 3, and the executor read the mod's honest report as a verdict of
/// failure and abandoned the rest of that bot's chain.
///
/// The counts alone cannot tell the two apart -- *3 of 17 moved* is what both
/// look like from here -- so the mod reports the destination's own state
/// beside them (`(destination holds H, room for R)`) and this function makes
/// the call. **The judgement is here and the observation is there**, on
/// purpose: only the mod can see `H` and `R`, and only this side is unit
/// testable against the real `control.lua`.
///
/// A reply whose shortfall line carries no such suffix -- an older
/// `workspace/mods` copy that a release build extracted before this existed --
/// fails exactly as it always did. That is the safe direction: a stale mod
/// under-claims rather than inventing a satisfied goal.
///
/// # A zero-move is still a failure, except in the one case where it is not
///
/// `moved == 0` with `holds > 0` and `room == 0` is a boiler whose fuel slot
/// was *already* a full stack of the item asked for. Nothing moved because
/// nothing needed to, and the goal held before the bot arrived. Every other
/// zero-move stays a failure, including the one this distinction is most
/// easily confused with: an inventory that would not take the item at all --
/// the wrong fuel, a filtered slot, a recipe that does not use it -- reports
/// `holds 0`, and `holds > 0` is what excludes it.
fn judge_transfer_reply(
    lines: Option<Vec<String>>,
    tick: Option<u64>,
) -> Result<TransferOutcome, ActionFailure> {
    let ticks = ActionTicks::at(tick);
    let Some(lines) = lines else {
        return Ok(TransferOutcome {
            ticks,
            destination_full: None,
        });
    };
    // Every surviving line must be a full-destination shortfall for this to be
    // anything but a failure. A reply that clamped *and* then overflowed
    // prints both lines, and the clamp is the one that means the bot could not
    // deliver -- so one unrecognised line is enough to refuse the whole thing.
    let shortfalls: Option<Vec<InsertShortfall>> = lines
        .iter()
        .map(|line| parse_insert_shortfall(line).filter(InsertShortfall::is_destination_full))
        .collect();
    if let Some(shortfalls) = shortfalls
        && let Some(first) = shortfalls.into_iter().next()
    {
        return Ok(TransferOutcome {
            ticks,
            destination_full: Some(first.into_report()),
        });
    }
    // The game answered, so it saw the command and judged it: a verdict
    // at a real tick, not a command that never landed.
    Err(ActionFailure::refused(
        RconError {
            message: format!("{lines:?}"),
        }
        .into(),
        ticks,
    ))
}

/// What a transfer the game judged actually did.
///
/// `ticks` is what [`judge_transfer_reply`] always returned. `destination_full`
/// is the fact that used to be destroyed: a transfer that succeeded *without*
/// moving everything asked, because the destination had no room for the rest.
///
/// It is `Some` only on the success path, and only for an insert -- the mod's
/// remove handler has no destination to be full. A caller that ignores it
/// loses nothing about the verdict; it loses the run record's only evidence
/// that the planner asked for more than the world could hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferOutcome {
    pub ticks: ActionTicks,
    pub destination_full: Option<DestinationFull>,
}

/// An insert that succeeded because the destination was already at capacity.
///
/// Carried out to the run record (`crates/executor`) rather than logged and
/// dropped, because a run whose every boiler top-up moves 3 of 17 and a run
/// whose top-ups all move 17 must not look identical afterwards: the first one
/// says the planner is sizing from demand against a world it cannot see, and
/// that is precisely the diagnosis this outcome used to hide behind a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationFull {
    pub item: String,
    /// What the command asked to insert.
    pub asked: u32,
    /// What the destination accepted. May be zero -- see
    /// [`judge_transfer_reply`].
    pub moved: u32,
    /// How much of `item` the destination holds now, `moved` included.
    pub holds: u32,
}

impl std::fmt::Display for DestinationFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "destination full: moved {} of {} {}, which now holds {}",
            self.moved, self.asked, self.item, self.holds
        )
    }
}

/// One `tried to insert ... but inserted ...` complaint, parsed.
///
/// The wording is `rcon_insert_to_inventory`'s
/// (`mods/BotBridge/control.lua`):
///
/// ```text
/// tried to insert 17x coal but inserted 3 (destination holds 50, room for 0)
/// ```
///
/// Parsed rather than pattern-matched wholesale so that a wording change costs
/// a `None` -- which refuses the transfer, the behaviour before this existed --
/// rather than a wrong verdict. Nothing here is inferred: every one of the four
/// numbers must parse or this declines.
///
/// The prefix through `but inserted <moved>` is *also* read by
/// `partial_transfer_detail`
/// (`crates/scripting_lua/src/globals/record.rs`), which builds the
/// `FailureKind::PartialTransfer` detail for the failures that survive.
/// The suffix was appended rather than folded into that wording so both
/// readers keep working; `the_shortfall_wording_carries_both_readers_numbers`
/// below is the pin on the wording, and
/// `a_short_source_is_a_failure_even_when_the_destination_was_also_full` is the
/// pin on the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InsertShortfall {
    item: String,
    asked: u32,
    moved: u32,
    holds: u32,
    room: u32,
}

impl InsertShortfall {
    /// Whether this shortfall is the destination being at capacity rather than
    /// the destination refusing the item.
    ///
    /// `room == 0` says no more of this item fits. `holds > 0` says the
    /// inventory is full *of the thing that was asked for*, which is what
    /// separates a fuelled boiler from a fuel slot that would not take the
    /// item at all -- the latter reports `holds 0` and stays a failure.
    fn is_destination_full(&self) -> bool {
        self.room == 0 && self.holds > 0
    }

    fn into_report(self) -> DestinationFull {
        DestinationFull {
            item: self.item,
            asked: self.asked,
            moved: self.moved,
            holds: self.holds,
        }
    }
}

fn parse_insert_shortfall(line: &str) -> Option<InsertShortfall> {
    let rest = line.trim().strip_prefix("tried to insert ")?;
    let (asked, rest) = rest.split_once("x ")?;
    let (item, rest) = rest.split_once(" but inserted ")?;
    let (moved, rest) = rest.split_once(" (destination holds ")?;
    let (holds, room) = rest.split_once(", room for ")?;
    let room = room.strip_suffix(')')?;
    Some(InsertShortfall {
        item: item.to_string(),
        asked: asked.parse().ok()?,
        moved: moved.parse().ok()?,
        holds: holds.parse().ok()?,
        room: room.parse().ok()?,
    })
}

/// Judges the reply to a `set_recipe` RPC.
///
/// Same rule as [`judge_transfer_reply`] and the same reason it is strong:
/// `rcon_set_recipe` (`mods/BotBridge/control.lua`) prints **only** its
/// `§tick§` stamp when the recipe is set, and a sentence when it is not. So an
/// empty reply once the stamp is off is the mod asserting all of it -- the
/// player exists, the recipe exists and the force has unlocked it, an
/// assembling machine is standing where the plan said, the game took the
/// recipe (checked by reading it back with `get_recipe`, because
/// `LuaEntity.set_recipe` returns evicted items rather than a verdict), and
/// whatever it evicted reached the bot's inventory.
///
/// Anything left over is a verdict of failure, carried through verbatim: the
/// mod's line is where "this recipe is not enabled for this force" and "there
/// is no machine there" are told apart, and re-deriving that distinction from
/// an error kind up here would be a second, weaker copy of a judgement the mod
/// already made.
///
/// A separate function from [`judge_transfer_reply`] despite the identical
/// body: that one's `Success` means "the requested count is the count that
/// moved", and this one's means the paragraph above. Merging them would leave
/// one doc comment claiming both.
fn judge_set_recipe_reply(
    lines: Option<Vec<String>>,
    tick: Option<u64>,
) -> Result<ActionTicks, ActionFailure> {
    if let Some(lines) = lines {
        // The game answered, so it saw the command and judged it.
        return Err(ActionFailure::refused(
            RconUnexpectedOutput {
                output: lines.join("\n"),
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

/// The mod's wording for a placement refused because some character *other*
/// than the acting bot is standing in the footprint.
///
/// Deliberately outside the [`CAN_PLACE_REFUSAL`] family -- see
/// `rcon_place_entity` in `mods/BotBridge/control.lua`, which chooses between
/// the three answers -- so it is never remembered as a fact about the ground.
/// It is a fact about a character, and a character moves. Matched here, at the
/// one place the game's own line is still a line, for the same reason
/// [`CAN_PLACE_REFUSAL`] is: a wording change costs a retry that no longer
/// happens, never a site fenced off for a reason nobody gave.
const FOOTPRINT_CHARACTER_REFUSAL: &str = "a character is standing in the footprint";

/// How many times [`FactorioRcon::place_entity_timed`] issues a placement whose
/// footprint a character is standing in, counting the first.
///
/// Four -- three waits of [`FOOTPRINT_CLEAR_BACKOFF`], so 1.8 s, about 108
/// ticks. The mod dispatches a step-aside walk for the blocker as it refuses,
/// so the question being asked again is "has it moved yet"; the one measurement
/// there is says 53 ticks (see [`FOOTPRINT_CLEAR_BACKOFF`]), and the budget is
/// twice that because the sample beat that produced the 53 is 60 ticks wide and
/// the true figure is anywhere inside it.
///
/// The only blocker that never moves is one the mod declined to steer -- it is
/// already walking or mining for an action of its own, and so is leaving anyway
/// -- or one the game found nowhere to put. Both of those keep the old
/// behaviour once the attempts run out, two seconds later.
const FOOTPRINT_CLEAR_ATTEMPTS: u32 = 4;

/// How long to wait between the attempts [`FOOTPRINT_CLEAR_ATTEMPTS`] counts.
///
/// Sized from the run that motivated the retry: the blocker in
/// `run-1788481380-80843` was clear of the footprint **53 ticks** -- under a
/// second at 60 UPS -- after the refusal that asked it to move. A step aside is
/// one or two tiles by construction (`placement_step_aside_target` leaves by
/// the *nearest* edge), so this is a walk measured in tens of ticks, not
/// hundreds. Wall clock rather than ticks because that is what this layer has:
/// nothing here reads the game clock, and a placement is judged in the tick it
/// is received.
const FOOTPRINT_CLEAR_BACKOFF: Duration = Duration::from_millis(600);

/// The clause the mod appends to [`FOOTPRINT_CHARACTER_REFUSAL`] naming each
/// character it found and what that character is doing --
/// ` (blockers: #1 mining, #3 stepping aside)`. See
/// `describe_footprint_blockers` in `mods/BotBridge/control.lua`, which owns
/// the vocabulary.
const FOOTPRINT_BLOCKERS_CLAUSE: &str = " (blockers: ";

/// The two words in that clause that mean **this blocker is leaving on its
/// own**: it is walking or mining for an action of its own, which is exactly
/// why `step_aside_from_footprint` declined to steer it.
const FOOTPRINT_BUSY_WORDS: [&str; 2] = ["mining", "walking"];

/// Whether the mod's footprint refusal names a blocker that is busy with an
/// action of its own.
///
/// Read from **after** the refusal sentence, never from the whole line: the
/// item name comes first and `burner-mining-drill` contains "mining". A line
/// from a mod copy old enough to have no clause answers `false` and keeps the
/// old fixed budget, which is the safe way round.
fn footprint_blocker_is_busy(line: &str) -> bool {
    let Some(rest) = line.split_once(FOOTPRINT_CHARACTER_REFUSAL) else {
        return false;
    };
    let Some(clause) = rest.1.split_once(FOOTPRINT_BLOCKERS_CLAUSE) else {
        return false;
    };
    let Some((blockers, _)) = clause.1.split_once(')') else {
        return false;
    };
    blockers.split(", ").any(|blocker| {
        FOOTPRINT_BUSY_WORDS
            .iter()
            .any(|word| blocker.rsplit(' ').next() == Some(word))
    })
}

/// How long a placement keeps asking while the footprint holds a blocker that
/// is busy with an action of its own.
///
/// # Why this is not [`FOOTPRINT_CLEAR_ATTEMPTS`]
///
/// That budget is sized for a *step aside* -- a walk of one or two tiles the
/// mod dispatched as it refused, measured once at 53 ticks. A blocker the mod
/// deliberately left alone is a different clock entirely: it is leaving when
/// **its own action** finishes, and that action is a whole mine or a whole
/// walk. Spending the step-aside budget on it asks four times inside two
/// seconds and then declares a transient permanent.
///
/// `run-1788655528-63394` is that: bot 1 mined copper ore at `(27.5, -47.5)`
/// from tick 5597 to 6079 while standing at `(26.29, -47.33)`, inside the
/// stone furnace bot 2 was to place at `[26, -48]`. The placement was refused
/// at 5887 and abandoned at 6001 -- **78 ticks before the blocker's own action
/// ended** -- and milestone 1 lost the action and replanned.
///
/// # Where the number comes from
///
/// Measured over the 24 archived runs in `workspace/runs`: the longest
/// successful `mine` action is **1,211 ticks** (20.2 s at 1x) and the longest
/// successful walk leg **1,977 ticks** (32.9 s); the 95th percentiles are 725
/// and 716. So 45 s covers every busy action ever observed here at 1x with
/// margin, and is an eighth of the executor's 360 s `ACTION_RESULT_DEADLINE`,
/// which is the deadline this must stay well inside. At a faster
/// `--game-speed` it covers proportionally more game time, which is the
/// direction that cannot hurt.
///
/// **It is a bound, not a promise.** A blocker that is still busy after this
/// fails the placement exactly as it did before, with the clause naming what
/// it was doing -- so the next reader gets the fact the run above did not
/// leave behind.
const FOOTPRINT_BUSY_BLOCKER_BUDGET: Duration = Duration::from_secs(45);

/// How long to wait between attempts while a busy blocker is in the way.
///
/// Longer than [`FOOTPRINT_CLEAR_BACKOFF`], because the question is different:
/// "has a two-tile walk landed yet" is worth asking every 0.6 s, "has a whole
/// mine finished yet" is not. At this cadence
/// [`FOOTPRINT_BUSY_BLOCKER_BUDGET`] costs about 22 extra RCON round trips in
/// the worst case, against the milestone replan it is there to prevent.
const FOOTPRINT_BUSY_BACKOFF: Duration = Duration::from_millis(2000);

/// What [`FactorioRcon::place_entity_timed`] does with a footprint refusal it
/// has just received.
///
/// Three answers rather than two, for the same reason the mod's branch has
/// three: a blocker that is leaving on its own, a blocker that has just been
/// asked to leave, and no more waiting. Decided in one place because the loop
/// has two arms that reach the same refusal -- the plain one and the one after
/// the actor has been walked aside -- and they must not drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FootprintWait {
    /// The blocker is busy with an action of its own. Wait
    /// [`FOOTPRINT_BUSY_BACKOFF`] and ask again **without** spending the
    /// step-aside budget: once it goes idle, the step-aside attempts are still
    /// there for it.
    Busy,
    /// Some other footprint refusal, with step-aside attempts left. Wait
    /// [`FOOTPRINT_CLEAR_BACKOFF`] and spend one.
    StepAside,
    /// Not a footprint refusal, or the budgets are spent. Report it.
    GiveUp,
}

impl FootprintWait {
    /// `attempt` is how many step-aside attempts have already been spent and
    /// `busy_waited` how long this placement has already waited on a busy
    /// blocker.
    fn decide(line: &str, attempt: u32, busy_waited: Duration) -> Self {
        if !line.contains(FOOTPRINT_CHARACTER_REFUSAL) {
            return Self::GiveUp;
        }
        if footprint_blocker_is_busy(line) && busy_waited < FOOTPRINT_BUSY_BLOCKER_BUDGET {
            return Self::Busy;
        }
        if attempt + 1 < FOOTPRINT_CLEAR_ATTEMPTS {
            return Self::StepAside;
        }
        Self::GiveUp
    }

    /// How long to wait before asking again, or `None` when there is no more
    /// asking to do.
    fn backoff(self) -> Option<Duration> {
        match self {
            Self::Busy => Some(FOOTPRINT_BUSY_BACKOFF),
            Self::StepAside => Some(FOOTPRINT_CLEAR_BACKOFF),
            Self::GiveUp => None,
        }
    }

    /// Which budget this wait is drawn from: `true` spends
    /// [`FOOTPRINT_BUSY_BLOCKER_BUDGET`], `false` spends one of
    /// [`FOOTPRINT_CLEAR_ATTEMPTS`]. Asked rather than inferred from the
    /// duration, so the two constants may take any values without the loop
    /// quietly charging the wrong account.
    fn spends_busy_budget(self) -> bool {
        matches!(self, Self::Busy)
    }
}

/// What a placement's retries cost, measured while they happen.
///
/// # The zero this exists to stop reporting
///
/// A placement is synchronous, so [`ActionTicks::at`] is the right shape for
/// one dispatch: the game receives the command and answers inside the same
/// tick, and both ends of the measurement genuinely are that one number. The
/// retry loop above turns *one action* into up to
/// [`FOOTPRINT_CLEAR_ATTEMPTS`] dispatches spread over
/// `3 x FOOTPRINT_CLEAR_BACKOFF`, and reporting the last dispatch's tick on
/// both ends throws every one of those ticks away: the settle arrives at
/// `elapsed_ticks: 0`, byte-identical to a placement that was refused on the
/// spot.
///
/// That is not academic. [`FOOTPRINT_CLEAR_BACKOFF`] was sized at twice a
/// single 53-tick observation, a second case later turned out to need >= 480
/// ticks, and **no run record could say how long the retries had actually
/// taken** -- the number the constant is meant to be sized from was the one
/// number the record did not have.
///
/// So the span is kept honestly: `dispatched` is the tick the game stamped on
/// the **first** dispatch, `replied` the tick it stamped on the one that
/// settled the action. A placement that never retried is unaffected -- its
/// first dispatch *is* its last, so both ends are the same number exactly as
/// before, and `elapsed_ticks: 0` still means "synchronous, first try".
///
/// # An absent first stamp stays absent
///
/// `first_dispatch` is `None` when the game did not stamp the first reply,
/// under the same rule as everything else in [`ActionTicks`]: a missing
/// measurement is a value, and substituting the *last* attempt's tick for it
/// would report a retried placement as instantaneous while looking measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PlacementAttempts {
    /// `game.tick` stamped on the first dispatch, when the game stamped one.
    first_dispatch: Option<u64>,
    /// How many dispatches have gone out, counting the first. Saturating for
    /// the same reason `Attempt::number` is: a count that wraps to zero would
    /// report a placement that was retried into the ground as never tried.
    ///
    /// This counts **dispatches**, not trips round the retry loop, so the
    /// extra `place_entity` the step-aside branch issues from the actor's new
    /// position is in here. It is deliberately not the loop's own budget
    /// counter -- that one governs behaviour and is left exactly as it was.
    made: u32,
    /// How long this side has slept between dispatches, accumulated as it
    /// sleeps rather than inferred from `made`. The two disagree by exactly
    /// the step-aside dispatch, which costs a dispatch and no backoff.
    waited: Duration,
}

impl PlacementAttempts {
    /// Called immediately after each `place_entity` RPC answers, with whatever
    /// tick it stamped.
    fn dispatched(&mut self, tick: Option<u64>) {
        if self.made == 0 {
            self.first_dispatch = tick;
        }
        self.made = self.made.saturating_add(1);
    }

    /// The span to report for an outcome the game answered at `replied`.
    ///
    /// Used on **both** the success and the failure returns: a placement that
    /// was retried three times and then refused took just as long as one that
    /// was retried three times and then built, and the record has to be able
    /// to say so about the failure -- which is the row it could not say it
    /// about before.
    fn ticks(&self, replied: Option<u64>) -> ActionTicks {
        ActionTicks::new(self.first_dispatch, replied)
    }

    /// Records one wait. Called where the sleep is, with the duration actually
    /// slept, so the number reported is the wait that was taken -- the two
    /// backoffs differ ([`FOOTPRINT_BUSY_BACKOFF`] against
    /// [`FOOTPRINT_CLEAR_BACKOFF`]) and a fixed constant here would report the
    /// wrong one.
    fn backed_off(&mut self, waited: Duration) {
        self.waited = self.waited.saturating_add(waited);
    }

    /// Whether the loop went round at all.
    fn retried(&self) -> bool {
        self.made > 1
    }

    /// The retry history, in words, for an outcome the game answered at
    /// `replied` -- `None` when there was no retry to describe.
    ///
    /// Appended to the failure the game gave rather than replacing it, so
    /// `classify_failure` (`crates/scripting_lua/src/globals/record.rs`) still
    /// sees the mod's own wording and still answers `FailureKind::Blocked`.
    /// **It contains no apostrophe on purpose**: that classifier reads a
    /// `MissingItem`'s item name out of the first pair of single quotes in the
    /// message, and a quote in here would hand it this sentence instead.
    fn describe(&self, replied: Option<u64>) -> Option<String> {
        if !self.retried() {
            return None;
        }
        let waited = self.waited.as_secs_f64();
        let spanned = match (self.first_dispatch, replied) {
            (Some(first), Some(last)) if last >= first => {
                format!("{} game ticks", last.saturating_sub(first))
            }
            // Absent, not zero. The game did not stamp both ends, so the only
            // honest span left is the one this side slept for.
            _ => "an unmeasured number of game ticks".to_string(),
        };
        Some(format!(
            "dispatched {made} times over {spanned} \
             ({waited:.1}s of waiting between attempts) and refused every time",
            made = self.made,
        ))
    }

    /// The game`s own refusal, with the retry history appended when there was
    /// one.
    ///
    /// Appended, never substituted: `line` is what the mod said and is what
    /// both `classify_failure` and a person read. This only adds the fact
    /// nothing in the record could otherwise supply -- that the refusal
    /// survived N dispatches rather than being the first answer.
    ///
    /// There is deliberately no counterpart on the success path. A placement
    /// that was retried and then *worked* says so through its ticks alone: a
    /// first-try placement settles at `elapsed_ticks: 0` because it is
    /// synchronous, so a non-zero elapsed on a `place` is a retried one. The
    /// failure path needs the words because the alternative there is inferring
    /// a count from a duration.
    fn explain(&self, line: &str, replied: Option<u64>) -> String {
        match self.describe(replied) {
            Some(note) => format!("{line}; {note}"),
            None => line.to_string(),
        }
    }
}

/// Remembers `line` as a refused site, if it is one.
///
/// Called on both arms that turn an unrecognised reply into an error: the
/// first attempt's, and the one after the actor has been walked aside. The
/// second matters as much as the first -- a refusal that survives the walk is
/// the strongest evidence there is that the blocker is not the actor.
fn note_placement_refusal(
    world: &Arc<FactorioSurface>,
    tick: Option<u64>,
    line: &str,
    item_name: &str,
    entity_position: &Position,
    direction: u8,
) {
    if !line.contains(CAN_PLACE_REFUSAL) {
        return;
    }
    // What the mod found in the box it had just had judged, read off the
    // line it appended it to. Nothing here is reconstructed from this side's
    // model: a refusal is the game disagreeing with the model, so the
    // model's opinion of the site is exactly the thing that cannot explain
    // it. An older mod that appended nothing yields an empty list and no
    // tile, which is what every dispatch refusal used to carry.
    let (blockers, tile) = parse_footprint_evidence(line);
    let refusal = PlacementRefusal::at_dispatch(
        tick,
        item_name,
        entity_position.clone(),
        direction,
        blockers,
        tile,
    );
    let cause = describe_blockers(&refusal.blockers, refusal.tile.as_deref());
    if world.record_placement_refusal(refusal) {
        warn!(
            "the game refused to build {} at {} facing {} ({}); the planner will avoid that \
             footprint for the rest of this run",
            item_name, entity_position, direction, cause
        );
    }
}

/// The parenthesis `rcon_place_entity` (`mods/BotBridge/control.lua`) appends
/// to the ground's refusal, read back as `(blockers, tile)`.
///
/// Two shapes, and only these two -- `describe_footprint` in the mod is the
/// writer and this is its reader:
///
/// ```text
/// ... said 'no' (in the footprint: small-electric-pole, tree-01; tile: grass-1)
/// ... said 'no' (nothing in the footprint; tile: grass-1)
/// ```
///
/// The tile clause is optional in both (the mod omits it when the tile is
/// invalid). A line with no parenthesis at all -- an older mod, or one of the
/// other exits in the same family -- reads as an empty list and no tile, the
/// same as before the mod named anything. The retry note
/// [`PlacementAttempts::explain`] appends comes *after* this parenthesis and
/// after a `;`, which is why the search is for the parenthesis that opens
/// right after the refusal rather than for the last `(` on the line.
fn parse_footprint_evidence(line: &str) -> (Vec<String>, Option<String>) {
    const FOUND: &str = "(in the footprint: ";
    const EMPTY: &str = "(nothing in the footprint";
    const TILE: &str = "; tile: ";
    let Some(after) = line
        .find(CAN_PLACE_REFUSAL)
        .map(|at| &line[at + CAN_PLACE_REFUSAL.len()..])
    else {
        return (Vec::new(), None);
    };
    let after = after.trim_start();
    let (blockers, rest) = if let Some(rest) = after.strip_prefix(FOUND) {
        let Some(end) = rest.find(')') else {
            return (Vec::new(), None);
        };
        let inside = &rest[..end];
        let (names, tile_part) = match inside.find(TILE) {
            Some(at) => (&inside[..at], Some(&inside[at + TILE.len()..])),
            None => (inside, None),
        };
        let blockers = names
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        (blockers, tile_part)
    } else if let Some(rest) = after.strip_prefix(EMPTY) {
        let Some(end) = rest.find(')') else {
            return (Vec::new(), None);
        };
        let inside = &rest[..end];
        (Vec::new(), inside.strip_prefix(TILE))
    } else {
        return (Vec::new(), None);
    };
    let tile = rest
        .map(str::trim)
        .filter(|tile| !tile.is_empty())
        .map(str::to_string);
    (blockers, tile)
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
    world: &Arc<FactorioSurface>,
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
            direction: Some(query.direction),
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
fn character_footprint(world: &FactorioSurface, at: &Position) -> Rect {
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
fn describe_blocker(world: &FactorioSurface, footprint: &Rect, blocker: &Rect) -> String {
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
fn standing_verdict(world: &FactorioSurface, at: &Position) -> StandingVerdict {
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

/// Half the side of the box the mod's walk follower halts in.
///
/// The follower stops "within a 0.3-by-0.3 box" of its last waypoint (see
/// [`approach_radius`]), so a walker aimed at a point can come to rest up to
/// this far from it on either axis. A bystander's box is grown by it before
/// the aim is tested, so "clear of the bystander" means clear of everywhere
/// the walker might actually stop, not only of the point it was aimed at.
const ARRIVAL_HALF_WIDTH: f64 = 0.3;

/// The ground the *other* bots are standing on, as boxes an aim must stay
/// out of.
///
/// The entity graph holds no characters -- `blocked_tree` is built from
/// entities and tiles, and a player's character is neither -- so a bot
/// standing still is invisible to [`standing_verdict`]. That is the whole of
/// `run-1788612263-27812`, bot 1, step 253 (tick 65,801): an insert at the
/// assembler at `[36.5, -5.5]`, aimed from `[29.26, -15.29]` at
/// `[34.73, -7.89]`, while bot 3 stood idle at `[34.25, -7.70]` and had done
/// since tick 55,330. The aim was 0.51 tiles from bot 3 -- closer than two
/// characters' half-widths plus the walker's stop box -- and the game's
/// pathfinder, which does see characters, answered `failed to path find`.
/// The batch replanned: nine steps and ~1,500 ticks for a goal the sweep
/// could have stepped 22.5° round.
///
/// Positions come from `world.players`, fed by the mod's
/// `on_player_changed_position` writeouts: once per tile while a bot walks,
/// and once more when its walker lets it stop (`writeout_player_position`
/// in `control.lua`). A resting bot -- the only kind that can be in the way
/// for long -- is therefore exact, and a walking one is within a tile of
/// where it was last seen, which the sweep then treats as occupied ground it
/// would rather not aim at. No RCON round trip is spent on this.
///
/// Each box is the character's own footprint at that position, grown by
/// [`ARRIVAL_HALF_WIDTH`]. `walker` is left out: it is the bot being aimed,
/// and its own position is on the near side of every ring it is aimed at.
fn bystander_boxes(world: &FactorioSurface, walker: Option<PlayerId>) -> Vec<Rect> {
    let mut boxes: Vec<(PlayerId, Rect)> = world
        .players
        .iter()
        .filter(|player| Some(*player.key()) != walker)
        .map(|player| {
            let footprint = character_footprint(world, &player.position);
            let grown = Rect::new(
                &Position::new(
                    footprint.left_top.x() - ARRIVAL_HALF_WIDTH,
                    footprint.left_top.y() - ARRIVAL_HALF_WIDTH,
                ),
                &Position::new(
                    footprint.right_bottom.x() + ARRIVAL_HALF_WIDTH,
                    footprint.right_bottom.y() + ARRIVAL_HALF_WIDTH,
                ),
            );
            (*player.key(), grown)
        })
        .collect();
    // `DashMap` iterates in no fixed order; the aim must not depend on it.
    boxes.sort_by_key(|(id, _)| *id);
    boxes.into_iter().map(|(_, rect)| rect).collect()
}

/// How far a character standing at `at` is from the nearest bystander's grown
/// box: zero when it overlaps one, `f64::INFINITY` when there are none.
fn bystander_clearance(world: &FactorioSurface, at: &Position, bystanders: &[Rect]) -> f64 {
    let footprint = character_footprint(world, at);
    bystanders
        .iter()
        .map(|bystander| {
            if boxes_overlap(bystander, &footprint) {
                0.0
            } else {
                distance_to_rect(at, bystander)
            }
        })
        .fold(f64::INFINITY, f64::min)
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
    world: &FactorioSurface,
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
///
/// # Zero-length legs are dropped here
///
/// The game repeats a waypoint: in `run-1788583161-11653`, 60 of 5,131 legs
/// across 192 paths were a position followed by itself -- 35 mid-path, 25 as
/// the final pair -- and those were the *only* legs shorter than half a tile
/// (every other one was 0.77 or longer; the median is a 1.41-tile diagonal).
/// The mod's follower survives such a leg, but not for free: it advances one
/// waypoint per tick, so a repeated waypoint is a tick spent standing inside a
/// box it was already in, and it counts as a leg in every `leg N of M` the
/// record carries. The path is the place to remove it, because this is the one
/// function every path passes through on its way to the mod.
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
    let mut positions: Vec<Position> = Vec::with_capacity(waypoints.len());
    for w in waypoints {
        if positions.last().is_some_and(|last| *last == w.position) {
            continue;
        }
        positions.push(w.position);
    }
    positions
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

/// The collision box of whatever stands *on* `target`, if anything does.
///
/// A tree or a rock the plan means to chop has one -- `blocked_tree` holds
/// every entity a character cannot walk through. Ore has none: a resource
/// does not collide with a character, so the tree never saw it, and `None`
/// here means "the target is ground a character can stand on".
fn blocking_box_at(world: &FactorioSurface, target: &Position) -> Option<Rect> {
    let probe = Rect::new(
        &Position::new(
            target.x() - BLOCKER_PROBE_MARGIN,
            target.y() - BLOCKER_PROBE_MARGIN,
        ),
        &Position::new(
            target.x() + BLOCKER_PROBE_MARGIN,
            target.y() + BLOCKER_PROBE_MARGIN,
        ),
    );
    // The quad tree admits boxes that merely come close; the exact test is
    // whether the target's own centre lies inside the box.
    world
        .entity_graph
        .blocking_boxes_within(&probe)
        .into_iter()
        .find(|b| b.contains(target))
}

/// Distance from `from` to the nearest point of `rect`; zero inside it.
fn distance_to_rect(from: &Position, rect: &Rect) -> f64 {
    let dx = (rect.left_top.x() - from.x())
        .max(from.x() - rect.right_bottom.x())
        .max(0.0);
    let dy = (rect.left_top.y() - from.y())
        .max(from.y() - rect.right_bottom.y())
        .max(0.0);
    dx.hypot(dy)
}

/// How far `from` is from a mining target, **the way the game measures it**.
///
/// `LuaControl::can_reach_entity` measures to the entity's collision box, not
/// its centre. For ore that is the same thing to within half a tile; for a
/// `huge-rock` (3 by 2.2) it is the difference between "1.4 from the rock"
/// and "2.9 from the rock", and the second number is what a centre rule
/// refused in run-1788551693-66583 after the bot had walked exactly where
/// the plan sent it. A boxless target falls back to the centre.
///
/// Public because the executor asks the same question after every walk
/// (`RconActuator::close_reach_gap`): "is the bot near enough to act on this?"
/// is one rule, and a second implementation of it beside this one would be a
/// second answer.
pub fn reach_distance(world: &FactorioSurface, from: &Position, target: &Position) -> f64 {
    match blocking_box_at(world, target) {
        Some(rect) => distance_to_rect(from, &rect),
        None => calculate_distance(from, target),
    }
}

/// [`within_resource_reach`], measured by [`reach_distance`].
fn within_mining_reach(
    world: &FactorioSurface,
    player: &Position,
    target: &Position,
    reach: f64,
) -> bool {
    match blocking_box_at(world, target) {
        Some(rect) => distance_to_rect(player, &rect) <= reach,
        None => within_resource_reach(player, target, reach),
    }
}

/// Where a mine's corrective walk aims, as `(goal, radius)` for
/// [`FactorioRcon::move_player_timed`].
///
/// On top of ore: the target itself at [`approach_radius`], as it always was.
/// **Beside** anything with a collision box: the annulus `(clearance, R]`
/// handed to [`approach_annulus`], where `clearance` is the sum of the box's
/// and the character's half-diagonals -- the same inner bound a `Place` walk
/// carries -- and `R` is the reach plus the box's shorter half-side, the
/// largest centre distance from which every point of the box is provably
/// within reach. A disc centred on a rock asks the pathfinder for a point
/// inside the rock, and `judge_path` refuses it before dispatch; that refusal
/// halted bot 1 in every batch of the first two live runs to chop one.
///
/// `walker` is the player doing the mining, so its own position is not
/// counted as a bystander on the ring.
fn mining_approach(
    world: &FactorioSurface,
    here: &Position,
    target: &Position,
    reach: f64,
    walker: Option<PlayerId>,
) -> (Position, f64) {
    match blocking_box_at(world, target) {
        // `approach_standing` derives the same clearance from the same box,
        // and on top of it keeps the aim off any *other* box on the ring --
        // and off the other bots standing round it.
        Some(rect) => approach_standing(
            world,
            target,
            0.0,
            reach + (rect.width() / 2.).min(rect.height() / 2.),
            Some(here),
            walker,
            // [`Approach::Inner`], not `Outer`: this walk only ever happens
            // because the bot is *already* out of reach, which is the case the
            // outer ring's margin was wrong about. A recovery walk buys
            // certainty, not ticks.
            Approach::Inner,
        ),
        None => (target.clone(), approach_radius(reach)),
    }
}

/// How far past the radius it asked for a walk actually comes to rest,
/// **measured on this game rather than reasoned about**, plus headroom.
///
/// A walk asks the game for a path ending within `slack` of a goal and the
/// mod's follower then walks it. The question the outer-ring aim turns on is
/// how far past `slack` the character ends up, because that is what has to be
/// held back from the action's own reach.
///
/// 128 probe walks were dispatched for this on seed 31337 (four headless
/// character bots, legs of 3 to 20 tiles in eight bearings, requested radii
/// 0.5, 0.75, 1, 1.5, 2, 2.5, 3 and 5), reading each resting position back off
/// `world.player(id)` — exact under `--headless`, where
/// `poll_character_bot` writes a character's position on every tick it
/// changes. **The largest overshoot in the whole set was 0.301 tiles**, which
/// is the follower's own 0.3-by-0.3 stop box and nothing else; the mean was
/// *negative* at every radius (the bot usually stops short, inside the disc it
/// asked for). Nothing came close to the `R + 1.1` this file used to guess at
/// on the strength of one 2026-08-30 walk.
///
/// 0.6 is twice that maximum. The doubling is not superstition: the stop box
/// is a box, so its worst case on a diagonal is `0.3 * sqrt(2) = 0.424`, and a
/// margin has to cover a case the probe did not happen to draw.
///
/// **It is deliberately not big enough to cover the worst case the system
/// permits.** [`PATH_ENDPOINT_SLACK`] lets [`judge_path`] dispatch a path
/// whose last waypoint is a whole tile past the requested radius (tile-centre
/// snapping), so a resting position up to `slack + 1.3` out is legal and was
/// simply never observed. Paying 1.3 tiles of margin on every walk to insure
/// against a case that did not occur in 128 trials is the wrong trade when the
/// executor re-checks reach after every walk anyway
/// (`RconActuator::walk`): the margin is sized on what happens, and the
/// re-check is what makes being wrong about it cost one short step instead of
/// a failed action.
pub const ARRIVAL_MARGIN: f64 = 0.6;

/// Which end of the annulus a walk should stop at.
///
/// The band a walk has to end in is `(min_radius, radius]`, and both ends
/// satisfy the plan equally: `Condition::AtPosition` measures distance and
/// nothing else. What differs is the price. `radius` is a *reach* — mining
/// reach, build reach, the distance an insert works from — so every tile
/// walked inside it is a tile walked for nothing, and the planner charges for
/// it: 8,200-10,200 ticks a four-bot green run, ~16% of its planned makespan
/// (see [`factorio_bot_planner::schedule::travel_ticks`]).
///
/// So the ordinary aim is [`Approach::Outer`]. [`Approach::Inner`] is what a
/// *corrective* walk asks for — the one dispatched when a bot came to rest
/// short of the reach after all — where being close matters and a handful of
/// ticks does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    /// Stop as far from the target as the reach allows, less the margin the
    /// arrival needs. The default for every planned walk.
    Outer,
    /// Stop as close to the target as the annulus allows. What the executor's
    /// corrective walk asks for, and what every walk asked for before the
    /// outer ring existed.
    Inner,
}

/// How far from `target` a walk into `(min_radius, radius]` should aim, and
/// the path radius to request, as `(aim, slack)`.
///
/// **One rule, in one place, for the model and the game alike.** The planner
/// charges `travel_ticks` to `aim` and simulates arrival there; the executor
/// asks the game for a path ending within `slack` of the point `aim` out.
/// They were separately derived before, and the model's number was the fiction
/// the walk RCA measured.
///
/// # The arithmetic
///
/// A path may end anywhere within `slack` of the goal, and the character then
/// rests up to [`ARRIVAL_MARGIN`] past that, so a resting position lies
/// between `aim - slack - ARRIVAL_MARGIN` and `aim + slack + ARRIVAL_MARGIN`.
/// [`Approach::Outer`] therefore aims at `radius - slack - ARRIVAL_MARGIN`,
/// which puts the far end of that interval exactly on `radius` — in reach, by
/// construction, with no fudge factor.
///
/// The aim is floored at the inner aim, `min_radius + slack`, so it can never
/// come out *closer* than the walk asked for before this existed: on a band
/// too narrow to hold the margin the two collapse onto each other and the
/// behaviour is the old one exactly.
///
/// `slack` halves whatever the band has left after the margin, capped at
/// [`PATH_ENDPOINT_SLACK`], for the reason [`approach_annulus`] gives: the
/// whole request disc has to fit inside the band, and a tile of room is what
/// lets the pathfinder find ground rather than hit a named coordinate.
///
/// # The degenerate cases answer as they always did
///
/// An aim of zero or less — a disc so small that the margin eats it, which is
/// every `radius <= ~1.6` — is not a ring at all, and asking for a goal a
/// fraction of a tile off the target buys nothing. Those fall back to the old
/// answer: the target itself, at [`approach_radius`]. So does
/// [`Approach::Inner`] on a plain disc, which is what that arm has always
/// meant.
///
/// A NaN bound is normalised to zero rather than propagated: a NaN fails every
/// comparison, and handing the game a NaN goal is the one answer worse than a
/// wrong one.
pub fn approach_aim(min_radius: f64, radius: f64, approach: Approach) -> (f64, f64) {
    let min_radius = if min_radius.is_nan() {
        0.0
    } else {
        min_radius.max(0.0)
    };
    let radius = if radius.is_nan() {
        0.0
    } else {
        radius.max(0.0)
    };
    let inner_slack = ((radius - min_radius).max(0.0) / 2.0).min(PATH_ENDPOINT_SLACK);
    let inner = |slack: f64| {
        if min_radius <= 0.0 {
            (0.0, approach_radius(radius))
        } else {
            (min_radius + slack, slack)
        }
    };
    if approach == Approach::Inner {
        return inner(inner_slack);
    }
    let slack = ((radius - min_radius - ARRIVAL_MARGIN).max(0.0) / 2.0).min(PATH_ENDPOINT_SLACK);
    let floor = if min_radius <= 0.0 {
        0.0
    } else {
        min_radius + slack
    };
    let aim = (radius - slack - ARRIVAL_MARGIN).max(floor);
    if aim <= 0.0 {
        return inner(inner_slack);
    }
    (aim, slack)
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

/// Which way to leave `target` when a walk has to stop short of it.
///
/// Towards `from` — the bot's own position — because the bot is already on
/// that side and the annulus is satisfied equally well all the way round: the
/// planner's `Condition::AtPosition` measures distance, never bearing. Sending
/// it round the far side would buy nothing and cost the diameter.
///
/// `+x` when there is no usable direction — an unknown player position, or a
/// bot standing exactly on the target. That is the same fixed fallback the
/// planner's own `arrival_point` uses, so the two agree about the degenerate
/// case rather than disagreeing arbitrarily.
fn approach_direction(target: &Position, from: Option<&Position>) -> Position {
    let Some(from) = from else {
        return Position::new(1.0, 0.0);
    };
    let length = calculate_distance(from, target);
    if !length.is_finite() || length <= 0.0 {
        return Position::new(1.0, 0.0);
    }
    vector_multiply(&vector_substract(from, target), 1.0 / length)
}

/// The goal and path radius to request for a walk into the annulus
/// `(min_radius, radius]` around `target`.
///
/// # Why a plain radius cannot express this
///
/// `request_path` takes a goal and a radius: a *disc*. When `min_radius` is
/// zero that is exactly what the plan asked for and this is
/// [`approach_radius`] unchanged — the goal is the target, and the game, which
/// is the only party that knows what is walkable, resolves the ring.
///
/// A positive `min_radius` makes the target the one point that can **never**
/// satisfy the condition (it is the tile the furnace being placed will stand
/// on, plus the acting character's own footprint —
/// `PlanState::placement_clearance`). No disc centred on the target excludes
/// its own centre, so the request has to be a disc that fits *inside* the
/// annulus instead.
///
/// # The arithmetic, and why the band is exact
///
/// With `slack` the requested radius and the goal placed `min_radius + slack`
/// from the target, every point the pathfinder may legally answer with lies
/// between `min_radius` and `min_radius + 2 * slack` from the target. Capping
/// `slack` at half the annulus's width makes that upper end `radius` at worst,
/// so the whole request disc is inside the annulus by construction — no
/// tolerance, no fudge factor, and nothing for a later reader to re-derive.
///
/// `slack` is additionally capped at [`PATH_ENDPOINT_SLACK`] so the bot stops
/// *near* the target rather than at the far edge of a permissive bound. For a
/// stone furnace placed with a build reach of 10 the annulus is
/// `(1.2705824974445776, 10]`: half its width is 4.36, which would leave the
/// bot up to ten tiles out and exactly on the game's own reach limit. One tile
/// of slack instead puts the walk's goal 2.27 tiles from the site and every
/// legal answer between 1.27 and 3.27 — comfortably in reach, and a point the
/// pathfinder has a whole tile of room to find rather than a single named
/// coordinate to hit.
///
/// # What absorbs the follower's stop box
///
/// The mod's walker comes to rest anywhere within a 0.3-by-0.3 box of its last
/// waypoint, so a resting position can be ~0.3 inside `min_radius`. That is
/// already paid for by how `min_radius` is derived: it is the sum of two
/// **half-diagonals**, the distance at which the two boxes can only touch at a
/// corner in the worst-case orientation. Axis-aligned — which is how a
/// character approaching from any of the four sides actually stands — clearing
/// a stone furnace needs `0.69921875 + 0.19921875 = 0.8984375`, so there is
/// 0.3721 tiles of headroom inside `min_radius` itself. The stop box fits in
/// it; see `approach_annulus_leaves_room_for_the_walkers_stop_box`.
///
/// # The offset is nudged outward when it has to be nudged at all
///
/// `target + direction * (min_radius + slack)` is a rounded sum, and the
/// distance measured back out of it — which is what every consumer goes on —
/// can come back a few ulps short. That is not cosmetic here either: the
/// guarantee above is stated as `d - slack >= min_radius`, and one ulp of
/// shortfall makes it false. The planner's `arrival_point` pays for exactly
/// this, at exactly this scale (`-16.0 + 1.2705824974445776` measuring back as
/// `1.2705824974445772`), and the correction is the same one for the same
/// reason: round *away* from the target, never towards it, because the bound
/// being protected is a minimum.
///
/// The nudge moves the goal's own coordinates rather than the multiplier. A
/// multiplier ulp is a fraction of the *radius*, which for a target far from
/// the origin is smaller than an ulp of the coordinate it lands in, so nudging
/// there could fail to move the goal at all and the loop would not terminate.
///
/// # A degenerate annulus stays degenerate
///
/// `radius <= min_radius` describes nowhere at all. `slack` is then zero and
/// the goal sits exactly on the inner circle, which is a request the
/// pathfinder will refuse — deliberately, rather than being widened here into
/// room the plan never granted. Nothing produces such a condition today:
/// `radius` is a reach (10) and `min_radius` a clearance (~1.3), and a
/// `Place` whose prototype is unknown is refused by its own `AreaFree`
/// precondition before it can be walked to.
///
/// # Which ring
///
/// [`approach_aim`] decides how far out to aim and holds the whole rule; this
/// function is the geometry that turns that distance into a point. An
/// [`Approach::Outer`] aim stops on the *outer* ring, as far from the target
/// as the reach allows, which is the whole point of the exercise — every tile
/// closer than that is walked for nothing. [`Approach::Inner`] is the old
/// behaviour, kept for corrective walks.
pub fn approach_annulus(
    target: &Position,
    min_radius: f64,
    radius: f64,
    from: Option<&Position>,
    approach: Approach,
) -> (Position, f64) {
    let (aim, slack) = approach_aim_toward(target, min_radius, radius, from, approach);
    // An aim of zero is the plain disc: the goal is the target itself, and the
    // game — the only party that knows what is walkable — resolves the ring.
    // `approach_aim` normalises a NaN bound to zero and lands here, so a NaN
    // goal cannot reach the game.
    if aim <= 0.0 {
        return (target.clone(), slack);
    }
    let direction = approach_direction(target, from);
    (aim_along(target, &direction, aim), slack)
}

/// [`approach_aim`], with the one fact it cannot see: where the bot is.
///
/// Never *further out* than the bot already is. The outer ring is where a
/// walk should stop, not somewhere a bot should be marched to: a bot already
/// standing inside the band satisfies the condition where it is, and sending
/// it back out to the ring would walk it away from its own target — 6.5 tiles
/// backwards, in the geometry of run 10's second refusal. The planner agrees
/// by construction: `travel_ticks` charges zero for a bot already in the band,
/// so a walk it emits is one it believes has ground to cover.
///
/// The clamp cannot break either bound. It only ever *reduces* the aim, and
/// never below `min_radius + slack`, the inner aim of this same request: the
/// pathfinder may answer anywhere within `slack` of the goal, and the
/// exclusion zone is the one bound that must hold.
///
/// Separate from [`approach_annulus`] so [`approach_standing`]'s sweep starts
/// on exactly the ring the goal is on, to the ulp, rather than on a distance
/// measured back out of it.
fn approach_aim_toward(
    target: &Position,
    min_radius: f64,
    radius: f64,
    from: Option<&Position>,
    approach: Approach,
) -> (f64, f64) {
    match from {
        Some(from) => approach_aim_at(
            min_radius,
            radius,
            calculate_distance(from, target),
            approach,
        ),
        None => approach_aim(min_radius, radius, approach),
    }
}

/// [`approach_aim_toward`] over a distance rather than two points, so the
/// planner — which measures the same distance and must charge for the same
/// aim — reads the rule from here instead of restating it.
///
/// `distance` is how far the bot stands from the target now. The aim is capped
/// by it for the same reason [`approach_aim_toward`] gives: a bot nearer than
/// the outer ring is not marched back out to it.
pub fn approach_aim_at(
    min_radius: f64,
    radius: f64,
    distance: f64,
    approach: Approach,
) -> (f64, f64) {
    let (aim, slack) = approach_aim(min_radius, radius, approach);
    // A degenerate disc has already collapsed to the target itself, and its
    // `slack` is a *path radius* rather than an offset into a band -- flooring
    // against it would push the aim half a tile off a target the condition
    // measures exactly. `within 0 of [60, 0]` is such a condition, and it is
    // in the suite.
    if approach == Approach::Inner || aim <= 0.0 {
        return (aim, slack);
    }
    let floor = if min_radius > 0.0 {
        min_radius + slack
    } else {
        0.0
    };
    (aim.min(distance).max(floor), slack)
}

/// The point `aim` out from `target` along the unit vector `direction`,
/// guaranteed to measure back at least `aim` from `target`.
///
/// Each pass of the loop moves both coordinates strictly further from the
/// target's, so the measured distance strictly increases and the loop
/// terminates. One pass normally suffices; the loop is here so correctness
/// does not rest on "normally". See [`approach_annulus`] for why one ulp
/// matters.
fn aim_along(target: &Position, direction: &Position, aim: f64) -> Position {
    let mut goal = vector_add(target, &vector_multiply(direction, aim));
    while calculate_distance(&goal, target) < aim {
        goal = Position::new(
            nudge_away(goal.x(), target.x()),
            nudge_away(goal.y(), target.y()),
        );
    }
    goal
}

/// How many bearings [`approach_standing`] tries on each ring before moving
/// one tile further out. Sixteen is 22.5° apart: on the tightest ring in use
/// (a stone furnace's 1.27-tile clearance plus a tile of slack) neighbouring
/// candidates are ~0.9 tiles apart, finer than any collision box the graph
/// holds, so a ring with any free arc at all has a candidate on it.
const APPROACH_BEARINGS: usize = 16;

/// The goal and path radius for a walk that must end **beside** `target`, on
/// ground the entity graph cannot prove blocked.
///
/// [`approach_annulus`] answers the geometry -- a goal in the annulus
/// `(min_radius, radius]`, on the side the bot is already on -- and knows
/// nothing about the map. That is the right split for the arithmetic and the
/// wrong one for the aim: the pathfinder is handed a *point*, and when that
/// point is inside something, the path it returns ends inside it too, and
/// [`judge_path`] refuses the walk before dispatch. Two ways that has
/// happened, both on seed 31337 on 2026-09-05:
///
/// - **Into a neighbour.** `run-1788608011-14361`, bot 4, step 16: a chop of
///   the `big-rock` at `[-19, 24.375]` with the annulus's inner bound of
///   1.66. The bot stood at `[-10.5, -22.5]`, so the aim went 2.17 tiles up
///   the line toward it, to `[-18.56, 22.24]` -- 0.7 tiles inside the
///   *other* `big-rock` at `[-17.56, 21.5]`, which the same plan had bot 3
///   chop 700 ticks later. The game returned a route ending exactly there,
///   the judge refused it, and the batch lost a bot for 4,400 ticks.
/// - **Into the target itself.** `run-1788608648-56109`, bot 1, step 36: an
///   insert at the stone furnace at `[-13, -12]`, a plain disc (`min_radius`
///   0, radius 5) whose centre *is* the furnace. The pathfinder, asked for a
///   goal inside a building, answered `failed to path find` from
///   `[-22.2, -9.7]` -- while five earlier walks to the same centre from
///   other sides had been fine. Whatever the game's rule is, a goal nobody
///   can stand on is the one input that makes it matter.
///
/// So: the inner bound is at least the clearance of whatever stands on
/// `target` (the same sum of half-diagonals a `Place` walk carries), the
/// first candidate is the annulus's own aim, and when the graph can prove
/// that one blocked the search sweeps [`APPROACH_BEARINGS`] bearings round
/// the ring -- alternating either side of the bot's bearing, so the winner is
/// the nearest free one -- then one tile further out, until the ring no
/// longer fits inside `radius` with its slack. The requested radius is the
/// annulus's slack, so the path must end at the candidate and nowhere else.
///
/// **A goal this cannot clear is returned anyway**, exactly as
/// [`approach_annulus`] would have aimed it. The graph only ever proves
/// obstruction, never clearance (see [`StandingVerdict`]), so a ring with no
/// provably-free point is not a ring with no free point, and refusing the
/// walk here would refuse it on a guess; [`judge_path`] still gets the last
/// word on the route the game actually returns.
///
/// A target with nothing on it and no inner bound -- ore, a chest's tile
/// after it was picked up, a bare position -- is a plain disc and goes out
/// untouched, so the game keeps resolving the ring for the walks it always
/// has.
///
/// # The other bots are obstacles too
///
/// The graph never sees a character, so a bot standing still on the ring is
/// a box the sweep above would aim straight into -- which is what it did in
/// `run-1788612263-27812` (see [`bystander_boxes`]). Every other bot's
/// current position is therefore a box as well, grown by the walker's own
/// stop box, and a candidate is *free* only when the graph cannot prove it
/// blocked **and** no bystander stands on it. The two are ranked, not
/// equated: the graph's boxes are buildings, rocks and water, which do not
/// move, so a candidate inside one is never returned while a free one
/// exists; a bystander is a bot that may be about to walk off, so when every
/// graph-clear candidate has one on it, the candidate **farthest from any
/// bot** is returned anyway -- a walking bot will have moved, and an idle one
/// is what the mod's stall handling (`walk_stall_describe`, and the
/// step-aside it asks for) exists to deal with. Refusing here would refuse on
/// a position that may be seconds stale.
///
/// With no bot near the ring nothing changes: the annulus's own aim is
/// returned untouched when the graph cannot fault it, and the sweep's first
/// graph-clear candidate otherwise, exactly as before bystanders were
/// consulted. `walker` names the bot being aimed, so its own position is not
/// counted against it; `None` counts every player the world knows.
///
/// # Which way the sweep walks
///
/// The first candidate is always the aim [`approach_aim`] chose, and the
/// search moves **away from it towards the other end of the band**: inward, a
/// tile at a time, for an [`Approach::Outer`] aim, and outward for an
/// [`Approach::Inner`] one. Either way the fallback of last resort is where
/// walks stopped before the outer ring existed, so a ring the graph faults
/// degrades to the old behaviour rather than to nothing.
pub fn approach_standing(
    world: &FactorioSurface,
    target: &Position,
    min_radius: f64,
    radius: f64,
    from: Option<&Position>,
    walker: Option<PlayerId>,
    approach: Approach,
) -> (Position, f64) {
    let inner = match blocking_box_at(world, target) {
        Some(rect) => {
            let character = character_footprint(world, target);
            let clearance = (rect.width() / 2.).hypot(rect.height() / 2.)
                + (character.width() / 2.).hypot(character.height() / 2.);
            min_radius.max(clearance)
        }
        None => min_radius,
    };
    let (goal, slack) = approach_annulus(target, inner, radius, from, approach);
    if inner.is_nan() || inner <= 0.0 {
        return (goal, slack);
    }
    let bystanders = bystander_boxes(world, walker);
    let is_free = |at: &Position| -> bool {
        standing_verdict(world, at) == StandingVerdict::NotProvablyBlocked
            && bystander_clearance(world, at, &bystanders) > 0.0
    };
    if is_free(&goal) {
        return (goal, slack);
    }
    let towards = approach_direction(target, from);
    let step = std::f64::consts::TAU / APPROACH_BEARINGS as f64;
    // The best candidate the graph cannot fault but a bystander stands on,
    // ranked by how far it is from the nearest bot. Only consulted when no
    // candidate is free of both.
    let mut crowded: Option<(Position, f64)> = None;
    // Where the sweep starts and which way it goes. `approach_aim` already
    // chose the ring this walk wants; the search only ever gives ground
    // towards the *other* end of the band, and stops when the ring no longer
    // fits inside it with its slack.
    let (start, stride) = match approach {
        // From the aim `approach_annulus` just chose -- read back off the goal
        // it produced, so the clamp against the bot's own distance applies to
        // the sweep too rather than only to its first candidate.
        Approach::Outer => (
            approach_aim_toward(target, inner, radius, from, approach).0,
            -1.0,
        ),
        Approach::Inner => (inner + slack, 1.0),
    };
    let mut aim = start;
    // The bound is stated per direction rather than as one two-sided test:
    // `inner + slack - slack >= inner` is not reliably true in floating point,
    // and an inward sweep that skipped its own first ring on an ulp would be
    // a silent regression of exactly the kind this file is full of.
    while match approach {
        Approach::Outer => aim >= inner + slack,
        Approach::Inner => aim + slack <= radius,
    } {
        for k in 0..APPROACH_BEARINGS {
            // 0, +1, -1, +2, -2, ...: the bot's own bearing first, then out
            // either side of it, so the first free candidate is the nearest.
            let turn = if k % 2 == 0 {
                (k / 2) as isize
            } else {
                -(k.div_ceil(2) as isize)
            } as f64
                * step;
            let (sin, cos) = turn.sin_cos();
            let direction = Position::new(
                towards.x() * cos - towards.y() * sin,
                towards.x() * sin + towards.y() * cos,
            );
            let candidate = aim_along(target, &direction, aim);
            if standing_verdict(world, &candidate) != StandingVerdict::NotProvablyBlocked {
                continue;
            }
            let clearance = bystander_clearance(world, &candidate, &bystanders);
            if clearance > 0.0 {
                return (candidate, slack);
            }
            // Strictly greater, so the first of equals -- the nearest bearing
            // -- keeps its place.
            if crowded.as_ref().is_none_or(|(_, best)| clearance > *best) {
                crowded = Some((candidate, clearance));
            }
        }
        aim += stride;
    }
    match crowded {
        Some((candidate, _)) => (candidate, slack),
        None => (goal, slack),
    }
}

/// One ulp of `value`, directed away from `from`. Ties (`value == from`) move
/// up, which is still away: any move off an exact coincidence increases the
/// separation on that axis.
fn nudge_away(value: f64, from: f64) -> f64 {
    if value >= from {
        value.next_up()
    } else {
        value.next_down()
    }
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

/// The mod's wording for a walk whose current leg stopped making progress.
///
/// Owned by `mods/BotBridge/control.lua`'s walk follower, which fails a walk
/// with `ERROR: stuck while walking, leg <i> of <n> made no progress for <t>
/// ticks from (x/y) to (x/y)` once a leg outlives its timeout.
///
/// Both halves are matched, and the second one is the point. `stuck while
/// walking` prefixes every stuck verdict this mod has ever produced, including
/// two that are *answers*: `the destination is unreachable ... found no path`
/// and `gave up after 4 re-paths on one walk` both come from a build whose
/// pathfinder had already searched. `made no progress` is specifically the
/// stall -- nothing was searched, nothing was learned about reachability -- and
/// that is the only walk failure worth putting to the game again. A save
/// carrying an in-flight walk from an older build therefore does not get its
/// definitive answer retried into oblivion.
const WALK_STUCK: &str = "stuck while walking";
const WALK_LEG_STALLED: &str = "made no progress";

/// Whether a failed walk's verdict says a leg stalled, as opposed to saying
/// anything about whether the destination can be reached.
///
/// Matched on the mod's text because there is nothing else to match on -- the
/// verdict reaches Rust as one opaque string in [`crate::errors::RconError`] --
/// the same way [`mine_reports_target_gone`] is, and split out as a free
/// function so a reword in `control.lua` fails a test in this crate instead of
/// quietly retiring the retry below.
pub fn walk_reports_stalled_leg(message: &str) -> bool {
    message.contains(WALK_STUCK) && message.contains(WALK_LEG_STALLED)
}

/// The clause BotBridge appends to a stalled walk, naming **what was in the
/// way** at the instant the leg gave up.
///
/// # Why the mod has to be the one to look
///
/// A stall used to report where the bot stood and where it was steering and
/// nothing at all about why it stopped, so the cause was never established:
/// [`FactorioRcon::move_player_timed`] answers a stall by asking for a fresh
/// path, the fresh path usually works, and the bot walks around whatever it
/// was. By the time anything else could look, the obstruction is behind it.
/// The same class of stall then recurs run after run with no record of a
/// single cause. `control.lua`'s `walk_stall_cause` runs one
/// `find_entities_filtered` at the moment of the stall, which is the only
/// moment the answer exists.
///
/// # The grammar
///
/// ```text
/// ... to (-10.5/-18.5), moved 0.02 tiles, blocked at (-10.437/-18.702)
///     by <cause> on tile '<name>'
/// ```
///
/// with `(+N more)` when the probe box held more than the one thing it named,
/// and `blocker unknown (probe failed: <lua error>)` in place of `blocked at`
/// when the probe itself raised. `<cause>` is one of `character #3 (mining)`,
/// `character (no player)`, `entity 'stone-furnace' (ours)`, `tree 'tree-02'`,
/// `rock 'rock-huge'`, `cliff 'cliff'` or `nothing findable`.
///
/// **The whole clause sits after both coordinates**, so
/// [`walk_reports_stalled_leg`] still matches and `walk_endpoints`
/// (`crates/scripting_lua/src/globals/record.rs`) still reads `from` and
/// `destination` off the same string. The cause therefore reaches
/// `events.jsonl` for free, inside `EventKind::WalkSettled`'s `error` --
/// `WalkFailure` has no structured field for it yet, and giving it one is a
/// one-line change in `crates/core/src/record/mod.rs` plus a one-line change
/// in `classify_walk_failure`; `tools/run_analysis.py` reads it out of the
/// text in the meantime.
///
/// Both halves are pinned by `a_stalls_cause_is_read_from_the_mods_own_wording`
/// below, which runs the mod's own function against a stub game and parses its
/// real output with this parser -- the same reason
/// [`walk_reports_stalled_leg`]'s wording is read out of `control.lua` rather
/// than described.
const WALK_BLOCKED_AT: &str = "blocked at ";
const WALK_BLOCKED_BY: &str = " by ";
const WALK_ON_TILE: &str = " on tile '";
const WALK_MOVED: &str = ", moved ";
const WALK_MOVED_UNIT: &str = " tiles";
const WALK_LEG_OF_A: &str = " tiles of a ";
const WALK_LEG_UNIT: &str = "-tile leg";
const WALK_READ_BACK: &str = "walking_state read back walking=";
const WALK_PROBE_FAILED: &str = "blocker unknown (probe failed: ";
const WALK_NOTHING_FINDABLE: &str = "nothing findable";

/// What kind of thing a stalled walk was pressed against.
///
/// **Four of these are not degrees of the same answer.**
/// [`WalkBlockerKind::Character`] is the only blocker that moves on its own, so
/// it is the only one where "wait and ask again" is a strategy rather than a
/// hope -- and its `activity` decides even that, because
/// `step_aside_from_footprint` steers only a blocker that is neither walking
/// nor mining, so a `mining` blocker is one nothing is going to move.
/// [`WalkBlockerKind::Nothing`] says the probe looked and the tile was clear,
/// which points at the pathfinder rather than at the world.
/// [`WalkBlockerKind::ProbeFailed`] says the game raised while being asked, and
/// [`WalkBlockerKind::Unknown`] says *this build could not read what the mod
/// said* -- deliberately distinct, because collapsing them is how a reworded
/// mod turns into a silently empty column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkBlockerKind {
    /// Another bot's character. Transient: it may walk off on its own, and
    /// `activity` says whether anything is going to ask it to.
    Character,
    /// A built entity -- `ours` says whether this run put it there, which is
    /// the difference between the plan contradicting itself and the map.
    Entity,
    /// A tree. Scenery, and clearable.
    Tree,
    /// A rock (`simple-entity`). Scenery, and clearable.
    Rock,
    /// A cliff. Not clearable without explosives, and a fact about that tile.
    Cliff,
    /// The probe looked and found nothing solid. **An answer, not an absence**
    /// -- it means the obstruction was already gone, or there never was one and
    /// the path itself was the problem.
    Nothing,
    /// `walk_stall_cause` raised inside `on_tick` and the `pcall` caught it.
    /// `detail` carries the Lua error.
    ProbeFailed,
    /// The mod said something this build's grammar does not cover. `detail`
    /// carries it verbatim so a reader is never left with an empty answer.
    Unknown,
}

/// What a stalled walk was pressed against, read out of the mod's own clause.
///
/// Carried *beside* the error string rather than instead of it, for the same
/// reason `WalkFailure` is: the string is what a person reads, this is what a
/// query groups by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkBlocker {
    pub kind: WalkBlockerKind,
    /// The bot standing in the way. `None` for a character nobody is driving --
    /// a disconnected bot leaves its character behind and it is still solid.
    pub player: Option<PlayerId>,
    /// What that bot was doing: `walking`, `mining` or `idle`.
    pub activity: Option<String>,
    /// The prototype name of the entity, tree, rock or cliff.
    pub name: Option<String>,
    /// Whether a blocking entity is on the acting bot's own force. `None` for
    /// everything that is not [`WalkBlockerKind::Entity`].
    pub ours: Option<bool>,
    /// **Observed.** Where the probe looked: one step ahead of where the
    /// character actually stood, not the waypoint it was steering to.
    pub at: Option<Position>,
    /// The tile under the probe. Present whether or not an entity was found,
    /// because the ground is the answer when nothing is standing on it.
    pub tile: Option<String>,
    /// How many *other* blocking things shared the probe box.
    pub others: u32,
    /// **Observed.** How far the character actually travelled on the leg that
    /// gave up.
    ///
    /// `made no progress for <t> ticks` used to be a **leg timeout**, counted
    /// from the tick the leg began, so this was the only number that said
    /// whether the character was wedged. The mod's clock is a progress clock
    /// now (`WALK_STALL_TICKS`: ticks since the distance to the waypoint last
    /// shrank), so the wording and this number finally agree -- and together
    /// with [`WalkBlocker::leg_tiles`] they say *where on the leg* the
    /// character stopped. Run 9 stopped 1.04 tiles into a 1.42-tile leg on
    /// open ground: it had walked, and then the game held it.
    ///
    /// `None` when the mod could not say -- a walk still in flight across a
    /// save written by a build that did not stamp the leg's origin, the same
    /// case the `w.stuck == true` fallback covers.
    pub moved_tiles: Option<f64>,
    /// **Observed.** The straight-line length of the leg that gave up, from
    /// where it began to the waypoint. Beside `moved_tiles` this is what turns
    /// "moved 1.04 tiles" from a bare distance into a position on the leg.
    /// `None` from a mod that did not say.
    pub leg_tiles: Option<f64>,
    /// **Observed.** What `player.walking_state.walking` read back at the
    /// instant of the stall -- the engine's own value, one simulated tick
    /// after the follower last set it. `Some(true)` means the game kept the
    /// steer and did not move the character; `Some(false)` means something
    /// overwrote the steer between the follower's write and the next tick.
    /// Those are two different mechanisms, and run 9 could not tell them
    /// apart. `None` from a mod that did not say.
    pub engine_walking: Option<bool>,
    /// The unparsed cause text, for [`WalkBlockerKind::ProbeFailed`] and
    /// [`WalkBlockerKind::Unknown`].
    pub detail: Option<String>,
}

impl WalkBlocker {
    fn of(kind: WalkBlockerKind) -> Self {
        Self {
            kind,
            player: None,
            activity: None,
            name: None,
            ours: None,
            at: None,
            tile: None,
            others: 0,
            moved_tiles: None,
            leg_tiles: None,
            engine_walking: None,
            detail: None,
        }
    }

    /// `after 1.04 tiles`, or `after 1.04 of 1.42 tiles` when the mod said how
    /// long the leg was.
    fn moved_clause(&self) -> Option<String> {
        let moved = self.moved_tiles?;
        Some(match self.leg_tiles {
            Some(leg) => format!("after {moved} of {leg} tiles"),
            None => format!("after {moved} tiles"),
        })
    }

    /// A short, stable rendering for a line somebody reads while the run is
    /// happening. The full clause is still in the error string.
    pub fn summary(&self) -> String {
        match self.kind {
            WalkBlockerKind::Character => match (self.player, self.activity.as_deref()) {
                (Some(player), Some(activity)) => format!("bot #{player}, {activity}"),
                (Some(player), None) => format!("bot #{player}"),
                // The mod says `character (no player)` with no activity for a
                // character nobody is driving, and `character #N (doing)`
                // otherwise -- so an activity with no id means the id itself
                // did not parse, which is a different thing and must not read
                // as "nobody is driving it".
                (None, Some(activity)) => format!("an unnamed character, {activity}"),
                (None, None) => "an undriven character".to_string(),
            },
            WalkBlockerKind::Entity => match (self.name.as_deref(), self.ours) {
                (Some(name), Some(true)) => format!("our own {name}"),
                (Some(name), _) => name.to_string(),
                (None, _) => "an entity".to_string(),
            },
            WalkBlockerKind::Tree | WalkBlockerKind::Rock | WalkBlockerKind::Cliff => self
                .name
                .clone()
                .unwrap_or_else(|| format!("{:?}", self.kind).to_lowercase()),
            WalkBlockerKind::Nothing => {
                let mut parts = vec!["nothing".to_string()];
                if let Some(tile) = self.tile.as_deref() {
                    parts.push(format!("on {tile}"));
                }
                if let Some(moved) = self.moved_clause() {
                    parts.push(moved);
                }
                match self.engine_walking {
                    Some(true) => parts.push("game still walking".to_string()),
                    Some(false) => parts.push("game not walking".to_string()),
                    None => {}
                }
                parts.join(", ")
            }
            WalkBlockerKind::ProbeFailed => "unknown -- the probe raised".to_string(),
            WalkBlockerKind::Unknown => {
                self.detail.clone().unwrap_or_else(|| "unknown".to_string())
            }
        }
    }
}

/// One `(x/y)` pair as the mod's `coord()` writes it.
///
/// Nothing here rounds. The mod already rounded this one -- to a thousandth of
/// a tile, and *before* building the box it queried, so the number in the
/// message is the number the game was asked about. Rounding again on this side
/// would put the point somewhere the probe never looked, which is the same
/// mistake `walk_endpoints` refuses to make with the two observed positions
/// beside it.
fn parse_probe_coord(text: &str) -> Option<Position> {
    let text = text.trim();
    let inner = text.strip_prefix('(')?.strip_suffix(')')?;
    let (x, y) = inner.split_once('/')?;
    Some(Position::new(
        x.trim().parse().ok()?,
        y.trim().parse().ok()?,
    ))
}

/// The contents of the first `'...'` in `text`.
fn parse_quoted(text: &str) -> Option<String> {
    let (_, rest) = text.split_once('\'')?;
    let (name, _) = rest.split_once('\'')?;
    Some(name.to_string())
}

/// Reads BotBridge's blocker clause out of a stalled walk's error text.
///
/// `None` means **the message carries no clause at all** -- an archived run
/// from a build before the probe existed, or a walk failure that is not a
/// stall. It never means "there was nothing in the way": that is
/// [`WalkBlockerKind::Nothing`], and it never means "this build cannot read
/// it": that is [`WalkBlockerKind::Unknown`]. Three answers, because a reader
/// who cannot tell them apart cannot tell a fixed run from a broken parser.
pub fn walk_blocker(message: &str) -> Option<WalkBlocker> {
    // Read before the branch: the mod emits it whether or not the probe
    // itself got an answer, because it is measured by the follower rather
    // than asked of the game.
    let moved_tiles = parse_moved_tiles(message);
    if let Some((_, why)) = message.split_once(WALK_PROBE_FAILED) {
        let mut blocker = WalkBlocker::of(WalkBlockerKind::ProbeFailed);
        blocker.detail = Some(why.trim_end().trim_end_matches(')').trim().to_string());
        blocker.moved_tiles = moved_tiles;
        blocker.leg_tiles = parse_leg_tiles(message);
        blocker.engine_walking = parse_engine_walking(message);
        return Some(blocker);
    }
    let (_, tail) = message.split_once(WALK_BLOCKED_AT)?;
    let (at, tail) = tail.split_once(WALK_BLOCKED_BY)?;
    let at = parse_probe_coord(at);
    let (cause, tile) = match tail.split_once(WALK_ON_TILE) {
        Some((cause, rest)) => (cause, rest.split_once('\'').map(|(t, _)| t.to_string())),
        None => (tail, None),
    };
    // `(+N more)` trails the tile clause when the game answered `get_tile` and
    // the cause otherwise, so it is read off the whole tail and taken off the
    // cause either way.
    let others = parse_more_suffix(tail);
    let cause = cause.split(" (+").next().unwrap_or(cause).trim();

    let mut blocker = if let Some(rest) = cause.strip_prefix("character") {
        let rest = rest.trim();
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Character);
        if rest != "(no player)" {
            blocker.player = rest
                .strip_prefix('#')
                .map(|digits| digits.trim_start())
                .map(|digits| {
                    digits
                        .split(|c: char| !c.is_ascii_digit())
                        .next()
                        .unwrap_or_default()
                })
                .and_then(|digits| digits.parse().ok());
            blocker.activity = rest
                .split_once('(')
                .and_then(|(_, rest)| rest.split_once(')'))
                .map(|(activity, _)| activity.trim().to_string());
        }
        blocker
    } else if let Some(rest) = cause.strip_prefix("entity ") {
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Entity);
        blocker.name = parse_quoted(rest);
        blocker.ours = Some(rest.contains("(ours)"));
        blocker
    } else if let Some(rest) = cause.strip_prefix("tree ") {
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Tree);
        blocker.name = parse_quoted(rest);
        blocker
    } else if let Some(rest) = cause.strip_prefix("rock ") {
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Rock);
        blocker.name = parse_quoted(rest);
        blocker
    } else if let Some(rest) = cause.strip_prefix("cliff ") {
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Cliff);
        blocker.name = parse_quoted(rest);
        blocker
    } else if cause.starts_with(WALK_NOTHING_FINDABLE) {
        WalkBlocker::of(WalkBlockerKind::Nothing)
    } else {
        // **Not `None`.** The mod said something; this build does not know the
        // word. Saying so, with the words, is the difference between a reader
        // seeing a new wording and a reader seeing an empty column -- which is
        // exactly how `classify_walk_failure` once lost 19 of 20 failures to
        // `other`.
        let mut blocker = WalkBlocker::of(WalkBlockerKind::Unknown);
        blocker.detail = Some(cause.to_string());
        blocker
    };
    blocker.at = at;
    blocker.tile = tile;
    blocker.others = others;
    blocker.moved_tiles = moved_tiles;
    blocker.leg_tiles = parse_leg_tiles(message);
    blocker.engine_walking = parse_engine_walking(message);
    Some(blocker)
}

/// `, moved <d> tiles` out of the stall wording.
///
/// `None` for the mod's own `moved unknown tiles` as well as for a message
/// that has no such clause: both mean "not measured", and neither may be read
/// as zero -- reporting a wedged bot for a leg nobody timed is the same class
/// of confident wrong answer as naming ore as a blocker.
fn parse_moved_tiles(message: &str) -> Option<f64> {
    let (_, tail) = message.split_once(WALK_MOVED)?;
    let (value, _) = tail.split_once(WALK_MOVED_UNIT)?;
    value.trim().parse().ok()
}

/// `moved <d> tiles of a <len>-tile leg` -- the `<len>`.
fn parse_leg_tiles(message: &str) -> Option<f64> {
    let (_, tail) = message.split_once(WALK_LEG_OF_A)?;
    let (value, _) = tail.split_once(WALK_LEG_UNIT)?;
    value.trim().parse().ok()
}

/// `walking_state read back walking=<true|false>` -- the boolean. Anything
/// else the mod wrote there (`unreadable`) is `None`.
fn parse_engine_walking(message: &str) -> Option<bool> {
    let (_, tail) = message.split_once(WALK_READ_BACK)?;
    let word = tail
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    match word {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The digits at the start of `text`, or zero. Used for the `(+N more)` count,
/// where a missing or unreadable number means "no others named" rather than an
/// error: the count is a hint about the probe box, not a fact anything branches
/// on.
fn parse_leading_u32(text: &str) -> u32 {
    text.trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or_default()
        .parse()
        .unwrap_or(0)
}

/// `(+N more)` wherever it sits in the clause.
fn parse_more_suffix(tail: &str) -> u32 {
    tail.split_once(" (+")
        .map(|(_, rest)| parse_leading_u32(rest))
        .unwrap_or(0)
}

/// [`walk_reports_stalled_leg`] against the failure a walk actually comes back
/// with.
///
/// Two conditions, and the [`Dispatch`] one is load-bearing in both directions:
///
/// - [`Dispatch::NotDispatched`] is never retried. Everything raised before the
///   walk is sent -- a [`judge_path`] refusal, a `failed to path find`, an
///   unknown player -- is a fact about the ground or the map that asking again
///   does not change. That is what keeps the pathfinder's definitive answer
///   definitive.
/// - [`Dispatch::NoVerdict`] is never retried either, and this one is a safety
///   property rather than an honesty one: the game took that walk and never
///   said how it ended, so the bot may still be walking. A second dispatch
///   would put two walks on one character.
///
/// Only [`Dispatch::Refused`] -- the game answered, and the answer was a
/// stalled leg -- is retried.
fn is_stalled_walk(failure: &ActionFailure) -> bool {
    failure.dispatch == Dispatch::Refused
        && failure
            .error
            .downcast_ref::<RconError>()
            .is_some_and(|refused| walk_reports_stalled_leg(&refused.message))
}

/// The path radius [`FactorioRcon::move_player_timed`] falls back to when the
/// route the game returned ends somewhere nobody can stand.
///
/// Half a tile: the pathfinder puts waypoints on tile centres, so any goal is
/// within `sqrt(2)/2` of one and a request this tight still has an answer,
/// while the answer can no longer be a different point of the disc.
const TIGHT_PATH_RADIUS: f64 = 0.5;

/// The radius to ask for again after `failure`, if asking again is worth
/// anything -- `None` when it is not.
///
/// The one failure this answers is [`RconWalkEndsWhereNobodyCanStand`], and
/// only when the goal *itself* is not the problem: the graph cannot prove the
/// goal blocked, the request left the game room to stop elsewhere in the
/// disc, and the game used that room to stop inside a building. Tightening
/// the radius to [`TIGHT_PATH_RADIUS`] takes the room away. A goal the graph
/// can prove blocked gets `None` -- a tighter request for it would be refused
/// for the same reason, one path request later -- and so does a request that
/// was already tight, or any other failure at all.
fn tightened_radius(
    world: &FactorioSurface,
    goal: &Position,
    radius: Option<f64>,
    failure: &ActionFailure,
) -> Option<f64> {
    failure
        .error
        .downcast_ref::<RconWalkEndsWhereNobodyCanStand>()?;
    if radius.unwrap_or(DEFAULT_PATH_RADIUS) <= TIGHT_PATH_RADIUS {
        return None;
    }
    match standing_verdict(world, goal) {
        StandingVerdict::NotProvablyBlocked => Some(TIGHT_PATH_RADIUS),
        StandingVerdict::Blocked { .. } => None,
    }
}

/// How many times one [`FactorioRcon::move_player_timed`] call may be put to the
/// game before its stall is reported as a failure.
///
/// # Why the budget is a count and not a clock
///
/// Every retry is spent on an attempt the game *answered*, because
/// [`is_stalled_walk`] refuses to retry anything else. So an attempt cannot
/// both consume [`ACTION_RESULT_DEADLINE`] and be retried: a walk the game goes
/// quiet on is [`Dispatch::NoVerdict`] and ends the loop. What a retry actually
/// costs is one path request round trip plus the mod's own stall clock
/// (`WALK_STALL_TICKS`, 60 ticks without the leg getting any closer to its
/// waypoint) -- seconds, not minutes. Three attempts therefore bound a
/// hopelessly stuck walk to the same order the mod's re-path budget did, which
/// is the property that must not regress: being told in seconds instead of
/// after the executor's 360-second deadline is the whole point.
///
/// # Why three
///
/// The mod's `WALK_REPATH_LIMIT` was 4 for a *weaker* retry -- it re-pathed to
/// the last node of the stale path at a radius of 0.5, judged against nothing.
/// This one re-asks for the caller's own goal at the caller's own radius and
/// puts the answer through [`judge_path`], so it both succeeds more often and
/// fails faster when it is going to fail: the second attempt of an unreachable
/// destination is refused *before* dispatch rather than walked and stalled.
/// The observed need across the 23 archived runs was one re-path; 3 keeps the
/// headroom the mod's note argued for -- the world changing again while the new
/// route is being walked -- without paying for four stalls.
const WALK_ATTEMPTS: u32 = 3;

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
    /// `game.speed` as this process last set or read it, as `f64` bits.
    ///
    /// Every wall-clock deadline in this file is sized for a world running at
    /// normal speed; at `game.speed = 10` a craft that needs 60 game-seconds
    /// is done in six wall seconds, and a deadline that still waits six
    /// minutes for it would hide a lost reply for ten times longer than it
    /// should. See [`scale_deadline`]. Initialised to `1.0` and updated by
    /// [`FactorioRcon::set_game_speed`] and [`FactorioRcon::game_speed`].
    speed: Arc<std::sync::atomic::AtomicU64>,
}

/// A wall-clock deadline sized for normal speed, rescaled to `speed`.
///
/// Divides: at speed 10 the game delivers its ticks ten times sooner, so the
/// same number of game ticks fits in a tenth of the wall clock. A speed below
/// one lengthens the deadline for the same reason. A speed that is not
/// positive is treated as normal rather than dividing by it, and the result
/// never drops below ten seconds -- RCON round trips and the mod's own
/// reply latency do not speed up with the game.
fn scale_deadline(base: Duration, speed: f64) -> Duration {
    const FLOOR: Duration = Duration::from_secs(10);
    if speed.is_nan() || speed <= 0.0 || speed == 1.0 {
        return base;
    }
    Duration::from_secs_f64(base.as_secs_f64() / speed).max(FLOOR)
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
            speed: Arc::new(std::sync::atomic::AtomicU64::new(1.0f64.to_bits())),
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
            speed: Arc::new(std::sync::atomic::AtomicU64::new(1.0f64.to_bits())),
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

    /// Turns the mod's world-state sampling on, and reports the game tick it
    /// took effect at.
    ///
    /// The cadence deliberately does *not* live here. A sample line carries
    /// the tick it was written at, and only the game knows that: an RCON
    /// command arrives whenever it arrives, so a caller driving the cadence
    /// from out here could only stamp its own loop counter -- and a counter
    /// cannot fail to produce a contiguous, plausible sequence, even when the
    /// game skipped a beat. This is a toggle, not a shutter.
    ///
    /// The returned tick is the game's own (BotBridge's `stamp_tick`), so a
    /// caller can check that every sample it later reads was written at or
    /// after the moment sampling began, rather than trusting that it was.
    ///
    /// `run_id` tags the session with an opaque identifier the mod stamps onto
    /// every sample line and never interprets. Pass one when the samples will
    /// have to be matched against something produced elsewhere in the same run
    /// -- a replay document, say -- so a consumer can check the two came from
    /// the same session instead of trusting that ticks lining up means they
    /// did. Pass `None` when nothing needs correlating: the lines then carry
    /// no `run` key at all, and a consumer that finds none knows it cannot
    /// tell, which is the honest answer. It never inherits the previous run's
    /// id.
    ///
    /// This call is made on every recorded run, and that is what makes the run
    /// observable: the mod's `sample_force` (300 ticks) and `sample_bots` (60
    /// ticks) beats are both gated on an active session, so `samples.jsonl` --
    /// research, production, power, bot inventories -- exists only while one
    /// is running.
    ///
    /// Until 2026-09-02 this toggle also drove per-camera screenshots. Those
    /// are gone: run `run-1788365280-15443` wrote 2,164 JPEGs / **947 MB** of
    /// them against **290 MB** for the same 45 minutes of video, and
    /// `game.take_screenshot` renders synchronously inside the game loop where
    /// the video grabber reads a frame the GPU already drew. Video is the
    /// visual record; the session is what survived.
    ///
    /// Taken by value rather than as `Option<&str>` because this `impl` is
    /// `#[automock]`ed and mockall cannot elide a lifetime inside a generic.
    pub async fn sampling_start(&self, run_id: Option<String>) -> Result<Option<u64>> {
        sampling_verdict(
            self.remote_call_timed("sampling_start", sampling_args(run_id.as_deref()))
                .await?,
        )
    }

    /// Turns the mod's sampling off, reporting the game tick it stopped at.
    /// Samples already written stay on disk.
    pub async fn sampling_stop(&self) -> Result<Option<u64>> {
        sampling_verdict(self.remote_call_timed("sampling_stop", vec![]).await?)
    }

    /// Asks the server to write the world out as `<instance>/saves/<name>.zip`,
    /// reporting the game tick the request was stamped at.
    ///
    /// **Returning is not finishing.** The engine writes at the end of a tick
    /// and the reply comes back long before the bytes land, so a caller that
    /// treats this as done is describing a file that may not exist yet. Wait
    /// for it with [`crate::record::savepoint::await_save`], which watches the
    /// artefact rather than the log.
    ///
    /// Unlike [`FactorioRcon::server_save`] this always names a file, and so
    /// can never overwrite the `level.zip` the instance is running on.
    pub async fn savepoint(&self, name: &str) -> Result<Option<u64>> {
        sampling_verdict(
            self.remote_call_timed("savepoint", vec![str_to_lua(name)])
                .await?,
        )
    }

    /// Drops the mod's run-scoped `storage` -- walk and mining state, craft and
    /// research waiters, the sampling session -- and reports what it dropped as
    /// the mod's own JSON.
    ///
    /// Called when resuming from a savepoint, where the loaded world arrives
    /// carrying the previous run's in-flight state. Action ids are minted
    /// `% 1000` from zero every run, so a leftover waiter settles a *different*
    /// action in the new run rather than merely lingering. See
    /// `rcon_session_reset` in `mods/BotBridge/control.lua`.
    ///
    /// Harmless on a fresh world, where it drops nothing and says so.
    ///
    /// The reply is judged, not merely returned. Factorio writes
    /// `Cannot execute command. Error: ...` into the reply *body* when the mod
    /// raises or does not define the function -- so a caller that took the body
    /// as the answer would report "cleared the mod's run-scoped state: Cannot
    /// execute command" and read as a success. That is the exact shape of the
    /// bug [`expect_silence`] exists for; here the counts are the payload, so
    /// the check is that the payload is the JSON object the mod writes.
    pub async fn session_reset(&self) -> Result<String> {
        let (lines, _tick) = self.remote_call_timed("session_reset", vec![]).await?;
        let reply = lines.unwrap_or_default().join("");
        let trimmed = reply.trim();
        if !trimmed.starts_with('{') {
            return Err(RconUnexpectedOutput {
                output: if trimmed.is_empty() {
                    "session_reset answered nothing; the mod is older than this feature".to_string()
                } else {
                    trimmed.to_string()
                },
            }
            .into());
        }
        Ok(trimmed.to_string())
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

    /// Save the current game on server, **over the save it is running on**.
    ///
    /// `/server-save` with no name overwrites the file the instance was
    /// started from -- `saves/level.zip` for every instance this project
    /// starts, which is the map every measurement is taken against. Use
    /// [`FactorioRcon::savepoint`] instead for anything that wants to keep a
    /// world; this exists for the hand-operated "flush what I just did to
    /// disk" endpoint and is the only caller.
    pub async fn server_save(&self) -> Result<()> {
        self.send("/server-save").await?;
        Ok(())
    }

    /// Asks the mod to create `count` server-side character bots, ids
    /// `1..=count`, and answers with every id that now has one -- created or
    /// kept from a resumed save. The mod refuses when a player is connected:
    /// a run is all clients or all characters.
    pub async fn spawn_bots(&self, count: u8) -> Result<Vec<u8>> {
        let lines = self
            .remote_call("spawn_bots", vec![count.to_string()])
            .await?
            .ok_or_else(|| miette!("spawn_bots: the mod answered nothing"))?;
        let reply = lines.join("");
        if let Some(err) = reply.strip_prefix("Error: ") {
            return Err(miette!("{err}"));
        }
        // `helpers.table_to_json({})` yields `{}` for an empty list, so each
        // half is read as a `Value` and an object counts as empty.
        #[derive(Deserialize)]
        struct Reply {
            spawned: Value,
            kept: Value,
        }
        let parsed: Reply = serde_json::from_str(&reply)
            .into_diagnostic()
            .wrap_err_with(|| format!("spawn_bots: unreadable reply: {reply}"))?;
        let ids =
            |v: Value| -> Vec<u8> { serde_json::from_value::<Vec<u8>>(v).unwrap_or_default() };
        let mut all = ids(parsed.spawned);
        all.extend(ids(parsed.kept));
        all.sort_unstable();
        Ok(all)
    }

    /// Sets `game.speed` and remembers it for [`scale_deadline`].
    pub async fn set_game_speed(&self, speed: f64) -> Result<()> {
        let lines = self
            .remote_call("set_game_speed", vec![speed.to_string()])
            .await?
            .ok_or_else(|| miette!("set_game_speed: the mod answered nothing"))?;
        let got: f64 = lines
            .join("")
            .trim()
            .parse()
            .into_diagnostic()
            .wrap_err_with(|| format!("set_game_speed: unreadable reply: {lines:?}"))?;
        self.speed
            .store(got.to_bits(), std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Reads `game.speed` from the game and remembers it.
    pub async fn game_speed(&self) -> Result<f64> {
        let lines = self
            .remote_call("game_speed", vec![])
            .await?
            .ok_or_else(|| miette!("game_speed: the mod answered nothing"))?;
        let got: f64 = lines
            .join("")
            .trim()
            .parse()
            .into_diagnostic()
            .wrap_err_with(|| format!("game_speed: unreadable reply: {lines:?}"))?;
        self.speed
            .store(got.to_bits(), std::sync::atomic::Ordering::Relaxed);
        Ok(got)
    }

    /// Stops or restarts the game clock (`game.tick_paused`), answering with
    /// the tick it happened at.
    ///
    /// The game keeps serving RCON while paused, so the questions a plan asks
    /// (`can_place_entities`, buffer contents) still get answered -- against a
    /// world that is not moving. What does **not** move is anything that needs
    /// a later tick to answer: a path request (`request_path`) is resolved by
    /// the game on a subsequent tick and would wait forever, so nothing that
    /// probes a path may run inside a paused window. `goal.plan` refreshes the
    /// buffers (which re-probes benched bots) *before* it pauses for exactly
    /// that reason.
    ///
    /// The tick comes from the mod's stamp on the reply rather than a second
    /// round trip, and is remembered as the last known tick so a record entry
    /// written straight after is stamped with the moment the clock stopped
    /// rather than whatever was last seen.
    pub async fn set_tick_paused(&self, paused: bool) -> Result<u64> {
        let (lines, tick) = self
            .remote_call_timed("set_tick_paused", vec![paused.to_string()])
            .await?;
        let got = lines
            .unwrap_or_default()
            .join("")
            .trim()
            .parse::<bool>()
            .into_diagnostic()
            .wrap_err("set_tick_paused: unreadable reply")?;
        if got != paused {
            return Err(miette!(
                "set_tick_paused: asked for {paused}, the game reports {got}"
            ));
        }
        tick.ok_or_else(|| miette!("set_tick_paused: the mod answered without a tick stamp"))
    }

    /// The speed the deadlines are scaled by: the last value set or read,
    /// normal speed until then.
    pub fn speed_factor(&self) -> f64 {
        let s = f64::from_bits(self.speed.load(std::sync::atomic::Ordering::Relaxed));
        if s > 0.0 { s } else { 1.0 }
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
    /// The wait is sized from `expected_ticks`, the plan's own duration for the
    /// research, by [`FactorioRcon::sized_deadline`]; it used to be the flat
    /// `ACTION_RESULT_DEADLINE` of 360 wall-clock seconds, and
    /// run-1788559688-08406 declared `research logistic-science-pack` -- 75
    /// units of 5 s in one lab, 375 s -- lost while the lab was still working,
    /// which the supervisor then replanned around. Research is the one action
    /// kind whose real duration is set by the factory rather than by the bot
    /// -- lab count, science supply, speed modules -- so it is also the one
    /// most able to outlast any deadline honestly. A research that does so is
    /// reported [`Dispatch::NoVerdict`], which is the correct claim (the game
    /// took the command and we stopped listening) but is not the same as a
    /// failure.
    pub async fn research_timed(
        &self,
        world: &Arc<FactorioSurface>,
        technology_name: &str,
        expected_ticks: u32,
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
        self.sleep_for_action_result_until(
            world,
            action_id,
            dispatched,
            Self::sized_deadline(expected_ticks),
        )
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
        action_id: ActionId,
        dispatched: Option<u64>,
    ) -> Result<ActionTicks, ActionFailure> {
        self.sleep_for_action_result_until(
            world,
            action_id,
            dispatched,
            scale_deadline(ACTION_RESULT_DEADLINE, self.speed_factor()),
        )
        .await
    }

    /// How long to wait for a hand craft's verdict.
    ///
    /// `energy` is the recipe's own seconds per craft; the queue also crafts every
    /// missing intermediate, which vanilla's early recipes make at most a few
    /// times the product's own time (a science pack is 5 s and its gear 0.5 s;
    /// an inserter is 0.5 s over ~2.5 s of parts). Three times the product's
    /// time, plus a minute, covers that tree and a game running slower than the
    /// clock; and it is never *less* than [`ACTION_RESULT_DEADLINE`], so a short
    /// craft keeps the deadline every other action has. Wall clock, not ticks:
    /// the wait is measured with `Instant`.
    fn craft_deadline(energy: f64, count: u32) -> Duration {
        let ticks = (energy.max(0.0) * f64::from(count) * 60.0).round();
        Self::sized_deadline(ticks.min(f64::from(u32::MAX)) as u32)
    }

    /// How long to wait for the verdict of an action whose nominal duration
    /// the plan already knows: three times that duration on the wall clock,
    /// plus a minute, never less than [`ACTION_RESULT_DEADLINE`]. The factor
    /// covers a lab short of packs or a game running below 1x; the floor keeps
    /// a short action's deadline the one every other action has. One rule for
    /// crafts and research, so the next long action kind is a one-line change.
    fn sized_deadline(expected_ticks: u32) -> Duration {
        let expected = Duration::from_secs_f64(f64::from(expected_ticks) / 60.0 * 3.0 + 60.0);
        expected.max(ACTION_RESULT_DEADLINE)
    }
    /// [`FactorioRcon::sleep_for_action_result`] with the deadline named, so a
    /// test can reach the timeout branch without waiting six minutes for it.
    /// Nothing else about the two differs.
    async fn sleep_for_action_result_until(
        &self,
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
    /// as unreachable — while the mod's re-path could not have succeeded, since
    /// its radius was 0.5 and standing clear of a stone furnace needs
    /// 0.8984375.
    ///
    /// So the destination is also checked for standability, against the entity
    /// graph, and refused as [`RconWalkEndsWhereNobodyCanStand`] naming the
    /// obstruction. Only a *provable* overlap refuses: see
    /// [`StandingVerdict`] for why "cannot tell" has to be allowed through.
    ///
    /// # A walk whose leg stalls is asked again, from here
    ///
    /// The waypoints a walk follows were chosen once, at dispatch time, by a
    /// run that is *building things* — so they go stale as a matter of course,
    /// and the mod's follower wedges against whatever appeared across the
    /// route. Recovering from that used to live in the mod, first as a teleport
    /// and then as a re-path, and both were the wrong place for it: all
    /// `control.lua` has is the waypoint list, so the best goal it could name
    /// was the last node of the path that had just gone stale, at a tolerance
    /// nobody asked for, judged against nothing. Every retry was strictly
    /// harder than the request that had already failed.
    ///
    /// The retry is here instead, because *here* is where the goal, the radius
    /// and the judgement above already are. A stalled leg — and only a stalled
    /// leg, see [`is_stalled_walk`] — costs a fresh [`FactorioRcon::player_path`]
    /// from where the character actually stands, put through the same
    /// [`judge_path`] as the first attempt, up to [`WALK_ATTEMPTS`] times. The
    /// second attempt of a destination that has since been built on is
    /// therefore refused *before* it is walked, rather than stalling again.
    pub async fn move_player_timed(
        &self,
        world: &Arc<FactorioSurface>,
        player_id: PlayerId,
        goal: &Position,
        radius: Option<f64>,
    ) -> Result<ActionTicks, ActionFailure> {
        let mut attempts_left = WALK_ATTEMPTS;
        let mut radius = radius;
        let mut tightened = false;
        loop {
            let outcome = self
                .move_player_attempt(world, player_id, goal, radius)
                .await;
            match outcome {
                Err(failure)
                    if !tightened
                        && let Some(tight) = tightened_radius(world, goal, radius, &failure) =>
                {
                    // The goal is fine; the game chose to stop somewhere
                    // else inside the disc, and that somewhere is inside a
                    // building. Ask once more for the goal itself. Not a
                    // `WALK_ATTEMPTS` attempt: nothing was walked, and this
                    // is a different question, not the same one again.
                    tightened = true;
                    warn!(
                        "#{} was routed to a spot nobody can stand on ({}), asking again for {}/{} at a radius of {} instead of {}",
                        player_id,
                        failure.error,
                        goal.x(),
                        goal.y(),
                        tight,
                        radius.unwrap_or(DEFAULT_PATH_RADIUS)
                    );
                    radius = Some(tight);
                }
                Err(failure) if attempts_left > 1 && is_stalled_walk(&failure) => {
                    attempts_left -= 1;
                    // The cause first, in a fixed shape, because this line is
                    // read while the run is happening and the interesting word
                    // used to be buried mid-sentence in a message whose front
                    // half never varies. The full clause is still in
                    // `failure.error` behind it -- the summary is a lead, not a
                    // replacement, and a message with no clause at all (an
                    // older mod) simply has no lead.
                    //
                    // Read off the same `RconError::message` [`is_stalled_walk`]
                    // matched on, not off `Report::to_string()`: the latter
                    // renders only the outermost error, so a wrapper added
                    // between here and the mod would silently take the cause
                    // away while everything still compiled.
                    let blocked_by = failure
                        .error
                        .downcast_ref::<RconError>()
                        .and_then(|refused| walk_blocker(&refused.message))
                        .map(|blocker| format!("blocked by {}, ", blocker.summary()))
                        .unwrap_or_default();
                    warn!(
                        "#{} stalled walking to {}/{} ({}{}), asking the game for a fresh path ({} attempts left)",
                        player_id,
                        goal.x(),
                        goal.y(),
                        blocked_by,
                        failure.error,
                        attempts_left
                    );
                }
                other => return other,
            }
        }
    }

    /// One dispatch of [`FactorioRcon::move_player_timed`]: path, judge, send,
    /// wait.
    ///
    /// Split out so the retry above is a loop over a whole attempt rather than
    /// a branch inside one. Every attempt takes a **fresh** action id and asks
    /// for a **fresh** path, which is the entire reason a retry here is worth
    /// more than the mod's was: `player_path` starts from where the character
    /// stands now, against the world as it is now.
    async fn move_player_attempt(
        &self,
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        if !within_mining_reach(world, &here, position, resource_reach_distance) {
            warn!("too far away, moving first!");
            // Beside a rock, on top of ore: see `mining_approach`. Aiming this
            // walk at the target's own centre with a plain disc is what
            // `judge_path` refused in every batch of run-1788551693-66583 --
            // the centre of a huge-rock is inside the huge-rock.
            let (goal, slack) = mining_approach(
                world,
                &here,
                position,
                resource_reach_distance,
                Some(player_id),
            );
            self.move_player_timed(world, player_id, &goal, Some(slack))
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
            if !within_mining_reach(world, &landed, position, resource_reach_distance) {
                return Err(ActionFailure::not_dispatched(
                    RconOutOfResourceReach {
                        target_x: position.x(),
                        target_y: position.y(),
                        distance: reach_distance(world, &landed, position),
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
        //
        // **The answer is read, not discarded.** `resource_mined` reports
        // `Absent` for a target the resource model has no tile for, and its
        // own doc says which targets those are: a tree or a rock. That is the
        // whole population of `EntityGraph::minables`, and nothing else in
        // this process removes a chopped tree -- the mod destroys the entity
        // and emits no event for it. Dropping this return value therefore
        // leaves a stump in the model that the planner re-offers for ever, and
        // the second visit fails with "no entity to mine" about ground the
        // bot itself cleared. `Absent` for a resource whose tile was already
        // retired is the harmless other reader of this branch:
        // `retire_minable` finds no such name and answers `false`.
        match &outcome {
            Ok(_) => {
                if world.entity_graph.resource_mined(name, position, count)
                    == ResourceDepletion::Absent
                {
                    world.entity_graph.retire_minable(name, position);
                }
            }
            Err(failure) if mine_reports_target_gone(&failure.error.to_string()) => {
                // Gone is gone, whichever model held it. `retire_resource`
                // answers `false` for a name that is not a resource, so the
                // second call is the one that does the work for a tree.
                if !world.entity_graph.retire_resource(name, position) {
                    world.entity_graph.retire_minable(name, position);
                }
            }
            Err(_) => {}
        }
        outcome
    }

    pub async fn player_craft(
        &self,
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        // A craft's honest duration is known before it is queued, so the
        // deadline is sized from it rather than from the flat six minutes:
        // `craft 75 automation-science-pack` is 75 x 5 s of packs plus the
        // gears the queue crafts on the way, and run-1788552801-73005 declared
        // it lost at 360 s while the character was still crafting (queue 1 on
        // the live game), then replanned around a craft that finished anyway.
        let energy = world
            .recipes
            .get(recipe)
            .map(|r| f64::from(*r.energy))
            .unwrap_or(0.0);
        self.sleep_for_action_result_until(
            world,
            action_id,
            dispatched,
            scale_deadline(Self::craft_deadline(energy, count), self.speed_factor()),
        )
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
    /// Ask the engine to generate the ground around a position.
    ///
    /// **This is the half of exploration that walking cannot do.** A bot
    /// cannot walk into ungenerated ground: the game's pathfinder returns no
    /// path for any destination past the edge of the generated world, so
    /// [`FactorioRcon::move_player`] refuses before dispatching anything.
    /// Measured live on seed 31337, whose fresh map is 400 chunks spanning
    /// `[-320, 320)` -- x=100 and x=200 reached, x=300 through x=600 all
    /// `failed to path find`, and a five-leg tour of the diagonals refused
    /// every leg.
    ///
    /// # It is bounded, and the bound is the point
    ///
    /// `radius` is in **chunks** and the mod clamps it to 4, which is the
    /// reveal a character standing there would have been given for free (a
    /// character placed on virgin ground generates a 9x9 block centred on it,
    /// 81 chunks, measured at two locations). The clamp is enforced mod-side
    /// so no caller can widen it; asking for more silently gets four.
    ///
    /// # It is not `force.chart`
    ///
    /// It reveals nothing to the force -- `is_chunk_charted` stays false for
    /// every chunk it makes, exactly as it does for ground a character is
    /// standing on. The world model learns only what `on_chunk_generated`
    /// writes out, which is the same channel every other chunk arrives
    /// through.
    ///
    /// # It is still not free, and the caller must record it
    ///
    /// The ground appears *before* the bot reaches it rather than as it
    /// arrives, so a plan can see one reveal further than a player would at
    /// the same moment. Small and bounded, but real: every call must reach
    /// the run record, the way `research_trigger_emulated` does. See
    /// `docs/superpowers/notes/2026-09-06-exploration.md`.
    pub async fn generate_chunks(
        &self,
        position: &Position,
        radius: u32,
    ) -> Result<GeneratedChunks> {
        let json = self
            .remote_call_json(
                "generate_chunks",
                vec![
                    position.x().to_string(),
                    position.y().to_string(),
                    radius.to_string(),
                ],
            )
            .await?;
        serde_json::from_str(json.as_str())
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to parse the generate_chunks reply: {json}"))
    }

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
        world: &Arc<FactorioSurface>,
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
        underground_half: Option<UndergroundHalf>,
        world: &Arc<FactorioSurface>,
    ) -> Result<FactorioEntity> {
        self.place_entity_timed(
            player_id,
            item_name,
            entity_position,
            direction,
            underground_half,
            world,
        )
        .await
        .map(|(entity, _ticks)| entity)
        .map_err(ActionFailure::into_report)
    }

    /// [`FactorioRcon::place_entity`], reporting the game tick it ran at
    /// alongside the entity it created.
    ///
    /// Placement is synchronous -- `surface.create_entity` returns within the
    /// tick the command was received -- so a placement that goes out **once**
    /// reports the same number on both ends of [`ActionTicks`], and its settle
    /// reads `elapsed_ticks: 0`.
    ///
    /// **A retried placement spans its retries instead.** Both the
    /// `§player_blocks_placement§` walk-and-reissue path and the
    /// [`FOOTPRINT_CLEAR_ATTEMPTS`] loop turn one action into several
    /// dispatches, and the pair reported is then `(first dispatch, settling
    /// reply)`: the reply end is the dispatch that actually built or was
    /// finally refused, and the dispatch end is where the attempt began.
    /// Reporting the last dispatch on both ends -- which is what this used to
    /// do -- threw the whole retry window away and made "refused four times
    /// over 1.8s" byte-identical to "refused instantly". See
    /// [`PlacementAttempts`], which owns the measurement and is tested without
    /// a game.
    ///
    /// `underground_half` is `Some` only when `item_name` is an
    /// underground-belt half; the mod's `rcon_place_entity` forwards it to
    /// `surface.create_entity` as `type`, and only for that entity, because
    /// the game rejects `type` on a prototype that has none. Every other
    /// caller passes `None`.
    pub async fn place_entity_timed(
        &self,
        player_id: PlayerId,
        item_name: String,
        entity_position: Position,
        direction: u8,
        underground_half: Option<UndergroundHalf>,
        world: &Arc<FactorioSurface>,
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
        // **The loop is the missing half of `step_aside_from_footprint`.**
        //
        // The mod answers a placement refused for a character *other* than the
        // acting bot with wording deliberately outside the
        // `can_place_entity said 'no'` family -- nothing durable is learned --
        // and, since `537adf30`, it also asks that character to walk out of the
        // footprint (`mods/BotBridge/control.lua`). Its own comment says what
        // it cannot do: "This does not make the placement succeed ... the
        // difference is that by the time anything asks again, the blocker is
        // somewhere else." **Nothing asked again.** The action failed, the
        // executor stopped that bot's chain, and the supervisor loop re-planned
        // the milestone from scratch.
        //
        // `run-1788481380-80843` is what that costs. Bot 4 walked to within
        // build reach of the furnace it was to load at `[-38, -16]`, stopped at
        // `(-38.25, -11.2)`, and stood there from tick 11760 waiting for bot 1
        // to build that furnace -- 4,000 ticks parked a third of a tile inside
        // the footprint of a *different* furnace, `[-39, -12]`, that bot 1 was
        // to place next. The placement was refused at tick 15727. The mod's
        // step-aside walk moved bot 4 clear by tick 15780, **53 ticks later**;
        // by then the run had already been abandoned with 39 of its 194 steps
        // never dispatched and re-planned from scratch, and the milestone
        // finished at 41,365 instead of the 34,065 the first plan was on course
        // for -- 10.5 minutes against 8.5.
        //
        // So: ask again. A bounded number of times, with a wait long enough for
        // a step aside -- a walk of one or two tiles -- to land. A blocker that
        // is *not* going to move (one the mod declined to steer because it is
        // already walking or mining for an action of its own, or one that fits
        // nowhere near) costs the attempts and then reports exactly what it
        // reported before, so nothing that used to be diagnosable stops being.
        //
        // Not the planner's job. `PlanState::is_area_free` already refuses a
        // site with a character in it, roster bot or not, and did so here: at
        // plan time bot 4 was at `(0.6, -0.6)`, twelve thousand ticks and forty
        // tiles from where it would be standing. A snapshot cannot see that,
        // which is why the recovery has to be here, where the game's own
        // refusal is still a line.
        // "input" / "output" / `nil` -- the fifth argument `rcon_place_entity`
        // (`mods/BotBridge/control.lua`) forwards to `surface.create_entity` as
        // `type`, and only when `item_name` is an underground-belt half. Built
        // once and reused on the retry below so both dispatches agree.
        let underground_half_lua = match underground_half {
            Some(UndergroundHalf::Input) => str_to_lua("input"),
            Some(UndergroundHalf::Output) => str_to_lua("output"),
            None => String::from("nil"),
        };
        let mut attempt: u32 = 0;
        // Kept apart from `attempt` on purpose: a blocker that is busy with an
        // action of its own is on its own clock, and spending the step-aside
        // budget while waiting for it leaves nothing for the step aside it may
        // still need once it goes idle. See `FOOTPRINT_BUSY_BLOCKER_BUDGET`.
        let mut busy_waited = Duration::ZERO;
        // Purely observational, and deliberately separate from `attempt`:
        // `attempt` is the budget that decides whether to go round again,
        // `attempts` is the measurement that reaches the run record. Keeping
        // them apart is what makes this whole change unable to alter what the
        // loop does. See `PlacementAttempts`.
        let mut attempts = PlacementAttempts::default();
        // Labelled because the actor-blocks branch nested inside also needs to
        // come back here -- see `continue 'place` in it.
        'place: loop {
            let (lines, tick) = self
                .remote_call_timed(
                    "place_entity",
                    vec![
                        player_id.to_string(),
                        str_to_lua(&item_name),
                        position_to_lua(&entity_position),
                        direction.to_string(),
                        underground_half_lua.clone(),
                    ],
                )
                .await?;
            attempts.dispatched(tick);
            // Past this point the game has answered the RPC, so every failure
            // below is a verdict the game gave and carries the tick it gave it
            // at. On the blocked-and-retry path the *reply* end is re-bound to
            // the retry's stamp -- that is the dispatch the outcome belongs to
            // -- while the *dispatch* end stays on the first attempt, so the
            // span covers the retries instead of collapsing to zero. See
            // `PlacementAttempts`.
            let refused_at = attempts.ticks(tick);
            let Some(lines) = lines else {
                return Err(ActionFailure::refused(
                    RconUnexpectedEmptyResponse {}.into(),
                    refused_at,
                ));
            };
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
            // `starts_with`, not a grapheme index. The old form built a
            // grapheme vector and read `chars[0]`, which panics on the
            // empty line an empty reply body splits into -- a panic inside
            // a run's dispatch task, on the failure path, where a returned
            // error is what the executor is waiting for.
            if line.starts_with('{') {
                note_pushed_out(player_id, line);
                return Ok((
                    parse_reply("place_entity", line)
                        .map_err(|e| ActionFailure::refused(e, refused_at))?,
                    attempts.ticks(tick),
                ));
            }
            if &line[..] == "§player_blocks_placement§" {
                // The eight compass points. This was `0..8u8` on the
                // Factorio 1.x scale, where those were all eight
                // directions; on the 2.x scale `0..8` is only half a
                // circle, so it has to be named rather than counted.
                for test_direction in Direction::compass() {
                    let Some(test_position) = move_position(&player_position, test_direction, 5.0)
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
                                    underground_half_lua.clone(),
                                ],
                            )
                            .await?;
                        attempts.dispatched(tick);
                        let refused_at = attempts.ticks(tick);
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
                                note_pushed_out(player_id, line);
                                Ok((
                                    parse_reply("place_entity", line)
                                        .map_err(|e| ActionFailure::refused(e, refused_at))?,
                                    attempts.ticks(tick),
                                ))
                            } else if &line[..] == "§player_blocks_placement§" {
                                Err(ActionFailure::refused(
                                    RconPlayerBlockesPlacement {}.into(),
                                    refused_at,
                                ))
                            } else if let Some((wait, backoff)) = {
                                let wait = FootprintWait::decide(line, attempt, busy_waited);
                                wait.backoff().map(|backoff| (wait, backoff))
                            } {
                                // Both blockers at once: the actor was in the
                                // expanded box (which is why the mod answered
                                // with the sentinel, actor first) *and* someone
                                // else is in the raw one, so walking the actor
                                // aside only uncovered the second. The same
                                // transient as below, reached by a different
                                // road, and it goes back to the same loop --
                                // which re-issues from the actor's new position
                                // and gives the step-aside walk the mod has just
                                // dispatched time to land.
                                if wait.spends_busy_budget() {
                                    busy_waited = busy_waited.saturating_add(backoff);
                                } else {
                                    attempt += 1;
                                }
                                attempts.backed_off(backoff);
                                sleep(backoff).await;
                                continue 'place;
                            } else {
                                note_placement_refusal(
                                    world,
                                    tick,
                                    line,
                                    &item_name,
                                    &entity_position,
                                    direction,
                                );
                                Err(ActionFailure::refused(
                                    RconError {
                                        message: attempts.explain(line, tick),
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
                return Err(ActionFailure::refused(
                    RconPlayerBlockesAllPlacement {}.into(),
                    refused_at,
                ));
            }
            // The transient the mod has just acted on. Retried rather than
            // reported, because the report is what threw the run away. How
            // long it is worth waiting depends on what the mod said the
            // blocker is doing -- see `FootprintWait`.
            let wait = FootprintWait::decide(line, attempt, busy_waited);
            if let Some(backoff) = wait.backoff() {
                if wait.spends_busy_budget() {
                    busy_waited = busy_waited.saturating_add(backoff);
                } else {
                    attempt += 1;
                }
                attempts.backed_off(backoff);
                sleep(backoff).await;
                continue;
            }
            note_placement_refusal(world, tick, line, &item_name, &entity_position, direction);
            return Err(ActionFailure::refused(
                RconError {
                    message: attempts.explain(line, tick),
                }
                .into(),
                refused_at,
            ));
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
        world: &Arc<FactorioSurface>,
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
    ///
    /// Returns a [`TransferOutcome`] rather than bare ticks because an insert
    /// has one success that is not a full delivery: the destination had no room
    /// for the rest. See [`judge_transfer_reply`] for why that is a success and
    /// [`DestinationFull`] for why the fact travels instead of being dropped.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_to_inventory_timed(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioSurface>,
    ) -> Result<TransferOutcome, ActionFailure> {
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
        world: &Arc<FactorioSurface>,
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
    ///
    /// Bare ticks, unlike [`FactorioRcon::insert_to_inventory_timed`]: a remove
    /// has no destination that can be full. Its shortfall wording (`tried to
    /// remove N ITEM but removed M`) means the *source* did not have it, which
    /// is the failure this whole distinction exists to keep failing.
    #[allow(clippy::too_many_arguments)]
    pub async fn remove_from_inventory_timed(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        inventory_type: u32,
        item_name: String,
        item_count: u32,
        world: &Arc<FactorioSurface>,
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
        judge_transfer_reply(lines, tick).map(|o| o.ticks)
    }

    /// Put `recipe` on the crafting machine named `entity_name` at
    /// `entity_position`, reporting the game tick it ran at.
    ///
    /// Synchronous, so both ends of [`ActionTicks`] are that tick: the game
    /// assigns a recipe inside the call, and there is no later event to wait
    /// for. Shaped like [`FactorioRcon::insert_to_inventory_timed`] -- a
    /// player, a named entity at a position, a walk first if the machine is
    /// out of reach -- because it is the same kind of action: a bot standing
    /// at a machine and operating it.
    ///
    /// # What the game actually returns, and why that matters here
    ///
    /// `LuaEntity.set_recipe` does **not** return a success flag. Per
    /// `workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17) it
    /// returns an *array of `ItemWithQualityCount`*: "Any items removed from
    /// this entity as a result of setting the recipe" -- the old recipe's
    /// ingredients and products, evicted because they no longer belong in the
    /// machine. Dropping that array destroys those items, which is the same
    /// discarded-return-value defect that `remove_item`, `create_entity` and
    /// `player.teleport` each cost this project once. The mod
    /// (`rcon_set_recipe`) hands them to the acting player and refuses in the
    /// reply body if it cannot, so a caller here never has to know.
    ///
    /// Because the return value is not a verdict, the mod's success check is
    /// `entity.get_recipe()` afterwards, and *that* is what an empty reply
    /// asserts.
    ///
    /// # A refusal is a sentence in the reply body
    ///
    /// The mod prints nothing but its `§tick§` stamp when the recipe is set,
    /// so any surviving line is the game -- or the mod -- saying no, and it is
    /// classified [`Dispatch::Refused`]: the game saw the command and judged
    /// it. Same shape as [`FactorioRcon::add_research_timed`], and for the
    /// same reason it is not left to `?`, which would claim
    /// [`Dispatch::NotDispatched`] and throw the dispatch tick away.
    ///
    /// The refusals worth telling apart are told apart **by the mod, in the
    /// line it prints**, and they are carried through here verbatim rather
    /// than being re-derived from an error kind: a recipe the acting force has
    /// not unlocked names itself and says it is not enabled, and that is a
    /// durable fact about the force -- it will keep being true until the
    /// unlocking technology is researched -- while "no such player" is a
    /// failure to ask at all. Nothing here records a durable refusal the way
    /// [`note_placement_refusal`] does for a build site: the planner already
    /// models recipe availability from the world's own `enabled` flags
    /// (`crates/planner`'s `recipe_gate`), so a locked recipe is a plan that
    /// should never have been dispatched rather than ground to be remembered.
    pub async fn set_recipe_timed(
        &self,
        player_id: PlayerId,
        entity_name: String,
        entity_position: Position,
        recipe: String,
        world: &Arc<FactorioSurface>,
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
        let (lines, tick) = self
            .remote_call_timed(
                "set_recipe",
                vec![
                    player_id.to_string(),
                    str_to_lua(&entity_name),
                    position_to_lua(&entity_position),
                    str_to_lua(&recipe),
                ],
            )
            .await?;
        let verdict = judge_set_recipe_reply(lines, tick)?;
        // The world model learns it here, and here is the only place it can.
        //
        // Nothing else ever tells it: the mod sends a machine's recipe on
        // every `serialize_entity`, but the only entity events it raises are
        // *created* (which fires at build time, before any recipe is on the
        // machine), *deleted*, and *updated* -- which the mod raises solely
        // from `on_player_rotated_entity` and `FactorioWorld` handles as a
        // no-op. So the planner replanned against machines whose stored recipe
        // was `None` for ever, and `Goal::Producing`, which counts machines by
        // the recipe on them, could never hold against a cell that really
        // stood. See `EntityGraph::set_recipe` for what that cost.
        //
        // This is a recorded fact and not an assumption: `rcon_set_recipe`
        // reads `entity.get_recipe()` back and refuses in the reply body
        // unless it names this recipe, and a refusal never reaches this line.
        if !world.entity_graph.set_recipe(&entity_position, &recipe) {
            // Not a failure of the action -- the game did it. It means this
            // graph does not know the machine, which `add` reports on its own
            // path; saying so here keeps the two halves of "did the model
            // learn" from disagreeing silently.
            warn!(
                // `tracing`, not `paris` -- this file imports `tracing::warn`,
                // so colour markup would print literally.
                recipe = %recipe,
                entity = %entity_name,
                position = %entity_position,
                "recipe was set in the game but no such entity stands there in the world model"
            );
        }
        Ok(verdict)
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

    /// Asks the running game for the map-exchange string of the map it is
    /// running -- the *producer*, and the counterpart of
    /// [`FactorioRcon::parse_map_exchange_string`] below, which only ever
    /// consumed one.
    ///
    /// **A seed is not a map.** A map is noise-generated from the seed *plus*
    /// the map-gen settings, and the exchange string encodes both, which is
    /// why it is the identity and the seed is not. Nothing in this project
    /// could produce one before 2026-09-06, which is why every archived run
    /// carries `map_exchange_string: null` and the maps behind their timings
    /// are unidentifiable.
    ///
    /// Errors are for the caller to turn into `None` meaning **"not
    /// captured"**. Never substitute an empty string: see
    /// [`crate::record::provenance::Provenance::map_exchange_string`] for why
    /// absence and a value must stay distinguishable.
    pub async fn map_exchange_string(&self) -> Result<String> {
        let lines = self
            .remote_call("map_exchange_string", vec![])
            .await?
            .ok_or_else(|| miette!("map_exchange_string: the mod answered nothing"))?;
        parse_map_exchange_reply(&lines)
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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

    /// Asks the game whether the character can leave the spot it stands on:
    /// one short path request per [`hop_targets`] direction, each allowed to
    /// stop within [`HOP_RADIUS`] tiles of its target. Returns every target
    /// with the game's answer, in the order asked.
    ///
    /// # Why the game and not the occupancy model
    ///
    /// `crates/core::graph::enclosure` answers the same question from what
    /// this process believes occupies the ground, and in
    /// `run-1788614781-38058` it answered wrongly: bot 6 stood overlapping a
    /// furnace, the model's fill from the tile centre found open ground, and
    /// the pathfinder -- reasoning from the character's real box at its real
    /// position -- refused every request from it. Only the game holds the
    /// character's own collision state, so only the game is asked.
    ///
    /// # What each answer means
    ///
    /// This is a bare [`Self::player_path_attempt`] per hop, deliberately not
    /// [`Self::player_path`]: the offset-goal fallback would rotate the target
    /// around the point and could return a route to somewhere the hop never
    /// asked for. A full queue is retried inside the attempt; a `failed to
    /// path find` that survives it is the game's word that there is no route
    /// from here to there. The *judgement* -- how many refusals make a
    /// benched bot -- is `crates/executor`'s (`walk_memory::judge_mobility`),
    /// which is where the walk that triggered the question failed.
    ///
    /// No mod change: `async_request_player_path` is the same remote call
    /// every walk already makes.
    pub async fn probe_player_hops(
        &self,
        world: &Arc<FactorioSurface>,
        player_id: PlayerId,
        from: &Position,
    ) -> Vec<(Position, Result<()>)> {
        let mut answers = Vec::with_capacity(4);
        for target in hop_targets(from) {
            let outcome = self
                .player_path_attempt(world, player_id, &target, Some(HOP_RADIUS))
                .await
                .map(|_| ());
            answers.push((target, outcome));
        }
        answers
    }

    pub async fn path(
        &self,
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
        world: &Arc<FactorioSurface>,
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
/// [`DashMap`](dashmap::DashMap) on the shared [`FactorioSurface`] for a reply the
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
    use crate::factorio::world::FactorioSurface;
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
        let world = Arc::new(FactorioSurface::new());
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
            let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
    use crate::factorio::world::FactorioSurface;

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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
    /// run. `repo_mods_path!` is the same checkout a debug run symlinks
    /// `workspace/mods/BotBridge` to, so the guard and the run cannot end up
    /// talking about different files.
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

    /// The wording that decides whether a failed walk is worth asking again is
    /// the mod's, not ours.
    ///
    /// Read out of `control.lua` for the same reason the mining verdict and the
    /// busy pathfinder are: a cross-language contract with nothing but a string
    /// on either side. A reword that went unnoticed here would retire the whole
    /// retry silently -- walks would simply start failing on the first stall
    /// again, and every test in this file would still pass.
    #[test]
    fn a_stalled_leg_is_recognised_from_the_mods_own_wording() {
        use crate::process::instance_setup::repo_mods_path;
        const CONTROL_LUA: &str = include_str!(repo_mods_path!("/BotBridge/control.lua"));
        assert!(
            CONTROL_LUA.contains(WALK_STUCK) && CONTROL_LUA.contains(WALK_LEG_STALLED),
            "the mod no longer says {WALK_STUCK:?} .. {WALK_LEG_STALLED:?}, so no              stalled walk is ever asked again"
        );
        assert!(walk_reports_stalled_leg(
            "ERROR: stuck while walking, leg 3 of 12 made no progress for 187 ticks              from (6.9/30.1) to (-22.3/18.2)"
        ));
    }

    /// **A stall is worth asking again; an answer is not.**
    ///
    /// The mod's older stuck verdicts came from a build that re-pathed for
    /// itself, and two of them are the pathfinder's own answer about the world:
    /// it searched and there is no way there. A save carrying an in-flight walk
    /// from such a build must not have that answer retried into oblivion, so
    /// the match is on the stall specifically and not on `stuck while walking`,
    /// which prefixes all of them.
    #[test]
    fn only_a_stall_is_worth_asking_the_game_again() {
        assert!(!walk_reports_stalled_leg(
            "ERROR: stuck while walking, the destination is unreachable: the game's              pathfinder found no path from (6.9/30.1) to (-22.3/18.2)"
        ));
        assert!(!walk_reports_stalled_leg(
            "ERROR: stuck while walking, gave up after 4 re-paths on one walk"
        ));
        assert!(!walk_reports_stalled_leg(
            "ERROR: stuck while walking, aborted before reaching last waypoint"
        ));
        assert!(
            !walk_reports_stalled_leg("ERROR: cannot place item 'stone-furnace'"),
            "and nothing that is not a walk failure at all"
        );
    }

    /// **How far the dispatch got decides this as much as the wording does.**
    ///
    /// A refusal the game handed down is the only one worth repeating.
    /// `NotDispatched` covers every pre-dispatch judgement -- `judge_path`'s
    /// two refusals and `failed to path find` -- which are facts about the
    /// ground and the map that asking again does not change; that is what keeps
    /// the pathfinder's definitive answer definitive. `NoVerdict` is worse: the
    /// game took that walk and never said how it ended, so the bot may still be
    /// walking, and a second dispatch would put two walks on one character.
    #[test]
    fn a_stall_is_only_retried_when_the_game_actually_answered() {
        let stall = || -> Report {
            RconError {
                message: "ERROR: stuck while walking, leg 3 of 12 made no progress for                           187 ticks from (6.9/30.1) to (-22.3/18.2)"
                    .to_string(),
            }
            .into()
        };
        assert!(is_stalled_walk(&ActionFailure::refused(
            stall(),
            ActionTicks::UNKNOWN
        )));
        assert!(
            !is_stalled_walk(&ActionFailure::not_dispatched(stall())),
            "nothing was sent, so there is no stall to have happened"
        );
        assert!(
            !is_stalled_walk(&ActionFailure::no_verdict(stall(), ActionTicks::UNKNOWN)),
            "the walk may still be running; a second dispatch would double it up"
        );

        // The negative control that matters most: the pre-dispatch standability
        // refusal. It is a fact about the ground, and run 30 spent four
        // re-paths and a leg timeout per walk relearning it.
        let unstandable: Report = RconWalkEndsWhereNobodyCanStand {
            goal_x: -23.5,
            goal_y: 18.5,
            end_x: -22.30078125,
            end_y: 18.22265625,
            blocker: "stone-furnace at [-22, 18]".to_string(),
        }
        .into();
        assert!(!is_stalled_walk(&ActionFailure::not_dispatched(
            unstandable
        )));
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
/// directory" line, which names what `workspace/mods/BotBridge` actually is:
/// in a debug build a symlink to this checkout, so there is no copy left to
/// drift (see `process::instance_setup`); in a release build a copy of the
/// embedded snapshot, checked by `asset_sync::warn_if_stale`. The bytes below
/// and the directory that line names both come from `repo_mods_path!`, so the
/// guard and the run cannot end up talking about different files.
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
        let world = Arc::new(FactorioSurface::new());
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
            0,
            Vec::new(),
            None,
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
    /// transfer handlers to run. Everything here is a stub *except* the four
    /// numbers the handlers do arithmetic on or report: `held`, what the player
    /// has; `moves`, what the target inventory will actually accept or give up;
    /// and `dest_holds` / `dest_room`, the destination's own state that
    /// `rcon_insert_to_inventory` reads back after a shortfall so this side can
    /// tell a full destination from one that refused the item.
    fn stub_game(held: i64, moves: i64, dest_holds: i64, dest_room: i64) -> String {
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
                -- The destination's own state, read back after a shortfall.
                get_item_count = function(name) return {dest_holds} end,
                get_insertable_count = function(name) return {dest_room} end,
            }}
            local entity = {{ get_inventory = function(t) return inventory end }}
            local player = {{
                -- A live player has a character; the entry points refuse one without.
                character = {{ type = "character", name = "character" }},
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
            dest_holds = dest_holds,
            dest_room = dest_room,
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

    /// A transfer into a destination that is not full and holds none of the
    /// item -- the shape every test written before the full-destination
    /// distinction existed assumed, and the one that keeps a shortfall a
    /// failure.
    fn transfer(
        call: &str,
        held: i64,
        moves: i64,
    ) -> (Result<TransferOutcome, ActionFailure>, String) {
        transfer_into(call, held, moves, 0, 0)
    }

    /// [`transfer`] with the destination's own state spelled out: how much of
    /// the item it holds afterwards and how much more it would take.
    fn transfer_into(
        call: &str,
        held: i64,
        moves: i64,
        dest_holds: i64,
        dest_room: i64,
    ) -> (Result<TransferOutcome, ActionFailure>, String) {
        let printed = run_handler(stub_game(held, moves, dest_holds, dest_room), call);
        let body = reply_body(&printed);
        let (lines, tick) = take_tick_stamp(split_reply(&body, true));
        (judge_transfer_reply(lines, tick), printed.join("\n"))
    }

    /// Enough of the API for `rcon_place_entity` to reach its refusal branches.
    /// `can_place` decides which one: `false` with the player inside the
    /// footprint is the `§player_blocks_placement§` case, and `held` at zero is
    /// the "does not have any" case.
    fn stub_place(can_place: bool, held: i64) -> String {
        // Standing dead centre of the tile it is about to build on: the live
        // 2026-09-02 case, and what puts the acting player inside its own
        // footprint.
        stub_place_at(can_place, held, 38.3046875, 16.4765625, "{}")
    }

    /// [`stub_place`] with the acting player's position and the characters the
    /// surface reports named.
    ///
    /// The third refusal branch -- some *other* character in the footprint --
    /// is unreachable from `stub_place`: `rcon_place_entity` tests the acting
    /// player first, on purpose, so the acting player has to be standing clear
    /// before `character_in_footprint` is ever asked.
    fn stub_place_at(
        can_place: bool,
        held: i64,
        player_x: f64,
        player_y: f64,
        characters: &str,
    ) -> String {
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

            local characters = {characters}
            local surface = {{
                can_place_entity = function(args) return {can_place} end,
                create_entity = function(args) return nil end,
                find_entity = function(name, pos) return nil end,
                -- Only `character_in_footprint` and
                -- `step_aside_from_footprint` ask, and both ask for
                -- characters, so the filter is not modelled.
                find_entities_filtered = function(args) return characters end,
            }}
            local player = {{
                -- A live player has a character; the entry points refuse one without.
                character = {{ type = "character", name = "character" }},
                name = "bot1",
                position = {{ x = {player_x}, y = {player_y} }},
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
            player_x = player_x,
            player_y = player_y,
            characters = characters,
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

    /// **The seam the retry hangs on.** `place_entity_timed` decides whether to
    /// ask again by matching [`FOOTPRINT_CHARACTER_REFUSAL`] against the line
    /// the mod prints, so the two have to be pinned together: a wording change
    /// on the Lua side would otherwise cost the retry silently and the only
    /// symptom would be a run thrown away, six weeks later, for a blocker that
    /// walked out of the way one second after being asked.
    ///
    /// Also asserts what the line must *not* say. The refusal ledger is
    /// permanent and never expires, so a character -- which moves -- must never
    /// be recorded as a fact about the ground; that is decided here, by the
    /// wording, and [`the_footprint_character_refusal_is_not_remembered`] holds
    /// the Rust half.
    #[test]
    fn a_character_in_the_footprint_is_refused_in_its_own_wording() {
        // The acting player stands well clear, so the actor-first branch does
        // not claim this refusal. The blocker's `player` is nil -- a character
        // nobody is driving -- which is the one case `step_aside_from_footprint`
        // must skip rather than raise on, and skipping it keeps this stub to
        // the branch under test.
        let blocker = r#"{ {
            position = { x = 38.5, y = 16.5 },
            bounding_box = {
                left_top = { x = 38.3, y = 16.3 },
                right_bottom = { x = 38.7, y = 16.7 },
            },
            player = nil,
        } }"#;
        let printed = run_handler(stub_place_at(false, 1, 30.5, 16.5, blocker), PLACE_FURNACE);
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
            lines[0].contains(FOOTPRINT_CHARACTER_REFUSAL),
            "`place_entity_timed` retries on this exact substring; got {lines:?}"
        );
        assert!(
            !lines[0].contains(CAN_PLACE_REFUSAL),
            "a character is not a fact about the ground, and the refusal ledger \
             never expires; got {lines:?}"
        );
    }

    /// Enough of the API for `walk_stall_cause` to run: a surface that reports
    /// `entities` inside the probe box and `tile` under it, and a `storage.p`
    /// saying what each bot is doing.
    ///
    /// The acting player stands at the origin steering east, so the probe point
    /// is a fixed `(0.75/0.0)` and the coordinate in the clause is checkable
    /// rather than incidental.
    fn stub_walk_probe(entities: &str, tile: &str, storage_p: &str) -> String {
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

            storage = {{ p = {storage_p} }}

            local found = {entities}
            local surface = {{
                find_entities_filtered = function(args) return found end,
                get_tile = function(x, y) return {tile} end,
            }}
            local character = {{ type = "character", name = "character" }}
            local player = {{
                index = 1,
                name = "bot1",
                position = {{ x = 0, y = 0 }},
                character = character,
                force = {{ name = "player" }},
                surface = surface,
            }}
            game = {{
                tick = {tick},
                players = {{ player }},
                forces = {{ player = {{ print = noop }} }},
            }}
        "#,
            entities = entities,
            tile = tile,
            storage_p = storage_p,
            tick = STUB_TICK,
        )
    }

    /// A blocking entity of `kind`/`name`, with a real collision box so
    /// `walk_stall_collides` keeps it.
    fn blocker_entity(kind: &str, name: &str, extra: &str) -> String {
        format!(
            r#"{{ valid = true, type = "{kind}", name = "{name}", force = {{ name = "player" }},
                  prototype = {{ collision_box = {{
                      left_top = {{ x = -0.4, y = -0.4 }},
                      right_bottom = {{ x = 0.4, y = 0.4 }} }} }},
                  {extra} }}"#
        )
    }

    /// Runs the mod's own probe and hands back the clause it produced.
    fn probe(entities: &str, tile: &str, storage_p: &str) -> String {
        let printed = run_handler(
            stub_walk_probe(entities, tile, storage_p),
            "rcon.print(walk_stall_cause(game.players[1], {x = 0, y = 0}, {x = 5, y = 0}, 1, 0))",
        );
        assert_eq!(
            printed.len(),
            1,
            "the probe returns one clause; got {printed:?}"
        );
        printed[0].clone()
    }

    /// **The seam this whole feature hangs on**, driven from both ends: the
    /// mod's real `walk_stall_cause` runs against a stub game, and
    /// [`walk_blocker`] parses the string it actually produced.
    ///
    /// Pinned this way rather than by asserting a substring appears in
    /// `control.lua`, because the two halves can disagree in ways a substring
    /// cannot see -- a class word moved to a different position in the
    /// sentence, a quote style changed, a coordinate rendered differently. The
    /// failure mode being guarded against has already happened twice here:
    /// `classify_walk_failure` lost 19 of 20 walk failures to `other` because
    /// it did not know the wording the mod emitted, and `run_analysis.py`
    /// matched the *mining* refusal's wording for a *placement* refusal and
    /// agreed with the record for the wrong reason. Both were silent.
    #[test]
    fn a_stalls_cause_is_read_from_the_mods_own_wording() {
        let grass = r#"{ valid = true, name = "grass-1" }"#;
        let no_bots = "{}";

        // A character, and **what it is doing**, which is the distinction the
        // feature exists for: `step_aside_from_footprint` steers only a blocker
        // that is neither walking nor mining, so a blocker reported `mining` is
        // one nothing is going to move -- the exact case that cost a run.
        let clause = probe(
            &format!(
                "{{ {} }}",
                blocker_entity("character", "character", r#"player = { index = 3 }"#)
            ),
            grass,
            "{ [3] = { mining = { action_id = 7 } } }",
        );
        let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
        assert_eq!(blocker.kind, WalkBlockerKind::Character, "{clause}");
        assert_eq!(blocker.player, Some(3), "{clause}");
        assert_eq!(blocker.activity.as_deref(), Some("mining"), "{clause}");
        assert_eq!(blocker.tile.as_deref(), Some("grass-1"), "{clause}");
        assert_eq!(
            blocker.at,
            Some(Position::new(0.75, 0.0)),
            "the probe looks one step ahead of where the character stands, not at \
             the waypoint: {clause}"
        );
        assert_eq!(blocker.others, 0, "{clause}");

        // The same character with nothing recorded against it is idle, not
        // unknown.
        let clause = probe(
            &format!(
                "{{ {} }}",
                blocker_entity("character", "character", r#"player = { index = 3 }"#)
            ),
            grass,
            no_bots,
        );
        assert_eq!(
            walk_blocker(&clause).and_then(|b| b.activity),
            Some("idle".to_string()),
            "{clause}"
        );

        // A character nobody is driving is still solid, and must not be
        // reported as a bot that could be asked to move.
        let clause = probe(
            &format!("{{ {} }}", blocker_entity("character", "character", "")),
            grass,
            no_bots,
        );
        let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
        assert_eq!(blocker.kind, WalkBlockerKind::Character, "{clause}");
        assert_eq!(blocker.player, None, "{clause}");

        // Something we built. `ours` is the difference between the plan
        // contradicting itself and the map being in the way.
        let clause = probe(
            &format!("{{ {} }}", blocker_entity("furnace", "stone-furnace", "")),
            grass,
            no_bots,
        );
        let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
        assert_eq!(blocker.kind, WalkBlockerKind::Entity, "{clause}");
        assert_eq!(blocker.name.as_deref(), Some("stone-furnace"), "{clause}");
        assert_eq!(blocker.ours, Some(true), "{clause}");

        // Scenery.
        for (kind, name, expected) in [
            ("tree", "tree-02", WalkBlockerKind::Tree),
            ("simple-entity", "rock-huge", WalkBlockerKind::Rock),
            ("cliff", "cliff", WalkBlockerKind::Cliff),
        ] {
            let clause = probe(
                &format!("{{ {} }}", blocker_entity(kind, name, "")),
                grass,
                no_bots,
            );
            let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
            assert_eq!(blocker.kind, expected, "{clause}");
            assert_eq!(blocker.name.as_deref(), Some(name), "{clause}");
            assert_eq!(
                blocker.ours, None,
                "only a built entity is ours or theirs: {clause}"
            );
        }

        // **Nothing findable is an answer.** It says the tile ahead was clear,
        // which points at the pathfinder rather than at the world -- and the
        // tile is still named, because the ground is the answer when nothing
        // is standing on it.
        let clause = probe("{}", r#"{ valid = true, name = "water" }"#, no_bots);
        let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
        assert_eq!(blocker.kind, WalkBlockerKind::Nothing, "{clause}");
        assert_eq!(blocker.tile.as_deref(), Some("water"), "{clause}");

        // **Ore is not a blocker.** A bot stuck on an ore patch stands in a
        // solid block of resource entities, and naming one would be a
        // confident wrong answer on the most common terrain a bot walks over.
        let clause = probe(
            &format!("{{ {} }}", blocker_entity("resource", "iron-ore", "")),
            grass,
            no_bots,
        );
        assert_eq!(
            walk_blocker(&clause).map(|b| b.kind),
            Some(WalkBlockerKind::Nothing),
            "resources collide on the resource layer only: {clause}"
        );

        // Neither is anything with no footprint, whatever its type -- the
        // general half of the test, which covers types this list has never
        // heard of.
        let clause = probe(
            r#"{ { valid = true, type = "some-mod-thing", name = "marker",
                   prototype = { collision_box = {
                       left_top = { x = 0, y = 0 },
                       right_bottom = { x = 0, y = 0 } } } } }"#,
            grass,
            no_bots,
        );
        assert_eq!(
            walk_blocker(&clause).map(|b| b.kind),
            Some(WalkBlockerKind::Nothing),
            "a prototype with an empty collision box occupies nothing: {clause}"
        );

        // A crowded box names the one that matters and says how many others
        // there were, rather than picking one and hiding the rest. The
        // character wins because it is the only blocker that moves on its own.
        let clause = probe(
            &format!(
                "{{ {}, {} }}",
                blocker_entity("tree", "tree-02", ""),
                blocker_entity("character", "character", r#"player = { index = 4 }"#)
            ),
            grass,
            "{ [4] = { walking = { idx = 1 } } }",
        );
        let blocker = walk_blocker(&clause).unwrap_or_else(|| panic!("unparsed: {clause}"));
        assert_eq!(blocker.kind, WalkBlockerKind::Character, "{clause}");
        assert_eq!(blocker.activity.as_deref(), Some("walking"), "{clause}");
        assert_eq!(blocker.others, 1, "{clause}");
    }

    /// The wording the mod produces since the progress clock: the leg's
    /// length and origin ride after `moved`, and the steering observation
    /// rides after the tile. Every older reader still finds what it read
    /// before -- the retry's two words, the archive's two coordinates, the
    /// `moved` distance, the tile -- and the new fields come out beside them.
    /// This is run 9's stall, as the new mod would have reported it.
    #[test]
    fn the_leg_and_the_steering_ride_after_the_cause_without_displacing_it() {
        let stall = "ERROR: stuck while walking, leg 2 of 86 made no progress for 54 ticks \
                     from (-44.8125/74.71484375) to (-44.5/74.5), moved 1.04 tiles of a \
                     1.42-tile leg that began at (-45.55078125/75.453125), blocked at \
                     (-44.062/74.715) by nothing findable on tile 'dirt-3', steering east \
                     at 0.150 tiles/tick, walking_state read back walking=true";
        assert!(walk_reports_stalled_leg(stall), "the retry still fires");
        let blocker = walk_blocker(stall).expect("the clause is there");
        assert_eq!(blocker.kind, WalkBlockerKind::Nothing);
        assert_eq!(blocker.tile.as_deref(), Some("dirt-3"));
        assert_eq!(blocker.moved_tiles, Some(1.04));
        assert_eq!(blocker.leg_tiles, Some(1.42));
        assert_eq!(blocker.engine_walking, Some(true));
        assert_eq!(
            blocker.summary(),
            "nothing, on dirt-3, after 1.04 of 1.42 tiles, game still walking"
        );

        // A named blocker carries them too, and `(+N more)` is still found
        // with a clause after it.
        let tree = "ERROR: stuck while walking, leg 3 of 4 made no progress for 61 ticks \
                    from (1.0/2.0) to (2.0/2.0), moved 0.00 tiles of a 1.00-tile leg that \
                    began at (1.0/2.0), blocked at (1.75/2.0) by tree 'tree-01' on tile \
                    'grass-1' (+2 more), steering east at 0.150 tiles/tick, walking_state \
                    read back walking=false";
        let blocker = walk_blocker(tree).expect("the clause is there");
        assert_eq!(blocker.kind, WalkBlockerKind::Tree);
        assert_eq!(blocker.name.as_deref(), Some("tree-01"));
        assert_eq!(blocker.others, 2);
        assert_eq!(blocker.leg_tiles, Some(1.0));
        assert_eq!(blocker.engine_walking, Some(false));

        // And a message from before either clause existed reads as before:
        // absent, never zero or false.
        let old = "ERROR: stuck while walking, leg 9 of 10 made no progress for 61 ticks \
                   from (1.0/2.0) to (2.0/2.0), moved 0.02 tiles, blocked at (1.75/2.0) \
                   by nothing findable on tile 'grass-1'";
        let blocker = walk_blocker(old).expect("the clause is there");
        assert_eq!(blocker.leg_tiles, None);
        assert_eq!(blocker.engine_walking, None);
        assert_eq!(blocker.summary(), "nothing, on grass-1, after 0.02 tiles");
    }

    /// The game repeats waypoints, and the repeat is dropped before the path
    /// reaches the mod: 60 of run 11's 5,131 legs were a position followed by
    /// itself, and they were the only legs shorter than half a tile.
    #[test]
    fn a_repeated_waypoint_is_dropped_from_the_path() {
        let path = [(1.5, 1.5), (1.5, 1.5), (2.5, 2.5), (3.5, 3.5), (3.5, 3.5)]
            .into_iter()
            .map(|(x, y)| PathWaypoint {
                position: Position::new(x, y),
                needs_destroy_to_reach: false,
            })
            .collect();
        let positions = waypoint_positions(path, "test");
        assert_eq!(
            positions,
            vec![
                Position::new(1.5, 1.5),
                Position::new(2.5, 2.5),
                Position::new(3.5, 3.5)
            ]
        );
    }

    /// The clause is appended to the stall wording the retry already matches,
    /// and appended **after** the two coordinates -- so
    /// [`walk_reports_stalled_leg`] still fires and the archive's endpoint
    /// parser (`walk_endpoints`, `crates/scripting_lua/src/globals/record.rs`)
    /// still reads `from` and `destination` out of the same string.
    ///
    /// This is the regression that would otherwise be found in a run: a cause
    /// bought at the price of the retry that made stalls survivable.
    #[test]
    fn the_cause_rides_on_the_stall_wording_without_displacing_it() {
        let stall = "ERROR: stuck while walking, leg 9 of 10 made no progress for 61 ticks \
                     from (-9.90625/-18.171875) to (-10.5/-18.5), moved 0.02 tiles, \
                     blocked at (-10.65625/-18.5) by character #3 (mining) on tile 'grass-1'";
        assert!(
            walk_reports_stalled_leg(stall),
            "the cause must not cost the retry"
        );
        let blocker = walk_blocker(stall).expect("the clause is there");
        assert_eq!(blocker.kind, WalkBlockerKind::Character);
        assert_eq!(blocker.summary(), "bot #3, mining");
        assert_eq!(blocker.moved_tiles, Some(0.02));

        // **The stall wording is a leg TIMEOUT and always has been**, so the
        // distance is the only thing that says whether the character was
        // wedged. A leg that covered three tiles and ran out of clock is not
        // the same event as one that covered none, and before this the record
        // called both "made no progress".
        let slow = "ERROR: stuck while walking, leg 2 of 4 made no progress for 240 ticks \
                    from (1.0/2.0) to (9.0/2.0), moved 3.40 tiles, blocked at (1.75/2.0) \
                    by nothing findable on tile 'grass-1'";
        let blocker = walk_blocker(slow).expect("the clause is there");
        assert_eq!(blocker.kind, WalkBlockerKind::Nothing);
        assert_eq!(blocker.moved_tiles, Some(3.4));
        assert_eq!(blocker.summary(), "nothing, on grass-1, after 3.4 tiles");

        // A walk crossing a save written by a build that did not stamp the
        // leg's origin says `unknown`, and `unknown` is not zero.
        let unmeasured = "... made no progress for 61 ticks from (1.0/2.0) to (9.0/2.0), \
                          moved unknown tiles, blocked at (1.75/2.0) by nothing findable";
        assert_eq!(
            walk_blocker(unmeasured)
                .expect("the clause is there")
                .moved_tiles,
            None,
            "not measured must never read as `did not move`"
        );

        // And the mod really does append it: the follower's own source, not a
        // description of it.
        assert!(
            CONTROL_LUA.contains(r#".. ", moved " .. moved .. " tiles" .. leg .. ", " .. cause"#),
            "the stall no longer carries a cause clause and a distance"
        );
        assert!(
            CONTROL_LUA.contains(WALK_PROBE_FAILED),
            "a probe that raises must still say so, not fall back to the old \
             message: {WALK_PROBE_FAILED:?}"
        );
    }

    /// **Three answers, not two.** A message with no clause is an older build;
    /// a clause this parser cannot read is a reworded mod; a clause saying
    /// nothing was there is a fact about the world. A reader who cannot tell
    /// them apart cannot tell a fixed run from a broken parser -- which is
    /// exactly what "19 of 20 failures were `other`" looked like from outside.
    #[test]
    fn a_missing_cause_an_unreadable_one_and_an_empty_one_are_three_answers() {
        assert_eq!(
            walk_blocker(
                "ERROR: stuck while walking, leg 3 of 12 made no progress for 187 ticks \
                 from (6.9/30.1) to (-22.3/18.2)"
            ),
            None,
            "an archived run from before the probe existed says nothing, and must \
             not be read as saying nothing was there"
        );
        let unreadable = walk_blocker(
            "... made no progress ..., blocked at (1.5/2.5) by hovercraft 'thing' on tile 'grass-1'",
        )
        .expect("a clause is present");
        assert_eq!(unreadable.kind, WalkBlockerKind::Unknown);
        assert_eq!(
            unreadable.detail.as_deref(),
            Some("hovercraft 'thing'"),
            "the words the mod used survive, so a reader sees the new wording \
             instead of an empty column"
        );
        assert_eq!(unreadable.tile.as_deref(), Some("grass-1"));

        let raised = walk_blocker(
            "... made no progress ..., blocker unknown (probe failed: control.lua:12: \
             attempt to index a nil value)",
        )
        .expect("a clause is present");
        assert_eq!(raised.kind, WalkBlockerKind::ProbeFailed);
        assert_eq!(
            raised.detail.as_deref(),
            Some("control.lua:12: attempt to index a nil value"),
            "the Lua error is the whole value of this case"
        );
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
                -- A live player has a character; the entry points refuse one without.
                character = {{ type = "character", name = "character" }},
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

    /// The command that killed `run-1788432181-42528`, verbatim in shape: bot 1
    /// topping a boiler up with 17 coal at (-7.0, -54.5).
    const INSERT_SEVENTEEN_COAL: &str = r#"rcon_insert_to_inventory(
        1, "boiler", {x=-7.0, y=-54.5}, 1, {name="coal", count=17})"#;

    /// **The fix.** A shortfall the *destination* caused is a success.
    ///
    /// The bot held all 17, offered all 17, and the boiler's one fuel slot took
    /// 3 because it already held 47. Nobody can make that inventory hold more
    /// coal; the goal ("the boiler has fuel") holds, and the remaining 14 stay
    /// with the bot, which is where they belong. Reporting this as a failure
    /// cost bot 1 its whole remaining chain at tick 211399.
    #[test]
    fn an_insert_the_destination_had_no_room_for_is_a_success() {
        let (verdict, printed) = transfer_into(INSERT_SEVENTEEN_COAL, 17, 3, 50, 0);
        let outcome = verdict.expect(
            "a boiler that took 3 of 17 because its fuel slot is full is a goal that \
             holds, not a command the game refused",
        );
        assert_eq!(outcome.ticks, ActionTicks::at(Some(STUB_TICK)));
        assert_eq!(
            outcome.destination_full,
            Some(DestinationFull {
                item: "coal".to_string(),
                asked: 17,
                moved: 3,
                holds: 50,
            }),
            "the numbers have to survive: a run whose every top-up moves 3 of 17 \
             must not look like one whose top-ups all move 17. It printed {printed:?}"
        );
    }

    /// The same, with nothing moved at all: the fuel slot was *already* a full
    /// stack of the item asked for.
    ///
    /// This is the one exception to "a zero-move is never green", and it is
    /// narrow on purpose -- `holds > 0` is what makes it, and the test below is
    /// the case it must not widen to.
    #[test]
    fn an_insert_into_a_destination_already_full_of_the_item_moves_nothing_and_succeeds() {
        let (verdict, printed) = transfer_into(INSERT_SEVENTEEN_COAL, 17, 0, 50, 0);
        let outcome = verdict.expect("a boiler already holding a full stack of coal has its fuel");
        assert_eq!(
            outcome.destination_full.map(|f| (f.moved, f.holds)),
            Some((0, 50)),
            "printed {printed:?}"
        );
    }

    /// **The case a zero-move must keep failing.** An inventory that would not
    /// take the item at all -- the wrong fuel, a filtered slot, a recipe that
    /// does not use it -- reports the same counts as a full one and holds none
    /// of what was offered. Nothing was delivered and nothing is satisfied.
    #[test]
    fn an_insert_the_destination_refused_outright_is_still_a_failure() {
        let (verdict, printed) = transfer_into(INSERT_SEVENTEEN_COAL, 17, 0, 0, 0);
        let failure = verdict
            .expect_err("an inventory holding none of the item did not accept it, it refused it");
        assert!(matches!(failure.dispatch, Dispatch::Refused));
        assert!(
            printed.contains("destination holds 0"),
            "the mod must report the destination's own state, which is the only \
             thing separating this from the test above; it printed {printed:?}"
        );
    }

    /// A destination that still has room and yet took less is a report that
    /// contradicts itself. Nothing here guesses which half to believe: an
    /// unrecognised outcome refuses, which is what the code did before the
    /// distinction existed.
    #[test]
    fn a_shortfall_into_a_destination_with_room_left_is_still_a_failure() {
        let (verdict, _) = transfer_into(INSERT_SEVENTEEN_COAL, 17, 3, 3, 47);
        verdict.expect_err("room left and a shortfall cannot both be true; this is not a success");
    }

    /// **The failure that must stay one.** The bot did not hold what the plan
    /// believed it held.
    ///
    /// The mod clamps to what the player has and says so, then the clamped
    /// insert fills the destination and says that too -- two lines, opposite
    /// meanings. The clamp is the one that matters: the delivery did not
    /// happen and the plan's model of the bot's inventory is wrong. One
    /// unrecognised line refuses the whole reply, which is what makes this
    /// work without ranking the lines against each other.
    #[test]
    fn a_short_source_is_a_failure_even_when_the_destination_was_also_full() {
        let (verdict, printed) = transfer_into(INSERT_SEVENTEEN_COAL, 10, 3, 50, 0);
        verdict.expect_err(
            "a bot that could not deliver what the plan believed it held is exactly \
             the silent divergence this project keeps being bitten by",
        );
        assert!(
            printed.contains("only has 10"),
            "the clamp is what makes this a failure; it printed {printed:?}"
        );
    }

    /// The shortfall wording, pinned whole, for the same reason the clamp and
    /// remove wordings above are.
    ///
    /// Two readers depend on this line and neither can check the other.
    /// [`parse_insert_shortfall`] reads the `(destination holds H, room for R)`
    /// suffix to make the success/failure call. `partial_transfer_detail`
    /// (`crates/scripting_lua/src/globals/record.rs`) reads the *prefix* --
    /// everything through `but inserted <moved>` -- to build the
    /// `FailureKind::PartialTransfer` detail for the shortfalls that stay
    /// failures. The suffix was appended rather than folded into the wording
    /// precisely so both keep working; if this assertion fails because the mod
    /// moved, that classifier's own tests are the other half to update.
    #[test]
    fn the_shortfall_wording_carries_both_readers_numbers() {
        let (_, printed) = transfer_into(INSERT_SEVENTEEN_COAL, 17, 3, 50, 0);
        assert_eq!(
            printed,
            format!(
                "tried to insert 17x coal but inserted 3 \
                 (destination holds 50, room for 0)\n\u{a7}tick\u{a7}{STUB_TICK}"
            )
        );
    }

    /// A `workspace/mods` copy that predates the suffix -- a release build
    /// extracted it once and never refreshes -- says nothing about the
    /// destination, so nothing can be concluded and the transfer fails exactly
    /// as it always did. Under-claiming is the safe direction here; inventing a
    /// satisfied goal is not.
    #[test]
    fn a_shortfall_from_a_mod_that_reports_no_destination_state_is_a_failure() {
        assert_eq!(
            parse_insert_shortfall("tried to insert 17x coal but inserted 3"),
            None
        );
        let verdict = judge_transfer_reply(
            Some(vec!["tried to insert 17x coal but inserted 3".to_string()]),
            Some(STUB_TICK),
        );
        verdict.expect_err("an unreadable shortfall is not evidence of a satisfied goal");
    }

    /// Every number must parse or the line is not understood. Nothing is
    /// inferred from a partial match.
    #[test]
    fn a_shortfall_with_an_unparseable_number_is_not_understood() {
        assert_eq!(
            parse_insert_shortfall(
                "tried to insert 17x coal but inserted lots (destination holds 50, room for 0)"
            ),
            None
        );
        assert_eq!(
            parse_insert_shortfall(
                "tried to insert 17x coal but inserted 3 (destination holds 50, room for )"
            ),
            None
        );
    }

    /// Enough of the API for `rcon_set_recipe` to reach the game, with the
    /// force's `enabled` flag as the one variable.
    ///
    /// `set_recipe` here is the game's real contract: it returns the items it
    /// evicted (none, for a fresh machine) and never a verdict, so the only
    /// thing that distinguishes success from a silent no-op is the
    /// `get_recipe` read the handler does afterwards.
    fn stub_set_recipe(enabled: bool) -> String {
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

            local entity = {{
                name = "assembling-machine-1",
                type = "assembling-machine",
                position = {{ x = 12.5, y = 8.5 }},
                recipe = nil,
            }}
            entity.set_recipe = function(name) entity.recipe = name; return {{}} end
            entity.get_recipe = function()
                if entity.recipe == nil then return nil end
                return {{ name = entity.recipe }}
            end
            local surface = {{
                find_entity = function(name, pos) return entity end,
            }}
            local force = {{
                recipes = {{ ["automation-science-pack"] =
                    {{ name = "automation-science-pack", enabled = {enabled} }} }},
                print = noop,
            }}
            local player = {{
                -- A live player has a character; the entry points refuse one without.
                character = {{ type = "character", name = "character" }},
                index = 1,
                name = "bot1",
                force = force,
                surface = surface,
                position = {{ x = 12.5, y = 10.5 }},
                insert = function(stack) return stack.count end,
                print = noop,
            }}
            prototypes = {{ item = {{}}, entity = {{}}, recipe = {{}} }}
            game = {{
                tick = {tick},
                players = {{ player }},
                forces = {{ player = force }},
            }}
        "#,
            enabled = if enabled { "true" } else { "false" },
            tick = STUB_TICK,
        )
    }

    const SET_ASP: &str = r#"rcon_set_recipe(
        1, "assembling-machine-1", {x=12.5, y=8.5}, "automation-science-pack")"#;

    /// Drives the real mod handler and the real judgement across the real
    /// reply-splitting, so the seam between them is what is tested rather than
    /// either half's idea of the other.
    fn set_recipe(enabled: bool) -> (Result<ActionTicks, ActionFailure>, String) {
        let printed = run_handler(stub_set_recipe(enabled), SET_ASP);
        let body = reply_body(&printed);
        let (lines, tick) = take_tick_stamp(split_reply(&body, true));
        (judge_set_recipe_reply(lines, tick), printed.join("\n"))
    }

    /// A recipe that was set prints its stamp and nothing else, and the
    /// judgement reads that as success at the game's own tick.
    #[test]
    fn a_recipe_that_was_set_succeeds_at_the_games_tick() {
        let (verdict, printed) = set_recipe(true);
        assert_eq!(
            printed,
            format!("§tick§{STUB_TICK}"),
            "a set recipe prints its stamp and nothing else"
        );
        assert_eq!(
            verdict.expect("a set recipe must succeed"),
            ActionTicks::at(Some(STUB_TICK))
        );
    }

    /// **A recipe the force has not unlocked is a refusal that names it.**
    ///
    /// The gate is force-scoped -- `player.force.recipes[name].enabled` -- and
    /// `automation-science-pack` is `false` until its trigger technology
    /// fires. `Dispatch::Refused`, not `NotDispatched`: the game saw the
    /// command and judged it, which is the stronger and truer claim, and the
    /// one that lets `crates/executor`'s `classify` keep it out of
    /// `NoVerdict`.
    ///
    /// Note what does **not** happen here: nothing is written to
    /// `FactorioWorld`'s placement-refusal ledger. A locked recipe is not
    /// ground to avoid; it is a plan that should never have been dispatched,
    /// and `crates/planner`'s `recipe_gate` already models it from the same
    /// `enabled` flag the world carries.
    #[test]
    fn a_locked_recipe_is_refused_and_the_reply_names_it() {
        let (verdict, printed) = set_recipe(false);
        let failure = verdict.expect_err(
            "a recipe the force has not unlocked reported success -- the machine \
             would then be dead and the plan would believe it finished",
        );
        assert!(
            matches!(failure.dispatch, Dispatch::Refused),
            "the game judged this one: got {:?}",
            failure.dispatch
        );
        assert_eq!(
            printed, "Error: recipe automation-science-pack is not enabled for this force",
            "the refusal names the recipe and why, and carries no tick stamp"
        );
        assert!(
            failure
                .error
                .to_string()
                .contains("automation-science-pack"),
            "and the name survives all the way out: got {}",
            failure.error
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
            let outcome = verdict.expect("a complete transfer must succeed");
            assert_eq!(outcome.ticks, ActionTicks::at(Some(STUB_TICK)));
            assert_eq!(
                outcome.destination_full, None,
                "nothing was left undelivered, so there is nothing to qualify"
            );
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
    use crate::factorio::world::FactorioSurface;

    fn world() -> Arc<FactorioSurface> {
        Arc::new(FactorioSurface::new())
    }

    /// The line four runs died on.
    const GENERIC: &str =
        "cannot place item 'stone-furnace' because surface.can_place_entity said 'no'";

    #[test]
    fn the_generic_refusal_is_remembered_with_its_site() {
        let world = world();
        let at = Position::new(-16., -58.);
        note_placement_refusal(&world, Some(6198), GENERIC, "stone-furnace", &at, 0);
        let refusals = world.placement_refusals();
        assert_eq!(refusals.len(), 1, "got {refusals:?}");
        assert_eq!(refusals[0].entity, "stone-furnace");
        assert_eq!(refusals[0].position, at);
        assert_eq!(refusals[0].tick, Some(6198));
        assert_eq!(refusals[0].direction, Some(0));
        assert!(
            refusals[0].blockers.is_empty() && refusals[0].tile.is_none(),
            "a line that names nothing reports nothing: {:?}",
            refusals[0]
        );
    }

    /// **The line `run-1788569499-05724` should have had.** The mod scans the
    /// box it just had judged and appends what it found; this side reads it
    /// back into the ledger, so a dispatch refusal no longer reaches the
    /// record as `blockers: []` -- which reads as "nothing was there" and was
    /// the whole of what that run's refusal said.
    #[test]
    fn what_the_mod_found_in_the_footprint_is_kept_with_the_refusal() {
        let world = world();
        let at = Position::new(40.5, -5.5);
        note_placement_refusal(
            &world,
            Some(172826),
            "cannot place item 'steam-engine' because surface.can_place_entity said 'no' \
             (in the footprint: pipe, small-electric-pole; tile: grass-1)",
            "steam-engine",
            &at,
            4,
        );
        let refusals = world.placement_refusals();
        assert_eq!(refusals.len(), 1, "got {refusals:?}");
        assert_eq!(
            refusals[0].blockers,
            vec!["pipe".to_string(), "small-electric-pole".to_string()]
        );
        assert_eq!(refusals[0].tile.as_deref(), Some("grass-1"));
        assert_eq!(
            refusals[0].direction,
            Some(4),
            "the box the game tested is the one turned east; the ledger has to \
             exclude that shape and not the north-frame one"
        );
    }

    /// Every shape the mod writes, and the shapes it does not, read back
    /// without inventing a name.
    #[test]
    fn the_footprint_parenthesis_reads_back_exactly() {
        let base = "cannot place item 'x' because surface.can_place_entity said 'no'";
        assert_eq!(parse_footprint_evidence(base), (vec![], None));
        assert_eq!(
            parse_footprint_evidence(&format!("{base} (nothing in the footprint; tile: water)")),
            (vec![], Some("water".to_string()))
        );
        assert_eq!(
            parse_footprint_evidence(&format!("{base} (nothing in the footprint)")),
            (vec![], None)
        );
        assert_eq!(
            parse_footprint_evidence(&format!("{base} (in the footprint: tree-01)")),
            (vec!["tree-01".to_string()], None)
        );
        // The retry note `PlacementAttempts::explain` appends comes after,
        // and must not be read as part of the evidence.
        assert_eq!(
            parse_footprint_evidence(&format!(
                "{base} (in the footprint: a, b; tile: dirt-4); dispatched 4 times over 120 \
                 game ticks (1.8s of waiting between attempts) and refused every time"
            )),
            (
                vec!["a".to_string(), "b".to_string()],
                Some("dirt-4".to_string())
            )
        );
        assert_eq!(
            parse_footprint_evidence("§player_blocks_placement§"),
            (vec![], None)
        );
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
            0,
        );
        assert!(
            world.placement_refusals().is_empty(),
            "the mod named the cause and the RCON layer retries it; that is \
             not a fact about the ground"
        );
    }

    /// The third answer, and the one that cost `run-1788481380-80843` its
    /// nine-minute target. A bot parked in the footprint is a transient the mod
    /// has already asked to move; remembering it would fence the planner out of
    /// good ground for the rest of the run **and** suppress the retry, because
    /// `place_entity_timed` re-issues this refusal and `recover`'s tier 1 is
    /// gated on `is_site_refused`.
    #[test]
    fn the_footprint_character_refusal_is_not_remembered() {
        let world = world();
        note_placement_refusal(
            &world,
            Some(15727),
            "cannot place item 'stone-furnace' because a character is standing in the footprint",
            "stone-furnace",
            &Position::new(-39., -12.),
            0,
        );
        assert!(
            world.placement_refusals().is_empty(),
            "a character moves -- and this one had moved 53 ticks later"
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
            note_placement_refusal(
                &world,
                None,
                line,
                "stone-furnace",
                &Position::new(0., 0.),
                0,
            );
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
            note_placement_refusal(&world, Some(tick), GENERIC, "stone-furnace", &at, 0);
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

/// The run id is the only thing on the wire, and an absent one has to stay
/// absent: the mod distinguishes "no id" from any value, and a placeholder
/// would give an untagged session an identity it never asked for.
#[cfg(test)]
mod sampling_arg_tests {
    use super::sampling_args;

    #[test]
    fn a_tagged_session_sends_its_quoted_run_id() {
        assert_eq!(sampling_args(Some("run-1")), vec!["'run-1'".to_string()]);
    }

    #[test]
    fn an_untagged_session_sends_no_argument_at_all() {
        assert!(sampling_args(None).is_empty());
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
    fn world_with(furnaces: &[Position]) -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
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
        let world = FactorioSurface::new();
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

/// What the executor asks the game for when the plan hands it an *annulus*.
///
/// The data is run 10's, `workspace/runs/run-1788413329-43771`, whose two
/// placement walks are the two ways the old lowering went wrong. Both aimed at
/// the single point `arrival_point` picks — exactly `min_radius` from the site,
/// along a fixed `+x` — with `approach_radius(0.0)`, which is its 0.5 floor:
///
/// ```text
/// tick 142398  to [-51.72941750255542, 11]   failed
///   ERROR: stuck while walking, the destination is unreachable: the game's
///   pathfinder found no path from (-51.11328125/11.37890625) to (-51.7265625/11)
///
/// tick 199124  to [-61.72941750255542, 11]   failed, never dispatched
///   the walk to [-61.72941750255542, 11] would end at [-61.7265625, 11],
///   inside a collision box spanning [-61.71, 10.54] to [-60.91, 11.34] — a
///   character cannot stand there, so the walk could only stall
/// ```
///
/// `+x` is not a neutral direction against a plan that builds in columns. Every
/// placement in that run sits at `x = -53` or `x = -63`, so the point one
/// clearance east of a site is aimed straight down the next column — which is
/// where the run found a tree, and where the plan will later find its own
/// buildings.
#[cfg(test)]
mod approach_annulus_tests {
    use super::*;
    use crate::test_utils::fixture_entity_prototypes;
    use crate::types::FactorioEntityPrototype;

    /// The build reach a `Place` action's `AtPosition` carries: the character's
    /// `build_distance`, which is 10.
    const BUILD_REACH: f64 = 10.0;

    /// `PlanState::placement_clearance("stone-furnace")`, re-derived here from
    /// the same prototypes the planner reads rather than copied as a literal:
    /// half the furnace's collision-box diagonal plus half the character's.
    fn stone_furnace_clearance() -> f64 {
        let prototypes = fixture_entity_prototypes();
        let half_diagonal = |name: &str| {
            let b = &prototypes
                .get(name)
                .expect("fixture prototype")
                .collision_box;
            (b.width() / 2.).hypot(b.height() / 2.)
        };
        half_diagonal("stone-furnace") + half_diagonal(CHARACTER_PROTOTYPE)
    }

    /// A world holding the fixture prototypes and one tree, which is the
    /// blocker shape run 10 reported: `FactorioEntity::new_tree` is a
    /// 0.8-by-0.8 box, and the entity tree never sees a tree, which is why the
    /// refusal named a box instead of a name.
    fn world_with_tree(at: &Position) -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        world
            .update_chunk_entities(vec![FactorioEntity::new_tree(at)])
            .unwrap();
        Arc::new(world)
    }

    /// The old lowering, reproduced: the planner's fixed-direction arrival
    /// point, at `approach_radius(0.0)`.
    fn the_old_request(site: &Position, clearance: f64) -> (Position, f64) {
        (
            Position::new(site.x() + clearance, site.y()),
            approach_radius(0.0),
        )
    }

    /// Run 10, tick 199124. The point the plan named is inside a tree, and the
    /// half-tile tolerance it named with it was the pathfinder's whole licence
    /// to find somewhere else. The guard was right to refuse; the destination
    /// should never have reached it.
    #[test]
    fn run_10s_refused_placement_walk_is_no_longer_aimed_at_that_point() {
        let clearance = stone_furnace_clearance();
        let site = Position::new(-63., 11.);
        // The tree, positioned so its box is the one the refusal named.
        let tree = Position::new(-61.31, 10.94);
        let world = world_with_tree(&tree);

        // First: this really is the run's geometry, not a lookalike.
        let (old_goal, old_radius) = the_old_request(&site, clearance);
        assert_eq!(
            old_goal.x(),
            -61.72941750255542,
            "the site and the clearance are run 10's own numbers"
        );
        assert_eq!(old_radius, 0.5, "and a zero radius came back as the floor");
        let old_end = Position::new(-61.7265625, 11.);
        assert!(
            walk_arrives(&old_goal, Some(old_radius), &old_end),
            "the arrival check passed this path -- the standing check is what refused it"
        );
        let failure = judge_path(&world, &old_goal, Some(old_radius), &[old_end], None)
            .expect_err("run 10 refused this walk before dispatch");
        let message = format!("{:?}", failure.error);
        assert!(
            message.contains("[-61.71, 10.54]") && message.contains("[-60.91, 11.34]"),
            "the reproduction must be run 10's own blocker box: {message}"
        );

        // Now the request the executor actually makes. The bot is where run
        // 10 left it: its previous walk settled at tick 199116 on the column
        // at x = -51.729, y = 17.
        let here = Position::new(-51.72941750255542, 17.);
        let (goal, slack) =
            approach_annulus(&site, clearance, BUILD_REACH, Some(&here), Approach::Outer);

        assert_eq!(
            slack, PATH_ENDPOINT_SLACK,
            "a real tolerance, not `approach_radius`'s floor: the pathfinder              needs somewhere to look, and a tile is what the endpoint              quantisation already costs"
        );
        let reach = calculate_distance(&goal, &site);
        assert!(
            reach >= clearance && reach <= BUILD_REACH,
            "the goal itself must satisfy the annulus: {reach} outside              ({clearance}, {BUILD_REACH}]"
        );
        assert!(
            !matches!(
                standing_verdict(&world, &goal),
                StandingVerdict::Blocked { .. }
            ),
            "aiming towards the bot leaves the column the tree is in"
        );
        assert!(
            judge_path(
                &world,
                &goal,
                Some(slack),
                std::slice::from_ref(&goal),
                None
            )
            .is_ok(),
            "and the pre-flight guard has nothing to refuse"
        );
    }

    /// Run 10, tick 142398 — the other refusal, and the one that shows what
    /// the missing tolerance cost even when nothing was in the way.
    ///
    /// The bot stood 1.924 tiles from the site it was to build on. That is
    /// *inside* the annulus `(1.2705824974445776, 10]`: the placement's own
    /// precondition already held, and the correct number of tiles to walk was
    /// zero. The old request named a point 0.723 tiles away with a 0.5
    /// tolerance, so the bot had to move anyway — and the ground it was sent
    /// to could not be reached.
    #[test]
    fn a_bot_already_inside_the_annulus_is_no_longer_marched_out_of_it() {
        let clearance = stone_furnace_clearance();
        let site = Position::new(-53., 11.);
        let here = Position::new(-51.11328125, 11.37890625);

        let standing = calculate_distance(&here, &site);
        assert!(
            standing >= clearance && standing <= BUILD_REACH,
            "the bot already satisfied the placement's precondition: {standing}"
        );

        let (old_goal, old_radius) = the_old_request(&site, clearance);
        assert_eq!(old_goal.x(), -51.72941750255542, "run 10's own goal");
        assert!(
            calculate_distance(&here, &old_goal) > old_radius,
            "which the bot was outside, so it had to walk -- 0.723 tiles, onto              ground the pathfinder then refused"
        );

        let (goal, slack) =
            approach_annulus(&site, clearance, BUILD_REACH, Some(&here), Approach::Outer);
        assert!(
            calculate_distance(&here, &goal) <= slack,
            "the bot is already inside the request, so there is nothing to              dispatch: {} from a goal with {slack} of slack",
            calculate_distance(&here, &goal)
        );
    }

    /// The guarantee the arithmetic is for, stated over the whole request disc
    /// rather than over the one point in it the goal happens to be.
    ///
    /// The furthest and nearest points of a disc from an outside centre lie on
    /// the line through both centres, so `|goal - target| +/- slack` are the
    /// extremes: checking those two checks every point.
    ///
    /// The two ends are held to different standards, on purpose. The **inner**
    /// bound is the safety bound -- a point inside it is the entity's own
    /// footprint, which is the whole reason the annulus exists -- and the
    /// outward ulp correction in [`approach_annulus`] makes it exact. The
    /// **outer** bound is a reach, and that same correction necessarily spends
    /// a few ulps of it. Those are allowed against a game whose positions are
    /// fixed-point 1/256ths of a tile: a nanotile is four million times finer
    /// than anything the game can represent, so an overrun this tolerates
    /// cannot exist in the world.
    #[test]
    fn every_point_the_pathfinder_may_answer_with_is_inside_the_annulus() {
        /// Far below the game's own 1/256 position quantum, far above f64
        /// rounding at these magnitudes.
        const NANOTILE: f64 = 1e-9;
        let clearance = stone_furnace_clearance();
        let target = Position::new(-63., 11.);
        let mut checked = 0u32;
        for x in [-80., -63., -40., 0.] {
            for y in [-30., 11., 11.5, 60.] {
                for radius in [2.0, 3.5, BUILD_REACH] {
                    let here = Position::new(x, y);
                    let (goal, slack) =
                        approach_annulus(&target, clearance, radius, Some(&here), Approach::Outer);
                    let d = calculate_distance(&goal, &target);
                    assert!(
                        d - slack >= clearance,
                        "nearest point of the request is inside the exclusion zone: {} < {clearance}",
                        d - slack
                    );
                    assert!(
                        d + slack <= radius + NANOTILE,
                        "furthest point of the request is outside the plan's tolerance: {} > {radius}",
                        d + slack
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 48);
    }

    /// The slack `min_radius` itself carries, which is what the mod's walker
    /// stopping "within a 0.3-by-0.3 box" of its waypoint is paid for out of.
    ///
    /// `min_radius` is a sum of half-*diagonals* — the distance at which two
    /// boxes can only touch at a corner, in the worst orientation. A character
    /// approaching from any of the four sides stands axis-aligned, where
    /// clearing the furnace needs only the half-widths. The difference is the
    /// headroom, and it has to be more than the stop box or a walk that lands
    /// exactly on the inner circle could still rest inside the footprint.
    #[test]
    fn approach_annulus_leaves_room_for_the_walkers_stop_box() {
        let prototypes = fixture_entity_prototypes();
        let half_width = |name: &str| {
            prototypes
                .get(name)
                .expect("fixture prototype")
                .collision_box
                .width()
                / 2.
        };
        let axis_aligned = half_width("stone-furnace") + half_width(CHARACTER_PROTOTYPE);
        let headroom = stone_furnace_clearance() - axis_aligned;
        assert!(
            headroom > 0.3,
            "the inner bound must out-reach the walker's stop box on its own:              {headroom} of headroom over {axis_aligned}"
        );
    }

    /// A plain disc stops on the outer ring now, and the ring is inside the
    /// reach with the margin to spare.
    ///
    /// This is most walks: mining, inserting, removing and crafting all set
    /// `min_radius` to zero, and every one of them used to be aimed at the
    /// target's own centre -- the bot walked the action's whole reach for
    /// nothing, and `travel_ticks` charged it. A disc of radius 10 arrived
    /// 66-67 ticks late in every run measured for the walk RCA, which is
    /// exactly `10 / 0.15`.
    ///
    /// [`Approach::Inner`] still answers what this always answered, which is
    /// what a corrective walk asks for.
    #[test]
    fn a_plain_disc_now_stops_on_the_outer_ring() {
        let target = Position::new(-31., -31.);
        // Far enough away that the aim is the ring rather than the bot's own
        // distance: 14.1 tiles out.
        let here = Position::new(-21., -21.);
        let (goal, slack) =
            approach_annulus(&target, 0.0, BUILD_REACH, Some(&here), Approach::Outer);
        let d = calculate_distance(&goal, &target);
        assert_eq!(slack, PATH_ENDPOINT_SLACK);
        assert!(
            (d - (BUILD_REACH - slack - ARRIVAL_MARGIN)).abs() < 1e-9,
            "aimed at {d}, not at the outer ring less the slack and the margin"
        );
        // The guarantee, over the whole request disc rather than its centre:
        // every point the pathfinder may answer with, plus the margin the
        // follower's stop box needs, is still in reach.
        assert!(d + slack + ARRIVAL_MARGIN <= BUILD_REACH + 1e-9);
        assert!(d > 0.0, "and it is outside min_radius, which is zero");
        // It aims towards the bot, so the walk is shorter and not longer.
        assert!(calculate_distance(&goal, &here) < calculate_distance(&here, &target));

        // A bot already closer than the ring is not marched back out to it.
        let inside = Position::new(-28.6640625, -26.8046875);
        let (goal, _) = approach_annulus(&target, 0.0, BUILD_REACH, Some(&inside), Approach::Outer);
        assert_eq!(goal, inside, "it is already in reach: nothing to walk");

        // And the old rule is still available, unchanged, for the corrective
        // walk that uses it.
        let (goal, slack) =
            approach_annulus(&target, 0.0, BUILD_REACH, Some(&here), Approach::Inner);
        assert_eq!(goal, target);
        assert_eq!(slack, approach_radius(BUILD_REACH));
        assert_eq!(slack, 5.0);
    }

    /// With nowhere to measure from, the offset falls back on the same fixed
    /// direction the planner's `arrival_point` uses, so the two agree about
    /// the degenerate case instead of disagreeing arbitrarily.
    #[test]
    fn an_unknown_bot_position_falls_back_on_the_planners_own_direction() {
        let clearance = stone_furnace_clearance();
        let site = Position::new(-63., 11.);
        let (goal, slack) = approach_annulus(&site, clearance, BUILD_REACH, None, Approach::Outer);
        // `+x`, to within the outward ulp nudge `aim_along` applies to both
        // coordinates: the direction is the planner's, the distance is the
        // outer ring.
        assert!((goal.y() - 11.).abs() < 1e-9, "aimed at {goal}");
        assert!(
            (goal.x() - (-63. + BUILD_REACH - slack - ARRIVAL_MARGIN)).abs() < 1e-9,
            "aimed at {goal}"
        );
        let d = calculate_distance(&goal, &site);
        assert!(d - slack >= clearance && d + slack <= BUILD_REACH + 1e-9);

        // A bot standing exactly on the target has no direction to offer
        // either, and must not produce a NaN goal. It is aimed the same way,
        // at the *inner* aim rather than the outer one: a bot inside the
        // clearance is never sent further out than the clearance needs, and
        // zero distance is as far inside as it gets.
        let (on_top, slack) =
            approach_annulus(&site, clearance, BUILD_REACH, Some(&site), Approach::Outer);
        assert!((on_top.y() - goal.y()).abs() < 1e-9, "aimed at {on_top}");
        assert!(
            (calculate_distance(&on_top, &site) - (clearance + slack)).abs() < 1e-9,
            "aimed at {on_top}"
        );
    }

    /// An annulus too thin for a whole tile of slack shrinks to fit rather
    /// than overrunning either bound.
    #[test]
    fn a_narrow_annulus_shrinks_the_request_instead_of_overrunning_it() {
        let target = Position::new(0., 0.);
        let here = Position::new(10., 0.);
        let (goal, slack) = approach_annulus(&target, 2.0, 3.0, Some(&here), Approach::Outer);
        // The band is one tile wide and 0.6 of it is the arrival margin, so
        // the slack is half of what is left and the outer and inner aims
        // coincide: a band too narrow to hold the margin behaves exactly as it
        // did before the outer ring existed.
        assert_eq!(
            slack, 0.2,
            "half of what the margin leaves, not a whole tile"
        );
        assert_eq!(goal, Position::new(2.2, 0.));
        assert_eq!(
            approach_annulus(&target, 2.0, 3.0, Some(&here), Approach::Inner).0,
            Position::new(2.5, 0.),
            "the inner rule is unchanged and is the wider of the two here"
        );
    }
}

/// What a retried placement is allowed to claim about itself.
///
/// The loop in [`FactorioRcon::place_entity_timed`] needs a live game to
/// exercise, so the measurement it carries is a separate value with no I/O in
/// it and these pin that value directly. Every number below is the shape the
/// motivating run produced: a placement refused because a character stood in
/// the footprint, re-issued while the mod walked that character aside.
#[cfg(test)]
mod placement_retry_measurement_tests {
    use super::*;

    #[test]
    fn a_placement_that_never_retried_still_reports_one_tick_on_both_ends() {
        // The regression guard for the whole change. A `place` is synchronous,
        // so a first-try placement must keep reporting `elapsed_ticks: 0` --
        // if this drifts, every placement in every archived run starts
        // claiming a duration nobody measured.
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(Some(15_727));
        assert_eq!(attempts.ticks(Some(15_727)), ActionTicks::at(Some(15_727)));
        assert!(!attempts.retried());
        assert_eq!(
            attempts.describe(Some(15_727)),
            None,
            "there is no retry history to report, so nothing may be appended"
        );
    }

    #[test]
    fn a_retried_placement_spans_from_the_first_dispatch_to_the_last_reply() {
        // The failure this exists to stop: three dispatches over 110 ticks
        // reported as `elapsed_ticks: 0`, indistinguishable from an instant
        // refusal.
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(Some(15_727));
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(15_780));
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(15_837));
        let ticks = attempts.ticks(Some(15_837));
        assert_eq!(ticks, ActionTicks::new(Some(15_727), Some(15_837)));
        assert_eq!(
            ticks
                .replied
                .zip(ticks.dispatched)
                .map(|(end, start)| end - start),
            Some(110),
            "the elapsed a settle reports must be the whole retry window, not zero"
        );
    }

    #[test]
    fn the_refusal_says_how_many_dispatches_it_survived() {
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(Some(15_727));
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(15_780));
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(15_837));
        let refusal = "cannot place stone-furnace: a character is standing in the footprint";
        let explained = attempts.explain(refusal, Some(15_837));
        assert!(
            explained.starts_with(refusal),
            "the mod's own wording leads, so `classify_failure` still reads it: {explained}"
        );
        assert!(explained.contains("dispatched 3 times"), "{explained}");
        assert!(explained.contains("110 game ticks"), "{explained}");
        assert!(explained.contains("1.2s"), "{explained}");
        assert_ne!(
            explained,
            attempts.explain(refusal, None),
            "an unmeasured span and a measured one may not read the same"
        );
    }

    /// The refusal `run-1788655528-63394` lost milestone 1's furnace to, as
    /// the mod writes it now. Bot 1 was mining copper ore from tick 5597 to
    /// 6079 while standing inside the furnace's box at `[26, -48]`; the
    /// placement was dispatched at 5887 and abandoned at 6001.
    const MINING_BLOCKER_REFUSAL: &str = "cannot place item 'stone-furnace' because a character is standing in \
         the footprint (blockers: #1 mining)";

    /// **A blocker that is busy is on its own clock, and the four step-aside
    /// attempts are not it.**
    ///
    /// The whole of the defect: 78 ticks after the placement gave up, bot 1's
    /// mine finished and it walked away.
    #[test]
    fn a_blocker_busy_with_its_own_action_is_waited_for_past_the_step_aside_budget() {
        assert!(footprint_blocker_is_busy(MINING_BLOCKER_REFUSAL));
        // Every step-aside attempt already spent, which is where the run gave
        // up.
        let spent = FOOTPRINT_CLEAR_ATTEMPTS - 1;
        assert_eq!(
            FootprintWait::decide(MINING_BLOCKER_REFUSAL, spent, Duration::ZERO),
            FootprintWait::Busy,
            "the mod says this blocker is mining for an action of its own, so \
             it is leaving; four dispatches over 1.8 s is not a reason to stop \
             asking"
        );
        assert_eq!(
            FootprintWait::decide(MINING_BLOCKER_REFUSAL, spent, Duration::ZERO).backoff(),
            Some(FOOTPRINT_BUSY_BACKOFF)
        );
        assert!(
            FOOTPRINT_BUSY_BLOCKER_BUDGET >= Duration::from_secs(33),
            "the longest busy action measured across the 24 archived runs is a \
             1,977-tick walk leg -- 32.9 s at 1x -- and a budget under it \
             cannot cover the case this exists for"
        );
    }

    /// And it is a bound. A blocker still busy after the budget falls back to
    /// exactly the old behaviour rather than waiting forever, which is what
    /// keeps this inside the executor's 360 s `ACTION_RESULT_DEADLINE`.
    #[test]
    fn a_busy_blocker_that_outlasts_the_budget_gives_up_as_before() {
        assert_eq!(
            FootprintWait::decide(
                MINING_BLOCKER_REFUSAL,
                FOOTPRINT_CLEAR_ATTEMPTS - 1,
                FOOTPRINT_BUSY_BLOCKER_BUDGET
            ),
            FootprintWait::GiveUp
        );
        assert_eq!(
            FootprintWait::decide(MINING_BLOCKER_REFUSAL, 0, FOOTPRINT_BUSY_BLOCKER_BUDGET),
            FootprintWait::StepAside,
            "the step-aside attempts are still there once the waiting ends: a \
             blocker that finished its action and then parked is exactly the \
             case they were written for"
        );
    }

    /// Waiting on a busy blocker must not spend the step-aside budget, or the
    /// bot that goes idle in the footprint after its mine finishes gets no
    /// walk dispatched at it.
    #[test]
    fn waiting_for_a_busy_blocker_does_not_spend_a_step_aside_attempt() {
        assert!(FootprintWait::Busy.spends_busy_budget());
        assert!(!FootprintWait::StepAside.spends_busy_budget());
    }

    /// **The substring trap.** `burner-mining-drill` contains "mining", and
    /// the item name is in the same sentence. Reading the whole line would
    /// call every drill placement busy and wait 45 s for a blocker that has
    /// just been asked to step aside and will be gone in tens of ticks.
    #[test]
    fn the_item_name_is_not_read_as_what_the_blocker_is_doing() {
        let line = "cannot place item 'burner-mining-drill' because a character is standing in \
                    the footprint (blockers: #3 stepping aside)";
        assert!(
            !footprint_blocker_is_busy(line),
            "only the clause after the refusal names what the blocker is doing"
        );
        assert_eq!(
            FootprintWait::decide(line, 0, Duration::ZERO),
            FootprintWait::StepAside
        );
    }

    /// A workspace whose `mods/BotBridge` predates the clause answers the old
    /// wording, and must get the old behaviour rather than a new one keyed on
    /// a sentence it never writes.
    #[test]
    fn a_refusal_with_no_blocker_clause_keeps_the_old_budget() {
        let old = "cannot place item 'stone-furnace' because a character is standing in the \
                   footprint";
        assert!(!footprint_blocker_is_busy(old));
        assert_eq!(
            FootprintWait::decide(old, 0, Duration::ZERO),
            FootprintWait::StepAside
        );
        assert_eq!(
            FootprintWait::decide(old, FOOTPRINT_CLEAR_ATTEMPTS - 1, Duration::ZERO),
            FootprintWait::GiveUp
        );
    }

    /// Every other refusal is answered on the spot, as before: a ground
    /// verdict is not waited out.
    #[test]
    fn a_refusal_that_is_not_about_a_character_is_never_waited_for() {
        assert_eq!(
            FootprintWait::decide(
                "cannot place item 'stone-furnace' because surface.can_place_entity said 'no' \
                 (in the footprint: tree-01; tile: grass-1)",
                0,
                Duration::ZERO
            ),
            FootprintWait::GiveUp
        );
    }

    /// The other blocker words are not busy: a `stuck` bot is idle with
    /// nowhere the game will put it, and an unclaimed character has no bot
    /// behind it at all. Neither is leaving, and waiting 45 s for either buys
    /// nothing.
    #[test]
    fn only_walking_and_mining_are_treated_as_leaving() {
        let base =
            "cannot place item 'stone-furnace' because a character is standing in the footprint";
        for clause in [
            "#3 stuck",
            "#3 gone",
            "an unclaimed character",
            "#3 stepping aside",
        ] {
            let line = format!("{base} (blockers: {clause})");
            assert!(!footprint_blocker_is_busy(&line), "{line}");
        }
        for clause in ["#1 mining", "#1 walking", "#3 stuck, #1 mining"] {
            let line = format!("{base} (blockers: {clause})");
            assert!(footprint_blocker_is_busy(&line), "{line}");
        }
    }

    /// The retry note is appended after the clause and must not be read as a
    /// blocker: it ends in `refused every time`, and `and refused every time`
    /// is not a state word.
    #[test]
    fn the_retry_note_after_the_clause_is_not_read_as_a_blocker() {
        let line = format!(
            "{MINING_BLOCKER_REFUSAL}; dispatched 4 times over 114 game ticks (1.8s of waiting \
             between attempts) and refused every time"
        );
        assert!(
            footprint_blocker_is_busy(&line),
            "the clause is still the clause with the note after it: {line}"
        );
    }

    #[test]
    fn the_appended_note_carries_no_single_quote() {
        // `classify_failure` reads a `MissingItem`'s item name out of the first
        // pair of single quotes in the message. A quote in this sentence would
        // hand it a fragment of this sentence as an item name -- a wrong answer
        // that looks like a right one, which is the whole family of bug this
        // work is about.
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(Some(1));
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(2));
        let note = attempts.describe(Some(2)).expect("a retry happened");
        assert!(!note.contains('\''), "{note}");
    }

    #[test]
    fn a_first_dispatch_the_game_did_not_stamp_stays_absent() {
        // Absent is not zero and is not the last attempt's tick. Substituting
        // the retry's stamp here would report a placement that took 1.2s as
        // instantaneous while looking measured.
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(None);
        attempts.backed_off(FOOTPRINT_CLEAR_BACKOFF);
        attempts.dispatched(Some(15_837));
        assert_eq!(attempts.ticks(Some(15_837)).dispatched, None);
        let note = attempts.describe(Some(15_837)).expect("a retry happened");
        assert!(
            note.contains("unmeasured"),
            "a span with a missing end says so rather than inventing one: {note}"
        );
        assert!(
            note.contains("0.6s"),
            "the wait itself was still measured: {note}"
        );
    }

    #[test]
    fn the_step_aside_dispatch_counts_as_a_dispatch_and_costs_no_backoff() {
        // The one place the dispatch count and the backoff count disagree: the
        // actor-blocks branch re-issues the placement from the actor's new
        // position without sleeping first.
        let mut attempts = PlacementAttempts::default();
        attempts.dispatched(Some(10));
        attempts.dispatched(Some(12));
        let note = attempts.describe(Some(12)).expect("a retry happened");
        assert!(note.contains("dispatched 2 times"), "{note}");
        assert!(note.contains("0.0s"), "{note}");
    }
}

#[cfg(test)]
mod mining_reach_tests {
    use super::*;
    use crate::test_utils::fixture_entity_prototypes;
    use crate::types::{EntityType, FactorioEntity, FactorioEntityPrototype};

    /// A character's reach in 2.1.17, as the game reports it.
    const REACH: f64 = 2.7;

    /// A world holding one `huge-rock` at (10, 10) with its live collision
    /// box, 3 by 2.2 -- the shape that refused run-1788551693-66583 -- and
    /// the fixture prototypes, so the character has a footprint.
    fn world_with_a_huge_rock() -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        let rock = FactorioEntity {
            name: "huge-rock".into(),
            entity_type: EntityType::SimpleEntity.to_string(),
            position: Position::new(10., 10.),
            bounding_box: Rect::new(&Position::new(8.5, 8.9), &Position::new(11.5, 11.1)),
            ..Default::default()
        };
        world.update_chunk_entities(vec![rock]).unwrap();
        Arc::new(world)
    }

    /// The build reach a `Place` or insert walk carries: the character's
    /// `build_distance` of 10, which `approach_radius` halves to a disc of 5.
    const BUILD_REACH: f64 = 10.0;

    /// `PlanState::placement_clearance("stone-furnace")`, re-derived from the
    /// fixture prototypes as `approach_annulus_tests` does.
    fn stone_furnace_clearance() -> f64 {
        let prototypes = fixture_entity_prototypes();
        let half_diagonal = |name: &str| {
            let b = &prototypes
                .get(name)
                .expect("fixture prototype")
                .collision_box;
            (b.width() / 2.).hypot(b.height() / 2.)
        };
        half_diagonal("stone-furnace") + half_diagonal(CHARACTER_PROTOTYPE)
    }

    /// A world holding the fixture prototypes and a stone furnace on each of
    /// `furnaces`, with the game's own collision box.
    fn world_with(furnaces: &[Position]) -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
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

    /// The two `big-rock`s of `run-1788608011-14361`, with the boxes the
    /// game reported for them: `{{-1, -0.8984375}, {1, 1}}` around each
    /// centre.
    fn world_with_run_as_two_rocks() -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        let rock = |x: f64, y: f64| FactorioEntity {
            name: "big-rock".into(),
            entity_type: EntityType::SimpleEntity.to_string(),
            position: Position::new(x, y),
            bounding_box: Rect::new(
                &Position::new(x - 1., y - 0.8984375),
                &Position::new(x + 1., y + 1.),
            ),
            ..Default::default()
        };
        world
            .update_chunk_entities(vec![rock(-17.5625, 21.5), rock(-19., 24.375)])
            .unwrap();
        Arc::new(world)
    }

    /// `run-1788608011-14361`, bot 4, step 16: a chop of the rock at
    /// `[-19, 24.375]`, aimed from `[-10.5, -22.5]`. The annulus's own aim
    /// lands inside the neighbouring rock at `[-17.56, 21.5]` -- the first
    /// half of this test pins that, with the run's own numbers -- and the
    /// game then returned a route ending there, which `judge_path` refused.
    /// `approach_standing` must aim at a point on the ring the graph cannot
    /// prove blocked, still inside the annulus.
    #[test]
    fn run_a_bot_4_is_aimed_off_the_neighbouring_rock() {
        let world = world_with_run_as_two_rocks();
        let target = Position::new(-19., 24.375);
        let here = Position::new(-10.5, -22.5);
        let rect = blocking_box_at(&world, &target).expect("the rock has a box");
        let clearance =
            (rect.width() / 2.).hypot(rect.height() / 2.) + 0.19921875f64.hypot(0.19921875);

        // The mechanism: the plan's annulus put the aim 0.7 tiles inside the
        // other rock. `here` is the ore tile bot 4 had just mined, not the
        // exact spot it stood on, so the run's `[-18.56, 22.24]` is
        // reproduced to within a tenth of a tile rather than exactly.
        // `Approach::Inner` is the rule that made the run's request.
        let (naive, _) = approach_annulus(&target, clearance, REACH, Some(&here), Approach::Inner);
        assert!(
            (naive.x() - -18.5613).abs() < 0.1 && (naive.y() - 22.2393).abs() < 0.1,
            "the run's aim was [-18.56, 22.24], this reproduces {naive}"
        );
        assert!(matches!(
            standing_verdict(&world, &naive),
            StandingVerdict::Blocked { .. }
        ));

        let (goal, slack) = approach_standing(
            &world,
            &target,
            clearance,
            REACH,
            Some(&here),
            None,
            Approach::Outer,
        );
        assert_eq!(
            standing_verdict(&world, &goal),
            StandingVerdict::NotProvablyBlocked,
            "aimed at {goal}"
        );
        let d = calculate_distance(&goal, &target);
        assert!(d - slack + 1e-9 >= clearance, "{d} - {slack} < {clearance}");
        assert!(d + slack <= REACH + 1e-9, "{d} + {slack} > {REACH}");
        // Still on the bot's side of the rock: the winner is the nearest
        // free bearing, not an arbitrary one.
        assert!(goal.y() < target.y(), "aimed at {goal}, the bot is north");
    }

    /// `run-1788608648-56109`, bot 1, step 36: an insert at the stone furnace
    /// at `[-13, -12]`, a plain disc whose centre is the furnace. The
    /// pathfinder answered `failed to path find` from `[-22.2, -9.7]`. The
    /// row of furnaces is the run's own (`map.jsonl`, keyframe at tick
    /// 101,156). The aim must leave the furnace's box, stay within the
    /// plan's reach of the centre, and stand clear of every other furnace
    /// in the row.
    #[test]
    fn a_disc_walk_to_a_furnace_is_aimed_beside_it() {
        let row = [
            Position::new(-13., -12.),
            Position::new(-15., -13.),
            Position::new(-17., -12.),
            Position::new(-19., -12.),
            Position::new(-21., -11.),
            Position::new(-23., -11.),
        ];
        let world = world_with(&row);
        let target = Position::new(-13., -12.);
        let here = Position::new(-22.21875, -9.70703125);

        // Control: the disc as it was asked for is centred on the furnace.
        // `Approach::Inner` is the rule that produced the run's own request --
        // a disc aimed at its own centre -- so the reproduction stays exact.
        let (naive, _) = approach_annulus(&target, 0.0, BUILD_REACH, Some(&here), Approach::Inner);
        assert!(matches!(
            standing_verdict(&world, &naive),
            StandingVerdict::Blocked { .. }
        ));

        let (goal, slack) = approach_standing(
            &world,
            &target,
            0.0,
            BUILD_REACH,
            Some(&here),
            None,
            Approach::Outer,
        );
        assert_eq!(
            standing_verdict(&world, &goal),
            StandingVerdict::NotProvablyBlocked,
            "aimed at {goal}"
        );
        let d = calculate_distance(&goal, &target);
        assert!(
            d - slack + 1e-9 >= stone_furnace_clearance(),
            "{d} - {slack} is inside the furnace's clearance"
        );
        assert!(d + slack <= BUILD_REACH + 1e-9);
        // On the bot's side, south of the row.
        assert!(goal.y() > target.y(), "aimed at {goal}");
    }

    /// The bots of `run-1788612263-27812` at tick 65,801, as `samples.jsonl`
    /// has them at tick 65,820 (the first beat after the refusal), with bot
    /// 1 -- the walker -- at the position its refusal named.
    fn run_1788612263_bots() -> [(PlayerId, Position); 4] {
        [
            (1, Position::new(29.26171875, -15.29296875)),
            (2, Position::new(36.28515625, -11.25390625)),
            (3, Position::new(34.25390625, -7.69921875)),
            (4, Position::new(31.21484375, -13.75)),
        ]
    }

    /// The two assemblers bot 2 had placed in the 30 ticks before the walk,
    /// as the run's `map.jsonl` has them, plus the bots in `players`.
    fn world_of_run_1788612263(players: &[(PlayerId, Position)]) -> Arc<FactorioSurface> {
        let world = FactorioSurface::new();
        let prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
            .iter()
            .map(|v| v.clone())
            .collect();
        world.update_entity_prototypes(prototypes).unwrap();
        let entities = [Position::new(36.5, -5.5), Position::new(36.5, -9.5)]
            .into_iter()
            .map(|position| {
                FactorioEntity::from_prototype(
                    "assembling-machine-1",
                    position,
                    None,
                    None,
                    None,
                    world.entity_prototypes.clone(),
                )
                .expect("the fixture has an assembling-machine-1 prototype")
            })
            .collect();
        world.update_chunk_entities(entities).unwrap();
        for (id, position) in players {
            world.players.insert(
                *id,
                FactorioPlayer {
                    player_id: *id,
                    position: position.clone(),
                    ..Default::default()
                },
            );
        }
        Arc::new(world)
    }

    /// `run-1788612263-27812`, bot 1, step 253, tick 65,801: an insert at
    /// the assembler at `[36.5, -5.5]`, aimed from `[29.26, -15.29]`. The
    /// annulus put the aim at `[34.73, -7.89]` -- the first half of this
    /// test reproduces the run's own number -- half a tile from where bot 3
    /// had been standing idle since tick 55,330. The graph holds no
    /// characters, so nothing in it could fault the aim; the game's
    /// pathfinder could, and did (`failed to path find`), and the batch
    /// replanned. `approach_standing` must now aim clear of bot 3 and still
    /// inside the annulus, on bot 1's own side of the machine.
    #[test]
    fn run_1788612263_bot_1_is_aimed_off_the_bot_standing_on_the_ring() {
        let bots = run_1788612263_bots();
        let world = world_of_run_1788612263(&bots);
        let target = Position::new(36.5, -5.5);
        let here = bots[0].1.clone();
        let bystander = bots[2].1.clone();
        let rect = blocking_box_at(&world, &target).expect("the assembler has a box");
        let clearance =
            (rect.width() / 2.).hypot(rect.height() / 2.) + 0.19921875f64.hypot(0.19921875);

        // The mechanism, with the run's numbers: the annulus aim is the one
        // the refusal named, the graph cannot fault it, and bot 3 is on it.
        // `Approach::Inner` is the rule of the day, so the reproduction is
        // exact; the outer ring is a later choice and would not land here.
        let (naive, _) = approach_annulus(
            &target,
            clearance,
            BUILD_REACH,
            Some(&here),
            Approach::Inner,
        );
        assert!(
            (naive.x() - 34.730088110096574).abs() < 1e-6
                && (naive.y() - -7.8945866745752244).abs() < 1e-6,
            "the run's aim was [34.730088110096574, -7.8945866745752244], this reproduces {naive}"
        );
        assert!(calculate_distance(&naive, &bystander) < 0.52);
        assert_eq!(
            standing_verdict(&world, &naive),
            StandingVerdict::NotProvablyBlocked,
            "the graph holds no characters, so it cannot see bot 3"
        );
        let bystanders = bystander_boxes(&world, Some(1));
        assert_eq!(bystanders.len(), 3, "bots 2, 3 and 4; never the walker");
        assert_eq!(bystander_clearance(&world, &naive, &bystanders), 0.0);

        let (goal, slack) = approach_standing(
            &world,
            &target,
            0.0,
            BUILD_REACH,
            Some(&here),
            Some(1),
            Approach::Outer,
        );
        assert_ne!(goal, naive, "the aim must move off bot 3");
        assert!(
            bystander_clearance(&world, &goal, &bystanders) > 0.0,
            "aimed at {goal}, still on a bot"
        );
        assert_eq!(
            standing_verdict(&world, &goal),
            StandingVerdict::NotProvablyBlocked,
            "aimed at {goal}"
        );
        // Clear of bot 3 by more than two characters' half-widths plus the
        // walker's stop box, on both axes at once or on one of them.
        let footprint = character_footprint(&world, &goal);
        let dx = (goal.x() - bystander.x()).abs();
        let dy = (goal.y() - bystander.y()).abs();
        let apart = footprint.width() + ARRIVAL_HALF_WIDTH;
        assert!(dx >= apart || dy >= apart, "aimed at {goal}: {dx} x {dy}");
        // Still in the annulus the plan granted.
        let d = calculate_distance(&goal, &target);
        assert!(d - slack + 1e-9 >= clearance, "{d} - {slack} < {clearance}");
        assert!(
            d + slack <= BUILD_REACH + 1e-9,
            "{d} + {slack} > {BUILD_REACH}"
        );
        // And on bot 1's side of the machine, south-west of it.
        assert!(
            goal.x() < target.x() && goal.y() < target.y(),
            "aimed at {goal}"
        );
    }

    /// With the same world and no other bot near the ring, the aim is the
    /// annulus's own, byte for byte -- bystanders change nothing on open
    /// ground. And the walker's *own* position is never a bystander: bot 1
    /// standing exactly on the annulus aim is still aimed there.
    #[test]
    fn a_ring_with_no_other_bot_on_it_is_aimed_as_before() {
        let target = Position::new(36.5, -5.5);
        let here = Position::new(29.26171875, -15.29296875);
        let rect = {
            let world = world_of_run_1788612263(&[]);
            blocking_box_at(&world, &target).expect("the assembler has a box")
        };
        let clearance =
            (rect.width() / 2.).hypot(rect.height() / 2.) + 0.19921875f64.hypot(0.19921875);
        let (naive, naive_slack) = approach_annulus(
            &target,
            clearance,
            BUILD_REACH,
            Some(&here),
            Approach::Outer,
        );

        // The other bots far away, the walker where it was.
        let far = [
            (1, here.clone()),
            (2, Position::new(0., 0.)),
            (4, Position::new(-30., 20.)),
        ];
        let world = world_of_run_1788612263(&far);
        assert_eq!(
            approach_standing(
                &world,
                &target,
                0.0,
                BUILD_REACH,
                Some(&here),
                Some(1),
                Approach::Outer
            ),
            (naive.clone(), naive_slack)
        );

        // The walker itself already standing on the aim.
        let world = world_of_run_1788612263(&[(1, naive.clone())]);
        assert_eq!(
            approach_standing(
                &world,
                &target,
                0.0,
                BUILD_REACH,
                Some(&naive),
                Some(1),
                Approach::Outer
            ),
            approach_annulus(
                &target,
                clearance,
                BUILD_REACH,
                Some(&naive),
                Approach::Outer
            )
        );
        // But the same position under another id is in the way.
        let world = world_of_run_1788612263(&[(1, here.clone()), (3, naive.clone())]);
        let (moved, _) = approach_standing(
            &world,
            &target,
            0.0,
            BUILD_REACH,
            Some(&here),
            Some(1),
            Approach::Outer,
        );
        assert_ne!(moved, naive);
    }

    /// A ring with a bot on every bearing is still aimed -- at the graph-clear
    /// candidate farthest from any bot, which with every candidate equally
    /// covered is the annulus's own aim -- rather than refused. A bot is not
    /// a building: it may be about to move, and the walk's own stall
    /// handling is the party that knows whether it did.
    #[test]
    fn a_ring_crowded_with_bots_on_every_bearing_is_aimed_at_the_clearest() {
        let target = Position::new(36.5, -5.5);
        let here = Position::new(29.26171875, -15.29296875);
        let rect = {
            let world = world_of_run_1788612263(&[]);
            blocking_box_at(&world, &target).expect("the assembler has a box")
        };
        let clearance =
            (rect.width() / 2.).hypot(rect.height() / 2.) + 0.19921875f64.hypot(0.19921875);
        // An annulus with room for exactly one ring, so the sweep cannot
        // step outward past the crowd.
        let radius = clearance + 2.0 * PATH_ENDPOINT_SLACK;
        let (naive, slack) =
            approach_annulus(&target, clearance, radius, Some(&here), Approach::Outer);
        let ring = clearance + slack;
        let mut crowd: Vec<(PlayerId, Position)> = vec![(1, here.clone())];
        for k in 0..APPROACH_BEARINGS {
            let a = std::f64::consts::TAU * k as f64 / APPROACH_BEARINGS as f64;
            let towards = approach_direction(&target, Some(&here));
            let direction = Position::new(
                towards.x() * a.cos() - towards.y() * a.sin(),
                towards.x() * a.sin() + towards.y() * a.cos(),
            );
            crowd.push((10 + k as PlayerId, aim_along(&target, &direction, ring)));
        }
        let world = world_of_run_1788612263(&crowd);
        let bystanders = bystander_boxes(&world, Some(1));
        assert_eq!(bystanders.len(), APPROACH_BEARINGS);
        let (goal, got_slack) = approach_standing(
            &world,
            &target,
            0.0,
            radius,
            Some(&here),
            Some(1),
            Approach::Outer,
        );
        assert_eq!(got_slack, slack);
        assert_eq!(bystander_clearance(&world, &goal, &bystanders), 0.0);
        assert_eq!(
            goal, naive,
            "every candidate equally covered: the nearest bearing wins"
        );
    }

    /// Ore has no box and a disc has no inner bound, so nothing here has a
    /// reason to move: the request goes out exactly as `approach_annulus`
    /// always made it.
    #[test]
    fn a_disc_walk_onto_open_ground_is_untouched() {
        let world = world_with(&[Position::new(-22., 18.)]);
        let ore = Position::new(-31., -31.);
        let here = Position::new(-28.6640625, -26.8046875);
        assert_eq!(
            approach_standing(
                &world,
                &ore,
                0.0,
                BUILD_REACH,
                Some(&here),
                None,
                Approach::Outer
            ),
            approach_annulus(&ore, 0.0, BUILD_REACH, Some(&here), Approach::Outer)
        );
    }

    /// A ring the graph can prove blocked all the way round is returned as
    /// the annulus would have aimed it -- the graph proves obstruction, not
    /// clearance, and refusing here would refuse on a guess.
    #[test]
    fn a_ring_with_no_provably_free_point_falls_back_on_the_annulus() {
        // Furnaces on every side of the target, close enough that the whole
        // ring at clearance-plus-slack is inside one of them.
        let target = Position::new(0., 0.);
        let ring: Vec<Position> = (0..16)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 16.;
                Position::new(2.3 * a.cos(), 2.3 * a.sin())
            })
            .collect();
        let mut all = vec![target.clone()];
        all.extend(ring);
        let world = world_with(&all);
        let here = Position::new(10., 0.);
        let clearance = stone_furnace_clearance();
        let (goal, slack) = approach_standing(
            &world,
            &target,
            clearance,
            3.0,
            Some(&here),
            None,
            Approach::Outer,
        );
        assert_eq!(
            (goal, slack),
            approach_annulus(&target, clearance, 3.0, Some(&here), Approach::Outer)
        );
    }

    /// The second question `move_player_timed` asks after a route that ends
    /// inside a box: only for that failure, only when the goal itself is
    /// clear, and only when the request left the game room to stop elsewhere.
    #[test]
    fn a_route_ending_inside_a_box_is_asked_again_tightly_when_the_goal_is_clear() {
        let world = world_with(&[Position::new(-22., 18.)]);
        let refusal = || {
            ActionFailure::not_dispatched(
                RconWalkEndsWhereNobodyCanStand {
                    goal_x: -24.,
                    goal_y: 18.,
                    end_x: -22.3,
                    end_y: 18.2,
                    blocker: "stone-furnace at [-22, 18]".into(),
                }
                .into(),
            )
        };
        let clear = Position::new(-24., 18.);
        assert_eq!(
            tightened_radius(&world, &clear, Some(5.0), &refusal()),
            Some(TIGHT_PATH_RADIUS)
        );
        // Factorio's own default of 1 is wider than tight, too.
        assert_eq!(
            tightened_radius(&world, &clear, None, &refusal()),
            Some(TIGHT_PATH_RADIUS)
        );
        // Already tight: the game had no room, asking again changes nothing.
        assert_eq!(
            tightened_radius(&world, &clear, Some(TIGHT_PATH_RADIUS), &refusal()),
            None
        );
        // The goal is the furnace: a tighter request for it is refused for
        // the same reason.
        assert_eq!(
            tightened_radius(&world, &Position::new(-22., 18.), Some(5.0), &refusal()),
            None
        );
        // Any other failure is somebody else's question.
        let short = ActionFailure::not_dispatched(
            RconWalkFallsShort {
                goal_x: -24.,
                goal_y: 18.,
                end_x: -30.,
                end_y: 18.,
                shortfall: 6.,
                tolerance: 2.,
            }
            .into(),
        );
        assert_eq!(tightened_radius(&world, &clear, Some(5.0), &short), None);
    }

    #[test]
    fn reach_to_a_rock_is_measured_to_its_box_not_its_centre() {
        let world = world_with_a_huge_rock();
        let rock = Position::new(10., 10.);
        // 3.0 from the centre, 1.5 from the box: the game lets this mine.
        let beside = Position::new(13., 10.);
        assert!(
            !within_resource_reach(&beside, &rock, REACH),
            "control: the centre rule refuses it"
        );
        assert!(within_mining_reach(&world, &beside, &rock, REACH));
        assert_eq!(reach_distance(&world, &beside, &rock), 1.5);
        // And a target with no box -- ore -- is still the centre rule.
        let ore = Position::new(40.5, 40.5);
        assert!(!within_mining_reach(
            &world,
            &Position::new(43.5, 40.5),
            &ore,
            REACH
        ));
        assert!(within_mining_reach(
            &world,
            &Position::new(42.5, 40.5),
            &ore,
            REACH
        ));
    }

    #[test]
    fn a_mines_corrective_walk_aims_beside_a_rock_and_on_top_of_ore() {
        let world = world_with_a_huge_rock();
        let rock = Position::new(10., 10.);
        // The box as the graph hands it back -- edges snapped to the game's
        // 1/256 grid, so 8.9 is 8.8984375 -- not the numbers typed above.
        let rect = blocking_box_at(&world, &rock).expect("the rock has a box");
        let (half_w, half_h) = (rect.width() / 2., rect.height() / 2.);
        let clearance = half_w.hypot(half_h) + 0.19921875f64.hypot(0.19921875);
        let outer = REACH + half_w.min(half_h);
        for here in [
            Position::new(20., 10.),
            Position::new(10., -5.),
            Position::new(0., 0.),
        ] {
            let (goal, slack) = mining_approach(&world, &here, &rock, REACH, None);
            let d = calculate_distance(&goal, &rock);
            // `1e-9` on both bounds: far below the game's 1/256 position
            // quantum, far above f64 rounding of `d - slack` at this scale.
            assert!(
                d - slack + 1e-9 >= clearance,
                "the request reaches inside the rock: {} < {clearance}",
                d - slack
            );
            assert!(
                d + slack <= outer + 1e-9,
                "the request may end out of reach: {} > {outer}",
                d + slack
            );
            // Every point of the request is within the game's reach of the box.
            assert!(distance_to_rect(&goal, &rect) + slack <= REACH + 1e-9);
        }
        let ore = Position::new(40.5, 40.5);
        let (goal, slack) = mining_approach(&world, &Position::new(50., 50.), &ore, REACH, None);
        assert_eq!(goal, ore, "ore is stood on, as it always was");
        assert_eq!(slack, approach_radius(REACH));
    }

    #[test]
    fn distance_to_a_rect_is_zero_inside_and_euclidean_outside() {
        let rect = Rect::new(&Position::new(0., 0.), &Position::new(2., 2.));
        assert_eq!(distance_to_rect(&Position::new(1., 1.), &rect), 0.);
        assert_eq!(distance_to_rect(&Position::new(5., 1.), &rect), 3.);
        assert_eq!(distance_to_rect(&Position::new(5., 6.), &rect), 5.);
    }
}

#[cfg(test)]
mod craft_deadline_tests {
    use super::*;

    /// The craft that was declared lost: 75 packs at 5 s each. The old flat
    /// deadline was 360 s; the packs alone are 375 s before their gears.
    #[test]
    fn a_long_craft_gets_a_deadline_sized_from_its_recipe() {
        let d = FactorioRcon::craft_deadline(5.0, 75);
        assert!(
            d > Duration::from_secs(375),
            "packs alone take 375 s: {d:?}"
        );
        assert_eq!(d, Duration::from_secs_f64(5.0 * 75.0 * 3.0 + 60.0));
    }

    /// The research that was declared lost: 75 units of 5 s = 22,500 ticks.
    #[test]
    fn a_long_research_gets_a_deadline_sized_from_the_plan() {
        let d = FactorioRcon::sized_deadline(22_500);
        assert!(d > Duration::from_secs(375), "one lab needs 375 s: {d:?}");
        assert_eq!(FactorioRcon::sized_deadline(0), ACTION_RESULT_DEADLINE);
    }

    /// A short craft keeps the deadline every other action has.
    #[test]
    fn a_short_craft_keeps_the_flat_deadline() {
        assert_eq!(FactorioRcon::craft_deadline(0.5, 1), ACTION_RESULT_DEADLINE);
        assert_eq!(
            FactorioRcon::craft_deadline(0.0, 100),
            ACTION_RESULT_DEADLINE
        );
    }
}

#[cfg(test)]
mod scale_deadline_tests {
    use super::*;

    #[test]
    fn deadlines_scale_with_game_speed() {
        let base = Duration::from_secs(360);
        assert_eq!(scale_deadline(base, 10.0), Duration::from_secs(36));
        assert_eq!(scale_deadline(base, 1.0), base);
        assert_eq!(scale_deadline(base, 0.5), Duration::from_secs(720));
        // Not positive: treated as normal, never divided by.
        assert_eq!(scale_deadline(base, 0.0), base);
        assert_eq!(scale_deadline(base, -3.0), base);
        // The floor: round trips do not get faster with the game.
        assert_eq!(
            scale_deadline(Duration::from_secs(20), 100.0),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn a_fresh_client_assumes_normal_speed() {
        assert_eq!(FactorioRcon::new_empty().speed_factor(), 1.0);
    }
}

/// Judges what the game answered [`FactorioRcon::map_exchange_string`] with.
///
/// Split out of the round trip so the shape can be tested without a running
/// Factorio -- which is the only thing that *can* be tested here, since no
/// test in this workspace has a game to ask.
///
/// A Factorio exchange string is delimited `>>>`...`<<<` and its base64 body
/// carries embedded whitespace (the shipped one in `settings.rs` had spaces
/// every 60-odd characters). RCON may hand the body back across several
/// lines, so the lines are joined; whitespace *inside* is left exactly as the
/// game wrote it, because this string is an identity and normalising it would
/// make two recordings of one map compare unequal.
///
/// Anything that is not delimited is refused rather than stored. A reply that
/// is a mod error, an empty string, or a truncated read must reach the caller
/// as an error so it can record "not captured", never as a value.
fn parse_map_exchange_reply(lines: &[String]) -> Result<String> {
    let joined = lines.join("");
    let trimmed = joined.trim();
    if trimmed.starts_with(">>>") && trimmed.ends_with("<<<") && trimmed.len() > 6 {
        Ok(trimmed.to_string())
    } else {
        Err(miette!("map_exchange_string: unreadable reply: {lines:?}"))
    }
}

#[cfg(test)]
mod map_exchange_string_tests {
    use super::parse_map_exchange_reply;

    fn lines(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_delimited_reply_is_taken_verbatim() {
        let got = parse_map_exchange_reply(&lines(&[">>>eNpjZI CDBnsQ<<<"])).expect("accepted");
        // The inner space survives: the string is an identity, not a value to
        // tidy. The shipped default in `settings.rs` is full of them.
        assert_eq!(got, ">>>eNpjZI CDBnsQ<<<");
    }

    #[test]
    fn a_reply_split_across_lines_is_joined_not_separated() {
        let got = parse_map_exchange_reply(&lines(&[">>>eNpjZI", "CDBnsQ<<<"])).expect("accepted");
        assert_eq!(got, ">>>eNpjZICDBnsQ<<<");
    }

    #[test]
    fn surrounding_whitespace_is_stripped() {
        let got = parse_map_exchange_reply(&lines(&["  >>>eNpjZICDBnsQ<<<\n"])).expect("accepted");
        assert_eq!(got, ">>>eNpjZICDBnsQ<<<");
    }

    #[test]
    fn an_undelimited_reply_is_an_error_and_never_a_value() {
        // A mod error, an empty reply and a truncated read all have to reach
        // the caller as an error: `None` in provenance means "not captured",
        // and a fabricated or partial string would claim a map identity that
        // nobody established.
        for reply in [
            vec![],
            lines(&[""]),
            lines(&["   "]),
            lines(&[">>><<<"]),
            lines(&["eNpjZICDBnsQ"]),
            lines(&[">>>eNpjZICDBnsQ"]),
            lines(&["eNpjZICDBnsQ<<<"]),
            lines(&["Error: attempt to call a nil value"]),
        ] {
            assert!(
                parse_map_exchange_reply(&reply).is_err(),
                "should have refused {reply:?}"
            );
        }
    }
}
