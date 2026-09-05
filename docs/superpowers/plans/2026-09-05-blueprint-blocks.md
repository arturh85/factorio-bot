# Blueprint Blocks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `goal.built(blueprint, position)` makes four bots build a designed block by hand — walking, reaching and paying for every entity — starting with the repo's 37-entity `MinerLine` and ending with its 179-entity `FurnaceLine`.

**Architecture:** A blueprint string is decoded in `crates/core` into entities, offsets and directions (pure data, no game). A new `Goal::Built { blueprint, anchor }` expands to *the entities not yet standing*, so it is re-checkable on every replan and a no-op if built twice. A new planner method emits ordinary `ActionKind::Place` actions — the same shape `connect.rs` and `power.rs` emit — so walking, reach, step-aside and divergence handling all come for free. The roster splits the block into vertical bands balanced by entity count.

**Tech Stack:** Rust 2024. `base64 0.23`, `flate2 1.0`, `serde_json 1.0` — **all three are already dependencies of `crates/core`; this plan adds none.**

**Spec:** `docs/superpowers/specs/2026-09-05-blueprint-blocks-design.md`

## Global Constraints

- **Every cargo command needs `nix develop -c`**, with `CARGO_TARGET_DIR=/home/arturh/projects/private/factorio-bot/.worktrees/blocks/target`.
- **Never `cargo fmt`.** Format single files: `nix develop -c rustfmt --edition 2024 <file>`. The `--edition 2024` is not optional; bare `rustfmt` defaults to 2015 and dies on every `async fn`.
- **Commit with explicit paths.** Never `git add -A`, never `--amend`, never `git stash`.
- **`crates/planner` is pure and deterministic**: no I/O, no async, no wall-clock, ordered collections only, float ordering via `total_cmp`.
- **Another session owns `crates/planner/src/method/{have,produce,assemble,power}.rs`.** Task 2 adds exactly ONE line to `have.rs`'s `default_registry()`. Touch nothing else in those files, and keep `method/mod.rs` edits to the module-registration line.
- **Refuse before placing.** Every failure path returns a refusal rather than a partial list of actions.
- **A placement count is not evidence.** This project has twice shipped layouts that placed 100% correctly and did nothing. Live verification reads the world back.
- **Live runs use `just headless` semantics** (`--headless --bots 4 --game-speed 5`) on ports **34200/4324** with their own settings file — never the defaults, which belong to another session running measured experiments. Ask before starting one.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/blueprint.rs` | **Create.** Decode a blueprint string to entities/offsets/directions; migrate pre-2.0 directions. Pure. |
| `crates/core/src/lib.rs` | **Modify.** `pub mod blueprint;` |
| `crates/core/tests/blueprint_decode.rs` | **Create.** Decode fixtures against the repo's own real blueprints. |
| `crates/core/tests/blueprints/{miner_line,furnace_line}.txt` | **Create.** The two strings, copied from `scripts/rcontest.lua`. |
| `crates/planner/src/goal.rs` | **Modify.** Add the `Built` variant. |
| `crates/planner/src/method/blueprint.rs` | **Create.** `BuildBlock`: band split, entities-not-yet-standing, placement emission, refusals. |
| `crates/planner/src/method/mod.rs` | **Modify.** One line: `pub mod blueprint;` |
| `crates/planner/src/method/have.rs` | **Modify.** One line in `default_registry()`. |
| `crates/scripting_lua/src/globals/goal/value.rs` | **Modify.** `goal.built(...)` constructor. |
| `crates/core/src/types.rs`, `crates/core/src/factorio/rcon.rs`, `mods/BotBridge/control.lua` | **Modify (Task 5).** Carry an underground belt's input/output half. |

---

### Task 1: Decode a blueprint, and migrate its directions

**Files:**
- Create: `crates/core/src/blueprint.rs`, `crates/core/tests/blueprint_decode.rs`, `crates/core/tests/blueprints/miner_line.txt`, `crates/core/tests/blueprints/furnace_line.txt`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Produces: `Blueprint { pub entities: Vec<BlueprintEntity>, pub version: u64 }`, `BlueprintEntity { pub name: String, pub offset: Position, pub direction: u8, pub underground_half: Option<UndergroundHalf> }`, `enum UndergroundHalf { Input, Output }`, `enum BlueprintError { NotBase64, NotDeflate, NotJson, NoBlueprint, Unsupported(String) }`, and `pub fn decode(text: &str) -> Result<Blueprint, BlueprintError>`.

- [ ] **Step 1: Copy the fixtures.** From `scripts/rcontest.lua`, copy the `MinerLine` string into `crates/core/tests/blueprints/miner_line.txt` and `FurnaceLine` into `furnace_line.txt`, each as one line with no quotes and no trailing newline issues. Do not retype them; extract them.

- [ ] **Step 2: Write the failing test.** Create `crates/core/tests/blueprint_decode.rs`:

```rust
use factorio_bot_core::blueprint::{UndergroundHalf, decode};
use std::collections::BTreeMap;

