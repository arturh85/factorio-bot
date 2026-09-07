# The block was drill-limited, and the arithmetic predicted the fix

*2026-09-07. A prediction made from prototype numbers, then tested.*

## The claim

With the output cap removed, `OreToPlateTee` ran at **28.7 plates/min**. From
the game's own prototypes:

```
burner-mining-drill  mining_speed 0.25/s  ->  2 drills   = 30.0 ore/min
stone-furnace        crafting_speed 1     ->  2 furnaces = 37.5 plates/min
```

28.7 is **96% of the drills' 30.0** and 77% of the furnaces' 37.5, which says
the block is drill-limited and its furnaces are over-provisioned.

That is a reading, not a measurement — 28.7 sits near 30 and it could be
coincidence. So it was stated as a falsifiable prediction: **a third drill takes
supply to 45.0 ore/min, past the furnaces' ceiling, so the block should become
furnace-limited and land near 37.5. If it does not move, the arithmetic was a
coincidence and the constraint is elsewhere.**

## The result

```
drills   supply      furnace cap   measured    % of the binding constraint
2        30.0/min    37.5/min      28.7/min    96%   of DRILLS
3        45.0/min    37.5/min      36.2/min    96.5% of FURNACES
```

**+26% for one drill and one belt tile.** Reproduced twice, identical to the
plate. The prediction was "near 37.5" and the answer is 36.2 — the constraint
moved to where the arithmetic said it would.

The curve shape agrees too: it climbs and flattens at 36.2 rather than decaying,
which is what a supply-satisfied block looks like when nothing is filling up.

## Why this is the useful kind of result

**The model is predictive rather than descriptive.** Two prototype numbers and a
count of machines said where the constraint was and what moving it would buy,
before anything was built. That is the first time in this record a rate has been
forecast and then hit.

It also settles an ordering that three separate attempts got wrong today:

| lever | effect on rate |
|---|---|
| the output side (drain the furnaces) | **+16%**, and turns a decaying curve into a steady one |
| drill coverage 6/8 -> 8/8 | **0%**, roughly doubles longevity |
| **a third drill** | **+26%** |

Coverage was the one I predicted would matter and it moved nothing, because
Factorio does not scale a drill's speed by how many tiles it covers. The
arithmetic knew that and I did not apply it until the measurement forced me to.

## Where it stands now

`OreToPlateThree` is at **96.5% of its furnace ceiling**, so the next gain needs
more furnaces, not more drills. The honest ratio is **five drills to four
furnaces** (18.75 ore/min consumed against 15.0 supplied), and the next rung is
therefore a 5:4 block rather than another drill.

Coal becomes the thing to watch on the way: three drills burn it and the extra
smelting consumes it, which is why this run carries 150 in the chest against the
two-drill run's 100. It did not run out, and nothing here has measured how close
it came.
