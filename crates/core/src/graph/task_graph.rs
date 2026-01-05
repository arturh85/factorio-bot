use crate::factorio::util::format_dotgraph;
use crate::gantt_mermaid::MermaidGanttBuilder;
use crate::num_traits::FromPrimitive;
use crate::types::{
    Direction, FactorioEntity, InventoryItem, InventoryLocation, MineTarget, PlayerId,
    PositionRadius,
};
use miette::Result;
use noisy_float::types::{r64, R64};
use num_traits::ToPrimitive;
use parking_lot::RwLock;
use petgraph::algo::astar;
use petgraph::dot::{Config, Dot};
use petgraph::graph::{DefaultIx, EdgeIndex, NodeIndex};
use petgraph::stable_graph::{Edges, NodeIndices, StableGraph};
use petgraph::visit::EdgeRef;
use petgraph::Directed;
use ptree::graph::print_graph;
use std::collections::HashMap;
use std::fmt::Formatter;
use std::sync::Arc;

#[derive(Clone)]
pub struct TaskGraph {
    inner: TaskGraphInner,
    pub start_node: NodeIndex,
    pub end_node: NodeIndex,
    pub cursor: NodeIndex,
    groups: Vec<HashMap<PlayerId, NodeIndex>>,
}

impl TaskGraph {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut graph = TaskGraphInner::new();
        let start_node = graph.add_node(TaskNode::new(None, "Process Start", None, 0.));
        let end_node = graph.add_node(TaskNode::new(None, "Process End", None, 0.));
        let cursor = start_node;
        graph.add_edge(start_node, end_node, 0.);
        TaskGraph {
            inner: graph,
            start_node,
            end_node,
            cursor,
            groups: Vec::new(),
        }
    }

    fn add_to_cursor(&mut self, node: NodeIndex) {
        if let Some(edge) = self.inner.find_edge(self.cursor, self.end_node) {
            self.inner.remove_edge(edge);
        }
        self.inner.add_edge(self.cursor, node, 0.);
        self.cursor = node;
        self.inner.add_edge(self.cursor, self.end_node, 0.);
    }

    fn add_to_group(&mut self, player_id: PlayerId, node: NodeIndex, cost: f64) {
        let mut cursor = self.cursor;
        if let Some(group) = self.groups.last_mut() {
            if let Some(player_cursor) = group.get(&player_id) {
                cursor = *player_cursor
            }
            group.insert(player_id, node);
            self.inner.add_edge(cursor, node, cost);
        } else {
            panic!("no group to add to?");
            // self.inner.add_edge(self.cursor, node, 0.);
        }
    }

    pub fn group_start(&mut self, label: &str) {
        let group_start =
            self.inner
                .add_node(TaskNode::new(None, &format!("Start: {}", label), None, 0.));
        self.add_to_cursor(group_start);
        self.groups.push(HashMap::new());
    }

    pub fn group_end(&mut self) {
        let group = self.groups.pop().expect("no open group");
        let group_end = self.inner.add_node(TaskNode::new(None, "End", None, 0.));
        if group.is_empty() {
            self.inner.add_edge(self.cursor, group_end, 0.);
        } else {
            let mut weights: HashMap<NodeIndex, R64> = HashMap::new();
            for (_, cursor) in group {
                weights.insert(cursor, self.weight(self.cursor, cursor));
            }
            let max_weight = *weights.values().max().unwrap();
            for (cursor, weight) in weights {
                self.inner
                    .add_edge(cursor, group_end, (max_weight - weight).to_f64().unwrap());
            }
        }
        if let Some(edge) = self.inner.find_edge(self.cursor, self.end_node) {
            self.inner.remove_edge(edge);
        }
        self.cursor = group_end;
        self.inner.add_edge(self.cursor, self.end_node, 0.);
    }

    pub fn add_mine_node(&mut self, player_id: PlayerId, cost: f64, target: MineTarget) {
        let mut task_node = TaskNode::new_mine(player_id, target.clone(), cost);
        // Populate outputs with mined resources
        task_node.outputs.push(ResourceFlow {
            item_name: target.name.clone(),
            count: target.count,
        });
        let node = self.inner.add_node(task_node);
        self.add_to_group(player_id, node, cost);
    }

    pub fn add_walk_node(&mut self, player_id: PlayerId, cost: f64, target: PositionRadius) {
        let node = self
            .inner
            .add_node(TaskNode::new_walk(player_id, target, cost));
        self.add_to_group(player_id, node, cost);
    }

    pub fn add_place_node(&mut self, player_id: PlayerId, cost: f64, entity: FactorioEntity) {
        let mut task_node = TaskNode::new_place(player_id, entity.clone(), cost);
        // Populate inputs - placing an entity consumes 1 item from inventory
        task_node.inputs.push(ResourceFlow {
            item_name: entity.name.clone(),
            count: 1,
        });
        let node = self.inner.add_node(task_node);
        self.add_to_group(player_id, node, cost);
    }

    pub fn add_insert_into_inventory_node(
        &mut self,
        player_id: PlayerId,
        cost: f64,
        location: InventoryLocation,
        item: InventoryItem,
    ) {
        let mut task_node = TaskNode::new_insert_to_inventory(
            player_id,
            location.clone(),
            item.clone(),
            cost,
        );
        // Populate inputs - inserting an item consumes it from player's inventory
        task_node.inputs.push(ResourceFlow {
            item_name: item.name.clone(),
            count: item.count,
        });
        let node = self.inner.add_node(task_node);
        self.add_to_group(player_id, node, cost);
    }

    pub fn add_remove_from_inventory_node(
        &mut self,
        player_id: PlayerId,
        cost: f64,
        location: InventoryLocation,
        item: InventoryItem,
    ) {
        let node = self.inner.add_node(TaskNode::new_remove_from_inventory(
            player_id, location, item, cost,
        ));
        self.add_to_group(player_id, node, cost);
    }

    /// Automatically creates dependency edges based on resource flow
    /// For each task with inputs, finds tasks that produce matching outputs and adds edges
    pub fn resolve_dependencies(&mut self) {
        let node_indices: Vec<_> = self.inner.node_indices().collect();

        for consumer_idx in &node_indices {
            let consumer = match self.inner.node_weight(*consumer_idx) {
                Some(n) => n.clone(),
                None => continue,
            };

            // Skip if no inputs required
            if consumer.inputs.is_empty() {
                continue;
            }

            // Find all producer tasks that output resources this consumer needs
            for producer_idx in &node_indices {
                if producer_idx == consumer_idx {
                    continue;
                }

                let producer = match self.inner.node_weight(*producer_idx) {
                    Some(n) => n,
                    None => continue,
                };

                // Check if producer outputs match consumer inputs
                let has_matching_resource = producer.outputs.iter().any(|output| {
                    consumer.inputs.iter().any(|input| {
                        input.item_name == output.item_name
                    })
                });

                if has_matching_resource {
                    // Only add edge if it doesn't already exist
                    if self.inner.find_edge(*producer_idx, *consumer_idx).is_none() {
                        // Get the estimated time from producer's status
                        let time = match *producer.status.read() {
                            TaskStatus::Planned(t) => t,
                            _ => 0.0,
                        };
                        self.inner.add_edge(*producer_idx, *consumer_idx, time);
                    }
                }
            }
        }
    }

    /// Validates that all resource requirements are satisfied
    /// Returns an error if any task requires resources that aren't produced
    pub fn validate_resource_flow(&self) -> Result<()> {
        use crate::errors::InsufficientResources;

        // Track available resources per player
        let mut player_resources: HashMap<PlayerId, HashMap<String, u32>> = HashMap::new();

        // Process nodes in topological order (start to end)
        for node_idx in self.inner.node_indices() {
            let node = match self.inner.node_weight(node_idx) {
                Some(n) => n,
                None => continue,
            };

            // Skip nodes without a player (start/end markers)
            let player_id = match node.player_id {
                Some(id) => id,
                None => continue,
            };

            // Initialize player's resource tracker if needed
            let resources = player_resources.entry(player_id).or_default();

            // Check if inputs are satisfied
            for input in &node.inputs {
                let available = resources.get(&input.item_name).copied().unwrap_or(0);
                if available < input.count {
                    return Err(InsufficientResources {
                        task_name: node.name.clone(),
                        player_id,
                        item_name: input.item_name.clone(),
                        required: input.count,
                        available,
                    }
                    .into());
                }
                // Consume the input
                resources.insert(input.item_name.clone(), available - input.count);
            }

            // Add outputs to available resources
            for output in &node.outputs {
                let current = resources.get(&output.item_name).copied().unwrap_or(0);
                resources.insert(output.item_name.clone(), current + output.count);
            }
        }

        Ok(())
    }

    pub fn node_weight(&self, idx: NodeIndex) -> Option<&TaskNode> {
        self.inner.node_weight(idx)
    }
    pub fn weight(&self, start: NodeIndex, goal: NodeIndex) -> R64 {
        let (weight, _) = self.astar(start, goal).expect("failed to find path");
        r64(weight)
    }

    pub fn node_indices(&self) -> NodeIndices<'_, TaskNode, DefaultIx> {
        self.inner.node_indices()
    }

    pub fn shortest_path(&self) -> Option<f64> {
        let process_start = self.inner.node_indices().next().unwrap();
        let process_end = self.inner.node_indices().next_back().unwrap();
        self.shortest_path_between(process_start, process_end)
    }

    pub fn shortest_path_between(&self, start: NodeIndex, end: NodeIndex) -> Option<f64> {
        let (weight, _) = self.astar(start, end)?;
        Some(weight)
    }

    pub fn add_group_start_node(&mut self, parent: NodeIndex, label: &str) -> NodeIndex {
        let start =
            self.inner
                .add_node(TaskNode::new(None, &format!("Start: {}", label), None, 0.));
        self.inner.add_edge(parent, start, 0.);

        start
    }
    pub fn print(&self) {
        print_graph(&self.inner.clone().into(), self.start_node).unwrap();
    }
    pub fn graphviz_dot(&self) -> String {
        format_dotgraph(Dot::with_config(&self.inner, &[Config::GraphContentOnly]).to_string())
    }

    pub fn mermaid_gantt(&self, bot_ids: Vec<PlayerId>, title: &str) -> String {
        let mut builder = MermaidGanttBuilder::new(title);
        let total_runtime = self.shortest_path().expect("no path found");

        builder = builder.add_milestone("test", "m1", &duration_to_timestamp(total_runtime), 0.);
        // let milestone_by_index: HashMap<NodeIndex, String> = HashMap::new();

        for player_id in bot_ids {
            builder = builder.add_section(&format!("Bot {}", player_id));
            let mut cursor = self.start_node;
            let mut last_weight: Option<f64> = None;

            while cursor != self.end_node {
                let node = self
                    .node_weight(cursor)
                    .expect("NodeIndices should all be valid");

                let status = node.status.read();
                match *status {
                    TaskStatus::Planned(estimated) => {
                        builder = builder.add_action(
                            &node.name,
                            if estimated > 0. {
                                estimated
                            } else {
                                last_weight.unwrap_or(0.)
                            },
                            if cursor == self.start_node {
                                Some("00:00:00")
                            } else {
                                None
                            },
                        );
                    }
                    TaskStatus::Success(estimated, _realtime) => {
                        builder = builder.add_action(
                            &node.name,
                            if estimated > 0. {
                                estimated
                            } else {
                                last_weight.unwrap_or(0.)
                            },
                            if cursor == self.start_node {
                                Some("00:00:00")
                            } else {
                                None
                            },
                        );
                    }
                    _ => {}
                };

                let cursor_copy = cursor;
                for edge in self.edges_directed(cursor, petgraph::Direction::Outgoing) {
                    let target_idx = edge.target();
                    last_weight = Some(*edge.weight());
                    let target = self
                        .node_weight(target_idx)
                        .expect("NodeIndices should all be valid");
                    if target.player_id.is_none() || target.player_id.unwrap() == player_id {
                        cursor = target_idx;
                    }
                }
                if cursor == cursor_copy {
                    error!("no change in cursor!?");
                    break;
                }
            }
        }

        builder.build()
    }

    pub fn add_node(&mut self, task: TaskNode) -> NodeIndex {
        self.inner.add_node(task)
    }

    pub fn add_edge(&mut self, a: NodeIndex, b: NodeIndex, weight: f64) -> EdgeIndex {
        self.inner.add_edge(a, b, weight)
    }

    pub fn astar(&self, start: NodeIndex, goal: NodeIndex) -> Option<(f64, Vec<NodeIndex>)> {
        astar(
            &self.inner,
            start,
            |finish| finish == goal,
            |e| *e.weight(),
            |_| 0.,
        )
    }

    pub fn edges_directed(
        &self,
        i: NodeIndex,
        dir: petgraph::Direction,
    ) -> Edges<'_, f64, Directed, DefaultIx> {
        self.inner.edges_directed(i, dir)
    }
}

