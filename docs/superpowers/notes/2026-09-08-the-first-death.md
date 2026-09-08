# The first death this project has seen, and the 2,041 it did not explain

`run-1788833726-34821` (seed 31337, four headless character bots, 10x, debug,
commit `ed6db8e8`) charted to the crude oil at 372 tiles, then finished
`done=true success=246 failed=7 lost=1 pending=2041` out of 2,295 actions.

Three things were believed about it going in. **Two were wrong.**

---

## 1. It was not two deaths. It was three, and a tree.

Two deaths were narrated on stdout — a small-biter at tick 27,065, a
small-worm-turret at 53,619. **A third is in the record and was not read**: bot
4 was killed by a small-worm-turret at tick 34,873, and it is in the archive as
a *walk* failure rather than an action failure, so it never appeared beside the
other two.

And the largest single loss in that run has nothing to do with a biter:

```
bot 3  HALTED at tick    7980  walking to [197.4, -179.1]  (stalled)
       474 of this bot's 482 planned action(s) never settled
       ERROR: stuck while walking, leg 2 of 10 ... blocked at (205.599/-189.333)
       by tree 'tree-01' on tile 'grass-4'
```

**Bot 3 died to a tree, 19,000 ticks before the first bot was killed**, and 23%
of the plan died with it.

The four halts, complete:

| bot | tick | cause | planned actions lost |
|---|---:|---|---:|
| 3 | 7,980 | walk stalled on a `tree-01` | 474 |
| 4 | 34,873 | killed by a small-worm-turret | 391 |
| 1 | 53,619 | killed by a small-worm-turret | 740 |
| 2 | 53,619 | killed by a small-biter (at 27,065) | 436 |
| | | | **2,041** |

2,041 is `pending` **exactly**. That is the whole of the missing plan, and it is
derivable from the archived record — it just took reconstructing `plan_created`
and diffing it against the settles by hand. `just analyse` does it now.

## 2. The executor is not the culprit. Halting is correct; being silent was not.

`run_bot_signalled` stops a bot at a failed walk, deliberately, and its doc is
right about why: *a walk's effect is the bot's position, and no plan edge
carries a position*, so pressing on would dispatch every remaining step from
wherever the bot got stuck. A failed *action*, by contrast, does not stop the
bot — that was fixed earlier and stayed fixed.

What was wrong is that **the stop wrote nothing anywhere**. `halt()` published
`Failed`/`Lost` over the `watch` senders so dependents would abandon
themselves; the senders are not the record. An abandoned step is never
dispatched, so it produces no attempt, no `action_settled`, and the run's own
counters call it `pending` — **which is exactly what a run somebody killed
early also reports.**

`WalkSettled.abandoned` now carries the executor's own count, on the walk that
caused it. It rides there rather than in a `halts` map of its own so that every
existing caller of `record.walks` gets it with no new call to forget — which is
precisely how `record.deaths()` came to be uncalled for the one run in this
project's history that had deaths in it.

## 3. `get_player` reported every dead character bot as "not connected"

`character_missing_reason` exists to end exactly this confusion; its own doc
says so. The fix landed on the paths that call `no_character_error` directly
and **missed the entry point most of the mod's verbs go through**, which asked
`connected` first:

```lua
if player == nil or not player.connected then ... "not connected"
elseif not player.character then ... no_character_error(...)
```

For a **character bot** those two are the same fact:
`CHARACTER_PROXY_OWN.connected` is `entity ~= nil and entity.valid`, and a dead
bot's `entity` is nil. So in headless mode — the mode this project iterates in
— the `character` branch was **unreachable**, and the archived run ends with

```
Error: player 1 not connected
Error: player 2 not connected
```

which `classify_walk_failure` files as `WalkFailureKind::Other`. Those are the
walks that ended bots 1 and 2's slices, so **the last thing the record says
about either bot is a connect stall** — over a bot the game had named the
killer of, one statement earlier. `just analyse` counted 4 `no_character`
failures where there were 6.

This is `occupant_of` in another file, and the same lesson: **a cheap check
placed ahead of the specific one masks it, and the reader goes looking for
whatever the message named.** Reordering is safe because Factorio keeps a
*disconnected* player's character, so a client that merely left still falls
through to the `connected` branch — and there is a test for that direction too,
or the fix would be satisfied by deleting the check.

---

## The threat framing: the corridor measurement was right and was of the wrong line

Measured against `entity_graph.threats` in `map-31337-explored.json` (32
structures charted: 13 `biter-spawner`, 13 `small-worm-turret`, 6
`spitter-spawner`; no medium or big worms exist on this map yet).

**The oil corridor is clean.** Spawn to the crude-oil centroid at
(144.8, −351.8), 380 tiles: **zero enemy structures within 60 tiles**, minimum
distance **65.9**. The premise this session was handed — "zero within 50 tiles
of the spawn→well line" — holds.

**The deaths were nowhere near it.**

| site | what happened | nearest charted structure |
|---|---|---|
| (−259.75, 76.75) | bot 1 chopping a huge rock, killed by a small-worm-turret | **three small-worm-turrets at 20.4, 20.7 and 20.7 tiles** |
| walk to (158.1, 302.6) | bot 4 en route, killed by a small-worm-turret | 70.0 tiles from the destination; 36.1 perpendicular to the segment |
| ≈(−330, −279) | bot 2 standing still crafting, killed by a small-biter | **166.3 tiles** |

So the reading splits, and it is not one story:

