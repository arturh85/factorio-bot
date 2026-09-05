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

### rockpath — merged: the approach aim stands clear of what the graph knows

RCA answer to hl-02's refused walk and hl-03's "no path to [-13,-12]": neither
the pathfinder, the collision mask, the planner's clearance nor the entity
graph — **the executor's own aim**. `approach_annulus` put the goal
`min_radius + slack` from the target *toward the bot*, which for bot 4's rock
landed 0.7 tiles inside the neighbouring big-rock (prototype box
`{{-1,-0.90},{1,1}}`, in the dump); and the insert/fuel walk to a furnace is a
disc centred on the furnace itself, so the offset-goal fallback ended on the
bot's own position. Both refusals were correct and both inputs were wrong.
Now `approach_standing` bounds the inner radius by the target's own
clearance, keeps the annulus aim unless the graph proves it blocked, else
sweeps 16 bearings per ring (bot's side first) and one ring out; the walk
retries once at a tight path radius when only the route's end is inside a
box; and the refusal is classified `DestinationBlocked` instead of `other`.
Six tests; core/executor/scripting_lua tests, workspace clippy, contract spec
green; offline plans byte-identical (the planner is untouched). Also learnt:
a pre-dispatch refusal's `walk_settled` tick is the record's high-water mark,
which reads as "refused at batch end" (4,400 and 7,800 ticks late here) —
open item; and bots wedge in the 0.6-tile gaps between touching furnaces
(the game routes a 0.4-wide character through them) — open item.

### reuse — merged (`28d101d9`): a replan finishes what it finds standing

Four mechanisms behind hl-03's three pumps, two boilers, four labs and six
assemblers, all in `crates/planner`: (1) `power.rs` `supply_for` had no tier
between "adopt a pole that reaches a generator with headroom" and "site a new
plant", so a pump + boiler with no engine read as no supply, and its own
pump blocked its own shoreline site, so the next tile up the shore won;
(2) `state.rs` `electric_demand_kw` charged full draw for every recipe-less
assembler of an abandoned cell, so the one working engine read as full and
a third plant was sited, after which the shoreline was full and the goal
refused; (3) `assemble.rs` counted only complete cells and sited only on
clear ground ("nothing here plans repairs"); (4) `have.rs` `lab_site`
ignored a standing lab that was merely unpowered. Now a standing pump
anchors a plant to finish, dead machines draw nothing, a partial cell is
finished around its machines, and a dark lab is lit with a pole. Three
integration tests (`tests/standing_site_reuse.rs`) fail before and pass
after; eleven unit tests; planner suite and clippy green; t=0 plans for all
three goals byte-identical. Open: parts placed with drift are passed over;
partial-cell inserter facing is checked only through `delivers_into`.

### hl-04 — green, 4 bots, 5x, everything merged (`run-1788611922-87269`)

The A/B hl-03 was meant to be, now with `tail`, `character-identity`,
`walk-into-rock`, `replan-reuses-site` and the peer's `belt-routing` on
master, workspace tests green.

| | hl-01 (before) | hl-04 (after) |
|---|---|---|
| plan | 623 / 71,167 | 569 / 57,752 |
| green cell (5x) | 80,531 (22:22) | **68,051 (18:54)**, −15.5% |
| green witness | 82,828 | 70,240 |
| plans / failed / lost | 1 / 0 / 0 | 1 / 0 / 0 |
| walks | 194 / 194 | 384 walk events, none failed |

One pathfinder no-path early, recovered. Executed/planned is 1.18 here
against 1.13 in hl-01, so the plan tightened more than the execution did;
where the extra 10,000 ticks go is the next thing to read off the record
(the 5x hop tax is ~9,000 of it on hl-01's numbers). Run 14 at 1x with
clients is the honest measurement of the same master.

### aimbots — merged: the aim stands clear of the other bots too

Run 14's one failed walk: the annulus aim for bot 1 landed 0.515 tiles from
idle bot 3 (two character half-widths 0.398 + the follower's 0.3 stop box),
invisible to the entity graph, visible to the game's pathfinder. Now
`approach_standing` takes the walker's id and treats every other bot's
character footprint at its `world.players` position (fed by the mod's
position writeouts; exact for a resting bot) grown by 0.3 as an obstacle:
free candidates first, else the farthest from any bot, never a refusal. No
extra RCON round trip. Three tests from the run's own coordinates; core and
executor tests and clippy green. Judgement: a walking bystander's up-to-a-
tile-stale position is treated as occupied (conservative).

### hl-05 — two headless runs at once (a: warm workspace, c: brand-new), automation, 5x

| | headless-a `run-1788614064-08543` | headless-c `run-1788614152-60729` |
|---|---|---|
| automation | 22,724 ticks (6:19) | 22,670 ticks (6:18) |
| plans / failed | 1 / 0 | 1 / 0 |
| delivered tps | 217.9 | 228.6 |

Both against the 1x client bench of 22,271 (6:11) on the same seed, and
with the box at load ~9 from another session's builds. So two instances on
their own ports and workspaces coexist, each holds ~3.7x of its requested
5x under contention, and the pair costs about as much wall time as one
(~3 min from launch to done). **Parallel runs are the cheap axis**, as the
headless section of CLAUDE.md predicted; the limit is cores.

Two things a researcher hits on the way, one fixed and one not:

- **A brand-new `workspace_path` is refused**: `Failed to find workspace …
  failed to find workspace! help: correct settings.workspace_path to a valid
  directory` (`factorio::workspace::not_found`). Everything under it is
  derived from the archive path, so the directory should be created, with a
  line saying so. Open — a small `instance_setup` change.
- **Scripts seeding** (merged `3cd57c70`): on the second attempt, with the
  directory made by hand, the run printed `Using scripts directory
  ".../headless-c/scripts" (debug build; seeded by copying ".../scripts" …)`
  and ran. Before today this would have been `path not found`.

### hl-06 — automation at 10x (`run-1788614294-64261`), and a correction

| speed | run | automation | plan | executed/planned | wall from launch |
|---|---|---|---|---|---|
| 1x clients (release bench) | run-1788582657-14978 | 22,271 | 21,985 | 1.01 | ~8 min |
| 5x headless (beside another run) | run-1788614064-08543 | 22,724 | 21,765 | 1.04 | ~2.5 min |
| **10x headless, quiet box** | run-1788614294-64261 | **23,715 (6:35)** | 21,765 | 1.09 | **~80 s** |

One plan, nothing failed, at every speed. 10x delivers roughly 360 of the
600 ticks/s asked for on this box with four polled characters, and still
gets automation done in eighty seconds of wall time.

**Correction to hl-01's reading.** The settle→next-dispatch hop is not
~10-20 ticks: restricted to hops under 60 ticks the median is **1 tick at
1x, 5x and 10x** (n = 151 / 121 / 113). The larger "gaps" I summed earlier
were legitimate waits (predecessors, lag, research). So the executor's
dispatch path adds no per-action latency worth chasing, and the speed tax
(+4% at 5x, +9% at 10x on automation; +6% at 5x on green) lives inside the
long waits — the tick-polled lag wait sleeps a wall-clock estimate between
readings, and at speed each sleep is worth more ticks. That is the mechanism
to measure next, not the hop. The open item "per-action executor hop
latency" in the state memory is withdrawn.

### hl-07 — eight character bots, 5x, on a workspace that did not exist (`run-1788614781-38058`) — STUCK

`Created workspace ".../headless-d" (new instance; …)` printed and the run
started (merged `675f93b7`); `Using bot mode characters (8 requested, ids
[1..8])`; the planner divided 298 steps over eight bots. Then:

- **The first walks of bots 1, 5 and 6 were refused from the spawn pile**
  ((0,0), (0,-0.5), (0,0.5): eight 0.8-tile characters spawned within half
  a tile of each other) and the walk memory learned three destinations as
  unreachable — for the map, not for the pile. With four bots this never
  happens. Spawn must spread the characters (`find_non_colliding_position`
  per bot), and a refusal whose start is inside other characters must not
  be learned.
- **Bot 6 was wedged by a furnace.** Bot 3's plan-2 `place stone-furnace at
  [-5, -28]` went down while bot 6 was at/through (-5.2, -29.1); the
  furnace's box spans y −29..−27 and bot 6's 0.4 half-box overlaps it. A
  real player is pushed out by the game; a server-side character is not,
  and the mod's in-footprint check sees bots standing, not walking through.
  From then on every path request from bot 6 failed, the walk memory said
  "not boxed in" (it reasons over the entity graph, which holds no
  characters and evidently not that furnace's box against the bot's), and
  **six replans handed bot 6 the same walk to (12.5, −33.5)** with
  `learned=false` each time — the ledger knew and the plan did not care.
  Halted `stuck` with seven healthy bots idle.

Dispatched: `spawn` (mod: spread the spawn; placement over a character,
standing or walking, either refuses or pushes it out the way the game does
for a player) and `bench` (executor/planner: a refused walk from a start
the game cannot leave is a boxed-in bot, not an unreachable destination;
the ledger's answer must reach the next plan; a bot that cannot move is
benched and the roster continues without it).

### bench — merged: a boxed-in bot is probed, benched and released

Three mechanisms behind hl-07's seven plans for one wedged bot: (1) the
walk ledger only reorders candidate tiers in `schedule.rs` and never empties
one, and bot 6's coal share had one candidate (itself); (2) the enclosure
fill seeds from the tile centre and holds no characters, so a character
overlapping a furnace's edge read `Open`; (3) nobody ever asked the game
whether the character could move. Now a no-path refusal triggers four
3-tile hop probes over the existing `async_request_player_path`; all four
definitively refused = `BoxedIn`, the bot is benched where it stands
(`bot_benched` event, `boxed_in` failure kind), the planner gives a benched
bot no work with travel and refuses by name if the whole roster is benched,
and a successful walk, a pre-plan re-probe, or moving a tile away releases
it. 21 new tests across executor, planner and scripting; offline automation
unchanged at 176 / 21,765. Not yet seen live; the spawn branch's eight-bot
run is the first chance.

### spawn — merged (`f24c02a6`): eight bots on eight tiles, and my furnace story refuted

The agent read the record instead of trusting my summary: bot 3's furnaces
at [-5,-28] and [-6,-30] went down at ticks 15,852–15,854 while bot 6 was
still mining copper 40 tiles away; bot 6's later walk followed a path
computed before those furnaces existed and **steered it into the 0.6-tile
crack between them**, touching a tile a furnace covers, from which the
pathfinder will not start. Nothing was built over anyone. Three changes in
`mods/BotBridge/control.lua`: characters spawn on their own tile centres
two apart in a square spiral (the pile was eight 0.8-tile characters in one
2×2 square); a second footprint scan right before `create_entity`, and a
push-out with a `teleport` record and `pushed_out` in the reply if a build
ever lands on a character anyway; and the actual fix — a walker stalled on
a tile something solid covers re-targets once at the nearest clear tile
(`walk_step_clear` event), then fails the walk with the stall's cause so
the executor re-paths from clear ground. Ten stub-game tests. Live on
headless-e, 8 bots, 5x: **one plan (296 steps), automation at tick 19,968
(≈5:26 from start), 113 walks, 0 failed, 0 teleports** — against six
replans and `stuck` before. Four bots took 6:18 on the same goal.

### hl-08 — green, eight bots, 5x, everything merged (`run-1788617269-96746`)

One plan, 693 actions / 50,665 ticks, **0 failed, 0 lost, 500 walk events
none failed**, three pathfinder no-paths recovered by the executor, no
bench engaged. Green at **71,936 ticks (19:58)**, witness 20:34, 242 tps.

| | 4 bots (hl-04) | 8 bots (hl-08) |
|---|---|---|
| plan | 569 / 57,752 | 693 / 50,665 |
| executed | 68,051 (1.18×) | 71,936 (1.42×) |

So doubling the roster shortens the plan by 12% and lengthens the run by
6%: the gap between plan and execution grows with bots on the ground.
Where it goes — walk stalls between bots in the same cell, the
`background_conflict` waits on a busy assembler site, lag waits sized for
a roster the planner assumed would not collide — is the next thing to
read off this record. The mode now runs eight bots without a single
failure, which is what today set out to establish.

### crowd — RCA: the extra 21,000 ticks are a research waiting for a pole it did not depend on

Worktree `crowd`, branch `eight-bot-execution-gap`, from `78bca60d`. Read
off hl-04 (`run-1788611922-87269`, 4 bots) and hl-08
(`run-1788617269-96746`, 8 bots) with `tools/run_analysis.py` and a
per-action planned-vs-settled join on `(bot, dispatch order)`; the plan
carries `planned_start`/`planned_duration` per action and per walk, so
every overrun below is `settled - dispatched - planned_duration`.

**Accounting.** 8 bots: executed 71,936 = 8,385 before the first dispatch
+ 12,886 slip on the critical path + the 50,665 planned. 4 bots: 68,051 =
6,438 + 3,861 + 57,752. The pre-dispatch gap is planning wall time under
5x (`14:07:49` "no buffers to read before planning" → `14:08:17` "planned
943 steps": 28 s, against 22 s for 4 bots) and is a 5x artefact worth
1,700 ticks at 1x. The slip is where the roster matters, ranked:

1. **Labs unpowered, ~8,500 ticks on the critical path (planner).** The
   two `research` actions overran by 23,226 ticks between them —
   `logistic-science-pack` planned 7,500, took 15,974; `automation`
   planned 3,000, took 17,752 — and every other verb is within noise
   (`craft` +2,209 over 104 actions, `mine` +493 over 152). All 85 packs
   were in the three labs by tick 54,076; the labs read `no_power` in
   every machine sample from 48,900 to 61,200, `research_progress` stayed
   `0.0`, and at 61,500 `generated_kw` went 0 → 900 and both researches
   ran at their modelled rate (logistic 7,300, automation 3,100). What
   went down at 61,329 was `place small-electric-pole at [38.5, -7.5]`
   (id 6): the one pole whose supply box meets the steam engine at
   `[40.5, -5.5]` (direction 4, footprint x 38–43; pole 19's box ends at
   38 and `boxes_overlap` refuses the edge-touch, as the game does). The
   plan put it at 49,411 and the research at 42,905, and the research's
   deps name only the lab-side pole (170), the labs and the inserts — not
   pole 6, not the engine. `Condition::Powered` is a state predicate no
   `Effect` satisfies, so `ActionNetwork::infer_edges` draws no edge to
   it; `schedule()` checks it against a sim that holds every placement
   already *chosen*, whatever tick it was given, so a pole chosen early
   on a busy bot satisfied it at 42,905. `Researched`'s method only
   states the edges when it builds the plant itself (`power_links`);
   here `supply_for` found the assembler cell's plant standing in the
   plan state and stated nothing. The 4-bot plan had the same hole and
   got away with it: its pole 6 was at 20,700 by ledger luck. Offline on
   `map.json` the 8-bot baseline plan shows the defect verbatim (research
   41,092, pole 6 49,316).
2. **Two researches planned concurrently, 3,000 ticks (planner).** The
   plan overlaps `automation` (43,215–46,215, two labs) under `logistic`
   (42,905–50,405, three labs); the game researches one technology at a
   time and ran logistic first (dispatched first), so automation — the
   one the assembler cells wait on — finished 3,000 after logistic. The
   4-bot plan overlaps them by 2,867 too and paid 1,316. Not fixed here:
   the fix is a force-wide research slot in the scheduler, sequencing
   the critical research first.
3. **Walks are the 5x round-trip tax, not crowding.** 250 walks overran
   by 17,599 summed, but the per-walk overrun is the same for both
   rosters — median 52.5 / 53, p90 150 / 138, max 327 / 279 — and no
   `blocked by character` stall reached a walk's settle. Two
   `bot_stepped_aside` events, both at the end. Off the critical path
   except as ~1,150 of accumulated late dispatch on the research.
4. **`background_conflict` and `predecessor` waits are downstream.**
   `place assembling-machine-1 at [36.5, -9.5]` waited 60 s on its own
   `craft 4 assembling-machine-1`, which waited on `research automation`;
   bots 7/8's pack inserts waited on labs the research held. Symptoms of
   1, not causes. Bots 5–8 were idle 72–79% of the run because the plan
   gave them 11–14k ticks of work, and that is the plan being short, not
   the execution being slow.

**Fix (this branch).** `PlanState::powering_entities(area)` names the
poles of every wired component whose supply box meets `area` and the
generators those poles cover; `Researched` states each as a
`Condition::EntityAt` precondition, so inference pairs the research with
whichever action places them and with nothing for what the world already
carries. Pinned by
`a_research_names_the_poles_and_generator_its_power_comes_through`, which
fails on the parent. Offline on `map.json`, `producing:logistic-science-pack:6`:

| bots | before | after |
|---|---|---|
| 1,2,3,4 | 569 / 57,752 | 569 / 59,476 |
| 1–8 | 706 / 50,874 (offline; the live plan was 693 / 50,665) | 706 / 53,326 |

Same action sets; only the schedule moves. The 8-bot plan is the honest
one now: pole 6 comes forward to 45,745 and both researches start at
45,775. **The 4-bot plan is 1,724 longer, and that is a scheduler
tie-break, not the model:** before, `place steam-engine` had no
successor, so bot 1's take→gears→engine block won the lookahead bound
against the cell's plate take by ten ticks; with the research hanging
off the engine its `remaining` grows, the block loses the tie, and bot 1
idles 5,839 ticks at 29,867 before doing the engine at 47,446. The
execution it replaces had power in time by luck, so the run may not pay
it; the plan does. Dropping the engine from the condition would restore
57,752 and leave a late engine with the same hole as a late pole, so it
stays.

**Validated (hl-f, `workspace/headless-f/runs/run-1788619567-57945`,
8 bots, 5x, seed 31337 fresh, `factory_stage3.lua`, load 5.5 at launch).**
One plan, 693 actions / 53,421 (live; the offline plan on the t=0 dump is
706 / 53,326), 0 failed, 0 lost, 250/250 walks, three pathfinder no-paths
recovered.

| | hl-08 (before) | hl-f (after) |
|---|---|---|
| plan | 693 / 50,665 | 693 / 53,421 |
| green cell (5x) | 71,936 (19:58) | **67,636 (18:48)**, −6.0% |
| executed / planned | 1.42× | 1.27× |
| research overrun (both) | +23,226 | +1,498 |
| labs `no_power` window | 48,900–61,200 | 49,200–58,200, with the research not yet dispatched |

The pole went down at rel. 49,303 and both researches dispatched at
49,304 — the edge did what it says. Eight bots now finish 415 ticks ahead
of four (68,051), which is still not the 12% the plan promises: the
residual is mechanism 2 exactly as predicted (`automation` ran first
this time, 1,999; `logistic` then took 9,999 against its 7,500 — the two
are planned side by side and the game runs them in series), plus pole 6
itself now being the critical path at 49,303 against a planned 45,840,
because its `craft 1 copper-cable` waited 1,055 ticks on a furnace for
the plate. The planning gap is unchanged at 8,042 ticks (28 s at 5x).
Per-walk overrun is again the 5x tax: median 53.5, p90 146.
### lagwait — RCA: the speed tax is planning time, not the lag wait

**Claim under test (hl-06):** the +4% at 5x / +9% at 10x on the same
automation plan "lives inside the long waits — the tick-polled lag wait
sleeps a wall-clock estimate between readings, and at speed each sleep is
worth more ticks." **Refuted by the records.** Worktree `lagwait`, branch
`lag-wait-at-speed`; analysis script kept as `scratch/waits.py` there.

**Where the ticks went.** `splits.json`'s `elapsed_ticks` runs from
`milestone_started` to `milestone_satisfied`. Split at the first dispatch:

| speed | run | milestone span | start → first dispatch | first dispatch → last settle | plan |
|---|---|---|---|---|---|
| 1x clients | run-1788582657-14978 | 22,271 | **334** (3116→3450) | 21,937 | 21,985 |
| 5x headless | run-1788614064-08543 | 22,724 | **942** (415→1357) | 21,782 | 21,765 |
| 10x headless | run-1788614294-64261 | 23,715 | **1,837** (415→2252) | 21,878 | 21,765 |
| 1x clients, green | run-1788612263-27812 | 62,408 | **1,983** (3610→5593) | 60,425 | 57,752 |
| 5x headless, green | run-1788611922-87269 | 68,051 | **6,438** (415→6853) | 61,613 | 57,752 |

The execution span is flat across speeds (21,937 / 21,782 / 21,878 — the
10x run executes the plan in *fewer* ticks than 1x). Everything that grows
is **before the first dispatch**: 942 ticks at 5x and 1,837 at 10x are the
same ~3.1 s of wall clock (`942/300 = 3.14 s`, `1837/600 = 3.06 s`), which
is the planner expanding and scheduling automation; green's 6,438 at 5x is
21.5 s, its planning time. The game does not wait for the planner, so a run
at `game.speed = s` is charged `60·s` ticks per second of thinking. At 1x
the same charge exists (334 / 1,983 ticks) and is small enough to have read
as noise. Of green's +5,643 ticks at 5x, 4,455 are planning; the remaining
~1,200 are inside execution (walk stalls between bots, `place` settling —
not this note's subject).

Note the `plan_created` tick in a headless record is **stale**: it is
`FactorioRcon::last_tick`, the stamp on the last RCON reply, and nothing
between `record.start` and the first dispatch refreshes it, so it reads
415 while the game is actually at 1,357 (5x) or 2,252 (10x). The client
run's 3,435 was fresh only because client polling kept the stamp moving.

**The lag wait itself is tick-exact at every speed.** A furnace `take n`
is linked to its `insert` with a lag of `192 · (n + 1)` ticks (one cycle
of headroom; `crates/planner/src/method/produce.rs`), and the executor
serves it in `wait_out_lag` (`crates/executor/src/run.rs`) by reading
`game.tick`, sleeping `owed / (60 · speed)` seconds, re-reading, and
stopping when `deadline - now == 0`. Measured as settle(insert) →
dispatch(take) on the same bot:

| speed | lag edge | modelled | waited | overshoot |
|---|---|---|---|---|
| 1x | `insert 5 copper-ore` → `take 5 copper-plate` (bot 2, id 40) | 1,152 | 1,155 | +3 |
| 5x | `insert 4 copper-ore at [22,-51]` → `take 4 copper-plate` (bot 3, id 152) | 960 | 962 | +2 |
| 10x | same edge, same bot | 960 | 965 | +5 |
| 5x / 10x | `take 4 iron-ore from the wooden-chest` → `take 4 iron-plate` (bot 1, id 67) | 960 | 957 / 957 | −3 (the insert was earlier than the chest take) |

The sleep is sized *at the speed* (`ticks_to_wall_clock(ticks, speed)`),
so a 10x sleep is ten times shorter in seconds and worth the same ticks;
the overshoot is one RCON round trip (2–5 ticks) regardless of speed.
`Actuator::game_speed` reads the real `game.speed` over RCON
(`rcon_actuator.rs`), and `scale_deadline` in `rcon.rs` only bounds the
reply wait. The mod polls character bots **every tick**
(`poll_character_bots` from `on_tick`, `control.lua`), so a craft or
mining completion is noticed on the tick it happens. The per-action
`reply` wait is the one thing that does scale — ~3 ticks per round trip
at 10x versus ~0.3 at 1x, visible as `place` growing 545 → 1,071 ticks
over 22 placements and mine/walk gaps +400/+540 — but it is offset by
slack elsewhere and the execution span does not move.

**Delivered tick rate**, from `batch_progress` (tick vs `elapsed_ms`):
1x 60/60, 5x 253 of 300 (84%, beside another run), 10x **529 of 600
(88%)**, green 5x 299/300. hl-06's "roughly 360 of 600" was a
wall-from-launch estimate including startup, not the executing rate.

**Fix (general, no speed in it):** `goal.plan` stops the game clock while
it thinks. `mods/BotBridge/control.lua` gains `set_tick_paused`
(`game.tick_paused`, answering with the tick), `FactorioRcon::set_tick_paused`
carries it, and `goal.plan` (`crates/scripting_lua/src/globals/goal/plan.rs`,
`PlanningClock`) pauses after the buffer refresh — which must run
unpaused: it re-probes benched bots with path requests the game answers on
a later tick — and resumes on every exit path, refusal included. RCON is
served while paused, so the placement pre-check still gets its answers. A
pause that fails is narrated and the plan proceeds as before. The receipt
is a new `planning_timed` event (`planning_ms`, `paused`, `tick_before`,
`tick_after`), and `just analyse` prints it beside a new
`delivered tick rate: N tps of M nominal (P%)` line that says `STARVED`
below 80%. Four `goal.plan` tests pin pause→resume ordering, the
error-path resume, the failed-pause fallback and refresh-before-pause.

This changes what a run's `elapsed_ticks` means — planning is no longer
charged, at 1x either — so a number from before this commit carries up to
~330 (automation) / ~2,000 (green) ticks of planning at 1x that a number
after it does not. `planning_timed` on the new records says exactly how
much.

**Validation — automation, 4 character bots, seed 31337, 10x, headless-g
(`run-1788619571-63940`, commit `abfcd2c3`, dirty tree: no).**

| | before (`run-1788614294-64261`) | after (`run-1788619571-63940`) |
|---|---|---|
| plan | 176 actions / 21,765 | 176 actions / 21,681 |
| milestone `elapsed_ticks` | **23,715** (1.09×) | **21,992** (1.014×) |
| start → first dispatch | 1,837 | 154 (429 → 583: server settle, roster, `RconActuator::new`) |
| first dispatch → last settle | 21,878 | 21,843 |
| `planning_timed` | — | 3,013 ms, `paused: true`, tick 480 → 480 |
| delivered tick rate | 529 of 600 (88%) | 528 of 600 (88%) |
| failed / lost | 0 / 0 | 0 / 0 |

−1,723 ticks, and the executed/planned ratio at 10x is now the 1x
client run's 1.01. The planner took its usual 3.0 s and the game did not
move: RCON served the placement pre-check while `game.tick_paused` held.

**And at 5x** (`run-1788619691-37853`, same box, same commit): milestone
**21,781** ticks against 22,724 before (−943; plan 21,681, ratio 1.005),
start → first dispatch 36 ticks, execution span 21,722, `planning_timed`
3,131 ms paused at tick 431, delivered 262 of 300 tps (87%), 0 failed.
Three speeds now read 1.01 / 1.005 / 1.014 against their plans.

**Two follow-ups from review.** (1) The pause is gated on the run
**owning** the server: `Planner::server` is `ServerOwnership::Owned` from
every constructor but the CLI's `--connect` and `--server <host>` branches,
which use `Planner::attached`. An attached run — possibly someone's live
multiplayer game, where `game.tick_paused` would freeze every human in it
— plans with `ClockPolicy::LeaveRunning`, reads the tick either side of
the plan instead, and its `planning_timed` carries `paused: false, reason:
"attached server, clock left running"` (new `reason` field, `null` when
paused). No explicit `--pause-while-planning` flag: ownership is the whole
decision and the CLI already knows it, so a flag would only let an attached
run opt into freezing someone else's game. (2) `plan_created.tick` was
stale on headless runs (`FactorioRcon::last_tick`, unrefreshed since run
start). `goal.plan` now returns the tick the clock answered on the way out
as `plan.tick`; `supervisor.lua` carries it onto the shaped table and
`record.plan_created` stamps the event with it, falling back to the live
tick only when the plan has none. Tests: attached clock is read twice and
never stopped; `plan.tick` is the clock's answer / `nil` with no clock;
`record.plan_created` takes `plan.tick`.

### triggers — merged (`1f296b47`): every trigger enumerated from the live game

32 trigger technologies (19 craft-item, 10 mine-entity, 1 build-entity,
1 capture-spawner, 1 create-space-platform), table in
`docs/superpowers/notes/2026-09-05-research-triggers.md`. Measured with the
sweep switched off: the game fires **mine-entity** for a character's own
chop and for a drill or pumpjack on the resource — `oil-processing` needs
no emulation at all, headless or not — and fires **craft-item** for machine
output ~400 ticks after the count; it does **not** fire craft-item for a
character's hand craft nor build-entity for a scripted build. The sweep now
gates on prerequisites (it used to ignore them), emulates build-entity from
a per-prototype tally of what our mod placed for our force, keeps the
hand-craft tally for craft-item, leaves mine-entity to the game (pinned by
a test), and can be switched off for measurements with a record line. The
`research_trigger_emulated` event reached `events.jsonl` for the first time
(the parser had been answering it with "unexpected action"). Refused by
name with the act: capture-spawner, create-space-platform. Eleven stub-game
tests. Live: `run-1788620523-34289`, 4 bots at 5x, automation done with
electronics fired by the game and steam-power / automation-science-pack
emulated from honest counts.

### hl-09 — green, 4 bots, 5x, master with the pole edge and the clock pause (`run-1788620990-78758`)

One plan of 571 / **64,100** (the live plan, with real inventories, is longer
than the offline 59,476 and than hl-04's 57,752 — the pole edge lengthened
the plan, not only the offline tie-break), 0 failed, 0 lost. Two
`planning_timed` receipts, 17,595 ms and 15 ms, both `paused: true`.
Green at **70,881 (19:41)** against hl-04's 68,051; executed/planned 1.106
against 1.18. So the ratio improved as designed and the absolute got worse
by 2,800 ticks, with my own workspace build and test suite running on the
box for most of it (load 17). Not a clean point; run 15 at 1x is.

### fourbot — RCA: run 15's 1,724 plan ticks and its 5,900 execution ticks (`four-bot-regression`)

Runs 14 (`run-1788612263-27812`, master `e30d98c8`) and 15
(`run-1788621697-14165`, master `917fdcd2`): four clients, 1x, seed 31337
`--new`, `producing:logistic-science-pack:6`, same map, same roster. Plans
diffed per bot on `(bot, planned_start)`; dispatch/settle joined per bot in
dispatch order (never on `id`); every tick below is relative to the first
`walk_dispatched` of the run (5,593 and 6,780).

**Where the 1,724 plan ticks went.** The two plans are identical up to bot
1's step 163 (29,857). There, run 14 has bot 1 do `take 16 iron-plate from
the furnace at [-6, -25]` → `craft 8 iron-gear-wheel` → `craft 1
steam-engine` → `place steam-engine` (30,851–31,431); run 15 has bot 1 idle
5,839 ticks, take 34 plates from the cell at 35,706, idle 5,810 more, and
do the same four steps at 47,446–48,026. `research logistic-science-pack`
moves 42,667 → 48,026 (bot 4 → bot 3) behind the engine it now names, and
the cell's last `set`/`charge` steps 57,655 → 59,426 behind the research.

It is not a tie-break, and the engine edge is not wrong. The scheduler
(`crates/planner/src/schedule.rs`) commits one `(action, bot)` per round,
ranked by `(bound, end, action, bot)` where `bound` is *that bot's* finish
over its ready work. `1ba679b3` hung the research off the engine, so
`remaining` grew for everything upstream of it — including bot 4's
`insert 4 iron-ore at [-6, -25]`, the fourth ore into the furnace whose 16
plates open the engine block. Traced round by round: that insert was ready
from tick 21,016 (`deps_ready`), acted at 27,522, and carried bound
50,713; bot 1's cell takes carried bound 45,146 (their tails are short) and
won round after round, so bot 1 was committed to 35,706 and then to
41,526 while the insert stayed uncommitted. It was committed with six
actions left; its successor, bot 1's `take 16 iron-plate` (ready 30,796),
then found `free_at[1] = 47,436`, and `free_at` never moves back. Before
`1ba679b3` the insert's bound was small enough to be committed early, and
the block landed at 30,851 by that accident.

**Fix (`40aaa211`).** Each bot's own next action is still the bound's;
*which bot's choice is committed this round* is no longer, because the
bound compares nothing across bots. A candidate is committed no earlier in
the round order than any other bot's choice that finishes before it starts
acting (`schedule.rs`, "The gap, measured"). Pinned by
`tests/scheduling.rs::a_bot_is_not_committed_past_a_gap_another_bots_finish_could_fill`
(3,120 on the parent, 2,110 fixed).

| offline, `map.json`, `producing:logistic-science-pack:6` | before | after |
|---|---|---|
| bots 1–4 | 569 / 59,476 | 569 / **52,554** |
| bots 1–8 | 706 / 53,326 | 706 / **48,756** |

Research after the pole and the engine in both: four bots pole 6 at 22,240,
engine 29,754, researches 33,821 and 40,856; eight bots engine 15,271, pole
6 at 20,049, researches 27,257 and 34,757. Two pinned fixture makespans in
`have.rs` moved down with it (7,156 → 6,982; 7,654 → 7,007).

**Where the execution ticks went.** Slip by verb, `elapsed − planned_duration`
summed, first plan only:

| verb | run 14 | run 15 |
|---|---|---|
| walk | +12,084 (191) | +11,415 (188) |
| research | **+1,083** (2) | **+7,003** (2) |
| craft | +2,204 | +2,883 |
| mine | +469 | +466 |
| everything else | ≤ 0 | ≤ 0 |

Per bot, `actual_end − planned_end`: run 14 {1: +2,463, 2: +2,459, 3:
+1,940, 4: −3,408}; run 15 {1: +7,795, 2: +8,627, 3: +1,920, 4: +7,809}.
`waiting_on` histograms are the same shape (`predecessor` 34 → 44 samples,
`background_conflict` 23 → 28, `reply` 59 → 63, `walk` 28 → 26). No
`bot_benched`, no `research_trigger_emulated` (clients, not characters:
the sweep never runs), and the force samples put technologies 1–3
unlocked at 6,1xx / 8,2xx / 15,4xx in *both* runs — the trigger gate is not
in this.

The research is the whole difference. `research logistic-science-pack` ran
11,249 in both runs. `research automation` was dispatched at 52,810 (run
14) / 53,892 (run 15) while the first research held the labs, and the game
runs one research per force: it settled 7,234 / **13,154** against 6,000 —
exactly the first research's end (54,344 / 61,346) plus its own 5,700. Run
14 queued 1,534; run 15 queued 7,454, because the pole-edge plan had moved
the first research 5,359 later and the second only 863. The force samples
agree: `research` becomes `automation` at 54,407 / 61,620, one sample after
the first finishes. That is 5,920 of the 7,090 between the runs' cell
ticks; the plan's 1,724 is the rest.

**Fix (same commit).** The scheduler keeps the force's research slot: a
`Research` starts no earlier than the previous one ends, whichever bot
dispatches it (`research_free_at` in `schedule`). Pinned by
`tests/scheduling.rs::two_researches_never_overlap_whoever_runs_them`
(500 on the parent, 800 fixed). Modelled in the scheduler rather than as
an edge because which research goes first is a choice, not a dependency.

**Weighed and not it.** The client walks: +11.4k to +12.1k in both runs,
188 vs 193 walks, the same per-bot overruns (±150). The bench/probe path:
no `bot_benched` event in either run. The trigger sweep: never runs with
clients, and the unlock ticks match. Crafts +679 worse in run 15 — real
but small, and not pursued.

**Validated live (hl-i, `workspace/headless-i/runs/run-1788625111-28152`,
four character bots, 5x, seed 31337 `--new`, `factory_stage3.lua`, load 0.7
at launch, own instance on ports 34218/4338).** One plan, 571 actions /
**53,249**, 0 failed, 0 lost, 232 walks, green at **56,370** — executed /
planned **1.059**.

| | hl-09 (`run-1788620990-78758`, master, load 17) | hl-i (this branch, load 0.7) |
|---|---|---|
| plan | 571 / 64,100 | 571 / **53,249** |
| green | 70,881 | **56,370** |
| executed / planned | 1.105 | **1.059** |
| `research automation` | dispatched 55,049, settled 58,048 (el 2,999) | dispatched 49,500, settled 55,923 (el 6,423 vs 6,000) |
| `research logistic-science-pack` | dispatched 55,680, settled 70,648, **el 14,968 vs 7,500** | dispatched 38,974, settled 50,223, el 11,249 = planned |

hl-09 is a different roster shape from runs 14/15 (character bots, 5x) and
ran under load 17, so the absolute times are not comparable; the ratio and
the research overrun are. The plan puts the researches at 35,226 and 46,626
with no overlap; execution slip still dispatched `automation` 723 ticks
before the first settled, so it queued 423 — against hl-09's 7,468 and run
15's 7,154. The residual is execution slip against a serialised plan, not
an overlap in the plan.

**What remains.** (1) A strictly chronological commit order (per-bot best
by bound, cross-bot by `end`) planned 51,303 four-bot against this rule's
52,554 — but it hands a shared research to the busy chain owner whenever
the ends tie (`an_action_that_is_nobodys_goes_to_the_bot_that_finishes_it_soonest`),
so the gap rule is the principled one; ~1,250 may be on the table there.
(2) The research slot serialises but the research *duration* still assumes
the lab count the method saw; a research planned on two labs runs on
however many stand when it is dispatched. (3) Walk overruns of ~11–12k per
four-bot run are the largest slip in both runs and untouched here. (4)
Executor side: a research dispatched while another runs settles when the
queue drains, and its verdict wait is `sized_deadline(expected_ticks)` —
three times the plan's duration plus a minute (`rcon.rs`). Run 15's 13,154
against 6,000 fit; a queue longer than 2x the second research's own
duration would be reported `NoVerdict` and replanned around. The slot in
the plan makes that unreachable unless the plan is wrong about the
research durations themselves.
