# Planner Execution Increment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn a `Schedule` into real Factorio gameplay — bots that walk, mine, craft, smelt and place under the planner's schedule — and retire the old `TaskGraph` planner it replaces.

**Architecture:** A new `crates/executor` crate sits above both `core` and `planner` (the only place that may depend on both, since `planner -> core` already). It owns an `Actuator` trait that names the six things a bot can be told to do, an `RconActuator` that implements it against `FactorioRcon`, and a per-bot async task that walks that bot's slice of the `Schedule`. Execution state lives in an `ExecutionLog` keyed by `ActionId`, never inside the plan, so `Schedule` stays an immutable value and estimated-versus-actual is a join. Failure is handled in tiers: re-schedule from observed state, then re-expand, then surface.

**Tech Stack:** Rust 2021, tokio (executor crate only), `async_trait`, `mockall` (executor's own mock — see Global Constraints), miette, mlua.

**Spec:** `docs/superpowers/specs/2026-08-29-multi-agent-planner-design.md` (Layer 5 — `Executor`)

## Global Constraints

- **`crates/planner` stays pure and deterministic.** No tokio, no async, no I/O, no wall-clock. Ordered collections only: `BTreeMap` / `BTreeSet` / `Vec` — never `HashMap` / `HashSet`. Float comparison via `total_cmp`. This constraint has been enforced across three prior plans; do not weaken it to make execution easier. All async lives in `crates/executor`.
- **`MockFactorioRcon` is NOT available downstream.** `FactorioRcon` carries `#[cfg_attr(test, mockall::automock)]`, which is gated on *core's own* `cfg(test)`. Other crates cannot see it. The executor therefore defines its own `Actuator` trait and its own `mockall::mock!` double. Do not attempt to import `MockFactorioRcon`.
- **Never hardcode `defines.inventory` integers.** Their meaning is entity-type dependent (see `mods/BotBridge/control.lua:66` `inventory_type_name(invtype, enttype)`) and they vary across Factorio versions. Resolve them from the running game once, as specified in Task 3.
- **A second agent is working in this repo concurrently** (dependency/stack modernization, mostly `app/` and `crates/server`). Therefore: commit with explicit paths — `git commit -m "..." -- <paths>` — never a bare `git commit` or `git add -A`, which would sweep their staged work into your commit. Run `cargo fmt` scoped: `cargo fmt -p <crate>`, never `--all`.
- **Branch:** work on `master`, committing directly, per the repository owner's instruction.
- Conventional commit messages. Every task ends with at least one commit.
- **Running the workspace suite dirties seven files you must not commit.** `crates/scripting_lua/tests/` holds `task_graph-{1,2}.{dot,md}` and three `.png` files that the fixture script rewrites on every run. Nothing reads or compares them (see Task 8). After any `cargo test --workspace`, `git status` will show them modified — revert with `git checkout -- crates/scripting_lua/tests/` and never include them in a commit. This matters most in Task 7, whose commit path is `-- crates/scripting_lua` and would otherwise sweep them in.
- Verification command for the whole workspace: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` then `cargo test --workspace`.

## Sign-off — granted

The repository owner approved all eight tasks on 2026-08-30, including Task 8's deletion of `crates/core/src/plan/`, `crates/core/src/graph/task_graph.rs`, and the published Lua `plan.*` API, plus migration of the six scripts under `scripts/`.

Three design decisions were made at the same time and are folded into the tasks below. They override anything in an earlier draft of this plan:

1. **The executor discovers its own bots.** `RconActuator` asks the game which players are connected and assigns `BotId(0..n)` in ascending player-id order. No caller supplies a mapping. (Task 3)
2. **`goal.execute` is handle-based, not blocking.** It returns a handle immediately; `goal.wait(h)` blocks for completion and `goal.progress(h)` returns a live snapshot. This is what forces the execution log to be shared state rather than a value merged at the end. (Tasks 5 and 7)
3. **Manual grouping is dropped.** There is no `goal.group`; the planner's method expansion is the only source of grouping. `plan.group_start` / `plan.group_end` disappear with the old API in Task 8. (Task 7)

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/planner/src/action.rs` (modify) | `InventorySlot` enum; `Insert`/`Remove` gain `entity` + `slot` |
| `crates/planner/src/method/have.rs` (modify) | `Smelt` fills the new fields |
| `crates/executor/Cargo.toml` (create) | New crate; depends on core + planner + tokio |
| `crates/executor/src/lib.rs` (create) | Public surface |
| `crates/executor/src/log.rs` (create) | `ExecutionLog`, `Attempt`, `Status` — pure data |
| `crates/executor/src/actuator.rs` (create) | `Actuator` trait, `ActuatorError` |
| `crates/executor/src/rcon_actuator.rs` (create) | `RconActuator`, `InventoryDefines` resolution |
| `crates/executor/src/run.rs` (create) | Per-bot loop, multi-bot orchestration, completion signals |
| `crates/executor/src/recover.rs` (create) | Failure tiers 1 and 2 |
| `crates/scripting_lua/src/globals/goal.rs` (create) | New Lua `goal.*` table |
| `crates/core/src/plan/` (delete, Task 8) | Old planner |
| `crates/core/src/graph/task_graph.rs` (delete, Task 8) | Old task graph |

---

### Task 1: Make the action model executable

The planner's `ActionKind::Insert` and `ActionKind::Remove` carry `{ pos, item, count }`. RCON's `insert_to_inventory` needs `entity_name` and `inventory_type` as well. Today those two variants cannot be executed at all. This task closes that gap in the planner crate — no async, no I/O.

**Files:**
- Modify: `crates/planner/src/action.rs`
- Modify: `crates/planner/src/method/have.rs`
- Modify: `crates/planner/src/lib.rs` (re-export `InventorySlot`)
- Test: `crates/planner/src/action.rs` (inline `#[cfg(test)]`), `crates/planner/tests/red_science.rs`

**Interfaces:**
- Produces: `planner::InventorySlot`; `ActionKind::Insert { pos, entity, slot, item, count }`; `ActionKind::Remove { pos, entity, slot, item, count }`. Task 3 maps `InventorySlot` to a numeric inventory id; Task 4 dispatches on these variants.

- [ ] **Step 1: Write the failing test**

In `crates/planner/src/action.rs`, inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn an_insert_names_the_entity_and_slot_it_targets() {
    let a = ActionKind::Insert {
        pos: Position::new(3.0, 4.0),
        entity: "stone-furnace".to_string(),
        slot: InventorySlot::FurnaceSource,
        item: ItemId::new("iron-ore"),
        count: 8,
    };
    match a {
        ActionKind::Insert { entity, slot, .. } => {
            assert_eq!(entity, "stone-furnace");
            assert_eq!(slot, InventorySlot::FurnaceSource);
        }
        _ => panic!("expected Insert"),
    }
}

#[test]
fn inventory_slots_order_deterministically() {
    let mut v = vec![
        InventorySlot::FurnaceResult,
        InventorySlot::Chest,
        InventorySlot::FurnaceSource,
    ];
    v.sort();
    assert_eq!(
        v,
        vec![
            InventorySlot::Chest,
            InventorySlot::FurnaceSource,
            InventorySlot::FurnaceResult
        ]
    );
}
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-planner an_insert_names_the_entity`
Expected: FAIL — `InventorySlot` not found, and `Insert` has no field `entity`.

- [ ] **Step 3: Add the enum**

In `crates/planner/src/action.rs`, above `ActionKind`:

```rust
/// Which inventory of a target entity an insert or remove addresses.
///
/// Deliberately semantic rather than numeric. Factorio's `defines.inventory`
/// integers are entity-type dependent — the same number means different things
/// for a chest and a furnace (see `mods/BotBridge/control.lua`
/// `inventory_type_name(invtype, enttype)`) — and they move between game
/// versions. The executor resolves these to numbers against the running game.
///
/// Ordering is derived and load-bearing: the planner is deterministic, so every
/// type reachable from an `ActionNetwork` must order totally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InventorySlot {
    Chest,
    FurnaceSource,
    FurnaceResult,
    Fuel,
    AssemblerInput,
    AssemblerOutput,
    LabInput,
}

impl InventorySlot {
    /// The name used to look this slot up in the game's `defines.inventory`.
    pub fn defines_key(self) -> &'static str {
        match self {
            InventorySlot::Chest => "chest",
            InventorySlot::FurnaceSource => "furnace_source",
            InventorySlot::FurnaceResult => "furnace_result",
            InventorySlot::Fuel => "fuel",
            InventorySlot::AssemblerInput => "assembling_machine_input",
            InventorySlot::AssemblerOutput => "assembling_machine_output",
            InventorySlot::LabInput => "lab_input",
        }
    }
}
```

- [ ] **Step 4: Widen the two variants**

In the same file, replace the `Insert` and `Remove` variants of `ActionKind`:

```rust
    Insert {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
    Remove {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
```

- [ ] **Step 5: Fix every construction site**

Run: `cargo check -p factorio-bot-planner 2>&1 | grep -E "^error" -A5`

`Smelt` in `crates/planner/src/method/have.rs` is the only method that builds these. It smelts in a `stone-furnace`: the ore goes into `FurnaceSource`, the plate comes out of `FurnaceResult`. Fill both new fields at each construction site accordingly, with `entity: "stone-furnace".to_string()`.

Do not invent a furnace kind that the method does not already choose. If `Smelt` already selects a furnace entity name, reuse that binding rather than hardcoding a second copy of the string.

- [ ] **Step 6: Re-export and run the suite**

Add `InventorySlot` to the `pub use action::{...}` line in `crates/planner/src/lib.rs`.

Run: `cargo test -p factorio-bot-planner`
Expected: PASS, all of it. The red-science integration tests must still pass **with their existing makespan assertions unchanged** — this task adds fields, it does not change scheduling. If a makespan assertion moves, stop and report: something other than data shape changed.

- [ ] **Step 7: Commit**

```bash
cargo fmt -p factorio-bot-planner
cargo clippy -p factorio-bot-planner --all-targets -- --deny warnings
git commit -m "feat(planner): name the target entity and inventory slot on insert and remove" -- crates/planner
```

---

### Task 2: The executor crate and its execution log

**Files:**
- Create: `crates/executor/Cargo.toml`, `crates/executor/src/lib.rs`, `crates/executor/src/log.rs`
- Modify: `Cargo.toml` (workspace members)
- Test: inline `#[cfg(test)]` in `crates/executor/src/log.rs`

**Interfaces:**
- Produces: `ExecutionLog`, `Attempt`, `Status`. Task 4 writes to the log; Task 6 reads it to decide recovery.

- [ ] **Step 1: Create the crate**

`crates/executor/Cargo.toml`:

```toml
[package]
name = "factorio-bot-executor"
version = "0.2.4-dev"
edition = "2021"

[dependencies]
factorio-bot-core = { path = "../core", version = "0.2.4-dev" }
factorio-bot-planner = { path = "../planner", version = "0.2.4-dev" }
async-trait = "0.1"
miette = { version = "7.4", features = ["fancy"] }
thiserror = "2.0"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt", "sync", "time"] }

[dev-dependencies]
mockall = "0.13"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "test-util"] }
```

Add `"crates/executor"` to `members` in the workspace `Cargo.toml`, after `"crates/planner"`.

- [ ] **Step 2: Write the failing test**

`crates/executor/src/log.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_planner::ActionId;

    fn id(n: u32) -> ActionId {
        ActionId(n)
    }

    #[test]
    fn an_unrecorded_action_is_pending() {
        let log = ExecutionLog::default();
        assert_eq!(log.status(id(1)), Status::Pending);
    }

    #[test]
    fn starting_then_finishing_records_a_duration() {
        let mut log = ExecutionLog::default();
        log.start(id(1), 100);
        assert_eq!(log.status(id(1)), Status::Running);
        log.succeed(id(1), 340);
        assert_eq!(log.status(id(1)), Status::Success);
        assert_eq!(log.observed_duration(id(1)), Some(240));
    }

    #[test]
    fn a_failure_keeps_its_message() {
        let mut log = ExecutionLog::default();
        log.start(id(7), 10);
        log.fail(id(7), 20, "cannot reach target".to_string());
        assert_eq!(log.status(id(7)), Status::Failed);
        assert_eq!(
            log.attempt(id(7)).and_then(|a| a.error.as_deref()),
            Some("cannot reach target")
        );
    }

    #[test]
    fn failed_actions_are_listed_in_id_order() {
        let mut log = ExecutionLog::default();
        for n in [9u32, 3, 5] {
            log.start(id(n), 0);
            log.fail(id(n), 1, "x".to_string());
        }
        assert_eq!(log.failed(), vec![id(3), id(5), id(9)]);
    }
}
```

`ActionId` is a public tuple struct (`pub struct ActionId(pub u32)` in `crates/planner/src/ids.rs`), so `ActionId(n)` constructs one directly. No new constructor is needed.

- [ ] **Step 3: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor`
Expected: FAIL — the module does not exist yet.

- [ ] **Step 4: Implement the log**

Top of `crates/executor/src/log.rs`:

```rust
use factorio_bot_planner::{ActionId, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Running,
    Success,
    Failed,
}

/// One execution attempt of one action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub status: Status,
    pub started_tick: Ticks,
    pub ended_tick: Option<Ticks>,
    pub error: Option<String>,
}

/// Observed execution state, keyed by action.
///
/// Deliberately separate from `Schedule`: the schedule is an immutable plan
/// value, and estimated-versus-actual is a join over these two, not a mutation
/// of the plan. `BTreeMap` because iteration order is part of the contract —
/// `failed()` returns ids in a stable order so recovery is reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLog {
    attempts: BTreeMap<ActionId, Attempt>,
}

