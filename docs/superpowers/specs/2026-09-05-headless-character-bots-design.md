# Headless character bots and a game-speed knob — design

2026-09-05. Owner decision: the recommendations in
`docs/superpowers/notes/2026-09-05-remote-control-approach-review.md` are
approved; design coordinated with the speedrun session (factorio-bot-78), which
raised no objection and named the seams listed under "Constraints from the
other workstream".

## Problem

Every run costs 20+ minutes of wall clock because the world runs at 1x and
every bot is a graphical Factorio client (26 s sprite load, connect wait, one
window each). Three days produced 24 runs. The Factorio Learning Environment
dropped the client in its 0.3.0 and runs at `game.speed = 10`; this project's
executor has `Actuator::game_speed` stubbed at `1.0` and the mod refuses a walk
unless `player.connected and player.character` hold.

## Spike result (2026-09-05, scratch server, seed 31337, base only)

A bare `character` entity created with `surface.create_entity` and no player:

| capability | result |
|---|---|
| walk via `walking_state` set per tick | 57 tiles in 391 ticks, 0.15 tiles/tick, same as a player |
| hand-mine via `mining_state` | **nothing** until `update_selected_entity(pos)` is called first; then 121 ticks per iron ore, same as a player. `mining_state` reads back `mining=false` throughout, so it cannot be used as the progress signal |
| `begin_crafting` | real timing (2 gears in 62 ticks); `crafting_queue_size` reports the queue |
| reach | `reach_distance` 10, `resource_reach_distance` 2.7, `build_distance` 10, on the entity |
| `can_reach_entity`, `get_main_inventory`, `insert`, `remove_item` | work |
| `on_player_*` events | **none fire** for it (crafted, mined, inventory changed, position) |
| `game.speed = 10` | the headless server delivered ~620 ticks per wall second |
| `game.tick_paused = true` | freezes the tick |
| server with zero players | keeps ticking because `server-settings.json` already has `auto_pause: false` |

So the approach is feasible; the work is (a) making the mod address a bot that
is a character entity rather than a player, (b) replacing the four player
events with per-tick polling for such bots, and (c) plumbing the speed.

## Goals

1. `factorio-bot lua <script> --headless --bots N` runs the script against N
   server-side character bots with no graphical client, honest play (real walk
   speed, mining time, crafting time, reach), and the same run record.
2. `--game-speed S` runs the world at `game.speed = S`, with every wall-clock
   deadline scaled so the executor's behaviour is unchanged in game ticks.
3. Provenance records both, and `--compare` says so rather than refusing.
4. Client mode is untouched: same code paths, same defaults.

## Non-goals

- Pause-between-steps (turn-based execution). Not in v1; the executor is
  event-driven across bots and speed alone gives the iteration win.
- Mixed rosters (some clients, some characters). Refused with a named error.
- Video in headless mode. `record.start({video = true})` errors: video is
  filmed from a client window.
- Blueprint placement through the player cursor. `rcon_place_blueprint` and
  `rcon_cheat_blueprint` are cheat or development paths and stay player-only;
  they refuse a character bot by name.

## Design

### 1. The mod: a bot is a player or a character, behind one handle

`mods/BotBridge/control.lua` addresses bots as `game.players[id]` in 22 places
and iterates `game.players` in the per-tick loop, `rcon_players`,
`writeout_players` and `on_player_changed_distance`. Nearly every member it
uses is on `LuaControl`, which `LuaEntity` (character) shares with
`LuaPlayer`. The exceptions are `.character`, `.connected`, `.index`, `.name`,
`.print`, `.ticks_to_respawn`, `.controller_type`.

**Registry.** `storage.bots[id] = { entity = <LuaEntity>, name = "bot-<id>" }`
for character bots. LuaEntity references are storable; the entry persists
across a save, so a savepoint resume finds its characters again.

**Handle.** `bot_handle(id)` returns `game.players[id]` when no character bot
has that id, else a proxy table whose metatable forwards every read and write
to the entity and answers the player-only members itself:

- `character` → the entity; `connected` → `entity.valid`; `index` → id;
  `name` → the registry name; `print` → no-op; `ticks_to_respawn` → nil;
  `controller_type` → `defines.controllers.character`.
