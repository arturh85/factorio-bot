use crate::factorio::util::{add_to_rect, format_dotgraph};
use crate::graph::entity_graph::{EntityGraph, EntityNode, QuadTreeRect};
use crate::num_traits::FromPrimitive;
use crate::types::{
    Direction, EntityName, EntityType, FactorioEntity, FactorioEntityPrototype, FactorioRecipe,
    Position, Rect,
};
use aabb_quadtree::{ItemId, QuadTree};
use dashmap::DashMap;
use euclid::{TypedPoint2D, TypedSize2D};
use miette::Result;
use num_traits::ToPrimitive;
use parking_lot::{RwLock, RwLockReadGuard};
use petgraph::dot::{Config, Dot};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::{depth_first_search, Bfs, Control, DfsEvent, EdgeRef};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

pub struct FlowGraph {
    entity_graph: Arc<EntityGraph>,
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    recipes: Arc<DashMap<String, FactorioRecipe>>,
    flow_tree: RwLock<FlowQuadTree>,
    inner: RwLock<FlowGraphInner>,
}

impl Clone for FlowGraph {
    fn clone(&self) -> Self {
        FlowGraph {
            entity_graph: Arc::new((*self.entity_graph).clone()),
            entity_prototypes: Arc::new((*self.entity_prototypes).clone()),
            recipes: Arc::new((*self.recipes).clone()),
            flow_tree: RwLock::new((*self.flow_tree.read()).clone()),
            inner: RwLock::new((*self.inner.read()).clone()),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.entity_graph = Arc::new((*source.entity_graph).clone());
        self.entity_prototypes = Arc::new((*source.entity_prototypes).clone());
        self.recipes = Arc::new((*source.recipes).clone());
        self.flow_tree = RwLock::new((*source.flow_tree.read()).clone());
        self.inner = RwLock::new((*source.inner.read()).clone());
    }
}

impl FlowGraph {
    pub fn new(entity_graph: Arc<EntityGraph>) -> Self {
        FlowGraph {
            entity_prototypes: entity_graph.entity_prototypes(),
            recipes: entity_graph.recipes(),
            entity_graph,
            flow_tree: RwLock::new(FlowQuadTree::new(
                QuadTreeRect::new(
                    TypedPoint2D::new(-5120., -5120.),
                    TypedSize2D::new(10240., 10240.),
                ),
                true,
                32,
                128,
                32,
                8,
            )),
            inner: RwLock::new(FlowGraphInner::new()),
        }
    }

    pub fn update(&self) -> Result<()> {
        let _started = Instant::now();
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
                                let miner_ore = entity_root.miner_ore.as_ref().unwrap();
                                let mining_speed = self
                                    .entity_prototypes
                                    .get(&entity_root.entity_name)
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "entity '{}' not found in prototypes",
                                            &entity_root.entity_name
                                        )
                                    })
                                    .mining_speed
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "entity '{}' has no mining_speed",
                                            &entity_root.entity_name
                                        )
                                    })
                                    .to_f64()
                                    .unwrap();
                                let mining_time = self
                                    .entity_prototypes
                                    .get(miner_ore)
                                    .unwrap_or_else(|| {
                                        panic!("entity '{}' not found in prototypes", &miner_ore)
                                    })
                                    .mining_time
                                    .unwrap_or_else(|| {
                                        panic!("entity '{}' has no mining_time", &miner_ore)
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
                                        let mut output: FlowRates = vec![];
                                        for product in recipe.products.iter() {
                                            // FIXME: only if enough input?
                                            output.push((
                                                product.name.clone(),
                                                product.amount as f64 / 3.2, // FIXME: correct amount based on assembler speed
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
                                /*
                                Smelting iron, copper, and stone each take a base 3.2 seconds to finish.
                                Smelting steel takes a base 16 seconds.
                                Stone Furnaces have a crafting speed of 1.
                                Both Steel and Electric Furnaces have a crafting speed of 2.
                                One furnace making iron can support one furnace making steel.
                                Stone and Steel Furnaces consume 0.0225 coal/second.
                                             */

                                let mut output: FlowRates = vec![];
                                for (name, _rate) in &incoming {
                                    if let Ok(name) = EntityName::from_str(name) {
                                        match name {
                                            EntityName::IronOre => output.push((
                                                EntityName::IronPlate.to_string(),
                                                1. / 3.2,
                                            )),
                                            EntityName::CopperOre => output.push((
                                                EntityName::CopperPlate.to_string(),
                                                1. / 3.2,
                                            )),
                                            EntityName::Stone => output.push((
                                                EntityName::StoneBrick.to_string(),
                                                1. / 3.2,
                                            )),
                                            EntityName::IronPlate => output
                                                .push((EntityName::Steel.to_string(), 1. / 3.2)),
                                            EntityName::Coal => {}
                                            _ => warn!("invalid furnace input: {}", name),
                                        }
                                    } else {
                                        warn!("invalid furnace input: {}", name);
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

    pub fn get_or_create_flow_node(&self, entity_node: &EntityNode) -> NodeIndex {
        self.node_at(&entity_node.position).unwrap_or_else(|| {
            let entity_id = entity_node.entity_id.unwrap();
            let entity = self.entity_graph.entity_by_id(entity_id).unwrap();
            let new_index = self.inner.write().add_node(FlowNode::new(
                &entity,
                entity_node.miner_ore.clone(),
                entity_id,
            ));
            self.flow_tree
                .write()
                .insert_with_box(new_index, entity_node.bounding_box.clone().into());
            new_index
        })
    }

    pub fn update_flow_edge(
        &self,
        flow: FlowEdge,
        source_entity_node: &EntityNode,
        target_entity_node: &EntityNode,
    ) {
        let source_flow_idx = self.get_or_create_flow_node(source_entity_node);
        let target_flow_idx = self.get_or_create_flow_node(target_entity_node);
        self.inner
            .write()
            .update_edge(source_flow_idx, target_flow_idx, flow);
    }

    pub fn inner_graph(&self) -> RwLockReadGuard<FlowGraphInner> {
        self.inner.read()
    }
    pub fn node_at(&self, position: &Position) -> Option<NodeIndex> {
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
        map.into_iter()
            .map(|(name, production_rate)| (name, production_rate))
            .collect()
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
    pub fn graphviz_dot(&self) -> String {
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
    pub fn new(entity: &FactorioEntity, miner_ore: Option<String>, entity_id: ItemId) -> FlowNode {
        let direction = Direction::from_u8(entity.direction).unwrap();
        let entity_type = EntityType::from_str(&entity.entity_type).unwrap();
        FlowNode {
            position: entity.position.clone(),
            entity_id: Some(entity_id),
            entity_name: entity.name.clone(),
            direction,
            miner_ore,
            entity_type,
        }
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

#[cfg(test)]
mod tests {
    use crate::test_utils::entity_graph_from;

    use super::*;

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
