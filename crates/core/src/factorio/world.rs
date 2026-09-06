use crate::factorio::snapshot::WorldSnapshot;
use crate::factorio::ticks::ActionOutcome;
use crate::graph::entity_graph::EntityGraph;
use crate::graph::flow_graph::FlowGraph;
use crate::types::{
    ActionId, FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic,
    FactorioItemPrototype, FactorioPlayer, FactorioRecipe, FactorioTile, InventoryResponse,
    PlayerChangedDistanceEvent, PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent,
    PlayerId, Pos, Position, SurfaceId,
};
use dashmap::DashMap;
use image::RgbaImage;
use miette::{IntoDiagnostic, Result};
use parking_lot::Mutex as SyncMutex;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::{fmt, fs};
use tokio::sync::Mutex;

/// The payload of a `"teleport"` writeout, emitted by both of
/// `mods/BotBridge/control.lua`'s remaining `player.teleport` call sites (see
/// `teleport_writeout` there): the two that move a bot out of a ghost's or a
/// blueprint's bounding box before reviving it.
///
/// There used to be a third, the stuck-walk timeout, and it was the only one
/// that carried an `action_id`. It was removed, because teleporting a stalled
/// walk reports *arrival* at a destination the character could not reach -- so
/// the planner never learned a site was unreachable and kept choosing it. A
/// stalled leg now fails the walk, and `FactorioRcon::move_player_timed` asks
/// the game for a fresh path. `action_id` is therefore always absent now: both
/// remaining sites are synchronous RCON calls with no dispatched action to
/// attach to.
///
/// The field stays because 1,619 archived teleport events carry it and must
/// remain readable.
#[derive(Debug, Clone, Deserialize)]
pub struct TeleportEvent {
    pub player_id: PlayerId,
    pub reason: String,
    pub from: Position,
    pub to: Position,
    pub distance: f64,
    #[serde(default)]
    pub action_id: Option<ActionId>,
}

/// The payload of a `"player_died"` writeout: `mods/BotBridge/control.lua`'s
/// `on_player_died` handler, fired by the game's `on_player_died` event.
///
/// A dead player keeps its `LuaPlayer` and loses its `character`; the game
/// respawns one after `ticks_to_respawn` (600 by default) unless a scenario
/// says otherwise. Until this event existed nothing on this side could tell a
/// dead bot from a client that never connected: `get_player` in the mod said
/// `not connected` for both, and the roster -- computed once at script start --
/// went on assigning work to a character that was not there. See
/// `docs/superpowers/specs/2026-09-04-exploration-design.md`, "Enemies".
///
/// Every field but `player_id` is optional on the wire, because each is read
/// under `pcall` in the mod: a death whose cause the game did not name, or
/// whose respawn timer was not readable, is still a death.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DeathEvent {
    pub player_id: PlayerId,
    /// Where the player was when it died -- the ghost's position, which is
    /// where the character stood. `None` when the mod could not read one.
    #[serde(default)]
    pub position: Option<Position>,
    /// The name of the entity that killed the player (`medium-worm-turret`,
    /// `small-biter`, ...), when the game named one.
    #[serde(default)]
    pub cause: Option<String>,
    /// Its prototype type (`turret`, `unit`, ...), when the game named one.
    #[serde(default)]
    pub cause_type: Option<String>,
    /// `LuaPlayer::ticks_to_respawn` as read in the death handler, when the
    /// game had already set it. `None` is "not readable", never "never".
    #[serde(default)]
    pub respawn_in: Option<u32>,
}

/// The payload of a `"player_respawned"` writeout: the character is back.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RespawnEvent {
    pub player_id: PlayerId,
    /// Where the new character stands. `None` if the mod could not read it.
    #[serde(default)]
    pub position: Option<Position>,
}

/// The payload of a `"research_trigger_emulated"` writeout: the mod completed
/// a trigger technology on a headless run because the force had already done
/// what the trigger names (`mods/BotBridge/control.lua`,
/// `emulate_research_triggers`).
///
/// Two spellings of the count, because the mod writes the counter it read:
/// `produced` for a `craft-item` trigger (production statistics or the
/// hand-craft tally, whichever was larger) and `built` for a `build-entity`
/// one. Both default so a line written by an older mod still parses; the
/// record folds them into one `count`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ResearchTriggerEvent {
    pub technology: String,
    /// The trigger's `type`: `craft-item` or `build-entity`.
    pub trigger: String,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub entity: Option<String>,
    pub needed: u32,
    #[serde(default)]
    pub produced: Option<u32>,
    #[serde(default)]
    pub built: Option<u32>,
}

impl ResearchTriggerEvent {
    /// The count that earned the technology, whichever counter it came from.
    pub fn count(&self) -> u32 {
        self.produced.or(self.built).unwrap_or(0)
    }
}

/// The payload of a `"surface_chunk_dropped"` writeout: `on_chunk_generated`
/// in `mods/BotBridge/control.lua` refused a chunk because it is not on
/// Nauvis.
///
/// **The refusal is deliberate and the disclosure is the point.** The world
/// model keys entities, resources and tiles by position alone, so a chunk from
/// a second surface would merge into Nauvis with no error anywhere -- wrong ore
/// amounts, wrong `entity_at`, a `resource_fingerprint` that loses the
/// overlapping tiles entirely. The guard prevents that; until now it also
/// hid it, printing a bare `"unknown surface"` with no `§tick§key§` envelope,
/// so it reached the server log and no record artefact at all. A run that
/// silently discarded a whole planet's chunks looked identical to one that
/// never visited it.
///
/// One line per dropped chunk on the wire, deliberately dumb -- the mod keeps
/// no state, so this survives a save/load. The aggregation into one row per
/// surface happens in [`FactorioSurface::record_surface_chunk_dropped`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SurfaceChunkDropEvent {
    /// The surface that was refused, by name -- see [`SurfaceId`].
    pub surface: SurfaceId,
    /// The refused chunk's top-left corner, in **tile** coordinates -- so
    /// (-32, 64), not chunk (-1, 2).
    ///
    /// Named for what it is. `on_chunk_generated` hands the mod an `area`, and
    /// the mod's own locals for `area.left_top` are called `chunk_x`/`chunk_y`;
    /// a field called `chunk` carrying those numbers would be confidently
    /// about the wrong object, which is the one record defect a reader cannot
    /// detect.
    pub left_top: Position,
}

/// Everything dropped for one surface since the last flush.
///
/// A count rather than a list: a generated planet is tens of thousands of
/// chunks, and the reader's question is "was a surface discarded, which one,
/// and how much of it", not "which tile". The first chunk is kept because a
/// single example is what makes the row concrete.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceChunkDrops {
    /// The tick of the *first* chunk dropped for this surface in this window.
    pub first_tick: u64,
    /// That first chunk's top-left corner, in tile coordinates.
    pub first_left_top: Position,
    /// How many chunks were dropped for this surface in this window.
    pub chunks: u64,
}

/// One entry of the death queue: a bot lost its character, or got one back.
///
/// One queue for both rather than two, because the reader
/// (`record.deaths()`, `crates/scripting_lua`) wants them in the order they
/// happened, and a bot that died and respawned between two flushes must be
/// written as a death *then* a respawn.
#[derive(Debug, Clone, PartialEq)]
pub enum BotLifeEvent {
    Died(DeathEvent),
    Respawned(RespawnEvent),
}

/// A build the game refused with no cause it was willing to name.
///
/// `mods/BotBridge/control.lua`'s `rcon_place_entity` asks
/// `surface.can_place_entity{... build_check_type = manual}` and, when the
/// answer is no, prints one of two things: `§player_blocks_placement§` when
/// the *acting* player is standing in the footprint, and
/// `cannot place item '<item>' because surface.can_place_entity said 'no'`
/// otherwise. Only the second one lands here. The first names its cause,
/// [`crate::factorio::rcon::FactorioRcon::place_entity_timed`] already walks
/// the actor aside and retries it, and the ground it complains about is
/// legitimately buildable the moment the actor steps off — remembering it
/// would fence the planner out of a tile nothing is wrong with.
///
/// # Why this is worth keeping
///
/// The generic refusal failed four separate runs
/// (`docs/superpowers/notes/2026-09-02-placement-refusal*.md`), each time for
/// a different unmodelled obstacle — a forest, a non-roster character, a
/// roster character — and each time the planner re-chose the same tile on the
/// next iteration, because nothing carried the refusal from one plan to the
/// next. This is that carrier: the planner reads it back through
/// `PlanState::from_world` and stops offering the site.
///
/// # What it does *not* claim
///
/// It does not say what is there. The game examined the entity's collision
/// box at this position and said no; the only sound reading is "something in
/// that box blocks a build", which is why `entity` and `position` are kept
/// rather than a bare tile — the box is recoverable from the prototype, and
/// the box is the extent of what was actually tested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementRefusal {
    /// The `game.tick` the refusal was stamped at, or `None` when the reply
    /// carried no stamp. Never defaulted to zero here: a record writing this
    /// out clamps it to the log's own clock rather than inventing a tick.
    pub tick: Option<u64>,
    /// The item the bot was holding — which is also the name the planner
    /// looked the collision box up under when it chose the site, so the box
    /// this refusal covers is recoverable with the same lookup.
    pub entity: String,
    /// The centre the build was aimed at, exactly as dispatched.
    pub position: Position,
    /// The `defines.direction` the build was aimed with, when it is known.
    ///
    /// The box the game tested is the prototype's box **turned this way**: a
    /// steam engine is 2.5 by 4.7 tiles facing north and 4.7 by 2.5 facing
    /// east, so a refusal without its direction names a centre and the wrong
    /// shape around it. `PlanState::from_world` (`crates/planner`) excludes
    /// the turned box when this is present and the north-frame one when it is
    /// not -- which is the reading every ledger entry got before this field
    /// existed, kept for the entries that predate it (`#[serde(default)]`:
    /// a dumped world from an older build still loads).
    #[serde(default)]
    pub direction: Option<u8>,
    /// Whether the game was asked before or after the plan committed to this
    /// site. See [`RefusalSource`].
    pub source: RefusalSource,
    /// What the game found standing in the tested collision box, deduplicated
    /// and sorted by name.
    ///
    /// Both sources fill this now. The pre-flight check always did; a refusal
    /// observed at dispatch carries it since the mod started appending what
    /// it found to the `said 'no'` line (see [`Self::at_dispatch`]), so an
    /// empty list means the game scanned the box and found no entity in it
    /// -- the ground itself (see `tile`) is the answer -- on either source.
    /// The one exception is a dispatch refusal from a mod that did not append
    /// anything, which arrives empty *and* with `tile == None`; that pair is
    /// the signature of "not asked", and `tile` is what tells the two apart.
    pub blockers: Vec<String>,
    /// The name of the tile under the refused centre, when the game reported
    /// one. `None` only when the reply named nothing at all.
    pub tile: Option<String>,
}

impl PlacementRefusal {
    /// A refusal observed **at dispatch**: a bot tried to build `entity` at
    /// `position` facing `direction` and the game said no.
    ///
    /// `blockers` and `tile` are what the mod's own line said stood there --
    /// `rcon_place_entity` scans the box it just had judged and appends the
    /// result, and `note_placement_refusal` (`crate::factorio::rcon`) reads
    /// it back. They are an **observation the mod made**, never something a
    /// caller reconstructs from a model of its own: a refusal is the game
    /// disagreeing with the model, so the model's opinion of the site is the
    /// one thing that cannot explain it. A caller with no such observation
    /// passes an empty list and `None`, which is what the record shows for
    /// every refusal from before the mod named anything.
    pub fn at_dispatch(
        tick: Option<u64>,
        entity: impl Into<String>,
        position: Position,
        direction: u8,
        blockers: Vec<String>,
        tile: Option<String>,
    ) -> Self {
        PlacementRefusal {
            tick,
            entity: entity.into(),
            position,
            direction: Some(direction),
            source: RefusalSource::Dispatch,
            blockers,
            tile,
        }
    }
}

/// When the game was asked about a site — and therefore what the answer cost.
///
/// The two are not redundant with each other and a reader must be able to
/// tell them apart. A `Dispatch` refusal is one a bot flew to the site and
/// tried to build at: an action failed, its dependents were abandoned, and
/// recovery had to escalate. A `PreCheck` refusal cost a plan-time
/// re-expansion and no game action at all — the plan that reaches the
/// executor never contained the bad site.
///
/// Which also means the two carry different evidence: only `PreCheck` can say
/// *what* was in the way, because only it asks a question that has room for
/// an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalSource {
    /// The game refused a build a bot actually attempted.
    #[default]
    Dispatch,
    /// The game answered `can_place_entity` for a site a plan had chosen but
    /// not yet run.
    PreCheck,
}

impl RefusalSource {
    /// The wire spelling, shared by the record's `EventKind::PlacementRefused`
    /// and by anything else that has to name one.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalSource::Dispatch => "dispatch",
            RefusalSource::PreCheck => "pre_check",
        }
    }
}

