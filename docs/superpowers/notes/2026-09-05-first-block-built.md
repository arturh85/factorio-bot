# The first live build: MinerLine can't be sited; StarterSteamEngineBoiler stood 6/6; a synthetic fixture proves the direction migration

Task 4 of the blueprint-blocks plan: run `goal.built(blueprint, anchor)`
against a real, headless, four-bot Factorio server on seed 31337, and read
the world back to check every entity's position *and* direction.

## Correction (added after coordinator review): this was never a crash

Everything below the line was written describing `PlannerError::ChainOwnerInfeasible`
as a "crash" and an "internal scheduler invariant violation". That framing
was wrong and the coordinator caught it: `ChainOwnerInfeasible` is a
**documented, typed refusal** raised by `schedule()`
(`crates/planner/src/error.rs:92`), not a panic or a bug. A chain becomes
"owned" when a `Holder::Share` bill is sized against one bot — the planner's
own band-binding, not something a caller asks for — and the error fires
correctly the moment that bot's fixed-offset entity can't actually go where
the band was bound assuming it could. **The planner behaved correctly.**
What was actually wrong was the assumption underneath this task: that
"given an anchor, build the block" was a complete, sited operation.
Siting — checking that a fixed-offset design's footprint is buildable
*before* committing to it — was scoped out of this sub-project, and MinerLine
(a 21-tile belt run) makes the gap visible on real terrain, where one
obstructed tile anywhere along that run refuses the whole plan rather than
just that tile. That finding stands and is left below exactly as it was
written, with only this correction of what the error actually is. `goal.plan`
still propagates it as an uncaught Lua error to a script with no `pcall`
around the call (matching every other script in this repo, none of which
wrap it either), so a caller does still see a hard stop rather than a
graceful "can't site this here" — that presentation gap is real even though
the underlying error is not a bug.

## MinerLine: 37 entities, and it never got past planning

37 entities: 13 electric-mining-drill, 21 transport-belt, 3
small-electric-pole, from `scripts/rcontest.lua`.

**Result: it never got that far.** All four attempts below hit
`ChainOwnerInfeasible` inside `goal.plan`, before a single action was
dispatched. No entity was placed in any of them. This is reported as a
scoping gap (no siting story for `Goal::Built`), not a planner defect, per
the correction above.

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

**`goal.built` has no siting story, and `ChainOwnerInfeasible` is where that
gap becomes visible.** `plan_verified`'s re-siting loop (re-derive
`PlanState`, which now excludes the refused footprint, and re-expand) helps
goals whose placement sites the planner chooses freely (`free_area_near`
picks a different tile). `Goal::Built`'s sites are the blueprint's own fixed
offsets from the anchor — nothing about them can move on a re-expansion — so
re-expanding produces the same designed layout every time, `schedule()`
correctly notices the band it bound to a bot still can't be satisfied at that
fixed offset, and correctly refuses with `ChainOwnerInfeasible`. It
reproduced at three different anchors, with drills and with belts as the
triggering entity, with and without a materials bill (the cheated attempts
refused identically, on the same belt, ruling out "the bill's item counts"
as the mechanism), so this is not anchor-specific and not a gathering
artefact — it is what correctly happens every time a fixed-offset block
can't fit where it was asked to go.

**A structural reason to expect this on almost any anchor:** the belt run is
21 tiles long down the middle of the block. A single tile anywhere along a
21-tile line being a tree, a rock, or the edge of the ore patch's own
surrounding terrain is the common case, not a rare one, and with no siting
story, the first such tile — not the ability to walk to it, mine around it,
or route the belt one tile over — refuses the whole plan.

**No entity was placed in any attempt.** There is nothing to check for
position or direction — the deliverable this task exists to produce (37
entities read back from `map.jsonl`, position and direction verified) could
not be attempted, because nothing got past planning.

**Concrete finding for whoever picks this up:** `goal.built` needs either
(a) an anchor-siting story so a designed block's ore-dependent entities are
checked against real resources *before* `goal.plan` ever queries the live
game, or (b) `plan_verified` recognising that a fixed-offset block cannot be
re-sited and surfacing that as a named, callable-facing refusal earlier
(today it is still only visible as an uncaught Lua error, since `goal.plan`
propagates `ChainOwnerInfeasible` with no `pcall` anywhere in this repo's
scripts), or (c) both. Not attempted here, per the task brief — this was
explicitly out of scope for this sub-project, and the fix belongs to
whoever picks siting back up.

## Evidence (MinerLine)

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
  each attempt; verified with `ss -lntu` before starting the next one.

---

## Retarget: StarterSteamEngineBoiler -- 6 entities, all 6 stood, correctly

