# Hierarchical Factory Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver bounded starter-factory planning with reusable modules and a reproducible comparison against the existing planner.

**Architecture:** Keep the current goal interface, action network, scheduler, and executor. Introduce request-scoped planning control, explicit module artifacts and instances, then integrate an experimental module mode through the existing production methods. An independent observer verifies production; a frozen experiment compares both modes with identical execution and observation rules.

**Tech Stack:** Rust 2024, existing Cargo workspace, serde JSON, Lua 5.4, BotBridge, Factorio Space Age, existing headless runner. No optimization solver, learning framework, service, or frontend dependency is introduced.

**Spec:** [Hierarchical factory planning design](../specs/2026-09-10-hierarchical-factory-planning-design.md), approved in conversation by the owner's “continue” after written review was requested.

## Global Constraints

- “The first implementation project is deliberately narrower: explicit reusable modules for existing starter production, bounded planning, and an experiment comparing the current planner with a module-backed planner.”
- “Nested calls cannot reset it.” This applies to the shared planning budget across expansion, routing, scheduling, policy attempts, placement verification, and recovery.
- “Structural capacity, model-predicted output, and observed output are three separate report fields.”
- “The existing mode remains the default during evaluation.”
- “A missing fixture is not a passing measurement.”
- “Parameter search inside a handwritten family is labeled parameter optimization, not invention of arbitrary topology.”
- “Seed 31337 is a development/diagnostic case, not evidence of held-out transfer.”
- First suite: peaceful Nauvis, explored-only observations, 1 and 4 bots; the ten seeds in spec §8.3; automation and the five-window red-science delivery task.
- Build/test through `nix develop -c`. Planner integration tests use `--test suite`; new files must be registered in `crates/planner/tests/suite.rs`. Core tests use `suite` or `botbridge` as appropriate.
- Preserve unrelated changes. Format only edited Rust files using `rustfmt --edition 2024 <explicit paths>`. No `cargo fmt --all`, commit amend, blanket staging, or shared-target cleanup.
- Run focused tests per task, then the repository's `just test` once at integration. Benchmark runs are explicit and sequential; they are not per-commit tests.
- Follow-on discovery, global investment optimization, moving horizons, green-science completion, rocket/platform progression, and new scheduling solvers are excluded from this implementation plan.

## Execution boundaries

The baseline inspected for this plan is `82c22eee`; the design document was committed as `e9ab3de7`. Reconcile source drift before execution. This plan records proposed APIs, not existing symbols.

Execute in three batches, with a reviewable checkpoint at each:

1. Tasks 1–5: control, measurement, and the independent verifier.
2. Tasks 6–12: reusable modules integrated with existing planning.
3. Tasks 13–16: experiment runner, frozen comparison, and final validation.

Use an isolated worktree at implementation time. Do not automatically run the complete experiment matrix alongside compilation. All benchmark child processes must be owned by the runner and shut down on timeout or cancellation.

## File map

| Files | Responsibility |
|---|---|
| `crates/planner/src/control.rs` | Shared budget, stop latch, phase/counter observation hooks |
| `crates/planner/src/request.rs` | Controlled request result, incumbent selection, retry orchestration |
| `crates/planner/src/lib.rs`, `state.rs`, `error.rs` | Compatibility entry point, control propagation, errors |
| `crates/planner/src/method/{mod,assemble,produce,connect,pipe,power,blueprint,sustain}.rs`, `schedule.rs`, `enclosure.rs` | Budget checkpoints and localized module integration |
| `crates/core/src/graph/{route,enclosure}.rs` | Cancellable variants of inner graph searches |
| `crates/planner/src/modules/{mod,model,error,artifact,families,cache,ledger,instance,select,compile}.rs` | Explicit module representation and planning components |
| `crates/planner/src/memory.rs` | Persist module identity and connection intent |
| `crates/core/src/record/{mod,planning,module_delivery}.rs` | Planning records and independent delivery evidence |
| `mods/BotBridge/module_delivery.lua`, `control.lua` | Passive direct-edge production observation and watch registration |
| `crates/scripting_lua/src/globals/goal/{mod,plan,recovery,value}.rs` | Session/options plumbing, controlled outcomes, instance memory |
| `crates/scripting_lua/src/globals/record.rs` | Planning and delivery record bridge |
| `crates/executor/src/recover.rs` | Preserve planning control and intent through recovery |
| `app/src-tauri/src/cli/{mod,plan,experiment}.rs` | Offline controls and experiment entry point |
| `app/src-tauri/src/experiment/{mod,manifest,runner,report}.rs` | Manifest validation, isolated trials, comparison |
| `scripts/module_benchmark.lua` | Fixed benchmark goals, research sink, commissioning/watch lifecycle |
| `experiments/starter-modules.json` | Source manifest: seeds, scenarios, budgets, explicit conditions |
| `docs/research/starter-modules.md` | Reproduction commands, results, and interpretation |

Tests live with pure implementation modules when they exercise private helpers. Cross-component planner tests go in the named integration files below and are registered in `tests/suite.rs`. Lua mod tests go in core's `tests/botbridge.rs` target. Do not move unrelated code out of large files.

## Task 1: Shared planning control with a sticky stop reason

**Files:** Create `crates/planner/src/control.rs`; modify `crates/planner/src/{lib,error}.rs`.

