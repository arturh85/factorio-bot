# Belt banding and sustain-reachability, live for the first time

`run-1788936524-99544`, seed 31337, four headless bots at 10x, `--new`, release
binary built 08:48 from master `c1fd2620`. **Nothing cheated.**

## Belt banding worked

```
steps/bot  {1: 434, 2: 189, 3: 155, 4: 135}     (prior runs: {1: ~520, 2: 136, 3: 119, 4: 105})
abandoned: 11                                    (prior runs: 48, 80, 107)
```

Bot 1's share of the plan fell from ~58% to ~35%, and the truncation that has
dogged every earlier run shrank by 4–10x. This is the first run to carry
`60b12d31`; the improvement is consistent with the fix, though this run also
hit a different, earlier blocker (below), so the two are not cleanly
separable from one run alone.

## Sustain reachability: no adoption effect, as predicted

t=0 has no standing cells, so `within_haul_reach` is a no-op here — exactly
what the offline baselines already showed. Electric `inserter`s do appear in
this plan (`craft 4 inserter`, placements at `[31.5,-29.5]` etc.), but they
are the science cell's own load/unload wiring — `assemble.rs`'s pre-existing
electric-inserter choice, not the new sustain-offtake leg. Checked, not
assumed: those coordinates match the cell's own construction, and the run
never reached the point where a *second* cell's offtake would be sited.

## What actually stopped the run

A different, already-named blocker, on the **first** plan this time:

```
a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it to
the supply chest at [42.5,-26.5]: the obstacle needs an underground span of 7
tiles and the belt allows 5
```

Same family as `run-1788923927-04849`'s 10-tile version and `run-1788920460-
08860`'s 4-tile perimeter, all instances of the science cell's coal ring
sealing its own exit. **Unowned, named tonight, offline-reproducible.** No
science this run — it never got past this to the science cell at all.

## What is settled and what remains open

Belt banding: **working, visibly, in a live run.** Sustain reachability: **no
adoption effect at t=0, exactly as designed** — its payoff is on a replan
against a standing world, already proven offline. The coal-ring exit is now
the single most-repeated blocker across six runs and the clearest next target.