The coordinator retargeted the direction check to
`StarterSteamEngineBoiler` (also `scripts/rcontest.lua`): two steam-engine,
two small-electric-pole, one boiler, one pipe. Small footprint (fits inside
an 11-tile span), so it doesn't need the siting story MinerLine's 21-tile
belt run does. Same everything else: own ports (34200/4324), own workspace
(`workspace-blocks-task4`), headless, four bots, `--game-speed 5`, seed
31337, fresh map (`--new`).

**Cheats, disclosed:** every bot was given 2 steam-engine, 2
small-electric-pole, 1 boiler, 1 pipe via `rcon.cheat_item` before
planning, for the same reason as MinerLine's attempts 3-4 — isolating
placement from gathering. **No entity was cheated into the world**; the
build below is a real `goal.run` dispatch. The tick count reported excludes
gathering for that reason and is not an honest end-to-end build time.

New driver script: `scripts/starter_run.lua`. Anchor `(10, 10)` — the same
bare-dirt spot MinerLine's attempt 1 found refused for drills, which is
irrelevant here since none of these six entities need ore under them.

### It planned and ran clean, no refusal at all

`goal.plan` produced 10 steps (6 placements + 4 walks) across 4 bots in one
round, no refusal, no `ChainOwnerInfeasible`. `goal.run` reported
`done=true success=6 failed=0 lost=0`. Ran it **twice**
(`run-1788625294-24845`, `run-1788625403-87799`) for independent
confirmation; both identical.

### Reading it back: position and direction, two independent ways

**From inside the script**, `rcon.find_entities_in_radius(anchor, 15, name)`
— a direct query against the live surface, not a log — found all 6:

```
steam-engine        @ (9.50, 6.50)  dir=0
steam-engine        @ (9.50,11.50)  dir=0
small-electric-pole @ (11.50, 4.50) dir=0
small-electric-pole @ (11.50,11.50) dir=0
boiler              @ (9.50,15.00)  dir=0
pipe                @ (11.50,15.50) dir=0
```

**From outside the process**, the standalone `factorio-bot rcon -s
localhost --settings scratch/task4-settings.toml` CLI, fired in a loop from
a separate shell against the run's own server while it was still up (11
attempts before it shut down; one landed mid-build and returned a partial
read, the rest a `bb8` timeout as the server closed) — genuinely
independent of both the executor and this script:

```
EXTPROBE:small-electric-pole,11.5,4.5,0;steam-engine,9.5,6.5,0;steam-engine,9.5,11.5,0
```

Agrees exactly with the in-script reading, for the three entities that had
landed by that instant.

**Positions match, once the anchor convention is accounted for.** The
blueprint's own JSON has no `direction` key on any of the 6 entities at
all (decoded the same way as MinerLine: base64-decode, zlib-inflate, parse
JSON) — Factorio's export omits it when an entity is unrotated, so the
intended direction for all six is 0. Their relative positions are integers
(`steam-engine` at `(-1,-4)` and `(-1,1)`, poles at `(1,-6)` and `(1,1)`,
boiler at `(-1,4.5)`, pipe at `(1,5)`), and every one of the six landed at
exactly `anchor + relative_offset + (0.5, 0.5)` — a **uniform** half-tile
shift across every entity regardless of name or footprint, which is the
integer-corner-to-tile-centre convention Factorio placement uses, not an
error: `plan.steps`' own `pos` field (e.g. `(9.00, 6.00)` for the first
steam-engine) already reports the pre-shift, anchor-relative coordinate, and
the placed entity's centre is that coordinate's tile centre. Confirmed this
isn't a bug rather than assumed it: the shift is identical in both axes and
across all four entity kinds, which a real placement defect (a snap against
a specific obstacle, a footprint-parity accident) would not produce
uniformly.

One caveat on evidence quality: `plan.steps` does **not** expose a
`direction` field for `kind == "place"` steps at all (checked in
`crates/scripting_lua/src/globals/goal/plan.rs`'s `ActionKind::Place` arm —
it sets only `kind`, `entity`, `pos`). So "expected direction" here comes
from the blueprint's own decoded JSON, not from the plan the executor ran;
the plan and the executed placement could in principle disagree about
direction with nothing in the Lua-visible plan to check it against. Not
observed here (both blueprint and the two live reads agree at 0), but worth
noting as a gap in what a script can verify without decoding the blueprint
by hand.

