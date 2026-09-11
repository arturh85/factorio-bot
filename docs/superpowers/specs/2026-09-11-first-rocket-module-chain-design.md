# First rocket through connected production modules

Date: 2026-09-11

Status: continuation design requested by the owner after review of the proposed architecture. This document and its implementation plan are the deliverable; implementation and live benchmarking have not been performed.

Source baseline: `8514f45b`. Parent: [Hierarchical factory planning](2026-09-10-hierarchical-factory-planning-design.md) and its [implementation plan](../plans/2026-09-10-hierarchical-factory-planning.md). Execution plan: [First rocket module chain](../plans/2026-09-11-first-rocket-module-chain.md).

## 1. Outcome and inherited boundaries

On seed 31337, Factorio Space Age 2.1.17, four character bots and default freeplay inventory, construct and feed a rocket silo, launch one space-platform starter pack from Nauvis, and observe the resulting platform. The target is fewer than 864,000 elapsed game ticks (four hours). Launch and platform creation are separately recorded; both are required for this payload-specific terminal condition. This is not Space Age completion or sustained space-science production.

The primary trial uses default map-generation and enemy settings, explored-only observations, and game speed 10. A peaceful trial is a separate diagnostic condition, never substituted for primary success. Record actual initial inventory, settings, mod versions, map/save hash, source/library hashes, roster, observation rules and pause policy. The installed set is base, space-age, elevated-rails, quality, recycler and BotBridge; resolve exact versions from the runtime. Default inventory is measured per bot, not assumed to be one shared inventory or silently multiplied after setup.

The parent remains authoritative for pure module generation, conditional operating contracts, deterministic work budgets, candidate-local state, persistent identity, and independent observations. Its starter comparison remains a separate experiment. This continuation neither claims its acceptance gates have passed nor reruns its ten-seed matrix as a prerequisite for every edit.

Global constraints, also copied into the plan:

- Module extraction and planning remain pure, deterministic, synchronous, and free of I/O.
- One shared PlanControl covers selection, routing, scheduling, correction and recovery; nested work cannot reset it.
- Structural capacity, predicted supported capacity, and observed production are separate facts.
- Preserve existing public Goal semantics and the Legacy default; the launch policy explicitly selects Modules.
- Legacy acquisition remains available for supported construction bills; complex acquisition becomes explicit finite machine-production work.
- No input, machine, fuel, power, transport capacity, research unlock or exploration is free.
- Preserve default-enemy primary trials; peaceful and fully-charted diagnostics have distinct manifests.
- No global solver, learned library, interplanetary factory, train system, or general combat planner is introduced.

The work is a dependency-ordered extension: repair inherited contracts; connect and commission a small chain; add the finite launch policy; then benchmark. Each checkpoint is independently reviewable.

## 2. Evidence and source reconciliation

### 2.1 What the checkout actually does

| Location | Observed gap | Required continuation |
|---|---|---|
| `modules/select.rs` | Enumerates all families, then sorts by enum order | Explicit compatibility filter and deterministic complete-alternative ranking |
| `modules/compile.rs` | Compiles several candidate designs for the same request | Evaluate alternatives transactionally; commit only the winning selection |
| `production_item_from_goal` | No recursive All; Have becomes rate 1; Produced count becomes rate | Preserve quantities, rates, holder, recipe and unlock metadata |
| `site_candidates` | Checks original world, six-tile anchor grid, instance ID zero | Shared reservations, resource-aware placement, correct transforms and unique IDs |
| `modules/families.rs` | Steel/brick use plate ratios; fluid ports carry “pipe”; category guessed from names | Prototype-derived rates and real material ports, including machine category compatibility |
| `modules/ledger.rs` | Flow admission accepts partial overlap of source availability | Require full interval containment and sweep all stock commitments transactionally |
| `PlannerSession`, `ReplanMemory` | Session lacks instance memory; returned memory does not carry module intent | Persist selected instance IDs, bindings, operating commitments and reconciliation |
| Flat schedule fallback | Assigns earliest-free bot without full temporal/ownership validation | Respect predecessor lags, pinned actors, chain owners, travel and action preconditions |
| Supervisor | Has list/chart/witness/sustain mechanisms, but no first-launch source | Add policy and Limiter on these mechanisms rather than another executor |

These are source observations, not fresh live failures. A generated schedule or an action-count measurement is not evidence that a placed chemical plant, silo or starter-pack cell runs.

### 2.2 Workspace speedrun material

