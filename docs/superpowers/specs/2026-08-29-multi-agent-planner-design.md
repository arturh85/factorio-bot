# A layered multi-agent planner

**Date:** 2026-08-29
**Status:** Approved design, not yet implemented

## Why

The project's stated vision — `goal("launch_rocket")` decomposed and executed by
8-16 cooperating bots — has no data model behind it. `Planner` is a husk and
`execute_plan()` (`crates/core/src/plan/planner.rs:71-110`) is an infinite loop
of commented-out code. Every attempt to build goal decomposition on top of the
existing `TaskGraph` fights the structure rather than extending it.

`TaskNode` (`crates/core/src/graph/task_graph.rs:410-417`) holds four concerns
at once: what to do (`data`), who does it (`player_id`), how long it takes
(inside `status`, as `TaskStatus::Planned(f64)`), and runtime state (`status`).
The consequences are concrete:

- **Assignment is welded into the topology.** `add_to_group(player_id, …)`
  (`task_graph.rs:58-68`) chains each bot's nodes off that bot's own cursor. The
  bot is not an attribute of a task; it is the graph's shape. Load balancing,
  reassignment, and re-planning on failure are not unimplemented — they are
  unrepresentable.
- **Walking forces assignment.** `PlanBuilder::mine` inserts a walk node at
  authoring time (`plan_builder.rs:33-38`), which requires knowing which bot
  walks. This is the mechanism that welds assignment into authoring.
- **There is no state, only a transcript.** `PlanBuilder` advances `plan_world`
  as it appends. The plan is a recording of one timeline; it cannot be forked,
  compared, or backtracked.
- **Forking is economically impossible.** `FactorioWorld::clone()`
  (`world.rs:503-523`) deep-clones the entity graph, its quadtrees, and all
  prototypes. Exactly one fork exists in the system, created once per script run
  (`lua_runner.rs:38`).
- **Dependency inference is wrong.** `resolve_dependencies()`
  (`task_graph.rs:165-213`) is O(n²) and matches producers to consumers on item
  *name* alone, ignoring order, quantity, and player. It will link a consumer to
  a producer that runs after it.
- **Validation lies about its order.** `validate_resource_flow()`
  (`task_graph.rs:215`) documents topological traversal but iterates
  `node_indices()`, which is insertion order. True only because the cursor
  happens to build it that way.
- **Nothing owns anything.** Two bots mining the same tile — listed as an open
  problem in PLAN.md 2.3 — is not preventable in this model.

Planning is `state → action → state'`. No type in this codebase represents "the
world at a point in a hypothetical plan," which is why the data structures felt
impossible to design.

## Decisions

| Decision | Choice |
|---|---|
| Planner generality | Domain-specific. Hand-written methods; no search engine. |
| Preconditions/effects | Data, not code, so validation and scheduling stay generic. |
| Existing `TaskGraph` and Lua `plan.*` | Replaced. `Schedule` becomes what rendering consumes. |
| State forking | Overlay over an immutable `Arc<FactorioWorld>` base. |
| Bot assignment | Layer 4 only. Absent from layers 1-3. |
| `Walk` | Not an authorable action. Emitted by the scheduler. |
| Time units | Game ticks (`u32`) internally; seconds only for rendering. |
| First slice | `Have(automation-science-pack, N)` with 4 bots. |
| Crate | New `crates/planner`, depending on `core`. |

## Architecture

Five layers, each a separate type, each independently testable. Layers 0-4 are
pure — no RCON, no process spawning, no Factorio. `Ticks` throughout is
`type Ticks = u32`, a count of game ticks.

```
Goal  ──expand──▶  Action network  ──schedule──▶  Schedule  ──execute──▶  ExecutionLog
 (1)      (2)           (3)              (4)         (4)          (5)
                    ▲                 ▲
                    └── PlanState (0) ┘
```

### Layer 0 — `PlanState`

```rust
pub struct PlanState {
    base: Arc<FactorioWorld>,          // shared, never mutated during planning
    bots: BTreeMap<BotId, BotState>,   // position, main inventory, craft queue
    entities: EntityOverlay,           // added / removed, relative to base
    research: ResearchOverlay,
    reservations: Reservations,
}
```