/// Every [`PlacementRefusal`] this run has collected, plus how many of them a
/// record has already been told about.
///
/// Append-only and **never drained**, unlike
/// [`FactorioSurface::teleports`](FactorioWorld#structfield.teleports): a
/// teleport is an event that needs writing once, while a refusal is a
/// standing fact the planner has to re-read on every plan. The `reported`
/// cursor is what lets `record.refusals()` write each one exactly once
/// without taking it away from the planner.
#[derive(Debug, Default)]
pub struct PlacementRefusals {
    sites: Vec<PlacementRefusal>,
    reported: usize,
}

/// # Serialized as a bare list, and the cursor is not part of it
///
/// A dumped world is read back by something that has been told about none of
/// these -- an offline planner, a seed scorer, a resumed record -- so
/// `reported` restarts at zero on load, exactly as [`FactorioSurface::clone`]
/// resets it and for the same reason it argues there: a second reader has
/// heard nothing, and writing a site twice into two independent records is
/// the honest answer. Nothing in `crates/planner` reads the cursor, so this
/// cannot move a plan; see `crates/planner/tests/world_round_trip.rs`, which
/// pins that.
///
/// Serializing the cursor would also hand a hostile file a `reported` past the
/// end of `sites`, which the `sites[from..]` slice in
/// [`FactorioSurface::unreported_placement_refusals`] would panic on. There is
/// nothing to gain by carrying it and a panic to lose.
impl Serialize for PlacementRefusals {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.sites.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PlacementRefusals {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(PlacementRefusals {
            sites: Vec::deserialize(deserializer)?,
            reported: 0,
        })
    }
}

impl PlacementRefusals {
    /// Remembers a refusal, or does nothing if this exact site is already
    /// known. Returns whether it was new.
    ///
    /// "The same site" is the same entity name at the same centre, compared
    /// exactly. Every position that reaches here came off the integer grid
    /// `free_area_near` searches, so exact comparison is not the fragile
    /// float test it looks like; and a near-miss costs a duplicate entry,
    /// which excludes the same ground twice, rather than a wrong answer.
    ///
    /// `source`, `blockers` and `tile` are deliberately **not** part of the
    /// identity: a site the pre-flight check learned about and a site a
    /// dispatch was refused at are the same fact about the ground, and the
    /// first one recorded is the one kept. In practice that means the
    /// pre-check's answer wins, which is the one carrying the evidence.
    fn note(&mut self, refusal: PlacementRefusal) -> bool {
        if self.sites.iter().any(|known| {
            known.entity == refusal.entity
                && known.position.x.total_cmp(&refusal.position.x).is_eq()
                && known.position.y.total_cmp(&refusal.position.y).is_eq()
        }) {
            return false;
        }
        self.sites.push(refusal);
        true
    }
}

/// A walk the game's pathfinder searched for and did not find.
///
/// # What it claims, which is narrower than "unreachable"
///
/// One character, standing at one place, asked for a route to one
/// destination, and the game answered that there is none. That is a fact
/// about **a pair of points**, not about the destination: a bot on the other
/// side of the obstacle, or the same bot once something has been mined or
/// built, may well have a path. `from` is therefore part of the fact and not
/// decoration, and every reader is expected to test it -- see
/// [`WalkRefusal::applies_to`].
///
/// # Only the definitive answer is recorded here
///
/// `mods/BotBridge/control.lua` distinguishes two refusals and so does this
/// ledger's one writer (`RconActuator::walk`). `try again later` means the
/// request queue was full and the question was never asked; nothing was
/// learned and nothing is remembered. `failed to path find` means the
/// pathfinder searched and came back empty -- and by the time it reaches the
/// writer, `FactorioRcon::player_path` has also searched from four rotated
/// offset goals around the target, so five searches from that spot found
/// nothing.
///
/// # Why remembering it is the point
///
/// Run `run-1788432181-42528` -- the furthest this project has reached --
/// lost most of its remaining throughput to the absence of this type. Bots 2
/// and 3 stopped moving at tick ~53 700, boxed in where the plant was being
/// built around them; every later plan sent them at an ore tile, the walk was
/// refused before dispatch, and `abandon_rest` cut the rest of that bot's
/// chain. Bot 3 was sent to `(-54.5, -12.5)` five separate times, bot 2 to
/// `(-46.5, -9.5)` four times, each from the same frozen position. Nothing
/// carried the refusal from one plan to the next, which is exactly the gap
/// [`PlacementRefusal`] was built to close one level down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkRefusal {
    /// The `game.tick` the refusal was stamped at, or `None` -- which is the
    /// ordinary case, because a path request that finds nothing is refused
    /// *before* the walk is dispatched and so is never stamped. Never
    /// defaulted to zero, for the reason [`PlacementRefusal::tick`] gives.
    pub tick: Option<u64>,
    /// The Factorio player whose walk was refused. A [`crate::types::PlayerId`]
    /// rather than a bot id because this crate has no bot ids; they are the
    /// same number (`BotId(3)` is player 3).
    pub player: PlayerId,
    /// Where the character was standing when it asked, exactly as the world
    /// reported it.
    pub from: Position,
    /// The destination the plan named -- the scheduler's `StepKind::Walk.to`,
    /// not the offset goal `approach_annulus` derived from it, so a reader
    /// can compare it against a plan without redoing that arithmetic.
    pub to: Position,
}

impl WalkRefusal {
    /// How far a character may have drifted and still count as standing where
    /// this refusal was earned, in tiles.
    ///
    /// The fact recorded is "no path *from here*", so a reader has to decide
    /// what "here" means, and a float equality test would make the answer
    /// unusable: the position a plan reasons with is a snapshot taken at a
    /// different instant from the one the walk was dispatched with.
    ///
    /// One tile is the smallest radius that survives that, and the choice
    /// barely matters for the case this exists for -- a boxed-in bot reports
    /// the *same* position for tens of thousands of ticks, while a bot that
    /// genuinely relocated has moved tens of tiles. Erring small is
    /// deliberate: a miss costs one repeated refusal, a false hit fences a bot
    /// away from ground it can reach.
    pub const SAME_PLACE_TOLERANCE: f64 = 1.0;

    /// Whether this refusal answers the question `player` is now asking.
    ///
    /// Three conditions, and the middle one is the whole design:
    ///
    /// * the same character -- two bots on opposite sides of an obstacle are
    ///   not asking the same question, and answering one with the other's
    ///   evidence is a claim nobody established;
    /// * from within [`WalkRefusal::SAME_PLACE_TOLERANCE`] of where the
    ///   refusal was earned -- once the bot is somewhere else, this refusal
    ///   has nothing to say and the destination is offered again;
    /// * to exactly the same destination -- compared with `total_cmp`,
    ///   exactly as [`PlacementRefusals::note`] compares sites, because a plan
    ///   re-deriving the same site derives the same coordinates bit for bit,
    ///   and widening this into a radius would fence off neighbouring ground
    ///   the game never refused.
    ///
    /// The walk's `radius`/`min_radius` are deliberately not part of the test.
    /// They decide where near the target the walk may stop, and this ledger
    /// only ever holds the answer to "is there a route toward it at all",
    /// which came back no for the goal *and* for four offsets around it.
    pub fn applies_to(&self, player: PlayerId, from: &Position, to: &Position) -> bool {
        self.player == player
            && self.to.x.total_cmp(&to.x).is_eq()
            && self.to.y.total_cmp(&to.y).is_eq()
            && (self.from.x - from.x)
                .hypot(self.from.y - from.y)
                .total_cmp(&Self::SAME_PLACE_TOLERANCE)
                .is_le()
    }
}

/// Every [`WalkRefusal`] this run has collected.
///
/// Append-only and never drained, for the same reason
/// [`PlacementRefusals`] is: a refusal is a standing fact a planner re-reads
/// on every plan, not an event to be written once. There is no `reported`
/// cursor here because nothing writes a record event for it -- the run
/// archive already carries every failed walk, with its error text, as
/// `walk_settled`.
#[derive(Debug, Default)]
pub struct WalkRefusals {
    walks: Vec<WalkRefusal>,
}

/// Serialized as a bare list of the walks, for the reason
/// [`PlacementRefusals`]' own serde impl gives. This one has no cursor to
/// drop.
impl Serialize for WalkRefusals {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.walks.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WalkRefusals {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(WalkRefusals {
            walks: Vec::deserialize(deserializer)?,
        })
    }
}

impl WalkRefusals {
    /// Remembers a refusal, or does nothing if this exact question has already
    /// been asked and answered. Returns whether it was new.
    ///
    /// "The same question" is the same player, from the same position, to the
    /// same destination, all compared exactly -- a stricter test than
    /// [`WalkRefusal::applies_to`] uses to *read* the ledger, and stricter on
    /// purpose. Deduplication decides what is stored; a near-miss costs one
    /// duplicate row. Matching decides what is believed; a near-miss there
    /// would cost a bot ground it can walk on.
    fn note(&mut self, refusal: WalkRefusal) -> bool {
        if self.walks.iter().any(|known| {
            known.player == refusal.player
                && known.from.x.total_cmp(&refusal.from.x).is_eq()
                && known.from.y.total_cmp(&refusal.from.y).is_eq()
                && known.to.x.total_cmp(&refusal.to.x).is_eq()
                && known.to.y.total_cmp(&refusal.to.y).is_eq()
        }) {
            return false;
        }
        self.walks.push(refusal);
        true
    }
}

/// A character that cannot reach open ground from where it stands.
///
/// # What it claims
///
/// A flood fill over the occupancy model
/// ([`crate::graph::enclosure::escape_from`]) started at `at` and closed
/// without reaching the edge of a window `searched_tiles` across. So: every
/// tile the game's pathfinder would route this character to is inside a
/// pocket of `pocket_tiles` whole tiles, and it gets out only by mining, by
/// being moved, or by something in the way being removed.
///
/// It is a fact about a **position**, not about a bot. A bot teleported clear,
/// or one whose wall is mined away, is no longer enclosed and this row says
/// nothing about it -- exactly as [`WalkRefusal`] is a fact about a pair of
/// points rather than about a destination.
///
/// # Why it is a ledger row and not a log line
///
/// `run-1788432181-42528` ran its whole budget with two of four bots frozen
/// from tick ~48 000, and nothing anywhere named the condition. Twenty walks
/// failed on bots 2/3/4 and none on bot 1; bot 1 performed 746 of 831
/// dispatches. That ~90% concentration read as a scheduling quirk for days.
/// The record is where a run is read back after the fact, so a condition that
/// only reaches a log line is a condition nobody finds -- which is the failure
/// this type exists to stop repeating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enclosure {
    /// The `game.tick` the observation was stamped at, or `None`. Ordinarily
    /// `None`: the walk refusal that triggers the check is answered *before*
    /// the walk is dispatched and so is never stamped. Never defaulted to
    /// zero, for the reason [`PlacementRefusal::tick`] gives.
    pub tick: Option<u64>,
    /// The Factorio player the check was run for. A [`PlayerId`] rather than a
    /// bot id because this crate has no bot ids; they are the same number.
    pub player: PlayerId,
    /// Where the character was standing. The fill was seeded here, so this is
    /// the position the claim is about and not an approximation of it.
    pub at: Position,
    /// How much ground the character can still reach, in square tiles of
    /// *configuration space* -- obstacles grown by the character's own
    /// half-box, so this is where its centre may go rather than the floor area
    /// a person would measure by eye. Reported because "boxed into 3 square
    /// tiles" and "boxed into 300" are different situations.
    pub pocket_tiles: f64,
    /// The radius of the window the fill was allowed to search, in tiles.
    /// Carried so a reader knows how strong the claim is: an enclosure larger
    /// than this window is invisible to the search and is reported as no
    /// enclosure at all.
    pub searched_tiles: f64,
}

/// Every [`Enclosure`] this run has observed, plus how many of them a record
/// has already been told about.
///
/// Append-only and never drained, and carrying a `reported` cursor, exactly
/// like [`PlacementRefusals`] and for the same two reasons: the condition is a
/// standing fact rather than an event, and `record.enclosures()` has to write
/// each one once without taking it away from anyone else who reads the ledger.
#[derive(Debug, Default)]
pub struct Enclosures {
    found: Vec<Enclosure>,
    reported: usize,
}

/// Serialized as a bare list, with `reported` restarting at zero on load --
/// see [`PlacementRefusals`]' serde impl for the whole argument, which applies
/// here unchanged.
impl Serialize for Enclosures {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.found.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Enclosures {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Enclosures {
            found: Vec::deserialize(deserializer)?,
            reported: 0,
        })
    }
}

impl Enclosures {
    /// Remembers an enclosure, or does nothing if this character has already
    /// been found enclosed at this spot. Returns whether it was new.
    ///
    /// Identity is the player and the position, within
    /// [`WalkRefusal::SAME_PLACE_TOLERANCE`] -- and that tolerance rather than
    /// an exact float test is the point. A boxed-in bot is asked about once
    /// per refused walk, which in `run-1788432181-42528` was five times for
    /// one bot from one spot; the position reported for it drifts in the last
    /// bits between reads even when the bot has not moved a tile. Comparing
    /// exactly would write the same condition five times and make the record
    /// read as five separate findings.
    ///
    /// `pocket_tiles` is deliberately not part of the identity: the pocket
    /// grows and shrinks as things are built and mined around a bot that is
    /// still stuck in the same place, and that is the same condition, not a
    /// new one.
    fn note(&mut self, found: Enclosure) -> bool {
        if self.found.iter().any(|known| {
            known.player == found.player
                && (known.at.x - found.at.x)
                    .hypot(known.at.y - found.at.y)
                    .total_cmp(&WalkRefusal::SAME_PLACE_TOLERANCE)
                    .is_le()
        }) {
            return false;
        }
        self.found.push(found);
        true
    }
}

