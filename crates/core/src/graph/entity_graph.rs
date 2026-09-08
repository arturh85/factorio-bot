use crate::aabb_quadtree::{ItemId, QuadTree};
use crate::factorio::util::{
    add_to_rect, bounding_box, calculate_distance, format_dotgraph, move_position, rect_fields,
    rect_floor,
};
use crate::num_traits::FromPrimitive;
use crate::record::map::{EntitySnapshot, resource_position_from_pos};
use crate::types::{
    Direction, EntityName, EntityType, FactorioEntity, FactorioEntityPrototype,
    FactorioFluidBoxConnection, FactorioFluidBoxPrototype, FactorioRecipe, FactorioTile, Pos,
    Position, Rect, ResourcePatch, TileFluid,
};
use dashmap::DashMap;
use euclid::{Point2D, Rect as EuclidRect, Size2D};
use factorio_blueprint::{BlueprintCodec, Container};
use miette::Result;
use parking_lot::{RwLock, RwLockReadGuard};
use petgraph::dot::{Config, Dot};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Bfs, EdgeRef};
use serde::de::{MapAccess, Visitor};
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::{debug, error, warn};

/// `rect` grown by `by` tiles on every side.
///
/// Used to turn an entity's footprint into the area an edge could reach it
/// from; see [`EntityGraph::connect_nodes_near`].
fn grow_rect(rect: &Rect, by: f64) -> Rect {
    Rect::new(
        &Position::new(rect.left_top.x() - by, rect.left_top.y() - by),
        &Position::new(rect.right_bottom.x() + by, rect.right_bottom.y() + by),
    )
}

/// What [`EntityGraph::resource_mined`] did with a mine's result.
///
/// Four answers, deliberately not collapsed into an `Option<u32>`: "the model
/// has no such tile", "the model has the tile but nobody ever said how much is
/// in it" and "the tile is now empty and has been retired" are three different
/// facts, and the whole reason the resource map stores `Option<u32>` is that
/// conflating "nobody said" with a number is what sent bots to tiles holding a
/// twentieth of what the planner believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceDepletion {
    /// The model has no tile of that resource at that position, so there was
    /// nothing to debit. Mining a tree or a rock lands here, and so does a
    /// second report for a tile already retired.
    Absent,
    /// The tile is in the model, but no payload ever carried an `amount` for
    /// it, so there is no number to subtract from. Left exactly as it was --
    /// inventing a capacity in order to decrement it would be the same mistake
    /// `DEFAULT_RESOURCE_PER_TILE` already made once.
    AmountUnknown,
    /// The tile still holds ore, this much of it.
    Remaining(u32),
    /// Nothing is left, and the tile has been taken out of the model. It will
    /// not be offered to the planner again.
    Exhausted,
}

/// What [`EntityGraph::resource_fingerprint`] found: an identity for the map,
/// and the human-readable counts behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceFingerprint {
    /// FNV-1a over every charted resource tile, sorted by name and position.
    /// Equal digests mean the same map; unequal digests mean **unknown**, since
    /// charting grows as bots explore. See the method's own documentation.
    pub digest: String,
    /// How many tiles of each resource were charted, by name. This is the half
    /// a person reads: "one map had coal and the other did not" is visible here
    /// and nowhere else in a run record.
    pub tiles: BTreeMap<String, usize>,
}

/// How far the world model's knowledge reaches from the map origin, and what
/// is out there at that range.
///
/// # What this is for
///
/// The model is fed by `on_chunk_generated` (`mods/BotBridge/control.lua`),
/// which fires when the *engine* creates a chunk and never consults the
/// force's charted area. So the model learns about ground no character has
/// ever been near, for free -- in `run-1788532631-48030` the furthest any bot
/// reached was 63.8 tiles while the model it handed on held crude oil at 380
/// and 505 tiles, 559 uranium tiles and 36 biter spawners out to 500.
///
/// That is vision no player could have paid for, and until
/// [`crate::record::EventKind::VisionMeasured`] existed no run said so. This
/// is the model's half of that disclosure; the bot's half comes from the
/// sample stream. See `docs/superpowers/specs/2026-09-04-exploration-design.md`.
///
/// # What it is measured over
///
/// **Resource tiles and enemy structures**, the two things the model keys by
/// position in an ordered map. Water, terrain and built entities live in
/// quadtrees, which have no cheap extent and no stable iteration order, and
/// they would not change the answer: on the measured workspace the furthest
/// things known are oil at 505 tiles and nests at 500, both in these two maps.
/// A reader must still take this as a *lower bound* on what the model was
/// given rather than as the whole of it.
///
/// # It measures the model, not the force
///
/// **This is not the charted area.** Nothing in this process knows what the
/// force has charted -- `force.is_chunk_charted` appears nowhere in this repo
/// -- so this reports the extent of what *we were told*, which is the number
/// the disclosure is about. When the ingest becomes charted-only, this number
/// falls to the charted extent by itself and needs no change here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionExtent {
    /// Euclidean distance from the map origin `(0, 0)` to the furthest
    /// resource tile or enemy structure the model holds, in tiles.
    ///
    /// Euclidean, and computed here rather than through
    /// [`Position::distance`], which is **Manhattan** despite its name and
    /// would overstate a diagonal by up to 41%. A radius is the quantity a
    /// reader means by "how far out does this reach".
    ///
    /// From the origin rather than from spawn: spawn is within a tile or two
    /// of the origin on every freeplay map this project runs, and no run
    /// record states a spawn position, so the origin is the one datum both
    /// halves of the comparison can agree on without inventing anything.
    pub tiles: f64,
    /// What is out there -- an entity name such as `"crude-oil"` or
    /// `"biter-spawner"`. A distance alone reads as an abstraction; the name
    /// is what makes a reader look.
    pub name: String,
    /// Where it is, as the model holds it (resource keys have their half-tile
    /// centre restored, see [`resource_position_from_pos`]).
    pub position: Position,
    /// How many resource tiles the model holds in total.
    pub resource_tiles: usize,
    /// How many enemy structures the model holds in total. Counted apart from
    /// the resource tiles because a nest 500 tiles out and an ore tile 500
    /// tiles out are the same disclosure but not the same finding.
    pub enemy_structures: usize,
}

/// Euclidean distance from the map origin `(0, 0)`, in tiles.
///
/// Written out rather than taken from [`Position::distance`], which is
/// **Manhattan** -- `|dx| + |dy|` -- despite the name. That is the right
/// metric for the walking cost it was written for and the wrong one for a
/// radius: it reports a point 300 tiles diagonally out as 424 tiles away.
/// Every number this module reports as a *reach* uses this.
pub fn radius_from_origin(position: &Position) -> f64 {
    position.x().hypot(position.y())
}

pub struct EntityGraph {
    /// Bumped once by every method that changes what this graph says about the
    /// world: [`EntityGraph::add`], [`EntityGraph::remove`],
    /// [`EntityGraph::connect`] and [`EntityGraph::set_recipe`].
    /// [`EntityGraph::add_blueprint_entities`] bumps it through `add`.
    ///
    /// It exists for [`crate::graph::flow_graph::FlowGraph`], which is built
    /// *from* this graph and has no other way to learn that its answer has
    /// gone stale. Before it existed, the flow graph was walked twice in the
    /// life of a world -- at `initial discovery done` and on a `--connect`
    /// snapshot -- and never again, so every machine a run built was invisible
    /// to it and the first reader would have got the world as at tick 0, with
    /// no error and no warning. See
    /// `docs/superpowers/notes/2026-09-06-the-flow-graph-has-no-caller-and-no-refresh.md`.
    ///
    /// It counts **mutations, not versions of the content**: a mutation that
    /// changes nothing still bumps it, so a reader may rebuild for nothing.
    /// That direction is the safe one -- the other loses correctness -- and a
    /// rebuild is cheap, because the flow walk starts only from offshore pumps
    /// and drills standing on ore.
    ///
    /// Not serialised. A graph loaded from a snapshot starts at 0, and
    /// `FlowGraph`'s own counter starts at a sentinel no generation can equal,
    /// so the first read after a load rebuilds rather than trusting whatever
    /// was cached.
    generation: AtomicU64,
    entity_graph: RwLock<EntityGraphInner>,
    blocked_tree: RwLock<BlockedQuadTree>,
    entity_tree: RwLock<EntityQuadTree>,
    tile_tree: RwLock<TileQuadTree>,
    entity_nodes: DashMap<ItemId, NodeIndex>,
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    recipes: Arc<DashMap<String, FactorioRecipe>>,
    /// Every tile each named resource covers, keyed by the *floored* tile so
    /// one tile is one entry, with the ore the game says is left in it.
    ///
    /// A map keyed by tile, not a `Vec`, and that is a correctness choice
    /// rather than a performance one: the same ore tile reaches
    /// [`EntityGraph::add`] more than once (a chunk's entities are written out
    /// by both `on_chunk_generated` and the mod's `initial_discovery` replay
    /// of the chunks that already exist), and a `Vec` grew a second copy each
    /// time. The recorded run at `workspace/runs/run-1788319014-01846` held
    /// every resource exactly twice -- 900 `iron-ore` tiles against the game's
    /// 417. See `add`.
    ///
    /// # The value is how much ore is left, and `None` means nobody said
    ///
    /// `serialize_entity` (`mods/BotBridge/types.lua`) has always sent
    /// `record.amount = entity.amount` for a `type == "resource"` entity, and
    /// `FactorioEntity::amount` has always carried it in. This map used to be
    /// a `BTreeSet<Pos>` and threw it away at the door, so every consumer had
    /// to invent a number -- `crates/planner`'s `DEFAULT_RESOURCE_PER_TILE`,
    /// 500, applied to every tile on every map.
    ///
    /// Run `workspace/runs/run-1788334911-41961` is what that cost. One bot,
    /// six iron-ore tiles, six different tiles, and five of the six mines
    /// failed with `the target iron-ore was gone before mining finished --
    /// something else mined it first` after delivering 15, 6, 14, 10 and 2
    /// ore against asks of 22, 7, 50, 36 and 26. Nothing else mined them: the
    /// bot mined each tile dry and the mod's `ent.valid` check reports a
    /// vanished target with the only wording it has. The tiles held what
    /// twenty earlier runs had left in them, and the planner believed all six
    /// held 500.
    ///
    /// `None` is *not* zero and not a default: it is a tile nobody reported an
    /// amount for, which is every tile a hand-built fixture spawns
    /// (`FactorioEntity::new_resource` leaves `amount: None`). Substituting a
    /// number is the reader's decision, made once, in
    /// `PlanState::resource_available`.
    resources: DashMap<String, BTreeMap<Pos, Option<u32>>>,
    resource_tree: RwLock<ResourceQuadTree>,
    /// Every minable entity that is *not* a resource: trees and rocks, by
    /// entity name, by the tile they stand on, holding the position the game
    /// itself reported.
    ///
    /// # Why this is a third map and not a widened `resources`
    ///
    /// A resource is a *tile* with an amount that a drill can sit on and that
    /// `any_resource_at` reports as ground-with-ore. A tree is an *entity*
    /// that yields a fixed bill once and then is gone. Putting trees into
    /// `resources` would make every forest read as ore underfoot to
    /// `PlanState::stands_on_resources` and to the drill siting behind it,
    /// which is a different claim about the world than the one being made
    /// here. Admitting them to `entity_tree` instead was the other candidate
    /// and is worse: that tree feeds `find_entities_in_radius`, the entity
    /// graph's own nodes and `snapshot_within`'s keyframe comparison, so the
    /// map's ~10,500 trees would become ~10,500 petgraph nodes and a standing
    /// diff against a keyframe query that filters trees out by type.
    ///
    /// # The position is the game's, not the tile's
    ///
    /// The key is floored, as everywhere else in this struct, but the *value*
    /// is the position the mod serialised. The mod's own
    /// `surface.find_entity(name, position)` matches exactly, so handing back
    /// a floored corner would be the same half-tile fault that made every ore
    /// mine fail with "no entity to mine" -- and unlike ore, a tree is not
    /// obliged to sit on a tile centre, so there is no offset to restore it
    /// with afterwards.
    ///
    /// Two entities of the same name in one tile collapse to one entry. That
    /// under-reports what is there, which refuses work that could have been
    /// done rather than sending a bot at a tree that is not there -- the
    /// conservative direction, and the same one `resources` takes.
    minables: DashMap<String, BTreeMap<Pos, Position>>,
    /// Every standing enemy *structure* the model has been told about, by
    /// entity name, by the tile it stands on, holding the position the game
    /// reported. The sibling of `minables`, stored the same way for the same
    /// reasons.
    ///
    /// # Why this map exists at all
    ///
    /// The mod has always sent these. `writeout_entities` filters only on
    /// `ent.type ~= "character"`, so a chunk holding a nest writes out its
    /// `biter-spawner` and `small-worm-turret` records exactly like its ore.
    /// One measured run's `workspace/server-log.txt` carried **36 spawners
    /// and 28 worm turrets**, the nearest at `(-237.5, 66.5)` -- 246.6 tiles
    /// from spawn.
    ///
    /// They then died at the door. [`EntityType`] has no `unit-spawner`,
    /// `turret` or `unit` variant, so `EntityType::from_str` is `Err` for all
    /// three and [`EntityGraph::add`]'s whitelist is never consulted. What
    /// survived was one anonymous rectangle in `blocked_tree`, which can say
    /// "something is in the way" and cannot say *what*, *whose*, or where its
    /// centre is. So nothing above this layer could tell a map with no nests
    /// apart from a map whose nests were discarded on arrival -- and those two
    /// have opposite consequences for a bot sent to walk somewhere.
    ///
    /// # Structures only, deliberately
    ///
    /// Keyed on the *entity type* the mod reports, and only `"unit-spawner"`
    /// (`biter-spawner`, `spitter-spawner`) and `"turret"` (the worm turrets)
    /// are admitted. In vanilla those two types are exactly the immobile
    /// enemy buildings: a player's own turrets are `ammo-turret` /
    /// `electric-turret` / `fluid-turret` and do not match.
    ///
    /// `"unit"` -- a live biter -- is deliberately **not** stored. A unit
    /// walks, so its position is true for the tick it was serialised in and a
    /// lie thereafter, and a chunk is written out once. Recording one would
    /// leave a permanent phantom on a tile nothing is standing on, which is
    /// the failure `retire_minable` exists to undo for stumps. A nest does not
    /// move, and a nest is what a route has to give room to anyway.
    ///
    /// # This is a lower bound, and the bound is not small
    ///
    /// It holds what the model has been shown, which today means what the game
    /// happened to generate -- see
    /// `docs/superpowers/specs/2026-09-04-exploration-design.md`. A nest in
    /// ground nobody has looked at is absent from this map, and absence here is
    /// never evidence of safety.
    threats: DashMap<String, BTreeMap<Pos, Position>>,
}

/// The entity types [`EntityGraph::add`] records as enemy structures.
///
/// A `&[&str]` of the mod's own `entity.type` spellings rather than
/// [`EntityType`] variants, because the point is to catch types this crate
/// deliberately does *not* model as buildings -- admitting them to
/// [`EntityType`] would put them in `entity_tree`, the petgraph and
/// `snapshot_within`'s keyframes, which is a much larger claim than "remember
/// where the nests are".
pub const ENEMY_STRUCTURE_TYPES: [&str; 2] = ["unit-spawner", "turret"];

/// The entity types [`EntityGraph::add`] keeps **out** of `blocked_tree`,
/// alongside resources and rails: ghosts.
///
/// **A ghost does not collide.** Measured live against Factorio 2.1.17 before
/// `ActionKind::StampGhosts` existed: a real placement consumes the ghost
/// beneath it cleanly rather than being refused by it, which is exactly why
/// `PlanState::occupant_of` (`crates/planner`) skips `entity-ghost` by name in
/// both of its entity loops, unconditionally, with the note that no caller
/// should ever want a ghost to collide.
///
/// **`blocked_tree` defeated that skip.** `add` filed every entity with a
/// non-zero box into the blocked tree regardless of name, and the tree stores
/// a bare `is_minable` flag and no name -- so a ghost that reached this
/// function came back out of `blocking_boxes_within` as an anonymous
/// rectangle, and `occupant_of` reported it as
/// *"occupied by a tree, cliff, rock or unit"*. The same box reached
/// `enclosure::grid_for` and the belt router's obstacle grid, where a ghost
/// is equally not an obstacle.
///
/// # What this is NOT a claim about
///
/// This is a **latent** defect of the same shape as
/// `docs/superpowers/notes/2026-09-06-a-failed-placement-blames-a-tree.md`,
/// and it is **not** established to be that note's cause. Whether a stamped
/// ghost reaches this crate at all on the live path is contested by a
/// measurement taken the same day: nothing printed inside an RCON-invoked mod
/// function reaches stdout, so `rcon_place_blueprint`'s ghost writeouts were
/// measured arriving zero times, with a non-ghost control that also never
/// arrived (see CLAUDE.md, "Nothing printed inside an RCON-invoked mod
/// function reaches stdout"). The executor discards the ghosts
/// `place_blueprint` returns, so that reply is not a second path either.
///
/// The rule is unconditional regardless of which paths exist today: a ghost
/// does not collide, so it must not be filed as ground that blocks. What
/// stood at that tile in that run is open.
pub const GHOST_ENTITY_TYPES: [&str; 2] = ["entity-ghost", "tile-ghost"];

/// File one collision box into `blocked_tree`, unless that exact box with that
/// exact `is_minable` bit is already filed.
///
/// **Every box used to be filed twice.** The mod writes a chunk's entities out
/// from `on_chunk_generated`, and `initial_discovery` calls
/// `on_chunk_generated` again by hand for every chunk that already existed when
/// the server came up (`mods/BotBridge/control.lua`) -- `writeout_entities` is
/// unguarded on that path, unlike `writeout_tiles`, which `tile_chunks` gates.
/// So a chunk generated after the parser attached and before the discovery
/// snapshot reaches [`EntityGraph::add`] twice, and every collision box in it
/// was inserted twice. Measured on a clean seed-31337 map over one wooded
/// rectangle: **26 boxes for 13 trees**, 13 `BOTH` and 13 `DUPLICATE` with zero
/// `MODEL ONLY` (`scripts/blocked_diff.lua`; see
/// `docs/superpowers/notes/2026-09-07-ask-what-is-in-the-blocked-tree.md`).
///
/// This is the same defect [`EntityGraph::resources`] was made a `Pos`-keyed
/// map to fix, from the same two writeout paths. **A `Pos` key is not enough
/// here**, and that is the reason this is a box comparison rather than a map:
/// a blocked box is *sub-tile* and unaligned -- a tree sits at
/// `(-18.211, -99.148)..(-17.414, -98.352)` -- so several genuinely different
/// boxes share one floored tile, and a water tile's `1x1` box shares its tile
/// with nothing at all. The identity of a blocked box is the rectangle plus the
/// one bit the tree stores, so that is what is compared.
///
/// **Exact equality, and the `minable` bit is part of the key.** The two
/// filings of one entity are byte-identical `f32`, so nothing needs an epsilon;
/// widening to one would collapse *neighbouring* boxes, which is a different
/// and much worse bug. Two boxes with the same rectangle but different
/// `minable` bits are two different claims about that ground -- a rock and
/// something that is not one -- and both are kept, because
/// [`EntityGraph::blocking_boxes_within_minable`] hands that bit to callers
/// deciding whether an obstacle can be chopped.
///
/// **Not done with the quad tree's own `allow_duplicates: false`**, which
/// `entity_tree` and `tile_tree` use, and the first reason is decisive: that
/// flag does not *skip* a duplicate, it **panics** on one. `QuadNode::insert`
/// ends `panic!("didn't insert {..} into {..}")` when no node accepted the
/// item, in release as much as in debug, and `[profile.release]` sets
/// `panic = "abort"` -- so flipping the flag here would have killed the run on
/// the first replayed chunk. (`entity_tree` survives it only because `add`
/// checks `entity_at` and `continue`s before inserting; `tile_tree` survives
/// it only because the mod's `tile_chunks` guards the tile writeout. Both are
/// upstream conventions, not properties of the tree.) Two lesser reasons: the
/// flag compares rectangles with an epsilon and ignores the payload, so it
/// would collapse the two different claims above; and it is serialised with
/// the tree, so a graph loaded from a `world.dump` would keep whatever flag it
/// was written with. This check runs on every insert regardless of where the
/// tree came from.
///
/// **A dump already written is not repaired by this.** A `world.dump` taken
/// before this landed carries its duplicates inside the serialised tree;
/// deduplication happens on the way in, and there is no pass over an existing
/// tree.
fn file_blocked_box(blocked: &mut BlockedQuadTree, minable: bool, rect: QuadTreeRect) {
    // `query` is a narrowing pass -- it admits boxes that merely come close --
    // so the exact test happens here, on each candidate it hands back.
    let already_filed = blocked
        .query(rect)
        .into_iter()
        .any(|(filed_minable, filed_rect, _)| *filed_minable == minable && filed_rect == rect);
    if !already_filed {
        blocked.insert_with_box(minable, rect);
    }
}

