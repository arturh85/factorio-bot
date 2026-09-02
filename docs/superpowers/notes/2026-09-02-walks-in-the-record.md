# Walks reach the record — 2026-09-02

**Closes:** the hole named at the end of
`docs/superpowers/notes/2026-09-02-rung-7-unreachable.md` — *"Three walk
failures, zero of them in `events.jsonl`."*

Walking is most of a run's wall clock and none of it was in the run record.
`EventKind` had `RunStarted, MilestoneStarted, MilestoneSatisfied,
MilestoneStuck, PlanCreated, ActionDispatched, ActionSettled, Frame, Teleport,
PlacementRefused, RunFinished` and no walk. Run 30 failed three walks; the run
directory held one `last_error` string and the other two existed only in
`workspace/server-log.txt`, which the next run overwrites. The 12-of-75
through-a-furnace paths and the 18-of-18 stalls at up to 1/256 of a tile
clearance that the rung-7 note measured were all reconstructed from that file
by hand, and could not be asked of any run again.

`obs.walks` already carried everything needed. It was simply never written.

## The shape

Two events, deliberately the same pair the action side uses.

```jsonc
{"tick": 81381, "kind": "walk_dispatched",
 "bot": 2, "step_index": 4,
 "to": {"x": -23.5, "y": 18.5},
 "planned_start": 0, "planned_duration": 240}

{"tick": 81661, "kind": "walk_settled",
 "bot": 2, "step_index": 4,
 "to": {"x": -23.5, "y": 18.5},
 "status": "failed", "elapsed_ticks": 280,
 "error": "game rejected the command: Unexpected Response: ERROR: stuck while walking, the destination is unreachable: the game's pathfinder found no path from (6.90234375/30.09765625) to (-22.30078125/18.22265625)",
 "failure": {"kind": "no_path",
             "from": {"x": 6.90234375, "y": 30.09765625},
             "destination": {"x": -22.30078125, "y": 18.22265625}}}
```

Written by `record.walks(observation.walks)`
(`crates/scripting_lua/src/globals/record.rs`), flushed once per supervisor
"ran" transition alongside `record.actions`, `record.teleports` and
`record.refusals`.

### Why two events rather than one

The dispatch needs a tick the *game* stamped, and the settle needs only a
verdict. Folding them into one line carrying `elapsed_ticks` would make a
walk's start unknown exactly when `elapsed_ticks` is null — which is the case
worth drawing. Two events also let `derive_lanes` pair them with the code that
already pairs the action pair.

### Why `planned_start` / `planned_duration` are on the dispatch

They are in the record nowhere else. `plan_created`'s `plan` carries only
steps that have an `ActionId`, and `plan_for_record` skips a step with no `id`
— so a walk's prediction has never been archived at all, and a measured walk
had nothing to be compared against. These are **plan-relative** ticks, not
`game.tick`; the event's own `tick` is absolute. Same two clocks
`PlannedStep::planned_start` keeps apart, converted in the viewer in exactly
one place (`observedOrigin()`).

## Every walk settles exactly once

The settle is gated on the **verdict** — `success | failed | lost` — never on a
measured reply tick. That is the shape `fcb4ed68` had to fix on the action
side, where the settle lived inside `if let Some(replied) = replied` and a
`Status::Lost` attempt is *defined* by no reply ever arriving, so `lost` was
structurally unrecordable. This was built with that already known, and two
tests hold it there:

- `a_lost_walk_settles_even_though_the_game_never_replied`
- `a_walk_the_game_never_stamped_a_tick_for_still_settles`

Re-introducing the old gate (`&& replied.is_some()`) fails exactly those two
and nothing else — measured, see *Mutations* below.

`pending` and `running` write no settle: not a verdict, nothing to report,
and inventing one is the failure mode a verdict gate has to avoid now that it
no longer waits for a tick.

## `lost` stays distinct from `failed`

They are different facts with different fixes. `failed` is the game refusing
the walk; `lost` is the game acknowledging it and never answering — a bot that
may still be walking as far as anything here knows. Three places keep them
apart, and each is pinned:

