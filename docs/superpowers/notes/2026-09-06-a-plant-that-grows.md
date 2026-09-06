# A plant that grows — and the pole that stopped it at three boilers

2026-09-06. Branch `a-plant-that-grows`.

`method::power` laid out **one** boiler and refused above 1.8 MW by name. It
now chains up to twenty, and the ceiling that remains is the **water behind one
offshore pump** rather than anything about our geometry.

The design was written first
(`docs/superpowers/notes/2026-09-06-one-place-that-decides-power.md`, section
2) and this built it. Two things it got right, one it got wrong, and one it did
not mention at all — which is the one that cost the day.

## What was built

`plant_size_for(state, kw) -> PlantSize { boilers, engines }` replaces
`engines_for` as the sizing decision (`engines_for` survives as a thin wrapper
so nothing else moved). Engines come from the demand at 900 kW each; boilers
come from the engines at `MAX_ENGINES_PER_BOILER = 2`. `Plant` grew a
`boilers: Vec<Position>` beside its existing `boiler`, exactly mirroring the
`engine` / `engines` pair that already existed. `layout` takes both counts and
stands the chain along the shore, each boiler with its own steam pipe and its
own inland engine row.

| demand | before | now |
|---|---|---|
| 60 kW (a lab) | 1 boiler, 1 engine | **unchanged** |
| 189 kW (a red cell) | 1 boiler, 1 engine | **unchanged** |
| 1,170 kW (`MinerLine`) | 1 boiler, 2 engines | **unchanged** |
| 1,800.5 kW | `PowerPlantTooSmall` | 2 boilers, 3 engines |
| 4,320 kW (24 electric furnaces) | `PowerPlantTooSmall` | 3 boilers, 5 engines |
| 36,000 kW | `PowerPlantTooSmall` | **20 boilers, 40 engines** |
| 36,000.5 kW | `PowerPlantTooSmall` | `PowerPlantTooSmall`, and now honestly |

**The three offline baselines are byte-identical** — 176 / 21,784 · 316 /
22,463 · 442 / 47,542, same release binary built before and after, whole
`--steps` report diffed, seed-31337 t=0 dump, four bots. Today's plans ask for
60 kW and ~189 kW, so they get the plant they always got.

## The task's premise about `MinerLine` does not hold

It was given as *"`MinerLine` draws 1,170 kW and is refused for capacity"*.
It was not. The old refusal fired when `ceil(kw / 900) > 2`, i.e. above
1,800 kW; 1,170 kW is two engines and one boiler, and sized fine before this
change. Recorded because the number was one of the two measurements that put
this work on the roadmap, and half of that case was wrong.

The other half stands and is the real one: `FurnaceLine` at 624 kW converted to
electric furnaces at 180 kW each is **4,320 kW for one yellow belt's 24
furnaces**, which used to refuse and now plans as three boilers.

Also worth saying: `method::blueprint` does not call `ensure_powered` at all
today. It checks that a block distributes its own draw and refuses one that
cannot, but nothing yet hops supply to a block's pole. So no live path reaches
the new ceiling yet — this removes the ceiling ahead of the caller, exactly as
`connect_steps` was built ahead of its first caller, with that note's warning
in mind.

## The thing the design missed: **one pole cannot power a grid**

The design listed five owners to change (`Plant`, `plant_steps`, `PLANT_COAL`,
`assemble::fuel_for` / `boiler_near`, `PIPE_COUNT`, the shore search). All five
were real. It did not list `pole_site`, and `pole_site` is what actually
stopped the first grown plant.

A `small-electric-pole` supplies a **five-by-five** area. `pole_site` asked for
one tile whose area covered **every** engine of the plant — correct and
necessary for a two-engine row, and impossible for a grid. Three boilers' worth
of engines span six tiles across and ten inland. So `pole_site` returned
`None`, `fit` refused every shoreline candidate, and the failure came out as

> `PowerPlantNeedsShore { distance: 9.5, radius: 10.0 }`