Forking clones the overlay only. Planning touches a vanishing fraction of a
world, so a fork is cheap enough to do per candidate assignment. If overlays
ever grow large, the `BTreeMap`s swap for a persistent map behind the same
interface — a contained change, not a redesign.

`Reservations` earmarks a resource to a specific `ActionId`:

- **Resource tiles** — position plus count, so two bots cannot mine the same ore.
- **Entity positions** — so two bots cannot place at the same tile.
- **Item counts** — within a bot's inventory or a chest, so two actions cannot
  spend the same stack.

A reservation conflict is a precondition failure at plan time, not a runtime
race.

### Layer 1 — `Goal`

Declarative and bot-agnostic.

```rust
pub enum Goal {
    Have { item: ItemId, count: u32, whose: Holder },
    Researched(TechId),
    Built { blueprint: BlueprintRef, at: Placement },
    Producing { item: ItemId, rate: PerMinute },
    All(Vec<Goal>),
}

pub enum Holder {
    Anyone,            // satisfied by the sum across all bots' inventories
    Bot(BotId),        // one named bot's inventory
    Chest(Position),   // a specific container
}
```

`Holder::Anyone` is what makes multi-bot gathering parallel: `Have(science, 10,
Anyone)` decomposes into four independent per-bot chains of roughly three each,
with no coordination. Physical consolidation into one inventory is a separate
concern, handled by an explicit method (below) and needed only when a single
action requires the full count in one place.

`Producing { item, rate }` is functorio's `BusLane item rate` type transcribed
into Rust. It is the entire bridge between the two systems; nothing further is
required for them to share a vocabulary. No method implements it in this spec.

### Layer 2 — `Method`

```rust
pub trait Method {
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool;
    fn expand(&self, goal: &Goal, state: &PlanState) -> Result<Vec<Step>>;
}

pub enum Step { Subgoal(Goal), Act(Action) }
```

A registry keyed by goal discriminant, ordered by preference. Expansion recurses
until only `Action`s remain, producing an unordered set. Methods are hand-written
and readable; there is no backtracking search.

Methods needed for the first slice:

| Goal | Methods, in preference order |
|---|---|
| `Have` | `AlreadySatisfied` → `TakeFromChest` → `Craft` → `Smelt` → `Mine` |
| `Have` with `Holder::Anyone` | `SplitAcrossBots`, dividing `count` into `min(bot_count, count)` shares as evenly as possible, each an independent per-bot subgoal |
| `Researched` | `Consolidate` science into a lab, then `Research` |

`Consolidate` is the method that emits transfers through a chest when a single
action needs `count` items in one inventory. It exists so that `Holder::Anyone`
can stay cheap in the common case.

**Expansion returns no ordering.** Ordering is inferred at layer 4 from
preconditions and effects. This is what leaves the scheduler free to parallelize.

### Layer 3 — `Action`

```rust
pub struct Action {
    id: ActionId,
    kind: ActionKind,          // Mine | Craft | Place | Insert | Remove | Research
    pre: Vec<Condition>,
    eff: Vec<Effect>,
    duration: Ticks,           // nominal estimate
    pinned: Option<BotId>,     // normally None; an escape hatch, not the default
}

pub enum Actor { Bound(BotId), Role }   // Role = "whichever bot runs this action"

pub enum Condition {
    HasItem { who: Actor, item: ItemId, count: u32 },
    AtPosition { who: Actor, pos: Position, radius: f64 },
    EntityAt { pos: Position, name: ItemId },
    Researched(TechId),
    ResourceAvailable { pos: Position, item: ItemId, count: u32 },
}

pub enum Effect {
    GainItem { who: Actor, item: ItemId, count: u32 },
    LoseItem { who: Actor, item: ItemId, count: u32 },
    MoveTo { who: Actor, pos: Position },
    CreateEntity(FactorioEntity),
    RemoveEntity { pos: Position },
    ConsumeResource { pos: Position, item: ItemId, count: u32 },
    Researched(TechId),
}
```

Two choices carry most of the design's weight.

**`Actor::Role` instead of a `BotId`.** It denotes "whichever bot runs this
action," bound at layer 4. This is the single change that lifts assignment out of
authoring.

**`Walk` is absent from `ActionKind`.** Walking is a consequence of an
`AtPosition` precondition. The scheduler emits it at assignment time, because
travel cost depends on which bot is chosen and where that bot last was. Removing
`Walk` from the vocabulary makes the decoupling structural rather than a
convention a future method can violate.

