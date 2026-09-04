---
name: measured-run
description: Use before starting any live Factorio run in factorio-bot - the pre-flight checks, how to launch, and how to measure the result so the number is worth quoting
---

# Running a measured experiment

A run costs 20-60 minutes. **Six runs in one night died on a stale mod copy**
that four seconds of checking would have caught. Do the checks.

## 0. Can this be answered offline instead?

Most planner questions can, in ~4 seconds:

```bash
factorio-bot plan --world workspace/scripts/map.json \
    --goal researched:automation --bots 1,2,3,4 --steps
factorio-bot score-map --world workspace/scripts/map.json --bots 1,2,3,4
```

**Three blind spots** — each has produced a wrong "the bug is absent":
`world.dump` never calls `refresh_buffers`, so a dump's `inventories` is `[]`
and the whole `Withdraw` path is unreachable; a `--resume-from` savepoint
restores *saved* inventories and positions, not the ones at the moment of
failure; and a fresh map has charted almost nothing.

If the question is about live game state, `factorio-bot rcon -s localhost` is
faster than a run. See the `diagnose-run` skill.

## 1. Pre-flight — never skip

```bash
command ls -l workspace/mods/BotBridge          # must be a symlink to mods/BotBridge
grep -c set_recipe workspace/mods/BotBridge/control.lua
grep -o '"BotBridge"' workspace/mods/mod-list.json | head -1   # present == enabled
(ss -lntu || netstat -lntu) | grep -E "34197|4321" || echo "ports free"
git status --porcelain                          # no agent mid-edit
```

- **Never delete `workspace/mods/BotBridge`** — every instance's mods dir is a
  symlink to it. Deleting leaves the server with no bridge mod: it hangs at
  `start waiting` and writes a `level.zip` that poisons every later run. Copy,
  do not delete.
- A mod present on disk but absent from `mod-list.json` is a **disabled** mod,
  and that failure is silent.
- **Scripts are resolved against `workspace/scripts/`, not the repo**, with no
  fallback. Copy the script across before running it.
- **Never start a run while an agent is building.** `cargo` linking to the
  output path trips over a running binary (`ETXTBSY`).

## 2. Launch

```bash
nohup env DISPLAY=:0 nix develop -c timeout 5400 \
  ./target/debug/factorio-bot lua <script>.lua \
  --clients 4 --bots 4 --seed 31337 --logs > "$SP/run.log" 2>&1 &
echo $! > "$SP/run.pid"
```

- **Every command needs `nix develop -c`**; the binary cannot load `liblzma`
  outside it, and `DISPLAY=:0` is required for graphical clients.
- **`--seed` only takes effect with `--new`**, which **deletes the map**. Back
  up `workspace/server/saves/level.zip` first — `workspace/known-good-map/`
  holds a copy.
- Capture the **PID**, and test liveness with `kill -0 "$PID"`. Never key a
  monitor or `pkill` to a process *name*; `pgrep -f` also matches the shell
  running it.

## 3. Confirm the roster before believing anything

```bash
grep -m1 plan_created workspace/runs/<run>/events.jsonl   # bots must be [1,2,3,4]
```

A stall does **not** abort the run — it proceeds with whoever turned up,
logging `Gave up waiting for clients`. That degrades silently to a one-bot
roster and a *different plan*. A one-bot run misread as a four-bot regression
cost a good commit a revert.

## 4. Measure

**Game time, from `run_started` to `milestone_satisfied`** — not wall clock,
not roster-ready.

```bash
python3 - workspace/runs/<run>/events.jsonl <<'PY'
import json,sys
ev=[json.loads(l) for l in open(sys.argv[1]) if l.strip()]
start=next(e["tick"] for e in ev if e.get("kind")=="run_started")
for e in ev:
    if e.get("kind")=="milestone_satisfied":
        print(f'milestone {e.get("index")}: {(e["tick"]-start)/3600:.2f} min')
PY
```

**Structural rungs are not evidence of production.** `producing(item, n)` means
the cell *stands* — machines placed, recipes set, links present, power
headroom. `PlanState` reads no container contents and no fuel level, so an
empty cell satisfies it exactly as a working one does. **Only a
`supervisor.witness` is evidence**: it dispatches nothing, reads output
inventories before and after, and asserts the count rose while every bot stood
still.

## 5. Record it

Quote the **seed** with every number. Compare with `just analyse --compare`,
which refuses comparisons across differing seed, commit, Factorio version,
build profile or `resumed_from`. Milestone savepoints
(`runs/<run>/savepoints/`) let a later experiment resume instead of re-deriving
the prelude: `--resume-from <run>[:<milestone>]`.
