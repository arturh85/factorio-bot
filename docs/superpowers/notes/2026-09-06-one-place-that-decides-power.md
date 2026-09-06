# One place that decides power — and why the plant still stops at one boiler

2026-09-06. Branch `one-place-that-decides-power`.

Two things: an extraction that is proven pure, and an investigation that
deliberately produced a design instead of code.

## 1. `ensure_powered`, and the honest reason the two call sites differ

`method::assemble` and `method::extract` were described as containing the same
judgement twice — *if this site is not powered, get supply, and lay poles to
it*. **Only half of that was actually shared**, and the half that was not is
the half a third caller needs.

| | `extract` | `assemble` |
|---|---|---|
| where the site comes from | a charted resource tile, **fixed before power is considered** | chosen **from** the anchor, after |
| gates on `Condition::Powered` first | yes | no — it always asks for supply, because it needs an anchor to site cells from |
| lays a pole run | yes | **no** — `pole_run` had exactly one caller in the tree |
| what it does with the returned ids | links them to its own `Place` | links them to every `needs_power` action of the cell |

So `assemble` never carried the pole half at all. Forcing a single function on
both would have meant giving `assemble` a pole run it does not want, or giving
`ensure_powered` an "anchor only" mode that is a different function wearing the
same name. Neither is an extraction.

What was done instead is two functions, one nested in the other, so there is
still exactly one place per decision:

* **`supply_anchor`** — the shared half. *Where is there capacity for `kw`?*
  Returns the anchor position, the plant's steps if one had to be built, and
  the ids everything downstream must wait for. Both callers use it.
* **`ensure_powered`** — `supply_anchor` plus the pole run, for a caller whose
  site is fixed. `extract` uses it; `method::blueprint` is the intended second
  caller.

```rust
pub fn ensure_powered(
    ctx: &mut ExpansionCtx,
    consumer: &str,            // the prototype standing at `site`
    site: &Position,
    area: &Rect,               // its collision footprint at `site`
    kw: f64,                   // its draw — blueprint: sum of `consumer_kw`
    radius: f64,               // the cheap first supply tier's reach
    occupants: &[FactorioEntity], // reserved in the routing fork, not in ctx.state
) -> Result<Option<Powering>, PlannerError>;

pub struct Powering {
    pub steps: Vec<Step>,      // plant, pole bills, pole placements, in build order
    pub ids: Vec<ActionId>,    // every id the caller's own Place must follow
    pub powered: Condition,    // the headroom test to put on that Place
}
```

Three answers, and the middle one matters:

* `Err` — supply is impossible and `supply_for` says why by name.
* `Ok(None)` — supply exists, **no run of ≤ `MAX_POLE_RUN` poles reaches it**,
  or the model cannot see the finished run carrying power. The caller names its
  own refusal; `extract` calls it `ExtractionNotModelled`.
* `Ok(Some(_))` — an already-powered site answers this way too, with **empty
  steps and empty ids**, so no caller writes the "already powered" branch.

The `powered` condition travels back with the steps on purpose: it is the same
value `pole_run` validated the run against, so a caller cannot state a
different draw, prototype or tile on its `Place` than the poles were laid for.

`WIRE_REACH`, `POLE_STEP`, `MAX_POLE_RUN`, `pole_run`, `route_poles`,
`ring_search` and `place_step` moved from `extract.rs` to `power.rs` with them.

### It is a pure refactor, and that is measured, not asserted

Same release binary before and after, whole `--steps` plan report diffed (not
just the headline pair), seed-31337 t=0 dump, four bots:

| goal | actions | makespan | diff |
|---|---|---|---|
| `researched:automation` | 176 | 21,784 | **byte-identical** |
| `producing:automation-science-pack:6` | 316 | 22,463 | **byte-identical** |
| `producing:logistic-science-pack:6` | 442 | 47,542 | **byte-identical** |

### The tests, and the one that was too weak

Two new tests in `power::ensure_powered_tests`, each falsified one break at a
time with the failure count checked:

| break | expected | got |
|---|---|---|
| never take the already-powered early return | `an_already_powered_site…` red | 1 failed, that one |
| lay zero poles from the routed run | `an_unpowered_site…` red on `POWERED` | 1 failed, that message |
| `supply_anchor` drops the plant's **steps** | red | **green — the test was wrong** |
| `supply_anchor` drops the plant's **ids** | red | 1 failed, "id is not one the caller is told to order against" |

The third is the interesting one. `plant_steps` creates its parts in
`ctx.state` *as well as* emitting them, so a version that built the plant into
the planning overlay and threw the steps away still satisfied
`Condition::Powered` afterwards — the site read as powered in a world where
nobody had built anything. **The overlay is not the plan.** The test now also
asserts the emitted steps contain a `Place` of the engine and of a pole, and
that every `Place` id is one the caller was told to order against. With that,
break three goes red for the right reason.

This is exactly the shape
`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md` warns
about, reached from a different direction: the assertion was true of a broken
implementation because it was checking a side effect rather than the product.

## 2. The plant cannot grow past one boiler, and the ceiling is ours

### The ratio, verified

