# What a beacon lane costs

2026-09-06. `scripts/beacon_lane_cost.lua`, seed 31337, planning only.

The owner wants space left for beacons from the beginning, so production can be
improved later without a teardown — electric smelters already force one, and
that is the repetition he is trying to avoid.

## Resolved: the lane is 3 tiles, and it was derivable all along

**The cheapest usable lane is the beacon's own footprint.** With machines flush
against it the reach condition is `0 < d`, true of any beacon that supplies
anything, so `supply_area_distance` only bounds how much *wider* a lane could
usefully be. The footprint comes from `collision_box`, which we always had:

```
beacon collision box 2.398  ->  ceil = 3 tiles
```

**`ceil`, not `round`** — and I got this wrong first, on a rule this repo
already records for drills: *compare in TILES, never in collision-box extents*.
`round(2.398)` is 2 and the beacon is 3x3. The same arithmetic makes a stone
furnace 1x1 instead of 2x2, so it is not a beacon-specific slip. A box of
+/-1.2 centred on a tile centre spans -0.7..1.7, crossing three tile
boundaries.

**So a 3-tile lane, and the cost table below already covers it**: gap 3 sites at
(-16,-20) with 37 placements, against 33 for no lane. Four extra belt tiles buys
the reservation.

## Two things that still do not reach us

`uses_beacon_effects` is **not a field we receive** — it appears nowhere in
`crates/` or `mods/`. So "every machine we build today ignores beacons, and the
two that do not are in-place upgrades" is true of the game and is **knowledge
our planner cannot check**. Anything that gates on it would be gating on a
constant someone typed.

And the test fixture `live-2.1.17-world-snapshot.json` predates the new fields:
`supply_area_distance` appears zero times in it. Live runs carry it, captured
fixtures do not, so an offline test cannot assert on it yet.

## The original blocker, kept for the record



```
beacon prototype fields:
  collision_box, collision_mask, entity_type, mine_result, mining_time, name
  collision_box: +/-1.19921875     (a 3x3 footprint -- that much we have)
  supply_area_distance:      ABSENT
  distribution_effectivity:  ABSENT
```

`grep` finds no `supply_area`, `distribution_effectivity` or `get_radius`
anywhere in the mod, and `FactorioEntityPrototype` carries nothing
beacon-shaped. **So the row spacing a beacon needs is unknown here**, and
picking one from memory is the hard-coded-rate defect in another hat. The field
is being added; until it lands the spacing is parked.

## A lane only pays if machines flank it on both sides

Worth stating because my own blocks do not qualify. A beacon between two machine
rows reaches both; a beacon beside a single row wastes half its area. Every
block here — `TJunctionSmelter`, `OreToPlateTee` — has **one** furnace column
fed by arms off a single stem, and an inserter reaches exactly one tile, so a
second column across a 3-tile lane cannot be fed from the same stem.

So the reservation belongs in a **two-column** design that does not exist yet,
and this measurement prices the ground for it rather than retrofitting the
blocks that exist.

## So measure the half that does not need it

A reserved lane is not empty space in a blueprint — blueprints hold only
entities. It is **the machines moving apart**, which grows the footprint, and a
bigger footprint can refuse siting on ground that works today. An inserter
reaches exactly one tile and cannot span the lane, so each extra tile also needs
another belt tile to carry ore across it.

`OreToPlateTee` with the furnace column pushed east by 0..6 tiles, each sited
with `Site::Anywhere` — which seeds on ore, the constrained case:

| gap | footprint | sited | placements |
|---|---|---|---|
| 0 | 10x13 | (-17,-20) | 33 |
| 1 | 10x13 | (-17,-20) | 35 |
| 2 | 10x13 | (-16,-20) | 35 |
| 3 | 10x13 | (-16,-20) | 37 |
| 4 | 11x13 | (-16,-20) | 39 |
| 5 | 12x13 | (-16,-20) | 41 |
| 6 | 13x13 | (-17,-20) | 43 |

**Seven of seven site. None refused.**

## What it costs, then

**Not siting.** Up to six reserved tiles, the block still lands in the same
place. The cost is **materials and ground**: two extra belt tiles per gap tile,
one for each furnace row, so 33 placements become 43 across the range.

That prices the decision without knowing the beacon's supply area: whatever it
turns out to be, up to six tiles is free of siting risk and costs about 1.7
entities per tile of lane.

## Bounds on that claim

- **One map.** Seed 31337, one ore patch, `Site::Anywhere` seeded on ore. A
  tighter map could refuse where this does not.
- **One block shape.** `OreToPlateTee`, 2 drills and 2 furnaces. A 48-furnace
  line is a much larger footprint and this says nothing about it.
- **Up to 6 tiles.** Not tested further; the refusal boundary was never reached,
  so the honest statement is "no cost found up to 6", not "no cost".
- **Planning only.** `goal.plan` is pure, so nothing was built and this measures
  siting, not building. The build is separately known to be non-deterministic.
