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

### hl-02 — automation, 4 bots, **1x**, fresh seed 31337 (`run-1788608011-14361`)

The 1x point of the latency sweep, and the first headless run to fail a walk.
Result: `researched:automation` in **29,364 ticks (8:09)** over two plans,
against 22,271 (6:11) for the release client bench on the same seed. Not a
speed effect — delivered rate was 59.5 tps — but a replan: at tick 8,513 bot
4's walk to (-19, 24.4) was **refused before dispatch** by `judge_path`
(`walk_ends_where_nobody_can_stand`, `crates/core/src/factorio/rcon.rs`): the
route the game's pathfinder returned ended at (-18.56, 22.24), inside a
2.0 × 1.9 collision box at (-17.56, 21.55) — a rock, by its size — 2.2 tiles
short of the goal the planner asked for. The refusal is correct (the follower
would wedge), the classifier filed it as `kind: other` (the wording is not in
`classify_walk_failure`), and the batch replanned at 103/177. The replan's
iron cell had one drill feeding one furnace, so `take 50 iron-plate` waited
~4 minutes (a lag wait with no deadline — legitimate, and not a deadlock, as
`rcon --settings` confirmed live: 47 plates and climbing, four idle
characters). Cost of the refused walk: ~7,000 ticks.

Open question for the RCA: why does a path *request* aimed 3.2 tiles beside
a rock come back ending inside it? The request carries a goal radius, and a
waypoint within that radius but inside a solid box is a legal answer from
Factorio's pathfinder only if the box's mask does not collide with the
request's — which a rock's does. Either the request's radius is too wide, the
goal is nearer the rock than the planner believes, or the entity graph's box
for that rock is stale. To be decided from the record, not guessed.

Also seen at 1x, and headless-only: **bot 3 stalled against bot 1 and the
message read "blocked by an undriven character … character (no player)"**.
`walk_stall_describe` (`mods/BotBridge/control.lua:496`) names a character
through `LuaEntity.player`, which a server-side character never has, so a
walking bot reads as a parked one — and the comment on the function says why
that matters: `step_aside_from_footprint` steers only a blocker that is
neither walking nor mining, and a caller waiting for a "(no player)" blocker
to move is waiting for nothing. It should resolve through the mod's bot
registry (`storage.bots`) and say `character #N (walking)` as it does for a
client bot.

Latency point: median settle→dispatch hop **8 ticks at headless 1x**, against
10 in run 13 (clients, 1x, debug), 16 in the release client bench, 20 at
headless 5x. So the hop is ~130 ms of wall time whatever the speed, and the
mode does not add latency of its own; the client runs pay slightly more,
presumably the four graphical clients competing for the box.

### hl-03 — green, 4 bots, 5x, tail fix merged (`run-1788608648-56109`) — STUCK after four plans

Intended as the A/B against hl-01 (same seed, same speed, the `tail` merge
the only change). The plan came out as the offline one, 569 / 57,752, and
`research automation` ran on bot 3 as designed. Then the run replanned four
times and halted `stuck` at run tick ~100,700 (28 min game time). No number
from it is usable; the record is.

| plan | steps / makespan | ended by |
|---|---|---|
| 1 | 569 / 57,752 | `place burner-mining-drill` refused 4× over 543 ticks: *a character is standing in the footprint* (tick 35,206) + 1 lost |
| 2 | 316 / 29,080 | `place assembling-machine-1`, same refusal (72,575) |
| 3 | 93 / 16,675 | `place assembling-machine-1`, same refusal (83,971) |
| 4 | 96 / 11,051 | walk to [-13, -12] no path (best ends 9.5 tiles away) + 1 lost |
| 5 | refused | *the nearest water is 69.3 tiles away, but no shoreline within 10 tiles of it has room for a pump, a boiler, a steam engine and the pipes between them* |

Three mechanisms, all now dispatched:

1. **A headless character has no player, and everything that identifies a
   blocking bot through `entity.player` misses it** (`walk_stall_describe`,
   and whatever the pre-place step-aside uses). With clients the standing
   bot is asked to move; here the placement is refused four times and the
   action fails, which costs a replan each time. Worktree `charid`.
2. **Walk targets against our own furnace/drill cluster at spawn** and a
   route that ends inside a rock: worktree `rockpath` (see hl-02).
3. **A replan does not reuse what the previous plans built.** The final
   keyframe (`map.jsonl`, tick 101,156) holds **3 offshore pumps, 2 boilers,
   1 steam engine, 4 labs, 6 assemblers, 31 stone furnaces and 20 iron
   chests** for a goal that needs one of each power part, one or two labs
   and four assemblers. Each replan sited a *new* steam site until the
   shoreline had no room, and then the planner refused the goal on a map it
   had already powered. The world model must recognise an existing pump →
   boiler → engine → pole network as satisfying `Powered`, an existing lab
   as a lab, and an existing cell as the cell — for a replan, and for every
   researcher who starts a run on a save that already has a factory.
   Worktree `reuse`.

What the mode taught in one hour: a client run at 1x hides all three behind
the step-aside and the absence of replans; the headless run at 5x surfaced
them in six minutes each, with a record that names the tick.

### charid — merged: a character bot in the way is a bot, not a stranger

Two layers, both in `mods/BotBridge/control.lua`, core untouched. (1) Both
blocker sites resolved a character through `LuaEntity.player`, nil for every
registry character: `walk_stall_describe` said "character (no player)" and
`step_aside_from_footprint` asked nobody to move — the four-refusals-over-543-
ticks pattern. Now `bot_of_character(entity)` resolves through `entity.player`
then `storage.bots`, so a character bot reads `character #N (walking)` and
gets the same step-aside walk. (2) Found by the first live run with (1) alone:
the step-aside landing could sit in the 0.6-tile crack between two assemblers,
within the walker's 0.3 arrival box, still inside the footprint; landings now
must clear the footprint by the character half-box + arrival half-width.
Three stub-game tests in `crates/core/tests/botbridge_*`. Live, headless-b at
5x, seed 31337: run 1 (layer 1 only) one refusal, two plans; **run 2
(`run-1788610299-95745`) zero refusals, one plan of 569 steps, green
witnessed, `state=done`.** Under load ~30 from other builds, so no timing.
