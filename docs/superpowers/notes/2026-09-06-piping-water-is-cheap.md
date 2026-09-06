# Piping water is cheap, and the siting objective is the belted distance

2026-09-06. Branch `water-moves-and-pipe-is-cheap`.

`crates/planner/src/method/power.rs` opened with a paragraph that justified
building every power plant on a shoreline, and it was wrong twice. Correcting
it turns shoreline adjacency from a physical law into one term of a cost
comparison — and once coal is priced beside water and power, the objective
that falls out is not the one the module has.

This note records the corrected prices, the design of the weighted objective,
and exactly what it would take to build. **It is a design, not a
half-implementation.** What landed on this branch is the price correction, the
costing functions, and a world-anchored fallback for plant siting. The
objective below is not built.

## 1. The retracted premise

> *"Water is the one input that cannot be moved. Coal is five items in an
> inventory. Siting the plant at the coal and running pipe to the lake costs
> `pipe-to-ground` at 15 iron plates per 10 tiles — a 60-tile separation
> roughly doubles rung 7's whole iron bill, which is about 98 plates. Siting it
> at the water costs one walk."*

**Water moves. Through pipes.** That is what pipes are for.

**And the price was the wrong item's, and misread.** From
`workspace/server/data/base/prototypes/recipe.lua` and
`.../entity/entities.lua`:

| recipe | ingredients | yields | span | iron plates per tile |
|---|---|---|---|---|
| `pipe` | 1 iron-plate | **1** | 1 tile | **1.00** |
| `pipe-to-ground` | 10 pipe + 5 iron-plate | **2** | 12 tiles (`max_underground_distance = 10`, plus the two ends) | 1.25 |

So the paragraph quoted the *dearer* item as the price of piping, and even that
number was misread: 15 plates buys 12 tiles, not 10. `pipe-to-ground` exists to
**cross an obstacle**, not to save material. A 60-tile pipe run is **60 iron
plates**, not 90.

The owner's judgement, now the module's premise: *"piping water is not
expensive! its usually a better option than restricting ourselves to places
with water."*

## 2. The three transports, priced

All three from `recipe.lua`, asserted in
`method::power::capacity_tests::the_three_transports_are_the_recipes_own`:

| move | recipe | per tile |
|---|---|---|
| **power**, `small-electric-pole` | 1 wood + 2 copper-cable → **2** poles; `maximum_wire_distance` 7.5, so ~7 tiles apart | ~0.07 wood + ~0.07 copper plate |
| **water**, `pipe` | 1 iron-plate → 1 pipe, one tile | **1.0 iron plate** |
| **coal**, `transport-belt` | 1 iron-plate + 1 iron-gear-wheel (= 2 plates) → **2** belts | **1.5 iron plates** |

`copper-cable` is 1 copper-plate → **2** cable, so one pole craft is one copper
plate, not two. Both doublings (poles, belts) are the easiest thing to get
wrong here and each halves a bill.

**Belt is the dearest, power is nearly free.** The rule:
**wire the power, pipe the water, do not move the coal.**

### The crossover, honestly

There is **no material crossover** between pipe and pole: poles are ~7x cheaper
per tile at every distance. Any claim that piping is ruled out *on price* is
false in the other direction too.

What binds the pole route is **supply**. Wood is the one item this planner
cannot make — a four-bot run starts with four and no method in `crates/planner`
mines a tree — so four wood is eight poles is about **56 tiles of wire, ever**,
and poles are wanted elsewhere. Iron plate is what a run mines by the hundred,
so the pipe route has no ceiling: the 355-tile separation §4 is about is 355
plates of pipe, expensive but buildable, against 51 poles a four-wood run
cannot craft.

So the crossover is a **supply** crossover at ~56 tiles, and it is an artefact
of this planner rather than of the game. Teach it to mine a tree and the pole
route wins everywhere on materials.

## 3. The weighted objective (designed, not built)

Today the plant is sited on a shoreline and the pole run carries the power out.
Under the prices above the objective should instead be:

    minimise   1.5 * belted(coal -> boiler)
             + 1.0 * piped(water -> boiler)
             + 0.14 * wired(pole run to every consumer)

subject to the pump standing on a legal shoreline, and to the wood budget on
the wire term.

### The regime switch that must not be lost

**In today's bootstrap the coal is not belted.** `PLANT_COAL` is five coal a
bot carries and inserts once. While that holds the belt term is **zero**, the
objective collapses to "pipe the water, wire the power", and siting at the
water is free and correct. **Today's behaviour is not wrong.**

