use crate::error::PlannerError;
use crate::goal::Holder;
use crate::ids::{BotId, ItemId};
use factorio_bot_core::factorio::util::add_to_rect;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{
    FactorioEntity, FactorioTechnology, Pos, Position, Rect, ResourcePatch,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// How far two collision boxes may reach into each other before it counts.
///
/// Factorio lets boxes that merely touch along an edge coexist, and the
/// prototype numbers are binary fractions (a stone furnace is ±0.69921875),
/// so exact touching really happens. A 1/512 tile is finer than the 1/256 the
/// game stores positions at, so nothing this admits is a collision the game
/// would see; it only keeps float noise from reading as one.
const TOUCH_SLACK: f64 = 1. / 512.;

/// Do two collision boxes share ground? Touching along an edge does not count.
fn boxes_overlap(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x() - TOUCH_SLACK
        && b.left_top.x() < a.right_bottom.x() - TOUCH_SLACK
        && a.left_top.y() < b.right_bottom.y() - TOUCH_SLACK
        && b.left_top.y() < a.right_bottom.y() - TOUCH_SLACK
}

/// The one-tile box covering `tile`. Tile `(x, y)` spans `[x, x+1)`.
fn tile_area(tile: &Pos) -> Rect {
    Rect::new(
        &Position::new(tile.0 as f64, tile.1 as f64),
        &Position::new(tile.0 as f64 + 1., tile.1 as f64 + 1.),
    )
}

/// Every tile `area` reaches into, in a fixed order.
///
/// The slack keeps a box that stops exactly on a tile boundary from claiming
/// the tile beyond it, which is the same edge case `boxes_overlap` handles.
fn tiles_under(area: &Rect) -> Vec<Pos> {
    let x0 = (area.left_top.x() + TOUCH_SLACK).floor() as i32;
    let x1 = (area.right_bottom.x() - TOUCH_SLACK).floor() as i32;
    let y0 = (area.left_top.y() + TOUCH_SLACK).floor() as i32;
    let y1 = (area.right_bottom.y() - TOUCH_SLACK).floor() as i32;
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(Pos(x, y));
        }
    }
    out
}

/// How much ore one tile yields before this plan exhausts it.
///
/// Factorio reports per-tile resource amounts, but they do not survive into the
/// entity graph: `FactorioEntity::new_resource` (`core/src/types.rs:686`) leaves
/// `amount` as `None`, and `EntityGraph::add` (`core/src/graph/entity_graph.rs:217`)
/// never inserts resource entities into the entity tree. A constant is therefore
/// the only capacity available. Honouring real amounts needs a BotBridge change and
/// is out of scope.
pub const DEFAULT_RESOURCE_PER_TILE: u32 = 500;

/// A bot's simulated state during planning.
#[derive(Clone, Debug)]
pub struct BotState {
    pub position: Position,
    pub inventory: BTreeMap<ItemId, u32>,
    pub build_distance: f64,
    pub reach_distance: f64,
    pub resource_reach_distance: f64,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            position: Position::new(0., 0.),
            inventory: BTreeMap::new(),
            build_distance: 10.0,
            reach_distance: 10.0,
            resource_reach_distance: 3.0,
        }
    }
}