impl EntityGraph {
    #[allow(clippy::new_without_default)]
    pub fn new(
        entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
        recipes: Arc<DashMap<String, FactorioRecipe>>,
    ) -> Self {
        let max_area = QuadTreeRect::new(Point2D::new(-5120., -5120.), Size2D::new(10240., 10240.));
        EntityGraph {
            entity_prototypes,
            recipes,
            generation: AtomicU64::new(0),
            entity_graph: RwLock::new(EntityGraphInner::new()),
            entity_tree: RwLock::new(QuadTree::new(max_area, false, 32, 128, 128, 8)),
            blocked_tree: RwLock::new(QuadTree::new(max_area, true, 8, 64, 1024, 8)),
            resource_tree: RwLock::new(QuadTree::new(max_area, true, 8, 64, 1024, 8)),
            tile_tree: RwLock::new(QuadTree::new(max_area, false, 32, 128, 128, 8)),
            entity_nodes: DashMap::new(),
            resources: DashMap::new(),
            minables: DashMap::new(),
            threats: DashMap::new(),
        }
    }
    /// How many times this graph has been mutated. See [`EntityGraph`]'s
    /// `generation` field for what it is for and what it does *not* promise.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Called by every mutating method, at the point the mutation is complete.
    fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn inner_graph(&self) -> RwLockReadGuard<'_, EntityGraphInner> {
        self.entity_graph.read()
    }
    pub fn inner_tree(&self) -> RwLockReadGuard<'_, EntityQuadTree> {
        self.entity_tree.read()
    }
    pub fn tile_tree(&self) -> RwLockReadGuard<'_, TileQuadTree> {
        self.tile_tree.read()
    }
    pub fn blocked_tree(&self) -> RwLockReadGuard<'_, BlockedQuadTree> {
        self.blocked_tree.read()
    }
    pub fn resource_tree(&self) -> RwLockReadGuard<'_, ResourceQuadTree> {
        self.resource_tree.read()
    }
    pub fn entity_prototypes(&self) -> Arc<DashMap<String, FactorioEntityPrototype>> {
        self.entity_prototypes.clone()
    }
    pub fn recipes(&self) -> Arc<DashMap<String, FactorioRecipe>> {
        self.recipes.clone()
    }

    pub fn node_by_id(&self, id: &ItemId) -> Option<NodeIndex> {
        self.entity_nodes.get(id).map(|e| *e.value())
    }

    /// Which resources are **charted on this surface**, in name order.
    ///
    /// # Present, not declared
    ///
    /// The discriminator [`crate::factorio::world::GameGlobals`]'s prototype
    /// table cannot provide. Every `entity_type == "resource"` prototype the
    /// game knows is in that table on every surface -- this mod set declares
    /// twelve, `calcite`, `scrap`, `tungsten-ore`, `lithium-brine`,
    /// `fluorine-vent` and `sulfuric-acid-geyser` among them -- and a Nauvis
    /// map has five of them. `PlanState::resource_names` answers the declared
    /// set and is right to; this answers the charted one.
    ///
    /// **Empty is "nothing charted", which is not "nothing here".** A world
    /// attached from a snapshot, or a fresh map nobody has walked, charts no
    /// resources at all, and a caller reading this as "this surface has no
    /// ore" would be asserting something nobody established. The one caller
    /// today -- `crates/planner`'s supply closure -- treats an empty answer as
    /// *unknown* and falls back to admitting every candidate, which is the
    /// safe direction: it refuses to choose rather than choosing wrongly.
    #[must_use]
    pub fn resource_names_present(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .resources
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        names.sort();
        names
    }

    pub fn resource_contains(&self, resource_name: &str, pos: Pos) -> bool {
        let elements = self.resources.get(resource_name);
        if let Some(elements) = elements {
            elements.contains_key(&pos)
        } else {
            false
        }
    }

    /// How much of `resource_name` the game last said is left in `pos`, or
    /// `None` when nobody has said.
    ///
    /// Three answers collapse into `None` and a caller must not tell them
    /// apart here, because the honest answer to all three is the same: there
    /// is no tile of that name at `pos`, or there is one and the payload that
    /// delivered it carried no `amount`. Neither is "the tile is empty" -- an
    /// empty resource entity does not exist -- the game destroys it, and
    /// [`EntityGraph::resource_mined`] retires the tile here when what a mine
    /// took empties it. (This used to name an `on_resource_depleted` that has
    /// never existed in the mod or in this crate, which is how a mined-out tile
    /// came to stay in the model for a whole run.) Use
    /// [`EntityGraph::resource_contains`] to ask whether the tile is there at
    /// all; use this to ask what it holds.
    ///
    /// What to do with `None` is the reader's decision. `crates/planner`'s
    /// `PlanState::resource_available` substitutes `DEFAULT_RESOURCE_PER_TILE`,
    /// which is the fixture fallback and nothing more -- see the
    /// [`resources`](EntityGraph#structfield.resources) field for the run that
    /// established the difference between a reported amount and a modelled
    /// one.
    pub fn resource_amount(&self, resource_name: &str, pos: &Pos) -> Option<u32> {
        self.resources
            .get(resource_name)
            .and_then(|elements| elements.get(pos).copied().flatten())
    }

    /// Takes `mined` units out of the resource tile under `position`, and
    /// **retires the tile** once the model says nothing is left.
    ///
    /// # Why the model has to be told at all
    ///
    /// Nothing else tells it. The mod reports a tile's `amount` when the chunk
    /// it lives in is written out and never again, and it emits no event when a
    /// tile is mined dry -- `mined_item` is commented out in `control.lua`, and
    /// the `on_resource_depleted` this file's [`EntityGraph::resource_amount`]
    /// once claimed does the retiring has never existed. So a tile a bot mined
    /// to zero stayed in `resources` at its pre-run reading, kept turning up in
    /// `resource_patches`, and the planner kept sending a bot back to walk to
    /// ore that was not there.
    ///
    /// # The position is a tile centre
    ///
    /// `position` is the real entity position -- `(-40.5, -48.5)`, never
    /// `(-41, -49)` -- exactly as it was handed to
    /// [`FactorioRcon::player_mine_timed`](crate::factorio::rcon::FactorioRcon::player_mine_timed).
    /// The `Pos` key is derived here by flooring, the same way `add` derived
    /// it, and the retirement rebuilds the centre rather than reusing the
    /// floored key. A caller that floors first and passes the corner will miss
    /// the tile in the quad tree.
    ///
    /// # `mined` is what the game took, not what was asked for
    ///
    /// A mine that fails part-way has not removed what it was asked for, so a
    /// caller must pass the count only once the game has said the mine
    /// succeeded. When the game instead reports the target vanished mid-mine,
    /// [`EntityGraph::retire_resource`] is the honest call: the entity is gone
    /// whatever arithmetic says.
    pub fn resource_mined(
        &self,
        resource_name: &str,
        position: &Position,
        mined: u32,
    ) -> ResourceDepletion {
        let pos: Pos = position.into();
        // Both guards are dropped before anything below takes the map again:
        // `retire_resource` needs a `get_mut` on the same shard, and holding a
        // read guard across it deadlocks the calling task.
        let stored = self
            .resources
            .get(resource_name)
            .and_then(|tiles| tiles.get(&pos).copied());
        let Some(stored) = stored else {
            return ResourceDepletion::Absent;
        };
        let Some(amount) = stored else {
            return ResourceDepletion::AmountUnknown;
        };
        let left = amount.saturating_sub(mined);
        if left == 0 {
            self.retire_resource(resource_name, position);
            return ResourceDepletion::Exhausted;
        }
        if let Some(mut tiles) = self.resources.get_mut(resource_name) {
            tiles.insert(pos, Some(left));
        }
        ResourceDepletion::Remaining(left)
    }

    /// Takes a resource tile out of the model entirely: out of `resources`, and
    /// out of `resource_tree` with it. Answers whether there was one to take.
    ///
    /// This is [`EntityGraph::remove`] under a name that says what it is for
    /// and a signature a caller who only knows *what* was mined and *where* can
    /// actually reach. `position` is the tile centre, as
    /// [`EntityGraph::resource_mined`] explains; the entity handed to `remove`
    /// is rebuilt on the centre of the tile the key names, so passing a
    /// position anywhere inside the tile retires that tile and only that tile.
    pub fn retire_resource(&self, resource_name: &str, position: &Position) -> bool {
        let pos: Pos = position.into();
        if !self.resource_contains(resource_name, pos.clone()) {
            return false;
        }
        let centre = resource_position_from_pos(pos);
        let entity = FactorioEntity::new_resource(&centre, Direction::North, resource_name);
        if let Err(err) = self.remove(&entity) {
            warn!("failed to retire mined-out {resource_name} at {centre:?}: {err}");
            return false;
        }
        true
    }

    /// Every position at which a minable entity called `entity_name` stands,
    /// in tile order.
    ///
    /// Ordered because the planner picks from this and its output has to be
    /// byte-identical across runs; the order is the `BTreeMap`'s, i.e. the
    /// data's, not a hash seed's.
    pub fn minable_positions(&self, entity_name: &str) -> Vec<Position> {
        self.minables
            .get(entity_name)
            .map(|tiles| tiles.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Which minable entities in the model yield `item`, and how much each
    /// one yields, in entity-name order.
    ///
    /// Read from the prototype's own `mine_result` -- the game's answer to
    /// "what does mining this give you" -- rather than from a table here. An
    /// entity the model holds but has no prototype for yields nothing, which
    /// refuses the work rather than guessing a bill.
    ///
    /// Name order, not discovery order: the backing map is a `DashMap`, whose
    /// iteration order is a hash seed's and would otherwise reach the planner.
    pub fn minables_yielding(&self, item: &str) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = self
            .minables
            .iter()
            .filter(|entry| !entry.value().is_empty())
            .filter_map(|entry| {
                let name = entry.key().clone();
                let yields = self
                    .entity_prototypes
                    .get(&name)?
                    .mine_result
                    .as_ref()?
                    .get(item)
                    .copied()?;
                (yields > 0).then_some((name, yields))
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// Every enemy structure the model knows of, nearest first, as
    /// `(name, position, distance)` measured from `from`.
    ///
    /// **Euclidean**, via [`calculate_distance`] — deliberately not
    /// [`Position::manhattan_distance`], which is what this used at first. A
    /// nest 200 tiles away diagonally reads as 283 under Manhattan, i.e.
    /// **further than it is**, and callers use this to decide whether
    /// somewhere is safe to walk to. That is the dangerous direction to be
    /// wrong in.
    ///
    /// Ordered by distance and then by `(x, y)`, never by the backing
    /// `DashMap`'s iteration order, for the same reason
    /// [`EntityGraph::minables_yielding`] sorts: a hash seed must not reach a
    /// planner whose output has to be byte-identical across runs. Floats are
    /// compared with `total_cmp`.
    ///
    /// # An empty answer means "none charted", not "none there"
    ///
    /// This reads the model, and the model holds what it has been shown.
    /// Callers deciding whether somewhere is safe to walk to must treat an
    /// empty result as *unknown*; see the `threats` field's own docs.
    pub fn threats_from(&self, from: &Position) -> Vec<(String, Position, f64)> {
        let mut out: Vec<(String, Position, f64)> = self
            .threats
            .iter()
            .flat_map(|entry| {
                let name = entry.key().clone();
                entry
                    .value()
                    .values()
                    .map(|pos| (name.clone(), pos.clone(), calculate_distance(from, pos)))
                    .collect::<Vec<_>>()
            })
            .collect();
        out.sort_by(|a, b| {
            a.2.total_cmp(&b.2)
                .then(a.1.x().total_cmp(&b.1.x()))
                .then(a.1.y().total_cmp(&b.1.y()))
                .then(a.0.cmp(&b.0))
        });
        out
    }

    /// The nearest charted enemy structure to `from`, or `None` when the model
    /// holds none. See [`EntityGraph::threats_from`] for what `None` does and
    /// does not establish.
    pub fn nearest_threat(&self, from: &Position) -> Option<(String, Position, f64)> {
        self.threats_from(from).into_iter().next()
    }

    /// How many enemy structures of each name the model holds, in name order.
    ///
    /// The census half, the counterpart of
    /// [`ResourceFingerprint::tiles`]: "this map has nests and that one does
    /// not" is the sentence a person reads, and nothing else in a run record
    /// says it.
    pub fn threat_census(&self) -> BTreeMap<String, usize> {
        self.threats
            .iter()
            .filter(|entry| !entry.value().is_empty())
            .map(|entry| (entry.key().clone(), entry.value().len()))
            .collect()
    }

    /// How far out the model's knowledge reaches. See [`VisionExtent`].
    ///
    /// `None` when the model holds neither a resource tile nor an enemy
    /// structure -- a world nobody has read yet, which is a different fact
    /// from a model that reaches zero tiles. Callers must keep the two apart;
    /// reporting an unread model as `0.0` would say a run was given no free
    /// vision when nothing had looked.
    ///
    /// Deterministic: the backing maps are `DashMap`s, whose iteration order
    /// is seeded per process, so ties are broken on position and then name
    /// rather than on whichever entry came out first. Two runs on one map
    /// therefore report the same extent, which is the whole point of a number
    /// meant to be compared.
    pub fn vision_extent(&self) -> Option<VisionExtent> {
        let mut resource_tiles = 0usize;
        let mut enemy_structures = 0usize;
        let mut furthest: Option<(f64, Position, String)> = None;
        let mut consider = |tiles: f64, position: Position, name: &str| {
            let better = match &furthest {
                None => true,
                Some((best, best_pos, best_name)) => matches!(
                    tiles
                        .total_cmp(best)
                        .then(position.x().total_cmp(&best_pos.x()))
                        .then(position.y().total_cmp(&best_pos.y()))
                        .then(name.cmp(best_name.as_str())),
                    std::cmp::Ordering::Greater
                ),
            };
            if better {
                furthest = Some((tiles, position, name.to_string()));
            }
        };
        for entry in self.resources.iter() {
            resource_tiles += entry.value().len();
            for pos in entry.value().keys() {
                let position = resource_position_from_pos(pos.clone());
                consider(radius_from_origin(&position), position, entry.key());
            }
        }
        for entry in self.threats.iter() {
            enemy_structures += entry.value().len();
            for position in entry.value().values() {
                consider(radius_from_origin(position), position.clone(), entry.key());
            }
        }
        let (tiles, position, name) = furthest?;
        Some(VisionExtent {
            tiles,
            name,
            position,
            resource_tiles,
            enemy_structures,
        })
    }

    /// Takes one mined-out tree or rock out of the model. Answers whether
    /// there was one to take.
    ///
    /// The sibling of [`EntityGraph::retire_resource`], and needed for the
    /// same reason: nothing else removes it. A tree is destroyed by the swing
    /// that mines it and the mod emits no event for it, so without this the
    /// planner keeps offering a stump, the bot walks there, and
    /// `surface.find_entity` answers nil -- "Error: no entity to mine" for a
    /// tree that really was there when the plan was made.
    ///
    /// Unlike a resource tile there is no partial state to weigh: one swing
    /// takes the whole entity, so there is no `mined` count and no
    /// [`ResourceDepletion`] to report.
    pub fn retire_minable(&self, entity_name: &str, position: &Position) -> bool {
        let pos: Pos = position.into();
        // Cloned out and the guard dropped before `remove` runs: `remove`
        // takes `get_mut` on this same map, and holding a read guard across it
        // deadlocks the calling task. The same trap `resource_mined` documents.
        let centre = self
            .minables
            .get(entity_name)
            .and_then(|tiles| tiles.get(&pos).cloned());
        let Some(centre) = centre else {
            return false;
        };
        // The prototype's own collision box, so the rectangle handed to
        // `remove` is the one `add` put into `blocked_tree`. A guessed box that
        // is too small leaves the stump blocking placements for ever.
        let Some(collision) = self
            .entity_prototypes
            .get(entity_name)
            .map(|proto| proto.collision_box.clone())
        else {
            warn!("cannot retire minable {entity_name} at {centre}: no prototype for it");
            return false;
        };
        let entity = FactorioEntity {
            name: entity_name.to_string(),
            position: centre.clone(),
            bounding_box: add_to_rect(&collision, &centre),
            ..Default::default()
        };
        if let Err(err) = self.remove(&entity) {
            warn!("failed to retire mined {entity_name} at {centre}: {err}");
            return false;
        }
        true
    }

    /// Whether a resource of *any* name covers `pos`.
    ///
    /// `resource_contains` answers only for one named ore, which is no help to
    /// a caller asking whether a tile is buildable: it would have to guess the
    /// ore. Read-only and allocation-free — a short-circuiting scan of the same
    /// `resources` map `resource_contains` indexes into.
    pub fn any_resource_at(&self, pos: &Pos) -> bool {
        self.resources
            .iter()
            .any(|entry| entry.value().contains_key(pos))
    }

    pub fn find_entities_in_radius(
        &self,
        search_center: Position,
        radius: f64,
        search_name: Option<String>,
        search_type: Option<String>,
    ) -> Vec<FactorioEntity> {
        let tree = self.entity_tree.read();
        let rect = QuadTreeRect::new(
            Position::new(search_center.x - radius, search_center.y - radius).into(),
            Size2D::new(2. * radius as f32, 2. * radius as f32),
        );
        let mut entities = vec![];
        for (entity, _rect, _item_id) in tree.query(rect) {
            if let Some(search_name) = search_name.as_ref()
                && entity.name != *search_name
            {
                continue;
            }
            if let Some(search_type) = search_type.as_ref()
                && entity.entity_type != *search_type
            {
                continue;
            }
            // Euclidean, via `calculate_distance`. `Position::distance` is
            // **Manhattan** despite the name -- `|dx| + |dy|` -- and this line
            // called it until 2026-09-04, so "radius" here meant an L1 diamond
            // while the name, this function's Lua doc ("searches in circular
            // radius"), every planner caller and the game's own
            // `find_entities_filtered` all mean a disc. The diamond reaches
            // the full radius along the axes and only `radius / sqrt(2)`
            // diagonally.
            //
            // Run `run-1788504490-09380` is what that cost. Rung 3 asked for a
            // green cell "on the plant red already stood up";
            // `method::power::supply_for` looks for a standing pole within
            // `PLANT_ADOPT_RADIUS` (256) before it will build a second plant,
            // and the bot it expanded from stood at [142.3, -187.3] with the
            // plant's pole at [10.5, -41.5] -- 196.5 tiles away, comfortably
            // inside 256, but `131.8 + 145.8 = 277.6` outside the diamond. The
            // pole was therefore invisible, adoption answered `None` at both
            // tiers, and `plan_plant` refused from the bot's own position with
            // `PowerPlantNeedsWater` -- an error naming water, on a map whose
            // water is 46.7 tiles from spawn and already had a working plant
            // on it.
            //
            // The same mismatch made `PlanState::entities_within` disagree with
            // itself (it filters its own overlay with `calculate_distance`, so
            // plan-placed entities got a disc and world entities a diamond),
            // and invalidated `PlanState::is_area_clear_of`'s stated safety
            // argument, which reasons "by the triangle inequality for the
            // Euclidean norm" about a radius this line was narrowing.
            if calculate_distance(&entity.position, &search_center) > radius {
                continue;
            }
            entities.push(entity.clone())
        }
        entities
    }

    /// Every collision box inside `bounds` that would make the game refuse a
    /// player build there, as world-space rectangles.
    ///
    /// [`Self::find_entities_in_radius`] cannot answer this and never could.
    /// `add` inserts only a *whitelist* of entity types into `entity_tree` --
    /// furnaces, inserters, belts, containers and the two big rocks -- because
    /// that tree exists to model a factory, not the ground it stands on. Trees,
    /// small rocks, cliffs and units therefore never enter it, and neither do
    /// tiles; a caller asking "does a stone furnace fit here" and reading
    /// `entity_tree` gets "yes" over a forest. In the recorded run at
    /// `workspace/runs/run-1788309767-54739` that is exactly what happened:
    /// the planner sited a furnace, the game answered `can_place_entity said
    /// 'no'`, and the run stuck on its first dispatched action.
    ///
    /// `blocked_tree` is the tree that does see them. `add` puts every entity
    /// with a non-zero collision box into it except resources, rails (ore
    /// and rails are asked about separately, by tile) and ghosts (which do not
    /// collide at all -- see [`GHOST_ENTITY_TYPES`]), and `add_tiles` adds
    /// every `player_collidable` tile -- water. So this is the ground truth for
    /// buildability that the graph already had and nothing but `draw.rs` was
    /// reading.
    ///
    /// Boxes, not entities: `blocked_tree`'s payload is a bare `is_minable`
    /// flag, so there is no name or position to hand back. The rectangle is
    /// what the question needs. Results are ordered by the quad-tree's own
    /// item ids (see `QuadTree::query`), so the vector is deterministic for a
    /// given tree state.
    ///
    /// The quad-tree query is a *narrowing* pass -- it admits boxes that merely
    /// come close, by its own epsilon -- so a caller deciding overlap must
    /// still test each rectangle exactly.
    ///
    /// Edges come back snapped to Factorio's own 1/256-of-a-tile position
    /// grid. The quad-tree stores its rectangles as `f32`, which perturbs every
    /// edge by a few ulps; a caller that reads the *centre* of the recovered
    /// box -- to key it by tile, say -- would see a centre a hair below the
    /// integer it should be and floor it into the neighbouring tile. Every
    /// `MapPosition` the game reports is an exact multiple of 1/256, and `f32`
    /// has far more precision than that at map coordinates, so rounding to
    /// that grid recovers the exact edge rather than approximating it.
    pub fn blocking_boxes_within(&self, bounds: &Rect) -> Vec<Rect> {
        self.blocking_boxes_within_minable(bounds)
            .into_iter()
            .map(|(rect, _minable)| rect)
            .collect()
    }

    /// [`Self::blocking_boxes_within`], keeping the one bit the tree stores.
    ///
    /// `blocked_tree`'s payload is a bare `is_minable` flag
    /// ([`FactorioEntity::is_minable`]: the entity's type is `tree` or
    /// `simple-entity`), and `blocking_boxes_within` throws it away. That is
    /// the whole reason a refusal built on these boxes could only say
    /// *"a tree, cliff, rock or unit"* -- four different things, one of which
    /// it names first and none of which it read.
    ///
    /// The flag does not name the obstacle and this does not pretend it
    /// does. It splits the boxes in two, honestly: `true` is a tree or a
    /// rock, which a bot could in principle mine out of the way; `false` is
    /// **anything else with a collision box that the entity tree does not
    /// hold** -- a cliff, a unit, a water tile, a corpse, an item on the
    /// ground. A caller that wants to say what it found says the first and
    /// admits the second, rather than reciting a list it did not read.
    ///
    /// Same ordering, same snapping and the same narrowing-pass caveat as
    /// [`Self::blocking_boxes_within`], which is now written in terms of this.
    pub fn blocking_boxes_within_minable(&self, bounds: &Rect) -> Vec<(Rect, bool)> {
        /// Factorio stores map positions as fixed point with this denominator.
        const POSITION_GRID: f64 = 256.;
        fn snap(v: f32) -> f64 {
            (v as f64 * POSITION_GRID).round() / POSITION_GRID
        }
        let query: QuadTreeRect = bounds.clone().into();
        self.blocked_tree
            .read()
            .query(query)
            .into_iter()
            .map(|(minable, rect, _id)| {
                (
                    Rect::new(
                        &Position::new(snap(rect.origin.x), snap(rect.origin.y)),
                        &Position::new(
                            snap(rect.origin.x + rect.size.width),
                            snap(rect.origin.y + rect.size.height),
                        ),
                    ),
                    *minable,
                )
            })
            .collect()
    }

    /// Every tile this graph has been told about inside `bounds`, **by name**,
    /// ordered by `(x, y, name)`.
    ///
    /// The tile tree has always carried the name -- `add_tiles` stores whole
    /// `FactorioTile`s -- and until now nothing could read it. The only public
    /// accessor was [`Self::tile_tree`], a raw `RwLockReadGuard` over a quad
    /// tree, whose one caller repo-wide was a test. So a caller asking "is
    /// there water here" had to go through [`Self::blocking_boxes_within`]
    /// instead, which answers a *different* question: its payload is a bare
    /// `is_minable` flag, so water arrives anonymous and indistinguishable
    /// from a tree or a cliff. An offshore pump needs to know a tile **is
    /// water**, not that something blocks there, and a boiler needs the exact
    /// opposite; one bit cannot carry both.
    ///
    /// # Ordering, because the planner reads this
    ///
    /// The quad tree hands its results back in item-id order, i.e. the order
    /// chunks happened to arrive in, which differs between two runs of the
    /// same map. `crates/planner` is pure and deterministic and must produce
    /// byte-identical plans for identical inputs, so the vector is sorted
    /// explicitly on `(x, y, name)` with `total_cmp` -- the same convention
    /// and the same comparator `PlanState::entities_within` already uses.
    ///
    /// # Clipped to `bounds`, unlike the obstruction queries
    ///
    /// Results are re-checked with [`overlaps_bounds`], for the reason
    /// [`Self::snapshot_within`] sets out at length: the quad tree's own
    /// predicate is half-open, so a box abutting the query's *left or top*
    /// edge comes back while its mirror image on the right or bottom does not.
    /// A tile box is a full 1x1, so without the clip the row of tiles
    /// immediately left of `bounds` is reported as inside it -- the same
    /// asymmetry that was worth 691 spurious keyframe divergences.
    ///
    /// This is not the narrowing that `1b2b2149` warns against. That commit
    /// filtered the keyframe query and deliberately left `attach_world`,
    /// `is_area_empty` and the placement obstruction checks wide, because a
    /// *negative* question ("is anything in the way?") is safe when it
    /// over-reports and dangerous when it under-reports. This is a
    /// **positive** question ("where is the water?"), where over-reporting is
    /// the unsafe direction: it would put a lake one tile outside every rect
    /// anybody asks about.
    pub fn tiles_within(&self, bounds: &Rect) -> Vec<FactorioTile> {
        let query: QuadTreeRect = bounds.clone().into();
        let mut out: Vec<FactorioTile> = self
            .tile_tree
            .read()
            .query(query)
            .into_iter()
            .filter(|(_tile, rect, _id)| overlaps_bounds(bounds, rect))
            .map(|(tile, _rect, _id)| tile.clone())
            .collect();
        out.sort_by(|a, b| {
            a.position
                .x
                .total_cmp(&b.position.x)
                .then(a.position.y.total_cmp(&b.position.y))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    /// Whether the tile covering `position` is water.
    ///
    /// The discriminator [`Self::blocking_boxes_within`] cannot provide. Its
    /// rectangles carry no name, but a tile's box is exactly the 1x1 square of
    /// the tile it came from, so `is_water_at(&rect.center())` names it --
    /// keyed by the same floored `Pos` that `PlanState::is_area_clear` already
    /// uses on those very boxes.
    ///
    /// `position` is a point anywhere in the tile, not the tile's corner.
    ///
    /// # It asks the ground first and the name second
    ///
    /// The test is [`FactorioTile::yields_water`], not the bare name pair:
    /// a charted tile is judged by the fluid its prototype names, and only a
    /// tile whose sender never filled `fluid` in falls back to
    /// [`FactorioTile::WATER_NAMES`]. See [`Self::fluid_at`] for the three
    /// answers and `yields_water` for why the fallback may not be dropped.
    #[must_use]
    pub fn is_water_at(&self, position: &Position) -> bool {
        let pos = Pos::from(position);
        // A quarter-tile box strictly inside the tile: small enough that no
        // neighbour's box can reach it, and non-degenerate so the tree's
        // half-open `contains` still admits the tile it is inside of. The
        // `Pos` comparison below is what actually decides -- this only narrows.
        let query: QuadTreeRect = Rect::new(
            &Position::new(f64::from(pos.0) + 0.25, f64::from(pos.1) + 0.25),
            &Position::new(f64::from(pos.0) + 0.75, f64::from(pos.1) + 0.75),
        )
        .into();
        self.tile_tree
            .read()
            .query(query)
            .into_iter()
            .any(|(tile, _rect, _id)| Pos::from(&tile.position) == pos && tile.yields_water())
    }

    /// Which fluid an offshore pump standing on the tile covering `position`
    /// would draw, as the tile's **prototype** says.
    ///
    /// The successor to [`Self::is_water_at`], and not a synonym for it.
    /// `is_water_at` tests the tile's *name* against a hard-coded pair read
    /// off a vanilla capture; this install runs Space Age, where
    /// `ammoniacal-ocean` yields ammonia and Vulcanus' lava yields lava, and
    /// neither name is in that pair. Asking what the ground gives rather than
    /// what it is called is what makes the answer survive a mod -- the same
    /// argument that replaced this repo's hard-coded smelting rate.
    ///
    /// # Three answers, and a tile nobody charted is not a dry one
    ///
    /// [`TileFluid::Unknown`] is returned both when the tile carries that
    /// answer and when **there is no tile here at all** -- unexplored ground,
    /// or a chunk written by a mod predating the field. That collapse is
    /// deliberate and is the *safe* direction: both genuinely mean "we could
    /// not tell", and the alternative -- reporting unexplored ground as
    /// [`TileFluid::Dry`] -- would let a caller refuse a lake it has simply
    /// never walked to. What must never happen is the other way round, and it
    /// cannot: a `Dry` here is always a charted tile whose prototype named no
    /// fluid.
    ///
    /// `position` is a point anywhere in the tile, not the tile's corner.
    #[must_use]
    pub fn fluid_at(&self, position: &Position) -> TileFluid {
        let pos = Pos::from(position);
        // The same quarter-tile narrowing query `is_water_at` uses, for the
        // same reason: the `Pos` comparison below is what decides.
        let query: QuadTreeRect = Rect::new(
            &Position::new(f64::from(pos.0) + 0.25, f64::from(pos.1) + 0.25),
            &Position::new(f64::from(pos.0) + 0.75, f64::from(pos.1) + 0.75),
        )
        .into();
        self.tile_tree
            .read()
            .query(query)
            .into_iter()
            .find(|(tile, _rect, _id)| Pos::from(&tile.position) == pos)
            .map_or(TileFluid::Unknown, |(tile, _rect, _id)| tile.fluid.clone())
    }

    /// The water tile nearest `from`, or `None` if there is none within
    /// `max_radius`.
    ///
    /// Where a plant gets sited. A boiler and a steam engine have to stand
    /// next to a lake because the water is the one input that cannot be
    /// carried, so "how far is the water" is the question a plant method asks
    /// first and the one that a refusal ("the nearest water is 300 tiles from
    /// the nearest coal") is worth making on.
    ///
    /// # Three things that are easy to get wrong here
    ///
    /// * **Distance is Euclidean**, via
    ///   [`calculate_distance`](crate::factorio::util::calculate_distance).
    ///   `Position::distance` is *Manhattan* despite the name -- it is
    ///   `|dx| + |dy|`. This note used to add that
    ///   [`Self::find_entities_in_radius`] used it too, so that two "radius"
    ///   arguments in this file did not mean the same thing; that stopped
    ///   being true on 2026-09-04, when the L1 filter there was found refusing
    ///   to adopt a standing power plant 196 tiles away on the diagonal. Both
    ///   radii are now discs. `Position::distance` itself is still Manhattan
    ///   and is still the wrong function to reach for by name.
    /// * **Distance is measured to the tile's centre**, `position + (0.5,
    ///   0.5)`, because `FactorioTile::position` is the tile's top-left
    ///   *corner* -- that is what the game reports and what `add_tiles`
    ///   assumes when it builds the 1x1 box. The returned tile keeps its
    ///   corner position, unchanged, so this is the only place the half tile
    ///   appears. Getting the same half-tile wrong for resources made mining
    ///   fail on every ore on every map.
    /// * **Water is what the ground yields, not what it is called.** The
    ///   filter is [`FactorioTile::yields_water`]: a charted tile is judged by
    ///   `LuaTilePrototype::fluid`, and the vanilla name pair
    ///   ([`FactorioTile::WATER_NAMES`], where `deepwater` outnumbers `water`
    ///   four to one in the archived stdout) is consulted only for a tile
    ///   whose sender never said. Every archived dump is such a sender, which
    ///   is why the fallback exists at all.
    ///
    /// # Determinism
    ///
    /// Candidates are ordered by `(distance, x, y)` with `total_cmp` and the
    /// first is taken, so two tiles equally far away resolve by position and
    /// never by which chunk arrived first. Same shape as
    /// `PlanState::nearest_supply_anchor`.
    ///
    /// The search is one bounded query over the `max_radius` box, not an
    /// expanding ring: a ring search that stops at the first ring holding a
    /// hit does not return the nearest tile (a hit in the corner of ring `r`
    /// is `r * sqrt(2)` away, further than any tile in ring `r + 1`), and
    /// making it correct means re-querying anyway. Cost is therefore linear in
    /// the tiles inside `max_radius`, which is the caller's to bound.
    #[must_use]
    pub fn nearest_water_tile(&self, from: &Position, max_radius: f64) -> Option<FactorioTile> {
        let bounds = Rect::new(
            &Position::new(from.x() - max_radius, from.y() - max_radius),
            &Position::new(from.x() + max_radius, from.y() + max_radius),
        );
        let mut candidates: Vec<(f64, FactorioTile)> = self
            .tiles_within(&bounds)
            .into_iter()
            .filter(FactorioTile::yields_water)
            .filter_map(|tile| {
                let centre = Position::new(tile.position.x() + 0.5, tile.position.y() + 0.5);
                let distance = calculate_distance(&centre, from);
                (distance <= max_radius).then_some((distance, tile))
            })
            .collect();
        candidates.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.position.x.total_cmp(&b.1.position.x))
                .then(a.1.position.y.total_cmp(&b.1.position.y))
        });
        candidates.into_iter().next().map(|(_, tile)| tile)
    }

    /// Everything this graph believes lies within `bounds`: the entities the
    /// entity tree tracks (see `add`), plus resources.
    ///
    /// Resources are read out of `resource_tree` rather than `entity_tree`
    /// (they never enter it -- see `add`), and their position is recovered
    /// through [`resource_position_from_pos`]: the box each one was inserted
    /// under has its origin at the *floored* tile, same convention as the
    /// `resources` map itself, and a caller reading that origin back out
    /// without restoring the half tile would put every resource 0.5 off from
    /// where the game actually has it.
    ///
    /// # The quad tree answers a slightly wider question than it was asked
    ///
    /// Both queries are re-checked against `bounds` with [`overlaps_bounds`]
    /// before anything is reported. `my_intersects` (`crate::aabb_quadtree`)
    /// falls back on `euclid`'s `Rect::contains`, which is half-open --
    /// `min <= p < max` -- so a box whose *maximum* corner lands exactly on the
    /// query's left or top edge is returned, while one whose minimum corner
    /// lands on the right or bottom edge is not. A resource is stored under a
    /// full 1x1 tile box, so the tile immediately left of `bounds.left` abuts
    /// the boundary line and came back from a query it is entirely outside of.
    ///
    /// That is not a rounding question and not a resource-specific one: it was
    /// worth 691 spurious `only_in: "model"` entries across the archived runs
    /// in `workspace/runs/`, every single one of them on the left or top edge
    /// and none on the right or bottom -- exactly the asymmetry the half-open
    /// `contains` predicts. The keyframe is the only caller, so this clip
    /// changes what the *diagnostic* claims and nothing the planner reads;
    /// leaving it would go on reporting a divergence that is not one, and a
    /// diagnostic nobody believes is worse than no diagnostic.
    pub fn snapshot_within(&self, bounds: &Rect) -> Vec<EntitySnapshot> {
        let mut out = Vec::new();
        let entity_query: QuadTreeRect = bounds.clone().into();
        for (entity, rect, _id) in self.entity_tree.read().query(entity_query) {
            if !overlaps_bounds(bounds, &rect) {
                continue;
            }
            out.push(EntitySnapshot {
                name: entity.name.clone(),
                position: entity.position.clone(),
                direction: entity.direction,
            });
        }
        let resource_query: QuadTreeRect = bounds.clone().into();
        for (name, rect, _id) in self.resource_tree.read().query(resource_query) {
            if !overlaps_bounds(bounds, &rect) {
                continue;
            }
            let pos = Pos(rect.origin.x as i32, rect.origin.y as i32);
            out.push(EntitySnapshot {
                name: name.clone(),
                position: resource_position_from_pos(pos),
                direction: 0,
            });
        }
        out
    }

    /// Does the world hold any tile of `resource_name` at all?
    ///
    /// [`EntityGraph::resource_patches`] answers this too -- `is_empty()` on
    /// what it returns -- but it answers it *loudly*: a miss logs the name and
    /// then dumps every resource the world does have, which is right for a
    /// caller that expected a patch and wrong for a caller that is only asking
    /// whether an item is raw. `craft_ticks`
    /// (`crates/planner/src/method/produce.rs`) asks exactly that, once per
    /// item per recursion, and every intermediate it walks -- plates, gears,
    /// the drill itself -- is a legitimate miss. One plan emitted ~22 pairs of
    /// `no resource patch found for 'burner-mining-drill'` and the full
    /// resource dump beside it, which is the volume that teaches a reader to
    /// skip the log; two real problems went unnoticed behind it in one day.
    ///
    /// So this is the predicate and `resource_patches` stays the query, with
    /// its warning intact for callers that mean it. It also does no flood fill
    /// and allocates nothing.
    pub fn has_resource_patches(&self, resource_name: &str) -> bool {
        self.resources
            .get(resource_name)
            .is_some_and(|tiles| !tiles.is_empty())
    }

    /// A stable identity for the **map**, derived from every charted resource
    /// tile.
    ///
    /// # Why this exists
    ///
    /// Two runs are only comparable if they ran on the same map, and until
    /// 2026-09-03 nothing recorded which map a run used. `--seed` is the
    /// control that would answer it going forward, but it cannot answer it for
    /// a map that already exists: every one of the 24 archived runs was made on
    /// a `level.zip` whose seed nobody wrote down, and no amount of later
    /// carefulness recovers it. This is the one identity that can still be
    /// computed from such a map -- it reads what is there rather than what was
    /// asked for.
    ///
    /// It is also the difference that actually mattered. The retracted "four
    /// bots do double the work of one" compared two runs whose maps differed by
    /// a resource patch about 100 tiles east; that shows up here as a different
    /// digest and a different tile count, and in nothing else the record keeps.
    ///
    /// # What it can and cannot conclude
    ///
    /// **A matching digest means the same map. A differing digest means
    /// `unknown`, not "a different map".** The resource table holds *charted*
    /// tiles, and charting grows as bots walk around, so the same map read at
    /// two different moments legitimately gives two digests. Taken at the start
    /// of a run the charted area is essentially the generated spawn region and
    /// two runs on one map agree, but that is a strong tendency rather than a
    /// guarantee, and a comparison tool must treat a mismatch as inconclusive
    /// in the same way [`crate::record`] treats a missing run id as unknown.
    ///
    /// Returns `None` when nothing is charted at all, which is a world that has
    /// not been read yet rather than a map with no ore.
    pub fn resource_fingerprint(&self) -> Option<ResourceFingerprint> {
        // Sorted by name, and each name's tiles already sorted: `resources` is
        // a `DashMap` (iteration order is not stable) of `BTreeMap` (which is).
        // Collecting into a `BTreeMap` here is what makes the digest a property
        // of the map rather than of this process's allocator.
        let mut tiles: BTreeMap<String, Vec<Pos>> = BTreeMap::new();
        for entry in self.resources.iter() {
            if entry.value().is_empty() {
                continue;
            }
            tiles.insert(entry.key().clone(), entry.value().keys().cloned().collect());
        }
        if tiles.is_empty() {
            return None;
        }
        // FNV-1a, written out rather than taken from `DefaultHasher`, whose
        // output std explicitly does not promise to keep stable across
        // releases. A fingerprint that changes when the toolchain changes would
        // report every run as a different map after an upgrade -- the exact
        // false negative this is meant to remove.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        };
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (name, positions) in &tiles {
            feed(name.as_bytes());
            for pos in positions {
                feed(&pos.0.to_le_bytes());
                feed(&pos.1.to_le_bytes());
            }
            counts.insert(name.clone(), positions.len());
        }
        Some(ResourceFingerprint {
            digest: format!("{hash:016x}"),
            tiles: counts,
        })
    }

    pub fn resource_patches(&self, resource_name: &str) -> Vec<ResourcePatch> {
        let mut patches: Vec<ResourcePatch> = vec![];
        let mut positions_by_id: HashMap<Pos, Option<u32>> = HashMap::new();
        let resource = self.resources.get(resource_name);
        if resource.is_none() {
            warn!("no resource patch found for '{}'", resource_name);
            warn!(
                "available resource paths '{:?}'",
                self.resources
                    .iter()
                    .map(|f| f.key().to_string())
                    .collect::<Vec<_>>()
            );
            return vec![];
        }
        for point in resource.unwrap().keys() {
            positions_by_id.insert(point.clone(), None);
        }
        let mut next_id: u32 = 0;
        while let Some((next_pos, _)) = positions_by_id.iter().find(|(_, value)| value.is_none()) {
            next_id += 1;
            let next_pos = next_pos.clone();
            let mut stack: Vec<Pos> = vec![next_pos.clone()];
            positions_by_id.insert(next_pos.clone(), Some(next_id));
            while let Some(pos) = stack.pop() {
                // The eight compass points: this is an 8-connected flood fill
                // over ore tiles. `Direction::all()` is sixteen values since
                // the 2.x widening and would visit eight half-diagonals that
                // name no tile at all.
                for direction in Direction::compass() {
                    let Some(other) = move_position(&(&pos).into(), direction, 1.0) else {
                        continue;
                    };
                    let other: Pos = (&other).into();
                    if let Some(p) = positions_by_id.get(&other)
                        && p.is_none()
                    {
                        positions_by_id.insert(other.clone(), Some(next_id));
                        stack.push(other);
                    }
                }
            }
        }
        for id in 1..=next_id {
            let mut elements: Vec<Position> = vec![];
            for (k, v) in &positions_by_id {
                if v.unwrap() == id {
                    // Tile *centre*, not tile corner. `resources` is keyed by
                    // `Pos`, which floors, so the half-tile offset that every
                    // real resource entity has (`(-40.5, -48.5)`, never
                    // `(-41, -49)`) is not in the key and must be put back
                    // here. It matters because these positions leave the
                    // process: the planner copies one into a `Mine` action and
                    // the executor sends it to `action_start_mining`, whose
                    // `surface.find_entity(name, position)` matches the entity
                    // position *exactly*. A corner matched nothing, so mining
                    // failed with `Error: no entity to mine` for every ore on
                    // every map. Same convention as the `resource_tree` insert
                    // in `add` below, which already recovers the centre with
                    // `.floor() + 0.5`.
                    elements.push(Position::new(k.0 as f64 + 0.5, k.1 as f64 + 0.5));
                }
            }
            patches.push(ResourcePatch {
                name: resource_name.into(),
                rect: bounding_box(&elements).unwrap(),
                elements,
                id,
            });
        }
        patches.sort_by_key(|a| std::cmp::Reverse(a.elements.len()));
        patches
    }

    pub fn add_tiles(&self, tiles: Vec<FactorioTile>, _clear_rect: Option<Rect>) -> Result<()> {
        let mut tree = self.tile_tree.write();
        let mut blocked = self.blocked_tree.write();
        for tile in tiles {
            let rect: QuadTreeRect = add_to_rect(
                &Rect::from_wh(1., 1.),
                &Position::new(tile.position.x() + 0.5, tile.position.y() + 0.5),
            )
            .into();
            if tile.player_collidable {
                let minable = false; // player_collidable tiles like water are not minable
                file_blocked_box(&mut blocked, minable, rect);
            }
            tree.insert_with_box(tile, rect);
        }
        Ok(())
    }

    pub fn add_blueprint_entities(&self, str: &str) -> Result<()> {
        let decoded = BlueprintCodec::decode_string(str).expect("failed to parse blueprint");
        let mut entities: Vec<FactorioEntity> = vec![];
        match decoded {
            Container::Blueprint(blueprint) => {
                let version = blueprint.version;
                for ent in blueprint.entities {
                    entities.push(FactorioEntity::from_blueprint_entity(
                        ent,
                        version,
                        self.entity_prototypes.clone(),
                    )?);
                }
            }
            _ => panic!("blueprint books not supported"),
        }
        self.add(entities, None)
    }

    pub fn add(&self, entities: Vec<FactorioEntity>, _clear_rect: Option<Rect>) -> Result<()> {
        // Every node this call mints, with the footprint it stands on -- the
        // input to the incremental wiring at the end of this method. See
        // `connect_nodes_near`.
        let mut added: Vec<(NodeIndex, Rect)> = vec![];
        let mut resource_tree = self.resource_tree.write();
        for entity in &entities {
            if entity.entity_type == EntityType::Resource.to_string() {
                // One tile, one entry -- however many times the tile is
                // delivered.
                //
                // Resources arrive here more than once by design of the
                // transport, not by accident: `writeout_entities` runs from
                // both arms of the mod's `on_chunk_generated`, the real event
                // and the `initial_discovery` replay, and only the *tiles*
                // writeout next to it is guarded against emitting a chunk
                // twice. Guarding the entities writeout the same way would
                // lose data instead -- discovery emits `{}` for a chunk that
                // is listed but not yet generated, and the real event that
                // follows carries the contents -- so the deduplication belongs
                // here, at the one insertion point every reader is behind.
                //
                // Left unhandled it inflated the resource half of this graph
                // ~2x. `resource_patches` happened to hide that from the
                // planner (it keys tiles into a `HashMap`, which collapses the
                // copies), but `snapshot_within` reported them all, and
                // `remove` deleted only one copy from this map -- so a mined
                // tile stayed in the model as ore.
                //
                // The *amount* is refreshed on every delivery, though, and
                // that is the one thing a repeat is good for: the mod sends
                // `entity.amount` with every resource it serialises, so a
                // later payload for a tile already known carries a fresher
                // reading than the one stored. Dropping it because the tile is
                // not new would be the same mistake as never reading it at
                // all. A delivery that carries no amount (a fixture, a
                // blueprint) leaves whatever is stored alone rather than
                // erasing it -- silence is not a report of zero.
                let pos: Pos = (&entity.position).into();
                let already_known = {
                    let mut tiles = self.resources.entry(entity.name.clone()).or_default();
                    match tiles.get_mut(&pos) {
                        Some(stored) => {
                            if entity.amount.is_some() {
                                *stored = entity.amount;
                            }
                            true
                        }
                        None => {
                            tiles.insert(pos, entity.amount);
                            false
                        }
                    }
                };
                if already_known {
                    continue;
                }
                let rect: QuadTreeRect = add_to_rect(
                    &Rect::from_wh(1., 1.),
                    &Position::new(
                        entity.position.x().floor() + 0.5,
                        entity.position.y().floor() + 0.5,
                    ),
                )
                .into();
                resource_tree.insert_with_box(entity.name.clone(), rect);
            }
        }
        let mut blocked = self.blocked_tree.write();
        // println!("inserted {}", blocked.len());
        for mut entity in entities {
            if entity.entity_type == EntityType::FlyingText.to_string()
                || entity.entity_type == EntityType::Fish.to_string()
                || entity.bounding_box.width() == 0.
            {
                continue;
            }
            if entity.entity_type != EntityType::Resource.to_string()
                && entity.entity_type != EntityType::StraightRail.to_string()
                && entity.entity_type != EntityType::CurvedRail.to_string()
                && !GHOST_ENTITY_TYPES.contains(&entity.entity_type.as_str())
            {
                file_blocked_box(
                    &mut blocked,
                    entity.is_minable(),
                    entity.bounding_box.clone().into(),
                );
                // The same `is_minable` the line above hands to the blocked
                // tree, kept here by name and position as well. `blocked_tree`
                // stores a bare rectangle, so a caller reading it back can say
                // "something minable is in the way" and nothing else -- not
                // what it is, not where its centre is, and therefore not
                // enough to ask the game to mine it.
                if entity.is_minable() {
                    self.minables
                        .entry(entity.name.clone())
                        .or_default()
                        .insert((&entity.position).into(), entity.position.clone());
                }
                // Recorded here rather than in the `EntityType::from_str`
                // whitelist below, because the whole point is that these
                // types are not in `EntityType` and must not be: see
                // `ENEMY_STRUCTURE_TYPES`. Like `minables` this is keyed by
                // the floored tile and holds the game's own position, so a
                // caller can hand the position straight back to the mod.
                if ENEMY_STRUCTURE_TYPES.contains(&entity.entity_type.as_str()) {
                    self.threats
                        .entry(entity.name.clone())
                        .or_default()
                        .insert((&entity.position).into(), entity.position.clone());
                }
            }
            if entity.name == EntityName::Pumpjack.to_string() {
                // for some reason pumpjacks report their drop position at their position so we fix it
                let offset =
                    Direction::from_u8(entity.direction).and_then(|direction| match direction {
                        Direction::North => Some(Position::new(1., -2.)),
                        Direction::East => Some(Position::new(2., -1.)),
                        Direction::South => Some(Position::new(-1., 2.)),
                        Direction::West => Some(Position::new(-2., 1.)),
                        _ => None,
                    });
                match offset {
                    Some(offset) => entity.drop_position = Some(entity.position.add(&offset)),
                    None => {
                        // Aborting here killed the bot outright; so would leaving
                        // the uncorrected drop position in place, only quietly.
                        error!(
                            "<red>unusable pumpjack direction</> <bright-blue>{}</> at <bright-blue>{}</>: cannot place its drop position -- entity skipped",
                            entity.direction, entity.position
                        );
                        continue;
                    }
                }
            }

            if let Ok(entity_type) = EntityType::from_str(&entity.entity_type) {
                match (entity.name.as_str(), &entity_type) {
                    (_, EntityType::Furnace)
                    | (_, EntityType::Inserter)
                    | (_, EntityType::Boiler)
                    | (_, EntityType::Lab)
                    | (_, EntityType::OffshorePump)
                    | (_, EntityType::MiningDrill)
                    | (_, EntityType::StorageTank)
                    | (_, EntityType::Container)
                    | (_, EntityType::Splitter)
                    | (_, EntityType::TransportBelt)
                    | (_, EntityType::UndergroundBelt)
                    | (_, EntityType::Pipe)
                    | (_, EntityType::PipeToGround)
                    | (_, EntityType::LogisticContainer)
                    | (_, EntityType::AssemblingMachine)
                    // The electric network, admitted 2026-09-02. A pole and a
                    // steam engine reached `blocked_tree` above -- so they
                    // refused placements -- and stopped there, which made a
                    // hand-built power plant unreadable *by name* and
                    // `PlanState::electric_supply_kw` score every live base
                    // 0 kW. They draw no edges in `connect` (its match ends
                    // `_ => {}`, and neither carries a drop or pickup
                    // position) and `FlowGraph` prunes anything it reaches
                    // that it does not model, so admitting them adds nodes
                    // and no behaviour beyond being nameable.
                    | (_, EntityType::ElectricPole)
                    | (_, EntityType::Generator)
                    | (_, EntityType::SolarPanel)
                    | ("rock-big", _)
                    | ("rock-huge", _) => {
                        if let Some(entity_id) = self.entity_at(&entity.position) {
                            let tree = self.entity_tree.read();
                            let block = tree.get(entity_id).unwrap();
                            warn!(
                                "failed to add {}@{} -> blocked by {}@{}",
                                entity.name, entity.position, block.name, block.position
                            );
                            continue;
                        }
                        if let Some(entity_id) = {
                            let mut tree = self.entity_tree.write();
                            tree.insert(entity.clone())
                        } {
                            let miner_ore = if entity_type == EntityType::MiningDrill {
                                let rect = rect_floor(&entity.bounding_box);
                                let mut miner_ore: Option<String> = None;
                                for resource in &[
                                    EntityName::IronOre,
                                    EntityName::CopperOre,
                                    EntityName::Coal,
                                    EntityName::Stone,
                                    EntityName::CrudeOil,
                                    EntityName::UraniumOre,
                                ] {
                                    let resource = resource.to_string();
                                    let resource_found = rect_fields(&rect).iter().any(|p| {
                                        self.resources
                                            .get(&resource)
                                            .and_then(|resources| {
                                                if resources.contains_key(&p.into()) {
                                                    Some(true)
                                                } else {
                                                    None
                                                }
                                            })
                                            .is_some()
                                    });
                                    if resource_found {
                                        miner_ore = Some(resource);
                                        break;
                                    }
                                }
                                if miner_ore.is_none() {
                                    warn!(
                                        "no ore found under miner {} @ {}",
                                        entity.name, entity.position
                                    );
                                }
                                miner_ore
                            } else {
                                None
                            };
                            let Some(new_node) =
                                EntityNode::new(entity.clone(), miner_ore, entity_id)
                            else {
                                continue;
                            };
                            let mut inner = self.entity_graph.write();
                            let new_node_index = inner.add_node(new_node);
                            self.entity_nodes.insert(entity_id, new_node_index);
                            added.push((new_node_index, entity.bounding_box.clone()));
                        } else {
                            warn!("failed to insert entity into quad tree");
                        }
                    }
                    _ => {}
                }
            }
        }
        // Released before the wiring below, which takes `entity_tree` and
        // `entity_graph` and must not be holding a second write guard while it
        // does.
        drop(blocked);
        drop(resource_tree);
        // **The edges, not only the nodes.** Until 2026-09-07 this method
        // added nodes and stopped, and `connect` ran from exactly three
        // places -- `OutputParser::on_init` (once, at `initial discovery
        // done`), `factorio::snapshot` (the `--connect` path) and
        // `FactorioSurface::import`. So every machine, belt and inserter a
        // *run* built entered the graph unwired, and everything reading
        // connectivity -- `FlowGraph`, which is rebuilt on this graph's
        // generation, `PlanState`'s electric network walk, `method::connect`
        // -- was answering about the world as it stood at tick 0, with a stale
        // answer that looks exactly like a current one.
        self.connect_nodes_near(&added);
        // The mutation is complete: anything built from this graph is now
        // one generation behind. See `generation`.
        self.bump_generation();
        Ok(())
    }

    /// The distance past its own footprint at which an entity that is *not* an
    /// underground belt or pipe can still draw an edge to a newly added one.
    ///
    /// Every rule in [`Self::connect_node`] but the two underground ones is
    /// local: a drop or pickup position just outside the machine that owns it,
    /// a belt or splitter output one tile on, a storage tank's connection
    /// points two tiles from its centre. Three tiles past the footprint covers
    /// all of them with a tile to spare.
    const NEAR_REACH: f64 = 3.;

    /// How far the two long-range rules reach, **derived from the prototypes**
    /// rather than tabled, so it survives a mod that changes an underground
    /// belt's span. Vanilla's longest is the underground pipe at 10.
    fn underground_reach(&self) -> f64 {
        self.entity_prototypes
            .iter()
            .filter_map(|prototype| prototype.max_underground_distance)
            .max()
            .map(f64::from)
            .unwrap_or(Self::NEAR_REACH)
            .max(Self::NEAR_REACH)
    }

    /// Wire the nodes just added **and everything that might point at them**.
    ///
    /// The second half is why this is not simply "connect the new nodes": an
    /// edge is drawn while visiting its *source*, so a belt built downstream of
    /// one that was already standing needs the standing belt re-visited, not
    /// the new one. The candidate set is therefore the new nodes plus their
    /// neighbourhood, deduplicated, which keeps the work proportional to what
    /// changed instead of to the size of the world.
    ///
    /// Safe to re-run over nodes that are already wired: see [`Self::connect`]
    /// on appending, deduplication and deletion.
    fn connect_nodes_near(&self, added: &[(NodeIndex, Rect)]) {
        if added.is_empty() {
            return;
        }
        let far = self.underground_reach();
        let mut candidates: BTreeSet<NodeIndex> = BTreeSet::new();
        for (node_index, bounding_box) in added {
            candidates.insert(*node_index);
            let near_box = grow_rect(bounding_box, Self::NEAR_REACH);
            let far_box = grow_rect(bounding_box, far);
            let hits: Vec<(Position, Option<EntityType>, ItemId)> = {
                let tree = self.entity_tree.read();
                tree.query(far_box.into())
                    .iter()
                    .map(|(entity, _rect, item_id)| {
                        (
                            entity.position.clone(),
                            EntityType::from_str(&entity.entity_type).ok(),
                            *item_id,
                        )
                    })
                    .collect()
            };
            for (position, entity_type, item_id) in hits {
                // Anything further away than `NEAR_REACH` can only reach this
                // footprint by tunnelling, and only those two types tunnel.
                let reaches = near_box.contains(&position)
                    || matches!(
                        entity_type,
                        Some(EntityType::UndergroundBelt) | Some(EntityType::PipeToGround)
                    );
                if reaches && let Some(index) = self.entity_nodes.get(&item_id) {
                    candidates.insert(*index);
                }
            }
        }
        self.connect_nodes(candidates.into_iter().collect());
    }

    pub fn condense(&self) -> EntityGraphInner {
        let _started = Instant::now();
        let mut graph = self.entity_graph.read().clone();
        let _starting_nodes = graph.node_indices().count();
        let mut roots: Vec<usize> = vec![];
        loop {
            let mut next_node: Option<NodeIndex> = None;
            for node_index in graph.externals(petgraph::Direction::Incoming) {
                if !roots.contains(&node_index.index()) {
                    roots.push(node_index.index());
                    next_node = Some(node_index);
                    break;
                }
            }
            if let Some(next_node) = next_node {
                let mut bfs = Bfs::new(&graph, next_node);
                while let Some(node_index) = bfs.next(&graph) {
                    let node = graph.node_weight(node_index).unwrap();
                    let incoming: Vec<String> = graph
                        .edges_directed(node_index, petgraph::Direction::Incoming)
                        .map(|edge| {
                            graph
                                .node_weight(edge.target())
                                .unwrap()
                                .entity_name
                                .clone()
                        })
                        .collect();
                    let outgoing: Vec<String> = graph
                        .edges_directed(node_index, petgraph::Direction::Outgoing)
                        .map(|edge| {
                            graph
                                .node_weight(edge.target())
                                .unwrap()
                                .entity_name
                                .clone()
                        })
                        .collect();
                    if incoming.len() == 1
                        && outgoing.len() == 1
                        && node.entity_name == incoming[0]
                        && incoming[0] == outgoing[0]
                    {
                        let incoming: NodeIndex = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.source())
                            .find(|_| true)
                            .unwrap();
                        let outgoing = graph
                            .edges_directed(node_index, petgraph::Direction::Outgoing)
                            .map(|edge| edge.target())
                            .find(|_| true)
                            .unwrap();
                        let weight = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| *edge.weight())
                            .find(|_| true)
                            .unwrap()
                            + graph
                                .edges_directed(node_index, petgraph::Direction::Outgoing)
                                .map(|edge| *edge.weight())
                                .find(|_| true)
                                .unwrap();
                        graph.add_edge(incoming, outgoing, weight);
                        if let Some(edge) = graph.find_edge(incoming, node_index) {
                            graph.remove_edge(edge);
                        }
                        if let Some(edge) = graph.find_edge(node_index, outgoing) {
                            graph.remove_edge(edge);
                        }
                        graph.remove_node(node_index);
                    } else if incoming.len() == 2
                        && outgoing.len() == 2
                        && node.entity_name == incoming[0]
                        && incoming[0] == outgoing[0]
                    {
                        let incoming: Vec<NodeIndex> = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.source())
                            .collect();
                        let weights: Vec<f64> = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| *edge.weight())
                            .collect();
                        let weight = weights[0] + weights[1];
                        graph.add_edge(incoming[0], incoming[1], weight);
                        graph.add_edge(incoming[1], incoming[0], weight);
                        for connected_index in incoming {
                            if let Some(edge) = graph.find_edge(connected_index, node_index) {
                                graph.remove_edge(edge);
                            }
                            if let Some(edge) = graph.find_edge(node_index, connected_index) {
                                graph.remove_edge(edge);
                            }
                        }
                        graph.remove_node(node_index);
                    }
                }
            } else {
                break;
            }
        }

        let mut orphans: Vec<NodeIndex> = vec![];
        for node_index in graph.node_indices() {
            if graph
                .edges_directed(node_index, petgraph::Direction::Incoming)
                .count()
                == 0
                && graph
                    .edges_directed(node_index, petgraph::Direction::Outgoing)
                    .count()
                    == 0
            {
                orphans.push(node_index);
            }
        }
        for orphan in orphans {
            graph.remove_node(orphan);
        }
        // info!(
        //     "condensing entity graph from {} to {} entities took {:?}",
        //     starting_nodes,
        //     graph.node_indices().count(),
        //     started.elapsed()
        // );
        graph
    }

    /// Takes an entity the game says is gone out of every structure that
    /// tracks it.
    ///
    /// # A mined resource that still holds ore is not gone
    ///
    /// The mod raises this for a resource on **every mining swing**, not on
    /// depletion: `on_player_mined_entity` is wired to both `on_mined_entity`
    /// and `on_some_entity_deleted` (`control.lua`), and the first of those
    /// counts one swing's delivery against the action's remaining need -- so a
    /// `mine 5` on a tile holding hundreds raises it five times and the tile
    /// survives all five.
    ///
    /// Deleting on the first swing is what the archive caught: across the runs
    /// in `workspace/runs/`, **every** ore tile a keyframe found in the game and
    /// not in the model was a tile a bot had mined at -- 583 of them, none
    /// unexplained -- while the game still had ore there. It also made
    /// `resource_mined`'s retirement unreachable, because by the time an action
    /// settled the tile it was about to debit had already left the model.
    ///
    /// So a resource payload carrying a **positive `amount`** is an amount
    /// report, not a removal, and the tile stays. The paths that mean the tile
    /// is really gone still delete: `on_resource_depleted` writes the same line
    /// for an emptied entity, and [`EntityGraph::retire_resource`] builds its
    /// entity through `FactorioEntity::new_resource`, which carries no amount
    /// at all.
    ///
    /// The reported amount is deliberately *not* written into the model.
    /// [`EntityGraph::resource_mined`] is the debit authority; applying the
    /// game's reading here as well would take the same ore out twice and could
    /// retire a tile that still holds a few units -- the same defect in a
    /// smaller costume.
    pub fn remove(&self, entity: &FactorioEntity) -> Result<()> {
        if entity.entity_type == EntityType::Resource.to_string()
            && let Some(amount) = entity.amount
            && amount > 0
        {
            return Ok(());
        }
        let is_resource = entity.entity_type == EntityType::Resource.to_string();
        let mut nodes_to_remove: Vec<NodeIndex> = vec![];
        let mut edges_to_remove: Vec<EdgeIndex> = vec![];
        let mut entities_to_remove: Vec<ItemId> = vec![];

        // A resource has no node: `add` admits only the named machine types
        // into `entity_tree`, and `entity_at` is a point query on that tree.
        // Asked at an ore tile's centre it answers with whatever machine
        // stands *over* the ore -- a drill, on the tiles it is eating -- and
        // the block below would then unhook that machine's node and edges
        // from the graph while `entity_tree` kept the entity. The blocked-box
        // sweep further down had the same shape and the same victim; see the
        // comment there for the run that paid for it.
        if !is_resource && let Some(entity_id) = self.entity_at(&entity.position) {
            if let Some(node_index) = self.entity_nodes.get(&entity_id) {
                let inner = self.entity_graph.read();
                for edge in inner.edges_directed(*node_index, petgraph::Direction::Incoming) {
                    edges_to_remove.push(edge.id());
                }
                for edge in inner.edges_directed(*node_index, petgraph::Direction::Outgoing) {
                    edges_to_remove.push(edge.id());
                }
                nodes_to_remove.push(*node_index);
            }
            entities_to_remove.push(entity_id);
        }
        let mut inner = self.entity_graph.write();
        for edge in edges_to_remove {
            inner.remove_edge(edge);
        }
        for entity_id in entities_to_remove {
            self.entity_nodes.remove(&entity_id);
        }
        for node in nodes_to_remove {
            inner.remove_node(node);
        }

        // Only the box `add` filed for *this* entity leaves `blocked_tree`.
        //
        // This used to drop every box the removed entity's bounds overlapped,
        // which is a different set whenever the removed entity is *inside*
        // another one -- and ore under a mining drill is exactly that. A
        // burner drill stands on four ore tiles and eats them; when one runs
        // dry the mod reports the tile deleted (`on_resource_depleted` ->
        // `on_some_entity_deleted`, `amount: 0`), this ran with the ore's
        // 0.2-tile bounding box, the quad tree answered with the drill's box
        // around it, and the drill silently stopped blocking anything. It was
        // still in `entity_tree` -- that sweep filters by name -- so the model
        // knew the drill was there *by name* and had forgotten it *by ground*.
        // `PlanState::resource_tile_blocked` reads the ground, so the next
        // plan hand-mined the three tiles left under the drill and the game
        // answered `expected iron-ore at (-7.5/-29.5), found
        // burner-mining-drill` (`run-1788559688-08406`: six of seven drills
        // from plan 1 had lost their box before plan 2 was made, and both of
        // that plan's failures were this). `resource_mined` reaches the same
        // sweep through `retire_resource`, so it was not only the mod's
        // report that could do it.
        //
        // Two rules, both about which box is *the entity's own*:
        //
        // * A resource never had one. `add` keeps ore and rails out of
        //   `blocked_tree` on purpose, so a resource removal has nothing to
        //   take out of it and must not go looking.
        // * Anything else owns the box whose centre lies inside its own
        //   bounds. That is `add`'s box for the same entity under any
        //   direction (`retire_minable` rebuilds a box from the prototype
        //   without turning it, and a turned box still holds the centre), and
        //   it is never a neighbour's: two standing entities do not overlap,
        //   so no neighbour's centre can be inside these bounds. Matching the
        //   rectangle exactly instead would leave a stump blocking for ever
        //   the moment a rebuilt box differs by a rotation, which is the
        //   failure `retire_minable` documents.
        if !is_resource {
            let mut blocked_item_ids_to_remove: Vec<ItemId> = vec![];
            let blocked_tree = self.blocked_tree.read();
            for (_, rect, item_id) in blocked_tree.query(entity.bounding_box.clone().into()) {
                let centre = Position::new(
                    (rect.origin.x + rect.size.width / 2.) as f64,
                    (rect.origin.y + rect.size.height / 2.) as f64,
                );
                if entity.bounding_box.contains(&centre) {
                    blocked_item_ids_to_remove.push(item_id);
                }
            }
            drop(blocked_tree);
            let mut blocked_tree = self.blocked_tree.write();
            for item_id in blocked_item_ids_to_remove {
                blocked_tree.remove(item_id);
            }
            drop(blocked_tree);
        }
        let mut entity_item_ids_to_remove: Vec<ItemId> = vec![];
        let entity_tree = self.entity_tree.read();
        for (other_entity, _, item_id) in entity_tree.query(entity.bounding_box.clone().into()) {
            if entity.name == other_entity.name {
                entity_item_ids_to_remove.push(item_id);
            }
        }
        drop(entity_tree);
        let mut entity_tree = self.entity_tree.write();
        for item_id in entity_item_ids_to_remove {
            entity_tree.remove(item_id);
        }
        drop(entity_tree);

        if entity.entity_type == EntityType::Resource.to_string() {
            let mut resource_item_ids_to_remove: Vec<ItemId> = vec![];
            let resource_tree = self.resource_tree.read();
            for (_, _, item_id) in resource_tree.query(entity.bounding_box.clone().into()) {
                resource_item_ids_to_remove.push(item_id);
            }
            drop(resource_tree);
            let mut resource_tree = self.resource_tree.write();
            for item_id in resource_item_ids_to_remove {
                resource_tree.remove(item_id);
            }
            drop(resource_tree);
            if let Some(mut positions) = self.resources.get_mut(&entity.name) {
                let entity_pos: Pos = (&entity.position).into();
                // One removal empties the tile, because `add` only ever put it
                // in once. While duplicates were kept this removed a single
                // copy and left the others, so a tile the game had just been
                // mined out of stayed in `resource_patches` as ore.
                positions.remove(&entity_pos);
            }
        }

        // Unconditional, and not behind an `is_minable()` check on the entity
        // handed in: a caller that rebuilt this entity from a name and a
        // position (`retire_minable` does exactly that) has no entity type to
        // check, and a name that is in `minables` is by construction one that
        // `add` put there.
        if let Some(mut tiles) = self.minables.get_mut(&entity.name) {
            tiles.remove(&(&entity.position).into());
        }
        // Same shape, same reason. Nothing in this workspace kills a nest
        // today, but `remove` is the one door an entity leaves by, and a map
        // that only ever grows would keep refusing routes past a spawner that
        // is no longer there.
        if let Some(mut tiles) = self.threats.get_mut(&entity.name) {
            tiles.remove(&(&entity.position).into());
        }

        // The mutation is complete: anything built from this graph is now
        // one generation behind. See `generation`.
        self.bump_generation();
        Ok(())
    }

    /// Wire the whole graph: every node's outgoing and incoming edges, from
    /// its drop and pickup positions and its type's own adjacency rules.
    ///
    /// # It appends, it dedupes, and it is not the only maintainer
    ///
    /// Three facts about this method decide how it may be called, established
    /// 2026-09-07 (`docs/superpowers/notes/2026-09-07-edges-that-outlive-tick-zero.md`):
    ///
    /// * **It appends.** Nothing here clears an edge, so it can only ever add.
    /// * **It dedupes**, twice: every candidate is guarded by `contains_edge`
    ///   when it is gathered and again when it is applied, so re-running it on
    ///   an unchanged world adds nothing and is a no-op.
    /// * **It does not have to delete**, because [`Self::remove`] already
    ///   does: it drops the entity's node, both directions of its edges and
    ///   its `entity_nodes` mapping, over a `StableGraph` whose indices
    ///   survive a removal. `node_at` resolves through `entity_tree`, which
    ///   `remove` also empties, so the position-reuse trap that forced
    ///   `FlowGraph::update` to rebuild from scratch does not exist here.
    ///
    /// Together those make it safe to re-run incrementally, which is what
    /// [`Self::add`] does through [`Self::connect_nodes`]. This full sweep is
    /// still what a bulk install wants (`OutputParser::on_init`,
    /// `factorio::snapshot`, `FactorioSurface::import`).
    pub fn connect(&self) -> Result<()> {
        let nodes: Vec<NodeIndex> = self.entity_graph.read().node_indices().collect();
        self.connect_nodes(nodes);
        // The mutation is complete: anything built from this graph is now
        // one generation behind. See `generation`.
        self.bump_generation();
        Ok(())
    }

    /// [`Self::connect`] restricted to a chosen set of nodes, and
    /// **deliberately not bumping the generation** -- it is called from inside
    /// [`Self::add`], which is one mutation and bumps once for the whole of
    /// it. A second bump there would invalidate every generation-keyed cache
    /// twice per add for no change.
    fn connect_nodes(&self, nodes: Vec<NodeIndex>) {
        let mut edges_to_add: Vec<(NodeIndex, NodeIndex, f64)> = vec![];
        for node_index in nodes {
            self.connect_node(node_index, &mut edges_to_add);
        }
        let mut inner = self.entity_graph.write();
        for (a, b, w) in edges_to_add {
            if !inner.contains_edge(a, b) {
                inner.add_edge(a, b, w);
            }
        }
    }

    /// Every edge one node draws, gathered into `edges_to_add` rather than
    /// written, so the caller can apply a whole sweep under one write lock.
    ///
    /// **It holds no `entity_tree` guard across the body**, and that is not
    /// tidiness. `node_at` -- called from nearly every rule below -- takes
    /// `entity_tree.read()` itself, and parking_lot documents a recursive read
    /// as a deadlock hazard: a writer queued between the outer and the inner
    /// acquisition blocks both. The sweep used to hold one guard for its whole
    /// pass, which was survivable while it ran twice in a process; it now runs
    /// on every `add`, on the parser thread, beside readers on others. So the
    /// two positions this needs are copied out under a guard released at once,
    /// and the rest of the identity comes off the `EntityNode`.
    fn connect_node(
        &self,
        node_index: NodeIndex,
        edges_to_add: &mut Vec<(NodeIndex, NodeIndex, f64)>,
    ) {
        let inner = self.entity_graph.read();
        if let Some(node) = inner.node_weight(node_index) {
            let (drop_position, pickup_position) = {
                let tree = self.entity_tree.read();
                let Some(node_entity) = node.entity_id.and_then(|id| tree.get(id)) else {
                    return;
                };
                (
                    node_entity.drop_position.clone(),
                    node_entity.pickup_position.clone(),
                )
            };
            if let Some(drop_position) = drop_position.as_ref() {
                // if node_entity.entity_type == "mining-drill" {
                //     info!(
                //         "drop position for {} -> {} @ {}",
                //         node_entity.name, node_entity.position, drop_position
                //     );
                // }
                match self.node_at(drop_position) {
                    Some(drop_index) => {
                        // if node_entity.name == "pumpjack" {
                        //     info!(
                        //         "found pipe?",
                        //     );
                        // }

                        if !inner.contains_edge(node_index, drop_index) {
                            edges_to_add.push((node_index, drop_index, 1.));
                        }
                    }
                    // `debug!`, not `error!`. Nothing is wrong here: a drill
                    // dropping ore on the ground, or an inserter whose target
                    // is not a type this graph models, is ordinary. It read as
                    // an error only while this swept twice in a process; on
                    // every `add` it is thousands of false alarms per run.
                    None => debug!(
                        "connect entity graph could not find entity at Drop position {} for {} @ {}",
                        drop_position, node.entity_name, node.position
                    ),
                }
            }
            if let Some(pickup_position) = pickup_position.as_ref() {
                match self.node_at(pickup_position) {
                    Some(pickup_index) => {
                        if !inner.contains_edge(pickup_index, node_index) {
                            edges_to_add.push((pickup_index, node_index, 1.));
                        }
                    }
                    None => debug!(
                        "connect entity graph could not find entity at Pickup position {} for {} @ {}",
                        pickup_position, node.entity_name, node.position
                    ),
                }
            }
            // Fluid joints, from the prototypes rather than from a list
            // of type names. Two entities are joined where one's fluid box
            // faces a tile the other's fluid box sits on, and their two
            // `production_type`s agree on which way fluid may cross.
            //
            // That last clause is what keeps the flow walk's roots roots: an
            // offshore pump and a pumpjack each declare a single `output`
            // box, so a pipe beside one draws an edge out of it and never one
            // back in, and `FlowGraph::update`'s
            // `externals(Direction::Incoming)` still finds them.
            let my_boxes = self.fluid_boxes(node);
            for mine in &my_boxes {
                for port in &mine.ports {
                    let Some(other_index) = self.node_at(port) else {
                        continue;
                    };
                    if other_index == node_index {
                        continue;
                    }
                    let Some(other) = inner.node_weight(other_index) else {
                        continue;
                    };
                    for theirs in self.fluid_boxes(other) {
                        // Symmetric on purpose: `node_at` answers for any tile
                        // of a footprint, so without asking the neighbour where
                        // ITS box sits, a pipe beside any of a refinery's
                        // twenty-five tiles would read as joined to it.
                        // **Each box must reach into the other's footprint,
                        // both ways.** Stated on footprints rather than on
                        // anchors so that it holds under either `positions`
                        // convention (see [`FluidBoxPorts::certain`]), and
                        // asked of the neighbour as well as of us because
                        // `node_at` answers for any tile of a footprint: a pipe
                        // beside any of a refinery's twenty-five tiles would
                        // otherwise read as joined to it, when only the four it
                        // declares connections on are.
                        if !other.bounding_box.contains(port)
                            || !theirs
                                .ports
                                .iter()
                                .any(|tile| node.bounding_box.contains(tile))
                        {
                            continue;
                        }
                        if mine.gives_out
                            && theirs.takes_in
                            && !inner.contains_edge(node_index, other_index)
                        {
                            edges_to_add.push((node_index, other_index, 1.));
                        }
                        if theirs.gives_out
                            && mine.takes_in
                            && !inner.contains_edge(other_index, node_index)
                        {
                            edges_to_add.push((other_index, node_index, 1.));
                        }
                    }
                }
            }
            match node.entity_type {
                EntityType::Splitter => {
                    // `turn` is `None` for anything but a cardinal; a
                    // splitter facing one of the 2.x half-diagonals has no
                    // computable output tiles, so it gets no output edges
                    // rather than fabricated ones.
                    let (Some(o1), Some(o2)) = (
                        Position::new(-0.5, -1.).turn(node.direction),
                        Position::new(0.5, -1.).turn(node.direction),
                    ) else {
                        return;
                    };
                    let out1 = node.position.add(&o1);
                    let out2 = node.position.add(&o2);
                    for pos in &[&out1, &out2] {
                        if let Some(next_index) = self.node_at(pos) {
                            let next = inner.node_weight(next_index).unwrap();
                            // info!(
                            //     "found splitter output: {} @ {}",
                            //     next.entity.name, next.entity.position
                            // );
                            if !inner.contains_edge(node_index, next_index)
                                && self.is_entity_belt_connectable(node, next)
                            {
                                edges_to_add.push((node_index, next_index, 1.));
                            }
                            // } else {
                            //     warn!(
                            //         "NOT found splitter output: for {} @ {} -> searched @ {}",
                            //         node.entity.name, node.entity.position, pos
                            //     );
                        }
                    }
                }
                EntityType::TransportBelt => {
                    if let Some(next_index) =
                        self.node_at_moved(&node.position, node.direction, 1.0)
                    {
                        let next = inner.node_weight(next_index).unwrap();
                        if !inner.contains_edge(node_index, next_index)
                            && self.is_entity_belt_connectable(node, next)
                        {
                            edges_to_add.push((node_index, next_index, 1.));
                            // } else {
                            //     warn!(
                            //         "2 not found transport belt connect from {} to {} ({:?})",
                            //         node.position,
                            //         move_position(&node.position, node.direction, 1.0),
                            //         node.direction
                            //     )
                        }
                        // } else {
                        //     warn!(
                        //         "1 not found transport belt connect from {} to {} ({:?})",
                        //         node.position,
                        //         move_position(&node.position, node.direction, 1.0),
                        //         node.direction
                        //     )
                    }
                }
                EntityType::UndergroundBelt => {
                    let mut found = false;
                    if let Some(prototype) = self.entity_prototypes.get(&node.entity_name) {
                        if let Some(max_distance) = prototype.max_underground_distance.as_ref() {
                            for length in 1..=*max_distance {
                                if let Some(next_index) = self.node_at_moved(
                                    &node.position,
                                    node.direction.opposite(),
                                    length as f64,
                                ) {
                                    let next = inner.node_weight(next_index).unwrap();
                                    if next.entity_type == EntityType::UndergroundBelt
                                        && next.direction == node.direction
                                    {
                                        if !inner.contains_edge(next_index, node_index) {
                                            edges_to_add.push((
                                                next_index,
                                                node_index,
                                                length as f64,
                                            ));
                                        }
                                        found = true;
                                        break;
                                    }
                                }
                            }
                        } else {
                            warn!("underground belt without max distance?!");
                        }
                    } else {
                        warn!("underground belt prototype not found");
                    }
                    if found
                        && let Some(next_index) =
                            self.node_at_moved(&node.position, node.direction, 1.)
                    {
                        let next = inner.node_weight(next_index).unwrap();
                        if !inner.contains_edge(node_index, next_index)
                            && self.is_entity_belt_connectable(node, next)
                        {
                            edges_to_add.push((node_index, next_index, 1.));
                        }
                    }
                }
                EntityType::PipeToGround => {
                    if let Some(prototype) = self.entity_prototypes.get(&node.entity_name) {
                        if let Some(max_distance) = prototype.max_underground_distance.as_ref() {
                            for length in 1..=*max_distance {
                                if let Some(next_index) = self.node_at_moved(
                                    &node.position,
                                    node.direction,
                                    -(length as f64),
                                ) {
                                    let next = inner.node_weight(next_index).unwrap();
                                    if next.entity_type == EntityType::PipeToGround
                                        && next.direction == node.direction.opposite()
                                    {
                                        if !inner.contains_edge(next_index, node_index) {
                                            edges_to_add.push((
                                                next_index,
                                                node_index,
                                                length as f64,
                                            ));
                                        }
                                        if !inner.contains_edge(node_index, next_index) {
                                            edges_to_add.push((
                                                node_index,
                                                next_index,
                                                length as f64,
                                            ));
                                        }
                                        break;
                                    }
                                }
                            }
                        } else {
                            warn!("underground pipe without max distance?!");
                        }
                    } else {
                        warn!("underground pipe prototype not found");
                    }
                    // The surface hop this used to draw by hand is the
                    // fluid-joint rule above now: a pipe-to-ground's own box is
                    // exact, because it carries an `underground` connection at
                    // the same anchor and so faces `direction` on the surface.
                }
                _ => {}
            }
        }
    }
    pub fn entity_by_id(&self, id: ItemId) -> Option<FactorioEntity> {
        self.entity_tree.read().get(id).cloned()
    }

    /// Record that the crafting machine standing at `position` is now set to
    /// `recipe`. Returns whether there was a machine there to record it on.
    ///
    /// **The one write-back this graph has, and it exists because the graph is
    /// otherwise append-only.** Entities enter through [`Self::add`], which
    /// refuses a tile something already stands on ("failed to add ... blocked
    /// by"), and `FactorioSurface::on_some_entity_updated` is a no-op that the
    /// mod raises only on rotation. So a recipe -- which is put on a machine
    /// by an RCON call *after* it was built, never at build time -- had no
    /// route into the world model at all.
    ///
    /// What that cost is the whole of `run-1788485718-45723`. The planner's
    /// `Goal::Producing` predicate counts machines whose stored `recipe` is
    /// the one the cell needs; every stored machine read `None`; so a cell
    /// that stood, was powered, was fed and had produced four science packs
    /// counted as zero cells, and four consecutive replans each built another
    /// complete cell somewhere else before the supervisor gave up. Every
    /// action in all four succeeded.
    ///
    /// In place through [`crate::aabb_quadtree::QuadTree::get_mut`] rather
    /// than remove-and-reinsert, because reinserting mints a fresh [`ItemId`]
    /// and `entity_nodes` maps the old one to this entity's graph node. A
    /// recipe changes no footprint and draws no edge -- `connect` routes on
    /// drop and pickup positions -- so nothing else in the graph has to move.
    pub fn set_recipe(&self, position: &Position, recipe: &str) -> bool {
        let Some(id) = self.entity_at(position) else {
            return false;
        };
        let mut tree = self.entity_tree.write();
        let set = match tree.get_mut(id) {
            Some(entity) => {
                entity.recipe = Some(recipe.to_string());
                true
            }
            None => false,
        };
        drop(tree);
        if set {
            // A recipe is what an assembling machine's flow edge is computed
            // from, so this changes what the flow graph says exactly as a
            // placement does.
            self.bump_generation();
        }
        set
    }

    /// [`node_at`] one offset step away along `direction`.
    ///
    /// `None` when the direction names no tile -- the Factorio 2.x
    /// half-diagonals rails report -- as well as when nothing is there.
    ///
    /// [`node_at`]: EntityGraph::node_at
    pub fn node_at_moved(
        &self,
        position: &Position,
        direction: Direction,
        offset: f64,
    ) -> Option<NodeIndex> {
        self.node_at(&move_position(position, direction, offset)?)
    }

    pub fn node_at(&self, position: &Position) -> Option<NodeIndex> {
        self.entity_at(position)
            .and_then(|entity_id| self.entity_nodes.get(&entity_id).map(|e| *e))
    }

    pub fn entity_at(&self, position: &Position) -> Option<ItemId> {
        let tree = self.entity_tree.read();
        let results: Vec<ItemId> = tree
            .query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            .iter()
            .map(|(_entity, _rect, item_id)| *item_id)
            .collect();

        if results.is_empty() {
            None
        } else if results.len() == 1 {
            Some(results[0])
        } else {
            warn!(
                "multiple entity quad tree results for {}: {:?}",
                position,
                tree.query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            );
            Some(results[0])
        }
    }
    fn is_entity_belt_connectable(&self, node: &EntityNode, next: &EntityNode) -> bool {
        (next.entity_type == EntityType::TransportBelt
            || next.entity_type == EntityType::UndergroundBelt
            || next.entity_type == EntityType::Splitter)
            && next.direction != node.direction.opposite()
    }
    pub fn graphviz_dot(&self) -> String {
        format_dotgraph(
            Dot::with_config(&self.inner_graph().deref(), &[Config::GraphContentOnly]).to_string(),
        )
    }

    pub fn graphviz_dot_condensed(&self) -> String {
        let condensed = self.condense();
        format_dotgraph(Dot::with_config(&condensed, &[Config::GraphContentOnly]).to_string())
    }

    pub fn node_weight(&self, i: NodeIndex) -> Option<EntityNode> {
        self.entity_graph.read().node_weight(i).cloned()
    }

    pub fn edges_directed(&self, i: NodeIndex, dir: petgraph::Direction) -> Vec<NodeIndex> {
        self.entity_graph
            .read()
            .edges_directed(i, dir)
            .map(|e| e.target())
            .collect()
    }
}

