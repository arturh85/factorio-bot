# First Rocket Module Chain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect, commission and orchestrate existing production modules through a real starter-pack rocket launch on seed 31337 with four character bots, targeting fewer than 864,000 game ticks.

**Architecture:** Continue the September 10 module architecture. Repair contracts and persistence first, compile one supported demand graph with transactional spatial/supply reservations, then drive bounded construction and research from the existing Lua supervisor. Keep launch observation and live experiment execution at the I/O boundary.

**Tech Stack:** Rust 2024, existing Cargo workspace and serde, Lua 5.4, BotBridge, installed Factorio Space Age 2.1.17, existing CLI and headless process management. No new solver, service or frontend dependency.

**Spec:** [First rocket through connected production modules](../specs/2026-09-11-first-rocket-module-chain-design.md). Parent plan: [Hierarchical factory planning](2026-09-10-hierarchical-factory-planning.md).

## Global Constraints

- Module extraction and planning remain pure, deterministic, synchronous, and free of I/O.
- One shared PlanControl covers selection, routing, scheduling, correction and recovery; nested work cannot reset it.
- Structural capacity, predicted supported capacity, and observed production are separate facts.
- Preserve existing public Goal semantics and the Legacy default; the launch policy explicitly selects Modules.
- Legacy acquisition remains available for supported construction bills; complex acquisition becomes explicit finite machine-production work.
- No input, machine, fuel, power, transport capacity, research unlock or exploration is free.
- Preserve default-enemy primary trials; peaceful and fully-charted diagnostics have distinct manifests.
- No global solver, learned library, interplanetary factory, train system, or general combat planner is introduced.
- Build/test through `nix develop -c`. Register planner integration files in `crates/planner/tests/suite.rs` and BotBridge files in `crates/core/tests/botbridge.rs`.
- Preserve unrelated work; format only edited Rust files with `rustfmt --edition 2024` and explicit paths. No blanket staging, shared-target cleanup, or edits to existing workspace saves.
- This is a written implementation plan, not a record of passing tests or live launches. All proposed symbols below are new unless identified as existing.

## Execution boundaries

Baseline `8514f45b`; reconcile drift before editing. The parent spec's status and unchecked task boxes are not evidence of what landed. In particular, `app/src-tauri/src/experiment/runner.rs::run_experiment` currently returns an empty vector. Complete live execution in Task 12 rather than using its existence as evidence.

Use an isolated worktree at implementation time. Keep builds and benchmark trials sequential. Use these checkpoints:

1. Tasks 1–4: trustworthy contracts, goal semantics, ledgers and identity.
2. Tasks 5–7: executable connected production and safe scheduling.
3. Tasks 8–11: commissioned milestone progression, hostile access and actual launch.
4. Task 12: reproducible full-run validation and performance tuning.

A failed checkpoint is recorded and repaired before dependent live trials. No checkpoint may silently relax enemy settings, inventories, action capabilities or terminal evidence. Do not dispatch concurrent agents into shared files; task order below is the default execution sequence.

## File map

| File group | Responsibility |
|---|---|
| `crates/planner/src/modules/{artifact,families,cache}.rs` | Prototype-derived machine variants, contracts and cache compatibility |
| New `modules/{demand,reservations,routing,fallback}.rs` | Goal normalization, spatial claims, connections and validated fallback |
| Existing `modules/{select,ledger,instance,compile,mod}.rs` | Alternative selection, operating commitments and compilation |
| `crates/planner/src/{memory,request,products}.rs` | Persistent intent, complete outcomes, recipe/product resolution |
| `crates/planner/src/method/{connect,pipe,power}.rs` | Reused external construction and powered supply |
| `crates/scripting_lua/src/globals/goal/{plan,value,recovery}.rs` | Plan options, policy reports, sessions/savepoints |
| New `scripts/{rocket_policy,rocket_speedrun}.lua`; existing `scripts/supervisor.lua` | Pure policy, Limiter and driver |
| New `mods/BotBridge/rocket_launch.lua`; existing `control.lua` | Narrow request/launch adapter and terminal observer |
| New `crates/core/src/record/rocket_launch.rs` | Serializable launch evidence |
| New `crates/scripting_lua/src/globals/rocket.rs` | Documented Lua launch interface |
| Existing `app/src-tauri/src/experiment/{manifest,runner,report}.rs`, CLI `experiment.rs` | Isolated live trials, failure-inclusive reporting |
| New `experiments/first-rocket.json`, `docs/research/first-rocket.md` | Frozen policy, trials and evidence |

Tests use existing fixture construction and `common::assert_preconditions_hold_over_time`. New helper APIs below are deliberately small; full integration fixtures must exercise real extraction/selection/compilation rather than returning canned reports.

## Task 1: Freeze the installed launch closure and prove contract compatibility