/// A character that was walked clear of a placement which would otherwise
/// have sealed it in -- the enclosure that did *not* happen.
///
/// # Why it is a record entry and not a log line
///
/// Run `run-1788552801-73005` is the case. Bot 1 walked to the east side of
/// the site it was about to build `assembling-machine-1 [31.5, -4.5]` on --
/// a two-by-two patch of ground between an older cell's chest column and its
/// pole, every exit from which crossed the footprint -- placed it four ticks
/// after arriving, and never moved again: 40 000 ticks, three refused walks,
/// no event. The executor now asks, before every placement, whether the
/// footprint would close the fill around the character that is about to
/// build it (`crates/core::graph::enclosure::escape_with`), and walks it to
/// the nearest tile that stays open first. That walk is a bot doing
/// something the plan did not ask for, and a reader comparing the plan
/// against the record has to be able to see why -- the same reason a
/// teleport or a refusal is written rather than logged.
///
/// Ephemeral, like [`FactorioSurface::teleports`]: this is an event that
/// happened, not knowledge the next plan needs, so it is queued for
/// `record.enclosures()` to drain and is never serialised with the world.
#[derive(Debug, Clone, PartialEq)]
pub struct StepAside {
    /// The `game.tick` the step-aside walk settled at, or `None` if the game
    /// never stamped it.
    pub tick: Option<u64>,
    pub player: PlayerId,
    /// Where the character stood when the placement was about to be made.
    pub from: Position,
    /// The tile centre it was walked to instead.
    pub to: Position,
    /// The placement that would have sealed it in, as `name` and position.
    pub placing: String,
    pub site: Position,
    /// Why the walk was made. See [`StepAsideReason`].
    pub reason: StepAsideReason,
    /// How many tiles the character would have been left with, had it stayed.
    ///
    /// For [`StepAsideReason::Footprint`] this is `0.0`, and literally so:
    /// the placement would stand on the character's own tile.
    pub pocket_tiles: f64,
}

/// Why the pre-place check walked a character before a build.
///
/// Two conditions, one walk. They are told apart in the record because they
/// say different things about the plan: an `Enclosure` is a site the plan
/// chose legally whose *surroundings* made the stand-point a trap, while a
/// `Footprint` is the acting bot standing where the plan is about to build
/// -- the plan's annulus (`Condition::AtPosition { min_radius }`) said not
/// to, and the walk that brought the bot here landed inside it anyway, which
/// is a fact about the walker, not the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepAsideReason {
    /// The placement would have walled the character in where it stood.
    Enclosure,
    /// The character stood inside the placement's own collision box, so the
    /// game would have refused the build for the actor's sake.
    Footprint,
}

impl StepAsideReason {
    /// The wire spelling, shared by the record's `EventKind::BotSteppedAside`.
    pub fn as_str(self) -> &'static str {
        match self {
            StepAsideReason::Enclosure => "enclosure",
            StepAsideReason::Footprint => "footprint",
        }
    }
}

/// How far from the character each mobility-probe hop is aimed, in tiles.
///
/// Three tiles: far enough that the target is outside the character's own
/// tile and the one beside it, so reaching it proves the character can
/// *leave*, and near enough that a free character in a corridor between two
/// furnace rows still has some direction it can go. Paired with
/// [`HOP_RADIUS`], each probe accepts anywhere from one to five tiles out.
pub const HOP_DISTANCE: f64 = 3.0;

/// The radius each hop's path request is allowed to stop short in.
///
/// Two tiles, so that a hop whose exact target happens to be inside a
/// building is still satisfied by the ground beside it. A probe that had to
/// land on one named point would read "the tile three east is a furnace" as
/// "the character cannot move east".
pub const HOP_RADIUS: f64 = 2.0;

/// The four points a mobility probe asks the game for a path to, from
/// `from`: [`HOP_DISTANCE`] tiles north, east, south and west, in that order.
pub fn hop_targets(from: &Position) -> [Position; 4] {
    [
        Position::new(from.x(), from.y() - HOP_DISTANCE),
        Position::new(from.x() + HOP_DISTANCE, from.y()),
        Position::new(from.x(), from.y() + HOP_DISTANCE),
        Position::new(from.x() - HOP_DISTANCE, from.y()),
    ]
}

/// A character the *game* has said cannot move from where it stands, and
/// which the next plan must therefore not send anywhere.
///
/// # What it claims, and who established it
///
/// After a walk the pathfinder refused, the executor asked the game for a
/// short path from the character to each of [`hop_targets`], and every one
/// of them came back `failed to path find`. That is the game's own verdict
/// on the *character*, as distinct from a [`WalkRefusal`], which is the
/// game's verdict on one (from, to) pair, and from an [`Enclosure`], which is
/// this crate's flood fill over its occupancy model.
///
/// # Why a third ledger, when two already exist
///
/// Run `run-1788614781-38058` is the case. Bot 6 stood at
/// `(-5.2, -29.1)`, overlapping a stone furnace another bot had placed at
/// `[-5, -28]`. Every path request from it was refused -- four walks, always
/// to a destination other bots reached. The walk ledger held the pair, and
/// the fill said `Open`: the furnace's box grown by the character's half-box
/// did not cover the tile centre the fill seeds from, so the model saw a
/// free cell in a free region. The model was internally right and the
/// character could not move. Seven plans sent it the same walk, and the run
/// halted `stuck` with seven healthy bots idle.
///
/// The fill reasons from a model that holds no characters and rasterises to
/// tile centres; the pathfinder reasons from the character's real box at its
/// real position. Only the second can answer "can this character leave its
/// tile", so that is what is asked, and this is where the answer is kept.
///
/// # It is a fact about a position, and it is released
///
/// A bench names the position it was earned at. A bot that has since moved --
/// pushed clear by a step-aside, or by anything else -- is not the bot the
/// game answered for, and `crates/planner`'s `PlanState::from_world` treats a
/// bench more than [`WalkRefusal::SAME_PLACE_TOLERANCE`] from the bot's
/// current position as not applying. The executor also *releases* a bench
/// outright when a walk for that player succeeds, or when a re-probe before
/// the next plan finds a hop the game will path -- see
/// [`FactorioSurface::release_bench`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bench {
    /// The `game.tick` the verdict was stamped at, or `None` -- ordinarily
    /// `None`, because a refused path request is answered before anything is
    /// dispatched. Never defaulted to zero, for the reason
    /// [`PlacementRefusal::tick`] gives.
    pub tick: Option<u64>,
    /// The Factorio player. A [`PlayerId`] rather than a bot id because this
    /// crate has no bot ids; they are the same number.
    pub player: PlayerId,
    /// Where the character stood when every hop was refused.
    pub at: Position,
    /// How many hops were asked for and refused -- four, unless a probe is
    /// ever narrowed. Carried so the record says how strong the claim is.
    pub refused_hops: u8,
    /// How far each hop was aimed, in tiles: [`HOP_DISTANCE`] as this build
    /// had it.
    pub hop_tiles: f64,
}

/// Why a bench was lifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchRelease {
    /// A walk for this player succeeded, so it can plainly move.
    Walked,
    /// A re-probe found a hop the game would path.
    Probed,
}

impl BenchRelease {
    /// The wire spelling, shared by the record's `EventKind::BotReleased`.
    pub fn as_str(self) -> &'static str {
        match self {
            BenchRelease::Walked => "walked",
            BenchRelease::Probed => "probed",
        }
    }
}

/// One change to the bench ledger, queued for the record.
///
/// Ephemeral, like [`StepAside`]: the *state* of the ledger is what the next
/// plan reads, and it is serialised with the world; the transitions are
/// events for `record.enclosures()` to drain and are not.
#[derive(Debug, Clone, PartialEq)]
pub enum BenchChange {
    Benched(Bench),
    Released {
        tick: Option<u64>,
        player: PlayerId,
        at: Position,
        why: BenchRelease,
    },
}

/// Every character the game has said cannot move, as of now, plus the
/// changes nobody has recorded yet.
///
/// Keyed by player, because unlike the two append-only ledgers this one is a
/// *current* state: a bot is benched or it is not, and the answer changes.
#[derive(Debug, Default)]
pub struct Benches {
    active: BTreeMap<PlayerId, Bench>,
    changes: Vec<BenchChange>,
}

/// Serialised as the bare list of active benches, in player order; the
/// change queue is ephemeral and restarts empty on load.
impl Serialize for Benches {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.active
            .values()
            .cloned()
            .collect::<Vec<Bench>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Benches {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let rows: Vec<Bench> = Vec::deserialize(deserializer)?;
        Ok(Benches {
            active: rows.into_iter().map(|row| (row.player, row)).collect(),
            changes: Vec::new(),
        })
    }
}

impl Benches {
    /// Benches a player, or does nothing if it is already benched within
    /// [`WalkRefusal::SAME_PLACE_TOLERANCE`] of this spot. Returns whether
    /// anything changed.
    ///
    /// A bench earned somewhere else replaces the old one: the bot moved
    /// between the two, so the old fact was about a position it no longer
    /// occupies, and the record gets a fresh row saying where it is stuck now.
    fn note(&mut self, bench: Bench) -> bool {
        let same_spot = self.active.get(&bench.player).is_some_and(|known| {
            (known.at.x - bench.at.x)
                .hypot(known.at.y - bench.at.y)
                .total_cmp(&WalkRefusal::SAME_PLACE_TOLERANCE)
                .is_le()
        });
        if same_spot {
            return false;
        }
        self.active.insert(bench.player, bench.clone());
        self.changes.push(BenchChange::Benched(bench));
        true
    }

    /// Lifts a player's bench, if it has one. Returns whether it had one.
    fn release(&mut self, player: PlayerId, tick: Option<u64>, why: BenchRelease) -> bool {
        let Some(bench) = self.active.remove(&player) else {
            return false;
        };
        self.changes.push(BenchChange::Released {
            tick,
            player,
            at: bench.at,
            why,
        });
        true
    }
}

/// What a container or machine was last observed to be holding.
///
/// # Why this is not a field on the stored [`FactorioEntity`]
///
/// `FactorioEntity` already carries `output_inventory` and `fuel_inventory`,
/// and the entity graph already stores a `FactorioEntity` per entity — so the
/// obvious place for this is "refresh the stored entity". It is the wrong
/// place, for two reasons that are both about
/// [`EntityGraph`](crate::graph::entity_graph::EntityGraph) rather than about
/// inventories:
///
/// * `EntityGraph::add` **refuses** an entity when something is already at
///   that position (it warns `failed to add ... blocked by ...` and skips), so
///   a refresh cannot simply re-add. It would have to remove first.
/// * `EntityGraph::remove` clears every `blocked_tree` box that *intersects*
///   the entity's bounding box, not only the entity's own, and every
///   `entity_tree` entry of the same name in that box. That over-removal is
///   tolerable once, when the game really did destroy something; paid on
///   every replan by a refresh loop it would quietly erode the planner's
///   model of what ground is occupied.
///
/// So contents live beside the graph rather than inside it. The split is also
/// the honest one: the graph models **geometry**, which changes when something
/// is built or destroyed, and this models **contents**, which change
/// continuously. They have different refresh rates and different truth
/// horizons, and giving them one home would give them one staleness story.
///
/// # Snake-cased counts, not the wire shape
///
/// The game reports `Vec<InventoryItemWithQuality>`; this keeps a
/// `BTreeMap<item, count>` with quality summed away, exactly as
/// [`FactorioSurface::player_changed_main_inventory`] does for a player. Both
/// readers ask "how much of X is in there", nothing here reasons about
/// quality, and a map answers that without a linear scan. Ordered, so a
/// caller that iterates gets the same order every time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservedInventory {
    /// The entity's name, as the query named it. Kept so a reader can check
    /// that the thing standing here now is still the thing these contents
    /// were read out of — a position alone cannot say that.
    pub name: String,
    /// The centre the contents were read at, unrounded. [`Pos`] is the key and
    /// floors; this is what the reading was actually taken at.
    pub position: Position,
    /// `LuaEntity::get_output_inventory()`: a chest's whole inventory, a
    /// furnace's result slot, an assembler's output slot.
    pub output: BTreeMap<String, u32>,
    /// `LuaEntity::get_fuel_inventory()`.
    pub fuel: BTreeMap<String, u32>,
}

impl ObservedInventory {
    /// Sums an inventory reply's item list by name, discarding quality.
    fn fold(items: &Option<Vec<crate::types::InventoryItemWithQuality>>) -> BTreeMap<String, u32> {
        let mut out: BTreeMap<String, u32> = BTreeMap::new();
        for item in items.iter().flatten() {
            *out.entry(item.name.clone()).or_insert(0) += item.count;
        }
        out
    }

    /// True when the entity holds nothing at all in either inventory.
    pub fn is_empty(&self) -> bool {
        self.output.is_empty() && self.fuel.is_empty()
    }
}

