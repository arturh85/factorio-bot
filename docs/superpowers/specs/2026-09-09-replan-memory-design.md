# Replan Memory: carrying intent across plan boundaries

> **Design for:** `crates/planner` replan memory, bridging the gap between what
> a plan intended and what the next expansion sees as map facts.

**Status:** design. Nothing below is built. Every claim about current behaviour
is read off the source or measured from archived runs.

**Owner decision needed:** Everything in §5.

## 1. The problem

A plan is made, truncated mid-batch, and remade. The second expansion meets the
first plan's work as ordinary map facts: `added` entities, `consumed` resources,
`researched` technologies. What it does *not* meet is any record of *why* those
things were done — which goal an entity was placed for, whether a half-built
belt was intended to reach a particular chest, whether an anchor was chosen by
search or by instruction.

This is documented as the project's standing defect (`CLAUDE.md`):
> *Nothing tracks provenance. A placed belt is a belt; a chest is a chest.*

### Three failure modes, each measured

**FM1: Cross-block anchor ambiguity.** `recover_anchor` trusts an anchor once
*two* of a blueprint's entities stand at the right relative offsets. Blocks
sharing a sub-layout therefore recover into each other — `ElectricSmelter`
finds 21 of its 28 entities inside a standing `FurnaceLine`. Raising the
threshold does not fix it: 21 of 28 is 75%, and any threshold loose enough to
resume a genuinely half-built block accepts it. The fix landed (`Site::Anchored`,
owner ruling 2026-09-07) but **only a caller that was given the anchor can
record it**. A block sited by search, then replanned, must re-derive the anchor
from geometry — and geometry is what is ambiguous.

**FM2: Half-built chains.** A replan sees 25 belts reaching from `[30.5,-46.5]`
to `[31.5,-31.5]` with no arm at either end. The belts are obstacles; the run
they were built for is lost. `standing_run` (`assemble.rs`) matches a chain
that reaches the sink's door, but a chain that stops short is an obstacle.
Nothing says "these 25 belts were the supply link for the cell at anchor X" —
so the replan routes around them or lays a second link.

**FM3: Double siting.** A cut batch leaves a cell's machines unfinished.
`complete_cell` re-detects the part, finds that no machine stands on its
layout tile, and siting sees clear ground. The same cell is sited a second
time, drawing from a chest whose remaining free sides the first link already
spent. Run `1788941729-70024` measured: 48 steps abandoned, both
`assembling-machine-1`s among them.

### What already works

The `standing` module (`crates/planner/src/standing.rs`) is the correct
foundation: `world_after()` applies a plan's physical effects to a clone of the
world and returns a new surface, so `PlanState::from_world` over it is what the
supervisor's next round would build. And `complete_cell` in `assemble.rs`
recovers a cell from any two standing parts — chest, inserter, or machine.

What neither does is **connect the standing entity to the goal that placed it**.
The geometry match is functional and requires no memory. It is also the thing
that fails in all three modes above.

## 2. Core design: a lightweight intent sidecar

The replan memory is a **separate, serializable structure** that travels beside
the plan. It is not part of `PlanState` — that type is rebuilt from the world
on every expansion and must stay a pure function of the world — but it is
consumed by the expansion after the world overlay is built.

```rust
/// What the LAST plan intended for the entities it placed, carried across
/// replan boundaries as a recoverable claim rather than as ground truth.
///
/// Not serialized as part of a `PlanState`; read from the run record or
/// passed explicitly to `plan_best` / `plan_round`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanMemory {
    /// The plan round this memory was captured at.
    pub plan_round: u32,
    /// Every entity placed by the last plan, keyed by position, annotated
    /// with the goal or sub-goal that placed it.
    pub entities: BTreeMap<Pos, EntityIntent>,
    /// Every cell assembled by the last plan, by anchor position.
    pub cells: Vec<CellIntent>,
    /// Every belt chain built by the last plan, by its terminal positions.
    pub chains: Vec<ChainIntent>,
    /// Every block placed by the last plan, by Site.
    pub blocks: Vec<BlockIntent>,
    /// The overrides `complete_cell` made: which entities were accepted as
    /// standing even though the layout would not have claimed them.
    pub recovery_overrides: BTreeSet<Pos>,
}
```