impl ExecutionLog {
    pub fn attempt(&self, id: ActionId) -> Option<&Attempt> {
        self.attempts.get(&id)
    }

    pub fn status(&self, id: ActionId) -> Status {
        self.attempts.get(&id).map_or(Status::Pending, |a| a.status)
    }

    pub fn start(&mut self, id: ActionId, tick: Ticks) {
        self.attempts.insert(
            id,
            Attempt {
                status: Status::Running,
                started_tick: tick,
                ended_tick: None,
                error: None,
            },
        );
    }

    pub fn succeed(&mut self, id: ActionId, tick: Ticks) {
        if let Some(a) = self.attempts.get_mut(&id) {
            a.status = Status::Success;
            a.ended_tick = Some(tick);
        }
    }

    pub fn fail(&mut self, id: ActionId, tick: Ticks, error: String) {
        if let Some(a) = self.attempts.get_mut(&id) {
            a.status = Status::Failed;
            a.ended_tick = Some(tick);
            a.error = Some(error);
        }
    }

    /// Wall-clock ticks the action actually took, once finished.
    pub fn observed_duration(&self, id: ActionId) -> Option<Ticks> {
        let a = self.attempts.get(&id)?;
        a.ended_tick.map(|end| end.saturating_sub(a.started_tick))
    }

    /// Failed action ids, in ascending id order.
    pub fn failed(&self) -> Vec<ActionId> {
        self.attempts
            .iter()
            .filter(|(_, a)| a.status == Status::Failed)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }
}
```

`crates/executor/src/lib.rs`:

```rust
pub mod log;

pub use log::{Attempt, ExecutionLog, Status};
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p factorio-bot-executor`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
cargo fmt -p factorio-bot-executor
cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings
git commit -m "feat(executor): add the execution log keyed by action id" -- crates/executor Cargo.toml crates/planner
```

---

### Task 3: The actuator trait and its RCON implementation

**Files:**
- Create: `crates/executor/src/actuator.rs`, `crates/executor/src/rcon_actuator.rs`
- Modify: `crates/executor/src/lib.rs`
- Test: inline in both new files

**Interfaces:**
- Consumes: `planner::InventorySlot` (Task 1).
- Produces: `trait Actuator`, `ActuatorError`, `RconActuator::new(rcon, world) -> Result<Self, ActuatorError>`. Task 4 is generic over `Actuator`.
- Produces (in core): `FactorioRcon::connected_players() -> Result<Vec<FactorioPlayer>>`.

**Why a trait at all:** `FactorioRcon`'s `automock` is gated on core's own `cfg(test)` and is invisible here. The trait is what makes the run loop testable without a game, and it also normalizes RCON's inconsistent argument order (`world` is the first parameter of `move_player` and the last of `place_entity`).

- [ ] **Step 1: Write the trait**

`crates/executor/src/actuator.rs`:

```rust
use async_trait::async_trait;
use factorio_bot_core::types::Position;
use factorio_bot_planner::{BotId, InventorySlot};

#[derive(Debug, thiserror::Error)]
pub enum ActuatorError {
    #[error("no RCON connection available")]
    NotConnected,
    #[error("bot {0:?} has no mapped Factorio player")]
    UnknownBot(BotId),
    #[error("the game does not define inventory slot {0}")]
    UnknownInventorySlot(&'static str),
    #[error("game rejected the command: {0}")]
    Rejected(String),
}

/// Everything the executor can tell a bot to do.
///
/// One method per `ActionKind` variant plus `walk`, which the schedule emits as
/// its own step. Argument order is normalized here; `FactorioRcon`'s own
/// signatures are inconsistent about where `world` goes.
#[async_trait]
pub trait Actuator: Send + Sync {
    async fn walk(&self, bot: BotId, to: Position) -> Result<(), ActuatorError>;
    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<(), ActuatorError>;
    async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<(), ActuatorError>;
    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<(), ActuatorError>;
    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError>;
    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError>;
    async fn research(&self, tech: &str) -> Result<(), ActuatorError>;
}
```

- [ ] **Step 2: Write the failing test for defines resolution**

`crates/executor/src/rcon_actuator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_are_parsed_from_the_games_reply() {
        let json = r#"{"chest":1,"furnace_source":2,"furnace_result":3,"fuel":1}"#;
        let d = InventoryDefines::from_json(json).expect("parses");
        assert_eq!(d.get(InventorySlot::FurnaceSource).unwrap(), 2);
        assert_eq!(d.get(InventorySlot::FurnaceResult).unwrap(), 3);
    }

    #[test]
    fn a_slot_the_game_does_not_define_is_an_error() {
        let d = InventoryDefines::from_json(r#"{"chest":1}"#).expect("parses");
        assert!(matches!(
            d.get(InventorySlot::LabInput),
            Err(ActuatorError::UnknownInventorySlot("lab_input"))
        ));
    }
}
```

- [ ] **Step 3: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor defines_are_parsed`
Expected: FAIL — `InventoryDefines` not found.

- [ ] **Step 4: Implement defines resolution**

Above those tests in `rcon_actuator.rs`:

```rust
use crate::actuator::{Actuator, ActuatorError};
use async_trait::async_trait;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::types::{FactorioWorld, PlayerId, Position};
use factorio_bot_planner::{BotId, InventorySlot};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The game's own `defines.inventory` table, read once at construction.
///
/// Never hardcoded: these integers are entity-type dependent and change
/// between Factorio versions.
#[derive(Debug, Clone, Default)]
pub struct InventoryDefines {
    by_name: BTreeMap<String, u32>,
}

impl InventoryDefines {
    pub fn from_json(s: &str) -> Result<Self, ActuatorError> {
        let by_name: BTreeMap<String, u32> = serde_json::from_str(s)
            .map_err(|e| ActuatorError::Rejected(format!("bad defines reply: {e}")))?;
        Ok(Self { by_name })
    }

    pub fn get(&self, slot: InventorySlot) -> Result<u32, ActuatorError> {
        let key = slot.defines_key();
        self.by_name
            .get(key)
            .copied()
            .ok_or(ActuatorError::UnknownInventorySlot(key))
    }
}

/// The Lua the game runs to report its inventory defines.
pub const DEFINES_QUERY: &str = "/silent-command \
local t={} for k,v in pairs(defines.inventory) do t[k]=v end \
rcon.print(game.table_to_json(t))";
```

Add `serde_json` to the crate's dependencies (it is already a workspace-wide dependency via core; use `serde_json = "1"`).

- [ ] **Step 4b: Add `connected_players` to core**

`FactorioRcon::connected_player_count` (`crates/core/src/factorio/rcon.rs:161`) already fetches the full player objects via `remote_call("players", vec![])` and then throws everything away but the length. The mod's `rcon_players()` returns a JSON array of serialized players, and `FactorioPlayer` (`crates/core/src/types.rs:130`) carries `player_id: PlayerId`.

Add a sibling that keeps them, and make the counter delegate to it so the two cannot disagree:

```rust
    /// Every connected player that has a character, as the mod reports them.
    ///
    /// `helpers.table_to_json({})` yields `"{}"` rather than `"[]"` for an
    /// empty table, so an empty result arrives as an object. That is not an
    /// error; it means nobody is connected.
    pub async fn connected_players(&self) -> Result<Vec<FactorioPlayer>> {
        let response = self.remote_call("players", vec![]).await?;
        let Some(lines) = response else {
            return Ok(vec![]);
        };
        let json_str = lines.join("");
        if json_str == "{}" || json_str.is_empty() {
            return Ok(vec![]);
        }
        serde_json::from_str::<Vec<FactorioPlayer>>(&json_str)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to parse players: {json_str}"))
    }
```

Then rewrite `connected_player_count` as `Ok(self.connected_players().await?.len())`. Run the workspace tests: `process_control.rs:213` is its only caller and its behaviour must not change.

- [ ] **Step 5: Implement the adapter**

Still in `rcon_actuator.rs`:

```rust
pub struct RconActuator {
    rcon: Arc<FactorioRcon>,
    world: Arc<FactorioWorld>,
    defines: InventoryDefines,
    /// Planner bot ids to Factorio player ids. Bots are interchangeable to the
    /// planner; this is where they acquire an identity in the game.
    players: BTreeMap<BotId, PlayerId>,
}

