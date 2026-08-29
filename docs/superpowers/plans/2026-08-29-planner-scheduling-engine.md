# Planner Scheduling Engine — Implementation Plan (1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `crates/planner` up to the point where it can take a hand-constructed set of `Action`s and a `PlanState`, infer their ordering, assign them across N bots with travel-aware greedy scheduling, and render the result as a Mermaid Gantt chart — with no Factorio, no RCON, and no goal decomposition.

**Architecture:** Three of the spec's six layers. `PlanState` (layer 0) is an overlay over an immutable `Arc<FactorioWorld>`, so forking is cheap. `Action`/`Condition`/`Effect` (layer 3) carry preconditions and effects as data, so the machinery that consumes them never learns a recipe name. `schedule()` (layer 4) is a pure function of `(ActionNetwork, PlanState, &[BotId])` that binds `Actor::Role` to a concrete bot, emits walks, and produces an immutable `Schedule`. Rendering consumes `Schedule`.

**Tech Stack:** Rust 2021, `factorio-bot-core` (path dependency), `petgraph` 0.6, `miette` 7, `thiserror` 2, `serde` 1.

**Spec:** `docs/superpowers/specs/2026-08-29-multi-agent-planner-design.md`

**Follow-on plans (not this one):** (2) `Goal`, `Method`, the expansion driver and the methods for `Have(automation-science-pack, N)`; (3) layer 5 execution over RCON, the Lua `plan.*` rewrite, and deletion of `core/plan` and `graph/task_graph.rs`.

## Global Constraints

- Workspace root: `/home/arturh/projects/private/factorio-bot`. Rust edition 2021.
- **All commands run inside the Nix devShell with mise tools on PATH.** Prefix every cargo invocation:
  `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated` must pass at the end of every task.
- **`cargo fmt -p factorio-bot-planner` before every commit — never `cargo fmt --all`.** Another agent owns the other crates in this checkout; `--all` reformats their in-progress files and dirties the shared tree. (This already happened once.)
- Crate name `factorio-bot-planner`, directory `crates/planner`, version `0.2.4-dev` to match `factorio-bot-core`.
- **`Ticks` is `u32`, a count of game ticks. 60 ticks = 1 second.** No `f64` durations anywhere in layers 0-4. Seconds appear only in `render.rs` output.
- **`factorio_bot_core::types::ActionId` already exists** (a `u32` alias used for RCON action correlation). The planner's `ActionId` is a distinct newtype in `crates/planner/src/ids.rs`. Never import core's.
- **`Actor::Role` must not be resolved outside `schedule()`.** Layers 0 and 3 take an explicit `binding: BotId` parameter; they never guess.
- Large enum variants get `Box`ed (`FactorioEntity` is ~200 bytes) — clippy denies `large_enum_variant` under `--deny warnings`.
- Determinism is required: every tie-break in the scheduler is broken by `(ActionId, BotId)` ascending, so golden tests are stable.
- Build structs with functional-update syntax (`Foo { a, b, ..Default::default() }`), never `let mut x = Foo::default();` followed by field assignments — clippy's `field_reassign_with_default` fires under `--deny warnings`, and an `#[allow]` is the wrong fix.
- **Another agent works in `app/**` in this same checkout and keeps changes staged.** `git add` followed by `git commit` commits the WHOLE INDEX, including their staged files — this has already happened once. Always commit with the partial-commit form, which builds the commit from the working tree for the named paths and ignores the index entirely:
  `git commit -m "<message>" -- <explicit paths>`
  Never `git add -A`, never `git add .`, never `git commit -a`. Never `git checkout`, `git stash`, or `git reset` anything outside `crates/planner/`.
- Workspace-wide clippy is the per-task gate and is safe to run even with another agent's uncommitted work present: their changes are formatting-only and cannot break compilation.

## Deviation from the spec, flagged

The spec puts a `Reservations` structure in `PlanState` to prevent two bots mining the same tile or placing at the same position. **This plan omits it**, because the scheduler applies each action's effects to the simulated state at the moment it assigns the action. Once `ConsumeResource` decrements the remaining ore at a tile and `CreateEntity` occupies a position, a second action's `ResourceAvailable` or empty-tile precondition fails on its own. A separate reservation table would be a second copy of the same fact.

The guarantees the spec asked for are preserved and property-tested in Task 6. Reservations become genuinely necessary only when actions are in flight during execution and a re-schedule must avoid them — that is plan 3's problem, and the structure can be added there against a real need.

## File Structure

| File | Responsibility |
|---|---|
| `crates/planner/Cargo.toml` | Crate manifest |
| `crates/planner/src/lib.rs` | Module declarations and re-exports |
| `crates/planner/src/ids.rs` | `Ticks`, `BotId`, `ActionId`, `ItemId`, `ActionIdGen` |
| `crates/planner/src/state.rs` | `PlanState`, `BotState`, the world overlay |
| `crates/planner/src/action.rs` | `Actor`, `Condition`, `Effect`, `Action`, `ActionKind` |
| `crates/planner/src/network.rs` | `ActionNetwork`, `Edge`, inference, validation, topological order |
| `crates/planner/src/schedule.rs` | `schedule()`, `Schedule`, `ScheduledStep`, travel model |
| `crates/planner/src/render.rs` | Mermaid Gantt and graphviz from `Schedule` |
| `crates/planner/src/error.rs` | `PlannerError` |
| `crates/planner/tests/scheduling.rs` | Integration and property tests |

Split by responsibility: `state` owns what is true, `action` owns what can be done, `network` owns ordering, `schedule` owns who and when. Each stays well under 400 lines.

---

### Task 1: Crate skeleton, ids, and `PlanState` fork semantics

