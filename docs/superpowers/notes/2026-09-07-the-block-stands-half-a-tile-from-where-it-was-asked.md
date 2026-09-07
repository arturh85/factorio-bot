# The block stands half a tile from where it was asked

*2026-09-07. Found by giving `Site::Anchored` its first caller, which is the
whole reason a first caller is worth the trouble.*

## The measurement

Build `OreToPlateTee` on a fresh seed-31337 map, then sweep pinned anchors in
half-tile steps around the one its own stamp reported, counting how many
entities each pinned plan still wants to place. Zero means *this is where the
block stands*, because that is the question `already_stands` answers.

```
stamp reported:      (-15.5, -18.5)
pinned sweep, 81 candidates, exactly one reads the block as complete:
BEST pinned anchor:  (-15.0, -18.0)   offset (+0.5, +0.5)   0 to place
```

Two controls make it a finding rather than a coincidence:

- **A recovery replan reads the same block as complete** (`0 to place`), so the
  block is intact and the disagreement is about the anchor alone.
- The build finished clean — `done=true failed=0 pending=0`, 29 of 29 — so this
  is not a partial build with a confused vote.

## What is established, and what is not

**Established**: the anchor a block's `StampGhosts` action reports is **not** the
anchor its entities stand at. The offset here is exactly (+0.5, +0.5), constant
across both axes.

**Not the planner mis-recording it.** `entity_for` applies no alignment: it
builds each entity at `anchor.add(&e.offset)` and nothing rounds. And the unit
test `the_stamp_carries_the_anchor_a_caller_must_pin` passes — the stamp really
does carry what `resolve_site` returned. Both halves of the planner agree; the
*ground* is what disagrees with them.

**Leading hypothesis, not yet proven**: the game snaps. Factorio places an
entity on the legal grid for its footprint parity — an even (2x2) footprint on a
tile boundary, an odd (3x3) on a tile centre, which is what
`method::util::tile_alignment` exists to describe. Ask for a 2x2 drill at a
half-integer position and the game puts it on the nearest integer. Every entity
in the block shifts the same way, so the block lands intact and half a tile off.

The direct confirmation nobody has run yet is one line: place a single entity at
a deliberately wrong-parity position and read its position back off the surface.

## Why it matters more than half a tile

**It makes the documented `Site::Anchored` pattern fail.** The whole point of
the owner's ruling is that a caller records the anchor and pins it on replan.
The only anchor a caller can obtain is the stamp's, and pinning it refuses:

```
pinned replan REFUSED: cannot build transport-belt at tile (-9, -18):
                       occupied by transport-belt
```

The block's own belt. The planner asks for it half a tile from where it already
stands, sees something in the way, and refuses — correctly, on inputs that are
wrong.

**And it is a candidate root cause for the stranded tile**, which was closed
twice already. In the same run the game refused **four** drill placements:

```
the game would refuse to build burner-mining-drill at [-10.5, -16.5]
  (no entity in the footprint; tile dirt-6)
```

*No entity in the footprint* — nothing is in the way, the ground is dirt, and
the game still says no. That is the signature of a drill with no ore under it.
But `drills_are_fed` checked ore at `anchor + offset` and passed. If the game
then snaps the drill half a tile, it is placed somewhere the planner never
checked — so the check can be correct and the placement still land off the ore.
**A drill verified on ore and built off it is exactly the stranded tile**, and
it needs no hijacked anchor to explain it.

That does not retract the recovery-crosstalk finding, which is separately
measured and real. It means there were likely two causes, and the sessions that
found one each time stopped there.

## What this says about the method

`Site::Anchored` shipped with unit tests that all passed, and it was wrong in
practice on its first real use. The tests were not weak — they asserted the
right things about the planner. **They could not have caught this, because both
halves of the planner agreed with each other and disagreed with the game**, and
nothing in a fixture world snaps anything.

The repo already says this in another voice: *"a module with no caller cannot
discover that its bill is unbuildable"*, and *"it had no caller until
2026-09-06, and that absence is what hid the geometry defect through four
reviews"*. This is the third instance, and the first where the feature's own
author wrote the tests, believed them, and shipped in the same session.

## CONFIRMED, and fixed

**The snap is real** (`scripts/does_the_game_snap.lua`). Asked for a position,
read back what the game did with it — with both controls, so a snap could not
be confused with this script mis-computing parity:

```
stone-furnace  (2x2)  asked (10.0, 10.0)  stands (10.0, 10.0)   exact   <- control
stone-furnace  (2x2)  asked (20.5, 20.5)  stands (21.0, 21.0)   MOVED +0.5
transport-belt (1x1)  asked (30.5, 30.5)  stands (30.5, 30.5)   exact   <- control
transport-belt (1x1)  asked (40.0, 40.0)  stands (40.5, 40.5)   MOVED +0.5
```

An even footprint belongs on a tile boundary, an odd one on a tile centre, and
the game moves an entity that asks for the wrong one **without failing**.

**The cause was in the seed, not in the search.** `search_site` steps in whole
tiles, so every candidate inherits the seed's fractional part — and
`Site::Anywhere` seeds at `nearest_ore_seed`, an ore position, which is a tile
*centre*. **A block containing any 2x2 entity could therefore never be sited
legally under `Anywhere`**, on any map, ever. It is not a rare parity accident;
it was every drill block this project has built.

`anchor_alignment` now derives the fractional part the blueprint requires — from
every entity, refusing to guess when they disagree — and `search_site` aligns
the seed once before scanning, since whole-tile steps preserve parity.

## Measured after the fix, same map, same script

```
                        before              after
anchor                  (-15.5, -18.5)      (-15.0, -16.0)   integral
block stands at         (-15.0, -18.0)      (-15.0, -16.0)   agrees
pinned replan           REFUSED             0 to place
drills the game refused 4                   0
```

The last row is the one that matters beyond this bug. Those refusals read
*"no entity in the footprint; tile dirt-6"* — nothing in the way, still refused,
which is a drill with no ore under it. `drills_are_fed` checked ore at
`anchor + offset` and the game then moved the drill somewhere the check never
looked. **That is the stranded tile, and it is now gone from this run.**

So the stranded tile had two causes, and three sessions each stopped at one:
recovery crosstalk hijacking an anchor, and this — a check that was correct
about a position the game did not use. Neither retracts the other.

## Still open

1. Whether `Site::At` and `Site::Anchored` should also be parity-checked. They
   are caller-supplied, so the honest options are to refuse a wrong-parity
   anchor by name or to align it; silently aligning a *recorded* anchor would
   defeat the point of recording it.
2. 29 `entity-ghost`s remain standing after a completed build. Harmless here —
   the pinned replan reads the block as complete — but `recover_anchor_from_
   ghosts` consults exactly those, so it is worth knowing why they survive.
