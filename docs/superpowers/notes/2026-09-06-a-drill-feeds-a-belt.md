# A drill feeds a belt, and the planner finds the ore itself

2026-09-06. `BurnerMinerLine`, 15 entities, headless 2 bots at 5x, seed 31337.

Two things this project had never done, both in one run.

## Ore-aware siting, live for the first time

**No site was passed, on purpose.** `Site::Anywhere` is what reaches
`nearest_ore_seed`, so naming an anchor would have skipped the path under test.

```
drills standing: 4 at (-16,-20) (-13,-18) (-16,-16) (-13,-14)
resource within 3 tiles of the first drill: iron-ore x32
```

The planner put the block on an iron patch **25 tiles from spawn, by itself**.
`nearest_ore_seed` and `drills_are_fed` had unit tests and had never run in a
game.

## A drill delivering onto a belt, with no inserter

```
ORE DELIVERED TO THE SINK: 221 (iron-ore x221)
```

A mining drill drops onto the tile in front of it — no inserter belongs between
a drill and a belt, and none is in this block. `MinerLine` encodes the shape and
had never been built.

**The drop tiles were derived, not eyeballed.** `delivery_offset` gives a
`burner-mining-drill` `(-0.35, -1.3)` facing north, turned by direction. So a
drill at `(2, y)` facing east drops at `(3.3, y-0.35)`, which is belt tile
`(3.5, y-0.5)`. All four were checked that way *before* the blueprint string was
encoded, and the run confirmed it. Getting this wrong yields a block that places
100% correctly and mines into the ground.

## The rate, and why it is not a drill efficiency

```
220 ore in 16,564 ticks = 0.80 ore/s
four burner drills at 0.25 ore/s = 1.00 ore/s nameplate  -> 80%
```

**Do not read that 80% as drill throughput.** The block ends in a *single*
burner inserter feeding the sink chest, which is a serial bottleneck downstream
of four parallel drills, and its throughput is in the same neighbourhood as the
delivered rate. The drills would then back up onto the belt and idle.

I cannot confirm it: **the mod exposes no way to read a belt's contents**, so
"the belt was full" is the likely explanation rather than a measured one. The
same blind spot forced the sink chest to exist at all.

This is the third time in one session that the *instrumentation* has been the
limiting element rather than the block — the electric smelter's sink arm sat one
tile outside pole coverage, and before that a sink arm starved for fuel. A block
measured through a single arm is measured through that arm's throughput.

## What it completes

The t=0 chain now exists end to end as three proven blocks, none needing
electricity or research:

| block | proves |
|---|---|
| `BurnerMinerLine` | ore out of the ground onto a belt, sited on ore automatically |
| `TJunctionSmelter` | ore and coal merged on one belt, feeding furnaces |
| `SmeltingBlock` / `MovingBlock` | a machine consuming and producing; belts moving items |

They have never been run as one chain, and the joins are untested: nothing yet
routes `BurnerMinerLine`'s output belt into `TJunctionSmelter`'s ore lane.

## Disclosed

Build materials cheated; 10 coal into each drill; 5 coal into the sink arm. A
burner drill mining **iron** cannot fuel itself — one mining **coal** could,
which is a block worth building. The sink arm carries ore and has no fuel source
at all, the same limit every burner output arm in this tree has.