**Files:**
- Create: `crates/planner/Cargo.toml`, `crates/planner/src/lib.rs`, `crates/planner/src/ids.rs`, `crates/planner/src/error.rs`, `crates/planner/src/state.rs`
- Modify: `Cargo.toml` (workspace members)
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/state.rs`

**Interfaces:**
- Produces: `Ticks = u32`; `BotId(pub u8)`; `ActionId(pub u32)`; `ItemId = String`; `ActionIdGen::new() -> Self`, `ActionIdGen::next(&mut self) -> ActionId`; `PlanState::from_world(base: Arc<FactorioWorld>, bots: &[BotId]) -> PlanState`; `PlanState::fork(&self) -> PlanState`; `PlanState::bot(&self, BotId) -> Option<&BotState>`; `PlanState::inventory_count(&self, BotId, &str) -> u32`; `PlanState::total_count(&self, &str) -> u32`; `PlanState::gain(&mut self, BotId, &str, u32)`; `PlanState::lose(&mut self, BotId, &str, u32) -> Result<(), PlannerError>`; `PlanState::set_position(&mut self, BotId, Position)`; `BotState { position, inventory, build_distance, reach_distance, resource_reach_distance }`; `PlannerError`.

- [ ] **Step 1: Write the failing test**

Create `crates/planner/src/state.rs` with only this test module at the bottom (the implementation comes in step 5):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
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
    fn missing_bots_get_default_reach_distances() {
        let a = state();
        let bot = a.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.build_distance, 10.0);
        assert_eq!(bot.reach_distance, 10.0);
        assert_eq!(bot.resource_reach_distance, 3.0);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — the package does not exist yet.

- [ ] **Step 3: Create the crate manifest**

`crates/planner/Cargo.toml`:

```toml
[package]
name = "factorio-bot-planner"
version = "0.2.4-dev"
authors = ["Artur Hallmann <arturh@arturh.de>"]
edition = "2021"

[package.metadata.release]
tag = false
push = false
publish = false

[dependencies]
factorio-bot-core = { path = "../core", version = "0.2.4-dev" }
petgraph = { version = "0.6.5", features = ["serde-1"] }
miette = { version = "7.4", features = ["fancy"] }
thiserror = "2.0"
serde = { version = "1.0", features = ["derive"] }
```

- [ ] **Step 4: Register the crate in the workspace**

In the root `Cargo.toml`, add `"crates/planner",` to `[workspace] members`, immediately after `"crates/core",`.

- [ ] **Step 5: Write `ids.rs`, `error.rs` and the `state.rs` implementation**

`crates/planner/src/ids.rs`:

```rust
use serde::{Deserialize, Serialize};

/// A count of Factorio game ticks. 60 ticks = 1 second.
pub type Ticks = u32;

/// An item or entity prototype name, e.g. `"iron-plate"`.
pub type ItemId = String;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BotId(pub u8);

impl std::fmt::Display for BotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bot {}", self.0)
    }
}

/// Distinct from `factorio_bot_core::types::ActionId`, which correlates RCON calls.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ActionId(pub u32);

#[derive(Debug, Default)]
pub struct ActionIdGen(u32);

impl ActionIdGen {
    pub fn new() -> Self {
        ActionIdGen(0)
    }
    pub fn next(&mut self) -> ActionId {
        let id = ActionId(self.0);
        self.0 += 1;
        id
    }
}
```

`crates/planner/src/error.rs`:

```rust
use crate::ids::{ActionId, BotId, ItemId};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum PlannerError {
    #[error("{bot} has {available} {item}, needs {required}")]
    #[diagnostic(code(planner::insufficient_items))]
    InsufficientItems {
        bot: BotId,
        item: ItemId,
        required: u32,
        available: u32,
    },

    #[error("no bot {0:?} in this plan state")]
    #[diagnostic(code(planner::unknown_bot))]
    UnknownBot(BotId),

    #[error("action network contains a cycle involving {0:?}")]
    #[diagnostic(code(planner::cyclic_network))]
    CyclicNetwork(ActionId),

    #[error("action {action:?} is unreachable: its predecessors can never all complete")]
    #[diagnostic(code(planner::deadlock))]
    Deadlock { action: ActionId },

    #[error("precondition {condition} of action {action:?} does not hold for {bot}")]
    #[diagnostic(code(planner::precondition_unsatisfied))]
    PreconditionUnsatisfied {
        action: ActionId,
        bot: BotId,
        condition: String,
    },

    #[error("no bots supplied to the scheduler")]
    #[diagnostic(code(planner::no_bots))]
    NoBots,
}
```

`crates/planner/src/state.rs` — put this **above** the test module written in step 1:

```rust
use crate::error::PlannerError;
use crate::ids::{BotId, ItemId};
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::Position;
use std::collections::BTreeMap;
use std::sync::Arc;

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
        PlanState { base, bots: map }
    }

    pub fn fork(&self) -> PlanState {
        self.clone()
    }

    pub fn base(&self) -> &Arc<FactorioWorld> {
        &self.base
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
}
```

`crates/planner/src/lib.rs`:

```rust
pub mod action;
pub mod error;
pub mod ids;
pub mod network;
pub mod render;
pub mod schedule;
pub mod state;

pub use error::PlannerError;
pub use ids::{ActionId, ActionIdGen, BotId, ItemId, Ticks};
pub use state::{BotState, PlanState};
```

Create empty placeholder modules so the crate compiles: `action.rs`, `network.rs`, `render.rs`, `schedule.rs` each containing only a `//!` doc comment for now. They are filled in by Tasks 3-7.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 5 tests.

- [ ] **Step 7: Verify lints**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'`
Expected: no output, exit 0.

- [ ] **Step 8: Commit**

```bash
git commit -m "feat(planner): add crate skeleton with forkable PlanState" -- Cargo.toml Cargo.lock crates/planner
```

---

### Task 2: World overlay — entities, resources, research

**Files:**
- Modify: `crates/planner/src/state.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/state.rs`

**Interfaces:**
- Consumes: `PlanState`, `BotState`, `PlannerError` from Task 1.
- Produces: `PlanState::entity_at(&self, &Position) -> Option<FactorioEntity>`; `PlanState::is_position_free(&self, &Position) -> bool`; `PlanState::create_entity(&mut self, FactorioEntity)`; `PlanState::remove_entity(&mut self, &Position)`; `PlanState::resource_available(&self, &Position, &str) -> u32`; `PlanState::consume_resource(&mut self, &Position, &str, u32) -> Result<(), PlannerError>`; `PlanState::is_researched(&self, &str) -> bool`; `PlanState::set_researched(&mut self, &str)`; `PlanState::resource_patches(&self, &str) -> Vec<ResourcePatch>`.

This is what makes reservations unnecessary: consuming ore at a tile and occupying a position are recorded here, so a second action's precondition fails against the same overlay.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` in `crates/planner/src/state.rs`:

