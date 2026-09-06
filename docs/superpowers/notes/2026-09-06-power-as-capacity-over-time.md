# Power as capacity over time, not as coverage

2026-09-06. Roadmap item 3. Branch `power-as-capacity-over-time`.

## What was already there, which is most of it

The brief said "nothing checks that generation covers the demand of whatever
else the plan stands up". Read against the tree, that is no longer true in
general, and it is important to say exactly where it *is* still true, because
the residual is one function call wide and the rest of the machinery is sound.

Already in place before this change:

* `PlanState::electric_supply_kw(area)` — coverage, then wire connectivity by
  union-find over `pole_wire_reach`, then the nameplate output of every
  generator a pole of that same component covers. Not a coverage check.
* `PlanState::electric_demand_kw(area, except)` — the same `ElectricNetwork`
  walk, summing `consumer_kw` over every consumer on it, skipping crafting
  machines with no recipe set, skipping the consumer being asked about.
* `Condition::Powered { pos, entity, kw }` — checked as
  `supply - demand >= kw`. A **network budget**, not a per-consumer test.
* `PlanState::nearest_supply_anchor(from, radius, kw)` — the same headroom
  test, applied when choosing a pole to hang a new consumer off.
* `PlanState::consumer_draw_kw(name)` — so a method states the number the
  ledger will bill it, rather than a second copy of it.

Verified against `workspace/data/base` (2.1.17) rather than from memory:
`pumpjack` 90 kW, `assembling-machine-2` 150 kW, `lab` 60 kW,
`electric-mining-drill` 90 kW, `boiler` `energy_consumption = "1.8MW"`,
`steam-engine` `fluid_usage_per_tick = 0.5`, `effectivity = 1`,
`maximum_temperature = 165`. The engine's 900 kW is derivable rather than
asserted: steam has 0.2 kJ/unit/°C, 165 °C against the 15 °C default is
30 kJ/unit, and 0.5 units/tick × 60 ticks/s × 30 kJ = 900 kW. So one boiler
carries **exactly two** steam engines, and that ratio is a fact about the
prototypes rather than a chosen constant.

## The residual, which is the whole of this change

`method::power::supply_for` answers the question "where does a consumer needing
`kw` get its power" in four tiers. **Three of the four check `kw`. The fourth
does not.**

```rust
pub fn supply_for(state, from, near_radius, kw) -> Result<Supply, PlannerError> {
    for radius in [near_radius, PLANT_ADOPT_RADIUS] {
        if let Some(a) = state.nearest_supply_anchor(from, radius, kw) { ... }  // kw
    }
    if let Some(plant) = complete_plant(state, from, kw) { ... }                // kw
    Ok(Supply::Build(plan_plant(state, from)?))                                 // no kw
}
```

`plan_plant` has no `kw` parameter at all. It builds one fixed plant — pump,
three pipes, boiler, one steam engine, one small pole, 900 kW — and returns it
for any demand whatever. Ask it for 60 kW (a lab) and it is oversized. Ask it
for 2,000 kW and it returns `Ok` with a plant that is short by 1,100, and
nothing downstream says so by name: the consumer's own `Condition::Powered`
then fails at schedule time as a `PreconditionUnsatisfied` naming a kilowatt
figure, which is the *symptom*, several layers from the decision that caused it.

That is the coverage-is-not-capacity failure with the halves swapped, and it is
the one the brief's own quotation describes. It has not bitten yet only because
both callers ask for less than 900: `method::have` asks 60 for a lab, and
`method::assemble` asks about 189 for a red-science cell. The agent siting a
pumpjack (90 kW) alongside a refinery (420 kW) and chemical plants (210 kW
each) crosses 900 with three machines.

## The design question: which "over time"

Three readings were on the table. The choice, and the reason, matter more than
the code.

### 1. A static end-of-plan check — **chosen**

Sum generation, sum demand, compare, over the final state of the plan.

### 2. A time-phased check — **rejected, and the argument is the interesting part**

The intuition is that consumers come online at different ticks and the plant is
built partway through, so a single sum over the end state must be the wrong
question. Worked through, it is not:

> **Demand in this planner is monotone non-decreasing over a plan, and so is
> supply.** No method removes a consumer, and no method unsets a recipe. Every
> `Effect::CreateEntity` adds; nothing subtracts. Therefore
> `max_over_ticks(demand) == demand(final)`, and a static check on the final
> state is **exactly** the worst case a time-phased check would find.

A time-phased ledger would therefore only ever charge *less* than the static one
at intermediate ticks. That is the permissive direction, on a ledger whose
`consumer_kw` table already errs permissive for names it does not know (its own
doc says so). Buying extra machinery to make an already-permissive check more
permissive is a bad trade.