1. `walk_settled.status` carries the executor's `Status` verbatim.
2. `failure.kind` is `timeout` for a lost walk, which says only that no verdict
   arrived — it makes no claim about the walk itself.
3. `derive_lanes` closes a lost walk's lane **as `lost`**, not as `failed` and
   not by leaving it open (which would say a third thing again: the run was
   interrupted here).

## Recording what happened, not what was intended

**No arrival position is recorded, on purpose.**
`on_player_changed_position` fires once per *tile crossed*, so a bot that comes
to rest partway into a tile reports its entry and nothing corrects it — every
parked bot's observed position is up to a tile stale, which produced a wrong
diagnosis earlier today. There is no honest arrival position available, so the
record states none rather than a stale one.

`to` is therefore labelled for what it is: the destination the **schedule**
asked for. It is routinely a position the bot cannot stand on — it comes from
the `Condition::AtPosition` the walk exists to satisfy, so it is often the tile
a furnace occupies.

**The two observed positions that do exist are on the failure.** BotBridge
writes `player.character.position` and the goal it was steering at into the
`no path` message, at the instant it gives up, and `walk_endpoints` parses them
out:

- `failure.from` — where the character actually stood. Read from the game, not
  inferred from a tile boundary.
- `failure.destination` — **the last waypoint of the path the game returned**,
  which is *not* `to`. In run 30 all three failures had this land strictly
  inside the collision box of a furnace the same run had placed, up to 1.2
  tiles from the `to` that was requested. Recording only `to` would have hidden
  precisely the fact the rung-7 note had to reconstruct by hand.

Parsed at full precision and refused rather than rounded: `-22.30078125` and
`-22.3` are inside different collision boxes.

## The distinction the mod makes, kept

`WalkFailureKind` is a separate enum from `FailureKind` — every distinction in
it is about the pathfinder, and none of `FailureKind`'s substantive variants
means anything for a walk.

| kind | the mod's wording | what it means |
| --- | --- | --- |
| `no_path` | `the destination is unreachable: … found no path from … to …` | it **searched** and found nothing. A fact about the destination; repeating the walk gets the same answer. |
| `pathfinder_busy` | `the game refused a re-path request`, `the pathfinder did not answer a re-path within N ticks` | it **never searched** — `try again later` on a full request queue, or an accepted request that never answered. Nothing was learned; repeating is the right move. |
| `repath_limit` | `gave up after N re-paths on one walk` | a path existed every time it was asked for and the bot still never arrived. About the walking, not the map. |
| `stalled` | `aborted before reaching last waypoint` | a leg timed out with no re-path answer to blame. |
| `timeout` | `the game reported no readable outcome: …` | no verdict at all. Pairs with `status: "lost"`. |
| `other` | anything else | the wording moved; `error` still carries it whole. |

`timeout` is tested **first**, before any wording that would claim knowledge
the run does not have.

**Known gap, and it is not mine to close:** another agent is landing a
pre-flight refusal in `crates/core/src/factorio/rcon.rs` for a path whose
terminal waypoint is somewhere a character cannot stand
(`docs/superpowers/notes/2026-09-02-unstandable-destination.md`). Its wording
is not in this classifier, so such a refusal will land in `other` with the full
text in `error` until an arm is added for it. Guessing at the wording now would
have been worse than an honest `other`.

## Can a walk be joined to the action it serves? No.

**Not by id, and not by anything in this record.** Stated plainly because the
alternative is a table that suggests a join it cannot perform.

- A walk has no `ActionId` at all. `StepKind::Walk { to, radius }`
  (`crates/planner/src/schedule.rs`) names no action, so there is nothing to
  join *from*.
- `step_index` is the walk's index in **that bot's own slice** of the schedule
  (`run_bot_signalled` filters to one bot, then enumerates). It is not an
  `ActionId`, and it is not an index into `plan_created.plan` either — that
  array is over every bot's steps and omits walks entirely. Joining it to
  either produces confident nonsense.
- The `walk_stuck` teleport's `action_id` remains unjoinable for the reason
  already documented on `EventKind::Teleport`: it is the RCON/mod id minted
  from a run-global counter that wraps at 1000, while `ActionDispatched::id` is
  the planner's per-plan id. This work does not change that.