```rust
    use factorio_bot_core::types::{FactorioEntity, Position};

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
    fn research_defaults_to_unresearched_and_can_be_set() {
        let mut a = state();
        assert!(!a.is_researched("automation"));
        a.set_researched("automation");
        assert!(a.is_researched("automation"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `resource_available`, `consume_resource`, `is_position_free`, `create_entity`, `remove_entity`, `entity_at`, `is_researched`, `set_researched`, `resource_patches` are not defined.

- [ ] **Step 3: Add the overlay fields**

First add the per-tile capacity constant near the top of the file, above `BotState`:

```rust
/// How much ore one tile yields before this plan exhausts it.
///
/// Factorio reports per-tile resource amounts, but they do not survive into the
/// entity graph: `FactorioEntity::new_resource` (`core/src/types.rs:686`) leaves
/// `amount` as `None`, and `EntityGraph::add` (`core/src/graph/entity_graph.rs:217`)
/// never inserts resource entities into the entity tree. A constant is therefore
/// the only capacity available. Honouring real amounts needs a BotBridge change and
/// is out of scope.
pub const DEFAULT_RESOURCE_PER_TILE: u32 = 500;
```

Then replace the `PlanState` struct definition and add the imports:

```rust
use factorio_bot_core::types::{FactorioEntity, Pos, Position, ResourcePatch};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Technologies the plan has completed.
    researched: BTreeSet<String>,
}
```

Update `from_world` to initialise the four new fields with `Default::default()`.

- [ ] **Step 4: Implement the overlay methods**

Append to `impl PlanState`:

```rust
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

    pub fn is_position_free(&self, position: &Position) -> bool {
        self.entity_at(position).is_none()
    }

    pub fn create_entity(&mut self, entity: FactorioEntity) {
        let key = Pos::from(&entity.position);
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

    pub fn is_researched(&self, tech: &str) -> bool {
        if self.researched.contains(tech) {
            return true;
        }
        self.base
            .forces
            .iter()
            .any(|force| matches!(force.technologies.get(tech), Some(t) if t.researched))
    }

    pub fn set_researched(&mut self, tech: &str) {
        self.researched.insert(tech.to_string());
    }

    pub fn resource_patches(&self, item: &str) -> Vec<ResourcePatch> {
        self.base.entity_graph.resource_patches(item)
    }
```

`PlannerError::InsufficientItems` carries `BotId(0)` for resource shortfalls because no bot owns a map tile; the message still names the item and the counts. If that reads badly in practice, add a dedicated variant then — not now.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 11 tests.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add entity, resource and research overlay to PlanState" -- crates/planner/src/state.rs
```

---

### Task 3: `Actor`, `Condition`, `Effect`

**Files:**
- Modify: `crates/planner/src/action.rs`, `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/action.rs`

**Interfaces:**
- Consumes: `PlanState` and its overlay methods from Tasks 1-2.
- Produces: `Actor::{Bound(BotId), Role}` with `Actor::resolve(&self, binding: BotId) -> BotId`; `Condition` with `Condition::holds(&self, &PlanState, binding: BotId) -> bool` and `Display`; `Effect` with `Effect::apply(&self, &mut PlanState, binding: BotId) -> Result<(), PlannerError>`. Re-exported from `lib.rs`.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/action.rs` with only this test module (implementation in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    #[test]
    fn role_resolves_to_the_binding_and_bound_ignores_it() {
        assert_eq!(Actor::Role.resolve(BotId(2)), BotId(2));
        assert_eq!(Actor::Bound(BotId(1)).resolve(BotId(2)), BotId(1));
    }

    #[test]
    fn has_item_checks_the_bound_actor_not_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Bound(BotId(1)),
            item: "coal".into(),
            count: 5,
        };
        // Holds for bot 1's inventory even when bot 2 is executing.
        assert!(cond.holds(&s, BotId(2)));
    }

    #[test]
    fn has_item_with_role_follows_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 5,
        };
        assert!(cond.holds(&s, BotId(1)));
        assert!(!cond.holds(&s, BotId(2)));
    }

    #[test]
    fn at_position_respects_the_radius() {
        let mut s = state();
        s.set_position(BotId(1), Position::new(0., 0.));
        let near = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 5.0,
        };
        let far = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 4.9,
        };
        assert!(near.holds(&s, BotId(1)));
        assert!(!far.holds(&s, BotId(1)));
    }

    #[test]
    fn gain_and_lose_effects_move_items() {
        let mut s = state();
        Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 4);

        Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 3,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 1);
    }

    #[test]
    fn losing_items_the_bot_lacks_is_an_error() {
        let mut s = state();
        let result = Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }
        .apply(&mut s, BotId(1));
        assert!(result.is_err());
    }

    #[test]
    fn conditions_render_for_error_messages() {
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 2,
        };
        assert_eq!(cond.to_string(), "has 2 coal");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `Actor`, `Condition`, `Effect` are not defined.

- [ ] **Step 3: Write the implementation**

Put this **above** the test module in `crates/planner/src/action.rs`:

```rust
use crate::error::PlannerError;
use crate::ids::{BotId, ItemId};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::{FactorioEntity, Position};

/// Who an action's condition or effect applies to.
///
/// `Role` means "whichever bot runs this action" and is bound by the
/// scheduler. Nothing outside `schedule()` may resolve it without an
/// explicit binding.
#[derive(Clone, Debug, PartialEq)]
pub enum Actor {
    Bound(BotId),
    Role,
}

impl Actor {
    pub fn resolve(&self, binding: BotId) -> BotId {
        match self {
            Actor::Bound(id) => *id,
            Actor::Role => binding,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    HasItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    AtPosition {
        who: Actor,
        pos: Position,
        radius: f64,
    },
    EntityAt {
        pos: Position,
        name: ItemId,
    },
    PositionFree {
        pos: Position,
    },
    Researched(String),
    ResourceAvailable {
        pos: Position,
        item: ItemId,
        count: u32,
    },
}

impl Condition {
    pub fn holds(&self, state: &PlanState, binding: BotId) -> bool {
        match self {
            Condition::HasItem { who, item, count } => {
                state.inventory_count(who.resolve(binding), item) >= *count
            }
            Condition::AtPosition { who, pos, radius } => match state.bot(who.resolve(binding)) {
                Some(bot) => calculate_distance(&bot.position, pos) <= *radius,
                None => false,
            },
            Condition::EntityAt { pos, name } => {
                matches!(state.entity_at(pos), Some(e) if &e.name == name)
            }
            Condition::PositionFree { pos } => state.is_position_free(pos),
            Condition::Researched(tech) => state.is_researched(tech),
            Condition::ResourceAvailable { pos, item, count } => {
                state.resource_available(pos, item) >= *count
            }
        }
    }

    /// The position this condition requires the acting bot to stand near, if any.
    /// The scheduler reads this to decide whether a walk is needed.
    pub fn required_position(&self) -> Option<(Position, f64)> {
        match self {
            Condition::AtPosition {
                who: Actor::Role,
                pos,
                radius,
            } => Some((pos.clone(), *radius)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Condition::HasItem { item, count, .. } => write!(f, "has {} {}", count, item),
            Condition::AtPosition { pos, radius, .. } => {
                write!(f, "within {} of {}", radius, pos)
            }
            Condition::EntityAt { pos, name } => write!(f, "{} at {}", name, pos),
            Condition::PositionFree { pos } => write!(f, "{} is free", pos),
            Condition::Researched(tech) => write!(f, "{} researched", tech),
            Condition::ResourceAvailable { pos, item, count } => {
                write!(f, "{} {} available at {}", count, item, pos)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    GainItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    LoseItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    CreateEntity(Box<FactorioEntity>),
    RemoveEntity {
        pos: Position,
    },
    ConsumeResource {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Researched(String),
}

impl Effect {
    pub fn apply(&self, state: &mut PlanState, binding: BotId) -> Result<(), PlannerError> {
        match self {
            Effect::GainItem { who, item, count } => {
                state.gain(who.resolve(binding), item, *count);
                Ok(())
            }
            Effect::LoseItem { who, item, count } => {
                state.lose(who.resolve(binding), item, *count)
            }
            Effect::CreateEntity(entity) => {
                state.create_entity((**entity).clone());
                Ok(())
            }
            Effect::RemoveEntity { pos } => {
                state.remove_entity(pos);
                Ok(())
            }
            Effect::ConsumeResource { pos, item, count } => {
                state.consume_resource(pos, item, *count)
            }
            Effect::Researched(tech) => {
                state.set_researched(tech);
                Ok(())
            }
        }
    }

    /// The item and count this effect makes available, used by dependency inference.
    pub fn produces(&self) -> Option<(&str, u32)> {
        match self {
            Effect::GainItem { item, count, .. } => Some((item.as_str(), *count)),
            _ => None,
        }
    }
}
```

There is no `MoveTo` effect. Bot movement is produced by the scheduler when it emits a walk, never by an action — see the spec's Layer 3 note.

- [ ] **Step 4: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use action::{Actor, Condition, Effect};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 18 tests.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add Actor, Condition and Effect with data-driven semantics" -- crates/planner/src
```

---

### Task 4: `Action` and `ActionNetwork`

**Files:**
- Modify: `crates/planner/src/action.rs`, `crates/planner/src/network.rs`, `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/network.rs`

**Interfaces:**
- Consumes: `Actor`, `Condition`, `Effect` from Task 3; `ActionId`, `ActionIdGen`, `Ticks` from Task 1.
- Produces: `Action { id, kind, pre, eff, duration, pinned, label }`; `ActionKind::{Mine, Craft, Place, Insert, Remove, Research}`; `Action::required_position(&self) -> Option<(Position, f64)>`; `ActionNetwork::new()`, `::add(&mut self, Action) -> ActionId`, `::link(&mut self, ActionId, ActionId, Ticks)`, `::infer_edges(&mut self)`, `::validate(&self) -> Result<(), PlannerError>`, `::actions(&self) -> impl Iterator<Item = &Action>`, `::action(&self, ActionId) -> Option<&Action>`, `::preds(&self, ActionId) -> Vec<(ActionId, Ticks)>`, `::len(&self) -> usize`, `::is_empty(&self) -> bool`, `::topo_order(&self) -> Result<Vec<ActionId>, PlannerError>`.

Edges carry a minimum lag in ticks: "satisfied, but not until *t* ticks after the producer finishes." This is what models a furnace, and it is what lets a bot do other work while one runs.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/network.rs` with only this test module:

```rust
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
        net.validate().expect("inference must not introduce a cycle");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `ActionNetwork` and `Action` are not defined.

- [ ] **Step 3: Add `Action` and `ActionKind` to `action.rs`**

First widen the id import at the top of the file, since `Action` needs two more types:

```rust
use crate::ids::{ActionId, BotId, ItemId, Ticks};
```

Then append this above the test module:

```rust
/// What a bot physically does. Carries the payload the executor needs;
/// the planner reasons from `pre` and `eff`, never from this.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionKind {
    Mine {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Craft {
        item: ItemId,
        count: u32,
    },
    Place {
        entity: Box<FactorioEntity>,
    },
    Insert {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Remove {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Research {
        tech: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub id: ActionId,
    pub kind: ActionKind,
    pub pre: Vec<Condition>,
    pub eff: Vec<Effect>,
    /// Nominal estimate. The observed duration lives in the execution log.
    pub duration: Ticks,
    /// An escape hatch for hand-tuned work. Normally `None`.
    pub pinned: Option<BotId>,
    pub label: String,
}

impl Action {
    /// Where the acting bot must stand, taken from its `AtPosition`
    /// precondition. The scheduler emits a walk to satisfy it.
    pub fn required_position(&self) -> Option<(Position, f64)> {
        self.pre.iter().find_map(|c| c.required_position())
    }
}
```

- [ ] **Step 4: Write the network implementation**

Put this above the test module in `crates/planner/src/network.rs`:

```rust
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
        if let Some(existing) = self
            .edges
            .iter_mut()
            .find(|e| e.from == from && e.to == to)
        {
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
                if self.edges.iter().any(|e| e.from == *producer && e.to == *consumer) {
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
```

- [ ] **Step 5: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use action::{Action, ActionKind};
pub use network::{ActionNetwork, Edge};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 25 tests.

- [ ] **Step 7: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add Action and ActionNetwork with lag-carrying edges" -- crates/planner/src
```

---

### Task 5: The scheduler

**Files:**
- Modify: `crates/planner/src/schedule.rs`, `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/schedule.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: `schedule(net: &ActionNetwork, state: &PlanState, bots: &[BotId]) -> Result<Schedule, PlannerError>`; `Schedule { steps: Vec<ScheduledStep>, makespan: Ticks }` with `Schedule::steps_for(&self, BotId) -> Vec<&ScheduledStep>` and `Schedule::assignment(&self, ActionId) -> Option<BotId>`; `ScheduledStep { what: StepKind, bot: BotId, start: Ticks, end: Ticks }`; `StepKind::{Act { action: ActionId, label: String }, Walk { to: Position }}`; `WALK_TILES_PER_TICK: f64`; `travel_ticks(from: &Position, to: &Position, radius: f64) -> Ticks`.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/schedule.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind, Actor, Condition, Effect};
    use crate::ids::ActionIdGen;
    use crate::network::ActionNetwork;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        let mut s = PlanState::from_world(Arc::new(fixture_world()), bots);
        for bot in bots {
            s.set_position(*bot, Position::new(0., 0.));
        }
        s
    }

    /// An action needing nothing, at the origin, taking `duration` ticks.
    fn free(gen: &mut ActionIdGen, label: &str, duration: Ticks) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration,
            pinned: None,
            label: label.into(),
        }
    }

    /// An action requiring the bot to stand within `radius` of `pos`.
    fn at(gen: &mut ActionIdGen, label: &str, pos: Position, radius: f64) -> Action {
        Action {
            id: gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius,
            }],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: label.into(),
        }
    }

    #[test]
    fn no_bots_is_an_error() {
        let net = ActionNetwork::new();
        let s = state(&[]);
        assert!(schedule(&net, &s, &[]).is_err());
    }

    #[test]
    fn an_empty_network_schedules_to_zero() {
        let net = ActionNetwork::new();
        let s = state(&[BotId(1)]);
        let result = schedule(&net, &s, &[BotId(1)]).unwrap();
        assert_eq!(result.makespan, 0);
        assert!(result.steps.is_empty());
    }

    #[test]
    fn independent_actions_run_in_parallel_on_two_bots() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut gen, "a", 100));
        net.add(free(&mut gen, "b", 100));
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 100, "two bots should overlap the work");
    }

    #[test]
    fn one_bot_serialises_the_same_actions() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut gen, "a", 100));
        net.add(free(&mut gen, "b", 100));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 200);
    }

    #[test]
    fn a_dependency_lag_delays_the_successor() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(free(&mut gen, "insert", 10));
        let b = net.add(free(&mut gen, "remove", 10));
        net.link(a, b, 200);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        // insert ends at 10, lag 200 -> remove starts no earlier than 210.
        assert_eq!(result.makespan, 220);
    }

    #[test]
    fn a_walk_is_emitted_when_the_bot_is_out_of_range() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut gen, "far", Position::new(30., 0.), 3.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert!(
            matches!(result.steps[0].what, StepKind::Walk { .. }),
            "first step should be the walk"
        );
        assert!(matches!(result.steps[1].what, StepKind::Act { .. }));
        assert!(result.makespan > 60, "walking must cost time");
    }

    #[test]
    fn no_walk_is_emitted_when_already_in_range() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut gen, "near", Position::new(1., 1.), 5.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.makespan, 60);
    }

    #[test]
    fn a_pinned_action_goes_to_its_bot() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut gen, "pinned", 60);
        action.pinned = Some(BotId(2));
        let id = net.add(action);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(id), Some(BotId(2)));
    }

    #[test]
    fn an_unsatisfiable_precondition_is_an_error() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut gen, "needs plates", 60);
        action.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        net.add(action);
        let bots = [BotId(1)];
        assert!(schedule(&net, &state(&bots), &bots).is_err());
    }

    #[test]
    fn effects_accumulate_so_a_later_action_sees_them() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut producer = free(&mut gen, "produce", 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let mut consumer = free(&mut gen, "consume", 10);
        consumer.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let p = net.add(producer);
        let c = net.add(consumer);
        net.link(p, c, 0);
        // One bot only, so the consumer sees the producer's items.
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(c), Some(BotId(1)));
    }

    #[test]
    fn scheduling_is_deterministic() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        for i in 0..6 {
            net.add(free(&mut gen, &format!("a{}", i), 40));
        }
        let bots = [BotId(1), BotId(2), BotId(3)];
        let first = schedule(&net, &state(&bots), &bots).unwrap();
        let second = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(first.steps, second.steps);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `schedule`, `Schedule`, `StepKind` are not defined.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `crates/planner/src/schedule.rs`:

```rust
use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, Ticks};
use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::Position;
use std::collections::{BTreeMap, BTreeSet};

/// Character walking speed in tiles per tick (roughly 9 tiles/second).
pub const WALK_TILES_PER_TICK: f64 = 0.15;

/// Ticks to get from `from` to within `radius` of `to`. Zero if already there.
pub fn travel_ticks(from: &Position, to: &Position, radius: f64) -> Ticks {
    let distance = calculate_distance(from, to);
    if distance <= radius {
        return 0;
    }
    ((distance - radius) / WALK_TILES_PER_TICK).ceil() as Ticks
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepKind {
    Act { action: ActionId, label: String },
    Walk { to: Position },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledStep {
    pub what: StepKind,
    pub bot: BotId,
    pub start: Ticks,
    pub end: Ticks,
}

/// An immutable assignment of actions to bots over time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    pub steps: Vec<ScheduledStep>,
    pub makespan: Ticks,
}

impl Schedule {
    pub fn steps_for(&self, bot: BotId) -> Vec<&ScheduledStep> {
        self.steps.iter().filter(|s| s.bot == bot).collect()
    }

    pub fn assignment(&self, action: ActionId) -> Option<BotId> {
        self.steps.iter().find_map(|s| match &s.what {
            StepKind::Act { action: id, .. } if *id == action => Some(s.bot),
            _ => None,
        })
    }
}

struct Candidate {
    action: ActionId,
    bot: BotId,
    start: Ticks,
    travel: Ticks,
    end: Ticks,
}

/// Assign every action in `net` to one of `bots`, travel-aware and greedy.
///
/// A pure function of its three arguments. Ties break on `(ActionId, BotId)`
/// ascending, so the output is stable across runs.
pub fn schedule(
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
) -> Result<Schedule, PlannerError> {
    if bots.is_empty() {
        return Err(PlannerError::NoBots);
    }
    net.validate()?;

    let mut sim = state.fork();
    let mut free_at: BTreeMap<BotId, Ticks> = bots.iter().map(|b| (*b, 0)).collect();
    let mut finished: BTreeMap<ActionId, Ticks> = BTreeMap::new();
    let mut done: BTreeSet<ActionId> = BTreeSet::new();
    let mut steps: Vec<ScheduledStep> = Vec::new();

    while done.len() < net.len() {
        let ready: Vec<&crate::action::Action> = net
            .actions()
            .filter(|a| !done.contains(&a.id))
            .filter(|a| net.preds(a.id).iter().all(|(p, _)| done.contains(p)))
            .collect();

        let stuck = net
            .actions()
            .find(|a| !done.contains(&a.id))
            .map(|a| a.id);
        if ready.is_empty() {
            return Err(PlannerError::Deadlock {
                action: stuck.expect("loop condition guarantees an unfinished action"),
            });
        }

        let mut best: Option<Candidate> = None;
        for action in &ready {
            let deps_ready = net
                .preds(action.id)
                .iter()
                .map(|(p, lag)| finished[p] + lag)
                .max()
                .unwrap_or(0);

            let candidate_bots: Vec<BotId> = match action.pinned {
                Some(pinned) if bots.contains(&pinned) => vec![pinned],
                Some(pinned) => return Err(PlannerError::UnknownBot(pinned)),
                None => bots.to_vec(),
            };

            for bot in candidate_bots {
                let from = &sim.bot(bot).ok_or(PlannerError::UnknownBot(bot))?.position;
                let travel = match action.required_position() {
                    Some((ref pos, radius)) => travel_ticks(from, pos, radius),
                    None => 0,
                };
                let start = free_at[&bot].max(deps_ready);
                let end = start + travel + action.duration;
                let better = match &best {
                    None => true,
                    Some(b) => (end, action.id, bot) < (b.end, b.action, b.bot),
                };
                if better {
                    best = Some(Candidate {
                        action: action.id,
                        bot,
                        start,
                        travel,
                        end,
                    });
                }
            }
        }

        let chosen = best.expect("ready is non-empty and bots is non-empty");
        let action = net
            .action(chosen.action)
            .expect("candidate came from this network");

        // Walk first, so the AtPosition precondition can hold when checked.
        if chosen.travel > 0 {
            let (target, _) = action
                .required_position()
                .expect("travel is non-zero only when a position is required");
            steps.push(ScheduledStep {
                what: StepKind::Walk {
                    to: target.clone(),
                },
                bot: chosen.bot,
                start: chosen.start,
                end: chosen.start + chosen.travel,
            });
            sim.set_position(chosen.bot, target);
        }

        for condition in &action.pre {
            if !condition.holds(&sim, chosen.bot) {
                return Err(PlannerError::PreconditionUnsatisfied {
                    action: action.id,
                    bot: chosen.bot,
                    condition: condition.to_string(),
                });
            }
        }

        for effect in &action.eff {
            effect.apply(&mut sim, chosen.bot)?;
        }

        steps.push(ScheduledStep {
            what: StepKind::Act {
                action: action.id,
                label: action.label.clone(),
            },
            bot: chosen.bot,
            start: chosen.start + chosen.travel,
            end: chosen.end,
        });

        free_at.insert(chosen.bot, chosen.end);
        finished.insert(chosen.action, chosen.end);
        done.insert(chosen.action);
    }

    let makespan = steps.iter().map(|s| s.end).max().unwrap_or(0);
    Ok(Schedule { steps, makespan })
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use schedule::{schedule, travel_ticks, ScheduledStep, Schedule, StepKind, WALK_TILES_PER_TICK};
```