/// Every surface this process knows about, keyed by [`SurfaceId`].
///
/// # Why the container carries the surface and [`Position`] does not
///
/// A coordinate is only ever comparable within one surface, which
/// `Position { x, y }` already says correctly. Putting a surface *on* the
/// value would force `p1 - p2`, `Sub`, `manhattan_distance` and `PartialEq`
/// to answer "how far is Nauvis from Vulcanus?", whose honest answer --
/// undefined -- cannot be returned as an `f64`. So the surface goes on the
/// container, and every `Pos`-keyed map inside a [`FactorioSurface`] keeps
/// its integer tile key unchanged, because there is a **separate map per
/// surface**. A chest at (10, 10) on Nauvis and a chest at (10, 10) on a
/// platform cannot collide, by construction rather than by care. See
/// `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.
///
/// # What is per-surface and what is not
///
/// This is the design decision this type exists to record, and getting it
/// wrong in the other direction is a subtler version of the same aliasing
/// bug: **duplicated global state lets two surfaces disagree about what is
/// researched.**
///
/// **Per-surface** -- everything spatial, and everything a `Pos` keys:
///
/// | Field of [`FactorioSurface`] | Why |
/// |---|---|
/// | `entity_graph` | Four quadtrees over one ±5120 coordinate space, plus `resources`/`minables`/`threats` as `BTreeMap<Pos, _>`. This is the aliasing case itself. |
/// | `flow_graph` | Derived from exactly one entity graph. |
/// | `inventories` | `DashMap<Pos, ObservedInventory>`. |
/// | `placement_refusals`, `walk_refusals`, `enclosures`, `step_asides` | Every one is a fact about a *place*: a site the game refused, a spot a walk could not leave. None of it says anything about the same coordinates elsewhere. |
///
/// **Game- or force-global** -- must exist once per world, never once per
/// surface:
///
/// | Field | Why |
/// |---|---|
/// | `forces` | A force's technologies and research progress are force-wide; `LuaForce::technologies` is not surface-indexed. Two copies is the disagree-about-research bug by name. |
/// | `recipes`, `entity_prototypes`, `item_prototypes`, `graphics`, `image_cache` | Prototype data, loaded once per save from the mod set. It does not vary by surface. (Recipe *availability* varies by force, not by surface.) |
/// | `actions`, `next_action_id`, `path_requests` | One id space for the session. Two counters would hand two surfaces the same `action_id`, and the executor's completion signal is keyed on exactly that. |
/// | `research_triggers` | Force-level facts, like `forces`. |
/// | `teleports`, `deaths` | A teleport *crosses* surfaces -- that is what makes it a teleport -- so it belongs to neither endpoint. Respawn likewise: the mod respawns a dead bot on `game.surfaces[1]` wherever it died. |
/// | `surface_chunk_drops` | Already keyed by [`SurfaceId`], and it is about surfaces this world deliberately does **not** hold. It could only ever belong to the aggregate. |
///
/// **Genuinely ambiguous, and said so rather than guessed:**
///
/// * `players`. A bot has one identity across the whole game and stands on
///   exactly one surface at a time; [`FactorioPlayer`] already carries a
///   `surface: Option<SurfaceId>`. The map is keyed by `PlayerId`, so it
///   cannot alias -- but `crates/planner`'s `PlanState` reads it as "the
///   bots I may give steps to", which is a per-surface question. Splitting
///   it per surface would mean *moving* a row when a bot crosses, an
///   operation nothing in this project can currently perform. Left global,
///   flagged here.
/// * `benches`. Keyed by `PlayerId` like `players`, but the fact it holds
///   ("this bot cannot move from *here*") is positional. It follows
///   `players` for now, for the same reason.
///
/// # One surface today, and it refuses to hold two
///
/// The split above is **written down and not yet enforced by the types**:
/// every field named global still lives on [`FactorioSurface`], so a second
/// surface added here would duplicate them. [`FactorioWorld::insert_surface`]
/// therefore **refuses** a second surface by name
/// ([`SurfaceNotYetSeparable`](crate::errors::SurfaceNotYetSeparable))
/// rather than accepting one and quietly forking the research state. A
/// refusal that names the reason is worth more than a container that is
/// silently wrong; this repo has paid for the other choice enough times to
/// have a rule about it. Lifting the refusal means moving the global fields
/// off the surface first, which is a mechanical change of its own and is
/// deliberately not in this commit.
///
/// The mod's Nauvis guard in `mods/BotBridge/control.lua` is the matching
/// half upstream: no non-Nauvis chunk reaches Rust at all, and what it drops
/// is recorded in `surface_chunk_drops`.
pub struct FactorioWorld {
    surfaces: BTreeMap<SurfaceId, Arc<FactorioSurface>>,
}

impl FactorioWorld {
    /// A world holding exactly the one surface it was handed, under `id`.
    pub fn new(id: SurfaceId, surface: Arc<FactorioSurface>) -> Self {
        let mut surfaces = BTreeMap::new();
        surfaces.insert(id, surface);
        FactorioWorld { surfaces }
    }

    /// A world holding one Nauvis surface. The shape every run has had so
    /// far, said out loud instead of assumed.
    pub fn nauvis_only(surface: Arc<FactorioSurface>) -> Self {
        FactorioWorld::new(SurfaceId::nauvis(), surface)
    }

    /// The surface under `id`, or `None` when this world has never seen it.
    ///
    /// `None` means **not observed**, never "empty": the mod drops every
    /// chunk that is not on Nauvis, so an unknown surface is a surface
    /// nothing was ever told about.
    pub fn surface(&self, id: &SurfaceId) -> Option<&Arc<FactorioSurface>> {
        self.surfaces.get(id)
    }

    /// The Nauvis surface, when this world has one.
    pub fn nauvis(&self) -> Option<&Arc<FactorioSurface>> {
        self.surface(&SurfaceId::nauvis())
    }

    /// The one surface this world holds, for callers written before there
    /// could be more than one.
    ///
    /// **This is the porting seam, and it is deliberately not `nauvis()`.**
    /// A caller reaching through here is one that has not yet been told
    /// which surface it means; it returns `None` on an empty world and would
    /// have to be looked at again on a world with two, which is exactly the
    /// review the later rungs need. `nauvis()` is for a caller that genuinely
    /// means Nauvis.
    pub fn only_surface(&self) -> Option<&Arc<FactorioSurface>> {
        let mut surfaces = self.surfaces.values();
        let first = surfaces.next()?;
        // Not `is_empty`-style: a second surface cannot exist today, and if
        // one ever does this must stop answering rather than pick one.
        match surfaces.next() {
            None => Some(first),
            Some(_) => None,
        }
    }

    /// Every surface id this world holds, in name order.
    pub fn surface_ids(&self) -> impl Iterator<Item = &SurfaceId> {
        self.surfaces.keys()
    }

    /// How many surfaces this world holds. One, today, always.
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    /// True when nothing has been observed yet.
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Adds a surface, or refuses because the world's global state has not
    /// been separated from the surface's yet.
    ///
    /// Re-inserting the id this world already holds is accepted and replaces
    /// it -- that is one surface being refreshed, not two coexisting. Any
    /// *other* id is refused: see the type's doc for what would be
    /// duplicated and why a refusal is the honest answer.
    pub fn insert_surface(
        &mut self,
        id: SurfaceId,
        surface: Arc<FactorioSurface>,
    ) -> Result<(), crate::errors::SurfaceNotYetSeparable> {
        if !self.surfaces.contains_key(&id)
            && let Some(held) = self.surfaces.keys().next()
        {
            return Err(crate::errors::SurfaceNotYetSeparable {
                held: held.clone(),
                offered: id,
            });
        }
        self.surfaces.insert(id, surface);
        Ok(())
    }
}

/// One surface's model of the game -- **and, for now, the game-global state
/// beside it.**
///
/// This type was called `FactorioWorld` until 2026-09-06 and was never a
/// world: one `EntityGraph`, one `FlowGraph`, one set of `Pos`-keyed
/// overlays, all of them describing a single surface. [`FactorioWorld`] is
/// now the aggregate that owns surfaces, and its doc carries the field-by-
/// field argument for which of the fields below are per-surface and which
/// are global. Read it before adding a field here.
pub struct FactorioSurface {
    pub players: DashMap<PlayerId, FactorioPlayer>,
    pub forces: DashMap<String, FactorioForce>,
    pub graphics: DashMap<String, FactorioGraphic>,
    pub recipes: Arc<DashMap<String, FactorioRecipe>>,
    pub entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    pub item_prototypes: DashMap<String, FactorioItemPrototype>,
    pub image_cache: DashMap<String, Box<RgbaImage>>,
    /// Outcomes the game reported for dispatched actions, keyed by the
    /// `action_id` the dispatch used.
    ///
    /// The value carries the game tick alongside the result. The mod has always
    /// stamped one on its `action_completed` event; until this map could hold
    /// it, `OutputParser` threw it away -- which is why the executor had no
    /// game-clock source, and why every "duration" it reported was a plan value
    /// round-tripped through the log.
    pub actions: DashMap<u32, ActionOutcome>,
    pub path_requests: DashMap<u32, String>,
    pub next_action_id: Mutex<u32>,
    pub entity_graph: Arc<EntityGraph>,
    pub flow_graph: Arc<FlowGraph>,
    /// Teleports the mod has reported since the last
    /// [`FactorioSurface::drain_teleports`], each tagged with the game tick the
    /// mod stamped on its `writeout` line.
    ///
    /// `OutputParser` (`crates/core/src/process/output_parser.rs`) parses the
    /// line and pushes here; `crates/scripting_lua`'s `record.teleports()`
    /// drains it into `events.jsonl`. The queue exists because those two live
    /// in different crates and the dependency only runs one way -- this
    /// crate cannot depend on `crates/scripting_lua`, where the run recorder
    /// lives -- so a teleport crosses the boundary as data sitting here
    /// rather than as a direct call, the same shape `actions` already uses
    /// for `action_completed`.
    pub teleports: SyncMutex<Vec<(u64, TeleportEvent)>>,
    /// Deaths and respawns the mod has reported since the last
    /// [`FactorioSurface::drain_deaths`], each tagged with the game tick the
    /// mod stamped on its `writeout` line. Same shape and same reason as
    /// `teleports`: `OutputParser` pushes, `record.deaths()` drains.
    pub deaths: SyncMutex<Vec<(u64, BotLifeEvent)>>,
    /// Trigger technologies the mod has completed on a headless run since the
    /// last [`FactorioSurface::drain_research_triggers`], each tagged with the
    /// game tick. Same shape as `deaths`: `OutputParser` pushes,
    /// `record.research_triggers()` drains. Until this queue existed the
    /// mod's line reached only the server log, and `events.jsonl` showed a
    /// research finishing with no research ever started.
    pub research_triggers: SyncMutex<Vec<(u64, ResearchTriggerEvent)>>,
    /// Chunks the mod refused because they are not on Nauvis, aggregated per
    /// surface since the last [`FactorioSurface::drain_surface_chunk_drops`].
    ///
    /// **A map, not a `Vec`, and that is the whole design.** The mod writes one
    /// line per dropped chunk because keeping a counter there would mean
    /// keeping it in `storage` across save/load; a generated planet is tens of
    /// thousands of chunks, and tens of thousands of `events.jsonl` rows saying
    /// the same thing is not a disclosure, it is a denial of service on the
    /// reader. Folding here costs one map lookup per line and gives the record
    /// one row per surface per flush, carrying the count.
    pub surface_chunk_drops: SyncMutex<BTreeMap<SurfaceId, SurfaceChunkDrops>>,
    /// Sites the game has refused a build at, for the life of this world.
    ///
    /// Two readers, which is why it sits here rather than in either of them.
    /// `crates/planner`'s `PlanState::from_world` reads it on **every** plan
    /// and excludes the refused footprints, which is what stops a replan
    /// re-choosing the tile the game just turned down; `crates/scripting_lua`'s
    /// `record.refusals()` writes the new ones into `events.jsonl` so the
    /// avoidance is visible to whoever reads the run back. Neither crate can
    /// see the other, and both can see this.
    ///
    /// **Believed for as long as this world lives, and not expired.** See
    /// [`PlacementRefusal`] for what a refusal does and does not claim; the
    /// horizon is argued in
    /// `docs/superpowers/notes/2026-09-02-refusal-memory.md`. In short: every
    /// obstacle the planner *can* model — characters, entities, ore, terrain —
    /// is already re-read from this world on every plan, so a refusal only
    /// ever carries what the model cannot see, and an unexplained obstruction
    /// is not something a clock can talk us out of. Forgetting it immediately
    /// is the behaviour that cost four runs.
    /// What each container and machine was last observed to be holding,
    /// keyed by the tile its centre sits on.
    ///
    /// # Pull, not push, and why there is no event to listen to
    ///
    /// Nothing writes here on its own. Factorio raises no event for "a
    /// chest's contents changed" that a mod could cheaply subscribe to, and
    /// the one event this crate does receive that carries an inventory —
    /// `on_some_entity_created`, through
    /// [`FactorioSurface::on_some_entity_created`] — describes an entity at the
    /// instant it was built, which for a chest or a furnace means an empty
    /// one. `on_some_entity_updated` is not the missing channel either: the
    /// mod raises it from exactly one subscription,
    /// `defines.events.on_player_rotated_entity`, so it fires when something
    /// turns and never when something is filled.
    ///
    /// So contents are **asked for** — one `inventory_contents_at` RCON call
    /// naming the entities a caller cares about — and the answer is written
    /// here. That makes staleness bounded by the caller's own choice of when
    /// to ask, rather than by an event that may never come.
    ///
    /// # Deterministic to read
    ///
    /// A `DashMap` iterates in hash order, which moves with the hash seed.
    /// Every reader must go through [`FactorioSurface::observed_inventories`],
    /// which sorts by [`Pos`]; `crates/planner` reads it exactly once per
    /// plan, in [`PlanState::from_world`], and keys it into a `BTreeMap`.
    ///
    /// # Forgotten when the entity goes
    ///
    /// [`FactorioSurface::on_some_entity_deleted`] clears the entry. A mined
    /// furnace hands its contents to whoever mined it, so leaving the reading
    /// standing would report items that are now in a player's pocket as
    /// still sitting on the ground — and a later build on that same tile
    /// would inherit them.
    pub inventories: DashMap<Pos, ObservedInventory>,
    pub placement_refusals: SyncMutex<PlacementRefusals>,
    /// Destinations the game's pathfinder has refused a bot a route to, for
    /// the life of this world.
    ///
    /// The walking half of [`FactorioSurface::placement_refusals`], and here for
    /// the same reason: `crates/planner`'s `PlanState::from_world` reads it on
    /// every plan, and the crate that writes it (`crates/executor`) cannot see
    /// the crate that reads it. See [`WalkRefusal`] for what one claims, which
    /// is narrower than the placement ledger's claim -- it is about a pair of
    /// points, not about a destination.
    pub walk_refusals: SyncMutex<WalkRefusals>,
    /// Characters this run has found unable to reach open ground from where
    /// they stand.
    ///
    /// Not read by the planner -- this ledger exists to be *reported*, which
    /// is the whole finding of `run-1788432181-42528`: the condition was real
    /// for two of four bots for 163 000 ticks and appeared in no artefact.
    /// Acting on it is a separate change with a separate risk (evacuating a
    /// bot before a build, or mining a way out), and neither can be reasoned
    /// about before a run record says when it happens.
    pub enclosures: SyncMutex<Enclosures>,
    /// Characters walked clear of a placement that would have sealed them in,
    /// since the last [`FactorioSurface::drain_step_asides`]. A queue, not
    /// knowledge -- see [`StepAside`].
    pub step_asides: SyncMutex<Vec<StepAside>>,
    /// Characters the game itself has said cannot move from where they
    /// stand, and which the next plan must not send anywhere -- see
    /// [`Bench`] for the run that needed it and why neither ledger above
    /// could have said so. Read by `crates/planner`'s `PlanState::from_world`,
    /// written and released by `crates/executor`.
    pub benches: SyncMutex<Benches>,
}