- Everything else, reads and writes (`walking_state`, `mining_state`,
  `position`, `surface`, `force`, `update_selected_entity`, `selected`,
  `can_reach_entity`, `get_main_inventory`, `begin_crafting`, `insert`,
  `remove_item`, `crafting_queue`, reach distances…) forwards to the entity.
  Factorio API methods are dot-called with no `self`, so returning the
  entity's function is a correct forward.

The proxy is built on each `bot_handle` call, never stored (metatables do not
survive `storage`).

**Iteration.** `each_bot()` yields `(id, handle)` over `game.players` and then
`storage.bots`, and replaces the four `pairs(game.players)` loops above.
`get_player(id)` calls `bot_handle`, so the eleven `get_player` sites and the
merged death and reach guards keep their shape. The remaining direct
`game.players[player_id]` reads in RCON entry points become `bot_handle`.
Event handlers keyed on `event.player_index` are untouched: only real players
raise them.

**Mixed mode is refused.** `rcon_spawn_bots` refuses when any player is
connected; `on_player_joined_game` refuses (kicks with a message and prints
`ERROR: ...`) when `storage.bots` is non-empty. Ids would otherwise collide:
a character bot is id 1..N, and the first joining player is also index 1.

### 2. Character-bot completions by polling

Four player events carry state today. For character bots, `on_tick` polls
instead, under the same waiters and writeouts so the Rust side sees identical
records:

| player event | character-bot equivalent |
|---|---|
| `on_player_changed_position` → `writeout` position | per tick, if the position differs from the last written one, the same writeout |
| `on_player_main_inventory_changed` → `writeout` inventory | per tick, compare `get_contents()` with the last snapshot (name → count map); on difference the same writeout |
| `on_player_crafted_item` → `settle_crafted_item(event)` | per tick, snapshot `crafting_queue` as recipe → total count; a drop of k for a recipe means k crafts finished: call `settle_crafted_item({player_index = id, recipe = force.recipes[name], tick})` k times and append the same `recent_item_additions` entry. A recipe that vanishes from the queue counts as its whole remaining count finished. Cancellation cannot happen without a player. |
| `on_player_mined_entity` → `mining.left` accounting from `event.buffer` | when a mine starts for a character bot, record the product-name counts in the main inventory; each tick `delivered = sum(now − before)` over the prototype's product names, and the rest of the accounting is the existing code. Checked **before** the `ent.valid` test, because a tree or rock is destroyed on the same tick its wood lands. |
| `on_player_died` / `on_player_respawned` | `on_entity_died` for an entity whose `unit_number` is in the registry: same writeout (`player_died`, `cause`, `respawn_in = 600`) and the same waiter failures; a respawn entry in `storage.bots[id].respawn_at = tick + 600`; on that tick a new character is created at the force spawn, the registry entity is replaced, and the `player_respawned` writeout and position writeout follow. |

Mining for a character bot calls `update_selected_entity(ent.position)` — the
miner already does this for players, so no change there; the spike confirmed
it is what makes `mining_state` take effect.

`sample_bots` (`game.connected_players`) uses `each_bot()` so `samples.jsonl`
carries character bots.

### 3. Spawning, and what the Rust side asks for