The test module's `use super::*;` picks up `BotId`, `Ticks` and `PlanState` from the implementation's import block, so it needs no id imports of its own.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 36 tests.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add travel-aware greedy scheduler" -- crates/planner/src
```

---

### Task 6: Scheduler properties

**Files:**
- Create: `crates/planner/tests/scheduling.rs`

**Interfaces:**
- Consumes: the full public API from Tasks 1-5.
- Produces: nothing new. This task exists to pin the guarantees the spec promised, including the two that the omitted `Reservations` structure was meant to provide.

**This task is not TDD and has no RED phase.** Every behaviour it tests is already implemented by Tasks 1-5; these tests exist to pin guarantees, not to drive new code. They are expected to pass on the first run. **If any of them fails, that is a real defect in Tasks 1-5 — report it and stop. Do not adjust an assertion to match observed output.**

- [ ] **Step 1: Write the property tests**

Create `crates/planner/tests/scheduling.rs`. Note that `factorio-bot-core` is already a regular dependency of the planner crate, and Cargo makes `[dependencies]` available to integration-test targets, so no manifest change is needed:

```rust
use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect};
use factorio_bot_planner::ids::ActionIdGen;
use factorio_bot_planner::{
    schedule, ActionNetwork, BotId, PlanState, PlannerError, StepKind, Ticks,
};
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, Position};
use std::sync::Arc;