impl FactorioSurface {
    pub fn update_entity_prototypes(
        &self,
        entity_prototypes: Vec<FactorioEntityPrototype>,
    ) -> Result<()> {
        for entity_prototype in entity_prototypes {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype);
        }
        Ok(())
    }

    pub fn update_item_prototypes(
        &self,
        item_prototypes: Vec<FactorioItemPrototype>,
    ) -> Result<()> {
        for item_prototype in item_prototypes {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype);
        }
        Ok(())
    }

    pub fn remove_player(&self, player_id: PlayerId) -> Result<()> {
        self.players.remove(&player_id);
        Ok(())
    }

    pub fn player_changed_distance(&self, event: PlayerChangedDistanceEvent) -> Result<()> {
        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: existing_player.position.clone(),
                main_inventory: existing_player.main_inventory.clone(),
                build_distance: event.build_distance,
                reach_distance: event.reach_distance,
                drop_item_distance: event.drop_item_distance,
                item_pickup_distance: event.item_pickup_distance,
                loot_pickup_distance: event.loot_pickup_distance,
                resource_reach_distance: event.resource_reach_distance,
                // Carried, not re-derived: a distance-changed event says
                // nothing about where the bot is, so dropping the surface here
                // would let a partial update erase a fact the roster already
                // knew.
                surface: existing_player.surface.clone(),
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                build_distance: event.build_distance,
                reach_distance: event.reach_distance,
                drop_item_distance: event.drop_item_distance,
                item_pickup_distance: event.item_pickup_distance,
                loot_pickup_distance: event.loot_pickup_distance,
                resource_reach_distance: event.resource_reach_distance,
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    pub fn player_changed_position(&self, event: PlayerChangedPositionEvent) -> Result<()> {
        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: event.position,
                main_inventory: existing_player.main_inventory.clone(),
                build_distance: existing_player.build_distance,
                reach_distance: existing_player.reach_distance,
                drop_item_distance: existing_player.drop_item_distance,
                item_pickup_distance: existing_player.item_pickup_distance,
                loot_pickup_distance: existing_player.loot_pickup_distance,
                resource_reach_distance: existing_player.resource_reach_distance,
                // **A position event does not carry a surface**, so this keeps
                // the last one known rather than clearing it. That is the
                // honest choice and also the limitation: a bot that crossed to
                // another surface would report new coordinates under the old
                // surface name until something re-reads the roster. Nothing
                // can cross today; when something can, this line is one of the
                // places that has to learn about it.
                surface: existing_player.surface.clone(),
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                position: event.position,
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    pub fn update_force(&self, force: FactorioForce) -> Result<()> {
        let name = force.name.clone();
        self.forces.insert(name, force);
        Ok(())
    }

    /// Still a no-op, and **deliberately not the channel container contents
    /// arrive on**.
    ///
    /// The mod raises this from exactly one subscription --
    /// `script.on_event(defines.events.on_player_rotated_entity,
    /// on_some_entity_updated)` in `mods/BotBridge/control.lua` -- so it fires
    /// when a player turns an entity and at no other time. Its payload does
    /// carry `output_inventory` and `fuel_inventory` (`serialize_entity`
    /// includes both for every entity), which makes it look like the place to
    /// learn what a chest holds; it is not, because a chest nobody rotates
    /// never produces one.
    ///
    /// Buffer contents are pulled instead, into
    /// [`FactorioSurface::inventories`]. See that field for why there is no
    /// event to push them.
    ///
    /// The direction the original TODO names is still not applied. Doing so
    /// means replacing the stored entity, which
    /// [`FactorioSurface::observe_inventories`]'s own doc explains is not free
    /// on this `EntityGraph`; it is left as it was rather than half-done.
    pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> Result<()> {
        // TODO: update entity direction
        Ok(())
    }

    /// Records what the game just said each of these entities is holding.
    ///
    /// The argument is the reply shape of
    /// [`FactorioRcon::inventory_contents_at`](crate::factorio::rcon::FactorioRcon::inventory_contents_at)
    /// verbatim, so a caller hands the answer straight over without
    /// reshaping it -- and so nothing between the game and this map can
    /// disagree about what was asked for.
    ///
    /// **An entity the game did not answer for is not overwritten here.** The
    /// mod's `rcon_inventory_contents_at` skips a position where
    /// `surface.find_entity(name, position)` finds nothing, so a query for
    /// five entities can come back with three, and the two missing ones say
    /// "not found", not "empty". Erasing them would turn a failed *lookup*
    /// into an observation of emptiness -- exactly the confusion this
    /// codebase has paid for elsewhere. A caller that knows an entity is gone
    /// says so through [`FactorioSurface::forget_inventory`] or by deleting the
    /// entity.
    ///
    /// An entity that answers with *empty* inventories is recorded as empty,
    /// which is a real observation and different from silence.
    pub fn observe_inventories(&self, replies: Vec<InventoryResponse>) {
        for reply in replies {
            let observed = ObservedInventory {
                name: reply.name,
                position: reply.position,
                output: ObservedInventory::fold(&reply.output_inventory),
                fuel: ObservedInventory::fold(&reply.fuel_inventory),
            };
            self.inventories
                .insert(Pos::from(&observed.position), observed);
        }
    }

    /// Every inventory reading this world holds, ordered by tile.
    ///
    /// The only way to read [`FactorioSurface::inventories`] -- the map itself
    /// iterates in hash order, and `crates/planner` is required to be
    /// deterministic, so the sort belongs here where every reader gets it
    /// rather than in each reader.
    pub fn observed_inventories(&self) -> Vec<(Pos, ObservedInventory)> {
        let mut out: Vec<(Pos, ObservedInventory)> = self
            .inventories
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Drops the reading for whatever stood on this tile. Returns whether
    /// there was one.
    pub fn forget_inventory(&self, position: &Position) -> bool {
        self.inventories.remove(&Pos::from(position)).is_some()
    }

    pub fn on_some_entity_created(&self, entity: FactorioEntity) -> Result<()> {
        // Deliberately silent. This used to dump the whole `FactorioEntity` here
        // through `info!`, which in this crate is `paris` on stdout -- the
        // narration channel a person reads while a run happens, not a log. It
        // fired once per entity created and was a large share of a run's output.
        // The entity reaches `entity_graph` on the next line, so any question
        // the dump could answer is a graph query away.
        self.entity_graph.add(vec![entity], None)?;
        Ok(())
    }

    pub fn on_some_entity_deleted(&self, entity: FactorioEntity) -> Result<()> {
        // Whatever it was holding went with it. A mined furnace hands its
        // contents to whoever mined it, so a reading left standing here would
        // report items now in a player's pocket as still sitting on the
        // ground -- and the next thing built on this tile would inherit them.
        self.forget_inventory(&entity.position);
        self.entity_graph.remove(&entity)?;
        Ok(())
    }

    pub fn player_changed_main_inventory(
        &self,
        event: PlayerChangedMainInventoryEvent,
    ) -> Result<()> {
        // Convert Vec<InventoryItemWithQuality> to BTreeMap<String, u32>
        // (sum counts by item name, ignore quality for now)
        let main_inventory: BTreeMap<String, u32> =
            event
                .main_inventory
                .into_iter()
                .fold(BTreeMap::new(), |mut acc, item| {
                    *acc.entry(item.name).or_insert(0) += item.count;
                    acc
                });

        let player = if self.players.contains_key(&event.player_id) {
            let existing_player = self.players.get(&event.player_id).unwrap();
            FactorioPlayer {
                player_id: event.player_id,
                position: existing_player.position.clone(),
                main_inventory,
                build_distance: existing_player.build_distance,
                reach_distance: existing_player.reach_distance,
                drop_item_distance: existing_player.drop_item_distance,
                item_pickup_distance: existing_player.item_pickup_distance,
                loot_pickup_distance: existing_player.loot_pickup_distance,
                resource_reach_distance: existing_player.resource_reach_distance,
                // An inventory event says nothing about place; carried, as
                // above.
                surface: existing_player.surface.clone(),
            }
        } else {
            FactorioPlayer {
                player_id: event.player_id,
                main_inventory,
                ..Default::default()
            }
        };
        self.players.insert(event.player_id, player);
        Ok(())
    }

    /// Loads the static half of a world out of a [`WorldSnapshot`].
    ///
    /// The RCON equivalent of the four `entity_prototypes` / `item_prototypes` /
    /// `recipes` / `force` arms of [`crate::process::output_parser::OutputParser`],
    /// routed through the very same `update_*` methods so an attached world is
    /// built by the same code that builds an owned one.
    ///
    /// Additive, like every `update_*` here: calling it again on a live world
    /// refreshes what the snapshot covers and leaves players and the entity
    /// graph alone.
    pub fn apply_snapshot(&self, snapshot: WorldSnapshot) -> Result<()> {
        self.update_entity_prototypes(snapshot.entity_prototypes)?;
        self.update_item_prototypes(snapshot.item_prototypes)?;
        self.update_recipes(snapshot.recipes)?;
        for force in snapshot.forces {
            self.update_force(force)?;
        }
        Ok(())
    }

    pub fn update_recipes(&self, recipes: Vec<FactorioRecipe>) -> Result<()> {
        for recipe in recipes {
            self.recipes.insert(recipe.name.clone(), recipe);
        }
        Ok(())
    }

    pub fn update_graphics(&self, graphics: Vec<FactorioGraphic>) -> Result<()> {
        for graphic in graphics {
            self.graphics.insert(graphic.entity_name.clone(), graphic);
        }
        Ok(())
    }

    pub fn update_chunk_tiles(&self, tiles: Vec<FactorioTile>) -> Result<()> {
        self.entity_graph.add_tiles(tiles, None)?; // FIXME: add clear rect from chunk_position
        Ok(())
    }

    #[allow(clippy::map_clone)]
    pub fn update_chunk_entities(&self, entities: Vec<FactorioEntity>) -> Result<()> {
        self.entity_graph.add(entities, None)?; // FIXME: add clear rect
        Ok(())
    }

    pub fn import(&mut self, world: Arc<FactorioSurface>) -> Result<()> {
        for player in world.players.iter() {
            self.players.insert(player.player_id, player.clone());
        }
        for entity_prototype in world.entity_prototypes.iter() {
            self.entity_prototypes
                .insert(entity_prototype.name.clone(), entity_prototype.clone());
        }
        for item_prototype in world.item_prototypes.iter() {
            self.item_prototypes
                .insert(item_prototype.name.clone(), item_prototype.clone());
        }
        for recipe in world.recipes.iter() {
            self.recipes.insert(recipe.name.clone(), recipe.clone());
        }
        for force in world.forces.iter() {
            self.forces.insert(force.name.clone(), force.clone());
        }
        self.entity_graph.connect()?;
        Ok(())
    }

    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let forces: DashMap<String, FactorioForce> = DashMap::new();
        let players: DashMap<PlayerId, FactorioPlayer> = DashMap::new();
        let graphics: DashMap<String, FactorioGraphic> = DashMap::new();
        let image_cache: DashMap<String, Box<RgbaImage>> = DashMap::new();
        let item_prototypes: DashMap<String, FactorioItemPrototype> = DashMap::new();
        let recipes: Arc<DashMap<String, FactorioRecipe>> = Arc::new(DashMap::new());
        let entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>> =
            Arc::new(DashMap::new());
        let entity_graph = Arc::new(EntityGraph::new(entity_prototypes.clone(), recipes.clone()));
        let flow_graph = Arc::new(FlowGraph::new(entity_graph.clone()));
        FactorioSurface {
            image_cache,
            players,
            graphics,
            recipes,
            forces,
            entity_prototypes,
            item_prototypes,
            actions: DashMap::new(),
            path_requests: DashMap::new(),
            next_action_id: Mutex::new(1),
            entity_graph,
            flow_graph,
            teleports: SyncMutex::new(Vec::new()),
            deaths: SyncMutex::new(Vec::new()),
            research_triggers: SyncMutex::new(Vec::new()),
            surface_chunk_drops: SyncMutex::new(BTreeMap::new()),
            inventories: DashMap::new(),
            placement_refusals: SyncMutex::new(PlacementRefusals::default()),
            walk_refusals: SyncMutex::new(WalkRefusals::default()),
            enclosures: SyncMutex::new(Enclosures::default()),
            step_asides: SyncMutex::new(Vec::new()),
            benches: SyncMutex::new(Benches::default()),
        }
    }

    /// Queues a teleport `OutputParser` just parsed, for
    /// [`FactorioSurface::drain_teleports`] to pick up.
    pub fn record_teleport(&self, tick: u64, event: TeleportEvent) {
        self.teleports.lock().push((tick, event));
    }

    /// Takes every teleport queued since the last drain, oldest first.
    pub fn drain_teleports(&self) -> Vec<(u64, TeleportEvent)> {
        std::mem::take(&mut *self.teleports.lock())
    }

    /// Queues a death `OutputParser` just parsed, for
    /// [`FactorioSurface::drain_deaths`] to pick up.
    pub fn record_death(&self, tick: u64, event: DeathEvent) {
        self.deaths.lock().push((tick, BotLifeEvent::Died(event)));
    }

    /// Queues a respawn `OutputParser` just parsed, behind whatever death
    /// preceded it.
    pub fn record_respawn(&self, tick: u64, event: RespawnEvent) {
        self.deaths
            .lock()
            .push((tick, BotLifeEvent::Respawned(event)));
    }

    /// Takes every death and respawn queued since the last drain, oldest
    /// first.
    pub fn drain_deaths(&self) -> Vec<(u64, BotLifeEvent)> {
        std::mem::take(&mut *self.deaths.lock())
    }

    /// Queues a trigger technology the mod just completed, for
    /// [`FactorioSurface::drain_research_triggers`] to pick up.
    pub fn record_research_trigger(&self, tick: u64, event: ResearchTriggerEvent) {
        self.research_triggers.lock().push((tick, event));
    }

    /// Takes every emulated trigger queued since the last drain, oldest first.
    pub fn drain_research_triggers(&self) -> Vec<(u64, ResearchTriggerEvent)> {
        std::mem::take(&mut *self.research_triggers.lock())
    }

    /// Folds one refused chunk into the per-surface tally for
    /// [`FactorioSurface::drain_surface_chunk_drops`] to pick up.
    ///
    /// The first chunk seen for a surface in this window is the one kept, with
    /// its tick: it is the moment the surface first appeared, which is the
    /// interesting one. Later chunks only raise the count.
    pub fn record_surface_chunk_dropped(&self, tick: u64, event: SurfaceChunkDropEvent) {
        let mut drops = self.surface_chunk_drops.lock();
        drops
            .entry(event.surface)
            .and_modify(|seen| seen.chunks += 1)
            .or_insert(SurfaceChunkDrops {
                first_tick: tick,
                first_left_top: event.left_top,
                chunks: 1,
            });
    }

    /// Takes every surface's dropped-chunk tally since the last drain, in
    /// surface-name order.
    pub fn drain_surface_chunk_drops(&self) -> BTreeMap<SurfaceId, SurfaceChunkDrops> {
        std::mem::take(&mut *self.surface_chunk_drops.lock())
    }

    /// Remembers a build the game refused. Returns whether the site was new.
    ///
    /// Called from [`crate::factorio::rcon::FactorioRcon::place_entity_timed`],
    /// at the point the game's own answer is still a line of text rather than
    /// an error four wrappers deep — so the two refusals the mod distinguishes
    /// are told apart structurally there, not by re-parsing a message here.
    pub fn record_placement_refusal(&self, refusal: PlacementRefusal) -> bool {
        self.placement_refusals.lock().note(refusal)
    }

    /// Every refused site, oldest first. Non-destructive: this is the
    /// planner's read, and it happens once per plan.
    pub fn placement_refusals(&self) -> Vec<PlacementRefusal> {
        self.placement_refusals.lock().sites.clone()
    }

    /// Remembers a walk the pathfinder searched for and did not find. Returns
    /// whether the question was new.
    ///
    /// Called from `RconActuator::walk` (`crates/executor`), which is the one
    /// layer holding both halves of the fact: the destination the plan named,
    /// and where the character was actually standing when it asked. Neither
    /// the run loop above it nor the record below it has the second.
    pub fn record_walk_refusal(&self, refusal: WalkRefusal) -> bool {
        self.walk_refusals.lock().note(refusal)
    }

    /// Every refused walk, oldest first. Non-destructive: this is the
    /// planner's read, and it happens once per plan.
    pub fn walk_refusals(&self) -> Vec<WalkRefusal> {
        self.walk_refusals.lock().walks.clone()
    }

    /// Remembers a character found unable to reach open ground. Returns
    /// whether the condition was new -- see [`Enclosures::note`] for what
    /// "new" means, which is not "a different float".
    ///
    /// Called from `walk_memory` (`crates/executor`), off the back of a walk
    /// the game's own pathfinder refused. That trigger is deliberate: the
    /// check costs one quad-tree query and a fixed-size fill, which is cheap
    /// but not free, and a refused walk is the moment we already know
    /// something is wrong.
    pub fn record_enclosure(&self, found: Enclosure) -> bool {
        self.enclosures.lock().note(found)
    }

    /// Queues a step-aside the executor just made, for
    /// [`FactorioSurface::drain_step_asides`] to pick up.
    pub fn record_step_aside(&self, step: StepAside) {
        self.step_asides.lock().push(step);
    }

    /// Takes every step-aside queued since the last drain, oldest first.
    pub fn drain_step_asides(&self) -> Vec<StepAside> {
        std::mem::take(&mut *self.step_asides.lock())
    }

    /// Every enclosure observed, oldest first. Non-destructive.
    pub fn enclosures(&self) -> Vec<Enclosure> {
        self.enclosures.lock().found.clone()
    }

    /// Benches a character the game has refused every short hop from where
    /// it stands. Returns whether the ledger changed -- `false` when the
    /// player was already benched at (about) this spot.
    ///
    /// Called from `walk_memory` (`crates/executor`) with the verdict of a
    /// mobility probe, never from a fill over this crate's own model: see
    /// [`Bench`] for why the model's answer is not admissible here.
    pub fn record_bench(&self, bench: Bench) -> bool {
        self.benches.lock().note(bench)
    }

    /// Lifts a player's bench because the game has shown it can move --
    /// see [`BenchRelease`]. Returns whether it was benched.
    pub fn release_bench(&self, player: PlayerId, tick: Option<u64>, why: BenchRelease) -> bool {
        self.benches.lock().release(player, tick, why)
    }

    /// Every character currently benched, in player order. Non-destructive:
    /// this is the planner's read, once per plan.
    pub fn benches(&self) -> Vec<Bench> {
        self.benches.lock().active.values().cloned().collect()
    }

    /// Whether this player is benched right now.
    pub fn is_benched(&self, player: PlayerId) -> bool {
        self.benches.lock().active.contains_key(&player)
    }

    /// Takes every bench change queued since the last drain, oldest first,
    /// for the record.
    pub fn drain_bench_changes(&self) -> Vec<BenchChange> {
        std::mem::take(&mut self.benches.lock().changes)
    }

    /// The enclosures no record has been told about yet, oldest first, marking
    /// them reported. The rows themselves stay -- see [`Enclosures`].
    pub fn unreported_enclosures(&self) -> Vec<Enclosure> {
        let mut ledger = self.enclosures.lock();
        let from = ledger.reported;
        ledger.reported = ledger.found.len();
        ledger.found[from..].to_vec()
    }

    /// The refusals no record has been told about yet, oldest first, marking
    /// them reported. The sites themselves stay — see [`PlacementRefusals`].
    pub fn unreported_placement_refusals(&self) -> Vec<PlacementRefusal> {
        let mut ledger = self.placement_refusals.lock();
        let from = ledger.reported;
        ledger.reported = ledger.sites.len();
        ledger.sites[from..].to_vec()
    }

    pub fn dump(&self, save_path: Option<&str>) -> Result<()> {
        match save_path {
            Some(save_path) => self.dump_to(std::path::Path::new(save_path)),
            None => {
                println!("{}", serde_json::to_string_pretty(self).into_diagnostic()?);
                Ok(())
            }
        }
    }

    /// Writes this world to `path` as JSON.
    ///
    /// Takes a `&Path` rather than a `&str` because the callers that matter
    /// have already resolved one: `world.dump` in `crates/scripting_lua` runs
    /// its argument through `resolve_script_path`'s write-side sibling before
    /// it gets here, and a `to_str()` on the way back out would turn a path
    /// this crate cannot render into a *dump printed to stdout* -- silently
    /// writing no file while reporting success.
    ///
    /// What comes out is the input to offline planning: `serde_json::from_str`
    /// into a `FactorioWorld`, then `PlanState::from_world`. Everything that
    /// function reads round-trips; see
    /// `crates/planner/tests/world_round_trip.rs`.
    ///
    /// # The file is not byte-stable across processes, and the plan is
    ///
    /// The maps whose key order the planner depends on are written in sorted
    /// order -- `inventories` here, `resources` and `minables` in
    /// [`crate::graph::entity_graph::EntityGraph`] -- so what the planner
    /// reads is fixed. The remaining `DashMap` fields (`players`, `forces`,
    /// `graphics`, `item_prototypes`, `actions`, `path_requests`,
    /// `entity_prototypes`, `recipes`) are still written in hash order, which
    /// is stable within one process and not across two. So two dumps of the
    /// same world from two processes may differ *as bytes* while loading to
    /// the same world and producing the identical plan -- do not checksum a
    /// dump to decide whether two runs saw the same map.
    /// [`EntityGraph::resource_fingerprint`] is the thing that answers that.
    pub fn dump_to(&self, path: &std::path::Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self).into_diagnostic()?;
        fs::write(path, &content).into_diagnostic()?;
        Ok(())
    }

    pub fn dump_entitiy_prototypes(&self, save_path: Option<&str>) -> Result<()> {
        let content = serde_json::to_string_pretty(&*self.entity_prototypes).into_diagnostic()?;
        if let Some(save_path) = save_path {
            fs::write(save_path, &content).into_diagnostic()?;
        } else {
            println!("{content}");
        }

        Ok(())
    }

    pub fn dump_item_prototypes(&self, save_path: Option<&str>) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.item_prototypes).into_diagnostic()?;
        if let Some(save_path) = save_path {
            fs::write(save_path, &content).into_diagnostic()?;
        } else {
            println!("{content}");
        }

        Ok(())
    }

    pub fn dump_recipes(&self, save_path: Option<&str>) -> Result<()> {
        let content = serde_json::to_string_pretty(&*self.recipes).into_diagnostic()?;
        if let Some(save_path) = save_path {
            fs::write(save_path, &content).into_diagnostic()?;
        } else {
            println!("{content}");
        }

        Ok(())
    }
}

