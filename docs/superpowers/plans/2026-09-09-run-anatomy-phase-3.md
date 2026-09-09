# Run Anatomy Phase 3 (Flow) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the run page a "Flow at cursor" tab: a `FlowExport` snapshot of
the flow graph, persisted at every keyframe as `flow.jsonl`, served for an
archived run and for the live world, and drawn as a node/edge diagram that
compares the model's rate against the record's measured rate per node.

**Architecture:** A new `crates/core/src/graph/flow_export.rs` defines the
wire format (`FlowExport`/`FlowExportNode`/`FlowExportEdge`/`FlowExportRate`);
`FlowGraph::export()` (added to `flow_graph.rs`) builds one from
`inner_graph()`, joining each node's recipe off `EntityGraph::entity_by_id`.
The recorder writes it to `flow.jsonl` at the same two call sites that already
write a `map.jsonl` keyframe. Two new server routes serve it — archived
(`/runs/{id}/flow`, tick-windowed) and live (`/game/flow`) — through the
existing OpenAPI/TS seam. A pure `flowJoin.ts` lib joins one keyframe to the
run's machine samples by position at a cursor tick, and `FlowPanel.vue` draws
the result as a third tab beside Map and Video in `RunSidePanel.vue`.

**Tech Stack:** Rust (petgraph `StableGraph`, utoipa, schemars), Vue 3 + SVG,
the existing `runsStore.ts` `Promise.allSettled` enrichment pattern.

**Spec:** `docs/superpowers/specs/2026-09-08-run-anatomy-design.md` (section
1 item 12, section 2.3, section 3, section 4, section 5, section 6 Phase 3
row, section 7 "Determinism of the export").

## Global Constraints

- **Rates are items per second internally, items per minute on screen** —
  match every other band on the page (`ProductionBand` etc. already convert
  `/s` model rates to `/min` for display).
- **Absent is not zero.** No machine sample at a node's position is
  `measuredPerMinute: null`, never `0`. No outgoing edge is
  `modelPerMinute: null`. A `gap` is `null` whenever either side is `null` or
  measured is `0` — a ratio against nothing said is not a number.
- **Colour follows status, not rank.** Node fill comes from the same
  `--color-status-{good,warn,serious,critical,neutral}` tokens the machine
  heatmap uses (`lib/machineTimeline.ts::statusClass`), so a node reads the
  same whether you found it on the heatmap or the flow panel first.
- **One clock.** `flowJoin.ts` takes the cursor tick and reads samples up to
  it, exactly like every other band's cursor-driven lib. It does not invent
  a second notion of "now".
- **Determinism.** `FlowGraph::export()` must not re-sort `FlowRates` by
  value — the graph already yields them name-sorted (`152a3ba0`) and that
  order is preserved end to end, or the export becomes nondeterministic
  again. Read the rates as `inner_graph()` yields them; do not `sort_by` on
  the rate.
- **Deviation from the spec's literal Rust sketch, ruled here so no
  implementer is surprised by it:** the spec's section 2.3 writes
  `FlowExportEdge.lanes: Vec<Vec<(String, f64)>>`. This plan uses
  `Vec<Vec<FlowExportRate>>` instead, where `FlowExportRate { item: String,
  per_second: f64 }` is a named struct. Reason: `utoipa::ToSchema` and
  `schemars::JsonSchema` have no blanket implementation for bare tuples in
  this workspace (no existing `#[derive(ToSchema)]` struct anywhere in the
  crate uses a tuple field — grepped and confirmed empty), and every other
  wire-format type here (`Provenance`, `MapRecord`, `Savepoint`, …) already
  uses named fields for exactly this reason. The data carried is identical;
  only the Rust/TS shape of one rate entry changes from a 2-tuple to
  `{item, per_second}`.
- **`crates/core/src/graph/entity_graph.rs` and `flow_graph.rs`'s own
  traversal (`update`, `condense`, `balance`, …) are NOT touched by this
  plan.** Task 1 only *adds* a new `impl FlowGraph` method and a new
  sibling file. Before starting Task 1, run `git status` on both files —
  this plan was written against master `2f3a5a05..2f3a5a05`-adjacent
  history where both are clean; if a peer session has uncommitted work
  there, coordinate before editing either file.
- **Every cargo command needs `nix develop -c`.** Every commit message goes
  through a quoted heredoc + `git commit -F` (backticks in `-m` are command
  substitution). `rustfmt --edition 2024 <file>`, never `cargo fmt --all` or
  `cargo fmt -p`.
- **`pnpm lint` is part of the seam, not a style pass** — `vitest` does not
  type-check. Run it before calling any frontend task done.
- **The OpenAPI seam**: a Rust field/route change fails
  `cargo test -p factorio-bot-server --features lua --test openapi` until
  `UPDATE_OPENAPI_SNAPSHOT=1` regenerates the snapshot; then it fails
  `openapi.contract.spec.ts` until `types.ts` and the `objectContract` block
  mirror it.

---

### Task 1: `FlowExport` wire format and `FlowGraph::export()`

