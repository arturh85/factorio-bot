# The two reply-shaped failures of run `run-1788304631-16800`

Commit: `a5dfad9d` — `fix(rcon): report what an unparseable reply contained, not
just that it failed`. Not committed: this note.

## `Unexpected Response: nil` — milestone 2 — **certain**

`mods/BotBridge/control.lua`, `on_tick`'s mining branch:

```lua
action_failed(event.tick, storage.p[idx].mining.action_id)   -- no reason
```

`action_failed` writes `"fail " .. action_id .. " " .. tostring(reason)`, so the
verdict on the wire was the literal string `nil`. `sleep_for_action_result`
wraps whatever `ActionOutcome.result` holds in `RconError { message }`, whose
Display is `Unexpected Response: {message}`.

The record agrees precisely: `id 3, bot 4, mine 5 copper-ore`, dispatched at
tick 5204, **settled at 5765**. It was dispatched and the game gave a verdict
561 ticks later — which is only reachable through the `on_tick` branch above.
That branch fires when the entity a bot was mining disappears without
`on_mined_entity` firing for it, i.e. another bot got the ore first. Four bots
were mining copper at the same time.

## `expected value at line 1 column 1` — milestone 4 — **one call named, with a caveat**

serde_json's message for input that is not JSON at all, reached through a bare
`serde_json::from_str(..).into_diagnostic()`. Eight sites in `rcon.rs` produced
it identically.

What the record fixes without inference:

* the failing step was an **action**, not a walk — `obs.first_error` is only
  populated from actions with `Status::Failed` (`goal/run.rs`), and the
  supervisor only reads it when `failed > 0`;
* it carried **no ticks at all** — `record.actions` writes an
  `action_dispatched` only when `dispatched_tick` is present, and milestone 4
  has **zero** action events across three iterations;
* it was the plan's **first** action for the only bot in the plan (`bots: [1]`),
  `place stone-furnace at [38, 16]`, `planned_start = 0`, no preceding walk
  (`schedule.rs` emits a `Walk` step only when `chosen.travel > 0`);
* bot 1 was standing at `(38.3046875, 16.4765625)` — **inside the 2×2 footprint
  of the furnace it was told to place**. `samples.jsonl`, tick 5880.

So the placement necessarily hit `can_place_entity{build_check_type = manual}`
returning false with the player inside the bounding box, i.e.
`§player_blocks_placement§`. That is now pinned by a test that drives the real
`control.lua` with exactly those coordinates
(`a_refused_placement_still_stamps_the_tick_it_was_refused_at`).

