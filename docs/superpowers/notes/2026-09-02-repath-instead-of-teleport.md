# Re-path a stalled walk; never teleport it

**Date:** 2026-09-02
**Touches:** `mods/BotBridge/control.lua`, `crates/core/src/factorio/rcon.rs`,
`crates/core/tests/botbridge_walk_repath.rs` (replaces
`crates/core/tests/botbridge_walk_stuck_teleport.rs`)

## The cause, and the confirmation that it is the cause

`control.lua` steers a walking character along waypoints the game's pathfinder
chose **once**, at dispatch time. The run doing the walking is *building
things*. So the path goes stale, and it goes stale because of us.

The evidence was already in the archive. In `workspace/runs/run-1788344167-58471`
bot 1 kept trying to reach an ore tile at `(-23.5, 18.5)` from behind a
`stone-furnace` at `(-22.0, 18.0)` that the same run had placed 4,400 ticks
earlier — a 2x2 furnace spans x in `[-23, -21]`, squarely across the route.

Re-reading all 23 runs in `workspace/runs` made it stronger than "one furnace".
Grouping the 1,619 `teleport` events by `(run, bot, action_id)` gives **151
stuck-walk episodes**, and printing each episode's teleport destinations shows
the same shape every time: a bot being conveyed **tile by tile along its whole
route**, not hopped over one obstacle.

```
run-1788325660-10154 walk 234, 16 teleports:
  (-34.5,18.5) (-33.5,18.5) (-32.5,18.5) (-31.5,18.5) (-30.5,18.5) (-29.5,18.5)
  (-28.5,18.5) (-26.5,17.5) (-25.5,17.5) (-24.5,17.5) (-22.5,17.5) (-21.5,17.5)
  (-20.5,18.5) (-19.5,19.5) (-18.5,20.5) (-18.5,21.5)
run-1788315106-86443 walk 255, 9 teleports:
  (-38.5,-4.5) (-39.5,-5.5) (-40.5,-6.5) (-41.5,-6.5) … (-46.5,-6.5)
```

Consecutive gaps of 1.0 and 1.41 tiles: adjacent waypoints, in order. The
character was not walking *at all* along that corridor — every leg timed out at
its 60-tick floor and the teleport carried it one tile further. `y = 18.5` is
the same corridor `run-1788344167-58471` died in, i.e. the row of furnaces the
run itself had built.

So the RCA holds, and it is broader than the one furnace: **a stale path is the
normal case, and the blockage is usually a wall of our own making rather than a
single tile.**

### Run 28 (`run-1788358260-07659`), which landed mid-change

It reached 6 of 7 milestones and stuck on rung 7, `last error: ERROR: stuck
while walking, aborted before reaching last waypoint`, with every rung-7
iteration ending `walks(failed=1 lost=0)`. Its four rung-7 plans were 84, 55,
119 and 118 steps and completed 46, 19, 1 and 0 of them — 78 steps pending
behind one walk.

25 walk-stuck teleports fired across 10 walks in that run and rescued none of
them. The number that settles the argument is the destination histogram: **four
teleports went to `(-22.0, 19.0)` in four *different* walks.** That is the same
tile `run-1788344167-58471` spun on 1,414 times. Reporting arrival at a tile
nothing can reach is precisely what let the planner keep choosing it, run after
run.

### Why the teleport was worse than nothing

It converts an *unreachable destination* into a *reported arrival*. The planner
therefore never learns a site is unreachable and keeps choosing it, and the one
piece of information the system needed — "there is no way there" — is destroyed
at the exact moment the game was offering it. Every later fix
(`b3ed1beb` not landing on an occupied tile, `7da3d5e9` a cap and spending a leg,
`537adf30`/`c99e2ce2` bots parked in footprints) was compensation for that single
inversion.

Its measured value was about zero: across the last six runs it fired 0, 2, 4, 5,
15 and 1414 times, and in the most recent run it fired 4 times and the walk
failed anyway.

