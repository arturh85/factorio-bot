# The first live build: `goal.built` crashes before it ever dispatches

Task 4 of the blueprint-blocks plan: run `goal.built(MinerLine, anchor)` (37
entities: 13 electric-mining-drill, 21 transport-belt, 3 small-electric-pole,
from `scripts/rcontest.lua`) against a real, headless, four-bot Factorio
server on seed 31337, and read the world back to check every entity's
position *and* direction.

**Result: it never got that far.** All four attempts below crashed inside
`goal.plan`, before a single action was dispatched. No entity was placed in
any of them. This is a real defect, reproduced four times, at three
different anchors, with and without gathering, and is reported rather than
fixed, per the task brief.

**Cheats used, disclosed up front:** attempts 3 and 4 gave every bot the
full material set (`rcon.cheat_item(bot, "electric-mining-drill", 13)`,
`"transport-belt"` x21, `"small-electric-pole"` x3) before planning, on
explicit permission from the coordinator, to isolate placement from
gathering — gathering is already answered offline (491 actions, researches
and crafts everything). **No entity was ever cheated into the world**;
every placement in every attempt was a real `goal.run` dispatch (none
completed). Any construction-time number below would exclude gathering for
that reason and must not be read as an honest end-to-end build time — moot
here anyway, since nothing built.

## Setup

- Branch `blueprint-blocks` (worktree `.worktrees/blocks`), binary already
  built at `target/debug/factorio-bot` (commit `abe5bc1c`).
- Own settings file, `scratch/task4-settings.toml`: `factorio_port = 34200`,
  `rcon_port = 4324`, `workspace_path =
  ~/.local/share/factorio-bot-dev/workspace-blocks-task4` (fresh, not shared
  with the other session's `workspace-headless`).
- `--headless --bots 4 --game-speed 5 --seed 31337 --new`.
- Driver script `scripts/block_run.lua` (new; copied into the workspace's own
  `scripts/` before each run, since a debug build's workspace copy has no
  repo fallback): plans `goal.built(MinerLine, anchor)`, prints the plan,
  calls `goal.run(plan)`, then `record.actions`/`record.walks`/
  `record.finish` the way `factory_stage2.lua` does per batch, collapsed to
  one milestone since this goal has no ladder.
- This worktree branched off master **before** commit `17cdd5f7`
  (`lag-wait-at-speed`), so it has no `planning_timed` event and no
  `planning_ms` receipt — noted, not fixed. Any tick count below may include
  planning time and is not comparable to a number measured on top of that
  commit.

## Attempt 1: anchor `(10, 10)` — block_smoke.lua's own anchor

`goal.plan` ran its live pre-flight check (`plan_verified` in
`crates/scripting_lua/src/globals/goal/plan.rs`) and the game refused **13 of
77 placements** — every electric-mining-drill in the blueprint, each with
`no entity in the footprint; tile dirt-N`. The anchor sits on bare ground on
this fresh map, not on ore: `MinerLine`'s drill offsets were captured
somewhere else, and nothing checks offline that an anchor's ore matches a
blueprint's assumption before dispatch. (The 491-action/45,303-tick estimate
in the task brief comes from the standalone `plan` CLI against a `map.json`
dump, which has no live game to ask and so never discovers this — it assumes
every placement succeeds.)

The pre-check's own re-siting loop kicked in ("re-siting rather than
dispatching (round 1 of 2)") and round 1 raised:

```
Error:   × goal: bot 1 owns chain ChainId(0) because its bill was sized against it,
  │ but electric-mining-drill fits at [11.5, 18.5] facing 4 does not hold
  │ there
```

