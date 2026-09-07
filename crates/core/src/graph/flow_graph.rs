use crate::aabb_quadtree::{ItemId, QuadTree};
use crate::factorio::util::{add_to_rect, format_dotgraph};
use crate::graph::entity_graph::{EntityGraph, EntityNode, QuadTreeRect};
use crate::num_traits::FromPrimitive;
use crate::types::{
    Direction, EntityName, EntityType, FactorioEntity, FactorioEntityPrototype, FactorioRecipe,
    Position, Rect,
};
use dashmap::DashMap;
use euclid::{Point2D, Size2D};
use miette::Result;
use num_traits::ToPrimitive;
use parking_lot::{RwLock, RwLockReadGuard};
use petgraph::dot::{Config, Dot};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Bfs, Control, DfsEvent, EdgeRef, depth_first_search};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::{error, warn};

/// The `built_generation` of a graph that has never been walked.
///
/// A sentinel rather than `Option<u64>` so the check is one atomic load, and
/// `u64::MAX` rather than 0 because 0 is a real generation -- the one a freshly
/// constructed or freshly deserialised [`EntityGraph`] is at. With 0 as the
/// sentinel, a flow graph built beside an untouched entity graph would call
/// itself current while holding nothing at all.
const NEVER_BUILT: u64 = u64::MAX;

/// How far [`FlowGraph::ration`] steps toward a line's target each round.
///
/// Small enough that the iteration walks down from the ceiling rather than
/// jumping past a fixed point into a lower basin -- see that function for why
/// the **greatest** fixed point is the physical one.
///
/// Not a tuning knob, and measured rather than argued: on the world-record
/// base the mean absolute log error over the fourteen items the game reports
/// is **0.141 at both 0.05 and 0.1**, 0.143 at 0.25, and then degrades
/// smoothly toward the old descent's 0.184 as the step grows -- 0.150 at 0.5
/// and 0.210 at 0.9. A halving of the step that moves the answer by 0.2% is a
/// converged solution; a full step is not.
const RATION_DAMPING: f64 = 0.1;

/// The largest movement [`FlowGraph::ration`] will call converged. Small
/// enough that a caller may compare a balanced rate against an exact
/// expectation within 1e-9, which its unit tests do.
const RATION_TOLERANCE: f64 = 1e-12;

/// A bound rather than "until it converges", so a pathological recipe cycle
/// cannot hang a caller. The world-record base's 3,722 lines reach
/// [`RATION_TOLERANCE`] in 2,406 rounds for phase one and 4,353 for phase
/// two, in about a second, so the cap is loose by a factor of four.
const RATION_ROUNDS: usize = 20_000;

/// One machine making one item, and what that costs it.
///
/// The unit of the whole-base balance in
/// [`FlowGraph::sustained_production_rates`]: a producer appears once per
/// **product**, so a furnace credited with two outputs is throttled
/// independently on each.
#[derive(Debug, Clone)]
struct ProductionLine {
    /// What this machine makes.
    item: String,
    /// How much of it, per second, at nameplate -- ingredients assumed.
    rate: f64,
    /// What one second at `rate` eats, per ingredient. Empty for a drill and
    /// an offshore pump, which take their output from the ground.
    needs: BTreeMap<String, f64>,
}

pub struct FlowGraph {
    entity_graph: Arc<EntityGraph>,
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    recipes: Arc<DashMap<String, FactorioRecipe>>,
    flow_tree: RwLock<FlowQuadTree>,
    inner: RwLock<FlowGraphInner>,
    /// The [`EntityGraph::generation`] the contents of `inner` were walked
    /// from, or [`NEVER_BUILT`].
    built_generation: AtomicU64,
    /// Held for the length of a rebuild so two readers finding the graph stale
    /// at the same moment produce one walk rather than two interleaved ones.
    ///
    /// It is not the lock that protects `inner` -- that is `inner`'s own
    /// `RwLock`, which the walk takes and releases many times. This one exists
    /// only to make the *decision* to rebuild single.
    rebuilding: parking_lot::Mutex<()>,
}

impl Clone for FlowGraph {
    fn clone(&self) -> Self {
        FlowGraph {
            entity_graph: Arc::new((*self.entity_graph).clone()),
            entity_prototypes: Arc::new((*self.entity_prototypes).clone()),
            recipes: Arc::new((*self.recipes).clone()),
            flow_tree: RwLock::new((*self.flow_tree.read()).clone()),
            inner: RwLock::new((*self.inner.read()).clone()),
            built_generation: AtomicU64::new(self.built_generation.load(Ordering::Acquire)),
            rebuilding: parking_lot::Mutex::new(()),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.entity_graph = Arc::new((*source.entity_graph).clone());
        self.entity_prototypes = Arc::new((*source.entity_prototypes).clone());
        self.recipes = Arc::new((*source.recipes).clone());
        self.flow_tree = RwLock::new((*source.flow_tree.read()).clone());
        self.inner = RwLock::new((*source.inner.read()).clone());
        self.built_generation = AtomicU64::new(source.built_generation.load(Ordering::Acquire));
        self.rebuilding = parking_lot::Mutex::new(());
    }
}

impl FlowGraph {
    pub fn new(entity_graph: Arc<EntityGraph>) -> Self {
        FlowGraph {
            entity_prototypes: entity_graph.entity_prototypes(),
            recipes: entity_graph.recipes(),
            entity_graph,
            flow_tree: RwLock::new(fresh_flow_tree()),
            inner: RwLock::new(FlowGraphInner::new()),
            built_generation: AtomicU64::new(NEVER_BUILT),
            rebuilding: parking_lot::Mutex::new(()),
        }
    }

    /// Walk the entity graph again if it has moved on since this graph was
    /// built, and do nothing if it has not.
    ///
    /// **Every public reader calls this first.** That is what makes a stale
    /// answer structurally impossible to observe rather than merely unlikely:
    /// there is no accessor on this type that can hand out a number computed
    /// from a world that has since changed. It is the design's option A
    /// (rebuild on demand, keyed by a generation counter) and it was chosen
    /// over incremental invalidation for a reason that is about correctness
    /// and not effort --
    ///
    /// **a delta refresh cannot be made correct here.** `node_at` matches on
    /// **position alone** and returns before the prototype is consulted, so a
    /// furnace built where a chest stood inherits the chest's flow node, type
    /// and contents. Visiting only what changed fixes the stale *nodes* and
    /// keeps that silently, which is worse than the frozen graph this
    /// replaces, because it would look maintained. If a rebuild ever shows up
    /// in a profile, incremental invalidation is the optimisation and this is
    /// the oracle to test it against -- and it must be tested on a world where
    /// a position is *reused*, which is exactly the case a naive delta passes
    /// by construction.
    ///
    /// Re-entrant by construction: [`FlowGraph::update`] records the
    /// generation **before** it walks, so the `node_at` calls the walk itself
    /// makes find the graph current and do not recurse.
    fn ensure_current(&self) {
        let generation = self.entity_graph.generation();
        if self.built_generation.load(Ordering::Acquire) == generation {
            return;
        }
        let _guard = self.rebuilding.lock();
        // Another reader may have rebuilt while we waited for the lock.
        if self.built_generation.load(Ordering::Acquire) == generation {
            return;
        }
        if let Err(report) = self.update() {
            error!("<red>flow graph rebuild failed</>: {report:?}");
        }
    }