/// A `DashMap<String, BTreeMap<Pos, V>>` on its way onto the wire.
///
/// **`Pos` cannot be a JSON object key.** It is a two-field tuple struct, and
/// `serde_json` refuses the whole document with `key must be a string` the
/// moment one appears in key position -- so `resources` and `minables`, the
/// two maps keyed that way, made *every* `FactorioSurface` serialization of a
/// world containing a single ore tile fail. It never showed up because nothing
/// wrote a world to disk: the only worlds that serialised were empty ones.
///
/// So the inner map travels as a list of `[pos, value]` pairs. A
/// `BTreeMap<Pos, _>` already iterates in tile order, and the outer names are
/// sorted here, which makes this half of a dump byte-stable for a given world
/// -- the same discipline [`crate::factorio::world::FactorioSurface::observed_inventories`]
/// exists to enforce, and for the same reason.
struct TileMaps<'a, V>(&'a DashMap<String, BTreeMap<Pos, V>>);

impl<V: Serialize> Serialize for TileMaps<'_, V> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut names: Vec<String> = self.0.iter().map(|entry| entry.key().clone()).collect();
        names.sort();
        let mut map = serializer.serialize_map(Some(names.len()))?;
        for name in &names {
            if let Some(tiles) = self.0.get(name) {
                let pairs: Vec<(&Pos, &V)> = tiles.iter().collect();
                map.serialize_entry(name, &pairs)?;
            }
        }
        map.end()
    }
}