Read the existing [world-record replay analysis](../notes/2026-09-04-world-record-replays.md), inspect the supplied ZIP entries and archive hashes, and read the text of `workspace/Factorio speedrun Guide.pdf`. The two large `workspace/wrload/scripts/wr-census*.json` files are final-world dumps; their leading metadata includes advanced research and modified force bonuses, so they cannot be used as startup fixtures.

| Artifact | SHA-256 | Evidence status |
|---|---|---|
| `workspace/any-wr-6-39-53.zip` | `5d264c768420572669c13aa2ac4ea1c02a22c6411f2aa67f40433ee52675d6b3` | Archive inspected; previous note reports 2.0.66, seed 1856629886 |
| `workspace/RSNG_3_17_06.zip` | `c0f6572fd2fd7e91b2e4752f13cdc830a7092553386d6da8d9708258c166d47b` | Archive inspected; previous note reports 2.0.77, seed 3278808720 |

No new replay execution, first-rocket timestamp extraction, or certification of record category was performed. The prior note's two-minute production bins are secondary local evidence from final-save statistics, not decoded player inputs. Its original scratch extraction apparatus was not retained, so those values are hypothesis-generating, not new reproducible benchmark results.

Transfer these lessons: invest in mining before optimizing automation time; use observed rock yields for early stone/coal; expand iron before copper; maintain substantial fuel supply; establish upstream capacity with each downstream build; cap construction-stock buffers. The PDF explicitly includes an older rocket-control-unit/purple/yellow-science path and altered map settings. Do not copy its recipe list, resource settings, human timings, or blueprint placement privileges.

This supersedes the earlier chat suggestion of four iron cells as a permanent bootstrap cap. Initial batches may contain four cells, but the policy evaluates further investment rather than freezing production at that scale.

### 2.3 Local 2.1.17 facts

Use `workspace/data/base/prototypes/{recipe,technology}.lua`, `workspace/data/space-age/{base-data-updates.lua,prototypes/recipe.lua,prototypes/technology.lua}` and `workspace/factorio-api-docs/runtime-api.json` as inspection sources. The runtime API file identifies 2.1.17. Final exported runtime prototypes remain authoritative because data-stage modifications can change base definitions.

- Space Age overrides rocket parts to 50 and each part to one processing unit, LDS and rocket fuel.
- The starter pack costs 60 foundation, 20 steel and 20 processing units. Each foundation costs 20 steel and 20 cable in this install. Payload therefore requires 1,220 steel, 1,200 cable and 20 processing units before its upstream inputs are expanded.
- Silo research unlocks starter pack and foundation. `space-platform` is triggered by platform creation and must not gate their manufacture.
- Silo research requires concrete, rocket fuel, processing unit, logistic robotics, LDS and advanced material processing 2. The logistic-robotics closure includes battery, lubricant, electric-engine and robotics research. Researching these does not by itself require constructing logistic robots or replacing the four character bots.
- Steel furnaces require advanced material processing; electric furnaces require advanced material processing 2. Research the latter for the silo but keep steel furnaces unless a measured benefit justifies replacement.
- Processing units require a compatible fluid-capable assembler; chemical plants cannot be selected merely because a recipe has fluid ingredients. Rocket fuel needs its actual fluid inputs too.

Pin a small final-runtime fixture of the entire selected recipe/technology closure, machine categories (including category lists), collision boxes, mining/fuel values, fluid boxes, belt/inserter limits and rocket state fields. A missing field is an unsupported capability, not an invented constant.

## 3. Selection and growth policy

Retain the module families and add typed machine/row variants, not a replacement planner. Artifact identity includes schema, generator version, normalized parameters, relevant prototype hash and mod versions. Bump schemas for incompatible port/parameter changes; explicitly reject incompatible stored designs while preserving unrelated old replan records.

Extraction describes designs independent of research progress. Selection checks unlocked recipes, recipe-machine compatibility, surface, finite construction dependencies, supply and site feasibility every time, including cache hits. A primitive remains a valid design above one-copy output: capacity rejection belongs to the policy's copy budget, not `Unsupported` in extraction.

Use a small portfolio (initial cap: eight complete alternatives) with stable enumeration. For each alternative reconcile standing/in-flight capacity, calculate the missing supported rate, choose copy counts, site and route, acquire construction stock, schedule and validate obligations. Rank valid alternatives lexicographically by predicted milestone completion tick, incremental construction bot-ticks, material expenditure using a manifest-fixed item order, then design IDs and anchors. Shortlisting can use coarse integer estimates, but only complete supported schedules are incumbents. Score finite batches by quantity completion, not perpetual throughput.

