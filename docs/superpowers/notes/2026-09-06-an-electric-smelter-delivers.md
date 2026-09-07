# An electric smelter, and the output side no burner block can have

2026-09-06. `electric_smelter_live.lua`, headless 2 bots at 5x, seed 31337.

Every burner block in this tree stops at the furnace. An arm carrying iron
plates never touches coal, so it has no fuel source and dies with its hand
charge — measured on `SmeltingBlock`, and the reason `TJunctionSmelter`,
`TwoRowSmelter` and `SaturatedSmelter` all end where the plates are made.

`ElectricSmelter` is that block with electric arms and an output side.

## It delivers

```
build: done=true failed=0 lost=0 pending=0
apparatus standing: solar-panel x4, small-electric-pole x7
>> FIRST PLATE REACHED THE SINK at tick 2645
PLATES DELIVERED TO THE SINK: 78
still in the furnaces: A=0 B=0   ore left: 0
```

**7,296 ticks for 77 plates across two furnaces is 94.8 ticks a plate, or 189.6
per furnace against a stone furnace's 192 — 98.7% of theoretical.**

The number that says the *output side* works is not the total, though: it is
that **the furnaces read `0/0` for the entire run.** In every burner block here
the plate count climbs, because nothing can remove them. Here the arms took
plates as fast as the furnaces made them, put them on a belt, and a chest at the
end filled.

## Cheated, disclosed

- `cheat_all_technologies`. The honest path to `electronics` is proven
  separately and costs 2,220 ticks; repeating it would measure that, not this.
- Build materials to the bots.
- **Solar panels placed by hand.** `Goal::Built` cannot plan a generator yet:
  `ensure_powered` charges a block's own consumers against its own budget, so a
  78 kW block asking for 78 kW is refused. Solar rather than a boiler because no
  Lua binding exposes water tiles, and freeplay starts at midday so solar is
  deterministic at t=0.
- **A sink chest and one arm are instrumentation, not block.** The mod cannot
  read a belt's contents at all, so plates delivered onto the output belt would
  otherwise be invisible.

## Two apparatus failures, and the one that looked like a block failure

**The first run reported 0 plates in the sink with 61 in the furnaces**, and its
own failure branch said the right thing: *"if they are rising, smelting works
and the OUTPUT arms are the failure."* The arms were not the failure. The sink
arm at (51.5, 5.5) sat **outside every pole's supply area** — the nearest covers
x 44..49 — so nothing could unload the belt, the belt backed up, the output arms
had nowhere to drop, and the plates piled up behind them in the furnaces.

**One tile of missing pole coverage, four entities downstream, presenting as a
smelting failure.** Worth remembering when a block reads as broken at its
source: back-pressure travels backwards, so the symptom appears upstream of the
cause.

The other failure the game named itself: `electric-pole at [45.5, 7.5] ...
blocked by solar-panel`. A solar panel is 3x3 and I put a pole inside one I had
just placed.

## And I counted the wrong thing

The first run printed `solar panels placed: 4` because four `pcall`s returned
without raising. **That is not four panels standing** — and the pole the game
refused by name was counted as placed by the same logic. The rewrite asks the
world what is there and refuses to continue if no panel stands:

```
apparatus standing: solar-panel x4, small-electric-pole x7
```

This is the session's recurring shape one more time — `only_ghosts = true`
validating nothing, `0 uncovered` passing when the loop never runs, a load guard
that could not run and shrugged. **A call that did not raise is not a thing that
happened.**

## What is still missing

The generator is hand-placed. `Goal::Built` will plan one when
`ensure_powered`'s demand exclusion is fixed — a block's own consumers are
currently counted as pre-existing demand against its own budget, so
`FurnaceLine` asking for 624 kW is refused against a 900 kW plant that already
"has" 611 kW of load. `ElectricSmelter` at 78 kW is the smallest fixture that
tests that fix.