## What replaces it

On a stalled leg, `on_tick` asks the game for a fresh path **from where the
character actually stands** to **the walk's destination** (the last waypoint,
not the leg that stalled — the next waypoint is on the far side of whatever is
in the way). The request reuses the existing `request_path` shape: the
bounding box, collision mask and `entity_to_ignore` that make a path a
*character's* path were factored out of `rcon_async_request_player_path` into
`request_player_path`, so there is one request builder rather than two.

Nothing polls. The answer arrives on `on_script_path_request_finished`, which is
already pushed with a real `game.tick`; the walk holds still and steers nothing
while it waits, and `on_script_path_request_finished` now routes an answer to
`walk_repath_finished` first. Answers that belong to the mod are **not** written
out: `world.path_requests` (`sleep_for_path_request_result`) is keyed by handle
and nothing would ever remove an entry no Rust caller is waiting on.

Three answers, three outcomes:

- **a path** — it replaces the whole remaining route, `idx` restarts at 1, and
  the walk carries on. The walk's **destination is not negotiable**: the Rust
  side checked the *dispatched* path's last waypoint against what the caller
  asked for before any of this began (`move_player_timed`) and nothing
  re-checks it afterwards, so the original final waypoint is kept as the
  terminal one when the fresh path stops short of it. "The walk completed"
  therefore still means "the character stood inside the 0.3 arrival box of the
  place it was sent to". Substituting a nearby endpoint would be the teleport's
  lie wearing a different hat.
- **`try again later`** — the queue was full and nothing was searched, so the
  same question is asked again, and it does **not** spend the re-path budget.
- **no path** — the walk fails with
  `ERROR: stuck while walking, the destination is unreachable: the game's
  pathfinder found no path from (x/y) to (x/y)`. That is the fact the teleport
  destroyed; it reaches the supervisor through `obs.walks_failed` and
  `first_error`.

The final leg is no longer special. It used to abort with a reason that said
nothing about *why*, and it is the leg most likely to be blocked by something
the run itself built at the destination — exactly the failure the planner most
needs stated honestly.

## The bound: 4, and where it comes from

`WALK_REPATH_LIMIT = 4`.

A re-path is strictly stronger than the teleport it replaces. The teleport hopped
one leg, so a route blocked along its length cost one teleport per tile; a
re-path replaces every remaining leg at once. Measured against the archive: **all
151 stuck-walk episodes are a single blockage** — the destinations march
monotonically along one route, and not one episode shows two separate blockages.
One re-path covers every recorded episode.

**The bound is not what makes a bad walk fail fast.** The common case — the
destination is behind something we built — is answered on the *first* re-path,
because the pathfinder says there is no path and the walk fails there and then.
What the bound governs is only the pathological case where the pathfinder keeps
finding a route and the bot keeps not arriving. That matters given how much plan
sits behind one walk (78 pending steps in run 28): the honest failure arrives in
one cycle, not four.

4 is that observed need (1) with headroom for the case re-pathing has and
teleporting did not: the world changing again *while the new route is being
walked*. That can happen — `place` actions are dispatched in bursts, with a
25th-percentile gap between consecutive placements of 18 ticks across the same
runs — so a second and third re-path are real and a fourth is slack.

The cost side agrees. One cycle costs the leg timeout that detects the stall
(>= 60 ticks) plus one pathfinder round trip, so four bound a runaway to a few
hundred ticks: the same order as the 488 ticks the old cap of 8 bounded, and two
orders of magnitude below the executor's 21,600-tick (360 s) deadline. The point
is that the supervisor is told in seconds, instead of the run spending 87,766
ticks the way `run-1788344167-58471` did.

`WALK_REPATH_RADIUS = 0.5` is the smallest radius that does not collapse onto
the goal's own tile — the request shape that makes `request_path` fail outright,
and the same floor `approach_radius` clamps to on the Rust side. Factorio's
default of 1 would let the pathfinder legitimately stop a tile short.