Rates use checked rational arithmetic and the requested product, not the largest output in a multi-output design. Plate rows use actual recipe time and furnace speed: 24 steel furnaces nominally give 900 iron/copper plates per minute, but only 180 steel/minute with 900 iron/minute input. Stone brick needs two stone per brick. Inserters, belt lanes and mining coverage may reduce delivered capacity.

Initial development policy (hypotheses, recorded as supplied knowledge):

- Bootstrap iron in total-count stages 4, 8, 12; copper 2, 4, 6 only as demand appears. Coal and stone get burner extraction variants; rocks bridge startup and do not count as recurring mining.
- Authorize the next stage only if it fits observed ore, construction reserves and five minutes of supported fuel/haul work. After 12 iron cells, compare another bounded batch against available electric alternatives; do not blindly continue burner growth.
- Electric drill variants use 6/12/30 drills; smelting variants use 6/12/24 steel furnaces or appropriately sized stone-furnace rows. A six-machine section is a valid commissioning boundary.
- Keep productive old cells. Prefer extra yellow lanes to a red-belt migration until route capacity is a measured bottleneck. Electric furnaces and assembler 3 are optional construction, not prerequisites inferred from family names.
- Limit construction batches to one array section or four small cells, with up to four batches in flight only when their material and bot ownership is disjoint. Candidate count, copy count and simultaneous construction are different limits.

## 4. Combined goals, quantities and material flow

Do not silently redefine `Goal::Producing`: it remains the existing structural gross-rate predicate. `Sustain` and commissioning observations establish operation over finite windows. `Have` requires the specified holder to possess the quantity; `Produced` retains cumulative production and recipe/trigger metadata. Repeated identical constraints in All merge by maximum, not sum, since conjunction is idempotent. Distinct holders, recipes and predicates remain distinct. Mixed research/production/stock children cannot be discarded or marked complete by the supported subset.

The launch policy additionally declares explicit external output reservations in its plan options. These do not change public Goal serialization. Given `All(iron:60, copper:60, gears:30)` with no external iron reservation, 60 iron/minute can feed all 30 gears; the iron predicate measures gross output. With a separate 60 iron/minute export reservation, the plan needs 120 iron/minute. This explicitly revises the earlier chat proposal to make every Producing request a net export.

Build a demand DAG using a frozen Nauvis recipe preference map over unlocked compatible recipes. Honor an explicit `via` selection. Recipe name and product name are separate: basic-oil-processing is a recipe producing petroleum gas. Exclude recycling and off-planet alternatives from this launch policy. Reject unsupported cycles with their recipe path; do not recurse indefinitely.

For each item, gross required output is at least the maximum of its explicit production floor and the sum of allocated internal consumers plus external exports. Finite demands are separate time-stamped stock claims. Expand final-product amounts and all coproduct amounts correctly. Preserve existing source/route allocations, use only unallocated supply, and reserve mine yield, belt lanes, pipe networks, electricity and fuel over explicit intervals. Stock covers only a finite interval. A planned provider becomes available after construction, commissioning, production latency and transport delay.

Extend ports with BeltOutput, FluidInput, FluidOutput and InventoryInput alongside existing modes. Each names the transported substance, actual rotated connection endpoint, lane/fluid-box identity, direction, and conditional capacity. Rocket progress is a machine state observation, not an inventory-output port for taking rocket parts.

Choose deterministic corridors and direct feeds where possible. Material flow is established only after compiling actual belts, pipes, inserters or priced bot transfers. Copper cable can use a short local feed rather than occupying a long shared bus. For oil, explicitly select basic processing for bootstrap, then advanced processing with heavy/light/petroleum outputs and cracking/storage obligations. Reserve sulfur for blue science before expanding acid production. No fluid dumping or unlimited hidden storage is permitted.

## 5. Siting, compilation, operation and recovery

A candidate owns one reservation overlay for all modules, routes, access corridors and power. Rotate parts, footprints, ports, clearance and pitch together in integer half-tiles. Check full footprints, resource extraction areas, fluid connection positions, charted terrain and existing threat exclusions. Six-tile generic anchor spacing is not a clearance guarantee. Routes may reject a site and selection may try another under the same budget.

Compile connection work and construction bills through existing primitive actions. Keep Goal::Have fallback for supported simple construction acquisition. Replace the current skip list's implicit assumption with explicit readiness obligations: a skipped chemical plant/refinery/silo must be present and assigned to its builder, or the supervisor must schedule a finite batch in an already available compatible machine. A machine cannot be the sole producer of its own first construction item. Handcraftable bootstrap machines may still use existing legitimate handcraft acquisition. No legacy expansion of belt-input ingredient trees is reintroduced.