    /// Walks the entity graph from every source root and writes flow edges,
    /// **discarding whatever was here before**.
    ///
    /// Callers should not normally need this: every public reader on this type
    /// calls [`FlowGraph::ensure_current`], which calls this when and only when
    /// the entity graph has moved on. It stays public for the two callers that
    /// predate the generation counter -- `OutputParser::on_init` and
    /// `factorio::snapshot` -- for which an eager first walk is still the
    /// cheapest place to pay for one.
    ///
    /// # It clears first, and that is not an optimisation
    ///
    /// It used to accumulate: the walk wrote into `self.inner`, which was never
    /// emptied, so a second call on a *changed* world added nodes beside the
    /// stale ones. On an *unchanged* world it was idempotent
    /// (`get_or_create_flow_node` reuses the node at a position and
    /// `update_edge` replaces a weight), which is presumably why nobody
    /// noticed -- the obvious test, call it twice and compare, came back clean.
    ///
    /// Two things went wrong on a changed world and only one of them was
    /// staleness. A removed entity's node and edges stayed forever; and, worse,
    /// [`FlowGraph::node_at`] matches on **position alone** and returns before
    /// the prototype is consulted, so a furnace built where a chest stood
    /// inherited the chest's flow node, type and contents. Clearing is what
    /// makes the second impossible.
    ///
    /// The generation is recorded **before** the walk, so the `node_at` calls
    /// the walk makes see a current graph and `ensure_current` does not
    /// recurse.
    pub fn update(&self) -> Result<()> {
        let _started = Instant::now();
        // Claim the generation first: everything below reads through
        // `node_at`, which calls `ensure_current`.
        self.built_generation
            .store(self.entity_graph.generation(), Ordering::Release);
        *self.inner.write() = FlowGraphInner::new();
        *self.flow_tree.write() = fresh_flow_tree();
        let inner = self.entity_graph.inner_graph();
        for entity_root_index in inner.externals(petgraph::Direction::Incoming) {
            let entity_root = inner.node_weight(entity_root_index).unwrap();
            if entity_root.entity_type == EntityType::OffshorePump
                || (entity_root.entity_type == EntityType::MiningDrill
                    && entity_root.miner_ore.is_some())
            {
                let entity_graph = self.entity_graph.inner_graph();
                depth_first_search(&*entity_graph, Some(entity_root_index), |event| {
                    if let DfsEvent::TreeEdge(source_node_index, target_node_index) = event {
                        let source_node = entity_graph.node_weight(source_node_index).unwrap();
                        let target_node = entity_graph.node_weight(target_node_index).unwrap();
                        match source_node.entity_type {
                            EntityType::MiningDrill => {
                                // The ore and the speed are *this* drill's, not
                                // the root's. They are the same entity only in
                                // the common case of a lone drill at the head of
                                // the walk; a drill reached through a fuel
                                // inserter is a different drill mining a
                                // different ore, and reading `entity_root` here
                                // reported it as producing the root's ore at the
                                // root's rate.
                                //
                                // Reading the root also made the `unwrap` below
                                // look safe -- the root filter guarantees a
                                // `miner_ore` for a `MiningDrill` root -- while
                                // an `OffshorePump` root, which the power-plant
                                // work now places for real, carries `None` and
                                // would have taken the whole process down:
                                // `[profile.release]` sets `panic = "abort"`.
                                let Some(miner_ore) = source_node.miner_ore.as_ref() else {
                                    // Not an invariant violation. `EntityGraph::add`
                                    // stores `None` for a drill with no resource
                                    // under it and warns as it does so, and such a
                                    // drill produces nothing -- so the honest edge
                                    // is no edge, the same policy
                                    // `get_or_create_flow_node` documents for a node
                                    // it cannot build. Said again rather than
                                    // inherited, because this is the walk that turns
                                    // it into a missing flow.
                                    warn!(
                                        "no flow out of {} @ {}: no ore under it",
                                        source_node.entity_name, source_node.position
                                    );
                                    return Control::Continue;
                                };
                                let mining_speed = self
                                    .entity_prototypes
                                    .get(&source_node.entity_name)
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "entity '{}' not found in prototypes",
                                            source_node.entity_name
                                        )
                                    })
                                    .mining_speed
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "entity '{}' has no mining_speed",
                                            source_node.entity_name
                                        )
                                    })
                                    .to_f64()
                                    .unwrap();
                                let mining_time = self
                                    .entity_prototypes
                                    .get(miner_ore)
                                    .unwrap_or_else(|| {
                                        panic!("entity '{}' not found in prototypes", miner_ore)
                                    })
                                    .mining_time
                                    .unwrap_or_else(|| {
                                        panic!("entity '{}' has no mining_time", miner_ore)
                                    })
                                    .to_f64()
                                    .unwrap();
                                // https://wiki.factorio.com/Mining
                                // The rate at which resources are produced is given by:
                                // Mining speed / Mining time = Production rate (in resource/sec)
                                let production_rate = mining_speed / mining_time;
                                self.update_flow_edge(
                                    FlowEdge::Single(vec![(miner_ore.clone(), production_rate)]),
                                    source_node,
                                    target_node,
                                );
                                Control::Continue
                            }
                            EntityType::OffshorePump => {
                                self.update_flow_edge(
                                    FlowEdge::Single(vec![(EntityName::Water.to_string(), 1.)]),
                                    source_node,
                                    target_node,
                                );
                                Control::Continue
                            }
                            EntityType::AssemblingMachine => {
                                // can have multiple incoming and multiple outgoing
                                let tree = self.entity_graph.inner_tree();
                                let entity = tree.get(source_node.entity_id.unwrap()).unwrap();

                                if let Some(recipe) = entity.recipe.as_ref() {
                                    if let Some(recipe) = self.recipes.get(recipe) {
                                        let crafting_speed =
                                            self.crafting_speed(&source_node.entity_name);
                                        let seconds = recipe.energy.to_f64().unwrap_or_default();
                                        let mut output: FlowRates = vec![];
                                        for product in recipe.products.iter() {
                                            // Derived, not assumed: how many
                                            // this recipe yields, times how
                                            // fast this machine runs it, over
                                            // how long one run takes. The `/
                                            // 3.2` this replaces was the
                                            // *smelting* time of iron applied
                                            // to every assembler recipe, and
                                            // ignored `crafting_speed`
                                            // entirely -- an
                                            // `assembling-machine-1` is 0.5 and
                                            // was reported at 1x, an
                                            // `assembling-machine-3` is 1.25.
                                            //
                                            // A recipe whose `energy` is zero
                                            // or unreadable would divide by
                                            // zero, so it yields no edge rather
                                            // than an infinity.
                                            if seconds <= 0. {
                                                continue;
                                            }
                                            output.push((
                                                product.name.clone(),
                                                f64::from(product.amount) * crafting_speed
                                                    / seconds,
                                            ));
                                        }
                                        self.update_flow_edge(
                                            FlowEdge::Single(output),
                                            source_node,
                                            target_node,
                                        );
                                        Control::Continue
                                    } else {
                                        // warn!("recipe not found: {}", recipe);
                                        Control::Prune
                                    }
                                } else {
                                    Control::Prune
                                }
                            }
                            EntityType::Splitter => {
                                // can have multiple incoming and multiple outgoing
                                let incoming =
                                    self.sum_incoming_edge_weights(&source_node.position);
                                let outgoing_count = entity_graph
                                    .edges_directed(
                                        source_node_index,
                                        petgraph::Direction::Outgoing,
                                    )
                                    .count();
                                self.update_flow_edge(
                                    self.divide_flowrate(&incoming, outgoing_count),
                                    source_node,
                                    target_node,
                                );
                                Control::Continue
                            }
                            EntityType::Furnace => {
                                // can have multiple incoming and multiple outgoing
                                let incoming =
                                    self.sum_incoming_edge_weights(&source_node.position);
                                // Every rate here is derived from the recipe
                                // table and this furnace's own prototype. Coal
                                // needs no special case: it is an ingredient of
                                // no smelting recipe, so `smelting_output`
                                // answers `None` for it exactly as it does for
                                // anything else a furnace does not smelt, and
                                // fuel falls out of the data rather than out of
                                // a name.
                                //
                                // **A furnace runs one recipe at a time**, so
                                // its outputs share its time rather than each
                                // getting the whole of it. This used to add a
                                // full-rate edge per smeltable input, which on
                                // a mixed belt reported one furnace smelting
                                // iron *and* copper *and* stone at 100% each.
                                // Measured against the 6:39:53 world-record
                                // save on 2026-09-06: stone-brick came out at
                                // 7,125/min against the game's 450, because
                                // stone reaches furnaces the game has set to
                                // iron. The share is the input's share of the
                                // smeltable ore arriving -- the only signal
                                // this graph has about what the furnace spends
                                // its time on, since `EntityType::Furnace`
                                // carries no recipe.
                                let smeltable: Vec<(&String, f64, FlowRate)> = incoming
                                    .iter()
                                    .filter_map(|(name, rate)| {
                                        self.smelting_output(&source_node.entity_name, name)
                                            .map(|out| (name, *rate, out))
                                    })
                                    .collect();
                                let arriving: f64 = smeltable.iter().map(|(_, rate, _)| rate).sum();
                                let mut output: FlowRates = vec![];
                                if arriving > 0. {
                                    for (_, rate, (product, full)) in &smeltable {
                                        self.add_production_rate(
                                            &mut output,
                                            (product.clone(), full * rate / arriving),
                                        );
                                    }
                                }
                                self.update_flow_edge(
                                    FlowEdge::Single(output),
                                    source_node,
                                    target_node,
                                );
                                Control::Continue
                            }
                            EntityType::Container
                            | EntityType::LogisticContainer
                            | EntityType::PipeToGround
                            | EntityType::StorageTank
                            | EntityType::Pipe
                            | EntityType::Inserter => {
                                // can have one incoming and one outgoing
                                let incoming =
                                    self.sum_incoming_edge_weights(&source_node.position);
                                self.update_flow_edge(
                                    FlowEdge::Single(incoming),
                                    source_node,
                                    target_node,
                                );
                                Control::Continue
                            }
                            EntityType::TransportBelt | EntityType::UndergroundBelt => {
                                // can have multiple incoming and multiple outgoing
                                let entity_edge_count = entity_graph
                                    .edges_directed(
                                        source_node_index,
                                        petgraph::Direction::Incoming,
                                    )
                                    .count();
                                let (left, right) = self.sum_incoming_edge_weights_by_side(
                                    entity_edge_count,
                                    &source_node.position,
                                );
                                match target_node.entity_type {
                                    EntityType::TransportBelt
                                    | EntityType::UndergroundBelt
                                    | EntityType::Splitter => {
                                        self.update_flow_edge(
                                            FlowEdge::Double(left, right),
                                            source_node,
                                            target_node,
                                        );
                                        Control::Continue
                                    }
                                    EntityType::Inserter => {
                                        let mut both = left;
                                        for e in &right {
                                            self.add_production_rate(&mut both, e.clone());
                                        }
                                        self.update_flow_edge(
                                            FlowEdge::Single(both),
                                            source_node,
                                            target_node,
                                        );
                                        Control::Continue
                                    }
                                    _ => Control::Prune,
                                }
                            }
                            _ => Control::<()>::Prune,
                        }
                    } else {
                        Control::Continue
                    }
                });
            }
        }
        // info!("flow graph build took {:?}", started.elapsed());
        Ok(())
    }

    /// `None` when the entity cannot be turned into a `FlowNode` -- see
    /// `FlowNode::new`. The edge that wanted it is then dropped rather than
    /// drawn against a fabricated node.
    pub fn get_or_create_flow_node(&self, entity_node: &EntityNode) -> Option<NodeIndex> {
        if let Some(existing) = self.node_at(&entity_node.position) {
            return Some(existing);
        }
        let entity_id = entity_node.entity_id?;
        let entity = self.entity_graph.entity_by_id(entity_id)?;
        let node = FlowNode::new(&entity, entity_node.miner_ore.clone(), entity_id)?;
        let new_index = self.inner.write().add_node(node);
        self.flow_tree
            .write()
            .insert_with_box(new_index, entity_node.bounding_box.clone().into());
        Some(new_index)
    }

    pub fn update_flow_edge(
        &self,
        flow: FlowEdge,
        source_entity_node: &EntityNode,
        target_entity_node: &EntityNode,
    ) {
        let (Some(source_flow_idx), Some(target_flow_idx)) = (
            self.get_or_create_flow_node(source_entity_node),
            self.get_or_create_flow_node(target_entity_node),
        ) else {
            return;
        };
        self.inner
            .write()
            .update_edge(source_flow_idx, target_flow_idx, flow);
    }

    pub fn inner_graph(&self) -> RwLockReadGuard<'_, FlowGraphInner> {
        self.ensure_current();
        self.inner.read()
    }

    /// Items per second arriving at the entity standing at `position`, per item
    /// name, as the standing arrangement would deliver them.
    ///
    /// The smallest question this graph can answer, and the first one anything
    /// outside this file ever asked it. `Vec` rather than a map, and unsorted,
    /// because that is what every rate in this file already is
    /// ([`FlowRates`]); the order is the order the incoming edges were walked.
    ///
    /// # What an empty answer means, and what it does not
    ///
    /// **Empty is "this graph knows of nothing arriving here", never "nothing
    /// arrives here".** The walk starts only from offshore pumps and mining
    /// drills standing on ore, so a machine a *bot* hand-loads has no incoming
    /// edge and reads as zero -- correctly, for a question about what stands on
    /// its own, and misleadingly for a question about what the machine is
    /// producing. A caller that cannot tell those apart should not use this.
    ///
    /// # It does not model back-pressure, and a blocked sink reads as healthy
    ///
    /// Stated here rather than left to be discovered, because the failure is
    /// silent and travels the wrong way. Every rate in this graph is computed
    /// **forwards** from a source: a drill's ore rate flows down the belt, a
    /// furnace's output rate is what one furnace of that crafting speed
    /// produces. Nothing anywhere reduces an upstream rate because a
    /// downstream one cannot accept it.
    ///
    /// So an electric smelter whose unloading arm sits one tile outside pole
    /// coverage -- measured on this project on 2026-09-06: 0 plates in the
    /// sink, 61 stuck in the furnaces -- is reported by this function as
    /// delivering its full modelled rate at both ends. **Back-pressure travels
    /// backwards, so the symptom appears upstream of the cause**, and this
    /// model cannot see either end of that.
    ///
    /// Nor does it model **buffers**, which is what actually decides how a
    /// shared line splits. Measured the same day: two furnaces of six took 78%
    /// of the ore and two took three plates between them, because a fuel slot
    /// caps at 5 and refuses more while an ore input has no small ceiling, so
    /// the near arm absorbs everything. Coal balanced itself; ore did not. This
    /// function would predict an even split and be wrong.
    ///
    /// Read a number from here as **an upper bound under ideal distribution**,
    /// and never as evidence that a line is working.
    /// What every producer standing in this graph makes, per item name, in
    /// items per second -- the whole-base question, where [`FlowGraph::throughput_at`]
    /// answers one tile at a time.
    ///
    /// A producer is a mining drill, a furnace, an assembling machine or an
    /// offshore pump: the four arms of [`FlowGraph::update`] that *originate* a
    /// rate. Belts, inserters, pipes and splitters only carry one, so counting
    /// them would count the same items again at every tile they cross.
    ///
    /// # A machine is counted once, not once per outgoing edge
    ///
    /// [`FlowGraph::update_flow_edge`] writes a machine's **whole** output on
    /// **each** of its outgoing edges -- that is what makes
    /// `throughput_at` correct for any one of its consumers. A drill that both
    /// drops onto a belt and has an inserter picking out of it therefore has
    /// two edges of 0.5 ore/s, and it mines 0.5, not 1.0. So this takes the
    /// **maximum** per item across a node's outgoing edges rather than the sum.
    /// The world-record base has 1,544 drills and 1,222 furnaces, and summing
    /// would have inflated every one of them that feeds two things.
    ///
    /// # It is a CAPACITY, and the gap to a real base is not one thing
    ///
    /// Every caveat on [`FlowGraph::throughput_at`] applies and compounds here:
    /// no back-pressure, no buffers, and nothing that models a machine standing
    /// idle. Measured against the 6:39:53 Space Age world record save on
    /// 2026-09-06, this number is an **upper bound that the base runs at 87-95%
    /// of**, and it omits force bonuses (mining productivity was +10% there) and
    /// module and beacon effects entirely, neither of which this project models.
    /// See `docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`.
    pub fn production_rates(&self) -> BTreeMap<String, f64> {
        self.ensure_current();
        let mut total: BTreeMap<String, f64> = BTreeMap::new();
        for (_, made) in self.producer_nameplates() {
            for (name, rate) in made {
                *total.entry(name).or_insert(0.) += rate;
            }
        }
        total
    }

    /// One entry per producing machine: what it makes, per item, in items per
    /// second, **with unlimited ingredients**. The per-machine half of
    /// [`FlowGraph::production_rates`], factored out because
    /// [`FlowGraph::sustained_production_rates`] needs the machines and not
    /// just the sum.
    fn producer_nameplates(&self) -> Vec<(bool, BTreeMap<String, f64>)> {
        let graph = self.inner.read();
        let mut per_machine: Vec<(bool, BTreeMap<String, f64>)> = vec![];
        for node_index in graph.node_indices() {
            let Some(node) = graph.node_weight(node_index) else {
                continue;
            };
            if !matches!(
                node.entity_type,
                EntityType::MiningDrill
                    | EntityType::Furnace
                    | EntityType::AssemblingMachine
                    | EntityType::OffshorePump
            ) {
                continue;
            }
            let mut best: BTreeMap<String, f64> = BTreeMap::new();
            for edge in graph.edges_directed(node_index, petgraph::Direction::Outgoing) {
                let rates: Vec<&FlowRate> = match edge.weight() {
                    FlowEdge::Single(vec) => vec.iter().collect(),
                    FlowEdge::Double(left, right) => left.iter().chain(right.iter()).collect(),
                };
                for (name, rate) in rates {
                    let slot = best.entry(name.clone()).or_insert(0.);
                    if rate.total_cmp(slot).is_gt() {
                        *slot = *rate;
                    }
                }
            }
            if !best.is_empty() {
                // A drill and a pump take their output from the GROUND, not
                // from a recipe, so nothing may be charged against them. Coal
                // is where that bites: `coal` has a synthesis recipe in Space
                // Age, so charging every producer of an item through "the
                // recipe that makes it" billed 559 coal drills for carbon and
                // sulfur they never touch and reported the base making 46
                // coal a minute against a real 4,096.
                let crafts = matches!(
                    node.entity_type,
                    EntityType::Furnace | EntityType::AssemblingMachine
                );
                per_machine.push((crafts, best));
            }
        }
        per_machine
    }

    /// One entry per **product of one machine**: what it makes at nameplate,
    /// and what that costs it per second in ingredients.
    ///
    /// The unit is a machine's *output*, not the machine: a furnace this graph
    /// credits with copper plate and stone brick at once -- the mixed-belt
    /// artefact `update`'s furnace arm shares time between -- has two
    /// independent products, and a stone shortage must not throttle its
    /// copper. Gating the machine on the minimum over everything it touches
    /// took copper plate from 9% high to 12% low while the ore that feeds it
    /// was never short.
    ///
    /// `needs` is empty for a drill and an offshore pump: they take their
    /// output from the **ground**, not from a recipe, so nothing may be
    /// charged against them.
    fn nameplate_lines(&self) -> Vec<ProductionLine> {
        self.ensure_current();
        let nameplate = self.producer_nameplates();
        // What the base makes of each item before anything is throttled. Used
        // only to pick which recipe an item is being made BY -- see
        // `recipe_making`.
        let mut standing: BTreeMap<String, f64> = BTreeMap::new();
        for (_, made) in &nameplate {
            for (item, rate) in made {
                *standing.entry(item.clone()).or_insert(0.) += rate;
            }
        }
        let mut lines: Vec<ProductionLine> = vec![];
        for (crafts, made) in nameplate {
            for (item, rate) in made {
                let mut needs: BTreeMap<String, f64> = BTreeMap::new();
                if crafts
                    && let Some(recipe_name) = self.recipe_making(&item, &standing)
                    && let Some(recipe) = self.recipes.get(&recipe_name)
                    && let Some(product) = recipe.products.iter().find(|p| p.name == item)
                    && product.amount > 0
                {
                    let crafts = rate / f64::from(product.amount);
                    for ingredient in recipe.ingredients.iter().flatten() {
                        *needs.entry(ingredient.name.clone()).or_insert(0.) +=
                            crafts * f64::from(ingredient.amount);
                    }
                }
                lines.push(ProductionLine { item, rate, needs });
            }
        }
        lines
    }

    /// What the base can actually keep making, once no machine is allowed to
    /// consume more of an item than the base makes of it, **or to make more of
    /// one than anything takes**.
    ///
    /// [`FlowGraph::production_rates`] is a **nameplate** figure: every machine
    /// at 100%, ingredients assumed and output assumed to vanish. This adds
    /// both halves of idleness -- supply in
    /// [`FlowGraph::sustained_production_rates`]'s own balance, and demand in
    /// [`FlowGraph::balance`]'s outlet cap. Against the world-record base it
    /// takes the mean absolute log error over the fourteen items the game
    /// reports from **0.298 at nameplate to 0.141**.
    ///
    /// It is a prediction, and some of the fourteen are worse than nameplate.
    /// `production_rates_of_a_dumped_world` prints the whole table with the
    /// error, computed rather than quoted;
    /// `docs/superpowers/notes/2026-09-07-a-full-consumer-stops-pulling.md`
    /// and `2026-09-07-the-mall-was-not-the-problem.md` say what each
    /// regression means.
    ///
    /// # Why this is a WHOLE-BASE balance and not a per-machine duty cycle
    ///
    /// The obvious implementation is per machine: divide what
    /// `sum_incoming_edge_weights` says is arriving at a machine's tile by
    /// what its recipe eats. **That was built first and it is unsound**, and
    /// the falsification is
    /// `what_consumers_see_arriving_is_not_a_conserved_flow` in this file's
    /// tests. On the world-record base the supply a consumer *sees* runs from
    /// **0.02x to 1,194x** of the supply the graph says exists, because
    /// [`FlowGraph::update_flow_edge`] writes a machine's whole output on each
    /// of its outgoing edges and `sum_incoming_edge_weights` adds those up. It
    /// is not a bound in either direction, so a duty cycle derived from it is
    /// noise: shipping it moved iron plate by 2% and pushed steel plate from
    /// 31% high to 24% low.
    ///
    /// Summed over the whole surface those routing errors cancel, because
    /// [`FlowGraph::production_rates`] already takes a maximum per machine.
    /// **This aggregation is also what makes the answer robust to the graph's
    /// inability to route at all** -- ore that reaches a smelter by train, by
    /// bot, or through a chest nothing feeds is counted in the base's supply
    /// even though no edge carries it.
    ///
    /// # It throttles on shortage, and on surplus only where the surplus is
    ///
    /// Demand here is only what *modelled producers* consume. Everything else
    /// a base does with an item -- a wall, a rocket, a lab, a chest somebody
    /// fills -- is invisible, so demand is systematically understated and a
    /// **surplus proves nothing**. That is why no producer is ever capped at
    /// the consumption this model can see: on the world-record base the model
    /// sees 844 coal/min being eaten against 4,096 the game really makes,
    /// because a burner's fuel is not a recipe ingredient and nothing here
    /// charges it.
    ///
    /// What it does instead is decide, per line, **whose demand is real** --
    /// see [`FlowGraph::balance`].
    ///
    /// An item **nothing in the model produces** is unknown, not absent, and
    /// does not constrain anybody -- the same rule `runMatch.ts` applies to a
    /// missing run id. Otherwise every machine fed a fluid, or fed from
    /// another surface, would read as stopped.
    pub fn sustained_production_rates(&self) -> BTreeMap<String, f64> {
        self.ensure_current();
        let lines = self.nameplate_lines();
        let scale = Self::balance(&lines);

        let mut total: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            *total.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
        }
        total
    }

    /// How much of its nameplate each line can actually run at, given that the
    /// base's own supply is all any of them has to eat.
    ///
    /// Pure: it reads nothing but `lines`, so it is testable without a world
    /// and its result depends on no clock, no map and no iteration order.
    ///
    /// # A full consumer stops pulling, and that is the demand side
    ///
    /// The first version of this charged **every** consumer at nameplate --
    /// every machine assumed to pull its ingredients at its full rate, always.
    /// That is false in exactly the way the world-record base measures: at
    /// tick 1,443,169, **21.7% of its assemblers read `full_output`** and
    /// **49.3% of its inserters read `waiting_for_space_in_destination`**. A
    /// machine whose output has nowhere to go is not eating, and charging it
    /// as though it were invents a shortage that then propagates down the
    /// whole chain. It is why `processing-unit` came out at 0.54x the game's
    /// own statistics and `advanced-circuit` at 0.70x -- both *under*, while
    /// everything shallower was over.
    ///
    /// # Predicting it from topology and rates, never from an observed status
    ///
    /// `FactorioEntity::status` exists now and says which machines are backed
    /// up. **It is deliberately not read here.** A plan is scored on machines
    /// that do not exist yet, so a model that needs their status is an oracle
    /// that evaporates the moment it is needed. The census validates this
    /// prediction; it is not an input to it.
    ///
    /// What the model can see instead is **whether anything it knows about
    /// drains a line's output**:
    ///
    /// - A line whose product some other line eats has a **known drain**. It
    ///   keeps pulling, because something downstream keeps taking.
    /// - A line whose product **nothing in the model consumes** has an
    ///   *unknown* drain. On the record base that is the mall and the science
    ///   block -- roboports at 9/min, labs at 22.5/min, six science packs,
    ///   splitters, poles, solar panels: 42 items whose modelled consumption
    ///   is exactly zero. Their real drain is a bot request, a lab, or a
    ///   player, none of which this graph models.
    ///
    /// The rule is the same one the supply side already uses for an item
    /// nothing produces, applied to the other end: **unknown is not
    /// nameplate.** A known drain is served first; an unknown drain gets what
    /// is left. That is not an arbitrary tie-break -- it is the mechanism
    /// itself. A mall assembler with a full output chest stops its input
    /// inserters, which releases its share of the belt to the machines
    /// downstream that are still pulling.
    ///
    /// It never invents supply, only refuses to invent demand, so it can only
    /// move a rate **up** toward nameplate, never above it.
    ///
    /// # Two phases, because the priority is the whole point
    ///
    /// The known-drain lines are rationed to their fixed point first, and the
    /// unknown-drain lines are then fitted to what that leaves. This used to
    /// be justified by the solver instead -- the ration was a monotone
    /// descent, so a line clamped early could never take back what its
    /// neighbours freed -- and that reason expired when [`FlowGraph::ration`]
    /// became a fixed point a line may climb back up to. **The priority is
    /// the reason that remains**, and it is the one that was always doing the
    /// work: a known drain is served before an unknown one.
    ///
    /// Phase one may ignore the unknown-drain lines entirely rather than
    /// merely deprioritise them, and that is exact, not an approximation: a
    /// line is unknown-drain precisely because **no** line's `needs` mention
    /// its product, so it can supply nothing that phase one is rationing.
    ///
    /// # What the mall pulls, and why it is still nothing
    ///
    /// The brief for this shape warned that giving the mall zero is as much
    /// an extreme as giving it nameplate, and named `advanced-circuit` at
    /// 0.61x as the cost. **Measured on the record base, it is not.** Freeing
    /// `advanced-circuit`'s outlet cap completely -- an infinite mall pull on
    /// that one item -- moves it from 0.61 to 0.61: it was never
    /// outlet-limited. Nor does any mall prior help in aggregate. Charging
    /// the unknown-drain lines at a fraction of nameplate was measured at
    /// twenty-six settings across four families -- as a competitor for
    /// ingredients and as a relaxation of the outlet alone, uniform and split
    /// by whether the product is a building or a consumed good -- and **every
    /// one of them is worse than zero**, the error growing with the size of
    /// the prior. See
    /// `docs/superpowers/notes/2026-09-07-the-mall-was-not-the-problem.md`.
    fn balance(lines: &[ProductionLine]) -> Vec<f64> {
        // Every item some line eats. A line making one of these has a drain
        // this model can point at.
        let mut drained: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for line in lines {
            for ingredient in line.needs.keys() {
                drained.insert(ingredient.as_str());
            }
        }
        let known: Vec<bool> = lines
            .iter()
            .map(|line| drained.contains(line.item.as_str()))
            .collect();

        let mut scale = vec![1_f64; lines.len()];
        // Phase one: the lines whose output is drained, rationed against each
        // other, and each held to the outlet its own product has among them.
        Self::ration(
            lines,
            &mut scale,
            |index| known[index],
            &BTreeMap::new(),
            true,
        );
        // What phase one leaves on the table, per item.
        //
        // Keyed on what phase one **makes**, never on what it merely eats: an
        // item with no producer in the model is unknown rather than absent,
        // and a residual of `-demand` for it would read as "every gram is
        // spoken for" and stop a line dead on an ingredient the model has
        // simply never heard of -- a fluid off another surface, say.
        //
        // Every ingredient any line needs is by definition drained, so all of
        // its producers are phase-one lines: this map cannot miss supply that
        // phase two is entitled to.
        let mut made: BTreeMap<String, f64> = BTreeMap::new();
        let mut eaten: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            if !known[index] {
                continue;
            }
            *made.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
            for (ingredient, rate) in &line.needs {
                *eaten.entry(ingredient.clone()).or_insert(0.) += rate * scale[index];
            }
        }
        let residual: BTreeMap<String, f64> = made
            .into_iter()
            .map(|(item, rate)| {
                let left = rate - eaten.get(&item).copied().unwrap_or_default();
                (item, left.max(0.))
            })
            .collect();
        // Phase two: the rest, against that residual and against each other.
        // No outlet cap here -- a phase-two line is one whose outlet the model
        // cannot see at all, so there is nothing to cap it at.
        Self::ration(lines, &mut scale, |index| !known[index], &residual, false);
        scale
    }

    /// One fixed point over the lines `selected` picks, against `floor` -- the
    /// supply available to them before any of them makes anything, which is
    /// zero in phase one and the leftovers in phase two.
    ///
    /// # A line that stops pulling releases its share, and a line still
    /// pulling TAKES IT UP
    ///
    /// Until 2026-09-07 this was a **monotone descent**: a scale could only
    /// fall, and the loop stopped when nothing had fallen. That made it
    /// converge, and it made the second half of the sentence above
    /// impossible. The mechanism this file has claimed since the outlet cap
    /// landed -- *"a mall assembler with a full output chest stops its input
    /// inserters, which releases its share of the belt to the machines
    /// downstream that are still pulling"* -- was **written in the doc and
    /// absent from the code**: the release happened, and no line was ever
    /// allowed to climb back up and take it.
    ///
    /// Worse, the descent and the outlet cap compound. A shortage rations
    /// every consumer of an item proportionally, including the ones that are
    /// really running flat out; their reduced pull then caps their own
    /// suppliers, whose reduced output rations them again. On the
    /// world-record base the whole chain settles with **supply exactly equal
    /// to demand for every modelled item** -- the residual left for anything
    /// the model cannot see is exactly zero, for iron plate, gears, copper
    /// plate and advanced circuit alike -- which is not a property of that
    /// base but an artefact of the ratchet.
    ///
    /// So the scale is now solved as a **fixed point a line may climb back up
    /// to**: each round recomputes every selected line's target from the
    /// current allocation and steps a fraction [`RATION_DAMPING`] of the way
    /// there, up or down, never above the ceiling it started at. Measured on
    /// the record base this takes the mean absolute log error over the
    /// fourteen items the game reports from **0.184 to 0.141**.
    ///
    /// # Why damped, and why the damping is not a tuning knob
    ///
    /// The undamped iteration converges too, and to a **worse** fixed point:
    /// a full step overshoots downward and settles in a lower basin, landing
    /// back at 0.204. The system has many fixed points -- an outlet cap and a
    /// supply share can hold each other consistent at any level -- and the
    /// one that is physically right is the **greatest**, because a factory
    /// does not choose to run slower than it can. Stepping down from the
    /// ceiling in small steps stops at the first fixed point below it, which
    /// is that one.
    ///
    /// The sweep behind [`RATION_DAMPING`] is the evidence that the step size
    /// is not fitted to this base.
    ///
    /// With `cap_on_outlet`, a line is additionally held to what the selected
    /// lines eat of its own product: **a full consumer stops pulling**. That
    /// is the one place this model predicts `full_output` and
    /// `waiting_for_space_in_destination`, and it is predicted from recipes
    /// and rates, never from an observed status.
    ///
    /// Two exemptions, both the same rule the input side already applies:
    ///
    /// - **An item nothing here eats has an unknown outlet, not a zero one**,
    ///   and is not capped. Otherwise the deepest item in the chain -- the one
    ///   whose only consumers are the mall this phase excludes -- would be
    ///   held at zero. On the record base that is `processing-unit` exactly.
    /// - **A machine that takes its output from the ground is capped by
    ///   nothing**, the same exemption `the_ground_is_not_a_recipe` gives it
    ///   on the input side, and for the same reason: the model has neither its
    ///   bill nor its buyers. A burner's fuel is not a recipe ingredient, so
    ///   the model sees **844 coal/min being eaten on the record base against
    ///   a real 4,096** -- 21% of the true drain. Capping a coal drill at what
    ///   the model can see it feed would be wrong by a factor of five, and
    ///   measured to be.
    fn ration(
        lines: &[ProductionLine],
        scale: &mut [f64],
        selected: impl Fn(usize) -> bool,
        floor: &BTreeMap<String, f64>,
        cap_on_outlet: bool,
    ) {
        // What each line started at: its nameplate in phase one, and in phase
        // two whatever phase one left it. Nothing may climb above it.
        let ceiling: Vec<f64> = scale.to_vec();
        let mut next: Vec<f64> = scale.to_vec();
        // Each line's outlet ratio from the round before, so the relief below
        // can tell a line that is output-blocked from one that is merely
        // short. `INFINITY` is "nothing known to block it".
        let mut blocked: Vec<f64> = vec![f64::INFINITY; lines.len()];
        for _ in 0..RATION_ROUNDS {
            let mut supply: BTreeMap<String, f64> = floor.clone();
            let mut demand: BTreeMap<String, f64> = BTreeMap::new();
            for (index, line) in lines.iter().enumerate() {
                if !selected(index) {
                    continue;
                }
                *supply.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
                for (ingredient, rate) in &line.needs {
                    *demand.entry(ingredient.clone()).or_insert(0.) += rate * scale[index];
                }
            }
            // What each item's consumers would take **if that item were
            // abundant** -- see `FlowGraph::relieved_pull`.
            let pull = Self::relieved_pull(
                lines, scale, &ceiling, &supply, &demand, &blocked, &selected,
            );
            let mut worst = 0_f64;
            for (index, line) in lines.iter().enumerate() {
                if !selected(index) {
                    continue;
                }
                // How much of what it is running at now it may run at, given
                // what everybody else is doing. Above 1.0 means it may climb.
                let mut ratio = f64::INFINITY;
                for ingredient in line.needs.keys() {
                    match Self::supply_ratio(ingredient, &supply, &demand, floor) {
                        Some(share) => ratio = ratio.min(share),
                        None => continue,
                    }
                }
                // The outlet. `needs` is empty for a drill and an offshore
                // pump, which is the ground exemption.
                blocked[index] = f64::INFINITY;
                if cap_on_outlet && !line.needs.is_empty() {
                    let outlet = pull.get(&line.item).copied().unwrap_or_default();
                    let standing = supply.get(&line.item).copied().unwrap_or_default();
                    // Nobody here eats it: unknown outlet, not a closed one.
                    if outlet > 0. && standing > 0. {
                        blocked[index] = outlet / standing;
                        ratio = ratio.min(blocked[index]);
                    }
                }
                let target = if ratio.is_finite() {
                    (scale[index] * ratio).min(ceiling[index])
                } else {
                    // Every ingredient it needs is one nobody models, so
                    // nothing here holds it below its ceiling.
                    ceiling[index]
                };
                worst = worst.max((target - scale[index]).abs());
                next[index] = scale[index] + RATION_DAMPING * (target - scale[index]);
            }
            scale.copy_from_slice(&next);
            if worst < RATION_TOLERANCE {
                break;
            }
        }
    }

    /// A line's share of one ingredient, as a multiple of what it is running
    /// at: `Some(1.0)` means exactly its current draw is available.
    ///
    /// `None` is **unknown, not zero** -- nobody in the model makes the item,
    /// so it constrains nobody, or every machine fed a fluid or fed from
    /// another surface would read as stopped. `Some(0.0)` is the other case,
    /// and only phase two can see it: the item has a *residual* of zero, which
    /// means every gram is already spoken for rather than that the model has
    /// never heard of it.
    fn supply_ratio(
        ingredient: &str,
        supply: &BTreeMap<String, f64>,
        demand: &BTreeMap<String, f64>,
        floor: &BTreeMap<String, f64>,
    ) -> Option<f64> {
        let have = supply.get(ingredient).copied().unwrap_or_default();
        if have <= 0. {
            return if floor.contains_key(ingredient) {
                Some(0.)
            } else {
                None
            };
        }
        let want = demand.get(ingredient).copied().unwrap_or_default();
        if want > 0. { Some(have / want) } else { None }
    }

    /// Per item, what its consumers would take **if it were abundant**.
    ///
    /// # Being short of a thing is not the same as not wanting it
    ///
    /// The outlet cap asks "is anybody still pulling this?", and the honest
    /// answer must not count a consumer's own shortage *of this very item* as
    /// evidence that it has stopped pulling. Charging the outlet at what
    /// consumers are currently drawing does exactly that, and it makes a
    /// producer and its consumer **neutrally stable at any level**: each is
    /// consistent with the other at 90% of nameplate and equally consistent
    /// at 73%, so the answer becomes whatever the transient happened to leave,
    /// and the fixed point is no longer a property of the base.
    /// `a_line_that_stops_pulling_releases_its_share_to_one_that_has_not`
    /// measures that as 7.31 where the plate supply allows 9.
    ///
    /// So a consumer contributes what it would run at with every constraint
    /// it has EXCEPT its share of this item -- its other ingredients, its own
    /// outlet, and its ceiling. A machine that is genuinely output-blocked
    /// still contributes only its reduced pull, which is the whole point of
    /// the cap; a machine that is merely starved contributes what it would
    /// take.
    ///
    /// Its own outlet ratio is one round stale, which the iteration absorbs:
    /// at the fixed point the lag is nil, because nothing is moving.
    fn relieved_pull(
        lines: &[ProductionLine],
        scale: &[f64],
        ceiling: &[f64],
        supply: &BTreeMap<String, f64>,
        demand: &BTreeMap<String, f64>,
        blocked: &[f64],
        selected: &impl Fn(usize) -> bool,
    ) -> BTreeMap<String, f64> {
        let mut pull: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            if !selected(index) {
                continue;
            }
            // The smallest and second-smallest share among its ingredients,
            // so "the smallest among the others" is one lookup rather than a
            // rescan per ingredient.
            let mut smallest = f64::INFINITY;
            let mut runner_up = f64::INFINITY;
            let mut scarcest: Option<&str> = None;
            for ingredient in line.needs.keys() {
                let Some(share) = Self::supply_ratio(ingredient, supply, demand, &BTreeMap::new())
                else {
                    continue;
                };
                if share < smallest {
                    runner_up = smallest;
                    smallest = share;
                    scarcest = Some(ingredient.as_str());
                } else if share < runner_up {
                    runner_up = share;
                }
            }
            for (ingredient, rate) in &line.needs {
                let others = if scarcest == Some(ingredient.as_str()) {
                    runner_up
                } else {
                    smallest
                };
                let relieved = others.min(blocked[index]);
                let would_run = if relieved.is_finite() {
                    (scale[index] * relieved).min(ceiling[index])
                } else {
                    ceiling[index]
                };
                *pull.entry(ingredient.clone()).or_insert(0.) += rate * would_run;
            }
        }
        pull
    }

    /// The name of the recipe that makes `item`, for charging its ingredients.
    ///
    /// Only a recipe whose **first** product is `item` counts: a recipe is
    /// named for its main output, and taking any recipe that mentions the item
    /// among its products would charge a plate's ingredients to whatever
    /// by-product happened to sort first.
    ///
    /// `recycling` is excluded by category. Space Age gives almost every item
    /// a recycling recipe whose products are its own ingredients, so without
    /// this a plate would be charged as if it were made by recycling something
    /// that is made of plates -- a cycle, and a fictitious demand.
    ///
    /// Ties are broken by name and warned about, for the reason
    /// [`FlowGraph::smelting_recipe_taking`] gives: `self.recipes` is a
    /// `DashMap` whose iteration order is not stable.
    fn recipe_making(&self, item: &str, standing: &BTreeMap<String, f64>) -> Option<String> {
        let mut candidates: Vec<String> = self
            .recipes
            .iter()
            .filter(|recipe| recipe.valid && recipe.category != "recycling")
            .filter(|recipe| recipe.products.first().is_some_and(|p| p.name == item))
            .map(|recipe| recipe.name.clone())
            .collect();
        candidates.sort();
        if candidates.len() > 1 {
            // The base makes what it has the ingredients for. Sorting alone
            // picked `casting-iron` over `iron-plate` and charged every plate
            // to molten iron, which nothing on Nauvis makes -- so the charge
            // was against an item with no supply, and `sustained_production_
            // rates` skips those as unknown. The whole iron and copper
            // constraint silently did nothing, and the table still looked
            // plausible because the numbers merely stayed at nameplate.
            if let Some(supplied) = candidates.iter().find(|name| {
                self.recipes.get(*name).is_some_and(|recipe| {
                    recipe
                        .ingredients
                        .iter()
                        .flatten()
                        .all(|used| standing.get(&used.name).is_some_and(|rate| *rate > 0.))
                })
            }) {
                return Some(supplied.clone());
            }
        }
        candidates.first().cloned()
    }

    pub fn throughput_at(&self, position: &Position) -> FlowRates {
        self.ensure_current();
        if self.node_at(position).is_none() {
            return vec![];
        }
        self.sum_incoming_edge_weights(position)
    }

    pub fn node_at(&self, position: &Position) -> Option<NodeIndex> {
        self.ensure_current();
        let tree = self.flow_tree.read();
        let results: Vec<&NodeIndex> = tree
            .query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            .iter()
            .map(|(node_index, _rect, _item_id)| *node_index)
            .collect();

        if results.is_empty() {
            None
        } else if results.len() == 1 {
            Some(*results[0])
        } else {
            warn!(
                "multiple entity quad tree results for {}: {:?}",
                position,
                tree.query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            );
            Some(*results[0])
        }
    }

    pub fn condense(&self) -> FlowGraphInner {
        let _started = Instant::now();
        self.ensure_current();
        let mut graph = self.inner.read().clone();
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
            if let Some(node_index) = next_node {
                let mut bfs = Bfs::new(&graph, node_index);
                while let Some(node_index) = bfs.next(&graph) {
                    let node = graph.node_weight(node_index).unwrap();

                    let incoming: Vec<FlowNode> = graph
                        .edges_directed(node_index, petgraph::Direction::Incoming)
                        .map(|edge| graph.node_weight(edge.source()).unwrap().clone())
                        .collect();
                    let outgoing: Vec<FlowNode> = graph
                        .edges_directed(node_index, petgraph::Direction::Outgoing)
                        .map(|edge| graph.node_weight(edge.target()).unwrap().clone())
                        .collect();

                    // if we have 1 incoming and 1 outgoing and all three of us have same flow name
                    if incoming.len() == 1
                        && outgoing.len() == 1
                        && node.entity_name == incoming[0].entity_name
                        && incoming[0].entity_name == outgoing[0].entity_name
                    {
                        let incoming: NodeIndex = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.source())
                            .find(|_| true)
                            .unwrap();
                        let outgoing: NodeIndex = graph
                            .edges_directed(node_index, petgraph::Direction::Outgoing)
                            .map(|edge| edge.target())
                            .find(|_| true)
                            .unwrap();
                        let weight = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.weight().clone())
                            .find(|_| true)
                            .unwrap();
                        if let Some(edge) = graph.find_edge(incoming, node_index) {
                            graph.remove_edge(edge);
                        }
                        if let Some(edge) = graph.find_edge(node_index, outgoing) {
                            graph.remove_edge(edge);
                        }
                        graph.add_edge(incoming, outgoing, weight);
                        graph.remove_node(node_index);
                    }
                }
            } else {
                break;
            }
        }
        // info!(
        //     "condensing flow graph from {} to {} entities took {:?}",
        //     starting_nodes,
        //     graph.node_indices().count(),
        //     started.elapsed()
        // );
        graph
    }

    fn sum_incoming_edge_weights_by_side(
        &self,
        entity_edge_count: usize,
        position: &Position,
    ) -> (FlowRates, FlowRates) {
        let flow_node_index = self.node_at(position).unwrap();
        let mut left: FlowRates = vec![];
        let mut right: FlowRates = vec![];

        let graph = self.inner.read();
        let flow_node = graph.node_weight(flow_node_index).unwrap();

        for edge in graph.edges_directed(flow_node_index, petgraph::Direction::Incoming) {
            let weight = edge.weight();
            let prev_node = graph.node_weight(edge.source()).unwrap();

            // info!(
            //     "sum by -> flow {} @ {} {:?} next {} @ {} {:?} -> edges {}",
            //     flow_node.entity.name,
            //     flow_node.entity.position,
            //     flow_node.direction,
            //     prev_node.entity.name,
            //     prev_node.entity.position,
            //     prev_node.direction,
            //     entity_edge_count
            // );
            // let intersection_left = flow_node.direction.clockwise() == prev_node.direction && entity_node_at(entity_graph, Position::)

            if flow_node.direction == prev_node.direction || entity_edge_count == 1 {
                match weight {
                    FlowEdge::Single(vec) => {
                        for (name, production_rate) in vec {
                            self.add_production_rate(
                                &mut left,
                                (name.clone(), production_rate / 2.),
                            );
                            self.add_production_rate(
                                &mut right,
                                (name.clone(), production_rate / 2.),
                            );
                        }
                    }
                    FlowEdge::Double(l, r) => {
                        for e in l {
                            self.add_production_rate(&mut left, e.clone());
                        }
                        for e in r {
                            self.add_production_rate(&mut right, e.clone());
                        }
                    }
                }
            } else if flow_node.direction.clockwise().opposite() == prev_node.direction {
                match weight {
                    FlowEdge::Single(vec) => {
                        for (name, production_rate) in vec {
                            self.add_production_rate(&mut right, (name.clone(), *production_rate));
                        }
                    }
                    FlowEdge::Double(l, r) => {
                        for e in l {
                            self.add_production_rate(&mut right, e.clone());
                        }
                        for e in r {
                            self.add_production_rate(&mut right, e.clone());
                        }
                    }
                }
            } else if flow_node.direction.clockwise() == prev_node.direction {
                match weight {
                    FlowEdge::Single(vec) => {
                        for (name, production_rate) in vec {
                            self.add_production_rate(&mut left, (name.clone(), *production_rate));
                        }
                    }
                    FlowEdge::Double(l, r) => {
                        for e in l {
                            self.add_production_rate(&mut left, e.clone());
                        }
                        for e in r {
                            self.add_production_rate(&mut left, e.clone());
                        }
                    }
                }
            }
        }

        (left, right)
    }

    #[allow(clippy::ptr_arg)]
    fn divide_flowrate(&self, incoming: &FlowRates, divisor: usize) -> FlowEdge {
        let mut left: FlowRates = vec![];
        let mut right: FlowRates = vec![];
        for (name, rate) in incoming {
            self.add_production_rate(&mut left, (name.clone(), rate / (2 * divisor) as f64));
            self.add_production_rate(&mut right, (name.clone(), rate / (2 * divisor) as f64));
        }
        FlowEdge::Double(left, right)
    }

    #[allow(clippy::ptr_arg)]
    fn add_production_rate(&self, vec: &mut FlowRates, entry: (String, f64)) {
        match vec.iter_mut().find(|e| e.0 == entry.0) {
            Some(e) => e.1 += entry.1,
            None => vec.push(entry),
        }
    }

    fn sum_production_rates(&self, input: Vec<FlowRates>) -> FlowRates {
        let mut map: HashMap<String, f64> = HashMap::new();
        for vec in input {
            for (name, production_rate) in vec {
                if let Some(v) = map.get(&name) {
                    let v = *v;
                    map.insert(name, v + production_rate);
                } else {
                    map.insert(name, production_rate);
                }
            }
        }
        map.into_iter().collect()
    }

    fn sum_incoming_edge_weights(&self, position: &Position) -> FlowRates {
        let flow_node_index = self.node_at(position).unwrap();
        let graph = self.inner.read();
        let incoming: Vec<FlowEdge> = graph
            .edges_directed(flow_node_index, petgraph::Direction::Incoming)
            .map(|i| graph.edge_weight(i.id()).unwrap().clone())
            .collect();
        let mut rates: Vec<FlowRates> = vec![];
        for edge in incoming {
            match edge {
                FlowEdge::Single(vec) => {
                    rates.push(vec);
                }
                FlowEdge::Double(left, right) => {
                    rates.push(left);
                    rates.push(right);
                }
            }
        }
        self.sum_production_rates(rates)
    }
    /// How fast a machine named `machine` runs a recipe, from its own
    /// prototype. 1.0 when the game says nothing.
    ///
    /// A `stone-furnace` is 1, a `steel-furnace` and an `electric-furnace` are
    /// both **2**, an `assembling-machine-1` is 0.5. Reading this is the
    /// difference between a rate and a guess: the two-speed furnaces were
    /// reported at 1x for as long as this file hard-coded its smelting times.
    fn crafting_speed(&self, machine: &str) -> f64 {
        self.entity_prototypes
            .get(machine)
            .and_then(|proto| proto.crafting_speed)
            .unwrap_or(1.)
    }

    /// What a furnace named `machine` turns `input` into, and how many per
    /// second, **derived from the game's own recipe table**.
    ///
    /// `None` when no smelting recipe takes `input` as an ingredient -- which
    /// is the honest answer for coal, and is why fuel needs no special case
    /// here.
    ///
    /// # The derivation, and why it is not three constants
    ///
    /// ```text
    /// items per second = product.amount * machine.crafting_speed / recipe.energy
    /// ```
    ///
    /// `FactorioRecipe::energy` is how long one craft takes in seconds, and
    /// `crafting_speed` is on the furnace's own prototype; both are in the live
    /// 2.1.17 dump. This used to be a `match` on [`EntityName`] returning
    /// `1/3.2` for iron, copper and stone and `1/16` for steel, which was wrong
    /// three ways at once and only one of them was the arithmetic:
    ///
    /// * it ignored `crafting_speed`, so a `steel-furnace` and an
    ///   `electric-furnace` (both 2) were reported at 1x;
    /// * it matched on an enum, so a **modded** ore was not smelted slowly, it
    ///   was `warn!("invalid furnace input")` and no edge at all;
    /// * its own test asserted the constants the function read, so under any
    ///   rebalance the code and the check moved together and the suite stayed
    ///   green.
    ///
    /// The rule it broke is the owner's and is about mods before it is about
    /// drift: **rates get derived from the game's own data, because that is
    /// what survives mods.** The mining arm in [`FlowGraph::update`] has always
    /// obeyed it (`mining_speed / mining_time`); this file simply did it right
    /// in one place out of three.
    ///
    /// # Ties are broken by name, deliberately
    ///
    /// `self.recipes` is a `DashMap`, whose iteration order is not stable, so
    /// an input smelted by more than one recipe would otherwise make this
    /// graph non-deterministic run to run. Candidates are sorted and the first
    /// taken, and the ambiguity is warned about rather than hidden -- vanilla
    /// has none, a mod may.
    fn smelting_output(&self, machine: &str, input: &str) -> Option<FlowRate> {
        let recipe_name = self.smelting_recipe_taking(input)?;
        let recipe = self.recipes.get(&recipe_name)?;
        let seconds = recipe.energy.to_f64().unwrap_or_default();
        if seconds <= 0. {
            warn!(
                "recipe {} claims to take no time; no flow edge",
                recipe_name
            );
            return None;
        }
        let product = recipe.products.first()?;
        Some((
            product.name.clone(),
            f64::from(product.amount) * self.crafting_speed(machine) / seconds,
        ))
    }

    /// The name of the smelting recipe that takes `input`, chosen the same way
    /// for every caller so two of them cannot disagree about what a furnace is
    /// doing. Ties broken by name and warned about -- see
    /// [`FlowGraph::smelting_output`].
    fn smelting_recipe_taking(&self, input: &str) -> Option<String> {
        let mut candidates: Vec<String> = self
            .recipes
            .iter()
            .filter(|recipe| recipe.valid && recipe.category == "smelting")
            .filter(|recipe| {
                recipe
                    .ingredients
                    .as_ref()
                    .is_some_and(|list| list.iter().any(|used| used.name == input))
            })
            .map(|recipe| recipe.name.clone())
            .collect();
        candidates.sort();
        let recipe_name = candidates.first()?;
        if candidates.len() > 1 {
            warn!(
                "{} smelting recipes take {}; taking {} by name",
                candidates.len(),
                input,
                recipe_name
            );
        }
        Some(recipe_name.clone())
    }

    pub fn graphviz_dot(&self) -> String {
        self.ensure_current();
        format_dotgraph(
            Dot::with_config(&*self.inner.read(), &[Config::GraphContentOnly]).to_string(),
        )
    }
    pub fn graphviz_dot_condensed(&self) -> String {
        let condensed = self.condense();
        format_dotgraph(Dot::with_config(&condensed, &[Config::GraphContentOnly]).to_string())
    }
}