`WALK_REPATH_PENDING_TIMEOUT_TICKS = 300` bounds the `try again later` retrying
in *ticks* rather than in attempts, because "worth repeating" is a statement
about the queue draining, not about how many times we asked. `since` is
deliberately not restamped by a retry. 300 ticks is the same bound
`MINE_BLOCKED_TIMEOUT_TICKS` puts on the other place this mod waits for the
world to become workable.

## The same distinction, on the Rust side

`try again later` and `failed to path find` were already documented as different
in `RconPathRequestFailed`'s help text — and `player_path` treated them
identically. **Any** error dropped into the offset-goal fallback, which retries
against a *synthesised* goal offset away from the real one. So a momentarily
busy pathfinder produced a path to somewhere the caller never asked for, which
`move_player_timed`'s arrival check then refuses as `RconWalkFallsShort`: a walk
that was fine, refused for a reason that was never about the walk.

Now:

- `path_request_was_busy` / `is_busy_path_request` recognise a full queue, and
  `player_path_attempt` / `path_attempt` put the **same** question again up to
  `PATH_REQUEST_BUSY_ATTEMPTS` (3) times with a 200 ms backoff.
- `path_search_found_nothing` gates the offset-goal fallback. That fallback
  answers exactly one question — "this goal cannot be reached, is anywhere near
  it?" — so a full queue, a request the game never took, a reply that never came
  and a reply that would not parse now all return as themselves. Substituting a
  goal on the strength of *not knowing* is the same reported-arrival shape the
  teleport had.
- The wording is pinned by a test that reads `control.lua` itself, the same way
  `mine_reports_target_gone` is, so a reword in the mod fails here loudly rather
  than quietly turning every busy pathfinder into an unreachable goal.

While gating the fallback, `world.players.get(&player_id).unwrap()` inside it
became reachable for errors that previously short-circuited, and it panicked a
test. It is now an early return: the fallback needs the player's position to
pick a direction, and inventing one would aim the substituted goal at random.

## What happened to the teleport, and to the `teleport` event kind

**The walk-stuck teleport is gone**, along with `WALK_STUCK_TELEPORT_LIMIT`,
`WALK_STUCK_TELEPORT_RADIUS` and `WALK_STUCK_TELEPORT_PRECISION`.

**`teleport_writeout`, the `teleport` `EventKind`, `record.teleports()` and the
`+N teleports` term in the supervisor line all stay**, and this is not a
"just in case" — they have live producers. `control.lua` had *three*
`player.teleport` sites, not one:

| site | what it does | status |
| --- | --- | --- |
| `walk_stuck` (`on_tick`) | moved a walking character onto its next waypoint | **deleted** |
| `revive_ghost_blocked` (`rcon_revive_ghost`) | moves a bot out of a ghost's bounding box so it can be revived | kept |
| `place_blueprint_blocked` (`rcon_place_blueprint`) | same, for a blueprint | kept |

The two kept sites are a different mechanism: they are synchronous RCON calls
that step a bot out of its own build site, they carry no `action_id`, and they
do not report an arrival to anybody. Nothing about them turns an unreachable
destination into a reported one. They were not in scope here and removing their
telemetry would be removing telemetry that still has an emitter.

Independently of that, the archive settles it: existing `events.jsonl` files
contain 1,619 `teleport` events (one run has 1,414), and reading those records
must not break. Deleting the kind would break the viewer's ability to read
history for no gain even if the emitters had all gone.

## Deliberately not done

- **No machine-readable `walk_repath` event.** Emitting a new writeout key needs
  an arm in `crates/core/src/process/output_parser.rs` or the parser logs
  `unexpected action` for every line; a new `EventKind` additionally touches
  `crates/scripting_lua` and `app/src/api/types.ts`, both outside this change's
  file boundaries (and `app/src` is being worked on by other agents right now).
  Re-paths are narrated with `print` instead — never `rcon.print`, whose output
  lands in the RCON reply body and would turn a successful action into a
  reported failure. If re-path frequency needs measuring the way teleport
  frequency was, that is a follow-up spanning those three files.
