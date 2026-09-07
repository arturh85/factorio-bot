//! The half of the world model that is **not** about a place.
//!
//! See [`FactorioWorld`](crate::factorio::world::FactorioWorld)'s doc for the
//! field-by-field argument. In one line: a coordinate is only comparable
//! within a surface, so everything a `Pos` keys belongs to a
//! [`FactorioSurface`](crate::factorio::world::FactorioSurface) -- and
//! everything that is a fact about the *game* or about a *force* must exist
//! exactly once no matter how many surfaces the run has seen, because two
//! copies of the research state is a world that disagrees with itself about
//! what is unlocked.

use crate::factorio::ticks::ActionOutcome;
use crate::factorio::world::{
    Benches, BotLifeEvent, ResearchTriggerEvent, SurfaceChunkDrops, TeleportEvent,
};
use crate::types::{
    FactorioEntityPrototype, FactorioForce, FactorioGraphic, FactorioItemPrototype, FactorioPlayer,
    FactorioRecipe, FactorioSurfaceInfo, PlayerId, SurfaceId,
};
use dashmap::DashMap;
use image::RgbaImage;
use parking_lot::Mutex as SyncMutex;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Everything one running game knows that is not about one surface.
///
/// # Why this is a type and not a set of fields on the surface
///
/// It used to be the latter, and
/// [`FactorioWorld::insert_surface`](crate::factorio::world::FactorioWorld::insert_surface)
/// refused a second surface by name rather than accept one and quietly hand
/// the run two research states. Lifting that refusal meant giving the globals
/// somewhere to live that a surface does not own. Every
/// [`FactorioSurface`](crate::factorio::world::FactorioSurface) in one
/// [`FactorioWorld`](crate::factorio::world::FactorioWorld) holds the **same**
/// `Arc<GameGlobals>` -- shared by construction, not by care -- and
/// `insert_surface` checks exactly that with `Arc::ptr_eq` before accepting a
/// surface.
///
/// # A clone is a fork, not a share
///
/// [`Clone`] deep-copies, because the one caller that clones a surface is
/// `Planner`'s plan world: a speculative fork whose writes must not reach the
/// live model. Sharing an `Arc` there would have let a plan's imagined
/// research escape into the world the executor reads. The ephemeral queues
/// (`teleports`, `deaths`, `research_triggers`, `surface_chunk_drops`) start
/// empty in a clone and `next_action_id` resets, exactly as they did when
/// these fields lived on the surface.
///
/// # `players` and `benches` are here under protest
///
/// They are keyed by [`PlayerId`], so they cannot alias the way a `Pos`-keyed
/// map can, and a bot has one identity across the whole game. But the
/// question `crates/planner` asks of `players` -- "which bots may I give
/// steps to" -- is a per-surface question, and answering it properly means
/// *moving a row when a bot crosses*, which nothing in this project can do.
/// They stay global and stay flagged rather than being quietly resolved in
/// either direction.
pub struct GameGlobals {
    /// **Ambiguous** -- see the type's doc. Global today.
    pub players: DashMap<PlayerId, FactorioPlayer>,
    /// A force's technologies and research progress are force-wide;
    /// `LuaForce::technologies` is not surface-indexed. Two copies is the
    /// disagree-about-research bug by name.
    pub forces: DashMap<String, FactorioForce>,
    pub graphics: DashMap<String, FactorioGraphic>,
    /// Prototype data, loaded once per save from the mod set. Recipe
    /// *availability* varies by force, not by surface.
    pub recipes: Arc<DashMap<String, FactorioRecipe>>,
    pub entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    pub item_prototypes: DashMap<String, FactorioItemPrototype>,
    pub image_cache: DashMap<String, Box<RgbaImage>>,
    /// Outcomes the game reported for dispatched actions, keyed by the
    /// `action_id` the dispatch used.
    ///
    /// The value carries the game tick alongside the result. The mod has
    /// always stamped one on its `action_completed` event; until this map
    /// could hold it, `OutputParser` threw it away -- which is why the
    /// executor had no game-clock source, and why every "duration" it
    /// reported was a plan value round-tripped through the log.
    pub actions: DashMap<u32, ActionOutcome>,
    pub path_requests: DashMap<u32, String>,
    /// **One id space for the session.** Two counters would hand two surfaces
    /// the same `action_id`, and the executor's completion signal is keyed on
    /// exactly that.
    pub next_action_id: Mutex<u32>,
    /// Teleports the mod has reported since the last drain, each tagged with
    /// the game tick the mod stamped on its `writeout` line.
    ///
    /// A teleport *crosses* surfaces -- that is what makes it a teleport --
    /// so it belongs to neither endpoint.
    ///
    /// `OutputParser` (`crates/core/src/process/output_parser.rs`) parses the
    /// line and pushes here; `crates/scripting_lua`'s `record.teleports()`
    /// drains it into `events.jsonl`. The queue exists because those two live
    /// in different crates and the dependency only runs one way.
    pub teleports: SyncMutex<Vec<(u64, TeleportEvent)>>,
    /// Deaths and respawns the mod has reported since the last drain. Same
    /// shape and same reason as `teleports`, and global for the same reason:
    /// the mod respawns a dead bot on `game.surfaces[1]` wherever it died.
    pub deaths: SyncMutex<Vec<(u64, BotLifeEvent)>>,
    /// Trigger technologies the mod has completed on a headless run since the
    /// last drain. Force-level facts, like `forces`.
    pub research_triggers: SyncMutex<Vec<(u64, ResearchTriggerEvent)>>,
    /// Chunks the mod refused because they are not on Nauvis, aggregated per
    /// surface since the last drain.
    ///
    /// Already keyed by [`SurfaceId`], and about surfaces this world
    /// deliberately does **not** hold -- it could only ever belong to the
    /// aggregate.
    ///
    /// **A map, not a `Vec`, and that is the whole design.** The mod writes
    /// one line per dropped chunk because keeping a counter there would mean
    /// keeping it in `storage` across save/load; a generated planet is tens
    /// of thousands of chunks, and tens of thousands of `events.jsonl` rows
    /// saying the same thing is not a disclosure, it is a denial of service
    /// on the reader.
    pub surface_chunk_drops: SyncMutex<BTreeMap<SurfaceId, SurfaceChunkDrops>>,
    /// Characters the game itself has said cannot move from where they stand.
    ///
    /// **Ambiguous** -- the fact it holds ("this bot cannot move from
    /// *here*") is positional, but it is keyed by [`PlayerId`] and follows
    /// `players`. See the type's doc.
    pub benches: SyncMutex<Benches>,
    /// What surfaces the running game **has**, as opposed to the one this
    /// bridge observes. See [`FactorioSurfaceInfo`].
    ///
    /// **Here and not on a surface, and that is not a preference.** A census
    /// of `game.surfaces` is a fact about the save, and a surface cannot hold
    /// the list of surfaces without every surface holding its own copy of the
    /// same list -- the exact shape the globals move exists to end. It is also
    /// the aggregate `output_parser` and `snapshot` can *reach*: both hold a
    /// [`FactorioSurface`](crate::factorio::world::FactorioSurface) and
    /// nothing above it, while
    /// [`FactorioWorld`](crate::factorio::world::FactorioWorld) is
    /// constructed in one place and reached only through `FactorioInstance`.
    /// `FactorioWorld::surface_census` reads it back from the top.
    ///
    /// **`None` is *nobody enumerated*, and an empty list would be a
    /// different claim** -- that a running game has no surfaces, which cannot
    /// happen. Folding the two together puts the field back into the silence
    /// it exists to end, so the `Option` is load-bearing and not a
    /// convenience.
    pub surfaces: SyncMutex<Option<Vec<FactorioSurfaceInfo>>>,
}