on a **two-hundred-tile clean artificial beach**. The map blamed for a fact
about the layout — the exact defect class this change exists to remove, arriving
from the inside while removing it.

`pole_site` is now `pole_chain`: one pole per boiler's engine row, each covering
that row and each within `WIRE_REACH` of the one before, so the plant is one
network. At one boiler it is one call with the old predicate and the old
result — which is why the baselines did not move.

## And then the pitch had to change, for the same reason

The first chain used a **three**-tile boiler pitch, derived from `BOILER_WATER`
the way `layout` already derives the engine pitch from `ENGINE_STEAM`: both
entries are target tiles one beyond each end of the body, so the body is the
three tiles between them, and at that pitch each boiler's far water target
lands *inside* the next boiler — the game joins them with no pipe at all. It is
elegant, it is how the engine row already works, and **it cannot be powered**.

A steam engine is three tiles wide against a three-tile pitch, so the engine
columns tile the ground with no gap. A pole can then only reach a two-engine
column from *beside* the block. Two columns have a side; the ones in the middle
do not. Measured: 1, 2 and 3 boilers site; **4 and up refuse everywhere**, on
every map, including a clean beach.

So the pitch is **four**, the raw separation of the two water targets. Boiler
`k`'s far target and boiler `k+1`'s near target are then the same tile and one
pipe joins them — which is the module's own stated joining rule (*two entities
are joined when a pipe stands on a tile both of them name*), applied along the
chain instead of across a joint. The columns leave a one-tile corridor, a pole
in it reaches the two engines on either side, consecutive corridors are four
tiles apart (inside the 7.5 wire reach), and the full 36 MW plant sites on the
shared fixture's own four-by-four lake.

The bill for that is **one pipe per boiler**: `pipe_count(n) = 1 + 2n`, with
`PIPE_COUNT = 3` still exactly the one-boiler case.

**The elegant derivation was the wrong one, and only a real caller could say
so.** Two constants in this file were derived by the same argument on the same
day; one of them was right.

## The singular assumptions, and which were real bugs

The task named `boiler_near` and `fuel_for` finding *a* boiler once there are
several. Both were fixed, and a third was found on the way:

* **`plant_steps` emitted one `Insert`** into `plant.boiler`. Fuelling one
  boiler of four leaves three cold while `Condition::Powered` credits the plan
  with the full nameplate. It now emits one per boiler.
* **`bill` and `plant_steps` already disagreed, before any chain existed.**
  `bill` acquired `PLANT_COAL * engines` and `plant_steps` inserted a flat
  `PLANT_COAL`, so a two-engine plant carried ten coal and put five of them
  nowhere. `coal_charges` is now the one place, per boiler, per engine it
  feeds, and the test asserts the bill equals the sum of the emitted inserts.
* **`assemble::boiler_near` → `boilers_near`.** The nearest boiler anchors a
  group and everything within one chain-length (`BOILER_CHAIN_SPAN`, 76 tiles at
  the four-tile pitch) of *it* joins. Measured from the nearest boiler, not from
  the anchor, deliberately: widening `BOILER_SEARCH_RADIUS` itself would sweep
  in unrelated plants — `run-1788408407-02764`'s second pump stood 86 tiles from
  its first. The network's charge is **split** across the group, not repeated.

## The refusal says whose limit it is

`plant_size_for` now refuses only above `BOILERS_PER_PUMP ×
MAX_ENGINES_PER_BOILER`, and `BOILERS_PER_PUMP` is **derived in code** as
`PUMP_WATER_PER_SECOND / BOILER_WATER_PER_SECOND` = 1,200 / 60 = 20, both
constants carrying the prototype file and the date they were read. That is a
fact about Factorio. Everything else that can stop a plant — no water in range,
no shoreline the chain fits — happens against the map and says so by name
(`PowerPlantNeedsWater`, `PowerPlantNeedsShore`). A reader told "no plant
generates that" can now tell whether to ask for less or to look at the map.