fn counts(bp: &factorio_bot_core::blueprint::Blueprint) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for e in &bp.entities {
        *out.entry(e.name.clone()).or_insert(0) += 1;
    }
    out
}

#[test]
fn the_miner_line_decodes_to_its_37_entities() {
    let bp = decode(include_str!("blueprints/miner_line.txt").trim()).expect("decodes");
    assert_eq!(bp.entities.len(), 37);
    let c = counts(&bp);
    assert_eq!(c.get("electric-mining-drill"), Some(&13));
    assert_eq!(c.get("transport-belt"), Some(&21));
    assert_eq!(c.get("small-electric-pole"), Some(&3));
}

/// **The trap this test exists for.** These blueprints are Factorio 1.x
/// (`version` 281474976710656 == 1.0.0.0) and their directions are on the old
/// EIGHT-point scale, where 2 is east. Factorio 2.0 uses SIXTEEN points, where
/// east is 4 and 2 is a diagonal. `import_stack` migrates on import, so the mod
/// never had to care -- decoding here bypasses that entirely, and raw 1.x
/// directions would turn every belt and inserter a half-turn: a factory that
/// places 100% correctly and moves nothing.
#[test]
fn pre_two_point_zero_directions_are_doubled_onto_the_sixteen_point_scale() {
    let bp = decode(include_str!("blueprints/miner_line.txt").trim()).expect("decodes");
    assert_eq!(bp.version, 281474976710656, "fixture is a 1.x blueprint");
    assert!(
        bp.entities.iter().all(|e| e.direction % 4 == 0),
        "every migrated direction must be a cardinal on the 16-point scale: {:?}",
        bp.entities.iter().map(|e| e.direction).collect::<Vec<_>>()
    );
    assert!(
        bp.entities.iter().any(|e| e.direction == 4),
        "the fixture has east-facing entities, which must be 4 and not 2"
    );
}

#[test]
fn an_underground_belt_carries_which_half_it_is() {
    let bp = decode(include_str!("blueprints/furnace_line.txt").trim()).expect("decodes");
    let halves: Vec<Option<UndergroundHalf>> = bp
        .entities
        .iter()
        .filter(|e| e.name == "underground-belt")
        .map(|e| e.underground_half)
        .collect();
    assert_eq!(halves.len(), 2, "the furnace line has one pair");
    assert!(halves.contains(&Some(UndergroundHalf::Input)));
    assert!(halves.contains(&Some(UndergroundHalf::Output)));
}

#[test]
fn a_string_that_is_not_a_blueprint_is_refused_by_name() {
    assert!(matches!(
        decode("not a blueprint"),
        Err(factorio_bot_core::blueprint::BlueprintError::NotBase64)
            | Err(factorio_bot_core::blueprint::BlueprintError::NotDeflate)
    ));
}
```

- [ ] **Step 3: Run it and watch it fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test blueprint_decode`
Expected: FAIL — `unresolved import factorio_bot_core::blueprint`.