unsafe impl Send for FactorioSurface {}
unsafe impl Sync for FactorioSurface {}

impl Serialize for FactorioSurface {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FactorioSurface", 14)?;
        state.serialize_field("players", &self.players)?;
        state.serialize_field("forces", &self.forces)?;
        state.serialize_field("graphics", &self.graphics)?;
        state.serialize_field("recipes", &*self.recipes)?;
        state.serialize_field("entity_prototypes", &*self.entity_prototypes)?;
        state.serialize_field("item_prototypes", &self.item_prototypes)?;
        state.serialize_field("actions", &self.actions)?;
        state.serialize_field("path_requests", &self.path_requests)?;
        state.serialize_field("entity_graph", &*self.entity_graph)?;
        // The four ledgers `PlanState::from_world` reads and the derived
        // fields above do not carry. Without them a mid-run dump plans
        // against a world that has forgotten every chest it looked inside,
        // every site the game refused a build at, every destination the
        // pathfinder refused a route to, and every bot found boxed in -- and
        // it forgets them *silently*, which is the failure mode this file
        // has paid for before.
        //
        // `observed_inventories()` rather than the `DashMap` itself, and not
        // only for determinism (a `DashMap` iterates in hash order, which
        // moves with the seed): `Pos` is a tuple struct and cannot be a JSON
        // object key at all, so the sorted pair list is the only shape that
        // works. See the `inventories` field's own doc.
        state.serialize_field("inventories", &self.observed_inventories())?;
        state.serialize_field("placement_refusals", &*self.placement_refusals.lock())?;
        state.serialize_field("walk_refusals", &*self.walk_refusals.lock())?;
        state.serialize_field("enclosures", &*self.enclosures.lock())?;
        // The fifth ledger, the one the game wrote: a dump taken with a bot
        // benched must plan with it benched, or the offline plan re-sends
        // exactly the walk the live run halted on.
        state.serialize_field("benches", &*self.benches.lock())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for FactorioSurface {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Players,
            Forces,
            Graphics,
            Recipes,
            EntityPrototypes,
            ItemPrototypes,
            Actions,
            PathRequests,
            EntityGraph,
            Inventories,
            PlacementRefusals,
            WalkRefusals,
            Enclosures,
            Benches,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`secs` or `nanos`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "players" => Ok(Field::Players),
                            "forces" => Ok(Field::Forces),
                            "graphics" => Ok(Field::Graphics),
                            "recipes" => Ok(Field::Recipes),
                            "entity_prototypes" => Ok(Field::EntityPrototypes),
                            "item_prototypes" => Ok(Field::ItemPrototypes),
                            "actions" => Ok(Field::Actions),
                            "path_requests" => Ok(Field::PathRequests),
                            "entity_graph" => Ok(Field::EntityGraph),
                            "inventories" => Ok(Field::Inventories),
                            "placement_refusals" => Ok(Field::PlacementRefusals),
                            "walk_refusals" => Ok(Field::WalkRefusals),
                            "enclosures" => Ok(Field::Enclosures),
                            "benches" => Ok(Field::Benches),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct FactorioSurfaceVisitor;

        impl<'de> Visitor<'de> for FactorioSurfaceVisitor {
            type Value = FactorioSurface;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FactorioSurface")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut players = None;
                let mut forces = None;
                let mut graphics = None;
                let mut recipes = None;
                let mut entity_prototypes = None;
                let mut item_prototypes = None;
                let mut actions = None;
                let mut path_requests = None;
                let mut entity_graph = None;
                let mut inventories: Option<Vec<(Pos, ObservedInventory)>> = None;
                let mut placement_refusals: Option<PlacementRefusals> = None;
                let mut walk_refusals: Option<WalkRefusals> = None;
                let mut enclosures: Option<Enclosures> = None;
                let mut benches: Option<Benches> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Players => {
                            if players.is_some() {
                                return Err(de::Error::duplicate_field("players"));
                            }
                            players = Some(map.next_value()?);
                        }
                        Field::Forces => {
                            if forces.is_some() {
                                return Err(de::Error::duplicate_field("forces"));
                            }
                            forces = Some(map.next_value()?);
                        }
                        Field::Graphics => {
                            if graphics.is_some() {
                                return Err(de::Error::duplicate_field("graphics"));
                            }
                            graphics = Some(map.next_value()?);
                        }
                        Field::Recipes => {
                            if recipes.is_some() {
                                return Err(de::Error::duplicate_field("recipes"));
                            }
                            recipes = Some(map.next_value()?);
                        }
                        Field::EntityPrototypes => {
                            if entity_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("entity_prototypes"));
                            }
                            entity_prototypes = Some(map.next_value()?);
                        }
                        Field::ItemPrototypes => {
                            if item_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("item_prototypes"));
                            }
                            item_prototypes = Some(map.next_value()?);
                        }
                        Field::Actions => {
                            if actions.is_some() {
                                return Err(de::Error::duplicate_field("actions"));
                            }
                            actions = Some(map.next_value()?);
                        }
                        Field::PathRequests => {
                            if path_requests.is_some() {
                                return Err(de::Error::duplicate_field("path_requests"));
                            }
                            path_requests = Some(map.next_value()?);
                        }
                        Field::EntityGraph => {
                            if entity_graph.is_some() {
                                return Err(de::Error::duplicate_field("entity_graph"));
                            }
                            entity_graph = Some(map.next_value()?);
                        }
                        Field::Inventories => {
                            if inventories.is_some() {
                                return Err(de::Error::duplicate_field("inventories"));
                            }
                            inventories = Some(map.next_value()?);
                        }
                        Field::PlacementRefusals => {
                            if placement_refusals.is_some() {
                                return Err(de::Error::duplicate_field("placement_refusals"));
                            }
                            placement_refusals = Some(map.next_value()?);
                        }
                        Field::WalkRefusals => {
                            if walk_refusals.is_some() {
                                return Err(de::Error::duplicate_field("walk_refusals"));
                            }
                            walk_refusals = Some(map.next_value()?);
                        }
                        Field::Enclosures => {
                            if enclosures.is_some() {
                                return Err(de::Error::duplicate_field("enclosures"));
                            }
                            enclosures = Some(map.next_value()?);
                        }
                        Field::Benches => {
                            if benches.is_some() {
                                return Err(de::Error::duplicate_field("benches"));
                            }
                            benches = Some(map.next_value()?);
                        }
                    }
                }
                let players = players.ok_or_else(|| de::Error::missing_field("players"))?;
                let forces = forces.ok_or_else(|| de::Error::missing_field("forces"))?;
                let graphics = graphics.ok_or_else(|| de::Error::missing_field("graphics"))?;
                let recipes = recipes.ok_or_else(|| de::Error::missing_field("recipes"))?;
                let entity_prototypes = entity_prototypes
                    .ok_or_else(|| de::Error::missing_field("entity_prototypes"))?;
                let item_prototypes =
                    item_prototypes.ok_or_else(|| de::Error::missing_field("item_prototypes"))?;
                let actions = actions.ok_or_else(|| de::Error::missing_field("actions"))?;
                let path_requests =
                    path_requests.ok_or_else(|| de::Error::missing_field("path_requests"))?;
                let entity_graph =
                    entity_graph.ok_or_else(|| de::Error::missing_field("entity_graph"))?;
                // The four ledgers are **optional**, unlike everything above.
                // They were added long after the first dumps were written, and
                // absent is exactly what an older file means by "empty" -- a
                // world with no observed inventories and no refusals is a
                // perfectly ordinary t=0 world. Requiring them would make
                // every dump written before this change unreadable in exchange
                // for nothing.
                let inventories: DashMap<Pos, ObservedInventory> =
                    inventories.unwrap_or_default().into_iter().collect();
                let placement_refusals = placement_refusals.unwrap_or_default();
                let walk_refusals = walk_refusals.unwrap_or_default();
                let enclosures = enclosures.unwrap_or_default();
                let benches = benches.unwrap_or_default();

                let entity_graph: Arc<EntityGraph> = Arc::new(entity_graph);
                let flow_graph = Arc::new(FlowGraph::new(entity_graph.clone()));
                Ok(FactorioSurface {
                    players,
                    forces,
                    graphics,
                    recipes: Arc::new(recipes),
                    entity_prototypes: Arc::new(entity_prototypes),
                    item_prototypes,
                    image_cache: Default::default(),
                    actions,
                    path_requests,
                    next_action_id: Default::default(),
                    entity_graph,
                    flow_graph,
                    teleports: Default::default(),
                    deaths: Default::default(),
                    research_triggers: Default::default(),
                    surface_chunk_drops: Default::default(),
                    inventories,
                    placement_refusals: SyncMutex::new(placement_refusals),
                    walk_refusals: SyncMutex::new(walk_refusals),
                    enclosures: SyncMutex::new(enclosures),
                    step_asides: Default::default(),
                    benches: SyncMutex::new(benches),
                })
            }
        }

        const FIELDS: &[&str] = &[
            "players",
            "forces",
            "graphics",
            "recipes",
            "entity_prototypes",
            "item_prototypes",
            "actions",
            "path_requests",
            "entity_graph",
            "inventories",
            "placement_refusals",
            "walk_refusals",
            "enclosures",
            "benches",
        ];
        deserializer.deserialize_struct("FactorioSurface", FIELDS, FactorioSurfaceVisitor)
    }
}

