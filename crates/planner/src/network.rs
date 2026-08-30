//! A partially ordered set of actions. No bot appears here; ordering only.

use crate::action::{Action, Condition};
use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ChainId, Ticks};
use factorio_bot_core::petgraph::algo::toposort;
use factorio_bot_core::petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: ActionId,
    pub to: ActionId,
    pub lag: Ticks,
}

/// A partially ordered set of actions. No bot appears anywhere in it: a
/// `ChainId` says which actions must share a runner, never which bot that is.
#[derive(Clone, Debug, Default)]
pub struct ActionNetwork {
    actions: BTreeMap<ActionId, Action>,
    edges: Vec<Edge>,
    /// Which chain each action belongs to, where it belongs to one at all.
    /// A `BTreeMap` because everything that can reach the output is ordered.
    chains: BTreeMap<ActionId, ChainId>,
    /// Chains a caller pinned to a bot by naming it in a `Holder::Bot` goal.
    /// Distinct from `chains`: that says which actions travel together, this
    /// says a caller demanded a particular runner. Still no `BotId` on any
    /// action — the constraint belongs to the chain.
    chain_owner: BTreeMap<ChainId, BotId>,
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

    /// Record that `action` belongs to `chain`.
    ///
    /// The driver stamps every action it emits inside a per-bot subtree, and
    /// the scheduler reads it back to bind the whole chain to one bot.
    pub fn set_chain(&mut self, action: ActionId, chain: ChainId) {
        self.chains.insert(action, chain);
    }

    /// The chain `action` belongs to, or `None` if it is freely assignable.
    pub fn chain_of(&self, action: ActionId) -> Option<ChainId> {
        self.chains.get(&action).copied()
    }

    /// Record that a caller's instruction pins `chain` to `bot`.
    pub fn set_chain_owner(&mut self, chain: ChainId, bot: BotId) {
        self.chain_owner.insert(chain, bot);
    }