- [ ] **Step 4: Implement the decoder.** Create `crates/core/src/blueprint.rs`:

```rust
//! Decoding a Factorio blueprint string into entities we can place.
//!
//! The format is a version byte (`0`), then base64, then zlib, then JSON.
//! Nothing here touches the game: a blueprint decodes in a unit test, and a
//! block can be planned against a world dump with nothing running.

use crate::types::Position;
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndergroundHalf {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct BlueprintEntity {
    pub name: String,
    /// Offset from the blueprint's own origin, not a world position.
    pub offset: Position,
    /// Already migrated to Factorio 2.0's 16-point scale.
    pub direction: u8,
    /// `Some` only for `underground-belt`; the blueprint names which half.
    pub underground_half: Option<UndergroundHalf>,
}

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub entities: Vec<BlueprintEntity>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlueprintError {
    NotBase64,
    NotDeflate,
    NotJson,
    NoBlueprint,
    /// Content this build refuses to place rather than silently drop.
    Unsupported(String),
}

/// Factorio encodes a version as four 16-bit fields; 2.0.0.0 is this value.
/// Anything below it used the eight-point direction scale.
const VERSION_2_0: u64 = 2 << 48;

#[derive(Deserialize)]
struct Envelope {
    blueprint: Option<Body>,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    entities: Vec<RawEntity>,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    tiles: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawEntity {
    name: String,
    position: RawPos,
    #[serde(default)]
    direction: u8,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    items: Option<serde_json::Value>,
    #[serde(default)]
    recipe: Option<String>,
}

#[derive(Deserialize)]
struct RawPos {
    x: f64,
    y: f64,
}

pub fn decode(text: &str) -> Result<Blueprint, BlueprintError> {
    // The leading byte is the format version, not part of the payload.
    let payload = text.strip_prefix('0').unwrap_or(text);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|_| BlueprintError::NotBase64)?;
    let mut json = Vec::new();
    {
        use std::io::Read;
        flate2::read::ZlibDecoder::new(&raw[..])
            .read_to_end(&mut json)
            .map_err(|_| BlueprintError::NotDeflate)?;
    }
    let envelope: Envelope =
        serde_json::from_slice(&json).map_err(|_| BlueprintError::NotJson)?;
    let body = envelope.blueprint.ok_or(BlueprintError::NoBlueprint)?;

    if !body.tiles.is_empty() {
        return Err(BlueprintError::Unsupported("tiles".into()));
    }

    let mut entities = Vec::with_capacity(body.entities.len());
    for e in body.entities {
        if e.items.is_some() {
            return Err(BlueprintError::Unsupported(format!(
                "{} carries module or item requests",
                e.name
            )));
        }
        if e.recipe.is_some() {
            return Err(BlueprintError::Unsupported(format!(
                "{} carries a recipe",
                e.name
            )));
        }
        let underground_half = match e.kind.as_deref() {
            Some("input") => Some(UndergroundHalf::Input),
            Some("output") => Some(UndergroundHalf::Output),
            _ => None,
        };
        entities.push(BlueprintEntity {
            name: e.name,
            offset: Position::new(e.position.x, e.position.y),
            direction: migrate_direction(e.direction, body.version),
            underground_half,
        });
    }
    Ok(Blueprint {
        entities,
        version: body.version,
    })
}

/// **Pre-2.0 blueprints used eight directions; 2.0 uses sixteen.**
/// Doubling is the whole migration: old 2 (east) becomes 4 (east).
fn migrate_direction(direction: u8, version: u64) -> u8 {
    if version < VERSION_2_0 {
        direction.saturating_mul(2)
    } else {
        direction
    }
}
```

- [ ] **Step 5: Register the module.** Add `pub mod blueprint;` to `crates/core/src/lib.rs`, beside the other `pub mod` lines.