/// The world at a point in a hypothetical plan.
///
/// `base` is shared and never mutated; every difference lives in the overlay
/// fields, so `fork` costs the overlay rather than a world copy.
#[derive(Clone)]
pub struct PlanState {
    base: Arc<FactorioWorld>,
    bots: BTreeMap<BotId, BotState>,
    /// Bots `from_world` was asked for that `base` has no player for.
    ///
    /// Every bot in here got a `BotState::default()` instead of the world's
    /// real inventory and reach distances — an empty inventory and guessed
    /// reach limits, not "this bot has none of that yet". Today that never
    /// happens on the production path: `initiate_missing_players_with_default_inventory`
    /// (`crates/core/src/plan/planner.rs`) seeds every player id the run's
    /// roster names before `from_world` ever sees it, and every planner
    /// fixture that constructs a `PlanState` directly relies on the very
    /// fallback this records (`fixture_world()` never has players, so every
    /// existing test is "unknown" by this definition).
    ///
    /// That is exactly why `from_world` cannot turn this into a hard error
    /// without also rewriting every one of those fixtures, several of which
    /// (`tests/scheduling.rs`, `tests/red_science.rs`) are pinned byte-for-byte.
    /// So the substitution stays, but stops being silent: a caller that names
    /// a `BotId` the world does not have — the per-bot goal API this was
    /// flagged against — can check `unknown_bots()` and refuse to build a plan
    /// against invented reach distances, which `crates/scripting_lua` (out of
    /// this crate's scope) is where that refusal belongs.
    unknown_bots: BTreeSet<BotId>,
    /// Entities added by the plan, keyed by tile.
    added: BTreeMap<Pos, FactorioEntity>,
    /// Positions whose base-world entity the plan has removed.
    removed: BTreeSet<Pos>,
    /// Ore taken from a tile by the plan, subtracted from the base amount.
    consumed: BTreeMap<Pos, u32>,
    /// The one force this plan acts for, or `None` if `base` carries no forces.
    ///
    /// Chosen once, here, and read by everything that asks a question about
    /// technology — `is_researched` and `technology` both. That single
    /// selection site is the point. Before it there were two: `is_researched`
    /// answered "yes" if *any* force had the technology researched, while the
    /// cost of that same technology was read from whichever force sorted
    /// first. Each was individually defensible and together they disagreed
    /// about who we are planning for, so a world where `alpha` has not
    /// researched automation and `zeta` has made the planner silently emit
    /// nothing for `Researched("automation")` — refusing to research something
    /// the acting force does not have.
    ///
    /// The planner has no concept of acting for several forces at once, and
    /// inventing one to fix this would be speculative. So it acts for exactly
    /// one, and the type says so: there is nowhere else to make the choice and
    /// nothing else to disagree with.
    ///
    /// Which force: the alphabetically first, because `FactorioWorld::forces`
    /// is a `DashMap` whose iteration order moves with the hash seed and
    /// planning has to be reproducible. Every world this plans against has a
    /// single force, so the tie-break decides nothing in practice; it exists so
    /// that a world with several cannot make planning depend on the seed.
    force: Option<String>,
    /// Technologies the plan has completed.
    ///
    /// Keyed by bare technology name, which is unambiguous precisely because a
    /// `PlanState` acts for one force: the overlay cannot mean a different
    /// force's copy of the technology than `force` above names. If the planner
    /// ever learns to act for several, this has to become `(force, tech)` at
    /// the same time — the two are one decision, not two.
    researched: BTreeSet<String>,
    /// Holdings expansion has already promised to an action it is about to
    /// emit, keyed the way [`Holder`] keys a shortfall: per bot for
    /// `Holder::Bot`/`Holder::Share`, and once for the roster as a whole for
    /// `Holder::Anyone`.
    ///
    /// This is *not* inventory. Nothing here has been spent — the items are
    /// still in the bot's hands, and [`PlanState::inventory_count`] and
    /// [`PlanState::lose`] both still see them, which is what keeps the
    /// emitted plan's own arithmetic (and `Condition::HasItem`'s check of it)
    /// reading the real simulated inventory. A reservation only answers a
    /// different question: how much is left over for some *other* goal to
    /// count towards itself. See [`PlanState::available`].
    reserved: BTreeMap<BotId, BTreeMap<ItemId, u32>>,
    /// The `Holder::Anyone` half of `reserved`, which names no bot.
    reserved_by_anyone: BTreeMap<ItemId, u32>,
    /// Half the diagonal of the largest collision box among `base`'s known
    /// entity prototypes, or `0.` if it carries none.
    ///
    /// Computed once from `base` in [`PlanState::from_world`] rather than on
    /// every [`PlanState::is_area_clear`] call: `base` never changes after
    /// construction (see the struct doc), so the value cannot go stale, and
    /// this avoids rescanning the prototype table for every candidate
    /// placement a search considers. `entity_prototypes` is a `DashMap` with
    /// no defined iteration order, but a maximum over its values does not
    /// depend on that order, so this stays deterministic.
    max_prototype_half_diagonal: f64,
}

impl PlanState {
    pub fn from_world(base: Arc<FactorioWorld>, bots: &[BotId]) -> PlanState {
        let mut map = BTreeMap::new();
        let mut unknown_bots = BTreeSet::new();
        for id in bots {
            let state = match base.players.get(&id.0) {
                Some(player) => BotState {
                    position: player.position.clone(),
                    inventory: player.main_inventory.clone(),
                    build_distance: player.build_distance as f64,
                    reach_distance: player.reach_distance as f64,
                    resource_reach_distance: player.resource_reach_distance,
                },
                None => {
                    unknown_bots.insert(*id);
                    BotState::default()
                }
            };
            map.insert(*id, state);
        }
        let max_prototype_half_diagonal = base
            .entity_prototypes
            .iter()
            .map(|entry| {
                let b = &entry.value().collision_box;
                (b.width() / 2.).hypot(b.height() / 2.)
            })
            .fold(
                0.0_f64,
                |acc, d| if d.total_cmp(&acc).is_gt() { d } else { acc },
            );
        let force = base.forces.iter().map(|entry| entry.key().clone()).min();
        PlanState {
            base,
            bots: map,
            unknown_bots,
            added: Default::default(),
            removed: Default::default(),
            consumed: Default::default(),
            force,
            researched: Default::default(),
            reserved: Default::default(),
            reserved_by_anyone: Default::default(),
            max_prototype_half_diagonal,
        }
    }

    pub fn fork(&self) -> PlanState {
        self.clone()
    }

    pub fn base(&self) -> &Arc<FactorioWorld> {
        &self.base
    }