Because `Condition` and `Effect` are data, the applier, the validator, the
dependency inferencer and the scheduler are written once and never learn a
recipe name.

### Layer 4 — `Schedule`

```rust
pub fn schedule(net: &ActionNetwork, state: &PlanState, bots: &BotRoster)
    -> Result<Schedule>;

pub struct Schedule {
    steps: Vec<ScheduledStep>,   // { what: Act(ActionId) | Walk(Position), bot, start, end }
    makespan: Ticks,
    critical_path: Vec<ActionId>,
}
```

Phase one infers the ordering graph by matching `Effect`s to `Condition`s with
quantity and direction — what `resolve_dependencies()` gets wrong today.

**Ordering edges carry a minimum lag in ticks.** A lag expresses "this
dependency is satisfied, but not until *t* ticks after its producer finishes."
It is what models machine processes: the `Smelt` method emits `Place(furnace)`,
`Insert(ore)` and `Remove(plate)`, and the edge from the insert to the removal
carries the furnace's smelting time. A bot is free to do other work across a
lag — which is most of where multi-bot parallelism comes from.

Phase two is greedy travel-aware list scheduling. Maintain a per-bot fork of
`PlanState`. Repeatedly select the ready `(action, bot)` pair minimising
`max(bot_free, deps_done) + travel + duration`; bind `Actor::Role` to that bot,
emit the walk, claim reservations, apply effects to that bot's fork. An action
with `pinned: Some(bot)` is a constraint the selection honours rather than a
candidate it ranks.

Scheduling is a pure function of three values. This is what the layer is for:

- Scheduler variants can be benchmarked on byte-identical inputs. This is the
  only place speedrun optimisation needs to touch.
- Bot count is an argument. One, four and sixteen bots require no other change.
- It is property-testable. See Testing.

Version one is greedy. Critical-path priority and local search over assignments
are drop-in successors; the boundary is a value in and a value out.

**Deadlock hazard.** Greedy assignment plus reservations can deadlock if claims
are made in a bad order. It is prevented by claiming only at assignment time and
assigning strictly in dependency order, so a claimed resource is never required
by an already-scheduled predecessor. This gets an explicit property test rather
than trust.

**Version-one simplification.** A `Craft` action occupies its bot exclusively,
though Factorio permits crafting while walking. Overlapping the two is a
documented follow-up; `ScheduledStep` carries explicit `start` and `end` rather
than being implicitly sequential, so the representation already admits it.

### Layer 5 — `Executor`

```rust
pub struct ExecutionLog { attempts: BTreeMap<ActionId, Attempt> }

pub struct Attempt {
    status: Status,              // Pending | Running | Success | Failed
    started_tick: u32,
    ended_tick: Option<u32>,
    error: Option<String>,
}
```

Execution state is keyed by `ActionId` and never stored in the plan, so
`Schedule` stays an immutable value and estimated-versus-actual is a join. The
Gantt renderer is `(&Schedule, Option<&ExecutionLog>) -> String`, which makes the
comparison a TAS run needs available for free.

One task per bot walks its own assignment list. Cross-bot dependencies wait on a
per-action completion signal rather than the 100 ms poll loop at
`execute.rs:79`.

Failure is handled in tiers:

1. Re-schedule from observed real state. Cheap; no re-expansion.
2. If no valid schedule exists, re-expand from layer 2.
3. If that fails, surface to the caller.

Tier 3 is where an LLM plugs in later: rare, high-level, not time-critical.

## Crafting and machine latency

Factorio's player crafting queue is asynchronous — a `Craft` action's effect does
not land when the command is issued. Getting this wrong silently corrupts every
downstream estimate, so it is settled here rather than deferred.

- **Estimate**, used by the scheduler: `recipe.energy * count` converted to
  ticks, at player crafting speed.
- **Actual**, used by the executor: issue the craft, then poll the bot's
  inventory until the produced count materialises. Completion is observed, never
  assumed.

Smelting works the same way one level up: the estimate is the lag on the
insert-to-removal edge, taken from the furnace prototype; the actual is the
executor polling the furnace's output slot before issuing the removal.

