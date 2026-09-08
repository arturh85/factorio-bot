# Which port takes which fluid — ask the game, it answers

2026-09-08, branch `which-port-takes-which-fluid`, seed 31337, Factorio 2.1.17
Space Age, headless debug build, isolated instance (`workspace/headless-a.toml`,
rcon 4330 / game 34210).

## The wall

`have:sulfur:10` refused, and through it so did `researched:rocket-silo`,
`have:rocket-silo:1`, `have:rocket-part:1`, `have:low-density-structure:1` and
`have:rocket-fuel:1`:

> sulfur runs in chemical-plant and takes 2 fluids in — water and
> petroleum-gas — and no field this planner receives says which of the
> chemical-plant's input fluidboxes accepts which, so a pipe would be a guess

The prototype genuinely cannot say: last night's fluidbox work measured a
chemical plant's two input boxes as both `Any`.

## The experiment, and it took thirty seconds

Hold a headless game open, place the machine, set the recipe, read
`LuaEntity::get_fluid_filter(index)` back off the standing entity:

```
BEFORE set_recipe: fluids_count=4
 box1 prod=input  filter=nil
 box2 prod=input  filter=nil
 box3 prod=output filter=nil
 box4 prod=output filter=nil
AFTER  set_recipe: recipe=sulfur fluids_count=2
 box1 prod=input filter=water          at (-1,-1)
 box2 prod=input filter=petroleum-gas  at ( 1,-1)
recipe ingredients in order = fluid:water:30, fluid:petroleum-gas:30
```

**The game says.** A recipe-set crafting machine has concrete fluidbox filters,
and `set_fluid_filter`'s own doc says why — *"some entities cannot have their
fluidbox filter set, notably fluid wagons and crafting machines"*, because the
recipe already decided.

Note `fluids_count` **drops from 4 to 2**: the boxes a recipe does not use stop
existing, so a live index is not a prototype index.

## All thirteen multi-fluid recipes, swept

Three rules came out, and **the second one is not what this crate believed**.

**Rule 1 — an explicit `fluidbox_index` wins, and it is 1-based *within the
production type*.** `basic-oil-processing` declares `fluidbox_index = 2` for
crude oil and `= 3` for petroleum-gas. A live refinery set to it reports crude
on input box **2** at offset `(1,2)` and petroleum on output box **3** at
`(2,-2)` — the refinery's second *input* and third *output*, not its second and
third fluidboxes (box 3 is the first output). Its first input box, at `(-1,2)`,
carries nothing at all.

**Rule 2 — when the counts match, positional is exact.** Eleven recipes, every
one the *k*th fluid on the *k*th box: `sulfur`, `advanced-oil-processing`,
`coal-liquefaction`, `casting-low-density-structure`,
`concrete-from-molten-iron`, `electrolyte`, `electromagnetic-science-pack`,
`heavy-oil-cracking`, `light-oil-cracking`, `lithium`,
`solid-fuel-from-ammonia`, `ammonia-rocket-fuel`.

**Rule 3 — otherwise the game MERGES surplus boxes, and not positionally.** A
`cryogenic-plant` running `fluoroketone` has three input boxes and two fluids:
it merges boxes **1 and 2** for fluorine and gives ammonia box **3**. A
positional rule pipes ammonia into the fluorine box. Fluid #1 landed on box 0 in
every case measured, merged or not, so it is answered and everything after it
refuses.

## The premise I was given was right, and the one this crate held was wrong

The owner's hypothesis — *"a machine with a recipe set has concrete fluidbox
filters"* — is **confirmed**, and he was right to say test it first.

What did not survive is `PipeEnd::port_index`'s own doc, which asserted rule 2
as universal *and named `basic-oil-processing` as the example*:

> What the game does is assign a recipe's fluid ingredients to the machine's
> input boxes **in order** … So the nth fluid of the recipe belongs to the nth
> box of that direction

That recipe is the counter-example. **So the already-planning oil rig — the
2,295-action `gathered:crude-oil` + `Produced{petroleum-gas}` composition — was
piping a refinery to a port the game leaves empty**, on both the input and the
output side, silently. Nobody had built it live (the bots died walking to the
field), so it had never had a chance to fail visibly. This is the third time
this repo has met the shape: a placement that is 100% correct and moves nothing,
after inserter direction and after belt lane loading.