### One edit is still owed, and it is not in my boundary

`PlannerError::PowerPlantTooSmall`'s `#[diagnostic(help(...))]` in
`crates/planner/src/error.rs` still reads

> *"this is a limit of the LAYOUT, not of the game: one boiler drives at most
> two steam engines … and this planner lays out exactly one boiler in a rigid
> pump-pipes-boiler-engines row …"*

which is now **false in both directions** — the planner lays out up to twenty,
and the limit reported is the game's, not the layout's. The `#[error(...)]`
line ("the largest plant this planner lays out generates {plant_kw} kW") stays
true and now carries 36,000. `error.rs` was owned by another agent for the
duration of this branch, so the replacement is left here rather than applied:

> this is the WATER, not the layout: one offshore pump moves 1200 water/s and a
> boiler burns 60/s, so one pump carries twenty boilers of two steam engines
> each -- forty engines, ~36 MW (verified against
> base/prototypes/entity/entities.lua). The planner chains that many along the
> shore. Ask for less, or site a second plant on another pump. A plant that
> would fit no shoreline refuses separately, by name

The doc comment above the variant needs the same correction — it currently says
`Plant` "carries exactly one `boiler` position that `plant_steps` fuels once",
and that the growth "is deliberately not built here".

## Tests, falsified one break at a time

Seven tests, each confirmed to have run (`grep -c "<name> ... ok"` over the full
output) and each falsified with the failure count checked:

| break | expected red | got |
|---|---|---|
| `plant_steps` fuels only the first boiler | the fuelling test | 1 failed, that one |
| `boilers_near` takes only the nearest | the assemble top-up test | 1 failed, that one |
| `engine_split` dumps every engine on boiler 0 | the split test | 3 failed, incl. it |
| no linking pipe between boilers | the pipe-bill test | 1 failed, that one |
| the ceiling goes back to one boiler | the sizing test | 4 failed, incl. it |
| the coal charge ignores its engines | the fuelling + coal-bill tests | 2 failed, both |
| the chain steps inland, not along the shore | the pitch + overlap tests | 2 failed, both |
| every pole but the first emitted 20 tiles off | the pole-network test | 1 failed, that one |
| **`pole_chain` drops its wire-reach predicate** | the pole-network test | **green** |

The last row is the finding, and it is written into that test's own doc rather
than left implied. On open ground each row's ring search lands within a few
tiles of the previous one anyway, so `linked` is slack and nothing in the
fixture forces it to bind. The test pins the *outcome* (moving the poles apart
does fail it) and not the *predicate*. A falsification that comes back green is
the result, not a footnote —
`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`.

The two geometry tests that existed — `every_facing_puts_every_building_on_its
_own_grid` and `the_plants_own_buildings_never_overlap_each_other` — now run
over **every** size from one boiler to twenty rather than over the one-boiler
plant, which is the shape that could not have shown the chain was wrong.

## What is still open

* **Nothing draws this yet.** No live run has asked for a second boiler; every
  green run in the record reads `roster-fed` at 120 kW. The ceiling was worth
  removing and is not yet demonstrated to bind.
* **Fuel is still a one-shot charge.** `PLANT_COAL` per engine, a boiler's slot
  is one stack, and nothing monitors it. Twenty boilers is twenty walks to
  twenty fuel slots; a belt-fed plant is the obvious next thing and none of
  this addresses it.
* **The chain assumes a straight shore.** `fit` checks every part against the
  world, so a curving shoreline refuses rather than mis-building — correct, but
  a twenty-boiler chain wants eighty tiles of straight beach and no siting
  logic looks for one. `PowerPlantNeedsShore` cannot say "the shore bent"; it
  says "no candidate in ten tiles".
* **`pole_chain` places one pole per boiler.** At the four-tile pitch a single
  pole in a corridor reaches both adjacent columns, so a twenty-boiler plant
  could probably run on eleven poles rather than twenty. Not pursued: correct
  first.
