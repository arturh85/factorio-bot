# An offline plan cannot see the replan path, and every baseline is offline

`run-1788923927-04849`, seed 31337, four headless bots at 10x, `--new`, release
binary built 05:14 from `9f549b1c`. **Nothing cheated.**

## The contradiction, measured both ways on one binary

| | actions | result |
|---|---:|---|
| offline, `plan --world workspace/scripts/map.json`, bots 1,2,3,4 | **885 / 52,891** | plans through to `place assembling-machine-1` and `set assembling-machine-1 to automation-science-pack` |
| live, same goal, same binary | **1,067 planned** | REFUSES, 48 abandoned, zero science |

The live refusal is byte-for-byte the one `9f549b1c` was written to remove:

```
a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it to the
supply chest at [30.5,-24.5]: from the iron-chest at [29.5,-46.5]: no belt route,
blocked by 4 tile(s): [28.5,-46.5] [29.5,-48.5] [29.5,-45.5] [30.5,-46.5]
```

`action verdicts: {success: 835, abandoned: 48, failed: 1}`. Fleet utilisation
24.7%. `automation-science-pack` 0 at every mark.

## What the refusal's own wording locates

**"a cell ALREADY MAKES copper-plate".** That is the *replan* path — the
supervisor plans, executes, something forces a replan, and the second expansion
meets a cell that is already standing with its surroundings already laid as map
facts.

**An offline plan from the t=0 dump can never reach it.** There is no cell yet,
so the exit is chosen fresh on clean ground and everything works. The fix is
genuinely correct for the world it was tested against; that world just is not
the one the failure lives in.

## Why this is bigger than one fix

**Every baseline this project takes is offline**, against `map.json`, from t=0.
That is deliberate and it is why the loop is fast — `db612be9` bought seconds
instead of twenty-minute runs, and it has caught a planning ceiling, a
capability gap and a whole workstream's result without spending a run. Nothing
here argues against it.

But it means **the entire baseline regime is blind to replanning**, and
replanning is not an edge case: it is what the supervisor does every time a
batch is truncated, which tonight is every run. A change can move all eight
baselines correctly and still fail the only path that matters.

CLAUDE.md already says *"a dump is t=0-shaped unless you make it otherwise"* and
lists it under three blind spots that have each produced a wrong "the bug is
absent". This is a fourth instance, and the first where the blind spot swallowed
a fix that had been measured, reviewed and merged.

**Nothing enforces it.** There is no test in the tree that plans against a world
with a half-built factory standing in it.

## The cheap remedy that already exists

`--resume-from <run>[:<milestone>]` starts a run on a milestone savepoint, and
every milestone writes one. So a regression test for anything on the replan path
can begin from a world where the cell already stands, rather than from t=0. That
turns "reproduce the failure by paying for the whole prelude again" into a short
start plus an offline plan — which is exactly what the savepoints were built for
and what nobody has used them for here.

## What is NOT concluded

Which of two mechanisms causes it. Either the reservation is never re-derived on
replan (so `choose_exit` may not run at all when the cell is found standing, and
the tile it reserved is ordinary ground to the second expansion), or the exit is
kept but sealed — the fix's own author measured that on seed 31337 *"every kept
exit is sealed"* by a closed ring of four coal runs, so a route out must tunnel,
and tunnelling needs `logistics`. Those predict different fixes and the record
can tell them apart: read what stands on those four tiles at the moment of the
replan, and which run placed each. Left open rather than guessed.
