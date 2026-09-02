# Run 9's throughput regression: three deadlines, not a long walk

Run 7 `workspace/runs/run-1788307982-79011` — 107 actions, 99 successes, 603 s.
Run 9 `workspace/runs/run-1788310810-27811` — 9 actions, 6 successes, killed at
the 25-minute timeout.

## 1. Stall, not travel. The measurement that decides it

Run 9's `events.jsonl` has exactly three executed plans. `record.actions` is
called once per `"ran"` transition, so every event of a plan shares that plan's
finishing `wall_ms`, and the gap between consecutive `plan_created` lines *is*
the wall clock `goal.run(plan)` took:

| plan | wall span (s) | duration |
|------|---------------|----------|
| 1 | 0.006 → 369.276 | **369.27** |
| 2 | 369.372 → 735.640 | **366.27** |
| 3 | 735.661 → 1095.847 | **360.19** |

Sum: 1095.7 s. `ACTION_RESULT_DEADLINE` is 360 s
(`crates/core/src/factorio/rcon.rs:38`). Three consecutive plans pinned at it.
Nothing clusters *near* 360; all three are ≥ 360 and the smallest is 360.19.

Plan 1 nails it to the tick. Bot 2's mine (`id 1`) was dispatched at game tick
**4512** and has no `action_settled` line at all. `run_started` is tick 3959 at
`wall_ms` 0, so tick 4512 is 9.2 s in — and 9.2 + 360 = **369.2**, against a
recorded 369.276.

The anchor is independent of the record. `workspace/server/factorio-current.log`
starts at 02:58:57, logs `New RCON connection` at t=72.997, and run 9's id
`run-1788310810` decodes to 03:00:10 — the same instant. The server ran
continuously (autosaves at t=648.2 and t=1248.3, 600 s apart), and was SIGTERMed
at t=1495.6. The game was never paused or slow; the executor was waiting.

How much of the run was work:

|                              | run 7  | run 9 |
|------------------------------|--------|-------|
| dispatch→settle, summed      | 26,198 ticks (436.6 s) | 2,315 ticks (38.6 s) |
| wall clock                   | 603 s  | 1095.7 s |
| fraction inside an action    | ~72%   | **3.5%** |
| wall per success             | 6.1 s  | 182.6 s |

Run 7's wall clock (603 s) and its game clock (36,072 ticks = 601 s) are the
same number: it had no dead time at all. Run 9 is 1,080 s of deadline and 39 s
of game.

## 2. `too far too mine`: arrived on target, still out of reach

Not "never arrived" and not "drifted". The bot walked exactly where the mod told
it to, and the mod told it to stand out of reach.

`mods/BotBridge/control.lua`, mining handler: when `update_selected_entity`
picks up another **character** instead of the ore, it dispatched

```lua
rcon_action_start_walk_waypoints(4711, idx, {{ ent.position.x - 2, ent.position.y - 2 }})
```

That waypoint is `sqrt(8)` = **2.828** tiles from the entity. The guard three
lines above is

```lua
if distance(player.position, ent.position) > player.resource_reach_distance then
    action_failed(event.tick, ..., "ERROR: too far too mine")
```

and a character's `resource_reach_distance` is **2.7** (`crates/core/src/types.rs`,
`LIVE_2_1_PLAYERS`). 2.828 > 2.7. The step-aside waypoint is from `850f8c19`
(Aug 2021), when that guard was a flat `> 6`; the guard was tightened to the real
reach in `dd2852e3` (2026-08-31). The two have contradicted each other since, and
the contradiction is deterministic — every step-aside ends in `too far too mine`.

The observed latencies fit. Plan 2: dispatched tick 25838, failed tick 25853 —
15 ticks, which is about 2.8 tiles at 0.15 tiles/tick. Plan 3: failed 0 ticks
after dispatch, the bot still parked at the spot the previous attempt sent it to.

Three components disagree here only because two of them are the same component:
`crates/planner` picks the tile centre correctly (`EntityGraph::resource_patches`
does restore the half-tile offset — that trap is already handled), and
`crates/core`'s `within_resource_reach` uses the same 2.7 the mod does. The
disagreement is entirely inside `control.lua`, between its own corrective walk
and its own guard.

## 3. What actually burned the 1,080 seconds

Not the same thing. `too far too mine` is fast — it settles in 0–15 ticks. The
360 s came from branches of the same handler that gave **no verdict at all**:

```lua
if (ent2 == nil) then
    print("wtf, not mining any target")            -- no verdict, loops forever
elseif (... mismatch ...) then
    if ent2.type == "tree" then ... mining_state   -- fine
    elseif ent2.name == "character" then ... walk  -- also no verdict, loops
    else
        print("wtf, not mining the expected target")  -- no verdict, loops forever
    end
```

A branch that neither completes nor fails the action costs the caller its whole
`ACTION_RESULT_DEADLINE` and teaches it nothing. That is where bot 2's plan-1
mine went (dispatched tick 4512, never answered).

Plans 2 and 3 are the same fault one layer up. Bot 2 appears in both plans and
produced **no events whatsoever** — neither a dispatch nor a settle tick — which
`record.actions` only does for an action carrying `ActionTicks::UNKNOWN`, i.e. a
failure raised before the mine was dispatched. By elimination (every other bot's
action settled within ~120 ticks) the 360 s is bot 2's *corrective walk* hanging,
inside `player_mine_timed`'s `move_player`.

And that is invisible on purpose, by accident:

```rust
pub async fn move_player(...) -> Result<()> {
    self.move_player_timed(...).await.map(|_| ()).map_err(ActionFailure::into_report)
}
```

