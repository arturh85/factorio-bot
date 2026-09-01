# Walk-fix report (A1, A6) -- 2026-09-02

## Status

Done. All gates green.

## Commit(s)

`c64a995b` -- fix(mod): stop the stuck-walk teleport from firing on ordinary long legs
(branch: master; touches app/src/api/{types.ts,openapi.snapshot.json,openapi.contract.spec.ts},
crates/core/src/{factorio/rcon.rs,process/output_parser.rs,record/mod.rs},
mods/BotBridge/control.lua)

## Test summary

- `nix shell nixpkgs#lua5_4 -c luac -p mods/BotBridge/control.lua` -- OK
- `cargo fmt -p factorio-bot-core -p factorio-bot-executor -p factorio-bot-server -p factorio-bot-scripting-lua -- --check` -- clean (did not touch/format crates/planner, which another agent is editing)
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` -- clean
- `cargo test --workspace` -- all green (0 failures), including the OpenAPI snapshot test after regeneration
- `cd app && pnpm run test:coverage` -- 781/781 tests pass, coverage gate met
- `cd app && pnpm lint` -- clean (tsc, vue-tsc, eslint)

## Character speed

Used `player.character_running_speed` (`LuaControl.character_running_speed`, "the current movement speed of this character, including effects from exoskeletons, tiles, stickers and shooting"), read live at each leg boundary rather than the hardcoded 0.15 tiles/tick. Verified `LuaPlayer`'s `parent` is `LuaControl` in `runtime-api.json`, so `player.character_running_speed` is valid (this is a `LuaControl`/`LuaPlayer` attribute, not a `LuaEntity` one -- unlike the `mining_target` crash this audit warned about). A 0.15 fallback is kept only for the case `speed` is nil/non-positive, which should not happen for a connected player with a character (the only case this is ever called for).

Per-leg timeout = `max(60, ceil(leg_length / speed * 3))` ticks: straight-line distance over live speed, tripled for a generous margin (turning, deceleration, other entities in the way), floored at 60 ticks (one second, matching the previous flat constant) for very short legs. Stamped at walk start (`rcon_action_start_walk_waypoints`), at every leg advance, and again right after a teleport (defensive -- the next tick's arrival check would normally re-stamp it anyway once it sees the character at the destination).

## needs_destroy_to_reach (A6)

Carried all the way through, not just recorded as a side-note:

1. `control.lua`'s `on_script_path_request_finished` now flattens `needs_destroy_to_reach` onto each waypoint (`{x=.., y=.., needs_destroy_to_reach=..}`) instead of sending bare `{x=.., y=..}`. Additive wire shape -- an older Rust build parsing this as a plain position still works.
2. `crates/core/src/factorio/rcon.rs` gained `PathWaypoint { #[serde(flatten)] position: Position, #[serde(default)] needs_destroy_to_reach: bool }`. `sleep_for_path_request_result` now parses `Vec<PathWaypoint>` (was `Vec<Position>`); the existing `path_request_reply_wakes_the_waiter` regression test still passes unmodified because its literal JSON (no `needs_destroy_to_reach` field) defaults to `false`.
3. New helper `waypoint_positions()` extracts `Vec<Position>` (so `player_path`/`path`'s public signatures are unchanged) and `tracing::warn!`s once per call if any waypoint in the result needs destroying, naming how many of how many.

This is short of *acting* on the flag (refusing/rerouting a blocked leg) -- that's a bigger behavioural change than this task's scope, and the task explicitly allowed "at minimum stop discarding it and record it" as the fallback. What shipped is strictly more than the minimum: the flag reaches the Rust caller of the walk (not just a log line keyed by an opaque request id), and it's structurally available to any future caller that wants to act on it.

## Concerns / things worth a second look

- **The "already-arrived, zero-waypoint walk" case.** `move_player_timed` can legitimately dispatch `action_start_walk_waypoints` with an empty waypoint list when the bot is already within tolerance of the goal. That hits the same `dest == nil` branch as a stuck-abort. I added a `w.stuck` flag set only by the stuck-abort branch, so the zero-waypoint case still reports success (`action_completed`) and the stuck-abort case reports failure (`action_failed`) -- but this is inferred from reading `move_player_timed`/`walk_end_position`/`walk_arrives`, not from a live-tested Factorio run (I was told not to run Factorio). Worth a live smoke test of "goal already met" walks before trusting this in production.
- **`action_failed` on the last-waypoint stuck-abort is a behaviour change.** Before this fix, a walk stuck on its last waypoint silently spun forever while reporting `ok` every tick. Now it reports a real failure after `leg_timeout` ticks. Any caller (executor, supervisor scripts) that assumed "walk actions always eventually say ok" needs to tolerate this failure path -- I did not audit every executor call site for that assumption, only confirmed the workspace test suite (which includes executor tests) still passes.
- **`EventKind::Teleport` has no live writer yet.** `record_live` (the only thing that ever calls `RunRecorder`/writes `events.jsonl`) lives in `crates/scripting_lua/src/globals/record.rs` and is driven entirely by Lua script calls (`record.action_dispatched`, etc.); `crates/core`'s `OutputParser` has no access to that recorder (wrong dependency direction: core does not, and should not, depend on scripting_lua). So today a teleport is loud in `tracing` diagnostics (stderr) but does not yet land in a run's `events.jsonl` -- the schema exists and is contract-tested end-to-end, but wiring an actual call site (presumably a new Lua-exposed `record.teleport(...)` plus a supervisor.lua call, or a poll of `world` state) is follow-up work outside this task's stated file list (control.lua, output_parser.rs, record/mod.rs).
- **Untouched, by instruction:** did not touch `crates/planner/` (another agent has `crates/planner/src/method/have.rs` modified in the working tree; left entirely alone and excluded from the commit via explicit pathspec).
- **Did not run Factorio**, per instructions -- everything above is verified by `luac -p`, `cargo test --workspace`, `cargo clippy`, and reading `runtime-api.json` directly, not by an actual game session.