`place_entity_timed`'s recovery for that case is an escape dance: for each of
eight compass points, `is_area_empty` (radius 2), then `move_player` to a point
5 tiles away. Exactly two calls inside it can produce a bare serde_json message,
both with `ActionTicks::UNKNOWN` (the mod's refusal path did not stamp a tick):

1. **`move_player` → `player_path` → `sleep_for_path_request_result`**, parsing
   the mod's own `"Error: failed to path find"` / `"Error: try again later!"` as
   JSON;
2. `is_area_empty` → `find_entities_filtered` / `find_tiles_filtered`, if the
   mod raised and Factorio put `Cannot execute command. Error: ...` in the reply.

**The timing separates them.** `plan_created` for milestone 4 lands at
wall_ms 36058, 36828, 37612 — and `milestone_started` is at 36038, so planning a
95-step plan costs 20 ms and **execution costs ~750 ms per iteration**. An
`is_area_empty` failure on the first direction is one RCON round trip, ~15 ms.
Five path requests (`player_path` tries once and then four offset goals, each
polling on a 50 ms tick) plus a handful of area queries is ~600–750 ms. Only (1)
fits.

**Caveat, stated plainly:** (1) is an inference from timing, not a measurement.
(2) remains possible. It does not change the fix — after this commit both name
themselves, so the next run settles it in one line rather than an hour.

## What changed

`crates/core/src/factorio/rcon.rs`
* `sleep_for_path_request_result` no longer feeds the mod's plain-text verdicts
  to serde_json. Anything not starting with `[` becomes
  `RconPathRequestFailed { reason }`, carrying the mod's words verbatim. This is
  a protocol bug independent of the diagnosis above: the mod deliberately writes
  those strings and this side deliberately misread them.
* `parse_reply(call, text)` wraps every remaining reply deserialisation
  (`place_blueprint`, `revive_ghost`, `cheat_blueprint`, `retrieve_map_data`,
  `inventory_contents_at`, `player_force`, `find_entities_filtered`,
  `find_tiles_filtered`, `place_entity`, `async_request_path`). It produces
  `RconReplyNotJson`, which names the call, the byte count, serde's own message
  and the first 200 characters of what actually arrived, truncated on a `char`
  boundary. serde's message is kept because for a typed mismatch deep in a valid
  document it is the useful half and the snippet is not.
* `place_entity`'s two `serde_json::from_str(line).unwrap()` are gone. They
  panicked inside a run's dispatch task on the failure path. The grapheme index
  beside them (`chars[0]`) panicked one line earlier on an empty reply line,
  which `split_reply` can produce; both are now `line.starts_with('{')`.

`mods/BotBridge/control.lua`
* the mining `action_failed` call site names the entity and what happened;
  `action_failed` itself substitutes a self-describing default rather than
  `tostring(nil)`.
* `rcon_place_entity` calls `stamp_tick()` on all three refusal exits. They
  returned bare, so a placement the game *had judged* carried no tick, and
  `record.actions` writes nothing for an action with no ticks — which is the
  whole reason milestone 4's failure has no `action_dispatched` naming it.
  `take_tick_stamp` lifts the stamp out from anywhere in the reply, so the
  shape-based judgement in `place_entity_timed` is unaffected.
* three unconditional debug `print`s in `rcon_place_entity` removed. They were
  on stdout, not `rcon.print`, so they were not causing failures — but they were
  three `helpers.table_to_json` calls per placement on the hot path.

`crates/core/src/process/output_parser.rs`
* `on_script_path_request_finished` used `split('#').collect()` and read
  `parts[1]`: it truncated any payload containing a second `#` and **panicked**
  on a line with none, inside the parser task. Now `split_once`.

`workspace/mods/BotBridge/control.lua` was refreshed from the repo copy (it was
byte-identical to `HEAD` beforehand), because a debug build uses the workspace
copy and has no refresh path.

## The design issue, with options — not fixed, not guessed at

**The planner will reliably schedule a placement the bot is standing on.** An
action's `AtPosition { pos, radius }` is satisfied by "within radius of the
entity's own tile", which includes "on it"; `schedule()` emits a `Walk` step
only when `travel > 0`, so a bot that ends its previous action on the target
tile is never moved off it. Factorio's `can_place_entity{build_check_type =
manual}` then refuses, and the whole `§player_blocks_placement§` dance — up to
16 RCON queries and a pathfinder round trip — exists to undo it. That is the
proximate cause of milestone 4's failure whichever of the two calls threw.

Three ways out, in decreasing order of how much they fix and increasing order of
how much they cost:

1. **Planner: make the standing position an annulus, not a disc.** Give a
   `Place` action a minimum as well as a maximum distance — the entity's
   collision box plus the character's radius — so `travel > 0` whenever the bot
   is inside the footprint and a real `Walk` step is scheduled. Removes the
   failure at its source, makes the walk visible in the plan and the record, and
   costs the executor nothing. Needs the placed entity's collision box in the
   planner's world snapshot; `entity_prototypes` already carries it.
2. **Mod: let the game move the character.** `LuaSurface.create_entity` takes
   `move_stuck_players` (verified in `runtime-api.json`, Factorio 2.1.17). The
   blocker is the `can_place_entity{... manual}` pre-check, which refuses first;
   `build_check_type` would have to change, and `manual` is what enforces the
   ordinary build rules. Cheap to write, but it trades a rule the mod currently
   respects for one it does not, and I cannot verify the trade without running
   the game.
3. **Executor: keep the dance but stop it needing the pathfinder.** Teleporting
   is out; a short `move_player` to a point just outside the footprint rather
   than 5 tiles away, checked with one `is_area_empty` rather than eight, would
   cut the round trips by an order of magnitude. Papers over (1) rather than
   fixing it, and still fails when the pathfinder says no.

My recommendation is (1). It is the only one that makes the plan honest about
what the bot has to do, and the record shows the bot standing in its own build
site as a fact the planner could have known.

## Gates

`luac -p mods/BotBridge/control.lua` clean; `cargo fmt --all -- --check` clean
(only my three files were rewritten — another agent is editing
`crates/planner` and `crates/executor` in this checkout, so `cargo fmt --all`
was not run); `cargo clippy --workspace --all-features --all-targets --deny
warnings` clean; `cargo test --workspace --all-features` — every suite ok, no
failures. Eight new tests: two on the pathfinder verdict, four on the snippet
and `parse_reply`, one driving the real `control.lua` for the placement stamp,
one for `action_failed`'s reason, three on the output parser.
