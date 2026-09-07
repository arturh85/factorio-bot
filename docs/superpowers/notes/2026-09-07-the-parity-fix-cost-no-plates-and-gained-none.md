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
3,000-tick mark over the last five.

**That shape is fully explained, and the explanation makes the 200 not a rate
at all.** Per furnace at the end of the window:

```
furnace 1: 100 plates   <- FULL STACK
furnace 2: 100 plates   <- FULL STACK
```

**200 is exactly two output stacks.** A stone furnace's output slot holds 100
iron plates, this block has two furnaces and **no output side** — a burner arm
carries coal and ore, never plates, so nothing empties them. The decline is each
furnace approaching its own cap and stalling; the supply side was never the
constraint. 986 ore sat in reach against ~200 consumed, and the coal chest was
nowhere near empty.

So **the 30,000-tick figure measures the furnaces' output slots, not this
block's production**. The 6,300-tick figure of 43 is below the cap and is a real
rate; the 200 is a ceiling. Any window long enough to approach it is measuring
storage.

This also retroactively vindicates the old 78: it was below 200, so it was a
genuine non-terminal value, exactly as that note said.

## An instrument with a dead half

The run that settled this asked for furnace `status` alongside the plate count,
on the theory that ore, coal, lane and output failures are indistinguishable in
a plate count and obvious in a status. **It returned `?` for every furnace at
every mark**: `inventory_contents_at` answers with output and fuel inventories
and does not carry `status`, which lives on the entity record from
`find_entities_in_radius`.

Worth recording twice over. The question was settled by the *other* half of the
instrument — the per-furnace split — and had that been left out, the run would
have produced ten rows of `?` and no answer. And a value that is **uniformly**
absent is the same tell as the stale-binary case in CLAUDE.md: a real "the game
does not know" is almost never perfectly uniform.

## What is now the visible constraint

`DRILL COVERAGE: 6 of 8`. One drill has all four of its tiles; the other has
**two**. That is the siting-quality gap this repo has had documented since
2026-09-06 and deliberately not closed — `drills_are_fed` asks whether a drill
has *some* ore, which is right for feasibility and silent about quality, and a
drill on two tiles exhausts its ground twice as fast as one on four.

Before the parity fix that gap was invisible under a larger one.

**But it is not the top of the list, and the output cap is why.** With no output
side the block stops at 200 plates however much ore its drills can reach, so
better drill coverage buys a longer run at the same rate and the same ceiling.
The ordering is: give the block an output side, then coverage becomes worth
fixing. An output side needs electric inserters, and this project has already
shown a burner block earning `electronics` from its own copper in about 37
seconds of game time.

That is the second time today that diagnosing before treating changed the
answer. Fixing coverage first would have been a real fix that moved no plates —
which is precisely what the parity fix turned out to be.

## The method note

**A measurement I expected to confirm an improvement showed none, and that is
the result.** I had already told the owner the old numbers were suspect; the
honest move was to measure on the original's exact basis rather than publish the
30,000-tick figure beside a 6,300-tick one and let the difference read as a
gain. Two windows, two numbers, and only one of them is a comparison.