/// A [`TileMaps`] as it arrives off the wire: names to `[pos, value]` pairs.
type WireTileMaps<V> = BTreeMap<String, Vec<(Pos, V)>>;

/// The other half of [`TileMaps`]: pair lists back into tile-keyed maps.
fn tile_maps_from<V>(wire: WireTileMaps<V>) -> DashMap<String, BTreeMap<Pos, V>> {
    wire.into_iter()
        .map(|(name, pairs)| (name, pairs.into_iter().collect()))
        .collect()
}

impl Serialize for EntityGraph {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("EntityGraph", 11)?;
        state.serialize_field("entity_graph", &*self.entity_graph.read())?;
        state.serialize_field("blocked_tree", &*self.blocked_tree.read())?;
        state.serialize_field("entity_tree", &*self.entity_tree.read())?;
        state.serialize_field("tile_tree", &*self.tile_tree.read())?;
        state.serialize_field("entity_nodes", &self.entity_nodes)?;
        state.serialize_field("entity_prototypes", &*self.entity_prototypes)?;
        state.serialize_field("recipes", &*self.recipes)?;
        state.serialize_field("resources", &TileMaps(&self.resources))?;
        state.serialize_field("resource_tree", &*self.resource_tree.read())?;
        state.serialize_field("minables", &TileMaps(&self.minables))?;
        state.serialize_field("threats", &TileMaps(&self.threats))?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for EntityGraph {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            EntityGraph,
            BlockedTree,
            EntityTree,
            TileTree,
            EntityNodes,
            EntityPrototypes,
            Recipes,
            Resources,
            ResourceTree,
            Minables,
            Threats,
        }

        // This part could also be generated independently by:
        //
        //    #[derive(Deserialize)]
        //    #[serde(field_identifier, rename_all = "lowercase")]
        //    enum Field { Secs, Nanos }
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
                            "entity_graph" => Ok(Field::EntityGraph),
                            "blocked_tree" => Ok(Field::BlockedTree),
                            "entity_tree" => Ok(Field::EntityTree),
                            "tile_tree" => Ok(Field::TileTree),
                            "entity_nodes" => Ok(Field::EntityNodes),
                            "entity_prototypes" => Ok(Field::EntityPrototypes),
                            "recipes" => Ok(Field::Recipes),
                            "resources" => Ok(Field::Resources),
                            "resource_tree" => Ok(Field::ResourceTree),
                            "minables" => Ok(Field::Minables),
                            "threats" => Ok(Field::Threats),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct EntityGraphVisitor;

        impl<'de> Visitor<'de> for EntityGraphVisitor {
            type Value = EntityGraph;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct EntityGraph")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut entity_graph = None;
                let mut blocked_tree = None;
                let mut entity_tree = None;
                let mut tile_tree = None;
                let mut entity_nodes = None;
                let mut entity_prototypes = None;
                let mut recipes = None;
                let mut resources: Option<WireTileMaps<Option<u32>>> = None;
                let mut resource_tree = None;
                let mut minables: Option<WireTileMaps<Position>> = None;
                let mut threats: Option<WireTileMaps<Position>> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::EntityGraph => {
                            if entity_graph.is_some() {
                                return Err(de::Error::duplicate_field("entity_graph"));
                            }
                            entity_graph = Some(map.next_value()?);
                        }
                        Field::BlockedTree => {
                            if blocked_tree.is_some() {
                                return Err(de::Error::duplicate_field("blocked_tree"));
                            }
                            blocked_tree = Some(map.next_value()?);
                        }
                        Field::EntityTree => {
                            if entity_tree.is_some() {
                                return Err(de::Error::duplicate_field("entity_tree"));
                            }
                            entity_tree = Some(map.next_value()?);
                        }
                        Field::TileTree => {
                            if tile_tree.is_some() {
                                return Err(de::Error::duplicate_field("tile_tree"));
                            }
                            tile_tree = Some(map.next_value()?);
                        }
                        Field::EntityNodes => {
                            if entity_nodes.is_some() {
                                return Err(de::Error::duplicate_field("entity_nodes"));
                            }
                            entity_nodes = Some(map.next_value()?);
                        }
                        Field::EntityPrototypes => {
                            if entity_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("entity_prototypes"));
                            }
                            entity_prototypes = Some(map.next_value()?);
                        }
                        Field::Recipes => {
                            if recipes.is_some() {
                                return Err(de::Error::duplicate_field("recipes"));
                            }
                            recipes = Some(map.next_value()?);
                        }
                        Field::Resources => {
                            if resources.is_some() {
                                return Err(de::Error::duplicate_field("resources"));
                            }
                            resources = Some(map.next_value()?);
                        }
                        Field::ResourceTree => {
                            if resource_tree.is_some() {
                                return Err(de::Error::duplicate_field("resource_tree"));
                            }
                            resource_tree = Some(map.next_value()?);
                        }
                        Field::Minables => {
                            if minables.is_some() {
                                return Err(de::Error::duplicate_field("minables"));
                            }
                            minables = Some(map.next_value()?);
                        }
                        Field::Threats => {
                            if threats.is_some() {
                                return Err(de::Error::duplicate_field("threats"));
                            }
                            threats = Some(map.next_value()?);
                        }
                    }
                }
                let entity_graph =
                    entity_graph.ok_or_else(|| de::Error::missing_field("entity_graph"))?;
                let blocked_tree =
                    blocked_tree.ok_or_else(|| de::Error::missing_field("blocked_tree"))?;
                let entity_tree =
                    entity_tree.ok_or_else(|| de::Error::missing_field("entity_tree"))?;
                let tile_tree = tile_tree.ok_or_else(|| de::Error::missing_field("tile_tree"))?;
                let entity_nodes =
                    entity_nodes.ok_or_else(|| de::Error::missing_field("entity_nodes"))?;
                let entity_prototypes = entity_prototypes
                    .ok_or_else(|| de::Error::missing_field("entity_prototypes"))?;
                let recipes = recipes.ok_or_else(|| de::Error::missing_field("recipes"))?;
                let resources =
                    tile_maps_from(resources.ok_or_else(|| de::Error::missing_field("resources"))?);
                let resource_tree =
                    resource_tree.ok_or_else(|| de::Error::missing_field("resource_tree"))?;
                // Defaulted rather than required, unlike every field above it.
                // Every graph serialised before this map existed is still a
                // valid graph -- it just knows of no trees -- and refusing to
                // load one would turn a new planner capability into a failure
                // to read yesterday's snapshot.
                let minables = tile_maps_from(minables.unwrap_or_default());
                // Defaulted for the same reason `minables` is: every graph
                // serialised before this map existed is a valid graph that
                // knows of no nests.
                let threats = tile_maps_from(threats.unwrap_or_default());

                Ok(EntityGraph {
                    // Not a serialised field: a loaded graph is generation 0
                    // and every `FlowGraph` reading it rebuilds on its first
                    // read, because `FlowGraph`'s own counter starts at a
                    // sentinel no generation can equal.
                    generation: AtomicU64::new(0),
                    entity_graph: RwLock::new(entity_graph),
                    blocked_tree: RwLock::new(blocked_tree),
                    entity_tree: RwLock::new(entity_tree),
                    tile_tree: RwLock::new(tile_tree),
                    entity_nodes,
                    entity_prototypes: Arc::new(entity_prototypes),
                    recipes: Arc::new(recipes),
                    resources,
                    resource_tree: RwLock::new(resource_tree),
                    minables,
                    threats,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "entity_graph",
            "blocked_tree",
            "entity_tree",
            "tile_tree",
            "entity_nodes",
            "entity_prototypes",
            "recipes",
            "resources",
            "resource_tree",
            "minables",
            "threats",
        ];
        deserializer.deserialize_struct("EntityGraph", FIELDS, EntityGraphVisitor)
    }
}

impl Clone for EntityGraph {
    fn clone(&self) -> Self {
        EntityGraph {
            generation: AtomicU64::new(self.generation()),
            entity_graph: RwLock::new(self.entity_graph.read().clone()),
            blocked_tree: RwLock::new(self.blocked_tree.read().clone()),
            entity_tree: RwLock::new(self.entity_tree.read().clone()),
            tile_tree: RwLock::new(self.tile_tree.read().clone()),
            entity_nodes: self.entity_nodes.clone(),
            entity_prototypes: Arc::new((*self.entity_prototypes).clone()),
            recipes: Arc::new((*self.recipes).clone()),
            resources: self.resources.clone(),
            resource_tree: RwLock::new(self.resource_tree.read().clone()),
            minables: self.minables.clone(),
            threats: self.threats.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.generation = AtomicU64::new(source.generation());
        self.entity_graph = RwLock::new(source.entity_graph.read().clone());
        self.blocked_tree = RwLock::new(source.blocked_tree.read().clone());
        self.entity_tree = RwLock::new(source.entity_tree.read().clone());
        self.tile_tree = RwLock::new(source.tile_tree.read().clone());
        self.entity_nodes = source.entity_nodes.clone();
        self.entity_prototypes = Arc::new((*source.entity_prototypes).clone());
        self.recipes = Arc::new((*source.recipes).clone());
        self.resources = source.resources.clone();
        self.resource_tree = RwLock::new(source.resource_tree.read().clone());
        self.minables = source.minables.clone();
        self.threats = source.threats.clone();
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub bounding_box: Rect,
    pub position: Position,
    pub direction: Direction,
    pub entity_name: String,
    pub entity_type: EntityType,
    pub entity_id: Option<ItemId>,
    pub miner_ore: Option<String>,
}

impl std::fmt::Display for EntityNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{}: ", miner_ore)
            } else {
                String::new()
            },
            self.entity_type,
            self.position
        ))?;
        Ok(())
    }
}
impl std::fmt::Debug for EntityNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{} ", miner_ore)
            } else {
                String::new()
            },
            self.entity_type,
            self.position
        ))?;
        Ok(())
    }
}

impl EntityNode {
    /// `None` when the game reports something this build cannot represent.
    ///
    /// `Direction` now covers all sixteen values of 2.x's `defines.direction`,
    /// so `from_u8` is total over `0..=15` and this arm only fires if the game
    /// sends 16 or above. It stays because unwrapping it aborted the process --
    /// this crate builds `panic = "abort"` -- which let one belt facing a
    /// 2.x-only direction kill the bot back when the enum stopped at 7. The
    /// same reasoning as 04f8e76f/0cb7636f applies: a game whose schema drifts
    /// must not be able to crash us. Report loudly and skip.
    ///
    /// Skipping rather than defaulting is deliberate. Defaulting to `North`
    /// would put a node with a fabricated orientation into the graph, and the
    /// graph's whole job is to answer "what feeds what" from orientation -- a
    /// wrong answer nobody can see is worse than a missing one somebody can.
    pub fn new(
        entity: FactorioEntity,
        miner_ore: Option<String>,
        entity_id: ItemId,
    ) -> Option<EntityNode> {
        let Some(direction) = Direction::from_u8(entity.direction) else {
            error!(
                "<red>unreadable direction</> <bright-blue>{}</> on <bright-blue>{}</> at <bright-blue>{}</>: defines.direction is 0..=15 -- entity skipped",
                entity.direction, entity.name, entity.position
            );
            return None;
        };
        let Ok(entity_type) = EntityType::from_str(&entity.entity_type) else {
            error!(
                "<red>unknown entity type</> <bright-blue>{}</> on <bright-blue>{}</> at <bright-blue>{}</> -- entity skipped",
                entity.entity_type, entity.name, entity.position
            );
            return None;
        };
        Some(EntityNode {
            position: entity.position.clone(),
            bounding_box: entity.bounding_box.clone(),
            direction,
            miner_ore,
            entity_id: Some(entity_id),
            entity_name: entity.name,
            entity_type,
        })
    }
}

pub type EntityGraphInner = StableGraph<EntityNode, f64>;

pub type QuadTreeRect = EuclidRect<f32, Rect>;
pub type BlockedQuadTree = QuadTree<bool, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type EntityQuadTree = QuadTree<FactorioEntity, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type TileQuadTree = QuadTree<FactorioTile, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type ResourceQuadTree = QuadTree<String, Rect, [(ItemId, QuadTreeRect); 4]>;

/// Whether a box the quad tree handed back genuinely overlaps `bounds`, rather
/// than merely abutting one of its edges.
///
/// Strict on every side, deliberately. The quad tree's own predicate is not:
/// `my_intersects` (`crate::aabb_quadtree`) falls back on `euclid`'s
/// `Rect::contains`, which is half-open (`min <= p < max`), so it admits a box
/// touching the query's left or top edge and rejects the mirror image on the
/// right or bottom. Touching is not overlapping in either direction, and a
/// predicate that says so on two sides and not the other two cannot be
/// compared against anything.
///
/// `bounds` is `f64` and the tree is `f32`; the comparison happens in `f32`,
/// which is the precision the boxes were stored at, so a coordinate is never
/// widened into a difference that was not in the tree to begin with.
#[allow(clippy::cast_possible_truncation)]
fn overlaps_bounds(bounds: &Rect, rect: &QuadTreeRect) -> bool {
    let left = bounds.left_top.x() as f32;
    let top = bounds.left_top.y() as f32;
    let right = bounds.right_bottom.x() as f32;
    let bottom = bounds.right_bottom.y() as f32;
    let min_x = rect.origin.x;
    let min_y = rect.origin.y;
    let max_x = min_x + rect.size.width;
    let max_y = min_y + rect.size.height;
    min_x < right && left < max_x && min_y < bottom && top < max_y
}

#[cfg(test)]
mod tests {
    use crate::factorio::util::rect_fields;
    use crate::num_traits::ToPrimitive;
    use crate::test_utils::{
        entity_graph_from, fixture_entity_prototypes, fixture_recipes, fixture_world, spawn_ore,
    };

    use super::*;

    /// Unwrapping `Direction::from_u8` aborted the process (`panic = "abort"`),
    /// so one belt facing a direction we could not read killed the bot. Skip
    /// the entity loudly instead.
    ///
    /// This used **12**, which was out of range on the Factorio 1.x scale and
    /// is `West` on the 2.x one. 16 is the first value still outside
    /// `defines.direction`, so it is what keeps this path covered.
    /// A helper naming an edge by the two tiles it runs between, so a failure
    /// says *which* connection is missing rather than "3 != 4".
    fn has_edge(graph: &EntityGraph, from: &Position, to: &Position) -> bool {
        let (Some(a), Some(b)) = (graph.node_at(from), graph.node_at(to)) else {
            return false;
        };
        graph.inner_graph().contains_edge(a, b)
    }

    /// Every edge in the graph, as `(from tile, to tile)` pairs, sorted -- the
    /// comparable form of "what this graph is wired like".
    fn edge_set(graph: &EntityGraph) -> Vec<(Pos, Pos)> {
        let inner = graph.inner_graph();
        let mut edges: Vec<(Pos, Pos)> = inner
            .edge_indices()
            .map(|edge| {
                let (source, target) = inner.edge_endpoints(edge).unwrap();
                (
                    (&inner.node_weight(source).unwrap().position).into(),
                    (&inner.node_weight(target).unwrap().position).into(),
                )
            })
            .collect();
        edges.sort();
        edges
    }