`rcon_spawn_bots(count)` (new): for id 1..count, if `storage.bots[id]` holds a
valid entity keep it (savepoint resume); else create a `character` at
`force.get_spawn_position(surface)` via `find_non_colliding_position`, insert
the freeplay starting items (`remote.call("freeplay", "get_created_items")`
when the interface exists, so the inventory matches a joining player's), and
register it. Then emit for each bot what a join emits: the inventory writeout,
the distance writeout, the position writeout. Reply: JSON list of ids spawned
or kept. Refuses with `Error: ...` when a player is connected.

`rcon_players()` includes character bots through `serialize_player(handle)`,
which reads `name, index, position` and the six distances off the proxy, so
`FactorioPlayer` deserialises unchanged, including `resource_reach_distance`
as a double. `RconActuator::new`, `Planner::roster`, `refuse_unknown_bots`,
`pre_place` and every record join therefore work without change.

`rcon_set_game_speed(v)` (new) sets `game.speed`; `rcon_game_speed()` prints
it.

### 4. Core and CLI

- `FactorioParams` gains `character_bots: u8` (0 = none) and
  `game_speed: f64` (1.0). `FactorioInstance::start`: when `character_bots >
  0`, `client_count` must be 0 (refused otherwise, before any process is
  spawned); after the server is ready it calls `spawn_bots`, then the existing
  connect watcher waits for `character_bots` players in `rcon_players` (they
  appear at once; the watcher is reused for its logging and stall handling).
  Then `set_game_speed` when it is not 1.0. Character bots need no `whoami`
  and no window arrangement.
- `lua` and `start` commands gain `--headless` (bool: "bots are server-side
  characters; no graphical client; implies --clients 0") and `--game-speed
  <f64>` (default 1). `--headless` with `--clients > 0` is a usage error.
  `--connect` and `--server` ignore both with the same loud warning `--seed`
  gets. In `lua`, headless mode does **not** call
  `initiate_missing_players_with_default_inventory`: the bots are real.
- `FactorioRcon` learns the speed it set (`game_speed: RwLock<f64>`), and
  `ACTION_RESULT_DEADLINE` and `craft_deadline` are divided by it where they
  are applied (`sleep_for_action_result_until`), with a floor so a speed
  below 1 lengthens them. All `#[cfg(test)]` modules in `rcon.rs` stay at the
  end of the file.
- `RconActuator::game_speed` overrides the stub by asking `rcon_game_speed`
  once and caching; `ticks_to_wall_clock` then converts correctly.
- `record.start({video = true})` refuses in headless mode with "video is
  filmed from a graphical client; this run has none".
- Provenance gains `bot_mode: Option<String>` (`"clients"` | `"characters"`)
  and `game_speed: Option<f64>`, both `#[serde(default)]`, no schema bump.
  `--compare` prints a one-line note when the two differ; it does not refuse,
  because play fidelity is the same and the clock is game ticks.
- `--headless` writes `Using bot mode characters (N)` on every run, not gated
  on `silent`, next to the `Using mods directory` line.

### 5. Testing

- Rust unit tests: deadline scaling (`craft_deadline` and the action deadline
  at speeds 0.5, 1, 10); provenance round-trips with and without the new
  fields; `FactorioParams` refusal of clients + characters; the `lua` CLI's
  count resolution with `--headless`.
- Stub-driven: the existing `botbridge_craft_action.rs` path is untouched,
  which is the point of feeding `settle_crafted_item` the same event shape.
- Live, on an isolated workspace and ports (a second `AppSettings.toml`
  with its own `workspace_path`, `rcon_port`, and a `factorio_port` setting
  added for this purpose, `#[serde(default)]`): `goal_smoke.lua --headless
  --bots 4`, then a `factory_stage*.lua` run at `--game-speed 5`, checked
  with `just analyse` for the same event classes a client run produces:
  walks settle, mines settle with counts, crafts settle, inventory and
  position writeouts arrive. The live check is the acceptance test for the
  mod half, which has no unit tests.

## Constraints from the other workstream (factorio-bot-78)

- Not touched: `crates/planner/src/schedule.rs`, `method/produce.rs`,
  `state.rs`, the Withdraw part of `method/have.rs`, `refresh_buffers`.
- `rcon_players` must carry `resource_reach_distance` as a double, and the
  build and pickup distances as the real record does.
- The crafting poll feeds `settle_crafted_item` with the same event shape.
- `rcon.rs` test modules stay at the end of the file.
- Default ports and `target/debug/factorio-bot` belong to the live run;
  development happens in `.worktrees/headless` with its own target
  directory; live tests use ports 34200+ and RCON 4324+.

## Risks

- A proxy that forwards a member the entity lacks raises inside the RCON reply
  and reads as `Unexpected Response`. Mitigation: the live smoke run exercises
  every RCON verb the executor issues; unknown-member reads are wrapped so the
  message names the member and the bot.
- Character bots at high speed may lag the walker's per-tick steering if the
  server cannot deliver the ticks; the executor already reads the game clock
  for lag waits, and the speed is opt-in.
- A savepoint written in client mode resumed in headless mode has players in
  the save but none connected; `rcon_spawn_bots` treats that as no players
  connected and spawns characters with fresh inventories, which is a
  different world than the savepoint. Provenance records the mode; `--compare`
  notes it.
