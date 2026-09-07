# The anchor came from another block

*2026-09-07. Reopens and answers `2026-09-07-the-stranded-tile-was-siting-all-along.md`,
which was half right.*

## What yesterday's note said

That every refusal in the chain was correct, and the defect was upstream in
siting: siting chose an anchor where one drill had no ore. Evidence: coverage
`charted`, 0 model boxes over the refused tile, 41 iron ore in the rectangle
and 0 under the drill's footprint.

All of that is still true. The conclusion drawn from it was not.

## What was actually wrong

I set out to fix "siting" and began by re-reading `drills_are_fed`, which I had
described as checking whether *some* drill covers *some* resource. It does not.
It loops over every drill and refuses if any one of them covers nothing. The
check was correct and was correct all along.

It has exactly one call site, inside `first_obstruction`, which has exactly one
call site, inside `search_site`'s candidate loop. And `resolve_site` opens with:

```rust
if let Some(recovered) = recover_anchor(state, bp) {
    return Ok((recovered, AnchorSource::Recovered));
}
```

**Recovery runs first and unconditionally.** When it hits, `search_site` never
runs — no ring scan, no `first_obstruction`, no per-drill ore check.

`recover_anchor` trusts an anchor once **two** of the blueprint's entities stand
as designed at the right relative offsets. Measured over the 12 fixtures in
`scripts/rcontest.lua` (`crates/core/tests/recovery_crosstalk_probe.rs`):

| block | finds | inside |
|---|---|---|
| `ElectricSmelter` | 21 of its 28 entities | `FurnaceLine` |
| `SaturatedSmelter` | 17 of 33 | `FurnaceLine` |
| `TwoRowSmelter` | 15 of 27 | `FurnaceLine` |
| `OreToPlate` | 14 of 24 | `MinerLine` |
| `OreToPlateTee` | 13 of 29 | `MinerLine` |

These fixtures are variations of each other by construction — they share
chest/inserter pairs and belt runs — and the chain scripts run several of them
on **one map, in sequence**. `FurnaceLine` (179 entities) stood on that map.

So the anchor was never chosen by siting. It was read off a *different block*.
A drill then landed wherever `FurnaceLine`'s geometry put it, which is not on
ore; the game refused the placement; the refusal named no blocker because
nothing was on the tile — the problem was what was absent; the footprint was
remembered as refused; and every later replan reported occupied ground.

Two sessions looked for a tree. There was no tree, and there was also no siting
decision.

## Why the threshold is not the fix

21 of 28 is 75% of the blueprint. Any threshold loose enough to recover a
genuinely half-built block — which is what recovery exists for, and it is
right to exist: an anchor that moves across a replan builds two half-factories
with no error — is loose enough to accept this. **Recovery by geometry is
ambiguous whenever two blocks share a sub-layout.**

The fix is to persist the anchor with the goal instead of re-deriving it from
the ground. That changes `Goal::Built` semantics and was deliberately not done
in passing.

## What did land

`06296b0a` screens every anchor for ore, not only the ones siting chose.
`Site::At` and recovered anchors now get `drills_are_fed` before any step is
emitted, refusing as `PlannerError::BlockDrillUnfed`, which names the drill,
the tile, and — via `AnchorSource` — whether the remedy is to move the anchor
or to clear the half-built block. A hijacked anchor is now refused by name
rather than stranding a block behind a message that blames terrain.

Two things it deliberately is not:

- **Not a quality threshold.** A drill sharing one ore tile with three
  neighbours is a slow block, and slow blocks work. Zero is a different kind
  of fact: a placement the game itself rejects, knowable before commitment.
- **Not a confusion of unknown with zero.** `drills_are_fed` answers "not fed"
  both when ore is absent and when the prototype table cannot say what the
  drill extracts — true of every dump predating `resource_categories`, and of
  `fixture_world()` itself, which is how five existing tests failed on the
  first run. Inside `search_site` that is merely conservative; as a hard
  refusal it would turn *I cannot describe this drill* into *there is no ore
  here*. `drill_capability_is_known` gates the refusal on the model being in a
  position to be right.

That last point is the peer's lesson from this morning, applied to code: a
retraction can be as unsupported as the claim it replaces. Absent evidence
stays unknown rather than becoming an opposite certainty.

## The general shape

**A check that is correct, and correct per-item, still answers nothing about
the cases that never reach it.** Both of us reasoned about `drills_are_fed`'s
logic — I even mis-stated it, and the mis-statement did not change the
conclusion, because the logic was never the problem. What mattered was its one
call site, two frames above.

The tell was available cheaply: `grep -n drills_are_fed` shows one non-test
call site. I ran it only after deciding to change the function.