fn state(bots: &[BotId]) -> PlanState {
    let mut s = PlanState::from_world(Arc::new(fixture_world()), bots);
    for bot in bots {
        s.set_position(*bot, Position::new(0., 0.));
    }
    s
}

fn ore_tile(s: &PlanState) -> Position {
    s.resource_patches("iron-ore")
        .first()
        .expect("fixture has iron ore")
        .elements
        .first()
        .expect("patch has tiles")
        .clone()
}

/// Mine `count` ore from `pos`, consuming it from the map.
fn mine_at(gen: &mut ActionIdGen, pos: &Position, count: u32) -> Action {
    Action {
        id: gen.next(),
        kind: ActionKind::Mine {
            pos: pos.clone(),
            item: "iron-ore".into(),
            count,
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 3.0,
            },
            Condition::ResourceAvailable {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count,
            },
        ],
        eff: vec![
            Effect::ConsumeResource {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count,
            },
            Effect::GainItem {
                who: Actor::Role,
                item: "iron-ore".into(),
                count,
            },
        ],
        duration: 60,
        pinned: None,
        label: format!("mine {} iron-ore", count),
    }
}

/// Place a furnace at `pos`, requiring the tile to be free.
fn place_at(gen: &mut ActionIdGen, pos: &Position) -> Action {
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
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 10.0,
            },
            Condition::PositionFree { pos: pos.clone() },
        ],
        eff: vec![Effect::CreateEntity(Box::new(furnace))],
        duration: 30,
        pinned: None,
        label: "place stone-furnace".into(),
    }
}

fn free(gen: &mut ActionIdGen, duration: Ticks) -> Action {
    Action {
        id: gen.next(),
        kind: ActionKind::Craft {
            item: "iron-gear-wheel".into(),
            count: 1,
        },
        pre: vec![],
        eff: vec![],
        duration,
        pinned: None,
        label: "craft".into(),
    }
}

#[test]
fn two_bots_cannot_mine_the_same_exhausted_tile() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);
    let available = s.resource_available(&pos, "iron-ore");
    assert!(available > 0);

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    // Together these demand more ore than the tile holds.
    net.add(mine_at(&mut gen, &pos, available));
    net.add(mine_at(&mut gen, &pos, 1));

    let result = schedule(&net, &s, &bots);
    assert!(
        matches!(result, Err(PlannerError::PreconditionUnsatisfied { .. })),
        "the second miner must fail on an exhausted tile, got {:?}",
        result.map(|s| s.makespan)
    );
}

#[test]
fn two_bots_cannot_place_at_the_same_position() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = Position::new(5., 5.);

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(place_at(&mut gen, &pos));
    net.add(place_at(&mut gen, &pos));

    let result = schedule(&net, &s, &bots);
    assert!(
        matches!(result, Err(PlannerError::PreconditionUnsatisfied { .. })),
        "the second placement must fail on an occupied tile"
    );
}

