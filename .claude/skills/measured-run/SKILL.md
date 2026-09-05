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

## 0.5 Can this run headless? Usually yes, and it is ~5x cheaper

**`--headless` replaces the graphical clients with server-side `character`
entities.** No client process, no window, no display, no GPU, and no connect
wait: **the script starts 12 seconds after launch instead of minutes.** With
`--game-speed 5` a measured acceptance run did **14.9 minutes of game time in
3.0 minutes of wall clock** (`run-1788597952-96167`, 4 bots, seed 31337, all
three `factory_stage2` milestones satisfied, `outcome: done`).

```bash
just headless factory_stage2.lua          # 4 character bots at 5x
factorio-bot lua <script> --headless --bots 4 --game-speed 5
```

**Use headless for** iterating on the planner or executor, reproducing a
failure, and anything where the answer is "did it work", not "how long did it
take".

**Use clients (`just bench`) for** a number you will quote, and for anything
filmed — video is captured from a client window, so a headless run records
everything except video and says so.

**A run is all clients or all characters.** The mix is refused by name, in the
mod and in core, before a process is spawned; a human joining while character
bots exist is refused too.

**Three things to know before trusting a headless result:**

- **Never compare a 5x headless timing to a 1x client run.** Provenance records
  `bot_mode` and `game_speed` for exactly this reason, and `just analyse`
  prints a difference as a note rather than refusing.
- **Trigger technologies are emulated, and only the `craft-item` ones.**
  Factorio 2.0 unlocks 32 technologies by doing rather than researching, and
  the game fires those from *player* actions a characterless bot never
  performs — a headless run once crafted a lab, placed it, and still could not
  craft red science. The mod now completes such a technology when the force has
  already produced what the trigger names, writing a
  `research_trigger_emulated` event each time. The 11 `mine-entity` triggers
  (**including `oil-processing`**), plus `build-entity` and the two space ones,
  are **not** emulated, because the mod cannot read their condition — so a
  headless run still cannot cross them.
- **Nothing above 4 bots has been run.** Bot ids are `u8`, so 255 is the
  ceiling, and the mod polls every bot every tick (whole inventory read, sorted
  signature, crafting-queue scan), so bot count is the first thing that would
  cost tick rate.

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