**Files:**
- Create: `crates/core/src/graph/flow_export.rs`
- Modify: `crates/core/src/graph/mod.rs` (add `pub mod flow_export;`)
- Modify: `crates/core/src/graph/flow_graph.rs` (add `use super::flow_export::{FlowExport, FlowExportEdge, FlowExportNode, FlowExportRate};` and a new `impl FlowGraph` block with `export()`; add tests to the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `FlowGraph::inner_graph(&self) -> RwLockReadGuard<FlowGraphInner>` (existing, calls `ensure_current()` itself), `FlowGraphInner = StableGraph<FlowNode, FlowEdge>` (existing), `FlowNode { position: Position, direction: Direction, entity_name: String, entity_type: EntityType, entity_id: Option<ItemId>, miner_ore: Option<String> }` (existing), `FlowEdge::{Single(FlowRates), Double(FlowRates, FlowRates)}` (existing), `FlowRates = Vec<(String, f64)>` (existing), `EntityGraph::entity_by_id(&self, id: ItemId) -> Option<FactorioEntity>` (existing, `FactorioEntity.recipe: Option<String>`).
- Produces: `pub struct FlowExport { pub tick: u64, pub nodes: Vec<FlowExportNode>, pub edges: Vec<FlowExportEdge> }`, `pub struct FlowExportNode { pub id: u32, pub position: Position, pub name: String, pub kind: String, pub recipe: Option<String>, pub miner_ore: Option<String> }`, `pub struct FlowExportEdge { pub from: u32, pub to: u32, pub lanes: Vec<Vec<FlowExportRate>> }`, `pub struct FlowExportRate { pub item: String, pub per_second: f64 }`, and `impl FlowGraph { pub fn export(&self, tick: u64) -> FlowExport }`. Later tasks (2, 4, 5) consume `FlowExport` and `FlowGraph::export`.

- [ ] **Step 1: Write the failing tests**

In `crates/core/src/graph/flow_graph.rs`'s existing `#[cfg(test)] mod tests` (it already has `use crate::test_utils::entity_graph_from;` and a `drill_and_two_belts() -> Arc<EntityGraph>` fixture — reuse it), add:

```rust
    #[test]
    fn export_carries_every_node_and_edge_with_a_stable_kind_and_position() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph);
        let export = flow_graph.export(12345);

        assert_eq!(export.tick, 12345);
        assert_eq!(export.nodes.len(), 3, "the drill and two belts");
        let drill = export
            .nodes
            .iter()
            .find(|n| n.kind == "mining-drill")
            .expect("the drill is a node");
        assert_eq!(drill.position, Position::new(0.5, -1.5));
        assert_eq!(drill.miner_ore.as_deref(), Some("iron-ore"));
        assert_eq!(drill.recipe, None, "a drill has no recipe, a furnace does");

        assert_eq!(export.edges.len(), 2, "drill->belt, belt->belt");
        for edge in &export.edges {
            assert!(
                export.nodes.iter().any(|n| n.id == edge.from),
                "every edge names a node id that exists in this same export"
            );
            assert!(export.nodes.iter().any(|n| n.id == edge.to));
        }
    }

    #[test]
    fn export_reads_a_machines_recipe_off_the_entity_graph_by_position() {
        let entity_graph = Arc::new(
            entity_graph_from(vec![FactorioEntity::new_assembling_machine(
                &Position::new(5.5, 5.5),
                Direction::North,
            )])
            .unwrap(),
        );
        entity_graph.set_recipe(&Position::new(5.5, 5.5), "electronic-circuit");
        let flow_graph = FlowGraph::new(entity_graph);
        let export = flow_graph.export(0);

        let machine = export
            .nodes
            .iter()
            .find(|n| n.kind == "assembling-machine")
            .expect("the assembler is a node");
        assert_eq!(machine.recipe.as_deref(), Some("electronic-circuit"));
    }

    #[test]
    fn export_preserves_the_name_sorted_rate_order_the_graph_already_yields() {
        let entity_graph = drill_and_two_belts();
        let flow_graph = FlowGraph::new(entity_graph);
        let export = flow_graph.export(0);

        for edge in &export.edges {
            for lane in &edge.lanes {
                let names: Vec<&str> = lane.iter().map(|r| r.item.as_str()).collect();
                let mut sorted = names.clone();
                sorted.sort_unstable();
                assert_eq!(
                    names, sorted,
                    "export must not re-sort what inner_graph() already yields name-sorted"
                );
            }
        }
    }

    #[test]
    fn a_double_edge_exports_as_two_lanes_a_single_edge_as_one() {
        let single = FlowEdge::Single(vec![("iron-ore".to_string(), 0.5)]);
        let double = FlowEdge::Double(
            vec![("iron-ore".to_string(), 0.3)],
            vec![("copper-ore".to_string(), 0.2)],
        );
        assert_eq!(export_lanes_for_test(&single).len(), 1);
        assert_eq!(export_lanes_for_test(&double).len(), 2);
    }
```

Add one more tiny test-only helper right above those tests, calling the
private `export_rates` function Step 3 introduces (so the last test can
exercise the lane-shaping logic in isolation without building a whole graph):

```rust
    #[cfg(test)]
    fn export_lanes_for_test(edge: &FlowEdge) -> Vec<Vec<FlowExportRate>> {
        match edge {
            FlowEdge::Single(v) => vec![super::export_rates(v)],
            FlowEdge::Double(l, r) => vec![super::export_rates(l), super::export_rates(r)],
        }
    }
```

You will also need `FactorioEntity::new_assembling_machine` — check it exists
(`crates/core/src/types.rs`, search `impl FactorioEntity`); every other
`new_*` constructor used above (`new_transport_belt`, `new_electric_mining_drill`,
`new_resource`) already exists and is used elsewhere in this same test module.

In `crates/core/src/graph/flow_export.rs` (the new file), add its own unit
tests at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Position;

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
```

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo test -p factorio-bot-core --lib graph::flow_graph::tests::export -- --list
nix develop -c cargo build -p factorio-bot-core 2>&1 | tail -40
```

Expected: FAIL to compile — `flow_export` module and `FlowGraph::export` do
not exist yet.

- [ ] **Step 3: Implement**

`crates/core/src/graph/flow_export.rs`:

```rust
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
```

`crates/core/src/graph/mod.rs`:

```rust
pub mod enclosure;
pub mod entity_graph;
pub mod flow_export;
pub mod flow_graph;
pub mod route;
```

`crates/core/src/graph/flow_graph.rs`: add the import near the top (beside
the existing `use crate::graph::entity_graph::{...}` line):

```rust
use crate::graph::flow_export::{FlowExport, FlowExportEdge, FlowExportNode, FlowExportRate};
```

Then add a new `impl FlowGraph` block (anywhere after the existing one that
defines `inner_graph`/`condense`/etc. — right after `condense` reads
naturally, since both walk `inner_graph()`):

```rust
impl FlowGraph {
    /// A point-in-time snapshot of this graph, in the wire format
    /// `flow.jsonl` and `/api/v1/runs/{id}/flow`/`/api/v1/game/flow`
    /// publish.
    ///
    /// `tick` is stamped by the caller rather than read here: this method
    /// has no RCON connection of its own, and the two existing callers (the
    /// recorder's keyframe writers, `crates/server`'s live `/game/flow`
    /// handler) each already have a tick from their own source (the
    /// recorder's `not_before`, the live instance's `rcon.last_tick()`) and
    /// must not disagree with it by asking a third place.
    ///
    /// Calls [`Self::inner_graph`], which calls [`Self::ensure_current`], so
    /// the export always reflects the entity graph's current generation --
    /// the same freshness guarantee every other public reader on this type
    /// has.
    ///
    /// Rates are read in the order `inner_graph()` yields them -- name-sorted
    /// since `152a3ba0` -- and never re-sorted here. See the Global
    /// Constraints in the Run Anatomy Phase 3 plan for why that matters.
    pub fn export(&self, tick: u64) -> FlowExport {
        let graph = self.inner_graph();
        let mut nodes = Vec::with_capacity(graph.node_count());
        for index in graph.node_indices() {
            let node = graph
                .node_weight(index)
                .expect("node_indices only yields indices with a weight");
            let recipe = node
                .entity_id
                .and_then(|id| self.entity_graph.entity_by_id(id))
                .and_then(|entity| entity.recipe);
            nodes.push(FlowExportNode {
                id: index.index() as u32,
                position: node.position.clone(),
                name: node.entity_name.clone(),
                kind: node.entity_type.to_string(),
                recipe,
                miner_ore: node.miner_ore.clone(),
            });
        }
        let mut edges = Vec::with_capacity(graph.edge_count());
        for edge in graph.edge_references() {
            let lanes = match edge.weight() {
                FlowEdge::Single(rates) => vec![export_rates(rates)],
                FlowEdge::Double(left, right) => {
                    vec![export_rates(left), export_rates(right)]
                }
            };
            edges.push(FlowExportEdge {
                from: edge.source().index() as u32,
                to: edge.target().index() as u32,
                lanes,
            });
        }
        FlowExport { tick, nodes, edges }
    }
}

/// One [`FlowRates`] vector, converted to the wire format's named-field
/// entries in the same order.
fn export_rates(rates: &FlowRates) -> Vec<FlowExportRate> {
    rates
        .iter()
        .map(|(item, per_second)| FlowExportRate {
            item: item.clone(),
            per_second: *per_second,
        })
        .collect()
}
```

If `FactorioEntity::new_assembling_machine` does not already exist (check
`crates/core/src/types.rs` for `impl FactorioEntity`), add it following the
exact pattern of the neighbouring `new_transport_belt`/`new_electric_mining_drill`
constructors in that same `impl` block — same field defaults, only the
`name`/`entity_type`/`bounding_box` differ (`"assembling-machine-1"`,
`EntityType::AssemblingMachine`, a 3x3 box).

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo test -p factorio-bot-core --lib graph::flow_graph::tests::export
nix develop -c cargo test -p factorio-bot-core --lib graph::flow_export
nix develop -c cargo clippy -p factorio-bot-core --all-features --all-targets -- --deny warnings
```

Expected: PASS, no clippy warnings.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(core): a flow graph can say what it saw, not only what a live reader asks it

FlowExport/FlowExportNode/FlowExportEdge/FlowExportRate is the flow graph's
wire format, and FlowGraph::export() builds one from inner_graph() -- the
first thing here that turns the graph into a document instead of answering
one reader's question about one position.
EOF
git commit -F /tmp/msg -- crates/core/src/graph/flow_export.rs crates/core/src/graph/mod.rs crates/core/src/graph/flow_graph.rs crates/core/src/types.rs
```

(Drop `crates/core/src/types.rs` from the path list if
`new_assembling_machine` already existed and nothing there changed.)

---

### Task 2: `flow.jsonl` — the recorder's writer and reader

**Files:**
- Create: `crates/core/src/record/flow.rs`
- Modify: `crates/core/src/record/mod.rs` (add `pub mod flow;`, a `flow: File` field on `RunRecorder`, open it in `start()`, add `record_flow()`)

**Interfaces:**
- Consumes: `FlowExport` (Task 1), the existing `RunRecorder` struct and its `start()`/field-init pattern (`crates/core/src/record/mod.rs`), the existing `read_map`/`ReadMap` pattern in `crates/core/src/record/map.rs` as the template to mirror.
- Produces: `pub struct ReadFlow { pub records: Vec<FlowExport>, pub skipped: usize }`, `pub fn read_flow(path: &Path) -> io::Result<ReadFlow>` (both in `crates/core/src/record/flow.rs`), and `impl RunRecorder { pub fn record_flow(&mut self, export: &FlowExport) -> io::Result<()> }`. Task 3 calls `record_flow`; Task 4's server route calls `read_flow`.

- [ ] **Step 1: Write the failing tests**

`crates/core/src/record/flow.rs` (new file), bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::flow_export::{FlowExport, FlowExportEdge, FlowExportNode, FlowExportRate};
    use crate::types::Position;
    use std::io::Write;

    fn write_lines(dir: &std::path::Path, lines: &[&str]) -> std::path::PathBuf {
        let path = dir.join("flow.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path
    }

    fn sample_export(tick: u64) -> FlowExport {
        FlowExport {
            tick,
            nodes: vec![FlowExportNode {
                id: 0,
                position: Position::new(0.5, 0.5),
                name: "stone-furnace".to_string(),
                kind: "furnace".to_string(),
                recipe: None,
                miner_ore: None,
            }],
            edges: vec![FlowExportEdge {
                from: 0,
                to: 0,
                lanes: vec![vec![FlowExportRate {
                    item: "iron-plate".to_string(),
                    per_second: 0.3125,
                }]],
            }],
        }
    }

    #[test]
    fn read_flow_reads_every_record() {
        let tmp = tempfile::tempdir().unwrap();
        let export = sample_export(100);
        let line = serde_json::to_string(&export).unwrap();
        let path = write_lines(tmp.path(), &[&line]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.skipped, 0);
        assert_eq!(read.records, vec![export]);
    }

    #[test]
    fn read_flow_skips_a_truncated_final_line() {
        let tmp = tempfile::tempdir().unwrap();
        let good = serde_json::to_string(&sample_export(100)).unwrap();
        let path = write_lines(tmp.path(), &[&good, r#"{"tick":200,"nodes":[{"i"#]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn read_flow_skips_blank_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let good = serde_json::to_string(&sample_export(100)).unwrap();
        let path = write_lines(tmp.path(), &[&good, "", "   "]);

        let read = read_flow(&path).unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.skipped, 0, "a blank line is not a parse failure");
    }
}
```

In `crates/core/src/record/mod.rs`'s `mod tests` (or a new `mod flow_tests`
placed beside the existing `mod map_tests` at the bottom of the file — follow
whichever the file already does for `map`):

```rust
    #[test]
    fn record_flow_appends_one_line_per_call_and_flushes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut recorder = RunRecorder::start(tmp.path(), "run-test-flow").unwrap();
        let export = crate::graph::flow_export::FlowExport {
            tick: 100,
            nodes: vec![],
            edges: vec![],
        };
        recorder.record_flow(&export).unwrap();
        recorder.record_flow(&crate::graph::flow_export::FlowExport {
            tick: 200,
            ..export
        }).unwrap();

        let read = crate::record::flow::read_flow(&recorder.dir().join("flow.jsonl")).unwrap();
        assert_eq!(read.records.len(), 2);
        assert_eq!(read.records[0].tick, 100);
        assert_eq!(read.records[1].tick, 200);
    }
```

(Adjust `crate::graph::flow_export::FlowExport { tick: 200, ..export }` if
`FlowExport` does not derive `Clone`+struct-update-friendly fields — it does,
per Task 1; `..export` moves the first `export`, so bind two separate values
instead if the borrow checker objects: `let a = FlowExport{ tick: 100, nodes: vec![], edges: vec![] }; let b = FlowExport{ tick: 200, nodes: vec![], edges: vec![] };`.)

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo build -p factorio-bot-core 2>&1 | tail -40
```

Expected: FAIL to compile — `record::flow` module, `read_flow`, and
`RunRecorder::record_flow` do not exist yet.

- [ ] **Step 3: Implement**

`crates/core/src/record/flow.rs`:

```rust
//! `flow.jsonl`: the flow graph's own keyframes.
//!
//! One [`crate::graph::flow_export::FlowExport`] per line, written at the
//! same two moments `map.jsonl` gets a `MapKind::Keyframe` line -- see
//! `RunRecorder::record_flow`'s callers in `crates/scripting_lua`. Unlike
//! `map.jsonl`, every line here is a full snapshot, not a delta: nothing
//! reconstructs a flow graph from a base plus edits, so a reader wanting the
//! graph nearest some tick picks the latest line at or before it.

use crate::graph::flow_export::FlowExport;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReadFlow {
    pub records: Vec<FlowExport>,
    /// Lines that did not parse -- in practice the truncated final line of a
    /// crashed run. Reported rather than swallowed, exactly like
    /// [`super::map::read_map`] and [`super::read_events`].
    pub skipped: usize,
}

pub fn read_flow(path: &Path) -> io::Result<ReadFlow> {
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<FlowExport>(&line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    Ok(ReadFlow { records, skipped })
}
```

`crates/core/src/record/mod.rs`:

1. Add `pub mod flow;` to the module list (beside `pub mod exposure;` etc.,
   keep alphabetical: after `pub mod exposure;`, before `pub mod lanes;`).
2. Add a field to `RunRecorder`, next to the existing `map: File` field:
   ```rust
       flow: File,
   ```
3. In `RunRecorder::start`, open it beside `map`:
   ```rust
       let map = File::create(dir.join("map.jsonl"))?;
       let flow = File::create(dir.join("flow.jsonl"))?;
   ```
   and add `flow,` to the struct literal beside the existing `map,` line.
4. Add the writer method, right after `record_map`:
   ```rust
       /// Appends one line to `flow.jsonl` and flushes it, for the same
       /// reason [`Self::record_map`] does: a crashed run must leave a
       /// readable flow history, not just a readable event log.
       ///
       /// Unlike `record_map`, this does no bookkeeping beyond the write --
       /// there is no equivalent of `placed_bounds` to maintain, because a
       /// flow keyframe is a full snapshot rather than a delta a later
       /// reconstruction depends on.
       pub fn record_flow(&mut self, export: &crate::graph::flow_export::FlowExport) -> io::Result<()> {
           let mut line = serde_json::to_string(export).map_err(io::Error::other)?;
           line.push('\n');
           self.flow.write_all(line.as_bytes())?;
           self.flow.flush()
       }
   ```

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo test -p factorio-bot-core --lib record::flow
nix develop -c cargo test -p factorio-bot-core --lib record::tests::record_flow_appends_one_line_per_call_and_flushes
nix develop -c cargo clippy -p factorio-bot-core --all-features --all-targets -- --deny warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(core): flow.jsonl -- the flow graph's own keyframes, alongside map.jsonl

record_flow() writes one FlowExport per line, read back by read_flow() with
the same skip-and-count-what-did-not-parse discipline map.jsonl already has.
No callers yet; the next commit wires it into the two moments a run already
writes a map.jsonl keyframe.
EOF
git commit -F /tmp/msg -- crates/core/src/record/flow.rs crates/core/src/record/mod.rs
```

---

### Task 3: Write a flow keyframe wherever a map keyframe is written

**Files:**
- Modify: `crates/scripting_lua/src/globals/record.rs` (two call sites: the opening keyframe inside `record.start()`, and `record.keyframe()`)

**Interfaces:**
- Consumes: `RunRecorder::record_flow` (Task 2), `FlowGraph::export` (Task 1, reached as `world.flow_graph.export(tick)` — `FactorioSurface.flow_graph: Arc<FlowGraph>` already exists on the `world` value both call sites already hold).
- Produces: nothing new for later tasks — this is the last piece that makes `flow.jsonl` actually appear on disk for a live run. Tasks 4/5 read files that this task is what populates.

- [ ] **Step 1: Write the failing test**

There is already a Phase 2 precedent test proving a *different* recorder
method runs with no sink present
(`a_run_with_a_recorder_and_no_sink_still_writes_its_replay`, added when
`emit_replay` was fixed in `crates/scripting_lua/src/globals/goal/run.rs`).
This task's own test lives beside the existing `record.keyframe()` tests in
`crates/scripting_lua/src/globals/record.rs`'s test module. Find that
module's existing keyframe test (search for `fn a_keyframe` or
`record.keyframe` in the test module) and add a new test next to it that
starts a recorder, places one entity through the same test harness the
existing keyframe test uses, calls `record.keyframe()` via Lua, and then
asserts `flow.jsonl` has exactly one line naming that entity:

```rust
    #[tokio::test]
    async fn a_keyframe_also_writes_a_flow_line_naming_what_it_saw() {
        // Follow the exact setup the neighbouring keyframe test in this file
        // uses (same stub_rcon/world/scripts_root construction, same
        // `record.start()` + one placement + `record.keyframe()` Lua call).
        // The only new assertion is on flow.jsonl.
        let harness = /* same harness constructor the neighbouring test calls */;
        harness.eval("record.start()").await.unwrap();
        // ... place one entity, exactly as the neighbouring test does ...
        let wrote: bool = harness.eval("return record.keyframe()").await.unwrap();
        assert!(wrote);

        let run_dir = harness.latest_run_dir(); // however the neighbouring test locates it
        let flow = factorio_bot_core::record::flow::read_flow(&run_dir.join("flow.jsonl")).unwrap();
        assert!(!flow.records.is_empty(), "record.keyframe() must also write a flow line");
        assert!(
            flow.records.last().unwrap().nodes.iter().any(|n| n.name == "stone-furnace"),
            "the flow line must see the entity this test placed"
        );
    }
```

**This step is deliberately written as a sketch, not exact code** — unlike
every other step in this plan, because the exact harness (`stub_rcon`,
`FactorioSurface` construction, how a test drives `record.start()` /
`record.keyframe()` through the Lua interpreter, how it locates the run
directory) already exists in this file's test module for the neighbouring
keyframe test and must be copied from there verbatim, not re-invented. The
implementer's first sub-step is: `grep -n "fn.*keyframe" crates/scripting_lua/src/globals/record.rs`
inside the `#[cfg(test)] mod tests` block, read that test in full, and use
its exact setup with one added assertion block as above.

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo test -p factorio-bot-scripting-lua --lib a_keyframe_also_writes_a_flow_line
```

Expected: FAIL — `flow.jsonl` does not exist yet (the two call sites do not
write it).

- [ ] **Step 3: Implement**

Add the import near the top of `crates/scripting_lua/src/globals/record.rs`,
beside the existing `use factorio_bot_core::record::{...};` block:

```rust
use factorio_bot_core::graph::flow_export::FlowExport;
```

**Call site 1 — the opening keyframe in `record.start()`.** Find the block
(search `A keyframe at the run's own opening`) that ends with:

```rust
                    if let Some(bounds) =
                        bounds_around(players.into_iter().map(|p| p.position), 16.0)
                    {
                        let (game, model, divergence) =
                            keyframe_snapshot(&rcon, &world, &bounds).await?;
                        recorder
                            .record_map(MapRecord {
                                tick: opened_at,
                                kind: MapKind::Keyframe {
                                    bounds,
                                    game,
                                    model,
                                    divergence,
                                },
                            })
                            .map_err(record_error)?;
                    }
```

Add one line right after the `record_map` call, still inside the `if let
Some(bounds) = ...` block (a flow keyframe at the run's opening is exactly
as meaningful as a map one, and pairing them 1:1 means a reader never finds
a map keyframe with no matching flow keyframe or vice versa):

```rust
                    if let Some(bounds) =
                        bounds_around(players.into_iter().map(|p| p.position), 16.0)
                    {
                        let (game, model, divergence) =
                            keyframe_snapshot(&rcon, &world, &bounds).await?;
                        recorder
                            .record_map(MapRecord {
                                tick: opened_at,
                                kind: MapKind::Keyframe {
                                    bounds,
                                    game,
                                    model,
                                    divergence,
                                },
                            })
                            .map_err(record_error)?;
                        recorder
                            .record_flow(&world.flow_graph.export(opened_at))
                            .map_err(record_error)?;
                    }
```

**Call site 2 — `record.keyframe()`.** Find:

```rust
                    let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
                    recorder
                        .record_map(MapRecord {
                            tick,
                            kind: MapKind::Keyframe {
                                bounds,
                                game,
                                model,
                                divergence,
                            },
                        })
                        .map_err(record_error)?;
                    Ok(true)
```

and add the flow write right after `record_map`, before `Ok(true)`:

```rust
                    let tick = recorder.not_before(rcon.last_tick().unwrap_or(0));
                    recorder
                        .record_map(MapRecord {
                            tick,
                            kind: MapKind::Keyframe {
                                bounds,
                                game,
                                model,
                                divergence,
                            },
                        })
                        .map_err(record_error)?;
                    recorder
                        .record_flow(&world.flow_graph.export(tick))
                        .map_err(record_error)?;
                    Ok(true)
```

`world` is already captured and cloned into both closures (it is what
`keyframe_snapshot(&rcon, &world, &bounds)` is called with two lines above
each site), so no new capture is needed — `world.flow_graph` is a field of
`FactorioSurface`, already `Arc<FlowGraph>`.

Also update `record.keyframe()`'s Lua doc string (the `__doc_entry_keyframe`
string a few lines above the binding, starting `--- writes a keyframe:
what the game and our belief about it agree on`) to add one line noting it
also writes a `flow.jsonl` line — follow the existing prose style (see how
the samples-ingestion side effect is documented in the same doc string as a
model: `"Also ingests whatever the mod has written to samples.jsonl..."`).
Add a matching sentence: `"Also exports the flow graph's current state to
flow.jsonl, for the same reason: a reader of this run's flow history should
never find a gap where a milestone boundary should be."`

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo test -p factorio-bot-scripting-lua --lib a_keyframe_also_writes_a_flow_line
nix develop -c cargo test -p factorio-bot-scripting-lua --lib record::
nix develop -c cargo clippy -p factorio-bot-scripting-lua --all-features --all-targets -- --deny warnings
```

Expected: PASS, including every pre-existing `record::` test (this task
must not change `map.jsonl`'s content or ordering).

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(scripting_lua): a run's flow.jsonl fills at the same two moments its map.jsonl does

record.start()'s opening keyframe and record.keyframe() both now export the
flow graph and write it to flow.jsonl, paired 1:1 with the map.jsonl
keyframe each already writes -- a reader of one never finds a gap where the
other should be.
EOF
git commit -F /tmp/msg -- crates/scripting_lua/src/globals/record.rs
```

