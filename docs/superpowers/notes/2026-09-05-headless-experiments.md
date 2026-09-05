# Headless mode as the main path: experiments

2026-09-05. Owner: "experiment with the headless mode and perfect it as it will
probably be the main way researchers use our project." This note is the log of
that work: each experiment states what was asked of the mode, what happened,
and what it taught about the system. Timings at 5x are validity checks, never
numbers to compare with a 1x client run.

## Setup

A second instance beside the default one, so nothing here touches a measured
run: `workspace/headless-a.toml` (game port 34210, RCON 4330, workspace
`workspace/headless-a`, scripts copied from `scripts/`), launched as

```
factorio-bot lua <script> --settings workspace/headless-a.toml --headless \
    --bots 4 --game-speed 5 --seed 31337 --new --logs
```

Things a researcher hits before the first run, in order:

1. **A second workspace has no scripts.** The CLI resolves a script name against
   `<workspace>/scripts` and never seeds that directory from the checkout, so
   the first run says `path not found`. Copied by hand here; should be seeded.
2. **The settings file is the only way to name ports and a workspace.** There
   is `--settings` and per-field overrides, but no `--instance <name>` that
   derives ports and a workspace from a name.

## Experiment log

### hl-01 — green, 4 bots, 5x, fresh seed 31337 (`workspace/session-logs/hl-01-green-5x.log`)

Launch to script: **63 s** on a fresh workspace (server extraction included);
the peer measured 12-13 s on a warm one. The plan is the same 623-action /
71,167-tick plan run 13 executed at 1x.

- **Reproduces run 13's pinch-point stalls exactly** — bots 1, 3, 4 stall at
  the same tick near (-12.5, -11.5)/(-12.5, -13.5) against our own furnace
  and drill. A 25-minute client run's early-game behaviour is reproduced in
  under two minutes here, deterministically. That is the case for the mode.
- **The stall classifier does not know character bots.** Bot 3's blocker is
  reported as "an undriven character … character (no player)"; at 1x the same
  event read "blocked by bot #1, walking". The walker's blocker naming resolves
  a character through `player`, which a headless bot has none of; it should
  resolve through the mod's bot registry. Defect, small, headless-only.
- **`factorio-bot rcon` ignores `--settings`.** It accepts the flag and then
  builds its connection from `FactorioSettings::default()`
  (`app/src-tauri/src/cli/rcon.rs:40`), so against a second instance it dials
  4321, finds nothing, and dies with `Timed out in bb8` — a pool timeout that
  says nothing about the port. A researcher with one non-default instance
  cannot ask their game anything. Fix: resolve the settings like `lua` does.

**Result: green end to end, 623/623, 0 failed, 0 lost, 194/194 walks, in 5 m 44 s
of wall time from launch to `RUN FINISHED`** (`run-1788607602-28753` under
`workspace/headless-a/runs`). Provenance says `bot_mode: characters`,
`game_speed: 5.0`, seed 31337, fingerprint `c161fa3f437221d0` — the same map
as run 13. `just analyse` reads it without complaint.

| | run 13 (clients, 1x) | hl-01 (characters, 5x) |
|---|---|---|
| green cell | 75,543 ticks (20:59) | **80,531 ticks (22:22)** |
| plan | 623 / 71,167 | identical |
| delivered tick rate | 60.0 tps | **250.2 tps** (of 300 requested) |
| walk ticks, 194 walks | 43,666 | 43,557 |
| dispatch→settle per verb (ticks) | place/insert/take 0; mine med 241; craft med 92 | identical |
| gap settle→next dispatch, same bot | n=812, **sum 36,845, median 10** | n=813, **sum 45,928, median 20** |

So a 5x run is a faithful execution of the same plan — every game-time
quantity matches to within noise — except one: **the executor's wall-clock
latency between actions**. At 1x each settle→dispatch hop costs ~10 ticks
(~170 ms); at 5x the hop is faster on the clock (~67 ms) but costs 20 ticks,
and 813 hops add ~9,000 ticks. Two consequences, both worth acting on:

1. **The 1x speedrun is paying ~37,000 bot-ticks (about 2 min of wall time
   on the critical path) to executor latency.** Mechanism not yet named:
   candidates are the mod's per-tick completion poll → `writeout` → stdout
   parser → watch channel → next RCON dispatch. A headless speed sweep
   (1x / 5x / 10x) separates the tick-fixed part from the wall-fixed part in
   minutes; that is experiment hl-02/03.
2. **A 5x headless time is systematically ~6% slower than 1x**, and the
   number is not comparable, as the record already says — but it is
   *predictably* slower, so it is a valid A/B instrument for plan changes.

Delivered 250 tps against 300 requested with the box otherwise quiet: the
server itself cannot hold 5x with four polled characters. `provenance` does
not record the delivered rate; the analyser derives it from `batch_progress`
(tick vs `elapsed_ms`), which should become a printed line.