## What landed

- **`fluidbox_index` crosses the bridge** — `serialize_ingredient` and
  `serialize_product` in the mod, `FactorioIngredient` / `FactorioProduct` in
  core. Verified end to end by dumping a live world: exactly **3 of 662
  recipes** carry it (`basic-oil-processing` ×2, `simple-coal-liquefaction`
  ×1), `sulfur` correctly carries none, and absence stays `None` rather than
  becoming `1`.
- **`pipe::fluid_box_ordinals`** states the three rules and returns one box
  ordinal per fluid, or the first fluid it cannot decide.
- **`FluidPort::box_ordinal`**, and `select` filters on it. `port_index` now
  means the fluidbox it always claimed to mean rather than a position in a list
  that a multi-connection box shifts — 21 of 56 boxes on a live capture declare
  more than one connection, though no crafting machine among them.
- **`plan_fluid_rig` lays one run per fluid ingredient**, each to its own box,
  each routed against the ground the previous ones took. `FluidRig.inbound` is
  a `Vec`.
- **`ManyFluidIngredients` → `FluidBoxUndecidable`**, which fires only for
  rule 3 and names the fluid.

## Where the ladder stands now

Wall 1 is closed and every rung moved to the *same* refusal, which is a
composition problem rather than a planner gap:

| goal | before | after |
|---|---|---|
| `have:sulfur:10` | which fluidbox? | nothing standing supplies **water** |
| `researched:rocket-silo` | which fluidbox? | nothing standing supplies water |
| `have:rocket-silo:1` | which fluidbox? | nothing standing supplies water |
| `have:rocket-part:1` | which fluidbox? | nothing standing supplies water |
| `have:low-density-structure:1` | which fluidbox? | nothing standing supplies water |
| `have:rocket-fuel:1` | which fluidbox? | nothing standing supplies water |
| `have:plastic-bar:10` | nothing supplies petroleum | unchanged |
| `have:processing-unit:1` | 40 recipes produce it | unchanged |

**Nothing new plans on its own, and that is the honest headline.** What changed
is the *kind* of refusal: seven of eight rungs now fail on a missing standing
supply, which composition answers, instead of on a fact the planner could not
represent.

## Wall 2 is a goal-authoring problem, and that is measured

`have:plastic-bar:10` alone refuses because nothing standing supplies
petroleum-gas. Composed, it does not:

```
factorio-bot plan --world map-31337-explored-with-categories.json \
  --goal gathered:crude-oil \
  --goal produced:petroleum-gas:100:basic-oil-processing \
  --goal have:plastic-bar:10
```

reaches **pipe geometry** — *"no pipe route from the oil-refinery at
[137.5, -352.5] to the chemical-plant at [139.5, -358.5]"* — not "nothing
supplies petroleum". The composition works; what is left is siting two fluid
machines near each other, which is a different and much smaller problem than
teaching `Have` to build fluid producers. **Do not build that.**

The sulfur equivalent needs `gathered:water` as well, and on
`map-31337-explored-with-categories.json` that refuses honestly: *"no water is
charted anywhere this plan can see"*. The explored dump was charted **toward
oil**, 372 tiles from spawn, and there is water near spawn on this seed
(48.1 tiles). **A dump that has both charted does not exist yet**, and making
one is the next cheap step for anybody continuing this.

## Two things worth carrying

**A falsification found an accidental pass, again.** A mutation sending *both*
inbound runs to input box 0 left `the_two_runs_do_not_share_a_port` green: the
test asked which ports the plan's pipes overlap over all pipes at once, and the
refinery's two input ports are two tiles apart on one edge, so the second run's
route lies across the first port's tile on its way past. **"Which port did this
run aim at" is not answerable from a flat list of tiles.** The pipes are now
partitioned by the fluid in their own `Place` label. Eleven mutations in all;
that was the only green.

**And the harness itself lied once, in the direction that matters.** Its
classifier tested "0 tests passed" *before* "any target FAILED", so a genuine
RED was reported as *"NO TEST MATCHED — the harness proved nothing"*. It also
kept one shared backup path for every file, so restoring after a `fabricate.rs`
mutation wrote `fabricate.rs` over `pipe.rs`. Both were caught only because the
work was **committed before the sweep**, exactly as this repo's rule says.