### 2a. EntityIntent

```rust
/// What a single entity was placed FOR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityIntent {
    /// The goal kind that caused this placement.
    pub goal: IntentGoalKind,
    /// The goal's parameters, enough to map it to the caller's own goal list.
    pub goal_params: String, // e.g. "sustain:iron-plate:30:36000"
    /// Which plan round placed it.
    pub plan_round: u32,
    /// The action id that placed it in that plan, for cross-referencing the
    /// run record.
    pub action_id: Option<u32>,
    /// The anchor position of the cell this entity belongs to, if any.
    pub cell_anchor: Option<Position>,
    /// The goal index in this plan's top-level goal list, for re-mapping on
    /// replan.
    pub goal_index: Option<usize>,
}
```

`IntentGoalKind` is a compact enum of the goal families that place entities:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntentGoalKind {
    /// `Goal::Built` — a blueprint block.
    Built(String), // blueprint identifier
    /// `Goal::Producing`/`Goal::Sustain` — a cell.
    Cell {
        item: String,
        per_minute: u32,
    },
    /// `Goal::Produced` — a machine and its infrastructure.
    Produce { item: String, count: u32 },
    /// `Goal::Have` — infrastructure for hand-smelting or gathering.
    Have { item: String },
    /// `Goal::Extracted`/`Goal::Gathered` — pumpjack and tank.
    Extract,
    /// Power plant.
    Power,
    /// `Goal::Charted` — a radar or scouting walk.
    Chart,
}
```

### 2b. CellIntent

```rust
/// A cell the last plan assembled, by anchor and facing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellIntent {
    /// The anchor position of the intermediate machine.
    pub anchor: Position,
    /// Facing.
    pub facing: Direction,
    /// The spec's item.
    pub item: String,
    /// Which of the cell's parts the last plan placed. Empty means it
    /// was fully placed; a partial list means the batch was cut.
    pub placed: Vec<String>, // role names
    /// The supply chain's sink — the chest or lab the belts run to.
    pub supply_source: Option<Position>,
    /// The belted input positions, for chain recovery.
    pub mouths: Vec<MouthIntent>,
    /// The plan round that assembled this cell.
    pub plan_round: u32,
}
```

### 2c. BlockIntent

```rust
/// A `Site::Built` block the last plan placed, by anchor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockIntent {
    /// The resolved anchor.
    pub anchor: Position,
    /// The site variant — preserved so the next round knows whether to
    /// search again or honour the anchor.
    pub site: Site,
    /// The blueprint string, for re-verification.
    pub blueprint: String,
    /// How many entities the plan actually placed of the blueprint's total.
    pub placed: u32,
    /// How many entities the blueprint has.
    pub total: u32,
}
```

### 2d. ChainIntent

```rust
/// A belt run the last plan built, from source to sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainIntent {
    /// The entity this chain starts from.
    pub source: Position,
    /// What it carries.
    pub item: String,
    /// The entity it delivers to.
    pub sink: Position,
    /// Whether both arms were placed.
    pub arms_placed: bool,
    /// How many belts were placed.
    pub belts_placed: u32,
    /// How many belts the run needs.
    pub belts_needed: u32,
    /// Positions of every belt tile, for recognition.
    pub belt_positions: Vec<Pos>,
}
```

## 3. Where memory lives and how it flows

### 3a. Capture at plan finish

When `plan_best` returns a plan, the recorder captures an `IntentSnapshot`:

```rust
pub struct IntentSnapshot {
    pub plan_round: u32,
    pub memory: ReplanMemory,
}
```

This is written to the run record alongside `provenance.json` and
`plan_created` — a new JSONL file `intent.jsonl`, one entry per plan round.
The same data is returned from `plan_best` so the supervisor can hand it to the
next round without file I/O.

### 3b. Consumption at replan start

When a replan begins, the supervisor passes the *previous* round's
`ReplanMemory` to `plan_best` (or to a new `plan_round` entry point). The
planner uses it *after* `PlanState::from_world`, during expansion, as an
advisory overlay:

```
   world -> PlanState::from_world()  ->  [replan memory]  ->  expansion
                                              |
                                       EntityIntent map
                                       CellIntent list
                                       BlockIntent list
