# The chest-free/lab run stalled hard: 0 tps for ~990 seconds, then finished

`run-1788964673-32436`, seed 31337, four headless bots at 10x, `--new`,
release binary built 16:37 from master `8e198bc0` (phase 1 + phase 3 of the
no-chests work, both merged). **Nothing cheated.**

## The measurement

```
outcome: done, 20.3 min wall, ticks 410 -> 720132 (199.9m game time)
delivered tick rate: 84 tps of 600 nominal (14%) -- STARVED, the server was
  not keeping up. 33 of 36 judged intervals under 80% of nominal.
worst: 0 tps over 25:54 -> 25:54 game time
sagged 19:34 -> 25:54 game time: 990 s of wall at 23 tps average
```

**A real stall, not an artefact of the analyser.** The tick profile shows
game time frozen at `25:54` across roughly 20 consecutive sampled intervals
while wall clock kept advancing. It eventually cleared, and the run went on
to accumulate 199.9 minutes of game time in 20.3 minutes of wall clock
overall — consistent with the game running near-normal speed before and
(apparently) after the stall, with one very bad stretch in the middle.

## What was happening around the stall

```
117× "too far away, moving first!" (whole run)
 14× "planned 1 steps" -- the plan collapsing to single-step replans
  8× "recovered: rescheduled"
  5× "failed to find player_path() for #2 to <coord>" -- five distinct
     nearby targets, all within a few tiles of each other
```

Bot #2 repeatedly failed to path to a cluster of coordinates around
`(-5..-8, -22..-29)`, got "stepped closer" corrections, and the supervisor
cycled through several single-step replans in the same wall-clock second
(`14:56:53` for all of them) before the run finally halted with:

```
milestone 1: stuck after 6 iteration(s), best 1 steps, last error:
the game reported no readable outcome: no action result received in time
```

## What is NOT established, deliberately

- **Whether this is caused by the new lab-chain/chest-free geometry
  specifically**, or a pre-existing executor/pathing issue that any run could
  hit, or box contention unrelated to either. The coordinates bot #2 failed
  to reach are close to where a lab chain would site (south of the product
  machine), but that is a coincidence worth checking, not a conclusion.
- **Whether the 0-tps stretch is the game engine itself stalling**, or the
  analyser's own RCON polling queued behind a burst of other RCON traffic
  from the rapid replan cycling, which would make the "0 tps" reading itself
  a symptom of the request storm rather than a second, independent problem.
- Whether this reproduces on a second attempt, or was a one-off.

Handed to a dedicated investigation rather than diagnosed further here — this
is a live-execution/performance question, not a planning-logic one, and
deserves the same "read the artifact before naming the mechanism" discipline
the rest of tonight has used, with attention specifically to `map.jsonl` and
`events.jsonl` around ticks corresponding to 19:34-25:54 game time.