    /// Bots passed to [`PlanState::from_world`] that `base` had no player
    /// for, and so were seeded with `BotState::default()` — an empty
    /// inventory and guessed reach distances — instead of the world's own
    /// data. Empty whenever every requested bot was a real player, which is
    /// every production call today; see the field doc for why this is a
    /// detectable flag rather than a hard error.
    pub fn unknown_bots(&self) -> &BTreeSet<BotId> {
        &self.unknown_bots
    }

    /// Technology `name` as the acting force defines it.
    ///
    /// The only way to reach a `FactorioTechnology` from a `PlanState`, so
    /// that a caller cannot read one force's cost while `is_researched`
    /// answers about another's.
    pub fn technology(&self, name: &str) -> Option<FactorioTechnology> {
        let force = self.force.as_deref()?;
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.technologies.get(name).cloned())
    }

    /// The acting force's `manual_mining_speed_modifier`, or `0.` when the
    /// world does not report one.
    ///
    /// Read through the acting force for the same reason `technology` is: the
    /// planner acts for exactly one force, and a mining rate taken from a
    /// different force than the research questions are answered against would
    /// be the same silent disagreement that field doc describes.
    ///
    /// `0.` is the game's own default for an unmodified force, so a world
    /// with no forces at all (every planner fixture) reads as "no bonus"
    /// rather than as an error.
    pub fn manual_mining_speed_modifier(&self) -> f64 {
        let Some(force) = self.force.as_deref() else {
            return 0.;
        };
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.manual_mining_speed_modifier.as_deref().copied())
            .map(f64::from)
            .unwrap_or(0.)
    }

    /// Every technology the acting force defines, in name order.
    ///
    /// Ordered because `recipe -> technology` lookups scan this and must pick
    /// the same answer on every run; the underlying map is a `BTreeMap`, so the
    /// order is the data's, not the hash seed's.
    pub fn technology_names(&self) -> Vec<String> {
        let Some(force) = self.force.as_deref() else {
            return Vec::new();
        };
        self.base
            .forces
            .get(force)
            .map(|entry| entry.technologies.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn bot_ids(&self) -> Vec<BotId> {
        self.bots.keys().copied().collect()
    }

    pub fn bot(&self, id: BotId) -> Option<&BotState> {
        self.bots.get(&id)
    }

    pub fn inventory_count(&self, id: BotId, item: &str) -> u32 {
        self.bots
            .get(&id)
            .and_then(|b| b.inventory.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Every item any bot holds, with the roster-wide total for each.
    pub fn item_totals(&self) -> BTreeMap<ItemId, u32> {
        let mut out: BTreeMap<ItemId, u32> = BTreeMap::new();
        for bot in self.bots.values() {
            for (item, count) in &bot.inventory {
                *out.entry(item.clone()).or_insert(0) += *count;
            }
        }
        out
    }

    /// Sum across every bot — the meaning of `Holder::Anyone`.
    pub fn total_count(&self, item: &str) -> u32 {
        self.bots
            .values()
            .map(|b| b.inventory.get(item).copied().unwrap_or(0))
            .sum()
    }

    /// How much of `item` is left for a *new* goal to count towards itself:
    /// what `whose` holds, less what expansion has already promised to an
    /// action it is about to emit.
    ///
    /// This — not `inventory_count`/`total_count` — is the question
    /// "is this goal already satisfied" has to ask. Asking the raw holding
    /// double-counts an intermediate two sub-goals of one recipe both draw on:
    /// a lab's own ten gears are visible to the sub-goal that produces its
    /// transport belts, which then spends two of them and leaves the lab craft
    /// short. Reserving is what stops a holding being claimed twice.
    ///
    /// A per-bot reservation lowers the roster total too, because the items it
    /// names are a known bot's and so genuinely spoken for. An `Anyone`
    /// reservation does *not* lower any one bot's figure, because it names no
    /// bot: nothing says which of them holds the promised items, and guessing
    /// would refuse work a bot can really do.
    pub fn available(&self, whose: &Holder, item: &str) -> u32 {
        match whose {
            Holder::Anyone => {
                let promised: u32 = self
                    .reserved
                    .values()
                    .map(|held| held.get(item).copied().unwrap_or(0))
                    .sum::<u32>()
                    .saturating_add(self.reserved_by_anyone.get(item).copied().unwrap_or(0));
                self.total_count(item).saturating_sub(promised)
            }
            Holder::Bot(id) | Holder::Share(id) => self
                .inventory_count(*id, item)
                .saturating_sub(self.reserved_for(*id, item)),
        }
    }

    fn reserved_for(&self, id: BotId, item: &str) -> u32 {
        self.reserved
            .get(&id)
            .and_then(|held| held.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Promise `count` of `item` to `whose`, hiding it from [`available`] until
    /// [`PlanState::release`] gives it back. Reservations add up, so two
    /// promises of the same item hide both.
    ///
    /// [`available`]: PlanState::available
    pub fn reserve(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let entry = ledger.entry(item.to_string()).or_insert(0);
        *entry = entry.saturating_add(count);
    }

    /// Undo one [`PlanState::reserve`]. Saturating rather than fallible: a
    /// release is always paired with a reserve by the caller that made it, so
    /// there is no failure for a caller to handle, and clamping at zero cannot
    /// invent stock the way wrapping would.
    pub fn release(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let Some(entry) = ledger.get_mut(item) else {
            return;
        };
        *entry = entry.saturating_sub(count);
        if *entry == 0 {
            ledger.remove(item);
        }
    }

    pub fn gain(&mut self, id: BotId, item: &str, count: u32) {
        let bot = self.bots.entry(id).or_default();
        *bot.inventory.entry(item.to_string()).or_insert(0) += count;
    }

    pub fn lose(&mut self, id: BotId, item: &str, count: u32) -> Result<(), PlannerError> {
        let available = self.inventory_count(id, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: id,
                item: item.to_string(),
                required: count,
                available,
            });
        }
        let bot = self.bots.get_mut(&id).ok_or(PlannerError::UnknownBot(id))?;
        let entry = bot.inventory.entry(item.to_string()).or_insert(0);
        *entry -= count;
        if *entry == 0 {
            bot.inventory.remove(item);
        }
        Ok(())
    }

    pub fn set_position(&mut self, id: BotId, position: Position) {
        self.bots.entry(id).or_default().position = position;
    }

    pub fn entity_at(&self, position: &Position) -> Option<FactorioEntity> {
        let key = Pos::from(position);
        if let Some(entity) = self.added.get(&key) {
            return Some(entity.clone());
        }
        if self.removed.contains(&key) {
            return None;
        }
        self.base
            .entity_graph
            .entity_at(position)
            .and_then(|id| self.base.entity_graph.entity_by_id(id))
    }

    /// The ground an entity of `name` would cover with its centre at `position`,
    /// in world coordinates.
    ///
    /// Read from the game's own `collision_box` prototype — never assumed and
    /// never per-entity constants. A stone furnace is ±0.69921875 and an
    /// assembling machine ±1.19921875; the planner has no business knowing
    /// either number.
    ///
    /// `None` when the world has no prototype under that name. Callers treat
    /// that as *not placeable* rather than guessing a size: a guess that is too
    /// small is exactly the defect this function exists to remove, and the only
    /// names the planner ever places come from the world's own prototypes and
    /// recipes, so a miss means the world carries no prototype data at all.
    /// Failing to plan is recoverable; planning a placement the game refuses,
    /// which costs the bot its whole remaining slice, is not.
    pub fn collision_area(&self, name: &str, position: &Position) -> Option<Rect> {
        self.base
            .entity_prototypes
            .get(name)
            .map(|proto| add_to_rect(&proto.collision_box, position))
    }

    /// The ground an entity already in the plan covers.
    ///
    /// `create_entity` fills the entity's `bounding_box` from its prototype, so
    /// this is normally that box. The fallback — the single tile the entity
    /// stands on — is for entities put into the state directly with neither a
    /// bounding box nor a known prototype, and it under-reserves; it is the
    /// least this can claim without inventing a size.
    fn footprint_of(&self, entity: &FactorioEntity) -> Rect {
        if entity.bounding_box.width() > 0. && entity.bounding_box.height() > 0. {
            return entity.bounding_box.clone();
        }
        self.collision_area(&entity.name, &entity.position)
            .unwrap_or_else(|| tile_area(&Pos::from(&entity.position)))
    }

    /// Whether an entity of `name` could be built with its centre at `position`.
    ///
    /// The question `is_position_free` could not ask. A stone furnace is 1.398
    /// tiles across, so two of them one tile apart overlap while each one's
    /// *tile* is free — which is how the red-science plan came to site furnaces
    /// at `[-34, -1]` and `[-34, 0]` and have the game refuse the second.
    ///
    /// Entities are compared box against box rather than tile against tile, so
    /// a placement is refused when it would actually collide and not merely
    /// when it shares a tile.
    pub fn is_area_free(&self, name: &str, position: &Position) -> bool {
        match self.collision_area(name, position) {
            Some(area) => self.is_area_clear(&area),
            None => false,
        }
    }

    /// Whether anything the plan can see occupies `area`.
    ///
    /// Three sources, because no single one of them sees everything:
    /// entities this plan has placed, entities the base world already had, and
    /// ore. Ore is the odd one — `EntityGraph::add` routes resource entities
    /// into `resources`/`resource_tree` only, so the entity tree never sees
    /// them and they have to be asked for by tile.
    fn is_area_clear(&self, area: &Rect) -> bool {
        for entity in self.added.values() {
            if boxes_overlap(&self.footprint_of(entity), area) {
                return false;
            }
        }
        // `find_entities_in_radius` keeps only entities whose *centre* point
        // falls within `radius` of `search_center` (see
        // `EntityGraph::find_entities_in_radius`,
        // `core/src/graph/entity_graph.rs:132`) — it does not know the
        // entity's own footprint, so a neighbour whose box overlaps `area`
        // while its centre sits outside `radius` would silently be skipped.
        // By the triangle inequality for the Euclidean norm, an entity whose
        // box overlaps `area` has its centre no further from `area`'s centre
        // than `area`'s own half-diagonal plus that entity's half-diagonal.
        // Widening the radius by the largest half-diagonal among the world's
        // known prototypes therefore cannot miss a genuine overlap, as long
        // as no unknown prototype is wider than every known one. The exact
        // test is the `boxes_overlap` below; this only narrows the search.
        let area_half_diagonal = (area.width() / 2.).hypot(area.height() / 2.);
        let radius = area_half_diagonal + self.max_prototype_half_diagonal + TOUCH_SLACK;
        for entity in
            self.base
                .entity_graph
                .find_entities_in_radius(area.center(), radius, None, None)
        {
            if self.removed.contains(&Pos::from(&entity.position)) {
                continue;
            }
            if boxes_overlap(&entity.bounding_box, area) {
                return false;
            }
        }
        !tiles_under(area)
            .iter()
            .any(|tile| self.base.entity_graph.any_resource_at(tile))
    }

    /// Whether this tile is clear.
    ///
    /// A tile-granularity question, kept because that is what
    /// `Condition::PositionFree` asks. It says nothing about whether a
    /// *particular* entity fits — a placement wants [`is_area_free`], which
    /// knows how big the thing being placed is.
    ///
    /// Presence, not quantity: a tile whose ore this plan has drained to zero is
    /// still an ore tile in the ground, so it stays occupied. `remove_entity`
    /// frees a placed entity's tile, but it does not clear ore.
    pub fn is_position_free(&self, position: &Position) -> bool {
        self.is_area_clear(&tile_area(&Pos::from(position)))
    }

    /// Records a placed entity, giving it the footprint its prototype says it
    /// has if it arrived without one.
    ///
    /// The fill-in happens here, once, rather than at each of the places that
    /// ask about occupancy: methods build a `FactorioEntity` from a name and a
    /// position and leave `bounding_box` at its zero default, and a zero box
    /// collides with nothing.
    pub fn create_entity(&mut self, mut entity: FactorioEntity) {
        let key = Pos::from(&entity.position);
        if (entity.bounding_box.width() == 0. || entity.bounding_box.height() == 0.)
            && let Some(area) = self.collision_area(&entity.name, &entity.position)
        {
            entity.bounding_box = area;
        }
        self.removed.remove(&key);
        self.added.insert(key, entity);
    }

    pub fn remove_entity(&mut self, position: &Position) {
        let key = Pos::from(position);
        self.added.remove(&key);
        self.removed.insert(key);
    }

    /// Ore remaining at a tile: the tile's capacity less what this plan has taken.
    ///
    /// Presence comes from `resource_contains`, not `entity_at`: `EntityGraph::add`
    /// routes resource entities into `resources`/`resource_tree` only, so they are
    /// absent from the entity tree that `entity_at` queries.
    pub fn resource_available(&self, position: &Position, item: &str) -> u32 {
        let key = Pos::from(position);
        if !self.base.entity_graph.resource_contains(item, key.clone()) {
            return 0;
        }
        DEFAULT_RESOURCE_PER_TILE.saturating_sub(self.consumed.get(&key).copied().unwrap_or(0))
    }

    pub fn consume_resource(
        &mut self,
        position: &Position,
        item: &str,
        count: u32,
    ) -> Result<(), PlannerError> {
        let available = self.resource_available(position, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: BotId(0),
                item: item.to_string(),
                required: count,
                available,
            });
        }
        *self.consumed.entry(Pos::from(position)).or_insert(0) += count;
        Ok(())
    }

    /// Has the acting force finished `tech`, either already or under this plan?
    ///
    /// The union of the plan's overlay and what the acting force reports. The
    /// two cannot contradict each other on the *time* axis, because research
    /// is monotone: nothing in the game or in this planner un-researches a
    /// technology, so the overlay can only add. They could once contradict
    /// each other on the *force* axis, which is what `force` above fixes —
    /// this asks the same force `technology` reads costs from, and no other.
    pub fn is_researched(&self, tech: &str) -> bool {
        if self.researched.contains(tech) {
            return true;
        }
        matches!(self.technology(tech), Some(t) if t.researched)
    }

    pub fn set_researched(&mut self, tech: &str) {
        self.researched.insert(tech.to_string());
    }

    /// The base world's resource patches, with each patch's tiles in a stable
    /// order.
    ///
    /// `EntityGraph::resource_patches` builds `elements` by iterating a
    /// `HashMap` (`core/src/graph/entity_graph.rs:164-170`), so tile order
    /// varies with the randomised hash seed. Planning is required to be
    /// deterministic — the same world and the same goal must give the same
    /// schedule — and a method picking "the first tile of the patch" would
    /// otherwise pick a different one on every run. Sorting here keeps the fix
    /// inside the planner rather than perturbing `core`.
    ///
    /// **This makes tile order deterministic, not the patches themselves.** The
    /// flood fill at `entity_graph.rs:152-162` marks `pos` where it means
    /// `other`, so a tile pushed onto the stack is only ever labelled if it
    /// still has an unlabelled neighbour when it is popped; one that does not
    /// stays unlabelled and seeds a fresh patch. The fixture's contiguous
    /// 11x11 ore square therefore comes back as two or three patches, and which
    /// tiles are orphaned changes between calls *within one process*. Sorting
    /// cannot repair a partition that is wrong before it arrives. Fixing it is
    /// a one-word change in `core` and belongs with a test there.
    pub fn resource_patches(&self, item: &str) -> Vec<ResourcePatch> {
        let mut patches = self.base.entity_graph.resource_patches(item);
        for patch in &mut patches {
            patch
                .elements
                .sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        }
        patches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_world};
    use factorio_bot_core::types::{FactorioEntity, Position};

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    fn entity_at_pos(name: &str, x: f64, y: f64) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: "furnace".into(),
            position: Position::new(x, y),
            ..Default::default()
        }
    }

    #[test]
    fn a_second_furnace_one_tile_from_the_first_does_not_fit() {
        // The defect, in miniature. The neighbouring tile is empty by the tile
        // test -- the first furnace's *centre* is not on it -- but a stone
        // furnace is 1.398 tiles across, so the boxes overlap and the game
        // refuses the second placement. This is what sited the red-science
        // furnaces at [-34, -1] and [-34, 0].
        let mut s = state();
        let first = Position::new(0., -1.);
        assert!(s.is_area_free("stone-furnace", &first), "open ground");
        s.create_entity(entity_at_pos("stone-furnace", first.x, first.y));

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a furnace one tile from another overlaps it"
        );
        // Two tiles apart it does fit -- otherwise this test would also pass on
        // an `is_area_free` that simply always says no.
        assert!(
            s.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "two tiles of clearance is enough for a 1.398-wide entity"
        );
    }

    #[test]
    fn a_wide_base_world_neighbour_outside_the_old_radius_still_blocks() {
        // Reviewer's case. `find_entities_in_radius`
        // (`core/src/graph/entity_graph.rs:132`) filters candidates on each
        // entity's *centre* point, not its footprint. A real `rock-huge` is
        // 3 tiles across (half-extent 1.5, per its prototype's collision
        // box), so a copy of it centred 1.75 tiles from the query centre
        // geometrically overlaps a stone furnace placed there -- but the old
        // radius, half the query box's own span plus a flat `1.`, topped out
        // at ~1.699 and so never asked about a neighbour that far out. This
        // builds the neighbour with its *real* collision box rather than
        // going through `spawn_rocks`/`new_rock`, which hard-codes every rock
        // to 1.2 wide regardless of name and would not reproduce the defect.
        let world = fixture_world();
        let neighbour_pos = Position::new(1.75, 0.);
        let collision_box = fixture_entity_prototypes()
            .get("rock-huge")
            .expect("fixture has rock-huge")
            .collision_box
            .clone();
        world
            .update_chunk_entities(vec![FactorioEntity {
                name: "rock-huge".into(),
                entity_type: "simple-entity".into(),
                position: neighbour_pos.clone(),
                bounding_box: add_to_rect(&collision_box, &neighbour_pos),
                ..Default::default()
            }])
            .unwrap();
        let s = PlanState::from_world(Arc::new(world), &[]);

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a stone furnace at the origin overlaps the rock-huge's real \
             footprint, even though the rock's centre is 1.75 away and the \
             old query radius topped out at ~1.70"
        );
    }

    #[test]
    fn how_much_room_is_needed_comes_from_the_prototype() {
        // Identical geometry, two tiles apart, and the answer differs by
        // prototype: 1.398 across fits, 2.398 across does not. Neither number
        // appears here, and a fixed size -- whatever it was -- would have to
        // give these two the same answer.
        let mut small = state();
        small.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        assert!(
            small.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "a stone furnace fits two tiles from another"
        );

        let mut large = state();
        large.create_entity(entity_at_pos("assembling-machine-1", 0., -1.));
        assert!(
            !large.is_area_free("assembling-machine-1", &Position::new(0., 1.)),
            "an assembling machine does not fit in the same gap"
        );
    }

    #[test]
    fn an_entity_with_no_prototype_is_refused_rather_than_guessed_at() {
        let s = state();
        assert!(
            s.collision_area("not-a-real-entity", &Position::new(0., 0.))
                .is_none()
        );
        assert!(
            !s.is_area_free("not-a-real-entity", &Position::new(0., 0.)),
            "an unknown size must fail to plan, not be assumed small"
        );
    }

    #[test]
    fn a_placed_entity_carries_the_footprint_its_prototype_gives_it() {
        // Methods build a `FactorioEntity` from a name and a position and leave
        // `bounding_box` at zero; a zero box collides with nothing, so the fill
        // in `create_entity` is what makes the overlap test above possible.
        let mut s = state();
        assert_eq!(
            entity_at_pos("stone-furnace", 0., -1.).bounding_box.width(),
            0.
        );
        s.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        let stored = s.entity_at(&Position::new(0., -1.)).expect("placed");
        assert!(
            stored.bounding_box.width() > 1.,
            "expected the prototype's 1.398, got {}",
            stored.bounding_box.width()
        );
    }

    #[test]
    fn fork_shares_the_base_world() {
        let a = state();
        let b = a.fork();
        assert!(Arc::ptr_eq(a.base(), b.base()));
    }

    #[test]
    fn fork_isolates_inventory_changes() {
        let a = state();
        let mut b = a.fork();
        b.gain(BotId(1), "iron-ore", 5);
        assert_eq!(b.inventory_count(BotId(1), "iron-ore"), 5);
        assert_eq!(a.inventory_count(BotId(1), "iron-ore"), 0);
    }

    /// `item_totals` is the roster's inventory, not one bot's.
    ///
    /// It exists so the driver can diff a chain's produce, and a diff that saw
    /// only the bot it happened to start from would under-count every chain
    /// whose actor is anyone else — silently, since a smaller diff just
    /// reserves less.
    #[test]
    fn item_totals_reports_every_bots_holdings() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        a.gain(BotId(2), "coal", 1);
        let totals = a.item_totals();
        assert_eq!(totals.get("iron-plate"), Some(&7));
        assert_eq!(totals.get("coal"), Some(&1));
        assert_eq!(totals.get("wood"), None, "nothing is invented");
    }

    #[test]
    fn total_count_sums_across_bots() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        assert_eq!(a.total_count("iron-plate"), 7);
    }

    #[test]
    fn lose_more_than_held_is_an_error() {
        let mut a = state();
        a.gain(BotId(1), "coal", 2);
        assert!(a.lose(BotId(1), "coal", 3).is_err());
        assert_eq!(a.inventory_count(BotId(1), "coal"), 2);
    }

    #[test]
    fn a_bot_takes_its_position_inventory_and_reach_from_its_player() {
        // The only path a real world takes into the planner: `fixture_world()`
        // has no players, so every other test exercises the `None` arm and the
        // `Some` arm's field reads run nowhere else.
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: Position::new(12.5, -7.5),
                main_inventory: BTreeMap::from([("iron-plate".to_string(), 42u32)]),
                build_distance: 12,
                reach_distance: 8,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1), BotId(2)]);
        let bot = s.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.position, Position::new(12.5, -7.5));
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 42);
        assert_eq!(bot.build_distance, 12.0);
        assert_eq!(bot.reach_distance, 8.0);
        assert_eq!(bot.resource_reach_distance, 4.0);

        // Bot 2 has no player and still falls back to the defaults.
        let other = s.bot(BotId(2)).expect("bot 2 exists");
        assert_eq!(other.position, Position::new(0., 0.));
        assert_eq!(other.build_distance, 10.0);

        // The fallback is recorded: bot 2 got invented data, bot 1 did not.
        assert_eq!(s.unknown_bots(), &BTreeSet::from([BotId(2)]));
    }

    #[test]
    fn a_bot_with_a_real_player_is_not_unknown() {
        // Guards the flag in isolation from the fallback-data test above: a
        // `PlanState` built entirely from real players must report no unknown
        // bots at all, not merely "fewer than requested".
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            s.unknown_bots().is_empty(),
            "bot 1 has a real player and must not be flagged unknown"
        );
    }

    #[test]
    fn missing_bots_get_default_reach_distances() {
        let a = state();
        let bot = a.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.build_distance, 10.0);
        assert_eq!(bot.reach_distance, 10.0);
        assert_eq!(bot.resource_reach_distance, 3.0);
    }

    fn iron_ore_tile(state: &PlanState) -> Position {
        state
            .resource_patches("iron-ore")
            .first()
            .expect("fixture_world has an iron-ore patch")
            .elements
            .first()
            .expect("patch has tiles")
            .clone()
    }

    #[test]
    fn patch_tiles_come_back_sorted() {
        let a = state();
        for patch in a.resource_patches("iron-ore") {
            let mut sorted = patch.elements.clone();
            sorted.sort_by(|l, r| l.x.total_cmp(&r.x).then(l.y.total_cmp(&r.y)));
            assert_eq!(
                patch.elements, sorted,
                "every patch's tiles must come back sorted by (x, y)"
            );
        }
        // Deliberately not asserted: that two calls return the same patches.
        // They do not — see `resource_patches` for why, and why the planner
        // cannot fix it.
    }

    #[test]
    fn consuming_ore_reduces_what_is_available() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        assert!(before > 0, "fixture ore tile should hold ore");
        a.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before - 1);
    }

    #[test]
    fn consuming_more_ore_than_present_is_an_error() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let all = a.resource_available(&pos, "iron-ore");
        assert!(a.consume_resource(&pos, "iron-ore", all + 1).is_err());
        assert_eq!(a.resource_available(&pos, "iron-ore"), all);
    }

    #[test]
    fn consuming_ore_does_not_affect_the_fork_parent() {
        let a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        let mut b = a.fork();
        b.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before);
    }

    #[test]
    fn created_entities_occupy_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        assert!(a.is_position_free(&pos));
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        assert!(!a.is_position_free(&pos));
        assert_eq!(a.entity_at(&pos).unwrap().name, "stone-furnace");
    }

    #[test]
    fn removed_entities_free_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        a.remove_entity(&pos);
        assert!(a.is_position_free(&pos));
        assert!(a.entity_at(&pos).is_none());
    }

    #[test]
    fn a_tile_holding_ore_is_not_free() {
        let a = state();
        let ore = a
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|p| p.elements)
            .next()
            .expect("fixture has iron ore");
        assert!(a.resource_available(&ore, "iron-ore") > 0);
        // Resources never reach the entity tree, so `entity_at` is blind to
        // them; occupancy must still see them.
        assert!(a.entity_at(&ore).is_none());
        assert!(
            !a.is_position_free(&ore),
            "ore tile {:?} was reported free",
            ore
        );
    }

    #[test]
    fn research_defaults_to_unresearched_and_can_be_set() {
        let mut a = state();
        assert!(!a.is_researched("automation"));
        a.set_researched("automation");
        assert!(a.is_researched("automation"));
    }

    /// A reservation is a claim on stock, not a withdrawal of it. Both halves
    /// are asserted: `available` must fall, and the inventory the emitted plan
    /// is checked against must not move at all — a reservation that debited
    /// the inventory would make `Effect::LoseItem` fail on items the bot
    /// really holds.
    #[test]
    fn a_reservation_hides_stock_from_available_without_spending_it() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 10);
        let whose = Holder::Share(BotId(1));

        assert_eq!(a.available(&whose, "iron-plate"), 10);
        a.reserve(&whose, "iron-plate", 4);
        assert_eq!(a.available(&whose, "iron-plate"), 6, "four are spoken for");
        assert_eq!(
            a.inventory_count(BotId(1), "iron-plate"),
            10,
            "but none have been spent"
        );
        a.lose(BotId(1), "iron-plate", 10)
            .expect("a reservation must not block spending what is really held");

        a.release(&whose, "iron-plate", 4);
        a.gain(BotId(1), "iron-plate", 10);
        assert_eq!(a.available(&whose, "iron-plate"), 10, "and released again");
    }

    /// Reservations add up rather than overwrite: two claims on the same item
    /// hide both, which is the whole point when one recipe's ingredients each
    /// reduce to the same intermediate.
    #[test]
    fn two_reservations_on_one_item_hide_both() {
        let mut a = state();
        a.gain(BotId(1), "iron-gear-wheel", 10);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "iron-gear-wheel", 6);
        a.reserve(&whose, "iron-gear-wheel", 3);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 1);
        a.release(&whose, "iron-gear-wheel", 6);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 7);
    }

    /// The two ledgers, and the deliberate asymmetry between them. A bot's
    /// reservation names a bot, so it lowers the roster total too. An
    /// `Anyone` reservation names none, so it lowers the total and nobody's
    /// individual figure.
    #[test]
    fn a_bot_reservation_lowers_the_roster_total_but_an_anyone_reservation_lowers_no_bot() {
        let mut a = state();
        a.gain(BotId(1), "coal", 4);
        a.gain(BotId(2), "coal", 6);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 10);

        a.reserve(&Holder::Share(BotId(1)), "coal", 3);
        assert_eq!(a.available(&Holder::Share(BotId(1)), "coal"), 1);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "one bot's claim says nothing about another's stock"
        );
        assert_eq!(
            a.available(&Holder::Anyone, "coal"),
            7,
            "but it is spoken for as far as the roster is concerned"
        );

        a.reserve(&Holder::Anyone, "coal", 2);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 5);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "an Anyone claim names no bot, so it cannot be charged to one"
        );
    }

    /// Releasing more than was reserved clamps at zero rather than wrapping,
    /// which would otherwise hand a caller `u32::MAX` items of headroom.
    #[test]
    fn releasing_more_than_was_reserved_clamps_at_nothing_reserved() {
        let mut a = state();
        a.gain(BotId(1), "stone", 5);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "stone", 2);
        a.release(&whose, "stone", 9);
        assert_eq!(a.available(&whose, "stone"), 5);
    }
}