#[derive(Clone)]
pub struct FlowNode {
    pub position: Position,
    pub direction: Direction,
    pub entity_name: String,
    pub entity_type: EntityType,
    pub entity_id: Option<ItemId>,
    pub miner_ore: Option<String>,
}

impl FlowNode {
    /// `None` when the game reports something this build cannot represent --
    /// see `EntityNode::new` for why that is reported and skipped rather than
    /// unwrapped (`panic = "abort"`) or defaulted.
    pub fn new(
        entity: &FactorioEntity,
        miner_ore: Option<String>,
        entity_id: ItemId,
    ) -> Option<FlowNode> {
        let Some(direction) = Direction::from_u8(entity.direction) else {
            error!(
                "<red>unreadable direction</> <bright-blue>{}</> on <bright-blue>{}</> at <bright-blue>{}</>: defines.direction is 0..=15 -- flow node skipped",
                entity.direction, entity.name, entity.position
            );
            return None;
        };
        let Ok(entity_type) = EntityType::from_str(&entity.entity_type) else {
            error!(
                "<red>unknown entity type</> <bright-blue>{}</> on <bright-blue>{}</> at <bright-blue>{}</> -- flow node skipped",
                entity.entity_type, entity.name, entity.position
            );
            return None;
        };
        Some(FlowNode {
            position: entity.position.clone(),
            entity_id: Some(entity_id),
            entity_name: entity.name.clone(),
            direction,
            miner_ore,
            entity_type,
        })
    }
}

