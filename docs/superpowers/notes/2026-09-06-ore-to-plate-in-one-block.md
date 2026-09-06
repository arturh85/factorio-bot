# Ore to plate in one block

2026-09-06. `OreToPlate`, 24 entities, headless 2 bots at 5x, seed 31337.

Three blocks had each been proven alone. This is the first time they have run as
one chain, in a single blueprint, so **ore never touches a chest between the
ground and the plate**.

```
drills at (-11,-18) (-11,-14); furnaces at (-7,-9) (-7,-6)
under the first drill: iron-ore x16
>> FIRST PLATE at tick 1855
PLATES: 13 + 4 = 17     coal still in the furnaces: 5 / 5
build: done=true failed=0 lost=0 pending=0
```

**Not one item was cheated into the flow.** Coal is charged into the block's own
coal chest and into the drills, as fuel; the ore comes out of the ground.

## The lane arithmetic is the design

- Drills sit **west** of the belt facing east. A drill drops onto the **far**
  lane, which from the west is the **east** lane.
- Coal joins from the **west** as a T-junction, and a sideload fills the
  **near** lane — from the west, the **west** lane.
- The furnace arms sit **east** picking west, so they meet the far (coal) lane
  first and fall back to the near (ore) lane once a fuel slot fills.

Two commodities, two lanes, one belt, and nothing merging them but the belts.

## Siting had to satisfy two constraints at once

Built with **no site**, so `Site::Anywhere` reaches `nearest_ore_seed` and the
drills force the block onto ore. That makes the **furnaces** sited by the ore
rather than by clear ground, which no previous block here required — every other
one picked its own empty spot. It worked first try.

## It plateaued, and I do not know why

Production stopped at 17 plates with **both furnaces holding a full 5 coal**, so
the coal lane was fine and the **ore lane dried up**. Two candidates, neither
established:

- **Ore exhaustion under a burner drill's small mining area.** A burner drill's
  radius is 0.99, so it works about four tiles; `iron-ore x16` counts entities
  near the block, not tiles under the drills.
- **The belt backing up.** Lanes back up independently, so this would need the
  ore lane specifically to jam, which I cannot see.

**Nothing reads a belt's contents and nothing reports a drill's status**, so
neither can be distinguished from the outside. That is the third distinct thing
this session that the belt blind spot has made unanswerable.

The rate is also low — roughly 25 ore mined in ~6,000 ticks against two drills'
0.5 ore/s nameplate — and the 13/4 split between near and far furnace is the
same near-starves-far pattern `TwoRowSmelter` measured, arriving here without
being looked for.

## What this completes, and what it does not

The t=0 chain exists end to end and needs no electricity and no research. What
it is not is a *factory*: two drills and two furnaces, plateauing, at a fraction
of nameplate. The next questions are why it stopped and whether it scales — and
both want the belt visible.
