# Rung 4: "tried to remove 20 iron-plate but removed 18"

`workspace/runs/run-1788320177-77989`, milestone 4 (`research automation`),
stuck after 7 iterations at 99 steps.

## The record answered it. Two streams, plus one cross-check.

**`events.jsonl` was decisive.** Three queries were enough:

1. The only failure carrying the reported message is action id **74**, bot 1,
   at tick **47548**, and it is not an inventory action at all — the plan at
   tick 5714 names it `take 20 iron-plate from the furnace`, `deps [71, 72,
   73]`. The 20 plates were never in a bot's inventory; they were in a stone
   furnace's output slot, and the "removal" is from the furnace.
2. The dependency timeline: id 72 (`insert 20 iron-ore`) settled at tick
   **43949**, id 74 dispatched at tick **47548**. **3599 game ticks** between
   them.
3. A stone furnace smelts iron plate in 3.2 s = **192 ticks** at crafting speed
   1. `3599 / 192 = 18.74`. Eighteen plates. Exactly what the mod reported.

**`samples.jsonl` was a false lead and worth recording as one.** The bot
inventories are sampled on the mod's own *tick* cadence, not on a wall clock —
tick deltas run `0, 60, 60, 60, 60, 60` forever, one duplicate every 300 ticks
where the frame-capture writer adds a line. Nothing in that stream measures UPS,
which is what this failure turns on. Do not try to derive a rate from it.

**`manifest.json` supplied the cross-check.** 1217 frames over 60785 ticks is
`202 captures x 6 cameras` — six 1920x1080 JPEGs every 300 ticks, for the whole
run. That is the load that put the server behind, and it is why this bit rung 4
and not rungs 1-3.

## Which hypothesis was right: none of the four.

- **Stale belief (1)** — no. The planner sized 20 plates from 20 ore correctly;
  the plan's own `planned_start` values put 4032 ticks between the insert and
  the take, which is right.
- **Double spend (2)** — no. Nothing was spent from a bot's inventory. The
  ledger is not involved.
- **Production shortfall (3)** — nearest, but the mechanism is not the one
  described. The furnace was not broken and no step reported success falsely:
  the furnace was still *working*, and it was asked 433 ticks early.
- **Someone else spent them (4)** — no. One bot, one furnace, `map.jsonl` shows
  no competing take.

The actual cause is a fifth: **the executor honoured a game-tick lag edge by
sleeping on the wall clock.**

`await_preds` computed `max_lag = 4032` ticks and slept
`ticks_to_wall_clock(4032, game_speed)` = **67.2 s**, on the assumption that a
server delivers `60 * game.speed` ticks a second. This one delivered ~53.6.
`game.speed` cannot report that — it is the rate the game is *asked* to run at,
and `Actuator::game_speed` was a stub returning `1.0` besides. So the wait was
short by ~10.7%, or 433 ticks, or 2.25 plates.

Two details make this specifically a rung-4 bug:

- **The loss scales with the batch; the headroom does not.** `Smelt::expand`
  adds one craft cycle of headroom (192 ticks) precisely because an earlier run
  collected 9 of 10 at insert+1924 against a modelled 1920. 192 ticks is 4.8% of
  a 20-plate lag and covers a 10.7% deficit for batches up to about 9. Every
  smaller smelt in this same run came back clean — 15 copper, 10 iron, 4 iron,
  2 iron — and one 20-plate take at tick 40609 only survived because the bot's
  own schedule had it queued far behind its lag.
- **Nothing reports the deficit.** An early collection is indistinguishable from
  a slow furnace from inside the executor, so the run spent six more iterations
  replanning around a plan that was never wrong.

## The fix

**Wait on the game's clock, not on a converted one.**

- `FactorioRcon::game_tick()` (`crates/core/src/factorio/rcon.rs`) asks the game
  what tick it is now. It is `/silent-command rcon.print("§tick§"..game.tick)` —
  **vanilla Lua, no BotBridge**, so it keeps answering against a save whose
  `workspace/mods` copy predates the binary, and it reuses the mod's own
  `§tick§` stamp so exactly one parser (`take_tick_stamp`) reads a tick off a
  reply. Not `last_tick()`: that is the stamp off whatever was last *sent*, and
  the whole point of this wait is that nothing is being sent.