- **Bot 1's death is a planning defect and it is exact.** A small worm's range
  is ~25 tiles. The plan sent a bot to chop a rock with **three worm turrets
  inside that radius**, and those three worms are *in the dump the planner
  read*. Nothing consulted them. `entity_graph.threats` is populated and has no
  reader in `crates/planner`.
- **Bot 2's death is roaming aggro**, 166 tiles from anything static. Avoidance
  routing buys nothing against it.
- **Bot 4's is unexplained** and should stay that way. The nearest charted
  structure to its path is 36 tiles, outside worm range, but the world is only
  partly charted and the pathfinder's route is not the straight segment.

**The corrected framing is that the threat question is about TARGET SELECTION,
not about the corridor.** The oil goal's resource-gathering steps scatter bots
to rocks and ore up to 380 tiles out in every direction — much further, and in
far more directions, than the oil line itself. One target sat inside a nest's
kill radius. Nobody looked at the corridor's neighbours because the corridor
was never the risk.

---

## What was verified live, and how the death was reproduced

Waiting for a biter is slow and non-deterministic, so the death was **cheated
in**, disclosed here and in the run's own output: `scripts/kill_a_bot.lua`
opens a tick-bounded window and a shell outside kills a character over RCON.
Four runs on `workspace/headless-b.toml` (never the default ports), four bots,
10x, seed 31337 `--new`, debug build. `plan_created.bots` was `[1,2,3,4]` on
every one. **Nothing timed in any of them is a measurement.**

`run-1788852831-94439`, the last:

```
6 bot_died + 6 bot_respawned, correctly paired (600 ticks without a character)
8 failures classified no_character
0 failed walks saying "not connected"
3 halts, abandoned = 11 / 47 / 22, on the record
```

Two things only a live run could have found:

**A `bot_died` had never been produced by a real game.** The mod's own doc above
`on_player_died` said so. The whole chain — mod writeout, `output_parser`,
`FactorioSurface`'s queue, `record.deaths()` — was cold, and it works.

**Every death was stamped with the wrong tick.** `record.deaths()` ran each
drained event through `not_before`, which is `max` against the record's
high-water mark, and it is called *after* `record.actions` and `record.walks`.
`run-1788852300-80960` wrote **nineteen death and respawn events all stamped
12,033**, over deaths at 3,988 and 12,033, so every pairing read `respawned at
tick 12033 (0 ticks without a character)`. The gap is the entire reason the two
events are paired. Fixed; a death carries a measured tick and never needed the
clamp, and `events.jsonl` was never a sorted file anyway.

### Four things that cost a run each

- **`rcon.send()` discards the reply.** `factorio-bot rcon -- '<cmd>'` prints
  the command it sent and *nothing else* — `app/src-tauri/src/cli/rcon.rs`
  calls `rcon.send()`, which returns `()`. CLAUDE.md's "attaches to an already
  running instance and **prints the reply**" is wrong, and two probes were read
  as "the command did nothing" when the command was fine and the channel was
  mute.
- **A console `/c` cannot see a mod's `storage`.** The first kill addressed
  `storage.bots[2].entity` from console scope, where `storage` is nil, so the
  guard failed silently. Go through the surface:
  `game.surfaces[1].find_entities_filtered{name='character'}`.
- **1,800 game ticks is 3 seconds of wall clock at 10x.** The window closed
  before a second `factorio-bot` process had finished starting.
- **The roster is the wrong instrument for "did the kill land".** A character
  bot respawns after `CHARACTER_RESPAWN_TICKS = 600`, one second of wall at
  10x, and `rcon_players` filters on `connected and character` — so a poll
  every 3,000 ticks saw `roster=4` at every mark and the script printed **"NO
  KILL WAS SEEN"** over a run that had written a `bot_died`. The verdict has to
  come from `record.deaths()`, which drains a queue and cannot miss a death
  that already happened.

### And the falsification harness scored three tests green that catch nothing

`--exact <name>` does not match a unit test inside `mod tests` — it is
`run::tests::<name>` — so cargo exited 0 over "running 0 tests" for three of six
mutations. The harness now asserts a test actually ran. A fourth mutation then
came back green honestly: the new death-tick test used `record.milestone_stuck`
as its "later" event, which stamps the last tick the RCON connection saw, and
the sandbox has an empty one — so the prior event landed at tick 0,
`not_before(3988)` answered 3988, and **the test passed against the exact code
it was written to catch.** It pushes the mark with a `record.walks` reply tick
of 20,000 now, and asserts that it did.

---

## What was deliberately NOT changed

**Replanning onto the survivors, and retrying a stalled walk.** Both are policy
— they change what a run *means* — and one run does not choose between them.
The evidence now available for that decision:

- A halt is **not** mostly a death. In the archived run one of four was a
  pathfinder stall on a tree, and it was the second largest.
- A stalled walk is plausibly retryable (the mod reported it "stepped clear"
  after giving up); a `no_character` walk is not, but it *heals in 600 ticks*,
  which is under a second at 10x and under the executor's own deadlines. The
  executor has no concept of either, and telling them apart is now possible:
  `walk_settled.failure.kind` says which.
- The two want opposite treatments and share one code path, which is why
  guessing at a policy here would have been a workaround rather than a root
  cause.

**Threat-aware target selection.** `entity_graph.threats` is populated, has no
reader in `crates/planner`, and one death sits 20 tiles from three worms that
are already in the dump. That is a real and well-evidenced gap, and it is an
owner decision whether the answer is a refusal, a cost, or "oil is not a t=0
target on this seed".