What the record *does* now support, and it is worth having:

- **Epoch and bot.** A walk belongs to the `plan_created` epoch it falls in and
  names its bot, so "which milestone's walking cost this" is answerable.
- **A checkable correspondence.** A walk's `to` is the `Condition::AtPosition`
  of the action it exists to satisfy, and `action_dispatched.target` is that
  same position for every `mine`/`place`/`insert`/`remove`. So a reader can
  match a walk to the next action on the same bot in the same epoch whose
  `target` equals its `to` — and *check* the match rather than assume it. That
  is a heuristic with a test, not a join.

Making it a real join needs one of: an `ActionId` on `StepKind::Walk`
(`crates/planner`, owned elsewhere), or a step index carried on both
`PlannedStep` and `ActionDispatched` — and note those two index spaces differ
today (`plan_for_record` indexes all steps of all bots; a walk's index is
within one bot's slice), so it would have to be the bot-slice index on both.

## What the viewer shows

1. **A walking bot is no longer drawn idle.** `derive_lanes`
   (`crates/core/src/record/lanes.rs`) now emits a lane per walk, so the run
   viewer's per-bot track covers the largest stretch of every run instead of
   leaving a gap, and its "now" column reads `walk to (-23.5, 18.5)` where it
   used to read `—`.
   - `Lane.id` became `Option<u32>`: **null for a walk**, because a walk has no
     action id and borrowing its `step_index` would put a number from another
     id space under a name that means action id. Measured: doing exactly that
     makes an action's settle close a walk's lane (mutation M6 below).
   - `Lane.action` for a walk is composed by the server (`walk to (x, y)`),
     the one string in that type the plan did not write — the schedule gives a
     walk no label.
   - A walk settle with no dispatch beside it still draws, as a zero-length
     lane at its own tick. An action's is dropped, because `events.jsonl`
     carries it anyway; a walk's has no other line anywhere, and dropping it
     would put the failure back where it was before this work — nowhere.
2. **A Walks table on the run analysis page**, beside the overrun table it has
   never been able to cover: milestone, bot, step, requested destination,
   planned vs actual vs delta, status, failure kind, and — its own column —
   *where the walk was really steering*. `walksOf` (`app/src/lib/runDiff.ts`)
   scopes the join per plan epoch, exactly as `joinRunOutcome` does, because
   `step_index` restarts with every plan just as action ids do.

## Also fixed here: `plan_created.bots` was still wrong

Reported by the coordinator mid-task and verified before changing: `b356f8ee`
moved this field from "the bots appearing in the steps" to the roster
`create_lua_record` closes over — the bots the *process* was started with. That
is not the roster either whenever a script plans with `goal.plan{bots = ...}`,
which the shipped driver does, from `rcon.players()`. Run 30 recorded
`bots: [1, 2]` for a plan made for `[2]` alone: the record asserted a bot had
been offered work it was never offered.

Mechanism confirmed in `crates/scripting_lua/src/globals/goal/plan.rs`:
`opts.bots` is the roster the expansion runs against, and the plan reports it
back as `plan.bots`. The only thing that knows it is whatever called
`goal.plan`.

- `bots` is now `Option<Vec<u32>>` and comes from `record.plan_created`'s third
  argument, which the supervisor fills from `plan.bots`.
- Absent → **present-and-null**, the shape `run_started.seed` already uses. It
  does *not* fall back to the ambient roster: a plausible-looking substitute is
  worse than a null, because a reader cannot tell it from a stated fact.
- A `bots` that is not a list of positive bot ids is refused outright, the way
  `record.milestone_satisfied` refuses a reason it does not recognise. Writing
  `[]` would put "this plan was made for no bots" into the one part of the
  archive that stays trustworthy when outcomes do not.

This matters more than a normal field: `plan_created` is written at planning
time and carries the whole DAG, so it is sound even in runs where outcomes are
not (`docs/superpowers/specs/2026-09-02-material-convergence-design.md`
§2.3.1). A lie inside the reliable record is worse than one inside a suspect
one.

## Red-first, and the mutations

Every claim above went red before it went green, with the failure output, not a
summary of it.

| red | what it printed |
| --- | --- |
| `a_walk_reaches_the_event_log` | `attempt to call a nil value (field 'walks')` |
| four lane tests | `assertion left == right failed: left: 0, right: 1` (and `index out of bounds: the len is 0`) |
| `the_batchs_walks_are_flushed_to_the_record_on_every_ran_transition` | `left: 0, right: 1` — with the driver's call removed |
| four `plan_created` tests | `left: Some([1, 2, 3, 4]), right: Some([2])` |
| `walksOf` (7 tests) | `TypeError: walksOf is not a function` |

Then each mutation, and exactly which tests it took down:

| # | mutation | failed |
| --- | --- | --- |
| M1 | gate the walk settle on `replied.is_some()` (the old bug) | `a_lost_walk_settles_…`, `a_walk_the_game_never_stamped_a_tick_for_still_settles` — and nothing else |
| M2 | write `lost` as `failed` | `a_lost_walk_settles_…` only |
| M3 | drop the `pathfinder_busy` arm | `a_queue_that_would_not_search_is_not_a_search_that_found_nothing` only (`left: Other, right: PathfinderBusy`) |
| M4 | discard the observed endpoints | `a_no_path_failure_carries_the_two_positions_the_game_named` only |
| M5 | fall back to the ambient roster | `plan_created_with_no_roster_records_null_rather_than_the_ambient_one` only |
| M6 | put `step_index` in `Lane.id` | three lane tests, including `a_walk_lane_and_an_action_lane_do_not_close_each_other` — the id-space collision, demonstrated |
| M7 | drop a settle-only walk lane | `a_walk_that_was_never_dispatched_still_draws_at_its_settle` only |
| M8 | key a walk on `step_index` without the bot | `never closes one bot walk with another bot settle` |
| M9 | rename a variant in the TS contract table | `EventKind publishes exactly the variants the client union lists` |
| M10 | change one doc line in `record/mod.rs` | `schema WalkFailureKind differs from the committed snapshot` |

**M8 is the one worth reading.** The first version of that test passed under
the mutation: it happened to order its events so that a bot-less key still
closed the right row. The test was rewritten to settle the walk opened *first*,
which a bot-less key gets wrong, and only then did it fail. A test that cannot
be made to fail is pinning nothing.

## Files I own and changed

- `crates/core/src/record/mod.rs` — `WalkDispatched`, `WalkSettled`,
  `WalkFailure`, `WalkFailureKind`; `PlanCreated::bots` -> `Option<Vec<u32>>`.
- `crates/core/src/record/lanes.rs` — walk lanes; `Lane::id` -> `Option<u32>`.
- `crates/scripting_lua/src/globals/record.rs` — `record.walks`,
  `classify_walk_failure`, `walk_endpoints`, `roster_from_lua`.
- `app/src/api/openapi.snapshot.json`, `types.ts`, `openapi.contract.spec.ts` —
  both halves of the seam.
- `app/src/lib/runDiff.ts` (+ spec), `app/src/pages/RunAnalysisPage.vue`.

## Files outside my list that this needed, and why

Reported rather than assumed. `record.walks` and the roster argument are both
useless without a caller, and the only production caller lives in `scripts/`.

- `scripts/supervisor.lua` — carries `walks = obs.walks` and `bots = plan.bots`
  on the transitions it already returns. Two data fields; no logic.
- `scripts/research_run.lua` — calls `record.walks(t.walks)` beside the three
  flushes already there, and passes `t.bots` to `record.plan_created`.
- `crates/scripting_lua/src/research_run_lib.rs` — the stub harness needed a
  `record.walks` and a `walks` array on `goal.run`'s observation, plus the test
  that pins the driver actually flushing them.

All three were being edited by another agent at the same time, which is why
each change here is a data field or a stub and never logic. Their work landed
first (`7de7c6a0`, `51f4c6ca`), taking `supervisor.lua`'s
`walks = obs.walks` with it; what remains in this commit for these three files
is only mine.