- [ ] **Step 6: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test blueprint_decode`
Expected: 4 passed. If the furnace-line test fails with `Unsupported`, read the message: that blueprint may carry something the refusal list names, which is correct behaviour — adjust the *test* to assert the refusal by name, and say so in your report.

- [ ] **Step 7: Format and commit.**

```bash
nix develop -c rustfmt --edition 2024 crates/core/src/blueprint.rs
nix develop -c rustfmt --edition 2024 crates/core/tests/blueprint_decode.rs
git commit -m "feat(core): decode a blueprint, and migrate its pre-2.0 directions" -- crates/core/src/blueprint.rs crates/core/src/lib.rs crates/core/tests/blueprint_decode.rs crates/core/tests/blueprints/
```

---

### Task 2: The goal, the method, and the band split

**Files:**
- Create: `crates/planner/src/method/blueprint.rs`
- Modify: `crates/planner/src/goal.rs`, `crates/planner/src/method/mod.rs` (one line), `crates/planner/src/method/have.rs` (one line)

**Interfaces:**
- Consumes: `factorio_bot_core::blueprint::{Blueprint, BlueprintEntity, decode}` from Task 1.
- Produces: `Goal::Built { blueprint: String, anchor: Position }`; `pub struct BuildBlock` implementing `Method`; `pub fn bands(entities: &[BlueprintEntity], bots: usize) -> Vec<Vec<usize>>`.

- [ ] **Step 1: Write the failing band test.** Create `crates/planner/src/method/blueprint.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::blueprint::{BlueprintEntity, UndergroundHalf};
    use factorio_bot_core::types::Position;

    fn at(x: f64) -> BlueprintEntity {
        BlueprintEntity {
            name: "transport-belt".into(),
            offset: Position::new(x, 0.0),
            direction: 4,
            underground_half: None::<UndergroundHalf>,
        }
    }

    /// Bands are balanced by ENTITY COUNT, not by area: a block whose entities
    /// bunch at one end must still divide into equal work, or one bot builds
    /// while three watch.
    #[test]
    fn bands_split_by_count_not_by_width() {
        // Twelve entities: nine crowded in x 0..3, three spread to x 40..42.
        let mut ents: Vec<BlueprintEntity> = Vec::new();
        for i in 0..9 {
            ents.push(at((i % 3) as f64));
        }
        for i in 0..3 {
            ents.push(at(40.0 + i as f64));
        }
        let bands = bands(&ents, 3);
        assert_eq!(bands.len(), 3);
        for b in &bands {
            assert_eq!(b.len(), 4, "each of 3 bots takes 4 of 12: {bands:?}");
        }
    }

    #[test]
    fn every_entity_lands_in_exactly_one_band() {
        let ents: Vec<BlueprintEntity> = (0..10).map(|i| at(i as f64)).collect();
        let bands = bands(&ents, 4);
        let mut seen: Vec<usize> = bands.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn bands_are_deterministic() {
        let ents: Vec<BlueprintEntity> = (0..17).map(|i| at((i % 5) as f64)).collect();
        assert_eq!(bands(&ents, 4), bands(&ents, 4));
    }
}
```

- [ ] **Step 2: Run and watch it fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner blueprint`
Expected: FAIL — `cannot find function bands`.

- [ ] **Step 3: Implement the split** at the top of the same file:

```rust
//! Building a designed block: a blueprint, an anchor, and one band per bot.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, Step};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::{ExpansionCtx, Method};
use crate::state::PlanState;
use factorio_bot_core::blueprint::{Blueprint, BlueprintEntity, decode};
use factorio_bot_core::types::{FactorioEntity, Position};

/// How long one placement is modelled to take. Same figure `connect.rs` uses.
const PLACE_TICKS: u32 = 30;

/// Split a block into one band per bot, **balanced by entity count**.
///
/// Sorted by x, then chunked so each band holds as near an equal number of
/// entities as divides. Balancing by width instead would hand one bot a dense
/// corner and another an empty margin.
///
/// Deterministic: the sort is by `total_cmp` on x with the entity's index as
/// the tie-break, so equal-x entities always fall the same way.
pub fn bands(entities: &[BlueprintEntity], bots: usize) -> Vec<Vec<usize>> {
    if bots == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..entities.len()).collect();
    order.sort_by(|a, b| {
        entities[*a]
            .offset
            .x()
            .total_cmp(&entities[*b].offset.x())
            .then(a.cmp(b))
    });
    let mut out = vec![Vec::new(); bots];
    let per = entities.len().div_ceil(bots).max(1);
    for (slot, idx) in order.into_iter().enumerate() {
        out[(slot / per).min(bots - 1)].push(idx);
    }
    out
}
```

