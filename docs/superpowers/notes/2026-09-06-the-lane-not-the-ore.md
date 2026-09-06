# The chain plateaued on the lane, not on the ore

2026-09-06. `ore_to_plate_why.lua`, seed 31337.

`OreToPlate` stopped at 17 plates with both furnaces holding full coal. I
recorded two candidates and said neither was established: ore exhaustion under a
burner drill's small mining area, or the ore lane jamming. **One of them was
measurable after all.**

`FactorioEntity.amount` is populated for resources, so the ore remaining under
each drill can be read before and after — which separates the two outright.

```
drill 1 at (-11,-18): 143 ore across 2 tiles it can reach
drill 2 at (-11,-14):  60 ore across 1 tile  it can reach

after the plateau
drill 1: 143 -> 117  (mined 26)
drill 2:  60 ->  40  (mined 20)

TOTAL mined from the ground: 46      still in reach: 157
plates in the furnaces:      17      unaccounted:     29
```

**Not exhaustion.** 157 ore is still reachable and the drills were still mining
when output stopped. **29 of the 46 mined ore is stranded between the drill and
the furnace**, on a belt that nothing can read.

Mining itself is healthy: 46 ore in 6,242 ticks is 0.44 ore/s against two burner
drills' 0.5 nameplate, so **88%** — the drills are not the constraint either.

## The second finding: a drill can be sited mostly off the patch

A burner drill's radius is 0.99, so it works its own 2x2 — **four** tiles. These
two reach **two** and **one**:

```
drill 1: 2 tiles      drill 2: 1 tile
```

`drills_are_fed` requires the mining area to cover *some* extractable resource,
not all of it, so a drill hanging off the edge of a patch passes. It mines at
full speed regardless — Factorio does not scale a drill's rate by how many tiles
it covers — but it **exhausts its ground four times faster than one squarely on
the patch**, and a block sited this way needs re-siting much sooner than its
nameplate suggests. Worth knowing before anyone reads "the drills stopped" as a
throughput fact.

## What this makes of the belt blind spot

Three separate questions this session have ended at the same wall, and this is
the first where it blocks a *diagnosis* rather than a convenience:

| question | what could not be seen |
|---|---|
| `BurnerMinerLine` at 0.80 of nameplate | whether the sink arm or the drills were the limit |
| `ElectricSmelter` first run, 0 in the sink | whether plates had reached the belt at all |
| this plateau | where 29 ore is sitting |

Nothing in the mod exposes a transport line's contents, so "the belt backed up"
is an inference in all three. It is also the case a rate model gets wrong by
construction: throughput in equals throughput out here, and the line is stalled.

## Also measured: the build is not deterministic, and walking first fixes it

The same blueprint on the same seed lost a placement on one run and built
cleanly on the next. Planning is pure, so the plan can be made first, read for
where the block will land, and the bots walked there before building — the fix
that cured the saturated smelter's losses. With it, a clean build came first
attempt.

It works by putting the bots **near the site**, not by changing the ground. That
distinction was retracted once today and is worth keeping straight.