---

### Task 4: `GET /api/v1/runs/{id}/flow`, through the OpenAPI seam

**Files:**
- Modify: `crates/server/src/runs.rs` (new `RunFlowResponse` struct, new `get_run_flow` handler, register the route)
- Modify: `app/src/api/types.ts` (mirror `FlowExportRate`/`FlowExportNode`/`FlowExportEdge`/`FlowExport`/`RunFlowResponse`)
- Modify: `app/src/api/client.ts` (new `getRunFlow`)
- Modify: `app/src/api/openapi.contract.spec.ts` (pin the five new schemas)

**Interfaces:**
- Consumes: `factorio_bot_core::graph::flow_export::FlowExport` and `factorio_bot_core::record::flow::read_flow` (Tasks 1–2), the existing `TickWindow` query-param struct and `run_dir`/`runs_root` helpers already in `crates/server/src/runs.rs` (see `get_run_map` for the exact pattern to mirror).
- Produces: `getRunFlow(id: string, opts?: {from?: number; to?: number}): Promise<RunFlowResponse>` in `app/src/api/client.ts`, and the TS types `FlowExport`/`FlowExportNode`/`FlowExportEdge`/`FlowExportRate`/`RunFlowResponse`. Task 6 (`flowJoin.ts`) and Task 8 (`runsStore.ts`) consume both.