**Interfaces:** Produces the following public types in `control`; no planner method signatures change yet.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkKind {
    Goal, Site, RouteNode, Assignment, LookaheadPair, GraphScan, Retry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanPhase { Expansion, Placement, Routing, Scheduling, Recovery }
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetLimits { pub maxima: BTreeMap<WorkKind, u64> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason { Limit(WorkKind), Cancelled }
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetReport {
    pub used: BTreeMap<WorkKind, u64>,
    pub stopped: Option<StopReason>,
}
#[derive(Clone)]
pub struct PlanControl { inner: Arc<Mutex<ControlState>> }
struct ControlState {
    limits: BudgetLimits,
    report: BudgetReport,
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
```

Implement Debug manually without formatting the cancellation closure. Clone
shares the Arc. A missing maximum means unbounded for
that counter; zero means no operation of that kind is permitted.

Required methods:

```rust
impl PlanControl {
    pub fn new(limits: BudgetLimits) -> Self;
    pub fn with_cancel(limits: BudgetLimits,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>) -> Self;
    pub fn charge(&self, kind: WorkKind) -> Result<(), PlannerError>;
    pub fn checkpoint(&self) -> Result<(), PlannerError>;
    pub fn report(&self) -> BudgetReport;
}
```

- [ ] Add `PlannerError::PlanningStopped { reason: StopReason }`. Write this test inside `control.rs`:

```rust
#[test]
fn a_clone_cannot_reset_the_budget() {
    let control = PlanControl::new(BudgetLimits {
        maxima: BTreeMap::from([(WorkKind::Retry, 1)]),
    });
    assert!(control.charge(WorkKind::Retry).is_ok());
    assert!(control.clone().charge(WorkKind::Retry).is_err());
    assert_eq!(control.report().used[&WorkKind::Retry], 1);
    assert_eq!(control.report().stopped, Some(StopReason::Limit(WorkKind::Retry)));
    assert!(control.checkpoint().is_err());
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --lib control::`; confirm failure before implementation.
- [ ] Implement checked charging: check cancellation outside the mutex; under the mutex, return the existing stop reason if set; reject before exceeding a maximum or overflowing; otherwise increment. Cancellation latches permanently. Add tests for zero allowance, cancellation, independent top-level controls, and a counter reaching its exact maximum.
- [ ] Re-run the focused tests; verify the control does not read time or use a global/thread-local budget.
- [ ] Commit only these files: `feat(planner): add shared deterministic planning budgets`.

## Task 2: Controlled request outcomes and finite conflict retries

**Files:** Create `crates/planner/src/request.rs`; modify `lib.rs`, `state.rs`; create/register `tests/request_budget.rs`.

**Interfaces:** Consumes Task 1. Produces:

```rust
pub struct PlannedMilestone {
    pub net: ActionNetwork,
    pub schedule: Schedule,
    pub memory: ReplanMemory,
}
pub enum PlanStatus { Complete, Exhausted, Infeasible, Unsupported }
pub struct PlanResult {
    pub status: PlanStatus,
    pub incumbent: Option<PlannedMilestone>,
    pub diagnostic: Option<PlannerError>,
    pub budget: BudgetReport,
}
pub fn plan_controlled(
    goals: &[Goal], state: &PlanState, registry: &MethodRegistry,
    chain_actor: BotId, roster: &[BotId], control: &PlanControl,
) -> PlanResult;
```

`PlanState` gains `control: PlanControl`, `with_control(self, PlanControl) -> Self`,
and `control(&self) -> &PlanControl`. Forks inherit it. Existing constructors
create a control with no work limits; the request replaces it with its shared
control. Do not mutate a caller's state to install the control.

- [ ] Add a unit-testable retry helper in `request.rs`:

```rust
fn next_conflict_retry(
    control: &PlanControl, seen: &mut BTreeSet<Pos>, at: Pos,
) -> Result<bool, PlannerError> {
    control.checkpoint()?;
    if seen.contains(&at) { return Ok(false); }
    control.charge(WorkKind::Retry)?;
    seen.insert(at);
    Ok(true)
}

#[test]
fn repeated_conflict_does_not_start_another_attempt() {
    let control = PlanControl::new(BudgetLimits {
        maxima: BTreeMap::from([(WorkKind::Retry, 1)]),
    });
    let mut seen = BTreeSet::new();
    assert!(next_conflict_retry(&control, &mut seen, Pos(3, 4)).unwrap());
    assert!(!next_conflict_retry(&control, &mut seen, Pos(3, 4)).unwrap());
    assert!(next_conflict_retry(&control, &mut seen, Pos(4, 4)).is_err());
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --lib request::` before implementing the driver. Add a driver test seam accepting an attempt closure so scripted attempts can return a feasible plan followed by cancellation; assert the feasible incumbent survives and partial candidates never appear in `incumbent`.
- [ ] Replace both existing retry blocks with one iterative request driver. Keep policy ordering, policy probe short-circuit, and makespan tie behavior. Check the stop latch after every expansion/schedule call even if a lower-level helper returned a different error. Stop on repeated conflicts. Initially reuse the existing conflict-position parser; malformed/non-finite coordinates cannot start a retry.
- [ ] Implement `plan_best` as a compatibility wrapper with the existing tuple signature. Its default control allows one conflict retry and otherwise preserves unlimited legacy behavior. An exhausted result with a valid incumbent returns that incumbent; no incumbent returns `PlanningStopped`. Experimental callers consume the richer result directly.
- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite request_budget` and existing scheduling/automation tests. Check no successful unchanged fixture changes its plan solely from the wrapper.
- [ ] Commit: `fix(planner): bound conflict retries and preserve feasible incumbents`.

## Task 3: Charge inner work and propagate cancellation without false success

**Files:** Modify `method/{mod,assemble,produce,connect,pipe,power,blueprint,sustain}.rs`, `schedule.rs`, `enclosure.rs`, `state.rs`, `request.rs`; modify `crates/core/src/graph/{route,enclosure}.rs`; create/register `tests/budget_checkpoints.rs`.

**Interfaces:** Existing Result-returning methods propagate `PlanningStopped`. Core graph code accepts callbacks rather than depending on the planner crate. Add cancellable counterparts for the core route entry points used by `connect.rs` and the enclosure fill/BFS functions; keep old entry points delegating with an always-true callback. A cancelled core search returns a distinct cancelled result, never an empty successful route or `Escape::Open`.

Name the route counterparts `route_belt_with_tunnels_checked` and
`route_belt_launching_checked`. Preserve the original arguments and append
`keep_going: &mut dyn FnMut() -> bool`; their return type remains
`Result<Route, RouteError>` with a new `RouteError::Cancelled` variant. Core
enclosure counterparts are `fill_from_center_checked`,
`bfs_order_from_center_checked`, and `reachable_from_boundary_checked`, each
appending the same callback and returning respectively
`Result<Escape, SearchCancelled>`, `Result<Vec<(usize, usize)>, SearchCancelled>`,
and `Result<Vec<bool>, SearchCancelled>`. Define `pub struct SearchCancelled;`
in core enclosure.

- [ ] Build a fixture using `factorio_bot_core::test_utils::fixture_world()` with one bot and an unmet item goal. Add:

```rust
#[test]
fn zero_goal_budget_cannot_become_a_no_method_refusal() {
    let state = PlanState::from_world(
        Arc::new(factorio_bot_core::test_utils::fixture_world()), &[BotId(1)]);
    let control = PlanControl::new(BudgetLimits {
        maxima: BTreeMap::from([(WorkKind::Goal, 0)]),
    });
    let result = plan_controlled(
        &[Goal::Have { item: "iron-plate".into(), count: 100,
            whose: Holder::Anyone, via: None }],
        &state, &registry_for(&[BotId(1)]), BotId(1), &[BotId(1)], &control);
    assert!(matches!(result.status, PlanStatus::Exhausted));
    assert!(result.incumbent.is_none());
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite budget_checkpoints` and confirm the new cases fail.
- [ ] Insert charges before `expand_goal` work, each placement orientation attempt, route-node expansion, action/bot feasibility check, lookahead pair evaluation, repeated graph scan, and retry. Charge both rehearsal and actual expansion against the same control. Instrument source-selection and standing-route walks where they loop independently of a geometric search.

```rust
ctx.state.control().charge(WorkKind::Goal)?;
// In an Option-returning placement candidate helper:
state.control().charge(WorkKind::Site).ok()?;
// Every enclosing candidate loop must also check the sticky latch:
state.control().checkpoint()?;
```

- [ ] Add inner-loop cancellation callbacks for `route_belt_launching`, `route_belt_with_tunnels`, `fill_from_center`, `bfs_order_from_center`, and `reachable_from_boundary`. Check each popped search node. Preserve algorithm ordering. Avoid cancel-to-`None` conversion without the enclosing latch check.
- [ ] Add forced exhaustion fixtures for Site, RouteNode, Assignment, LookaheadPair, and GraphScan; assert exact consumed counts never exceed limits. Include cancellation during a placement search and during scheduler lookahead, plus two policy passes sharing one allowance.
- [ ] Run focused planner tests and `nix develop -c cargo test -p factorio-bot-core --test suite` filtered to the affected route/enclosure modules. Then run the planner suite once.
- [ ] Commit: `feat(planner): enforce budgets inside expansion routing and scheduling`.

## Task 4: Request-scoped control and phase measurements at real entry points

**Files:** Modify `control.rs`, `request.rs`, `report.rs`; create `crates/core/src/record/planning.rs`; modify core `record/mod.rs`; modify CLI `plan.rs`, Lua `globals/goal/{mod,plan,recovery,value}.rs`, `globals/record.rs`, executor `recover.rs`, and `scripts/supervisor.lua`.

**Interfaces:** Add an optional observer to `PlanControl`:

```rust
pub enum PlanEvent {
    PhaseEntered(PlanPhase), PhaseLeft(PlanPhase),
    Incumbent { makespan: u32, work: BudgetReport },
}
pub type PlanObserver = Arc<dyn Fn(PlanEvent) + Send + Sync>;
```

`PlanControl::with_observer(self, PlanObserver) -> Self` installs it. A scope
guard returned by `enter_phase(PlanPhase)` emits balanced enter/leave events,
including on error. Call observers outside locks. The outer recorder measures
elapsed wall time with `Instant`; the planner never ranks candidates by it.
Use a nesting stack to derive inclusive and exclusive phase times; do not sum
inclusive times as if they partition total request time.

Core's serialized `PlanningRecord` uses string phase/counter names, request ID,
outcome, optional incumbent ticks, limits, used counts, stop reason, exclusive
phase milliseconds, and total request milliseconds. This avoids a core→planner
dependency. Unknown/unmeasured fields are `Option`, not zero.

- [ ] Add Lua stub tests showing two placement-verification rounds receive the same request control, cancellation always releases the planning pause, and the supervisor records exhaustion without interpreting it as goal satisfaction. Add an observer test:

```rust
#[test]
fn failed_phase_still_closes_its_observation_scope() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let control = PlanControl::new(BudgetLimits::default())
        .with_observer(Arc::new(move |event| sink.lock().unwrap().push(event)));
    { let _phase = control.enter_phase(PlanPhase::Scheduling); }
    assert_eq!(seen.lock().unwrap().len(), 2);
}
```

- [ ] Run focused planner and scripting-lua tests before wiring the new behavior.
- [ ] Create the shared control before `plan_verified` enters `plan_rounds`; pass it through all re-siting rounds and recovery tiers. A supervisor milestone owns the work allowance across retries/replans; a new milestone starts a new allowance. Paused planning segments each obtain an outer deadline check, while the milestone retains cumulative counters. Refresh/path queries remain outside the paused section.
- [ ] Add offline options `--budget-json <path>` and `--planning-deadline-ms <u64>`. Serialize named outcomes under `--json`, including failures. Measure dump loading separately from the planning request and retain existing `WorkCounts` beside budget charges. Existing default command output remains available. Never discard consumed work on error.
- [ ] Add `recover_controlled` beside `recover`, taking `&PlanControl`; retain the old wrapper for existing callers. Tier-1 scheduling, tier-2 expansion, and tier-3 replanning all share control. Budget exhaustion does not trigger another retry tier.
- [ ] Run `nix develop -c cargo test -p factorio-bot-scripting-lua`, focused executor recovery tests, and planning report serialization tests. Confirm legacy records still deserialize and new exhaustion records cannot deserialize as success.
- [ ] Commit: `feat(research): record planning phases and preserve control through recovery`.

## Task 5: Independently verify direct machine-to-lab delivery

**Files:** Create `crates/core/src/record/module_delivery.rs`, `mods/BotBridge/module_delivery.lua`, `crates/core/tests/botbridge_module_delivery.rs`; modify core `record/mod.rs`, `tests/botbridge.rs`, mod `control.lua`, Lua `globals/record.rs`.

**Interfaces:** For the phase-1 red-science topology only, observe one product
assembler, its sole outgoing inserter, and that inserter's destination lab.
Core types:

```rust
pub struct EdgeReading {
    pub tick: u64,
    pub completed_items: u64,
    pub output_items: u64,
    pub held_items: u64,
    pub continuity_valid: bool,
}
pub enum DeliveryVerdict { Achieved, NotAchieved, Unknown }
pub fn delivered_between(a: &EdgeReading, b: &EdgeReading) -> Option<u64>;
pub fn verify_windows(readings: &[EdgeReading], minimum: u64,
    window_ticks: u64, windows: usize) -> DeliveryVerdict;
```

The conservation equation is:

```text
delivered(a,b) = completed_items(b) - completed_items(a)
              + output_items(a) + held_items(a)
              - output_items(b) - held_items(b)
```

This is valid only for a fixed deterministic recipe, unchanged entity IDs,
exclusive output path to the watched lab, and no manual inventory mutation or
other removal. A broken assumption makes the reading unknown. Do not infer
delivery from force totals or planner effects.

- [ ] Add the pure counter test and run `nix develop -c cargo test -p factorio-bot-core --lib module_delivery`:

```rust
#[test]
fn packs_still_in_the_arm_have_not_been_delivered() {
    let a = EdgeReading { tick: 0, completed_items: 0, output_items: 0,
        held_items: 0, continuity_valid: true };
    let b = EdgeReading { tick: 3600, completed_items: 6, output_items: 0,
        held_items: 1, continuity_valid: true };
    assert_eq!(delivered_between(&a, &b), Some(5));
}
```

- [ ] Implement checked arithmetic: use signed wide intermediates, reject counter regression or negative delivery. `verify_windows` requires readings at commissioning and every exact boundary, at least six newly delivered packs in each of five 3,600-tick windows, and valid continuity throughout. Missing boundaries yield Unknown. Adjacent windows share endpoints without double counting. For boundary `i`, compute cumulative delivery from the commissioning reading, subtract the commissioning output-plus-held stock with a floor of zero, and difference those adjusted cumulative values between boundaries. This excludes initial pipeline stock even when it drains into the first window. A decreasing adjusted cumulative count is invalid evidence.
- [ ] Implement a passive Lua watcher with `watch(id, assembler, arm, lab, item, commissioning_tick)`, `on_tick(tick)`, and `invalidate_entity(unit_number, reason)`. Validate IDs, recipe, pickup/drop targets, and exclusive path on every tick during the five-minute interval. Record boundary counters after game updates at one consistent tick phase. Register these helpers from the existing mod tick handler, not a second handler that overwrites it.
- [ ] At registration record initial output and held science and exclude it through the adjusted cumulative calculation. Initial lab inventory never enters the delivery equation and cannot earn credit. Invalidate on recipe/entity/connection changes and any BotBridge insert/remove/mine/configuration action touching the watched output path. Observe player-built/mined/rotated entities and inventory changes relevant to this path; the headless benchmark disallows uncontrolled additional clients or script writers. Failure to prove path exclusivity yields Unknown rather than trusting the equation.
- [ ] Add Lua stub tests for output backpressure, missing power, an arm holding packs at a boundary, preload, manual removal, recipe change, destroyed assembler, alternate output inserter, and a missing sample. Test `verify_windows` on `[6,6,6,6,5]` versus `[6,6,6,6,6]` deliveries.
- [ ] Run `nix develop -c cargo test -p factorio-bot-core --test botbridge botbridge_module_delivery`. During Task 14's live smoke, cross-check the watcher on a controlled single direct edge and verify that blocked delivery stays below production. Experimental setup-only grants in this observer calibration are explicitly tagged and never counted as benchmark trials.
- [ ] Commit: `feat(research): verify science delivery from observed material conservation`.

## Checkpoint A

Run the existing offline automation plan with and without generous work limits;
compare network, schedule, and reported work. Run forced budget exhaustion and
verify structured output plus process termination. Review phase measurements,
control propagation, and watcher correctness before extracting modules. Do not
claim the historical slowdown is explained unless these measurements identify it.

## Task 6: Module artifacts with explicit units and identity

**Files:** Create `crates/planner/src/modules/{mod,model,error,artifact}.rs`; modify planner `lib.rs`, `Cargo.toml`; create/register `tests/module_artifacts.rs`.

**Interfaces:** Public types are re-exported by `modules/mod.rs`. Use these
concrete shapes, deriving serde and equality where field types permit:

```rust
pub type DesignId = String; // SHA-256 of canonical semantic content
pub type InstanceId = u64;  // monotonic within one persisted planning session
pub type PortId = String;
pub struct Rate { pub numerator: u64, pub ticks: NonZeroU64 }
pub struct Offset { pub half_x: i32, pub half_y: i32 }
pub enum ModuleFamily { OreToPlate, RedScience }
pub enum KnowledgeOrigin { Extracted, Imported, Discovered }
pub enum PortMode { BeltInput, InventoryOutput, DirectLabOutput }
pub enum Lane { Left, Right, Both }
pub struct Port {
    pub id: PortId, pub mode: PortMode, pub item: String,
    pub offset: Offset, pub direction: u8, pub lane: Option<Lane>,
    pub maximum: Rate,
}
pub struct Part {
    pub role: String, pub entity: String, pub offset: Offset,
    pub direction: u8, pub recipe: Option<String>,
    pub underground_half: Option<UndergroundHalf>,
}
pub struct ModuleParameters {
    pub item: String, pub with_pole: bool, pub labs: u8,
}
pub struct OperatingContract {
    pub inputs: BTreeMap<String, Rate>, pub outputs: BTreeMap<String, Rate>,
    pub power_watts: u64, pub fuel_per_tick: BTreeMap<String, Rate>,
    pub startup_latency_ticks: u64,
    pub startup_items: BTreeMap<String, u64>,
    pub local_buffer_capacity: BTreeMap<String, u64>,
    pub required_research: Vec<String>, pub required_surface: String,
    pub unsupported_mechanisms: Vec<String>,
}
pub struct ModuleDesign {
    pub schema: u32, pub id: DesignId, pub family: ModuleFamily,
    pub generator_version: u32, pub origin: KnowledgeOrigin,
    pub parameters: ModuleParameters, pub prototype_hash: String,
    pub mod_versions: BTreeMap<String, String>,
    pub parents: Vec<DesignId>, pub training_manifest: Option<String>,
    pub parts: Vec<Part>, pub ports: Vec<Port>,
    pub required_clearance: Vec<Offset>, pub expansion_space: Vec<Offset>,
    pub bill: BTreeMap<String, u64>,
    pub precedence: Vec<(String, String)>,
    pub operation: OperatingContract,
}
pub enum ModuleError {
    Unsupported(String), InvalidArtifact(String), Incompatible(String),
    NoSite(String), Unfunded(String), ArithmeticOverflow, Cancelled,
}
pub fn canonical_bytes(design: &ModuleDesign) -> Result<Vec<u8>, ModuleError>;
pub fn design_id(design: &ModuleDesign) -> Result<DesignId, ModuleError>;
```

`Rate::new(numerator, ticks) -> Result<Rate, ModuleError>` rejects zero ticks
and normalizes by GCD; checked comparison/multiplication use u128 intermediates.
All offsets are half tiles, direction values are the existing game convention,
power is integer watts, and times are game ticks. Preserve prototype collision
boxes through lookup; do not approximate collision boxes with half-tile centers.
Schema-1 family adapters support only these settings. Unknown serialized fields
and unsupported blueprint configuration are rejected.

- [ ] Write tests that reorder parts/ports/research requirements but preserve identity, while changing a recipe, lane, prototype hash, required corridor, or research requirement changes identity. Include:

```rust
#[test]
fn normalized_rates_compare_exactly() {
    assert_eq!(Rate::new(6, 3600).unwrap(), Rate::new(1, 600).unwrap());
    assert!(Rate::new(1, 0).is_err());
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_artifacts`; confirm failure.
- [ ] Implement canonical JSON using a recursively sorted `serde_json::Value`, not HashMap iteration or serde field order assumptions. Exclude `id` from its own hash. Include all semantic design/provenance fields. Keep validation evidence in a separate artifact keyed by design ID. Add direct `sha2 = "0.10"` dependency for stable SHA-256; reuse core's serde_json re-export. Reject duplicate role IDs and invalid precedence references before hashing.
- [ ] Require prototype fingerprint input to include every entity, recipe, item/fuel property used by the adapters. Conservative hashing of the full prototype and recipe tables is acceptable initially; research state is an instance applicability condition, not proof that a design is invalid forever.
- [ ] Run focused tests, round-trip fixtures, and unknown-field rejection tests.
- [ ] Commit: `feat(planner): define versioned factory module artifacts`.

## Task 7: Extract the existing families without duplicating geometry

**Files:** Create `modules/families.rs`; modify `method/{produce,assemble}.rs`; create/register `tests/module_families.rs`.

**Interfaces:** Produces:

```rust
pub fn extract_design(state: &PlanState, family: ModuleFamily,
    parameters: &ModuleParameters) -> Result<ModuleDesign, ModuleError>;
```

Initial supported variants: ore-to-plate for `iron-plate` and `copper-plate`,
and automation-science-pack with the existing direct lab sink. `with_pole` is
false or true where the family supports it; red-science `labs` is 1 or 2. One
design describes one cell; capacity selection chooses a count of instances.
No arbitrary blueprint importer or topology synthesizer is added.

- [ ] Add unit tests beside `assemble.rs` that compare extracted parts, recipes, lanes, mouths, clearance, and bill against the existing native layout at every orthogonal orientation. Add equivalent tests for `produce::parts`. The native reference stays accessible to the tests before replacing its call sites.

```rust
#[test]
fn extracted_iron_cell_still_contains_the_actual_pair() {
    let state = PlanState::from_world(
        Arc::new(factorio_bot_core::test_utils::fixture_world()), &[BotId(1)]);
    let design = extract_design(&state, ModuleFamily::OreToPlate,
        &ModuleParameters { item: "iron-plate".into(), with_pole: false, labs: 0 })
        .unwrap();
    assert_eq!(design.bill.get("burner-mining-drill"), Some(&1));
    assert_eq!(design.bill.get("stone-furnace"), Some(&1));
    assert_eq!(design.operation.outputs["iron-plate"].numerator, 1);
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_families` before implementing adapters.
- [ ] Extract a crate-private native geometry snapshot from the existing layout builders. Normalize positions about a declared origin, preserve role names and orientation, and derive bills from actual parts. Include fuel charges, input recipe quantities, power requirements, lab sink configuration, and port capacity constraints. Required access lanes stay required; existing beacon clearance remains required in the compatibility extraction so that baseline differences are not disguised as caching gains.
- [ ] Validate supported transforms against actual entity alignment, pickup/drop reach, and belt lanes. A rotation transforms ports, clearance, entity direction, and role dependencies together; no mirroring is supported.
- [ ] Keep operating-model evidence at `Unvalidated` until Task 8 validates it. In particular, an ore-to-plate cell's output slot is an inventory output, not a free belt source. Existing offtake/routing work is external and must be costed by the compiler.
- [ ] Run family, placement, red-science, and furnace-ground tests; inspect geometry parity and unchanged baseline schedules.
- [ ] Commit: `refactor(planner): extract starter designs into module artifacts`.

## Task 8: Cache artifacts and validate only what is reusable

**Files:** Create `modules/cache.rs`; modify `modules/{artifact,families,mod}.rs`; create/register `tests/module_cache.rs`.

**Interfaces:**

```rust
pub enum ValidationLevel { Unvalidated, Structural, ModelChecked, LiveMeasured }
pub struct ValidationEvidence {
    pub design_id: DesignId, pub level: ValidationLevel,
    pub model_version: String, pub conditions_hash: String,
    pub fixture_ids: Vec<String>, pub observed_error: Option<Rate>,
}
#[derive(Default)]
pub struct LibraryCache {
    designs: BTreeMap<String, Arc<ModuleDesign>>,
    evidence: BTreeMap<String, ValidationEvidence>,
    sites: BTreeMap<String, bool>,
    pub generated: u64, pub hits: u64, pub misses: u64,
}
pub enum CacheMode { On, Off }
pub fn get_design(cache: &mut LibraryCache, state: &PlanState,
    family: ModuleFamily, parameters: &ModuleParameters,
    mode: CacheMode) -> Result<Arc<ModuleDesign>, ModuleError>;
pub fn validate_design(design: &ModuleDesign, state: &PlanState)
    -> Result<ValidationEvidence, ModuleError>;
```

Use private `BTreeMap` storage keyed by family/parameters/generator/prototype
fingerprints. Site cache keys additionally include surface, transform, and the
session's observed revision. No persistent disk cache is needed: a library file
stores designs/evidence, and an in-memory cache serves a trial. Every trial
starts with the manifest's declared warm/cold state.

- [ ] Write cache tests that count calls to an injected generator rather than asserting elapsed time. Identical requests generate once; a changed relevant prototype generates again; a world revision invalidates site feasibility but does not regenerate the same design. Cache Off invokes the generator every time without altering candidate order.
- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_cache`; confirm failure.
- [ ] Implement local structural checks for duplicate/overlapping parts, valid role references, acyclic precedence, recipe compatibility, port connectivity/reach within the declared family, and finite nonnegative capacities. Keep actual terrain/power-network/source checks out of design validation. Unsupported operation mechanisms return `Unsupported`, not a zero score.
- [ ] Store model coverage and live evidence separately. Structural success cannot promote a module to LiveMeasured. Do not call `search::rank` as the final strategy objective or use its restricted flow model to certify fuel behavior.
- [ ] Run all cache/artifact/family tests and a changed-research applicability test: cached geometry survives, but placement/compilation rechecks research gates.
- [ ] Commit: `feat(planner): cache validated module designs with explicit invalidation`.

## Task 9: Reserve source capacity and operating support over time

**Files:** Create `modules/ledger.rs`; modify `modules/{error,mod}.rs`; create/register `tests/module_supply_ledger.rs`.

**Interfaces:** Use these explicit ledgers instead of counting the same nominal
source rate independently for every consumer:

```rust
pub struct Interval { pub start: u64, pub end: u64 } // half-open, end > start
pub struct SourceCapacity {
    pub id: String, pub item: String, pub rate: Rate,
    pub available: Interval,
}
#[derive(Clone)]
pub struct FlowClaim {
    pub source: String, pub consumer: InstanceId,
    pub item: String, pub rate: Rate, pub interval: Interval,
}
pub struct StockDelivery { pub at: u64, pub quantity: u64 }
pub struct StockClaim { pub consumer: InstanceId, pub at: u64, pub quantity: u64 }
#[derive(Default)]
pub struct OperatingLedger {
    pub sources: BTreeMap<String, SourceCapacity>,
    pub flows: Vec<FlowClaim>,
    pub initial_stock: BTreeMap<String, u64>,
    pub stock_deliveries: BTreeMap<String, Vec<StockDelivery>>,
    pub stock_claims: BTreeMap<String, Vec<StockClaim>>,
}
impl OperatingLedger {
    pub fn reserve_flow(&mut self, claim: FlowClaim) -> Result<(), ModuleError>;
    pub fn reserve_stock(&mut self, key: &str, claim: StockClaim)
        -> Result<(), ModuleError>;
}
```

Power uses the same capacity accounting with a reserved item namespace
`power:<surface>:<network>` and rates in watt-ticks/tick. Belt lanes and external
routes each have source IDs so their throughput can bind separately. Capacity
does not become available merely because its provider is planned.

- [ ] Add this test and run the focused suite before implementation:

```rust
#[test]
fn consumers_cannot_each_claim_the_entire_source() {
    let mut ledger = OperatingLedger::default();
    ledger.sources.insert("iron-out".into(), SourceCapacity {
        id: "iron-out".into(), item: "iron-plate".into(),
        rate: Rate::new(15, 3600).unwrap(),
        available: Interval { start: 100, end: 18100 },
    });
    let claim = FlowClaim { source: "iron-out".into(), consumer: 1,
        item: "iron-plate".into(), rate: Rate::new(10, 3600).unwrap(),
        interval: Interval { start: 100, end: 18100 } };
    ledger.reserve_flow(claim.clone()).unwrap();
    assert!(ledger.reserve_flow(FlowClaim { consumer: 2, ..claim }).is_err());
    assert_eq!(ledger.flows.len(), 1);
}
```

- [ ] Implement transactional reservation: form a trial claim set, sweep sorted interval boundaries, and reject any overlapping demand exceeding source capacity or extending outside source availability. Commit only successful reservations. Use checked rational arithmetic and reject overflow. A failed claim must not consume capacity.
- [ ] For stock, sweep deliveries and claims by tick; available stock can never become negative. A claim at the same tick as a completed delivery follows that delivery, matching schedule completion-before-start semantics. A cyclic set of future producer promises without initial inputs or completed supply cannot bootstrap itself.
- [ ] Represent drill, furnace, burner-inserter, and boiler fuel explicitly. Derive fuel required for the operating horizon from prototype rates and existing charging helpers. Round supply requirements upward. If finite fuel/storage cannot cover the horizon, return an obligation to deliver more through scheduled bot work; do not stretch the source availability interval for free.
- [ ] Test overlapping versus disjoint claims, a delivery arriving after demand, missing power, shared belt-lane capacity, exhausted stock, and a cyclic unsupported source. Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_supply_ledger`.
- [ ] Commit: `feat(planner): account for module supply and fuel over time`.

## Task 10: Persist instance identity and reconcile interrupted builds

**Files:** Create `modules/instance.rs`; modify `modules/mod.rs`, `memory.rs`, `state.rs`; create/register `tests/module_recovery.rs`.

**Interfaces:**

```rust
pub struct Placement {
    pub surface: String, pub half_x: i32, pub half_y: i32, pub direction: u8,
}
pub struct PortBinding {
    pub port: PortId, pub provider_instance: Option<InstanceId>,
    pub provider_port: String, pub source_id: String,
}
pub enum PartState { Unknown, Missing, Standing, Configured }
pub struct ModuleInstance {
    pub id: InstanceId, pub design_id: DesignId, pub placement: Placement,
    pub bindings: Vec<PortBinding>, pub parts: BTreeMap<String, PartState>,
    pub construction_actions: BTreeMap<String, Vec<ActionId>>,
    pub commissioned_tick: Option<u64>,
}
pub struct InstanceMemory {
    pub next_id: InstanceId,
    pub instances: BTreeMap<InstanceId, ModuleInstance>,
}
pub fn reconcile(instance: &ModuleInstance, design: &ModuleDesign,
    world: &PlanState) -> Result<ModuleInstance, ModuleError>;
```

`ReplanMemory` gains `#[serde(default)] pub modules: InstanceMemory`. Implement
`Default` with next ID 1 and an empty map. Existing records load as no module
intent, not as evidence that every historical module was absent.

- [ ] Write a round-trip test for memory and a test with two neighboring instances sharing sub-layout geometry. Interrupt one before its final machine and route arm; reconcile and assert the same instance ID and anchor survive, with only those missing parts scheduled next.

```rust
#[test]
fn an_old_record_has_no_invented_module_identity() {
    let empty = InstanceMemory::default();
    assert_eq!(empty.next_id, 1);
    assert!(empty.instances.is_empty());
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_recovery` before implementing reconciliation.
- [ ] Resolve the recorded surface/anchor/role first. Verify observed prototype, orientation, recipe, and location; preserve Unknown for unobserved areas. Reuse an existing entity only under an unambiguous instance claim. Detect two instances claiming the same physical entity and return an explicit conflict. New instance IDs are monotonic, never reconstructed by hashing the current geometry.
- [ ] Keep candidate instance memory local to each trial PlanState. Failed candidates may populate immutable design caches but cannot commit reservations, action IDs, or instance claims into the session. Commit the chosen plan's memory only after its schedule is valid; reconcile it after actual execution.
- [ ] Add tests for changed recipe, missing observations, same coordinates on another surface, failed-candidate rollback, and an explicitly destroyed part. Run existing replan-memory and standing-cell tests.
- [ ] Commit: `feat(planner): preserve module identity across partial execution`.

## Task 11: Select and site a bounded portfolio using cached designs

**Files:** Create `modules/select.rs`; modify `modules/{mod,cache}.rs`, `method/{produce,assemble}.rs`, `state.rs`; create/register `tests/module_siting.rs`.

**Interfaces:**

```rust
pub struct ProductionRequest {
    pub item: String, pub per_minute: u32, pub support_ticks: u32,
}
pub struct ModuleSelection {
    pub designs: Vec<Arc<ModuleDesign>>, pub instances: Vec<ModuleInstance>,
    pub requests: Vec<ProductionRequest>,
}
pub fn select_candidates(request: &ProductionRequest, state: &PlanState,
    cache: &mut LibraryCache, mode: CacheMode, control: &PlanControl)
    -> Result<Vec<Arc<ModuleDesign>>, ModuleError>;
pub fn site_candidates(design: &ModuleDesign, state: &PlanState,
    near: &Position, count: u32, control: &PlanControl)
    -> Result<Vec<ModuleInstance>, ModuleError>;
```

Selection order is `(family, parameters, design_id)`. Initial global candidate
cap is 8 complete selections per milestone, included in the manifest. Candidate
generation must also obey work limits; the cap does not replace those limits.
Reuse the existing cell-count arithmetic. Do not optimize candidate choice on
steady-state rate alone.

- [ ] Write integration tests for a blocked port, a source outside route reach, a candidate needing an extra pole, and terrain changing after a design-cache hit. Include a cancellation test at the Site limit. Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_siting` before wiring selection.
- [ ] Implement source-aware siting using the existing native search order, cheap occupancy checks, power checks, escape checks, and routing helpers. Transform cached geometry; do not call the old internal layout generator for every site. Reconstruct native cell structs from the selected artifact's role map only after validating that the adapter supports its family/schema.

```rust
for facing in Direction::orthogonal() {
    state.control().charge(WorkKind::Site)
        .map_err(|_| ModuleError::Cancelled)?;
    // Each transformed candidate is checked against the current world;
    // successful immutable geometry validation does not skip these checks.
    state.control().checkpoint().map_err(|_| ModuleError::Cancelled)?;
}
```

- [ ] Resolve external supplier endpoints before accepting a site. Reject routes that cannot bind every required input, and include external route/power costs in the candidate. Keep reservations in candidate-local state. For each request first reconcile existing instances and subtract observed compatible capacity; do not overbuild simply because cache identity changed.
- [ ] Verify arbitrary transformed artifacts with unsupported role layouts are rejected, not silently regenerated into the nearest native shape. Add a spy test demonstrating no geometry-generator invocation during repeated placement attempts for one cached design.
- [ ] Run siting tests and existing placement/enclosure/source-reuse tests.
- [ ] Commit: `feat(planner): site bounded module candidates on observed terrain`.

## Task 12: Compile selected modules through the existing action machinery

**Files:** Create `modules/compile.rs`; modify `modules/mod.rs`, `request.rs`, `state.rs`, `method/{mod,produce,assemble,sustain}.rs`, `network.rs`, `memory.rs`; create/register `tests/module_compile.rs`.

**Interfaces:**

```rust
pub enum PlannerMode { Legacy, Modules }
pub struct PlannerOptions {
    pub mode: PlannerMode, pub cache_mode: CacheMode,
    pub candidate_limit: usize, pub support_ticks: u32,
}
pub struct PlannerSession {
    pub library: LibraryCache, pub memory: InstanceMemory,
    pub observed_revision: u64,
}
pub struct CompiledModules {
    pub steps: Vec<Step>, pub instances: Vec<ModuleInstance>,
    pub ledger: OperatingLedger,
}
pub fn compile_selection(selection: &ModuleSelection,
    ctx: &mut ExpansionCtx, control: &PlanControl)
    -> Result<CompiledModules, PlannerError>;
pub fn plan_with_session(goals: &[Goal], state: &PlanState,
    registry: &MethodRegistry, chain_actor: BotId, roster: &[BotId],
    control: &PlanControl, options: &PlannerOptions,
    session: &mut PlannerSession) -> PlanResult;
```

Extend `PlannedMilestone` with the selected instances, operating ledger, and
fallback diagnostics. `plan_controlled` stays the Legacy wrapper. Do not add a
new public Goal variant: module mode intercepts supported production-method
expansion through candidate-local state containing the selected instances.

- [ ] Add a compile test with the extracted iron cell: acquire its bill, place and fuel it, attach a real offtake if requested, and schedule with the existing scheduler. Use `common::assert_preconditions_hold_over_time` to verify actual action preconditions rather than only counting nodes. Add a test proving changes to a selected port or part are either honored or explicitly rejected, never ignored.
- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_compile` before integration.
- [ ] Extract crate-private `steps_for_module` adapters inside `produce.rs` and `assemble.rs` around existing `cell_steps_fuelled`/`cell_steps` logic. They accept validated native cells reconstructed from selected designs and preserve existing ownership, research, placement, configuration, and evacuation behavior. The module path bypasses native geometry synthesis only; it does not skip construction costs or primitive validation.
- [ ] Feed adapter steps through the existing expansion driver. Name every action's instance and role through a side table in the action network, copied through retaining/recovery operations. Do not infer this association by parsing human-readable labels. External routes and power construction also receive instance/connection provenance.
- [ ] Derive commissioning bounds from the completed schedule, then validate operating claims on that absolute timeline. Source availability begins only after construction, input arrival, first-production latency, and route transit permit delivery. Charge materials for startup and the entire support interval, including the power plant and output inserter. If stock is insufficient, expand the required delivery subgoals and reschedule under the same budget; cap this correction loop at two attempts and reject no-progress. Recurrent support is explicit scheduled work, not an immortal supply flag.

```rust
for correction in 0..=2 {
    control.checkpoint()?;
    // Compile all known acquisition/support obligations, schedule them,
    // and sweep the resulting ledger at actual completion timestamps.
    if correction > 0 { control.charge(WorkKind::Retry)?; }
}
```

- [ ] For each complete candidate, retain it only after network validation, scheduling, powered audit, and operating-ledger validation. Rank by milestone makespan and stable design IDs. Record each incumbent with cumulative work. If a later candidate exhausts the budget, return the earlier feasible plan.
- [ ] Add integrated tests for finite fuel ending before the horizon, simultaneous consumers overbooking power, failed candidate rollback, partial construction, and Legacy output parity. Unsupported production families in Modules mode return Unsupported; primitive acquisition and automation research may use named legacy fallback, reported explicitly.
- [ ] Run planner tests. Commit: `feat(planner): compile module selections into verified bot schedules`.

## Checkpoint B

Demonstrate module mode on the archived four-bot starter map with a release
binary. Show selected design IDs, geometry-cache reuse, actual emitted actions,
source/power reservations, support horizon, and bounded outcomes. Demonstrate
one interrupted construction resuming the same instance. Do not proceed to a
performance claim based solely on fewer primitive actions or predicted ticks.

## Task 13: Expose experimental mode and persistent sessions through CLI/Lua

**Files:** Modify CLI `plan.rs`, Lua `globals/goal/{mod,plan,recovery,value}.rs`, `supervisor_lib.rs`, `scripts/supervisor.lua`, executor `recover.rs`; create `crates/scripting_lua/src/module_benchmark_lib.rs` and register it in `lib.rs`.

**Interfaces:** Offline flags:

```text
plan --planner-mode legacy|modules --module-cache on|off
     --module-library <path> --support-ticks <u32>
     --candidate-limit <usize> --budget-json <path>
     --planning-deadline-ms <u64>
```

Lua options on `goal.plan` and supervisor options mirror those names with
underscores. Defaults: Legacy, cache On, candidate limit 8, support ticks
18,000. A module library path is required in experimental Modules trials but
not for old scripts. Validated embedded family generation produces the library
through the experiment preparation command in Task 14.

- [ ] Add table-driven parser tests: omitted options preserve Legacy behavior; unknown mode, unreadable library, zero candidate limit, zero support interval, and malformed budget fail explicitly. Add a Lua session test showing replans share instance memory/cache while a separate run receives a fresh session.

```lua
local options = {
    planner_mode = "modules", module_cache = "on",
    candidate_limit = 8, support_ticks = 18000,
}
assert(options.support_ticks == 5 * 3600)
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-scripting-lua module_benchmark` and CLI parser tests before wiring.
- [ ] Keep `PlannerSession` in the plan/supervisor context, not global across runs. Retain the chosen plan's module metadata in `PlanValue`. Thread it through `plan_verified`, live placement verification, run recording, and recovery; the inspected code currently discards returned memory, which must stop on the module path.
- [ ] Distinguish stop status from goal satisfaction. On budget exhaustion with an incumbent, the caller may execute it and records the exhaustion. Without an incumbent it ends the attempt; it cannot call `goal.holds` on structural capacity and convert the stop into success. Preserve release of the planning pause on every return path.
- [ ] Add schema documentation beside the installed Lua options and corresponding documentation-guard tests. Run scripting tests and offline Legacy/Modules JSON report smoke checks.
- [ ] Commit: `feat(research): expose module planner sessions and experiment options`.

## Task 14: Freeze a manifest and run isolated, observable trials

**Files:** Create `app/src-tauri/src/experiment/{mod,manifest,runner}.rs`, `cli/experiment.rs`, `scripts/module_benchmark.lua`, `experiments/starter-modules.json`; modify app crate `lib.rs`, CLI `mod.rs`, and scripting `module_benchmark_lib.rs`.

**Interfaces:** Register a normal CLI `Subcommand` with these operations:

```text
factorio-bot experiment prepare --manifest experiments/starter-modules.json --output <new-directory>
factorio-bot experiment run --manifest <prepared-manifest> --smoke --output <new-directory>
factorio-bot experiment run --manifest <prepared-manifest> --output <new-directory>
```

`prepare` resolves runtime-dependent fingerprints, exact versions, starting saves,
hardware, and library IDs. It writes a new resolved manifest and never overwrites
the source manifest. `run` accepts only resolved manifests. `--dry-run` prints
the matrix without starting trials. `--smoke` selects seed 31337/four bots and
one repetition, retaining the same conditions as the full suite.

Source manifest content:

```json
{
  "schema": 1,
  "seeds": [31337,104729,130363,155921,196613,262147,327673,393241,458879,524287],
  "bots": [1,4],
  "tasks": ["automation","red-delivery"],
  "variants": ["legacy","modules-cache-on","modules-cache-off"],
  "repetitions": 3,
  "surface": "nauvis",
  "peaceful": true,
  "visibility": "explored-only",
  "game_speed": 10,
  "pause_during_planning": true,
  "game_tick_limit": 216000,
  "trial_wall_seconds": 1800,
  "planning_deadline_ms": 120000,
  "candidate_limit": 8,
  "support_ticks": 18000,
  "budget_maxima": {
    "Goal": 100000, "Site": 100000, "RouteNode": 10000000,
    "Assignment": 2000000, "LookaheadPair": 10000000,
    "GraphScan": 10000000, "Retry": 8
  },
  "library_initial_state": "warm-artifacts-cold-placement",
  "reset_policy": "same-prepared-save-per-case",
  "sink_policy": "available-red-only-research-by-name"
}
```

These limits are experimental starting limits, not performance promises. Any
calibration change creates a new version before comparative trials. The full
matrix contains 360 trials; print that count and the 180-hour worst-case wall
ceiling before execution. The ceiling explains why smoke and full runs are
separate commands. Do not launch the full matrix as part of a build check.

- [ ] Add manifest tests for missing required provenance, duplicate trial IDs, reordered variants, unknown task/mode, nonpositive limits, and existing output directories. Define `TrialKey { seed: u32, bots: u32, task: String, variant: String, repetition: u32 }` and `expand_matrix(&Manifest) -> Result<Vec<TrialKey>, ManifestError>`; test:

```rust
#[test]
fn the_declared_matrix_has_no_hidden_extra_trials() {
    let raw = include_str!("../../../../experiments/starter-modules.json");
    let manifest: Manifest = serde_json::from_str(raw).unwrap();
    let trials = expand_matrix(&manifest).unwrap();
    assert_eq!(trials.len(), 360);
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot --lib experiment::manifest` before implementing parsing. `Manifest` follows the source JSON fields exactly; `ResolvedManifest` adds full versions/mods/prototype hash, source and baseline revisions, hardware, per-case save hashes, module-library hash, and primitive action semantics. `ManifestError` distinguishes invalid fields, missing provenance, incompatible runtime, and I/O errors.
- [ ] In `prepare`, create each seed/roster initial save through existing process/setup APIs, after the full roster is established. Record actual starting inventories and explored chunks. No grants, revelation of uncharted ground, or instant placement are allowed. Validate enough legitimate red-only research remains to accept 30 packs; otherwise the prepared task is Invalid, not changed silently. Preserve the invalid case in the manifest diagnostics.
- [ ] `module_benchmark.lua` uses the same explicit milestone policy in every variant. Automation asks for the researched flag. Red-delivery requests iron/copper supply and red-science production; rates derive from the recipe contract, rounded up, not a new handwritten recipe table. It queues available red-only research by sorted technology name, using legitimate game research, until the observation interval ends. No science packs may be manually delivered to the watched output path.
- [ ] Declare commissioning when the relevant assembly path is configured, connected, powered, and startup supply is present. Record that actual game tick and register the Task-5 watcher before unpausing it. Exclude initial pipeline stock as defined in Task 5; do not drain or destroy it for free. Watch five exact windows. Fuel depletion and starvation stay visible; they cannot be repaired by resetting the commissioning clock within a trial. Completion time includes all construction and startup ticks from the initial save.
- [ ] Implement isolated sequential trial ownership using existing instance/process APIs, a unique workspace under the requested output root, and a fresh copy of the hashed prepared save. The runner writes trial identity before launch, monitors actual game ticks plus wall time, and gracefully terminates only its own processes at either limit. Cleanup escalation targets owned process handles; do not use global process-name killing.
- [ ] Add fake-process runner tests for timeout, launch failure, cancellation, missing terminal record, manifest mismatch, and interrupted run recovery. Abnormal exits retain a named trial row. A resume skips only trials with final records matching the exact manifest hash; it does not retry failed seeds selectively.
- [ ] Run the watcher calibration described in Task 5, then the six-trial smoke matrix (two tasks × three variants). The smoke uses the same record schemas and strict verification as the full run. Save actual outcomes without requiring the module variant to win.
- [ ] Commit: `feat(research): run frozen starter-module experiments with isolated trials`.

## Task 15: Produce the paired comparison and freeze the baseline

**Files:** Create `app/src-tauri/src/experiment/report.rs`, `docs/research/starter-modules.md`; modify `experiment/mod.rs`, `cli/experiment.rs`.

**Interfaces:**

```rust
pub enum TrialOutcome { Success, Failure, Timeout, Unsupported, Invalid }
pub struct TrialResult {
    pub key: TrialKey, pub manifest_hash: String, pub outcome: TrialOutcome,
    pub game_ticks: Option<u64>, pub planning_ms: Option<u64>,
    pub wall_ms: u64, pub reason: Option<String>,
}
pub fn compare(results: &[TrialResult]) -> Result<Comparison, ReportError>;
```

`Comparison` contains declared/observed counts by variant and task, outcome
counts, paired successful tick/planning differences keyed by seed/roster/task/
repetition, and explicit missing trials. `ReportError` distinguishes mixed
manifests, duplicate rows, and invalid successful results. The detailed run
records supply phase/work/cache/model-error breakdowns; retain links to them
instead of flattening away provenance.

- [ ] Add a comparison test where the faster planner succeeds once and fails once while the baseline succeeds twice. Assert success rates remain 1/2 versus 2/2 and only one paired time comparison exists. Add duplicate/missing/mixed-manifest cases. Run `nix develop -c cargo test -p factorio-bot --lib experiment::report` before implementation.
- [ ] Implement `experiment report --manifest <prepared-manifest> --runs <directory> --output <new-directory>`, emitting `trials.csv`, `comparison.json`, and `report.md`. Report all trial outcomes, phase wall times, work counts, cache hits/misses, site/route work, retries, module IDs, and predicted-versus-observed times/rates. Missing observations print unavailable, not zero.
- [ ] Freeze a comparative baseline containing shared measurement, budget, and observation changes. Record both historical revision `82c22eee` and the new comparative baseline revision. Legacy and Modules execute in the same final binary with the same shared fixes. Confirm unchanged successful legacy fixtures before attributing differences to modules.
- [ ] Include cold-library preparation cost separately from warm-artifact trial costs. Cache-off trials regenerate designs under the same work limits and candidate order; report the consequence when the same budget permits fewer candidates. Do not claim identical chosen plans under tight budgets. A generous-budget cache-on/off check establishes semantic parity separately.
- [ ] Document reproduction and interpretation in `docs/research/starter-modules.md`, including the explicit full-run command:

```bash
nix develop -c cargo build --release -p factorio-bot
target/release/factorio-bot experiment run --manifest <prepared-manifest> --output <new-directory> --dry-run
target/release/factorio-bot experiment run --manifest <prepared-manifest> --output <new-directory>
target/release/factorio-bot experiment report --manifest <prepared-manifest> --runs <run-directory> --output <new-report-directory>
```

The angle-bracket tokens here are CLI usage metavariables, not values to pass
literally. The prepared command prints concrete resolved paths for reproduction.

- [ ] Execute the full matrix only as the explicit experiment stage, with no concurrent build/benchmark jobs. Preserve timed-out and failed rows. Analyze whether reuse reduces generation work, whether placement/scheduling dominates, and whether model errors explain poor live outcomes. If all cases fail, report that finding and the failure distribution; do not replace the benchmark suite with favorable cases.
- [ ] Commit the manifest, report, and small reproducibility fixtures. Large saves/logs stay in the designated artifact directory with content hashes and retrieval instructions. Commit: `docs(research): publish paired starter-module comparison`.

## Task 16: Integration verification and handoff

**Files:** Update the spec status and `docs/research/starter-modules.md` only after actual results exist; change source only to resolve failures from the checks below.

**Interfaces:** No new API. This task verifies the complete phase-0–2 deliverable.

- [ ] Run the following relevant tests, recording exit status and named failures:

```bash
nix develop -c cargo test -p factorio-bot-planner --lib
nix develop -c cargo test -p factorio-bot-planner --test suite
nix develop -c cargo test -p factorio-bot-core --test botbridge botbridge_module_delivery
nix develop -c cargo test -p factorio-bot-scripting-lua
nix develop -c cargo test -p factorio-bot-executor
nix develop -c cargo test -p factorio-bot --lib experiment::
```

- [ ] Run `just test` after focused failures are resolved. Do not run formatting or destructive cleanup as a hidden part of verification.
- [ ] Check the smoke report and full comparison for complete declared trial accounting. Confirm module geometry reuse, interrupted-instance recovery, same shared baseline fixes, no false observed success, and budget termination are each supported by a named test or artifact.
- [ ] Add a final checklist to the research report with actual pass/fail/unsupported evidence for the eight phase-1 acceptance gates in the spec. State the largest remaining bottleneck and the recommended next project from the data.
- [ ] Commit final documentation with explicit paths. Use the development-branch finishing workflow when implementation is complete; do not infer permission to publish externally from this plan.

## Spec coverage and self-review

| Spec requirement | Implementing tasks |
|---|---|
| Shared budgets and bounded retries, including recursive conflict path | 1–4 |
| Actual operating evidence independent of structural success | 5, 14 |
| Versioned geometry, ports, startup, compatibility, provenance | 6–7 |
| Design reuse with conservative world invalidation | 8, 11 |
| No double-spent source, power, fuel, or route capacity | 9, 12 |
| Stable identity and interrupted construction | 10, 12–13 |
| Existing action/scheduler/ownership behavior retained | 2, 7, 12–13 |
| Experimental mode and unchanged default | 12–13 |
| Frozen seeds, same saves, game settings, budgets and hardware | 14–15 |
| Failure-inclusive paired reports and cache ablation | 14–15 |
| Starter scope and follow-on separation | All tasks; no discovery or global solver implementation |
| Full comparison and reviewable conclusion | 15–16 |

The future learned-library and primitive-only tracks are architectural contracts
in the spec, not acceptance requirements for this first project. Their detailed
implementation plans follow the initial experiment. A whole-game completion
claim cannot be made from these starter-module results.