```

The memory is **not** merged into `PlanState`. It is consulted by:

1. `complete_cell` — the cell intent gives a primary anchor to try *before*
   the geometry scan, so the recovery is guided by intent rather than by
   the first matching sub-layout.
2. `resolve_site` — the block intent supplies the `Site::Anchored` record
   that geometry recovery cannot, so the second expansion uses the same
   anchor as the first.
3. `connect` — the chain intent tells the router that the 25 belts at
   `[30.5,-46.5]..[31.5,-31.5]` were the supply link from chest X to cell Y,
   so it can add the two missing arms rather than route a second link.

### 3c. The memory is a claim, not a command

The replan memory is always **advisory**. Every claim in it is verified against
the world before being acted on:

- A `CellIntent` whose anchor tile now holds a different entity is ignored.
- A `ChainIntent` whose belt positions are gone (mined out, deconstructed) is
  ignored.
- A `BlockIntent` whose blueprint string has changed (the caller sent a new
  one) is ignored.

This rule is what keeps the planner deterministic: the memory is an input that
can only *narrow* the search space, never override what the world says.

### 3d. Survival across sessions

The memory lives in two forms:

1. **In-process**: the supervisor holds the last round's `ReplanMemory` and
   passes it to the next `plan_round`.
2. **Archived**: `intent.jsonl` in the run directory, one entry per plan round.
   Used by `plan --standing-from-run <run> --at-tick <T>` to reconstruct the
   intent at any tick.

## 4. How each failure mode resolves

### FM1: Cross-block anchor ambiguity

**Before:** `resolve_site` opens with `recover_anchor`, which matches any two
of the blueprint's entities against the world. `ElectricSmelter` matches 21
entities inside a standing `FurnaceLine` and claims its anchor.

**After:** `resolve_site` first consults the memory's `blocks` list for a
block whose blueprint matches. The memory says "I placed FurnaceLine at
anchor X". A second call for `ElectricSmelter` finds no block intent matching
its blueprint — so recovery falls through to the geometry scan, which still
fires. But the *first* call (the one that correctly identifies FurnaceLine)
uses the memory anchor directly, bypassing the geometry ambiguity.

The key change: **recovery consults intent before geometry**. The
`ReplanMemory` carries the `Site::Anchored` designation that the plan
itself knew, so recovery does not have to reconstruct what was once recorded.

### FM2: Half-built chain completion

**Before:** 25 belts standing, no arm at either end. The replan sees obstacles
and routes around them or lays a second link.

**After:** `assemble::standing_run` checks the memory's `chains` list for a
chain whose `belt_positions` match the standing belts and whose total indicates
the run was incomplete. Finding `sink = cell_supply_chest` and `arms_placed =
false`, it adds two actions — load arm and unload arm — and marks the chain as
complete. The belts become infrastructure rather than obstacles.

Implementation detail: the chain intent is consumed by `connect_steps_reserving`
during the cell's own expansion. The cell method asks "do my supply belts
stand?" and the intent answers "yes, and they go to chest at P, and they need
two arms".

### FM3: Double siting

**Before:** `complete_cell` scans for any standing part. The machines are
missing, so no cell is found. Siting places a new cell.

**After:** `complete_cell` first checks the memory's `cells` list for a cell
whose anchor matches the one siting would propose *and* whose `placed` list
is incomplete. It finds `anchor = [44.5,-32.5]`, `placed = [SupplyChest,
OutputChest, three inserters]` — exactly the half-built cell — and finishes
it rather than siting new ground.

This is where the two-part consistency rule matters: the memory's cell anchor
is verified against the *world* (standing chest at that offset), and only if
at least one of the already-placed parts still stands is the intent trusted.
A memory from a run that was entirely reverted (all entities removed) is
harmlessly ignored.

## 5. Open design decisions (owner needed)

### D1: How does memory cross the Lua/Rust boundary?

The memory is captured in Rust (`plan_best` return) and held by the Lua
supervisor (`obs:plan(...)` return). Three options:

- **Option A (recommended).** `plan_best` returns `(ActionNetwork, Schedule,
  ReplanMemory)`. The Lua binding returns it as a table that the supervisor
  passes to the next `obs:plan(goals, {memory = ...})`. The memory is opaque
  to Lua — it is never inspected or modified in script.

- **Option B.** The memory is written to and read from `intent.jsonl` only.
  The supervisor never touches it directly. This is simpler at the binding
  level but makes in-process replans more roundabout (write, replan, read).

- **Option C.** The memory is folded into `PlanState` as an `Option` field
  that is serialized and deserialized with it. This conflicts with the
  principle that `PlanState` is a pure function of the world.

### D2: What is the memory's lifetime?

- **Intra-run:** Each plan round replaces the previous round's memory.
  Abandoned between rounds: only the last completed plan's memory survives.

- **Inter-run:** Not preserved. A fresh run has no memory of previous runs.
  `intent.jsonl` is for offline diagnostics and `--standing-from-run`.

- **Across a kill cycle:** If the supervisor kills the whole run (e.g. after
  `MAX_REPLANS`), the memory is discarded with it.

### D3: What about the batch that never settled?

A plan whose actions were all dispatched but zero have settled has no standing
entities — the mod writes `on_some_entity_created` only after the game confirms
placement. In this case the memory records the intended placements (planned
positions), but no entity stands to verify them. The memory is still useful:
the next expansion knows *where* it intended to build and can skip siting for
those goals.

**Bound:** The memory is trusted only when at least one entity stands at the
recorded position. A batch that was entirely reverted (e.g. the game refused
every placement) produces a memory with `placed = 0` for that goal, and is
ignored.

### D4: How does memory interact with `standing::world_after`?

`world_after` applies a plan's physical effects to a clone surface. The memory
should be **applied alongside it**: `world_after` returns the world, and the
caller pairs it with the previous round's memory to seed the next round.

```rust
pub fn world_after_with_memory(
    state: &PlanState,
    net: &ActionNetwork,
    done: impl Fn(ActionId) -> bool,
    previous_memory: Option<&ReplanMemory>,
) -> Result<(Arc<FactorioSurface>, Standing, Option<ReplanMemory>), PlannerError>
```

The memory returned is the *new* one (from `net`'s actions), and the
`previous_memory` is consulted for partial recovery.

### D5: Offline determinism

The memory is an *optional* input to `plan_best`. A caller that provides none
gets the current behaviour (geometry-only recovery). A caller that provides
one gets the intent-guided recovery. This is the determinism guarantee:

> **A plan built with memory M against world W is identical to a plan built
> without memory against world W, *unless* the memory guided a decision that
> geometry alone could not.**

Where geometry alone could: siting, cell recovery, chain routing — all
deterministic functions of the world. Where memory adds something: anchor
disambiguation (FM1), chain completion (FM2), cell continuation (FM3).
In all three cases the memory *narrows* a choice that geometry left open:
two blocks matched one anchor → memory says which; 25 belt tiles with no
arm → memory says where the arms go; no machine on the layout tile → memory
says which anchor the cell was at.

The `--replan` and `--standing-from-run` CLI flags accept an optional
`--intent <intent.jsonl>` for offline reproduction.

## 6. Implementation plan

### Phase 1: Data types and capture (small, ~1 day)

1. Define `ReplanMemory`, `EntityIntent`, `CellIntent`, `BlockIntent`,
   `ChainIntent` in a new file `crates/planner/src/memory.rs`.
2. Add `capture_intent(state, net, schedule) -> ReplanMemory` that walks the
   network and state to populate the intent records.
3. Return `ReplanMemory` from `plan_best`.
4. Write `intent.jsonl` alongside `plan_created` in the run record.

**Tests:** capture produces the right entity count; capture + empty plan =
empty memory; cross-block blueprints produce distinct block intents.

### Phase 2: Consumption in assemble (small, ~1 day)

1. Thread `Option<&ReplanMemory>` through `complete_cell` and `plan_cell`.
2. `complete_cell` checks the memory for a matching `CellIntent` before
   scanning geometry.
3. `plan_cell` checks the memory for a matching `BlockIntent` before siting.

**Tests:** the 70024 replan test (`replan_finishes_its_cell.rs`) passes with
memory when the cell's machines are missing; cross-block ambiguity resolves
to the correct anchor; a half-built cell found via memory is finished.

### Phase 3: Chain memory in connect (medium, ~1.5 days)

1. Add optional memory consultation to `connect_steps_reserving`.
2. When standing belts match a `ChainIntent` with `arms_placed = false`,
   emit only the two arm placements rather than routing the full run.
3. `standing_run` in `assemble.rs` uses chain memory to match a chain that
   stops short of the sink's door.

**Tests:** half-built chain (belts stand, arms missing) is completed;
chain whose belts intersect new obstacles refuses rather than overwriting.

### Phase 4: Supervisor integration (medium, ~1 day)

1. Thread memory through the Lua supervisor loop.
2. The supervisor passes `obs:plan(goals, {memory = last_memory})`.
3. On first round (no memory), the planner works as before.
4. On replan, the memory from the previous round is passed in.

**Tests:** supervisor replan with memory reproduces the known live fixes;
supervisor replan without memory matches the current behaviour.

### Phase 5: Offline replay (small, ~0.5 day)

1. `plan --standing-from-run <run> --at-tick <T>` reads `intent.jsonl`.
2. `plan --replan 1 --fail <label>` accepts `--intent <file>`.
3. The offline baselines match the live supervisor output for the same inputs.

## 7. Relationship to existing code

| Component | Interaction |
|---|---|
| `PlanState` | Unchanged. Memory is not a field of `PlanState`. |
| `standing::world_after` | Returns memory alongside the new surface. |
| `assemble::complete_cell` | Consults memory *before* geometry scan. |
| `assemble::plan_cell` | Unchanged — still siting from power. Memory only feeds `complete_cell` from the cell intent. |
| `connect::connect_steps` | Consults memory for chain completion. |
| `method::blueprint` / `BuildBlock` | Consults memory for anchor recovery. |
| `schedule` | Unchanged. The schedule is a function of the network only. |
| `goal::Goal::Built` and `Site` | Unchanged. `Site::Anchored` is still the instruction; memory feeds the *automatic* version. |
| `recover.rs` | Unchanged. Recovery (action/schedule repair) and replan (new plan) are separate. |
| `scripts/supervisor.lua` | Gains a `memory` parameter to `plan_round`. |

## 8. What this does NOT solve (explicit)

- **Walk failures across replans.** Walk memory is the executor's concern
  (`walk_memory`), not the planner's. A walk that fails mid-batch is still
  handled by `recover.rs`.
- **Inventory double-count.** Buffers and `Withdraw` already have their own
  ledger in `PlanState::buffers`. Memory is about entities, not items.
- **Research continuity.** `researched` is functionally re-derived from the
  world on each replan. If the research was set mid-batch, the mod's writeout
  records it, and the next plan sees it as a standing fact.
- **Multiple surfaces.** Memory is keyed by `Pos`, which is surface-local. A
  cross-surface memory would need `(SurfaceId, Pos)` keys. Not designed here.
- **Threat and safety.** Memory has nothing to say about enemy structures or
  pollution. Those remain invisible to the planner.