- [ ] **Step 1: Write the failing tests**

Rust, in `crates/server/src/runs.rs`'s test module (find the existing tests
for `get_run_map` and `get_run_replay` and place this beside them, reusing
whatever test-run-directory builder they already use — likely a helper that
creates a temp `runs/<id>/` directory):

```rust
    #[tokio::test]
    async fn get_run_flow_reads_flow_jsonl_and_reports_what_did_not_parse() {
        let (state, dir) = test_state_with_one_run("run-flow-1").await; // reuse whatever
                                                                          // helper get_run_map's
                                                                          // test uses to build a
                                                                          // run dir under `state`
        std::fs::write(
            dir.join("flow.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&factorio_bot_core::graph::flow_export::FlowExport {
                    tick: 100,
                    nodes: vec![],
                    edges: vec![],
                })
                .unwrap(),
                "not json"
            ),
        )
        .unwrap();

        let Json(response) = get_run_flow(
            State(state),
            Path("run-flow-1".to_string()),
            Query(TickWindow::default()),
        )
        .await
        .unwrap();

        assert_eq!(response.flow.len(), 1);
        assert_eq!(response.flow[0].tick, 100);
        assert_eq!(response.skipped, 1);
    }

    #[tokio::test]
    async fn get_run_flow_answers_an_empty_list_not_a_404_when_flow_jsonl_is_absent() {
        let (state, _dir) = test_state_with_one_run("run-flow-2").await;

        let Json(response) = get_run_flow(
            State(state),
            Path("run-flow-2".to_string()),
            Query(TickWindow::default()),
        )
        .await
        .unwrap();

        assert!(response.flow.is_empty());
        assert_eq!(response.skipped, 0);
    }
```

(Replace `test_state_with_one_run` with whatever this file's existing
`get_run_map`/`get_run_replay` tests actually call — read them first; do not
invent a second test-fixture helper if one already exists.)

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo build -p factorio-bot-server --features lua 2>&1 | tail -40
```

Expected: FAIL to compile — `RunFlowResponse` and `get_run_flow` do not exist
yet.

- [ ] **Step 3: Implement**

In `crates/server/src/runs.rs`, add the import near the existing
`factorio_bot_core::record::map::{...}` import:

```rust
use factorio_bot_core::graph::flow_export::FlowExport;
```

Add the response struct beside `RunMapResponse`:

```rust
/// `GET /api/v1/runs/{id}/flow` response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RunFlowResponse {
    pub flow: Vec<FlowExport>,
    /// Lines that did not parse. Reported rather than swallowed, matching
    /// `RunMapResponse.skipped`.
    pub skipped: usize,
}
```

Add the handler, right after `get_run_map`:

```rust
/// One archived run's flow-graph keyframes -- one per milestone boundary,
/// plus its opening one. Each entry is a full snapshot (unlike `map.jsonl`,
/// nothing here reconstructs from a base plus deltas), so a caller wanting
/// the graph nearest a cursor tick picks the latest entry at or before it.
///
/// A run recorded before this feature existed, or one that never reached a
/// keyframe, answers an empty list, not a 404 -- matching `get_run_map`: an
/// absent capability and a present-but-empty one read the same here, and the
/// page already treats an empty flow list as "nothing to draw yet".
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/flow",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id"), TickWindow),
    responses(
        (status = 200, body = RunFlowResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(window): Query<TickWindow>,
) -> Result<Json<RunFlowResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let path = dir.join("flow.jsonl");
    if !path.exists() {
        return Ok(Json(RunFlowResponse {
            flow: Vec::new(),
            skipped: 0,
        }));
    }
    let read = factorio_bot_core::record::flow::read_flow(&path)
        .map_err(|err| ErrorResponse::internal(format!("failed to read flow: {err}")))?;
    let flow = read
        .records
        .into_iter()
        .filter(|r| window.contains(r.tick))
        .collect();
    Ok(Json(RunFlowResponse {
        flow,
        skipped: read.skipped,
    }))
}
```

Register the route in the router-building function, beside the existing
`.routes(routes!(get_run_map))` line:

```rust
        .routes(routes!(get_run_flow))
```

Regenerate the OpenAPI snapshot and check the diff is exactly the five new
schemas plus the one new route:

```bash
UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
git diff --no-ext-diff app/src/api/openapi.snapshot.json | head -100
```

**TypeScript.** In `app/src/api/types.ts`, add (near `RunMapResponse`,
`MapRecord`):

```ts
/** A modelled flow rate for one item, items per second. */
export interface FlowExportRate {
    item: string;
    per_second: number;
}

/**
 * One flow-graph node: a machine, chest, drill or lab the flow graph
 * tracks.
 *
 * `id` is scoped to the one `FlowExport` it came from -- stable within that
 * document, meaningless across two. Join two exports on `position`, never
 * on `id`.
 */
export interface FlowExportNode {
    id: number;
    position: Position;
    name: string;
    kind: string;
    recipe: string | null;
    miner_ore: string | null;
}

/**
 * One flow-graph edge. `lanes` has one entry for a single-lane connection
 * (a pipe, most belts) and two for a belt whose left and right lane carry
 * different items.
 */
export interface FlowExportEdge {
    from: number;
    to: number;
    lanes: FlowExportRate[][];
}

/** A point-in-time snapshot of the flow graph -- one line of `flow.jsonl`. */
export interface FlowExport {
    tick: number;
    nodes: FlowExportNode[];
    edges: FlowExportEdge[];
}

/** `GET /api/v1/runs/{id}/flow` response. */
export interface RunFlowResponse {
    flow: FlowExport[];
    /** Lines that did not parse. Reported rather than swallowed, matching
     *  `RunMapResponse.skipped`. */
    skipped: number;
}
```

In `app/src/api/client.ts`, add beside `getRunMap`:

```ts
/**
 * The flow graph's own keyframes -- one per milestone boundary, plus the
 * run's opening one. Each entry is a full snapshot, not a delta.
 */