In both cases the gap between estimate and actual is recorded in `Attempt`, and
is exactly the signal needed to calibrate the estimate later.

## Time units

Game ticks (`u32`) throughout layers 0-5. Seconds appear only in rendered output.
The current code mixes both — `TaskStatus::Success(f64, u32)` — and float
durations drift against a tick-based simulation.

## Crate layout

New crate `crates/planner`, depending on `core`:

| File | Responsibility |
|---|---|
| `state.rs` | `PlanState`, overlays, `Reservations` |
| `goal.rs` | `Goal`, `Holder` |
| `action.rs` | `Action`, `Condition`, `Effect`, the effect applier |
| `method/mod.rs` | Registry and expansion driver |
| `method/have.rs` | `Have` methods, including `SplitAcrossBots` and `Consolidate` |
| `method/research.rs` | `Researched` methods |
| `network.rs` | Dependency inference from effects to preconditions; validation |
| `schedule.rs` | The scheduler |
| `execute.rs` | Executor, `ExecutionLog` |
| `render.rs` | Gantt and graphviz from `Schedule` and `ExecutionLog` |

Deleted from `core`: `plan/plan_builder.rs`, `plan/planner.rs`, `plan/execute.rs`,
`graph/task_graph.rs`, `gantt_mermaid.rs`. Their rendering logic moves to
`planner/render.rs`. `core` is then exactly "the game, and what we know about
it" — a boundary it does not currently have.

Call sites to update: `crates/scripting_lua/src/globals/plan.rs`,
`crates/scripting_lua/src/lua_runner.rs`, `crates/scripting_lua/src/lua_docs.rs`.
The implementation plan must enumerate these precisely rather than assuming the
list is complete.

## Lua API

```lua
plan.goal(goal.have("automation-science-pack", 10))
plan.goal(goal.researched("automation"))

local schedule = plan.solve{ bots = 4 }
print(schedule.makespan)
print(schedule:gantt())

plan.execute(schedule)
```

No `player_id` anywhere. `plan.solve` runs the layer 2 through layer 4 pipeline.
Scripts become declarative; bot count and scheduler choice are parameters rather
than structure. The existing scripts under `scripts/` and the generated Lua API
docs are rewritten as part of this change.

`Action::pinned` is reachable from Lua as an escape hatch for hand-tuned TAS
work, but it is not the default path and no first-slice method sets it.

## Testing

Layers 0-4 are pure. The whole planner unit-tests in milliseconds against a
synthetic `PlanState`, against the 120-180 seconds a multi-client run costs.
Only layer 5 needs the game.

1. **Method units.** Each method expands correctly given a state, including the
   negative cases: `AlreadySatisfied` when the items are present, `TakeFromChest`
   only when a reachable chest holds them.
2. **Properties.**
   - Every precondition holds at its scheduled time.
   - No reservation is claimed twice.
   - Makespan is monotone non-increasing in bot count.
   - Scheduling terminates on any well-formed network — the deadlock guard.
3. **Golden.** `Have(automation-science-pack, 10)` at 1, 2 and 4 bots pins a
   makespan. Scheduler regressions surface as a number moving.
4. **Integration**, slow and opt-in. The real 4-bot run against a live server.

## Sequencing

Implementation starts after `feat/axum-server` merges. This work is additive, but
the Lua rewrite touches `scripting_lua` and a new workspace member touches
`Cargo.toml` — precisely the files that branch has uncommitted. Axum plans 2 and
3 do not collide with the planner and can proceed in parallel afterwards.

## Out of scope

Each of the following attaches to this model without changing it, which is what
the layering buys. None is in the first implementation plan.

- **functorio integration.** A method satisfying `Producing { item, rate }` by
  generating a blueprint, binding its unbound `input` declarations to real ore
  patches via `EntityGraph::resource_patches()`, and verifying the built result
  against `FlowGraph`. The integration mechanics — invoking `lake build` versus
  vendoring generated blueprint JSON — are undecided and need their own design.
- **Blueprint siting and construction scheduling** (`Built`).
- **Search and backtracking over methods.** Layer 3 already carries the data a
  search engine would need.
- **LLM goal authoring.** Enters at layer 1 and at failure tier 3.
- **Seed rolling.**
- **Space Age.** Whether BotBridge is exercised beyond base 2.0 is unverified.