impl RconActuator {
    /// Discovers its own bots: every connected player becomes a bot, numbered
    /// `BotId(0..n)` in ascending player-id order.
    ///
    /// Sorted, so the mapping is a function of who is connected and not of the
    /// order the game happened to list them. The planner treats bots as
    /// interchangeable, so which player gets which id does not affect the plan
    /// — but it must be stable across a re-plan within one run, or the
    /// executor would hand a chain to a different body midway.
    pub async fn new(
        rcon: Arc<FactorioRcon>,
        world: Arc<FactorioWorld>,
    ) -> Result<Self, ActuatorError> {
        let mut ids: Vec<PlayerId> = rcon
            .connected_players()
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .into_iter()
            .map(|p| p.player_id)
            .collect();
        ids.sort_unstable();
        let players: BTreeMap<BotId, PlayerId> = ids
            .into_iter()
            .enumerate()
            .map(|(i, pid)| (BotId(i as u8), pid))
            .collect();
        if players.is_empty() {
            return Err(ActuatorError::Rejected("no connected players".to_string()));
        }
        // `FactorioRcon::send` is `async fn send(&self, command: &str)
        // -> Result<Option<Vec<String>>>` (crates/core/src/factorio/rcon.rs:79).
        // A silent-command reply arrives as one line; absence means the game
        // answered nothing, which is a hard error here.
        let reply = rcon
            .send(DEFINES_QUERY)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .and_then(|lines| lines.into_iter().next())
            .ok_or_else(|| ActuatorError::Rejected("no reply to defines query".to_string()))?;
        let defines = InventoryDefines::from_json(&reply)?;
        Ok(Self {
            rcon,
            world,
            defines,
            players,
        })
    }