export function getRunFlow(id: string, opts?: {from?: number; to?: number}): Promise<RunFlowResponse> {
    return request<RunFlowResponse>(`/api/v1/runs/${encodeURIComponent(id)}/flow`, {
        query: {from: opts?.from, to: opts?.to}
    });
}
```

(Add `FlowExportEdge, FlowExportNode, FlowExportRate, FlowExport,
RunFlowResponse` to whatever import already pulls `RunMapResponse` from
`./types` at the top of `client.ts` — only the ones actually referenced by
name in `client.ts` need importing there; the rest are used only in
`types.ts` and `openapi.contract.spec.ts`.)

In `app/src/api/openapi.contract.spec.ts`, add near the `RunMapResponse`
block, importing the four new type names alongside the existing
`RunMapResponse` import at the top of the file:

```ts
    FlowExportRate: objectContract<FlowExportRate>({
        item: {required: true, type: 'string'},
        per_second: {required: true, type: 'number'}
    }),
    FlowExportNode: objectContract<FlowExportNode>({
        id: {required: true, type: 'integer'},
        position: {required: true, ref: 'Position'},
        name: {required: true, type: 'string'},
        kind: {required: true, type: 'string'},
        recipe: {required: true, type: 'string', nullable: true},
        miner_ore: {required: true, type: 'string', nullable: true}
    }),
    FlowExportEdge: objectContract<FlowExportEdge>({
        from: {required: true, type: 'integer'},
        to: {required: true, type: 'integer'},
        // `Vec<Vec<FlowExportRate>>` -- this checker's `arrayOf` has no
        // vocabulary for a nested array of a named schema, so this is
        // declared the same way `RunSavepointsResponse.missing_zip`
        // (`Vec<u32>`) is: present and shaped as an array, not fully typed.
        lanes: {required: true, type: 'array'}
    }),
    FlowExport: objectContract<FlowExport>({
        tick: {required: true, type: 'integer'},
        nodes: {required: true, arrayOf: 'FlowExportNode'},
        edges: {required: true, arrayOf: 'FlowExportEdge'}
    }),
    RunFlowResponse: objectContract<RunFlowResponse>({
        flow: {required: true, arrayOf: 'FlowExport'},
        skipped: {required: true, type: 'integer'}
    }),
```

Run the contract test and `pnpm lint`; fix whichever `required`/`nullable`
combination it reports wrong (the utoipa-generated snapshot is the source
of truth — read `app/src/api/openapi.snapshot.json`'s `FlowExportNode` etc.
schemas if the contract test disagrees with what is written above).

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
nix develop -c cargo test -p factorio-bot-server --lib runs::
nix develop -c cargo clippy -p factorio-bot-server --all-features --all-targets -- --deny warnings
cd app && pnpm lint && pnpm vitest run src/api/openapi.contract.spec.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(server): GET /runs/{id}/flow serves the flow graph's own keyframes

Mirrors get_run_map's tick-windowed read and its "absent file is an empty
list, not a 404" rule. Regenerated the OpenAPI snapshot and mirrored the
five new schemas into types.ts and the contract spec.
EOF
git commit -F /tmp/msg -- crates/server/src/runs.rs app/src/api/types.ts app/src/api/client.ts app/src/api/openapi.contract.spec.ts app/src/api/openapi.snapshot.json
```

---

### Task 5: `GET /api/v1/game/flow` — the live world's flow graph

**Files:**
- Modify: `crates/server/src/game/query.rs` (new `flow` handler)
- Modify: `crates/server/src/game/mod.rs` (register the route)

**Interfaces:**
- Consumes: `FlowGraph::export` (Task 1), the existing `require_surface` helper and `ApiResult<T>` alias (`crates/server/src/game/mod.rs`, `crates/server/src/error.rs`), the `entity_prototypes` handler in `crates/server/src/game/query.rs` as the pattern to mirror for reading `state.instance`.
- Produces: a live route with no TS consumer in this plan — `app/src/api/game.ts`'s own header comment already documents that most of the seventeen `/api/v1/game/*` routes have no TS wrapper yet, "for the one route the map view needs so far"; this task follows that precedent rather than inventing a live-flow consumer nothing asks for.

- [ ] **Step 1: Write the failing test**

In `crates/server/src/game/query.rs`'s test module (or wherever
`entity_prototypes`'s own test lives — copy its exact "instance not started"
and "instance started" setup):

```rust
    #[tokio::test]
    async fn flow_answers_503_with_no_instance_running() {
        let state = test_state_with_no_instance(); // reuse whatever helper
                                                     // entity_prototypes's own
                                                     // "not started" test uses
        let err = flow(State(state)).await.unwrap_err();
        assert_eq!(err.code(), 2); // however this file's existing tests
                                    // assert ErrorResponse::not_started
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo build -p factorio-bot-server --features lua 2>&1 | tail -40
```

Expected: FAIL to compile — `flow` does not exist yet.

- [ ] **Step 3: Implement**

In `crates/server/src/game/query.rs`, add the import beside the existing
ones:

```rust
use factorio_bot_core::graph::flow_export::FlowExport;
```

Add the handler, following `entity_prototypes`'s exact shape:

```rust
/// The flow graph for the running world's one surface -- the live analogue
/// of `GET /api/v1/runs/{id}/flow`'s archived keyframes.
///
/// `tick` comes from the instance's own RCON connection, not from the
/// caller: a live query has exactly one tick worth reporting, the game's
/// current one.
#[utoipa::path(
    get,
    path = "/api/v1/game/flow",
    tag = "Query",
    responses(
        (status = 200, body = FlowExport),
        (status = 503, body = crate::error::ErrorResponse),
    )
)]
pub async fn flow(State(state): State<AppState>) -> ApiResult<FlowExport> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_surface(instance)?;
    let tick = instance.rcon.last_tick().unwrap_or(0);
    Ok(Json(world.flow_graph.export(tick)))
}
```

(`ApiResult`, `ErrorResponse`, `State`, `Json` are already imported at the
top of this file for the neighbouring handlers; `require_surface` is
imported from `super::require_surface` or already in scope via
`crate::game::require_surface` — match whatever `entity_prototypes` already
uses.)

Register in `crates/server/src/game/mod.rs`, beside the existing
`.routes(routes!(query::entity_prototypes))` line:

```rust
        .routes(routes!(query::flow))
```

Regenerate the OpenAPI snapshot (this adds one route reusing the
already-pinned `FlowExport` schema from Task 4 — no new TS mirror needed):

```bash
UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
git diff --no-ext-diff app/src/api/openapi.snapshot.json
```

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
nix develop -c cargo test -p factorio-bot-server --lib game::
nix develop -c cargo clippy -p factorio-bot-server --all-features --all-targets -- --deny warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(server): GET /game/flow -- the live world's flow graph, the running twin of /runs/{id}/flow

Reuses the FlowExport schema /runs/{id}/flow already pinned; no new TS
mirror needed. No frontend consumer in this change, matching api/game.ts's
existing precedent of wrapping only the routes something actually needs.
EOF
git commit -F /tmp/msg -- crates/server/src/game/query.rs crates/server/src/game/mod.rs app/src/api/openapi.snapshot.json
```

---

### Task 6: `flowJoin.ts` — joining a flow keyframe to the run's samples

**Files:**
- Create: `app/src/lib/flowJoin.ts`
- Test: `app/src/lib/flowJoin.spec.ts`

**Interfaces:**
- Consumes: `FlowExport`/`FlowExportNode`/`FlowExportEdge`/`FlowExportRate`, `Position`, `Sample`, `MachinesSample`, `MachineSample` (`@/api/types`, Task 4 + existing), `positionKey`, `statusClass`, `machineStatusAt`, `StatusClass` (`@/lib/machineTimeline`, existing), `TICKS_PER_MINUTE` (`@/lib/tickScale`, existing), `RATE_WINDOW_MINUTES` (`@/lib/runRates`, existing).
- Produces:
  ```ts
  export interface FlowViewNode {
      id: number; position: Position; name: string; kind: string;
      recipe: string | null; minerOre: string | null; status: StatusClass;
      primaryItem: string | null;
      modelPerMinute: number | null;
      measuredPerMinute: number | null;
      gap: number | null;
  }
  export interface FlowViewEdge { from: number; to: number; totalPerMinute: number }
  export interface FlowView { nodes: FlowViewNode[]; edges: FlowViewEdge[] }
  export function flowAt(flow: FlowExport[], tick: number): FlowExport | null
  export function flowView(flow: FlowExport, samples: Sample[], cursorTick: number, windowMinutes?: number): FlowView
  ```
  Task 7 (`FlowPanel.vue`) consumes `flowAt` and `flowView`.

- [ ] **Step 1: Write the failing tests**

`app/src/lib/flowJoin.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import {flowAt, flowView} from './flowJoin';
import {FlowExport, MachinesSample, Sample} from '@/api/types';

function pos(x: number, y: number) {
    return {x, y};
}

const FURNACE_FLOW: FlowExport = {
    tick: 6000,
    nodes: [
        {id: 0, position: pos(0.5, 0.5), name: 'burner-mining-drill', kind: 'mining-drill', recipe: null, miner_ore: 'iron-ore'},
        {id: 1, position: pos(1.5, 0.5), name: 'stone-furnace', kind: 'furnace', recipe: null, miner_ore: null}
    ],
    edges: [
        {from: 0, to: 1, lanes: [[{item: 'iron-ore', per_second: 0.5}]]}
    ]
};

function machinesSample(tick: number, produced: number): MachinesSample {
    return {
        kind: 'machines',
        tick,
        machines: {
            '1': {
                name: 'stone-furnace', type: 'furnace', position: pos(1.5, 0.5),
                status: 'working', network: null, recipe: 'iron-plate', crafting: true,
                progress: 0.5, products_finished: produced, produced,
                produced_source: 'counter'
            } as unknown as Sample extends never ? never : MachinesSample['machines'][string]
        }
    };
}

describe('flowAt', () => {
    it('is the latest keyframe at or before the tick', () => {
        const a: FlowExport = {...FURNACE_FLOW, tick: 1000};
        const b: FlowExport = {...FURNACE_FLOW, tick: 5000};
        expect(flowAt([a, b], 4000)).toEqual(a);
        expect(flowAt([a, b], 6000)).toEqual(b);
    });

    it('is null before any keyframe exists', () => {
        expect(flowAt([{...FURNACE_FLOW, tick: 1000}], 500)).toBeNull();
    });

    it('is null for an empty flow history', () => {
        expect(flowAt([], 1000)).toBeNull();
    });
});

