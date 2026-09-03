# The starter factory: machines produce, bots build

**Status: ~85% IMPLEMENTED — corrected 2026-09-03.** This line read *"design only. No implementation, no source edits"* over shipped code. **Stage 1 (burner smelting cell) is done and witnessed** — run 37: *"iron-plate in 1 watched machine(s) went 0 -> 1 … in 480 of 2400 ticks"*, against a predicted 432. **Stage 2 (powered red-science cell) is built and its recipes are set, but has never produced a pack** — run 10 settled all nine actions `success`, including both `set_recipe` calls, then died in the smelting work that charges the chests. **Stage 3 (green science) is unreachable by the shipped cell shape**, not by the ratio solver: `assembly_spec` (`crates/planner/src/method/assemble.rs:258-296`) requires a 2-ingredient recipe with exactly one single-ingredient intermediate, and `logistic-science-pack` has neither.

**Read before implementing anything here — the document contradicts itself and the code has already broken the tie:** §8.4 anchors stage 2's origin to the supplying pole; §14, §7 and the §6.1 table describe three furnaces and **two drills** at fixed offsets from that origin, which would put a drill on ore beside the water the power plant is sited on. **§8.4 won:** `BuildAssemblyCell::expand` (`assemble.rs:1359`) sites from `nearest_supply_anchor` (`crates/planner/src/state.rs:2066`), and the drills and furnaces were deleted from the design — their job replaced by bots hand-charging two chests for `CELL_CHARGE_TICKS = 9_000` ticks (`assemble.rs:112`). §14's part list describes the version that lost. See `docs/superpowers/notes/2026-09-03-red-science-layout.md:166-173`.

**Two arithmetic errors, both load-bearing downstream:** §6.1's ≈621 kW stage-2 figure is wrong for what was built — `cell_demand_kw` (`assemble.rs:659-664`) is `2×75 + 3×13` = **189 kW**, 249 kW with the lab, so **28% load on 900 kW, not 69%** — and every number in §6.2 and §7 is scaled off the phantom. §7's fuel-buffer arithmetic (one coal stack ≈ 5.4 minutes) follows from it; at the real 249 kW it is ~13 minutes. §7's `iron-chest` + `burner-inserter` fuel buffer was **not built** — what shipped is `boiler_coal()` (`assemble.rs:890-901`), a one-shot bot insert capped at `COAL_STACK = 50`, so the cell still needs a bot visit.

The original status line read: *design only. No implementation, no source edits, and no `cargo` invocation at all this pass* — another agent is building the power plant in
this same checkout and load has been hitting 12–35
(`docs/superpowers/notes/2026-09-02-runs-contend-with-agents.md`). Everything
below is read off source, off `workspace/runs/*/`, and off the shipped
prototypes in `workspace/data/`. §15 says exactly which claims a run has to
settle.

**Date:** 2026-09-03 (written late on 2026-09-02).