Finite construction buffers reserve the next two batches plus declared defense/recovery stock; they stop producing when funded. Reuse batch assemblers with explicit recipe changes and removal of incompatible leftovers. Silo and starter pack are finite quantity objectives, not reasons to replicate expensive machines to meet arbitrary rates.

Call ensure_powered for electric demand, reusing existing networks and reserving aggregate capacity plus 20 percent policy headroom. Supply the union of selected footprints/routes and power reservations. Represent boiler fuel, water, steam connectivity and recurring delivery. A fuel buffer's support ends at its calculated depletion tick.

Keep the flat fallback but move its validation into a focused helper. Respect predecessor end plus lag, actor/chain ownership, pinned actors, travel, inventory transfers and mutual exclusion. Run network and state-precondition validation on the result. Charge fallback work to the same control; cancellation or exhausted control cannot open an unbounded fallback. If a valid schedule cannot be constructed, report a typed failure with no incumbent.

Persist instances, bindings, action provenance, support horizon, module budgets and observation windows through ReplanMemory and Lua savepoints. Allocate nonzero monotonic IDs with checked overflow. Reconcile before planning more copies; unknown observations remain unknown. Failed candidates cannot commit identity or reservations. Dispatched actions stay fixed and completed effects cannot be replayed on resume.

## 6. Supervisor, Limiter and milestones

Add `scripts/rocket_policy.lua` as a pure policy module and `scripts/rocket_speedrun.lua` as the driver over the existing supervisor. The policy consumes a normalized observation snapshot, persistent policy memory and frozen configuration; it returns a prerequisite, construction, support, research, commissioning, launch or wait decision. The driver is the I/O boundary.

Limiter states are building, commissioning, complete and blocked. Per milestone persist absolute planned/constructed/commissioned IDs, target quantity or capacity, maximum new copies, construction stock, deadline, and last useful progress tick. Count physical instances, not calls to the planner. Reaching N copies stops expansion and starts commissioning; it does not imply Producing or Sustain succeeded. Missing supply returns a repair/prerequisite decision; it does not allocate another consumer. A capped bootstrap submilestone may finish and proceed to the next investment decision without claiming the full future production target is achieved.

Commission each section with three consecutive 3,600-tick windows at its declared operating target, allowing explicit warmup before the first window. Attribute machine production and delivery at actual sinks; whole-force production alone cannot prove a particular connection. An inventory delta is insufficient when another consumer drains it. Preserve existing direct-lab observation and add material-flow evidence where necessary. Finite batches use observed produced counts and holder inventory rather than rate windows. Periodic monitoring checks fuel support before expiry and reopens a failed supply obligation without duplicating commissioned modules.

The following deadlines are initial development budgets, not claimed results. Production continues across stages. Waiting for research must not idle the supervisor if a funded independent construction/resupply batch is ready.

| Stage | Checkpoint (game minutes) | Required result |
|---|---:|---|
| 1. Burner investment | 20 | Staged iron growth; coal/stone extraction and rock bootstrap; copper when needed; funded recurring fuel |
| 2. Power and red science | 35 | Legitimate trigger unlocks, cables/circuits, steam through ensure_powered, labs, 30 red/min target |
| 3. Green science and construction stock | 55 | Belts/inserters/gears, 30 green/min, electric mining/automation 2 prerequisites, next-batch machinery |
| 4. Metals and access | 80 | Electric mining and steel rows; initial 900 iron/min and 450 copper/min, additional iron for steel; chart and protect oil route |
| 5. Oil bootstrap | 100 | Pumpjack/refinery/chemical-plant construction funded; petroleum, plastic and sulfur physically connected |
| 6. Blue science | 125 | Engines/advanced circuits/sulfur and 30 blue/min minimum; red/green remain fed; target 45/min if costed capacity permits |
| 7. Research and finite reserves | 190 | Entire silo research closure; advanced oil/cracking, acid, processing units, LDS, rocket fuel, concrete and construction steel |
| 8. Rocket silo | 205 | One built and fed silo; actual internal progress reaches required parts |
| 9. Starter pack | 215 | 60 foundation and one starter pack made through unlocked compatible machines; stock budget includes payload steel |
| 10. Launch | 225 | Platform request, payload delivery, observed launch and platform establishment; 15-minute recovery margin |

The research closure in local source requires roughly 2,650 blue packs (including 1,000 for silo) before optional research. Verify the exact sum against runtime export. At 30 blue/min this alone needs about 88.4 minutes of pack supply; therefore a stage-6 completion at minute 125 is late for the minute-190 research checkpoint unless science was already accumulating or capacity grows. The policy computes remaining science/time at every checkpoint and raises the funded science target toward 45/min or reports an impossible checkpoint; it cannot assert the table is feasible by construction. Labs are sized from remaining research unit time and observed lab speed as well as pack rate.