describe('flowView', () => {
    it('reports a node with no machine sample at its position as measured: null, never 0', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.measuredPerMinute).toBeNull();
    });

    it('joins a node to its machine sample by position and computes a per-minute rate', () => {
        const samples: Sample[] = [machinesSample(3600, 10), machinesSample(6000, 34)];
        const view = flowView(FURNACE_FLOW, samples, 6000, 2);
        const furnace = view.nodes.find((n) => n.id === 1)!;
        // 24 items over the trailing 2-minute window = 12/min.
        expect(furnace.measuredPerMinute).toBe(12);
        expect(furnace.status).toBe('good'); // 'working' -> statusClass -> 'good'
    });

    it('derives model rate from the node\'s own outgoing edges, in items/minute', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.primaryItem).toBe('iron-ore');
        expect(drill.modelPerMinute).toBe(30); // 0.5/s * 60
    });

    it('reports gap as null whenever either side is null or measured is zero', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.gap).toBeNull(); // no measured sample at all
    });

    it('sums edge lanes into one totalPerMinute for stroke width', () => {
        const twoLane: FlowExport = {
            tick: 0,
            nodes: [
                {id: 0, position: pos(0, 0), name: 'transport-belt', kind: 'transport-belt', recipe: null, miner_ore: null},
                {id: 1, position: pos(1, 0), name: 'transport-belt', kind: 'transport-belt', recipe: null, miner_ore: null}
            ],
            edges: [{
                from: 0, to: 1,
                lanes: [[{item: 'iron-ore', per_second: 0.5}], [{item: 'copper-ore', per_second: 0.3}]]
            }]
        };
        const view = flowView(twoLane, [], 0);
        expect(view.edges[0].totalPerMinute).toBeCloseTo(48); // (0.5+0.3)*60
    });
});
```

(If `MachineSample`'s exact field list makes the `machinesSample` helper's
type assertion awkward, drop the `as unknown as ...` cast and instead build
it with every field `MachineSample` requires, reading the interface from
`app/src/api/types.ts` directly — the cast above is a placeholder for "match
whatever fields the real interface has"; replace it with a real object
literal satisfying `MachineSample`.)

- [ ] **Step 2: Run to verify failure**

```bash
cd app && pnpm vitest run src/lib/flowJoin.spec.ts
```

Expected: FAIL — `./flowJoin` does not exist yet.

- [ ] **Step 3: Implement**

`app/src/lib/flowJoin.ts`:

```ts
/**
 * Joining one flow-graph keyframe to a run's machine samples at a cursor
 * tick.
 *
 * Pure, like every other band's lib: `(flow, samples, tick) -> view model`,
 * no fetch, no store. `flow.jsonl` carries full snapshots rather than
 * deltas, so picking the right keyframe (`flowAt`) is a separate, trivial
 * step from joining it to samples (`flowView`).
 */

import {FlowExport, FlowExportNode, MachineSample, MachinesSample, Position, Sample} from '@/api/types';
import {machineStatusAt, positionKey, statusClass, StatusClass} from '@/lib/machineTimeline';
import {RATE_WINDOW_MINUTES} from '@/lib/runRates';
import {TICKS_PER_MINUTE} from '@/lib/tickScale';

export interface FlowViewNode {
    id: number;
    position: Position;
    name: string;
    kind: string;
    recipe: string | null;
    minerOre: string | null;
    status: StatusClass;
    /** The item this node's outgoing edges name at the highest modelled
     *  rate, or `null` for a node with no outgoing edge (a sink -- a chest,
     *  say, or the last node on a line). */
    primaryItem: string | null;
    /** Items/minute, modelled: `primaryItem`'s rate summed across every
     *  outgoing edge and lane. `null` exactly when `primaryItem` is. */
    modelPerMinute: number | null;
    /** Items/minute, measured: this node's own machine sample, `primaryItem`'s
     *  production over the trailing window. `null` when there is no machine
     *  sample at this position (a belt, a pipe) or no counter reading there
     *  -- never `0` for "we do not know". */
    measuredPerMinute: number | null;
    /** `modelPerMinute / measuredPerMinute`. `null` whenever either side is
     *  `null` or measured is `0` -- a ratio against nothing said is not a
     *  number. */
    gap: number | null;
}

export interface FlowViewEdge {
    from: number;
    to: number;
    /** Every lane's every item, summed, items/minute -- what stroke width
     *  is drawn from. */
    totalPerMinute: number;
}

export interface FlowView {
    nodes: FlowViewNode[];
    edges: FlowViewEdge[];
}

/** The flow keyframe nearest and at or before `tick`, or `null` when the
 *  run has none yet. `flow` need not be sorted -- this scans it fully,
 *  matching every other cursor-driven "latest at or before" helper on this
 *  page rather than assuming server order. */
export function flowAt(flow: FlowExport[], tick: number): FlowExport | null {
    let latest: FlowExport | null = null;
    for (const f of flow) {
        if (f.tick <= tick && (latest === null || f.tick > latest.tick)) latest = f;
    }
    return latest;
}

function machinesSamples(samples: Sample[]): MachinesSample[] {
    return samples
        .filter((s): s is MachinesSample => s.kind === 'machines')
        .sort((a, b) => a.tick - b.tick);
}

function byPosition(machines: Record<string, MachineSample>): Map<string, MachineSample> {
    const out = new Map<string, MachineSample>();
    for (const m of Object.values(machines)) out.set(positionKey(m.position), m);
    return out;
}

/**
 * Each machine's own production delta over `(lo, hi]`, by position key.
 *
 * Deliberately NOT `runAttribution.ts`'s `machineProduction` -- that
 * function aggregates by item and entity *name* for the roster-fed/factory
 * verdict, collapsing every furnace of the same kind into one bucket. This
 * needs one specific machine's own count, to join to one specific flow
 * node.
 */
function machineDeltaAt(samples: Sample[], lo: number, hi: number): Map<string, number> {
    const rows = machinesSamples(samples);
    let baseAt: MachinesSample | null = null;
    let endAt: MachinesSample | null = null;
    for (const s of rows) {
        if (s.tick <= lo) baseAt = s;
        if (s.tick <= hi) endAt = s;
        else break;
    }
    const out = new Map<string, number>();
    if (endAt === null) return out;
    const base = byPosition(baseAt?.machines ?? {});
    for (const m of Object.values(endAt.machines)) {
        if (m.produced === null || m.produced === undefined) continue;
        const key = positionKey(m.position);
        const before = base.get(key)?.produced ?? 0;
        const delta = m.produced - before;
        if (delta > 0) out.set(key, (out.get(key) ?? 0) + delta);
    }
    return out;
}

function outgoingRatesByNode(flow: FlowExport): Map<number, Map<string, number>> {
    const out = new Map<number, Map<string, number>>();
    for (const edge of flow.edges) {
        const totals = out.get(edge.from) ?? new Map<string, number>();
        for (const lane of edge.lanes) {
            for (const rate of lane) {
                totals.set(rate.item, (totals.get(rate.item) ?? 0) + rate.per_second);
            }
        }
        out.set(edge.from, totals);
    }
    return out;
}

function primaryOf(outgoing: Map<string, number> | undefined): {item: string | null; perSecond: number} {
    if (outgoing === undefined) return {item: null, perSecond: 0};
    let item: string | null = null;
    let perSecond = 0;
    for (const [candidate, rate] of outgoing) {
        if (item === null || rate > perSecond) {
            item = candidate;
            perSecond = rate;
        }
    }
    return {item, perSecond};
}

function viewNode(
    node: FlowExportNode,
    outgoing: Map<string, number> | undefined,
    statuses: Map<string, string | null>,
    produced: Map<string, number>,
    windowMinutes: number
): FlowViewNode {
    const key = positionKey(node.position);
    const status = statusClass(statuses.get(key) ?? null);
    const {item: primaryItem, perSecond} = primaryOf(outgoing);
    const modelPerMinute = primaryItem === null ? null : perSecond * 60;
    const madeThisWindow = produced.get(key);
    const measuredPerMinute = madeThisWindow === undefined ? null : madeThisWindow / windowMinutes;
    const gap =
        modelPerMinute !== null && measuredPerMinute !== null && measuredPerMinute > 0
            ? modelPerMinute / measuredPerMinute
            : null;
    return {
        id: node.id, position: node.position, name: node.name, kind: node.kind,
        recipe: node.recipe, minerOre: node.miner_ore, status,
        primaryItem, modelPerMinute, measuredPerMinute, gap
    };
}

/** Joins one flow keyframe to `samples` at `cursorTick`, over a trailing
 *  window of `windowMinutes` (default: the same `RATE_WINDOW_MINUTES` every
 *  other band on this page uses). */
export function flowView(
    flow: FlowExport,
    samples: Sample[],
    cursorTick: number,
    windowMinutes: number = RATE_WINDOW_MINUTES
): FlowView {
    const windowTicks = windowMinutes * TICKS_PER_MINUTE;
    const lo = Math.max(0, cursorTick - windowTicks);
    const statuses = machineStatusAt(samples, cursorTick);
    const produced = machineDeltaAt(samples, lo, cursorTick);
    const outgoing = outgoingRatesByNode(flow);

    const nodes = flow.nodes.map((n) => viewNode(n, outgoing.get(n.id), statuses, produced, windowMinutes));

    const edges: FlowViewEdge[] = flow.edges.map((e) => ({
        from: e.from,
        to: e.to,
        totalPerMinute: e.lanes.flat().reduce((sum, r) => sum + r.per_second * 60, 0)
    }));

    return {nodes, edges};
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cd app && pnpm vitest run src/lib/flowJoin.spec.ts
cd app && pnpm lint
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): flowJoin.ts -- one flow keyframe, joined to the run's samples

