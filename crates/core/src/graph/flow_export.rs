//! The flow graph's wire format -- what `flow.jsonl` holds, one line per
//! keyframe, and what `/api/v1/runs/{id}/flow` and `/api/v1/game/flow`
//! publish over HTTP.
//!
//! A named struct for one rate entry (`FlowExportRate`) rather than the bare
//! `(String, f64)` tuple `FlowGraph::FlowRates` uses internally: neither
//! `utoipa::ToSchema` nor `schemars::JsonSchema` has a blanket implementation
//! for tuples in this workspace, and every other wire-format struct here
//! already uses named fields for the same reason. See
//! `FlowGraph::export`'s doc for how this is built.

use crate::types::Position;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One modelled item rate, in items per second -- the same unit
/// `FlowGraph::FlowRates` already uses everywhere else in this crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct FlowExportRate {
    pub item: String,
    pub per_second: f64,
}

/// One flow-graph node: a machine, chest, drill or lab the flow graph
/// tracks, at the moment [`FlowExport::tick`] names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct FlowExportNode {
    /// Scoped to this one document -- the index the node held in
    /// [`super::flow_graph::FlowGraphInner`] when this export was built.
    /// Stable within the document (every edge's `from`/`to` names a node id
    /// that appears in this same export's `nodes`), not across two exports:
    /// a rebuild of the underlying `StableGraph` can renumber freely. Join
    /// two exports on `position`, never on `id`.
    pub id: u32,
    pub position: Position,
    /// The entity's prototype name, e.g. `stone-furnace`.
    pub name: String,
    /// The entity type, kebab-case (`EntityType`'s `Display`), e.g.
    /// `mining-drill`, `assembling-machine`, `transport-belt`.
    pub kind: String,
    /// What `get_recipe()` last reported for a crafting machine. `None` for
    /// every non-crafting node, and for a crafting machine with none set --
    /// the two are not distinguished here any more than they are on
    /// `FactorioEntity.recipe` itself.
    pub recipe: Option<String>,
    /// The ore a mining drill sits on, when this node is one.
    pub miner_ore: Option<String>,
}

/// One flow-graph edge: an inserter, belt or pipe link between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct FlowExportEdge {
    /// A [`FlowExportNode::id`] in this same export.
    pub from: u32,
    pub to: u32,
    /// One entry for a single-lane connection (a pipe, most belts); two for
    /// a belt whose left and right lane carry different items
    /// (`FlowEdge::Double` in `flow_graph.rs`). Never empty and never more
    /// than two -- a Factorio belt has exactly two lanes.
    pub lanes: Vec<Vec<FlowExportRate>>,
}

/// A point-in-time snapshot of the flow graph -- one line of `flow.jsonl`,
/// or the body of `/api/v1/runs/{id}/flow`'s array and of the live
/// `/api/v1/game/flow`.
///
/// Built by [`super::flow_graph::FlowGraph::export`]. Deliberately not
/// `Default` or otherwise constructible from nothing outside that method:
/// an empty-looking `FlowExport` and one honestly built from an empty graph
/// must not be interchangeable by accident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct FlowExport {
    pub tick: u64,
    pub nodes: Vec<FlowExportNode>,
    pub edges: Vec<FlowExportEdge>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_export_round_trips_through_json() {
        let export = FlowExport {
            tick: 500,
            nodes: vec![FlowExportNode {
                id: 0,
                position: Position::new(1.5, 2.5),
                name: "stone-furnace".to_string(),
                kind: "furnace".to_string(),
                recipe: None,
                miner_ore: None,
            }],
            edges: vec![FlowExportEdge {
                from: 0,
                to: 1,
                lanes: vec![vec![FlowExportRate {
                    item: "iron-plate".to_string(),
                    per_second: 0.3125,
                }]],
            }],
        };
        let json = serde_json::to_string(&export).unwrap();
        let back: FlowExport = serde_json::from_str(&json).unwrap();
        assert_eq!(export, back);
    }
}
