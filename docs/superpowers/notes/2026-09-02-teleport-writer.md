# Teleport writer: wiring EventKind::Teleport into events.jsonl

## Status

Done. Workspace builds and all tests pass except a pre-existing failure in
`crates/planner` (`red_science::every_expansion_replays_in_time_order`) that
belongs to the other agent's in-flight work there -- untouched by this change,
confirmed unrelated (a `stone-furnace` placement precondition, nothing to do
with recording). `cargo fmt --all -- --check` likewise shows diffs only inside
`crates/planner`.

## Commit

`62ae4580` -- `feat(record): write EventKind::Teleport to events.jsonl`

Note: the shared checkout's current branch was `master` at commit time (other
agents' commits, e.g. `adc952b9`, are already there). The branch was
`feat/axum-server` in this task's initial context but had moved under me by
the time I committed -- I did not switch it myself. Flagging in case that
matters; I did not attempt any branch surgery.

## Were the mod/parser halves already in place as claimed?

Yes, verified directly rather than assumed:

- `mods/BotBridge/control.lua`'s `teleport_writeout(tick, player_id, reason,
  from, to, action_id)` (used at all three `player.teleport` call sites: the
  stuck-walk timeout at the walk loop, and the `revive_ghost_blocked` /
  `place_blueprint_blocked` sites) already emits `{player_id, reason, from,
  to, distance, action_id}` via `writeout(tick, "teleport", ...)`.
- `crates/core/src/process/output_parser.rs` already had a `"teleport"` match
  arm that deserialises that JSON and logs it via `tracing::warn!`. It did
  nothing else -- the comment beside it said so explicitly.
- `crates/core/src/record::EventKind::Teleport` already existed with exactly
  the fields the mod emits (`bot`, `reason`, `from`, `to`, `distance`,
  `action_id`), and the OpenAPI snapshot / `app/src/api/types.ts` already
  matched it field-for-field. **No seam changes were needed** -- the shape was
  already right; only the missing hop from parser to recorder was missing.

## What the event carries

Unchanged from what already existed (confirmed sufficient, not modified):
`bot: u32`, `reason: String` (free text: `walk_stuck`, `revive_ghost_blocked`,
`place_blueprint_blocked`, distinguishing the three sites), `from: Position`,
`to: Position`, `distance: f64` (the mod's own Euclidean distance, not
recomputed on the Rust side), `action_id: Option<u32>` (present only for the
stuck-walk site).

## The wiring added

`crates/core` cannot depend on `crates/scripting_lua` (where `RunRecorder`
lives), so a teleport crosses the boundary as queued data rather than a
direct call, mirroring how `FactorioWorld::actions` already carries
`action_completed`:

- `crates/core/src/factorio/world.rs`: moved `TeleportEvent` here (was a
  private struct in `output_parser.rs`, now `pub`), added
  `FactorioWorld::teleports: parking_lot::Mutex<Vec<(u64, TeleportEvent)>>`
  plus `record_teleport`/`drain_teleports`. The tick stored is the real game
  tick the mod stamped on the `writeout` line, not an approximation.
- `crates/core/src/process/output_parser.rs`: the `"teleport"` arm now also
  calls `self.world.record_teleport(tick, event)` after logging. Added
  `OutputParser::with_world(Arc<FactorioWorld>)` so a test can share a world
  between a real parser and the Lua recording sandbox.
- `crates/scripting_lua/src/globals/record.rs`: new `record.teleports()` Lua
  binding -- drains the world's queue and writes each as
  `EventKind::Teleport`, using `recorder.not_before(tick)` on the event's own
  real tick (more accurate than `record_live`'s "last RCON reply tick"
  fallback, since the exact moment is already known here).
- `scripts/research_run.lua`: calls `record.teleports()` once per `"ran"`
  transition (next to `record.actions`) and once more right before
  `record.finish(...)`, so nothing queued after the last transition is lost.
- `crates/scripting_lua/src/research_run_lib.rs`: its Lua test harness stubs
  out `record.*` to drive `research_run.lua` without a real game; added
  `record.teleports = function() return 0 end` alongside the existing stubs
  -- without it, three of that file's tests failed with "attempt to call a
  nil value (field 'teleports')" once the script started calling it.

## Test summary

Two new tests in `crates/scripting_lua/src/globals/record.rs`:
`teleport_writeout_reaches_events_jsonl_through_the_real_parser` drives an
actual `writeout`-shaped line through a real `OutputParser` into a
`FactorioWorld` shared with the recording Lua sandbox, then calls
`record.teleports()` and asserts the resulting `events.jsonl` line matches;
`teleports_distinguishes_all_three_mod_sites_and_drains_the_queue` pushes all
three reasons, checks write order, `action_id: None` for the two
ghost/blueprint sites, and that a second drain reports zero. Both pass;
`cargo test --workspace --all-features --exclude factorio-bot-planner` is
otherwise all green (272+ tests across core/executor/scripting_lua/server).

## Concerns

- The shared-checkout branch drift noted above under Commit.
- `crates/executor/src/recover.rs` and `crates/executor/src/run.rs` each show
  a 1-line uncommitted change in `git status` that I did not make and did not
  touch -- presumably another agent's concurrent work in that crate. Left
  alone.
- I did not modify `mods/BotBridge/control.lua`, so the `luac -p` gate was not
  run (nothing to check).