    /// The bot a caller pinned `chain` to, or `None` if nobody did.
    pub fn owner_of(&self, chain: ChainId) -> Option<BotId> {
        self.chain_owner.get(&chain).copied()
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
    /// A producer is linked to a consumer when any of the producer's effects
    /// `satisfies` any of the consumer's preconditions, unless the edge would
    /// close a cycle. Candidates are considered in ascending
    /// `(producer, consumer)` order, so the result is deterministic.
    ///
    /// Item matching is by name only — both the produced and the required
    /// counts are ignored — so a consumer needing four plates is ordered after
    /// all ten producers rather than the four it actually consumes, which
    /// serialises work that could have run in parallel. Choosing *which*
    /// producers satisfy a consumer is an assignment problem that belongs with
    /// the methods that build the network, not with inference over a finished
    /// one.
    ///
    /// A pair is skipped when both actions carry a `ChainId`, the chains
    /// differ, and the pairing is inventory-scoped (`Condition::HasItem`):
    /// the scheduler re-derives that order on its own, since its per-bot
    /// feasibility check only offers a consumer to a bot that actually holds
    /// the items — which is the producer's bot — so an inferred item edge
    /// across chains is redundant, not load-bearing. World-state conditions
    /// (`EntityAt`, `Researched`, `PositionFree`, ...) are satisfied by *any*
    /// bot, so nothing re-derives them; those edges must stand regardless of
    /// chain, and two sibling chains can genuinely depend on each other's
    /// output even by item (`shortfall` can credit one chain's simulated
    /// production to another sharing a `chain_actor`), so the exclusion is
    /// deliberately narrower than "different chain, drop it." When either
    /// side carries no chain, nothing says they are separate work, so the
    /// edge stands.
    ///
    /// **Inferred edges enforce order, never location.** Ordering a consumer
    /// after ten producers spread across four bots does not put the items in
    /// the consumer's inventory, because `HasItem { who: Role }` is checked
    /// against the single bot that runs the consumer. Getting the items into
    /// one place is what a `Consolidate` method is for; inference cannot do it
    /// and does not pretend to.
    ///
    /// Only the pairings listed in `Effect::satisfies` are inferred. Effects
    /// that no condition can name — `LoseItem`, `ConsumeResource` — order
    /// nothing, and `ResourceAvailable` has no producing effect at all: ore in
    /// the ground is not made by an action.
    ///
    /// Cost is O(n² · (V+E)): every candidate edge runs a full `validate()`,
    /// which rebuilds the graph and topologically sorts it. Networks here are
    /// expected in the hundreds of actions at most, and inference runs once at
    /// planning time, not per tick. If that stops holding, replace the
    /// validate-and-rollback with a DFS reachability check from `to` to `from`
    /// before pushing the edge.
    pub fn infer_edges(&mut self) {
        let ids: Vec<ActionId> = self.actions.keys().copied().collect();
        for consumer in &ids {
            for producer in &ids {
                if producer == consumer {
                    continue;
                }
                let produces = self.actions[consumer]
                    .pre
                    .iter()
                    .any(|cond| self.actions[producer].eff.iter().any(|e| e.satisfies(cond)));
                if !produces {
                    continue;
                }
                // Only inventory-scoped pairings may be dropped across chains: a
                // HasItem is re-derived by the scheduler's per-bot feasibility
                // check, a world-state condition is not.
                let world_scoped = self.actions[consumer].pre.iter().any(|cond| {
                    !matches!(cond, Condition::HasItem { .. })
                        && self.actions[producer].eff.iter().any(|e| e.satisfies(cond))
                });
                if !world_scoped {
                    if let (Some(p), Some(c)) = (self.chain_of(*producer), self.chain_of(*consumer))
                    {
                        if p != c {
                            continue;
                        }
                    }
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
    use crate::ids::{ActionIdGen, BotId, ChainId};
    use factorio_bot_core::types::{FactorioEntity, Position};

    #[test]
    fn an_edge_survives_a_json_round_trip() {
        use factorio_bot_core::serde_json;
        let edge = Edge {
            from: crate::ids::ActionId(1),
            to: crate::ids::ActionId(2),
            lag: 192,
        };
        let json = serde_json::to_string(&edge).expect("serialises");
        assert_eq!(
            serde_json::from_str::<Edge>(&json).expect("deserialises"),
            edge
        );
    }

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

    #[test]
    fn a_chain_stamp_survives_the_round_trip() {
        use crate::ids::ChainIdGen;
        let mut gen = ActionIdGen::new();
        let mut chains = ChainIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(mine(&mut gen, "coal", 1));
        let b = net.add(mine(&mut gen, "stone", 1));
        let first = chains.next();
        let second = chains.next();
        net.set_chain(a, first);
        net.set_chain(b, second);
        assert_eq!(net.chain_of(a), Some(first));
        assert_eq!(net.chain_of(b), Some(second));
        assert_ne!(first, second, "the generator must not repeat itself");
    }

    #[test]
    fn an_unstamped_action_belongs_to_no_chain() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(mine(&mut gen, "coal", 1));
        assert_eq!(
            net.chain_of(a),
            None,
            "an action outside any per-bot subtree stays freely assignable"
        );
    }

    #[test]
    fn relinking_the_same_pair_keeps_the_larger_lag() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(mine(&mut gen, "iron-ore", 1));
        let b = net.add(mine(&mut gen, "iron-plate", 1));
        net.link(a, b, 5);
        net.link(a, b, 200);
        assert_eq!(net.preds(b), vec![(a, 200)], "the larger lag must win");
        net.link(a, b, 20);
        assert_eq!(
            net.preds(b),
            vec![(a, 200)],
            "a smaller lag must not shrink it"
        );
    }

    #[test]
    fn preds_are_sorted_by_predecessor_id() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(mine(&mut gen, "iron-ore", 1));
        let b = net.add(mine(&mut gen, "coal", 1));
        let c = net.add(mine(&mut gen, "stone", 1));
        let sink = net.add(craft(&mut gen, "iron-ore", 1, "iron-gear-wheel"));
        // Linked in descending order; preds must still come back ascending.
        net.link(c, sink, 0);
        net.link(b, sink, 0);
        net.link(a, sink, 0);
        assert_eq!(net.preds(sink), vec![(a, 0), (b, 0), (c, 0)]);
    }

    /// Place a furnace at `pos`, requiring the tile to be free first.
    fn place(gen: &mut ActionIdGen, pos: Position) -> Action {
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        Action {
            id: gen.next(),
            kind: ActionKind::Place {
                entity: Box::new(furnace.clone()),
            },
            pre: vec![Condition::PositionFree { pos }],
            eff: vec![Effect::CreateEntity(Box::new(furnace))],
            duration: 30,
            pinned: None,
            label: "place stone-furnace".into(),
        }
    }

    /// Insert ore into the furnace standing at `pos`.
    fn insert(gen: &mut ActionIdGen, pos: Position) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Insert {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count: 1,
            },
            pre: vec![Condition::EntityAt {
                pos,
                name: "stone-furnace".into(),
            }],
            eff: vec![],
            duration: 10,
            pinned: None,
            label: "insert iron-ore".into(),
        }
    }