`into_report` keeps only the message. The `?` in `player_mine_timed` then rebuilt
the failure through `From<Report>` as `Dispatch::NotDispatched` with
`ActionTicks::UNKNOWN`. Both halves are wrong for a walk the game acknowledged
and went quiet on: `NotDispatched` claims nothing is outstanding while the bot is
still walking somewhere the plan does not know about, and an untimed failure is
written to the record as *nothing at all*. Two of run 9's three six-minute plans
contain no event explaining where the six minutes went, and this is why.

## 4. Did tile reservation make travel pathological?

**No, and the record cannot show what it did do.** Two separate answers.

The design read: `resource_tiles_for` does *not* ignore the bot's position —
`Mine::expand` passes `ctx.state.bot(ctx.chain_actor).position` as `from`
(`crates/planner/src/method/have.rs:640-645`), and candidates are distance-ordered
from there. The gap is elsewhere, and it is two things:

- **No reachability.** A claimed tile inside a lake, in a forest or behind cliffs
  ranks exactly like an open one. `11c2655a` exposed `blocking_boxes_within` to
  `is_area_clear`, but only for *placement*; the mine target selector never
  consults it.
- **No occupancy.** Nothing stops the planner handing bot A the tile bot B's
  character is standing on. Before reservation four bots shared one tile and
  raced (`gone before mining finished` — 6 of run 7's 8 failures). After it they
  are sent to *adjacent* tiles, which makes "another character is on my ore" the
  steady state rather than a transient. That is the exact input to the
  `ent2.name == "character"` branch above.

So reservation did not lengthen the walks. It converted a race the mod reports
cleanly into a collision the mod handles with a 360-second silence and a
guaranteed `too far too mine`.

The measurement: **not available from the record.** `record.actions` hardcodes
`target: None` (`crates/scripting_lua/src/globals/record.rs:682`) even though
`EventKind::ActionDispatched` has the field, so no run record has ever said which
tile a mine aimed at. Run 9 additionally has no `samples.jsonl` (`research_run.lua`
never samples) and a zero-byte `map.jsonl`. There is no way to compare chosen
tiles against bot positions for this run.

## 5. The walk fix is not in this

`c64a995b` landed 2026-09-02 00:36. Run 7 started 02:13. **Run 7 is already a
walking run** — its `samples.jsonl` shows bots covering 8.9 tiles per 60 ticks,
0.148 tiles/tick, no teleports — and it is the fastest run on record at 6.1 s per
successful action, with wall clock equal to game clock. The honest price of
walking was paid in full in the baseline and bought 107 actions.

**None of the run 7 → run 9 regression is the walk fix.** 1,080 of run 9's
1,095.7 recorded seconds are three expiries of a 360-second deadline, and 39 s is
everything else including all the walking.

## What was changed

`mods/BotBridge/control.lua`:

- Every branch of the mining handler now either sets `mining_state` or records a
  reason it could not. `MINE_BLOCKED_TIMEOUT_TICKS` (300 ticks / 5 s) bounds the
  blocked state and fails the action with that reason. It is not a mining
  timeout: the clock only advances on ticks that failed to set `mining_state`,
  and any tick that sets it clears the clock, so a slow mine is never
  interrupted. This turns 360 s of silence into 5 s and a sentence.
- `mine_step_aside_waypoint` sizes the step-aside to `resource_reach_distance`
  by the same rule Rust's `approach_radius` uses (half the bound, split over two
  axes → a straight-line distance of exactly half the reach, 1.35 tiles), instead
  of the fixed 2.828 that the guard refuses.
- The step-aside is dispatched once per blocked episode, not once per tick.
  Re-dispatching every tick restamped the walk's own leg timer so it could never
  time out, and each completion wrote another reply under the hardcoded action
  id 4711.

`crates/core/src/factorio/rcon.rs`:

- `player_mine_timed` calls `move_player_timed`, so a corrective walk's
  `ActionFailure` arrives with its `Dispatch` phase and ticks intact. A hung walk
  is now `NoVerdict` → `Lost` with a real dispatch tick, and appears in
  `events.jsonl` as a dispatch with no settle. Everything raised before
  `action_start_walk_waypoints` returns is still `NotDispatched` and still
  renders `Rejected`, so the blast radius is only the silent case.

## What is reported, not fixed

1. **Five more corrective walks launder their failures the same way.**
   `rcon.rs:941, 1023, 1551, 1737, 1810` (place / insert / remove / build paths)
   all call `move_player` and drop the phase and ticks. Same latent defect; no
   evidence any of them fired in run 9, and each site's surrounding error
   handling differs, so they were left alone.
2. **`record.actions` never writes `target`.** One hardcoded `None`
   (`crates/scripting_lua/src/globals/record.rs:682`) is why question 4 above has
   no measurement. Fixing it needs the target plumbed through `ExecutionLog` into
   the Lua observation.
3. **The mining and walking state machines are gated on
   `player.connected and player.character`.** A bot whose character is nil
   freezes both silently, and every outstanding action for it hangs the full
   deadline. This is the shape that best fits bot 2 failing three times running
   across two different action types, but the record cannot confirm it —
   the mod's `print` output was not kept for run 9.
4. **`world.actions` leaks the hardcoded action id 4711.** Nothing ever removes
   it; the counter is `% 1000` so it cannot collide, but the entry is immortal.
5. **The tile selector has no reachability or occupancy notion** (section 4).
   The terrain data `11c2655a` surfaced is available to it; wiring it in is a
   design change, not a bugfix, so it is not made here.

## Verification

`just test` clean (fmt, clippy `--deny warnings`, `cargo test --workspace`,
release build). `control.lua` syntax-checked with `luac -p`. The mod change
cannot be exercised by a test — this repo has no harness that runs `control.lua`
— and the Rust change cannot either: reaching the hang needs a deadline
parameter threaded through `move_player_timed`, which does not exist. Both are
unverified against a live game.
