# What a beacon lane costs

2026-09-06. `scripts/beacon_lane_cost.lua`, seed 31337, planning only.

The owner wants space left for beacons from the beginning, so production can be
improved later without a teardown — electric smelters already force one, and
that is the repetition he is trying to avoid.

## The number that decides the spacing does not reach us

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