The genuine time-phased hazard is real, but it is the *other* asymmetry:
generation arriving after the demand that needs it. That is a **sequencing**
problem, not a capacity one, and it is already solved elsewhere and by a
different mechanism — `PlanState::powering_entities` turns the poles and
generators a `Condition::Powered` reads into `Condition::EntityAt`
preconditions, so `ActionNetwork::infer_edges` draws the edge from the
placement to the consumer. That machinery exists because of a measured failure
(`run-1788617269-96746`: research scheduled at tick 42,905, the joining pole
placed at 49,411, labs `no_power` with all 85 packs inside for 12,300 ticks). A
capacity ledger would not have caught it and does not need to.

**The monotonicity assumption is named here so a future change knows to come
back.** The static check stops being equivalent to the time-phased one the day
any method emits an effect that removes an entity, unsets a recipe, or models a
machine that stops drawing. None does today; `Effect` has no such variant.

### 3. A new `Goal` kind — **proposed, not implemented, reported instead**

`Goal::Powered { kw, near }` would make power something a caller asks for
rather than a side effect of asking for research. It is coherent and it is not
what this change does, for two reasons and one procedural one:

* **Procedural**: an eighth `Goal` kind requires four coordinated edits —
  `method/have.rs::holds()`, `crates/scripting_lua/src/globals/goal/value.rs`
  (both the match and its `KINDS` list, which has already silently rejected a
  valid goal by omission), and `crates/server/src/game/control.rs`. Three of
  those four files belong to other sessions right now.
* **Substantive**: power is a *precondition*, not an end. Every consumer this
  planner places already states its own `Condition::Powered` with the number
  the ledger will bill it, and `supply_for` already resolves that into a
  standing network or a plant. A `Goal::Powered` would be a second way to ask
  for the same thing, and the failure mode of two ways is that they disagree.
* The one case it would serve that nothing else does is *pre-provisioning*:
  "stand up 1.8 MW at this shore before I decide what to put on it." That is a
  real use, and it is speculative until somebody has it.

Recommendation: **do not add it yet.** If it is added later, it should be a thin
wrapper that calls `supply_for` with an explicit `kw`, so there stays one
implementation of the question.

## What was implemented

**`plan_plant` learns the demand it is being built for, and refuses by name
when it cannot meet it.**

* `layout` takes an engine count. Steam engines chain end to end off each
  other's steam connection — `ENGINE_STEAM[1]` is `(0., 3.)` from a 5-tile-long
  engine, so consecutive centres are 5 apart along the facing axis and no pipe
  goes between them. The rigid-body property is preserved: the whole thing is
  still one shape rotated about the pump's tile centre.
* `engines_for(kw)` = `ceil(kw / ENGINE_KW)`, clamped to at least 1.
* `MAX_ENGINES_PER_BOILER = 2`, **derived** from the two prototype numbers
  above rather than chosen: 1.8 MW of boiler over 900 kW of engine. Above that
  a second boiler is needed, which is a second water tap on the pipe run and
  genuinely more shoreline geometry than this change buys.
* `PlannerError::PowerPlantTooSmall { needed_kw, plant_kw }` is the refusal
  above `MAX_ENGINES_PER_BOILER`. Refusing by name is the requirement; the
  alternative — returning an undersized plant — is the silent failure this
  whole roadmap item exists to close.
