# Headless Character Bots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `factorio-bot lua <script> --headless --bots N [--game-speed S]` runs a script against N server-side character bots with no graphical client, honestly and at a chosen game speed, producing the same run record a client run does.

**Architecture:** The mod gains a bot registry and a `bot_handle(id)` proxy so a character entity answers like a `LuaPlayer` at every existing call site; the four player events it cannot raise are replaced by per-tick polling that feeds the same waiters and writeouts. Core gains `character_bots` and `game_speed` on `FactorioParams`, spawns bots over RCON instead of spawning clients, scales its wall-clock deadlines by the speed, and records both facts in provenance through a marker file, the way `resumed_from` already travels.

**Tech Stack:** Factorio 2.1 Lua mod (`mods/BotBridge/control.lua`), Rust workspace (`crates/core`, `crates/executor`, `crates/scripting_lua`, `app/src-tauri` CLI), `tools/run_analysis.py`.

**Spec:** `docs/superpowers/specs/2026-09-05-headless-character-bots-design.md`

## Global Constraints

- Every cargo command runs under `nix develop -c`. Format single files with `rustfmt --edition 2024 <file>`; never `cargo fmt`.
- Work in `.worktrees/headless` (branch `headless-character-bots`) with `CARGO_TARGET_DIR=.worktrees/headless/target`; never write `target/debug/factorio-bot` in the main checkout while a run holds it.
- Commit with explicit paths, never `git add -A`, never `--amend`.
- Do not touch `crates/planner/src/schedule.rs`, `method/produce.rs`, `state.rs`, the Withdraw part of `method/have.rs`, or `refresh_buffers`.
- Every `#[cfg(test)]` module in `crates/core/src/factorio/rcon.rs` stays at the end of that file.
- Live tests use ports 34200+ and RCON 4324+ and their own workspace; the default ports belong to the speedrun session.
- Mixed rosters (clients and characters) are refused by name, in the mod and in core.
- No `rcon.print` inside a mod function the executor calls, except the reply the function is meant to give.

---

### Task 1: Mod — bot registry, `bot_handle`, `each_bot`, spawn, speed

**Files:**
- Modify: `mods/BotBridge/control.lua` (`get_player` ~4863, the per-tick loop ~1123, `writeout_players` ~1554, `on_player_changed_distance` ~3057, `rcon_players` ~3969, `rcon_player_info` ~3910, `sample_bots_body` ~2032, the `game.players[player_id]` reads in `start_walk_waypoints` ~3194, `rcon_place_entity` ~3257, `rcon_can_place_entities` ~3614, `rcon_insert_to_inventory` ~3683, `rcon_remove_from_inventory` ~3744, `rcon_set_recipe` ~3822, `rcon_action_start_crafting` ~4361, `player_total_inventory` ~3015, `on_init` ~290, `on_player_joined_game` ~2676)

**Interfaces:**
- Produces (Lua globals): `bot_handle(id) -> LuaPlayer | proxy | nil`, `each_bot() -> iterator of (id, handle)`, `is_character_bot(id) -> bool`, `rcon_spawn_bots(count)` (prints JSON `{"spawned":[...],"kept":[...]}` or `Error: ...`), `rcon_set_game_speed(v)` (prints the new speed), `rcon_game_speed()` (prints `game.speed`).
- `storage.bots[id] = { entity = LuaEntity, name = "bot-<id>", respawn_at = nil }`.

- [ ] **Step 1: Add the registry and the handle** near `get_player`:

```lua
-- A bot is a connected player or a server-side character entity. The
-- executor addresses both by one small integer; `bot_handle` is the one
-- place that resolves it. The proxy answers the LuaPlayer-only members a
-- character has no equivalent for and forwards everything else to the
-- entity, whose LuaControl surface is what every action in this file uses.
local CHARACTER_PROXY_OWN = {
	connected = function(b) return b.entity ~= nil and b.entity.valid end,
	character = function(b) if b.entity ~= nil and b.entity.valid then return b.entity end end,
	index = function(b) return b.id end,
	name = function(b) return b.name end,
	print = function() return function() end end,
	ticks_to_respawn = function() return nil end,
	controller_type = function() return defines.controllers.character end,
	crafting_queue_size = function(b) return b.entity.crafting_queue_size end,
}

local function character_proxy(id, bot)
	local state = { id = id, name = bot.name, entity = bot.entity }
	return setmetatable({}, {
		__index = function(_, key)
			local own = CHARACTER_PROXY_OWN[key]
			if own ~= nil then return own(state) end
			local ent = state.entity
			if ent == nil or not ent.valid then
				error("bot " .. tostring(id) .. " has no character (entity gone) while reading " .. tostring(key))
			end
			return ent[key]
		end,
		__newindex = function(_, key, value)
			local ent = state.entity
			if ent == nil or not ent.valid then
				error("bot " .. tostring(id) .. " has no character (entity gone) while writing " .. tostring(key))
			end
			ent[key] = value
		end,
	})
end

function is_character_bot(id)
	return storage.bots ~= nil and storage.bots[id] ~= nil
end

function bot_handle(id)
	if is_character_bot(id) then
		return character_proxy(id, storage.bots[id])
	end
	return game.players[id]
end

-- Every bot the run has: connected players first, then character bots.
function each_bot()
	local players = {}
	for idx, player in pairs(game.players) do players[#players + 1] = { idx, player } end
	for id, bot in pairs(storage.bots or {}) do players[#players + 1] = { id, character_proxy(id, bot) } end
	local i = 0
	return function()
		i = i + 1
		local entry = players[i]
		if entry ~= nil then return entry[1], entry[2] end
	end
end
```

- [ ] **Step 2: Route `get_player` through the handle.** Replace its body's `local player = game.players[player_id]` with `local player = bot_handle(player_id)`; keep the connected and character checks (the proxy answers both). In `on_init`, add `storage.bots = {}`; in `rcon_session_reset` leave `storage.bots` alone (a resume keeps its bots).

- [ ] **Step 3: Replace the direct reads.** In each function listed under Files that does `local player = game.players[player_id]` (walk, place, can_place sites, insert, remove, set_recipe, crafting, `player_total_inventory`), use `bot_handle(player_id)`. Replace `for idx, player in pairs(game.players) do` in the per-tick loop, `writeout_players`, `on_player_changed_distance` and `rcon_players` with `for idx, player in each_bot() do`. In `sample_bots_body` replace `pairs(game.connected_players)` with `each_bot()` plus `if player.connected then`.

- [ ] **Step 4: Spawn and speed entry points**, next to `rcon_players`:

```lua
function rcon_spawn_bots(count)
	for _, player in pairs(game.connected_players) do
		rcon.print("Error: cannot spawn character bots: player " .. player.name .. " is connected; a run is all clients or all characters")
		return
	end
	storage.bots = storage.bots or {}
	local surface = game.surfaces[1]
	local force = game.forces["player"]
	local spawned, kept = {}, {}
	for id = 1, count do
		local bot = storage.bots[id]
		if bot ~= nil and bot.entity ~= nil and bot.entity.valid then
			kept[#kept + 1] = id
		else
			local origin = force.get_spawn_position(surface)
			local pos = surface.find_non_colliding_position("character", origin, 32, 0.5) or origin
			local ent = surface.create_entity{ name = "character", position = pos, force = force }
			storage.bots[id] = { entity = ent, name = "bot-" .. id }
			if remote.interfaces["freeplay"] and remote.interfaces["freeplay"]["get_created_items"] then
				for name, n in pairs(remote.call("freeplay", "get_created_items")) do
					ent.insert{ name = name, count = n }
				end
			end
			spawned[#spawned + 1] = id
		end
		if storage.p[id] == nil then storage.p[id] = {} end
		local tick = game.tick
		local handle = bot_handle(id)
		writeout(tick, "on_player_main_inventory_changed", helpers.table_to_json({
			player_id = id, main_inventory = handle.get_main_inventory().get_contents() }))
		writeout_player_position(tick, id, handle)
	end
	on_player_changed_distance({ tick = game.tick })
	rcon.print(helpers.table_to_json({ spawned = spawned, kept = kept }))
end

function rcon_set_game_speed(v)
	game.speed = v
	rcon.print(tostring(game.speed))
end

function rcon_game_speed()
	rcon.print(tostring(game.speed))
end
```

- [ ] **Step 5: Refuse a joining player while character bots exist.** At the top of `on_player_joined_game`:

```lua
	if storage.bots ~= nil and next(storage.bots) ~= nil then
		local player = game.players[event.player_index]
		print("ERROR: player " .. player.name .. " joined a world with character bots; a run is all clients or all characters")
		player.print("This run uses character bots; clients are refused.")
		return
	end
```