**A third, independent source agrees: `map.jsonl`'s own `placed` records.**
The record system tracks exactly this intent-vs-actual gap on purpose
(`crates/core/src/record/map.rs`'s `drift_between`), and every one of the 6
lines in `run-1788625403-87799/map.jsonl` shows it explicitly:

```
{"kind":"placed","bot":1,"intent":{"name":"steam-engine","position":{"x":9.0,"y":6.0},"direction":0},
                          "actual":{"name":"steam-engine","position":{"x":9.5,"y":6.5},"direction":0},
                          "drift":["position"]}
```

`drift` names exactly `"position"` on all 6 lines and never `"direction"` or
`"name"` — the record system's own accounting agrees that direction landed
exactly as intended (0) on every entity, and that the only difference is the
known, uniform tile-centre snap, not a silent divergence.

**Verdict: 6/6 entities stood at their expected position (anchor + relative
offset, uniformly shifted to the tile centre) and expected direction (0, matching
the blueprint's own unrotated export), confirmed three ways — in-script RCON
query, an independent external `factorio-bot rcon -s localhost` process, and
`map.jsonl`'s own drift tracking — for both `goal.run`s.** No 8-to-16-point
migration defect surfaced — every direction in this blueprint is the trivial
0 case, so this run does not exercise a non-zero migrated direction; it
confirms the identity case only.

### Construction time

`tick_before`/`tick_after` from `rcon.game_tick()` around `goal.run`:
**224 ticks** (run 1) and **224 ticks** (run 2, `660 - 436`) of game time at
5x, for 6 placements + 4 walks across 4 bots — both runs agree. This
**excludes gathering** (materials were cheated in) and is not an
end-to-end build time. No `planning_timed`/`planning_ms` receipt is
available either way, for the same reason as MinerLine: this worktree
predates commit `17cdd5f7`.

### Evidence (StarterSteamEngineBoiler)

- `scratch/starter_run.log`, `scratch/starter_run2.log`,
  `scratch/external_rcon.log` (this worktree; local, not committed).
- Run directories: `run-1788625294-24845`, `run-1788625403-87799`
  (workspace `workspace-blocks-task4/runs/`), both `done=true success=6
  failed=0 lost=0`.
- `scripts/starter_run.lua` (this worktree, tracked).

---

## Closing the gap: a SYNTHETIC fixture proves the direction migration

Every direction in `StarterSteamEngineBoiler` is 0 — the coordinator's own
retarget noted "a smaller sample" as its cost and missed that the real cost
was zero rotations, so the migration this whole plan exists to catch was
never exercised. **This fixture is synthetic and proves the migration, not
that real exports decode** — the two real blueprints (`MinerLine`,
`StarterSteamEngineBoiler`) already cover real-export decoding in this
repo's unit tests (`crates/core/tests/blueprint_decode.rs`); this run's job
is the direction arithmetic alone.

### Building the fixture

Hand-built four `transport-belt` at four distinct, non-merging adjacent
tiles in a row, carrying raw pre-2.0 (eight-point) directions `0, 2, 4, 6`:

| entity | raw (8-pt) | expected migrated (16-pt) |
|---|---|---|
| belt 1 | 0 (north) | 0 |
| belt 2 | 2 (east)  | 4 |
| belt 3 | 4 (south) | 8 |
| belt 4 | 6 (west)  | 12 |

Encoded the same way `crates/core/tests/blueprint_decode.rs`'s
`encode_blueprint` test helper does — reused its approach rather than
inventing an encoder: envelope
`{"blueprint":{"item":"blueprint","version":281474976710656,"entities":[...]}}`,
zlib-compressed, base64-encoded, `"0"`-prefixed. `version = 281474976710656`
is 1.0.0.0, which is what makes `import_stack` migrate the directions on
decode rather than pass them through unchanged (a 2.x-versioned blueprint's
directions are already 16-point and must not be doubled — not exercised by
this fixture, deliberately). Built and verified by eye first (round-tripped
the JSON back out of the encoded string in Python) before spending a run on
it.

**Cheats, disclosed:** every bot was given 4 `transport-belt` via
`rcon.cheat_item` before planning — the same reason as every other cheat in
this note, isolating placement from gathering. No entity was cheated in.

### Rebuilt first, to rule out a stale binary

Per the coordinator's instruction (a stale binary caused a false alarm
earlier today): rebuilt clean before this run.

```
$ CARGO_TARGET_DIR=.../target nix develop -c cargo build --no-default-features --features cli,lua
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.27s
$ stat -c '%y %n' target/debug/factorio-bot
2026-09-05 18:50:03.762765216 +0200 target/debug/factorio-bot
$ git log --oneline -1
c52d6ff2 docs(plan): correct the MinerLine finding, and StarterSteamEngineBoiler stands 6/6
```

Tree was clean (`git status` showed no changes under `crates/`, `app/`,
`Cargo.{toml,lock}`) before the build, and `provenance.json` for the run
below independently confirms the same commit and a clean tree:
`"git":{"commit":"c52d6ff2ef0551436d938cb1c68a35742052fc50","dirty":false}`.

New driver script: `scripts/synth_run.lua`. Anchor `(10, 10)` — the same
bare-dirt spot `StarterSteamEngineBoiler` built six entities around
cleanly. (An anchor of `(0, 0)` was tried first, offline, before the
coordinator's hold-the-box message arrived mid-check — a `--clients 0`
dry-plan found the game would refuse a belt at `[0.5, 0.5]` there,
unrelated to this fixture; that process was killed immediately on the
hold instruction and nothing further was run until the all-clear.)

### It planned and ran clean; the machine was otherwise idle throughout

`goal.plan` produced 8 steps (4 placements + 4 walks) across 4 bots, no
refusal. `goal.run` reported `done=true success=4 failed=0 lost=0`
(`run-1788627039-02448`).

### Reading the four directions back, off the surface, two independent ways plus the record

**In-script**, `rcon.find_entities_in_radius(anchor, 10, "transport-belt")`,
sorted left-to-right by x:

```
belt[1] raw=0 @ (10.50,10.50) expected_16pt=0  game_reports=0  OK
belt[2] raw=2 @ (11.50,10.50) expected_16pt=4  game_reports=4  OK
belt[3] raw=4 @ (12.50,10.50) expected_16pt=8  game_reports=8  OK
belt[4] raw=6 @ (13.50,10.50) expected_16pt=12 game_reports=12 OK
```

**External**, the standalone `factorio-bot rcon -s localhost --settings
scratch/task4-settings.toml` CLI, fired in a loop from a separate shell
against the run's own server while it executed (8 attempts; the last one
landed after all four were placed, the rest before any belt existed):

