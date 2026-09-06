# A belt-fed smelter, and the lane that had to be separated

2026-09-06. `TJunctionSmelter`, live, headless 2 bots at 5x on seed 31337,
second instance (`scratch/blocks.toml`, rcon 4360, `workspace/blocks`).

`MovingBlock` showed a block moves items. `SmeltingBlock` showed a furnace
smelts when an inserter hands it ore **from a chest**. Every real smelter is fed
by a **belt** carrying ore and coal together, and that step had never run here.

## The result

```
ORE consumed:  100 of 100      COAL consumed: 50 of 50
PLATES: furnace A = 39, furnace B = 39, total = 78
arm fuel at end: 1/3/1/1  (coal-loader/ore-loader/takeoff-A/takeoff-B)
```

18 entities, all placed at the blueprint's own offsets, 0 failed, 0 pending.

**77 plates over 7,384 ticks across two furnaces is 192 ticks per furnace per
plate — exactly a stone furnace's 3.2 s smelt time.** Both furnaces ran at 100%
of theoretical, so the belt never starved either of them. The 39/39 split says
the same thing from another direction: two takeoffs drew evenly off one lane.

The counts matter more than the total. **Both commodities drained to zero.**
Ore-only or coal-only movement is the failure this block exists to fix, and it
reads as a working belt unless both are checked.

## What the first attempt got wrong, and why it is worth recording

The predecessor put ore and coal in **one chest behind one loader** — the
obvious way to "mix" them. Measured over 2,500 ticks:

```
tick 553    chest ore=99  coal=50  armfuel=0/0/0  furnfuel=0/0  plates=0/0
tick 1445   chest ore=99  coal=38  armfuel=1/1/0  furnfuel=5/0  plates=1/0
tick 2894   chest ore=99  coal=19  armfuel=1/1/1  furnfuel=5/5  plates=1/0
```

Coal drained 50 → 17. **Ore never moved off 99.** One commodity took the belt
entirely, and the block made one plate — from the single ore that escaped before
the coal took over.

Every part of that block worked. All three arms self-fuelled, both furnaces
filled their fuel slots, the belt carried items the length of the run. Judged on
"did the belt move things", it passed. It was starving.

**The first version of the script said so, confidently and wrongly.** It read
only the furnaces and the chest's *ore*, saw coal in both fuel slots, and
printed `RESULT: the MIXED BELT fed the smelter`. Two facts in its own output
contradicted each other — coal in the furnaces meant the belt ran, one ore
moved meant it did not — and with no visibility into the arms there was no way
to tell which half was wrong, so the verdict picked the flattering one. Adding
the arms' `fuel_inventory` and the chest's **coal** count resolved it in one
run. *A verdict line can only be as honest as the narrowest thing it reads.*

## The two mechanics

Both come from the owner, and neither is visible in a picture of the block — a
layout with either one backwards places 100% correctly and moves nothing useful.

1. **An inserter drops on the belt's FAR lane.** So the ore loader sits *north*
   of the main belt and ore lands on the *south* lane.
2. **A belt running into the SIDE of another sideloads onto its NEAR lane.** So
   the coal branch runs south into the main belt from the north, and coal lands
   on the *north* lane.

No inserter merges the two; the belts do. The coal branch is a belt rather than
a chest arm because the real source is a miner column, which outputs onto a belt
directly.

The takeoffs sit *south* of the main belt, so they draw the far (coal) lane
first and fall back to the near (ore) lane once a furnace's fuel slot is full.
That is what lets **one arm per furnace deliver both commodities**.

Pinned in `the_t_junction_smelter_separates_its_lanes_and_needs_no_research`.

## One arm is hand-fuelled, and the reason is not a workaround

A burner inserter fuels itself from coal it carries. The coal loader and both
takeoffs therefore need nothing from us — they ended the run at 1/1/1 with
plates still coming.

**The ore loader carries only ore and can never self-fuel.** It gets 5 coal by
hand. In the real block there is no inserter there at all: ore arrives on a belt
from a miner column. The chest-and-arm stands in for those miners.

This is the same shape as `SmeltingBlock`'s starving output arm, and the general
rule is now clear: **a burner block works exactly where coal flows through it.**
Any arm that touches only ore, or only plates, is a hand-fuelled stub waiting for
the electric inserter.

## The block researches its own upgrade

Unplanned, from the run's own log:

```
trigger technology steam-power earned at tick 5880 (craft-item 50/50)
```

The block's plates fired `steam-power` by themselves. That is one of
`automation-science-pack`'s two prerequisites; the other is `electronics`, a
trigger technology fired by **10 copper plates** — about 32 seconds of one stone
furnace — and `electronics` is what unlocks `inserter` and `small-electric-pole`.

So the t=0 burner block bootstraps the research that unlocks the electric block.
Run it on copper as well as iron and both prerequisites fall out of its own
output, with no lab.

## Numbers for scaling, checked against the live 2.1.17 prototype dump

Iron ore → iron plate is **1:1** (`ingredients [(iron-ore, 1)]`,
`products [(iron-plate, 1)]`, 3.2 s), so **a saturated ore belt yields a
saturated plate belt** — not half. Stone→brick (2:1) and steel (5:1) are where a
belt halves.

| | rate | per full yellow belt (15/s) |
|---|---|---|
| stone furnace | 0.3125 plate/s | **48 furnaces** |
| electric drill | 0.5 ore/s (speed 0.5 ÷ mining_time 1) | **30 drills** |
| burner drill | 0.25 ore/s | 60 drills |
| coal for 48 stone furnaces | ~1.1 coal/s | **~7% of a belt** |

**Coal never needs its own full belt** — one would feed roughly 660 stone
furnaces. That is exactly why the standard design gives it a lane rather than a
belt, and why the T junction is the right merge.

`FurnaceLine` is already this block at scale: 24 furnaces in two rows (y=3 and
y=8, 12 each), 48 inserters, and three belt rows (y=0.5 / y=5.5 / y=10.5) with
the central belt serving both rows. At 24 furnaces it runs at **half** a yellow
belt; 12 → 24 per row saturates one. `MinerLine` is its front end at 13 electric
drills = 6.5 ore/s against FurnaceLine's 7.5 ore/s appetite.

Neither is buildable at t=0, checked recipe by recipe against the dump:

| enabled on a fresh force | needs research |
|---|---|
| `stone-furnace`, `transport-belt`, `burner-inserter`, `burner-mining-drill`, `iron-chest` | `inserter`, `small-electric-pole` ← **electronics**<br>`splitter`, `underground-belt` ← **logistics**<br>`electric-mining-drill` ← its own tech |

## Still not done

- **No output side.** Plates accumulate in the furnaces. Adding one costs an
  electric inserter, i.e. `electronics` — which this block now earns.
- **Two furnaces, not 48.** Nothing has tested a long belt with many takeoffs,
  where lane balance and inserter throughput start to matter.
- **The ore source is a chest.** No drill has ever been placed against a belt,
  though `MinerLine` encodes the geometry (drills facing east and west onto a
  central column).
- **Nothing expands anything.** "Planned in advance, expanded as we can" has no
  machinery yet.