    /// **What the wiring costs on the largest world available offline**, and
    /// whether the incremental sweep agrees with the full one at that size.
    ///
    /// Ignored by default and gated on `FACTORIO_BOT_WORLD_DUMP` naming a
    /// `world.dump` JSON, like its siblings in `flow_graph.rs`: the dump it was
    /// built for is ~2.9 GB and lives in a workspace, not in the repository.
    ///
    /// ```text
    /// FACTORIO_BOT_WORLD_DUMP=workspace/wrload/scripts/wr-census-status.json \
    ///   cargo test -p factorio-bot-core --release \
    ///   the_cost_of_wiring_a_recorded_base -- --ignored --nocapture
    /// ```
    ///
    /// It replays the dump's entities back through `add` in chunk-sized
    /// batches -- the shape the parser delivers them in during a live run --
    /// and then runs one full `connect` on top. **The edges the full sweep
    /// still finds is the number that matters**: zero means the incremental
    /// wiring is complete on a real base, not only on this file's fixtures.
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn the_cost_of_wiring_a_recorded_base() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };
        let batch: usize = std::env::var("FACTORIO_BOT_WIRING_BATCH")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(50);

        let read_started = Instant::now();
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        drop(json);
        let loaded = read_started.elapsed();

        let source = surface.entity_graph.clone();
        let entities: Vec<FactorioEntity> = {
            let inner = source.entity_graph.read();
            let tree = source.entity_tree.read();
            inner
                .node_indices()
                .filter_map(|index| inner.node_weight(index))
                .filter_map(|node| node.entity_id)
                .filter_map(|id| tree.get(id).cloned())
                .collect()
        };

        let graph = EntityGraph::new(source.entity_prototypes.clone(), source.recipes.clone());
        let replay_started = Instant::now();
        for chunk in entities.chunks(batch) {
            graph.add(chunk.to_vec(), None).expect("the replay adds");
        }
        let replayed = replay_started.elapsed();
        let edges_after_replay = graph.entity_graph.read().edge_count();

        let sweep_started = Instant::now();
        graph.connect().expect("the sweep connects");
        let swept = sweep_started.elapsed();
        let edges_after_sweep = graph.entity_graph.read().edge_count();

        println!("-- wiring cost of {path} --");
        println!("  load                {loaded:?}");
        println!("  entities replayed   {}", entities.len());
        println!("  batch size          {batch}");
        println!(
            "  nodes               {}",
            graph.entity_graph.read().node_count()
        );
        println!("  replay (add x N)    {replayed:?}");
        println!("  edges after replay  {edges_after_replay}");
        println!("  one full connect    {swept:?}");
        println!("  edges after sweep   {edges_after_sweep}");
        println!(
            "  edges the sweep still found: {}",
            edges_after_sweep - edges_after_replay
        );
        // `entity_at` answers `results[0]` when a 0.1-tile query hits more than
        // one entity, and a quad tree's result order is not a promise. So an
        // ambiguous tile can resolve to a different neighbour depending on how
        // populated the tree was when it was asked -- which is exactly the
        // difference between wiring during the replay and wiring after it. This
        // counts them, so a one-edge disagreement between the two orders has a
        // named candidate rather than a shrug.
        let ambiguous = {
            let tree = graph.entity_tree.read();
            entities
                .iter()
                .filter(|entity| {
                    tree.query(add_to_rect(&Rect::from_wh(0.1, 0.1), &entity.position).into())
                        .len()
                        > 1
                })
                .count()
        };
        println!("  positions where `entity_at` is ambiguous: {ambiguous}");
    }

    /// **What the double filing costs on the largest world available offline.**
    ///
    /// Ignored by default and gated on `FACTORIO_BOT_WORLD_DUMP`, like its
    /// sibling above: the dump it was built for is ~2.9 GB and lives in a
    /// workspace, not in the repository.
    ///
    /// ```text
    /// FACTORIO_BOT_WORLD_DUMP=workspace/wrload/scripts/wr-census-status.json \
    ///   cargo test -p factorio-bot-core --release \
    ///   what_the_blocked_tree_costs_at_world_record_scale -- --ignored --nocapture
    /// ```
    ///
    /// **It measures a dump written by the OLD code**, which is the point: the
    /// tree it deserialises is exactly what a live run built, duplicates and
    /// all. Re-filing every one of those boxes through [`file_blocked_box`] --
    /// in id order, so the run is repeatable -- gives the count the same world
    /// would produce today. The difference between the two is what the defect
    /// was costing.
    ///
    /// The bytes are a **lower bound** and are reported as one: each element
    /// occupies one entry in the tree's `elements` map and at least one
    /// `(ItemId, Rect)` in a node, and a box straddling a node boundary is
    /// filed in more than one node.
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn what_the_blocked_tree_costs_at_world_record_scale() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };

        let read_started = Instant::now();
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        drop(json);
        let loaded = read_started.elapsed();

        let mut filed: Vec<(ItemId, bool, QuadTreeRect)> = {
            let tree = surface.entity_graph.blocked_tree();
            tree.iter()
                .map(|(id, (minable, rect))| (*id, *minable, *rect))
                .collect()
        };
        // `iter` walks a HashMap, whose order is not a promise. Id order is
        // insertion order, which is the order a live run filed them in.
        filed.sort_by_key(|(id, _, _)| *id);

        let mut refiled: BlockedQuadTree = QuadTree::new(
            QuadTreeRect::new(Point2D::new(-5120., -5120.), Size2D::new(10240., 10240.)),
            true,
            8,
            64,
            1024,
            8,
        );
        let refile_started = Instant::now();
        for (_, minable, rect) in &filed {
            file_blocked_box(&mut refiled, *minable, *rect);
        }
        let refiled_in = refile_started.elapsed();

        let before = filed.len();
        let after = refiled.len();
        const PER_ELEMENT: usize = std::mem::size_of::<(ItemId, (bool, QuadTreeRect))>()
            + std::mem::size_of::<(ItemId, QuadTreeRect)>();

        println!("-- blocked tree of {path} --");
        println!("  load                    {loaded:?}");
        println!("  boxes as the dump has them   {before}");
        println!("  boxes after deduplication    {after}");
        println!("  duplicates                   {}", before - after);
        println!("  re-file cost                 {refiled_in:?}");
        println!(
            "  at >= {PER_ELEMENT} bytes each, that is >= {} KiB reclaimed",
            (before - after) * PER_ELEMENT / 1024
        );
    }

    /// **The defect this branch exists for.** `add` inserted nodes and stopped;
    /// `connect` ran from `OutputParser::on_init` (once, at `initial discovery
    /// done`), `factorio::snapshot` and `FactorioSurface::import` and nowhere
    /// else. So a belt a *run* built was a node with no edges, for ever.
    ///
    /// Asserted on the **edge**, not on the node count, because the node was
    /// never the thing that was missing.
    #[test]
    fn an_entity_added_after_the_graph_was_connected_is_wired_to_it() {
        // `entity_graph_from` adds and connects: this is the world as it stands
        // at `initial discovery done`.
        let graph = entity_graph_from(vec![FactorioEntity::new_transport_belt(
            &Position::new(0.5, 0.5),
            Direction::South,
        )])
        .unwrap();

        // ... and this is a run building one more belt, hours later.
        graph
            .add(
                vec![FactorioEntity::new_transport_belt(
                    &Position::new(0.5, 1.5),
                    Direction::South,
                )],
                None,
            )
            .unwrap();

        assert!(
            has_edge(&graph, &Position::new(0.5, 0.5), &Position::new(0.5, 1.5)),
            "the standing belt must feed the one built after it"
        );
    }

    /// The half that makes this more than "connect the new node": the edge is
    /// drawn while visiting its **source**, so a belt built *upstream* of one
    /// that was already standing needs the new node visited, and a belt built
    /// *downstream* needs the standing one re-visited. Both directions, one
    /// test, because getting only the easy one is the plausible half-fix.
    #[test]
    fn a_later_entity_is_wired_from_both_sides() {
        let graph = entity_graph_from(vec![FactorioEntity::new_transport_belt(
            &Position::new(0.5, 1.5),
            Direction::South,
        )])
        .unwrap();

        graph
            .add(
                vec![
                    // upstream of the standing belt
                    FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                    // downstream of it
                    FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
                ],
                None,
            )
            .unwrap();

        assert!(
            has_edge(&graph, &Position::new(0.5, 0.5), &Position::new(0.5, 1.5)),
            "the new belt above must feed the standing one (source is new)"
        );
        assert!(
            has_edge(&graph, &Position::new(0.5, 1.5), &Position::new(0.5, 2.5)),
            "the standing belt must feed the new one below (source was already standing)"
        );
    }

    /// A drill's ore lands on a belt the run lays afterwards. This is the
    /// `drop_position` rule rather than the belt rule, and it is the shape a
    /// real cell is built in: the machine first, the logistics after.
    #[test]
    fn a_drop_position_finds_a_target_built_after_the_machine() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
        ])
        .unwrap();

        graph
            .add(
                vec![FactorioEntity::new_transport_belt(
                    &Position::new(0.5, 0.5),
                    Direction::South,
                )],
                None,
            )
            .unwrap();

        assert!(
            has_edge(&graph, &Position::new(0.5, -1.5), &Position::new(0.5, 0.5)),
            "the drill must drop onto the belt built after it"
        );
    }

    /// The long-range rule, which is why the neighbourhood is not a tile or
    /// two: an underground belt pairs with its other half up to its
    /// prototype's `max_underground_distance` away.
    ///
    /// Arranged so the **standing** half is the one that draws the edge --
    /// `connect_node` searches backwards from the exit, so building the
    /// *entrance* last is the case that needs a node four tiles away
    /// re-visited. Building the exit last would pass on the new node alone and
    /// prove nothing about the reach.
    #[test]
    fn an_underground_pair_is_found_across_its_whole_span() {
        use crate::blueprint::UndergroundHalf;
        let graph = entity_graph_from(vec![FactorioEntity::new_underground_belt(
            &Position::new(0.5, 4.5),
            Direction::South,
            UndergroundHalf::Output,
        )])
        .unwrap();

        graph
            .add(
                vec![FactorioEntity::new_underground_belt(
                    &Position::new(0.5, 0.5),
                    Direction::South,
                    UndergroundHalf::Input,
                )],
                None,
            )
            .unwrap();

        assert!(
            has_edge(&graph, &Position::new(0.5, 0.5), &Position::new(0.5, 4.5)),
            "the entrance built afterwards must reach the standing exit"
        );
    }

    /// **The oracle.** Incremental wiring is only worth anything if it agrees
    /// with the full sweep, so build the same layout three ways and compare
    /// the edge sets: all at once, one entity per `add`, and the old
    /// add-then-`connect`.
    ///
    /// Built in a deliberately awkward order -- downstream before upstream,
    /// the underground exit before its entrance -- because an incremental
    /// scheme that only ever wires forwards passes a layout built in order.
    #[test]
    fn wiring_one_entity_at_a_time_agrees_with_one_full_sweep() {
        use crate::blueprint::UndergroundHalf;
        let layout = || {
            vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::IronOre.to_string(),
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 6.5), Direction::South),
                FactorioEntity::new_underground_belt(
                    &Position::new(0.5, 5.5),
                    Direction::South,
                    UndergroundHalf::Output,
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_underground_belt(
                    &Position::new(0.5, 1.5),
                    Direction::South,
                    UndergroundHalf::Input,
                ),
            ]
        };

        let all_at_once = {
            let graph = EntityGraph::new(
                Arc::new(fixture_entity_prototypes()),
                Arc::new(DashMap::new()),
            );
            graph.add(layout(), None).unwrap();
            edge_set(&graph)
        };

        let one_at_a_time = {
            let graph = EntityGraph::new(
                Arc::new(fixture_entity_prototypes()),
                Arc::new(DashMap::new()),
            );
            for entity in layout() {
                graph.add(vec![entity], None).unwrap();
            }
            edge_set(&graph)
        };

        let full_sweep = {
            let graph = EntityGraph::new(
                Arc::new(fixture_entity_prototypes()),
                Arc::new(DashMap::new()),
            );
            graph.add(layout(), None).unwrap();
            graph.connect().unwrap();
            edge_set(&graph)
        };

        assert!(
            full_sweep.len() >= 4,
            "the fixture must actually wire something, or this compares two empties: {full_sweep:?}"
        );
        assert_eq!(
            one_at_a_time, full_sweep,
            "wiring one entity per add must give the same edges as one sweep"
        );
        assert_eq!(all_at_once, full_sweep, "and so must one batched add");
    }

    /// **The one place the append-only sweep disagrees with itself**, found by
    /// replaying a 39,191-entity world-record base through `add` and comparing
    /// the result against one full `connect`: 41,670 edges against 41,669.
    /// This is that single edge, reduced to three entities.
    ///
    /// `connect_node`'s underground arm pairs a half with the **nearest**
    /// matching half behind it and stops there. Build the far pair first and it
    /// is drawn; drop a third half into the gap afterwards and the two short
    /// pairs are drawn as well -- but nothing removes the long one, because
    /// this graph only ever appends. A world built in one sweep has two edges
    /// here; a world built in the order a run builds it has three.
    ///
    /// **This is asserted as a known divergence, not as correct behaviour.**
    /// It is strictly better than what it replaces -- before 2026-09-07 a
    /// run-built belt had no edges at all -- and it is bounded: it needs an
    /// underground half placed *between* an already-paired one, which severs
    /// the pair in the game. Whoever removes the stale edge should delete this
    /// test rather than update it. See
    /// `docs/superpowers/notes/2026-09-07-edges-that-outlive-tick-zero.md`.
    #[test]
    fn a_half_dropped_into_a_tunnel_leaves_the_long_pair_behind() {
        use crate::blueprint::UndergroundHalf;
        let entrance = || {
            FactorioEntity::new_underground_belt(
                &Position::new(0.5, 0.5),
                Direction::South,
                UndergroundHalf::Input,
            )
        };
        let middle = || {
            FactorioEntity::new_underground_belt(
                &Position::new(0.5, 2.5),
                Direction::South,
                UndergroundHalf::Output,
            )
        };
        let exit = || {
            FactorioEntity::new_underground_belt(
                &Position::new(0.5, 4.5),
                Direction::South,
                UndergroundHalf::Output,
            )
        };

        // Built in the order a run builds one: the long tunnel, then a half
        // dropped into its gap.
        let incrementally = EntityGraph::new(
            Arc::new(fixture_entity_prototypes()),
            Arc::new(DashMap::new()),
        );
        incrementally.add(vec![entrance()], None).unwrap();
        incrementally.add(vec![exit()], None).unwrap();
        incrementally.add(vec![middle()], None).unwrap();

        // The same three entities, wired in one sweep.
        let in_one_sweep = EntityGraph::new(
            Arc::new(fixture_entity_prototypes()),
            Arc::new(DashMap::new()),
        );
        in_one_sweep
            .add(vec![entrance(), exit(), middle()], None)
            .unwrap();

        let long_pair = (Pos(0, 0), Pos(0, 4));
        assert!(
            !edge_set(&in_one_sweep).contains(&long_pair),
            "one sweep pairs each half with its nearest neighbour only: {:?}",
            edge_set(&in_one_sweep)
        );
        assert!(
            edge_set(&incrementally).contains(&long_pair),
            "and the incremental order keeps the pair it drew before the gap was filled: {:?}",
            edge_set(&incrementally)
        );
        assert_eq!(
            edge_set(&incrementally).len(),
            edge_set(&in_one_sweep).len() + 1,
            "exactly one stale edge, not a cascade"
        );
    }

    /// Question two of three about re-running the wiring: **it dedupes**. Every
    /// candidate is guarded by `contains_edge` when gathered and again when
    /// applied, so a second sweep over an unchanged world is a no-op. This is
    /// what makes calling it from `add` safe at all.
    #[test]
    fn connecting_an_unchanged_world_twice_adds_no_edges() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
        ])
        .unwrap();
        let before = edge_set(&graph);
        assert!(!before.is_empty(), "the fixture must have wired something");

        for _ in 0..3 {
            graph.connect().unwrap();
        }

        assert_eq!(
            edge_set(&graph),
            before,
            "re-running the sweep must not duplicate an edge"
        );
    }

    /// Question three: **`remove` deletes**, which is why the append-only
    /// sweep above is sound and why this graph needs no rebuild-from-scratch.
    /// `FlowGraph::update` had to clear precisely because nothing deleted for
    /// it, and because its `node_at` matched on position alone. Here `remove`
    /// takes the node, both directions of its edges and the `entity_tree` entry
    /// `node_at` resolves through, so a tile reused by something else cannot
    /// inherit the old node.
    #[test]
    fn removing_an_entity_takes_its_edges_and_frees_its_tile() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(edge_set(&graph).len(), 2, "a three-belt run has two edges");

        let middle = FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South);
        graph.remove(&middle).unwrap();

        assert_eq!(
            edge_set(&graph),
            vec![],
            "both of the middle belt's edges must go with it"
        );
        assert_eq!(graph.node_at(&Position::new(0.5, 1.5)), None);

        // And the freed tile takes a different entity, which gets its own node
        // rather than inheriting the belt's.
        graph
            .add(
                vec![FactorioEntity::new_inserter(
                    &Position::new(0.5, 1.5),
                    Direction::South,
                )],
                None,
            )
            .unwrap();
        let node = graph.node_at(&Position::new(0.5, 1.5)).unwrap();
        assert_eq!(
            graph.inner_graph().node_weight(node).unwrap().entity_name,
            "inserter",
            "the tile's new occupant is an inserter, not the belt that stood there"
        );
    }

    #[test]
    fn an_entity_whose_direction_cannot_be_read_is_skipped_not_aborted() {
        let mut belt =
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::North);
        belt.direction = 16;
        let graph = entity_graph_from(vec![belt]).expect("adding must not fail");
        assert_eq!(
            graph.inner_graph().node_count(),
            0,
            "an entity we cannot orient must not enter the graph"
        );
    }

    /// The other pumpjack abort: a direction this build *can* read but that has
    /// no drop-position offset (any diagonal) hit `panic!("invalid pumpjack
    /// position")`. Skipping keeps the uncorrected drop position -- which the
    /// game reports at the pumpjack's own position -- out of the graph.
    #[test]
    fn a_pumpjack_facing_a_diagonal_is_skipped_not_aborted() {
        let mut pumpjack =
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, 0.5), Direction::North);
        pumpjack.name = EntityName::Pumpjack.to_string();
        pumpjack.direction = Direction::SouthWest.to_u8().expect("fits in a u8");
        let graph = entity_graph_from(vec![pumpjack]).expect("adding must not fail");
        assert_eq!(
            graph.inner_graph().node_count(),
            0,
            "a pumpjack with no usable drop position must not enter the graph"
        );
    }

    /// The pumpjack drop-position fixup unwrapped the same `from_u8` and then
    /// `panic!`ed on anything non-orthogonal. Both arms aborted.
    #[test]
    fn a_pumpjack_whose_direction_cannot_be_read_is_skipped_not_aborted() {
        let mut pumpjack =
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, 0.5), Direction::North);
        pumpjack.name = EntityName::Pumpjack.to_string();
        pumpjack.direction = 16;
        let graph = entity_graph_from(vec![pumpjack]).expect("adding must not fail");
        assert_eq!(
            graph.inner_graph().node_count(),
            0,
            "an entity we cannot orient must not enter the graph"
        );
    }

    /// The entity tree cannot answer "is this ground buildable" and never
    /// could: `add` only admits a whitelist of factory entity types. A tree is
    /// invisible there and present in the blocked tree, which is the whole
    /// reason [`EntityGraph::blocking_boxes_within`] exists.
    #[test]
    fn a_tree_is_a_blocking_box_even_though_the_entity_tree_never_sees_it() {
        let tree = FactorioEntity::new_tree(&Position::new(3.5, 3.5));
        let graph = entity_graph_from(vec![tree.clone()]).expect("adding must not fail");
        let around = Rect::new(&Position::new(3., 3.), &Position::new(4., 4.));

        assert!(
            graph
                .find_entities_in_radius(Position::new(3.5, 3.5), 4., None, None)
                .is_empty(),
            "a tree is not a modelled entity"
        );
        let boxes = graph.blocking_boxes_within(&around);
        assert_eq!(boxes.len(), 1, "but it does block the ground: {boxes:?}");
        // Snapped to the 1/256 grid, so the edges are the nearest representable
        // ones rather than the fixture's decimal 0.8 -- but the centre, which
        // is what a caller keys by tile, comes back exact.
        assert_eq!(boxes[0].center(), tree.position);
        for (got, want) in [
            (boxes[0].left_top.x(), tree.bounding_box.left_top.x()),
            (boxes[0].left_top.y(), tree.bounding_box.left_top.y()),
            (
                boxes[0].right_bottom.x(),
                tree.bounding_box.right_bottom.x(),
            ),
            (
                boxes[0].right_bottom.y(),
                tree.bounding_box.right_bottom.y(),
            ),
        ] {
            assert!(
                (got - want).abs() <= 1. / 256.,
                "edge {got} is more than one position step from {want}"
            );
        }
    }

    /// **The same chunk written out twice files each box once.**
    ///
    /// The mod calls `writeout_entities` from `on_chunk_generated`, and
    /// `initial_discovery` calls `on_chunk_generated` again by hand for every
    /// chunk that already existed when the server came up -- unguarded, unlike
    /// the tile writeout beside it, which `tile_chunks` gates. So a chunk
    /// reaches [`EntityGraph::add`] twice and every box in it used to be filed
    /// twice: measured live as **26 boxes for 13 trees** over one wooded
    /// rectangle of a clean seed-31337 map.
    ///
    /// The second `add` here is that replay, not a contrivance: it is the same
    /// entity, in the same shape, arriving a second time.
    #[test]
    fn a_chunk_written_out_twice_files_each_box_once() {
        let tree = FactorioEntity::new_tree(&Position::new(3.5, 3.5));
        let graph = entity_graph_from(vec![tree.clone()]).expect("adding must not fail");
        graph
            .add(vec![tree.clone()], None)
            .expect("the discovery replay must not fail");

        let boxes =
            graph.blocking_boxes_within(&Rect::new(&Position::new(3., 3.), &Position::new(4., 4.)));
        assert_eq!(
            boxes.len(),
            1,
            "one tree written out twice is one obstacle, not two: {boxes:?}"
        );
    }

    /// An empty blocked tree shaped exactly like [`EntityGraph::new`]'s.
    fn empty_blocked_tree(allow_duplicates: bool) -> BlockedQuadTree {
        QuadTree::new(
            QuadTreeRect::new(Point2D::new(-5120., -5120.), Size2D::new(10240., 10240.)),
            allow_duplicates,
            8,
            64,
            1024,
            8,
        )
    }

    /// The water arm of the same fault, which `add_tiles` cannot be made to
    /// exercise: `tile_tree` refuses the duplicate tile first, and refusing is
    /// a panic (see the test below). The blocked insert happens *before* that
    /// refusal, so this arm is real in a release build -- it is only untestable
    /// through the public entry point.
    #[test]
    fn filing_one_box_twice_files_it_once() {
        let mut tree = empty_blocked_tree(true);
        let water = QuadTreeRect::new(Point2D::new(-70., 42.), Size2D::new(1., 1.));
        file_blocked_box(&mut tree, false, water);
        file_blocked_box(&mut tree, false, water);
        assert_eq!(tree.len(), 1, "one tile is one obstacle");
    }

    /// **Why this is not the quad tree's own `allow_duplicates: false`**, which
    /// is how `entity_tree` and `tile_tree` are built.
    ///
    /// That flag does not *skip* a duplicate. `QuadNode::insert` panics
    /// outright when no node accepted the item, in release as well as debug,
    /// and `[profile.release]` sets `panic = "abort"`. So flipping the flag on
    /// `blocked_tree` would not have deduplicated the replayed chunk -- it
    /// would have killed the run on it.
    #[test]
    #[should_panic(expected = "didn't insert")]
    fn a_deduplicating_quad_tree_panics_on_the_duplicate_rather_than_skipping_it() {
        let mut tree = empty_blocked_tree(false);
        let water = QuadTreeRect::new(Point2D::new(-70., 42.), Size2D::new(1., 1.));
        tree.insert_with_box(false, water);
        tree.insert_with_box(false, water);
    }

    /// **Why this is not keyed by `Pos` the way `resources` is.**
    ///
    /// [`EntityGraph::resources`] fixed the identical double-filing by keying
    /// on the floored tile, and that key is wrong here: a blocked box is
    /// sub-tile and unaligned, so two entirely different trees can stand on one
    /// tile. Collapsing them would erase ground that really is blocked.
    #[test]
    fn two_different_boxes_on_one_tile_are_both_filed() {
        let near = FactorioEntity::new_tree(&Position::new(3.2, 3.2));
        let far = FactorioEntity::new_tree(&Position::new(3.8, 3.8));
        assert_ne!(
            near.bounding_box, far.bounding_box,
            "the fixture must be two different boxes, or it asserts nothing"
        );
        let graph = entity_graph_from(vec![near, far]).expect("adding must not fail");

        let boxes =
            graph.blocking_boxes_within(&Rect::new(&Position::new(3., 3.), &Position::new(4., 4.)));
        assert_eq!(
            boxes.len(),
            2,
            "two obstacles sharing a tile are two obstacles: {boxes:?}"
        );
    }

    /// The `is_minable` bit is part of a box's identity, not decoration.
    ///
    /// [`EntityGraph::blocking_boxes_within_minable`] hands that bit to callers
    /// deciding whether an obstacle can be chopped out of the way. Two claims
    /// over one rectangle -- one minable, one not -- are two different answers
    /// to that question, so both are kept.
    #[test]
    fn one_rectangle_with_two_different_minable_bits_keeps_both() {
        let graph = graph_with_terrain(vec![terrain(10., 10., "water")]);
        let mut rock = FactorioEntity::new_tree(&Position::new(10.5, 10.5));
        rock.bounding_box = Rect::new(&Position::new(10., 10.), &Position::new(11., 11.));
        graph.add(vec![rock], None).expect("adding must not fail");

        let boxes = graph.blocking_boxes_within_minable(&Rect::new(
            &Position::new(10., 10.),
            &Position::new(11., 11.),
        ));
        assert_eq!(boxes.len(), 2, "both claims are kept: {boxes:?}");
        assert!(
            boxes.iter().any(|(_, minable)| *minable) && boxes.iter().any(|(_, minable)| !*minable),
            "and they are kept because they disagree: {boxes:?}"
        );
    }

    /// **The double filing was not merely wasteful: a belt read as a wall.**
    ///
    /// `enclosure::drop_walkable` is a *multiset* subtraction -- it removes one
    /// occurrence per walkable entity, deliberately, so that a coincidence of
    /// geometry cannot delete a real blocker. `entity_tree` deduplicates and
    /// `blocked_tree` did not, so a belt written out twice offered two boxes
    /// against one walkable entity and one survived, standing in the enclosure
    /// grid as an obstacle a character walks straight over.
    ///
    /// This is the reader the fix is *for*; the rest are `any`, `find`,
    /// `fill(true)` and bounding-box queries, which a duplicate cannot move.
    #[test]
    fn a_belt_written_out_twice_does_not_read_as_a_wall() {
        let belt = FactorioEntity::new_transport_belt(&Position::new(3.5, 3.5), Direction::North);
        let graph = entity_graph_from(vec![belt.clone()]).expect("adding must not fail");
        graph
            .add(vec![belt.clone()], None)
            .expect("the discovery replay must not fail");

        let window = Rect::new(&Position::new(2., 2.), &Position::new(5., 5.));
        let standing = crate::graph::enclosure::drop_walkable(
            graph.blocking_boxes_within(&window),
            std::slice::from_ref(&belt.bounding_box),
        );
        assert!(
            standing.is_empty(),
            "a belt is walkable, so nothing may be left blocking after it is \
             dropped: {standing:?}"
        );
    }

    /// A ghost is not an obstacle, and `blocked_tree` used to say it was.
    ///
    /// The defect from
    /// `docs/superpowers/notes/2026-09-06-a-failed-placement-blames-a-tree.md`:
    /// `BuildBlock` stamps ghosts, the mod writes each one out as
    /// `on_some_entity_created`, and `add` filed every entity with a non-zero
    /// box into the blocked tree. The tree keeps no name, so the stamped
    /// ghost came back an anonymous rectangle and the planner reported it as
    /// terrain -- at a tile the game said held nothing but ore.
    ///
    /// The tree is the hostile half of this fixture, not decoration: it sits
    /// one tile away, is added in the same call, and proves the query
    /// actually reaches this ground. Without it the ghost's absence would
    /// also be satisfied by a query that finds nothing anywhere.
    #[test]
    fn a_stamped_ghost_is_not_a_blocking_box() {
        let mut ghost =
            FactorioEntity::new_stone_furnace(&Position::new(3.5, 3.5), Direction::North);
        ghost.name = "entity-ghost".into();
        ghost.entity_type = "entity-ghost".into();
        let tree = FactorioEntity::new_tree(&Position::new(6.5, 6.5));
        let graph = entity_graph_from(vec![ghost.clone(), tree]).expect("adding must not fail");

        assert_eq!(
            graph
                .blocking_boxes_within(&Rect::new(&Position::new(3., 3.), &Position::new(4., 4.)))
                .len(),
            0,
            "a ghost does not collide: a real placement consumes it rather \
             than being refused by it"
        );
        assert_eq!(
            graph
                .blocking_boxes_within(&Rect::new(&Position::new(6., 6.), &Position::new(7., 7.)))
                .len(),
            1,
            "the control tree must be found, or the assertion above is about \
             a query that sees nothing at all"
        );
    }

    /// The one bit `blocked_tree` stores, kept rather than thrown away.
    ///
    /// `blocking_boxes_within` reduced every box to a bare rectangle, which
    /// is why a refusal built on it could only recite "a tree, cliff, rock or
    /// unit" -- four things, of which a reader takes the first, and none of
    /// which it had read. `is_minable` is exactly "type is `tree` or
    /// `simple-entity`", so it splits the boxes into "a tree or rock" and
    /// "something this model cannot name", which is all that is honestly
    /// available.
    ///
    /// Absolute counts and absolute flags, not a relation between them: a
    /// pair of wrong numbers can satisfy a relation.
    #[test]
    fn blocking_boxes_keep_whether_the_obstacle_is_minable() {
        let tree = FactorioEntity::new_tree(&Position::new(3.5, 3.5));
        let mut cliff =
            FactorioEntity::new_stone_furnace(&Position::new(9.5, 9.5), Direction::North);
        cliff.name = "cliff".into();
        cliff.entity_type = "cliff".into();
        let graph = entity_graph_from(vec![tree, cliff]).expect("adding must not fail");

        let minable = graph.blocking_boxes_within_minable(&Rect::new(
            &Position::new(3., 3.),
            &Position::new(4., 4.),
        ));
        assert_eq!(minable.len(), 1, "one tree: {minable:?}");
        assert!(minable[0].1, "a tree is minable");

        let anonymous = graph.blocking_boxes_within_minable(&Rect::new(
            &Position::new(9., 9.),
            &Position::new(10., 10.),
        ));
        assert_eq!(anonymous.len(), 1, "one cliff: {anonymous:?}");
        assert!(
            !anonymous[0].1,
            "a cliff is not minable, and nothing here knows anything else \
             about it"
        );
    }

    /// A hand-built power plant has to be readable **by name**, not merely
    /// collidable.
    ///
    /// Until 2026-09-02 `EntityGraph::add`'s entity-tree whitelist had no
    /// `electric-pole` and no `generator` arm -- and, one layer below that,
    /// `EntityType` had no variant to write one with, so
    /// `EntityType::from_str("electric-pole")` failed and the entity was
    /// dropped before the match was even reached. A pole and a steam engine a
    /// live world already contained went into `blocked_tree` (so they refused
    /// placements) and nowhere else, which made
    /// `PlanState::electric_supply_kw` score every real base 0 kW and refuse
    /// every research. See
    /// `docs/superpowers/notes/2026-09-02-building-power.md`.
    #[test]
    fn a_hand_built_power_plant_is_readable_by_name() {
        let graph = entity_graph_from(power_plant()).expect("adding must not fail");
        let mut names: Vec<String> = graph
            .find_entities_in_radius(Position::new(10.5, 10.5), 16., None, None)
            .into_iter()
            .map(|entity| entity.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "small-electric-pole".to_string(),
                "solar-panel".to_string(),
                "steam-engine".to_string(),
            ],
            "a pole, a generator and a panel a world already contains must be \
             nameable, not just collidable"
        );
    }

    /// The same three, as graph nodes carrying the type they were read as.
    ///
    /// `find_entities_in_radius` reads the quad tree; this reads the petgraph
    /// the tree is indexed against, so it fails separately if the entity lands
    /// in one and not the other.
    #[test]
    fn a_power_plants_entities_become_graph_nodes_with_their_own_type() {
        let graph = entity_graph_from(power_plant()).expect("adding must not fail");
        let inner = graph.inner_graph();
        let mut types: Vec<String> = inner
            .node_weights()
            .map(|node| node.entity_type.to_string())
            .collect();
        types.sort();
        assert_eq!(
            types,
            vec![
                "electric-pole".to_string(),
                "generator".to_string(),
                "solar-panel".to_string(),
            ]
        );
    }

    /// `radius` is a **disc**, not an L1 diamond.
    ///
    /// The pole sits on the diagonal at `(90.5, 90.5)`, so it is 127.99 tiles
    /// from the origin by the Euclidean norm and 181 by `|dx| + |dy|`. A
    /// search at radius 128 must find it; a search at radius 127 must not.
    ///
    /// Until 2026-09-04 [`EntityGraph::find_entities_in_radius`] filtered with
    /// `Position::distance`, which is Manhattan despite its name, so the
    /// reachable region was a diamond: full radius along the axes and only
    /// `radius / sqrt(2)` diagonally. Run `run-1788504490-09380` is what that
    /// cost -- `method::power::supply_for` could not see a standing power
    /// plant 196.5 tiles away on the diagonal, inside its 256-tile adoption
    /// radius, and planned a second plant instead; the refusal it produced
    /// named *water*, three levels away from the cause.
    ///
    /// The axis case is asserted alongside it because it passed under the old
    /// filter too: a test that only checked the axis would have gone green on
    /// the bug.
    #[test]
    fn a_radius_is_a_disc_and_not_a_diamond() {
        fn pole_at(position: Position) -> FactorioEntity {
            FactorioEntity {
                name: "small-electric-pole".into(),
                entity_type: "electric-pole".into(),
                bounding_box: add_to_rect(&Rect::from_wh(0.296_875, 0.296_875), &position),
                position,
                ..Default::default()
            }
        }
        let diagonal = Position::new(90.5, 90.5);
        let axis = Position::new(127.5, 0.5);
        let graph = entity_graph_from(vec![pole_at(diagonal.clone()), pole_at(axis.clone())])
            .expect("adding must not fail");
        let origin = Position::new(0.5, 0.5);
        let found = |radius: f64| -> Vec<Position> {
            let mut positions: Vec<Position> = graph
                .find_entities_in_radius(origin.clone(), radius, None, None)
                .into_iter()
                .map(|entity| entity.position)
                .collect();
            positions.sort_by(|a, b| a.x.total_cmp(&b.x));
            positions
        };
        // 127.28 and 127.0 away respectively: both inside a disc of 128.
        assert_eq!(
            found(128.),
            vec![diagonal.clone(), axis.clone()],
            "a disc of radius 128 holds both; the L1 diamond held only the \
             one on the axis"
        );
        // The diagonal one leaves the disc first, at 127.28.
        assert_eq!(
            found(127.),
            vec![axis],
            "shrinking the radius past the diagonal pole must drop it"
        );
        assert!(
            found(126.).is_empty(),
            "and past the axis pole must drop both"
        );
    }

    /// A pole, a steam engine and a solar panel, spaced so none of their
    /// collision boxes meet -- `add` refuses an entity whose position is
    /// already occupied, and a test that tripped that would be measuring the
    /// wrong thing.
    ///
    /// Boxes are the vanilla ones from `entity-prototype-fixtures.json`; a
    /// zero-width box is skipped by `add` outright, so they cannot be left at
    /// their default.
    fn power_plant() -> Vec<FactorioEntity> {
        fn boxed(
            name: &str,
            entity_type: &str,
            position: Position,
            w: f64,
            h: f64,
        ) -> FactorioEntity {
            FactorioEntity {
                name: name.into(),
                entity_type: entity_type.into(),
                bounding_box: add_to_rect(&Rect::from_wh(w, h), &position),
                position,
                ..Default::default()
            }
        }
        vec![
            boxed(
                "small-electric-pole",
                "electric-pole",
                Position::new(10.5, 10.5),
                0.296_875,
                0.296_875,
            ),
            boxed(
                "steam-engine",
                "generator",
                Position::new(14.5, 10.5),
                2.5,
                4.695_312_5,
            ),
            boxed(
                "solar-panel",
                "solar-panel",
                Position::new(10.5, 16.5),
                2.796_875,
                2.796_875,
            ),
        ]
    }

    /// Water is a *tile*, not an entity, and refuses a build just the same.
    /// The reconstructed box must come back exactly on tile boundaries --
    /// the quad-tree stores `f32`, and a box that came back a hair short
    /// would leave a sliver of buildable water at its edge.
    #[test]
    fn a_player_collidable_tile_is_a_blocking_box_on_exact_tile_bounds() {
        let graph = EntityGraph::new(
            Arc::new(fixture_entity_prototypes()),
            Arc::new(DashMap::new()),
        );
        graph
            .add_tiles(
                vec![FactorioTile {
                    position: Position::new(-70., 42.),
                    name: EntityName::Water.to_string(),
                    player_collidable: true,
                    color: None,
                    surface: None,
                    fluid: TileFluid::Yields {
                        fluid: "water".into(),
                    },
                }],
                None,
            )
            .expect("adding tiles must not fail");

        let boxes = graph.blocking_boxes_within(&Rect::new(
            &Position::new(-70., 42.),
            &Position::new(-69., 43.),
        ));
        assert_eq!(boxes.len(), 1);
        assert_eq!(
            boxes[0],
            Rect::new(&Position::new(-70., 42.), &Position::new(-69., 43.))
        );
    }

    /// One tile of terrain, named. `player_collidable` follows the name the
    /// way the live 2.1.17 capture does: water and deepwater collide, grass
    /// does not.
    fn terrain(x: f64, y: f64, name: &str) -> FactorioTile {
        FactorioTile {
            position: Position::new(x, y),
            player_collidable: FactorioTile::WATER_NAMES.contains(&name),
            name: name.into(),
            color: None,
            // Fixture terrain asserts no surface -- see
            // `FactorioTile::surface`. `EntityGraph` is still keyed by
            // position alone, so nothing here would read it if it did.
            surface: None,
            // Vanilla's two water tiles yield water and nothing else does.
            // Derived from the same list `player_collidable` is, so a fixture
            // cannot claim a dry lake or a wet meadow by typo.
            fluid: if FactorioTile::WATER_NAMES.contains(&name) {
                TileFluid::Yields {
                    fluid: "water".into(),
                }
            } else {
                TileFluid::Dry
            },
        }
    }

    fn graph_with_terrain(tiles: Vec<FactorioTile>) -> EntityGraph {
        let graph = EntityGraph::new(
            Arc::new(fixture_entity_prototypes()),
            Arc::new(DashMap::new()),
        );
        graph
            .add_tiles(tiles, None)
            .expect("adding tiles must not fail");
        graph
    }

    fn named(tiles: &[FactorioTile]) -> Vec<(f64, f64, String)> {
        tiles
            .iter()
            .map(|t| (t.position.x, t.position.y, t.name.clone()))
            .collect()
    }

    /// **The gap.** `EntityGraph` held every tile's *name* and had no way to
    /// hand one back: `tile_tree()` is a raw lock guard whose only caller
    /// repo-wide is a test, and `blocking_boxes_within` throws the name away.
    #[test]
    fn tiles_within_reports_the_terrain_by_name() {
        let graph = graph_with_terrain(vec![
            terrain(1., 0., "water"),
            terrain(0., 0., "grass-1"),
            terrain(0., 1., "deepwater"),
        ]);
        assert_eq!(
            named(&graph.tiles_within(&Rect::new(&Position::new(0., 0.), &Position::new(2., 2.)))),
            vec![
                (0., 0., "grass-1".to_string()),
                (0., 1., "deepwater".to_string()),
                (1., 0., "water".to_string()),
            ],
            "the planner cannot tell a lake from a forest without the name, \
             and ordered by (x, y, name) because it has to be the same order \
             every time"
        );
    }

    /// The half-open `contains` again. A tile box is a full 1x1, so the row
    /// immediately left of `bounds` abuts its edge and the quad tree returns
    /// it; the same tile one column to the *right* of `bounds` is correctly
    /// rejected. Unclipped, "the water in this rect" would name water that is
    /// not in it -- the asymmetry that cost 691 spurious keyframe divergences.
    #[test]
    fn a_tile_merely_abutting_the_bounds_is_not_inside_them() {
        let graph = graph_with_terrain(vec![
            terrain(-1., 0., "water"),
            terrain(0., -1., "water"),
            terrain(2., 0., "water"),
        ]);
        assert_eq!(
            named(&graph.tiles_within(&Rect::new(&Position::new(0., 0.), &Position::new(2., 2.)))),
            vec![],
            "left, top and right neighbours are all outside the rect and the \
             answer must not depend on which side they are on"
        );
    }

    /// The discriminator. Both of these reach `blocked_tree` and come back
    /// from `blocking_boxes_within` as bare rectangles, and until now that was
    /// everything a caller could learn about either.
    #[test]
    fn is_water_at_tells_a_lake_from_a_forest() {
        let graph = entity_graph_from(vec![FactorioEntity::new_tree(&Position::new(5.5, 0.5))])
            .expect("adding must not fail");
        graph
            .add_tiles(vec![terrain(0., 0., "water")], None)
            .expect("adding tiles must not fail");

        let bounds = Rect::new(&Position::new(-1., -1.), &Position::new(8., 8.));
        let boxes = graph.blocking_boxes_within(&bounds);
        assert_eq!(boxes.len(), 2, "a tree and a lake, both blocking");
        assert_eq!(
            boxes
                .iter()
                .filter(|b| graph.is_water_at(&b.center()))
                .count(),
            1,
            "exactly one of the two is water; an offshore pump may stand in \
             that one and a boiler may not"
        );
        assert!(graph.is_water_at(&Position::new(0.5, 0.5)), "the lake");
        assert!(
            !graph.is_water_at(&Position::new(5.5, 0.5)),
            "the tree -- collidable, minable, and not water"
        );
        assert!(
            !graph.is_water_at(&Position::new(20.5, 20.5)),
            "and open ground nobody has said anything about is not water either"
        );
    }

    /// `is_water` asks the name, not the collision flag, and the two are not
    /// the same question. `out-of-map` collides with a character exactly as
    /// water does; an offshore pump may stand in one of them.
    #[test]
    fn a_collidable_tile_that_is_not_water_is_not_water() {
        let graph = graph_with_terrain(vec![
            terrain(0., 0., "water"),
            FactorioTile {
                position: Position::new(2., 0.),
                name: "out-of-map".into(),
                player_collidable: true,
                color: None,
                surface: None,
                // Collidable and dry: the whole point of this fixture is that
                // "blocked" and "water" are different questions.
                fluid: TileFluid::Dry,
            },
        ]);
        assert!(graph.is_water_at(&Position::new(0.5, 0.5)));
        assert!(
            !graph.is_water_at(&Position::new(2.5, 0.5)),
            "collidable, and still not somewhere a pump can draw from"
        );
        assert_eq!(
            graph
                .nearest_water_tile(&Position::new(2.5, 0.5), 30.)
                .expect("the lake is in range")
                .position,
            Position::new(0., 0.),
            "and the water search must walk past it rather than return it"
        );
    }

    /// Distance is measured to the tile's **centre**, because
    /// `FactorioTile::position` is its top-left corner. Measuring corner to
    /// point biases every comparison up and to the left, and here it inverts
    /// the answer outright: from the origin the corner of `(2, 2)` is 2.83
    /// away against `(-3, 0)`'s 3.0, while the centres are 3.54 against 2.55.
    ///
    /// The half tile is the same one that, got wrong for resources, made
    /// mining fail with "no entity to mine" for every ore on every map.
    #[test]
    fn distance_to_water_is_measured_to_the_tile_centre() {
        let graph = graph_with_terrain(vec![terrain(2., 2., "water"), terrain(-3., 0., "water")]);
        assert_eq!(
            graph
                .nearest_water_tile(&Position::new(0., 0.), 30.)
                .expect("both lakes are in range")
                .position,
            Position::new(-3., 0.),
        );
    }

    /// `deepwater` outnumbers `water` four to one in the archived stdout
    /// (330,346 against 79,717), and
    /// `FactorioRcon::find_offshore_pump_placement_options` -- the shoreline
    /// rule this query exists to feed -- asks only for `"water"`.
    #[test]
    fn the_water_search_knows_both_names_for_water() {
        let graph = graph_with_terrain(vec![terrain(10., 0., "deepwater")]);
        let found = graph
            .nearest_water_tile(&Position::new(0.5, 0.5), 30.)
            .expect("a lake made only of deepwater is still a lake");
        assert_eq!(found.name, "deepwater");
        assert_eq!(found.position, Position::new(10., 0.));
    }

    /// A tile exactly as an ARCHIVED dump carries one: two fields and no
    /// `fluid`, so [`TileFluid::Unknown`] by `Default`. Every world dump and
    /// every server log this project has ever written is this shape.
    fn undeclared(x: f64, y: f64, name: &str) -> FactorioTile {
        FactorioTile {
            position: Position::new(x, y),
            player_collidable: FactorioTile::WATER_NAMES.contains(&name),
            name: name.into(),
            color: None,
            surface: None,
            fluid: TileFluid::Unknown,
        }
    }

    /// The fallback, and what it is worth. A dump written before
    /// `LuaTilePrototype::fluid` crossed the bridge says nothing about fluid
    /// at all, so a search that asked only `yields("water")` would find no
    /// water on any archived map and refuse every steam plant. Measured
    /// 2026-09-08: dropping this branch makes all three `map.json` baselines
    /// refuse with `PowerPlantNeedsWater`.
    #[test]
    fn a_tile_that_never_declared_a_fluid_is_still_water_by_name() {
        let graph = graph_with_terrain(vec![
            undeclared(3., 0., "water"),
            undeclared(0., 5., "deepwater"),
        ]);
        assert!(
            graph.is_water_at(&Position::new(3.5, 0.5)),
            "an archived `water` tile is water"
        );
        assert_eq!(
            graph
                .nearest_water_tile(&Position::new(0.5, 0.5), 30.)
                .expect("an archived lake is still a lake")
                .position,
            Position::new(3., 0.),
        );
    }

    /// The point of asking the ground. This install runs Space Age, and the
    /// vanilla name pair is a *fallback*, not the definition -- a tile whose
    /// prototype names `water` is water whatever it is called, and nobody has
    /// to extend a list for it.
    #[test]
    fn a_tile_named_nothing_like_water_that_yields_water_is_water() {
        let graph = graph_with_terrain(vec![FactorioTile {
            position: Position::new(4., 0.),
            name: "wetland-green-slime".into(),
            player_collidable: true,
            color: None,
            surface: None,
            fluid: TileFluid::Yields {
                fluid: "water".into(),
            },
        }]);
        assert!(graph.is_water_at(&Position::new(4.5, 0.5)));
        assert_eq!(
            graph
                .nearest_water_tile(&Position::new(0.5, 0.5), 30.)
                .expect("the ground said water, so it is water")
                .name,
            "wetland-green-slime",
        );
    }

    /// The other direction, and the one a name list can never get right: a
    /// tile the ground says yields something else, or nothing. `Dry` is a
    /// fact, so it OVERRIDES the name -- only [`TileFluid::Unknown`] falls
    /// back. Both of these are named in the vanilla pair and neither is water.
    #[test]
    fn the_ground_overrules_the_name_in_both_directions() {
        let graph = graph_with_terrain(vec![
            FactorioTile {
                position: Position::new(1., 0.),
                name: "water".into(),
                player_collidable: true,
                color: None,
                surface: None,
                // A modded `water` that a pump draws nothing from.
                fluid: TileFluid::Dry,
            },
            FactorioTile {
                position: Position::new(2., 0.),
                name: "deepwater".into(),
                player_collidable: true,
                color: None,
                surface: None,
                fluid: TileFluid::Yields {
                    fluid: "ammonia".into(),
                },
            },
        ]);
        assert!(
            !graph.is_water_at(&Position::new(1.5, 0.5)),
            "charted and dry, whatever it is called"
        );
        assert!(
            !graph.is_water_at(&Position::new(2.5, 0.5)),
            "charted and ammoniacal -- a boiler wants water, not ammonia"
        );
        assert!(
            graph
                .nearest_water_tile(&Position::new(0.5, 0.5), 30.)
                .is_none(),
            "and neither is offered to a plant search"
        );
    }

    /// Nearest by Euclidean distance to the tile's *centre*, with the tile's
    /// *corner* handed back -- the two halves of the half-tile convention that
    /// made mining fail on every ore on every map when it was got wrong for
    /// resources.
    #[test]
    fn the_nearest_water_tile_is_the_nearest_one() {
        let graph = graph_with_terrain(vec![
            terrain(9., 0., "water"),
            terrain(3., 0., "water"),
            terrain(0., 7., "deepwater"),
        ]);
        let found = graph
            .nearest_water_tile(&Position::new(0.5, 0.5), 30.)
            .expect("three lakes are within thirty tiles");
        assert_eq!(
            found.position,
            Position::new(3., 0.),
            "3 tiles beats 7 and 9, and the position handed back is the tile \
             corner the game reports, not the centre the distance was measured \
             to"
        );
    }

    /// A refusal is a useful answer -- "the nearest water is 300 tiles away"
    /// is what a plant method should refuse on -- so the radius has to be a
    /// real bound and not a hint.
    ///
    /// And the radius is a **circle**, not the square the quad tree was asked
    /// for. The tile at `(8, 8)` is inside the 10-tile query box and 11.3
    /// tiles away; reporting it would make the bound mean "within 10 tiles,
    /// except diagonally, where it means 14".
    #[test]
    fn no_water_within_the_radius_is_no_water() {
        let graph = graph_with_terrain(vec![terrain(100., 0., "water"), terrain(8., 8., "water")]);
        assert!(
            graph
                .nearest_water_tile(&Position::new(0.5, 0.5), 10.)
                .is_none()
        );
        assert_eq!(
            graph
                .nearest_water_tile(&Position::new(0.5, 0.5), 12.)
                .expect("the corner lake is 11.3 tiles away")
                .position,
            Position::new(8., 8.),
            "and the same lake is found when the radius reaches it"
        );
    }

    /// Determinism, which is not optional: this feeds `crates/planner`, whose
    /// contract is a byte-identical plan for identical inputs.
    ///
    /// Two tiles exactly equidistant, inserted in **both orders** into two
    /// graphs. The quad tree returns its items in insertion order, so an
    /// implementation that took "the first one" would answer differently for
    /// the two graphs; breaking the tie on `(x, y)` makes the answer a
    /// property of the map rather than of the arrival order of its chunks.
    #[test]
    fn two_lakes_equally_far_away_resolve_by_position_not_by_arrival() {
        let east = terrain(5., 0., "water");
        let south = terrain(0., 5., "water");
        let from = Position::new(0.5, 0.5);

        let one = graph_with_terrain(vec![east.clone(), south.clone()]);
        let other = graph_with_terrain(vec![south, east]);
        let expected = Position::new(0., 5.);

        for graph in [&one, &other] {
            for _ in 0..20 {
                assert_eq!(
                    graph
                        .nearest_water_tile(&from, 30.)
                        .expect("both lakes are in range")
                        .position,
                    expected,
                    "equal distance, so the lower x wins -- every time, and \
                     whichever chunk arrived first"
                );
            }
        }
    }

    /// And the same for the bulk read.
    #[test]
    fn tiles_within_comes_back_in_a_fixed_order() {
        let tiles = vec![
            terrain(1., 1., "water"),
            terrain(0., 1., "grass-1"),
            terrain(1., 0., "deepwater"),
            terrain(0., 0., "grass-1"),
        ];
        let mut reversed = tiles.clone();
        reversed.reverse();
        let bounds = Rect::new(&Position::new(0., 0.), &Position::new(2., 2.));

        let expected = named(&graph_with_terrain(tiles).tiles_within(&bounds));
        assert_eq!(expected.len(), 4);
        let other = graph_with_terrain(reversed);
        for _ in 0..20 {
            assert_eq!(named(&other.tiles_within(&bounds)), expected);
        }
    }

    /// A tile the same call is *not* asked about must stay out of the answer,
    /// or "blocked" would mean "something is blocked somewhere".
    #[test]
    fn a_blocking_box_outside_the_bounds_is_not_reported() {
        let graph = entity_graph_from(vec![FactorioEntity::new_tree(&Position::new(100.5, 100.5))])
            .expect("adding must not fail");
        assert!(
            graph
                .blocking_boxes_within(&Rect::new(&Position::new(0., 0.), &Position::new(1., 1.)))
                .is_empty()
        );
    }

    #[test]
    fn test_resource_patches_single_field_is_one_patch() {
        let world = fixture_world();
        // fixture_world spawns iron ore via
        // add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 40.))
        // which is left_top (-45, 35) .. right_bottom (-35, 45); rect_fields is
        // inclusive on both ends, so it actually yields 11x11 = 121 tiles, not 10x10.
        let iron_rect = add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 40.));
        let expected_tiles = rect_fields(&iron_rect).len();
        assert_eq!(expected_tiles, 121);

        let patches = world
            .entity_graph
            .resource_patches(&EntityName::IronOre.to_string());
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].elements.len(), expected_tiles);
    }

    #[test]
    fn test_resource_patches_separate_fields_stay_separate() {
        let world = fixture_world();
        let iron = world
            .entity_graph
            .resource_patches(&EntityName::IronOre.to_string());
        let copper = world
            .entity_graph
            .resource_patches(&EntityName::CopperOre.to_string());
        assert_eq!(iron.len(), 1);
        assert_eq!(copper.len(), 1);

        let iron_positions: std::collections::HashSet<Pos> =
            iron[0].elements.iter().map(Pos::from).collect();
        let copper_positions: std::collections::HashSet<Pos> =
            copper[0].elements.iter().map(Pos::from).collect();
        assert!(iron_positions.is_disjoint(&copper_positions));
    }

    /// The quiet predicate agrees with the loud query, on both answers.
    ///
    /// They must, because one exists only to spare the other's warning: a
    /// `craft_ticks` that asked `has_resource_patches` and got a different
    /// answer than `!resource_patches(..).is_empty()` would price an
    /// intermediate as ore or ore as free. Mined out counts as absent on both
    /// sides -- `remove_resource` drops a tile from `resources` -- so the
    /// depleted case is checked too.
    #[test]
    fn the_resource_predicate_agrees_with_the_query() {
        let world = fixture_world();
        for name in [
            EntityName::IronOre.to_string(),
            EntityName::CopperOre.to_string(),
            EntityName::Coal.to_string(),
            EntityName::Stone.to_string(),
            // Everything a plan's cost model walks past on its way to ore.
            "iron-plate".to_string(),
            "iron-gear-wheel".to_string(),
            "stone-furnace".to_string(),
            "burner-mining-drill".to_string(),
            "uranium-ore".to_string(),
        ] {
            assert_eq!(
                world.entity_graph.has_resource_patches(&name),
                !world.entity_graph.resource_patches(&name).is_empty(),
                "the predicate and the query disagree about '{name}'"
            );
        }
    }

    #[test]
    fn test_resource_patches_repeated_calls_agree() {
        let world = fixture_world();
        let name = EntityName::IronOre.to_string();
        let first = sorted_patches(world.entity_graph.resource_patches(&name));
        for _ in 0..5 {
            let next = sorted_patches(world.entity_graph.resource_patches(&name));
            assert_eq!(next.len(), first.len());
            assert_eq!(next, first);
        }
    }

    #[test]
    fn test_resource_patches_two_disjoint_fields_of_same_resource_are_two_patches() {
        let mut entities: Vec<FactorioEntity> = vec![];
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(4., 4.), &Position::new(0., 0.)),
            &EntityName::IronOre.to_string(),
        );
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(4., 4.), &Position::new(100., 100.)),
            &EntityName::IronOre.to_string(),
        );
        let graph = entity_graph_from(entities).unwrap();
        let patches = graph.resource_patches(&EntityName::IronOre.to_string());
        assert_eq!(patches.len(), 2);
    }

    /// Builds a graph holding one 4x4 iron patch at `at`, plus whatever else.
    fn graph_with_iron_at(at: Position, extra: Option<(Position, &str)>) -> EntityGraph {
        let mut entities: Vec<FactorioEntity> = vec![];
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(4., 4.), &at),
            &EntityName::IronOre.to_string(),
        );
        if let Some((pos, name)) = extra {
            spawn_ore(
                &mut entities,
                add_to_rect(&Rect::from_wh(4., 4.), &pos),
                name,
            );
        }
        entity_graph_from(entities).unwrap()
    }

    #[test]
    fn the_fingerprint_of_the_same_map_is_the_same_every_time() {
        // The property the whole comparison guard rests on. `resources` is a
        // `DashMap`, whose iteration order is not stable, so this would fail if
        // the digest were taken over it directly rather than over a sorted
        // copy.
        let a = graph_with_iron_at(Position::new(0., 0.), None)
            .resource_fingerprint()
            .expect("charted ore fingerprints");
        for _ in 0..8 {
            let b = graph_with_iron_at(Position::new(0., 0.), None)
                .resource_fingerprint()
                .expect("charted ore fingerprints");
            assert_eq!(a, b, "the same map must fingerprint identically");
        }
    }

    #[test]
    fn a_map_with_a_patch_the_other_lacks_fingerprints_differently() {
        // This is the difference that actually mattered: the retracted "four
        // bots do double the work of one" compared two maps that differed by a
        // resource patch about 100 tiles east.
        let near = graph_with_iron_at(Position::new(0., 0.), None)
            .resource_fingerprint()
            .unwrap();
        let with_extra = graph_with_iron_at(
            Position::new(0., 0.),
            Some((Position::new(100., 0.), &EntityName::CopperOre.to_string())),
        )
        .resource_fingerprint()
        .unwrap();
        assert_ne!(near.digest, with_extra.digest);
        assert_eq!(near.tiles.get("copper-ore"), None);
        assert_eq!(with_extra.tiles.get("copper-ore"), Some(&25));
    }

    #[test]
    fn moving_a_patch_changes_the_digest_even_with_the_same_tile_count() {
        // Counts alone are not an identity: two maps can hold the same amount
        // of ore in different places, and that is a different map.
        let here = graph_with_iron_at(Position::new(0., 0.), None)
            .resource_fingerprint()
            .unwrap();
        let there = graph_with_iron_at(Position::new(100., 100.), None)
            .resource_fingerprint()
            .unwrap();
        assert_eq!(here.tiles, there.tiles, "same counts");
        assert_ne!(here.digest, there.digest, "different places");
    }

    #[test]
    fn a_world_with_nothing_charted_has_no_fingerprint_rather_than_an_empty_one() {
        // "Not read yet" is not "a map with no ore", and a digest over an empty
        // table would make every unread world look like the same map.
        let graph = entity_graph_from(vec![]).unwrap();
        assert_eq!(graph.resource_fingerprint(), None);
    }

    // helper: sort each patch's elements (as Pos, for a total order) and sort the
    // patches themselves so repeated calls can be compared for equality even
    // though patch id assignment order is not guaranteed to be stable.
    fn sorted_patches(patches: Vec<ResourcePatch>) -> Vec<Vec<Pos>> {
        let mut result: Vec<Vec<Pos>> = patches
            .iter()
            .map(|patch| {
                let mut elements: Vec<Pos> = patch.elements.iter().map(Pos::from).collect();
                elements.sort();
                elements
            })
            .collect();
        result.sort();
        result
    }

    #[test]
    fn test_splitters() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 0.5), Direction::South),
            FactorioEntity::new_splitter(&Position::new(1., 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 2.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "transport-belt at [0.5, 0.5]" ]
    1 [ label = "transport-belt at [1.5, 0.5]" ]
    2 [ label = "splitter at [1, 1.5]" ]
    3 [ label = "transport-belt at [0.5, 2.5]" ]
    4 [ label = "transport-belt at [1.5, 2.5]" ]
    0 -> 2 [ label = "1" ]
    1 -> 2 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    2 -> 4 [ label = "1" ]
}
"#,
        );
    }
    #[test]
    fn test_condense() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 3.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 4.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            graph.graphviz_dot_condensed(),
            r#"digraph {
    0 [ label = "transport-belt at [0.5, 0.5]" ]
    4 [ label = "transport-belt at [0.5, 4.5]" ]
    0 -> 4 [ label = "4" ]
}
"#,
        );
    }

    #[test]
    fn test_splitters2() {
        let graph = entity_graph_from(vec![]).unwrap();
        graph.add_blueprint_entities("0eNqd0u+KwyAMAPB3yWd3TK/q5quM42i3MITWimbHleK7n64clK1lf74ZMb8kkhGa9oI+WEdgRrDH3kUwhxGiPbu6LXc0eAQDlrADBq7uShR9a4kwQGJg3Ql/wfDEHqZRqF30faBNgy3NkkX6YoCOLFmcGrgGw7e7dE0uY/iawcD3Maf1rlTN1EbxD8lgAKPzIZc42YDH6YEoPd7I4n6oBXP7b4rH4uczotytiGpBrJ6fXu7n0y9Y8h1L3P5ktSCrF2S9KquyCte1MbPlZPCDIU5fvuOVrvZaab5VUqX0B2ef55s=").expect("failed to read blueprint");
        graph.connect().unwrap();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "transport-belt at [-61.5, 71.5]" ]
    1 [ label = "splitter at [-60.5, 72]" ]
    2 [ label = "splitter at [-58.5, 72]" ]
    3 [ label = "transport-belt at [-59.5, 71.5]" ]
    4 [ label = "transport-belt at [-59.5, 72.5]" ]
    5 [ label = "transport-belt at [-57.5, 72.5]" ]
    0 -> 1 [ label = "1" ]
    1 -> 3 [ label = "1" ]
    1 -> 4 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    2 -> 4 [ label = "1" ]
    5 -> 2 [ label = "1" ]
}
"#,
        );
    }

    /// A resource tile keeps the amount the game reported for it.
    ///
    /// The mod has always sent it (`serialize_entity`, `mods/BotBridge/types.lua`)
    /// and `FactorioEntity::amount` has always carried it in; this map used to
    /// drop it at the door, which left `crates/planner` inventing 500 for every
    /// tile on every map. A live capture
    /// (`crates/core/tests/live-2.1.17-entities-resources.json`) has iron tiles
    /// holding 13.
    #[test]
    fn a_resource_tile_keeps_the_amount_the_game_reported() {
        let mut ore = FactorioEntity::new_resource(
            &Position::new(-40.5, -48.5),
            Direction::North,
            &EntityName::IronOre.to_string(),
        );
        ore.amount = Some(13);
        let graph = entity_graph_from(vec![ore]).unwrap();

        let tile = Pos(-41, -49);
        assert!(graph.resource_contains(&EntityName::IronOre.to_string(), tile.clone()));
        assert_eq!(
            graph.resource_amount(&EntityName::IronOre.to_string(), &tile),
            Some(13)
        );
    }

    /// A tile nobody reported an amount for reads as *unknown*, not as empty
    /// and not as full.
    ///
    /// Every hand-built fixture is in this state -- `FactorioEntity::new_resource`
    /// sets no amount -- so this is the case the planner's
    /// `DEFAULT_RESOURCE_PER_TILE` fallback exists for. It has to be
    /// distinguishable from `Some(0)`, which cannot happen for a live tile at
    /// all: the game destroys a resource entity the moment it empties.
    #[test]
    fn a_resource_tile_nobody_reported_an_amount_for_reads_as_unknown() {
        let graph = entity_graph_from(vec![FactorioEntity::new_resource(
            &Position::new(-40.5, -48.5),
            Direction::North,
            &EntityName::IronOre.to_string(),
        )])
        .unwrap();

        let tile = Pos(-41, -49);
        assert!(graph.resource_contains(&EntityName::IronOre.to_string(), tile.clone()));
        assert_eq!(
            graph.resource_amount(&EntityName::IronOre.to_string(), &tile),
            None,
            "no amount was reported, so none is known"
        );
    }

    /// A tile nobody has ever delivered has no amount either, and asking does
    /// not invent one.
    #[test]
    fn an_unknown_tile_has_no_amount() {
        let graph = entity_graph_from(vec![]).unwrap();
        assert_eq!(
            graph.resource_amount(&EntityName::IronOre.to_string(), &Pos(-41, -49)),
            None
        );
    }

    /// A second delivery of a tile already known refreshes its amount.
    ///
    /// Resource tiles arrive here repeatedly by design of the transport (see
    /// `add`), and the repeat is deduplicated -- but the *reading* it carries
    /// is newer than the stored one, and throwing it away would be the same
    /// mistake as never reading it. The tile itself must still be one entry.
    #[test]
    fn a_second_delivery_refreshes_the_amount_without_duplicating_the_tile() {
        let at = Position::new(-40.5, -48.5);
        let mut first =
            FactorioEntity::new_resource(&at, Direction::North, &EntityName::IronOre.to_string());
        first.amount = Some(13);
        let graph = entity_graph_from(vec![first]).unwrap();

        let mut again =
            FactorioEntity::new_resource(&at, Direction::North, &EntityName::IronOre.to_string());
        again.amount = Some(7);
        graph.add(vec![again], None).unwrap();

        let tile = Pos(-41, -49);
        assert_eq!(
            graph.resource_amount(&EntityName::IronOre.to_string(), &tile),
            Some(7)
        );
        let patches = graph.resource_patches(&EntityName::IronOre.to_string());
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].elements.len(), 1, "one tile, one entry");
    }

    /// A delivery carrying no amount leaves a known one alone.
    ///
    /// Silence is not a report of zero, and it is not a report of anything
    /// else either. A blueprint import or a fixture must not be able to erase
    /// what the game said about a tile.
    #[test]
    fn a_delivery_without_an_amount_does_not_erase_a_known_one() {
        let at = Position::new(-40.5, -48.5);
        let mut first =
            FactorioEntity::new_resource(&at, Direction::North, &EntityName::IronOre.to_string());
        first.amount = Some(13);
        let graph = entity_graph_from(vec![first]).unwrap();

        graph
            .add(
                vec![FactorioEntity::new_resource(
                    &at,
                    Direction::North,
                    &EntityName::IronOre.to_string(),
                )],
                None,
            )
            .unwrap();

        assert_eq!(
            graph.resource_amount(&EntityName::IronOre.to_string(), &Pos(-41, -49)),
            Some(13)
        );
    }

    /// The exact scenario `resource_position_from_pos`'s own doc comment
    /// warns about: `resources` (and the tree behind it) key a resource by
    /// its floored `Pos`, so a resource at a genuine tile centre --
    /// `(-40.5, -48.5)`, never `(-41, -49)` -- must come back out of
    /// `snapshot_within` at that same centre, not at the floored corner.
    /// Getting this wrong once made every ore on every map unmineable while
    /// every test passed, because the test fixture used integer positions,
    /// the one input the lossy round trip does not corrupt.
    #[test]
    fn snapshot_within_restores_a_resources_tile_centre() {
        use crate::record::map::divergence_between;

        let graph = entity_graph_from(vec![FactorioEntity::new_resource(
            &Position::new(-40.5, -48.5),
            Direction::North,
            &EntityName::IronOre.to_string(),
        )])
        .unwrap();

        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        let model = graph.snapshot_within(&bounds);

        let iron = model
            .iter()
            .find(|e| e.name == EntityName::IronOre.to_string())
            .expect("the resource must be found within its own bounds");
        assert_eq!(iron.position, Position::new(-40.5, -48.5));

        // The game agrees exactly: no divergence.
        let game = vec![iron.clone()];
        assert!(divergence_between(&game, &model).is_empty());
    }

    /// The mod writes a chunk's entities out from both arms of
    /// `on_chunk_generated` -- the real event and the `initial_discovery`
    /// replay -- so the same ore tile reaches `add` twice. It must land in the
    /// graph once.
    ///
    /// Two `add` calls rather than one call with a repeated entity, because
    /// that is the shape the transport actually delivers: two chunk writeouts,
    /// parsed independently.
    ///
    /// The positions are tile *centres* on purpose. `resources` keys by a
    /// flooring `Pos`, so a fixture built on integers would round-trip
    /// losslessly and could not tell a genuine second tile from a second copy
    /// of the first.
    #[test]
    fn a_resource_delivered_twice_occupies_one_tile() {
        let ore = || {
            FactorioEntity::new_resource(
                &Position::new(-40.5, -48.5),
                Direction::North,
                &EntityName::IronOre.to_string(),
            )
        };
        let graph = entity_graph_from(vec![ore()]).unwrap();
        graph.add(vec![ore()], None).unwrap();

        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        assert_eq!(
            graph.snapshot_within(&bounds).len(),
            1,
            "the same tile delivered twice must be modelled once"
        );

        let patches = graph.resource_patches(&EntityName::IronOre.to_string());
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].elements, vec![Position::new(-40.5, -48.5)]);
    }

    /// The negative control for `a_resource_delivered_twice_occupies_one_tile`:
    /// deduplication must collapse a repeat, never two neighbours. Adjacent
    /// tiles are the hard case -- their 0.8-wide boxes sit inside the same
    /// query rectangle and differ only in the key the dedup compares.
    #[test]
    fn two_adjacent_resource_tiles_both_survive() {
        let iron = EntityName::IronOre.to_string();
        let graph = entity_graph_from(vec![FactorioEntity::new_resource(
            &Position::new(-40.5, -48.5),
            Direction::North,
            &iron,
        )])
        .unwrap();
        graph
            .add(
                vec![FactorioEntity::new_resource(
                    &Position::new(-39.5, -48.5),
                    Direction::North,
                    &iron,
                )],
                None,
            )
            .unwrap();

        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        let mut modelled: Vec<Position> = graph
            .snapshot_within(&bounds)
            .into_iter()
            .map(|entity| entity.position)
            .collect();
        modelled.sort_by(|a, b| a.x().total_cmp(&b.x()));
        assert_eq!(
            modelled,
            vec![Position::new(-40.5, -48.5), Position::new(-39.5, -48.5)]
        );

        let patches = graph.resource_patches(&iron);
        assert_eq!(patches.len(), 1, "the two tiles touch, so it is one patch");
        assert_eq!(patches[0].elements.len(), 2);
    }

    /// What the duplicates cost beyond the count: `remove` takes the tile out
    /// once, so a second copy would keep a mined-out tile in the model as ore
    /// and the planner would keep sizing work against it.
    #[test]
    fn removing_a_resource_delivered_twice_empties_the_tile() {
        let iron = EntityName::IronOre.to_string();
        let ore =
            || FactorioEntity::new_resource(&Position::new(-40.5, -48.5), Direction::North, &iron);
        let graph = entity_graph_from(vec![ore()]).unwrap();
        graph.add(vec![ore()], None).unwrap();

        graph.remove(&ore()).unwrap();

        assert!(!graph.resource_contains(&iron, Pos(-41, -49)));
        assert!(graph.resource_patches(&iron).is_empty());
        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        assert!(graph.snapshot_within(&bounds).is_empty());
    }

    /// One tile, mined twice, the second mine emptying it.
    ///
    /// The position is a genuine tile centre. `resources` keys by a flooring
    /// `Pos`, so a fixture on integers round-trips losslessly and would prove
    /// nothing about the retirement, which has to rebuild the centre from the
    /// floored key to find the tile in `resource_tree` at all -- the exact
    /// round trip that once made every ore on every map unmineable while every
    /// test stayed green.
    #[test]
    fn mining_a_tile_dry_retires_it() {
        let iron = EntityName::IronOre.to_string();
        let at = Position::new(-40.5, -48.5);
        let mut ore = FactorioEntity::new_resource(&at, Direction::North, &iron);
        ore.amount = Some(10);
        let graph = entity_graph_from(vec![ore]).unwrap();

        assert_eq!(
            graph.resource_mined(&iron, &at, 4),
            ResourceDepletion::Remaining(6),
            "a partial mine debits the tile and leaves it standing"
        );
        assert!(graph.resource_contains(&iron, Pos(-41, -49)));
        assert_eq!(graph.resource_amount(&iron, &Pos(-41, -49)), Some(6));

        assert_eq!(
            graph.resource_mined(&iron, &at, 6),
            ResourceDepletion::Exhausted
        );

        // Every way the planner can find a tile must now agree it is gone.
        assert!(
            !graph.resource_contains(&iron, Pos(-41, -49)),
            "an emptied tile must leave the resources map"
        );
        assert_eq!(graph.resource_amount(&iron, &Pos(-41, -49)), None);
        assert!(
            graph.resource_patches(&iron).is_empty(),
            "an emptied tile must leave the patches the planner picks from"
        );
        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        assert!(
            graph.snapshot_within(&bounds).is_empty(),
            "an emptied tile must leave the quad tree too, or the model still \
             reports ore the game has destroyed"
        );

        assert_eq!(
            graph.resource_mined(&iron, &at, 1),
            ResourceDepletion::Absent,
            "a second report for a retired tile has nothing to debit"
        );
    }

    /// Retiring one tile must not take its neighbour with it. Adjacent tiles
    /// are the hard case: their boxes sit inside the same query rectangle and
    /// differ only in the key.
    #[test]
    fn retiring_a_tile_leaves_its_neighbour_alone() {
        let iron = EntityName::IronOre.to_string();
        let here = Position::new(-40.5, -48.5);
        let next = Position::new(-39.5, -48.5);
        let with_amount = |at: &Position, amount: u32| {
            let mut ore = FactorioEntity::new_resource(at, Direction::North, &iron);
            ore.amount = Some(amount);
            ore
        };
        let graph =
            entity_graph_from(vec![with_amount(&here, 3), with_amount(&next, 500)]).unwrap();

        assert_eq!(
            graph.resource_mined(&iron, &here, 3),
            ResourceDepletion::Exhausted
        );

        assert!(!graph.resource_contains(&iron, Pos(-41, -49)));
        assert!(
            graph.resource_contains(&iron, Pos(-40, -49)),
            "the neighbour was retired along with the tile that was mined"
        );
        assert_eq!(graph.resource_amount(&iron, &Pos(-40, -49)), Some(500));
        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        let modelled: Vec<Position> = graph
            .snapshot_within(&bounds)
            .into_iter()
            .map(|entity| entity.position)
            .collect();
        assert_eq!(modelled, vec![next]);
    }

    /// A tile nobody reported an amount for is left exactly as it was.
    ///
    /// `None` is "nobody said", not zero. Treating it as a capacity to
    /// decrement would retire real ore on the first mine, and every
    /// hand-built fixture tile is `None`.
    #[test]
    fn mining_a_tile_of_unknown_amount_changes_nothing() {
        let iron = EntityName::IronOre.to_string();
        let at = Position::new(-40.5, -48.5);
        let graph = entity_graph_from(vec![FactorioEntity::new_resource(
            &at,
            Direction::North,
            &iron,
        )])
        .unwrap();

        assert_eq!(
            graph.resource_mined(&iron, &at, 50),
            ResourceDepletion::AmountUnknown
        );
        assert!(graph.resource_contains(&iron, Pos(-41, -49)));
        assert_eq!(graph.resource_amount(&iron, &Pos(-41, -49)), None);
    }

    /// The mod raises `on_some_entity_deleted` for a resource on **every
    /// mining swing**, not on depletion, and the payload carries the amount
    /// still in the ground. Deleting on that is what emptied the model of ore
    /// the game still had: across `workspace/runs/`, every one of 583 ore tiles
    /// a keyframe found in the game and not in the model was a tile a bot had
    /// mined at, none of them exhausted.
    ///
    /// It also made `resource_mined`'s retirement unreachable -- the tile was
    /// gone before the action that mined it settled -- so the assertion at the
    /// end is not decoration: the debit must still find something to debit.
    #[test]
    fn a_mined_resource_that_still_holds_ore_is_not_removed() {
        let iron = EntityName::IronOre.to_string();
        let at = Position::new(-40.5, -48.5);
        let mut ore = FactorioEntity::new_resource(&at, Direction::North, &iron);
        ore.amount = Some(500);
        let graph = entity_graph_from(vec![ore]).unwrap();

        // One swing's worth: the game took an ore and says 499 are left.
        let mut mined = FactorioEntity::new_resource(&at, Direction::North, &iron);
        mined.amount = Some(499);
        graph.remove(&mined).unwrap();

        assert!(
            graph.resource_contains(&iron, Pos(-41, -49)),
            "a tile the game still holds ore in must stay in the model"
        );
        assert_eq!(
            graph.resource_patches(&iron).len(),
            1,
            "and must stay in the patches the planner picks from"
        );
        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        assert_eq!(graph.snapshot_within(&bounds).len(), 1);

        // The reported amount is not written through: `resource_mined` is the
        // debit authority, and applying both would take the same ore out twice.
        assert_eq!(graph.resource_amount(&iron, &Pos(-41, -49)), Some(500));
        assert_eq!(
            graph.resource_mined(&iron, &at, 1),
            ResourceDepletion::Remaining(499),
            "the retirement arithmetic must still have a tile to work on"
        );
    }

    /// The negative control for `a_mined_resource_that_still_holds_ore_is_not_removed`.
    ///
    /// `on_resource_depleted` is wired to the same writeout, and the entity it
    /// names is empty. Nothing about the swing-by-swing case may stop that
    /// deleting the tile, or a mined-out tile lives forever.
    #[test]
    fn an_emptied_resource_is_still_removed() {
        let iron = EntityName::IronOre.to_string();
        let at = Position::new(-40.5, -48.5);
        let mut ore = FactorioEntity::new_resource(&at, Direction::North, &iron);
        ore.amount = Some(500);
        let graph = entity_graph_from(vec![ore]).unwrap();

        let mut depleted = FactorioEntity::new_resource(&at, Direction::North, &iron);
        depleted.amount = Some(0);
        graph.remove(&depleted).unwrap();

        assert!(!graph.resource_contains(&iron, Pos(-41, -49)));
        assert!(graph.resource_patches(&iron).is_empty());
        let bounds = Rect::new(&Position::new(-50., -58.), &Position::new(-30., -38.));
        assert!(graph.snapshot_within(&bounds).is_empty());
    }

    /// A tile that only *abuts* the bounds is outside them.
    ///
    /// The quad tree disagrees, on two of four sides: `my_intersects` falls
    /// back on `euclid`'s half-open `Rect::contains`, so a box whose maximum
    /// corner sits on the query's left or top edge comes back, while the mirror
    /// image on the right or bottom does not. A resource is stored under a full
    /// 1x1 tile box, so the column immediately left of `bounds.left` was
    /// reported as modelled -- 691 spurious `only_in: "model"` entries across
    /// the archived runs, 443 on the left edge, 248 on the top, none anywhere
    /// else.
    ///
    /// The four outside tiles are all four sides on purpose: two of them were
    /// already excluded, and a test that only checks the two broken sides
    /// cannot tell a fix from an over-correction that drops the interior tile
    /// as well.
    #[test]
    fn snapshot_within_excludes_a_tile_that_only_abuts_the_bounds() {
        let iron = EntityName::IronOre.to_string();
        let tile = |x: f64, y: f64| {
            FactorioEntity::new_resource(&Position::new(x, y), Direction::North, &iron)
        };
        // Bounds as a live run had them, on whole tiles.
        let bounds = Rect::new(&Position::new(-42., -2.), &Position::new(25., 51.));
        let graph = entity_graph_from(vec![
            tile(-42.5, 12.5), // outside the left edge, touching it
            tile(12.5, -2.5),  // outside the top edge, touching it
            tile(25.5, 12.5),  // outside the right edge, touching it
            tile(12.5, 51.5),  // outside the bottom edge, touching it
            tile(-41.5, 12.5), // inside, one tile in from the left edge
        ])
        .unwrap();

        let modelled: Vec<Position> = graph
            .snapshot_within(&bounds)
            .into_iter()
            .map(|entity| entity.position)
            .collect();
        assert_eq!(
            modelled,
            vec![Position::new(-41.5, 12.5)],
            "only the tile actually inside the bounds is inside the bounds"
        );
    }

    /// The game destroyed the entity mid-mine. The model has to believe it
    /// whatever its own arithmetic says -- this is the case the live run hit,
    /// where a tile delivered 15 ore against an ask of 22 and then vanished,
    /// leaving the model holding a positive amount for a tile with nothing in
    /// it.
    #[test]
    fn a_vanished_target_is_retired_whatever_the_model_believed() {
        let iron = EntityName::IronOre.to_string();
        let at = Position::new(-40.5, -48.5);
        let mut ore = FactorioEntity::new_resource(&at, Direction::North, &iron);
        ore.amount = Some(500);
        let graph = entity_graph_from(vec![ore]).unwrap();

        assert!(graph.retire_resource(&iron, &at));
        assert!(!graph.resource_contains(&iron, Pos(-41, -49)));
        assert!(graph.resource_patches(&iron).is_empty());
        assert!(
            !graph.retire_resource(&iron, &at),
            "retiring a tile that is already gone must report that it found none"
        );
    }

    #[test]
    fn snapshot_within_finds_a_tracked_entity_by_bounds() {
        let graph = entity_graph_from(vec![FactorioEntity::new_transport_belt(
            &Position::new(0.5, 0.5),
            Direction::South,
        )])
        .unwrap();

        let bounds = Rect::new(&Position::new(-1., -1.), &Position::new(2., 2.));
        let model = graph.snapshot_within(&bounds);
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].name, "transport-belt");
        assert_eq!(model[0].position, Position::new(0.5, 0.5));

        let empty_bounds = Rect::new(&Position::new(50., 50.), &Position::new(60., 60.));
        assert!(graph.snapshot_within(&empty_bounds).is_empty());
    }

    // -----------------------------------------------------------------------
    // Minables: the trees and rocks, by name and position
    // -----------------------------------------------------------------------

    fn tree_at(name: &str, position: Position) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: EntityType::Tree.to_string(),
            bounding_box: crate::factorio::util::add_to_rect(&Rect::from_wh(0.8, 0.8), &position),
            position,
            ..Default::default()
        }
    }

    /// The position handed back is the one the game reported, not the tile
    /// corner the key is derived from.
    ///
    /// The mod matches with `surface.find_entity(name, position)`, which is
    /// exact. Handing back `(-41, -49)` for a tree at `(-40.5, -48.5)` is the
    /// same half-tile fault that once made mining fail on every real map while
    /// every test passed -- and unlike a resource tile, a tree is under no
    /// obligation to sit on a centre, so there is no offset to restore it with
    /// after the fact.
    #[test]
    fn a_minable_keeps_the_position_the_game_reported() {
        let graph = entity_graph_from(vec![tree_at("tree-01", Position::new(-40.5, -48.5))])
            .expect("adding must not fail");
        assert_eq!(
            graph.minable_positions("tree-01"),
            vec![Position::new(-40.5, -48.5)]
        );
    }

    /// A tree is *not* a resource, and must not become one: the ground it
    /// stands on is not ore.
    #[test]
    fn a_minable_does_not_land_in_the_resource_model() {
        let graph = entity_graph_from(vec![tree_at("tree-01", Position::new(5.5, 5.5))])
            .expect("adding must not fail");
        assert!(!graph.any_resource_at(&Pos(5, 5)));
        assert!(!graph.resource_contains("tree-01", Pos(5, 5)));
    }

    /// The item-to-entity direction, read off the prototype's own
    /// `mine_result`, in name order.
    #[test]
    fn minables_yielding_reads_the_prototypes_mine_result() {
        let graph = entity_graph_from(vec![
            tree_at("tree-02", Position::new(9.5, 0.5)),
            tree_at("tree-01", Position::new(5.5, 5.5)),
            FactorioEntity::new_rock(&Position::new(20.5, 20.5), "rock-big"),
        ])
        .expect("adding must not fail");
        assert_eq!(
            graph.minables_yielding("wood"),
            vec![("tree-01".to_string(), 4), ("tree-02".to_string(), 4)],
            "name order, because the backing map is a DashMap and its own \
             order is a hash seed's"
        );
        assert_eq!(
            graph.minables_yielding("stone"),
            vec![("rock-big".to_string(), 20)],
            "a rock yields stone, and the tree does not"
        );
        assert!(
            graph.minables_yielding("iron-plate").is_empty(),
            "nothing standing yields a crafted item"
        );
    }

    /// An entity the model holds but has no prototype for yields nothing --
    /// refusing the work rather than guessing a bill.
    ///
    /// This is not a hypothetical: `FactorioEntity::new_tree` names every tree
    /// it makes `tree-42`, and no prototype fixture carries that name, so the
    /// shared test world's hundred trees are exactly this case.
    #[test]
    fn a_minable_with_no_prototype_yields_nothing() {
        let graph = entity_graph_from(vec![FactorioEntity::new_tree(&Position::new(5.5, 5.5))])
            .expect("adding must not fail");
        assert_eq!(graph.minable_positions("tree-42").len(), 1, "it is stored");
        assert!(
            graph.minables_yielding("wood").is_empty(),
            "but nothing can be claimed from it"
        );
    }

    /// Retiring a chopped tree takes it out of the model *and* unblocks the
    /// ground it stood on.
    ///
    /// Nothing else does either. The mod destroys the entity and emits no
    /// event for it, so a stump left here is offered to the planner for ever
    /// and the second visit fails with "no entity to mine" about ground the
    /// bot itself cleared.
    #[test]
    fn retiring_a_minable_removes_it_and_frees_its_ground() {
        let at = Position::new(5.5, 5.5);
        let graph =
            entity_graph_from(vec![tree_at("tree-01", at.clone())]).expect("adding must not fail");
        assert!(
            !graph
                .blocking_boxes_within(&Rect::new(&Position::new(4., 4.), &Position::new(7., 7.)))
                .is_empty()
        );

        assert!(graph.retire_minable("tree-01", &at));
        assert!(graph.minable_positions("tree-01").is_empty());
        assert!(
            graph
                .blocking_boxes_within(&Rect::new(&Position::new(4., 4.), &Position::new(7., 7.)))
                .is_empty(),
            "a chopped tree stops blocking placements"
        );
        assert!(
            !graph.retire_minable("tree-01", &at),
            "a second report is answered honestly rather than pretended into a removal"
        );
    }

    /// Retiring by a position anywhere in the tile finds the entity, and the
    /// entity handed to `remove` is rebuilt on the position the game gave --
    /// so the box cleared out of `blocked_tree` is the one `add` put there.
    #[test]
    fn a_minable_is_retired_from_any_position_in_its_tile() {
        let at = Position::new(5.75, 5.25);
        let graph = entity_graph_from(vec![tree_at("tree-01", at)]).expect("adding must not fail");
        assert!(graph.retire_minable("tree-01", &Position::new(5.0, 5.0)));
        assert!(graph.minable_positions("tree-01").is_empty());
    }

    /// `Clone` carries the map. `FactorioSurface` clones its graph, so a map
    /// this did not copy would leave a cloned world holding a forest it could
    /// not name.
    ///
    /// A clone, not a serde round trip -- the round trip has its own test
    /// below now. This one used to carry a note saying JSON was impossible
    /// here, because both `resources` and this map key a `BTreeMap` by `Pos`,
    /// a tuple struct `serde_json` refuses in key position. That was true and
    /// it made every dump of a world containing one ore tile fail; both maps
    /// now travel as lists of `[pos, value]` pairs (see `TileMaps`).
    #[test]
    fn cloning_a_graph_carries_its_minables() {
        let graph = entity_graph_from(vec![tree_at("tree-01", Position::new(5.5, 5.5))])
            .expect("adding must not fail");
        let copy = graph.clone();
        assert_eq!(
            copy.minable_positions("tree-01"),
            vec![Position::new(5.5, 5.5)]
        );
        assert_eq!(
            copy.minables_yielding("wood"),
            vec![("tree-01".to_string(), 4)]
        );
    }
    /// A graph with ore in it survives a JSON round trip.
    ///
    /// It could not until 2026-09-03. `resources` and `minables` key their
    /// inner maps by [`Pos`], a two-field tuple struct, and `serde_json`
    /// refuses the whole document with `key must be a string` when one turns
    /// up as a key -- so `FactorioSurface`'s hand-written `Serialize`, which
    /// delegates here, failed on any world that had ever seen an ore tile or a
    /// tree. Nothing noticed because nothing wrote a world to disk: the only
    /// graphs that ever serialised were empty ones. Offline planning is
    /// exactly the thing that writes one, so this is the regression test that
    /// keeps it writable.
    #[test]
    fn a_graph_with_ore_and_trees_survives_a_json_round_trip() {
        let mut entities = vec![tree_at("tree-01", Position::new(5.5, 5.5))];
        crate::test_utils::spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(4., 4.), &Position::new(-40., 40.)),
            "iron-ore",
        );
        let graph = entity_graph_from(entities).expect("adding must not fail");

        let json = serde_json::to_string(&graph).expect("a graph with ore serialises");
        let back: EntityGraph = serde_json::from_str(&json).expect("and comes back");

        assert_eq!(
            back.minable_positions("tree-01"),
            graph.minable_positions("tree-01")
        );
        assert_eq!(
            back.resource_fingerprint(),
            graph.resource_fingerprint(),
            "the ore tiles and their amounts came back unchanged"
        );
    }

    /// And the same graph serialises to the same bytes every time.
    ///
    /// `resources` and `minables` are `DashMap`s, which iterate in hash order;
    /// a dump that wrote them in that order would differ between two processes
    /// holding the identical world, which makes a dump useless as an identity
    /// for a map. `TileMaps` sorts the names and the inner `BTreeMap`s are
    /// already in tile order.
    #[test]
    fn two_dumps_of_one_graph_are_the_same_bytes() {
        let mut entities = vec![
            tree_at("tree-01", Position::new(5.5, 5.5)),
            tree_at("tree-01", Position::new(-9.5, 3.5)),
        ];
        for (ore, at) in [
            ("iron-ore", Position::new(-40., 40.)),
            ("copper-ore", Position::new(-40., 0.)),
            ("coal", Position::new(-60., 0.)),
        ] {
            crate::test_utils::spawn_ore(
                &mut entities,
                add_to_rect(&Rect::from_wh(4., 4.), &at),
                ore,
            );
        }
        let graph = entity_graph_from(entities).expect("adding must not fail");
        let first = serde_json::to_string(&graph).expect("serialises");
        for _ in 0..8 {
            assert_eq!(
                serde_json::to_string(&graph).expect("serialises"),
                first,
                "the same graph wrote different bytes"
            );
        }
    }

    /// A machine standing here with a recipe on it.
    fn assembler(at: &Position) -> FactorioEntity {
        FactorioEntity {
            name: "assembling-machine-1".into(),
            entity_type: "assembling-machine".into(),
            position: at.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(2.4, 2.4), at),
            ..Default::default()
        }
    }

    /// **The recipe an RCON call put on a machine has to reach the model, and
    /// this is the only door it has.**
    ///
    /// A machine is built with no recipe and given one afterwards, so the
    /// *created* event carries `recipe: None` and nothing later corrects it:
    /// `add` refuses a tile that is already occupied, and
    /// `on_some_entity_updated` is a no-op the mod raises only on rotation.
    /// The planner's `Goal::Producing` counts machines by the recipe stored on
    /// them, so before this every replan read zero cells however many stood --
    /// see `set_recipe`'s own doc for the run that cost.
    #[test]
    fn a_recipe_set_on_a_standing_machine_is_readable_back() {
        let at = Position::new(6.5, -35.5);
        let graph = entity_graph_from(vec![assembler(&at)]).expect("adding must not fail");
        assert_eq!(
            graph
                .find_entities_in_radius(at.clone(), 1., None, None)
                .first()
                .and_then(|e| e.recipe.clone()),
            None,
            "a machine is built empty; anything else here would be invented"
        );

        assert!(graph.set_recipe(&at, "automation-science-pack"));

        assert_eq!(
            graph
                .find_entities_in_radius(at.clone(), 1., None, None)
                .first()
                .and_then(|e| e.recipe.clone()),
            Some("automation-science-pack".to_string()),
            "the reader the planner uses has to see it"
        );
        assert_eq!(
            graph
                .entity_at(&at)
                .and_then(|id| graph.entity_by_id(id))
                .and_then(|e| e.recipe),
            Some("automation-science-pack".to_string()),
            "and so does the by-tile reader `PlanState::entity_at` goes through"
        );
        assert_eq!(
            graph
                .find_entities_in_radius(at.clone(), 1., None, None)
                .len(),
            1,
            "the machine was updated in place, not duplicated"
        );
    }

    /// Setting a recipe on bare ground reports that it landed nowhere rather
    /// than inventing a machine to hang it on.
    #[test]
    fn a_recipe_set_where_nothing_stands_is_refused() {
        let graph = entity_graph_from(vec![]).expect("adding must not fail");
        assert!(!graph.set_recipe(&Position::new(6.5, -35.5), "automation-science-pack"));
    }

    /// A second recipe replaces the first. A machine has exactly one.
    #[test]
    fn setting_a_second_recipe_replaces_the_first() {
        let at = Position::new(6.5, -35.5);
        let graph = entity_graph_from(vec![assembler(&at)]).expect("adding must not fail");
        assert!(graph.set_recipe(&at, "iron-gear-wheel"));
        assert!(graph.set_recipe(&at, "automation-science-pack"));
        assert_eq!(
            graph
                .entity_at(&at)
                .and_then(|id| graph.entity_by_id(id))
                .and_then(|e| e.recipe),
            Some("automation-science-pack".to_string())
        );
    }
    // -----------------------------------------------------------------------
    // Threats: the enemy structures the mod has always been sending
    // -----------------------------------------------------------------------

    /// Built the way `serialize_entity` builds them, copied from a real
    /// `workspace/server-log.txt` line rather than invented: a spawner reports
    /// `entity_type: "unit-spawner"` with a ~4.4-tile box, a worm reports
    /// `"turret"`, a live biter reports `"unit"`.
    fn enemy_at(name: &str, entity_type: &str, position: Position, size: f64) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: entity_type.into(),
            bounding_box: crate::factorio::util::add_to_rect(&Rect::from_wh(size, size), &position),
            position,
            ..Default::default()
        }
    }

    /// The regression this whole map exists for.
    ///
    /// `EntityType::from_str` is `Err` for all three of these spellings, and
    /// before `threats` existed that meant an arriving nest left nothing
    /// behind but an anonymous rectangle in `blocked_tree`. The assertion on
    /// `from_str` is deliberate: it pins *why* the separate map is needed, so
    /// that anyone who later adds a `UnitSpawner` variant to `EntityType`
    /// finds this test rather than a silent duplicate.
    #[test]
    fn an_enemy_structure_is_remembered_by_name_and_a_biter_is_not() {
        for spelling in ["unit-spawner", "turret", "unit"] {
            assert!(
                EntityType::from_str(spelling).is_err(),
                "{spelling} is not an EntityType, which is why `threats` is a separate map"
            );
        }

        let graph = entity_graph_from(vec![
            enemy_at(
                "biter-spawner",
                "unit-spawner",
                Position::new(-237.5, 66.5),
                4.4,
            ),
            enemy_at(
                "small-worm-turret",
                "turret",
                Position::new(-242.1, 66.1),
                1.6,
            ),
            enemy_at("small-biter", "unit", Position::new(-240.0, 66.0), 0.4),
        ])
        .expect("adding must not fail");

        assert_eq!(
            graph.threat_census(),
            [
                ("biter-spawner".to_string(), 1),
                ("small-worm-turret".to_string(), 1)
            ]
            .into_iter()
            .collect::<BTreeMap<String, usize>>(),
            "the two structures are remembered; the biter walks, so it is not"
        );
    }

    // -----------------------------------------------------------------------
    // Vision extent: the free ground the model was given
    // -----------------------------------------------------------------------

    /// The shape of the finding itself: ore near spawn, a nest far out, and
    /// the extent has to report the far one and name it.
    ///
    /// The census is checked alongside, because the distance on its own cannot
    /// be told apart from a single stray entity 500 tiles away -- and a reader
    /// deciding whether a run's number carries an asterisk needs to know
    /// whether the model holds one thing out there or a thousand.
    #[test]
    fn vision_extent_reports_the_furthest_thing_the_model_holds_and_names_it() {
        let mut entities = vec![];
        spawn_ore(
            &mut entities,
            Rect::new(&Position::new(-2., -2.), &Position::new(0., 0.)),
            "iron-ore",
        );
        entities.push(enemy_at(
            "biter-spawner",
            "unit-spawner",
            Position::new(-300.5, 400.5),
            4.4,
        ));
        let graph = entity_graph_from(entities).expect("adding must not fail");

        let extent = graph.vision_extent().expect("the model holds something");
        assert_eq!(extent.name, "biter-spawner");
        assert_eq!(extent.position, Position::new(-300.5, 400.5));
        assert_eq!(extent.enemy_structures, 1);
        assert_eq!(extent.resource_tiles, 9, "a 3x3 of ore tiles");
        // Euclidean, not Manhattan. `Position::distance` is Manhattan despite
        // its name and would answer 701 for this point; the radius is 500.7.
        assert!(
            (extent.tiles - 300.5f64.hypot(400.5)).abs() < 1e-9,
            "expected the radius, got {}",
            extent.tiles
        );
        assert!(
            extent.tiles < Position::new(0., 0.).manhattan_distance(&Position::new(-300.5, 400.5)),
            "Manhattan is strictly larger off the axes, and this must not be it"
        );
    }

    /// The half-tile that has bitten this project twice: resource keys are
    /// floored, and a reach read back out of the map must add the centre back
    /// or every resource reads 0.5 short.
    #[test]
    fn vision_extent_restores_the_half_tile_a_resource_key_floors_away() {
        let mut entities = vec![];
        spawn_ore(
            &mut entities,
            Rect::new(&Position::new(100., 0.), &Position::new(100., 0.)),
            "crude-oil",
        );
        let graph = entity_graph_from(entities).expect("adding must not fail");

        let extent = graph.vision_extent().expect("the model holds something");
        assert_eq!(extent.name, "crude-oil");
        assert_eq!(
            extent.position,
            Position::new(100.5, 0.5),
            "the tile centre, exactly as `resource_position_from_pos` restores it"
        );
    }

    /// **`None` is "nothing has been read", never "the model reaches zero".**
    ///
    /// A run whose world model was never fed would otherwise disclose a free
    /// vision of 0.0 tiles -- a confident claim that it cheated by nothing,
    /// made by an instrument that had not looked. That is the exact failure
    /// shape this project has hit four times, so it is pinned here.
    #[test]
    fn vision_extent_is_none_for_a_model_nothing_has_been_read_into() {
        let graph = entity_graph_from(vec![]).expect("adding must not fail");
        assert!(
            graph.vision_extent().is_none(),
            "an unread model has no extent; it does not have an extent of zero"
        );
    }

    /// Two things at the same range must not swap places between processes.
    /// `resources` and `threats` are `DashMap`s, whose iteration order is
    /// seeded per process, so a run comparing its extent against yesterday's
    /// would otherwise see the name change for no reason.
    #[test]
    fn vision_extent_breaks_ties_deterministically() {
        let mut names = std::collections::BTreeSet::new();
        for _ in 0..8 {
            let graph = entity_graph_from(vec![
                enemy_at(
                    "biter-spawner",
                    "unit-spawner",
                    Position::new(300.0, 400.0),
                    4.4,
                ),
                enemy_at(
                    "spitter-spawner",
                    "unit-spawner",
                    Position::new(-300.0, 400.0),
                    4.4,
                ),
            ])
            .expect("adding must not fail");
            let extent = graph.vision_extent().expect("the model holds something");
            assert!((extent.tiles - 500.0).abs() < 1e-9);
            names.insert(extent.name);
        }
        assert_eq!(
            names.len(),
            1,
            "one answer across every iteration order, got {names:?}"
        );
    }

    /// Nearest-first, and the position handed back is the game's own -- the
    /// half-tile lesson `minables` learned, which matters here for the same
    /// reason: a caller asking the mod about a nest matches on that position.
    #[test]
    fn threats_come_back_nearest_first_with_the_reported_position() {
        let far = Position::new(-237.5, 66.5);
        let near = Position::new(10.5, -3.5);
        let graph = entity_graph_from(vec![
            enemy_at("biter-spawner", "unit-spawner", far.clone(), 4.4),
            enemy_at("spitter-spawner", "unit-spawner", near.clone(), 4.4),
        ])
        .expect("adding must not fail");

        let from = Position::new(0., 0.);
        let ordered = graph.threats_from(&from);
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].0, "spitter-spawner");
        assert_eq!(ordered[0].1, near, "the position the game reported");
        assert_eq!(ordered[1].0, "biter-spawner");

        let (name, at, distance) = graph.nearest_threat(&from).expect("one is charted");
        assert_eq!(name, "spitter-spawner");
        assert_eq!(at, near);
        assert!(
            (distance - calculate_distance(&from, &near)).abs() < f64::EPSILON,
            "the distance reported is the EUCLIDEAN distance from the point asked about"
        );
        // And specifically not Manhattan. `near` is diagonal, so the two
        // metrics disagree (11.07 against 14.0) -- which is what makes this
        // assertion able to catch a revert. Manhattan over-reports a diagonal
        // by up to 41%, so a nest would read as further away than it is, and
        // callers use this to decide whether somewhere is safe to walk to.
        assert!(
            (distance - from.manhattan_distance(&near)).abs() > 1.0,
            "a Manhattan answer here would say a nest is further than it is"
        );
    }

    /// An empty model answers `None`, and that answer is "nothing charted".
    /// It is asserted here so the distinction stays written down where the
    /// query lives: a caller that reads it as "safe" has misused it.
    #[test]
    fn no_charted_threat_is_not_a_claim_of_safety() {
        let graph = entity_graph_from(vec![]).expect("adding must not fail");
        assert!(graph.nearest_threat(&Position::new(0., 0.)).is_none());
        assert!(graph.threat_census().is_empty());
    }

    /// Killing a nest takes it out, so the model does not keep refusing a
    /// route past something that is gone.
    #[test]
    fn removing_an_enemy_structure_forgets_it() {
        let at = Position::new(-237.5, 66.5);
        let spawner = enemy_at("biter-spawner", "unit-spawner", at.clone(), 4.4);
        let graph = entity_graph_from(vec![spawner.clone()]).expect("adding must not fail");
        assert!(graph.nearest_threat(&at).is_some());
        graph.remove(&spawner).expect("removing must not fail");
        assert!(
            graph.nearest_threat(&at).is_none(),
            "the nest is gone from the model as well as from the map"
        );
    }

    /// Threats survive a clone and a serde round trip, and a graph serialised
    /// before this map existed still loads -- the same contract `minables`
    /// has, checked the same way, because a new field that broke yesterday's
    /// world dumps would be a worse bug than the one it fixes.
    #[test]
    fn threats_survive_cloning_and_a_round_trip_and_old_dumps_still_load() {
        let at = Position::new(-237.5, 66.5);
        let graph = entity_graph_from(vec![enemy_at(
            "biter-spawner",
            "unit-spawner",
            at.clone(),
            4.4,
        )])
        .expect("adding must not fail");

        let copy = graph.clone();
        assert_eq!(copy.threat_census(), graph.threat_census());

        let json = serde_json::to_string(&graph).expect("serialising must not fail");
        let back: EntityGraph = serde_json::from_str(&json).expect("deserialising must not fail");
        assert_eq!(back.threat_census(), graph.threat_census());
        assert_eq!(back.nearest_threat(&at).map(|t| t.1), Some(at));

        let mut older: serde_json::Value =
            serde_json::from_str(&json).expect("reparsing must not fail");
        older
            .as_object_mut()
            .expect("the graph serialises as an object")
            .remove("threats")
            .expect("the field was there to remove");
        let older: EntityGraph =
            serde_json::from_value(older).expect("a dump without `threats` must still load");
        assert!(
            older.threat_census().is_empty(),
            "it knows of no nests, which is not the same as asserting there are none"
        );
    }

    /// A standing machine, built the way the mod reports one: the fixture
    /// prototype's collision box around its position.
    fn standing(name: &str, entity_type: EntityType, at: Position) -> FactorioEntity {
        let collision = fixture_entity_prototypes()
            .get(name)
            .map(|proto| proto.collision_box.clone())
            .unwrap_or_else(|| panic!("the fixture has no prototype for {name}"));
        FactorioEntity {
            name: name.into(),
            entity_type: entity_type.to_string(),
            bounding_box: crate::factorio::util::add_to_rect(&collision, &at),
            position: at,
            ..Default::default()
        }
    }

    /// The four ore tiles a burner drill at `drill` stands on, each holding
    /// `amount`. Real tile centres, half-tile offsets included.
    fn ore_under(drill: &Position, name: &str, amount: u32) -> Vec<FactorioEntity> {
        let mut out = Vec::new();
        for dx in [-0.5, 0.5] {
            for dy in [-0.5, 0.5] {
                let mut ore = FactorioEntity::new_resource(
                    &Position::new(drill.x() + dx, drill.y() + dy),
                    Direction::North,
                    name,
                );
                ore.amount = Some(amount);
                out.push(ore);
            }
        }
        out
    }

    /// The defect behind `run-1788559688-08406`'s two failed actions. A
    /// burner drill eats the four tiles under itself; the moment one ran dry
    /// the mod reported the tile deleted, `remove` swept `blocked_tree` with
    /// the ore's bounding box, and the drill's own box -- the one around it
    /// -- went with it. Six of the seven drills plan 1 had built were
    /// unblocked this way before plan 2 was made, and plan 2 sent bots to
    /// hand-mine the ore still under two of them: `expected iron-ore at
    /// (-7.5/-29.5), found burner-mining-drill`.
    ///
    /// Both doors to `remove` are tried: the mod's `on_some_entity_deleted`
    /// (an amount of 0) and the executor's own debit through
    /// `resource_mined`. The drill's graph node is checked as well, because
    /// `entity_at` on the ore's centre found the drill and unhooked it too.
    #[test]
    fn a_tile_running_dry_under_a_drill_does_not_unblock_the_drill() {
        let iron = EntityName::IronOre.to_string();
        let drill_at = Position::new(-7., -29.);
        let mut entities = ore_under(&drill_at, &iron, 10);
        entities.push(standing(
            "burner-mining-drill",
            EntityType::MiningDrill,
            drill_at.clone(),
        ));
        let graph = entity_graph_from(entities).expect("adding must not fail");
        let footprint = Rect::new(&Position::new(-8., -30.), &Position::new(-6., -28.));
        assert_eq!(
            graph.blocking_boxes_within(&footprint).len(),
            1,
            "the drill blocks the ground it stands on"
        );
        assert!(graph.node_at(&drill_at).is_some(), "and has a node");

        // The mod's report for the tile the drill has just emptied.
        let mut emptied =
            FactorioEntity::new_resource(&Position::new(-6.5, -28.5), Direction::North, &iron);
        emptied.amount = Some(0);
        graph.remove(&emptied).expect("removing must not fail");
        assert!(
            !graph.resource_contains(&iron, Pos(-7, -29)),
            "the emptied tile leaves the model"
        );
        assert_eq!(
            graph.blocking_boxes_within(&footprint).len(),
            1,
            "the drill still blocks the ground it stands on"
        );
        assert!(
            graph.entity_at(&drill_at).is_some() && graph.node_at(&drill_at).is_some(),
            "and is still in the graph, by name and by node"
        );

        // The executor's own debit reaches `remove` through `retire_resource`.
        assert_eq!(
            graph.resource_mined(&iron, &Position::new(-7.5, -29.5), 10),
            ResourceDepletion::Exhausted
        );
        assert_eq!(
            graph.blocking_boxes_within(&footprint).len(),
            1,
            "a tile the model retires itself leaves the drill standing too"
        );
        assert!(graph.node_at(&drill_at).is_some());
    }

    /// The other half of the same rule: removing a machine still frees its
    /// own ground -- the whole point of `retire_minable` -- and only its own.
    /// A furnace touching its east edge and a chest touching its south edge
    /// keep their boxes.
    #[test]
    fn removing_a_machine_frees_its_own_ground_and_no_neighbours() {
        let drill_at = Position::new(-7., -29.);
        let furnace_at = Position::new(-5., -29.);
        let chest_at = Position::new(-7.5, -27.5);
        let graph = entity_graph_from(vec![
            standing(
                "burner-mining-drill",
                EntityType::MiningDrill,
                drill_at.clone(),
            ),
            standing("stone-furnace", EntityType::Furnace, furnace_at.clone()),
            standing("wooden-chest", EntityType::Container, chest_at.clone()),
        ])
        .expect("adding must not fail");
        let around = Rect::new(&Position::new(-9., -31.), &Position::new(-3., -26.));
        assert_eq!(graph.blocking_boxes_within(&around).len(), 3);

        graph
            .remove(&standing(
                "burner-mining-drill",
                EntityType::MiningDrill,
                drill_at.clone(),
            ))
            .expect("removing must not fail");

        let left = graph.blocking_boxes_within(&around);
        assert_eq!(left.len(), 2, "one box gone, two kept: {left:?}");
        assert!(
            left.iter().all(|b| !b.contains(&drill_at)),
            "the drill's own box is the one that went"
        );
        assert!(left.iter().any(|b| b.contains(&furnace_at)));
        assert!(left.iter().any(|b| b.contains(&chest_at)));
        assert!(graph.entity_at(&drill_at).is_none());
        assert!(graph.entity_at(&furnace_at).is_some());
        assert!(graph.entity_at(&chest_at).is_some());
    }

    /// The live 2.1.17 fluid boxes for the entities the tests below stand up,
    /// overlaid on the captured fixture map.
    ///
    /// **`tests/entity-prototype-fixtures.json` is a Factorio 1.x capture and
    /// its fluid boxes use the OLD convention**, where a `positions` entry
    /// names the tile outside the entity the connection reaches: its
    /// `oil-refinery` says `(-1, 3)`, which is a tile beyond a 5x5 footprint,
    /// and its `connection_type` values are `input` / `output` /
    /// `input-output`, which 2.0 replaced with `normal` / `underground`. Live
    /// 2.x data says `(-1, 2)` for the same connection -- a tile ON the
    /// refinery -- and `normal`. Both are read (see [`FluidBoxPorts::certain`]),
    /// and each needs its own fixture to be exercised, so these are quoted
    /// verbatim from `workspace/scripts/map-31337-t0.json`, a seed-31337
    /// `world.dump` written by this project against Factorio 2.1.17.
    fn live_2_1_prototypes() -> DashMap<String, FactorioEntityPrototype> {
        let prototypes = fixture_entity_prototypes();
        let live: HashMap<String, FactorioEntityPrototype> = serde_json::from_str(
            r#"{
  "oil-refinery": {
    "name": "oil-refinery", "entity_type": "assembling-machine",
    "collision_box": {"left_top": {"x": -2.19921875, "y": -2.19921875},
                      "right_bottom": {"x": 2.19921875, "y": 2.19921875}},
    "fluidbox_prototypes": [
      {"production_type": "input", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":-1.0,"y":2.0},{"x":-2.0,"y":-1.0},{"x":1.0,"y":-2.0},{"x":2.0,"y":1.0}]}]},
      {"production_type": "input", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":1.0,"y":2.0},{"x":-2.0,"y":1.0},{"x":-1.0,"y":-2.0},{"x":2.0,"y":-1.0}]}]},
      {"production_type": "output", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":0.0,"y":-2.0},{"x":2.0,"y":0.0},{"x":0.0,"y":2.0},{"x":-2.0,"y":0.0}]}]}
    ]
  },
  "pipe": {
    "name": "pipe", "entity_type": "pipe",
    "collision_box": {"left_top": {"x": -0.2890625, "y": -0.2890625},
                      "right_bottom": {"x": 0.2890625, "y": 0.2890625}},
    "fluidbox_prototypes": [
      {"production_type": "none", "pipe_connections": [
        {"connection_type": "normal", "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]},
        {"connection_type": "normal", "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]},
        {"connection_type": "normal", "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]},
        {"connection_type": "normal", "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]}]}
    ]
  },
  "storage-tank": {
    "name": "storage-tank", "entity_type": "storage-tank",
    "collision_box": {"left_top": {"x": -1.296875, "y": -1.296875},
                      "right_bottom": {"x": 1.296875, "y": 1.296875}},
    "fluidbox_prototypes": [
      {"production_type": "none", "pipe_connections": [
        {"connection_type": "normal", "positions": [{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0}]},
        {"connection_type": "normal", "positions": [{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0}]},
        {"connection_type": "normal", "positions": [{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0}]},
        {"connection_type": "normal", "positions": [{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0}]}]}
    ]
  },
  "chemical-plant": {
    "name": "chemical-plant", "entity_type": "assembling-machine",
    "collision_box": {"left_top": {"x": -1.19921875, "y": -1.19921875},
                      "right_bottom": {"x": 1.19921875, "y": 1.19921875}},
    "fluidbox_prototypes": [
      {"production_type": "input", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0}]}]},
      {"production_type": "input", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0}]}]},
      {"production_type": "output", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0}]}]},
      {"production_type": "output", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0},{"x":1.0,"y":-1.0}]}]}
    ]
  },
  "pumpjack": {
    "name": "pumpjack", "entity_type": "mining-drill",
    "collision_box": {"left_top": {"x": -1.19921875, "y": -1.19921875},
                      "right_bottom": {"x": 1.19921875, "y": 1.19921875}},
    "fluidbox_prototypes": [
      {"production_type": "output", "pipe_connections": [{"connection_type": "normal",
        "positions": [{"x":1.0,"y":-1.0},{"x":1.0,"y":1.0},{"x":-1.0,"y":1.0},{"x":-1.0,"y":-1.0}]}]}
    ]
  },
  "pipe-to-ground": {
    "name": "pipe-to-ground", "entity_type": "pipe-to-ground",
    "collision_box": {"left_top": {"x": -0.2890625, "y": -0.2890625},
                      "right_bottom": {"x": 0.2890625, "y": 0.19921875}},
    "fluidbox_prototypes": [
      {"production_type": "none", "pipe_connections": [
        {"connection_type": "normal", "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]},
        {"connection_type": "underground", "max_underground_distance": 10,
         "positions": [{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0},{"x":0.0,"y":0.0}]}]}
    ]
  }
}"#,
        )
        .expect("the live prototypes parse");
        for (name, prototype) in live {
            prototypes.insert(name, prototype);
        }
        prototypes
    }

    fn fluid_entity(
        name: &str,
        entity_type: &str,
        half: f64,
        position: Position,
    ) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: entity_type.into(),
            bounding_box: add_to_rect(&Rect::from_wh(half * 2., half * 2.), &position),
            position,
            ..Default::default()
        }
    }

    fn graph_of(
        entities: Vec<FactorioEntity>,
        prototypes: DashMap<String, FactorioEntityPrototype>,
    ) -> EntityGraph {
        let graph = EntityGraph::new(Arc::new(prototypes), Arc::new(fixture_recipes()));
        graph.add(entities, None).expect("adding must not fail");
        graph.connect().expect("connecting must not fail");
        graph
    }

    /// **The defect this rule was written for.** A pipe against an oil
    /// refinery's declared input drew no edge, because
    /// `EntityType::is_fluid_input` named four types and an assembling machine
    /// was not one of them. On the 6:39:53 world-record base that left all 55
    /// refineries with no entity-graph edge at all, incoming or outgoing, and
    /// `petroleum-gas` -- 28,800/min of demand -- with no modelled producer.
    #[test]
    fn a_pipe_reaches_the_assembling_machine_the_type_list_refused() {
        let refinery = Position::new(10.5, 10.5);
        let feeding = Position::new(9.5, 13.5);
        let draining = Position::new(10.5, 7.5);
        let graph = graph_of(
            vec![
                fluid_entity(
                    "oil-refinery",
                    "assembling-machine",
                    2.199_218_75,
                    refinery.clone(),
                ),
                fluid_entity("pipe", "pipe", 0.289_062_5, feeding.clone()),
                fluid_entity("pipe", "pipe", 0.289_062_5, draining.clone()),
            ],
            live_2_1_prototypes(),
        );
        assert!(
            has_edge(&graph, &feeding, &refinery),
            "the pipe on the refinery's input connection must feed it"
        );
        assert!(
            has_edge(&graph, &refinery, &draining),
            "the pipe on the refinery's output connection must drain it"
        );
        // **`production_type` decides the direction, and it is what keeps a
        // pumpjack and an offshore pump roots of the flow walk.** An `input`
        // box never gives fluid back and an `output` box never takes any.
        assert!(
            !has_edge(&graph, &refinery, &feeding),
            "an input box must not hand fluid back to the pipe feeding it"
        );
        assert!(
            !has_edge(&graph, &draining, &refinery),
            "an output box must not accept fluid from the pipe it drains into"
        );
    }

    /// A pipe touching a machine somewhere it declares no connection joins
    /// nothing, which is the half a footprint-wide `node_at` cannot do alone:
    /// it answers for any of a refinery's twenty-five tiles.
    ///
    /// The negative is paired with a positive from the same graph, so it cannot
    /// pass because the wiring never ran.
    #[test]
    fn a_pipe_against_a_machines_blank_side_joins_nothing() {
        let refinery = Position::new(10.5, 10.5);
        let blank = Position::new(13.5, 10.5);
        let connected = Position::new(11.5, 13.5);
        let graph = graph_of(
            vec![
                fluid_entity(
                    "oil-refinery",
                    "assembling-machine",
                    2.199_218_75,
                    refinery.clone(),
                ),
                fluid_entity("pipe", "pipe", 0.289_062_5, blank.clone()),
                fluid_entity("pipe", "pipe", 0.289_062_5, connected.clone()),
            ],
            live_2_1_prototypes(),
        );
        assert!(
            has_edge(&graph, &connected, &refinery),
            "the declared input connection must still join"
        );
        assert!(
            !has_edge(&graph, &blank, &refinery),
            "a pipe against a tile the refinery declares no connection on must not join it"
        );
        assert!(
            !has_edge(&graph, &refinery, &blank),
            "and not in the other direction either"
        );
    }

    /// The four joints the deleted `EntityType::StorageTank` arm listed by hand
    /// -- `(-1,-2)`, `(-2,-1)`, `(2,1)`, `(1,2)` -- come back out of the
    /// prototype, and the four tiles between them do not.
    ///
    /// This is the case the geometry resolves EXACTLY rather than as a
    /// superset: the tank's two corner anchors carry two connections each, so
    /// both of each corner's two outward sides are used.
    #[test]
    fn a_storage_tanks_joints_are_the_ones_the_hand_written_list_had() {
        let tank = Position::new(20.5, 20.5);
        let joints = [
            Position::new(19.5, 18.5),
            Position::new(18.5, 19.5),
            Position::new(22.5, 21.5),
            Position::new(21.5, 22.5),
        ];
        let blanks = [
            Position::new(20.5, 18.5),
            Position::new(18.5, 20.5),
            Position::new(22.5, 20.5),
            Position::new(20.5, 22.5),
        ];
        let mut entities = vec![fluid_entity(
            "storage-tank",
            "storage-tank",
            1.296_875,
            tank.clone(),
        )];
        for position in joints.iter().chain(blanks.iter()) {
            entities.push(fluid_entity("pipe", "pipe", 0.289_062_5, position.clone()));
        }
        let graph = graph_of(entities, live_2_1_prototypes());
        for joint in &joints {
            assert!(
                has_edge(&graph, joint, &tank) && has_edge(&graph, &tank, joint),
                "the tank must join both ways at {joint}"
            );
        }
        for blank in &blanks {
            assert!(
                !has_edge(&graph, blank, &tank),
                "the tank declares no connection at {blank}"
            );
        }
    }

    /// **A fluid producer keeps no incoming edge, so the flow walk keeps its
    /// roots.** `FlowGraph::update` starts from
    /// `externals(Direction::Incoming)` filtered to offshore pumps and drills
    /// on ore; one edge drawn back into a pumpjack would take it off that list
    /// and silently delete every rate downstream of it.
    #[test]
    fn nothing_draws_an_edge_back_into_a_fluid_producer() {
        let pumpjack = Position::new(40.5, 40.5);
        let outlet = Position::new(41.5, 38.5);
        let graph = graph_of(
            vec![
                fluid_entity("pumpjack", "mining-drill", 1.199_218_75, pumpjack.clone()),
                fluid_entity("pipe", "pipe", 0.289_062_5, outlet.clone()),
            ],
            live_2_1_prototypes(),
        );
        assert!(
            has_edge(&graph, &pumpjack, &outlet),
            "the pumpjack's output box must reach the pipe on it"
        );
        assert!(
            !has_edge(&graph, &outlet, &pumpjack),
            "an output box must take nothing back, or the pumpjack stops being a flow root"
        );
        assert!(
            graph
                .inner_graph()
                .externals(petgraph::Direction::Incoming)
                .any(|index| graph
                    .inner_graph()
                    .node_weight(index)
                    .is_some_and(|node| node.entity_name == "pumpjack")),
            "the pumpjack must still be a root of the walk"
        );
    }

    /// **Both `positions` conventions are read, and neither is guessed from a
    /// version number.** The 1.x fixture names the tile outside the entity; the
    /// 2.x capture names the tile on it. The same two pipes join the same
    /// storage tank under both.
    #[test]
    fn a_fluid_box_is_read_under_either_positions_convention() {
        let tank = Position::new(20.5, 20.5);
        let joint = Position::new(19.5, 18.5);
        let build = |prototypes: DashMap<String, FactorioEntityPrototype>| {
            graph_of(
                vec![
                    fluid_entity("storage-tank", "storage-tank", 1.296_875, tank.clone()),
                    fluid_entity("pipe", "pipe", 0.289_062_5, joint.clone()),
                ],
                prototypes,
            )
        };
        // The 1.x fixture states the outside tile directly, so it is exact and
        // needs none of the corner reasoning the 2.x form does.
        let old = build(fixture_entity_prototypes());
        let new = build(live_2_1_prototypes());
        assert!(
            has_edge(&old, &joint, &tank),
            "the 1.x fixture's outward positions must still join"
        );
        assert!(
            has_edge(&new, &joint, &tank),
            "the 2.x capture's anchor positions must join the same tiles"
        );
        assert_eq!(
            edge_set(&old),
            edge_set(&new),
            "the same arrangement must wire the same way under both conventions"
        );
    }

    /// The one box of the node at `position`, for the assertions below.
    fn boxes_at(graph: &EntityGraph, position: &Position) -> Vec<FluidBoxPorts> {
        let index = graph.node_at(position).expect("the entity must be there");
        let inner = graph.inner_graph();
        let node = inner.node_weight(index).expect("the node must be there");
        graph.fluid_boxes(node)
    }

    /// **A corner with as many connections as it has sides is exact.** A
    /// storage tank's two corner anchors carry two connections each, so both of
    /// each corner's two outward sides are used and nothing is guessed -- which
    /// is why the four tiles come back identical to the hand-written list the
    /// prototype rule replaced.
    #[test]
    fn a_corner_with_a_connection_per_side_is_resolved_exactly() {
        let tank = Position::new(20.5, 20.5);
        let graph = graph_of(
            vec![fluid_entity(
                "storage-tank",
                "storage-tank",
                1.296_875,
                tank.clone(),
            )],
            live_2_1_prototypes(),
        );
        let boxes = boxes_at(&graph, &tank);
        assert_eq!(boxes.len(), 4, "the tank declares four connections");
        for resolved in &boxes {
            assert!(
                resolved.certain,
                "a corner with two connections on it leaves nothing to guess: {resolved:?}"
            );
            assert_eq!(
                resolved.ports.len(),
                2,
                "each corner reaches its two outward tiles: {resolved:?}"
            );
        }
    }

    /// **A corner with fewer connections than sides is NOT exact, and says so.**
    /// A chemical plant's four connections each sit on a corner tile of its 3x3
    /// footprint, so each could face either of two ways, and the datum that
    /// would settle it -- `PipeConnectionDefinition::direction` -- is not one
    /// the mod sends. The candidate set is emitted whole and flagged, rather
    /// than one of the two being picked.
    ///
    /// The flag is asserted beside the anchor and the candidate tiles from the
    /// same call, so this cannot pass on a computation that did not run.
    #[test]
    fn a_corner_the_data_cannot_settle_is_reported_as_a_superset() {
        let plant = Position::new(30.5, 30.5);
        let graph = graph_of(
            vec![fluid_entity(
                "chemical-plant",
                "assembling-machine",
                1.199_218_75,
                plant.clone(),
            )],
            live_2_1_prototypes(),
        );
        let boxes = boxes_at(&graph, &plant);
        assert_eq!(boxes.len(), 4, "the plant declares four connections");
        let north_west = boxes
            .iter()
            .find(|resolved| resolved.anchor == Position::new(29.5, 29.5))
            .expect("one connection sits on the north-west corner tile");
        assert!(
            !north_west.certain,
            "one connection on a corner could face either of two ways"
        );
        assert_eq!(
            north_west.ports,
            vec![Position::new(29.5, 28.5), Position::new(28.5, 29.5)],
            "and both of them are offered"
        );
        assert!(
            north_west.takes_in && !north_west.gives_out,
            "the plant's north-west box is an input"
        );
    }

    /// **An underground connector faces the way it points, and that is exact.**
    /// A pipe-to-ground's normal connection sits on its own single tile, which
    /// has four outward sides and only one real one -- the ambiguity above,
    /// with 751 of them on the record base. It is settled by the box also
    /// carrying an `underground` connection, which is what an underground
    /// connector is: keyed on `max_underground_distance` being present rather
    /// than on the type being `PipeToGround`, so a mod's own underground pipe
    /// is read the same way.
    #[test]
    fn an_underground_connector_faces_the_way_it_points() {
        let underground = Position::new(50.5, 50.5);
        let entity = FactorioEntity {
            direction: Direction::East as u8,
            ..fluid_entity(
                "pipe-to-ground",
                "pipe-to-ground",
                0.289_062_5,
                underground.clone(),
            )
        };
        let graph = graph_of(vec![entity], live_2_1_prototypes());
        let boxes = boxes_at(&graph, &underground);
        assert_eq!(
            boxes.len(),
            1,
            "the underground connection contributes no surface tile of its own"
        );
        assert!(boxes[0].certain, "the facing settles it");
        assert_eq!(
            boxes[0].ports,
            vec![Position::new(51.5, 50.5)],
            "an east-facing underground pipe opens to the east"
        );
    }
}