    fn research(gen: &mut ActionIdGen, tech: &str) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Research { tech: tech.into() },
            pre: vec![],
            eff: vec![Effect::Researched(tech.into())],
            duration: 600,
            pinned: None,
            label: format!("research {}", tech),
        }
    }

    /// An action gated on `tech` having been researched.
    fn needs_research(gen: &mut ActionIdGen, tech: &str) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Craft {
                item: "transport-belt".into(),
                count: 1,
            },
            pre: vec![Condition::Researched(tech.into())],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "craft transport-belt".into(),
        }
    }

    #[test]
    fn inference_links_a_placement_to_what_needs_the_entity() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let pos = Position::new(5., 5.);
        let p = net.add(place(&mut gen, pos.clone()));
        let i = net.add(insert(&mut gen, pos));
        net.infer_edges();
        // The spec's Smelt method emits exactly this pair; before the general
        // matcher the insert got no edge back to the place at all.
        assert_eq!(net.preds(i), vec![(p, 0)]);
        assert!(net.preds(p).is_empty());
    }

    #[test]
    fn inference_matches_an_entity_by_tile_not_by_exact_position() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // The furnace sits at the tile's centre; the condition names its corner.
        let p = net.add(place(&mut gen, Position::new(5.5, 5.5)));
        let i = net.add(insert(&mut gen, Position::new(5., 5.)));
        net.infer_edges();
        assert_eq!(net.preds(i), vec![(p, 0)]);
    }

    #[test]
    fn inference_does_not_link_a_placement_of_another_entity() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut other = place(&mut gen, Position::new(5., 5.));
        other.eff = vec![Effect::CreateEntity(Box::new(FactorioEntity {
            name: "wooden-chest".into(),
            position: Position::new(5., 5.),
            ..Default::default()
        }))];
        net.add(other);
        let i = net.add(insert(&mut gen, Position::new(5., 5.)));
        net.infer_edges();
        assert!(net.preds(i).is_empty(), "a chest is not a furnace");
    }

    #[test]
    fn inference_links_a_removal_to_what_needs_the_tile_free() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let pos = Position::new(5., 5.);
        let mut clear = place(&mut gen, Position::new(0., 0.));
        clear.pre = vec![];
        clear.eff = vec![Effect::RemoveEntity { pos: pos.clone() }];
        clear.label = "mine the tree away".into();
        let r = net.add(clear);
        let p = net.add(place(&mut gen, pos));
        net.infer_edges();
        assert_eq!(net.preds(p), vec![(r, 0)]);
    }

    #[test]
    fn inference_links_research_to_what_it_unlocks() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let r = net.add(research(&mut gen, "logistics"));
        let c = net.add(needs_research(&mut gen, "logistics"));
        net.infer_edges();
        assert_eq!(net.preds(c), vec![(r, 0)]);
    }

    #[test]
    fn inference_does_not_link_an_unrelated_technology() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(research(&mut gen, "automation"));
        let c = net.add(needs_research(&mut gen, "logistics"));
        net.infer_edges();
        assert!(net.preds(c).is_empty());
    }

    #[test]
    fn inference_ignores_quantities_and_links_every_producer() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Two producers of one plate each; a consumer that needs four.
        let p1 = net.add(mine(&mut gen, "iron-plate", 1));
        let p2 = net.add(mine(&mut gen, "iron-plate", 1));
        let c = net.add(craft(&mut gen, "iron-plate", 4, "iron-gear-wheel"));
        net.infer_edges();
        assert_eq!(net.preds(c), vec![(p1, 0), (p2, 0)]);
    }

    #[test]
    fn a_chain_owner_round_trips_and_defaults_to_none() {
        let mut net = ActionNetwork::new();
        assert_eq!(net.owner_of(ChainId(0)), None);
        net.set_chain_owner(ChainId(0), BotId(3));
        assert_eq!(net.owner_of(ChainId(0)), Some(BotId(3)));
        assert_eq!(net.owner_of(ChainId(1)), None);
    }

    #[test]
    fn inference_does_not_link_across_chains() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a_mine = net.add(mine(&mut gen, "iron-plate", 2));
        let a_craft = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        let b_mine = net.add(mine(&mut gen, "iron-plate", 2));
        let b_craft = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.set_chain(a_mine, ChainId(0));
        net.set_chain(a_craft, ChainId(0));
        net.set_chain(b_mine, ChainId(1));
        net.set_chain(b_craft, ChainId(1));

        net.infer_edges();

        assert_eq!(net.preds(a_craft), vec![(a_mine, 0)], "chain 0 only");
        assert_eq!(net.preds(b_craft), vec![(b_mine, 0)], "chain 1 only");
    }

    #[test]
    fn inference_still_links_when_either_side_has_no_chain() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let m = net.add(mine(&mut gen, "iron-plate", 2));
        let c = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.set_chain(c, ChainId(0));
        // The producer belongs to no chain, so nothing says these are separate
        // work — the edge must stand.
        net.infer_edges();
        assert_eq!(net.preds(c), vec![(m, 0)]);

        // Symmetric case: the consumer belongs to no chain this time.
        let mut gen2 = ActionIdGen::new();
        let mut net2 = ActionNetwork::new();
        let m2 = net2.add(mine(&mut gen2, "iron-plate", 2));
        let c2 = net2.add(craft(&mut gen2, "iron-plate", 2, "iron-gear-wheel"));
        net2.set_chain(m2, ChainId(0));
        net2.infer_edges();
        assert_eq!(net2.preds(c2), vec![(m2, 0)]);
    }

    #[test]
    fn inference_still_links_a_world_scoped_condition_across_chains() {
        // Two sibling chains reuse the same tile: one places a furnace there,
        // the other needs the furnace present to insert into it. A HasItem
        // pairing may safely drop across chains because the scheduler
        // re-derives it, but EntityAt is satisfied by any bot — nothing
        // re-derives it — so this edge must stand even though the two
        // actions are in different chains. This is the regression this
        // task's original, too-broad exclusion would have produced (see
        // `inference_does_not_link_across_chains` for the HasItem case that
        // *should* drop).
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let pos = Position::new(5., 5.);
        let p = net.add(place(&mut gen, pos.clone()));
        let i = net.add(insert(&mut gen, pos));
        net.set_chain(p, ChainId(0));
        net.set_chain(i, ChainId(1));
        net.infer_edges();
        assert_eq!(net.preds(i), vec![(p, 0)]);
    }
}
