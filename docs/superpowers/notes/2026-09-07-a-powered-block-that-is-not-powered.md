# A powered block that is not powered

*2026-09-07. Handoff to whoever owns `method::power`. Two runs, two different
failures, and the more worrying one is the run that SUCCEEDED.*

## What was being attempted

An electric smelting block: ore and coal on one belt, **electric** inserters
feeding two stone furnaces, and — for the first time here — electric arms taking
plates *out* onto an output belt into a chest.

Every burner block in this project dead-ends at the furnace, because a burner
arm carrying plates has nothing to fuel itself with. That is the whole reason
the last plate rate had to be measured with a script-side drain standing in for
an output side, and why blocks cap at 200 plates (two full furnace stacks).

The block does not cheat power. It asks the planner for it.

## Run A: the planner built a plant, reported success, and the block was dead

`ElectricOreToPlate`, sited with no hint.

```
pass 1: 63 to place          <- 46 block + 17 the planner added for a plant
pass 1: done=true failed=0 pending=0
PLANT: 1 boiler, 1 engine, 1 pump
plates after 30,000 ticks: 0
coal consumed: 0 of 200
```

`ensure_powered` returned `Some` — it believed it had powered the block — and
**nothing in the block ever moved.** Not one inserter swung.

The plant landed at **(45.0, −5.5)**; the block was around **(−2.5, −14.5)**.
About 48 tiles apart. I fuelled the boiler by hand on a later run (50 coal, in
case an unfuelled boiler was the whole story) and the result was identical:
still zero.

**This is the failure worth attention.** A refusal is cheap; a block that
reports 63 of 63 placed, zero failures, and produces nothing is the shape this
project has repeatedly paid for — the planner's model and the game disagreeing,
with every signal on our side saying success.

## Run B: sited near the water, the planner refused — correctly and precisely

Same design, drills removed (see below), sited `near = (45, −5)`:

```
blueprint refused: the block draws 78 kW and supply exists, but no run of
poles this planner will build carries it to small-electric-pole at [41.5, -5.5]
```

That refusal is **good**. It names the draw, says supply was found, says what
could not be done, and names the entity and tile it could not reach. Nothing
was half-built. This is `ensure_powered` returning `Ok(None)` and
`method::blueprint` reporting it.

## What is established and what is not

**Established:**

- The planner sites and builds a plant for a `Goal::Built` block unprompted.
- With no siting hint it reports success and the block does not run.
- Sited near the water it refuses, naming the pole run as the thing it cannot
  do.
- The block itself builds cleanly every time: 63 of 63, zero failures.

**Not established, and I want to be explicit because I got it wrong once
already:** *why*. My first reading was "`ensure_powered` does one hop", and that
is **false** — `MAX_POLE_RUN` is 64 poles at `POLE_STEP` 6, about 380 tiles, far
more than the ~48 needed. So the bound is not the explanation and I have not
found the real one.

Two candidates, neither tested:

1. **The pole run cannot route.** The plant must sit at water and the run may
   have to cross it. `POLE_STEP` arithmetic and the union-find check are
   different tests, and this module's own doc already notes a case its
   arithmetic thought fine and the game refused.
2. **The run is planned and the game disagrees.** Run A is consistent with poles
   the planner believes connect and the game does not — which would make it the
   same class as the tile-parity bug found this morning: model and game each
   self-consistent, disagreeing at the boundary, invisible to any fixture.

Run A versus Run B is the useful pair: **the same planner, on the same seed,
returns `Some` in one siting and `Ok(None)` in another**, and the `Some` is the
one that produces a dead block.

## A constraint that shapes any fix

On seed 31337 a self-powered mining block is **not sitable**: drills must stand
on ore (iron at 18.4 from spawn) and a plant must reach water (48.1), and no
anchor satisfies both. That is why Run B has no drills — it was the only way to
get a block that could sit beside the water at all.

So "power a block" and "mine with a block" are, on this map, two blocks and a
run of poles between them. Any fix that assumes plant and block are adjacent
will work on a fixture and not here.

## Why this was reached at all, which is worth recording

I went looking for better block designs — drill-to-furnace ratios, output
sides, coverage. That was the wrong layer, and the owner said so: the ratios are
arithmetic off two prototype numbers that any Factorio player knows, the blocks
were 2-4 furnaces against a belt-saturating target of 48, and a good layout is
something a person encodes once. What the planner has to do is build, power and
reason about *whatever* layout it is handed.

The experiments were worth their cost only because of the planner defects they
turned up — tile parity, anchor crosstalk, per-type pole reach, and now this.
The designs themselves were not the contribution and should not be mistaken for
one.