/// Represents resource flow (items produced or consumed) for a task
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceFlow {
    pub item_name: String,
    pub count: u32,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum TaskData {
    Mine(MineTarget),
    Walk(PositionRadius),
    Craft(InventoryItem),
    InsertToInventory(InventoryLocation, InventoryItem),
    RemoveFromInventory(InventoryLocation, InventoryItem),
    PlaceEntity(FactorioEntity),
}

pub enum TaskStatus {
    Planned(f64),
    Running(u32, u32),
    Success(f64, u32),
    Failed(u32, u32, String),
}

#[derive(Clone)]
pub struct TaskNode {
    pub name: String,
    pub player_id: Option<PlayerId>,
    pub data: Option<TaskData>,
    pub status: Arc<RwLock<TaskStatus>>,
    pub inputs: Vec<ResourceFlow>,
    pub outputs: Vec<ResourceFlow>,
}

impl TaskNode {
    pub fn new(
        player_id: Option<PlayerId>,
        name: &str,
        data: Option<TaskData>,
        cost: f64,
    ) -> TaskNode {
        TaskNode {
            name: name.into(),
            player_id,
            data,
            status: Arc::new(RwLock::new(TaskStatus::Planned(cost))),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }
    pub fn new_craft(player_id: PlayerId, item: InventoryItem, cost: f64) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!(
                "Craft {}{}",
                item.name,
                if item.count > 1 {
                    format!(" x {}", item.count)
                } else {
                    String::new()
                }
            ),
            Some(TaskData::Craft(item)),
            cost,
        )
    }
    pub fn new_walk(player_id: PlayerId, target: PositionRadius, cost: f64) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!("Walk to {}", target.position),
            Some(TaskData::Walk(target)),
            cost,
        )
    }
    pub fn new_mine(player_id: PlayerId, target: MineTarget, cost: f64) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!(
                "Mining {}{}",
                target.name,
                if target.count > 1 {
                    format!(" x {}", target.count)
                } else {
                    String::new()
                }
            ),
            Some(TaskData::Mine(target)),
            cost,
        )
    }
    pub fn new_place(player_id: PlayerId, entity: FactorioEntity, cost: f64) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!(
                "Place {} at {} ({:?})",
                entity.name,
                entity.position,
                Direction::from_u8(entity.direction).unwrap()
            ),
            Some(TaskData::PlaceEntity(entity)),
            cost,
        )
    }
    pub fn new_insert_to_inventory(
        player_id: PlayerId,
        location: InventoryLocation,
        item: InventoryItem,
        cost: f64,
    ) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!(
                "Insert {}x{} into {} at {}",
                &item.name, &item.count, location.entity_name, location.position
            ),
            Some(TaskData::InsertToInventory(location, item)),
            cost,
        )
    }
    pub fn new_remove_from_inventory(
        player_id: PlayerId,
        location: InventoryLocation,
        item: InventoryItem,
        cost: f64,
    ) -> TaskNode {
        TaskNode::new(
            Some(player_id),
            &format!(
                "Remove {}x{} from {} at {}",
                &item.name, &item.count, location.entity_name, location.position
            ),
            Some(TaskData::RemoveFromInventory(location, item)),
            cost,
        )
    }
}