impl Clone for FactorioSurface {
    fn clone(&self) -> Self {
        let entity_prototypes = Arc::new((*self.entity_prototypes).clone());
        let recipes = Arc::new((*self.recipes).clone());
        let entity_graph = Arc::new((*self.entity_graph).clone());
        let _entity_graph = entity_graph.clone();
        FactorioSurface {
            entity_graph,
            recipes,
            entity_prototypes,
            players: self.players.clone(),
            forces: self.forces.clone(),
            graphics: self.graphics.clone(),
            item_prototypes: self.item_prototypes.clone(),
            image_cache: self.image_cache.clone(),
            actions: self.actions.clone(),
            path_requests: self.path_requests.clone(),
            next_action_id: Mutex::new(0),
            // Ephemeral, like `next_action_id` above: a clone starts with an
            // empty queue rather than duplicating in-flight teleports across
            // two independent recorders.
            teleports: SyncMutex::new(Vec::new()),
            deaths: SyncMutex::new(Vec::new()),
            research_triggers: SyncMutex::new(Vec::new()),
            surface_chunk_drops: SyncMutex::new(BTreeMap::new()),
            // Knowledge, like `placement_refusals` below and for the same
            // reason: what a chest was last seen holding does not stop being
            // our best reading because the world was cloned. Stale in exactly
            // the same way and to exactly the same degree as the original.
            inventories: self.inventories.clone(),
            // Knowledge, not a queue -- and knowledge about the *game*, which
            // a clone of our belief about it does not stop being true of. A
            // clone that started blank would hand the planner back exactly
            // the sites it has already been refused.
            placement_refusals: SyncMutex::new(PlacementRefusals {
                sites: self.placement_refusals.lock().sites.clone(),
                // Reset: a second recorder has been told nothing, and writing
                // a site twice into two different records is the honest
                // answer for two independent records.
                reported: 0,
            }),
            // Knowledge too, for the same reason, and with no cursor to
            // reset: nothing reports these into a record.
            walk_refusals: SyncMutex::new(WalkRefusals {
                walks: self.walk_refusals.lock().walks.clone(),
            }),
            // Knowledge, and with a cursor that resets like the placement
            // ledger's above: a second recorder has been told about none of
            // these, and a condition worth naming once is worth naming once
            // in each record that could otherwise not explain a frozen bot.
            enclosures: SyncMutex::new(Enclosures {
                found: self.enclosures.lock().found.clone(),
                reported: 0,
            }),
            // Ephemeral, like `teleports` above: an event, not knowledge.
            step_asides: SyncMutex::new(Vec::new()),
            // Knowledge -- the game's own verdict on who can move -- with its
            // change queue reset like the cursors above: a second recorder
            // has been told about none of it.
            benches: SyncMutex::new(Benches {
                active: self.benches.lock().active.clone(),
                changes: Vec::new(),
            }),
            flow_graph: Arc::new(FlowGraph::new(_entity_graph)),
        }
    }

