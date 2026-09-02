# The stuck-walk recovery manufactured the stuck, and the record could not say so

Two defects from `workspace/runs/run-1788341905-92036`, which halted at rung 6.
One caused the halt; the other is why the record could not explain it.

## 1. Verifying the account

Every claim below was checked against the run's own files rather than taken on
trust.

**Confirmed.**

- `events.jsonl`: `{"tick":19270,"kind":"milestone_stuck","index":6,
  "outcome":"stuck_silent","best_steps":43,"last_error":null}`.
- `events.jsonl` teleports: bot 3 → `(-22.5, 17.5)` at recorded tick 15780;
  bot 1 → `(-21.5, 17.5)` then → `(-22.5, 17.5)` at recorded tick 19007.
- `samples.jsonl`, every sample from tick 19920 to the last at 20220: bot 1 at
  `(-22.5, 17.5)` **and** bot 3 at `(-22.5, 17.5)`. Identical coordinates,
  frozen, while bots 2 and 4 sit elsewhere. Two characters on one tile.
- The only event kinds in the whole file are `action_dispatched` (113),
  `action_settled` (113), `teleport` (15), `plan_created` (11),
  `milestone_started` (6), `milestone_satisfied` (5), `partial_transfer` (3),
  `run_started`, `run_finished`, `rejected`, `placement_refused`,
  `milestone_stuck`. **There is no event kind for a path-request failure at
  all.** The pathfinder's refusal reached the console and nothing else.

**One correction.** *"In that SAME tick 19007, bot 2 and bot 1 were each
teleported to (-21.5, 18.5), and then each to (-20.5, 17.5) — two bots sent to
one destination in one tick."* The identical destinations are real. **The
same-tick-ness is not established by this record.** `record.teleports()`
stamps each drained teleport with `RunRecorder::not_before(tick)`
(`crates/core/src/record/mod.rs`), which raises a tick to the highest already
recorded. Teleports are drained in a batch at each `"ran"` transition, so a
batch whose real ticks straddle an already-recorded event is *flattened onto
one value*. Four teleports sharing tick 19007, in an interleaving
(bot2, bot1, bot2, bot1) that a single pass over `game.players` cannot produce,
is that flattening rather than a same-tick collision.

This does not weaken the diagnosis — the stack that actually froze the run is
bot 3 at 15780 and bot 1 at 19007, which are different ticks and are confirmed
by `samples.jsonl`. It does mean the record cannot currently distinguish "two
bots teleported to one tile in one tick" from "two bots teleported to one tile
at different times", and the same-tick case is now prevented by construction
rather than by evidence.

## 2. The recovery created the condition it exists to fix

`mods/BotBridge/control.lua`, the walk follower's stuck branch:

    teleport_writeout(event.tick, idx, "walk_stuck", pos, w.waypoints[w.idx], w.action_id)
    player.teleport(w.waypoints[w.idx])

