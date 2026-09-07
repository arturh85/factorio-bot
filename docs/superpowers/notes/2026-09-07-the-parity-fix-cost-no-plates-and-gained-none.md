# The parity fix cost no plates and gained none

*2026-09-07. A null result on the headline question, and it corrects an
implication I made before measuring.*

## What I implied, and what is true

After finding that every drill block was sited on the wrong tile grid, I told
the owner that *"the published throughput numbers were taken on blocks that were
partly misplaced"*. True as stated, and it invites a conclusion that is not:
that those numbers were therefore too low and would improve.

**They do not.** Measured on the exact basis the original used —
`chain_compare.lua`'s 6,300-tick window, the same 100 coal into the chest and
25 into each drill, the same seed, one fresh map:

| | plates in 6,300 ticks |
|---|---|
| before the parity fix | 44 |
| after the parity fix | **43** |

One plate apart, which is noise between two block positions. **The parity bug
cost no throughput at all.**

## Why not, which is the interesting half

Because the whole block moves *together*. The game snaps every entity by the
same half tile, so the block lands intact — belts still meet furnaces, arms
still reach both, the T junction still merges. It stands somewhere the
**planner** did not expect, and the game does not care where the planner
expected it.

So the bug was never a production defect. It was a **model** defect, and
everything it broke was on our side of the boundary:

- `already_stands` looked half a tile off, so a completed block read as unbuilt
- `Site::Anchored` could not be pinned, because the stamp's anchor was not the
  block's anchor
- `drills_are_fed` verified ore on tiles the game would not use

That last one *can* cost plates — a drill snapped off its patch is refused, and
the run that found this had four such refusals. But that depends on where the
block lands relative to the patch, not on parity as such, and in the measured
runs it did not happen.

## The new figure, on a longer window

Nothing had measured this block past the point where the old iteration cap
stopped it, so this is new rather than a comparison:

```
plates in 30,000 ticks: 200   (24.0/min)
peak rate:              28.0/min at 18,000 ticks
drill coverage:         6 of 8 possible tiles, 986 ore in reach
game-refused drills:    0
reproduced:             two runs, identical to the plate
```

The curve rises to 28/min and then falls away — 26, 24, 21, 16, 8 plates per
3,000-tick mark over the last five. **The block peaks and declines rather than
plateauing**, which is a different shape from the sideload designs that simply
stopped, and it is not explained by ore: 986 tiles-worth was in reach and about
200 was consumed.

## What is now the visible constraint

`DRILL COVERAGE: 6 of 8`. One drill has all four of its tiles; the other has
**two**. That is the siting-quality gap this repo has had documented since
2026-09-06 and deliberately not closed — `drills_are_fed` asks whether a drill
has *some* ore, which is right for feasibility and silent about quality, and a
drill on two tiles exhausts its ground twice as fast as one on four.

Before the parity fix that gap was invisible under a larger one. It is now the
top of the list.

## The method note

**A measurement I expected to confirm an improvement showed none, and that is
the result.** I had already told the owner the old numbers were suspect; the
honest move was to measure on the original's exact basis rather than publish the
30,000-tick figure beside a 6,300-tick one and let the difference read as a
gain. Two windows, two numbers, and only one of them is a comparison.
