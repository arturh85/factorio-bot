# The stranded tile was siting all along

2026-09-07. `scripts/stranded_tile.lua`, seed 31337. Closes a question open
since the previous evening, and corrects both sessions.

## The symptom

`OreToPlate` loses one placement on pass 1; the replan then refuses that tile,
so a partial build can never be completed. Two hypotheses were advanced and both
were wrong:

- **A tree.** Retracted: the game reports no tree there, before or after.
- **A stale ledger.** Retracted by its own author: `blocked_tree` and
  `placement_refusals` are different structures, and "a refusal that outlives
  its cause" describes both while explaining neither.

## What it actually is

With `world.blocked_boxes` finally able to enumerate the model, and
`Occupant::Refused` finally reachable rather than masked:

```
replan REFUSES: a footprint the game already refused a burner-mining-drill at,
                where it found no entity

model coverage: charted (64 of 64 tiles written out)
model boxes covering the refused tile: 0
iron-ore in the rectangle: 41
iron-ore under the drill's own footprint: 0
```

**Every step is correct.** Siting chose an anchor where one drill has no ore
beneath it; the game refused, because a mining drill cannot stand on ground with
nothing to mine; the refusal was recorded with no named blocker, so it cannot be
expired; and the replan refused the same footprint. **Nothing in the refusal
machinery is broken.**

The defect is **upstream, in siting** — and it is a finding already in this
repository, written the previous night and never connected:
`drills_are_fed` asks whether the mining area covers *some* extractable
resource, not whether every drill in it has ore. A block passes while an
individual drill in it stands on bare ground.

## Why two sessions chased phantoms

**The old message named the wrong cause.** `occupant_of` consulted
`blocking_boxes_within` before the refused-footprint check, so `Occupant::Terrain`
— *"occupied by a tree, cliff, rock or unit"* — masked `Occupant::Refused`. Both
of us went looking for terrain because the error said terrain, and there was
none, twice.

**And `coverage` is what makes the negative usable.** `0 model boxes` means
"clear" only under `charted`; under `unknown` it means nobody looked. The
retracted tree hypothesis lacked exactly that distinction, and asserted an
absence it could not support.

## What is worth fixing, and what is not

- **Not the refusal.** It is right, permanent, and its message now says so.
- **Not the expiry.** A refusal with no named blocker cannot safely be dropped,
  and this one should not be.
- **Siting.** A drill placed off the patch is the whole cause. `drills_are_fed`
  is correct as specified and the specification is what is missing — the check
  is a feasibility test where a per-drill quality measure is needed.

That is the same conclusion the siting-quality note reached from throughput
(a drill on one of four tiles exhausts its ground four times faster). It now has
a second, sharper consequence: a drill on **zero** tiles strands the whole
block.