flowAt picks the keyframe nearest a cursor tick; flowView joins it to
machine samples by position, computing each node's model rate (from its own
outgoing edges), measured rate (from its own machine's counter over the
trailing window) and their gap, absent staying absent throughout.
EOF
git commit -F /tmp/msg -- app/src/lib/flowJoin.ts app/src/lib/flowJoin.spec.ts
```

---

### Task 7: `FlowPanel.vue`

**Files:**
- Create: `app/src/components/run/FlowPanel.vue`
- Test: `app/src/components/run/FlowPanel.spec.ts`

**Interfaces:**
- Consumes: `FlowView`/`FlowViewNode`/`FlowViewEdge` (Task 6), `Position` (`@/api/types`), `projectionFor` (`@/lib/mapProjection`, existing — same helper `MapPanel.vue` uses).
- Produces: a `FlowPanel` component with props `{flow: FlowExport | null; flowError: string | null; samples: Sample[]; cursor: number}` and no emits. Task 8 mounts it as `RunSidePanel.vue`'s third tab.

- [ ] **Step 1: Write the failing tests**

`app/src/components/run/FlowPanel.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import FlowPanel from './FlowPanel.vue';
import {FlowExport} from '@/api/types';

const FLOW: FlowExport = {
    tick: 100,
    nodes: [
        {id: 0, position: {x: 0.5, y: 0.5}, name: 'burner-mining-drill', kind: 'mining-drill', recipe: null, miner_ore: 'iron-ore'},
        {id: 1, position: {x: 2.5, y: 0.5}, name: 'stone-furnace', kind: 'furnace', recipe: null, miner_ore: null}
    ],
    edges: [{from: 0, to: 1, lanes: [[{item: 'iron-ore', per_second: 0.5}]]}]
};

describe('FlowPanel', () => {
    it('draws one mark per node and one line per edge', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        expect(w.findAll('circle.flow-node')).toHaveLength(2);
        expect(w.findAll('line.flow-edge')).toHaveLength(1);
    });

    it('scales an edge\'s stroke width by its modelled rate', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        const line = w.get('line.flow-edge');
        expect(Number(line.attributes('stroke-width'))).toBeGreaterThan(0);
    });

    it('titles a node with model, measured and gap wording -- the wording is the claim', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        const drill = w.findAll('circle.flow-node')[0];
        expect(drill.find('title').text()).toContain('model');
        expect(drill.find('title').text()).toContain('measured');
    });

    it('shows a one-line reason instead of an empty plot when flow is null', () => {
        const w = mount(FlowPanel, {props: {flow: null, flowError: null, samples: [], cursor: 100}});
        expect(w.findAll('circle.flow-node')).toHaveLength(0);
        expect(w.text().toLowerCase()).toContain('no flow');
    });

    it('shows the fetch-failure reason, not an empty plot, when flowError is set', () => {
        const w = mount(FlowPanel, {props: {flow: null, flowError: 'this server does not provide /flow', samples: [], cursor: 100}});
        expect(w.text()).toContain('this server does not provide /flow');
    });
});
```

- [ ] **Step 2: Run to verify failure**

```bash
cd app && pnpm vitest run src/components/run/FlowPanel.spec.ts
```

Expected: FAIL — `./FlowPanel.vue` does not exist yet.

- [ ] **Step 3: Implement**

`app/src/components/run/FlowPanel.vue`:

```vue
<!-- app/src/components/run/FlowPanel.vue -->
<script setup lang="ts">
/**
 * The flow graph at the cursor: nodes at their real map position, edges as
 * lines whose width is the modelled rate. Each node's `<title>` carries the
 * claim in words -- model rate, measured rate, and the gap between them,
 * since a node with no measured reading must say so rather than draw a
 * silent zero.
 */
import {computed} from 'vue';
import {FlowExport, Sample} from '@/api/types';
import {flowView, FlowViewNode} from '@/lib/flowJoin';
import {projectionFor} from '@/lib/mapProjection';

const props = defineProps<{
    flow: FlowExport | null;
    flowError: string | null;
    samples: Sample[];
    cursor: number;
}>();

const VIEW_SIZE = 480;
const MARGIN = 4;

const view = computed(() => (props.flow === null ? null : flowView(props.flow, props.samples, props.cursor)));

const bounds = computed(() => {
    const nodes = props.flow?.nodes ?? [];
    if (nodes.length === 0) return {left: 0, top: 0, right: 1, bottom: 1};
    const xs = nodes.map((n) => n.position.x);
    const ys = nodes.map((n) => n.position.y);
    return {
        left: Math.min(...xs) - MARGIN, right: Math.max(...xs) + MARGIN,
        top: Math.min(...ys) - MARGIN, bottom: Math.max(...ys) + MARGIN
    };
});

const projection = computed(() => projectionFor(bounds.value, VIEW_SIZE, VIEW_SIZE));

function screenX(x: number): number { return x * projection.value.scale + projection.value.offsetX; }
function screenY(y: number): number { return y * projection.value.scale + projection.value.offsetY; }

const nodeById = computed(() => new Map((view.value?.nodes ?? []).map((n) => [n.id, n])));

function formatRate(perMinute: number | null): string {
    return perMinute === null ? 'not measured' : `${perMinute.toFixed(1)}/min`;
}

function nodeTitle(n: FlowViewNode): string {
    const model = n.primaryItem === null ? 'no outgoing flow' : `model ${n.primaryItem} ${formatRate(n.modelPerMinute)}`;
    const measured = `measured ${formatRate(n.measuredPerMinute)}`;
    const gap = n.gap === null ? '' : ` · gap ${n.gap.toFixed(2)}x`;
    return `${n.name} (${n.kind})\n${model}\n${measured}${gap}`;
}

/** 1 to 6 screen px, scaled by the edge's own modelled rate against the
 *  busiest edge in this keyframe -- so one keyframe's edges are comparable
 *  to each other, never to another keyframe's absolute numbers. */
const strokeWidth = computed(() => {
    const rates = (view.value?.edges ?? []).map((e) => e.totalPerMinute);
    const max = Math.max(1, ...rates);
    return (rate: number) => 1 + 5 * (rate / max);
});
</script>

<template>
  <div class="p-3">
    <p v-if="flowError !== null" class="text-sm text-ink-muted">{{ flowError }}</p>
    <p v-else-if="flow === null" class="text-sm text-ink-muted">no flow keyframe recorded yet</p>
    <svg v-else :viewBox="`0 0 ${VIEW_SIZE} ${VIEW_SIZE}`" class="w-full" role="img" aria-label="flow graph at cursor">
      <line v-for="e in view!.edges" :key="`${e.from}-${e.to}`" class="flow-edge"
            :x1="screenX(nodeById.get(e.from)?.position.x ?? 0)" :y1="screenY(nodeById.get(e.from)?.position.y ?? 0)"
            :x2="screenX(nodeById.get(e.to)?.position.x ?? 0)" :y2="screenY(nodeById.get(e.to)?.position.y ?? 0)"
            :stroke-width="strokeWidth(e.totalPerMinute)" stroke="var(--color-ink-muted)" stroke-linecap="round" />
      <g v-for="n in view!.nodes" :key="n.id">
        <circle class="flow-node" :cx="screenX(n.position.x)" :cy="screenY(n.position.y)" r="5"
                :fill="`var(--color-status-${n.status})`" stroke="var(--color-surface)" stroke-width="1">
          <title>{{ nodeTitle(n) }}</title>
        </circle>
      </g>
    </svg>
  </div>
</template>
```

- [ ] **Step 4: Run to verify pass**

```bash
cd app && pnpm vitest run src/components/run/FlowPanel.spec.ts
cd app && pnpm lint
```

Expected: PASS. If the `--color-status-{good,warn,serious,critical,neutral}`
and `--color-surface` tokens are not already defined in
`app/src/assets/tailwind.css` (they should be, from Phase 1's dark-mode
token work — verify with `grep -n "color-status" app/src/assets/tailwind.css`
before assuming a gap), add them following the Phase 1 pattern (`@theme
static` block, light + dark values) rather than inventing new ad-hoc colours.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): FlowPanel -- the flow graph at the cursor, model vs measured per node

Nodes at their real map position (reusing the same projectionFor helper
MapPanel uses), edges as lines whose width is the modelled rate, each node's
title spelling out model/measured/gap in words rather than leaving a viewer
to infer it from colour alone.
EOF
git commit -F /tmp/msg -- app/src/components/run/FlowPanel.vue app/src/components/run/FlowPanel.spec.ts app/src/assets/tailwind.css
```

(Drop `app/src/assets/tailwind.css` from the path list if no token was
added.)

---

### Task 8: Wire `flow` into `runsStore.ts` and `RunSidePanel.vue`