**Reading this against a moving tree.** `crates/planner/src/method/power.rs`
and `docs/superpowers/notes/2026-09-02-the-power-plant.md` were **untracked**
while this was written and landed as `39cf19f7` ("build the power plant a
research needs") before it was committed, so this spec sits directly on top of
them. Two consequences: where the text below calls the plant "in flight" it is
now on `master`, and — the part that has *not* changed — **the plant has still
never been built in a running game.** Every claim here about it is a claim
about a planner that produces a 171-action network, not about observed
behaviour. §15 keeps that distinction.

---

## 1. Recommendation in three sentences

Implement `Goal::Producing` in three stages whose first is **nine iron plates
and ten stone**: a burner mining drill dropping straight into a stone furnace,
which needs no research, no electricity, no inserter, no belt and no wood, and
which is the first machine-to-machine link this project would ever have built
(all 329 entities ever placed across 21 archived runs are stone furnaces). The
satisfaction question the brief calls hard splits cleanly in two and the split
is the whole design: **the planner answers a structural question** — do the
machines stand, face the right way, hold the right recipe, and sit on a network
whose *uncommitted* capacity covers their draw — while **the supervisor answers
the durative one**, by running a milestone that dispatches no actions at all
and asserting that a terminal machine's output inventory rose anyway. Neither
half is sufficient and the project has already paid for both failures
separately: run 30 shows a structural claim passing on a base with 0 kW in all
541 force samples, and `supervisor.lua`'s `plan_empty` branch **closes a
milestone as `satisfied` when `goal.holds` returns `nil`** — which is exactly
what `Goal::Producing` returns today, so shipping a `Producing` method without
also fixing `holds` would report a factory built on nothing.

---

## 2. The rate-vs-quantity framing, checked

The brief asked me to verify "every goal this planner has ever had is a
quantity" against source rather than take it. The coordinator's mid-task
correction is right that `Goal::Producing` exists. Both the original framing
and the correction are half right, and the half each gets wrong is where the
work is.

### 2.1 What exists, exactly

`crates/planner/src/goal.rs:100-115`:

```rust
pub enum Goal {
    Have { item: ItemId, count: u32, whose: Holder },
    Researched(String),
    Produced { item: ItemId, count: u32, whose: Holder, unlocks: Option<String> },
    /// The functorio bridge: `BusLane item rate` transcribed into Rust. No
    /// method satisfies this yet; blueprint generation is a later increment.
    Producing { item: ItemId, rate: f64 },
    All(Vec<Goal>),
}
```

So the *name* is there, with a doc comment that says it is unimplemented, and
a design (`docs/superpowers/specs/2026-09-01-producing-goal-design.md`, "Status:
specified, not implemented"). What is there is a **declared placeholder plus
three tests pinning that nothing claims it**:

* `method/have.rs:212` — `holds` returns `None` for it.
* `method/have.rs:3125` — a fixture literally named `unanswerable`.
* `method/mod.rs:1404` — a `Producing` goal against a registry that expands
  everything else still yields `NoApplicableMethod`.
* `crates/executor/src/recover.rs:499` — `recover`'s tier-2 test constructs a
  `Producing` goal *precisely because* nothing can decompose it.

### 2.2 Where the brief's framing holds

Everything *below* the goal is a quantity or a one-shot, and I checked each:

* **`Action` always terminates.** `action.rs:429` — `duration: Ticks`, no
  variant for an open-ended one. Six `ActionKind`s: `Mine`, `Craft`, `Place`,
  `Insert`, `Remove`, `Research`. No rotate, no set-recipe, no wire-connect,
  no blueprint.
* **`Schedule` is finite.** `schedule.rs:125` — `Vec<ScheduledStep>` plus a
  `makespan`; `schedule()` loops `while done.len() < net.len()`. `StepKind`
  is `Act | Walk`. There is no "and then it keeps going".
* **`PlanState` models presence, never process.** `state.rs:360` carries bot
  inventories, an entity overlay (`added` / `removed`, keyed by floored `Pos`),
  consumed resources, mining claims, a research overlay, characters and refused
  footprints. An `added` entry is a bare `FactorioEntity`; the type *has*
  `output_inventory`, `fuel_inventory` and `recipe: Option<String>`
  (`crates/core/src/types.rs:1155-1162`) and **`crates/planner` reads none of
  the three**. A furnace "smelting" is modelled entirely as a `Step::Link` lag
  edge and nothing else.
* **No `Effect` models contents.** `GainItem`, `LoseItem`, `CreateEntity`,
  `RemoveEntity`, `ConsumeResource`, `Researched`. That is the whole list.

### 2.3 Where the framing is wrong, and it is the useful part

**`Condition::Powered` is already a standing structural predicate, and it is
exactly the shape a rate goal needs.** `action.rs:109`:

```rust
Powered { pos: Position, entity: ItemId, kw: f64 },
```

Its own doc: *"Nothing produces this yet, so `ActionNetwork::infer_edges` draws
no edge to it and no method can satisfy it: it is a statement about the world,
checked at expansion time."* It becomes true not because an action satisfies it
but because `Effect::CreateEntity` landed a generator and a pole in the overlay
that `PlanState::electric_supply_kw` reads.

That is the entire mechanism a starter factory wants, one level up: *a claim
about a standing arrangement, discharged by placements, checked by a model.*
The design below does not invent a parallel notion of standing capability. It
adds two siblings to `Powered` and reuses the machinery around it.

So the correct statement of the problem is **not** "the planner cannot express
a standing capability". It is:

> The planner can express a *structural* claim about a standing arrangement and
> already does. What it cannot express — and must not pretend to — is that the
> arrangement is **actually producing**. `Powered` counts nameplate capacity, so
> a boiler with an empty fuel slot reads as 900 kW. That is the honest boundary,
> and the design has to put the observation on the other side of it rather than
> paper over it.

### 2.4 The live hazard this creates, today, in `master`

`scripts/supervisor.lua:280-322` handles an empty plan in three branches:

| `goal.holds` | supervisor |
|---|---|
| `true` | `satisfied`, reason `already_satisfied` |
| `nil` | **`satisfied`**, reason `plan_empty` |
| `false` | raises — "refusing to report it satisfied" |

`holds(Producing)` is `nil` (`have.rs:212`). So the moment a `Producing` method
exists and returns an empty step list for **any** reason — a site it thinks is
already built, a shortfall it computed as zero, a refusal it swallowed — the
milestone closes as satisfied with no evidence whatsoever. This is not
hypothetical drift; it is the wiring as it stands.

**D0: `holds(Producing)` must stop returning `None` in the same change that
gives `Producing` a method.** Landing the method first, "and the predicate
after", is the one ordering that manufactures a silent success. §9.4 says what
it returns instead.

---

## 3. Is the 2026-09-01 `Producing` spec still right?

Mostly, and it deserves credit for D3. Four amendments, three of them because
things landed since.

### 3.1 What stands

**D3 — "a sparse cell grid, not a layout solver."** Right then, and *more*
right now, because it has acquired a precedent in this repo:
`crates/planner/src/method/power.rs` builds a six-building plant as a rigid
body of derived relative offsets, rotated about a tile centre, sited by one
search, with a test asserting every building lands on its own build grid at all
four facings. §8 argues from that precedent rather than from the spike.

The three facts D3 says the layout code must encode rather than rediscover —
inserter direction points at the pickup side, coverage is not capacity,
positions follow from the tile-rect centre — all survive. §9.2 turns the first
into a modelled invariant instead of a comment, and §6.3 shows that the second
one has a second, sharper instance nobody has hit yet.

### 3.2 Amendment 1 — D4 names the wrong goal

D4 says: *"Each entity the layout calls for becomes a `Goal::Produced { item,
count }` subgoal."* That is wrong, and the `Produced` spec itself says why.
`Produced` **deliberately ignores inventory** — that is its entire reason to
exist (`2026-09-01-produced-goal-design.md`, D1: *"`Produced` never subtracts
what a bot already holds — that is the entire difference"*). A cell asking for
three assembling machines with `Produced` re-crafts three machines a bot is
already carrying, every replan.

`power.rs::bill()` already does the right thing and is the precedent to copy:

```rust
fn bill() -> Vec<(&'static str, u32)> {
    vec![(PUMP, 1), (PIPE, PIPE_COUNT), (BOILER, 1), (ENGINE, 1), (POLE, 1), ("coal", PLANT_COAL)]
}
```
each emitted as `Step::Subgoal(Goal::Have { item, count, whose: Holder::Share(ctx.chain_actor) })`.

**D4′: a cell's bill is `Goal::Have`, not `Goal::Produced`.** `Produced` is for
trigger events — "cause a craft to happen so the game fires
`research_trigger`" — and a factory's bill is not an event.

### 3.3 Amendment 2 — D1 and D2 belong to stage 3, not stage 1

D1 (solve `Ax = b` over a recipes-by-items matrix, à la Kirk McDonald) and D2
(exact rationals, never `f64`) are correct *for the general case*. They are also
the two most expensive items in the spec, and stages 1 and 2 need neither: a
cell with one machine per step and no recipe consuming its own output has a
machine count of `ceil(target_rate / per_machine_rate)`, which is one division.

But D2's *concern* is real and lands earlier than D2 itself. `rate: f64` makes
that one division the whole expansion's hinge, and a rate that is exactly one
machine's output — 6.0 red packs/min against an AM1's 6.0 — is a ceil sitting
on a representation boundary. `crates/planner`'s defining constraint is
byte-identical plans for identical inputs.

**D2′: change `Goal::Producing { item, rate: f64 }` to
`Goal::Producing { item, per_minute: u32 }` now, and defer the rational
arithmetic to stage 3 where the matrix needs it.** Integer numerator, integer
per-machine rate expressed as a rational constant, integer ceiling; no float
touches the machine count. This is a change to a variant **no method claims, no
Lua surface parses** (`crates/scripting_lua/src/globals/goal/value.rs:102-124`
handles `have`, `researched`, `all` and nothing else) and whose only two
non-test references are its own `Display` arm and `recover.rs:499`, where it is
used as a deliberately unsatisfiable goal. It costs approximately nothing today
and is unpayable later.

### 3.4 Amendment 3 — what has landed since 2026-09-01 that `Producing` needs

The 2026-09-01 spec could not have used any of these. All of them are load-bearing:

| Landed | Why `Producing` needs it |
|---|---|
| `Condition::AreaFree { direction }`, `collision_area_facing`, `rotated_collision_box` (`method/util.rs:414`) — **in flight with `power.rs`** | Every machine in a cell that faces anything. Before this, an `AreaFree` check on a rotated entity asked about ground the building does not stand on. This is the first planner code ever to emit a non-zero `direction`. |
| `PlanState::electric_supply_kw` three-step model + `nearest_supply_anchor` (`state.rs:1436, 1530`) | The only honest answer to "will this machine run". Coverage → connectivity (union-find over wire reach) → capacity. |
| The plant itself (`method/power.rs`, untracked) | 900 kW for ~45 iron plates. Without it a `Producing` cell for anything electric is unreachable, exactly as rung 7 was. |
| `Step::Owned { whose, steps }` (`method/mod.rs:37`) | The only way a method emits an action for a bot other than the chain's. A multi-bot cell build wants it; stage 1 and 2 do not, and say so. |
| Water is solid + `nearest_water_tile` / `tiles_within` / `is_water_at` (`fa8dabf3`, `9ca7229a`) | Before this `is_area_clear` would have approved a furnace standing in a lake. It also costs: ~410,063 additional blocking boxes on a charted map. §8.4. |
| `BOT_FORCE = "player"` by name (`state.rs:64`) | `PlanState::from_world` used to pick the force by `forces.keys().min()`, i.e. **`enemy`**, which reads every recipe as locked. Recipe gating is most of §5. |
| Placement pre-check `rcon_can_place_entities` + refusal ledger (`goal/plan.rs:490`, `MAX_RESITE_ROUNDS = 2`) | This is what makes stamping a multi-entity cell affordable: **one** batched RCON round trip validates the whole cell against the real game before any bot walks. §8.4. |
| `ClaimRunner::{Bot, Chain}` time-aware mining claims (`2026-09-02-time-aware-mining-claims.md`) | A drill occupies ore tiles the `Mine` method also wants; without runner-aware claims a cell and its own supply chain crowd each other out. |

### 3.5 Amendment 4 — the "known risk" is now half-answered

The 2026-09-01 spec's closing risk was `tried to remove 10 copper-plate but
removed 9` — smelt-time estimates drifting. `smelting_ticks` plus a headroom
cycle and runner-aware claims have improved it; it is not closed, and a factory
multiplies these couplings. §13 keeps it as an open failure mode rather than
declaring it fixed.

---

## 4. The production graph, from the game data

All figures read from `workspace/data/base/prototypes/` (base 2.1.17, mods
`base`/`elevated-rails`/`quality`/`space-age` per `workspace/mods/mod-list.json`)
and cross-checked against the pristine live capture
`crates/core/tests/live-2.1.17-world-snapshot.json` (0 of 277 technologies
researched, so its `enabled` flags *are* the answer to "available with no
research"). The two agree on every value below.

**Do not use `crates/core/tests/recipes-fixtures.json` for any of this.** It is
a Factorio 1.1 capture from June 2022; `crates/core/tests/README.md` lists four
defects it has already caused, and it marks the plant recipes `enabled: true`
when live marks them locked. A method that passes against `fixture_world` can
still refuse against a real game. Test against both.

### 4.1 Recipes

| Recipe | s | Ingredients | Yield | Start? |
| --- | --- | --- | --- | --- |
| `iron-plate` (smelting) | 3.2 | 1 iron-ore | 1 | **yes** |
| `copper-plate` (smelting) | 3.2 | 1 copper-ore | 1 | **yes** |
| `iron-gear-wheel` | 0.5 | 2 iron-plate | 1 | **yes** |
| `transport-belt` | 0.5 | 1 iron-plate, 1 iron-gear-wheel | **2** | **yes** |
| `stone-furnace` | 0.5 | 5 stone | 1 | **yes** |
| `burner-mining-drill` | 2 | 3 iron-plate, 3 iron-gear-wheel, 1 stone-furnace | 1 | **yes** |
| `burner-inserter` | 0.5 | 1 iron-plate, 1 iron-gear-wheel | 1 | **yes** |
| `iron-chest` | 0.5 | 8 iron-plate | 1 | **yes** |
| `copper-cable` | 0.5 | 1 copper-plate | **2** | electronics |
| `electronic-circuit` | 0.5 | 1 iron-plate, 3 copper-cable | 1 | electronics |
| `inserter` | 0.5 | 1 iron-plate, 1 iron-gear-wheel, 1 electronic-circuit | 1 | electronics |
| `small-electric-pole` | 0.5 | **1 wood**, 2 copper-cable | **2** | electronics |
| `lab` | 2 | 10 iron-gear-wheel, 10 electronic-circuit, 4 transport-belt | 1 | electronics |
| `pipe` | 0.5 | 1 iron-plate | 1 | steam-power |
| `automation-science-pack` | **5** | 1 copper-plate, 1 iron-gear-wheel | 1 | automation-science-pack |
| `logistic-science-pack` | **6** | 1 transport-belt, 1 inserter | 1 | logistic-science-pack |
| `assembling-machine-1` | 0.5 | 9 iron-plate, 5 iron-gear-wheel, 3 electronic-circuit | 1 | **automation** |
| `electric-mining-drill` | 2 | 10 iron-plate, 5 iron-gear-wheel, 3 electronic-circuit | 1 | electric-mining-drill |

Three yields of **2** — `transport-belt`, `copper-cable`, `underground-belt` —
are the ones a bill written from memory gets wrong.

### 4.2 Machines

| Entity | tiles | electric | burner | speed |
| --- | --- | --- | --- | --- |
| `stone-furnace` | 2×2 (±0.7) | — | **90 kW**, 1 fuel slot | crafting 1 |
| `burner-mining-drill` | 2×2 (±0.7) | — | **150 kW**, 1 fuel slot | mining 0.25 |
| `electric-mining-drill` | 3×3 (±1.35) | **90 kW** | — | mining 0.5 |
| `assembling-machine-1` | 3×3 (±1.2) | **75 kW** | — | crafting **0.5** |
| `assembling-machine-2` | 3×3 (±1.2) | **150 kW** | — | crafting 0.75 |
| `lab` | 3×3 (±1.2) | **60 kW** | — | researching 1 |
| `inserter` | 1×1 (±0.15) | drain **0.4 kW** + 5 kJ/movement + 5 kJ/rotation | — | — |
| `burner-inserter` | 1×1 (±0.15) | — | 50 kJ/movement, 1 fuel slot | — |
| `boiler` | 3×2 | — | **1.8 MW**, 1 fuel slot | 165 °C steam |
| `steam-engine` | 3×5 | **900 kW out** (derived) | — | — |
| `small-electric-pole` | 1×1 | supply 5×5, wire 7.5 | — | — |

`steam-engine` has **no `max_power_output` field**. 900 kW is derived:
`0.5 fluid/tick × 60 tick/s × (165 − 15) °C × 0.2 kJ × effectivity 1`. This
independently reproduces the constant already hardcoded at `state.rs:134`. The
chain: boiler 1.8 MW ÷ (150 °C × 0.2 kJ) = 60 steam/s; an engine eats 30
steam/s; so **one boiler feeds exactly two engines = 1.8 MW**, and one offshore
pump (1200 water/s) feeds twenty boilers.

Coal is **4 MJ**, stack 50. Wood is 2 MJ. Resource `mining_time` is 1 for
iron-ore, copper-ore, coal and stone alike, so a drill's rate is just its
`mining_speed`: burner 0.25 ore/s = **15/min**, electric 0.5 ore/s = **30/min**.

**The inserter's "~13 kW" is a duty-cycle model, not a prototype field.** The
data gives a 0.4 kW drain and 10 kJ per full swing. §6 uses 13 kW as a
deliberate pessimism and says so; a design that budgets 0.4 kW for a busy
inserter is the same error as counting coverage as capacity.

### 4.3 The chains, ore to pack

```
iron-ore   --[drill]--> (drop) --[stone-furnace 3.2s]--> iron-plate
copper-ore --[drill]--> (drop) --[stone-furnace 3.2s]--> copper-plate

iron-plate x2 --[AM1 0.5s/0.5]--> iron-gear-wheel
copper-plate + iron-gear-wheel --[AM1 5s/0.5]--> automation-science-pack

copper-plate --[AM1]--> copper-cable x2
iron-plate + copper-cable x3 --[AM1]--> electronic-circuit
iron-plate + gear + circuit --[AM1]--> inserter
iron-plate + gear --[AM1]--> transport-belt x2
inserter + transport-belt --[AM1 6s/0.5]--> logistic-science-pack
```

### 4.4 Machine counts at the design target

**The target is set by the research it has to feed, not chosen.** A lab
(`researching_speed 1`) consumes one of each ingredient per the technology's
`time`. The shortest-`time` early technology is `logistic-science-pack` itself
at 5 s, i.e. **12 red packs/min**. That is the design rate.

| Step | Per-machine rate | At 12 red/min | At +12 green/min |
| --- | --- | --- | --- |
| red pack (AM1, 5 s ÷ 0.5) | 6/min | **2** | 2 |
| green pack (AM1, 6 s ÷ 0.5) | 5/min | — | **3** |
| iron-gear-wheel (AM1) | 60/min | **1** (20 %) | 1 (30 %) |
| electronic-circuit (AM1) | 60/min | — | 1 (20 %) |
| copper-cable (AM1) | 120/min | — | 1 (30 %) |
| inserter (AM1) | 60/min | — | 1 (20 %) |
| transport-belt (AM1) | 120/min | — | 1 (10 %) |
| iron-plate (stone furnace) | 18.75/min | **2** (24/min) | **5** (90/min) |
| copper-plate (stone furnace) | 18.75/min | **1** (12/min) | **2** (30/min) |
| iron-ore (electric drill) | 30/min | **1** (80 %) | **3** |
| copper-ore (electric drill) | 30/min | **1** (40 %) | **1** |

Green science costs **5.5 iron plates and 1.5 copper plates per pack** (an
inserter is 4 iron all-in, a belt 1.5); red costs 2 iron and 1 copper. That
factor of ~2.5 is why the two are staged apart and not shipped together.

---

## 5. Research: prerequisites, and whether it is a cycle

**It is not a cycle. Three trigger technologies break it, and one of them the
repo does not yet treat as a trigger.**

The live capture has exactly 32 technologies with zero research energy and no
unit ingredients. **Three are in the early tier**, and every one of them needs
no lab, no power and no science pack:

| Technology | Trigger | Unlocks |
| --- | --- | --- |
| `steam-power` | `craft-item` **iron-plate ×50** | pipe, pipe-to-ground, offshore-pump, **boiler**, **steam-engine** |
| `electronics` | `craft-item` **copper-plate ×10** | copper-cable, electronic-circuit, **lab**, **inserter**, **small-electric-pole** |
| `automation-science-pack` | `craft-item` **lab ×1** | the red-pack recipe |

The repo's existing belief that `steam-power` and `electronics` are triggers is
confirmed. **`automation-science-pack` is a third one and should be treated the
same way.** After it, everything is a lab research:

| Technology | Cost | Prereqs | Unlocks |
| --- | --- | --- | --- |
| `automation` | 10 red × 10 s = **6,000 ticks** | automation-science-pack | **assembling-machine-1**, long-handed-inserter |
| `electric-mining-drill` | 25 red × 10 s | automation-science-pack | electric-mining-drill |
| `logistic-science-pack` | **75 red** × 5 s | automation-science-pack | the green-pack recipe |
| `logistics` | 20 red × 15 s | automation-science-pack | underground-belt, splitter |
| `steel-processing` | 50 red × 5 s | automation-science-pack | steel-plate |
| `electric-energy-distribution-1` | 120 (red+green) × 30 s | steel-processing, logistic-science-pack | medium/big pole |

Four consequences that decide the staging:

1. **`transport-belt` is enabled from the start.** It is *not* behind
   `logistics`, which only gives undergrounds and splitters. Green science's
   belt half is free of research; its inserter half is not.
2. **`assembling-machine-1` is behind exactly one lab research** —
   `automation`, 10 red packs — which is rung 7 of the current ladder. So stage
   2 begins the instant rung 7 closes and not before.
3. **Green science costs 75 red packs before a single green pack exists.** Run
   31 hand-crafted 10 red packs in 22,295 ticks with four bots (rungs 1-6 in
   37,586 ticks total). 75 packs by that method is ~167,000 ticks — 46 minutes
   of game time in which the bots do nothing else. **This is the argument for
   the staging, and it is arithmetic, not taste**: automated red science is not
   a nice-to-have on the way to green, it is what makes green reachable at all.
4. **The only real cycle in this neighbourhood is solar, and it is already
   excluded structurally.** `solar-energy` needs `logistic-science-pack` needs
   `automation-science-pack` — i.e. the lab it would power. `power.rs`'s header
   says so, and `state::generation_kw` has no `solar-panel` arm on purpose.
   Nothing in this design reintroduces it. `electric-energy-distribution-1`
   (medium poles) is behind green science and is likewise **not** on any stage's
   critical path; stage 3 must fit in small poles or wait.

**The 16-tick trap.** A trigger technology lands *after* the craft that fires
it settles — measured at ticks 53,469 → 53,485 on a real run
(`2026-09-02-recipe-not-enabled.md`), and it cost 27,462 ticks of redone work.
`await_preds` now ends with `await_research` reading `Condition::Researched`
off the action. **Every action in a cell whose recipe is trigger-gated must
carry `Condition::Researched`**, or the same race reappears one layer up, where
it is worse: a `SetRecipe` that lands 16 ticks early leaves an assembler with
no recipe and *no error*.

---

## 6. Power

### 6.1 The numbers

**Stage 2, one red-science cell plus its lab:**

| Consumer | Count | kW each | kW |
| --- | --- | --- | --- |
| assembling-machine-1 | 3 | 75 | 225 |
| electric-mining-drill | 2 | 90 | 180 |
| lab | 1 | 60 | 60 |
| inserter (duty model) | 12 | 13 | 156 |
| stone-furnace | 3 | 0 electric | 0 |
| **total** | | | **≈ 621 kW** |

**Stage 3, red and green together:**

| Consumer | Count | kW |
| --- | --- | --- |
| assembling-machine-1 | 10 | 750 |
| electric-mining-drill | 4 (incl. one on coal) | 360 |
| lab | 1 | 60 |
| inserter | ~30 | 390 |
| **total** | | **≈ 1,560 kW** |

So: **the brief's "several hundred kW, not 60" is right, and low.** Stage 2 is
ten times a lab. Stage 3 is twenty-six times a lab and does not fit in one
steam engine.

**Engines needed.** One engine is 900 kW; one boiler feeds two.
Stage 2 fits one boiler + **one** engine at 69 % load. Stage 3 needs one boiler
+ **two** engines (1,800 kW) at 87 % of the boiler's own 1.8 MW ceiling — i.e.
stage 3 is at the limit of a *single-boiler* plant and a third engine would
require a second boiler and more pipe.

**What that costs in iron.** From `power.rs`'s bill and the recipe table: pump
7, boiler 4, engine 31, pipes 1 each. A second engine is **+31 iron plates**
and one more pipe joint. The plant as built is ~45 plates; stage 3's plant is
~78. Against the ~98 iron plates rung 7 already spends, this is not the
expensive part — §6.2 is.

### 6.2 The expensive part is coal, not kilowatts

A boiler at effectivity 1 burns fuel in proportion to demand:

| | electric demand | boiler coal | stone furnaces | **total coal** |
| --- | --- | --- | --- | --- |
| stage 2 | 621 kW | 9.3 /min | ~2.6 /min | **≈ 12 /min** |
| stage 3 | 1,560 kW | 23.4 /min | ~8.9 /min | **≈ 32 /min** |

A character mines at speed 0.5 against coal's `mining_time` of 1, so **one bot
hand-mining coal produces ~30/min** — and mining has been observed running ~2×
over prediction. **Stage 3 therefore consumes more coal than a bot can mine.**
That is not a tuning problem; it is a proof obligation:

> **Stage 3 is not reachable with hand-mined coal.** A drill on a coal patch
> feeding the boiler is mandatory, not an optimisation. One electric drill on
> coal is 30 coal/min = 120 MJ/min of fuel for a 90 kW draw — enormously net
> positive, and it closes the loop that otherwise ends with four bots employed
> full-time as a conveyor belt.

### 6.3 Two capacity bugs the design must fix, one of them new

**(a) Nameplate, not throughput.** `electric_supply_kw` counts a steam engine's
900 kW whether or not it has steam, because nothing in `FactorioWorld` reports
that. A dry boiler reads as fully generating. `power.rs` says so in
`PLANT_COAL`'s doc and calls it the residual it does not close. This design does
not close it either (§13); it makes it *observable* via §9.4's witness.

**(b) `Condition::Powered` is a per-consumer test, not a network budget — and
this is new.** Each consumer asks `electric_supply_kw(my_area) >= my_kw`. Three
assemblers on a 900 kW network each ask for 75 and each pass. **Ten assemblers,
four drills, a lab and thirty inserters — 1,560 kW — all pass on the same 900 kW
network, individually, every one of them.** For one lab that was fine and
nobody could have hit it. For a factory it is precisely the *coverage is not
capacity* failure one level up: everything is connected, everything checks out,
and the network browns out. Factorio does not fail this loudly — under-supply
degrades every consumer proportionally, so a 1,560 kW load on 900 kW runs
everything at 58 % and produces a plan whose every lag edge is wrong by 1.7×
with no error anywhere.

**D5: `Powered` must be checked against uncommitted capacity.**

```rust
Powered { pos: Position, entity: ItemId, kw: f64 }
// holds  iff  electric_supply_kw(area) - electric_demand_kw(same network, excluding self)  >=  kw
```

`PlanState` gains `electric_demand_kw(area) -> f64`, summing the modelled draw
of every consumer in `added` and in the base world that shares an electric
network with `area`, by the same three-step coverage/connectivity walk
`electric_supply_kw` already does — so there is one notion of "same network",
not two. Draw comes from a `consumer_kw(name)` table written down in `state.rs`
beside `pole_supply_half_extent` and `generation_kw`, for the same stated
reason: **the mod does not send `energy_usage`**. Sending it is the follow-up
that deletes all three tables and is already named in the power note.

Two properties this must have, both testable without a game:

* **A second consumer on a full network refuses.** Two labs on one engine pass;
  thirteen assemblers do not. Today thirteen do.
* **Excluding self is not optional.** A consumer that has already been placed
  by an earlier step of the same plan must not be counted against itself, or
  the second replan of an identical plan refuses what the first accepted — a
  non-idempotent predicate, which in a supervisor loop is an oscillation.

---

## 7. Burner or electric, per step, and what "forever" costs

The brief frames this as "no power but a bot forever" versus "power but a
plant". The prototype data says the trade is not where it looks, because **a
fuel slot holds a whole stack**:

| Machine | draw | 50 coal = 200 MJ lasts |
| --- | --- | --- |
| `stone-furnace` | 90 kW | **37 min** |
| `burner-mining-drill` | 150 kW | **22 min** |
| `boiler` @ 621 kW demand (stage 2) | 621 kW | **5.4 min** |
| `boiler` @ 1,560 kW demand (stage 3) | 1,560 kW | **2.1 min** |

**The electric plant is the hungriest single object in the base, by an order of
magnitude.** "Burner needs a bot forever" is true per machine but generous per
visit; "electric needs one plant" is true but the plant needs a visit every two
minutes. Neither statement survives contact with the stack size.

The decision, per step:

* **Stage 1 — burner everywhere.** Burner drill and stone furnace, no
  electricity at all. One visit fuels a pair for **22 minutes** (the drill is
  the binding constraint). It needs no research, no plant, no wood and no
  inserter, and every trap it can hit is a geometry trap this repo already has
  machinery for.
* **Stage 2 — electric drills and assemblers, burner furnaces.** Assemblers
  have no burner variant, so the plant is forced the moment red science is
  automated. Smelting stays on stone furnaces because `electric-furnace` needs
  `advanced-material-processing` (far behind green science) and a stone furnace
  is 5 stone against a 3×3 electric furnace's much longer bill. Electric drills
  over burner ones because they are 2× the rate for the same footprint class
  and the plant already exists — but they cost `electric-mining-drill`
  (25 red packs), which stage 2 must budget for or use burner drills and accept
  half the rate.
* **Stage 3 — add a coal drill and a fuel buffer.** See below.

**What "forever" actually costs, and the one cheap answer.** A `boiler`'s fuel
inventory is **one slot**. Hand-fuelling it is a bot visit every 2–5 minutes
forever, which is not automation. The fix is available at stage 1 prices:

> **`iron-chest` (32 slots × 50 coal = 1,600 coal = 6.4 GJ) + one
> `burner-inserter` feeding the boiler.** At stage 2's 621 kW that is **2.9
> hours**; at stage 3's 1,560 kW, **68 minutes**. `iron-chest` (8 iron plates)
> and `burner-inserter` (1 iron plate + 1 gear) are both **enabled from the
> start** — no research, no electricity, and the inserter cannot brown out
> because it is not on the grid.

**Unverified and important:** a burner inserter is believed to refuel itself
from the coal it is moving. I could find no prototype flag asserting it — the
prototype gives `energy_source` burner, `fuel_inventory_size = 1`, 50 kJ per
movement (so one coal is 80 swings). If it does not self-fuel, the fallback is
a second burner inserter chest→inserter, or a bot top-up whose interval is 80
swings rather than 2 minutes. §15 lists this as a run-settled question.

---

## 8. Layout

### 8.1 The decision

**A fixed cell, stamped at a sited origin and rotated as a rigid body — not
per-machine siting with connection rules.**

### 8.2 Why per-machine siting cannot work here

`free_area_near_where` (`method/util.rs:370`) is the only siting primitive the
planner has. It ring-searches `0..=FREE_TILE_SEARCH_RADIUS` (12) around an
anchor, on the grid `tile_alignment` gives the entity, and returns the **first**
site that fits. Worst case 625 candidates.

That is exactly right for a furnace, whose only requirement is "somewhere near
the ore", and exactly wrong for a cell, because:

1. **Connection is the entire content of the design, not a constraint on it.**
   An inserter is not "near" two machines; it is on one specific tile between
   two specific tiles, facing one specific way. Two independent first-fit
   searches produce two positions with no relationship, and there is no repair
   step — you cannot slide an assembler one tile to meet an inserter without
   invalidating the inserter's own site.
2. **First-fit is state-dependent in a way a cell cannot absorb.** The search
   returns the first free ring position; a tree, a bot, or an entity this plan
   placed two steps ago moves it. Each machine sited independently multiplies
   that, and the cell's connectivity is the product of every one of those
   choices being simultaneously lucky.
3. **The servicing lane cannot even be stated.** The 18-of-18 measurement in
   `2026-09-02-rung-7-unreachable.md` is the whole argument: furnaces on the
   planner's 2-tile grid leave `2 − 2×0.69921875 = 0.6015625` between boxes
   against a `0.3984375`-wide character — 0.1015625 slack per side. All 18 of
   18 walk-stall detections had the character pressed against a furnace, max
   clearance 0.094, **eight of them at 0.00391 = 1/256 of a tile**. Widening to
   a 3-tile grid was put to the project owner and rejected. A first-fit search
   has nowhere to put "and leave a lane"; a fixed cell has the lane in the
   layout and can assert it.
4. **Determinism gets cheaper, not dearer.** One siting search plus a pure
   derivation is one state-dependent choice. Ten searches are ten.

### 8.3 Why the fixed cell is not a leap

**`power.rs` already is one**, and it is in this tree tonight. Its `layout()`
takes a pump position and a facing and derives five more buildings by
subtracting connection offsets from the joints they must reach; the whole thing
is *"a rigid body rotated about the pump's tile centre, which is what keeps
every building on its own build grid at all four facings"*, asserted by
`every_facing_puts_every_building_on_its_own_grid`. `plan_plant` sites it with
one ring search over shoreline candidates × four cardinal facings, and
`plant_steps` turns it into a bill of `Goal::Have` subgoals plus one `Place` per
part, reserving each site in `ctx.state` as it is emitted so two subtrees of one
plan cannot choose the same ground.

A cell is the same object with different offsets. The parts are:

```rust
/// One machine of a cell, with the direction it stands in.
pub struct CellPart { pub name: &'static str, pub offset: (f64, f64), pub direction: Direction }

/// A whole cell, sited and checked, ready to be turned into steps.
pub struct Cell {
    pub parts: Vec<CellPart>,      // in build order
    pub origin: Position,
    pub facing: Direction,
    /// The machine whose output inventory the witness reads (§9.4).
    pub terminal: Position,
    /// Every tile a bot must be able to stand on to service this cell.
    pub lane: Vec<Pos>,
    /// The cell's whole footprint, for the single free-rect query of §8.4.
    pub bounds: Rect,
}
```

**Stage 1's cell, concretely.** A burner mining drill is 2×2 with
`vector_to_place_result = (-0.35, -1.3)` in the north frame; a stone furnace is
2×2. A drill at integer-aligned `(X, Y)` facing north drops at
`(X − 0.35, Y − 1.3)`, which falls in tile `(X−1, Y−2)` — a tile covered by a
furnace centred at `(X, Y − 2)`. So:

```
part 0: burner-mining-drill  offset (0,  0)  direction North
part 1: stone-furnace        offset (0, -2)  direction North
lane:   the tiles east and west of both, two tiles wide
```

The drill's box bottom is `Y − 0.7` and the furnace's top is `Y − 1.3`, a 0.6
gap — **the same 0.6 that wedged eighteen bots**, so the lane is explicitly on
the flanks and the gap between drill and furnace is declared unwalkable. That
declaration is a test, not a comment: `every_lane_tile_admits_a_character`
asserts each `lane` tile's centre clears every part's box by at least
`0.19921875 + margin`.

**Stage 2 adds inserters, and the direction rule is the trap.** Per CLAUDE.md,
an inserter's `direction` points at the side it **picks up** from. The rule
that reproduces both documented cases is:

```
pickup = pos + turn((0, -1), direction)       // long-handed: (0, -2)
drop   = pos + turn((0,  1), direction)       // long-handed: (0,  2)
```

Check it: direction 12 (west) turns `(0,−1)` to `(−1,0)`, so pickup is one tile
west and drop one tile east — which is CLAUDE.md's "direction 12 is what moves
items west to east". And direction 0 picks up one tile north and drops one tile
south — CLAUDE.md's "for a row fed from a belt to its north, input and output
inserters are both `direction = 0`". Both cases fall out of the same two lines.

### 8.4 What this costs, and the ~410,000 boxes

The brief asks me to flag placement query cost. It is real and there are three
answers.

`PlanState::is_area_clear_of` (`state.rs:1243`) consults **six** occupancy
sources per candidate: the plan's own `added`, the entity quad tree, the
`blocked_tree` (trees, cliffs, rocks, units **and water**), every character, the
refusal ledger, and resource tiles. Since `fa8dabf3` a fully charted map carries
**~410,063 more blocking boxes** (79,717 `water` + 330,346 `deepwater`, measured
over 4,440,064 archived tile records — before that fix, *no water tile had ever
entered `blocked_tree` in this project's history*). `blocked_tree` is a quad
tree with a 1024-item node cap and 8 levels; a generated lake is dense. This has
already bitten exactly once, in `power.rs`'s water search, where an
unconditional 128-tile read meant cloning a 256×256 box (~65,000 tiles) on every
*successful* expansion.

So:

1. **One free-rect query per candidate origin, not one per part.** The cell
   declares `bounds`; siting tests `bounds` once. A ten-machine cell costs one
   query per candidate instead of ten. This needs a
   `PlanState::is_rect_clear(&Rect)` — `is_area_clear_of` already takes a
   `Rect` internally, so this is exposing what is there, not writing something
   new.
2. **Anchor the search, do not sweep.** Stage 1's origin is anchored to a
   resource tile (`nearest_resource_tile`, which the `Smelt` method already
   uses because `BotState::position` never advances during expansion); stage 2's
   is anchored to the supplying pole, exactly as `lab_site` is
   (`have.rs:1185-1198`). Neither searches open ground.
3. **The game validates the whole cell in one round trip.** `goal.plan`'s
   pre-check batches every `Place` in the plan into one `rcon_can_place_entities`
   call and re-expands around refusals, capped at `MAX_RESITE_ROUNDS + 1 = 3`
   (`goal/plan.rs:437, 490`), deduplicated to the plan's *chosen* sites rather
   than the 625-candidate window. Measured at under 1 % of a 103-step plan's own
   round trips. **A ten-machine cell costs the same one round trip a one-furnace
   plan does.** This is the single strongest argument that a stamped cell is
   affordable and per-machine siting is not.

**One thing I could not bound**: a cell footprint is a much larger `Rect` than a
2×2 furnace, and `is_area_clear_of` widens its quad-tree radius by the area's
own half-diagonal. A 20×20 cell bounds has a half-diagonal of ~14 tiles, so the
entity search radius grows accordingly and the `blocking_boxes_within` result
set grows with the area. Whether that is microseconds or milliseconds on a
charted map is not something the test suite can tell me. §15.

### 8.5 The blocker nobody has hit yet: ore blocks placement

`is_area_clear_of` ends:

```rust
!tiles_under(area).iter().any(|tile| self.base.entity_graph.any_resource_at(tile))
```

**Resource tiles are occupancy.** So `is_area_free` refuses every site where a
mining drill would legally stand — which is every site a mining drill can be
placed at, by definition. As it stands, **stage 1 cannot site its own first
machine.**

This is not a surprise so much as an untested corner: the only thing this
planner has ever placed is a stone furnace, which wants to be *near* ore and
never *on* it, and treating ore as occupancy is right for that. The fix has a
direct precedent: the offshore pump was taught to overlap water for exactly this
reason in `fa8dabf3`, via the `water_blocks` parameter already threaded through
this same function.

**D6: `is_area_clear_of` gains a `resource_blocks: bool`, defaulted `true`, and
mining drills pass `false`.** Narrower than the pump's exemption, and narrower
on purpose: a drill may stand on *any* resource, but nothing else may, and a
blanket exemption would let a furnace be sited on an ore patch the `Mine`
method is about to claim.

---

## 9. The shapes

### 9.1 The goal

```rust
// crates/planner/src/goal.rs
/// A standing arrangement that yields `item` at `per_minute` without further
/// intervention.
///
/// **Satisfied by structure, not by observation.** See `holds`: this asks
/// whether machines capable of the rate stand, face the right way, hold the
/// right recipe, feed one another and sit on a network with the capacity to
/// spare. It does **not** ask whether anything is coming out. A boiler with an
/// empty fuel slot reads as 900 kW and a drill on an exhausted patch reads as
/// a drill. The observation lives in `supervisor.witness` and nowhere in this
/// crate.
Producing { item: ItemId, per_minute: u32 },
```

### 9.2 Two new conditions, both siblings of `Powered`

```rust
// crates/planner/src/action.rs
/// The entity at `from` delivers into the entity at `to`.
///
/// **This is the inserter-direction trap, made checkable.** An inserter's
/// `direction` names the side it PICKS UP from; a backwards one places 100%,
/// passes every geometry check, and moves nothing. Stating the link rather
/// than the direction means a method cannot express the wrong thing without
/// the model refusing it.
///
/// Nothing produces this, exactly as nothing produces `Powered`: it becomes
/// true because two `Effect::CreateEntity`s landed with compatible positions
/// and directions. `infer_edges` draws no edge to it and the method that
/// needs it states its own `Step::Link`s.
Feeds { from: Position, to: Position },

/// The crafting machine at `pos` is set to `recipe`.
///
/// Unlike `Feeds` and `Powered`, this one IS produced — by
/// `Effect::SetRecipe`, because a recipe is set by an action and not by a
/// placement. So `infer_edges` can and does draw the edge.
RecipeSet { pos: Position, recipe: String },
```

`Feeds` is checked in `PlanState` by computing the source's delivery tile from
its name and direction and testing whether that tile is covered by the target's
rotated collision box. It needs one table:

```rust
/// Where a machine puts what it produces, as a north-frame offset from its
/// own position. Written down rather than read from the prototype for the
/// same reason as `pole_supply_half_extent` and the fluid-connection tables:
/// `FactorioEntityPrototype` carries no `vector_to_place_result` field, and
/// the mod does not send one. Sending it is the follow-up that deletes this.
fn delivery_offset(name: &str, direction: Direction) -> Option<Position> {
    let north = match name {
        "burner-mining-drill"   => (-0.35, -1.3),
        "electric-mining-drill" => ( 0.0,  -1.85),
        "inserter" | "fast-inserter" | "burner-inserter" => (0.0,  1.0),
        "long-handed-inserter"  => ( 0.0,   2.0),
        _ => return None,
    };
    Position::new(north.0, north.1).turn(direction)
}
```

A machine this table does not name `Feeds` nothing at all — refusing rather
than over-crediting, the same direction `pole_supply_half_extent` chose.

### 9.3 One new action kind and one new effect

```rust
// crates/planner/src/action.rs
ActionKind::SetRecipe { pos: Position, entity: String, recipe: String },
Effect::SetRecipe    { pos: Position, recipe: String },   // mutates PlanState.added
```

Stage 1 needs neither. Stage 2 cannot exist without them: **there is no
`set_recipe` anywhere** — not in `mods/BotBridge/control.lua`, not in
`FactorioRcon`, not in `ActionKind`. The only path to a set recipe today is
`rcon_place_blueprint` with a ghost carrying it, which the planner and executor
never call. An assembling machine placed today has no recipe and does nothing —
a placed-but-dead machine of exactly the class this project spent 2026-09-02
eliminating.

### 9.4 `holds`, and the split that answers the brief's question 1

**Planner side.** `holds(Producing { item, per_minute })` stops returning `None`
and returns `Some(bool)` from the structural predicate: *the state contains a
cell for `item`, complete, oriented, recipe-set, fed, and powered with
uncommitted capacity, whose modelled rate is at least `per_minute`.* Concretely
it is the conjunction the cell's own `Place` actions were stated against —
`EntityAt` for every part, `Feeds` for every link, `RecipeSet` for every
crafting machine, `Powered` (as amended by D5) for every electric one — read
against the current `PlanState` rather than at expansion time.

`Some(bool)` and not `None` because possession is not the only thing this
planner models: the entity overlay can answer a question about entities. But it
answers a **narrower question than the goal's name suggests**, and the name is
the risk, so the doc comment says the narrow thing in the first line (§9.1).

**Supervisor side — the durative check the planner does not have.**

```lua
-- scripts/supervisor.lua
-- A milestone that dispatches nothing and asserts something happened anyway.
supervisor.witness { item = "iron-plate", at = <terminal position>,
                     at_least = 8, within_ticks = 1800 }
```

It reads the terminal machine's output inventory, waits `within_ticks`
**dispatching no actions at all**, reads it again, and asserts the count rose by
`at_least`. Because no bot acts during the window, **any increase is
machine-made by construction** — which is the property the structural predicate
cannot have and the reason the two halves are not redundant.

Three things make this cheap:

* `rcon.inventory_contents_at(inventories)` is **already exposed to Lua**
  (`crates/scripting_lua/src/globals/rcon.rs:687`) and the mod's
  `rcon_inventory_contents_at` returns `output_inventory` and `fuel_inventory`.
  A furnace's and an assembler's *output* is exactly what a witness needs. (It
  does **not** return input slots, which is a real gap for other purposes and
  not for this one.)
* The supervisor already takes a keyframe at every milestone boundary.
* No Rust change, no planner change, no executor change, no mod change.

**And the ladder must alternate.** Every `Producing` milestone is followed by
its `witness`. A `Producing` that holds is a claim about ground; only the
witness is evidence about production. Stating that as a ladder convention
rather than a code invariant is deliberate — it is a fact about what a run
proves, and the run is where it belongs.

### 9.5 The method

```rust
// crates/planner/src/method/produce.rs   (new file, sibling of power.rs)
pub struct BuildCell;

impl Method for BuildCell {
    fn name(&self) -> &'static str { "build-cell" }

    /// Claims `Producing` and nothing else. `AlreadySatisfied` is registered
    /// ahead of it and now has a real answer for this goal (§9.4), so a cell
    /// that already stands expands to nothing — and, because `holds` no
    /// longer returns `None`, the supervisor reports that as
    /// `already_satisfied` rather than as `plan_empty`.
    fn applicable(&self, goal: &Goal, _: &PlanState) -> bool {
        matches!(goal, Goal::Producing { .. })
    }

    /// One bot builds one cell: the bill, the placements, the recipes and the
    /// fuel all belong in one pair of hands, for the same reason `power.rs`
    /// converges — three parts of a cell arriving on three bots is a cell
    /// nobody can assemble.
    fn converges(&self, _: &Goal, _: &PlanState) -> bool { true }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError>;
}
```

`expand` in order:

1. `let cells = plan_cells(&ctx.state, &from, item, per_minute)?` — how many,
   and where. Deterministic: origin from `nearest_resource_tile` (stage 1) or
   `nearest_supply_anchor` (stage 2), ring search in a fixed order, four
   cardinal facings tried north/east/south/west, exactly as `plan_plant` does.
   Refuses with a numbered error rather than improvising (§13).
2. One `Step::Subgoal(Goal::Have { item, count, whose: Holder::Share(ctx.chain_actor) })`
   per line of the bill, plus fuel — **`Have`, per D4′**.
3. One `Step::Act(Place)` per part, in build order, each carrying
   `AtPosition { min_radius: placement_clearance(name) }`, `AreaFree { pos,
   entity, direction }` and `HasItem`, with `Effect::CreateEntity` +
   `Effect::LoseItem`, and **each site reserved in `ctx.state` as it is
   emitted** — `expand` returns its whole step list before `run_steps` executes
   any of it, so an unreserved site is chosen twice by two subtrees of one plan.
4. One `Step::Act(SetRecipe)` per crafting machine (stage 2+), carrying
   `EntityAt` and `Condition::Researched(unlocking_technology(recipe))` — the
   16-tick guard of §5.
5. One `Step::Act(Insert { slot: Fuel })` per burner machine.
6. `Step::Link`s. `EntityAt` is world-scoped so `infer_edges` links a fuel
   insert after its placement on its own; the links that must be **stated** are
   the ones no effect can satisfy — every `Feeds` and every `Powered` — exactly
   as `plant_steps` returns every id so its caller can order the research after
   all of them, *"not just the generator's"*.

---

## 10. What changes in `crates/executor`

**Stage 1: nothing.** `Place` already carries direction all the way through —
`run.rs:656` → `Actuator::place(bot, item, at, direction)` → `place_entity_timed`
→ the mod's `rcon_place_entity(player_id, item_name, position, direction)`, with
`build_check_type` correctly forced to `manual` in `placement_check_args`.
`Insert { slot: Fuel }` is how the existing `Smelt` already fuels a furnace. A
burner drill and a stone furnace are two `Place`s and two `Insert`s.

**Stage 2: one method, one command, and nothing else.**

| Layer | Addition |
| --- | --- |
| `crates/planner` | `ActionKind::SetRecipe`, `Effect::SetRecipe`, `Condition::RecipeSet` |
| `crates/executor/src/run.rs` | one dispatch arm |
| `crates/executor/src/actuator.rs` | `async fn set_recipe(&self, bot, entity: &str, at: Position, recipe: &str)` |
| `crates/core/src/factorio/rcon.rs` | `set_recipe_timed`, in the shape of `insert_to_inventory_timed` |
| `mods/BotBridge/control.lua` | `rcon_set_recipe(player_id, entity_name, position, recipe)` |

Three mod-side rules the new command must obey, all already paid for elsewhere
in that file:

* **`rcon.print` is the verdict.** A debug line in the new function turns a
  success into a reported failure. Diagnostics go through `writeout`.
* **Check the return value.** `LuaEntity.set_recipe` can fail (locked recipe,
  wrong machine) — `2026-09-02-mod-placement-correctness.md` is two discarded
  return values in two lines, and this is the same shape.
* **Refuse a recipe the force has not unlocked, by name**, rather than setting
  nothing and reporting success. That is the 16-tick race's second landing spot.

**Not needed, and worth saying:** no rotate command (direction is placement-time
and that is sufficient), no inserter filters, no explicit pole wiring (poles
auto-connect by game rules and `electric_supply_kw` models exactly those rules),
no blueprint path.

**One executor property to check before stage 2, not to change:**
`Insert`/`Remove`/`Place` are not idempotent under recovery retry and
`recover.rs` cannot filter them (`recover.rs:44-70`). `SetRecipe` **is**
idempotent, which makes it the one new action that is safe under tier 1 — worth
a test asserting it, since the surrounding actions are not.

---

## 11. Determinism

`crates/planner` is pure: no I/O, no async, no wall clock, ordered collections,
floats via `total_cmp`, byte-identical plans for identical inputs. Every choice
above respects that, and the ones that took work:

* **Machine counts are integer.** D2′ removes the only float on the critical
  path of the expansion.
* **Siting is one ring search in a fixed order**, over facings tried
  north/east/south/west, over a `BTreeSet<Pos>` of terrain read once into a
  bounded box — `plan_plant`'s exact discipline, including the "narrow search
  first, wide one only on the failing path" rule that the 410,000 water boxes
  bought.
* **The cell's geometry is derivation, not search.** Offsets are constants
  turned by `Position::turn`; a rigid-body rotation about a tile centre takes
  tile centres to tile centres and tile corners to tile corners, so alignment
  holds at all four facings — and that is a test, as it is for the plant.
* **`electric_demand_kw` walks the same components `electric_supply_kw` does**,
  over the same ordered `entities_within`, so two plans that agree about the
  network agree about its budget.
* **`delivery_offset` is a match on a name**, not a lookup in a hash map whose
  iteration order could leak.

---

## 12. Prerequisites that must land before stage 1 can run

Not design work. Each is small, and each blocks a live run outright.

1. **D6 — the drill-on-ore exemption** (§8.5). Without it stage 1 refuses its
   own first placement, every time, on every map.
2. **D0 — `holds(Producing)` stops returning `None`** (§2.4). Without it the
   first empty plan closes the milestone as satisfied.
3. **`delivery_offset`'s two drill entries**, and a test asserting the stage-1
   drop tile lands inside the stage-1 furnace's box at all four facings. This is
   the `only_ghosts = true` lesson in a new coat: a drill and a furnace two
   tiles apart place 100 % whether or not the drill's output reaches the
   furnace.
4. **`PlanState::is_rect_clear(&Rect)`** exposed (§8.4).

Stage 2 additionally needs the power plant landed and rung 7 closed live, plus
§10's `set_recipe` chain and §6.3's D5 demand ledger.

---

## 13. Failure modes

Ordered by how quietly they fail. The quiet ones are the point.

| # | Failure | Detected? |
| --- | --- | --- |
| 1 | **A drill's fuel runs out.** | **No.** Nothing reads a fuel level. §7's stack arithmetic gives 22 min for a burner drill; after that the cell is silently dead. The witness catches it *once*, on the next witness milestone, and nothing catches it in between. |
| 2 | **A boiler runs dry.** | **No**, and worse than #1: `electric_supply_kw` counts nameplate, so a dry boiler reads as 900 kW and every `Powered` in the plan keeps passing. Inherited unchanged from `power.rs`, which says so in `PLANT_COAL`'s doc. |
| 3 | **The ore under a drill is exhausted.** | **No.** `Effect::ConsumeResource` is emitted by `Mine`, never by a drill; nothing decrements the patch for machine consumption. A cell on a thin patch dies at an unmodelled time. |
| 4 | **A machine's output backs up.** | **No.** A furnace with a full output stops. `PlanState` models no container contents at all. A cell whose terminal machine nobody empties stalls, and the structural predicate still holds. Stage 1's `Remove` is a bot visit; the general answer is a chest and an inserter, which is stage 3. |
| 5 | **An inserter faces backwards.** | **Yes, in the model** — `Feeds` cannot hold for it. But the model rests on `delivery_offset`, a hand-written table, so a wrong table is a wrong answer that every test agrees with. Mitigated only by the witness. |
| 6 | **A recipe is not set / is set to a locked recipe.** | **Yes** — `RecipeSet` + `Condition::Researched`, provided the mod refuses a locked recipe by name rather than reporting success (§10). |
| 7 | **Network under-supply.** | **Yes, after D5.** Today: no, and thirteen assemblers on one engine each pass individually (§6.3). |
| 8 | **A trigger technology lands 16 ticks late.** | **Yes** — `await_research`, provided every affected action carries `Condition::Researched`. |
| 9 | **The cell site is refused by the game.** | **Yes** — one batched `can_place_entities` round trip before any bot walks, `MAX_RESITE_ROUNDS + 1 = 3`. |
| 10 | **A bot is wedged in the servicing lane.** | **Partly.** The lane test asserts the lane is walkable *when empty*; a parked bot is a transient the mod now steps aside (action id 4712), and a bot the mod is already steering is not asked to move. Eighteen of eighteen stalls says this deserves a live check, not a claim. |
| 11 | **Smelt/craft time estimates drift.** | **No.** The 2026-09-01 spec's closing risk, still open; a factory multiplies the couplings. |
| 12 | **`game_speed` is stubbed at 1.0** (`actuator.rs:216`). | **No.** Every lag wait assumes 60 UPS. A cell's lag edges are long; on a server running below 60 UPS the executor takes plates out of a furnace that has not made them. |

---

## 14. Staged plan

### Stage 1 — the burner smelting cell. **No research, no power, no wood, no inserter.**

`Goal::Producing { item: "iron-plate", per_minute }` expands to
`ceil(per_minute / 15)` cells, each a burner mining drill dropping into a stone
furnace, with the bill, the placements and two fuel inserts. Bots stop carrying
ore.

**Cost: 9 iron plates and 10 stone per cell** (drill = 3 iron + 3 gears + 1
furnace; furnace = 5 stone). **Output: 15 plates/min, unattended for 22 minutes
per fuelling.**

**It does not beat four bots at a sprint, and the spec should not claim it
does.** Run 31 smelted 50 iron plates in 3,215 ticks. What it does is produce
*while the bots do something else*: over run 31's own 37,586-tick ladder, one
cell running from tick 0 makes ~156 plates and two cells make ~313 — more iron
than the entire ladder consumes (98 for rung 7, plus rungs 3 and 4). A cell pays
back its own 9 plates in 36 seconds.

**Independently testable, and worth having alone.** It is the first
machine-to-machine link in the project's history; it needs no executor change,
no mod change and no plant; and its witness is unambiguous — plates appear in a
furnace nobody loaded.

**Done when:** `Producing{iron-plate, 15}` plans, builds, and a `witness`
milestone with all bots idle shows the furnace's output rising.

### Stage 2 — the powered red-science cell

Blocked on rung 7 closing live. Adds: `set_recipe` end to end, D5's demand
ledger, `Condition::Feeds` and `RecipeSet`, electric drills, and inserters
between furnace, gear assembler and pack assembler. Two red-pack assemblers, one
gear assembler, three furnaces, two drills, ~12 inserters, **≈ 621 kW** on one
boiler and one engine, plus the `iron-chest` + `burner-inserter` fuel buffer that
turns "visit the boiler every 5.4 minutes" into "every 2.9 hours".

**Done when:** `Producing{automation-science-pack, 12}` holds and a witness
shows packs accumulating with every bot idle — and then, the real prize,
`Researched("logistic-science-pack")` completes without a bot hand-crafting 75
packs.

### Stage 3 — green science, the coal loop, and the ratio solver

Three more assemblers for packs, one each for circuits, cable, inserters and
belts; five iron furnaces and two copper; four drills, **one of them on coal
feeding the boiler's chest, which §6.2 proves is mandatory rather than
optional**; ≈ **1,560 kW** on two engines. This is the stage that needs D1's
matrix and D2's rationals, because it is the first with more than one machine
per step and the first where a ratio error compounds. Belt routing between
cells, the `FlowGraph` cross-check, and modules stay out of scope, as the
2026-09-01 spec already said.

---

## 15. What I could not determine without running

Nothing in this section is a hedge; each is a claim a run settles in one
observation.

1. **Does a burner mining drill's output actually land in a stone furnace two
   tiles away?** The arithmetic in §8.3 says yes and the whole of stage 1 rests
   on it. `vector_to_place_result` is not sent by the mod and not in
   `FactorioEntityPrototype`, so the table is hand-written — the same class of
   hand-written geometry as the fluid-connection tables, which turned out to be
   **unreadable from the prototype for two independent reasons**. This is the
   first thing to check and the cheapest.
2. **Does a burner inserter refuel itself from the coal it moves?** §7's fuel
   buffer is the answer to "what does forever cost" and it depends on this. I
   found no prototype flag either way.
3. **What does `is_area_clear_of` cost over a cell-sized `Rect` on a charted
   map?** §8.4. The test suite cannot answer it; the note that introduced the
   410,000 boxes says so explicitly.
4. **Does the whole cell survive one `can_place_entities` round trip?** A
   ten-entity cell is ten sites in one call; the pre-check has never been asked
   about more than a plan's worth of scattered furnaces.
5. **Can one bot hold a cell's bill?** `Holder::Share` welds a cell to one bot
   and archived runs top out at 90 iron plates and 65 iron ore on a single bot.
   Stage 2's cell bill is well past that, and the honest answer may be
   `Step::Owned` and a multi-bot build — which exists, and which this design
   deliberately does not use yet.
6. **Does an under-supplied network read as slow or as dead?** Factorio's
   documented behaviour is proportional degradation, which would make it *slow*;
   run 30's evidence for "dead" is a network with **zero** generation, which is
   a different case. D5 is right either way, but which failure it prevents
   changes how loudly it should refuse.
7. **Do the lane clearances hold with four bots actually moving?** The lane test
   asserts an empty lane. Eighteen of eighteen says empty is not the case that
   matters.
8. **Is `pole_wire_reach("big-electric-pole")` wrong?** `state.rs` says 30.0;
   the 2.1.17 prototype says **32**. The other three entries and all four supply
   areas match. Nothing in this design uses a big pole, so it is not a blocker —
   but that table's own doc admits it exists only because the mod does not send
   the field, and one entry has already drifted. Sending
   `supply_area_distance`, `maximum_wire_distance` and `energy_usage` from the
   mod deletes that table, `generation_kw`, `consumer_kw` and `delivery_offset`
   in one change, and is the highest-value follow-up this spec depends on.