* The pole must supply **every** engine, not just the first. A small pole's
  supply area is 5×5 and two engines end to end span 10 tiles, so a pole beside
  the *joint* between them overlaps both (the game's rule, and
  `ElectricNetwork::carries`'s, is overlap rather than containment) while a
  pole beside the first engine covers only the first. Getting this wrong would
  build the second engine and never wire it — 900 kW standing on the ground,
  reported as 1,800.
* The pole search origin is the **centroid of the engine positions**, which is
  the first engine's position when there is one engine, so the single-engine
  path is unchanged by construction rather than by luck.
* `PLANT_COAL` scales with the engine count. A two-engine plant is serving
  roughly twice the load, so it burns roughly twice the fuel; at one engine the
  bill is the unchanged 5.

`plan_plant(state, from)` keeps its old signature and is now
`plan_plant_for(state, from, 0.)` — `engines_for(0.)` is 1 — so every existing
caller and test sees exactly today's plant. `supply_for` is the one call site
that passes a real `kw`.

### The state.rs edit, which is minimal and additive

`generation_kw` is private to `state.rs`. `power.rs` needs a steam engine's
nameplate output to size the plant, and hardcoding 900 in a second place is the
drift the file's own table docs warn against. So `PlanState` gains
`generator_output_kw(&self, name) -> Option<f64>`, a four-line accessor
mirroring the `consumer_draw_kw` accessor that already sits beside it for
exactly the same reason. Nothing is moved, renamed or reformatted.

## What this still does not close

Stated rather than hidden, in the discipline of the tables it sits beside:

* **Nameplate, not observed.** A boiler with an empty fuel slot reads as 900 kW,
  and now a two-engine one reads as 1,800. Sizing the plant does not make it
  run; `PLANT_COAL`'s own doc already names the fuel monitor this wants.
* **A consumer `consumer_kw` does not name draws nothing**, so an unmodelled
  machine is headroom that is not there. Unchanged, and the one permissive
  table in the file.
* **Solar is still absent, deliberately.** See below.
* **Above 1.8 MW the planner refuses rather than building.** That is a bound on
  this change, not a claim about the game.

## Solar: engaged with, still excluded

The brief asks not to add a `solar-panel` arm without argument, and notes
correctly that a solar panel is entirely pre-oil (5 steel plate, 15 electronic
circuit, 5 copper plate) so the "solar needs oil" half of the old story was
never the binding constraint. Two things are true and they point opposite ways:

1. `power.rs`'s module doc says solar is excluded because `solar-energy` needs
   `logistic-science-pack`, which needs the lab the plant is powering.
   **Circular, and it stands as a reason not to plan a *first* plant out of
   solar** — but it is not a reason to refuse to *count* a solar panel a run
   already has, or to build one once the research is done. This note records
   that the module doc overstates its case.
2. `state.rs`'s `generation_kw` doc gives the other reason, and it is the
   binding one: **a solar panel's output is a function of the map clock.**
   `production = "60kW"` is peak; the Nauvis daily average is about 42 kW and
   the instantaneous value at night is zero. This crate is pure and
   deterministic — same inputs, same plan — and there is no in-game time of day
   among its inputs. Crediting 60 would plan a base that is dead for a third of
   every day; crediting 42 would plan one that browns out every night;
   crediting 0 is what the table does, and it under-credits, which is the
   direction every other table in that file chooses for an unknown.

So solar stays out, on reason (2) alone, and the module doc in `power.rs` is
corrected to say so rather than resting on the circularity. The honest way in
later is an **accumulator-backed** figure — panel average minus night draw,
with accumulators sized to carry it — which is a real model and not a table
entry, and which wants the map clock as a planner input before it can be
deterministic at all.

## Verification

Three offline baselines against `workspace/scripts/map.json`, before and after,
byte-identical:

| goal | actions | ticks |
|---|---|---|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,463 |
| `producing:logistic-science-pack:6` | 442 | 47,542 |

They are expected to be identical: every existing caller asks for under 900 kW,
`engines_for` answers 1, and the single-engine layout and pole siting are
unchanged by construction. Measured before and after; the `plan` output is
byte-identical, not merely equal on the two headline numbers.

`cargo test --workspace` exits 0 with `capacity_tests` appearing 13 times in
the run's own output — checked, because a suite here has reported green with
none of the new tests compiled in.

### Falsification: nine deliberate breakages, all red

Every test was made to fail on purpose before being believed, each by a text
substitution whose match count was asserted to be exactly one — a zero-match
substitution passes and reads as a hollow test.

| break | tests that went red |
|---|---|
| `MAX_ENGINES_PER_BOILER` 2 → 3 | the sizing test, the `supply_for` refusal |
| `pole_site` checks only the first engine | the blocked-ground pole test |
| `finish` adopts a pole covering only the first | the standing-decoy test |
| pole search starts at the first engine, not the row centre | 8 tests |
| engine pitch 5 → 6 tiles | the layout test |
| engine row grows towards the boiler | layout, overlap, decoy |
| coal stops scaling with the row | the bill test |
| `engines_for` `ceil` → `floor` | 7 tests |
| `supply_for`'s build tier ignores `kw` again | both refusal tests |

**Two of these were green on the first attempt, and that is the finding worth
keeping.** Weakening the pole rule to "reaches the first engine" changed
nothing, because `free_area_near_where` starts its ring search at the joint
between the two engines and the first free tile there covers both *by
geometry*. The test was asserting a property of the tile the search happened to
return, not of the rule that was supposed to guarantee it — precisely the shape
`2026-09-06-fixtures-agree-with-their-code.md` describes.

Two changes closed it:

* the rule was extracted into one function, `pole_site`, so `fit` and `finish`
  cannot drift apart and there is a single thing to break;
* the test now blocks **every** tile in the ring search that could reach both
  engines, asserts that a first-engine-only tile is still free (so a weakened
  rule would have taken it), and requires `pole_site` to answer `None`. A
  second test plants a standing decoy pole covering one engine and requires
  `finish` not to adopt it.

Both fixtures assert their own preconditions — that they blocked something,
and that the decoy really does reach one engine and not the other — so a
fixture that quietly stopped discriminating fails loudly instead of passing
vacuously.