impl GameGlobals {
    pub fn new() -> Self {
        GameGlobals {
            players: DashMap::new(),
            forces: DashMap::new(),
            graphics: DashMap::new(),
            recipes: Arc::new(DashMap::new()),
            entity_prototypes: Arc::new(DashMap::new()),
            item_prototypes: DashMap::new(),
            image_cache: DashMap::new(),
            actions: DashMap::new(),
            path_requests: DashMap::new(),
            next_action_id: Mutex::new(1),
            teleports: SyncMutex::new(Vec::new()),
            deaths: SyncMutex::new(Vec::new()),
            research_triggers: SyncMutex::new(Vec::new()),
            surface_chunk_drops: SyncMutex::new(BTreeMap::new()),
            benches: SyncMutex::new(Benches::default()),
            surfaces: SyncMutex::new(None),
        }
    }
}

impl Default for GameGlobals {
    fn default() -> Self {
        GameGlobals::new()
    }
}

impl Clone for GameGlobals {
    /// A deep copy, because the caller is a speculative fork.
    ///
    /// The ephemeral queues start empty rather than duplicating in-flight
    /// events across two independent recorders, and `next_action_id` resets
    /// -- both exactly as they behaved when these fields lived on
    /// [`FactorioSurface`](crate::factorio::world::FactorioSurface).
    fn clone(&self) -> Self {
        GameGlobals {
            players: self.players.clone(),
            forces: self.forces.clone(),
            graphics: self.graphics.clone(),
            recipes: Arc::new((*self.recipes).clone()),
            entity_prototypes: Arc::new((*self.entity_prototypes).clone()),
            item_prototypes: self.item_prototypes.clone(),
            image_cache: self.image_cache.clone(),
            actions: self.actions.clone(),
            path_requests: self.path_requests.clone(),
            next_action_id: Mutex::new(0),
            teleports: SyncMutex::new(Vec::new()),
            deaths: SyncMutex::new(Vec::new()),
            research_triggers: SyncMutex::new(Vec::new()),
            surface_chunk_drops: SyncMutex::new(BTreeMap::new()),
            // Knowledge -- the game's own verdict on who can move -- with its
            // change queue reset like the record cursors on the surface: a
            // second recorder has been told about none of it.
            benches: SyncMutex::new(self.benches.lock().fork()),
            // Knowledge, like `benches`: what surfaces the game has does not
            // change because a plan is being imagined against it, and a fork
            // that forgot the census would answer "nobody enumerated" for a
            // world where somebody did.
            surfaces: SyncMutex::new(self.surfaces.lock().clone()),
        }
    }
}