This is `crates/planner/src/error.rs:92`'s message template, raised from
inside `schedule()`. `goal.plan` propagated it as an uncaught Lua error;
`block_run.lua` has no `pcall` around `goal.plan` (matching `smelt_run.lua`'s
and `block_smoke.lua`'s own shape — neither wraps it either), so the whole
script died, the server shut down cleanly (ports 34200/4324 free
afterwards), and `record.finish` never ran: the run directory
(`run-1788623925-55344`) has `events.jsonl` and `provenance.json` but no
`manifest.json`, and `map.jsonl` holds only the opening keyframe (ore
resources the game already knew about) — no entity was ever placed.

## Attempt 2: anchor `(-27, -34)`, chosen to put every drill on real ore

To separate "the anchor was badly chosen" from "the re-siting path is
broken", I found a real anchor: queried the live server directly
(`rcon.find_entities_in_radius({x=0,y=0}, 150, "iron-ore")`, a one-shot
`scan_ore.lua`), got the seed's 940-tile iron patch (bbox `x[-43.5,-6.5]
y[-45.5,-10.5]`), took the 13 drill offsets from attempt 1's own refusal log
(`(1.5,1.5) .. (5.5,19.5)`, two columns 4 tiles apart), and brute-forced
which of ~1,900 candidate anchors made every one of the 13 offsets land on
an actual ore tile: 243 did. Picked `(-27.0, -34.0)`, close to the patch
centroid.

This time **zero drills were refused** — confirming the fix was in the right
place. But the *second* round of the same pre-check refused a
transport-belt instead (`transport-belt fits at [-23.5, -14.5] facing 8 does
not hold there`, presumably an obstacle — tree, rock, or another
entity — the blueprint's belt route didn't anticipate), and the identical
crash reappeared:

```
Error:   × goal: bot 3 owns chain ChainId(76) because its bill was sized against it,
  │ but transport-belt fits at [-23.5, -14.5] facing 8 does not hold there
```

Same shape, same file, same missing `manifest.json`, same empty `map.jsonl`
(`run-1788624199-06625`).

## Decoding the blueprint to test the real hypothesis

Two hypotheses were still open: "the re-siting path is broken" (any refusal
crashes it) versus "this belt route specifically crosses something on every
anchor tried so far". Decoded `MinerLine` directly (it is a standard
Factorio blueprint string: drop the version byte, base64-decode, zlib-inflate,
parse JSON) to get all 37 entities' exact relative positions and directions
rather than inferring them from refusal logs. Two parallel drill columns
(`x=1.5`/`x=5.5`) and one long belt run down the middle (`x=3.5`, `y=0.5` to
`y=20.5`, 21 tiles) with three poles at `x=2.5`.

## Attempt 3: cheated materials, same anchor `(-27, -34)`

With every bot pre-equipped (no gathering needed), `goal.plan` still crashed,
this time even before the "would refuse" round-0 summary line ever printed —
the failure is not specific to the re-siting *loop* (round ≥ 1); it can fire
on the very first `schedule()` call:

```
Error:   × goal: bot 3 owns chain ChainId(2) because its bill was sized against it,
  │ but transport-belt fits at [-23.5, -13.5] facing 8 does not hold there
```

`[-23.5, -13.5]` is one tile from attempt 2's `[-23.5, -14.5]` — both are the
last two belts of the long run (`x=3.5, y=19.5` and `y=20.5` relative,
entities 36/37 in the decode above). **This confirms the crash is not about
missing materials**: it reproduces identically, at the identical spot, with
materials cheated in.

## Attempt 4: cheated materials, new anchor `(-33, -40)`

Tried to find a fully clean anchor before giving up: wrote `scan_site.lua`,
a one-shot script that checks all 37 decoded offsets against the live game —
`iron-ore` present under every drill, no non-resource entity within 1.1
tiles of every belt/pole. It reported this anchor clean. It was not: the
same belt (`y=20.5` relative, the very last entity in the blueprint)
refused again —

```
Error:   × goal: bot 3 owns chain ChainId(2) because its bill was sized against it,
  │ but transport-belt fits at [-29.5, -19.5] facing 8 does not hold there
```

`find_entities_in_radius` cannot see what actually blocked it: Factorio
water and cliffs are terrain, not entities, and cannot be queried through
that RPC at all (`find_tiles_filtered` would be needed for water). The
`scan_site.lua` heuristic is therefore unreliable and this is reported as a
limitation, not chased further — four crashes at three anchors, with and
without materials, was judged enough evidence for the underlying claim
without spending more of the cleared run budget hunting for a lucky anchor.

## What this establishes

**`goal.built`'s live pre-check does not tolerate a single placement
refusal.** `plan_verified`'s documented design (re-derive `PlanState`, which
now excludes the refused footprint, and re-expand) is written for goals
whose placement sites the planner chooses freely (`free_area_near` picks a
different tile). `Goal::Built`'s sites are the blueprint's own fixed
offsets from the anchor — nothing about them can move on a re-expansion —
so the second round produces the same designed layout, yet something in how
`crates/planner/src/method/blueprint.rs`'s bands (`bands()`, entities sorted
by x and chunked evenly per bot, one "chain" per bot's band) are scheduled
against the refusal-updated world disagrees with the `ChainId` ownership the
first round already committed to, and `schedule()` raises rather than
producing a plan. It reproduced at three different anchors, with drills and
with belts as the triggering entity, with and without a materials bill (the
cheated attempts still crashed, on the same belt, ruling out "the bill's
item counts" as the specific mechanism), so this is not anchor-specific and
not a gathering artefact.

**A structural reason to expect this on almost any anchor:** the belt run is
21 tiles long down the middle of the block. A single tile anywhere along a
21-tile line being a tree, a rock, or the edge of the ore patch's own
surrounding terrain is the common case, not a rare one, and this defect
means the first such tile — not the ability to walk to it, mine around it,
or route the belt one tile over — ends the whole plan.

**No entity was placed in any attempt.** There is nothing to check for
position or direction — the deliverable this task exists to produce (37
entities read back from `map.jsonl`, position and direction verified) could
not be attempted, because nothing got past planning.

**Concrete finding for whoever picks this up:** `goal.built` needs either
(a) an anchor-siting story so a designed block's ore-dependent entities are
checked against real resources *before* `goal.plan` ever queries the live
game, or (b) a `plan_verified` re-siting path that understands a fixed-offset
block cannot be re-sited and should surface the refusal as an ordinary
`ConnectRefusal`-style refusal instead of an internal scheduler invariant
violation, or (c) both. Not attempted here, per the task brief.

## Evidence

- `scratch/run.log` .. `scratch/run5.log`, `scratch/scan.log`,
  `scratch/scan_site*.log` (this worktree; local, not committed).
- Run directories (workspace `~/.local/share/factorio-bot-dev/workspace-blocks-task4/runs/`):
  `run-1788623925-55344` (attempt 1), `run-1788624199-06625` (attempt 2),
  `run-1788624585-75753` (attempt 3), `run-1788624922-42456` (attempt 4) —
  each has `events.jsonl` (just `run_started`/`vision_measured`),
  `provenance.json` (seed 31337, map digest `c161fa3f437221d0`, git
  `abe5bc1c`, `bot_mode: characters`, `game_speed: 5.0`), and `map.jsonl`
  (opening keyframe only, no `placed` lines — nothing was ever built).
- `scripts/block_run.lua`, `scripts/scan_ore.lua`, `scripts/scan_site.lua`
  (this worktree, tracked).
- Every server process exited cleanly and released ports 34200/4324 after
  each crash; verified with `ss -lntu` before starting the next attempt.