**Files:** Create `scripts/export_rocket_fixture.lua`, `crates/planner/tests/fixtures/rocket-2.1.17.json`, `crates/planner/tests/module_rocket_contracts.rs`; modify `tests/suite.rs`, `modules/{artifact,families,cache}.rs` only for fixture loading/validation boundaries in this task.

**Interfaces:** Add `LaunchPrototypeFixture` in the test module, deserialized from the checked-in compact fixture. Fields: `game_version: String`, `mods: BTreeMap<String,String>`, `prototype_hash: String`, `rocket_parts_required: u32`, `recipes: BTreeMap<String, serde_json::Value>`, `technologies: BTreeMap<String, serde_json::Value>`, `machines: BTreeMap<String, serde_json::Value>`. Recipes contain normalized ingredients/products/categories/energy; technologies contain prerequisites, unlock effects, unit ingredients/count/time and trigger; machines contain categories/speed/footprint/fluid boxes. Fixture normalization code is test-local; runtime continues using existing graph/prototype types.

- [ ] Write the fixture assertions before adding the exported fixture:

```rust
#[test]
fn installed_launch_has_the_space_age_research_and_payload_contract() {
    let f: LaunchPrototypeFixture = serde_json::from_str(include_str!(
        "fixtures/rocket-2.1.17.json")).unwrap();
    assert_eq!(f.game_version, "2.1.17");
    assert_eq!(f.rocket_parts_required, 50);
    let prereqs = f.technologies["rocket-silo"]["prerequisites"].as_array().unwrap();
    assert!(prereqs.iter().any(|p| p == "logistic-robotics"));
    assert!(prereqs.iter().any(|p| p == "advanced-material-processing-2"));
    assert_eq!(f.recipes["space-platform-starter-pack"]["ingredients"]
        ["space-platform-foundation"], 60);
}
```

- [ ] Run `nix develop -c cargo test -p factorio-bot-planner --test suite module_rocket_contracts`; record the missing-fixture failure.
- [ ] Export from an owned isolated 2.1.17 instance with the actual six-mod set. Use observed force recipe/technology data after all mods load. Normalize maps in sorted order and retain source/save/mod hashes. Traverse only the selected Nauvis closure plus mining, construction and defense machinery; do not embed a multi-gigabyte world dump. Extraction is a data-acquisition action, not a successful run.
- [ ] Validate trigger research, actual machine categories, pipe positions, belt speed, drill yield inputs, furnace speed, silo inventory/progress fields and starter-pack unlock. Recompute closure science totals and finite silo/payload bills; record exact totals in the fixture report. Reject unresolved fields instead of filling fallback guesses.
- [ ] Re-run the fixture test. Commit the exporter, fixture and test: `test(planner): pin Space Age first-launch prototype closure`.

## Task 2: Make designs truthful and add incremental machine rows

**Files:** Modify `modules/{artifact,families,cache,mod}.rs`; extend `tests/module_rocket_contracts.rs` and existing module-family tests.

**Interfaces:** Extend `ModuleParameters` with serde-defaulted `machine: Option<String>` and `units: Option<u16>`. Extend `PortMode` with BeltOutput, FluidInput, FluidOutput, InventoryInput. Add `fluid_box: Option<u32>` to Port. Bump artifact schema/generator versions and reject incompatible cached layouts. Continue exporting `extract_design` and `get_design` with existing signatures.

Add checked helper in `families.rs`:

```rust
fn batch_rate(count: u64, crafts_per_period: u64, product_amount: u64,
              period_ticks: u64) -> Result<Rate, ModuleError>;
```

- [ ] Add a unit test and run `nix develop -c cargo test -p factorio-bot-planner --lib modules::families` expecting the missing helper to fail:

```rust
#[test]
fn steel_rate_uses_steel_recipe_time() {
    assert_eq!(batch_rate(24, 2, 1, 960).unwrap(), Rate::new(48, 960).unwrap());
    assert_eq!(batch_rate(24, 2, 5, 960).unwrap(), Rate::new(240, 960).unwrap());
    assert!(batch_rate(u64::MAX, 2, 1, 960).is_err());
}
```

The first rate is steel output, the second its iron input. Use canonical Rate construction if the current type does not reduce fractions.

- [ ] Implement checked multiplication and rational rate derivation from Task 1 prototype values. Count actual result amounts, crafting speed, mining speed/resource time, and all ingredients/coproducts. Remove hardcoded one-output-per-craft assumptions. Model inserter/belt limits independently.
- [ ] Add burner-only coal/stone mining variants, 6/12/30 electric-drill sections, 6/12/24 steel-furnace sections, and stone-furnace sections needed before steel upgrades. Derive output/fuel paths and footprints from the real layout. Keep primitive iron/copper cells eligible irrespective of total requested rate.
- [ ] Select compatible fluid-capable assemblers for processing units and rocket fuel using recipe category intersection; reserve chemical plants for categories they actually support. Emit actual pipes, power poles and all bill entities, not just entries in the bill. Use assembler 1/2 for the starter pack when compatible, and gate it on rocket-silo unlock. Remove inventory extraction of rocket parts; model internal silo progress separately.
- [ ] Add structural regression cases for wrong machine category, missing pipe part, wrong collision footprint, blocked fluid endpoint, changed recipe result count, steel/brick ratios and changed research after a geometry cache hit. Check unknown old artifact schemas return `Incompatible`, not a regenerated approximation.
- [ ] Run contract and family/cache tests; commit `fix(planner): derive module contracts and incremental variants from prototypes`.

