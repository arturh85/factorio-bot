---
name: diagnose-run
description: Use when a factorio-bot run looks stuck, failed, or produced a number you are about to quote - before killing it, before concluding a stall, and before trusting any measurement from its record
---

# Diagnosing a factorio-bot run

**Every wrong conclusion in this project came from a measurement, not from
carelessness.** A healthy run has been killed here on a confident stall
diagnosis. Work through this in order.

## 1. Is it actually stuck?

**Absence of events is NOT evidence of a stall.** A batch's events are written
only after `goal.run` returns, so a working run can be silent for 13+ minutes.
One was killed at 13m45s while executing normally, with bot inventories
climbing across consecutive samples.

Read `batch_progress` instead — it beats every 30 s with counters and no verdict:

```bash
python3 - workspace/runs/<run>/events.jsonl <<'PY'
import json,sys
ev=[json.loads(l) for l in open(sys.argv[1]) if l.strip()]
for e in [x for x in ev if x.get("kind")=="batch_progress"][-6:]:
    print(json.dumps({k:v for k,v in e.items() if k!="kind"})[:170])
PY
```

- **Counters advancing** → working. Leave it.
- **Counters frozen but `tick` advancing** → the game runs, one action is not
  settling. Go to step 2.
- **Advancing in bursts with pauses** → lag waits. Normal. Leave it.

## 2. Ask the running game — 30 seconds, not a 25-minute run

```bash
factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'

# who is doing what
factorio-bot rcon -s localhost -- '/c local o={} for _,p in pairs(game.players) do
  o[#o+1]=p.index..":"..(p.character and "char" or "nochar")
    ..(p.mining_state and p.mining_state.mining and " MINING" or "")
    ..(p.character and p.character.walking_state and p.character.walking_state.walking and " WALKING" or "")
end rcon.print(table.concat(o,"  "))'

# what the mod exposes
factorio-bot rcon -s localhost -- '/c rcon.print(serpent.line(remote.interfaces))'
```

A frozen-counter run once answered `1:char  2:char MINING  3:char MINING
4:char MINING` — bot 1 idle while the others worked, so the game was healthy
and **bot 1's completion signal was lost**, which is a different defect from
the lag wait the counters implied.

**Read-only during a measured run.** Provenance has no field recording a
mutation. Anything goes against a savepoint-resumed world.

## 3. Prefer waiting to killing

A run carries its own timeout (5400 s). Letting it expire costs nothing extra
and yields a complete record. **Never kill a process you did not start.**

## 4. Traps that have produced wrong answers here

- **Action ids are PER-PLAN, not global.** 44 of 147 collide in one run.
  Keying an aggregate on `id` once understated a bot's executing time by 8,067
  ticks; joining on it across a plan boundary invented three "blocker cleared
  in 10,000-40,000 ticks" figures that were replan boundaries. **Aggregate over
  settle events; identify actions by `(id, dispatch order)` or by position.**
- **Check the roster first.** `plan_created.bots` must be `[1,2,3,4]`. A
  one-bot run is a *different plan*; a good commit was reverted over this.
- **A verb histogram cannot see waiting.** Verbs settling in their dispatch
  tick contribute 0, and idle time appears nowhere. "88% hand-mining" was 88%
  of the *timed* verbs while 39% was one bot waiting.
- **Game time, not wall clock.** One run read 24.4 min on the clock and 21.4
  in game.
- **Runs are only comparable if the map is.** Use `--seed`, and note it was
  silently ignored before `61ec7364`, so every earlier run used an
  unidentifiable map.
- **An error can name the wrong thing.** `schedule.rs:543` reclassifies any
  rejected candidate whose chain has an owner, so a plain world-state failure
  inside an owned chain always arrives dressed as a chain-ownership problem.

## 5. Use the tooling rather than one-liners

`just analyse` (`tools/run_analysis.py`) reports milestone spans, per-verb
dispatch→settle ticks, steps/bot, failed walks, frozen-position detection,
sample coverage, per-network power and per-machine status. It exists because
ad-hoc one-liners produced unrepeatable wrong answers. `--compare` refuses
comparisons that mean nothing (differing seed, commit, profile, `resumed_from`).

Note `busy_ticks` is a *sum* of intervals, not a union, so it can exceed the
window and read over 100% since per-bot concurrency landed. `idle_gaps` is the
overlap-aware figure.
