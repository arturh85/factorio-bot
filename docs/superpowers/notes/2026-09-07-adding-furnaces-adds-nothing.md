# Adding furnaces adds nothing; the far end starves rather than sharing

2026-09-07. `scripts/chain_wide.lua`, seed 31337, fresh map per variant,
identical 6,300-game-tick window.

Supply held **fixed** at two burner drills; consumers scaled from two furnaces
to four. That isolates the question the 48-furnace target depends on: does a
longer line produce more?

```
2 furnaces:  29 / 15           total 44
3 furnaces:  29 / 15 /  1      total 45
4 furnaces:  30 / 14 /  2 / 0  total 46
```

**+2 plates for +2 furnaces — 4.5% more output for 100% more machines.** And the
distribution is not a gradient, it is a cliff: the near furnace takes ~65%, the
second ~32%, the third 4%, the fourth **nothing at all**.

## Measured, and it corrects the claim above

The plate counts were inferred from; `entity.status` can be read. Sampled 3,058
times across the same 6,300-tick window — a share of samples, not one instant,
because "87.7% working at tick N" and "working 87.7% of the time" are different
quantities:

| furnace | working | no_ingredients | no_fuel |
|---|---|---|---|
| 1 (near) | **91%** | 2% | 7% |
| 2 | 43% | 42% | 15% |
| 3 | 7% | 62% | 31% |
| 4 (far) | **0%** | 58% | **42%** |

**The gradient is confirmed and the attribution was wrong.** I wrote that the far
end "starves", meaning ore. `no_ingredients` is indeed the largest single idle
cause and grows with distance — but **`no_fuel` grows just as steadily, 7% to
42%**, and at the far furnace the two are comparable.

So the effect is **not about ore**. Coal is consumed near-first for exactly the
same reason ore is: each arm takes what passes it, on whichever lane. A
first-come-first-served belt starves the far end of **everything it carries**,
and naming ore was reading the mechanism off the commodity I happened to be
counting.

That also makes the buffer-size explanation narrower than I stated it. A fuel
slot capping at 5 does let coal ride past a *satisfied* furnace — but it does
nothing for a furnace whose arm never gets a turn, which is what the far end
actually suffers.

## Why: a belt is first-come-first-served, not fair

Two burner drills supply about 0.5 ore/s. Two stone furnaces can eat 0.625
plates/s. **The block is already supply-limited at two furnaces**, so every
furnace after that is idle by construction — and because each arm takes what
passes it, the ore is consumed by whoever is nearest and the far end sees an
empty lane.

This is the same effect measured twice before from other directions:
`TwoRowSmelter` under-supplied gave a 1:59 spread and saturated gave 1.66, and
the world-record base's dominant error term is idleness at +16-23%, with 194
drills reading `waiting_for_space_in_destination`. **Near-starves-far is not a
small-block artefact.**

## What it means for a 48-furnace line

- **Furnaces are not the lever.** Output is set by what arrives on the belt, so
  a longer line without more supply buys idle machines and nothing else.
- **48 furnaces needs a saturated belt**, which is 48 x 0.3125 = 15 items/s —
  one full yellow belt, and 30 electric drills to fill it. At two burner drills
  the honest ceiling is about **two** furnaces.
- **Balance is not automatic.** Even fully supplied, the far end only gets what
  the near end leaves; the saturated case measured 1.66 rather than 1.0.

## Method

Every variant on a **fresh map from the same seed**, sampled at the **same
game-tick offset** from its own charge — the two instrument faults corrected
earlier the same night. Per-furnace output rather than a total, because 44 split
40/4 and 44 split 22/22 are the same total and opposite findings.

## A separate finding: a wide drill array is hard to site

The first attempt scaled drills as well, to four in a row spanning eight
contiguous tiles. It could not be sited: the game refuses a mining drill with no
ore beneath it (*"no entity in the footprint; tile dirt-6"*), and the planner
re-sited twice before giving up.

**Every drill must individually sit on ore**, so a wide drill row needs a patch
both wide enough and aligned. `drills_are_fed` passing does not imply every
drill is placeable — it asks whether the mining area covers some extractable
resource, which a block can satisfy while an individual drill in it cannot.