Against `workspace/server/data/base/prototypes/entity/entities.lua` (Factorio
2.0 base, in this repo's `workspace`), read 2026-09-06 — **verified**:

* `offshore-pump`: `pumping_speed = 20`. That field is fluid units per **tick**,
  so **1,200 water/s**.
* `boiler`: `energy_consumption = "1.8MW"`, `target_temperature = 165`. Water
  carries 0.2 kJ per unit per °C, so 150 °C above the 15 °C default is 30 kJ a
  unit and the boiler consumes 1.8 MW / 30 kJ = **60 water/s**.
* `steam-engine`: `fluid_usage_per_tick = 0.5`, `effectivity = 1`,
  `maximum_temperature = 165` → 0.5 × 60 × 30 kJ = **900 kW**.

**1 offshore pump : 20 boilers : 40 engines ≈ 36 MW.**

Two corroborations and one correction:

* the owner's "1 boiler : 2 engines for small builds" matches
  `MAX_ENGINES_PER_BOILER = 2`, derived here independently from 1.8 MW / 900 kW;
* the owner's **"1 pump : 200 boilers : 400 engines" does not survive the
  prototypes** — it is 10× the measured ratio, and would put the ceiling at
  ~360 MW rather than ~36 MW. Recorded rather than quietly dropped, because the
  next reader will meet both numbers.

Either way the conclusion the task drew is unchanged and understated: our plant
tops out at **1.8 MW against ~36 MW of available water**, a factor of twenty.

### Can `plan_plant_for` just add boilers? Not as a small change.

Geometrically it is more promising than the task feared. `BOILER_WATER` is
`[(-2, 0.5), (2, 0.5)]` — a boiler's two water ports sit on opposite ends of
its long axis, so boilers **chain end to end along the shore**, water flowing
through, on a 4-tile pitch. Each boiler's `BOILER_STEAM` then feeds its own
inland row of ≤ 2 engines, and an engine is 3 tiles wide against that 4-tile
pitch, so the rows do not collide. The result is a 2D grid, not a straight row,
but it is a *regular* grid and the existing rigid-body rotation about the pump's
tile centre still carries it.

What makes it not small is everything hanging off `Plant` being singular:

1. **`Plant` carries one `boiler: Position`.** `plant_steps` emits exactly one
   `Insert` of `PLANT_COAL` into it. Multi-boiler means `boilers: Vec<Position>`
   and one fuel action each — and `PLANT_COAL` is a flat five, argued from a
   single boiler's idle window, so the bill's justification has to be redone
   rather than multiplied.
2. **`method::assemble::fuel_for` and `boiler_near`** find *the* boiler near an
   anchor and compute one coal charge from the whole network's demand. Both
   become "which of these boilers", and the coal split across them is a new
   decision nobody has made.
3. **The shore fit gets much bigger.** Today the plant's footprint is a short
   rigid row that `plan_plant_for` tries at four facings on candidate shore
   tiles. A 20-boiler grid is roughly 80 tiles along the shore by ~13 inland;
   on a real lake most candidate tiles will fail, and the failure has to say
   *why* by name rather than falling out as "no site". That is a search change,
   not a layout change.
4. **`PIPE_COUNT = 3` is a constant** because the current joint geometry is
   fixed. A chain of boilers needs pipe between consecutive water joints — a
   count that scales with the boiler number, so `pipe_run_plates` and the bill
   arithmetic move too.
5. **`engines_for` returns a `u32` bounded by `MAX_ENGINES_PER_BOILER`** and
   `PowerPlantTooSmall` is phrased around that single bound. Sizing becomes a
   pair (boilers, engines) with a *water* bound at 20 boilers — a genuinely
   different refusal.

That is five owners across three modules, a changed public struct, and a new
search failure mode. It is a task, not a tail-end of this one. **Left unbuilt
on purpose**, per the standing preference in this repo for a documented design
over a half-built mechanism.

### What was done instead: the ceiling says whose it is

`PlannerError::PowerPlantTooSmall`'s `help` used to read as though 1.8 MW were
a fact about Factorio:

> one boiler drives at most two steam engines (1.8 MW of boiler over 900 kW of
> engine); more generation than that needs a second boiler, and the planner does
> not yet site one

It now names the limit as the layout's, gives the real number, and shows the
arithmetic:

> this is a limit of the LAYOUT, not of the game: one boiler drives at most two
> steam engines (1.8 MW of boiler over 900 kW of engine), and this planner lays
> out exactly one boiler in a rigid pump-pipes-boiler-engines row. The water
> behind one offshore pump would carry about twenty boilers and forty engines —
> ~36 MW — because the pump moves 1200 water/s and a boiler burns 60/s (verified
> against base/prototypes/entity/entities.lua). Ask for less, or site a second
> plant

The same paragraph is on `MAX_ENGINES_PER_BOILER` and on the error variant's
doc, with the ratio marked **verified** and the source file named, so the next
reader checks the prototypes rather than trusting prose. Three documented
constants in this repo turned out wrong in a single day; this one states how to
re-derive itself.

### Scaling note for whoever picks this up

More generation is not obviously the next bottleneck. Every green run in the
record reads `roster-fed`, with **no generation at all for the first 8–10
minutes** and 120 kW drawn thereafter — two orders of magnitude below even the
current 1.8 MW ceiling. Nothing in this project has yet asked for a second
boiler in a live run. The ceiling is real and worth stating honestly; it is not
yet demonstrated to be *binding*.