#[test]
fn every_precondition_holds_at_its_scheduled_time() {
    // Replaying the schedule step by step must never hit a false precondition.
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut gen, &pos, 1));
    net.add(mine_at(&mut gen, &pos, 1));
    net.add(place_at(&mut gen, &Position::new(5., 5.)));

    let result = schedule(&net, &s, &bots).expect("schedulable");

    let mut replay = s.fork();
    for step in &result.steps {
        match &step.what {
            StepKind::Walk { to } => replay.set_position(step.bot, to.clone()),
            StepKind::Act { action, .. } => {
                let a = net.action(*action).expect("action exists");
                for condition in &a.pre {
                    assert!(
                        condition.holds(&replay, step.bot),
                        "precondition `{}` of `{}` failed on replay",
                        condition,
                        a.label
                    );
                }
                for effect in &a.eff {
                    effect.apply(&mut replay, step.bot).expect("effect applies");
                }
            }
        }
    }
}

#[test]
fn more_bots_never_increase_the_makespan() {
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    for _ in 0..12 {
        net.add(free(&mut gen, 50));
    }

    let mut previous = Ticks::MAX;
    for count in 1..=4u8 {
        let bots: Vec<BotId> = (1..=count).map(BotId).collect();
        let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
        assert!(
            result.makespan <= previous,
            "{} bots gave makespan {} against {} for {} bots",
            count,
            result.makespan,
            previous,
            count - 1
        );
        previous = result.makespan;
    }
}

#[test]
fn a_cyclic_network_is_rejected_rather_than_looping() {
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let a = net.add(free(&mut gen, 10));
    let b = net.add(free(&mut gen, 10));
    net.link(a, b, 0);
    net.link(b, a, 0);

    let bots = [BotId(1)];
    assert!(matches!(
        schedule(&net, &state(&bots), &bots),
        Err(PlannerError::CyclicNetwork(_))
    ));
}

#[test]
fn scheduling_terminates_on_a_deep_chain() {
    // A 200-long chain must not blow up or hang.
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let mut previous = net.add(free(&mut gen, 1));
    for _ in 0..199 {
        let next = net.add(free(&mut gen, 1));
        net.link(previous, next, 0);
        previous = next;
    }
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
    assert_eq!(result.makespan, 200, "a chain cannot be parallelised");
}
```

- [ ] **Step 2: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner --test scheduling'`
Expected: PASS — 6 tests.

If `more_bots_never_increase_the_makespan` fails, the greedy selection is at fault, not the test: the property is a genuine requirement. The likely cause is the tie-break comparing `end` before considering that a bot idle since tick 0 should win over one that just became free; verify the `(end, action.id, bot)` tuple ordering in Task 5 step 3 is exactly as written.

- [ ] **Step 3: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "test(planner): pin scheduler guarantees as properties" -- crates/planner
```

---

### Task 7: Rendering

**Files:**
- Modify: `crates/planner/src/render.rs`, `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/render.rs`

**Interfaces:**
- Consumes: `Schedule`, `ScheduledStep`, `StepKind`, `BotId`, `Ticks`.
- Produces: `mermaid_gantt(schedule: &Schedule, title: &str) -> String`; `graphviz(net: &ActionNetwork) -> String`; `ticks_to_timestamp(ticks: Ticks) -> String`.

`render` takes `&Schedule` and nothing else. When plan 3 adds `ExecutionLog`, it becomes a second optional argument; the signature is designed for that.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/render.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ActionId;
    use crate::schedule::{Schedule, ScheduledStep, StepKind};
    use factorio_bot_core::types::Position;

    fn schedule() -> Schedule {
        Schedule {
            steps: vec![
                ScheduledStep {
                    what: StepKind::Walk {
                        to: Position::new(10., 0.),
                    },
                    bot: BotId(1),
                    start: 0,
                    end: 60,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(0),
                        label: "mine 5 iron-ore".into(),
                    },
                    bot: BotId(1),
                    start: 60,
                    end: 360,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(1),
                        label: "craft iron-gear-wheel".into(),
                    },
                    bot: BotId(2),
                    start: 0,
                    end: 120,
                },
            ],
            makespan: 360,
        }
    }

    #[test]
    fn ticks_render_as_hours_minutes_seconds() {
        assert_eq!(ticks_to_timestamp(0), "00:00:00");
        assert_eq!(ticks_to_timestamp(60), "00:00:01");
        assert_eq!(ticks_to_timestamp(3600), "00:01:00");
        assert_eq!(ticks_to_timestamp(216_000), "01:00:00");
    }

    #[test]
    fn the_gantt_has_a_section_per_bot() {
        let out = mermaid_gantt(&schedule(), "Test");
        assert!(out.starts_with("gantt"));
        assert!(out.contains("title Test"));
        assert!(out.contains("section bot 1"));
        assert!(out.contains("section bot 2"));
    }

    #[test]
    fn the_gantt_names_every_action_and_walk() {
        let out = mermaid_gantt(&schedule(), "Test");
        assert!(out.contains("mine 5 iron-ore"));
        assert!(out.contains("craft iron-gear-wheel"));
        assert!(out.contains("walk to [10, 0]"));
    }

    #[test]
    fn the_gantt_places_steps_at_their_start_time() {
        let out = mermaid_gantt(&schedule(), "Test");
        // Bot 1's walk is its first step (a1), the mining act its second (a2):
        // ids are `bot_index * 1000 + step_index + 1`.
        assert!(
            out.contains("walk to [10, 0] :a1, 00:00:00, 1s"),
            "unexpected gantt body:\n{}",
            out
        );
        // The mining step starts one second in and runs for five.
        assert!(
            out.contains("mine 5 iron-ore :a2, 00:00:01, 5s"),
            "unexpected gantt body:\n{}",
            out
        );
        // Bot 2's only step opens a fresh id block.
        assert!(
            out.contains("craft iron-gear-wheel :a1001, 00:00:00, 2s"),
            "unexpected gantt body:\n{}",
            out
        );
    }

    #[test]
    fn an_empty_schedule_still_renders_a_valid_chart() {
        let out = mermaid_gantt(&Schedule::default(), "Empty");
        assert!(out.starts_with("gantt"));
        assert!(out.contains("title Empty"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `mermaid_gantt` and `ticks_to_timestamp` are not defined.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `crates/planner/src/render.rs`:

```rust
use crate::ids::{BotId, Ticks};
use crate::network::ActionNetwork;
use crate::schedule::{Schedule, StepKind};
use std::collections::BTreeSet;
use std::fmt::Write;