/// One fluid box of one standing entity, resolved into world tiles: where it
/// sits on the entity, which tiles a neighbour must occupy to join it, and
/// which way fluid may cross.
///
/// This is what replaced `EntityType::is_fluid_input` (deleted with it) as the rule
/// [`EntityGraph::connect_node`] joins fluid handlers by. The type test named
/// four types -- `Pipe`, `PipeToGround`, `StorageTank`, `Boiler` -- so a pipe
/// never drew an edge into an assembling machine, and on the world-record base
/// **all 55 oil refineries and all 896 steam engines had no entity-graph edge
/// at all, incoming or outgoing**, along with 109 of 146 chemical plants. That
/// is a mod-compatibility defect of exactly the shape this project has recorded
/// before (`pole_supply_half_extent`, the copied smelting rate): a hard-coded
/// name or type list standing in for data the game already sends.
#[derive(Debug, Clone, PartialEq)]
pub struct FluidBoxPorts {
    /// The tile of the entity's own footprint this fluid box connects through.
    pub anchor: Position,
    /// The tiles OUTSIDE the entity a neighbour must occupy to join this box.
    ///
    /// More than one means the prototype could not say which side the
    /// connection faces -- see [`Self::certain`].
    pub ports: Vec<Position>,
    /// Fluid may enter the entity here (`production_type` `input`,
    /// `input-output` or `none`).
    pub takes_in: bool,
    /// Fluid may leave the entity here (`production_type` `output`,
    /// `input-output` or `none`).
    pub gives_out: bool,
    /// Whether [`Self::ports`] is the connection set or a superset of it.
    ///
    /// **The datum that would make this always true is one field the mod does
    /// not send.** `positions[direction]` names the tile of the entity the
    /// connection sits ON, not the tile it faces, and 2.0 moved the facing into
    /// `PipeConnectionDefinition::direction`, which
    /// `mods/BotBridge/types.lua::serialize_fluidbox_connection` does not
    /// serialise. So the facing is recovered here from geometry, which is exact
    /// whenever the anchor tile touches the footprint's edge on one side only,
    /// and ambiguous at a corner: a chemical plant's four connections all sit on
    /// corner tiles, and each could face either of two ways.
    ///
    /// Two cases are still exact and neither is a name list:
    ///
    /// - **As many connections share the anchor as it has candidate sides**, so
    ///   every candidate is used. That is a storage tank, whose two corner
    ///   anchors carry two connections each -- and the four tiles this yields
    ///   are exactly the four the deleted `EntityType::StorageTank` arm listed
    ///   by hand. It is also an ordinary pipe: four connections on its own
    ///   single tile, four sides.
    /// - **The box also carries an `underground` connection at the same
    ///   anchor**, which is what a pipe-to-ground is. Such a connector faces
    ///   `direction` on the surface and burrows the opposite way, so the normal
    ///   connection is exactly one tile. Keyed on the connection carrying
    ///   `max_underground_distance` rather than on the type being
    ///   `PipeToGround`, so a mod's own underground pipe gets it too.
    ///
    /// What is left uncertain is emitted as the whole candidate set, because an
    /// edge too many is a reachability question and an edge too few is 55
    /// refineries the flow graph swore had no producer.
    pub certain: bool,
}