- [ ] **Step 4: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner blueprint`
Expected: 3 passed.

- [ ] **Step 5: Add the goal variant.** In `crates/planner/src/goal.rs`, add to `enum Goal`:

```rust
    /// This blueprint stands at this anchor.
    ///
    /// **Shaped to survive replanning.** Expanding it means *the entities not
    /// yet standing*, re-derived against the world every time, so it is
    /// verifiable rather than a remembered instruction, and building it twice
    /// is a no-op. Every other goal here is item-shaped for the same reason:
    /// this planner replans constantly, and a goal naming particular machines
    /// would be stale the moment a replan sited a different one.
    Built {
        /// The blueprint string, decoded on each expansion.
        blueprint: String,
        /// Where the blueprint's own origin lands in the world.
        anchor: Position,
    },
```

Fix every `match` the compiler now reports as non-exhaustive by giving `Built` the same treatment the arm's neighbours give a goal that owns no item: return `None` for holder/credit questions, and its own name where a label is wanted.

- [ ] **Step 6: Implement the method** in `blueprint.rs`:

```rust
/// Build a designed block by hand, one band per bot.
pub struct BuildBlock;

impl Method for BuildBlock {
    fn name(&self) -> &'static str {
        "BuildBlock"
    }

    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Built { .. })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Built { blueprint, anchor } = goal else {
            return Ok(Vec::new());
        };
        let bp: Blueprint = decode(blueprint)
            .map_err(|e| PlannerError::Refused(format!("blueprint refused: {e:?}")))?;

        // Only what is NOT already standing. This is what makes the goal
        // re-checkable on a replan and idempotent when built twice.
        let mut wanted: Vec<&BlueprintEntity> = Vec::new();
        for e in &bp.entities {
            let world = Position::new(anchor.x() + e.offset.x(), anchor.y() + e.offset.y());
            if !ctx.state.entity_stands(&e.name, &world) {
                wanted.push(e);
            }
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        let owned: Vec<BlueprintEntity> = wanted.iter().map(|e| (*e).clone()).collect();
        let roster = ctx.state.roster();
        let split = bands(&owned, roster.len().max(1));

        let mut steps = Vec::new();
        for (band, indices) in split.iter().enumerate() {
            for idx in indices {
                let e = &owned[*idx];
                let world = Position::new(anchor.x() + e.offset.x(), anchor.y() + e.offset.y());
                steps.push(place_step(ctx, e, world, band)?);
            }
        }
        Ok(steps)
    }
}
```

`entity_stands(name, pos)` and `roster()` may not exist under those names on `PlanState`. **Find the equivalents and use them** — `PlanState` already answers both questions for the cell methods; do not add new accessors if one is there.

- [ ] **Step 7: Implement the placement**, copying the shape from `crates/planner/src/method/connect.rs`'s `place_step` — the same `AtPosition`/`AreaFree`/`HasItem` preconditions, `LoseItem`/`CreateEntity` effects, `ctx.ids.next()`, `PLACE_TICKS`, and the `ctx.state.create_entity(entity)` overlay call so the next tile's `AreaFree` sees what this one took. Read that function and follow it; do not invent a different shape. Label each step `format!("place {} at {} -- block band {band}", e.name, world)`.

- [ ] **Step 8: Register.** Add `pub mod blueprint;` to `crates/planner/src/method/mod.rs`, and **one line only** to `default_registry()` in `crates/planner/src/method/have.rs`:

```rust
        .with(Box::new(crate::method::blueprint::BuildBlock))