## Task 3: Preserve mixed goal semantics and build material demand

**Files:** Create `modules/demand.rs`, `tests/module_demand.rs`; modify `modules/{mod,compile}.rs`, `products.rs`, `tests/suite.rs`.

**Interfaces:** Define in demand.rs:

```rust
#[derive(Debug, Default)]
pub struct DemandSet {
    pub gross_floor: BTreeMap<String, Rate>,
    pub finite: Vec<Goal>,
    pub residual: Vec<Goal>,
    pub exports: BTreeMap<String, Rate>,
}
pub fn normalize_goals(goals: &[Goal], exports: &BTreeMap<String, Rate>)
    -> Result<DemandSet, ModuleError>;
pub fn required_gross(floor: Rate, internal: Rate, exported: Rate)
    -> Result<Rate, ModuleError>;
```

Preserve Have/Produced goals verbatim in finite until recipe/holder-aware batch expansion. Recurse through All, merging identical constraints only. Research and other unsupported-by-module goals remain residual and required for overall completion. Add `external_rates: BTreeMap<String, Rate>` to PlannerOptions with an empty default.

- [ ] Add and run the following test under `--test suite module_demand` before implementation:

```rust
#[test]
fn gross_floor_is_not_an_implicit_export() {
    let r = |n| Rate::new(n, 3600).unwrap();
    assert_eq!(required_gross(r(60), r(60), r(0)).unwrap(), r(60));
    assert_eq!(required_gross(r(60), r(60), r(60)).unwrap(), r(120));
}
```

- [ ] Implement the material equation `max(floor, internal + exported)` with checked rational comparison/addition. Keep finite stock demand out of that rate equation. Replace the filter_map helper that turns quantities into rates.
- [ ] Build a deterministic recipe DAG using existing products.rs lookup. Honor via selections; select basic/advanced oil recipes through an explicit policy preference map. Reject off-planet/recycling candidates for this policy, and reject cycles with the recipe path. Sum separate consumers, preserve simultaneous coproducts, and use requested-product output for sizing.
- [ ] Add All fixtures with duplicate goals, nested research, finite Have with named holder, Produced with via/unlocks, petroleum product versus refinery recipe name, fractional recipe yields, and zero-rate requests. Zero-rate requires no new module. A missing child must make the whole plan incomplete/unsupported rather than a partial success.
- [ ] Re-run the focused suite; commit `feat(planner): normalize combined production and finite demand without changing goals`.

## Task 4: Repair supply admission and persist instance identity

**Files:** Modify `modules/{ledger,instance,compile}.rs`, `memory.rs`, `request.rs`, Lua `globals/goal/{plan,value,recovery}.rs`; create/register `tests/module_launch_memory.rs`.

**Interfaces:** Add `#[serde(default)] pub modules: InstanceMemory` to ReplanMemory and `pub memory: InstanceMemory` to PlannerSession. Store module ledger and instance/action provenance in PlannedMilestone with backward-compatible defaults. Change `InstanceMemory::allocate_id` to return `Result<InstanceId, ModuleError>`; default next_id is 1, overflow rejects.

- [ ] Write this regression in ledger.rs and run `--lib modules::ledger` before repairing admission:

```rust
#[test]
fn partial_overlap_does_not_fund_a_longer_claim() {
    let mut l = OperatingLedger::default();
    l.add_source(SourceCapacity { id: "iron".into(), item: "iron-plate".into(),
        rate: Rate::new(60, 3600).unwrap(),
        available: Interval::new(100, 200).unwrap() });
    let claim = FlowClaim { source: "iron".into(), consumer: 1,
        item: "iron-plate".into(), rate: Rate::new(30, 3600).unwrap(),
        interval: Interval::new(50, 150).unwrap() };
    assert!(l.reserve_flow(claim).is_err());
    assert!(l.flows.is_empty());
}
```

