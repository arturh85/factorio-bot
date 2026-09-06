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
use std::collections::HashMap;
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
                                let mut output: FlowRates = vec![];
                                for (name, _rate) in &incoming {
                                    if let Some(rate) =
                                        self.smelting_output(&source_node.entity_name, name)
                                    {
                                        self.add_production_rate(&mut output, rate);
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
        let recipe = self.recipes.get(recipe_name)?;
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
}