The owner's argument becomes decisive the moment coal delivery is *automated* —
where the self-feeding cell work is heading. So the siting function wants to
know **whether this plant's coal is belted or carried** and pick accordingly.
Hard-coding either answer is the failure mode: a siting rule that assumes belts
regresses today's runs, and one that assumes carriage silently wastes the
factory's iron later.

The signal is not currently expressible. `plan_plant_for` takes
`(state, from, kw)` and nothing says how this plant will be fuelled; the fuel
decision is made afterwards, in `plant_steps` and in `method::assemble`'s
`fuel_for`. Threading it needs a `FuelRegime { Carried, Belted { from: Position } }`
argument, which is a signature change across every caller.

### What building it would take

1. **A pipe router.** `graph::route::route_belt` in `crates/core` routes a
   belt run with obstacle avoidance; a pipe run is the same problem with a
   different prototype and no direction semantics (a pipe is undirected, which
   makes it *easier* than a belt). Reuse, do not rewrite. `method::connect` is
   the shape to follow, including its promise to **refuse before placing
   anything**: a half-built pipe run is a plant that never runs.
2. **A layout that is no longer a rigid body.** `layout()` hangs the whole
   plant off the pump's tile and facing, which is what keeps every building on
   its own build grid at all four facings. Splitting the pump from the boiler
   means two anchors and a route between them, and `PIPE_COUNT = 3` stops being
   a constant — it becomes the route's length. Several tests assert that
   constant by name.
3. **Two sites, not one.** Siting becomes: choose a shoreline for the pump,
   choose a boiler site near the coal, and route between them — with the
   objective above scoring the pair. The candidate set is the expensive part;
   the honest cheap version is to keep the shoreline ring search and add a
   bounded ring search around the coal patch, scoring their cross product.
4. **The fuel regime argument**, above.
5. **A wood budget on the wire term**, so the objective cannot choose a pole
   run the roster cannot craft. `method::have`'s shortfall machinery already
   refuses on materials; the objective should not propose what it will refuse.

Estimated: this is a multi-session change touching `layout`, `fit`,
`plan_plant_for`, `plant_steps` and every caller's signature, plus a new router
in `crates/core`. That is why it is written down instead.

## 4. The defect that was blocking all of it

**Plant siting was anchored on the caller, and the caller moves.**

Measured 2026-09-06, same seed, same binary, two dumps of the same map:

| dump | bots at | water from there | `researched:automation` |
|---|---|---|---|
| `workspace/scripts/map.json` (t=0) | `(0.5, -0.5)` | 48 tiles | plans, 176 actions / 21,784 ticks |
| `workspace/scripts/map-31337-explored.json` | `(255, 249)` | 355 tiles | **refuses** |

After the fix, on the same binary, that second row plans: **183 actions /
37,034 ticks**. Longer than the t=0 plan, and correctly so — the plant is on
the lake near spawn and the bots are 355 tiles away, so the walk is real and
`schedule` charges it. A worse plan is what a makespan is for; a refusal was
not.

`score-map` reports water at **48.1 tiles on both dumps**. The lake did not
move; the bots did, having finished an exploration ring and parked. Every
power-needing goal on the better-charted map then refused with *"a power plant
needs water, and the plan can see none within 128 tiles"* — a true statement
about what was looked at, and a false impression of the map.

**A lake is a property of the world; a bot's position is a property of its walk
history.** Both callers pass something that moves: `method::have` passes the
acting bot, `method::extract` passes the machine site it just chose.

### The fix that landed

`supply_for` now falls back to a **world-anchored** search when the
caller-anchored one refuses: `plant_world_anchor()` is the spawn tile, and the
retry re-runs the standing-network, complete-a-plant and build tiers from
there. Fixed in `supply_for` rather than in either caller, because that is the
one place the choice can be made once — lab, extractor, and any block that
needs power.

Four properties, each with a test:

* the caller-anchored search must genuinely fail from the parked position
  (the control, without which the next test asserts nothing);
* a roster that walked away still gets a plant, on the lake the origin sees;
* a dry world still refuses by name — the fallback **finds** water, it does not
  invent it;
* `PowerPlantTooSmall` is **not** retried: it is a statement about the demand,
  true from every anchor, and retrying it would read a quarter of a million
  terrain tiles to reach the identical error;
* a caller standing beside water is untouched — the fallback is a fallback.

It is only viable because `PlanState::electric_supply_kw` now **follows the
wire** out from the consumer rather than searching one 64-tile disc, so a pole
run of any length carries power the planner can see. Before that, this fallback
would have sited a plant the consumer could not be shown to draw from.

### One test the fix invalidated, and what that says