- [ ] Require full source interval containment, valid nonempty intervals and matching items. Sweep all boundaries with checked rational arithmetic. For stock, replay all deliveries and claims, including previously accepted future claims, before committing a new claim. A backdated claim cannot invalidate a later reservation. Simultaneous completed delivery precedes a claim at that tick.
- [ ] Add tests for a new early stock claim stealing later committed stock, fractional overlap, disjoint claims, interval/ID overflow, unavailable source, and failed-claim rollback. Share the same ledger for materials, belt lanes, pipe capacity, fuel and power.
- [ ] Persist IDs, bindings, support expiry and action provenance through plan output, supervisor savepoint and recovery. Allocate only inside candidate-local memory; commit only the chosen valid plan. Reconcile observed entity/recipe/facing/surface and preserve Unknown for unobserved parts. Detect one entity claimed by two instances.
- [ ] Test resume after one of two neighboring cells is partially built, old record deserialization, ID monotonicity, same coordinates on distinct surfaces, and rollback after failed candidate scheduling. Run planner memory tests plus `nix develop -c cargo test -p factorio-bot-scripting-lua`.
- [ ] Commit `fix(planner): preserve module commitments and reject unfunded intervals`.

## Task 5: Select one alternative and reserve the whole layout

**Files:** Create `modules/reservations.rs`, `tests/module_launch_selection.rs`; modify `modules/{mod,select,compile}.rs`, tests/suite.rs.

**Interfaces:** Reservation types in reservations.rs:

```rust
#[derive(Clone, Debug)]
pub struct HalfRect { pub left: i32, pub top: i32, pub right: i32, pub bottom: i32 }
#[derive(Clone, Default)]
pub struct ReservationSet { pub rects: Vec<(InstanceId, String, HalfRect)> }
impl ReservationSet {
    pub fn reserve(&mut self, owner: InstanceId, surface: &str, area: HalfRect)
        -> Result<(), ModuleError>;
}
```

Rectangles are half-open in half-tile coordinates. Add `reservations: &mut ReservationSet` to site_candidates. Use a separate copy per full alternative. Required access regions can have conservative rectangles initially; legal connection endpoints must be represented explicitly rather than allowing arbitrary same-owner overlap.

- [ ] Add and run under `--lib modules::reservations`:

```rust
#[test]
fn a_second_module_cannot_reuse_the_first_footprint() {
    let mut r = ReservationSet::default();
    let a = HalfRect { left: 0, top: 0, right: 8, bottom: 8 };
    r.reserve(1, "nauvis", a.clone()).unwrap();
    assert!(r.reserve(2, "nauvis", a.clone()).is_err());
    assert!(r.reserve(2, "other", a).is_ok());
    assert_eq!(r.rects.len(), 2);
}
```

- [ ] Implement transformed footprints and declared pitch, world occupancy, access/escape, route corridors, mining coverage, fluid positions and charted-area checks. Apply existing threat exclusions to cell and route candidates. Allocate monotonic instance IDs from Task 4 memory; never return every instance as zero.
- [ ] Replace enum-order priority with research-compatible portfolio enumeration. Compute missing supported capacity after standing/in-flight allocations. Evaluate at most eight complete alternatives under shared PlanControl. Rank by complete predicted milestone tick, incremental bot-ticks/material vector and stable IDs/anchors. On route failure continue to another site/alternative; on budget stop preserve only a fully validated incumbent.
- [ ] Test locked steel versus unlocked stone, affordable small rows versus unfunded full arrays, high demand with no unlocked upgrade, changed research after cache hit, overlapping same-family copies, red/green neighboring cells and a route rejected after footprint success. Verify only one alternative's BOM and reservations reach output.
- [ ] Run `--test suite module_launch_selection` plus existing module siting tests. Commit `feat(planner): select one feasible module portfolio with shared reservations`.

## Task 6: Compile real connections and finite construction acquisition

**Files:** Create `modules/routing.rs`, `tests/module_connected_chain.rs`; modify `modules/{compile,ledger,select}.rs`, method/{connect,pipe,power}.rs, tests/suite.rs.

**Interfaces:** Add in routing.rs:

```rust
pub struct ConnectionRequest {
    pub source: InstanceId, pub source_port: String,
    pub consumer: InstanceId, pub consumer_port: String,
    pub item: String, pub rate: Rate,
}
pub fn route_connections(requests: &[ConnectionRequest], selection: &ModuleSelection,
    ctx: &mut ExpansionCtx, reservations: &mut ReservationSet,
    control: &PlanControl) -> Result<Vec<Step>, ModuleError>;
```

Add `construction_shortfalls: BTreeMap<String,u64>` and `operating_shortfalls: BTreeMap<String,Rate>` to the module planning diagnostic report. These are distinct from unsupported recipes and budget stop. Adapter maps route errors into the existing typed ModuleError variants with source/consumer context.

