# The stuck-recovery manufactures the stuck

Run `run-1788341905-92036`, 2026-09-02. Rungs 1–5 satisfied, rung 6 stuck with
**33 steps planned and zero dispatched**, three iterations running, each giving
up in under a second.

## What the record said, and what it could not say

`events.jsonl`:

```
milestone_stuck  index 6  outcome stuck_silent  last_error null  tick 19270
```

`stuck_silent` with **no error** — while an error existed and was being logged
once per iteration:

```
failed to find player_path() for #1 to -22.5/21.5: the game's pathfinder returned no path
```

That is the second defect here, and it is about the instrument rather than the
game: the pathfinder failure never reached the record. This diagnosis needed the
console log, which no archived run keeps. A record that reports "stuck, and there
was no error" when the error was one layer down is not merely incomplete — it
actively points the reader away.

## What actually happened

`samples.jsonl`, final ticks:

```
tick 19920..20220   bot 1 (-22.5, 17.5)   bot 3 (-22.5, 17.5)
```

Two characters at **identical coordinates**, motionless to the end of the run.
The path target was `(-22.5, 21.5)` — four tiles due south. The game was right to
refuse: a character cannot leave a tile it is co-occupying.

The `teleport` events show how they got there:

```
15780  bot 3  walk_stuck  ->(-21.5,17.5) ->(-22.5,17.5)
19007  bot 2  walk_stuck  ->(-21.5,18.5) ->(-20.5,17.5)
19007  bot 1  walk_stuck  ->(-21.5,18.5) ->(-20.5,17.5)
19007  bot 1  walk_stuck  ->(-21.5,17.5) ->(-22.5,17.5)
```

Bot 1 was teleported onto the tile bot 3 had been sitting on since tick 15780.
And in the *same tick* 19007, bots 1 and 2 were each sent to `(-21.5, 18.5)` and
then each to `(-20.5, 17.5)` — two bots, one destination, no awareness of each
other. The first shared position appears at tick 16080.

So the recovery for "this bot is stuck walking" **creates a permanent stuck**:
it stacks characters, the pathfinder then correctly reports no path, and every
retry deepens it. The failure escalates instead of degrading.

## The site

`mods/BotBridge/control.lua`, in the walk state machine:

```lua
teleport_writeout(event.tick, idx, "walk_stuck", pos, w.waypoints[w.idx], w.action_id)
player.teleport(w.waypoints[w.idx])
```

Three things wrong in two lines:

1. **The destination is never checked for occupancy.** `LuaControl.teleport` does
   not respect collisions — it stacks characters happily. `LuaSurface::find_non_colliding_position("character", target, radius, precision)`
   is the API for exactly this.
2. **`teleport` returns a boolean and it is discarded.** Same class as the
   `remove_item` / `create_entity` returns fixed this morning: a call that reports
   what it actually did, asked as though it always succeeds.
3. **The record logs the intent, not the outcome** — `teleport_writeout` fires
   *before* the teleport, with the position we meant to reach. Here the two
   coincided, so the record was accurate by luck.

## A separate finding from the same plan

The stuck plan puts **30 of its 33 steps on bot 1**, one each on bots 2, 3 and 4.
It concentrates every walk onto one character and so concentrates the exposure to
this teleport bug; the two defects compound.

**Corrected later the same day.** I attributed this to the reported gap where
`Researched` sizes its whole bill against a single bot. That is a true
description of the code and a **false** account of the cause — see
[2026-09-02-research-bill-spread.md]. The enclosing share's chain binds the
subtree before `Researched` states anything, and hoisting the goal clear of the
share was measured to move the distribution by **zero** actions.

## What this cost

Nothing was lost — no failed actions, no destroyed material. The run simply
stopped, politely, with a record saying there was no error.

## A third record defect, found while diagnosing the first two

Teleport events and action events cannot be joined.

```
action_dispatched  {"tick":3769,"id":0,"bot":1,"action":"mine 5 iron-ore", ...}
teleport           {"tick":19007,"bot":1,"reason":"walk_stuck","action_id":93, ...}
```

`action_dispatched`/`action_settled` carry **`id`**, which is the plan-local node
id and restarts at 0 for every plan. `teleport` carries **`action_id`**, from a
different, run-global space (60, 87, 92, 93, 108, 110 in this run). Two
near-identically named fields, two id spaces, no mapping between them.

So the question this run raised — *which action was a bot walking for when the
recovery teleported it onto another bot* — cannot be answered from the record at
all. The teleport data alone still proves the collision, because `from`/`to` are
absolute coordinates and two bots show the same `to` in one tick. But that is
luck: the join would have been the direct evidence.

## What is working, confirmed in the same file

The morning's placement work shows up as **structured** failure data, not just a
message:

```json
{"status":"failed","failure":{"kind":"partial_transfer","detail":"moved 4 of 5 copper-plate"}}
```

Three of those in this run. Before today each would have silently destroyed
copper plate.
