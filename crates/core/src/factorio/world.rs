use crate::factorio::snapshot::WorldSnapshot;
use crate::factorio::ticks::ActionOutcome;
use crate::graph::entity_graph::EntityGraph;
use crate::graph::flow_graph::FlowGraph;
use crate::types::{
    ActionId, FactorioEntity, FactorioEntityPrototype, FactorioForce, FactorioGraphic,
    FactorioItemPrototype, FactorioPlayer, FactorioRecipe, FactorioTile,
    PlayerChangedDistanceEvent, PlayerChangedMainInventoryEvent, PlayerChangedPositionEvent,
    PlayerId, Position,
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

/// The payload of a `"teleport"` writeout, emitted by all three of
/// `mods/BotBridge/control.lua`'s `player.teleport` call sites (see
/// `teleport_writeout` there): the stuck-walk timeout, and the two sites that
/// move a bot out of a ghost's or a blueprint's bounding box before reviving
/// it. `action_id` is only ever present for the stuck-walk site -- the other
/// two are synchronous RCON calls with no dispatched action to attach to.
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
#[derive(Debug, Clone, PartialEq)]
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
}

/// Every [`PlacementRefusal`] this run has collected, plus how many of them a
/// record has already been told about.
///
/// Append-only and **never drained**, unlike
/// [`FactorioWorld::teleports`](FactorioWorld#structfield.teleports): a
/// teleport is an event that needs writing once, while a refusal is a
/// standing fact the planner has to re-read on every plan. The `reported`
/// cursor is what lets `record.refusals()` write each one exactly once
/// without taking it away from the planner.
#[derive(Debug, Default)]
pub struct PlacementRefusals {
    sites: Vec<PlacementRefusal>,
    reported: usize,
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

pub struct FactorioWorld {
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
    /// [`FactorioWorld::drain_teleports`], each tagged with the game tick the
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
    pub placement_refusals: SyncMutex<PlacementRefusals>,
}

impl FactorioWorld {
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

    pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> Result<()> {
        // TODO: update entity direction
        Ok(())
    }

    pub fn on_some_entity_created(&self, entity: FactorioEntity) -> Result<()> {
        info!("XXX on_some_entity_created {:?}", &entity);
        self.entity_graph.add(vec![entity], None)?;
        Ok(())
    }

    pub fn on_some_entity_deleted(&self, entity: FactorioEntity) -> Result<()> {
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

    pub fn import(&mut self, world: Arc<FactorioWorld>) -> Result<()> {
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
        FactorioWorld {
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
            placement_refusals: SyncMutex::new(PlacementRefusals::default()),
        }
    }

    /// Queues a teleport `OutputParser` just parsed, for
    /// [`FactorioWorld::drain_teleports`] to pick up.
    pub fn record_teleport(&self, tick: u64, event: TeleportEvent) {
        self.teleports.lock().push((tick, event));
    }

    /// Takes every teleport queued since the last drain, oldest first.
    pub fn drain_teleports(&self) -> Vec<(u64, TeleportEvent)> {
        std::mem::take(&mut *self.teleports.lock())
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

    /// The refusals no record has been told about yet, oldest first, marking
    /// them reported. The sites themselves stay — see [`PlacementRefusals`].
    pub fn unreported_placement_refusals(&self) -> Vec<PlacementRefusal> {
        let mut ledger = self.placement_refusals.lock();
        let from = ledger.reported;
        ledger.reported = ledger.sites.len();
        ledger.sites[from..].to_vec()
    }

    pub fn dump(&self, save_path: Option<&str>) -> Result<()> {
        let content = serde_json::to_string_pretty(self).into_diagnostic()?;
        if let Some(save_path) = save_path {
            fs::write(save_path, &content).into_diagnostic()?;
        } else {
            println!("{content}");
        }

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

unsafe impl Send for FactorioWorld {}
unsafe impl Sync for FactorioWorld {}

impl Serialize for FactorioWorld {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FactorioWorld", 9)?;
        state.serialize_field("players", &self.players)?;
        state.serialize_field("forces", &self.forces)?;
        state.serialize_field("graphics", &self.graphics)?;
        state.serialize_field("recipes", &*self.recipes)?;
        state.serialize_field("entity_prototypes", &*self.entity_prototypes)?;
        state.serialize_field("item_prototypes", &self.item_prototypes)?;
        state.serialize_field("actions", &self.actions)?;
        state.serialize_field("path_requests", &self.path_requests)?;
        state.serialize_field("entity_graph", &*self.entity_graph)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for FactorioWorld {
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
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct FactorioWorldVisitor;

        impl<'de> Visitor<'de> for FactorioWorldVisitor {
            type Value = FactorioWorld;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FactorioWorld")
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

                let entity_graph: Arc<EntityGraph> = Arc::new(entity_graph);
                let flow_graph = Arc::new(FlowGraph::new(entity_graph.clone()));
                Ok(FactorioWorld {
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
                    placement_refusals: Default::default(),
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
        ];
        deserializer.deserialize_struct("FactorioWorld", FIELDS, FactorioWorldVisitor)
    }
}

impl Clone for FactorioWorld {
    fn clone(&self) -> Self {
        let entity_prototypes = Arc::new((*self.entity_prototypes).clone());
        let recipes = Arc::new((*self.recipes).clone());
        let entity_graph = Arc::new((*self.entity_graph).clone());
        let _entity_graph = entity_graph.clone();
        FactorioWorld {
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
        let world = FactorioWorld {
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
            placement_refusals: Default::default(),
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
}
