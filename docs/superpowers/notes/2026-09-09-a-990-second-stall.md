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

## RETRACTED, same day: the game never stalled. The record's clock did.

Investigated from the record alone (`events.jsonl`, `samples.jsonl`,
`map.jsonl`), no second run. Every number above was computed correctly from
`batch_progress`, and `batch_progress` was lying about one field.

**The tick on every heartbeat is `FactorioRcon::last_tick`** -- the stamp off
the last command *sent* (`LiveRecord::record`). The one action in flight was
`research logistics (fed by the cell)`, dispatched at tick **93,673**, and
`research_timed` waits for it by polling an in-memory table every 50 ms for
`sized_deadline(18000)` = 18000/60·3 + 60 = **960 s of wall clock**, unscaled
by game speed, sending nothing. So thirty-one consecutive beats carried tick
93,673 while `elapsed_ms` ran 180 s → 1,110 s, and the analyser's
`delivered_tick_rate` -- which is beat tick over beat wall time -- read that
as *0 tps* and *84 of 600 tps, STARVED*. The "sag 19:34 → 25:54 at 23 tps" is
ticks 70,867 → 93,673 over the 990 s between two beats: same artefact.

What the game was doing meanwhile:

```
mod bot samples between tick 93,673 and 666,015:   9,539 of 9,539 expected
   (one every 60 ticks, no gap wider than 120 ticks anywhere in the run)
next tick the executor observed after the timeout: 666,015
   572,342 ticks in <= 960 s  ->  >= 596 tps of 600 nominal
```

The game ran at full speed through the whole "stall". Nothing on the box
starved it (hypothesis 3 falsified by the sample cadence), and there was no
RCON storm (hypothesis 2 inverted: the freeze happened because *nothing* was
sent, not because too much was). Hypothesis 1 -- the lab-chain geometry -- is
falsified too: every one of the 315 walks settled `success` and none carries a
stall; the five `player_path` failures for bot 2 were around the stage-1
ore-to-plate cell at `(-5..-8, -22..-29)` (drill, furnace, belts, burner
arms placed at ticks 27k-36k), retried inside the mod, and never reached the
record as failures. They are 40,000+ ticks before the research and unrelated.

**Why the research did not finish** is the real finding, and it is upstream
of the executor. The plan's research is "fed by the cell": its lag is
`units × ticks_per_item` on the assumption the cell holds its tempo. Read off
`samples.jsonl` at the moment of dispatch (tick 93,000):

```
lab             no_research_in_progress, holding 2 packs   (progress -> 0.15 = 3 of 20)
asm (packs)     item_ingredient_shortage, 2 copper-plate, no gears
asm (gears)     item_ingredient_shortage, empty
7 furnaces      no_ingredients (one still working)
boiler          no_fuel  -- already, before the research was dispatched
force           automation-science-pack made 3 in the whole run
```

The cell made **3 of the 20 packs** logistics needs and starved: no ore
reaching the furnaces, no coal reaching the boiler, `no_power` on both
assemblers from tick ~150,000. Nothing in the plan sustains the cell's inputs
for the 18,000 ticks the research was budgeted at, so the research could not
have finished in any deadline. That is the "cell outlives its charge" question
again, on the lab-fed path this time, and it is the owner's.

**And then it could not be retried**, which is the one executor defect here.
After the timeout the supervisor replanned (248 steps, all 247 dispatched
settled `success`, the cell re-fuelled) and re-dispatched the research. The
mod's `start_research` refused it: `add_research` answers false for a
technology that is *already* current research, and the mod treated that as a
refusal -- exactly the state rung 7 had found on 2026-09-02, when only the
*message* was fixed. Six single-step replans in one wall-clock second, each
refused the same way, then `stuck`. The actuator's own doc says "two bots
researching the same technology is idempotent in Factorio"; at the mod it was
not.

Landed with this retraction:

- `batch_progress` asks the game for its tick (`Actuator::game_tick`, one
  round trip per beat, the same call `wait_out_lag` makes) and records at
  that tick (`LiveRecord::record_at`). A wait that sends nothing can no
  longer freeze the record's clock, and `just analyse`'s tick-rate profile
  becomes true through such a wait rather than reading a frozen stamp as a
  frozen game.
- The mod joins a research that is already current or queued instead of
  refusing it: the action id stays registered and settles on
  `on_research_finished` like the first dispatch would have. A refusal still
  names `current_research` / `in_queue`, which can now only ever be a
  *different* technology.

Not landed, stated for the owner:

- `sized_deadline` is wall-clock and not scaled by `game_speed`. 960 s at 10x
  is 576,000 game ticks for an 18,000-tick research -- 32x the budget, and
  16 of this run's 20 minutes. The honest bound for a research is progress:
  `force.research.progress` was flat at 0.15 from tick 102,000 on, and the
  record could see it 90 minutes of game time before the deadline did.
- The lab-fed research needs its cell sustained for the whole research, or
  the research needs to be sized to what the cell can make before it stops.
  Offline this is `plan --standing-from-run run-1788964673-32436 --at-tick
  93000`; nothing here needs a live run to reproduce.

The lesson is the one this repo already has under *silence is not success*,
from a new angle: **a clock that is only advanced by sending something reads
as stopped exactly when the system is waiting, which is exactly when a reader
wants to know whether it is alive.** The tell was that the mod's own samples
-- on a tick cadence, not a command cadence -- disagreed with the executor's.
Two clocks that disagree are a measurement; one clock is a belief.