```
EXTPROBE:transport-belt,10.5,10.5,0;transport-belt,11.5,10.5,4;transport-belt,12.5,10.5,8;transport-belt,13.5,10.5,12
```

Exact match.

**`map.jsonl`'s own `placed` records**, the same drift-tracking mechanism
used for `StarterSteamEngineBoiler`, and this time with **no drift at
all** (`transport-belt` is a 1×1 footprint, so there is no tile-centre
parity snap the way there was for the odd-shaped entities in the six-entity
build):

```
{"kind":"placed","bot":1,"intent":{"name":"transport-belt","position":{"x":10.5,"y":10.5},"direction":0}, "actual":{...,"direction":0}, "drift":null}
{"kind":"placed","bot":2,"intent":{"name":"transport-belt","position":{"x":11.5,"y":10.5},"direction":4}, "actual":{...,"direction":4}, "drift":null}
{"kind":"placed","bot":3,"intent":{"name":"transport-belt","position":{"x":12.5,"y":10.5},"direction":8}, "actual":{...,"direction":8}, "drift":null}
{"kind":"placed","bot":4,"intent":{"name":"transport-belt","position":{"x":13.5,"y":10.5},"direction":12},"actual":{...,"direction":12},"drift":null}
```

### Verdict

**All four observed directions matched all four expected, in order: `0
vs 0`, `4 vs 4`, `8 vs 8`, `12 vs 12`.** No belt arrived on an odd number
and none arrived on its raw (undoubled) value. Confirmed three ways —
in-script RCON, an independent external `factorio-bot rcon -s localhost`
process, and `map.jsonl`'s drift-tracked record — all in exact agreement.

**What this does and does not prove.** It proves the eight-to-sixteen point
direction migration end to end, live, on all four cardinal directions, on a
fixture built specifically to exercise every non-trivial case
`StarterSteamEngineBoiler` could not. **It does not prove that real
Factorio exports decode correctly** — that is a separate claim, already
covered by this repo's own unit tests
(`the_miner_line_decodes_to_its_37_entities`,
`pre_two_point_zero_directions_are_doubled_onto_the_sixteen_point_scale` in
`crates/core/tests/blueprint_decode.rs`) against the two real blueprints
used elsewhere in this note. Had any belt come back odd or undoubled, that
would have been the single most valuable finding this whole plan could
produce; it did not happen.

### Evidence (synthetic fixture)

- `scratch/synth_run.log`, `scratch/external_rcon_synth.log`,
  `scratch/synth_plan_check.log` (the killed `(0,0)` dry-check; this
  worktree, local, not committed).
- Run directory: `run-1788627039-02448` (workspace
  `workspace-blocks-task4/runs/`), `done=true success=4 failed=0 lost=0`,
  `provenance.json` git commit `c52d6ff2`, `dirty: false`.
- `scripts/synth_run.lua` (this worktree, tracked). `scripts/synth_plan_check.lua`
  was a throwaway offline sanity check and is not tracked.