`crates/scripting_lua`'s
`a_research_that_cannot_reach_the_water_raises_a_recognisable_refusal` stood a
bot 240 tiles from the shared fixture's lake and asserted a refusal. It now
plans, correctly: the lake is 57 tiles from the origin whatever the bot did.

**The test was pinning the classification seam, not the geometry**, so the
fixture was changed rather than the assertion — `fixture_world_without_water()`
in `crates/core/src/test_utils.rs` builds the same world with no lake in it,
because `update_chunk_tiles` is additive and water cannot be removed from a
world that has some. The unreachable case is now exactly one thing: a map with
no water on it.

Worth naming as a general fact: **"the bot is far from X" stopped being a way
to make X unreachable.** Any other test that manufactures a refusal by walking
a bot away is now asserting something weaker than it thinks.

### The residual

The world-anchored retry is still a **radius** query —
`nearest_water_tile(origin, 128)` — not a true nearest-charted-water query. It
happens to be enough on seed 31337, where water is 48 tiles from spawn, and it
is a read-cost bound with a derivation (`PLANT_WATER_WIDE_SCAN_RADIUS`). A map
whose nearest water is 200 tiles from spawn would still refuse, and would still
say "within 128 tiles" without saying *of where*. Naming the anchor in
`PlannerError::PowerPlantNeedsWater` is a one-field change nobody has made yet;
it is what would have made the measurement above take a minute instead of an
hour.

## 5. What did not change

The three offline baselines on `workspace/scripts/map.json` are byte-identical
before and after, on the same release binary built from this branch:

| goal | actions | ticks |
|---|---|---|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,463 |
| `producing:logistic-science-pack:6` | 442 | 47,542 |

They must be: on that dump the bots stand at the origin, so the local search
wins on every call and the fallback never runs. The change is visible only
where the old code refused.


---

## CORRECTION (2026-09-06, from the owner): the supply crossover does not exist

The note above derives a **supply crossover at ~56 tiles** from the claim that
wood is unmakeable — four bots, four wood, eight poles, ever. **That claim is
false, and it was already false when this note was written.**

**1. The planner can chop.** `crates/planner/src/method/have.rs` carries a
`Chop` method — swinging at any standing minable entity, trees and rocks alike
— and its own doc records the exact history this note reproduced: *"before this
method every wood in a run was wood a bot had been holding since it spawned:
four bots, four wood ... That cap was a property of this model and of nothing
else."* A live run halted on it (`run-1788396958-07935`) and the method exists
to remove it. So the constraint was real, was read from a real place, and had
been lifted in a different file the pricing never consulted.

**2. And wood is a tier-one artefact anyway.** The owner, who plays the game:
*"wood is only needed for the very first tier of power poles, we will quickly
research the better tiers which don't need wood at all."* Confirmed from
`recipe.lua`:

| pole | ingredients | wire reach |
|---|---|---|
| `small-electric-pole` | 1 wood + 2 copper-cable -> **2** | 7.5 |
| `medium-electric-pole` | 4 iron-stick + 2 steel + 2 copper-cable -> 1 | **9.0** |
| `big-electric-pole` | 8 iron-stick + 5 steel + 4 copper-cable -> 1 | 30.0 |

No wood past tier one, and a longer reach. `electric-energy-distribution-1`
(120 red + green) unlocks the medium pole and `iron-stick`, and it is already
**on the oil ladder** — so by the time power is being run to a well 350 tiles
out, the pole that needs no wood is available.

**So "poles are supply-capped and pipe wins beyond 56 tiles" should be read as
"you are still holding the starting pole".** It is a research problem, not a
logistics one. The per-tile prices in the table above stand; the conclusion
drawn from them does not.

## And the siting problem is smaller than this note assumes

The owner on how oil is actually played: *"usually one would at least have one
fluid storage container thingy between the pumpjacks and the refineries. and
yes we want to pump it home, or to a dedicated refining area."*

So a wellhead needs a **pumpjack, a storage tank, and two long runs** — power
out on poles, crude back through pipe. It does **not** need a refinery, and
therefore does not need water, and therefore **does not need a power plant near
the well at all**. The refusal this whole lane has been fighing — *"a power
plant needs water, and the plan can see none within 128 tiles"* — is the
planner solving a problem real play does not have: it tried to site a plant at
the well because that is where the consumer was.

The plant stays where the water is. The distance is crossed twice, by two
different carriers, and both are cheap.

**A consequence for the fluid work:** the tank is not an optimisation, it is
the first thing that must exist before crude can be represented at all, since
no character inventory can hold a fluid (see
`2026-09-06-a-fluid-is-not-an-item.md`).