- [ ] Write an integration test using the existing fixture-world helpers: request iron/copper/gears, inspect connections and run `common::assert_preconditions_hold_over_time` on the resulting net/schedule. Before implementation it must fail because no belt link/ledger commitment exists. Run `--test suite module_connected_chain`.
- [ ] Implement directed belt lanes, compatible inserter handoffs, actual fluid-box endpoints and pipe networks using existing route methods. Reserve each link's bottleneck capacity in addition to supplier flow. Charge clearance/removal/build materials and bot work. Fail unbound ports; do not treat matching item strings as a connection.
- [ ] Compile simple construction Have subgoals normally. For skipped/complex construction items first check accessible stock and actor assignment, otherwise return the exact finite machine-production prerequisite. Preserve legitimate handcraftable bootstrap recipes. Do not recurse a chemical-plant bill through an unsupported legacy chemistry chain. Include finite input deliveries, output collection and recipe changes for batch assemblers.
- [ ] Call ensure_powered against combined module/route reservations and aggregate demand with explicit 20 percent policy headroom. Reuse standing networks before expansion. Price fuel, water supply and recurring replenishment through the support horizon; recompute availability using schedule completion and transport latency. Permit at most two schedule/ledger correction retries under shared control.
- [ ] Add fixtures for one supplier feeding two consumers, separate net exports, finite inventory exhausted mid-window, assembler fluid input, refinery coproduct storage saturation, boiler fuel depletion and a skipped silo absent from builder inventory. Verify unsupported cases return no Complete plan.
- [ ] Execute an isolated two-cell live fixture for three windows and retain the loading script, initial stock and output evidence. Preloaded fixture inputs are disclosed and do not count as from-start success. Commit `feat(planner): connect module flows and fund finite construction prerequisites`.

## Task 7: Keep the flat fallback executable under four-bot ownership

**Files:** Create `modules/fallback.rs`, `tests/module_fallback.rs`; modify `modules/{mod,compile}.rs`, tests/suite.rs. Reuse existing network.rs ownership and predecessor APIs.

**Interfaces:**

```rust
pub fn schedule_fallback(net: &ActionNetwork, state: &PlanState,
    roster: &[BotId], control: &PlanControl) -> Result<Schedule, PlannerError>;
fn earliest_start(bot_free: u32, predecessor_ends: &[(u32,u32)])
    -> Result<u32, ModuleError>;
```

- [ ] Add/run `--lib modules::fallback`:

```rust
#[test]
fn successor_waits_for_other_bot_and_edge_lag() {
    assert_eq!(earliest_start(20, &[(100, 30), (80, 0)]).unwrap(), 130);
    assert!(earliest_start(0, &[(u32::MAX, 1)]).is_err());
}
```

- [ ] Implement topo traversal with checked predecessor-end-plus-lag and chain/pinned-owner constraints. Determine travel and action eligibility from actual per-bot simulated state. Insert existing walk steps and explicit transfer work when supported; reject impossible ownership rather than assigning another bot for convenience. Update inventories/effects in chronological order.
- [ ] Run `net.validate()` and existing precondition-over-time validation on the completed schedule. The earliest-start helper is necessary but not sufficient evidence. Charge fallback assignments/scans and reject sticky cancellation/exhaustion; do not reset control after a primary scheduler failure.
- [ ] Test cross-bot acquisition/placement, actor-pinned insertion, unsatisfied inventory, travel, cyclic network, empty roster, unavailable chain owner, and cancellation. Compare safe single-bot fallback with normal scheduling on a small fixture.
- [ ] Run `--test suite module_fallback`; commit `fix(planner): preserve dependencies ownership and travel in flat schedules`.

## Checkpoint A: A connected plan is an executable plan

- [ ] Demonstrate Tasks 1–7 with exact prototype hashes, distinct instance IDs, funded physical connections and valid fallback.
- [ ] Save two-cell and All-chain fixture evidence including warmup and three 3,600-tick windows. Report unknown/missing observations as such.
- [ ] Reconcile the parent plan's ledger/identity/compile requirements against these results. Do not claim the parent benchmark matrix passed.

## Task 8: Add Limiter and commissioning decisions to the existing supervisor

**Files:** Create `scripts/rocket_policy.lua`; modify `scripts/supervisor.lua`, `crates/scripting_lua/src/supervisor_lib.rs`, Lua goal plan/value/recovery bridges. Tests live in supervisor_lib.rs using mlua and stubs as the existing supervisor tests do.

**Interfaces:** rocket_policy.lua returns a table with pure functions:

```lua
-- config: target_rate, max_new_copies, window_ticks, required_windows
-- memory: instance_ids, constructed_ids, commissioned_ids, state, windows
-- observation: tick, constructed_ids, delivered_rate, supply_ready, evidence_known
-- Returns a new memory table and one of build/observe/repair/complete/blocked.
function policy.limit(config, memory, observation) end
-- All tables returned by policy functions must be serializable; no closures in savepoints.
```

- [ ] Add the following Lua test through the existing embedded-Lua test harness and run `nix develop -c cargo test -p factorio-bot-scripting-lua supervisor` before implementation:

```lua
local cfg = {target_rate=60, max_new_copies=2, window_ticks=3600, required_windows=3}
local m = {instance_ids={11,12}, constructed_ids={}, commissioned_ids={},
           state="building", windows=0}
local obs = {tick=100, constructed_ids={11,12}, delivered_rate=0,
             supply_ready=false, evidence_known=true}
local next_m, decision = policy.limit(cfg, m, obs)
assert(decision == "repair")
assert(#next_m.instance_ids == 2)
assert(next_m.state ~= "complete")
assert(m.state == "building")
```

- [ ] Implement copy counting by unique persisted IDs, bounded batch allocation, transition to commissioning at the construction cap and three full consecutive windows. Missing evidence yields observe/blocked, not success. A rate below target resets the consecutive-window count and emits a diagnosed repair action when supply is known missing.
- [ ] Distinguish structural goal completion from commissioning completion. Use existing Sustain/witness interfaces but add instance/connection delivery evidence for drained outputs; do not infer zero production from a constantly drained inventory or infer link success from whole-force totals. Persist window start/count through savepoints.
- [ ] Track support expiry and schedule replenishment before it. A build-cap hit emits no more placement until commissioning or a new explicitly budgeted investment stage. A deadline expires to blocked with stage/reason, never to the next milestone as success.
- [ ] Test repeated observations, duplicate IDs, resume midway through a window, full output buffer, supply loss, unknown evidence and finite batches that require quantity/holder verification instead of rate windows. Commit `feat(supervisor): limit construction by instance and commission observed flow`.

## Task 9: Encode the research and construction milestone policy

**Files:** Extend scripts/rocket_policy.lua; create scripts/rocket_speedrun.lua and experiments/first-rocket.json; extend supervisor_lib.rs tests and Lua plan report options.

**Interfaces:** Add `policy.next(config, memory, snapshot) -> new_memory, decision`. A decision is a serializable table `{kind, stage, goal, limits, reason}`; goal is a normalized data description converted by the driver to existing goal constructors. Kinds are prerequisite, build, support, research, observe, launch, wait, blocked, complete. Snapshot fields are tick, researched, recipe_enabled, accessible_stock, instances, flow_evidence, lab_capacity, threats, launch_evidence and module_shortfalls. Unknown values are explicit, not zero.

- [ ] Add pure tests that a locked steel row first selects its research/prerequisite, an absent refinery emits a finite construction prerequisite, and a already-constructed consumer with no plastic requests upstream repair rather than another assembler. Run scripting-lua supervisor tests.
- [ ] Encode the ten spec stages with absolute game-tick deadlines. Burner investment uses total iron stages 4/8/12 and copper 2/4/6; each further stage competes with affordable electric alternatives. Coal, rock collection, stone and wood are explicit work. Initial roles prefer two builders, one acquisition/support bot and one scout when safe, but roles are scheduler preferences, not constraints that leave ready work idle.
- [ ] Generate a topological research queue from the runtime closure; apply the spec's preferred branch order only to ready nodes. Track trigger completion from observations. Never set researched/unlocks flags in the game to advance a stage.
- [ ] Compute remaining science and research service time from quantities, unit times and measured lab capacity. Use `remaining_packs / remaining_minutes` as a lower bound on required pack rate, then include construction/warmup margin. Start at 30/min, authorize funded increments toward 45/min where needed. Report deadline infeasibility instead of treating the milestone table as a proof.
- [ ] Reserve finite silo and payload quantities, including 1,220 payload steel and 1,200 cable from the pinned data. Stop buffer production after next-two-batches plus declared repair/defense stock are funded. Keep existing factory operation active while research proceeds; issue independent ready work instead of a blocking research wait.
- [ ] Export policy hash, recipe preferences, row variants, copy budgets, deadlines, support horizon, research priorities and rate/export distinction in the manifest. Unknown runtime provenance invalidates preparation. Add tests for mandatory logistic robotics/advanced material processing 2 research without unnecessary robot/furnace construction, 50-part target, and starter-pack unlock preceding space-platform trigger.
- [ ] Run focused Lua tests and offline plan calls for each stage with pinned intermediate fixtures; retain all refusals. Commit `feat(supervisor): orchestrate finite first-rocket milestones and research`.

## Task 10: Fund safe oil access and static defenses under default enemies

**Files:** Extend scripts/rocket_policy.lua and rocket_speedrun.lua; modify modules/select.rs and existing threat-aware routing call sites only where the module path bypasses them. Extend supervisor_lib.rs and create/register tests/module_oil_access.rs.

**Interfaces:** Pure policy helper `policy.oil_access(config, snapshot) -> decision` uses snapshot fields `oil_charted`, `safe_route`, `defense_stock_ready`, `defense_supported` and `survey_budget_remaining`. Values can be unknown. Decisions reuse Task 9's kind/stage/reason vocabulary.

- [ ] Add/run a pure test:

