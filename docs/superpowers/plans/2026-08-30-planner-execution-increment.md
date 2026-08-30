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
- Verification command for the whole workspace: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` then `cargo nextest run`.

## Sign-off Gate

**Tasks 1–7 are additive and reversible. Task 8 deletes user-facing code and removes a published Lua API.**

Do not begin Task 8 without explicit approval from the repository owner. If approval is not present, stop after Task 7, report that tasks 1–7 are complete, and leave the old `plan.*` API in place. Both APIs coexisting is a supported end state.

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
- Produces: `trait Actuator`, `ActuatorError`, `RconActuator::new(rcon, world, bots) -> Result<Self, ActuatorError>`. Task 4 is generic over `Actuator`.

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
    pub async fn new(
        rcon: Arc<FactorioRcon>,
        world: Arc<FactorioWorld>,
        players: BTreeMap<BotId, PlayerId>,
    ) -> Result<Self, ActuatorError> {
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

Then the trait impl, mapping each method onto the RCON call. Two representative arms; write the rest by the same pattern:

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

    // mine -> player_mine(&world, p, item, &at, count)
    // craft -> player_craft(&world, p, recipe, count)
    // place -> place_entity(p, item.to_string(), at, direction, &world) — discard the returned entity
    // remove -> remove_from_inventory(p, entity, at, inv, item, count, &world)
    // research -> add_research(tech) — server-wide, takes no player id
}
```

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test -p factorio-bot-executor`
Expected: PASS.

```bash
cargo fmt -p factorio-bot-executor
cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings
git commit -m "feat(executor): add the actuator trait and its rcon implementation" -- crates/executor crates/core
```

---

### Task 4: Run one bot's schedule

**Files:**
- Create: `crates/executor/src/run.rs`
- Modify: `crates/executor/src/lib.rs`
- Test: inline in `run.rs`

**Interfaces:**
- Consumes: `Actuator` (Task 3), `ExecutionLog` (Task 2), `planner::{Schedule, ScheduledStep, StepKind, ActionNetwork}`.
- Produces: `async fn run_bot(...) -> ExecutionLog`. Task 5 calls it once per bot.

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

**Interfaces:**
- Produces: `async fn run(act: Arc<dyn Actuator>, sched: &Schedule, net: &ActionNetwork) -> ExecutionLog`.

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
    let log = run(Arc::new(act), &sched, &net).await;

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
    let log = run(Arc::new(act), &sched, &net).await;

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
use tokio::sync::watch;

enum PredOutcome {
    Ready,
    Abandoned,
}

pub async fn run(act: &dyn Actuator, sched: &Schedule, net: &ActionNetwork) -> ExecutionLog {
    let mut senders: BTreeMap<ActionId, watch::Sender<Status>> = BTreeMap::new();
    let mut receivers: BTreeMap<ActionId, watch::Receiver<Status>> = BTreeMap::new();
    for id in net.actions().map(|a| a.id) {
        let (tx, rx) = watch::channel(Status::Pending);
        senders.insert(id, tx);
        receivers.insert(id, rx);
    }

    // BTreeSet, so the future order — and therefore the merge order — is a
    // function of the schedule alone, not of task completion timing.
    let bots: BTreeSet<BotId> = sched.steps.iter().map(|s| s.bot).collect();
    let logs = join_all(
        bots.iter()
            .map(|&bot| run_bot_signalled(act, bot, sched, net, &senders, &receivers)),
    )
    .await;

    let mut merged = ExecutionLog::default();
    for l in logs {
        merged.merge(l);
    }
    merged
}

async fn run_bot_signalled(
    act: &dyn Actuator,
    bot: BotId,
    sched: &Schedule,
    net: &ActionNetwork,
    senders: &BTreeMap<ActionId, watch::Sender<Status>>,
    receivers: &BTreeMap<ActionId, watch::Receiver<Status>>,
) -> ExecutionLog {
    let mut log = ExecutionLog::default();
    let mine: Vec<&ScheduledStep> = sched.steps.iter().filter(|s| s.bot == bot).collect();

    for (i, step) in mine.iter().enumerate() {
        match &step.what {
            StepKind::Walk { to } => {
                if act.walk(bot, to.clone()).await.is_err() {
                    abandon_rest(&mine[i..], senders);
                    return log;
                }
            }
            StepKind::Act { action, .. } => {
                if let PredOutcome::Abandoned = await_preds(net, *action, receivers).await {
                    abandon_rest(&mine[i..], senders);
                    return log;
                }
                log.start(*action, step.start);
                let Some(a) = net.action(*action) else {
                    log.fail(*action, step.start, "action not in network".to_string());
                    abandon_rest(&mine[i..], senders);
                    return log;
                };
                match perform(act, bot, &a.kind).await {
                    Ok(()) => {
                        log.succeed(*action, step.end);
                        let _ = senders[action].send(Status::Success);
                    }
                    Err(e) => {
                        log.fail(*action, step.end, e.to_string());
                        abandon_rest(&mine[i..], senders);
                        return log;
                    }
                }
            }
        }
    }
    log
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

Add `merge` to `ExecutionLog` in `crates/executor/src/log.rs`, with a test that merging is order-independent for disjoint key sets:

```rust
    /// Absorb another log. Bots own disjoint action sets, so no key collides;
    /// if one ever does, the later write wins and that is a bug worth finding.
    pub fn merge(&mut self, other: ExecutionLog) {
        for (id, attempt) in other.attempts {
            debug_assert!(
                !self.attempts.contains_key(&id),
                "two bots reported the same action {id:?}"
            );
            self.attempts.insert(id, attempt);
        }
    }