const TICKS_PER_SECOND: Ticks = 60;

pub fn ticks_to_timestamp(ticks: Ticks) -> String {
    let total_seconds = ticks / TICKS_PER_SECOND;
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

/// A Mermaid Gantt chart, one section per bot.
pub fn mermaid_gantt(schedule: &Schedule, title: &str) -> String {
    let mut out = String::new();
    out.push_str("gantt\n");
    let _ = writeln!(out, "    title {}", title);
    out.push_str("    dateFormat HH:mm:ss\n");
    out.push_str("    axisFormat %H:%M:%S\n");

    let bots: BTreeSet<BotId> = schedule.steps.iter().map(|s| s.bot).collect();
    for (bot_index, bot) in bots.iter().enumerate() {
        let _ = writeln!(out, "    section {}", bot);
        for (step_index, step) in schedule.steps_for(*bot).iter().enumerate() {
            let label = match &step.what {
                StepKind::Act { label, .. } => label.clone(),
                StepKind::Walk { to } => format!("walk to {}", to),
            };
            let seconds = (step.end.saturating_sub(step.start)) / TICKS_PER_SECOND;
            let _ = writeln!(
                out,
                "    {} :a{}, {}, {}s",
                label,
                bot_index * 1000 + step_index + 1,
                ticks_to_timestamp(step.start),
                seconds
            );
        }
    }
    out
}

/// The action network as a graphviz digraph, edges labelled with their lag.
pub fn graphviz(net: &ActionNetwork) -> String {
    let mut out = String::from("digraph {\n");
    for action in net.actions() {
        let _ = writeln!(
            out,
            "    {} [label=\"{}\"];",
            action.id.0,
            action.label.replace('"', "'")
        );
    }
    for action in net.actions() {
        for (pred, lag) in net.preds(action.id) {
            let _ = writeln!(
                out,
                "    {} -> {} [label=\"{}t\"];",
                pred.0, action.id.0, lag
            );
        }
    }
    out.push_str("}\n");
    out
}
```

The `a{}` step identifiers use `bot_index * 1000 + step_index + 1` so ids stay unique across sections without a shared counter. A bot with more than 1000 steps would collide; if that ever happens, switch to a running counter.

- [ ] **Step 4: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use render::{graphviz, mermaid_gantt, ticks_to_timestamp};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 44 unit tests plus 6 integration tests. (Counts assume Task 4's fix round added three network tests; if a task added others, the total shifts and that is fine. Never adjust an assertion to match an observed count — report the discrepancy instead.)

- [ ] **Step 6: Run the full workspace suite**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test --workspace'`
Expected: PASS. The existing tests are untouched by this plan — nothing in `core`, `scripting_lua` or `server` was modified.

The workspace suite rewrites `crates/scripting_lua/tests/task_graph-1.dot`, `task_graph-1.md` and three `.png` snapshots on every run. They are stale in the repo and the churn is not yours. Discard it before committing:

```bash
git checkout -- crates/scripting_lua/tests/
```

Another agent is working in `app/**` in this same checkout. **Only ever `git add` explicit paths** — never `git add -A` or `git add .`.

- [ ] **Step 7: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): render schedules as Mermaid Gantt and graphviz" -- crates/planner/src
```

---

## Self-Review

**Spec coverage.** Layer 0 `PlanState` with cheap forking (T1) and the world overlay (T2). Layer 3 `Actor`, `Condition`, `Effect` as data (T3) and `Action`/`ActionKind` (T4). Ordering inference with quantities and direction, plus lag edges for machine processes (T4). Layer 4 `schedule()` as a pure function, travel-aware, walk-emitting, honouring `pinned`, deterministic (T5). The spec's four named properties — preconditions hold at their scheduled time, no double-claiming, makespan monotone in bot count, scheduling terminates — are T6. Gantt rendering from `Schedule` (T7).

Spec items deliberately left to plans 2 and 3: `Goal`, `Holder`, `Method` and the expansion driver; every concrete method including `SplitAcrossBots`, `Craft`, `Smelt` and `Consolidate`; crafting and smelting latency calibration; `ExecutionLog` and the executor; the Lua API; deletion of `core/plan` and `graph/task_graph.rs`.

**Deviations from the spec, flagged.**

1. **`Reservations` is omitted**, with the rationale and the preserved guarantees documented above and property-tested in T6. The structure returns in plan 3 if in-flight actions during re-scheduling need it.
2. **`Condition::PositionFree` was added**, which the spec did not list. Without it, "two bots cannot place at the same position" has no way to be expressed as a precondition, and that guarantee is one the spec explicitly asks for.
3. **`Effect::MoveTo` was dropped.** The spec lists it, but the scheduler sets bot position directly when it emits a walk, and no action may move a bot — that is the whole point of removing `Walk` from `ActionKind`. Keeping a `MoveTo` effect would reopen the hole.
4. **`Goal::Built` is not defined anywhere in this plan**, since goals arrive in plan 2. When plan 2 defines `Goal`, `Built` should be omitted until blueprint work makes `BlueprintRef` and `Placement` real; `Producing { item, rate }` should be included, as the spec's documented functorio bridge, with a test asserting it has no applicable method yet.

**Placeholder scan.** No TBD or TODO entries. Every code step carries the code. Every test step carries the assertions. The one judgement call left to the implementer — whether `PlannerError::InsufficientItems` carrying `BotId(0)` for map resources reads badly enough to warrant its own variant — is stated with a decision (leave it) rather than deferred.

**Type consistency.** `PlanState::{from_world, fork, base, bot, bot_ids, inventory_count, total_count, gain, lose, set_position, entity_at, is_position_free, create_entity, remove_entity, resource_available, consume_resource, is_researched, set_researched, resource_patches}` are used with identical signatures in T2 through T7. `Condition::holds(&PlanState, BotId) -> bool` and `Effect::apply(&mut PlanState, BotId) -> Result<(), PlannerError>` are unchanged from T3 onward. `Action { id, kind, pre, eff, duration, pinned, label }` has the same seven fields in T4, T5 and T6. `schedule(&ActionNetwork, &PlanState, &[BotId])` has one signature throughout. `StepKind::Act` carries `{ action, label }` in T5, T6 and T7 alike.

**Import ordering across tasks.** `action.rs` is written by T3 and extended by T4. T3 imports only what T3 uses (`BotId`, `ItemId`, `Position`, `FactorioEntity`, `PlannerError`, `PlanState`, `calculate_distance`); T4 step 3 widens the id import to add `ActionId` and `Ticks` as its first action. Each task therefore passes `clippy --deny warnings` on its own, which the global constraints require.