    fn clone_from(&mut self, _source: &Self) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::redundant_clone)]
    fn test_tile_boundaries_0() {
        let world = FactorioSurface {
            players: Default::default(),
            forces: Default::default(),
            graphics: Default::default(),
            recipes: Arc::new(Default::default()),
            entity_prototypes: Arc::new(Default::default()),
            item_prototypes: Default::default(),
            image_cache: Default::default(),
            actions: Default::default(),
            path_requests: Default::default(),
            next_action_id: Default::default(),
            teleports: Default::default(),
            deaths: Default::default(),
            research_triggers: Default::default(),
            surface_chunk_drops: Default::default(),
            inventories: Default::default(),
            placement_refusals: Default::default(),
            walk_refusals: Default::default(),
            enclosures: Default::default(),
            step_asides: Default::default(),
            benches: Default::default(),
            entity_graph: Arc::new(EntityGraph::new(
                Arc::new(DashMap::new()),
                Arc::new(DashMap::new()),
            )),
            flow_graph: Arc::new(FlowGraph::new(Arc::new(EntityGraph::new(
                Arc::new(DashMap::new()),
                Arc::new(DashMap::new()),
            )))),
        };

        let _cloned = world.clone();
    }

    fn reply(name: &str, position: Position, output: &[(&str, u32)]) -> InventoryResponse {
        InventoryResponse {
            name: name.into(),
            position,
            output_inventory: Box::new(Some(
                output
                    .iter()
                    .map(|(item, count)| crate::types::InventoryItemWithQuality {
                        name: (*item).into(),
                        quality: "normal".into(),
                        count: *count,
                    })
                    .collect(),
            )),
            fuel_inventory: Box::new(None),
        }
    }

    #[test]
    fn an_observed_inventory_sums_quality_away_and_reads_back_by_tile() {
        let world = FactorioSurface::new();
        world.observe_inventories(vec![InventoryResponse {
            name: "stone-furnace".into(),
            position: Position::new(-40.5, 12.5),
            output_inventory: Box::new(Some(vec![
                crate::types::InventoryItemWithQuality {
                    name: "iron-plate".into(),
                    quality: "normal".into(),
                    count: 4,
                },
                // The same item at another quality. Nothing here reasons about
                // quality, so four and three plates are seven plates.
                crate::types::InventoryItemWithQuality {
                    name: "iron-plate".into(),
                    quality: "uncommon".into(),
                    count: 3,
                },
            ])),
            fuel_inventory: Box::new(None),
        }]);

        let observed = world.observed_inventories();
        assert_eq!(observed.len(), 1);
        // `Pos` floors, and the reading keeps the unrounded centre alongside
        // it -- a resource-position bug in miniature, and the reason
        // `ObservedInventory::position` exists at all.
        assert_eq!(observed[0].0, Pos(-41, 12));
        assert_eq!(observed[0].1.position, Position::new(-40.5, 12.5));
        assert_eq!(observed[0].1.output.get("iron-plate").copied(), Some(7));
    }

    #[test]
    fn observed_inventories_come_back_in_tile_order() {
        let world = FactorioSurface::new();
        // Inserted in an order that is neither sorted nor reverse-sorted, so a
        // `DashMap` iteration that happened to be right once cannot pass this.
        for (x, y) in [(5., 5.), (-3., 9.), (5., -1.), (-3., -1.)] {
            world.observe_inventories(vec![reply(
                "stone-furnace",
                Position::new(x, y),
                &[("iron-plate", 1)],
            )]);
        }
        let keys: Vec<Pos> = world
            .observed_inventories()
            .into_iter()
            .map(|(pos, _)| pos)
            .collect();
        assert_eq!(keys, vec![Pos(-3, -1), Pos(-3, 9), Pos(5, -1), Pos(5, 5)]);
        for _ in 0..20 {
            let again: Vec<Pos> = world
                .observed_inventories()
                .into_iter()
                .map(|(pos, _)| pos)
                .collect();
            assert_eq!(keys, again, "the order is the tile order, every time");
        }
    }

    #[test]
    fn a_second_reading_replaces_the_first() {
        let world = FactorioSurface::new();
        let at = Position::new(3., 4.);
        world.observe_inventories(vec![reply(
            "stone-furnace",
            at.clone(),
            &[("iron-plate", 9)],
        )]);
        world.observe_inventories(vec![reply(
            "stone-furnace",
            at.clone(),
            &[("iron-plate", 2)],
        )]);
        assert_eq!(
            world.observed_inventories()[0]
                .1
                .output
                .get("iron-plate")
                .copied(),
            Some(2),
            "a reading is what the game last said, not a running total"
        );
    }

    #[test]
    fn an_entity_that_answered_empty_is_recorded_as_empty() {
        let world = FactorioSurface::new();
        world.observe_inventories(vec![InventoryResponse {
            name: "stone-furnace".into(),
            position: Position::new(0., 0.),
            output_inventory: Box::new(Some(vec![])),
            fuel_inventory: Box::new(None),
        }]);
        let observed = world.observed_inventories();
        assert_eq!(observed.len(), 1, "an empty answer is still an answer");
        assert!(observed[0].1.is_empty());
    }

    /// Deleting the entity forgets what was in it.
    ///
    /// A mined furnace hands its contents to whoever mined it. A reading left
    /// standing would report items now in a player's pocket as still sitting
    /// on the ground -- and the next thing built on that tile would inherit
    /// them.
    #[test]
    fn deleting_an_entity_forgets_its_contents() {
        let world = FactorioSurface::new();
        let at = Position::new(6., 6.);
        let furnace = FactorioEntity::new_stone_furnace(&at, crate::types::Direction::North);
        world
            .on_some_entity_created(furnace.clone())
            .expect("a furnace");
        world.observe_inventories(vec![reply(
            "stone-furnace",
            at.clone(),
            &[("iron-plate", 5)],
        )]);
        assert_eq!(world.observed_inventories().len(), 1);

        world.on_some_entity_deleted(furnace).expect("mined away");
        assert!(
            world.observed_inventories().is_empty(),
            "the contents went with the entity"
        );
    }

    /// A clone carries the readings, like `placement_refusals` and unlike
    /// `teleports`.
    #[test]
    fn a_clone_keeps_what_it_last_saw_in_a_buffer() {
        let world = FactorioSurface::new();
        world.observe_inventories(vec![reply(
            "stone-furnace",
            Position::new(1., 1.),
            &[("copper-plate", 3)],
        )]);
        let cloned = world.clone();
        assert_eq!(
            cloned.observed_inventories()[0]
                .1
                .output
                .get("copper-plate")
                .copied(),
            Some(3),
            "a reading is knowledge about the game, and cloning our belief \
             about the game does not make it untrue"
        );
    }

    // ---- walk refusals ----
    //
    // The positions are run `run-1788432181-42528`'s own: bot 3 frozen at
    // `(-56.26, 14.75)` from tick 53 700 to the end of the run, sent at the
    // ore tile `(-54.5, -12.5)` on five separate plans and refused before
    // dispatch every time.

    const FROZEN: (f64, f64) = (-56.2578125, 14.74609375);
    const ORE_TILE: (f64, f64) = (-54.5, -12.5);

    fn walk_refusal(player: PlayerId, from: (f64, f64), to: (f64, f64)) -> WalkRefusal {
        WalkRefusal {
            tick: None,
            player,
            from: Position::new(from.0, from.1),
            to: Position::new(to.0, to.1),
        }
    }

    #[test]
    fn a_refused_walk_is_remembered_for_the_bot_that_asked() {
        let world = FactorioSurface::new();
        assert!(world.record_walk_refusal(walk_refusal(3, FROZEN, ORE_TILE)));
        let known = world.walk_refusals();
        assert_eq!(known.len(), 1);
        assert!(
            known[0].applies_to(
                3,
                &Position::new(FROZEN.0, FROZEN.1),
                &Position::new(ORE_TILE.0, ORE_TILE.1)
            ),
            "the same bot asking the same question from the same spot has \
             already been answered"
        );
    }

    /// The nuance the whole design turns on: this is a fact about a pair of
    /// points, so neither half may be dropped from it.
    #[test]
    fn it_says_nothing_about_another_bot_or_another_place() {
        let refusal = walk_refusal(3, FROZEN, ORE_TILE);
        let ore = Position::new(ORE_TILE.0, ORE_TILE.1);
        assert!(
            !refusal.applies_to(1, &Position::new(FROZEN.0, FROZEN.1), &ore),
            "bot 1 stood somewhere else all run; answering it with bot 3's \
             evidence asserts something nobody established"
        );
        assert!(
            !refusal.applies_to(3, &Position::new(-20., -30.), &ore),
            "`failed to path find` means unreachable from *here*, not forever"
        );
        assert!(
            !refusal.applies_to(
                3,
                &Position::new(FROZEN.0, FROZEN.1),
                &Position::new(-46.5, -9.5)
            ),
            "a different destination is a different question"
        );
    }

    /// A bot that has not really moved is still standing where the refusal was
    /// earned. Sub-tile drift must not lose the memory -- that is the failure
    /// mode this ledger exists to prevent, wearing a rounding error.
    #[test]
    fn a_bot_that_shuffled_half_a_tile_is_still_where_it_was() {
        let refusal = walk_refusal(3, FROZEN, ORE_TILE);
        let ore = Position::new(ORE_TILE.0, ORE_TILE.1);
        assert!(
            refusal.applies_to(3, &Position::new(FROZEN.0 + 0.4, FROZEN.1 - 0.3), &ore),
            "the snapshot a plan reasons with is not the instant the walk was \
             dispatched; a tile of slack is what makes the memory usable"
        );
        assert!(
            !refusal.applies_to(3, &Position::new(FROZEN.0 + 4., FROZEN.1), &ore),
            "four tiles away is a different question, and asking it again is \
             cheaper than being wrong about it"
        );
    }

    #[test]
    fn the_same_question_refused_twice_is_remembered_once() {
        let world = FactorioSurface::new();
        assert!(world.record_walk_refusal(walk_refusal(3, FROZEN, ORE_TILE)));
        assert!(
            !world.record_walk_refusal(walk_refusal(3, FROZEN, ORE_TILE)),
            "the second refusal taught nothing new"
        );
        assert!(
            world.record_walk_refusal(walk_refusal(2, FROZEN, ORE_TILE)),
            "a second bot refused the same destination is a second fact"
        );
        assert_eq!(world.walk_refusals().len(), 2);
    }

    /// Cloned like `placement_refusals`, and for the same reason.
    #[test]
    fn a_clone_keeps_the_walks_the_game_refused() {
        let world = FactorioSurface::new();
        world.record_walk_refusal(walk_refusal(3, FROZEN, ORE_TILE));
        assert_eq!(
            world.clone().walk_refusals().len(),
            1,
            "a clone that started blank would hand the planner back exactly \
             the destinations it has already been refused"
        );
    }
    /// The four ledgers `PlanState::from_world` reads survive a dump.
    ///
    /// They were the whole gap. `players`, `forces`, `recipes` and the entity
    /// graph have round-tripped since the hand-written impls were written;
    /// these four did not, so a mid-run dump reloaded into an offline planner
    /// silently forgot every chest it had looked inside and every site the
    /// game had refused -- the exact state that makes replanning from a
    /// milestone worth doing. `crates/planner/tests/world_round_trip.rs` pins
    /// the consequence (the plan is identical); this pins the mechanism.
    #[test]
    fn the_four_planner_ledgers_survive_a_json_round_trip() {
        let world = FactorioSurface::new();
        world.observe_inventories(vec![
            reply(
                "stone-furnace",
                Position::new(5., -1.),
                &[("iron-plate", 7)],
            ),
            reply("wooden-chest", Position::new(-3., 9.), &[("coal", 12)]),
        ]);
        world.record_placement_refusal(PlacementRefusal::at_dispatch(
            Some(4242),
            "stone-furnace",
            Position::new(-40.5, 39.5),
            0,
            Vec::new(),
            None,
        ));
        world.record_placement_refusal(PlacementRefusal {
            tick: None,
            entity: "burner-mining-drill".into(),
            position: Position::new(1.5, 2.5),
            direction: Some(4),
            source: RefusalSource::PreCheck,
            blockers: vec!["tree-01".into()],
            tile: Some("grass-1".into()),
        });
        world.record_walk_refusal(walk_refusal(3, FROZEN, ORE_TILE));
        world.record_enclosure(Enclosure {
            tick: None,
            player: 3,
            at: Position::new(FROZEN.0, FROZEN.1),
            pocket_tiles: 12.5,
            searched_tiles: 48.,
        });

        let json = serde_json::to_string(&world).expect("a world serialises");
        let back: FactorioSurface = serde_json::from_str(&json).expect("and comes back");

        assert_eq!(back.observed_inventories(), world.observed_inventories());
        assert_eq!(back.placement_refusals(), world.placement_refusals());
        assert_eq!(back.walk_refusals(), world.walk_refusals());
        assert_eq!(back.enclosures(), world.enclosures());
    }

    /// A dump written before the ledgers were serialized still loads.
    ///
    /// Absent is what an older file means by "empty", and that is a real
    /// world: at t=0 on a fresh map all four ledgers *are* empty. Making them
    /// required would have made every existing dump unreadable to buy
    /// nothing.
    #[test]
    fn a_dump_written_without_the_ledgers_still_loads() {
        let world = FactorioSurface::new();
        let mut value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&world).expect("serialises"))
                .expect("is an object");
        let object = value.as_object_mut().expect("a struct is a map");
        for gone in [
            "inventories",
            "placement_refusals",
            "walk_refusals",
            "enclosures",
        ] {
            assert!(object.remove(gone).is_some(), "{gone} was written");
        }
        let back: FactorioSurface =
            serde_json::from_value(value).expect("an older dump is still readable");
        assert!(back.observed_inventories().is_empty());
        assert!(back.placement_refusals().is_empty());
        assert!(back.walk_refusals().is_empty());
        assert!(back.enclosures().is_empty());
    }

    /// The dump's inventory list is in tile order, not hash order.
    ///
    /// `observed_inventories` exists because a `DashMap` iterates in hash
    /// order, which moves with the hash seed; a dump that wrote the raw map
    /// would produce a different file on every process for the same world,
    /// and the offline planner's whole claim is that identical inputs give
    /// identical output.
    #[test]
    fn a_dump_lists_inventories_in_tile_order() {
        let world = FactorioSurface::new();
        for (x, y) in [(5., 5.), (-3., 9.), (5., -1.), (-3., -1.)] {
            world.observe_inventories(vec![reply(
                "stone-furnace",
                Position::new(x, y),
                &[("iron-plate", 1)],
            )]);
        }
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&world).expect("serialises"))
                .expect("is an object");
        let keys: Vec<Pos> = value["inventories"]
            .as_array()
            .expect("a list of pairs")
            .iter()
            .map(|pair| serde_json::from_value(pair[0].clone()).expect("a Pos"))
            .collect();
        assert_eq!(keys, vec![Pos(-3, -1), Pos(-3, 9), Pos(5, -1), Pos(5, 5)]);
    }
}