impl std::fmt::Display for FlowNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{} ", miner_ore)
            } else {
                String::new()
            },
            self.entity_name,
            self.position
        ))?;
        Ok(())
    }
}

impl std::fmt::Debug for FlowNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{} ", miner_ore)
            } else {
                String::new()
            },
            self.entity_name,
            self.position
        ))?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum FlowEdge {
    Single(Vec<(String, f64)>),
    Double(Vec<(String, f64)>, Vec<(String, f64)>),
}

impl std::fmt::Display for FlowEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FlowEdge {
    pub fn split(&self) -> FlowEdge {
        match self {
            FlowEdge::Single(vec) => FlowEdge::Single(
                vec.iter()
                    .map(|(name, production_rate)| (name.clone(), production_rate / 2.))
                    .collect(),
            ),
            FlowEdge::Double(left, right) => FlowEdge::Double(
                left.iter()
                    .map(|(name, production_rate)| (name.clone(), production_rate / 2.))
                    .collect(),
                right
                    .iter()
                    .map(|(name, production_rate)| (name.clone(), production_rate / 2.))
                    .collect(),
            ),
        }
    }
}

impl Default for FlowEdge {
    fn default() -> Self {
        FlowEdge::Single(vec![])
    }
}

pub type FlowGraphInner = StableGraph<FlowNode, FlowEdge>;
pub type FlowRate = (String, f64);
pub type FlowRates = Vec<FlowRate>;