```lua
local d = policy.oil_access({}, {oil_charted=true, safe_route=false,
    defense_stock_ready=true, defense_supported=true, survey_budget_remaining=0})
assert(d.kind == "blocked")
assert(d.reason == "unsafe_oil_corridor")
```

- [ ] Reuse chart_until and existing threat standoff/source/path helpers. Survey observed route alternatives under the shared budget. Validate builders' travel paths as well as final pumpjack placement. Unknown terrain requests observation; no safe route after budget exhaustion reports unsafe_oil_corridor.
- [ ] Add finite stock goals and ordinary placement/refill actions for gun turrets, ammunition and repair supplies at declared defended construction/access nodes. Require actual range/coverage and supply capacity; account for recurring ammo and fuel under the same ledger. Do not invent a generic combat solver or remove nests with script calls.
- [ ] Add planner tests for a clear route, worm-covered endpoint, spawner-covered travel path, unknown area, exhausted ammo supply and module siting bypassing a legacy threat guard. Preserve default enemy settings and refuse unsupported clearance.
- [ ] Run an isolated default-enemy seed-31337 oil-access trial. Record survey routes, enemy observations, damage/deaths, defense consumption and delivery completion. A failed trial stays failed; run peaceful only with a distinct diagnostic label. Commit `feat(speedrun): account for safe oil access and funded static defense`.

## Task 11: Add idempotent launch control and independent terminal evidence

**Files:** Create mods/BotBridge/rocket_launch.lua, crates/core/src/record/rocket_launch.rs, crates/core/tests/botbridge_rocket_launch.rs and crates/scripting_lua/src/globals/rocket.rs; modify associated mod.rs/registration files, BotBridge control.lua, core tests/botbridge.rs and rocket_speedrun.lua.

**Interfaces:** Lua API `rocket.request{key, name, planet, starter_pack}`, `rocket.status(key)`, `rocket.launch{key, silo_unit_number}`. key is a persisted run-scoped unique string. Return named result tables, not truthy success strings. Status includes request identity, silo identity, launch_ordered_tick, launched_tick and platform_established_tick as optional values. The adapter uses installed API semantics and validates force/surface/payload before mutation.