Research priority is a topological queue with milestone tie-breaks: bootstrap triggers/automation/electric mining/logistics; steel and green science; advanced material processing/automation 2/engine/fluid handling/oil gathering; oil-processing trigger/plastics/advanced circuit/sulfur/chemical science; advanced oil, rocket fuel, processing unit, LDS, concrete, battery/lubricant/electric engine/robotics/logistic robotics, advanced material processing 2, rocket silo. Actual prerequisites always outrank this preferred order. Oil processing's extraction trigger is observed, never granted by a goal label.

## 7. Default enemies and oil corridor

The previous oil run recorded bot deaths; default-enemy execution is therefore an explicit capability gate. Reuse existing threat-aware source selection and path checks, not the older note's claim that no threat modeling exists. Stage 4 surveys via supervisor.chart_until and reserves an observed route to oil, avoiding enemy standoffs. Unknown terrain prompts survey; it is not safe by default.

Add bounded static defense around base chokepoints, power and the oil work area using legitimately acquired gun turrets, ammunition, repair materials and replenishment work. Defense cost and travel compete with factory investments. This project does not clear nests or solve general combat: if no safe reachable oil route exists under bounded survey/avoidance, or static defenses cannot preserve access, report `unsafe_oil_corridor`/`defense_insufficient` and a failed primary trial. Do not change enemy settings, teleport, replace lost bots for free, or silently choose a peaceful seed. A primary success claim requires live evidence of this gate, not merely peaceful factory operation.

## 8. Launch control and evidence

Use the installed runtime API at the adapter boundary. LuaForce.create_space_platform takes name, planet and starter_pack and returns an optional platform; verify in an isolated fixture that it creates the unbuilt logistics request rather than granting the terminal world state. Request creation is idempotent by persisted platform identity. If that operation cannot model the player UI request legitimately, return unsupported rather than manufacturing a completed platform.

Supply one actual starter pack to the correct silo inventory, verify the silo has the required internal parts, and order launch using supported destination semantics. A true launch_rocket return value/on_rocket_launch_ordered is initiation evidence only. Record actual on_rocket_launched and platform establishment with force, silo, platform, payload and tick identities. An existing unrelated rocket launch cannot satisfy the run. Retries after an ambiguous response first query persisted request and launch evidence.

No general new public Goal variant is necessary: these are named supervisor actions and terminal observations, exposed through a narrow BotBridge/Rust adapter with documented Lua access. Construction/movement/material actions remain in the normal executor. Register event observers without replacing existing handlers.

## 9. Acceptance and measurements

Gate A: runtime closure/contract validation, research-filtered alternatives, correct mixed All/quantity handling, fully contained ledger claims, safe fallback and resume tests pass.

Gate B: two connected modules then a shared iron/copper/gears chain demonstrate full reservations, correct material accounting and three observed windows without unpriced feeds. Include fuel exhaustion and broken route recovery.

Gate C: petroleum through blue science demonstrates physical fluid connections, coproduct drainage, research consumption and finite construction stock. Include a paused/resumed partially built module and no duplicate ID/cost.

Gate D: starter pack consumes real inputs and a real rocket creates its requested platform, with terminal evidence. Isolated preloaded fixtures can validate mechanics but are labeled fixtures, not speedruns.

Gate E: execute one peaceful diagnostic and then three primary seed-31337 trials from the same pristine save/settings with policy frozen between primary trials. Target all three below 864,000 ticks; retain failures/timeouts and report success fraction. Timing changes between attempts require a new policy hash and a new declared cohort. This is seed-specific engineering evidence, not held-out transfer or a world-record claim.

Use release builds and an owned isolated workspace. The runner records elapsed simulation ticks including build, travel, research, warmup and observation. Planning pauses are explicit and their wall duration is reported separately; four hours at 10x is approximately 24 minutes only while the server sustains the requested simulation speed. Record per-stage ticks, production/supply rates, lab utilization, construction expenditure, bot travel/idle/resupply, death/damage, planning work/time, retries and terminal evidence. Existing offline reports are not live successes.

## 10. Review status

This spec supersedes only the parent project's deferred launch scope and its enum-order selection policy. It strengthens implementation conformance to existing ledgers and identity contracts. It corrects the earlier chat's primitive cap, net-Producing suggestion and omitted mandatory research branches. The implementation plan supplies ordered checkpoints, concrete file ownership, contract tests and execution commands. No four-hour result is asserted by either document.