Three defects in two lines, all confirmed against
`workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17):

1. **No occupancy check.** `LuaControl.teleport` returns "`true` if the entity
   was successfully teleported" and says nothing about refusing an occupied
   destination. Characters collide, so a second bot sent to a tile a first one
   is standing on is *stacked* on it — after which the game's pathfinder
   correctly answers "no path" for every later request from either of them, and
   every retry makes it worse.
2. **The boolean was discarded.** Same class as the `remove_item` /
   `create_entity` placement fix: a recovery that moved nothing was
   indistinguishable from one that worked.
3. **The record logged the intent.** `teleport_writeout` was called with the
   waypoint, *before* the teleport. An adjusted or refused landing would leave
   the record asserting a position the bot is not at. In this run the two
   coincided, so the record was accurate by luck.

### What changed

`LuaSurface::find_non_colliding_position("character", target, radius,
precision)` — documented as returning "the non-colliding position. May be `nil`
if no suitable position was found" — is asked first, with the *character*
prototype so the collision box is the right one.

- **Nothing free** → no teleport. The walk aborts with
  `no free position within 4 tiles of the next waypoint`. Failing costs this
  walk; stacking costs both bots for the rest of the run.
- **`teleport` returns false** → the walk aborts with
  `the game refused to teleport the character`.
- **Otherwise** → `teleport_writeout` is called *after* the teleport, with
  `player.character.position`, so the record names where the bot landed. The
  leg's timeout is re-sized from the landing to the waypoint, because an
  adjusted landing still has the remaining offset to walk.

`w.stuck` now carries the reason as a string instead of a bare `true` with the
message hardcoded at the abort site — that message said "aborted before
reaching last waypoint", which is not what either new arm did. A `true` from an
older save (walk state lives in `storage`, which outlives a save/load) still
gets the old wording.

`WALK_STUCK_TELEPORT_RADIUS = 4` and `WALK_STUCK_TELEPORT_PRECISION = 0.5`. The
radius is deliberately small: the teleport exists to put the bot *on its
route*, and one free to move it several tiles sideways is inventing a different
walk rather than recovering this one. The precision is half a tile because a
character's collision box is about 0.4 tiles across.

### The same-tick case, and whether it is genuinely handled

**Yes, by ordering, and it is pinned by a test.** `on_tick` iterates
`game.players` one at a time and `teleport` takes effect immediately, so by the
time the second bot's `find_non_colliding_position` runs, the first bot is
already standing at its landing and is an obstacle like any other. No
bookkeeping of "destinations chosen this tick" is needed, and none was added —
a second list would be a second thing to get wrong.

`two_bots_stuck_in_one_tick_do_not_land_on_each_other`
(`crates/core/tests/botbridge_walk_stuck_teleport.rs`) drives one `on_tick`
with two bots stuck on the same leg. Against the old mod it reproduces the run
exactly — both at `(-22.5, 17.5)` — and against the new one they land apart.

## 3. The record said "stuck, no error" while the error existed

**Not in `crates/executor/`.** The executor records the walk failure correctly:
`run.rs` calls `log.fail_walk(bot, i, f.to_string())` and `WalkObservation`
carries `status` and `error`. The loss is one layer up.

`crates/scripting_lua/src/globals/goal/run.rs` built `first_error` from failed
**actions** only. A walk has no `ActionId`, so it is in neither `failed` nor
`lost` — and when a walk fails, `abandon_rest` abandons the rest of that bot's
slice, so every action stays `Pending`. The run therefore reports
`failed = 0, first_error = nil`, `scripts/supervisor.lua` computes
`failed = obs.failed + obs.lost` = 0, leaves `any_failures` false, and halts as
`stuck_silent` — which is defined as "no progress and *every run claimed
success*". A run that dispatched nothing at all was classified as a run in
which nothing went wrong.

Fixed in three places:

- `goal/run.rs` counts failed walks into a new `obs.walks_failed` and falls
  back to the first failed walk's error when no action failed. An action's
  error still wins, because it names the thing the plan was trying to do; the
  walk error is a fallback, so every existing reading of `first_error` is
  unchanged.
- `walks_failed` is a **separate count**, not folded into `failed`: `failed`
  is a count of actions and every field beside it is about actions, and a
  caller reading it as one must not silently start getting a different number.
- `scripts/supervisor.lua` adds the term itself, so a walk failure makes
  `any_failures` true and the halt is `stuck` carrying the pathfinder's own
  words.

## 4. Tests

All verified **red first**, against the unmodified code, in a clean worktree —
not merely written after the fix.

- `crates/core/tests/botbridge_walk_stuck_teleport.rs`, 7 tests. Loads the real
  `control.lua` into mlua and drives `on_tick` against a stub surface that
  models occupancy the way the game does: a position is free only if no other
  character is standing on it. **5 of 7 fail against the pre-fix mod**,
  including the two-bot reproduction; 2 pass and are the controls (a clear
  waypoint still lands exactly on it; a stuck *last* leg still aborts without
  teleporting).
- `globals::goal::run::tests::a_run_that_failed_only_in_its_walks_reports_the_walk_error`
  plus its clean-run control. Needed a new `StubActuator::with_failing_walks`,
  because the stub deliberately never failed a walk.
- `supervisor_lib::tests::walks_that_failed_are_failures_and_the_halt_carries_their_error`
  drives the real shipped `supervisor.lua`. Red output was literally
  `left: "stuck_silent", right: "stuck"` — the run's own misclassification,
  reproduced.

`cargo fmt --all --check` clean, `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` clean, `cargo test --workspace` **1267
passed, 0 failed, exit 0**, at `f7b60f1c` plus only this change.

## 5. Could not be verified without a live run

1. **That Factorio's `find_non_colliding_position("character", ..)` counts
   other characters as obstacles.** This is the load-bearing assumption. The
   character prototype has a collision box and characters demonstrably collide
   while walking, and the method takes the collision box "from this prototype",
   so it should. The test's stub models it that way *by construction* and
   therefore cannot prove it. **If it does not, the fix is inert** — the
   symptom would be unchanged stacking, visible as two bots at identical
   coordinates in `samples.jsonl`, which is now a thing to look for
   specifically.
2. **Whether a radius of 4 tiles is enough**, and whether being put up to 4
   tiles off the waypoint ever leaves a bot somewhere the next leg cannot be
   walked from. Too small and the recovery gives up where it used to (wrongly)
   succeed; too large and it stops being a recovery.
3. **Whether the new abort arms fire often enough to matter.** Both convert a
   silent stack into a failed walk, which is strictly better information, but a
   walk that now fails where it used to "succeed" changes the run's shape.
4. **Whether the underlying stuck is fixed.** It is not, and this does not
   claim to. Bots still get stuck on legs often enough for this recovery to run
   fifteen times in one run; what changed is that the recovery no longer makes
   the situation permanent. Why the legs go stuck in the first place is
   unexamined.
5. **The teleport record's tick.** `not_before` flattens a drained batch onto
   the highest recorded tick, so a teleport's recorded tick is an upper bound,
   not an observation. That is a separate defect in the record and was **not**
   fixed here — the mod stamps the real `event.tick` on the writeout and the
   flattening happens Rust-side, in `record.teleports()`.