Core evidence record:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RocketLaunchEvidence {
    pub run_key: String,
    pub silo_unit_number: u32,
    pub platform_index: u32,
    pub payload: String,
    pub launch_ordered_tick: Option<u64>,
    pub launched_tick: Option<u64>,
    pub platform_established_tick: Option<u64>,
}
impl RocketLaunchEvidence {
    pub fn achieved(&self) -> bool {
        self.payload == "space-platform-starter-pack"
            && self.launched_tick.is_some()
            && self.platform_established_tick.is_some()
    }
}
```

The observer joins matching force/request/silo/platform events before constructing this record; achieved is not a substitute for that attribution validation.

- [ ] Add a core test where launch_ordered_tick is Some but launched_tick is None; assert achieved is false. Run `nix develop -c cargo test -p factorio-bot-core --test botbridge botbridge_rocket_launch` before adapter implementation.
- [ ] Register persistent request memory and event handlers without replacing existing BotBridge handlers. Inspect create_space_platform in an isolated fixture: assert an unfulfilled request does not produce terminal evidence or a free built platform. Reject unsupported semantics explicitly. Query existing key before creating another request.
- [ ] Build the normal inventory insertion steps for one real starter pack; query silo's internal parts instead of taking rocket-part items. Order launch only with correct readiness/destination. A false launch_rocket result remains not_ready. An ambiguous response triggers query-before-retry.
- [ ] Record on_rocket_launched and the requested platform's established state independently; persist them across save/load. Reject unrelated old launches, mismatched forces/payloads and preexisting platform identities. Document the Lua API through the same registration/doc pattern as other globals.
- [ ] Test duplicate request, not-ready launch, missing pack, wrong silo, launch ordered without completion, save/resume after launch and unrelated platform. Execute an isolated loaded fixture and save its actual before/after inventory and event record. Commit `feat(speedrun): launch a real starter pack with persistent terminal evidence`.

## Checkpoint B: Full mechanics before timing claims

- [ ] Demonstrate petroleum-to-blue-science operation, funded construction stock and bounded commissioning.
- [ ] Demonstrate primary-condition oil access or record it as a failed capability gate; do not claim the target achieved while it remains unresolved.
- [ ] Demonstrate request-to-launch-to-platform in a disclosed isolated fixture. Confirm no grant of research, materials or platform establishment by the adapter.

## Task 12: Complete the live runner and measure the frozen policy

**Files:** Modify app/src-tauri/src/experiment/{manifest,runner,report}.rs and cli/experiment.rs; finish experiments/first-rocket.json; create docs/research/first-rocket.md. Add focused unit tests beside runner/report.

**Interfaces:** Implement existing run_experiment instead of adding a competing runner. Preserve existing TrialOutcome/TrialResult names and expand serialized records with run/policy/save hashes and terminal evidence paths. Add `live_first_rocket` task dispatch without changing starter task definitions. CLI preparation must resolve a source manifest to a file named `workspace/first-rocket-trials/prepared.json` in the commands below; refuse collisions rather than overwriting prior runs.

- [ ] Write a fake-process runner test expecting one final row per declared trial, including failed startup, timeout and missing terminal evidence. Assert the existing empty-vector stub fails. Run `nix develop -c cargo test -p factorio-bot --lib experiment::`.
- [ ] Reuse existing app instance/process setup to create owned, unique workspaces and ports. Copy a pristine seed-31337 save into each; never reset a user's active workspace. Stage the release binary and exact scripts/mod hashes. Resolve script names relative to each workspace's scripts directory, matching current CLI semantics.
- [ ] Monitor actual game ticks plus a separate manifest wall timeout. Shut down only owned child processes on timeout/cancellation and retain final failure rows and artifacts. Fresh runs share no warm instance/policy state unless the manifest explicitly declares it. Record planning pauses and phase wall time separately.
- [ ] Implement preparation, dry-run, run and report dispatch so these concrete commands work after building:

```bash
nix develop -c cargo build --release -p factorio-bot
target/release/factorio-bot experiment prepare --manifest experiments/first-rocket.json --output workspace/first-rocket-trials/prepared.json
target/release/factorio-bot experiment run --manifest workspace/first-rocket-trials/prepared.json --output workspace/first-rocket-trials/runs --dry-run
target/release/factorio-bot experiment run --manifest workspace/first-rocket-trials/prepared.json --output workspace/first-rocket-trials/runs
target/release/factorio-bot experiment report --manifest workspace/first-rocket-trials/prepared.json --runs workspace/first-rocket-trials/runs --output workspace/first-rocket-trials/report
```

These commands are target interfaces for this task, not claims that the present CLI/runner supports them. Preparation prints resolved paths and provenance; dry-run must not start a game. Declare one peaceful diagnostic followed by three primary default-enemy trials and distinguish them in every report.

- [ ] Reject success without matching actual launch/platform evidence and initial provenance. Verify tick duration is terminal tick minus initial tick, not last-action completion. Report all failures/timeouts and no-trial execution as invalid, never as zero-time success.
- [ ] Run focused verification once code is stable:

```bash
nix develop -c cargo test -p factorio-bot-planner --lib
nix develop -c cargo test -p factorio-bot-planner --test suite
nix develop -c cargo test -p factorio-bot-core --test botbridge
nix develop -c cargo test -p factorio-bot-scripting-lua
nix develop -c cargo test -p factorio-bot-executor
nix develop -c cargo test -p factorio-bot --lib experiment::
nix develop -c just test
```

- [ ] Execute the declared diagnostic/primary cohort sequentially. Target three of three primary launches below 864,000 ticks. If tuning is needed, diagnose science supply/lab time, mining/fuel, travel, construction buffers, oil transport or defense from records; freeze a new policy hash and declare another complete cohort. Do not selectively drop failed attempts.
- [ ] Publish a local report with all outcomes, actual milestone ticks, rates, lab starvation, bot utilization, material use, damage/deaths, planner work/time, retry/duplicate counts and terminal evidence links. Keep large saves in the artifact directory with hashes; commit the small reproduction apparatus and report. Clearly distinguish target versus measured result. Commit `feat(research): run and report reproducible first-rocket trials` only when the runner and report changes are verified; a failing speed target is an honest reported outcome, not completion of the performance objective.

## Spec coverage and self-review

| Spec requirement | Tasks |
|---|---|
| Parent continuity and source drift | Execution boundaries, 1, checkpoints |
| Installed prototypes and mandatory research closure | 1, 2, 9 |
| Incremental primitive/electric/steel variants and priority | 2, 5, 9 |
| Gross Goal semantics, explicit exports, finite quantities, All | 3, 6 |
| Source/stock/power/fuel commitments | 4, 6, 8 |
| Persistent identity and rollback | 4, 5, 8, 11 |
| Combined siting and physical fluid/belt routes | 5, 6 |
| Safe flat scheduler fallback | 7 |
| Limiter, commissioning and ongoing support | 8, 9 |
| Research and construction acquisition sequence | 6, 9 |
| Default-enemy access and defense cost | 10, 12 |
| Actual launch/platform terminal evidence | 11, 12 |
| Live runner, provenance, time and failure accounting | 12 |

Interface review: connection requests refer to ModuleSelection/InstanceId/Rate from existing modules; reservation coordinates use half-tiles throughout; normalized DemandSet keeps original Goal values for quantity/holder/recipe semantics; Lua decisions are data tables and remain serializable. The plan does not invent a new public Goal or a parallel executor. Test commands are prospective implementation checks, not results of the document-writing task.