```

Another session owns `have.rs`. One line, nothing else.

- [ ] **Step 9: Run the planner suite.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner`
Expected: all pass.

- [ ] **Step 10: Format and commit** the five files with explicit paths.

---

### Task 3: The Lua goal, and an offline plan

**Files:**
- Modify: `crates/scripting_lua/src/globals/goal/value.rs`
- Create: `scripts/block_smoke.lua`

**Interfaces:**
- Consumes: `Goal::Built` from Task 2.
- Produces: `goal.built(blueprint_string, {x=, y=})` in Lua.

- [ ] **Step 1: Add the constructor**, following the existing `have`/`researched`/`producing` entries in `value.rs` exactly: a `"built"` key, a table with `kind = "built"`, the blueprint string, and the anchor as `x`/`y`. Mirror whatever those do for validation and error messages.

- [ ] **Step 2: Write the smoke script.** Create `scripts/block_smoke.lua`:

```lua
-- Plans a designed block against the live world. Plans only; places nothing.
print("start block smoke")

local MINER_LINE = "<paste the MinerLine string from scripts/rcontest.lua>"

local p = goal.plan(goal.built(MINER_LINE, {x = 10, y = 10}))
print("makespan=" .. tostring(p.makespan) .. " steps=" .. #p.steps
      .. " bots=" .. #p.bots)
assert(p.makespan > 0, "a 37-entity block must take positive time")

local placed = p:count { kind = "place" }
print("placements: " .. tostring(placed))
assert(placed == 37, "every entity in the block is a placement, got " .. tostring(placed))

local per_bot = {}
for _, st in ipairs(p.steps) do
  per_bot[st.bot] = (per_bot[st.bot] or 0) + 1
end
for bot, n in pairs(per_bot) do print("bot " .. bot .. ": " .. n .. " step(s)") end
print("end block smoke")
```

- [ ] **Step 3: Copy the script to the workspace** — a bare name resolves against `<workspace>/scripts`, never the repo, with no fallback.

- [ ] **Step 4: Plan it offline** against the seed-31337 dump:

```bash
nix develop -c ./target/debug/factorio-bot plan \
  --world workspace/scripts/map.json --goal built --bots 1,2,3,4 --steps
```

If the `plan` CLI cannot express a blueprint goal on its argument line, say so in your report and verify through the smoke script on a headless run instead — **do not** add a CLI surface for it in this task.

- [ ] **Step 5: Commit.**

---

### Task 4: Build the miner line, live

**Files:**
- Create: `docs/superpowers/notes/2026-09-05-first-block-built.md`

- [ ] **Step 1: Check the box is free.** Another session runs measured Factorio experiments on the default ports and asks to be told before anyone builds or runs. Confirm before starting; use ports 34200/4324 and your own settings file and workspace.

- [ ] **Step 2: Run it headless at 5x** with four character bots, on a fresh seed-31337 map, executing `block_smoke.lua` extended to *run* the goal rather than only plan it (`goal.run` as the other scripts do — read `scripts/factory_stage2.lua` for the shape).

- [ ] **Step 3: Read the world back — this is the deliverable.** A placement count proves nothing. For every one of the 37 entities, assert it stands at its expected world position with its expected direction:

```bash
grep -o '"name":"electric-mining-drill"' <run dir>/map.jsonl | wc -l   # expect 13
grep -o '"name":"transport-belt"' <run dir>/map.jsonl | wc -l          # expect 21
grep -o '"name":"small-electric-pole"' <run dir>/map.jsonl | wc -l     # expect 3
```

Then check **directions**, because that is the failure this whole plan is shaped around: pull the placed entities out of `map.jsonl` and confirm their directions match the migrated blueprint. An entity at the right place facing the wrong way is the exact defect Task 1's migration exists to prevent, and a count will not see it.

- [ ] **Step 4: Write the note**, including per-bot step counts and wall-clock, and **anything that did not work**. If the bands left one bot idle while another finished late, say so with the numbers.

- [ ] **Step 5: Commit the note.**

---

### Task 5: Carry an underground belt's half through the placement path

