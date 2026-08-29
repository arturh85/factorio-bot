//! Action network (DAG) construction and validation. Filled in by Task 4.

use crate::action::Action;
use crate::error::PlannerError;
use crate::ids::{ActionId, Ticks};
use factorio_bot_core::petgraph::algo::toposort;
use factorio_bot_core::petgraph::graph::{DiGraph, NodeIndex};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: ActionId,
    pub to: ActionId,
    pub lag: Ticks,
}

/// A partially ordered set of actions. No bot appears anywhere in it.
#[derive(Clone, Debug, Default)]
pub struct ActionNetwork {
    actions: BTreeMap<ActionId, Action>,
    edges: Vec<Edge>,
}

impl ActionNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, action: Action) -> ActionId {
        let id = action.id;
        self.actions.insert(id, action);
        id
    }

    /// Add an explicit ordering edge. `lag` is the minimum number of ticks
    /// after `from` finishes before `to` may start — a furnace's smelting
    /// time, for example.
    pub fn link(&mut self, from: ActionId, to: ActionId, lag: Ticks) {
        if let Some(existing) = self.edges.iter_mut().find(|e| e.from == from && e.to == to) {
            existing.lag = existing.lag.max(lag);
            return;
        }
        self.edges.push(Edge { from, to, lag });
    }

    pub fn action(&self, id: ActionId) -> Option<&Action> {
        self.actions.get(&id)
    }

    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        self.actions.values()
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Predecessors of `id` with their lags, ascending by predecessor id.
    pub fn preds(&self, id: ActionId) -> Vec<(ActionId, Ticks)> {
        let mut out: Vec<(ActionId, Ticks)> = self
            .edges
            .iter()
            .filter(|e| e.to == id)
            .map(|e| (e.from, e.lag))
            .collect();
        out.sort_unstable();
        out
    }

    /// Add ordering edges implied by preconditions and effects.
    ///
    /// A producer of an item is linked to a consumer of that item unless the
    /// edge would close a cycle. Candidates are considered in ascending
    /// `(producer, consumer)` order, so the result is deterministic.
    pub fn infer_edges(&mut self) {
        let ids: Vec<ActionId> = self.actions.keys().copied().collect();
        for consumer in &ids {
            let wanted: Vec<(String, u32)> = self.actions[consumer]
                .pre
                .iter()
                .filter_map(|c| match c {
                    crate::action::Condition::HasItem { item, count, .. } => {
                        Some((item.clone(), *count))
                    }
                    _ => None,
                })
                .collect();
            if wanted.is_empty() {
                continue;
            }
            for producer in &ids {
                if producer == consumer {
                    continue;
                }
                let produces = self.actions[producer]
                    .eff
                    .iter()
                    .filter_map(|e| e.produces())
                    .any(|(item, _)| wanted.iter().any(|(w, _)| w == item));
                if !produces {
                    continue;
                }
                if self
                    .edges
                    .iter()
                    .any(|e| e.from == *producer && e.to == *consumer)
                {
                    continue;
                }
                self.link(*producer, *consumer, 0);
                if self.validate().is_err() {
                    self.edges.pop();
                }
            }
        }
    }

    fn as_graph(&self) -> (DiGraph<ActionId, Ticks>, BTreeMap<ActionId, NodeIndex>) {
        let mut graph = DiGraph::new();
        let mut index = BTreeMap::new();
        for id in self.actions.keys() {
            index.insert(*id, graph.add_node(*id));
        }
        for edge in &self.edges {
            if let (Some(from), Some(to)) = (index.get(&edge.from), index.get(&edge.to)) {
                graph.add_edge(*from, *to, edge.lag);
            }
        }
        (graph, index)
    }

    /// Fails if the network contains a cycle.
    pub fn validate(&self) -> Result<(), PlannerError> {
        let (graph, _) = self.as_graph();
        match toposort(&graph, None) {
            Ok(_) => Ok(()),
            Err(cycle) => Err(PlannerError::CyclicNetwork(graph[cycle.node_id()])),
        }
    }

    pub fn topo_order(&self) -> Result<Vec<ActionId>, PlannerError> {
        let (graph, _) = self.as_graph();
        match toposort(&graph, None) {
            Ok(order) => Ok(order.into_iter().map(|n| graph[n]).collect()),
            Err(cycle) => Err(PlannerError::CyclicNetwork(graph[cycle.node_id()])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind, Actor, Condition, Effect};
    use crate::ids::{ActionIdGen, BotId};
    use factorio_bot_core::types::Position;

    fn mine(gen: &mut ActionIdGen, item: &str, count: u32) -> Action {
        let id = gen.next();
        Action {
            id,
            kind: ActionKind::Mine {
                pos: Position::new(1., 1.),
                item: item.into(),
                count,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: Position::new(1., 1.),
                radius: 3.0,
            }],
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: item.into(),
                count,
            }],
            duration: 60,
            pinned: None,
            label: format!("mine {} {}", count, item),
        }
    }

    fn craft(gen: &mut ActionIdGen, from: &str, need: u32, to: &str) -> Action {
        let id = gen.next();
        Action {
            id,
            kind: ActionKind::Craft {
                item: to.into(),
                count: 1,
            },
            pre: vec![Condition::HasItem {
                who: Actor::Role,
                item: from.into(),
                count: need,
            }],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: from.into(),
                    count: need,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: to.into(),
                    count: 1,
                },
            ],
            duration: 30,
            pinned: None,
            label: format!("craft {}", to),
        }
    }

    #[test]
    fn inference_links_a_producer_to_its_consumer() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let m = net.add(mine(&mut gen, "iron-plate", 2));
        let c = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.infer_edges();
        assert_eq!(net.preds(c), vec![(m, 0)]);
        assert!(net.preds(m).is_empty());
    }

    #[test]
    fn inference_does_not_link_unrelated_items() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(mine(&mut gen, "copper-ore", 2));
        let c = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.infer_edges();
        assert!(net.preds(c).is_empty());
    }

    #[test]
    fn inference_never_creates_a_cycle() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Two actions that each produce what the other consumes.
        let a = net.add(craft(&mut gen, "iron-plate", 1, "iron-gear-wheel"));
        let b = net.add(craft(&mut gen, "iron-gear-wheel", 1, "iron-plate"));
        net.infer_edges();
        net.validate()
            .expect("inference must not introduce a cycle");
        // Exactly one direction survives. Consumers are visited in ascending id
        // order, so `a` is served first and keeps its incoming edge from `b`;
        // the reverse edge would close the cycle and is discarded. The rule is
        // "the first consumer visited keeps its edge", not "the lower id wins".
        assert_eq!(net.preds(a), vec![(b, 0)]);
        assert!(net.preds(b).is_empty());
    }

    #[test]
    fn explicit_links_carry_lag() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let insert = net.add(mine(&mut gen, "iron-ore", 1));
        let remove = net.add(mine(&mut gen, "iron-plate", 1));
        net.link(insert, remove, 192);
        assert_eq!(net.preds(remove), vec![(insert, 192)]);
    }

    #[test]
    fn an_explicit_cycle_fails_validation() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(mine(&mut gen, "iron-ore", 1));
        let b = net.add(mine(&mut gen, "coal", 1));
        net.link(a, b, 0);
        net.link(b, a, 0);
        assert!(net.validate().is_err());
    }

    #[test]
    fn topo_order_respects_edges() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let m = net.add(mine(&mut gen, "iron-plate", 2));
        let c = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.infer_edges();
        assert_eq!(net.topo_order().unwrap(), vec![m, c]);
    }

    #[test]
    fn pinned_actions_survive_the_round_trip() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = mine(&mut gen, "coal", 1);
        action.pinned = Some(BotId(2));
        let id = net.add(action);
        assert_eq!(net.action(id).unwrap().pinned, Some(BotId(2)));
    }
}