impl EntityGraph {
    /// Every fluid box of the entity behind `node`, resolved into world tiles.
    ///
    /// Empty when the prototype declares no fluid box, when it is unknown, or
    /// when the entity faces one of the twelve non-cardinal directions 2.0
    /// added: `pipe_connections[].positions` holds exactly four offsets, one
    /// per cardinal facing, so there is no offset to read for the other twelve
    /// and this returns nothing rather than an invented tile.
    pub fn fluid_boxes(&self, node: &EntityNode) -> Vec<FluidBoxPorts> {
        let facing = node.direction as usize;
        if !facing.is_multiple_of(4) {
            return vec![];
        }
        let facing = facing / 4;
        let Some(prototype) = self.entity_prototypes.get(&node.entity_name) else {
            return vec![];
        };
        let Some(fluid_boxes) = prototype.fluidbox_prototypes.as_ref() else {
            return vec![];
        };
        // How many normal connections sit on each anchor tile of the whole
        // prototype, which is what makes the storage tank's two-per-corner
        // exact. Counted across every box, because two boxes may share a tile.
        let mut normals_at: Vec<(Position, usize)> = vec![];
        for fluid_box in fluid_boxes {
            for connection in Self::normal_connections(fluid_box) {
                let Some(offset) = connection.positions.get(facing) else {
                    continue;
                };
                let anchor = node.position.add(offset);
                if !node.bounding_box.contains(&anchor) {
                    // A 1.x external tile, which shares nothing and needs no
                    // count -- see the convention note below.
                    continue;
                }
                match normals_at.iter_mut().find(|(at, _)| same_tile(at, &anchor)) {
                    Some(slot) => slot.1 += 1,
                    None => normals_at.push((anchor, 1)),
                }
            }
        }
        let mut resolved: Vec<FluidBoxPorts> = vec![];
        for fluid_box in fluid_boxes {
            let takes_in = matches!(
                fluid_box.production_type.as_str(),
                "input" | "input-output" | "none"
            );
            let gives_out = matches!(
                fluid_box.production_type.as_str(),
                "output" | "input-output" | "none"
            );
            let burrows = Self::connections_of(fluid_box)
                .any(|connection| connection.max_underground_distance.is_some());
            for connection in Self::normal_connections(fluid_box) {
                let Some(offset) = connection.positions.get(facing) else {
                    continue;
                };
                let stated = node.position.add(offset);
                // **The two conventions, told apart by the datum itself.**
                // A 1.x `positions` entry names the tile OUTSIDE the entity the
                // connection reaches; a 2.0 one names the tile of the entity's
                // own footprint the connection sits on, with the facing moved
                // into `PipeConnectionDefinition::direction`. Whether the tile
                // is inside the footprint says which, per connection, with no
                // version number to read and none available offline anyway.
                //
                // The 1.x form is exact and needs none of the machinery below:
                // it already IS the tile a neighbour must occupy.
                if !node.bounding_box.contains(&stated) {
                    resolved.push(FluidBoxPorts {
                        anchor: stated.clone(),
                        ports: vec![stated],
                        takes_in,
                        gives_out,
                        certain: true,
                    });
                    continue;
                }
                let anchor = stated;
                let candidates: Vec<Position> = Direction::orthogonal()
                    .into_iter()
                    .filter_map(|direction| move_position(&anchor, direction, 1.))
                    .filter(|tile| !node.bounding_box.contains(tile))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let shared = normals_at
                    .iter()
                    .find(|(at, _)| same_tile(at, &anchor))
                    .map(|(_, count)| *count)
                    .unwrap_or(1);
                let (ports, certain) = if burrows {
                    match move_position(&anchor, node.direction, 1.) {
                        Some(tile) if candidates.iter().any(|c| same_tile(c, &tile)) => {
                            (vec![tile], true)
                        }
                        _ => (candidates, false),
                    }
                } else if shared >= candidates.len() {
                    (candidates, true)
                } else {
                    (candidates, false)
                };
                resolved.push(FluidBoxPorts {
                    anchor,
                    ports,
                    takes_in,
                    gives_out,
                    certain,
                });
            }
        }
        resolved
    }

    fn connections_of(
        fluid_box: &FactorioFluidBoxPrototype,
    ) -> impl Iterator<Item = &FactorioFluidBoxConnection> {
        (*fluid_box.pipe_connections).iter().flatten()
    }

    /// Connections a neighbour on the surface could join. An `underground` one
    /// is reached by the pairing walk in [`Self::connect_node`], not by
    /// adjacency, so it must not contribute a surface tile.
    fn normal_connections(
        fluid_box: &FactorioFluidBoxPrototype,
    ) -> impl Iterator<Item = &FactorioFluidBoxConnection> {
        Self::connections_of(fluid_box)
            .filter(|connection| connection.max_underground_distance.is_none())
    }
}

/// Whether two positions name the same tile.
///
/// Exact `f64` equality would very nearly do -- every offset here is a whole or
/// half tile and both sides compute the same sum -- but "very nearly" is how
/// the resource-position rounding bug survived a full test suite, so the
/// comparison is explicit.
fn same_tile(a: &Position, b: &Position) -> bool {
    (a.x() - b.x()).abs() < 0.01 && (a.y() - b.y()).abs() < 0.01
}