impl std::fmt::Display for TaskNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)?;
        Ok(())
    }
}
impl std::fmt::Debug for TaskNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)?;
        Ok(())
    }
}

pub type TaskGraphInner = StableGraph<TaskNode, f64>;

fn duration_to_timestamp(duration: f64) -> String {
    let duration = duration as u64;
    let seconds = duration % 60;
    let minutes = (duration / 60) % 60;
    let hours = (duration / 60 / 60) % 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Position;

    #[test]
    fn test_simple_group() {
        let mut task_graph = TaskGraph::new();
        task_graph.group_start("foo");
        task_graph.add_mine_node(
            1,
            3.,
            MineTarget {
                position: Position::default(),
                count: 1,
                name: "iron-ore".into(),
            },
        );
        task_graph.group_end();

        assert_eq!(
            task_graph.graphviz_dot(),
            r##"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: foo" ]
    3 [ label = "Mining iron-ore" ]
    4 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "3" ]
    3 -> 4 [ label = "0" ]
    4 -> 1 [ label = "0" ]
}
"##,
        );
    }

    #[test]
    fn test_diverging_group() {
        let mut task_graph = TaskGraph::new();
        task_graph.group_start("foo");
        task_graph.add_mine_node(
            1,
            3.,
            MineTarget {
                position: Position::default(),
                count: 1,
                name: "iron-ore".into(),
            },
        );
        task_graph.add_mine_node(
            1,
            3.,
            MineTarget {
                position: Position::default(),
                count: 1,
                name: "iron-ore".into(),
            },
        );
        task_graph.add_mine_node(
            2,
            3.,
            MineTarget {
                position: Position::default(),
                count: 1,
                name: "iron-ore".into(),
            },
        );
        task_graph.group_end();

        assert_eq!(
            task_graph.graphviz_dot(),
            r##"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: foo" ]
    3 [ label = "Mining iron-ore" ]
    4 [ label = "Mining iron-ore" ]
    5 [ label = "Mining iron-ore" ]
    6 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "3" ]
    2 -> 5 [ label = "3" ]
    3 -> 4 [ label = "3" ]
    4 -> 6 [ label = "0" ]
    5 -> 6 [ label = "3" ]
    6 -> 1 [ label = "0" ]
}
"##,
        );
    }
}