```

Both accessors the loop needs already exist in `crates/planner/src/network.rs`: `actions()` returns an iterator over `&Action` (line 84) and `preds(id)` returns `Vec<(ActionId, Ticks)>` (line 97). Do not add new ones.

Use `tokio::time::pause()` in the tests so lag sleeps do not make the suite slow — that is why `test-util` is in the dev-dependencies.

- [ ] **Step 4: Run the tests, then the whole workspace**

Run: `cargo test -p factorio-bot-executor` then `cargo nextest run`
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

Run: `cargo test -p factorio-bot-executor` then `cargo nextest run`

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

Bind five functions:

```lua
-- @treturn table a plan handle
function goal.have(item_name, count)
-- @treturn table a plan handle
function goal.researched(technology_name)
-- @treturn number the makespan in ticks
function goal.schedule(plan_handle, bot_count)
-- @treturn string graphviz source
function goal.graphviz(plan_handle)
-- executes the schedule; blocks until every bot is done
function goal.execute(plan_handle)
```

`goal.have` and `goal.researched` build a `Goal`, run `expand`, and return a handle holding the `ActionNetwork`. `goal.schedule` runs `schedule` and stores the result on the handle. `goal.execute` builds an `RconActuator` from the live `FactorioInstance` and calls `executor::run`.

- [ ] **Step 2: Write the failing test script**

`crates/scripting_lua/tests/goal_script.lua`:

```lua
local p = goal.have("automation-science-pack", 10)
local makespan = goal.schedule(p, 4)
assert(makespan > 0, "expected a positive makespan")
assert(#goal.graphviz(p) > 0, "expected graphviz output")
```

Add a Rust test that runs it through the same harness `lua_runner.rs`'s existing `test_script` uses. Do not call `goal.execute` in this test — it needs a live game.

- [ ] **Step 3: Run it to make sure it fails, then implement, then pass**

Run: `cargo test -p factorio-bot-scripting-lua`
Expected: FAIL first (`goal` is nil), PASS after implementation.

- [ ] **Step 4: Fix the long-standing unbound binding**

`PlanBuilder::add_insert_into_inventory` exists in Rust but was never bound to Lua, while `scripts/test_phase_2_1.lua` and `scripts/test_phase_2_2.lua` call `plan.insert_into_inventory` — those scripts fail at runtime today. Since Task 8 may not run, fix it here on the old API: bind `plan.insert_into_inventory` in `crates/scripting_lua/src/globals/plan.rs` following the neighbouring bindings, and verify both scripts parse.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p factorio-bot-scripting-lua
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git commit -m "feat(lua): add the goal api backed by the new planner and executor" -- crates/scripting_lua
```

---

### Task 8: Retire the old planner — REQUIRES SIGN-OFF

**Do not start this task without explicit approval from the repository owner.** It deletes 1257 lines of user-facing code and removes a published Lua API. Tasks 1–7 stand on their own if approval is withheld.

**Files:**
- Delete: `crates/core/src/plan/` (4 files, 602 lines), `crates/core/src/graph/task_graph.rs` (655 lines)
- Modify: `crates/core/src/lib.rs:41` (drop `pub mod plan;`), `crates/core/src/graph/mod.rs`, `crates/core/src/gantt_mermaid.rs` (test-only use), `crates/core/src/errors.rs:176` (cosmetic diagnostic string)
- Modify: `app/src-tauri/src/{scripting.rs, cli/lua.rs, gui/command/script.rs, repl/run_script.rs}`
- Delete: `crates/scripting_lua/src/globals/plan.rs`; modify `lua_runner.rs`, `roll_best_seed.rs`, `lua_docs.rs`
- Migrate: `scripts/{test_phase_2_1,test_phase_2_2,api_test,example,multi_client_test,lib}.lua`, `crates/scripting_lua/tests/script.lua`

**Do not delete** `crates/core/src/graph/entity_graph.rs`, `crates/core/src/graph/flow_graph.rs`, or `crates/core/src/factorio/factorio_planner.rs` — despite the names, the first two are live and the third only decodes blueprint zlib.

- [ ] **Step 1: Migrate the Lua scripts first**

Rewrite each script's `plan.*` calls onto `goal.*`. Run each one that has a test harness. A script whose behaviour cannot be preserved gets a comment at the top naming what changed and why — do not silently drop functionality.

- [ ] **Step 2: Remove the Lua plan table**

Delete `globals/plan.rs`, drop its registration from `lua_runner.rs`, and update `lua_docs.rs` and `roll_best_seed.rs`.

Run: `cargo test -p factorio-bot-scripting-lua`

- [ ] **Step 3: Remove the Rust callers**

Update the four `app/src-tauri` sites to construct the new planner path instead of `Planner`. Update `gantt_mermaid.rs`'s test-only use to build a `Schedule` instead of a `TaskGraph`. Change the `errors.rs:176` diagnostic string.

Run: `cargo check --workspace --all-features`

- [ ] **Step 4: Delete the old planner**

```bash
git rm -r crates/core/src/plan crates/core/src/graph/task_graph.rs
```

Drop `pub mod plan;` from `crates/core/src/lib.rs` and the `task_graph` line from `crates/core/src/graph/mod.rs`.

- [ ] **Step 5: Verify and commit**

Run: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` then `cargo nextest run`
Expected: green, with no reference to `TaskGraph` remaining: `grep -rn "TaskGraph\|PlanBuilder" --include="*.rs" crates app` returns nothing.

```bash
cargo fmt -p factorio-bot-core -p factorio-bot-scripting-lua
git commit -m "feat!: replace the task-graph planner with the goal planner and executor" -- crates app scripts
```

---

## Open questions for the repository owner

1. **Bot-to-player mapping.** `RconActuator` takes `BTreeMap<BotId, PlayerId>`. Who builds it? The natural place is wherever clients are spawned, but that code currently has no notion of `BotId`. Task 3 assumes the caller supplies it.
2. **`goal.execute` blocking.** The Lua binding blocks until every bot finishes. A long plan makes the script unresponsive. An async/handle-based API would be better but complicates the Lua surface; deferred.
3. **Game speed.** `ticks_to_wall_clock` assumes 60 ticks per second. A server running at a non-default `game.speed`, or one that stutters under load, makes every lag wait wrong. Reading `game.speed` over RCON at executor construction would fix the first case but not the second; properly, the executor should wait on an observed game tick rather than wall-clock. Deferred, and it is the most likely source of flaky smelting.
4. **Task 8's script migration** may lose behaviour that only the old API expressed (explicit `plan.group_start`/`group_end` bracketing has no `goal.*` equivalent — the planner derives grouping itself). Confirm that losing manual grouping is acceptable.