pub type FlowQuadTree = QuadTree<NodeIndex, Rect, [(ItemId, QuadTreeRect); 4]>;

/// An empty flow quad tree over the same extent [`FlowGraph::new`] builds.
///
/// One function so a rebuild cannot drift from the constructor: the tree is
/// replaced wholesale on every rebuild, and a second copy of these six numbers
/// is exactly the kind of thing that agrees with its author until it does not.
fn fresh_flow_tree() -> FlowQuadTree {
    FlowQuadTree::new(
        QuadTreeRect::new(Point2D::new(-5120., -5120.), Size2D::new(10240., 10240.)),
        true,
        32,
        128,
        32,
        8,
    )
}

#[cfg(test)]
mod tests {
    use crate::test_utils::entity_graph_from;

    use super::*;

    /// A furnace with a `crafting_speed` of 2 smelts exactly twice as fast.
    ///
    /// Asserted against **game data**, not against a constant this file holds:
    /// `crates/core/tests/entity-prototype-fixtures.json` gives `stone-furnace`
    /// a `crafting_speed` of 1 and `steel-furnace` and `electric-furnace` 2,
    /// and `crates/core/tests/recipes-fixtures.json` gives `iron-plate` an
    /// `energy` of 3.2 for one plate out of one ore. Both files are live
    /// captures off a 2.1 game.
    ///
    /// It replaces a test deleted by this change, `steel_smelts_five_times_slower_than_iron`: doclint-allow
    ///
    /// That one asserted the same four constants the function it tested read,
    /// and so could never
    /// have caught a rebalance, a `crafting_speed` of 2, or a modded ore --
    /// the fourth failure shape in
    /// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`.
    #[test]
    fn a_two_speed_furnace_smelts_at_twice_the_rate() {
        let graph = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(crate::test_utils::fixture_recipes()),
        )));
        assert_eq!(
            graph.smelting_output("stone-furnace", "iron-ore"),
            Some(("iron-plate".to_string(), 1. / 3.2)),
            "one plate per 3.2 s at crafting speed 1"
        );
        assert_eq!(
            graph.smelting_output("steel-furnace", "iron-ore"),
            Some(("iron-plate".to_string(), 2. / 3.2)),
            "a steel furnace is crafting_speed 2 and the old code reported it at 1x"
        );
        assert_eq!(
            graph.smelting_output("electric-furnace", "iron-ore"),
            graph.smelting_output("steel-furnace", "iron-ore"),
            "both two-speed furnaces agree"
        );
    }

    /// Steel takes 16 s, and that number now comes from the recipe rather than
    /// from a literal beside a comment that disagreed with it for years.
    #[test]
    fn steel_takes_the_sixteen_seconds_its_recipe_says() {
        let graph = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(crate::test_utils::fixture_recipes()),
        )));
        assert_eq!(
            graph.smelting_output("stone-furnace", "iron-plate"),
            Some(("steel-plate".to_string(), 1. / 16.)),
            "one steel plate per 16 s"
        );
        assert_eq!(
            graph.smelting_output("stone-furnace", "stone"),
            Some(("stone-brick".to_string(), 1. / 3.2)),
            "two stone in, one brick out, per 3.2 s -- the OUTPUT rate is one brick"
        );
        assert_eq!(
            graph.smelting_output("stone-furnace", "coal"),
            None,
            "coal is an ingredient of no smelting recipe, so it needs no special case"
        );
        assert_eq!(
            graph.smelting_output("stone-furnace", "iron-gear-wheel"),
            None,
            "a gear wheel is crafted, not smelted"
        );
    }

    /// The derivation, not the value: double a recipe's `energy` and the rate
    /// must halve.
    ///
    /// A test that only checks `1/3.2` cannot tell a derived rate from a
    /// hard-coded one -- which is exactly how the old check came to certify the
    /// bug it was written beside. This one fails the moment anybody puts a
    /// literal back.
    ///
    /// Note the recipe table handed in is **modified**, and that is the point:
    /// the graph must read the table it was given rather than a table this file
    /// knows about.
    #[test]
    fn doubling_a_recipes_energy_halves_the_rate_it_is_smelted_at() {
        let recipes = crate::test_utils::fixture_recipes();
        let base = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(recipes.clone()),
        )))
        .smelting_output("stone-furnace", "iron-ore")
        .expect("iron smelts to begin with");

        {
            let mut recipe = recipes.get_mut("iron-plate").unwrap();
            let doubled = recipe.energy.to_f64().unwrap() * 2.;
            *recipe.energy = noisy_float::types::r64(doubled);
        }
        let slower = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(recipes),
        )))
        .smelting_output("stone-furnace", "iron-ore")
        .expect("it still smelts, only slower");

        assert_eq!(slower.0, base.0, "the same product comes out");
        assert_eq!(
            slower.1,
            base.1 / 2.,
            "a recipe that takes twice as long yields half the rate: {} against {}",
            slower.1,
            base.1
        );
    }

    /// A modded ore -- one this build's [`EntityName`] enum has never heard of
    /// -- smelts at the rate its recipe says.
    ///
    /// The old code matched on that enum, so an unknown input was not "smelted
    /// slowly", it was `warn!("invalid furnace input")` and **no flow edge at
    /// all**. This is the mod-compatibility half of the owner's rule, and it is
    /// the reason the rate had to be derived rather than corrected.
    #[test]
    fn an_ore_this_build_has_never_heard_of_smelts_at_its_recipes_rate() {
        assert!(
            EntityName::from_str("unobtainium-ore").is_err(),
            "the fixture only means anything if this name really is unknown"
        );
        let recipes = crate::test_utils::fixture_recipes();
        let iron = recipes.get("iron-plate").unwrap().clone();
        recipes.insert(
            "unobtainium-plate".to_string(),
            FactorioRecipe {
                name: "unobtainium-plate".to_string(),
                ingredients: Some(vec![crate::types::FactorioIngredient {
                    name: "unobtainium-ore".to_string(),
                    ingredient_type: "item".to_string(),
                    amount: 1,
                }]),
                products: vec![crate::types::FactorioProduct {
                    name: "unobtainium-plate".to_string(),
                    product_type: "item".to_string(),
                    amount: 3,
                    probability: Box::new(noisy_float::types::r64(1.)),
                }],
                energy: Box::new(noisy_float::types::r64(6.)),
                ..iron
            },
        );
        let graph = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(recipes),
        )));
        assert_eq!(
            graph.smelting_output("steel-furnace", "unobtainium-ore"),
            Some(("unobtainium-plate".to_string(), 3. * 2. / 6.)),
            "3 products, crafting_speed 2, 6 s: 1 per second"
        );
    }

    /// Same handling as `EntityNode::new`: `FlowNode::new` unwrapped
    /// `Direction::from_u8` and aborted on anything the enum could not read.
    /// This used 12, which is `West` since the 2.x widening; 16 is the first
    /// value still outside `defines.direction`.
    #[test]
    fn a_flow_node_is_not_built_from_a_direction_that_cannot_be_read() {
        let mut belt =
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::North);
        belt.direction = 16;
        assert!(
            FlowNode::new(&belt, None, ItemId::default()).is_none(),
            "a node we cannot orient must not be built"
        );
    }

    /// The world every refresh test below starts from: one electric drill on
    /// iron ore dropping onto two belts.
    ///
    /// Built through `EntityGraph::add` and `connect` rather than typed as a
    /// flow graph, so the fixture is a statement about the entity graph and not
    /// a restatement of what the flow walk is expected to do with it.
    fn drill_and_two_belts() -> Arc<EntityGraph> {
        Arc::new(
            entity_graph_from(vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::IronOre.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South),
            ])
            .unwrap(),
        )
    }

    fn rate_of(rates: &FlowRates, item: &str) -> Option<f64> {
        rates.iter().find(|(name, _)| name == item).map(|(_, r)| *r)
    }

    /// The first question anything outside this file has ever asked this graph.
    ///
    /// The number is asserted absolutely, not as a relation: an electric mining
    /// drill's `mining_speed` is 0.5 and iron ore's `mining_time` is 1.0 in
    /// `crates/core/tests/entity-prototype-fixtures.json`, both captured off a
    /// live 2.1 game, so `mining_speed / mining_time` = **0.5 ore/s** is what
    /// the game's own data says. A relation between two wrong numbers is not
    /// evidence.
    ///
    /// Note which prototype each field comes from: the `mining_time` is the
    /// **ore's**, not the drill's -- the drill prototype carries a
    /// `mining_time` of its own (0.3, how long it takes to mine the drill up)
    /// and reading that one instead gives 1.67 and looks entirely plausible.
    #[test]
    fn throughput_at_reports_what_the_drill_puts_on_the_belt() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph);
        let on_the_belt = flow_graph.throughput_at(&Position::new(0.5, 0.5));
        let rate = rate_of(&on_the_belt, "iron-ore").expect("the belt carries the drill's ore");
        assert_eq!(
            rate, 0.5,
            "an electric drill (mining_speed 0.5) on iron ore (mining_time 1.0) delivers \
             0.5 ore/s"
        );
        assert!(
            flow_graph
                .throughput_at(&Position::new(40.5, 40.5))
                .is_empty(),
            "nothing stands there, so nothing arrives there"
        );
    }

    /// Nobody calls `update()`. The reader does.
    ///
    /// This is the whole point of the generation counter: before it existed the
    /// only two callers of `update()` were one-shot at world initialisation, so
    /// a machine built by a run was invisible here and the answer was the world
    /// as at tick 0 -- with no error and no warning.
    #[test]
    fn a_belt_built_after_the_first_read_is_seen_without_anyone_calling_update() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph.clone());
        assert!(
            flow_graph
                .throughput_at(&Position::new(0.5, 2.5))
                .is_empty(),
            "no belt stands there yet"
        );
        entity_graph
            .add(
                vec![FactorioEntity::new_transport_belt(
                    &Position::new(0.5, 2.5),
                    Direction::South,
                )],
                None,
            )
            .unwrap();
        entity_graph.connect().unwrap();
        let rate = rate_of(
            &flow_graph.throughput_at(&Position::new(0.5, 2.5)),
            "iron-ore",
        )
        .expect("the third belt is fed by the two above it");
        assert_eq!(
            rate, 0.5,
            "the whole drill's 0.5 ore/s reaches the belt built after the first read"
        );
    }

    /// And a removal is seen too, which `update()` alone never could: it wrote
    /// into `self.inner` and never cleared, so a removed entity's node and
    /// edges stayed forever.
    #[test]
    fn a_removed_drill_stops_feeding_the_belt() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph.clone());
        assert!(
            rate_of(
                &flow_graph.throughput_at(&Position::new(0.5, 0.5)),
                "iron-ore"
            )
            .is_some(),
            "the drill feeds the belt to begin with"
        );
        entity_graph
            .remove(&FactorioEntity::new_electric_mining_drill(
                &Position::new(0.5, -1.5),
                Direction::South,
            ))
            .unwrap();
        assert!(
            flow_graph
                .throughput_at(&Position::new(0.5, 0.5))
                .is_empty(),
            "with the drill gone nothing arrives on the belt"
        );
    }

    /// The case that rules out the cheap fix, and the reason this graph is
    /// rebuilt rather than patched.
    ///
    /// [`FlowGraph::node_at`] matches on **position alone** and returns before
    /// the prototype is consulted, so an entity built where another stood
    /// inherits the old node -- its type, its name and its contents. A refresh
    /// that visited only what changed would fix the stale *nodes* and keep this
    /// silently, which is worse than a frozen graph because it would look
    /// maintained.
    ///
    /// The fixture is hostile on purpose: the two entities are both 1x1 and sit
    /// at exactly the same position, which is the only arrangement in which the
    /// bug can express itself. A test that swapped in an entity of a different
    /// size would pass for the wrong reason.
    #[test]
    fn an_entity_built_where_another_stood_does_not_inherit_its_flow_node() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph.clone());
        let reused = Position::new(0.5, 1.5);
        let name_at = |flow_graph: &FlowGraph, at: &Position| {
            flow_graph
                .node_at(at)
                .map(|index| flow_graph.inner_graph().node_weight(index).unwrap().clone())
                .map(|node| node.entity_name)
        };
        assert_eq!(
            name_at(&flow_graph, &reused).as_deref(),
            Some("transport-belt"),
            "the second belt is a flow node before anything is swapped"
        );

        entity_graph
            .remove(&FactorioEntity::new_transport_belt(
                &reused,
                Direction::South,
            ))
            .unwrap();
        entity_graph
            .add(
                vec![FactorioEntity::new_inserter(&reused, Direction::North)],
                None,
            )
            .unwrap();
        entity_graph.connect().unwrap();

        assert_eq!(
            name_at(&flow_graph, &reused).as_deref(),
            Some("inserter"),
            "an inserter built on the belt's tile is an inserter, not the belt's old node"
        );
    }

    /// The counter the refresh hangs off, asserted on its own so a failure says
    /// which half broke.
    #[test]
    fn every_mutation_of_the_entity_graph_bumps_its_generation() {
        let entity_graph = drill_and_two_belts();
        let after_setup = entity_graph.generation();
        assert!(
            after_setup > 0,
            "`entity_graph_from` adds and connects, so the setup itself is mutations"
        );

        entity_graph
            .add(
                vec![FactorioEntity::new_transport_belt(
                    &Position::new(0.5, 2.5),
                    Direction::South,
                )],
                None,
            )
            .unwrap();
        let after_add = entity_graph.generation();
        assert_eq!(after_add, after_setup + 1, "`add` bumps exactly once");

        entity_graph.connect().unwrap();
        assert_eq!(
            entity_graph.generation(),
            after_add + 1,
            "`connect` bumps exactly once"
        );

        entity_graph
            .remove(&FactorioEntity::new_transport_belt(
                &Position::new(0.5, 2.5),
                Direction::South,
            ))
            .unwrap();
        assert_eq!(
            entity_graph.generation(),
            after_add + 2,
            "`remove` bumps exactly once"
        );
    }

    #[test]
    fn test_splitters() {
        let entity_graph = entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 0.5), Direction::South),
            FactorioEntity::new_splitter(&Position::new(1., 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 2.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            entity_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore: mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "transport-belt at [1.5, 0.5]" ]
    3 [ label = "splitter at [1, 1.5]" ]
    4 [ label = "transport-belt at [0.5, 2.5]" ]
    5 [ label = "transport-belt at [1.5, 2.5]" ]
    0 -> 1 [ label = "1" ]
    1 -> 3 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    3 -> 4 [ label = "1" ]
    3 -> 5 [ label = "1" ]
}
"#,
        );
        let flow_graph = FlowGraph::new(Arc::new(entity_graph));
        flow_graph.update().unwrap();
        assert_eq!(
            flow_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore electric-mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "splitter at [1, 1.5]" ]
    3 [ label = "transport-belt at [0.5, 2.5]" ]
    4 [ label = "transport-belt at [1.5, 2.5]" ]
    0 -> 1 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    1 -> 2 [ label = "Double([(\"iron-ore\", 0.25)], [(\"iron-ore\", 0.25)])" ]
    2 -> 3 [ label = "Double([(\"iron-ore\", 0.125)], [(\"iron-ore\", 0.125)])" ]
    2 -> 4 [ label = "Double([(\"iron-ore\", 0.125)], [(\"iron-ore\", 0.125)])" ]
}
"#,
        );
    }
    /// A layout where a drill is *not* the root of the walk that reaches it:
    /// an iron drill feeds a belt, an inserter takes coal fuel off that belt
    /// into a burner drill, and the burner drill outputs onto a second belt.
    ///
    /// The second drill's output edge is computed from `entity_root` -- the
    /// iron drill the walk started at -- rather than from the drill actually
    /// producing on that edge, so it is labelled with the wrong ore at the
    /// wrong rate.
    #[test]
    fn a_drill_downstream_of_another_drill_reports_its_own_ore() {
        let entity_graph = entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_resource(
                &Position::new(0.5, 2.5),
                Direction::South,
                &EntityName::Coal.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
            FactorioEntity::new_burner_mining_drill(&Position::new(1., 3.), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 4.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            entity_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore: mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "inserter at [0.5, 1.5]" ]
    3 [ label = "coal: mining-drill at [1, 3]" ]
    4 [ label = "transport-belt at [1.5, 4.5]" ]
    0 -> 1 [ label = "1" ]
    1 -> 2 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    3 -> 4 [ label = "1" ]
}
"#
        );
        let flow_graph = FlowGraph::new(Arc::new(entity_graph));
        flow_graph.update().unwrap();
        assert_eq!(
            flow_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore electric-mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "inserter at [0.5, 1.5]" ]
    3 [ label = "coal burner-mining-drill at [1, 3]" ]
    4 [ label = "transport-belt at [1.5, 4.5]" ]
    0 -> 1 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    1 -> 2 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    2 -> 3 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    3 -> 4 [ label = "Single([(\"coal\", 0.25)])" ]
}
"#,
            "the burner drill mines the coal underneath it at its own speed, \
             not the iron the walk started from at the electric drill's speed"
        );
    }

    /// The same shape with nothing under the second drill.
    ///
    /// `EntityGraph::add` stores `None` for a drill it finds no resource
    /// beneath and warns as it does so, so this is an expected world, not a
    /// corrupt one. Reading the root's ore reported the drill as producing
    /// iron; reading the drill's own ore and unwrapping it would abort the
    /// process, which under `panic = "abort"` is not recoverable.
    #[test]
    fn a_drill_with_no_ore_under_it_produces_nothing() {
        let entity_graph = entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
            FactorioEntity::new_burner_mining_drill(&Position::new(1., 3.), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 4.5), Direction::South),
        ])
        .unwrap();
        let flow_graph = FlowGraph::new(Arc::new(entity_graph));
        flow_graph.update().unwrap();
        assert_eq!(
            flow_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore electric-mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "inserter at [0.5, 1.5]" ]
    3 [ label = "burner-mining-drill at [1, 3]" ]
    0 -> 1 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    1 -> 2 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    2 -> 3 [ label = "Single([(\"iron-ore\", 0.5)])" ]
}
"#,
            "the drill mines nothing, so the belt it drops onto gets no flow \
             edge -- and the walk neither aborts nor invents one"
        );
    }

    /// A furnace fed nothing but fuel produces nothing, and the arm on its
    /// output says so -- **a modelled node with an empty rate**.
    ///
    /// This is the exact condition `method::sustain`'s `flow_reaches` vetoes
    /// on, and it is why that veto is not vacuous: the graph can hold a node it
    /// positively knows nothing arrives at. It is also the honest reading of
    /// the world -- a stone furnace with coal in its fuel slot and no ore in
    /// its input smelts nothing at all.
    #[test]
    fn an_arm_on_a_furnace_that_is_only_fuelled_is_modelled_and_gets_nothing() {
        let entity_graph = Arc::new(
            entity_graph_from(vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::Coal.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
                FactorioEntity::new_stone_furnace(&Position::new(1., 3.), Direction::South),
                FactorioEntity::new_inserter(&Position::new(0.5, 4.5), Direction::North),
            ])
            .unwrap(),
        );
        let flow_graph = FlowGraph::new(entity_graph);
        let arm = Position::new(0.5, 4.5);
        assert!(
            flow_graph.node_at(&arm).is_some(),
            "the graph does model the output arm -- the veto turns on this being true"
        );
        assert!(
            flow_graph.throughput_at(&arm).is_empty(),
            "and nothing arrives at it, because coal is not something a furnace smelts: {:?}",
            flow_graph.throughput_at(&arm)
        );
        assert_eq!(
            rate_of(
                &flow_graph.throughput_at(&Position::new(1., 3.)),
                &EntityName::Coal.to_string()
            ),
            Some(0.5),
            "while the furnace itself is receiving the drill's coal at 0.5/s"
        );
    }

    #[test]
    fn test_furnace() {
        let entity_graph = entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
            FactorioEntity::new_stone_furnace(&Position::new(1., 3.), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 4.5), Direction::North),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 5.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 6.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 7.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            entity_graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "iron-ore: mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "inserter at [0.5, 1.5]" ]
    3 [ label = "furnace at [1, 3]" ]
    4 [ label = "inserter at [0.5, 4.5]" ]
    5 [ label = "transport-belt at [0.5, 5.5]" ]
    6 [ label = "transport-belt at [0.5, 6.5]" ]
    7 [ label = "transport-belt at [0.5, 7.5]" ]
    0 -> 1 [ label = "1" ]
    1 -> 2 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    3 -> 4 [ label = "1" ]
    4 -> 5 [ label = "1" ]
    5 -> 6 [ label = "1" ]
    6 -> 7 [ label = "1" ]
}
"#,
        );
        let flow_graph = FlowGraph::new(Arc::new(entity_graph));
        flow_graph.update().unwrap();
        assert_eq!(
            flow_graph.graphviz_dot_condensed(),
            r#"digraph {
    0 [ label = "iron-ore electric-mining-drill at [0.5, -1.5]" ]
    1 [ label = "transport-belt at [0.5, 0.5]" ]
    2 [ label = "inserter at [0.5, 1.5]" ]
    3 [ label = "stone-furnace at [1, 3]" ]
    4 [ label = "inserter at [0.5, 4.5]" ]
    5 [ label = "transport-belt at [0.5, 5.5]" ]
    7 [ label = "transport-belt at [0.5, 7.5]" ]
    0 -> 1 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    1 -> 2 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    2 -> 3 [ label = "Single([(\"iron-ore\", 0.5)])" ]
    3 -> 4 [ label = "Single([(\"iron-plate\", 0.3125)])" ]
    4 -> 5 [ label = "Single([(\"iron-plate\", 0.3125)])" ]
    5 -> 7 [ label = "Double([(\"iron-plate\", 0.15625)], [(\"iron-plate\", 0.15625)])" ]
}
"#,
        );
    }

    /// The whole-graph aggregate over the chain `test_furnace` builds: one
    /// drill and one furnace, five belts and two inserters between them.
    ///
    /// The two numbers are the two the chain's own edge labels carry -- 0.5
    /// ore/s off the drill, 0.3125 plate/s out of a `crafting_speed` 1 furnace
    /// on a 3.2 s recipe -- so this asserts the aggregate and not a second
    /// arithmetic.
    ///
    /// **The carriers are what this is really about.** Five belts and two
    /// inserters carry that same 0.5 ore/s and 0.3125 plate/s along the chain.
    /// Counting a tile because a rate is written on it would report 3 ore/s and
    /// 0.9375 plate/s out of a base with one drill in it.
    #[test]
    fn belts_and_inserters_carry_a_rate_and_do_not_add_to_it() {
        let flow_graph = FlowGraph::new(Arc::new(smelting_chain()));
        let rates = flow_graph.production_rates();
        assert_eq!(
            rates,
            BTreeMap::from([
                ("iron-ore".to_string(), 0.5),
                ("iron-plate".to_string(), 0.3125),
            ]),
            "one drill and one furnace, whatever stands between them"
        );
    }

    /// A drill that feeds two things mines once.
    ///
    /// [`FlowGraph::update_flow_edge`] writes the producer's whole output on
    /// **every** outgoing edge, which is what makes `throughput_at` right for
    /// each consumer and makes a naive sum wrong for the producer. This drill
    /// drops onto a belt *and* has an inserter reaching into it -- two edges of
    /// 0.5 ore/s off one drill that mines 0.5.
    ///
    /// Asserted through `throughput_at` as well, so a failure says which half
    /// broke: if the two edges were not both 0.5 the fixture would not be
    /// exercising the case at all, and the aggregate would pass for the wrong
    /// reason.
    #[test]
    fn a_drill_feeding_two_things_is_counted_once() {
        let entity_graph = Arc::new(
            entity_graph_from(vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::IronOre.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                // Reaches into the drill from the north and drops onto a belt
                // of its own: the second consumer.
                FactorioEntity::new_inserter(&Position::new(0.5, -3.5), Direction::South),
                FactorioEntity::new_transport_belt(&Position::new(0.5, -4.5), Direction::North),
            ])
            .unwrap(),
        );
        let flow_graph = FlowGraph::new(entity_graph);
        assert_eq!(
            rate_of(
                &flow_graph.throughput_at(&Position::new(0.5, 0.5)),
                "iron-ore"
            ),
            Some(0.5),
            "the belt below the drill is offered the drill's whole output"
        );
        assert_eq!(
            rate_of(
                &flow_graph.throughput_at(&Position::new(0.5, -3.5)),
                "iron-ore"
            ),
            Some(0.5),
            "and so is the inserter above it -- two edges of 0.5 is the case under test"
        );
        assert_eq!(
            flow_graph.production_rates(),
            BTreeMap::from([("iron-ore".to_string(), 0.5)]),
            "one drill mines 0.5 ore/s however many things it feeds"
        );
    }

    /// A furnace fed two ores smelts one furnace's worth between them, not one
    /// furnace's worth of each.
    ///
    /// Found by running this graph over the 6:39:53 world-record save rather
    /// than by reading the code: `stone-brick` came out at **7,125/min against
    /// the game's 450**, because the model saw stone reaching furnaces the game
    /// had set to iron and gave each of them a full brick edge on top of its
    /// full plate edge.
    ///
    /// The sum is what is asserted, because it is the physical claim -- a stone
    /// furnace is one machine and 0.3125 items/s is all of it. How the share
    /// splits between two ores arriving down one belt is a modelling choice
    /// (this graph splits by arrival rate); how much comes out in total is not.
    #[test]
    fn a_furnace_fed_two_ores_still_only_runs_one_at_a_time() {
        let entity_graph = Arc::new(
            entity_graph_from(vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::IronOre.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_resource(
                    &Position::new(0.5, 2.5),
                    Direction::North,
                    &EntityName::Stone.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, 2.5),
                    Direction::North,
                ),
                // Both drills drop onto the same tile: one mixed belt.
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::East),
                FactorioEntity::new_transport_belt(&Position::new(1.5, 0.5), Direction::East),
                FactorioEntity::new_inserter(&Position::new(2.5, 0.5), Direction::West),
                FactorioEntity::new_stone_furnace(&Position::new(4., 0.5), Direction::North),
                // The furnace needs a consumer: a rate is written on an
                // outgoing edge, and a machine nothing takes from has none.
                FactorioEntity::new_inserter(&Position::new(5.5, 0.5), Direction::West),
                FactorioEntity::new_transport_belt(&Position::new(6.5, 0.5), Direction::East),
            ])
            .unwrap(),
        );
        let flow_graph = FlowGraph::new(entity_graph);
        let arriving = flow_graph.throughput_at(&Position::new(4., 0.5));
        assert!(
            rate_of(&arriving, "iron-ore").is_some() && rate_of(&arriving, "stone").is_some(),
            "both ores must actually reach the furnace or this asserts nothing: {arriving:?}"
        );
        let rates = flow_graph.production_rates();
        let plate = rates.get("iron-plate").copied().unwrap_or_default();
        let brick = rates.get("stone-brick").copied().unwrap_or_default();
        assert!(
            plate > 0. && brick > 0.,
            "the furnace is fed both, so it makes some of both: {rates:?}"
        );
        assert!(
            (plate + brick - 0.3125).abs() < 1e-9,
            "one stone furnace on 3.2 s recipes makes 0.3125 items/s in total, \
             not 0.3125 of each: iron-plate {plate}, stone-brick {brick}"
        );
    }

    /// A **steel** furnace on a chain one electric drill feeds: it can smelt
    /// 0.625 ore/s and the drill mines 0.5, so a fifth of its time it has
    /// nothing to smelt.
    fn steel_furnace_fed_by_one_drill() -> Arc<EntityGraph> {
        let mut steel = FactorioEntity::new_stone_furnace(&Position::new(1., 3.), Direction::South);
        steel.name = "steel-furnace".to_string();
        Arc::new(
            entity_graph_from(vec![
                FactorioEntity::new_resource(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                    &EntityName::IronOre.to_string(),
                ),
                FactorioEntity::new_electric_mining_drill(
                    &Position::new(0.5, -1.5),
                    Direction::South,
                ),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
                steel,
                FactorioEntity::new_inserter(&Position::new(0.5, 4.5), Direction::North),
                FactorioEntity::new_transport_belt(&Position::new(0.5, 5.5), Direction::South),
            ])
            .unwrap(),
        )
    }

    /// The whole point of `sustained_production_rates`: a machine cannot make
    /// what its ingredients do not support, however fast its prototype says it
    /// runs.
    #[test]
    fn a_base_makes_only_what_its_ore_supply_supports() {
        let flow_graph = FlowGraph::new(steel_furnace_fed_by_one_drill());
        let nameplate = flow_graph.production_rates();
        assert_eq!(
            nameplate.get("iron-plate").copied(),
            Some(2. / 3.2),
            "a steel furnace is crafting_speed 2 on a 3.2 s recipe: {nameplate:?}"
        );
        let sustained = flow_graph.sustained_production_rates();
        let plate = sustained.get("iron-plate").copied().unwrap_or_default();
        assert!(
            (plate - 0.5).abs() < 1e-9,
            "one drill mines 0.5 ore/s and one ore makes one plate, so 0.5 plate/s \
             is the ceiling however fast the furnace is: got {plate}"
        );
    }

    /// The ore itself is **not** throttled by its own consumers: a drill takes
    /// its output from the ground, not from a recipe.
    ///
    /// The fixture gives `iron-ore` a synthesis recipe out of an ingredient
    /// nothing makes, which is exactly the shape `coal` has in Space Age --
    /// charging drills through "the recipe that makes what they make" reported
    /// the world-record base at 46 coal/min against a real 4,096.
    #[test]
    fn the_ground_is_not_a_recipe() {
        let recipes = crate::test_utils::fixture_recipes();
        recipes.insert(
            "iron-ore-synthesis".to_string(),
            FactorioRecipe {
                name: "iron-ore-synthesis".to_string(),
                valid: true,
                enabled: true,
                category: "chemistry".to_string(),
                // Iron PLATE, not an invented item, and three of them: the
                // charge has to be for something the base really makes and
                // really is short of, or the drill escapes the bill for the
                // unrelated reason that an unknown ingredient never
                // constrains -- and the test passes while testing nothing.
                // It did exactly that until the substitution was run.
                ingredients: Some(vec![crate::types::FactorioIngredient {
                    name: "iron-plate".to_string(),
                    ingredient_type: "item".to_string(),
                    amount: 3,
                }]),
                products: vec![crate::types::FactorioProduct {
                    name: "iron-ore".to_string(),
                    product_type: "item".to_string(),
                    amount: 1,
                    probability: Box::new(noisy_float::types::r64(1.)),
                }],
                hidden: false,
                energy: Box::new(noisy_float::types::r64(1.)),
                order: String::new(),
                group: String::new(),
                subgroup: String::new(),
            },
        );
        let entity_graph = EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(recipes),
        );
        let mut steel = FactorioEntity::new_stone_furnace(&Position::new(1., 3.), Direction::South);
        steel.name = "steel-furnace".to_string();
        entity_graph
            .add(
                vec![
                    FactorioEntity::new_resource(
                        &Position::new(0.5, -1.5),
                        Direction::South,
                        &EntityName::IronOre.to_string(),
                    ),
                    FactorioEntity::new_electric_mining_drill(
                        &Position::new(0.5, -1.5),
                        Direction::South,
                    ),
                    FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
                    FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
                    steel,
                    FactorioEntity::new_inserter(&Position::new(0.5, 4.5), Direction::North),
                    FactorioEntity::new_transport_belt(&Position::new(0.5, 5.5), Direction::South),
                ],
                None,
            )
            .unwrap();
        entity_graph.connect().unwrap();
        let sustained = FlowGraph::new(Arc::new(entity_graph)).sustained_production_rates();
        let ore = sustained.get("iron-ore").copied().unwrap_or_default();
        assert!(
            (ore - 0.5).abs() < 1e-9,
            "the drill mines 0.5 ore/s out of the ground and owes nobody an \
             ingredient for it: got {ore}"
        );
    }

    /// One machine making one item out of `needs`, for the pure tests of
    /// [`FlowGraph::balance`] below.
    fn line(item: &str, rate: f64, needs: &[(&str, f64)]) -> ProductionLine {
        ProductionLine {
            item: item.to_string(),
            rate,
            needs: needs
                .iter()
                .map(|(name, rate)| ((*name).to_string(), *rate))
                .collect(),
        }
    }

    /// One balanced rate, against what it should be.
    ///
    /// A tolerance rather than `assert_eq!` because [`FlowGraph::ration`] is
    /// an **iteration to a fixed point**, not a closed form: it stops when
    /// nothing moves by more than [`RATION_TOLERANCE`], so an exact 2.0 lands
    /// as 2.000000000006. 1e-9 is a thousand times that residual and a
    /// billionth of any rate these tests assert, so it distinguishes every
    /// answer they are trying to tell apart.
    fn assert_close(out: &BTreeMap<String, f64>, item: &str, want: f64, why: &str) {
        let got = out.get(item).copied().unwrap_or(f64::NAN);
        assert!(
            (got - want).abs() < 1e-9,
            "{item} should balance to {want}, got {got}: {why}: {out:?}"
        );
    }

    /// What each line's item comes to, once `balance` has decided its scale.
    fn balanced(lines: &[ProductionLine]) -> BTreeMap<String, f64> {
        let scale = FlowGraph::balance(lines);
        let mut total: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            *total.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
        }
        total
    }

    /// The demand side: **a machine whose output has nowhere to go stops
    /// pulling.**
    ///
    /// `cog` has capacity for 10/s and one consumer that can take 2/s. The
    /// old model charged it 10 plates a second regardless, which is the
    /// fictional demand that dragged `processing-unit` to 0.54x the game's own
    /// number.
    ///
    /// The chain is three deep on purpose: `widget` has to be drained by
    /// *something* or its own line would be exempt for the different reason
    /// tested by `an_item_nothing_here_eats_has_an_unknown_outlet`, and the
    /// test would pass without exercising the cap at all.
    #[test]
    fn a_producer_is_held_to_what_its_consumers_can_take() {
        let lines = vec![
            line("plate", 100., &[]),
            line("cog", 10., &[("plate", 10.)]),
            line("widget", 2., &[("cog", 2.)]),
            line("gizmo", 1., &[("widget", 1.)]),
        ];
        let out = balanced(&lines);
        assert_close(
            &out,
            "cog",
            2.,
            "one consumer takes 2 cogs a second, so ten a second is not \
             sustained however many machines stand there",
        );
        assert_close(
            &out,
            "plate",
            100.,
            "and the cog line stops pulling the other 8 plates",
        );
    }

    /// **A line that stops pulling releases its share, and a line still
    /// pulling takes it up.**
    ///
    /// The sentence [`FlowGraph::ration`] has claimed since the outlet cap
    /// landed, and could not do while the scale was a monotone descent.
    ///
    /// `cog` and `bolt` are two consumers of one 10/s plate supply, each able
    /// to eat all of it. `cog` has an outlet that takes 1/s; `bolt` has one
    /// that takes 10/s. So `cog` is output-blocked at a tenth of its
    /// nameplate and the nine plates a second it stops pulling belong to
    /// `bolt`.
    ///
    /// Under the descent both were first rationed to half the plate -- 5 and
    /// 5 -- and when `cog` then fell to 1 on its outlet, **`bolt` was frozen
    /// at 5**: it had already been written down and nothing could write it
    /// back up. The nine plates were released and nobody could take them.
    #[test]
    fn a_line_that_stops_pulling_releases_its_share_to_one_that_has_not() {
        let lines = vec![
            line("plate", 10., &[]),
            line("cog", 10., &[("plate", 10.)]),
            line("bolt", 10., &[("plate", 10.)]),
            // The two outlets, and the two phase-two lines that make them
            // drained rather than exempt.
            line("widget", 1., &[("cog", 1.)]),
            line("nut", 10., &[("bolt", 10.)]),
            line("gizmo", 1., &[("widget", 1.)]),
            line("doodad", 1., &[("nut", 1.)]),
        ];
        let out = balanced(&lines);
        assert_close(
            &out,
            "cog",
            1.,
            "its outlet takes one a second, so it pulls one plate a second",
        );
        assert_close(
            &out,
            "bolt",
            9.,
            "and the nine plates the cog line stopped pulling are the bolt \
             line's -- under the descent it was frozen at the 5 it had \
             already been rationed to",
        );
    }

    /// An **unknown** drain does not displace a **known** one.
    ///
    /// Both lines want 8 plate/s and only 10 exist. `cog` is eaten by
    /// something in the model; nothing at all eats `trinket`, so its real
    /// drain is a bot request, a lab or a player -- none of which this graph
    /// holds. Sharing the shortage proportionally, which is what one
    /// undifferentiated ration does, charges the unknown drain at nameplate
    /// and starves the known one.
    #[test]
    fn an_unknown_drain_does_not_displace_a_known_one() {
        let lines = vec![
            line("plate", 10., &[]),
            line("cog", 8., &[("plate", 8.)]),
            line("widget", 8., &[("cog", 8.)]),
            line("gizmo", 1., &[("widget", 1.)]),
            line("trinket", 8., &[("plate", 8.)]),
        ];
        let out = balanced(&lines);
        assert_close(
            &out,
            "cog",
            8.,
            "the line something is pulling from keeps its plates",
        );
        assert_close(
            &out,
            "trinket",
            2.,
            "and the line nothing is pulling from gets the two left over, \
             not four and a half",
        );
    }

    /// An item **nothing here eats** has an unknown outlet, not a closed one.
    ///
    /// Without this the deepest item in every chain -- the one whose only
    /// consumers are the mall the first phase excludes -- would be capped at
    /// zero. On the world-record base that is `processing-unit` exactly: its
    /// modelled consumption comes entirely from lines nothing drains.
    #[test]
    fn an_item_nothing_here_eats_has_an_unknown_outlet() {
        let lines = vec![
            line("plate", 100., &[]),
            line("core", 5., &[("plate", 5.)]),
            line("trinket", 1., &[("core", 1.)]),
        ];
        let out = balanced(&lines);
        assert_close(
            &out,
            "core",
            5.,
            "nothing in the first phase eats a core, which is not the same \
             claim as nothing wanting one",
        );
    }

    /// A machine that takes its output from the **ground** is capped by
    /// nothing, the same exemption `the_ground_is_not_a_recipe` gives it on
    /// the input side.
    ///
    /// This is the `coal` shape, and it is why the exemption is not
    /// cosmetic: a burner's fuel is not a recipe ingredient, so on the
    /// world-record base the model sees 844 coal/min eaten against a real
    /// 4,096. Capping the drills at what the model can see them feed would
    /// be a factor of five wrong.
    #[test]
    fn the_ground_is_not_capped_by_the_customers_the_model_can_see() {
        let lines = vec![
            line("coal", 100., &[]),
            line("brick", 1., &[("coal", 1.)]),
            line("trinket", 1., &[("brick", 1.)]),
        ];
        let out = balanced(&lines);
        assert_close(
            &out,
            "coal",
            100.,
            "the model sees one consumer of coal and knows nothing of the \
             boilers burning the rest",
        );
    }

    /// Which recipe an item is charged to is decided by **what the base can
    /// supply**, not by sorting.
    ///
    /// Space Age gives iron plate two recipes: `casting-iron`, from molten
    /// iron, and `iron-plate`, from ore. `casting-iron` sorts first, and
    /// charging every plate to molten iron -- which nothing on Nauvis makes,
    /// and which `sustained_production_rates` therefore treats as unknown --
    /// switched the entire iron and copper constraint off. Nothing looked
    /// wrong: the rates simply stayed at nameplate.
    #[test]
    fn a_recipe_whose_ingredients_the_base_never_makes_is_not_the_one_charged() {
        let recipes = crate::test_utils::fixture_recipes();
        recipes.insert(
            "casting-iron".to_string(),
            FactorioRecipe {
                name: "casting-iron".to_string(),
                valid: true,
                enabled: true,
                category: "metallurgy".to_string(),
                ingredients: Some(vec![crate::types::FactorioIngredient {
                    name: "molten-iron".to_string(),
                    ingredient_type: "fluid".to_string(),
                    amount: 10,
                }]),
                products: vec![crate::types::FactorioProduct {
                    name: "iron-plate".to_string(),
                    product_type: "item".to_string(),
                    amount: 2,
                    probability: Box::new(noisy_float::types::r64(1.)),
                }],
                hidden: false,
                energy: Box::new(noisy_float::types::r64(1.)),
                order: String::new(),
                group: String::new(),
                subgroup: String::new(),
            },
        );
        let flow_graph = FlowGraph::new(Arc::new(EntityGraph::new(
            Arc::new(crate::test_utils::fixture_entity_prototypes()),
            Arc::new(recipes),
        )));
        let standing: BTreeMap<String, f64> =
            [("iron-ore".to_string(), 1.), ("iron-plate".to_string(), 1.)]
                .into_iter()
                .collect();
        assert_eq!(
            flow_graph.recipe_making("iron-plate", &standing).as_deref(),
            Some("iron-plate"),
            "`casting-iron` sorts first but nothing here makes molten iron"
        );
    }

    /// The chain `test_furnace` asserts the shape of, as a fixture.
    fn smelting_chain() -> EntityGraph {
        entity_graph_from(vec![
            FactorioEntity::new_resource(
                &Position::new(0.5, -1.5),
                Direction::South,
                &EntityName::IronOre.to_string(),
            ),
            FactorioEntity::new_electric_mining_drill(&Position::new(0.5, -1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 1.5), Direction::North),
            FactorioEntity::new_stone_furnace(&Position::new(1., 3.), Direction::South),
            FactorioEntity::new_inserter(&Position::new(0.5, 4.5), Direction::North),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 5.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 6.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 7.5), Direction::South),
        ])
        .unwrap()
    }

    /// The falsification harness for every rate in this file, run against a
    /// **real** base rather than a fixture this repository wrote.
    ///
    /// Ignored by default and gated on `FACTORIO_BOT_WORLD_DUMP` naming a
    /// `world.dump` JSON, because the dump it was built for is 2.8 GB and lives
    /// in a workspace, not in the repository. It prints rather than asserts: the
    /// point is to put this graph's number beside one the game reported, and
    /// what the difference *means* is the analysis in
    /// `docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`, not a
    /// threshold.
    ///
    /// ```text
    /// FACTORIO_BOT_WORLD_DUMP=workspace/wrload/scripts/wr-census.json \
    ///   cargo test -p factorio-bot-core --release flow_graph -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn production_rates_of_a_dumped_world() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        let rates = surface.flow_graph.production_rates();
        let sustained = surface.flow_graph.sustained_production_rates();
        // The predecessor of `balance`, reproduced rather than remembered:
        // one ration over every line at once, which is what
        // `sustained_production_rates` did before a line's demand depended on
        // whether anything drains its output. Keeping the old column
        // computable is what makes a before/after honest -- both are measured
        // on the same dump by the same binary, and the note this feeds warns
        // that a baseline compared across two builds measures the builds.
        let lines = surface.flow_graph.nameplate_lines();
        let mut flat = vec![1_f64; lines.len()];
        FlowGraph::ration(&lines, &mut flat, |_| true, &BTreeMap::new(), false);
        let every_consumer_pulls = totals(&lines, &flat);
        // The shipped model as it stood before 2026-09-07: the same two
        // phases, solved as a monotone descent. Computed rather than
        // remembered, so the before column and the after column are the same
        // dump through the same binary -- this repository's standing warning
        // is that a baseline compared across two builds measures the builds.
        let descent = totals(&lines, &balance_by_descent(&lines));
        println!("-- production_rates of {path} --");
        println!(
            "{:>28}  {:>14}  {:>14}  {:>14}  {:>14}",
            "item", "nameplate/min", "pull-always/min", "descent/min", "sustained/min"
        );
        for (name, rate) in &rates {
            println!(
                "{name:>28}  {:>14.1}  {:>14.1}  {:>14.1}  {:>14.1}",
                rate * 60.,
                every_consumer_pulls.get(name).copied().unwrap_or_default() * 60.,
                descent.get(name).copied().unwrap_or_default() * 60.,
                sustained.get(name).copied().unwrap_or_default() * 60.
            );
        }
        println!("{} items", rates.len());
        println!();
        println!("-- against the game's own ten-minute statistics --");
        println!(
            "{:>22} {:>10} {:>10} {:>7} {:>10} {:>7} {:>10} {:>7}",
            "item", "game/min", "nameplate", "ratio", "descent", "ratio", "sustained", "ratio"
        );
        let columns = [&rates, &descent, &sustained];
        let mut error = [0_f64; 3];
        for (item, game) in game_reported_rates() {
            print!("{item:>22} {game:>10.0}");
            for (slot, column) in columns.iter().enumerate() {
                let model = column.get(item).copied().unwrap_or_default() * 60.;
                let ratio = model / game;
                error[slot] += ratio.ln().abs();
                print!(" {model:>10.1} {ratio:>7.2}");
            }
            println!();
        }
        let count = game_reported_rates().len() as f64;
        println!(
            "{:>22} {:>10} {:>19.3} {:>19.3} {:>19.3}",
            "mean abs log error",
            "",
            error[0] / count,
            error[1] / count,
            error[2] / count
        );
    }

    /// What the game reported making on Nauvis over ten minutes, from
    /// `docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`.
    fn game_reported_rates() -> Vec<(&'static str, f64)> {
        vec![
            ("copper-cable", 22367.),
            ("iron-ore", 15247.),
            ("iron-plate", 15170.),
            ("copper-ore", 15157.),
            ("copper-plate", 15147.),
            ("electronic-circuit", 6694.),
            ("coal", 4096.),
            ("plastic-bar", 2846.),
            ("stone", 1775.),
            ("steel-plate", 1213.),
            ("advanced-circuit", 942.),
            ("iron-gear-wheel", 578.),
            ("stone-brick", 450.),
            ("processing-unit", 249.),
        ]
    }

    /// What each line's item comes to at a given scale.
    fn totals(lines: &[ProductionLine], scale: &[f64]) -> BTreeMap<String, f64> {
        let mut total: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            *total.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
        }
        total
    }

    /// [`FlowGraph::balance`] as it was shipped between 2026-09-07 and this
    /// change: the same two phases and the same constraints, solved as a
    /// **monotone descent** in which a scale could only fall.
    ///
    /// Kept, and kept only here, so the before column of the record base's
    /// table is computed on the same binary as the after column instead of
    /// being quoted from a note.
    fn balance_by_descent(lines: &[ProductionLine]) -> Vec<f64> {
        let mut drained: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for line in lines {
            for ingredient in line.needs.keys() {
                drained.insert(ingredient.as_str());
            }
        }
        let known: Vec<bool> = lines
            .iter()
            .map(|line| drained.contains(line.item.as_str()))
            .collect();
        let mut scale = vec![1_f64; lines.len()];
        descend(lines, &mut scale, &known, true, &BTreeMap::new());
        let mut made: BTreeMap<String, f64> = BTreeMap::new();
        let mut eaten: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            if !known[index] {
                continue;
            }
            *made.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
            for (ingredient, rate) in &line.needs {
                *eaten.entry(ingredient.clone()).or_insert(0.) += rate * scale[index];
            }
        }
        let residual: BTreeMap<String, f64> = made
            .into_iter()
            .map(|(item, rate)| {
                let left = rate - eaten.get(&item).copied().unwrap_or_default();
                (item, left.max(0.))
            })
            .collect();
        let unknown: Vec<bool> = known.iter().map(|k| !*k).collect();
        descend(lines, &mut scale, &unknown, false, &residual);
        scale
    }

    /// The old monotone descent, for `balance_by_descent`.
    fn descend(
        lines: &[ProductionLine],
        scale: &mut [f64],
        selected: &[bool],
        cap_on_outlet: bool,
        floor: &BTreeMap<String, f64>,
    ) {
        for _ in 0..64 {
            let mut supply: BTreeMap<String, f64> = floor.clone();
            let mut demand: BTreeMap<String, f64> = BTreeMap::new();
            for (index, line) in lines.iter().enumerate() {
                if !selected[index] {
                    continue;
                }
                *supply.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
                for (ingredient, rate) in &line.needs {
                    *demand.entry(ingredient.clone()).or_insert(0.) += rate * scale[index];
                }
            }
            let mut moved = false;
            for (index, line) in lines.iter().enumerate() {
                if !selected[index] {
                    continue;
                }
                let mut limit = scale[index];
                for ingredient in line.needs.keys() {
                    let have = supply.get(ingredient).copied().unwrap_or_default();
                    let want = demand.get(ingredient).copied().unwrap_or_default();
                    if have <= 0. {
                        if floor.contains_key(ingredient) {
                            limit = 0.;
                        }
                        continue;
                    }
                    if want > have {
                        limit = limit.min(scale[index] * have / want);
                    }
                }
                if cap_on_outlet && !line.needs.is_empty() {
                    let outlet = demand.get(&line.item).copied().unwrap_or_default();
                    let standing = supply.get(&line.item).copied().unwrap_or_default();
                    if outlet > 0. && standing > outlet {
                        limit = limit.min(scale[index] * outlet / standing);
                    }
                }
                if limit < scale[index] {
                    scale[index] = limit;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }

    /// A probe: **what actually holds each line below its nameplate.**
    ///
    /// The question the brief for the outlet cap could not answer without it,
    /// and got wrong: it read `advanced-circuit` at 0.61x the game's own
    /// number as the outlet cap's sharp edge -- the mall pulling nothing --
    /// when advanced circuit is not outlet-limited at all. This prints, per
    /// item, how many of its lines are held by their outlet and how many by
    /// each ingredient, at the converged allocation. **Reach for it before
    /// attributing an error to a mechanism.**
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn what_holds_each_line_back() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        let lines = surface.flow_graph.nameplate_lines();
        let scale = FlowGraph::balance(&lines);
        let mut drained: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for line in &lines {
            for ingredient in line.needs.keys() {
                drained.insert(ingredient.as_str());
            }
        }
        let known: Vec<bool> = lines
            .iter()
            .map(|line| drained.contains(line.item.as_str()))
            .collect();
        let selected = |index: usize| known[index];
        let mut supply: BTreeMap<String, f64> = BTreeMap::new();
        let mut demand: BTreeMap<String, f64> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            if !selected(index) {
                continue;
            }
            *supply.entry(line.item.clone()).or_insert(0.) += line.rate * scale[index];
            for (ingredient, rate) in &line.needs {
                *demand.entry(ingredient.clone()).or_insert(0.) += rate * scale[index];
            }
        }
        let blocked = vec![f64::INFINITY; lines.len()];
        let ceiling = vec![1_f64; lines.len()];
        let pull = FlowGraph::relieved_pull(
            &lines, &scale, &ceiling, &supply, &demand, &blocked, &selected,
        );
        // The whole-surface tally, which is what the census's 21.7% of
        // assemblers reading `full_output` can be held against.
        let mut outlet_bound = 0_usize;
        let mut crafting = 0_usize;
        for (index, line) in lines.iter().enumerate() {
            if !known[index] || line.needs.is_empty() {
                continue;
            }
            crafting += 1;
            if scale[index] > 0.999_999 {
                continue;
            }
            let mut tightest = f64::INFINITY;
            for ingredient in line.needs.keys() {
                if let Some(share) =
                    FlowGraph::supply_ratio(ingredient, &supply, &demand, &BTreeMap::new())
                {
                    tightest = tightest.min(share);
                }
            }
            let outlet = pull.get(&line.item).copied().unwrap_or_default();
            let standing = supply.get(&line.item).copied().unwrap_or_default();
            if outlet > 0. && standing > 0. && outlet / standing < tightest {
                outlet_bound += 1;
            }
        }
        println!(
            "{outlet_bound} of {crafting} crafting lines are outlet-bound ({:.1}%), \
             out of {} lines in all",
            100. * outlet_bound as f64 / crafting as f64,
            lines.len()
        );
        println!("-- what holds each line below nameplate, at the fixed point --");
        for (item, _) in game_reported_rates() {
            let mut verdict: BTreeMap<String, usize> = BTreeMap::new();
            for (index, line) in lines.iter().enumerate() {
                if line.item != item || !known[index] {
                    continue;
                }
                if scale[index] > 0.999_999 {
                    *verdict.entry("at nameplate".into()).or_insert(0) += 1;
                    continue;
                }
                let mut tightest = f64::INFINITY;
                let mut who = "unconstrained".to_string();
                for ingredient in line.needs.keys() {
                    if let Some(share) =
                        FlowGraph::supply_ratio(ingredient, &supply, &demand, &BTreeMap::new())
                        && share < tightest
                    {
                        tightest = share;
                        who = format!("short of {ingredient}");
                    }
                }
                if !line.needs.is_empty() {
                    let outlet = pull.get(&line.item).copied().unwrap_or_default();
                    let standing = supply.get(&line.item).copied().unwrap_or_default();
                    if outlet > 0. && standing > 0. && outlet / standing < tightest {
                        who = "outlet".to_string();
                    }
                }
                *verdict.entry(who).or_insert(0) += 1;
            }
            println!("{item:>22}  {verdict:?}");
        }
    }

    /// A probe: what the model says is MADE of each item beside what it says
    /// is EATEN of it, both at nameplate.
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn what_the_model_thinks_is_consumed() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        let lines = surface.flow_graph.nameplate_lines();
        let mut drained: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for line in &lines {
            for ingredient in line.needs.keys() {
                drained.insert(ingredient.as_str());
            }
        }
        let mut made: BTreeMap<String, f64> = BTreeMap::new();
        let mut known: BTreeMap<String, f64> = BTreeMap::new();
        let mut unknown: BTreeMap<String, f64> = BTreeMap::new();
        for line in &lines {
            *made.entry(line.item.clone()).or_insert(0.) += line.rate;
            let bucket = if drained.contains(line.item.as_str()) {
                &mut known
            } else {
                &mut unknown
            };
            for (ingredient, rate) in &line.needs {
                *bucket.entry(ingredient.clone()).or_insert(0.) += rate;
            }
        }
        let mut items: Vec<&String> = made
            .keys()
            .chain(known.keys())
            .chain(unknown.keys())
            .collect();
        items.sort();
        items.dedup();
        println!(
            "{:>28}  {:>12}  {:>12}  {:>12}  {:>8}",
            "item", "made/min", "known/min", "unknown/min", "k/m"
        );
        for item in items {
            let m = made.get(item).copied().unwrap_or_default() * 60.;
            let k = known.get(item).copied().unwrap_or_default() * 60.;
            let u = unknown.get(item).copied().unwrap_or_default() * 60.;
            let r = if m > 0. {
                format!("{:.2}", k / m)
            } else {
                "-".into()
            };
            println!("{item:>28}  {m:>12.1}  {k:>12.1}  {u:>12.1}  {r:>8}");
        }
    }

    /// **Can this graph's edge weights carry a back-pressure term at all?**
    ///
    /// A duty cycle is `what arrives / what the machine could eat`, so it is
    /// only as good as "what arrives". This puts, per item, the total the
    /// graph says is *produced* beside the total its consumers say is
    /// *arriving*. In a conserved flow the second cannot exceed the first.
    ///
    /// It is not conserved, and the reason is by design:
    /// [`FlowGraph::update_flow_edge`] writes a machine's **whole** output on
    /// **each** of its outgoing edges -- which is what makes
    /// [`FlowGraph::throughput_at`] right for any one consumer -- and
    /// `sum_incoming_edge_weights` then adds those up. So one drill feeding
    /// three arms of a line reads as three drills' worth at each arm.
    /// [`FlowGraph::production_rates`] already compensates for this on the
    /// *producer* side by taking a maximum rather than a sum; nothing does on
    /// the consumer side, and there is nothing available to do it with.
    ///
    /// Printed rather than asserted, like its sibling: the ratio is a fact
    /// about a particular base, and the conclusion drawn from it lives in
    /// `docs/superpowers/notes/2026-09-07-a-machine-standing-still.md`.
    #[test]
    #[ignore = "needs a world dump named by FACTORIO_BOT_WORLD_DUMP"]
    fn what_consumers_see_arriving_is_not_a_conserved_flow() {
        let Ok(path) = std::env::var("FACTORIO_BOT_WORLD_DUMP") else {
            panic!("set FACTORIO_BOT_WORLD_DUMP to a world.dump JSON");
        };
        let json = std::fs::read_to_string(&path).expect("the dump reads");
        let surface: crate::factorio::world::FactorioSurface =
            serde_json::from_str(&json).expect("the dump parses");
        let produced = surface.flow_graph.production_rates();
        // What every consuming machine believes is arriving at its own tile.
        let mut arriving: BTreeMap<String, f64> = BTreeMap::new();
        let mut consumers = 0_usize;
        {
            let graph = surface.flow_graph.inner_graph();
            for node_index in graph.node_indices() {
                let Some(node) = graph.node_weight(node_index) else {
                    continue;
                };
                if !matches!(
                    node.entity_type,
                    EntityType::Furnace | EntityType::AssemblingMachine
                ) {
                    continue;
                }
                consumers += 1;
                for (name, rate) in surface.flow_graph.sum_incoming_edge_weights(&node.position) {
                    *arriving.entry(name).or_insert(0.) += rate;
                }
            }
        }
        println!("-- supply conservation over {consumers} consumers of {path} --");
        println!(
            "{:>28}  {:>12}  {:>12}  {:>8}",
            "item", "produced/min", "arriving/min", "ratio"
        );
        for (name, seen) in &arriving {
            let made = produced.get(name).copied().unwrap_or_default();
            let ratio = if made > 0. {
                format!("{:.2}", seen / made)
            } else {
                "-".to_string()
            };
            println!(
                "{name:>28}  {:>12.1}  {:>12.1}  {ratio:>8}",
                made * 60.,
                seen * 60.
            );
        }
    }
}
