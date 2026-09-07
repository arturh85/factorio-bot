# The ceiling was the output slot, and the real rate is 28.7/min

*2026-09-07. The first production number this project has published that is not
secretly a measurement of storage.*

## Two runs of the same block, differing in one thing

```
                     plates in 30,000 ticks   shape of the curve
nothing empties it            200 (24.0/min)   peaks 28.0 then decays to 24.0
furnaces drained              239 (28.7/min)   climbs to 28.7 and HOLDS
```

Reproduced twice, identical to the plate.

`OreToPlateTee` has no output side — a burner arm carries coal and ore, never
plates, so nothing takes them away. A stone furnace's result slot holds 100, the
block has two furnaces, and 200 is where it stops. **Every window long enough to
approach that is measuring the slot rather than the block.**

The shape is the better evidence than the total. Undrained, the rate *decays* —
each furnace stalls as its own slot fills, so throughput bleeds away rather than
stopping cleanly. Drained, it climbs to a flat 28.7 and stays there, which is
what a steady state looks like.

## What it costs

**16% of throughput** (24.0 against 28.7) and, worse, it turns a measurable
steady rate into a decaying one — so the *shape* of every previous plate curve
from this block was an artefact of its storage, not a property of its design.

## Where the remaining headroom is

Two stone furnaces at full rate are `2 × 60/3.2 = 37.5` plates/min.
**28.7 is 77% of that.** So even with the output unblocked, the furnaces idle
about a quarter of the time, and the constraint moves upstream to supply:

- `DRILL COVERAGE: 6 of 8 possible tiles` — one drill has all four of its
  tiles, the other has two. A burner drill works exactly its own 2x2, so the
  second drill mines at half the ground of the first.
- 986 ore was in reach against ~240 consumed, so this is not exhaustion.

That is the siting-quality gap documented on 2026-09-06 and deliberately left
open.

## CORRECTION: coverage was not where the difference lived

Siting now prefers the better-covered of equally legal sites, and the coverage
duly improved — **6 of 8 tiles to 8 of 8, ore in reach 986 to 1694.** The rate
did not move at all: **239 plates, 28.7/min, identical to the plate.**

Which is exactly what the older note about `drills_are_fed` says and I did not
apply here: *a drill on one ore tile mines at the full nameplate rate, because
Factorio does not scale a drill's speed by how many tiles it covers.* Coverage
buys **longevity, not throughput**. The block now runs the same rate for
roughly twice as long, which is worth having and is not what I said it would be.

The real constraint is arithmetic, from the game's own prototypes:

```
burner-mining-drill  mining_speed 0.25/s  ->  2 drills  = 30.0 ore/min
stone-furnace        crafting_speed 1     ->  2 furnaces = 37.5 plates/min
measured 28.7/min    = 96% of DRILL capacity, 77% of furnace capacity
```

**The block is drill-limited and running at 96% of what its drills can
deliver.** The furnaces are over-provisioned; the 23% I attributed to coverage
was the mismatch between two drills and two furnaces, nothing to do with which
tiles the drills sat on. One stone furnace consumes 18.75 ore/min and one burner
drill supplies 15, so the honest ratio is **five drills to four furnaces**, and
this block has two and two.

So the way to make it faster is more drills per furnace. Nothing else in the
block is close to its ceiling.

## The apparatus, disclosed

The drain is `rcon.remove_from_inventory` on the furnaces' result slot
(`inventory_type` 3) at each sampling mark. **It is apparatus, not a fix.** A
real block would put an electric inserter onto a belt, which needs
`electronics` — and this project has already shown a burner block earning that
from its own copper in about 37 seconds of game time.

Standing in for the output side rather than building it was the cheaper order:
it produces the number that says whether the real thing is worth building, and
the answer is yes but modestly — 16%, with a further 23% waiting upstream.

`remove_from_inventory` had **no caller anywhere in the tree** before this. Its
`inventory_type` indices had therefore never been exercised outside
`insert_to_inventory`, which is exactly the shape that has hidden three defects
here this week.

## The method note

**Diagnosing before treating changed the answer twice in one day.**

The parity fix was real, correct, and moved no plates. Had I gone straight from
"drills are being refused" to "fix drill coverage", I would have shipped a
second real fix that also moved nothing — because the block would still have
stopped at 200. Measuring the ceiling first said the order was: output, then
coverage.

That ordering was only visible because the *total* and the *shape* disagreed.
200 looked like a plateau, which reads as "supply ran out". The decaying curve
underneath it was the tell, and a single terminal number would have hidden it
completely.
