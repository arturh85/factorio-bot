# Saturation is the fix — and siting cannot see ground that does not exist yet

2026-09-06. `SaturatedSmelter`, 33 entities, headless 2 bots at 5x, seed 31337.

## The claim under test

`TwoRowSmelter` measured six furnaces off one *under-supplied* mixed belt and
found brutal unfairness: the two westmost took 117 of 150 plates and the two
eastmost took 3. From the mechanism — a fuel slot caps at 5 and refuses more so
coal rides past, while a furnace's ore input has no such ceiling so the near arm
absorbs everything — I concluded that **saturating the belt is the fix**, and
wrote that into CLAUDE.md.

**That was an inference, not a measurement, and it was already in the record.**
This run turns it into one.

The block is identical to `TwoRowSmelter` downstream of the coal junction —
same lanes, same takeoffs, same six furnace positions. The only change is three
ore loaders instead of one.

## Confirmed

|  | 1 loader (starved) | 3 loaders (fed) |
|---|---|---|
| north row, west→east | 59 / 3 / 1 | **63 / 50 / 40** |
| south row, west→east | 58 / 27 / 2 | **61 / 48 / 38** |
| lowest : highest | **1 : 59** | **38 : 63** |
| spread ratio | **59** | **1.66** |
| row totals | 63 vs 87 | **153 vs 147** |

Ore 300 of 300 consumed, coal 90 of 90.

**Ratio 59 → 1.66.** Feeding the belt is what makes the line fair, exactly as
the buffer-size mechanism predicted.

Two secondary readings:

- **The rows balance to within 4%** (153 vs 147). The starting lane asymmetry —
  northern arms meet ore first, southern arms meet coal — is real but
  inconsequential once ore is plentiful.
- **A residual west-heavy gradient of 1.66 remains.** Three burner loaders are
  *better* than one but are not a full belt, so this is consistent with partial
  saturation rather than with fairness being achieved. A genuinely saturated
  belt should flatten it further; nothing here has run one.

## The bigger finding: ungenerated ground reads as clear

Getting to the measurement took four failed attempts, and the reason is worth
more than the measurement.

**Attempt 1** — `near` siting. Pass 1 lost 4 placements; the replan refused by
name: `cannot build transport-belt at tile (44.5, 0.5): occupied by a tree,
cliff, rock or unit`. The refusal was correct. The siting was blind.

**Attempt 2** — chart first. `goal.charted(40, 0, 30)` planned **zero steps**:
the disc was already charted. The search still picked the tree. So this is not
an exploration gap.

**Attempt 3** — ask the running game instead of the model. `rcon` reported
`candidate anchor (40,0): CLEAR -- 0 entities, none blocking`, and the build hit
the same tree.

**Zero entities in a 10-tile disc of a fresh map was the tell.**

**Factorio generates chunks lazily.** An ungenerated chunk genuinely contains
nothing, so the planner's world model *and* a live RCON query are both telling
the truth when they report clear ground — and it fills with trees the instant
something forces generation. A bot walking there is such a thing, which is
exactly why pass 2 knew what pass 1 did not.

**Attempt 4** — walk a scout to each candidate before believing anything about
it, then use the *explicit* anchor form (the only one that refuses for occupied
ground rather than searching past it). `CLEAR -- 1 entities`, then
`done=true failed=0 lost=0 pending=0` on all 33 entities, first pass.

> **Siting far from explored territory is provisional, and no query can fix it.**
> "Charted" is not "generated", and neither the model nor the game can report
> what has not been created. The walk is what makes the ground real.

CLAUDE.md already says a fresh map "has charted almost nothing" and that
distances measure what has been *seen*. This is sharper: the entities do not
*exist* yet, so there is nothing to see and nothing to be stale about.

## Also fixed here

The `BUILD INCOMPLETE` guard printed the **pending** count whatever the failure
was, so a build with `failed=4, pending=0` announced "0 steps never ran". A
message that names the wrong number is worse than no message. It now names only
the counts that are actually non-zero.

## Open

- A genuinely saturated belt (the residual 1.66).
- Whether the far-lane-then-near-lane correction costs a swing cycle.
- The enclosure gap from the previous note: both searching forms of `goal.built`
  ignore characters, so a large block can wall the roster in.