- **Three stale doc comments** now name a third teleport site that no longer
  exists, in files outside these boundaries:
  `crates/core/src/factorio/world.rs:23-25`,
  `crates/core/src/process/output_parser.rs:406-409`, and
  `crates/scripting_lua/src/globals/record.rs:969-971`. They are prose only —
  nothing branches on them — and they are flagged rather than edited.

## Tests

`crates/core/tests/botbridge_walk_repath.rs` replaces
`botbridge_walk_stuck_teleport.rs`. It loads the real `control.lua` into Lua 5.4
and drives `on_tick` and `on_script_path_request_finished` against a stub
surface that records path requests and can answer them all three ways.

20 tests. Run against the **pre-change** `control.lua`, 19 fail and 1 passes —
the one that passes is `an_rcon_path_answer_is_still_written_out`, which is a
regression guard for behaviour that must *not* change, so passing before and
after is the correct result for it.

Each part of the fix was then mutated and the failures checked. Every mutation
was caught, and most by exactly the test written for it:

| mutation | caught by |
| --- | --- |
| re-path from the origin, not the character | `the_re_path_starts_from_where_the_character_actually_stands` |
| aim at the stalled leg, not the destination | `the_re_path_aims_at_the_walks_destination…` + 4 more |
| radius 1 (pathfinder may stop short) | `the_re_path_asks_for_the_destination_itself` |
| never re-append the destination | `a_re_path_that_stops_short_still_ends_at_the_destination` |
| always re-append the destination | `a_re_path_that_arrives_does_not_repeat_the_destination` + 1 |
| treat a busy queue as unreachable | both `a_busy_pathfinder_*` tests |
| charge a busy queue to the budget | `a_busy_pathfinder_does_not_spend_the_re_path_budget` |
| wait on the pathfinder forever | `a_pathfinder_that_never_answers_ends_the_walk` |
| loosen the bound to 8 | `a_walk_that_keeps_re_pathing_gives_up_at_the_limit` |
| drop the outstanding-request guard | `a_walk_waiting_on_a_re_path_holds_still…` + 1 |
| adopt an answer whose walk already ended | `an_answer_for_a_walk_that_already_ended_is_dropped` |
| ignore a refused request | `a_request_the_game_refuses_fails_the_walk` |
| restore the last-leg abort | `a_stalled_final_leg_re_paths_too` + 1 |
| teleport as well as asking | `a_stalled_leg_asks_for_a_fresh_path_and_never_teleports` + 1 |
| write our own answers out to Rust | `a_re_path_answer_is_not_written_out_to_the_rust_side` + 1 |
| busy predicate always false | `a_busy_pathfinder_is_recognised_from_the_mods_own_wording` + 1 |
| fallback also runs for a full queue | `a_full_queue_and_a_missing_path_are_told_apart_as_errors` |

Two mutations initially killed **no** test, and both were real gaps in the tests
rather than in the fix:

1. *treat a busy queue as unreachable* was invisible to
   `a_busy_pathfinder_does_not_spend_the_re_path_budget`, because a failed walk
   sets its verdict on one tick and writes it on the next, and that test never
   ticked again — it could not tell "still alive" from "failed and not yet
   reported". It ticks now.
2. *adopt an answer whose walk already ended* was invisible because the fixture
   cleared `walking` entirely, which the mutated guard still caught. The fixture
   now settles the walk and dispatches a **different** one, which is what
   actually happens when the pending budget runs out and the supervisor
   replans — the answer is offered a live walk it does not belong to, which is
   the walk it would hijack.

## What this cannot prove

That Factorio's own `request_path` finds a way round an obstruction the previous
path went through. That is the load-bearing assumption and only a live run
settles it. What the tests do prove is that the mod asks, that it asks from the
right place with the right goal, and that every answer — including "no" —
reaches the caller intact.