**Files:**
- Modify: `app/src/store/runsStore.ts` (fetch `/flow` in `openRun`'s `Promise.allSettled`, expose `flow`/`flowError` state)
- Modify: `app/src/components/run/RunSidePanel.vue` (third tab `'flow'`, mount `FlowPanel`)
- Test: extend `app/src/store/runsStore.spec.ts` and `app/src/components/run/RunSidePanel.spec.ts` (whichever already exist and cover `provenance`/`replay` — follow their exact pattern)

**Interfaces:**
- Consumes: `getRunFlow` (Task 4), `flowAt` (Task 6), `FlowPanel` (Task 7), the existing `provenance`/`provenanceError` fields and `enrichmentUnavailable` helper in `runsStore.ts` as the pattern to mirror exactly.
- Produces: `store.flow: FlowExport[]`, `store.flowError: string | null`, and a `flowAtCursor` getter, consumed nowhere further inside this plan (this is the last task) but reachable from `RunPage.vue` if a future task wants a headline chip.

- [ ] **Step 1: Write the failing tests**

Find `runsStore.spec.ts`'s existing test(s) covering `provenance` /
`provenanceError` after `openRun` (search `provenanceError` in that file)
and add a sibling test with `flow`/`flowError` substituted, using whatever
mock-fetch harness the existing test already sets up:

```ts
    it('exposes flow and flowError the same way it exposes provenance and provenanceError', async () => {
        // Mirror the existing "openRun sets provenanceError on a 404"
        // and "openRun sets provenance on success" tests in this file,
        // substituting getRunFlow / RunFlowResponse for
        // getRunProvenance / Provenance. Read those two tests first and
        // copy their mock-fetch setup exactly.
    });
```

Find `RunSidePanel.spec.ts`'s existing test(s) covering the `video` tab
(search `hasVideo` or `tab.value`) and add:

```ts
    it('shows a Flow tab and mounts FlowPanel when it is selected', async () => {
        // Mirror the existing video-tab test's mount + click sequence,
        // substituting the new `flow`/`flowError` props and the 'Flow' tab
        // button.
    });
```

**Both of these steps are sketches, matching Task 3's precedent** — the
exact mock-fetch and mount harness in each spec file already exists for the
sibling feature (`provenance`, `video`) and must be copied, not
re-invented. Read the existing test in full before writing the new one.

- [ ] **Step 2: Run to verify failure**

```bash
cd app && pnpm vitest run src/store/runsStore.spec.ts src/components/run/RunSidePanel.spec.ts
```

Expected: FAIL — the new state/tab do not exist yet.

- [ ] **Step 3: Implement**

`app/src/store/runsStore.ts`. Add state, beside the existing `provenance`/
`provenanceError` fields:

```ts
        /** The flow graph's own keyframes for this run -- empty for a run
         *  recorded before this feature existed, or one that never reached
         *  a keyframe. See `flowJoin.ts::flowAt` for picking the one nearest
         *  a cursor tick. */
        flow: [] as FlowExport[],
        flowError: null as string | null,
```

In `openRun`, add `getRunFlow(id)` to the `Promise.allSettled` array beside
`getRunProvenance(id)`, `getRunReplay(id)`, `getRunSavepoints(id)` — find
that exact array (search `provenanceResult,`) and add `flowResult` in the
same position pattern, then handle it exactly like the provenance branch:

```ts
                if (flowResult.status === 'fulfilled') {
                    this.flow = flowResult.value.flow;
                    this.flowError = null;
                } else {
                    this.flow = [];
                    this.flowError = enrichmentUnavailable('flow', '/flow', flowResult.reason);
                }
```

Reset `this.flowError = null;` beside the existing `this.provenanceError =
null;` reset at the top of `openRun`, and add `flow: [],` beside wherever
`provenance` is reset to its empty value on a fresh `openRun` call, if such
a reset block exists (read the surrounding code to match).

Import `FlowExport` and `getRunFlow` at the top of the file alongside the
existing `Provenance`/`getRunProvenance` imports.

`app/src/components/run/RunSidePanel.vue`. Widen the tab type and add
props:

```ts
const tab = ref<'map' | 'video' | 'flow'>('map');
```

Add to `defineProps`:

```ts
    flow: FlowExport | null; flowError: string | null;
```

(`FlowExport` here is the single keyframe nearest the cursor — the parent,
`RunPage.vue` or wherever `RunSidePanel` is mounted, computes it via
`flowAt(store.flow, cursor)` and passes it down, exactly the shape `props.records`/`props.entities` are already computed one level up and passed in.)

Import `FlowPanel` beside the existing `MapPanel` import, and add the third
tab button beside the existing `video` one in the `<template>`'s
`role="tablist"` div:

```html
      <button v-if="flow !== null || flowError !== null" role="tab" type="button" :aria-selected="tab === 'flow'"
              class="rounded-t border border-b-0 border-divider px-3 py-1 text-sm"
              :class="tab === 'flow' ? 'bg-card text-ink' : 'text-ink-muted'" @click="tab = 'flow'">Flow</button>
```

Add the panel body beside the existing `v-if="tab === 'video'"` block:

```html
      <FlowPanel v-if="tab === 'flow'" :flow="flow" :flow-error="flowError" :samples="[]" :cursor="cursor" />
```

**`samples="[]"` here is a known gap to close in the calling component, not
in `FlowPanel` or `RunSidePanel` themselves**: `RunSidePanel` does not
currently receive the run's `samples` array (only `records`/`entities`/
`bots`/`trail`, all pre-derived one level up). Add `samples: Sample[]` to
`RunSidePanel`'s own `defineProps` and thread it through from wherever
`RunSidePanel` is mounted (`RunPage.vue`, passing `store.samples` — check
that field's exact name on the store, likely `samples` already used by
`ProductionBand` etc.), then pass `:samples="samples"` instead of `:samples="[]"`
on the `FlowPanel` line above.

- [ ] **Step 4: Run to verify pass**

```bash
cd app && pnpm vitest run src/store/runsStore.spec.ts src/components/run/RunSidePanel.spec.ts src/lib/flowJoin.spec.ts src/components/run/FlowPanel.spec.ts
cd app && pnpm lint
cd app && pnpm run build:web
```

Expected: PASS, and the SPA builds.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): wire /runs/{id}/flow into the run page as a third Map/Video/Flow tab

runsStore fetches flow the same way it fetches provenance/replay/savepoints
-- Promise.allSettled, an enrichmentUnavailable reason on failure, never a
silent empty state. RunSidePanel gains a Flow tab that only appears once
there is something (or a reason) to show.
EOF
git commit -F /tmp/msg -- app/src/store/runsStore.ts app/src/components/run/RunSidePanel.vue app/src/store/runsStore.spec.ts app/src/components/run/RunSidePanel.spec.ts app/src/pages/RunPage.vue
```

(Drop `app/src/pages/RunPage.vue` from the path list if threading `samples`
through turned out to need no change there — e.g. if `RunSidePanel` is
mounted with `v-bind="$attrs"` or already receives the full store object.)

---

## Self-Review

**Spec coverage** (`docs/superpowers/specs/2026-09-08-run-anatomy-design.md`):
- Section 1 item 12 ("Flow at cursor... model/min, measured/min, gap × ...
  status stripe from the heatmap") → Tasks 6, 7.
- Section 2.3 `FlowExport` shape, `FlowGraph::export()`, `flow.jsonl` at
  every keyframe, `/runs/{id}/flow`, `/game/flow` → Tasks 1, 2, 3, 4, 5.
- Section 3 new component `FlowPanel` under `app/src/components/run/` →
  Task 7.
- Section 4 error handling: each enrichment fails independently, a 404
  reads "this server does not provide …" → Task 8 (mirrors the existing
  `provenance`/`replay` pattern exactly); "a `Position` with a non-finite
  float in `flow.jsonl` is skipped per node with a count, never a crash" →
  satisfied by Task 2's `read_flow` skip-on-parse-error counter (a JSON
  document cannot carry a literal NaN/Infinity float, so any such corruption
  already fails to parse and is counted, exactly like a truncated line).
- Section 5 testing: Rust round-trip + name-sorted-rate tests (Task 1),
  recorder write tests (Task 2), seam tests (Task 4) → covered. "The
  recorder writes `flow.jsonl` at each keyframe" → Task 3's test.
- Section 6 phasing table's Phase 3 row (`FlowExport`, `flow.jsonl`,
  `/runs/{id}/flow`, `/game/flow`, `FlowPanel`) → all five present across
  Tasks 1–7.
- Section 7 risk "Determinism of the export" → Global Constraints + Task 1's
  `export_preserves_the_name_sorted_rate_order` test. Risk "Peer work in
  flight" (naming `flow_export.rs` as a new file beside `flow_graph.rs`) →
  addressed in Global Constraints with a live `git status` check instead of
  the stale claim that `entity_graph.rs` is presently under another
  session's edit (verified clean at plan-writing time; re-check is on the
  implementer).

**Gaps intentionally left out of this plan, and why:**
- A headline chip surfacing `store.flow`/`store.flowError` on `RunHeadline.vue`
  (the way `replayError`/`savepointsError` chips were added in Phase 2) is
  not in this plan. The spec's Phase 3 scope is the flow panel itself;
  adding a headline chip is a small, independent follow-up the final
  whole-branch review can flag if it judges the omission a defect.
- No frontend consumer for `/game/flow` (Task 5) — matches `api/game.ts`'s
  own stated precedent of wrapping only routes an existing page needs.

**Placeholder scan:** Two steps are explicitly marked as sketches rather
than exact code (Task 3 Step 1, Task 8 Step 1) — both name precisely which
existing test in which file to copy the harness from, which is the
established pattern this plan follows rather than inventing test
infrastructure blind. Every other step contains complete, runnable code.

**Type consistency:** `FlowExport`/`FlowExportNode`/`FlowExportEdge`/
`FlowExportRate` field names and types are identical across Task 1 (Rust),
Task 4 (Rust route + TS mirror), Task 6 (`flowJoin.ts`), and Task 7
(`FlowPanel.vue`) — `recipe`/`miner_ore` stay `Option<String>`/`string|null`
throughout, `lanes: Vec<Vec<FlowExportRate>>`/`FlowExportRate[][]` throughout,
`id`/`from`/`to` stay `u32`/`number` throughout. `FlowView`/`FlowViewNode`/
`FlowViewEdge` (Task 6) are consumed by their exact declared shape in Task 7,
including the `StatusClass` import from `machineTimeline.ts`.
