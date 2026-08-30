use crate::error::PlannerError;
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
        for id in bots {
            let state = match base.players.get(&id.0) {
                Some(player) => BotState {
                    position: player.position.clone(),
                    inventory: player.main_inventory.clone(),
                    build_distance: player.build_distance as f64,
                    reach_distance: player.reach_distance as f64,
                    resource_reach_distance: player.resource_reach_distance as f64,
                },
                None => BotState::default(),
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
            added: Default::default(),
            removed: Default::default(),
            consumed: Default::default(),
            force,
            researched: Default::default(),
            max_prototype_half_diagonal,
        }
    }

    pub fn fork(&self) -> PlanState {
        self.clone()
    }

    pub fn base(&self) -> &Arc<FactorioWorld> {
        &self.base
    }

    /// The force this plan acts for. See the field.
    pub fn force(&self) -> Option<&str> {
        self.force.as_deref()
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

    /// Sum across every bot — the meaning of `Holder::Anyone`.
    pub fn total_count(&self, item: &str) -> u32 {
        self.bots
            .values()
            .map(|b| b.inventory.get(item).copied().unwrap_or(0))
            .sum()
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
        if entity.bounding_box.width() == 0. || entity.bounding_box.height() == 0. {
            if let Some(area) = self.collision_area(&entity.name, &entity.position) {
                entity.bounding_box = area;
            }
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
        assert!(s
            .collision_area("not-a-real-entity", &Position::new(0., 0.))
            .is_none());
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
        // three `as f64` casts here run nowhere else.
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
                resource_reach_distance: 4,
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
}
