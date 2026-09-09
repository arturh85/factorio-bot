# A run that never built its assembling machines, despite six replans

`run-1788949638-11792`, seed 31337, four headless bots at 10x, `--new`,
release binary built 12:26 from master `23c30a3f`. **Nothing cheated.**

## This is worse than the two milestone runs, and it must be said plainly

**No `assembling-machine` appears in the machine census at any mark, across
34.7 minutes of game time.** `automation-science-pack` stays at 0 throughout.
Two earlier runs on older binaries (`run-1788926478-07032`,
`run-1788931904-77495`) each built the science cell and made 14–15 packs. This
run does neither, despite six planning iterations against three of the
night's fixes.

## What actually happened: thrashing, not progress

`craft 2 assembling-machine-1` — and every step downstream of it (`place`,
`set` recipe) — was **abandoned in every one of at least four separate
attempts** (ticks 46,086/51,544, 63,781, 71,790, 72,326). Each replan after
the first correctly identifies the same unfinished work (place 2 machines, set
2 recipes) — that is the cell-recognition fix (`5323e71a`) working exactly as
designed, and it deserves credit for that — but the crafting step it re-tries
**fails at the same point every time**: a `take copper-plate` that removes 0,
the same `take_in_pieces`-hits-its-bound shape diagnosed earlier tonight.

So the sequence is: replan sees unfinished machine work → retries it → hits
the same copper shortage → abandons → replan sees the *same* unfinished work
→ retries → same failure. **Six iterations of that is not six attempts making
progress; it is the same attempt failing the same way six times.** The
cell-recognition fix has no way to tell "this is worth retrying" from "this
will fail the same way every time" — it only recognises what's unfinished,
not why the last attempt at it failed.

## The splitter-tap fix was never exercised

**Zero splitters placed.** The boxed-in-perimeter scenario the fix targets
apparently never arose in this run's particular sequence — this run never
got as far as needing a second route out of a fully-consumed chest. So the
fix remains **offline-verified only** (against `run-1788946451-86723`'s own
recorded state), not yet confirmed on a fresh live run. Not a failure of the
fix; simply not the failure mode this run hit.

## What this run adds to the open list

A new, real problem, distinct from the perimeter-sharing question and from
cell recognition: **a replan that keeps recognising the same leftover work
has no signal for "this keeps failing the same way, something upstream needs
attention" versus "this hasn't been tried yet."** Left open, not fixed here.
