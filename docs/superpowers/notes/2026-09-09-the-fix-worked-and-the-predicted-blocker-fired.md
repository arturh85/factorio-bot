# The exit fix worked exactly as designed — and the predicted next blocker fired exactly as predicted

`run-1788941729-70024`, seed 31337, four headless bots at 10x, `--new`,
release binary built 10:15 from master `7332b562`. **Nothing cheated.**

## The fix works, confirmed live for the first time

```
id 502  place inserter at [30.5, -46.5] -- load copper-plate out of iron-chest
```

**`[30.5,-46.5]` is the kept east exit**, not north — the first time in seven
runs the load arm has sited anywhere but north. This is the direct,
observable effect of `7332b562` (`kept_exit_of` reading the state's own
reservation instead of a blinded `product_exits` scan). Fewer abandoned steps
too: **11**, in the same range as the belt-banding run, both well below the
48–107 of earlier runs.

## The predicted next blocker fired, precisely as the fixing agent named it

One plan (909 steps), one iteration, `stuck` — and the refusal:

```
a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it to
the supply chest at [44.5,-32.5]: from the iron-chest at [29.5,-46.5]: no
belt route, blocked by 4 tile(s): [28.5,-46.5] [29.5,-47.5] [29.5,-45.5]
[30.5,-46.5]
```

**`[30.5,-46.5]` is in the blocked list — the arm's own tile from plan 1.**
The `7332b562` agent stated this exact risk in their own report before I ran
anything: *"a replan doesn't recognise its own standing link... the standing
chest honestly reports north as the side it has left"* — here it's the
mirror case: the kept exit is occupied by the *plan's own arm*, and a second
routing attempt (for a second supply link, presumably feeding a new
downstream consumer) sees no side left at all, because the chest genuinely
has none free — three sides consumed by coal, the fourth by the arm that was
correctly placed there.

## What this run does NOT show, and it is worse than the milestone runs on this axis

**No science cell was ever built.** Census tops out at 28 machines, no
`assembling-machine`, no `lab` working. This run got stuck *before* reaching
where `run-1788926478-07032` and `run-1788931904-77495` (both pre-fix)
managed 14–15 packs each. The exit-siting bug is fixed; the run that proves
it is honest about not yet being a net win, because the next-in-line
blocker — the same chest genuinely running out of free sides once its own
arm occupies the fourth — now bites earlier in the story than the old bug
used to.

## Reading this honestly

The fix did exactly what it was built to do, verified by its own predicted
side effect landing on schedule. The remaining chain is: **a fully-consumed
perimeter is not just a siting question any more — it may need a second
route to share the same tile, or the cell needs a second exit design, or
sustain and the downstream consumer need to negotiate which one gets the
chest's only free side.** That is a design question, not a bug, and it was
already implicitly flagged by the fixing agent's own "open" note. Not fixed
here.