- `Actuator::game_tick()` (`crates/executor/src/actuator.rs`) with a default of
  `Ok(None)` — "I have no clock". Every mock inherits it and takes the old path,
  which is why no existing test moved.
- `run::wait_out_lag` reads the tick, sleeps the *estimated* remaining time,
  reads again, and repeats until the game's own clock has advanced by the lag.
  The wall clock survives only as the estimate of how long to sleep between
  readings, where being wrong costs a round trip instead of a plan.
- The chase is budgeted (`LAG_CHASE_BUDGET = 1`, so at most ~2x the first
  estimate). A stopped clock — `/editor`, a paused host, a server mid-save —
  must not hang a bot forever; giving up and letting the action's own verdict be
  the report is a far better failure.

`game.speed` is now only an input to the estimate, so the stub returning `1.0`
can no longer make a wait wrong — only slightly slower to converge.

**Nothing in the planner changed.** It was right. Its one-cycle headroom now
does the job it was written for, because the clock underneath it is real.

**No mod change**, which is the pleasant part: the workspace copy does not need
re-seeding and there is no stale-mod trap to fall into.

## What a partial removal settles as

**Before:** `Status::Failed`, correctly — `judge_transfer_reply` already treats
any complaint left in the reply body as a refusal, so a partial move was never
green. But the *structured* failure the record wrote was
`{"kind": "rejected", "detail": null}`, which is a different claim from the
truth. A rejection moved nothing. A partial transfer already moved 18 plates
into the bot's hands, because `rcon_remove_from_inventory` removes whatever the
inventory had and inserts it into the player *before* it complains. Same kind,
opposite world states, and the counts were reachable only by re-parsing prose.

**Now:** `FailureKind::PartialTransfer` with
`detail: "moved 18 of 20 iron-plate"`. `classify_failure` parses all three of
BotBridge's transfer complaint wordings (`tried to remove N X but removed M`,
`tried to insert Nx X but inserted M`, `cannot insert Nx X, ... only has M.
clamping...`), and *parses* rather than pattern-matching wholesale — a wording
it cannot read degrades to `Rejected` with no detail rather than inventing a
number. It is checked before `Rejected` and must stay there, since the text
arrives wrapped in `game rejected the command`.

The producing and consuming sides are in different crates and cannot see each
other, so the mod's exact wording is now asserted **whole** in
`transfer_guarantee_tests` (which drives the real `control.lua` under an mlua
stub). If the mod's phrasing moves, that test fails and names the classifier.

## Fixed vs. reported

- **Fixed:** the lag wait; the loss of both counts on a partial transfer.
- **Reported, not fixed:** `Actuator::game_speed` is still stubbed at `1.0` (now
  harmless — it only shapes the sleep estimate). `events.jsonl`'s `wall_ms` is
  badly wrong in this run: it jumps `33780 -> 738866` while the tick moves 10,
  so it is stamped lazily somewhere and cannot be used to derive a rate. That is
  its own defect and was left alone.

## Tests

`crates/executor/src/run.rs`

- `a_lag_edge_waits_for_game_ticks_not_for_seconds` — the fixture's actuator
  runs a clock at **50 ticks/second** while reporting `game.speed` 1.0, so a
  60-tick lag must take 1.2 s and not 1.0. **Verified to fail against the old
  wall-clock code** ("waiting 1s means the lag was spent as seconds rather than
  as ticks") before being kept.
- `a_stopped_game_clock_ends_the_lag_wait_instead_of_hanging_the_run` — a clock
  at 0 ticks/second still finishes, between 1x and ~2x the estimate.

`crates/scripting_lua/src/globals/record.rs` — four: the live wording from this
run, the insert wording, the clamp wording, and an unparseable wording that must
fall back to `Rejected` with no detail rather than guess.

`crates/core/src/factorio/rcon.rs` — the two complaint wordings pinned whole
against the real mod source, plus the tick query's shape and round trip.

**No existing test moved.** Every mock actuator inherits `game_tick`'s `None`
default and keeps the old wall-clock path, which is what the default is for. The
only non-Rust edits are the OpenAPI snapshot and its TypeScript mirror, both
mechanical consequences of the new `FailureKind` variant and both required by
the seam that fails from either end.