    fn player(&self, bot: BotId) -> Result<PlayerId, ActuatorError> {
        self.players
            .get(&bot)
            .copied()
            .ok_or(ActuatorError::UnknownBot(bot))
    }
}
```

Then the trait impl, mapping each method onto the RCON call. Note that `move_player` takes `&world` first while `place_entity` takes it last — normalizing that inconsistency is one of the reasons this adapter exists:

```rust
#[async_trait]
impl Actuator for RconActuator {
    async fn walk(&self, bot: BotId, to: Position) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .move_player(&self.world, p, &to, None)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .insert_to_inventory(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .player_mine(&self.world, p, item, &at, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .player_craft(&self.world, p, recipe, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        // `place_entity` returns the created FactorioEntity; the executor does
        // not need it, because the plan already knows what it placed and the
        // world snapshot is refreshed by the event stream, not by this reply.
        self.rcon
            .place_entity(p, item.to_string(), at, direction, &self.world)
            .await
            .map(|_entity| ())
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .remove_from_inventory(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    /// Research is server-wide: it takes no player id, so `bot` is unused.
    /// Two bots researching the same technology is idempotent in Factorio.
    async fn research(&self, tech: &str) -> Result<(), ActuatorError> {
        self.rcon
            .add_research(tech)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }
}
```

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test -p factorio-bot-executor`
Expected: PASS.

```bash
cargo fmt -p factorio-bot-executor
cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings
git commit -m "feat(executor): add the actuator trait and its rcon implementation" -- crates/executor crates/core Cargo.toml
```

---

### Task 4: Run one bot's schedule

**Files:**
- Create: `crates/executor/src/run.rs`
- Modify: `crates/executor/src/lib.rs`
- Test: inline in `run.rs`

**Inherited hazard — `defines.inventory` integers collide across entity types.** `chest` and `fuel` are both **1**; `furnace_source`, `assembling_machine_input` and `lab_input` are all **2**. So an `InventorySlot` that is correct for one entity kind, paired with the wrong entity name, resolves to a plausible integer and fails *quietly* against the game rather than erroring.

This is not defensible in the actuator, which sees only a name and a slot and cannot know the entity's type. It is defensible here, where the action came from a method that chose both together. When dispatching `Insert`/`Remove`, treat a mismatched (entity, slot) pair as a bug in the planner rather than something to paper over: the executor's job is to make it loud, not to guess. At minimum, do not silently substitute a slot.

**Interfaces:**
- Consumes: `Actuator` (Task 3), `ExecutionLog` (Task 2), `planner::{Schedule, ScheduledStep, StepKind, ActionNetwork}`.
- Produces: `async fn run_bot(...) -> ExecutionLog`. **Task 5 supersedes this** with `run_bot_signalled`, which takes a shared log and waits on predecessors. Write `run_bot` here anyway: it is the single-bot core, its tests pin the per-step dispatch, and Task 5's version is a small delta on it. When Task 5 lands, delete `run_bot` and keep its two tests pointed at the new function — do not leave two near-identical loops in the file.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use mockall::predicate::*;

    mock! {
        pub Act {}
        #[async_trait::async_trait]
        impl Actuator for Act {
            async fn walk(&self, bot: BotId, to: Position) -> Result<(), ActuatorError>;
            async fn mine(&self, bot: BotId, item: &str, at: Position, count: u32) -> Result<(), ActuatorError>;
            async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<(), ActuatorError>;
            async fn place(&self, bot: BotId, item: &str, at: Position, direction: u8) -> Result<(), ActuatorError>;
            async fn insert(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<(), ActuatorError>;
            async fn remove(&self, bot: BotId, entity: &str, at: Position, slot: InventorySlot, item: &str, count: u32) -> Result<(), ActuatorError>;
            async fn research(&self, tech: &str) -> Result<(), ActuatorError>;
        }
    }

    #[tokio::test]
    async fn a_bot_performs_its_steps_in_schedule_order() {
        let mut act = MockAct::new();
        let mut seq = mockall::Sequence::new();
        act.expect_walk()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok(()));
        act.expect_mine()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _, _| Ok(()));

        let (net, sched) = walk_then_mine_fixture();
        let log = run_bot(&act, BotId(0), &sched, &net).await;

        assert_eq!(log.failed(), vec![]);
        assert_eq!(log.status(mine_action_id()), Status::Success);
    }

    #[tokio::test]
    async fn a_failed_step_is_logged_and_stops_that_bot() {
        let mut act = MockAct::new();
        act.expect_walk().returning(|_, _| Ok(()));
        act.expect_mine()
            .returning(|_, _, _, _| Err(ActuatorError::Rejected("out of reach".into())));

        let (net, sched) = walk_then_mine_fixture();
        let log = run_bot(&act, BotId(0), &sched, &net).await;

        assert_eq!(log.failed(), vec![mine_action_id()]);
    }
}
```

Write `walk_then_mine_fixture()` and `mine_action_id()` as test helpers in the same module: build a two-step `Schedule` for `BotId(0)` — a `StepKind::Walk` followed by a `StepKind::Act` referencing a `Mine` action in a one-action `ActionNetwork`. Use whatever constructors `ActionNetwork` already exposes; the red-science tests in `crates/planner/tests/` show the idiom.

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor a_bot_performs_its_steps`
Expected: FAIL — `run_bot` not found.

- [ ] **Step 3: Implement the loop**

```rust
/// Walk one bot's slice of the schedule, in order.
///
/// Stops that bot at its first failure: later steps in a chain depend on
/// earlier ones, and pressing on would issue commands whose preconditions the
/// game no longer satisfies. Recovery is the caller's decision (see `recover`).
pub async fn run_bot<A: Actuator + ?Sized>(
    act: &A,
    bot: BotId,
    sched: &Schedule,
    net: &ActionNetwork,
) -> ExecutionLog {
    let mut log = ExecutionLog::default();
    for step in sched.steps.iter().filter(|s| s.bot == bot) {
        match &step.what {
            StepKind::Walk { to } => {
                if act.walk(bot, to.clone()).await.is_err() {
                    return log;
                }
            }
            StepKind::Act { action, .. } => {
                log.start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    log.fail(*action, step.start, "action not in network".to_string());
                    return log;
                };
                match perform(act, bot, &a.kind).await {
                    Ok(()) => log.succeed(*action, step.end),
                    Err(e) => {
                        log.fail(*action, step.end, e.to_string());
                        return log;
                    }
                }
            }
        }
    }
    log
}

async fn perform<A: Actuator + ?Sized>(
    act: &A,
    bot: BotId,
    kind: &ActionKind,
) -> Result<(), ActuatorError> {
    match kind {
        ActionKind::Mine { pos, item, count } => {
            act.mine(bot, item.as_str(), pos.clone(), *count).await
        }
        ActionKind::Craft { item, count } => act.craft(bot, item.as_str(), *count).await,
        ActionKind::Place { entity } => {
            act.place(bot, &entity.name, entity.position.clone(), entity.direction)
                .await
        }
        ActionKind::Insert { pos, entity, slot, item, count } => {
            act.insert(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Remove { pos, entity, slot, item, count } => {
            act.remove(bot, entity, pos.clone(), *slot, item.as_str(), *count)
                .await
        }
        ActionKind::Research { tech } => act.research(tech).await,
    }
}
```

`ActionNetwork::action(id) -> Option<&Action>` already exists (`crates/planner/src/network.rs:80`). `ItemId` is a type alias for `String`, so `item.as_str()` is correct as written.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p factorio-bot-executor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-executor
cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings
git commit -m "feat(executor): run one bot's slice of a schedule" -- crates/executor crates/planner
```

---

### Task 5: Run every bot, with cross-bot dependencies

The old executor polled every 100 ms (`crates/core/src/plan/execute.rs:79`). The spec calls for a per-action completion signal instead.

**Files:**
- Modify: `crates/executor/src/run.rs`, `crates/executor/src/lib.rs`
- Test: inline in `run.rs`

**Delete `run_bot` from Task 4 as part of this task**, re-pointing its two tests at `run_into` with a single-bot schedule. Two near-identical per-bot loops in one file is exactly the duplication that drifts.

**Interfaces:**
- Produces: `async fn run_into(act: &dyn Actuator, sched: &Schedule, net: &ActionNetwork, progress: &Mutex<ExecutionLog>)` plus the wrapper `async fn run(...) -> ExecutionLog`. Task 7 calls `run_into` so it can read progress mid-run.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn a_bot_waits_for_another_bots_action_before_its_own() {
    // net: action A (bot 0) -> action B (bot 1). B must not start until A ends.
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut act = MockAct::new();
    {
        let order = order.clone();
        act.expect_mine().returning(move |_, item, _, _| {
            order.lock().unwrap().push(item.to_string());
            Ok(())
        });
    }
    act.expect_walk().returning(|_, _| Ok(()));

    let (net, sched) = cross_bot_fixture();
    let log = run(&act, &sched, &net).await;

    assert_eq!(log.failed(), vec![]);
    assert_eq!(
        *order.lock().unwrap(),
        vec!["iron-ore".to_string(), "copper-ore".to_string()]
    );
}

#[tokio::test]
async fn a_dependent_action_is_abandoned_when_its_predecessor_fails() {
    let mut act = MockAct::new();
    act.expect_walk().returning(|_, _| Ok(()));
    act.expect_mine().returning(|_, item, _, _| {
        if item == "iron-ore" {
            Err(ActuatorError::Rejected("no ore".into()))
        } else {
            Ok(())
        }
    });

    let (net, sched) = cross_bot_fixture();
    let log = run(&act, &sched, &net).await;

    assert_eq!(log.status(second_action_id()), Status::Pending);
}
```

`cross_bot_fixture()` builds a two-action network where the second action has an edge from the first, assigned to different bots.

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor a_bot_waits_for_another`
Expected: FAIL — `run` not found.

- [ ] **Step 3: Implement orchestration**

Each action gets a `tokio::sync::watch` channel carrying its `Status`. A bot awaits every predecessor of an action before starting it. Concurrency comes from `join_all` over borrowed futures rather than `tokio::spawn`, which keeps `&Schedule` and `&ActionNetwork` as plain borrows — no `Arc`, no `'static` bound.

Add `futures = "0.3"` to the crate's dependencies.

```rust
use crate::log::{ExecutionLog, Status};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use tokio::sync::watch;

enum PredOutcome {
    Ready,
    Abandoned,
}

/// Run every bot, recording progress into `progress` as it happens.
///
/// The log is shared rather than merged at the end because `goal.progress(h)`
/// must be able to read it mid-run. Keys are disjoint — each action belongs to
/// exactly one bot — so concurrent writers never collide on a key, and because
/// `ExecutionLog` is a `BTreeMap` the finished state is identical regardless of
/// the order the writes landed. That is what keeps this deterministic despite
/// being concurrent.
///
/// `std::sync::Mutex`, not tokio's: every critical section is a few map
/// operations with no `.await` inside. Never hold this guard across an await.
pub async fn run_into(
    act: &dyn Actuator,
    sched: &Schedule,
    net: &ActionNetwork,
    progress: &Mutex<ExecutionLog>,
) {
    let mut senders: BTreeMap<ActionId, watch::Sender<Status>> = BTreeMap::new();
    let mut receivers: BTreeMap<ActionId, watch::Receiver<Status>> = BTreeMap::new();
    for id in net.actions().map(|a| a.id) {
        let (tx, rx) = watch::channel(Status::Pending);
        senders.insert(id, tx);
        receivers.insert(id, rx);
    }

    // BTreeSet, so the future order is a function of the schedule alone, not of
    // task completion timing.
    let bots: BTreeSet<BotId> = sched.steps.iter().map(|s| s.bot).collect();
    join_all(
        bots.iter()
            .map(|&bot| run_bot_signalled(act, bot, sched, net, progress, &senders, &receivers)),
    )
    .await;
}

/// Convenience wrapper for callers that only want the final state.
pub async fn run(act: &dyn Actuator, sched: &Schedule, net: &ActionNetwork) -> ExecutionLog {
    let progress = Mutex::new(ExecutionLog::default());
    run_into(act, sched, net, &progress).await;
    progress
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
}

async fn run_bot_signalled(
    act: &dyn Actuator,
    bot: BotId,
    sched: &Schedule,
    net: &ActionNetwork,
    log: &Mutex<ExecutionLog>,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) {
    let mine: Vec<&ScheduledStep> = sched.steps.iter().filter(|s| s.bot == bot).collect();

    for (i, step) in mine.iter().enumerate() {
        match &step.what {
            StepKind::Walk { to } => {
                if act.walk(bot, to.clone()).await.is_err() {
                    abandon_rest(&mine[i..], senders);
                    return;
                }
            }
            StepKind::Act { action, .. } => {
                if let PredOutcome::Abandoned = await_preds(net, *action, receivers).await {
                    abandon_rest(&mine[i..], senders);
                    return;
                }
                lock(log).start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    lock(log).fail(*action, step.start, "action not in network".to_string());
                    abandon_rest(&mine[i..], senders);
                    return;
                };
                // `perform` awaits, so the guard is taken and dropped around
                // it, never held across it.
                match perform(act, bot, &a.kind).await {
                    Ok(()) => {
                        lock(log).succeed(*action, step.end);
                        let _ = senders[action].send(Status::Success);
                    }
                    Err(e) => {
                        lock(log).fail(*action, step.end, e.to_string());
                        abandon_rest(&mine[i..], senders);
                        return;
                    }
                }
            }
        }
    }
}

/// One place to take the log guard, so poisoning is handled identically
/// everywhere. A panic mid-run should not turn every later write into a second
/// panic; the log is observational, so recovering the inner value is right.
fn lock(log: &Mutex<ExecutionLog>) -> std::sync::MutexGuard<'_, ExecutionLog> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

/// Publish `Failed` for every action this bot will now never reach.
///
/// Without this the executor deadlocks: a bot that stops early leaves its
/// remaining actions at `Pending` forever, and any bot waiting on one of them
/// waits forever too. Abandonment has to propagate for the run to terminate.
fn abandon_rest(rest: &[&ScheduledStep], senders: &BTreeMap<ActionId, watch::Sender<Status>>) {
    for step in rest {
        if let StepKind::Act { action, .. } = &step.what {
            if let Some(tx) = senders.get(action) {
                let _ = tx.send(Status::Failed);
            }
        }
    }
}

async fn await_preds(
    net: &ActionNetwork,
    id: ActionId,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) -> PredOutcome {
    let mut max_lag: Ticks = 0;
    for (pred, lag) in net.preds(id) {
        let Some(rx) = receivers.get(&pred) else {
            continue;
        };
        let mut rx = rx.clone();
        loop {
            // Bind by value so the watch borrow is dropped before the await.
            let status = *rx.borrow_and_update();
            match status {
                Status::Success => break,
                Status::Failed => return PredOutcome::Abandoned,
                Status::Pending | Status::Running => {}
            }
            if rx.changed().await.is_err() {
                return PredOutcome::Abandoned;
            }
        }
        max_lag = max_lag.max(lag);
    }

    // A lag edge is machine time, not bot time: the furnace keeps working after
    // the bot walks away, and the plate is not there until it has. The
    // predecessor's own completion signal does not cover that wait, so honour
    // the lag once every predecessor has succeeded.
    if max_lag > 0 {
        tokio::time::sleep(ticks_to_wall_clock(max_lag)).await;
    }
    PredOutcome::Ready
}

/// Factorio runs at 60 ticks per second at normal speed.
///
/// See the open questions: a server running at a non-default `game.speed`
/// makes this conversion wrong, and the fix is to read the speed rather than
/// assume it.
fn ticks_to_wall_clock(ticks: Ticks) -> std::time::Duration {
    std::time::Duration::from_millis((u64::from(ticks) * 1000) / 60)
}
```

Add a determinism test: run the same fixture twice with the mock actuator's per-action delays reversed, and assert the two final `ExecutionLog`s are equal. Concurrent writers must not be able to change the finished state.

Both accessors the loop needs already exist in `crates/planner/src/network.rs`: `actions()` returns an iterator over `&Action` (line 84) and `preds(id)` returns `Vec<(ActionId, Ticks)>` (line 97). Do not add new ones.

Use `tokio::time::pause()` in the tests so lag sleeps do not make the suite slow — that is why `test-util` is in the dev-dependencies.

- [ ] **Step 4: Run the tests, then the whole workspace**

Run: `cargo test -p factorio-bot-executor` then `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-executor
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(executor): orchestrate every bot with per-action completion signals" -- crates/executor crates/planner
```

---

### Task 6: Recovery tiers one and two

**Files:**
- Create: `crates/executor/src/recover.rs`
- Modify: `crates/executor/src/lib.rs`
- Test: inline in `recover.rs`

**Interfaces:**
- Produces: `enum Recovery { Rescheduled(Schedule), Reexpanded { net: ActionNetwork, sched: Schedule }, Surfaced(Vec<ActionId>) }` and `fn recover(goal, net, observed_state, bots, log) -> Recovery`.

This function is **pure** — it takes observed state as a value and returns a decision. It performs no I/O, which is what makes the three tiers testable.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_recoverable_failure_reschedules_without_reexpanding() {
    let (goal, net, state, bots, log) = one_failed_mine_but_ore_still_reachable();
    match recover(&goal, &net, &state, &bots, &log) {
        Recovery::Rescheduled(s) => assert!(s.makespan > 0),
        other => panic!("expected a reschedule, got {other:?}"),
    }
}

#[test]
fn an_unschedulable_state_triggers_reexpansion() {
    let (goal, net, state, bots, log) = ore_patch_exhausted();
    assert!(matches!(
        recover(&goal, &net, &state, &bots, &log),
        Recovery::Reexpanded { .. }
    ));
}

#[test]
fn an_unexpandable_goal_is_surfaced_with_the_failures_that_caused_it() {
    let (goal, net, state, bots, log) = nothing_can_satisfy_the_goal();
    match recover(&goal, &net, &state, &bots, &log) {
        Recovery::Surfaced(ids) => assert_eq!(ids, log.failed()),
        other => panic!("expected surfacing, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor a_recoverable_failure`
Expected: FAIL — `recover` not found.

- [ ] **Step 3: Implement the tiers**

```rust
/// Decide what to do about a partially failed execution.
///
/// Tier 1 is cheap: keep the network, re-run `schedule` against observed state.
/// Tier 2 re-expands from layer 2, which is what you need when the world no
/// longer affords the plan's approach at all (the ore patch is gone, not merely
/// further away). Tier 3 gives up and names the failures — this is the seam an
/// LLM plugs into later: rare, high level, not time critical.
pub fn recover(
    goal: &Goal,
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
    log: &ExecutionLog,
) -> Recovery {
    let remaining = net.without_completed(log);
    if let Ok(s) = schedule(&remaining, state, bots) {
        return Recovery::Rescheduled(s);
    }
    let mut fresh = ActionNetwork::default();
    let mut ctx = ExpansionCtx::new(state);
    if expand(goal, &mut ctx, &mut fresh, &registry_for(bots)).is_ok() {
        if let Ok(s) = schedule(&fresh, state, bots) {
            return Recovery::Reexpanded { net: fresh, sched: s };
        }
    }
    Recovery::Surfaced(log.failed())
}
```

`ActionNetwork::without_completed(&ExecutionLog)` does not exist and must not: the planner may not depend on the executor. Instead give the planner `ActionNetwork::retaining(&BTreeSet<ActionId>) -> ActionNetwork` and have the executor compute the retained set from the log. Fix the call above accordingly when you implement it.

`ExpansionCtx::new` and `registry_for` signatures are as the planner already defines them — check, do not assume.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test -p factorio-bot-executor` then `cargo test --workspace`

```bash
cargo fmt -p factorio-bot-executor
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(executor): add reschedule and re-expand recovery tiers" -- crates/executor crates/planner
```

---

### Task 7: The new Lua goal API (additive)

Adds a `goal.*` table beside the existing `plan.*`. Nothing is removed in this task — both APIs work, and the old scripts keep running.

**Files:**
- Create: `crates/scripting_lua/src/globals/goal.rs`
- Modify: `crates/scripting_lua/src/globals/mod.rs`, `crates/scripting_lua/src/lua_runner.rs`
- Modify: `crates/scripting_lua/Cargo.toml` (add planner + executor deps)
- Test: `crates/scripting_lua/tests/goal_script.lua` plus a Rust test that runs it

**Interfaces:**
- Consumes: `planner::{Goal, Holder, expand, schedule, registry_for}`, `executor::run`.

- [ ] **Step 1: Write the Lua-facing functions**

Follow the existing idiom in `crates/scripting_lua/src/globals/plan.rs` exactly — including the `__doc_entry_*` documentation strings, which is where the published Lua API docs come from.

Bind seven functions:

```lua
-- @treturn number a plan handle
function goal.have(item_name, count)
-- @treturn number a plan handle
function goal.researched(technology_name)
-- @treturn number the makespan in ticks
function goal.schedule(plan_handle, bot_count)
-- @treturn string graphviz source
function goal.graphviz(plan_handle)
-- @treturn string mermaid gantt source for the scheduled plan
function goal.gantt(plan_handle, title)
-- starts execution and returns immediately
-- @treturn number a run handle
function goal.execute(plan_handle)
-- @treturn table {pending=n, running=n, success=n, failed=n, done=bool}
function goal.progress(run_handle)
-- blocks until the run finishes; returns the same table as goal.progress
-- @treturn table
function goal.wait(run_handle)
```

`goal.have` and `goal.researched` build a `Goal`, run `expand`, and return a handle holding the `ActionNetwork`. `goal.schedule` runs `schedule` and stores the result on the handle.

`goal.gantt` is a three-line binding: `factorio_bot_planner::render::mermaid_gantt(schedule, title)` already exists (`crates/planner/src/render.rs:33`). It replaces the old `plan.task_graph_mermaid_gantt`, so bind it here rather than leaving the migration without a Gantt renderer. It requires the handle to have been scheduled; error clearly if it has not.

**Execution is handle-based, not blocking** — this was an explicit decision, so do not "simplify" it back to a blocking call. `goal.execute` builds an `RconActuator` from the live `FactorioInstance` (which discovers its own bots — it takes no mapping), spawns a tokio task running `executor::run_into`, and returns a run handle immediately. `goal.progress` reads a snapshot of that run's shared `ExecutionLog`; `goal.wait` blocks on the join handle and then returns the final snapshot.

Handles are `u32` keys into a registry the Lua globals own, not Lua tables holding Rust pointers:

```rust
/// Live runs, keyed by the handle Lua holds.
///
/// A registry rather than userdata because the shared log outlives any single
/// Lua call and must be readable from `goal.progress` on a later call. Bots
/// keep working while the script does something else — that is the point of
/// the handle-based API.
struct Runs {
    next: u32,
    runs: BTreeMap<u32, RunEntry>,
}

struct RunEntry {
    progress: Arc<Mutex<ExecutionLog>>,
    join: tokio::task::JoinHandle<()>,
}
```

`goal.execute` needs owned values to move into the spawned task, so it clones the handle's `Arc<Schedule>` and `Arc<ActionNetwork>` and calls `run_into(&*act, &*sched, &*net, &progress)` inside. This is the one place `Arc` is needed; `run_into` itself stays borrow-based.

**There is no `goal.group`.** Manual grouping was dropped by decision: the planner's method expansion is the only source of grouping, and a manual bracket that disagreed with it would either be ignored or fight the scheduler. If a script needs to force specific work onto a specific bot, that is `Action::pinned`, which is a planner concern and not a Lua one.

- [ ] **Step 2: Write the failing test script**

`crates/scripting_lua/tests/goal_script.lua`:

```lua
local p = goal.have("automation-science-pack", 10)
local makespan = goal.schedule(p, 4)
assert(makespan > 0, "expected a positive makespan")
assert(#goal.graphviz(p) > 0, "expected graphviz output")
```

Add a Rust test that runs it through the same harness `lua_runner.rs`'s existing `test_script` uses. Do not call `goal.execute`, `goal.progress` or `goal.wait` in this script — they need a live game.

Cover the handle registry separately, in Rust, where the actuator can be mocked: assert that `goal.execute` returns a handle without waiting, that `goal.progress` on it reports counts that sum to the action total, and that `goal.wait` returns with `done = true`. That is the part of this task most likely to be wrong, and it is the part the Lua fixture cannot reach.

- [ ] **Step 3: Run it to make sure it fails, then implement, then pass**

Run: `cargo test -p factorio-bot-scripting-lua`
Expected: FAIL first (`goal` is nil), PASS after implementation.

- [ ] **Step 4: Record the unbound binding for Task 8, do not fix it here**

`PlanBuilder::add_insert_into_inventory` exists in Rust but was never bound to Lua, while `scripts/test_phase_2_1.lua` and `scripts/test_phase_2_2.lua` call `plan.insert_into_inventory`. Those two scripts fail at runtime today and always have.

An earlier draft fixed this by binding it into `globals/plan.rs`. Do **not** do that: Task 8 deletes that file, so the binding would live for one task and be deleted unread. Building something in order to delete it two tasks later is not caution, it is waste.

What matters instead is the consequence for Task 8, so write it into your report: **those two scripts have never successfully run.** Their `plan.*` calls were never all valid, so there is no observed behaviour to preserve when migrating them. Task 8 must port them from their evident intent and say so, rather than claiming behaviour preservation it cannot verify.

- [ ] **Step 4b: Do NOT touch the Lua filesystem sandbox — it is another plan's task**

An earlier draft of this plan hardened `world.draw`'s save path here. **That work has been ceded** to the concurrent web-server effort, whose plan `docs/superpowers/plans/2026-08-30-script-execution-jobs-and-sse.md` Task 1 owns it.

The reason is that `world.draw` is not one hole but four of identical shape — a caller-supplied string joined onto a root and handed to the filesystem with no bounds check:

| binding | sink |
| --- | --- |
| `world.draw(save_path)` | `globals/world.rs:200` → `core/test_utils.rs:232` |
| `globals.include(source_path)` | `globals/globals.rs:52` — reads AND executes arbitrary Lua |
| `globals.file_read(source_path)` | `globals/globals.rs:78` |
| `globals.file_write(target, body)` | `globals/globals.rs:99` |

Fixing one in isolation leaves three identical holes, and the correct fix is a single shared bounded resolver rather than four local patches. Do not write a fourth variant of it here.

If you are in `globals/` for the `goal.*` binding and notice another path-shaped sink not in that table, report it — do not fix it.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-executor
cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings
git commit -m "feat(executor): run one bot's slice of a schedule" -- crates/executor crates/planner
```

---

### Task 5: Run every bot, with cross-bot dependencies

The old executor polled every 100 ms (`crates/core/src/plan/execute.rs:79`). The spec calls for a per-action completion signal instead.

**Files:**
- Modify: `crates/executor/src/run.rs`, `crates/executor/src/lib.rs`
- Test: inline in `run.rs`

**Delete `run_bot` from Task 4 as part of this task**, re-pointing its two tests at `run_into` with a single-bot schedule. Two near-identical per-bot loops in one file is exactly the duplication that drifts.

**Interfaces:**
- Produces: `async fn run_into(act: &dyn Actuator, sched: &Schedule, net: &ActionNetwork, progress: &Mutex<ExecutionLog>)` plus the wrapper `async fn run(...) -> ExecutionLog`. Task 7 calls `run_into` so it can read progress mid-run.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn a_bot_waits_for_another_bots_action_before_its_own() {
    // net: action A (bot 0) -> action B (bot 1). B must not start until A ends.
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut act = MockAct::new();
    {
        let order = order.clone();
        act.expect_mine().returning(move |_, item, _, _| {
            order.lock().unwrap().push(item.to_string());
            Ok(())
        });
    }
    act.expect_walk().returning(|_, _| Ok(()));

    let (net, sched) = cross_bot_fixture();
    let log = run(&act, &sched, &net).await;

    assert_eq!(log.failed(), vec![]);
    assert_eq!(
        *order.lock().unwrap(),
        vec!["iron-ore".to_string(), "copper-ore".to_string()]
    );
}

#[tokio::test]
async fn a_dependent_action_is_abandoned_when_its_predecessor_fails() {
    let mut act = MockAct::new();
    act.expect_walk().returning(|_, _| Ok(()));
    act.expect_mine().returning(|_, item, _, _| {
        if item == "iron-ore" {
            Err(ActuatorError::Rejected("no ore".into()))
        } else {
            Ok(())
        }
    });

    let (net, sched) = cross_bot_fixture();
    let log = run(&act, &sched, &net).await;

    assert_eq!(log.status(second_action_id()), Status::Pending);
}
```

`cross_bot_fixture()` builds a two-action network where the second action has an edge from the first, assigned to different bots.

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor a_bot_waits_for_another`
Expected: FAIL — `run` not found.

- [ ] **Step 3: Implement orchestration**

Each action gets a `tokio::sync::watch` channel carrying its `Status`. A bot awaits every predecessor of an action before starting it. Concurrency comes from `join_all` over borrowed futures rather than `tokio::spawn`, which keeps `&Schedule` and `&ActionNetwork` as plain borrows — no `Arc`, no `'static` bound.

Add `futures = "0.3"` to the crate's dependencies.

```rust
use crate::log::{ExecutionLog, Status};
use futures::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use tokio::sync::watch;

enum PredOutcome {
    Ready,
    Abandoned,
}

/// Run every bot, recording progress into `progress` as it happens.
///
/// The log is shared rather than merged at the end because `goal.progress(h)`
/// must be able to read it mid-run. Keys are disjoint — each action belongs to
/// exactly one bot — so concurrent writers never collide on a key, and because
/// `ExecutionLog` is a `BTreeMap` the finished state is identical regardless of
/// the order the writes landed. That is what keeps this deterministic despite
/// being concurrent.
///
/// `std::sync::Mutex`, not tokio's: every critical section is a few map
/// operations with no `.await` inside. Never hold this guard across an await.
pub async fn run_into(
    act: &dyn Actuator,
    sched: &Schedule,
    net: &ActionNetwork,
    progress: &Mutex<ExecutionLog>,
) {
    let mut senders: BTreeMap<ActionId, watch::Sender<Status>> = BTreeMap::new();
    let mut receivers: BTreeMap<ActionId, watch::Receiver<Status>> = BTreeMap::new();
    for id in net.actions().map(|a| a.id) {
        let (tx, rx) = watch::channel(Status::Pending);
        senders.insert(id, tx);
        receivers.insert(id, rx);
    }

    // BTreeSet, so the future order is a function of the schedule alone, not of
    // task completion timing.
    let bots: BTreeSet<BotId> = sched.steps.iter().map(|s| s.bot).collect();
    join_all(
        bots.iter()
            .map(|&bot| run_bot_signalled(act, bot, sched, net, progress, &senders, &receivers)),
    )
    .await;
}

/// Convenience wrapper for callers that only want the final state.
pub async fn run(act: &dyn Actuator, sched: &Schedule, net: &ActionNetwork) -> ExecutionLog {
    let progress = Mutex::new(ExecutionLog::default());
    run_into(act, sched, net, &progress).await;
    progress
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
}

async fn run_bot_signalled(
    act: &dyn Actuator,
    bot: BotId,
    sched: &Schedule,
    net: &ActionNetwork,
    log: &Mutex<ExecutionLog>,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) {
    let mine: Vec<&ScheduledStep> = sched.steps.iter().filter(|s| s.bot == bot).collect();

    for (i, step) in mine.iter().enumerate() {
        match &step.what {
            StepKind::Walk { to } => {
                if act.walk(bot, to.clone()).await.is_err() {
                    abandon_rest(&mine[i..], senders);
                    return;
                }
            }
            StepKind::Act { action, .. } => {
                if let PredOutcome::Abandoned = await_preds(net, *action, receivers).await {
                    abandon_rest(&mine[i..], senders);
                    return;
                }
                lock(log).start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    lock(log).fail(*action, step.start, "action not in network".to_string());
                    abandon_rest(&mine[i..], senders);
                    return;
                };
                // `perform` awaits, so the guard is taken and dropped around
                // it, never held across it.
                match perform(act, bot, &a.kind).await {
                    Ok(()) => {
                        lock(log).succeed(*action, step.end);
                        let _ = senders[action].send(Status::Success);
                    }
                    Err(e) => {
                        lock(log).fail(*action, step.end, e.to_string());
                        abandon_rest(&mine[i..], senders);
                        return;
                    }
                }
            }
        }
    }
}

/// One place to take the log guard, so poisoning is handled identically
/// everywhere. A panic mid-run should not turn every later write into a second
/// panic; the log is observational, so recovering the inner value is right.
fn lock(log: &Mutex<ExecutionLog>) -> std::sync::MutexGuard<'_, ExecutionLog> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

/// Publish `Failed` for every action this bot will now never reach.
///
/// Without this the executor deadlocks: a bot that stops early leaves its
/// remaining actions at `Pending` forever, and any bot waiting on one of them
/// waits forever too. Abandonment has to propagate for the run to terminate.
fn abandon_rest(rest: &[&ScheduledStep], senders: &BTreeMap<ActionId, watch::Sender<Status>>) {
    for step in rest {
        if let StepKind::Act { action, .. } = &step.what {
            if let Some(tx) = senders.get(action) {
                let _ = tx.send(Status::Failed);
            }
        }
    }
}

async fn await_preds(
    net: &ActionNetwork,
    id: ActionId,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) -> PredOutcome {
    let mut max_lag: Ticks = 0;
    for (pred, lag) in net.preds(id) {
        let Some(rx) = receivers.get(&pred) else {
            continue;
        };
        let mut rx = rx.clone();
        loop {
            // Bind by value so the watch borrow is dropped before the await.
            let status = *rx.borrow_and_update();
            match status {
                Status::Success => break,
                Status::Failed => return PredOutcome::Abandoned,
                Status::Pending | Status::Running => {}
            }
            if rx.changed().await.is_err() {
                return PredOutcome::Abandoned;
            }
        }
        max_lag = max_lag.max(lag);
    }

    // A lag edge is machine time, not bot time: the furnace keeps working after
    // the bot walks away, and the plate is not there until it has. The
    // predecessor's own completion signal does not cover that wait, so honour
    // the lag once every predecessor has succeeded.
    if max_lag > 0 {
        tokio::time::sleep(ticks_to_wall_clock(max_lag)).await;
    }
    PredOutcome::Ready
}

/// Factorio runs at 60 ticks per second at normal speed.
///
/// See the open questions: a server running at a non-default `game.speed`
/// makes this conversion wrong, and the fix is to read the speed rather than
/// assume it.
fn ticks_to_wall_clock(ticks: Ticks) -> std::time::Duration {
    std::time::Duration::from_millis((u64::from(ticks) * 1000) / 60)
}
```

Add a determinism test: run the same fixture twice with the mock actuator's per-action delays reversed, and assert the two final `ExecutionLog`s are equal. Concurrent writers must not be able to change the finished state.

Both accessors the loop needs already exist in `crates/planner/src/network.rs`: `actions()` returns an iterator over `&Action` (line 84) and `preds(id)` returns `Vec<(ActionId, Ticks)>` (line 97). Do not add new ones.

Use `tokio::time::pause()` in the tests so lag sleeps do not make the suite slow — that is why `test-util` is in the dev-dependencies.

- [ ] **Step 4: Run the tests, then the whole workspace**

Run: `cargo test -p factorio-bot-executor` then `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-executor
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(executor): orchestrate every bot with per-action completion signals" -- crates/executor crates/planner
```

---

### Task 6: Recovery tiers one and two

**Files:**
- Create: `crates/executor/src/recover.rs`
- Modify: `crates/executor/src/lib.rs`
- Test: inline in `recover.rs`

**Interfaces:**
- Produces: `enum Recovery { Rescheduled(Schedule), Reexpanded { net: ActionNetwork, sched: Schedule }, Surfaced(Vec<ActionId>) }` and `fn recover(goal, net, observed_state, bots, log) -> Recovery`.

This function is **pure** — it takes observed state as a value and returns a decision. It performs no I/O, which is what makes the three tiers testable.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_recoverable_failure_reschedules_without_reexpanding() {
    let (goal, net, state, bots, log) = one_failed_mine_but_ore_still_reachable();
    match recover(&goal, &net, &state, &bots, &log) {
        Recovery::Rescheduled(s) => assert!(s.makespan > 0),
        other => panic!("expected a reschedule, got {other:?}"),
    }
}

#[test]
fn an_unschedulable_state_triggers_reexpansion() {
    let (goal, net, state, bots, log) = ore_patch_exhausted();
    assert!(matches!(
        recover(&goal, &net, &state, &bots, &log),
        Recovery::Reexpanded { .. }
    ));
}

#[test]
fn an_unexpandable_goal_is_surfaced_with_the_failures_that_caused_it() {
    let (goal, net, state, bots, log) = nothing_can_satisfy_the_goal();
    match recover(&goal, &net, &state, &bots, &log) {
        Recovery::Surfaced(ids) => assert_eq!(ids, log.failed()),
        other => panic!("expected surfacing, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p factorio-bot-executor a_recoverable_failure`
Expected: FAIL — `recover` not found.

- [ ] **Step 3: Implement the tiers**

```rust
/// Decide what to do about a partially failed execution.
///
/// Tier 1 is cheap: keep the network, re-run `schedule` against observed state.
/// Tier 2 re-expands from layer 2, which is what you need when the world no
/// longer affords the plan's approach at all (the ore patch is gone, not merely
/// further away). Tier 3 gives up and names the failures — this is the seam an
/// LLM plugs into later: rare, high level, not time critical.
pub fn recover(
    goal: &Goal,
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
    log: &ExecutionLog,
) -> Recovery {
    let remaining = net.without_completed(log);
    if let Ok(s) = schedule(&remaining, state, bots) {
        return Recovery::Rescheduled(s);
    }
    let mut fresh = ActionNetwork::default();
    let mut ctx = ExpansionCtx::new(state);
    if expand(goal, &mut ctx, &mut fresh, &registry_for(bots)).is_ok() {
        if let Ok(s) = schedule(&fresh, state, bots) {
            return Recovery::Reexpanded { net: fresh, sched: s };
        }
    }
    Recovery::Surfaced(log.failed())
}
```

`ActionNetwork::without_completed(&ExecutionLog)` does not exist and must not: the planner may not depend on the executor. Instead give the planner `ActionNetwork::retaining(&BTreeSet<ActionId>) -> ActionNetwork` and have the executor compute the retained set from the log. Fix the call above accordingly when you implement it.

`ExpansionCtx::new` and `registry_for` signatures are as the planner already defines them — check, do not assume.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test -p factorio-bot-executor` then `cargo test --workspace`

```bash
cargo fmt -p factorio-bot-executor
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(executor): add reschedule and re-expand recovery tiers" -- crates/executor crates/planner
```

---

### Task 7: The new Lua goal API (additive)

Adds a `goal.*` table beside the existing `plan.*`. Nothing is removed in this task — both APIs work, and the old scripts keep running.

**Files:**
- Create: `crates/scripting_lua/src/globals/goal.rs`
- Modify: `crates/scripting_lua/src/globals/mod.rs`, `crates/scripting_lua/src/lua_runner.rs`
- Modify: `crates/scripting_lua/Cargo.toml` (add planner + executor deps)
- Test: `crates/scripting_lua/tests/goal_script.lua` plus a Rust test that runs it

**Interfaces:**
- Consumes: `planner::{Goal, Holder, expand, schedule, registry_for}`, `executor::run`.

- [ ] **Step 1: Write the Lua-facing functions**

Follow the existing idiom in `crates/scripting_lua/src/globals/plan.rs` exactly — including the `__doc_entry_*` documentation strings, which is where the published Lua API docs come from.

Bind seven functions:

```lua
-- @treturn number a plan handle
function goal.have(item_name, count)
-- @treturn number a plan handle
function goal.researched(technology_name)
-- @treturn number the makespan in ticks
function goal.schedule(plan_handle, bot_count)
-- @treturn string graphviz source
function goal.graphviz(plan_handle)
-- starts execution and returns immediately
-- @treturn number a run handle
function goal.execute(plan_handle)
-- @treturn table {pending=n, running=n, success=n, failed=n, done=bool}
function goal.progress(run_handle)
-- blocks until the run finishes; returns the same table as goal.progress
-- @treturn table
function goal.wait(run_handle)
```

`goal.have` and `goal.researched` build a `Goal`, run `expand`, and return a handle holding the `ActionNetwork`. `goal.schedule` runs `schedule` and stores the result on the handle.

**Execution is handle-based, not blocking** — this was an explicit decision, so do not "simplify" it back to a blocking call. `goal.execute` builds an `RconActuator` from the live `FactorioInstance` (which discovers its own bots — it takes no mapping), spawns a tokio task running `executor::run_into`, and returns a run handle immediately. `goal.progress` reads a snapshot of that run's shared `ExecutionLog`; `goal.wait` blocks on the join handle and then returns the final snapshot.

Handles are `u32` keys into a registry the Lua globals own, not Lua tables holding Rust pointers:

```rust
/// Live runs, keyed by the handle Lua holds.
///
/// A registry rather than userdata because the shared log outlives any single
/// Lua call and must be readable from `goal.progress` on a later call. Bots
/// keep working while the script does something else — that is the point of
/// the handle-based API.
struct Runs {
    next: u32,
    runs: BTreeMap<u32, RunEntry>,
}

struct RunEntry {
    progress: Arc<Mutex<ExecutionLog>>,
    join: tokio::task::JoinHandle<()>,
}
```

`goal.execute` needs owned values to move into the spawned task, so it clones the handle's `Arc<Schedule>` and `Arc<ActionNetwork>` and calls `run_into(&*act, &*sched, &*net, &progress)` inside. This is the one place `Arc` is needed; `run_into` itself stays borrow-based.

**There is no `goal.group`.** Manual grouping was dropped by decision: the planner's method expansion is the only source of grouping, and a manual bracket that disagreed with it would either be ignored or fight the scheduler. If a script needs to force specific work onto a specific bot, that is `Action::pinned`, which is a planner concern and not a Lua one.

- [ ] **Step 2: Write the failing test script**

`crates/scripting_lua/tests/goal_script.lua`:

```lua
local p = goal.have("automation-science-pack", 10)
local makespan = goal.schedule(p, 4)
assert(makespan > 0, "expected a positive makespan")
assert(#goal.graphviz(p) > 0, "expected graphviz output")
```

Add a Rust test that runs it through the same harness `lua_runner.rs`'s existing `test_script` uses. Do not call `goal.execute`, `goal.progress` or `goal.wait` in this script — they need a live game.

Cover the handle registry separately, in Rust, where the actuator can be mocked: assert that `goal.execute` returns a handle without waiting, that `goal.progress` on it reports counts that sum to the action total, and that `goal.wait` returns with `done = true`. That is the part of this task most likely to be wrong, and it is the part the Lua fixture cannot reach.

- [ ] **Step 3: Run it to make sure it fails, then implement, then pass**

Run: `cargo test -p factorio-bot-scripting-lua`
Expected: FAIL first (`goal` is nil), PASS after implementation.

- [ ] **Step 4: Record the unbound binding for Task 8, do not fix it here**

`PlanBuilder::add_insert_into_inventory` exists in Rust but was never bound to Lua, while `scripts/test_phase_2_1.lua` and `scripts/test_phase_2_2.lua` call `plan.insert_into_inventory`. Those two scripts fail at runtime today and always have.

An earlier draft fixed this by binding it into `globals/plan.rs`. Do **not** do that: Task 8 deletes that file, so the binding would live for one task and be deleted unread. Building something in order to delete it two tasks later is not caution, it is waste.

What matters instead is the consequence for Task 8, so write it into your report: **those two scripts have never successfully run.** Their `plan.*` calls were never all valid, so there is no observed behaviour to preserve when migrating them. Task 8 must port them from their evident intent and say so, rather than claiming behaviour preservation it cannot verify.

- [ ] **Step 4b: Constrain `world.draw`'s save path**

While you are in `crates/scripting_lua/src/globals/`, fix a sandbox escape in the neighbouring `world.draw` binding. It takes a path string straight from the Lua script and `crates/core/src/test_utils.rs:232` does `buffer.save(cwd.join(save_path)).unwrap()`.

`Path::join` **replaces the base entirely** when the argument is absolute, so `world.draw("/etc/x.png")` writes to `/etc/x.png`. A relative `"../../x.png"` escapes upward, and a symlink planted at the target is followed. Lua scripts here are otherwise sandboxed to the exposed API and have no file-write capability, so this is the one primitive that breaks out. The `.unwrap()` also turns any write failure into a panic that takes down the caller.

Fix: resolve the path against the intended root and reject anything that escapes it — reject absolute paths outright, reject `..` components, and canonicalise the parent to confirm it stays under the root. Prefer `OpenOptions::create_new` semantics over check-then-write: `exists()` followed by `write()` follows a dangling symlink and writes through it, which is the same defect one directory over. Return a Lua error instead of panicking.

Add tests for: an absolute path is refused, a `..` escape is refused, a path through a symlink pointing outside the root is refused, and an ordinary relative name still works.

**Move `draw_world` out of `test_utils` as part of this fix.** It currently lives in `crates/core/src/test_utils.rs`, which `crates/core/src/lib.rs:47` deliberately does NOT gate behind `#[cfg(test)]` ("not possible because lua crate needs this"). That naming is the root cause: an `.unwrap()` and an unvalidated `join` are unremarkable in a test helper and indefensible in a live API path, so nobody looked twice. Fixing only the validation leaves the trap set for the next function added there.

`crates/core/src/draw.rs` already exists and holds the drawing primitives (`arrow_mut` and friends) — that is where it belongs. Move it there and update the two references: the import in `crates/scripting_lua/src/globals/world.rs` and the commented-out call at `crates/scripting_lua/src/lua_runner.rs:109`.

Verify first that `draw_world` is the only non-fixture consumer of `test_utils`: at time of writing, every other cross-crate use is `fixture_world`, which is a genuine test fixture that must stay public (a dependency's `#[cfg(test)]` items are invisible downstream, and the planner's unit tests need it). After the move, `test_utils` contains only fixtures and no longer exposes a filesystem write to the live API.

**Say this in the commit message.** The next person to read `test_utils.rs` will assume it is test-only, exactly as two agents did before checking the module declaration.

This is being fixed here rather than filed because a concurrent effort is about to expose script execution over HTTP, which turns a local sandbox escape into a remote arbitrary file write. That effort has agreed to gate on this fix and to verify it independently from its side rather than trusting that it landed.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-scripting-lua
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(lua): add the goal api backed by the new planner and executor" -- crates/scripting_lua
```

---

### Task 8: Retire the old planner

**Approved.** This deletes 1257 lines of user-facing code and removes the published Lua `plan.*` API. Sign-off was given for all eight tasks; proceed. Because it is the destructive task, it goes last and only after Tasks 1–7 are green — never fold any of it forward into an earlier task.

**Files:**
- Delete: `crates/core/src/plan/{plan_builder.rs, execute.rs}` (489 lines), `crates/core/src/graph/task_graph.rs` (655 lines)
- **KEEP `crates/core/src/plan/planner.rs`.** See the scope correction below — an earlier draft deleted it and that was wrong.
- Modify: `crates/core/src/plan/mod.rs` (drop the `plan_builder` and `execute` lines; **KEEP `pub mod planner;`**), `crates/core/src/graph/mod.rs` (drop `task_graph`), `crates/core/src/gantt_mermaid.rs` (test-only use), `crates/core/src/errors.rs:176` (cosmetic diagnostic string)
- Modify: `app/src-tauri/src/{scripting.rs, cli/lua.rs, gui/command/script.rs, repl/run_script.rs}`

**The contested files have landed. Signatures below are current as of `7583d09` — but still open each file and read it; do not code from this block alone.**

`run_script` and `language_by_filename` NO LONGER EXIST in `app/src-tauri/src/scripting.rs`; they moved to `crates/scripting_lua/src/run_script.rs`, re-exported as `factorio_bot_scripting_lua::{language_by_filename, run_script, run_script_file}`.

```rust
// app/src-tauri/src/scripting.rs — all that remains
pub async fn run_script_file(planner: &mut Planner, path: &str, bot_count: u8,
                             sink: Option<Arc<dyn OutputSink>>) -> miette::Result<(String, String)>

// crates/scripting_lua/src/run_script.rs — new home
pub fn language_by_filename(filename: &str) -> Option<&'static str>
pub async fn run_script_file(planner: &mut Planner, scripts_root: &Path, requested: &str,
                             bot_count: u8, sink: Option<Arc<dyn OutputSink>>) -> Result<(String, String)>
pub async fn run_script(planner: &mut Planner, language: &str, code: &str, scripts_root: &Path,
                        bot_count: u8, sink: Option<Arc<dyn OutputSink>>) -> Result<(String, String)>
```

`gui/command/script.rs`: every signature UNCHANGED (bodies changed). `repl/run_script.rs`: unchanged. `cli/lua.rs`: not modified at all.

**Three rules that come with this, all load-bearing:**

1. **`scripts_dir` now has ZERO callers workspace-wide. Use `ensure_scripts_dir`.** Adding a `scripts_dir` call back is a regression, not a convenience: it prefers `./scripts` relative to the process CWD, so the editor would list and save to one directory while execution ran from another. That split is exactly what the concurrent effort's task existed to eliminate.
2. **Never hardcode `sink: None` when forwarding.** The app-side wrapper is the only script-execution entry point; passing `None` severs the output-streaming path. If a call site looks like it is discarding a capability rather than forwarding it, that is a bug.
3. **A CLI behaviour change landed:** the old path stripped a leading `"scripts/"`, the new one does not. `factorio-bot lua scripts/example.lua` now fails where `example.lua` works. If this task touches CLI docs or example invocations, use the working form.

**Original guidance retained:** The concurrent web-server effort is moving `run_script_file` out of `app/src-tauri/src/scripting.rs` into `crates/scripting_lua`, giving it a `scripts_root` argument, and every one of these call sites changes signature as a result. That work is expected to land BEFORE this task. Do not work from the signatures quoted anywhere in this plan; open each file and read what is actually there. If `run_script_file` still lives in `app/src-tauri`, coordinate before touching it rather than racing.
- Delete: `crates/scripting_lua/src/globals/plan.rs`; modify `lua_runner.rs`, `roll_best_seed.rs`, `lua_docs.rs`
- Migrate: `scripts/{test_phase_2_1,test_phase_2_2,api_test,example,multi_client_test,lib}.lua`, `crates/scripting_lua/tests/script.lua`

### Scope correction: `Planner` survives

An earlier draft of this plan said "delete `crates/core/src/plan/`" wholesale. That is wrong, and acting on it would break far more than this plan intends.

`Planner` (`crates/core/src/plan/planner.rs`) holds four things, and this plan replaces exactly one of them:

| field / method | fate |
| --- | --- |
| `rcon: Option<Arc<FactorioRcon>>` | **stays** — the Lua `rcon.*` global is built from it |
| `real_world: Arc<FactorioWorld>` | **stays** — `rcon.*` needs it |
| `plan_world: Arc<FactorioWorld>` | **stays** — the Lua `world.*` global is built from it |
| `initiate_missing_players_with_default_inventory`, `update_plan_world`, `reset` | **stay** — general bot/world setup the new planner still needs |
| `graph: Arc<RwLock<TaskGraph>>` | **deleted** — this is the old planner being replaced |

So `Planner` remains the Lua runtime's context holder and **`run_lua(&mut Planner, ...)` keeps its signature**. Do not rename `Planner`; a rename here would be gratuitous churn across a crate another effort is actively editing.

What actually changes in `crates/scripting_lua`:
- `lua_runner.rs:43` — drop `let graph = planner.graph.clone();` and stop passing it.
- `lua_docs.rs:25` — drop the `create_lua_plan_builder` call; document `goal.*` instead.
- `roll_best_seed.rs:185` — `planner.graph().shortest_path()` has no replacement in the new planner. Either port it onto `Schedule::makespan` (a schedule's makespan is the same quantity this was using as a fitness score) or delete the seed-rolling path. Decide and say which; do not leave it referencing a deleted type.

**Do not delete** `crates/core/src/graph/entity_graph.rs`, `crates/core/src/graph/flow_graph.rs`, or `crates/core/src/factorio/factorio_planner.rs` — despite the names, the first two are live and the third only decodes blueprint zlib.

**Use core's script path resolution, do not reinvent it.** A concurrent migration added `factorio_bot_core::scripts` (commit `12e8451`) with `scripts_dir(workspace_path)` and `resolve_script_path(root, requested)`, both returning a miette `Result`. Anywhere this task needs to locate a script on disk, call those rather than rebuilding path logic in the binary.

**One trap in `resolve_script_path`:** it deliberately resolves `"/"` and `""` to the root directory itself, because a directory-listing endpoint needs that. It does not distinguish a file from a directory — that check belongs to the caller. So a migration step that resolves a script path and then reads it must verify it got a file, or it will try to read the scripts directory and fail with a confusing error. Related additive modules that landed in core during this work and may save you effort: `factorio_bot_core::paths`, `factorio_bot_core::app_settings` (`AppSettings`, `GuiSettings`, `SharedAppSettings`, `load_app_settings`), and `factorio_bot_core::settings::RestApiSettings`.

**Clean up seven write-only snapshot artifacts while you are here.** `crates/scripting_lua/tests/` contains `task_graph-1.dot`, `task_graph-1.md`, `task_graph-2.dot`, `task_graph-2.md`, `world_end-1.png`, `world_end-2.png` and `world_start.png`. Nothing reads or compares them: `lua_runner.rs:133` writes `stdout-N.txt`, the fixture script emits the rest, and no assertion touches any of them. They were last regenerated 122 commits ago (`4b68a8b`) and have drifted since — running the suite today rewrites them with different entity positions, and every test still passes.

Note that `crates/scripting_lua/tests/` contains no `.rs` files at all — it is a data directory holding the fixture `script.lua` and its outputs, so there is no test in it that could do any comparing.

All seven are still actively regenerated, so none of them is merely dead. The four `task_graph-*` files come from the planner this task deletes; delete them with their producer. The three PNGs come from the fixture itself — `script.lua:161` calls `world.draw("world_start.png")` and line 176 calls `world.draw("world_end-" .. bot_count .. ".png")`, routed through the live `draw_world` at `crates/scripting_lua/src/globals/world.rs:200`. (Do not be misled by the commented-out `draw_world` at `lua_runner.rs:109`; that is a superseded duplicate path, not the producer.) So the PNGs outlive this task, and the choice for them is real: make them assertions or delete them.

Either way, do not leave a file in the repo that regenerates differently on every run and is compared to nothing — it reads as a snapshot test to the next person and is not one.

**Migration strategy, decided from a script-by-script survey.** The groundwork report is at `.superpowers/sdd/2026-08-30-planner-execution-increment/task-8-groundwork.md` — read it before starting; it lists every call site with line numbers.

The survey found three scripts whose behaviour "cannot be expressed in `goal.*`". That framing is right but the conclusion is wrong: they do not need `goal.*`. **`rcon.*` survives untouched and already provides the primitives they use** — `rcon.move(player_id, position, radius)`, `rcon.mine(player_id, name, position, count)`, `rcon.craft`, `rcon.place_entity`. The distinction that matters is that `plan.walk` was *scheduled* work the planner reasoned about, while `rcon.move` is an immediate command. For a test or demo script driving specific bots to specific places, immediate is what it actually wanted.

So migrate by intent, not mechanically:

| script | disposition |
| --- | --- |
| `lib.lua` | `mine_with_bots` / `find_mine_with_bots` → `goal.have`. `mine_rocks` → `rcon.mine`; rocks are not an item-count goal. |
| `scripting_lua/tests/script.lua` | `build_starter_miner_furnace` → `goal.have`. **Delete `build_starter_coal_loop`** — it is dead and references an undefined global `ore`. This is the only script with a live test asserting on it; keep that test passing. |
| `api_test.lua`, `example.lua` | locomotion → `rcon.move`; `plan.task_graph_mermaid_gantt` → `goal.gantt`. |
| `multi_client_test.lua` | pins specific bots to specific destinations — that is `rcon.move` per bot, not a goal. Migrate wholly onto `rcon.*`. |
| `test_phase_2_1.lua`, `test_phase_2_2.lua` | Both call `plan.insert_into_inventory`, which was never bound, so **neither has ever run past that line**. Port from evident intent onto `goal.have` where it fits, and say in the commit that behaviour preservation was impossible to verify because the scripts never ran. Do not claim otherwise. |

**`roll_best_seed`: delete the `TaskGraph`-dependent scoring, do not port it.** The survey found it is already unreachable — its only caller loop is commented out and `score_seed`/`find_nearest_entities` have zero live callers. Porting would require new plumbing to read a `Schedule` back out of the handle-based Lua runtime, for a feature nothing exercises. Delete `score_seed`, `find_nearest_entities`, and the dead caller; leave a comment naming what was removed and why, so the intent is recoverable.

Also update the stale prose in `PLAN.md` (7 locations per the survey), including the now-moot TODO about resurrecting `roll_best_seed`.

- [ ] **Step 1: Migrate the Lua scripts first**

Rewrite each script's `plan.*` calls onto `goal.*`. Run each one that has a test harness. A script whose behaviour cannot be preserved gets a comment at the top naming what changed and why — do not silently drop functionality.

Expect `plan.group_start` / `plan.group_end` to have no replacement; that is a decided loss, not an oversight. Where a script used them, delete the bracketing and add a one-line comment saying the planner now derives grouping. Do not invent a `goal.group` to preserve them.

- [ ] **Step 2: Remove the Lua plan table**

Delete `globals/plan.rs`, drop its registration from `lua_runner.rs`, and update `lua_docs.rs` and `roll_best_seed.rs`.

Run: `cargo test -p factorio-bot-scripting-lua`

- [ ] **Step 3: Remove the Rust callers**

The four `app/src-tauri` sites keep constructing `Planner` — it survives. They change only where they referenced the old task graph, and where the concurrent effort's `run_script_file` move altered their signatures. Update `gantt_mermaid.rs`'s test-only use to build a `Schedule` instead of a `TaskGraph`. Change the `errors.rs:176` diagnostic string.

Run: `cargo check --workspace --all-features`

- [ ] **Step 4: Delete the old planner**

```bash
git rm -r crates/core/src/plan crates/core/src/graph/task_graph.rs
```

**Do NOT drop `pub mod plan;` from `crates/core/src/lib.rs`** — an earlier draft said to, and that contradicts the scope correction above. `Planner` lives at `crates/core/src/plan/planner.rs` and survives. Drop only the `plan_builder` and `execute` lines from `crates/core/src/plan/mod.rs`, and the `task_graph` line from `crates/core/src/graph/mod.rs`.

- [ ] **Step 5: Verify and commit**

Run: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` then `cargo test --workspace`
Expected: green, with no reference to `TaskGraph` remaining: `grep -rn "TaskGraph\|PlanBuilder" --include="*.rs" crates app` returns nothing.

The changelog is generated from commit messages by git-cliff, wired as a `cargo-release` pre-release hook (`release.toml:2`, config in `Cargo.toml`). So the `BREAKING CHANGE:` footer below is the only thing that carries this to users at release time — a script in the wild calling `plan.*` breaks at *runtime*, not at build time, and the published Lua API docs regenerate silently from `lua_docs.rs`. The footer is not decoration; write it.

```bash
cargo fmt -p factorio-bot-core -p factorio-bot-scripting-lua
git commit -F- -- crates app scripts <<'MSG'
feat!: replace the task-graph planner with the goal planner and executor

The Lua `plan.*` global is removed and replaced by `goal.*`, backed by the
new goal planner and the executor crate. `world.*` and `rcon.*` are
unchanged.

BREAKING CHANGE: the Lua `plan.*` API is removed. Scripts calling
`plan.mine`, `plan.place`, `plan.walk`, `plan.group_start`,
`plan.group_end`, `plan.finalize`, `plan.task_graph_graphviz` or
`plan.task_graph_mermaid_gantt` fail at runtime rather than at build time.

Migrate as follows:
  plan.mine / plan.place / plan.walk  ->  goal.have(item, count)
      The planner derives mining, placement and travel from the goal;
      you no longer schedule them individually.
  plan.group_start / plan.group_end   ->  removed, no replacement.
      Grouping is derived from the goal decomposition. To force specific
      work onto a specific bot, use the planner's pin rather than a
      Lua-side bracket.
  plan.finalize                       ->  goal.schedule(handle, bots)
  plan.task_graph_graphviz            ->  goal.graphviz(handle)
  plan.task_graph_mermaid_gantt       ->  removed; render from the
      Schedule instead.

Execution is now explicit and non-blocking: goal.execute(handle) returns
a run handle, goal.progress(handle) reports live status, and
goal.wait(handle) blocks for completion.
MSG
```

---

## Resolved before this plan: furnace siting

Planning surfaced a defect that would have made this whole increment fail at its first `Place`, and it has been fixed ahead of the plan in commit `9dc6c7b`.

`PlanState::is_position_free` consulted only the entity tree, and `EntityGraph::add` routes resource entities into `resources`/`resource_tree` instead — the same routing asymmetry behind the earlier `resource_available` bug. Ore tiles therefore read as free, and `Smelt` sited its furnace on top of the ore patch: measured furnace-to-ore distance **0**. Harmless to the planner's arithmetic, which is why it survived three plans of review, and fatal to execution, because the game rejects `place_entity` on an ore tile. Worse, it would have failed *silently* until the first live run — and recovery could not have helped, since tier 2 re-expands into the same invalid site.

The fix adds a read-only `EntityGraph::any_resource_at` in core (`resource_contains` is per-resource-name and cannot answer "is anything here") and makes `is_position_free` consult it. Furnace-to-ore is now sqrt(2) — one diagonal tile off the patch — and a regression assertion pins `to_ore > 0`.

It moved the recorded red-science makespans, as expected, because every ore-to-furnace trip gained that step: one bot 4749 -> 4751, four bots 1843 -> 1870, speedup 2.577x -> 2.541x. All re-measured, not computed.

## Remaining open question

**Game speed.** `ticks_to_wall_clock` assumes 60 ticks per second. A server running at a non-default `game.speed`, or one merely stuttering under load, makes every lag wait wrong — too short, and the executor removes a plate that is not smelted yet.

Reading `game.speed` over RCON at construction fixes the configured case but not the stutter case. The proper fix is to wait on an observed game tick rather than wall-clock: the mod already writes tick-stamped events, so the executor could await "tick >= start + lag" instead of sleeping.

Not resolved, and deliberately not blocking: the wall-clock version is correct on a default server at normal speed, which is every current use. It is recorded here because it is the most likely source of flaky smelting, and a flake here will look like a mysterious RCON failure rather than a timing bug. If smelting proves unreliable in practice, this is the first thing to suspect.

## Decisions already made

These were settled with the repository owner before implementation and are not open. Do not relitigate them mid-task:

- **Bot-to-player mapping**: the executor discovers it (Task 3). No caller supplies a map.
- **`goal.execute`**: handle-based, non-blocking (Tasks 5 and 7).
- **Manual grouping**: dropped, with no `goal.*` equivalent (Task 7). Task 8's script migration will lose explicit `plan.group_start`/`group_end` bracketing; that is accepted, because the planner derives grouping from the goal decomposition itself.
- **Scope**: all eight tasks, including the deletion of the old planner and the `plan.*` API.