- [ ] **Step 6: Live check on the scratch server.** Start a scratch headless server (see the spike layout in the spec: own `config.ini`, own `mods` dir holding a symlink to `mods/BotBridge`, `--port 34200 --rcon-port 4324`), then over RCON:

```
/sc remote.call("botbridge","spawn_bots",2)
/sc remote.call("botbridge","players")
/sc remote.call("botbridge","action_start_walk_waypoints",1,1,{{-38,-43}})
```

Expected: spawn prints `{"spawned":[1,2],"kept":[]}`; players prints two records with `resource_reach_distance: 2.7`; the walk settles (`action_completed` on the server's stdout) and a `on_player_changed_position` writeout follows — the latter only after Task 2.

- [ ] **Step 7: Commit** `mods/BotBridge/control.lua`.

---

### Task 2: Mod — polled completions for character bots

**Files:**
- Modify: `mods/BotBridge/control.lua` (`on_tick` after the per-bot loop, `on_mined_entity` ~1493, the miner's mine-start `rcon_action_start_mining` ~3231, `on_some_entity_deleted` ~2881, `on_player_died` ~2751)

**Interfaces:**
- Consumes: `each_bot`, `is_character_bot`, `bot_handle` (Task 1); `settle_crafted_item(event)`, `action_completed`, `action_failed`, `writeout_player_position`, `craft_actions()`.
- Produces: `poll_character_bot(tick, id, handle)` called once per tick per character bot.

- [ ] **Step 1: Position and inventory polling.** Add, and call from the end of `on_tick` for every `id` in `storage.bots`:

```lua
local function poll_character_bot(tick, id, handle)
	local bot = storage.bots[id]
	if bot.entity == nil or not bot.entity.valid then return end
	local pos = bot.entity.position
	if bot.last_pos == nil or bot.last_pos.x ~= pos.x or bot.last_pos.y ~= pos.y then
		bot.last_pos = { x = pos.x, y = pos.y }
		writeout(tick, "on_player_changed_position", helpers.table_to_json({ player_id = id, position = pos }))
	end
	local contents = bot.entity.get_main_inventory().get_contents()
	local sig = {}
	for _, stack in pairs(contents) do sig[#sig + 1] = stack.name .. ":" .. stack.count .. ":" .. tostring(stack.quality) end
	table.sort(sig)
	local key = table.concat(sig, ",")
	if bot.last_inventory ~= key then
		bot.last_inventory = key
		writeout(tick, "on_player_main_inventory_changed", helpers.table_to_json({ player_id = id, main_inventory = contents }))
		recent_item_additions[id] = {}
	end
	poll_character_crafts(tick, id, handle)
end
```

- [ ] **Step 2: Crafting polling**, feeding the same settle path:

```lua
function poll_character_crafts(tick, id, handle)
	local bot = storage.bots[id]
	local now = {}
	for _, item in pairs(handle.crafting_queue or {}) do
		now[item.recipe] = (now[item.recipe] or 0) + item.count
	end
	local before = bot.last_queue or {}
	for recipe_name, was in pairs(before) do
		local finished = was - (now[recipe_name] or 0)
		if finished > 0 then
			local recipe = handle.force.recipes[recipe_name]
			for _ = 1, finished do
				on_player_crafted_item({ tick = tick, player_index = id, recipe = recipe })
			end
		end
	end
	bot.last_queue = now
end
```

`on_player_crafted_item` already builds the `recent_item_additions` entry and calls `settle_crafted_item(event)`, so the stub-driven craft tests keep pinning one path.

- [ ] **Step 3: Mining completion by inventory delta.** In `rcon_action_start_mining`, when `is_character_bot(player_id)`, store on the mining record the product names and their counts:

```lua
	if is_character_bot(player_id) then
		local inv = player.get_main_inventory()
		local before = {}
		local products = ent.prototype.mineable_properties and ent.prototype.mineable_properties.products or {}
		for _, product in pairs(products) do
			if product.name then before[product.name] = inv.get_item_count(product.name) end
		end
		storage.p[player_id].mining.inventory_before = before
	end
```

In the per-tick miner, **before** the `if ent and ent.valid then` test, add:

```lua
				local m = storage.p[idx].mining
				if m.inventory_before ~= nil then
					local inv = player.get_main_inventory()
					local delivered = 0
					for name, was in pairs(m.inventory_before) do
						local now = inv.get_item_count(name)
						if now > was then delivered = delivered + (now - was); m.inventory_before[name] = now end
					end
					if delivered > 0 then
						m.left = m.left - delivered
						if m.left <= 0 then
							action_completed(event.tick, m.action_id)
							player.mining_state = { mining = false }
							storage.p[idx].mining = nil
						end
					end
				end
```

and guard the rest of the miner with `if storage.p[idx].mining ~= nil then`.

- [ ] **Step 4: Death and respawn.** In `on_some_entity_deleted` (registered for `on_entity_died`), when `event.entity.name == "character"`, find the registry id whose `entity.unit_number` matches and call a new `on_character_bot_died(event, id)` that emits the same `player_died` writeout and fails the same waiters as `on_player_died` (extract the shared tail of `on_player_died` into `fail_bot_actions(idx, tick, why)` and call it from both), then sets `storage.bots[id].respawn_at = event.tick + 600` and `storage.bots[id].entity = nil`. In `poll_character_bot`, when `bot.entity` is nil and `bot.respawn_at <= tick`, create a fresh character at the force spawn, set `bot.entity`, clear `respawn_at`, and emit the `player_respawned` writeout plus `writeout_player_position`.

- [ ] **Step 5: Live check.** On the scratch server from Task 1: dispatch `action_start_mining(2, 1, "iron-ore", <x>, <y>, 3)` after walking bot 1 next to ore; expect `action_completed` after ~363 ticks and an inventory writeout naming `iron-ore: 3`. Dispatch `action_start_crafting(3, 1, "iron-gear-wheel", 2)` after inserting 10 iron plates via `/sc`; expect `action_completed` within ~70 ticks. Kill the character with `/sc storage.bots[1].entity.die()` — wait, `die()` needs a force: `/sc storage.bots[1].entity.die("enemy")`; expect `player_died` on stdout and a `player_respawned` 600 ticks later.

- [ ] **Step 6: Commit** `mods/BotBridge/control.lua`.

---

### Task 3: Core — params, spawn, speed, deadline scaling, marker

**Files:**
- Modify: `crates/core/src/settings.rs` (add `factorio_port: Option<u16>` with `#[serde(default)]`), `crates/core/src/process/process_control.rs` (`FactorioParams`, `start`), `crates/core/src/factorio/rcon.rs` (`spawn_bots`, `set_game_speed`, `game_speed`, deadline scaling), `crates/core/src/record/run_mode.rs` (create), `crates/core/src/record/mod.rs` (export)
- Test: `crates/core/src/record/run_mode.rs` tests, `crates/core/src/factorio/rcon.rs` tests at the end of the file

**Interfaces:**
- Produces: `FactorioParams { character_bots: u8, game_speed: f64, .. }`; `FactorioRcon::spawn_bots(&self, count: u8) -> Result<Vec<u8>>`, `FactorioRcon::set_game_speed(&self, speed: f64) -> Result<()>`, `FactorioRcon::game_speed(&self) -> Result<f64>`, `FactorioRcon::speed_factor(&self) -> f64`; `record::run_mode::{RunMode, BotMode, write_run_mode, read_run_mode, clear_run_mode}` with `RunMode { bot_mode: BotMode, game_speed: f64 }` and `BotMode::{Clients, Characters}`, serialised as `run-mode.json` in the instance dir.

- [ ] **Step 1: Write the failing test for the marker** in `run_mode.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_marker_round_trips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_run_mode(dir.path()).is_none());
        write_run_mode(dir.path(), &RunMode { bot_mode: BotMode::Characters, game_speed: 5.0 }).unwrap();
        let back = read_run_mode(dir.path()).unwrap();
        assert_eq!(back.bot_mode, BotMode::Characters);
        assert_eq!(back.game_speed, 5.0);
        clear_run_mode(dir.path()).unwrap();
        assert!(read_run_mode(dir.path()).is_none());
    }
}
```

- [ ] **Step 2: Implement `run_mode.rs`** mirroring `savepoint::write_resume_marker` / `read_resume_marker` / `clear_resume_marker` (serde_json to `instance_dir/run-mode.json`; `read` returns `None` on a missing file). `BotMode` serialises as `"clients"` / `"characters"`.

- [ ] **Step 3: Params and start.** Add `character_bots: u8` (default 0) and `game_speed: f64` (default 1.0) to `FactorioParams`. In `start`, before any setup: `if params.character_bots > 0 && params.client_count > 0 { return Err(miette!("--headless spawns character bots and cannot also start {} graphical client(s); a run is all clients or all characters", params.client_count)) }`. After the `session_reset` block: write the marker (`BotMode::Characters` if `character_bots > 0` else `Clients`, with `params.game_speed`) — for an attached server (`server_host` set) skip it. Inside the `startup` block, when `character_bots > 0`: call `rcon.spawn_bots(params.character_bots)`, log `Using bot mode characters ({}) ` unconditionally with `info!` (not gated on `silent`), and set `expected_players = character_bots` for the existing connect watcher; skip `whoami` and `arrange_windows`. After the wait, when `params.game_speed != 1.0`: `rcon.set_game_speed(params.game_speed)`.

- [ ] **Step 4: RCON methods** next to `whoami`:

```rust
    pub async fn spawn_bots(&self, count: u8) -> Result<Vec<u8>> {
        let lines = self.remote_call("spawn_bots", vec![count.to_string()]).await?
            .ok_or_else(|| miette!("spawn_bots: no reply"))?;
        let reply = lines.join("");
        if let Some(err) = reply.strip_prefix("Error: ") { return Err(miette!("{err}")); }
        #[derive(serde::Deserialize)] struct Reply { spawned: Vec<u8>, kept: Vec<u8> }
        let r: Reply = serde_json::from_str(&reply).into_diagnostic().wrap_err_with(|| format!("spawn_bots reply: {reply}"))?;
        Ok(r.spawned.into_iter().chain(r.kept).collect())
    }
    pub async fn set_game_speed(&self, speed: f64) -> Result<()> {
        let lines = self.remote_call("set_game_speed", vec![speed.to_string()]).await?;
        let got: f64 = lines.and_then(|l| l.join("").trim().parse().ok()).ok_or_else(|| miette!("set_game_speed: no reply"))?;
        self.speed.store(got.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub async fn game_speed(&self) -> Result<f64> { /* remote_call("game_speed") and parse */ }
    pub fn speed_factor(&self) -> f64 { let s = f64::from_bits(self.speed.load(Ordering::Relaxed)); if s > 0.0 { s } else { 1.0 } }
```

with `speed: Arc<AtomicU64>` initialised to `1.0f64.to_bits()` in the struct and constructor.

- [ ] **Step 5: Deadline scaling test** (at the end of `rcon.rs`, in the last test module):

```rust
    #[test]
    fn deadlines_scale_with_game_speed() {
        assert_eq!(scale_deadline(Duration::from_secs(360), 10.0), Duration::from_secs(36));
        assert_eq!(scale_deadline(Duration::from_secs(360), 1.0), Duration::from_secs(360));
        assert_eq!(scale_deadline(Duration::from_secs(360), 0.5), Duration::from_secs(720));
        assert_eq!(scale_deadline(Duration::from_secs(360), 0.0), Duration::from_secs(360));
    }
```

- [ ] **Step 6: Implement** `fn scale_deadline(base: Duration, speed: f64) -> Duration` (divide by speed when speed > 0, never below 10 s) and apply it in `sleep_for_action_result` (`scale_deadline(ACTION_RESULT_DEADLINE, self.speed_factor())`) and where `craft_deadline` is used.

- [ ] **Step 7: Run** `nix develop -c cargo test -p factorio-bot-core run_mode deadlines` — expect pass. **Commit** the four files.

---

### Task 4: Executor — read the real game speed

**Files:**
- Modify: `crates/executor/src/rcon_actuator.rs` (override `game_speed`), `crates/executor/src/actuator.rs:221-250` (doc: the stub is now the mock default only)

- [ ] **Step 1:** In `impl Actuator for RconActuator`, add:

```rust
    async fn game_speed(&self) -> Result<f64, ActuatorError> {
        self.rcon.game_speed().await.map_err(|e| ActuatorError::Rejected(e.to_string()))
    }
```

- [ ] **Step 2:** Update the doc comment on the trait default to say the RCON actuator overrides it. Run `nix develop -c cargo test -p factorio-bot-executor` — expect pass. **Commit.**

---

### Task 5: Provenance, `record.start`, analysis policy

**Files:**
- Modify: `crates/core/src/record/provenance.rs` (fields), `crates/scripting_lua/src/globals/record.rs:1100-1115` (fill from `read_run_mode`) and `:1185-1210` (video refusal), `tools/run_analysis.py:121-150` (policy), the OpenAPI snapshot if `Provenance` is published (`UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi`), `app/src/api/types.ts` if the contract test fails.
- Test: `crates/core/src/record/provenance.rs` tests

- [ ] **Step 1: Failing test:** an archived `provenance.json` without the new fields deserialises with `bot_mode == None` and `game_speed == None`; one with `"bot_mode":"characters","game_speed":5.0` round-trips.

- [ ] **Step 2:** Add `#[serde(default)] pub bot_mode: Option<String>` and `#[serde(default)] pub game_speed: Option<f64>`; set them in `record.rs` from `run_mode::read_run_mode(&instance)`. In `record.rs`, before `VideoRecorder::start`, if the run mode is `Characters` return `record_error("video is filmed from a graphical client; this run has none (--headless)")`.

- [ ] **Step 3:** In `tools/run_analysis.py` add `"bot_mode": "note"` and `"game_speed": "note"` to the compare policy so a difference prints one line and does not refuse.

- [ ] **Step 4:** Run the provenance tests, the openapi snapshot test, and `cd app && pnpm run test:coverage` if the snapshot changed. **Commit.**

---

### Task 6: CLI — `--headless` and `--game-speed`

**Files:**
- Modify: `app/src-tauri/src/cli/lua.rs` (args ~110-140, `resolve_counts` ~183-195, `run` ~260-315), `app/src-tauri/src/cli/start.rs` (same two args)
- Test: `resolve_counts` unit tests in `lua.rs`

- [ ] **Step 1: Failing test:** `resolve_counts` on `--headless --bots 4` yields `(0 clients, 4 bots, headless = true)`; `--headless --clients 2` is a usage error naming both flags.

- [ ] **Step 2:** Add `Arg::new("headless").long("headless").action(ArgAction::SetTrue).help("bots are server-side characters; no graphical client (implies --clients 0)")` and `Arg::new("game-speed").long("game-speed").default_value("1").value_parser(value_parser!(f64)).help("run the world at this game.speed; wall-clock deadlines scale with it")`. Change `resolve_counts` to return a `RunCounts { clients, bots, headless }`, refusing `headless && clients > 0` and defaulting `bots` to 1 when headless. In `run`: `character_bots: if headless { bots } else { 0 }`, `game_speed`, and skip `initiate_missing_players_with_default_inventory` when headless. When `--connect` or `--server` is used with either flag, `warn!` that it is ignored, in the same style as the `--seed` warning.

- [ ] **Step 3:** Same two args on `start`. Run `nix develop -c cargo test -p factorio-bot --no-default-features --features cli,lua`. **Commit.**

---

### Task 7: Live acceptance and docs

**Files:**
- Create: `docs/superpowers/notes/2026-09-05-headless-first-run.md`
- Modify: `CLAUDE.md` (a "Headless character bots" subsection under Multi-Client Testing), `justfile` (a `headless` recipe: `cargo run ... lua {{script}} --headless --bots 4 --game-speed 5`)

- [ ] **Step 1:** Build in the worktree: `CARGO_TARGET_DIR=.worktrees/headless/target nix develop -c cargo build --no-default-features --features cli,lua`.

- [ ] **Step 2:** Write `scratchpad/headless-settings.toml` with `workspace_path = ".worktrees/headless/workspace-headless"` (absolute), `rcon_port = 4324`, `factorio_port = 34200`, the same archive path as the main settings. Run `goal_smoke.lua --headless --bots 4 --settings <that file>`; expect `Using bot mode characters (4)`, four players in the roster, and the script's goals settling.

- [ ] **Step 3:** Run `factory_stage2.lua --headless --bots 4 --game-speed 5 --settings <that file> --seed 31337 --new` with `timeout 1200`; then `just analyse` on the run. Expect the same event classes as a client run (walk, mine with counts, craft, inventory, position), provenance `bot_mode: characters`, `game_speed: 5`, and a wall clock well under the 20-minute client run. Record the numbers in the note, including anything that did not work.

- [ ] **Step 4:** Update `CLAUDE.md` and the justfile; commit; merge `headless-character-bots` into master with `git merge --no-ff` from the main checkout after confirming `git status` there is clean of others' uncommitted edits in the touched files.