**Files:**
- Modify: `crates/core/src/types.rs`, `crates/core/src/factorio/rcon.rs`, `mods/BotBridge/control.lua`, and the OpenAPI snapshot plus its TypeScript mirror.

**Interfaces:**
- Produces: `FactorioEntity` carries the underground half; `rcon_place_entity` accepts it; `FactorioEntity::new_underground_belt` regains a meaningful half parameter.

- [ ] **Step 1: Understand the seam before touching it.** `FactorioEntity` is published: `app/src/api/openapi.snapshot.json` is generated from the utoipa spec and `app/src/api/openapi.contract.spec.ts` ties every schema to a TypeScript declaration. Adding a field fails the Rust snapshot test until regenerated, then fails the TypeScript contract test until mirrored. That is the seam working; follow it rather than working around it.

- [ ] **Step 2: Add the field** to `FactorioEntity` as an `Option`, defaulted, so every archived record and dump still deserialises.

- [ ] **Step 3: Carry it over RCON.** `rcon_place_entity(player_id, item_name, entity_position, direction)` in `mods/BotBridge/control.lua` gains a fifth argument for the half, passed to `surface.create_entity` as `type`. Send it only for `underground-belt`; the game rejects `type` on entities that have none.

- [ ] **Step 4: Regenerate the snapshot and mirror it.**

```bash
UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
cd app && pnpm run test:coverage
```

- [ ] **Step 5: Restore the constructor's parameter.** `FactorioEntity::new_underground_belt` lost its `output` argument when it could not be carried; give it back now that it can, and assert in a test that the two halves differ.

- [ ] **Step 6: Run the workspace suite and clippy, format, and commit.**

---

### Task 6: Build the furnace line, live

- [ ] **Step 1: Confirm the box is free**, as in Task 4.

- [ ] **Step 2: Build `FurnaceLine`** — 179 entities: 87 transport-belt, 48 inserter, 24 stone-furnace, 13 small-electric-pole, 3 small-lamp, 2 splitter, 2 underground-belt — headless at 5x with four bots on a fresh seed-31337 map.

- [ ] **Step 3: Verify it stands AND works.** Every entity present, at position, facing correctly. Then the harder question: does it smelt? Insert ore and fuel at the input and read the output chest across consecutive samples in `samples.jsonl`. **This is the first moment anything in this line of work is proven to function rather than merely exist**, and if it does not, that is the result — say so plainly with what you saw.

- [ ] **Step 4: Record the numbers** — wall-clock, per-bot steps, idle tail — in the Task 4 note, and update CLAUDE.md with one subsection on `goal.built`: what it does, that it refuses tiles, recipes and module requests by name, and that bands are balanced by entity count.

- [ ] **Step 5: Commit.**

---

## Self-Review

**Spec coverage.** Decoding with direction migration (Task 1), the goal shaped to survive replanning (Task 2), placement in the ordinary action shape (Task 2), bands balanced by entity count (Task 2), the Lua surface and offline planning (Task 3), evidence by reading the world back (Tasks 4 and 6), the underground half promoted to a prerequisite (Task 5), and refusals by name for tiles, recipes and module requests (Task 1). The spec's out-of-scope list — siting, material supply, belts and joins — appears in no task, which is correct.

**Known soft spots, stated rather than hidden.** Task 2 names `entity_stands` and `roster` on `PlanState` without having confirmed those exact names, and tells the implementer to find the real ones rather than add accessors. Task 3 may find the `plan` CLI cannot express a blueprint goal, and says to report that rather than build a CLI surface. Task 1's furnace-line test may hit the `Unsupported` refusal, and says to assert the refusal and report rather than weaken the decoder.

**Type consistency.** `Blueprint`, `BlueprintEntity`, `UndergroundHalf` and `decode` keep one shape across Tasks 1, 2 and 5. `bands` returns `Vec<Vec<usize>>` in both its test and its use. `Goal::Built { blueprint, anchor }` is spelled identically in Tasks 2 and 3.
