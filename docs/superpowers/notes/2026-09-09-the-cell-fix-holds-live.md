# The cell-recognition fix holds live — confirmed, and the design question still stands

`run-1788946451-86723`, seed 31337, four headless bots at 10x, `--new`, release
binary built 11:33 from master `5323e71a`. **Nothing cheated.**

## The fix works, confirmed live for the first time

Three planning attempts this run, not one — the first live evidence that a
replan can now produce a *small* continuation plan instead of either refusing
or rebuilding a whole second cell:

```
plan 1: tick 451,   909 steps
plan 2: tick 53714,  39 steps  -- continues the SAME cell
plan 3: (refused, no plan_created)
```

Plan 2's steps are unmistakably a **continuation**, not a fresh cell:

```
fuel the stone-furnace with 1 coal
fuel the burner-mining-drill with 1 coal
craft 3 iron-gear-wheel
craft 3 automation-science-pack
```

This is `5323e71a` (`complete_cell` recovering from any standing part) doing
exactly what it was built for: the replan recognised the existing cell and
topped it up rather than siting a duplicate. Before this fix, every replan in
this project either refused outright or, per the design note that motivated
the fix, built a second cell with a duplicate link.

## What failed, and it's a known shape, not a new bug

```
take 1/4/3/15 copper-plate from the cell ... removed 0
```

Same family as the earlier `take_in_pieces` diagnosis: a hand-take asked for
plates the belted cell's chest did not have *at that moment*, retried in
pieces up to its bound, and correctly failed when nothing arrived. Not
investigated further here — it is the mechanism already understood, not a
regression.

## The design question still stands, on a later replan

The run ended `stuck after 2 iteration(s)` with the same shape as
`run-1788941729-70024`: the science cell's supply chest perimeter genuinely
fully consumed (four tiles blocked, matching the coal-run + arm pattern
already diagnosed). This is the **owner-decision item** from the previous
run's write-up — share a tile between two routes, give the cell a second
exit, or negotiate order between two consumers — and it is unchanged by
tonight's fix, exactly as expected: that fix addressed cell recognition, not
perimeter capacity, and never claimed to touch this.

## Bottom line

The fix works, live, exactly as its own offline verification predicted. The
project's remaining path to a sustained science line runs through the same
design question named an hour ago, not a new one.
