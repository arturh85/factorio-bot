-- This lua stub is dual-licensed under both the
-- terms of the MIT and the GPL3 license. You can choose which
-- of those licenses you want to use. Note that the MIT option
-- only applies to this file, and not to the rest of
-- factorio-bot (unless stated otherwise).

-- Copyright (c) 2020       Artur Hallmann
-- Copyright (c) 2017, 2018 Florian Jung
--
-- This file is part of factorio-bot.
--
-- factorio-bot is free software: you can redistribute it and/or
-- modify it under the terms of the GNU General Public License,
-- version 3, as published by the Free Software Foundation.
--
-- factorio-bot is distributed in the hope that it will be useful,
-- but WITHOUT ANY WARRANTY; without even the implied warranty of
-- MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
-- GNU General Public License for more details.
--
-- You should have received a copy of the GNU General Public License
-- along with factorio-bot. If not, see <http://www.gnu.org/licenses/>.


-- MIT License
--
-- Copyright (c) 2020       Artur Hallmann
-- Copyright (c) 2017, 2018 Florian Jung
--
-- Permission is hereby granted, free of charge, to any person obtaining a
-- copy of this factorio lua stub and associated
-- documentation files (the "Software"), to deal in the Software without
-- restriction, including without limitation the rights to use, copy, modify,
-- merge, publish, distribute, sublicense, and/or sell copies of the
-- Software, and to permit persons to whom the Software is furnished to do
-- so, subject to the following conditions:
-- 
-- The above copyright notice and this permission notice shall be included in
-- all copies or substantial portions of the Software.
-- 
-- THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
-- IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
-- FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
-- THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
-- LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
-- FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
-- DEALINGS IN THE SOFTWARE.


require "util"
require "types"

local my_client_id = nil

-- Per-peer state, deliberately NOT in `storage`: writing it there would
-- replicate it and desync. Nothing that must be identical across peers may
-- live here.
--
-- **Initialised at load, not on the first tick.** It used to be `nil` until
-- `on_tick` built it, which made every other reader -- `on_player_joined_game`,
-- `on_chunk_generated`, `rcon_whoami` -- depend on a tick having run first,
-- with nothing saying so. A player joining at tick 0 (the host's own player
-- does, under `--host`) hit `attempt to index upvalue 'client_local_data'` and
-- the mod went to `Failed`, non-recoverable, with a message naming a nil
-- upvalue rather than the ordering that caused it. See
-- `docs/superpowers/notes/2026-09-02-graphical-host-probe.md`.
--
-- Initialising rather than guarding each reader, because the value has a
-- natural empty form and this removes the ordering dependency instead of
-- tolerating it -- a guard would leave the next handler added to this file with
-- the same trap, and a guard that *skipped* would silently drop whatever the
-- pre-tick call was carrying (`rcon_whoami`'s identity, a counted join).
--
-- Not a desync risk: this is the same table every peer would have built on its
-- own first tick, built at the same point in every peer's load.
local client_local_data = { whoami = nil } -- DO NOT USE for anything replicated
local last_tick = 0

local wait_for_player = false
local todo_next_tick = {}
local todo_next_tick_other = {}
-- The crafts in flight used to live here, in a module local. They live in
-- `storage.craft_actions` now -- see `craft_actions()`.
local recent_item_additions   = {} -- recent_item_additions[character_index].{tick,itemlist,recipe?,action_id?}, itemlist = { {"foo",2}, {"bar",17} }

local tile_chunks = {}

function inventory_type_name(invtype, enttype)
	local burner = {
		[defines.inventory.fuel] = "fuel",
		[defines.inventory.burnt_result] = "burnt_result"
	}

	local chest = {
		[defines.inventory.chest] = "chest"
	}

	local furnace = {
		[defines.inventory.furnace_source] = "furnace_source",
		[defines.inventory.furnace_result] = "furnace_result",
		[defines.inventory.furnace_modules] = "furnace_modules"
	}

	local player = {
		[defines.inventory.character_main] = "character_main",
		[defines.inventory.character_guns] = "character_guns",
		[defines.inventory.character_ammo] = "character_ammo",
		[defines.inventory.character_armor] = "character_armor",
		[defines.inventory.character_vehicle] = "character_vehicle",
		[defines.inventory.character_trash] = "character_trash"
	}

	local god = {
		[defines.inventory.god_main] = "god_main"
	}

	local roboport = {
		[defines.inventory.roboport_robot] = "roboport_robot",
		[defines.inventory.roboport_material] = "roboport_material"
	}

	local robot = {
		[defines.inventory.robot_cargo] = "robot_cargo",
		[defines.inventory.robot_repair] = "robot_repair"
	}

	local machine = {
		[defines.inventory.assembling_machine_input] = "assembling_machine_input",
		[defines.inventory.assembling_machine_output] = "assembling_machine_output",
		[defines.inventory.assembling_machine_modules] = "assembling_machine_modules"
	}

	local lab = {
		[defines.inventory.lab_input] = "lab_input",
		[defines.inventory.lab_modules] = "lab_modules"
	}

	local mining_drill = {
		[defines.inventory.mining_drill_modules] = "mining_drill_modules"
	}

	local item = {
		[defines.inventory.item_main] = "item_main"
	}

	local silo = {
		[defines.inventory.rocket] = "rocket",
		[defines.inventory.rocket_silo_rocket] = "rocket_silo_rocket",
		[defines.inventory.rocket_silo_result] = "rocket_silo_result"
	}

	local car = {
		[defines.inventory.car_trunk] = "car_trunk",
		[defines.inventory.car_ammo] = "car_ammo"
	}

	local wagon = {
		[defines.inventory.cargo_wagon] = "cargo_wagon"
	}

	local turret = {
		[defines.inventory.turret_ammo] = "turret_ammo"
	}

	local beacon = {
		[defines.inventory.beacon_modules] = "beacon_modules"
	}

	local corpse = {
		[defines.inventory.character_corpse] = "character_corpse"
	}

	local map = {
		["player"] = {player},
		["container"] = {chest},
		["locomotive"] = {burner, car},
		["car"] = {burner, car},
		["wagon"] = {wagon, car},
		["robot"] = {robot},
		["roboport"] = {roboport},
		["boiler"] = {burner},
		["reactor"] = {burner},
		["drill"] = {burner,mining_drill,machine},
		["machine"] = {machine,burner},
		["furnace"] = {furnace,burner},
		["lab"] = {lab,burner},
		["silo"] = {silo},
		--["radar"] = {}, -- TODO FIXME
		["turret"] = {turret},
		["?"] = {machine, burner, player}
	}

	local lastpart = enttype:match(".*-\\([^-]*\\)")
	if lastpart == nil then lastpart = enttype end

	local mymap = map[lastpart]
	if mymap == nil then
		mymap = map["?"]
	else
		local offset = #mymap
		for i,x in ipairs(map["?"]) do
			mymap[offset+i] = x
		end
	end
	for _,m in ipairs(mymap) do
		if m[invtype] ~= nil then
			return m[invtype]
		end
	end
end

function complain(text)
	rcon.print(text)
	print(text)
	game.forces["player"].print(text)
end

-- TODO: use simplify_amount
function products_to_dict(products) -- input: array of products, output: dict["item"] = amount
	if products == nil then return nil end

	local result = {}
	for _,product in ipairs(products) do
		if product.amount then
			result[product.name] = (result[product.name] or 0) + product.amount
		elseif product.amount_min then
			result[product.name] = (result[product.name] or 0) + product.amount_min
		end
	end
	return result
end

-- Like products_to_dict, but for a real `LuaInventory` -- `on_player_mined_entity`'s
-- `buffer` -- rather than a prototype's `mineable_properties.products`. The
-- prototype lists what mining *usually* yields; the buffer is what THIS
-- mining action actually put there, including any productivity/quality bonus
-- the prototype knows nothing about. `get_contents()` already aggregates by
-- item name (across qualities), matching products_to_dict's own shape.
function inventory_to_dict(inventory) -- input: LuaInventory, output: dict["item"] = amount
	if inventory == nil then return nil end

	local result = {}
	for _, stack in ipairs(inventory.get_contents()) do
		result[stack.name] = (result[stack.name] or 0) + stack.count
	end
	return result
end

-- Turn freeplay's crash site off, before the first player exists.
--
-- Freeplay's `on_player_created` runs one block for the first player only,
-- and it does three things this project does not want: it creates the crash
-- site, it takes that player's `iron-plate 8` and `firearm-magazine 8` back
-- out of its inventory into the debris, and it puts that player into a
-- 750-tick cutscene. During a cutscene `LuaPlayer::character` is nil, and
-- `rcon_players()` filters on `player.connected and player.character` -- so
-- for 12.5 seconds after joining, **player 1 does not exist** as far as any
-- caller of `rcon.players()` is concerned, and only ever player 1. Four of
-- the nineteen archived runs with samples froze a roster inside that window;
-- run 30 planned the whole run for `[2]` and left three bots idle for 162,158
-- ticks.
--
-- The inventory half matters just as much and for longer. A first player that
-- has had its iron plates removed sits at `(0, 0)` holding
-- `{burner-mining-drill, stone-furnace, wood}`, which is *exactly* the
-- invented phantom bot `initiate_missing_players_with_default_inventory`
-- seeds -- neither position nor inventory can tell a healthy bot 1 from a
-- phantom, and that ambiguity has already produced one wrong diagnosis.
-- Leaving the plates where they are makes bot 1 look like every other bot.
--
-- `on_init` is the right moment: it runs when the map is created, tens of
-- seconds before any client connects, and freeplay asks for exactly this in
-- as many words -- "This is so that other mods and scripts have a chance to
-- do remote calls before we do things like charting the starting area,
-- creating the crash site". It is best effort: a scenario that is not
-- freeplay has no such interface and `remote.call` would raise, so the
-- interface is checked first and the outcome printed rather than assumed.
-- The cutscene exit in `on_player_joined_game` is what covers the case this
-- cannot reach -- a save that already existed when this landed.
function disable_crashsite()
	if remote.interfaces["freeplay"] == nil then
		print("no freeplay interface; leaving the crash site alone")
		return false
	end
	remote.call("freeplay", "set_disable_crashsite", true)
	print("crash site disabled: no cutscene, and player 1 keeps its iron plates")
	return true
end

function on_init()
	print("on_init!")
	disable_crashsite()
	storage.resources = {}
	storage.resources.last_index = 0
	storage.resources.list = {} -- might be sparse, so the #-operator won't work
	storage.resources.map = {}
	storage.map_area = {x1=0, y1=0, x2=0, y2=0} -- a bounding box of all charted map chunks
	storage.p = {} -- player-private data
	storage.bots = {} -- character bots, see `bot_handle`
	storage.pathfinding = {}
	storage.pathfinding.map = {}
	storage.n_clients = 1
	-- AND the module local, because `on_load` will NOT run in this session.
	--
	-- The two are mutually exclusive by design. Quoting the shipped 2.1.17 API
	-- docs (`LuaBootstrap::on_load`): "This is only called for mods that have
	-- been part of the save previously". A save created without BotBridge --
	-- somebody else's save, a world record, anything downloaded -- takes
	-- `on_init` instead, and `my_client_id` used to be assigned in `on_load`
	-- alone.
	--
	-- That is not cosmetic. `my_client_id` is what gates the readiness beacon
	-- in `on_tick`, and that beacon is the ONLY thing the host waits for
	-- before it opens RCON: `read_output` in
	-- `crates/core/src/process/output_reader.rs` blocks on a stdout line
	-- containing `my_client_id`, and only afterwards calls
	-- `initialize_server` -> `whoami("server")` -> `on_whoami`, which is what
	-- builds the initial-discovery replay. No beacon, no RCON, no discovery,
	-- no world -- and no error either: the host simply sits at
	-- `start waiting` for ever. It did, for 900 seconds, against a 6:39:53
	-- world-record save while Factorio itself hosted the map perfectly
	-- happily. See crates/core/tests/botbridge_ready_beacon.rs.
	--
	-- Every run this project has ever measured took the `on_load` path, which
	-- is why this survived: our own map is created by a separate `--create`
	-- invocation and then loaded by the server process, so by then the mod is
	-- already part of the save.
	my_client_id = storage.n_clients
end

function pos_str(pos)
	if #pos ~= 2 then
		return pos.x .. "," .. pos.y
	else
		return pos[1] .. "," .. pos[2]
	end
end

function aabb_str(aabb)
	return pos_str(aabb.left_top) .. ";" .. pos_str(aabb.right_bottom)
end

function distance(a,b)
	local x1
	local y1
	local x2
	local y2

	if a.x ~= nil then x1 = a.x else x1=a[1] end
	if a.y ~= nil then y1 = a.y else y1=a[2] end
	if b.x ~= nil then x2 = b.x else x2=b[1] end
	if b.y ~= nil then y2 = b.y else y2=b[2] end

	return math.sqrt((x1-x2)*(x1-x2) + (y1-y2)*(y1-y2))
end

-- How many consecutive ticks a leg may go without getting any closer to its
-- waypoint before the walk is failed as stalled.
--
-- **This is a progress clock, not a leg timeout.** It used to be
-- `walk_leg_timeout_ticks`: 3x the straight-line time of the leg, floored at
-- 60, counted from the tick the leg began -- and the message it produced said
-- `made no progress` while measuring nothing of the kind. Run 9
-- (`run-1788576604-65414`) is the case that retired it: bot 1 walked 1.04 of
-- a 1.42-tile leg in 7 ticks, the game then held the character still for 54
-- ticks on open dirt with nothing within twelve tiles of it, and the clock --
-- which had been running since the leg began -- reported "no progress for 61
-- ticks" from a position that was clearly not where the leg started. A
-- reader took `from` for the leg's origin and the leg for 0.38 tiles long.
--
-- Measured as progress instead, the clock restarts on every tick the distance
-- to the waypoint shrinks, so a leg walked slowly never trips it -- and a
-- lagging game, where a script-walked character advances only on some ticks,
-- is exactly when legs are walked slowly. The old scaling by leg length
-- existed only to tolerate slow legs and is not needed by a clock that
-- tolerates them by construction. What is left is the one question the check
-- was always meant to ask: has the character stopped getting closer?
--
-- 60 ticks is a second of standing still, the floor the old timeout had for
-- short legs. A wedged character is reported a second after it stops; a
-- character the game holds still for longer than that is reported as exactly
-- that, with `moved <d> tiles of a <len>-tile leg` and the steering the
-- follower was applying, so the record can tell the two apart.
WALK_STALL_TICKS = 60

-- The tick a leg last got closer to its waypoint, or the tick it began if it
-- never has. Stored on the walk as `progress_tick` / `best_dist`; a walk
-- restored from a save written by a build without those fields simply counts
-- from `idx_tick`, which is what the old timeout did.
function walk_leg_progress_tick(w)
	return w.progress_tick or w.idx_tick
end

-- Records this tick's distance to the waypoint. The first measurement of a
-- leg only sets the baseline -- it is not progress, or every leg would buy
-- itself a free second on its first tick. Every later tick that gets closer
-- than the best so far restarts the clock.
function walk_leg_note_distance(w, tick, dist)
	if w.best_dist == nil then
		w.best_dist = dist
	elseif dist < w.best_dist then
		w.best_dist = dist
		w.progress_tick = tick
	end
end

-- A copy of where a character stands as a leg begins, or nil if it has no
-- character to ask. Copied rather than referenced: `player.character.position`
-- is read fresh every time, so holding the table would hold the *current*
-- position and measure a distance of zero from itself.
function walk_leg_origin(player)
	local character = player.character
	if character == nil then return nil end
	local pos = character.position
	return { x = pos.x, y = pos.y }
end

-- How far ahead of a stalled character to look for whatever it is pressed
-- against, and how wide a box to look in.
--
-- The follower steers one axis-aligned step at a time, so at the instant a leg
-- gives up the character is leaning into the *next* tile, not standing in it.
-- 0.75 tiles ahead of `player.character.position` is past a character's own
-- 0.2-tile half-width and into the thing it cannot get past; the 0.35 half-box
-- is a little wider than the character so a blocker it is pressed against
-- corner-first is still inside it.
--
-- These are deliberately small. A generous box would name a furnace two tiles
-- to the side and read exactly like a furnace in the way, which is worse than
-- saying nothing -- see `walk_stall_cause` for why "nothing findable" is a
-- real answer here rather than a failure to look hard enough.
WALK_STALL_PROBE_AHEAD = 0.75
WALK_STALL_PROBE_HALF = 0.35

-- Entity types that exist on the ground and are **not** in a character's way.
--
-- The trap this list exists for: `find_entities_filtered{area=...}` returns
-- resources, and a bot stuck on an ore patch is standing in a solid block of
-- them. Reporting `iron-ore` as the blocker would be a confident wrong answer
-- on the single most common terrain a bot walks over -- resources collide on
-- the resource layer only, which is why a character walks straight through
-- them. Items on the ground, corpses, ghosts and flying robots are the same
-- shape of mistake.
--
-- This is the fallible half of the probe: a type absent from this list and
-- absent from a character's collision layers would be named as a blocker when
-- it is not. `walk_stall_collides` below covers the general case (a prototype
-- with no collision box at all cannot block anything); the list covers the
-- ones that *have* a box and still do not collide with a character.
WALK_STALL_PASSABLE_TYPES = {
	["resource"] = true,
	["item-entity"] = true,
	["corpse"] = true,
	["character-corpse"] = true,
	["entity-ghost"] = true,
	["tile-ghost"] = true,
	["item-request-proxy"] = true,
	["logistic-robot"] = true,
	["construction-robot"] = true,
	["combat-robot"] = true,
	["smoke"] = true,
	["smoke-with-trigger"] = true,
	["particle-source"] = true,
	["flying-text"] = true,
	["highlight-box"] = true,
	["arrow"] = true,
	["explosion"] = true,
	["projectile"] = true,
	["beam"] = true,
	["sticker"] = true,
	["stream"] = true,
	["speech-bubble"] = true,
	["rocket-silo-rocket"] = true,
	["rocket-silo-rocket-shadow"] = true,
}

-- Whether an entity has a footprint at all.
--
-- The general half of the passability test: a prototype whose collision box is
-- empty in either axis occupies no space and can block nothing, whatever its
-- type is. Catches the types `WALK_STALL_PASSABLE_TYPES` has never heard of,
-- which is most of what a modded game would put on the ground.
function walk_stall_collides(entity)
	local box = entity.prototype.collision_box
	if box == nil then return false end
	return (box.right_bottom.x - box.left_top.x) > 0
		and (box.right_bottom.y - box.left_top.y) > 0
end

-- Which of several things found in the probe box is the one to name.
--
-- A character first, and not because it is the likeliest -- it is the one
-- blocker that **moves on its own**, so it is the only one whose presence
-- changes what the caller should do next. A run lost a nine-minute plan to a
-- bot parked in a footprint while mining, and the record it left named the
-- ground. Everything below it is ranked by how specific an answer it is: a
-- cliff and a built entity are facts about that tile, a rock or a tree is
-- scenery that anything with a pickaxe can clear.
function walk_stall_rank(entity)
	local t = entity.type
	if t == "character" then return 1 end
	if t == "cliff" then return 2 end
	if t == "tree" then return 5 end
	if t == "simple-entity" then return 4 end
	return 3
end

-- What to call one blocker, in the wording `crates/core/src/factorio/rcon.rs`
-- parses (`walk_blocker`). The leading word is the class and everything up to
-- ` on tile '` is the detail; both sides are pinned by
-- `a_stalls_cause_is_read_from_the_mods_own_wording` in that file, which runs
-- THIS function against a stub game and parses what it really produced --
-- rather than asserting that some substring appears in this file, which cannot
-- see a word that moved to a different place in the sentence.
function walk_stall_describe(entity, acting_player)
	local t = entity.type
	if t == "character" then
		-- `LuaEntity.player` is nil for a character nobody is driving, and
		-- nil for every character bot -- see `bot_of_character`. A character
		-- neither a player nor the registry claims is still solid: a
		-- disconnected bot leaves its character standing exactly where it
		-- was.
		local blocker_id = bot_of_character(entity)
		if blocker_id == nil then return "character (no player)" end
		-- **What it is doing is the point of naming it.** `step_aside_from_footprint`
		-- steers only a blocker that is neither walking nor mining, so a
		-- blocker reported `(mining)` is one nothing is going to move, and a
		-- caller waiting for it to wander off is waiting for nothing.
		local state = storage.p[blocker_id]
		local doing = "idle"
		if state ~= nil then
			if state.walking ~= nil then
				doing = "walking"
			elseif state.mining ~= nil then
				doing = "mining"
			end
		end
		return "character #" .. blocker_id .. " (" .. doing .. ")"
	elseif t == "tree" then
		return "tree '" .. entity.name .. "'"
	elseif t == "simple-entity" then
		-- Rocks are the only `simple-entity` vanilla puts on a map with a
		-- collision box, and they are what this always is in practice.
		return "rock '" .. entity.name .. "'"
	elseif t == "cliff" then
		return "cliff '" .. entity.name .. "'"
	end
	-- **"did we build it" is a different question from "what is it".** A
	-- stone-furnace across the route is our own plan contradicting itself; a
	-- biter nest is the map. Force names are compared rather than force
	-- objects because a stub and a live game agree on the former only.
	local ours = "theirs"
	local mine = acting_player.force
	if type(mine) == "table" then mine = mine.name end
	local theirs = entity.force
	if type(theirs) == "table" then theirs = theirs.name end
	if mine ~= nil and mine == theirs then ours = "ours" end
	return "entity '" .. entity.name .. "' (" .. ours .. ")"
end

-- **What was in the way**, asked of the game at the instant a leg gives up.
--
-- Before this existed a stalled walk reported where the bot was and where it
-- was going and nothing about why it stopped, so the cause was never
-- established: the Rust side asks for a fresh path, the fresh path usually
-- works, and the same class of stall comes back next run. A placement
-- collision that took a whole run plus a reconstruction from sampled positions
-- to pin down was a bot parked in a footprint *while mining*; one line here
-- would have said so at the time.
--
-- Returns the clause appended to `w.stuck`, in this grammar:
--
--     blocked at (<x>/<y>) by <class> <detail> on tile '<name>'
--
-- appended to the stall wording after `, moved <d> tiles, ` -- after both
-- coordinates, so `walk_reports_stalled_leg` still fires and `walk_endpoints`
-- (crates/scripting_lua/src/globals/record.rs) still reads `from` and
-- `destination` out of the same string.
--
-- `on tile` is present whenever the game answered `get_tile`, blocker or not,
-- because the ground is the answer when nothing is standing on it -- a leg
-- steered into water finds no entity at all. `(+N more)` follows when the box
-- held more than the one named.
--
-- **"nothing findable" is a real answer and must stay one.** It says the probe
-- looked and the tile ahead is clear, which points at the pathfinder or at a
-- character that had already moved -- a different bug from any of the ones
-- above it. Widening the box until something is always found would turn that
-- answer into a plausible wrong one.
function walk_stall_cause(player, pos, dest, dx, dy)
	-- The follower reduced dx/dy to signs before this is reached, so they are
	-- the step it was trying to take. Both zero cannot happen with a live
	-- waypoint -- the arrival branch would have claimed it -- but a probe must
	-- not divide by zero over a case it merely believes impossible.
	local sx, sy = dx, dy
	if sx == 0 and sy == 0 then
		sx = dest.x - pos.x
		sy = dest.y - pos.y
	end
	local len = math.sqrt(sx * sx + sy * sy)
	local cx, cy = pos.x, pos.y
	if len > 0 then
		-- Rounded to a thousandth of a tile, and rounded *before* the box is
		-- built so the coordinate in the message is the one that was really
		-- queried. Unlike the positions either side of it in this message,
		-- this one is not observed -- it is `pos` plus a constant along a unit
		-- step -- so a diagonal produces fourteen significant digits of
		-- arithmetic noise in a line somebody has to read. A thousandth of a
		-- tile is four orders of magnitude finer than anything here collides
		-- at.
		cx = math.floor((pos.x + (sx / len) * WALK_STALL_PROBE_AHEAD) * 1000 + 0.5) / 1000
		cy = math.floor((pos.y + (sy / len) * WALK_STALL_PROBE_AHEAD) * 1000 + 0.5) / 1000
	end

	local surface = player.surface
	local half = WALK_STALL_PROBE_HALF
	local area = {
		left_top = { x = cx - half, y = cy - half },
		right_bottom = { x = cx + half, y = cy + half },
	}
	local best = nil
	local best_rank = nil
	local found = 0
	local self_character = player.character
	for _, entity in pairs(surface.find_entities_filtered{ area = area }) do
		if entity.valid ~= false
			and entity ~= self_character
			and not WALK_STALL_PASSABLE_TYPES[entity.type]
			and walk_stall_collides(entity)
		then
			found = found + 1
			local rank = walk_stall_rank(entity)
			if best_rank == nil or rank < best_rank then
				best_rank = rank
				best = entity
			end
		end
	end

	local cause = "nothing findable"
	if best ~= nil then cause = walk_stall_describe(best, player) end
	local msg = "blocked at " .. coord({ x = cx, y = cy }) .. " by " .. cause
	local tile = surface.get_tile(cx, cy)
	if tile ~= nil and tile.valid ~= false and tile.name ~= nil then
		msg = msg .. " on tile '" .. tile.name .. "'"
	end
	if found > 1 then
		msg = msg .. " (+" .. (found - 1) .. " more)"
	end
	return msg
end

--- How far, in whole tiles about its own, a stalled character looks for a
--- tile nothing else stands on before its walk is failed.
---
--- **A stall against a building can leave the character somewhere the
--- pathfinder will not start from.** Bot 6 of `run-1788614781-38058` walked a
--- path the game had computed *before* bot 3 built two furnaces across it
--- (`[-5, -28]` and `[-6, -30]`, ticks 15852-15854; the walk was dispatched
--- at ~15700 from forty tiles away), steered into the 0.6-tile crack between
--- their boxes, and stalled at `(-5.20, -29.10)` -- legal by its collision
--- box, touching the second furnace, on a tile that furnace covers. From
--- there every path request was refused: six replans, the same
--- `failed to path find` from the same coordinates, and the run halted
--- `stuck`. The executor's fresh path (`move_player_timed`,
--- crates/core/src/factorio/rcon.rs) starts from where the character stands,
--- and where it stood was the problem.
---
--- A player in that crack walks out by hand. A character bot has only the
--- follower, so the follower does the one thing a player would: before the
--- stall is reported, if the tile under the character holds anything that
--- collides with a walker, it steers to the nearest tile that holds nothing
--- (this many tiles out at most), and *then* fails the walk with the stall's
--- cause -- the destination was not reached and must not be reported as
--- reached -- so the executor's retry asks the pathfinder from clear ground.
--- Once per walk: a step that itself stalls is reported as the stall it is.
--- No teleport is involved; this is a walk, dispatched by the same follower.
WALK_STEP_CLEAR_RADIUS = 2

-- The centre of the tile `pos` is on.
function tile_centre(pos)
	return { x = math.floor(pos.x) + 0.5, y = math.floor(pos.y) + 0.5 }
end

-- Whether the whole tile centred at `centre` holds nothing a walker collides
-- with (other than `self_character`), and is itself walkable ground. The
-- same judgement `walk_stall_cause` makes about the probe box, made about a
-- tile: resources, ghosts, corpses and items are passable; anything with a
-- collision box is not.
function walk_tile_is_clear(surface, centre, self_character)
	local area = {
		left_top = { x = centre.x - 0.5, y = centre.y - 0.5 },
		right_bottom = { x = centre.x + 0.5, y = centre.y + 0.5 },
	}
	for _, entity in pairs(surface.find_entities_filtered{ area = area }) do
		if entity.valid ~= false
			and entity ~= self_character
			and not WALK_STALL_PASSABLE_TYPES[entity.type]
			and walk_stall_collides(entity)
		then
			return false
		end
	end
	local tile = surface.get_tile(centre.x, centre.y)
	if tile ~= nil and tile.valid ~= false and type(tile.collides_with) == "function"
		and tile.collides_with("player") then
		return false
	end
	return true
end

-- Where a character stalled at `pos` should step to before its walk is
-- failed: nil when the tile it stands on is already clear (the stall is not
-- a pinned character, and there is nothing to step out of), else the nearest
-- clear tile centre within WALK_STEP_CLEAR_RADIUS, nearest to `pos` first and
-- north-west first among equals, or nil when none is.
function walk_step_clear_landing(player, pos)
	local surface = player.surface
	local me = player.character
	local here = tile_centre(pos)
	if walk_tile_is_clear(surface, here, me) then return nil end
	local candidates = {}
	for dy = -WALK_STEP_CLEAR_RADIUS, WALK_STEP_CLEAR_RADIUS do
		for dx = -WALK_STEP_CLEAR_RADIUS, WALK_STEP_CLEAR_RADIUS do
			if dx ~= 0 or dy ~= 0 then
				local c = { x = here.x + dx, y = here.y + dy }
				candidates[#candidates + 1] = { centre = c, dist = distance(pos, c), dy = dy, dx = dx }
			end
		end
	end
	table.sort(candidates, function(a, b)
		if a.dist ~= b.dist then return a.dist < b.dist end
		if a.dy ~= b.dy then return a.dy < b.dy end
		return a.dx < b.dx
	end)
	for _, c in ipairs(candidates) do
		if walk_tile_is_clear(surface, c.centre, me) then return c.centre end
	end
	return nil
end

--- Where a bot standing in a refused footprint is asked to stand instead, and
--- how hard the game is asked to find it somewhere.
---
--- The target is the nearest edge of the footprint the game just judged, plus
--- the character's own half-width, plus this margin. The margin exists because
--- `can_place_entity` and the walker disagree about what "just outside" means:
--- the walker stops anywhere within a 0.3-by-0.3 box of its waypoint, so a
--- target exactly on the boundary is a coin flip about whether the retry finds
--- the footprint clear.
PLACEMENT_STEP_ASIDE_MARGIN = 0.4

--- The search precision handed to `find_non_colliding_position` when placing
--- that target. Half a tile is the coarsest step that can always find the gap
--- beside an occupied position -- a character's collision box is about 0.4
--- tiles across.
PLACEMENT_STEP_ASIDE_PRECISION = 0.5

--- The ladder of radii `placement_step_aside_landing` walks, nearest first.
---
--- A radius of 0 would search forever **[V]**, so none of these may be zero.
---
--- **A dense block runs out of near ground, and that is the whole of defect
--- (A).** The single radius this replaces was 4, sized for "a handful of
--- stacked bots" on open ground. Inside a 179-entity `FurnaceLine` -- 2x2 furnaces
--- packed against belt lanes -- there may be no spot a character fits within
--- four tiles of any exit at all, and `find_non_colliding_position` then
--- answers nil for every edge. The blocker is classified `stuck`, nothing
--- asks it to move, and `FootprintWait::decide`
--- (crates/core/src/factorio/rcon.rs) spends the four step-aside attempts on
--- a situation waiting cannot change: `run-1788663566-25023` refused a
--- transport-belt at `[20.5, 5.5]` four times over 534 ticks with bot 4
--- parked at `(20.02, 5.07)`, which never moved once in the 540 ticks either
--- side of it.
---
--- So the search escalates instead of giving up: still the nearest exit
--- first, still the nearest spot the game will offer, but if no exit has one
--- within four tiles the whole ring is asked again at twelve and then at
--- thirty-two -- far enough to leave a block of this size entirely.
---
--- **The ladder cannot change an answer the old radius already had.**
--- `find_non_colliding_position` searches outward from its target, so a spot
--- found at radius 4 is found at radius 4 whatever follows it in this list,
--- and radius 4 is asked of every exit before radius 12 is asked of any. It
--- can only turn a `stuck` into a longer walk.
---
--- Bounded by construction: three radii times four exits is twelve questions
--- and then `stuck`, which is still the honest answer when the block really
--- has no room in it. Ordered nearest-first, so it is a ladder and not a
--- search.
PLACEMENT_STEP_ASIDE_RADII = { 4, 12, 32 }

--- The action id an internal step-aside walk is dispatched under.
---
--- Outside the executor's own id space, which is `next_action_id % 1000`
--- (`crates/core/src/factorio/rcon.rs`), so an `action_completed` for one of
--- these can never be mistaken for a reply to a dispatched action. 4711 is the
--- same trick, used by the mining step-aside; this is its neighbour rather
--- than the same number so the two are told apart in a log.
PLACEMENT_STEP_ASIDE_ACTION_ID = 4712

--- How long the mining handler may fail to start mining before it gives the
--- action a verdict.
---
--- Three branches of that handler used to `print` and fall through: nothing
--- selectable under the cursor, another character standing on the target, and
--- a selection that is neither the target nor a tree. None of them completed
--- the action and none of them failed it, so the caller waited out its whole
--- `ACTION_RESULT_DEADLINE` (360s, crates/core/src/factorio/rcon.rs) and
--- learned nothing. Run 9 (`workspace/runs/run-1788310810-27811`) spent 1080
--- of its 1096 recorded seconds that way -- three consecutive plans, each 360
--- seconds long, each containing about two seconds of work.
---
--- 300 ticks (5s) is far longer than anything the handler legitimately waits
--- for -- the step-aside walk below is about 1.35 tiles, roughly 9 ticks --
--- and it is not a mining timeout: the clock only runs on ticks where
--- `mining_state` was NOT set, and any tick that sets it clears the clock. A
--- mine that is actually mining, however slowly, is never interrupted.
MINE_BLOCKED_TIMEOUT_TICKS = 300

--- Where to stand when another character is on the tile we mean to mine.
---
--- This used to be a flat `{ent.x - 2, ent.y - 2}`, from 2021, when the reach
--- guard above was a flat `> 6`. The guard was tightened to the real
--- `resource_reach_distance` in dd2852e3, and a character's is **2.7** --
--- while that waypoint is `sqrt(8)` = **2.828** tiles from the entity. So the
--- bot walked exactly where it was told and the guard refused it on the next
--- tick with `too far too mine`: arrived, on target, still out of reach.
---
--- Sized to the reach instead, and by the same rule the Rust side uses for a
--- mine's corrective walk (`approach_radius`, crates/core/src/factorio/rcon.rs):
--- aim at half the bound, so the follower's 0.3-by-0.3 stopping box still
--- leaves the bot comfortably inside it. Half the reach split over two axes is
--- `reach * 0.5 / sqrt(2)` each, giving a straight-line distance of exactly
--- half the reach.
--- Whether `player` may mine `ent` from where it stands, by the game's own
--- rule. `can_reach_entity` measures to the entity's collision box; the
--- centre-distance check it replaces refused a character standing 2.9 tiles
--- from a huge-rock's centre and 1.4 from the rock itself, in every batch of
--- run-1788551693-66583, after the bot had walked exactly where the plan
--- sent it. The Rust side measures the same way (`mining_distance`,
--- crates/core/src/factorio/rcon.rs). A player with no character has no
--- reach at all and the call raises, so the old centre rule stands in.
function mining_target_reachable(player, ent)
	local ok, reachable = pcall(function() return player.can_reach_entity(ent) end)
	if ok and type(reachable) == "boolean" then
		return reachable
	end
	return distance(player.position, ent.position) <= player.resource_reach_distance
end

function mine_step_aside_waypoint(player, ent)
	local reach = player.resource_reach_distance
	if reach == nil or reach <= 0 or reach > 1000 then
		-- A player without a character reports `MAX_DOUBLE` here; there is no
		-- sensible fraction of that, and 1.35 is what a character would get.
		reach = 2.7
	end
	local offset = (reach * 0.5) / math.sqrt(2)
	return { ent.position.x - offset, ent.position.y - offset }
end

-- Reports where a character *is*, under the same key the game's own movement
-- event uses.
--
-- **Factorio raises `on_player_changed_position` once per tile a character
-- crosses, not once per position change.** Read straight off run 27's log
-- (`workspace/runs/run-1788353986-24634`, and `workspace/server-log.txt`):
-- consecutive events for one walking bot are ~1.0 tiles and ~7 ticks apart.
--
-- While a bot is walking that costs nothing -- the next tile is a few ticks
-- away. When it stops it costs everything, because the character carries on up
-- to a tile past the boundary that raised the last event and then raises no
-- more. Bot 3's last event in that run was at tick 18187 naming
-- `(-23.19140625, 16.96484375)`; it came to rest at `(-23.5078125, 16.203125)`
-- and stood there for the remaining 13,000 ticks. Both are inside tile
-- `(-24, 16)`.
--
-- `crates/planner`'s `characters` occupancy source reads exactly that
-- position, and 0.825 tiles is the whole difference between a stone furnace's
-- 1.398-tile footprint clearing a parked bot and covering it. Milestone 6
-- sited a furnace at `[-23, 16]` three times, the game refused it three times,
-- and the planner was reasoning correctly the whole way -- from a fact the
-- world had grown out of.
--
-- So: every point the walker lets a character stop reports where it stopped.
-- That is the moment a resting position becomes a fact, and the only moment
-- anything is in a position to say so.
function writeout_player_position(tick, player_id, player)
	if player == nil or player.character == nil then
		return
	end
	local pos = player.character.position
	writeout(tick, "on_player_changed_position", helpers.table_to_json({
		player_id = player_id,
		position = { x = pos.x, y = pos.y },
	}))
end

-- Emits a machine-readable record of a `player.teleport` call, since neither
-- call site is otherwise distinguishable from ordinary walking to the Rust
-- side: `on_player_changed_position` fires identically for a teleport and a
-- walked step. `action_id` is nil for both, which are the blueprint and ghost
-- sites -- synchronous RCON calls with no action to attach to.
--
-- There used to be a third site, the stuck-walk recovery, and it was the only
-- one that moved a bot the executor had asked to *walk*. It is gone, because
-- teleporting turned an unreachable destination into a reported arrival. A
-- stalled leg now fails the walk promptly and Rust retries it against the live
-- world (`move_player_timed`, crates/core/src/factorio/rcon.rs).
function teleport_writeout(tick, player_id, reason, from, to, action_id)
	writeout(tick, "teleport", helpers.table_to_json({
		player_id = player_id,
		reason = reason,
		from = from,
		to = to,
		distance = distance(from, to),
		action_id = action_id,
	}))
end

function writeout_initial_stuff()
	writeout_pictures()
	writeout_entity_prototypes()
	writeout_item_prototypes()
	writeout_recipes()
	writeout_forces()
	writeout_daylight()
	writeout_surfaces()
	writeout(0, "STATIC_DATA_END", "done")
end

function writeout_proto_picture_dir(name, dir, picspec)
	if picspec.layers ~= nil then
		return writeout_proto_picture_dir(name, dir, picspec.layers[1])
	elseif picspec.hr_version ~= nil then
		return writeout_proto_picture_dir(name, dir, picspec.hr_version)
	elseif #picspec > 0 then
		return writeout_proto_picture_dir(name, dir, picspec[1])
	end
	
	if picspec.filename ~= nil and picspec.width ~= nil and picspec.height ~= nil then
		local shiftx = picspec.shift ~= nil and (picspec.shift[1]) or 0
		local shifty = picspec.shift ~= nil and (picspec.shift[2]) or 0
		local scale = picspec.scale or 1
		local xx = picspec.x or 0
		local yy = picspec.y or 0

		-- this uses "|", "*" and ":" as separators on purpose, because these
		-- may not be used in windows path names, and are thus unlikely to appear
		-- in the image filenames.
		local result = picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..yy..":"..scale
--		print(">>> "..name.."["..dir.."] -> "..result)
		return result
	else
--		print(">>> "..name.."["..dir.."] WTF")
		return nil
	end
end

function writeout_beltproto_picture(name, picspec)
	if picspec == nil then
--		print(">>> " .. name .. "(belt) empty, WTF?")
		return nil
	end
	if picspec.hr_version ~= nil then
		return writeout_beltproto_picture(name, picspec.hr_version)
	end

	if picspec.filename ~= nil and picspec.width ~= nil and picspec.height ~= nil then
		local shiftx = picspec.shift ~= nil and (picspec.shift[1]) or 0
		local shifty = picspec.shift ~= nil and (picspec.shift[2]) or 0
		local scale = picspec.scale or 1
		local frame_count = picspec.frame_count
		local line_length = picspec.line_length or frame_count

		if frame_count % line_length ~= 0 then
--			print(">>> "..name.."(belt) has frame_count which is not a multiple of line_length. can't handle that yet")
			return nil
		end

		local y_offset = picspec.height * (frame_count / line_length)

		-- this uses "|", "*" and ":" as separators on purpose, because these
		-- may not be used in windows path names, and are thus unlikely to appear
		-- in the image filenames.
		-- also, a trailing ">" or "<" indicate mirroring.
		local result = name

		local xx = 0
		local yy = 0
		
		result = result .. "*" .. picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..(yy+1*y_offset)..":"..scale -- north
		result = result .. "*" .. picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..(yy+0*y_offset)..":"..scale -- east
		result = result .. "*" .. picspec.filename..":"..picspec.width..":"..(-picspec.height)..":"..shiftx..":"..shifty..":"..xx..":"..(yy+1*y_offset)..":"..scale -- south
		result = result .. "*" .. picspec.filename..":"..(-picspec.width)..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..(yy+0*y_offset)..":"..scale -- west
		-- negative width or height signifies that the picture must be swapped afterwards
--		print(">>> "..name.."(belt) -> "..result)
		return result
	else
--		print(">>> "..name.."(belt) WTF")
		return nil
	end
end

function writeout_proto_picture(name, picspec)
--	if name == "pipe" then print("WOOP WOOP PIPE") end

	dirsNESW = {"north","east","south","west"}
	dirs_list = {
		dirsNESW,
		{"up","right","down","left"},
		{"straight_vertical","straight_horizontal","straight_vertical","straight_horizontal"}
	}

	local n_dirs, dirs
	for _,dirs_tmp in ipairs(dirs_list) do
		n_dirs = 0
		
		for _,dir in ipairs(dirs_tmp) do
			if picspec[dir] ~= nil then
				n_dirs = n_dirs + 1
			end
		end

		if n_dirs > 0 then
			dirs = dirs_tmp
			break
		end
	end
	
	local result = {}
	local subresult = nil
	if n_dirs > 0 then
		for i,dir in ipairs(dirs) do
			if picspec[dir] ~= nil then
				subresult = writeout_proto_picture_dir(name, (n_dirs == 1) and "any" or dirsNESW[i], picspec[dir])
				if subresult == nil then
					return nil
				else
					table.insert(result, subresult)
				end
			end
		end
	elseif picspec.sheet ~= nil then
--		print("TODO FIXME: cannot handle sheet for "..name.." yet!")
		return nil
	else
		subresult = writeout_proto_picture_dir(name, "any", picspec)
		if subresult == nil then
			return nil
		else
			table.insert(result, subresult)
		end
	end
	
	-- this uses "|", "*" and ":" as separators on purpose, because these
	-- may not be used in windows path names, and are thus unlikely to appear
	-- in the image filenames.
	return name.."*"..table.concat(result, "*")
end

function writeout_pictures()
	-- this is dirty. in data-final-fixes.lua, we wrote out "serpent.dump(data.raw)" into the
	-- order strings of the "DATA_RAW"..i entities. We had to use multiple of those, because
	-- there's a limit of 200 characters per string.

	if prototypes.virtual_signal["DATA_RAW_LEN"] == nil then
		print("Error: no DATA_RAW_LEN virtual-signal prototype?!")
	end

	local n = tonumber(prototypes.virtual_signal["DATA_RAW_LEN"].order)
--	print("n is " .. n)

	local i = 0
	local string = ""

	local step = math.floor(n/20)
	if step <= 0 then step = 1 end
--	print("reading data.raw")
	local strings = {}
	for i = 1,n do
		if (i % step == 0) then print(string.format("%3.0f%%", 100*i/n)) end
		table.insert(strings, prototypes.virtual_signal["DATA_RAW"..i].order)
	end
	string = table.concat(strings,'')
	data = {raw = loadstring(string)()}

	local lines = {}
	
	local group, members, proto, content
	for group,members in pairs(data.raw) do
		if true or (group ~= "recipe" and group ~= "item") then
			for proto, content in pairs(members) do
				local result = nil

				-- special treatment for belts
				if content.type == "transport-belt" and result == nil then
					result = writeout_beltproto_picture(proto, content.animations)
				end
				
				-- normal treatment for anything else
				if result == nil then
--					for k, _ in pairs(content) do
--						print("<<< " .. group .. ": " .. proto .. " -> "..k)
--					end
					for _,child in ipairs({"structure","animation","picture","animations","base_picture","pictures", "horizontal_animation", "vertical_animation", "on_animation", "off_animation"}) do
						if content[child] ~= nil then
--							print(">>> " .. group .. ": " .. tostring(proto) .. " -> ".. tostring(content.type) .. " -> " .. tostring(child) .. " -- " .. tostring(content[child]))
							result = writeout_proto_picture(proto, content[child])
							if result ~= nil then break end
						end
					end
				end
				
				if result ~= nil then
					table.insert(lines, result)
				end
			end
		end
	end
	--game.write_file("data.raw", serpent.block(data.raw))
	
	-- this uses "|", "*" and ":" as separators on purpose, because these
	-- may not be used in windows path names, and are thus unlikely to appear
	-- in the image filenames.
	writeout(0, "graphics", table.concat(lines, "|"))
end

-- The static world data, collected once and shared by both transports.
--
-- These four collectors are the single definition of *what the world is made
-- of*: which prototypes count, which recipes count, and which force we act
-- for. `writeout_*` prints them on stdout for a server this process started;
-- `rcon_world_snapshot` returns the same records over RCON for a server it
-- merely attached to. Neither transport gets to decide the contents, because
-- two copies of a collector drift and the Rust side cannot tell which one it
-- is looking at.

function collect_entity_prototypes()
	local result = {}
	for name, prot in pairs(prototypes.entity) do
		-- DATA_RAW* are the fake prototypes data-final-fixes.lua stuffs the
		-- serialised data.raw into for writeout_pictures; they are not world.
		if string.sub(name, 1, 8) ~= "DATA_RAW" then
			table.insert(result, serialize_entity_prototype(prot))
		end
	end
	return result
end

function collect_item_prototypes()
	local result = {}
	for name, prot in pairs(prototypes.item) do
		table.insert(result, serialize_item_prototype(prot))
	end
	return result
end

-- FIXME: this assumes that there is only one player force
--
-- **All** recipes, not just the enabled ones. Each carries its own `enabled`
-- flag, so the planner can tell the two apart.
--
-- Sending only enabled recipes asked the planner to plan toward a *future*
-- state while showing it only the *current* one: `goal.researched("automation")`
-- expands to "have 10 automation-science-pack", and that recipe is disabled
-- until automation is researched, so the recipe was absent from world data
-- entirely and the goal failed with "no method can satisfy". That broke
-- goal.researched for essentially every technology that unlocks anything.
-- `serialize_technology` sends the matching unlock-recipe effects, so the
-- planner can pair a locked recipe with the technology that unlocks it.
function collect_recipes()
	local result = {}
	for name, rec in pairs(game.forces["player"].recipes) do
		table.insert(result, serialize_recipe(rec))
	end
	return result
end

-- Only the force the bots act for. `game.forces` also holds `enemy` and
-- `neutral`, whose technology tables cost ~120kB each and describe nobody the
-- planner plans for.
function collect_player_force()
	return serialize_force(game.forces["player"])
end

-- The stdout transport's framing: one JSON record per element, "$"-joined,
-- because writeout is line-oriented and a record may not contain a newline.
function writeout_records(key, records)
	local lines = {}
	for _, record in ipairs(records) do
		table.insert(lines, helpers.table_to_json(record))
	end
	writeout(0, key, table.concat(lines, "$"))
end

function writeout_entity_prototypes()
	writeout_records("entity_prototypes", collect_entity_prototypes())
end

function writeout_item_prototypes()
	writeout_records("item_prototypes", collect_item_prototypes())
end

-- Expected number of items a product yields per craft.
-- Factorio 2.1 dropped Product.probability (it is independent_probability
-- times the shared_probability window now) and made amount optional, so every
-- field here has to be treated as possibly nil.
function simplify_amount(prod)
	if prod.amount ~= nil then
		return prod.amount
	end
	local min = prod.amount_min or prod.amount_max or 0
	local max = prod.amount_max or prod.amount_min or 0
	local probability = prod.probability
	if probability == nil then
		local independent = prod.independent_probability or 1
		local shared = prod.shared_probability
		local window = 1
		if shared ~= nil then
			window = shared.max - shared.min
		end
		probability = independent * window
	end
	return (min + max) / 2 * probability
end

function writeout_recipes()
	writeout_records("recipes", collect_recipes())
end
-- **The force the bots act for, and only that one.**
--
-- This used to walk `game.forces`, which also holds `enemy` and `neutral`.
-- All three then reached `FactorioSurface::forces`, where
-- `crates/planner/src/state.rs` picks its acting force with
-- `forces.keys().min()` -- which over that set returns `enemy`. In run 30 that
-- happened at the first `on_research_finished` (tick 26,449) and covered
-- milestones 6 and 7: at tick 53,485 `player` had `automation-science-pack`
-- researched and `enemy` did not, so all five milestone-7 plans re-derived the
-- trigger technology and planned a second lab for a technology the force
-- already had. That milestone burned 85,030 ticks and ended the run `stuck`.
--
-- The other half of that fix -- naming the force instead of sorting for it --
-- is in the planner and is independent of this: every archived run record
-- still contains three forces, so it has to be correct against those anyway.
--
-- Nothing read the other two. The only readers of `FactorioSurface::forces` are
-- the planner and `crates/executor/src/rcon_actuator.rs`, which already names
-- `"player"` and cites this emission as the reason it must not sort. And the
-- RCON transport already did this: `WorldSnapshot::forces` carries exactly
-- `player`, for these reasons. The two transports are meant to agree about
-- shape, so this is the stdout one catching up.
--
-- Also ~287 kB less stdout per research completion -- `enemy` and `neutral`
-- carry ~120 kB technology tables each and describe nobody.
function writeout_forces()
	writeout(0, "force", helpers.table_to_json(collect_player_force()))
end

-- The one surface this project models, for `collect_surface_daylight`.
--
-- `game.surfaces['nauvis']` and not `game.surfaces[1]`: the Nauvis guard in
-- `on_chunk_generated` already names it that way, and a numeric index says
-- "whichever surface was made first", which is a different claim.
function bridge_surface()
	return game.surfaces['nauvis']
end

-- The daylight curve, for both transports. See `serialize_surface_daylight`
-- in `types.lua` for what each field is and why prototypes cannot answer this.
function collect_surface_daylight()
	local surface = bridge_surface()
	if surface == nil then return nil end
	return serialize_surface_daylight(surface)
end

-- **What surfaces EXIST**, which is a different question from what this
-- bridge observes.
--
-- `on_chunk_generated` drops every non-Nauvis chunk and names the surface it
-- dropped, so each refusal is honest; but nothing enumerated `game.surfaces`,
-- so "this save has one surface" and "we never looked" were the same silence.
-- Space Age is enabled in this workspace and defines five planets, so the
-- second surface is one rocket away rather than a future modding decision.
--
-- **Reports what exists; ingests nothing.** The Nauvis guard stays, and
-- `FactorioWorld::insert_surface` still refuses a second surface by name --
-- the game-global fields (recipes, prototypes, forces and their research)
-- live on `FactorioSurface`, so holding two would give a run two copies of
-- the research state.
function collect_surfaces()
    local result = {}
    for _, surface in pairs(game.surfaces) do
        table.insert(result, serialize_surface(surface))
    end
    -- Sorted by index so the order is the game's and not `pairs()`'s. A
    -- census whose order changes between reads is a census two runs cannot be
    -- compared on.
    table.sort(result, function(a, b) return a.index < b.index end)
    return result
end

-- The census on the stdout transport, for a server this process started.
--
-- **This landed in the same commit as its `"surfaces"` arm in
-- `output_parser.rs`, and it had to.** The parser logs
-- `unexpected action: <key>` as an ERROR for any writeout key it has no arm
-- for, so emitting the census first would have put a red line in every run
-- that looks like a defect and is not.
--
-- One line carrying the whole JSON array, not a row per surface: the parser
-- takes the census all-rows-or-none, because a census missing a row
-- under-reports the surfaces a save has while reading as a complete answer.
--
-- Emitted unconditionally -- there is no `nil` case to guard, unlike
-- `writeout_daylight`, because a running game always has at least one surface.
-- So a run where this line is missing means an OLD MOD, never "no surfaces".
function writeout_surfaces()
	writeout(0, "surfaces", helpers.table_to_json(collect_surfaces()))
end

-- The census on demand, for `factorio-bot rcon -s <host> -- ...` and for an
-- attached (`--connect`) session. `rcon_world_snapshot` carries the same list
-- as one of its fields; this is the cheap way to ask the one question, against
-- a long-played save whose prototype tables are megabytes.
--
-- `rcon.print` is safe here for the reason the whole family above is: this is
-- not a function the executor ever calls, so nothing reads its output as an
-- action's result.
function rcon_surfaces()
    rcon.print(helpers.table_to_json(collect_surfaces()))
end

function writeout_daylight()
	local daylight = collect_surface_daylight()
	if daylight == nil then return end
	writeout(0, "daylight", helpers.table_to_json(daylight))
end

function on_whoami()
	if client_local_data.whoami == "server" then
		client_local_data.initial_discovery={}
		client_local_data.initial_discovery.chunks = {}
		for chunk in game.surfaces[1].get_chunks() do
			table.insert(client_local_data.initial_discovery.chunks, chunk)
		end
		client_local_data.initial_discovery.n = #client_local_data.initial_discovery.chunks
		client_local_data.initial_discovery.idx = 1
	end
end

function on_tick(event)
	poll_character_bots(event.tick)
	-- Per tick, and it has to be: a mining drill's output is only visible as
	-- the fall in the `amount` of the tile it is working, and that tile is
	-- destroyed when it empties. On the 300-tick sample beat the fall would be
	-- read across tile changes and be wrong in both directions. Returns
	-- immediately unless a sampling session is open. See `track_mining_drills`.
	track_mining_drills(event.tick)
	-- `client_local_data` used to be built here, and only here. It is
	-- initialised at its declaration now; see the comment there for why. Do
	-- not restore a lazy init: it would read as necessary and put the ordering
	-- dependency back for every handler in this file.
	if client_local_data.initial_discovery then
		local id = client_local_data.initial_discovery
		local maxi = id.idx + 1 -1
		if maxi > id.n then maxi = id.n end

		if id.idx % 50 == 0 then
			print("initial discovery, writing "..id.idx.."/"..id.n.." ("..math.floor(id.idx/id.n*100).."% done)")
		elseif id.idx == id.n then
			print("initial discovery done")
		end
		for i = id.idx, maxi do
			local chunk = id.chunks[i]
			on_chunk_generated({tick=event.tick, area={left_top={x=chunk.x*32, y=chunk.y*32}, right_bottom={x=32*chunk.x+32, y=32*chunk.y+32}}, surface=game.surfaces[1]})
		end

		id.idx = maxi+1
		if id.idx > id.n then client_local_data.initial_discovery = nil end
	end
		
	for idx, player in each_bot() do
		-- if storage.p[idx].walking and player.connected then

--		game.print("player " .. tostring(idx) .. " connected " .. tostring(player.connected) .. " character " .. tostring(player.character))
--		game.print("player " .. tostring(storage.p[idx]))

		if storage.p[idx] == nil then
			storage.p[idx] = {}
		end


		if player.connected and player.character then -- TODO FIXME
--			game.print("player " .. tostring(idx))
			if storage.p[idx].walking then
--				game.print("player WALKING " .. table_to_string(storage.p[idx]))
				local w = storage.p[idx].walking
				local pos = player.character.position
				local dest = w.waypoints[w.idx]

				if dest == nil then
					-- Two different roads lead here, and they are NOT the same
					-- outcome:
					--
					-- 1. A walk dispatched with zero waypoints (the caller
					--    already stood within arrival tolerance, so the path
					--    request came back empty) -- this is `dest == nil` on
					--    the very first tick, w.idx == 1, and it is a genuine
					--    no-op success.
					-- 2. The stuck-abort below, which nils the *last*
					--    remaining waypoint rather than advancing idx past it
					--    (compare the `w.idx > #w.waypoints` branch below,
					--    which is how a walk that actually arrives exits).
					--    That sets `w.stuck`, and this walk did NOT arrive.
					--
					-- Both must clear `walking` and `walking_state` -- before
					-- this fix NEITHER case did, so a zero-waypoint dispatch
					-- re-reported "ok" every tick forever, and a stuck-abort
					-- left the character walking in a straight line forever
					-- while reporting "ok" every tick forever too.
					player.walking_state = {walking=false}
					storage.p[idx].walking = nil
					-- The character has stopped. Say where, before anything
					-- else: see `writeout_player_position` for why nothing
					-- else ever will.
					writeout_player_position(event.tick, idx, player)
					if w.stuck then
						-- `w.stuck` carries the reason as a string. It used to
						-- be a bare `true` with the message hardcoded here,
						-- which reported "aborted before reaching last
						-- waypoint" for the two intermediate-leg aborts as
						-- well, neither of which is that. `storage` outlives a
						-- save, so an in-flight walk from an older version may
						-- still hold `true`; that keeps the old wording.
						local why = "ERROR: stuck while walking, aborted before reaching last waypoint"
						if type(w.stuck) == "string" then why = w.stuck end
						action_failed(event.tick, w.action_id, why)
					else
						action_completed(event.tick, w.action_id)
					end
				else
					local dx = dest.x - pos.x
					local dy = dest.y - pos.y

					-- A leg is done the moment the character is inside its
					-- 0.3-by-0.3 box, on whichever tick that is -- including
					-- the first tick of the walk, where the pathfinder's first
					-- waypoint is routinely the centre of the tile the
					-- character already stands on. That leg costs nothing:
					-- the next waypoint is steered at on this same tick.
					if (math.abs(dx) < 0.3 and math.abs(dy) < 0.3) then
						w.idx = w.idx + 1
						w.idx_tick = event.tick
						w.idx_pos = { x = pos.x, y = pos.y }
						-- New leg, new progress clock. See WALK_STALL_TICKS.
						w.best_dist = nil
						w.progress_tick = nil
						if w.idx > #w.waypoints then
							player.walking_state = {walking=false}
							if w.stepped_clear ~= nil then
								-- The last waypoint was the step-clear landing,
								-- not the caller's destination: this walk
								-- stalled, and stepping clear is how it ends,
								-- not where it was going. See
								-- WALK_STEP_CLEAR_RADIUS.
								action_failed(event.tick, w.action_id, w.stepped_clear.cause
									.. ", then stepped clear to " .. coord(pos))
							else
								action_completed(event.tick, w.action_id)
							end
							storage.p[idx].walking = nil
							writeout_player_position(event.tick, idx, player)
							dx = 0
							dy = 0
						else
							dest = w.waypoints[w.idx]
							dx = dest.x - pos.x
							dy = dest.y - pos.y
						end
					end

					if storage.p[idx].walking ~= nil then
						walk_leg_note_distance(w, event.tick, math.sqrt(dx * dx + dy * dy))
					end

					-- The steer is the complement of the arrival box above:
					-- `>= 0.3` here against `< 0.3` there. With `> 0.3` an
					-- offset of exactly 0.3 was neither arrived nor steered,
					-- and the character stood still until the clock ran out.
					if math.abs(dx) >= 0.3 then
						if dx < 0 then dx = -1 else dx = 1 end
					else
						dx = 0
					end

					if math.abs(dy) >= 0.3 then
						if dy < 0 then dy = -1 else dy = 1 end
					else
						dy = 0
					end

					local direction
					if dx < 0 then
						direction = "west"
					elseif dx == 0 then
						direction = ""
					elseif dx > 0 then
						direction = "east"
					end

					if dy < 0 then
						direction = "north"..direction
					elseif dy == 0 then
						direction = ""..direction
					elseif dy > 0 then
						direction = "south"..direction
					end

--					print("waypoint "..w.idx.." of "..#w.waypoints..", pos = "..coord(pos)..", dest = "..coord(dest).. ", dx/dy="..dx.."/"..dy..", dir="..direction)
					local since_progress = walk_leg_progress_tick(w)
					if since_progress ~= nil and event.tick - since_progress > WALK_STALL_TICKS then
						-- **The leg has stopped progressing, and the reason is almost
						-- always that the path is stale.** This mod steers along
						-- waypoints the game's pathfinder chose once, at dispatch time,
						-- and the run that dispatched them is *building things*. In
						-- run-1788344167-58471 bot 1 kept walking at an ore tile at
						-- (-23.5, 18.5) behind a stone furnace at (-22.0, 18.0) that the
						-- same run had placed 4,400 ticks earlier -- a 2x2 furnace
						-- spanning x in [-23, -21], squarely across the route. We are the
						-- thing changing the world, so a stale path is the normal case.
						--
						-- **The recovery does not live here, and it never should
						-- have.** This branch used to teleport the character onto the
						-- next waypoint, and then it re-pathed instead; both were
						-- attempts to rescue the walk from inside the mod, which is the
						-- worst-placed part of the system to try. All this code has is
						-- `w.waypoints`, so the best goal it can name is the last node
						-- of the path that has just gone stale -- not the goal the
						-- caller asked for, at a tolerance the caller never chose,
						-- judged against nothing. The re-path was therefore strictly
						-- harder than the request that had already failed, and in its
						-- most common case arithmetically impossible: run 30's three
						-- failed walks all ended inside a stone furnace, whose clearance
						-- needs 0.8984375 against a re-path radius of 0.5.
						--
						-- Rust has the goal, the radius and the standability judgement
						-- (`move_player_timed`, crates/core/src/factorio/rcon.rs), and
						-- it retries there against the *live* world. So the honest and
						-- useful thing to do here is to report the stall promptly and
						-- let the side that can ask a better question ask it.
						--
						-- The last leg is not special. It is the leg most likely to be
						-- blocked by something the run built at the destination, and it
						-- is exactly the leg whose failure the planner most needs to be
						-- honest about.
						print("Player is stuck on leg "..w.idx.." of "..#w.waypoints.." at "..coord(pos)..", failing the walk")
						-- **Ask the game what is there, now, while it still is.**
						-- This is the only instant at which the cause of a stall
						-- can be observed: the Rust side answers a stall by asking
						-- for a fresh path, which usually works, so by the time
						-- anything else looks the obstruction is behind a bot that
						-- walked around it.
						--
						-- Under `pcall` because it runs inside `on_tick`: an error
						-- raised here would take the whole handler down for every
						-- bot on the surface, and this is an observation, not a
						-- part of walking. A probe that raises reports itself --
						-- `blocker unknown (probe failed: ...)` -- rather than
						-- leaving the message looking like the old one.
						local probed, cause = pcall(walk_stall_cause, player, pos, dest, dx, dy)
						if not probed then
							cause = "blocker unknown (probe failed: " .. tostring(cause) .. ")"
						end
						-- **`made no progress` now means what it says**: the tick
						-- count is the time since the distance to the waypoint
						-- last shrank (WALK_STALL_TICKS), not the time since the
						-- leg began. `from` is where the character stands NOW --
						-- `walk_endpoints` (crates/scripting_lua/src/globals/record.rs)
						-- documents it as the observed stall position and the
						-- archive is read that way -- so the leg's own origin and
						-- length follow, or a reader takes `from`..`to` for the
						-- leg and a 1.42-tile leg for a 0.38-tile one, which is
						-- what happened with run 9.
						--
						-- The steering clause is the observation the engine
						-- question needs. Run 9's character stood still for 54
						-- ticks on open dirt with this follower setting
						-- `walking_state` every one of them while the server ran
						-- at 2-10 ticks a second. Whether the game kept the
						-- state it was given (`walking=true` read back: it held
						-- the character in place) or dropped it (`walking=false`:
						-- something else, in practice the client's own input,
						-- overwrote the steer) is the difference between two
						-- mechanisms, and only this instant can tell them apart.
						-- `player.walking_state` read here is the engine's value
						-- after the previous tick's steer was applied and the
						-- tick was simulated.
						local moved = "unknown"
						local leg = ""
						if w.idx_pos ~= nil then
							moved = string.format("%.2f", distance(w.idx_pos, pos))
							leg = " of a " .. string.format("%.2f", distance(w.idx_pos, dest))
								.. "-tile leg that began at " .. coord(w.idx_pos)
						end
						local read_back = "unreadable"
						local ok_state, state = pcall(function() return player.walking_state end)
						if ok_state and type(state) == "table" then
							read_back = tostring(state.walking == true)
						end
						local speed = "unknown"
						local ok_speed, running = pcall(function() return player.character_running_speed end)
						if ok_speed and type(running) == "number" then
							speed = string.format("%.3f", running)
						end
						local stuck = "ERROR: stuck while walking, leg " .. w.idx .. " of "
							.. #w.waypoints .. " made no progress for "
							.. (event.tick - since_progress) .. " ticks from "
							.. coord(pos) .. " to " .. coord(dest)
							.. ", moved " .. moved .. " tiles" .. leg .. ", " .. cause
							.. ", steering " .. (direction ~= "" and direction or "nowhere") .. " at " .. speed
							.. " tiles/tick, walking_state read back walking=" .. read_back
						-- A pinned character steps onto clear ground first, and
						-- the walk is failed when it gets there (or when that
						-- step stalls too). See WALK_STEP_CLEAR_RADIUS.
						local landing = nil
						if w.stepped_clear == nil then
							local ok, found = pcall(walk_step_clear_landing, player, pos)
							if ok then landing = found end
						end
						if landing ~= nil then
							w.stepped_clear = { from = { x = pos.x, y = pos.y }, to = landing, cause = stuck }
							w.waypoints = { landing }
							w.idx = 1
							w.idx_tick = event.tick
							w.idx_pos = { x = pos.x, y = pos.y }
							w.best_dist = nil
							w.progress_tick = nil
							print("Player is pinned at " .. coord(pos) .. ", stepping clear to "
								.. coord(landing) .. " before failing the walk")
							writeout(event.tick, "walk_step_clear", helpers.table_to_json({
								player_id = idx,
								action_id = w.action_id,
								from = w.stepped_clear.from,
								to = { x = landing.x, y = landing.y },
								cause = cause,
							}))
							-- Steered at the landing from the next tick on; the
							-- direction computed above was for the dead leg.
							direction = ""
							player.walking_state = {walking=false}
						else
							if w.stepped_clear ~= nil then
								stuck = stuck .. ", after stepping clear from "
									.. coord(w.stepped_clear.from) .. " towards " .. coord(w.stepped_clear.to)
							end
							w.stuck = stuck
							-- Nil the waypoint being steered at rather than advancing past
							-- it: the `dest == nil` arm above then clears `walking` and
							-- reports `w.stuck` on the next tick, which is the one exit a
							-- failed walk has. Stop steering now -- `direction` above was
							-- computed for a leg this walk is no longer walking.
							direction = ""
							player.walking_state = {walking=false}
							w.waypoints[w.idx] = nil
						end
					end

					if direction ~= "" then
						player.walking_state = {walking=true, direction=defines.direction[direction]}
					end
				end
			end

			if storage.p[idx].mining and storage.p[idx].mining.inventory_before ~= nil then
				-- Character bot: the products landed in the inventory, or did
				-- not. Checked before the entity test because a tree or a
				-- rock is destroyed on the same tick its yield lands.
				local m = storage.p[idx].mining
				local inv = player.get_main_inventory()
				local delivered = 0
				for name, was in pairs(m.inventory_before) do
					local now = inv.get_item_count(name)
					if now > was then
						delivered = delivered + (now - was)
						m.inventory_before[name] = now
					end
				end
				if delivered > 0 then
					-- The yield landed: a change this mod did not make but
					-- does observe, so the flag is exact here too.
					mark_bot_inventory_dirty(idx)
					m.left = m.left - delivered
					if m.left <= 0 then
						action_completed(event.tick, m.action_id)
						player.mining_state = { mining = false }
						storage.p[idx].mining = nil
					end
				end
			end

			if storage.p[idx].mining then
				local ent = storage.p[idx].mining.entity

				if ent and ent.valid then -- mining complete
					-- The bound the *game* enforces is resource_reach_distance, and it
					-- enforces it silently: mining_state aimed past it produces no event,
					-- no error and no progress. This guard used to be a flat `> 6`, which
					-- is looser than the ~2.7 a character actually has, so a request the
					-- game would refuse was accepted here and then hung until the caller's
					-- 360s deadline. Guarding with the real value turns that silence into
					-- an immediate, readable rejection.
					--
					-- It also has to *stop*: the old branch reported the failure and then
					-- fell straight through into setting mining_state anyway, so the same
					-- action could be reported failed and later completed.
					if not mining_target_reachable(player, ent) then
						action_failed(event.tick, storage.p[idx].mining.action_id, "ERROR: too far too mine")
						storage.p[idx].mining = nil
					else

					-- unfortunately, factorio doesn't offer a "select this entity" function
					-- we need to select stuff depending on the cursor position, which *might*
					-- select something different instead. (e.g., a tree or the player in the way)
					player.update_selected_entity(ent.position)
					local ent2 = player.selected
					local m = storage.p[idx].mining

					-- `blocked` is the reason this tick could NOT set
					-- `mining_state`, or nil if it could. Every branch below has
					-- to set one or the other, because a branch that sets
					-- neither is a mine that never mines and never answers --
					-- see MINE_BLOCKED_TIMEOUT_TICKS for what that used to cost.
					local blocked = nil

					if (ent2 == nil) then
						blocked = "nothing selectable at " .. coord(ent.position)
					elseif (ent.name ~= ent2.name or ent.position.x ~= ent2.position.x or ent.position.y ~= ent2.position.y) then
						if ent2.type == "tree" then
							print("mining: there's a tree in our way. deforesting...") -- HACK
							player.mining_state = { mining=true, position=ent.position }
						elseif ent2.name == "character" then
							blocked = "another character is standing on the " .. ent.name
							-- Step aside once per blocked episode, not once per
							-- tick: re-dispatching every tick restamps the
							-- walk's own leg timer, so the walk could never
							-- time out, and every completion wrote another
							-- reply under the same hardcoded action id.
							if not m.stepped_aside then
								m.stepped_aside = true
								print("mining: " .. blocked .. ", stepping aside")
								rcon_action_start_walk_waypoints(4711, idx, { mine_step_aside_waypoint(player, ent) })
							end
						else
							blocked = "expected " .. ent.name .. " at " .. coord(ent.position) .. ", found " .. ent2.name
						end
					else
						player.mining_state = { mining=true, position=ent.position }
					end

					if blocked == nil then
						-- Progress. Whatever was in the way is not any more, so
						-- the clock starts again if it ever comes back.
						m.blocked_since = nil
						m.blocked_reason = nil
						m.stepped_aside = nil
					else
						m.blocked_reason = blocked
						if m.blocked_since == nil then
							m.blocked_since = event.tick
						elseif event.tick - m.blocked_since > MINE_BLOCKED_TIMEOUT_TICKS then
							action_failed(event.tick, m.action_id,
								"ERROR: could not start mining for "
								.. (event.tick - m.blocked_since) .. " ticks: " .. blocked)
							storage.p[idx].mining = nil
						end
					end

					end
				else
					-- the entity to be mined has been deleted, but p[idx].mining is still true.
					-- this means that on_mined_entity() has *not* been called, indicating that something
					-- else has "stolen" what we actually wanted to mine :(
					--
					-- The reason is not optional. Called without one, `action_failed`
					-- writes `tostring(nil)`, and the executor reads the whole verdict
					-- as the literal string "nil" -- which is exactly what the
					-- 2026-09-02 run reported for a copper-ore mine two bots raced
					-- for: `game rejected the command: Unexpected Response: nil`.
					action_failed(event.tick, storage.p[idx].mining.action_id,
						"ERROR: the target " .. storage.p[idx].mining.prototype.name ..
						" was gone before mining finished -- something else mined it first")
					complain("failed to mine " .. storage.p[idx].mining.prototype.name)
					storage.p[idx].mining = nil
				end
			end
		end
	end

    if last_tick == 0 then
		writeout_initial_stuff()
	end

	if event.tick % 120 == 0 then
		local who = "?"
		if client_local_data.whoami then who = client_local_data.whoami end
		-- THE READINESS BEACON. Not a debug line: `read_output`
		-- (crates/core/src/process/output_reader.rs) blocks on a stdout line
		-- containing `my_client_id` before it opens RCON at all, so this is
		-- the handshake, and `who == "?"` is what stops it once the host has
		-- introduced itself with `whoami`.
		--
		-- `tostring`, and no `~= nil` guard. It used to refuse to print when
		-- `my_client_id` was nil, which made the signal go silent in exactly
		-- the state nobody expected -- the "silence is not success" shape
		-- CLAUDE.md enumerates, and worth 900 seconds of a stopped run. A
		-- beacon reading `my_client_id=nil` still unblocks the host and still
		-- says something true; `on_init` above is what makes it a number.
		if who == "?" then print("my_client_id="..tostring(my_client_id)..", who="..who) end
	end

	-- periodically update the objects around the player to ensure that nothing is missed
	-- This is merely a safety net and SHOULD be unnecessary, if all other updates don't miss anything
--	if event.tick % 300 == 0 and false then -- don't do that for now, as it eats up too much cpu on the c++ part
--		for idx, player in each_bot() do
--			if player.connected and player.character then
--				local x = math.floor(player.character.position.x/32)*32
--				local y = math.floor(player.character.position.x/32)*32
--
--				for xx = x-96, x+96, 32 do
--					for yy = y-96, y+96, 32 do
--						writeout_objects(event.tick, player.surface, {left_top={x=xx,y=yy}, right_bottom={x=xx+32,y=yy+32}})
--					end
--				end
--			end
--		end
--	end

	if #todo_next_tick > 0 then
		if not wait_for_player then
			print("on_tick executing "..#todo_next_tick.." stored callbacks")
		end
		for _,func in ipairs(todo_next_tick) do
			func()
		end
		todo_next_tick = {}
	elseif #todo_next_tick_other > 0 then
		if not wait_for_player then
			print("on_tick executing "..#todo_next_tick_other.." stored callbacks")
		end
		for _,func in ipairs(todo_next_tick_other) do
			func()
		end
		todo_next_tick_other = {}
	end
	last_tick = event.tick
end

-- `event.player_index` is who mined it, definitively -- the game hands it to
-- us. This used to loop over EVERY connected bot and complete the mining task
-- of any bot whose `mining.entity` happened to `==` the entity in THIS event,
-- not just the bot that actually did the mining.
--
-- `nearest_resource_tile`/`resource_tiles_for` (crates/planner) pick tiles
-- per bot from the *modeled* remaining amount, with no reservation between
-- bots planned in the same pass -- two bots gathering the same item from the
-- same patch can legitimately be assigned the same tile, each taking a share
-- of it. Concurrent mining of one entity by two different players is normal
-- multiplayer behaviour: the game gives each of them their own
-- `on_player_mined_entity` event, with its own `player_index`, for their own
-- swings. But every one of those events also satisfied `mining.entity ==
-- event.entity` for BOTH bots' stored mining tasks, so bot A's swings
-- decremented bot B's `left` too (and vice versa) -- whichever bot needed
-- fewer swings hit zero first, on a mix of its own and the other bot's
-- progress, and reported success having personally mined only part of its
-- count. Scoping to `event.player_index` makes each bot's completion depend
-- only on its own swings, which is what "requested count has actually been
-- delivered" requires.
function on_mined_entity(event)
	local idx = event.player_index
	local player = game.players[idx]
	if storage.p[idx] and player and player.connected and player.character then
		local mining = storage.p[idx].mining
		if mining and mining.entity == event.entity then
			-- The buffer is what this swing actually produced -- not
			-- `mining.prototype.mineable_properties.products`, which is what
			-- mining *usually* yields and says nothing about a productivity
			-- or quality bonus this particular swing got. Counting the
			-- prototype's expected amount instead of the buffer is the
			-- sibling of the player-scoping bug above: it under/over-counts
			-- "left" by the same kind of gap between "asked for" and
			-- "actually happened".
			local mined = inventory_to_dict(event.buffer)
			local tmp_recent_item_addition = {}
			tmp_recent_item_addition.tick = event.tick
			tmp_recent_item_addition.action_id = mining.action_id
			tmp_recent_item_addition.itemlist = mined
			if recent_item_additions[idx] == nil then recent_item_additions[idx] = {} end
			table.insert(recent_item_additions[idx], tmp_recent_item_addition)
			print("mining: " .. helpers.table_to_json(mining))
			-- **Keyed by what this entity YIELDS, not by what it is called.**
			-- The two are the same name for every ore -- mining `iron-ore`
			-- gives `iron-ore` -- and for nothing else. A tree called
			-- `tree-01` fills the buffer with `wood`, so the old
			-- `mined[mining.prototype.name]` read nil, `left` never reached
			-- zero, and the very next tick found the entity destroyed and
			-- reported "the target tree-01 was gone before mining finished --
			-- something else mined it first" about a tree this player had just
			-- successfully chopped. Every rock has the same shape.
			--
			-- Still read out of `event.buffer`, which is what this swing
			-- actually produced: the products list supplies the *names* to
			-- look up and never the amounts, so a productivity or quality
			-- bonus is counted as it happened rather than as the prototype
			-- predicted. That is the distinction the paragraph above is about,
			-- and it survives.
			local delivered = 0
			local products = mining.prototype.mineable_properties
				and mining.prototype.mineable_properties.products
			if products then
				for _, product in pairs(products) do
					if product.name then
						delivered = delivered + ((mined and mined[product.name]) or 0)
					end
				end
			else
				-- No products list at all: fall back to the old key rather
				-- than to zero, because zero is the answer that hangs.
				delivered = (mined and mined[mining.prototype.name]) or 0
			end
			mining.left = mining.left - delivered
			if mining.left <= 0 then
				action_completed(event.tick, mining.action_id)
				storage.p[idx].mining = nil
			end
		end
	end
end

function writeout_players(tick)
	local players={}
	local count = 0
	for idx, player in each_bot() do
		if player.connected and player.character then
			count = count + 1
			table.insert(players, idx.." "..player.character.position.x.." "..player.character.position.y)
		end
	end
	if count > 0 then
		writeout(tick, "players", table.concat(players, ","))
	end
end

function on_sector_scanned(event)
--	print("sector scanned")
--	print(event.radar)
end

function pos_id(x,y)
	return x.."/"..y
end

function max(list, fn)
	local highest = nil
	local highest_idx = nil

	for idx, val in ipairs(list) do
		local fnval = fn(val)
		if highest == nil or fnval > highest then
			highest = fnval
			highest_idx = idx
		end
	end

	return list[highest_idx], highest_idx
end

function on_chunk_generated(event)
	local area = event.area
	local surface = event.surface
	--print("chunk generated at ("..area.left_top.x..","..area.left_top.y..") -- ("..area.right_bottom.x..","..area.right_bottom.y..")")
	local chunk_x = area.left_top.x
	local chunk_y = area.left_top.y
	local chunk_xend = area.right_bottom.x
	local chunk_yend = area.right_bottom.y

	-- ONE SURFACE, AND THE DROP IS NOW RECORDED RATHER THAN PRINTED.
	--
	-- This guard is the only thing standing between this system and silent
	-- aliasing: the entity graph keys by position alone, so a chunk from a
	-- second surface would merge into Nauvis with no error anywhere. Space Age
	-- is enabled in this workspace, so the second surface is one rocket away.
	--
	-- It used to `print("unknown surface")`, which is NOT a writeout: it
	-- carries no `§tick§key§` envelope, so it reached the server log and no
	-- record artefact. A run that quietly discarded a whole planet's chunks
	-- looked identical to one that never visited it. Now it writes out, so the
	-- record can say what was dropped and how often -- the same disclosure
	-- `research_trigger_emulated` and the `ground_generate_*` counters make.
	--
	-- See docs/superpowers/notes/2026-09-06-surfaces-survey.md.
	--
	-- `left_top`, NOT `chunk`. The locals above are named `chunk_x`/`chunk_y`
	-- but hold `area.left_top`, which is a TILE coordinate -- the corner of the
	-- chunk, so (-32, 64) rather than chunk (-1, 2). Reporting that under the
	-- name `chunk` would be a field that is not blank but confidently about the
	-- wrong object, which a reader cannot detect. The name says which.
	if surface ~= game.surfaces['nauvis'] then
		writeout(event.tick, "surface_chunk_dropped", helpers.table_to_json({
			surface = surface.name,
			left_top = { x = chunk_x, y = chunk_y },
		}))
		return
	end

	if chunk_x < -512 then return end
	if chunk_y < -512 then return end
	if chunk_xend > 512 then return end
	if chunk_yend > 512 then return end

	if chunk_x < storage.map_area.x1 then storage.map_area.x1 = chunk_x end
	if chunk_y < storage.map_area.y1 then storage.map_area.y1 = chunk_y end
	if chunk_xend > storage.map_area.x2 then storage.map_area.x2 = chunk_xend end
	if chunk_yend > storage.map_area.y2 then storage.map_area.y2 = chunk_yend end

	writeout_entities(event.tick, surface, area)
	local chunk_id = chunk_x .. "/" .. chunk_y
	if tile_chunks[chunk_id] == nil then
		tile_chunks[chunk_id] = true
		writeout_tiles(event.tick, surface, area)
	end

	if client_local_data.whoami == "client1" then
		chunk_screenshot(chunk_x, chunk_y)
	end
end

function chunk_screenshot(chunk_x, chunk_y)
	local tpath = "tiles/tile" .. tostring(chunk_x) .. "_" .. tostring(chunk_y) .. ".png"
	game.take_screenshot({
		player = game.players[1],
		by_player = game.players[1],
		surface = game.surfaces[1],
		position = {chunk_x + 16,chunk_y + 16},
		resolution = {512,512},
		zoom = 0.5,
		path = tpath,
		show_entity_info = true
	})
	--game.set_wait_for_screenshots_to_finish()
end

function chunk_screenshot2(chunk_x, chunk_y)
	local tpath = "tiles/bigtile" .. tostring(chunk_x) .. "_" .. tostring(chunk_y) .. ".png"
	game.take_screenshot({
		player = game.players[1],
		by_player = game.players[1],
		surface = game.surfaces[1],
		position = {chunk_x + 128,chunk_y + 128},
		resolution = {512,512},
		zoom = 0.0625,
		path = tpath,
		show_entity_info = true
	})
	--game.set_wait_for_screenshots_to_finish()
end

-- ---------------------------------------------------------------------------
-- The recording session
-- ---------------------------------------------------------------------------
--
-- A *session* is what makes this mod write anything about the world: all
-- three samplers -- `sample_force` and `sample_machines` on the 300-tick beat,
-- `sample_bots` on the 60-tick one -- return early unless `storage.sampling`
-- is set, and every
-- sample line is tagged with the session's opaque `run` id so a consumer can
-- tell one run's stream from the next one's.
--
-- **This used to render screenshots as well, and no longer does.** Until
-- 2026-09-02 the 300-tick beat also drove `game.take_screenshot` through a
-- catalogue of cameras (`follow`, `bot-<index>`, `area`), writing JPEGs into
-- `script-output/frames/` beside a `frames/run.json` sidecar. That was
-- retired and is now removed: run `run-1788365280-15443` wrote 2,164 JPEGs /
-- 947 MB of them against 290 MB for the same 45 minutes of video, and
-- `game.take_screenshot` renders *synchronously inside the game loop*, once
-- per camera per capture, where the video grabber reads a frame the GPU
-- already drew. Video is the visual record now; see
-- `docs/superpowers/notes/2026-09-02-screenshots-retired.md` for what was
-- lost with them.
--
-- The session survived the screenshots on purpose: deleting it would have
-- taken `samples.jsonl` -- research, production, per-network power, bot
-- inventories and per-machine state -- with it, silently.

local SAMPLE_FORCE_INTERVAL = 300 -- game ticks between force samples (5 s at 60 UPS)

-- Sample schema. Bumped deliberately on every field change, because
-- info.json has read 0.0.1 since the project began and cannot tell a stale
-- workspace/mods from a current one. Rust refuses a schema it does not know.
local SAMPLE_SCHEMA = 2
local SAMPLE_DIR = "botbridge"
local SAMPLE_FILE = SAMPLE_DIR .. "/samples.jsonl"
local SAMPLE_BOT_INTERVAL = 60 -- 1 s at 60 UPS

-- ---------------------------------------------------------------------------
-- Machine telemetry
-- ---------------------------------------------------------------------------
--
-- The record could say what every *bot* held and what the *force* totalled,
-- and nothing whatsoever about any individual machine. So a run that built a
-- red-science cell -- twelve actions all reporting success, both recipes set,
-- chests charged three times, poles placed adjacent to both assemblers -- could
-- not answer "did the assemblers have power, hold ingredients, or make
-- anything". That is the gap `sample_machines` closes.
--
-- The one field that ends most of those arguments is `LuaEntity.status`. It is
-- the game's own verdict on why a machine is or is not running, and it
-- separates the cases that look identical from outside: `working`,
-- `no_power`, `low_power`, `no_ingredients`, `full_output`,
-- `not_enough_space_in_output`, `no_recipe`, `no_fuel`,
-- `not_plugged_in_electric_network`, `no_minable_resources`. Confirmed present
-- on `LuaEntity` in this install's `runtime-api.json` (2.1.17) with
-- `subclasses: None` -- so it is readable off any entity -- and `optional:
-- true`, so it can be nil and is written only when it is not.
--
-- NOTE the two names the brief for this work guessed wrong, both checked
-- against `defines.entity_status` rather than assumed: there is no
-- `output_full` (it is `full_output`, with `not_enough_space_in_output` as the
-- separate "the output slot cannot take the next craft" case).

-- Every `defines.entity_status` value, by number -- ONE table, in `types.lua`.
--
-- `entity_status_name` used to be this file's own local `ENTITY_STATUS_NAMES`.
-- It moved to `types.lua` (required at the top of this file) when
-- `serialize_entity` needed the same mapping for `FactorioEntity.status`: two
-- inversions of the same enum in one mod is two things to keep in step, and
-- the sample stream and the entity record must never disagree about what a
-- status is called.
--
-- Numbers are not written to the record. A status id is meaningless without
-- the table, and a reader holding an archive from a future Factorio would
-- have no way to resolve one -- so the *name* is what crosses the wire, and an
-- id this build cannot name is written as `unmapped_<n>` rather than dropped.

-- The machine types worth a row. Deliberately a short list rather than "every
-- entity": belts, inserters, chests and pipes are numerous, and none of them
-- has the recipe/ingredient/output triple that the question "is this machine
-- working" is about. `generator` is the steam engine and `boiler` its feeder --
-- both are here so that "the assembler is on a different electric network from
-- the thing generating the power" is answerable from one sample line.
local MACHINE_TYPES = {
	"assembling-machine",
	"furnace",
	"mining-drill",
	"lab",
	"boiler",
	"generator",
	-- Chests are here because of what a run actually died of, not because a
	-- chest is a machine. `run-1788459085-32452`'s red-science cell had power,
	-- had its recipes and did produce -- about 4 science and 16 gears per
	-- charge -- and then sat idle for ~76,000 ticks because nothing refilled
	-- its input chests. `status` on a container says nothing, but its
	-- *contents* are the difference between "the cell is broken" and "the cell
	-- ran dry", and those are opposite repairs. Six of them in that run, so the
	-- cost is nil.
	"container",
	"logistic-container",
}

-- Container types, whose contents come from `defines.inventory.chest` rather
-- than from an output or ingredient inventory.
local CONTAINER_TYPES = {
	["container"] = true,
	["logistic-container"] = true,
}

-- Which of those are `CraftingMachine` in the API's subclass sense, and so
-- accept `get_recipe`, `is_crafting`, `crafting_progress` and
-- `products_finished`. Reading any of those off a `lab`, `boiler` or
-- `mining-drill` raises, and a raise inside a sampler is what killed a live
-- six-minute run once already (see `record_sample_failure`).
local CRAFTING_MACHINE_TYPES = {
	["assembling-machine"] = true,
	["furnace"] = true,
}

-- Factorio 2.1.17 renamed the crafting-machine inventory defines: this
-- install's `defines.inventory` has `crafter_input`/`crafter_output`/
-- `crafter_modules` and has **no** `furnace_source`, `furnace_result`,
-- `assembling_machine_input` or `assembling_machine_output` at all (verified
-- against `runtime-api.json`, not assumed -- the dead `inventory_type_name`
-- near the top of this file still indexes the removed names and would raise
-- "table index is nil" if anything ever called it).
--
-- The fallback is for an older Factorio, not for this one, and `nil` is a
-- supported outcome: `machine_inventory` below refuses a nil index instead of
-- passing it to `get_inventory`.
local CRAFTER_INPUT_INVENTORY = defines.inventory.crafter_input
	or defines.inventory.assembling_machine_input
local LAB_INPUT_INVENTORY = defines.inventory.lab_input

-- A ceiling on rows per sample, so that a late-game base cannot turn a
-- telemetry line into a megabyte. Overflow is *counted and reported*
-- (`truncated`), never silently dropped: a reader must be able to tell "this
-- is every machine" from "this is the first 400 of them".
local MACHINE_SAMPLE_LIMIT = 400

-- Appends one JSON line, on the server only.
--
-- The fourth argument is what restricts the write. `on_nth_tick` runs on every
-- peer, so without it every client would write its own copy -- but force
-- statistics are identical on every peer and bot inventories are readable from
-- any of them, so four copies would be four identical files to reconcile for
-- nothing.
local function write_sample(line)
	helpers.write_file(SAMPLE_FILE, helpers.table_to_json(line) .. "\n", true, 0)
end


-- Generation and demand across every electric network the force owns.
--
-- Reported in kW to match the numbers a player sees. `satisfaction` is
-- consumed over demanded, or 1.0 when nothing demands anything -- a network
-- with no load is fully satisfied, not divided by zero.
--
-- NOT sourced from `LuaEntity.electric_network_statistics` (a
-- `LuaFlowStatistics`, the same class `sample_force` below uses for item
-- production). Two things rule it out, both confirmed against this Factorio
-- install's `runtime-api.json` rather than assumed:
--
--   1. `input_counts`/`output_counts` on that class are cumulative totals
--      since the network's statistics object existed -- the same
--      "since game start" accumulation `production` below deliberately
--      keeps. An ever-growing joule total labelled `generated_kw` would look
--      plausible and be wrong by orders of magnitude within minutes.
--   2. For electric networks specifically, the class's own docs invert the
--      usual reading: "the electric network GUI shows 'power consumption' on
--      the left side, so in this case `input` describes the power
--      consumption numbers" -- i.e. `input_counts` is demand, not
--      generation, the opposite of the item-production convention (where
--      `input` is what was made) that this file otherwise follows just
--      below. Porting that pattern here would silently swap the two.
--
-- `LuaElectricNetwork.flow_last_tick` is what the docs describe as "energy
-- amounts ... related to latest electric network update": one tick's actual
-- production, demand and delivered energy, named for what they are
-- (`maximum_production`, `maximum_consumption`, `total_transfer`) rather than
-- by GUI position. Its values are joules for that one tick; ticks run at
-- 60 UPS, so `* 60` converts to watts and `/ 1000` to the kW a player sees.
--
-- `flow_last_tick` lives on `LuaElectricNetwork`, not on what `pole
-- .electric_network` returns: that attribute's `read_type` is
-- `LuaElectricSubNetwork` (a copper-wire-connected cluster), whose complete
-- attribute list is `{id, neighbours, object_name, parent_network, valid}` --
-- no `flow_last_tick`. Indexing a nonexistent attribute on Factorio userdata
-- is a hard runtime error, not nil, so getting this wrong throws inside
-- `sample_force` on the first 300-tick beat after any pole exists and never
-- writes a force sample again. The parent `LuaElectricNetwork` -- reached via
-- `.parent_network` -- is what actually carries `flow_last_tick`.
--
-- Several sub-networks can share one parent (joined by a closed power
-- switch, for instance), so dedup happens at the parent, not the
-- sub-network: encountering an unseen sub-network marks every sibling listed
-- in its parent's `sub_networks` as seen before summing, so a shared parent
-- is added once no matter how many of its sub-networks a force's poles touch.
-- `LuaElectricNetwork` itself has no `id` attribute to key a seen-set on
-- directly (confirmed absent from `runtime-api.json`, not assumed) -- hence
-- keying the seen-set on sub-network ids instead, all marked together.
--
-- Verified against `runtime-api.json` for both classes actually touched here
-- (`LuaElectricSubNetwork` and `LuaElectricNetwork`), not just the one that
-- owns `flow_last_tick` -- unverified against a live game, since this task is
-- static-only by design (see the task brief).
--
-- ---------------------------------------------------------------------------
--
-- Since 2026-09-03 this also reports **each network separately**, and that is
-- the half that answers a question the totals structurally cannot. A force
-- reading `generated_kw: 900, consumed_kw: 60` looks healthy; if those 900 kW
-- are on the network holding the lab and the assemblers sit on an island whose
-- only pole reaches no generator, the force total says nothing at all about
-- it. Per-network figures show the island as its own entry with
-- `generated_kw = 0`, and `sample_machines`' `network` field says which
-- entry each machine is on. Coverage is not capacity, and neither is a
-- force-wide sum.
--
-- `networks` is keyed by the **smallest sub-network id under a parent**, not
-- by a network id, because `LuaElectricNetwork` has no `id` attribute (the
-- same absence the seen-set above works around). Each entry lists its whole
-- `sub_ids` set, which is what a machine's `electric_network_id` is matched
-- against -- so the key is only a handle, and nothing depends on it being the
-- game's idea of a name. It is a map rather than an array on purpose:
-- `helpers.table_to_json` renders an empty Lua table as `{}`, so an array here
-- would decode as an object the moment a force owned no poles, which is
-- exactly the defect that already makes `bots = {}` an unparseable line.
local function power_totals(force)
	local generated, consumed, demanded = 0.0, 0.0, 0.0
	local seen_subnetworks = {}
	local networks = {}
	for _, surface in pairs(game.surfaces) do
		for _, pole in pairs(surface.find_entities_filtered({
			type = "electric-pole", force = force,
		})) do
			local sub = pole.electric_network
			if sub and sub.valid and not seen_subnetworks[sub.id] then
				local network = sub.parent_network
				if network and network.valid then
					-- Mark every sub-network under this parent seen, not just
					-- `sub` itself, so the parent is summed once even though
					-- several sub-networks may lead to it. `sub.id` is marked
					-- explicitly too, rather than assumed to be among
					-- `network.sub_networks`'s own members: if a parent ever
					-- did not list itself there, `sub` would never be marked
					-- seen and every pole touching it would sum this parent
					-- again, silently multiplying the power numbers.
					seen_subnetworks[sub.id] = true
					-- `sub.id` seeds the list for the same reason it is marked
					-- seen explicitly above: a parent that did not list itself
					-- in `sub_networks` must still contribute the id the pole
					-- actually reached, or a machine on it would match no
					-- entry and read as unpowered when it is not.
					local sub_ids = { sub.id }
					local listed = { [sub.id] = true }
					for _, sibling in pairs(network.sub_networks) do
						seen_subnetworks[sibling.id] = true
						if not listed[sibling.id] then
							listed[sibling.id] = true
							sub_ids[#sub_ids + 1] = sibling.id
						end
					end
					table.sort(sub_ids)
					local flow = network.flow_last_tick
					local net_generated = flow.maximum_production * 60 / 1000
					local net_consumed = flow.total_transfer * 60 / 1000
					local net_demanded = flow.maximum_consumption * 60 / 1000
					generated = generated + net_generated
					consumed = consumed + net_consumed
					demanded = demanded + net_demanded
					local net_satisfaction = 1.0
					if net_demanded > 0 then
						net_satisfaction = math.min(1.0, net_consumed / net_demanded)
					end
					networks[tostring(sub_ids[1])] = {
						sub_ids = sub_ids,
						generated_kw = net_generated,
						consumed_kw = net_consumed,
						demanded_kw = net_demanded,
						satisfaction = net_satisfaction,
					}
				end
			end
		end
	end
	local satisfaction = 1.0
	if demanded > 0 then satisfaction = math.min(1.0, consumed / demanded) end
	return {
		generated_kw = generated,
		consumed_kw = consumed,
		satisfaction = satisfaction,
		networks = networks,
	}
end

-- Telemetry must never be able to end a live game. A sampler is not part of
-- what keeps the game running -- unlike an on_tick handler moving a bot --
-- so there is never a case where propagating an error here beats skipping
-- this one sample. This is not a hypothetical: reading
-- `LuaEntity.mining_target` off a character (see `character_mining_name`
-- below for the fix) raised inside `sample_bots`, which raised into
-- Factorio's own tick loop, which the game treats as fatal -- ending a live
-- six-minute multi-bot run and every RCON connection with it, mid-packet.
-- Do not remove these pcalls to "simplify" the samplers; they are the only
-- thing standing between a future bad attribute read and another dead
-- server.
--
-- Failures are both counted (`storage.telemetry_failures`, a lifetime total
-- readable over RCON) and, once per failing streak, written where the Rust
-- side can see them (`writeout`, parsed by `output_parser.rs`) -- silence on
-- failure is this codebase's recurring defect, so a persistently broken
-- sampler must show up as failed samples rather than as nothing at all.
-- "Once per streak" (edge-triggered on `storage.telemetry_failing`, cleared
-- on the next success) rather than on every attempt: `sample_bots` runs
-- once a second, so logging every failure would spam a line every 60 ticks
-- for as long as the underlying bug lives -- itself a way to make a log
-- unreadable.
local function record_sample_failure(kind, tick, err)
	storage.telemetry_failures = storage.telemetry_failures or { bots = 0, force = 0, machines = 0 }
	storage.telemetry_failing = storage.telemetry_failing
		or { bots = false, force = false, machines = false }
	storage.telemetry_failures[kind] = (storage.telemetry_failures[kind] or 0) + 1
	if not storage.telemetry_failing[kind] then
		storage.telemetry_failing[kind] = true
		writeout(tick, "sample_error", kind .. " sampler failed (#"
			.. storage.telemetry_failures[kind] .. "): " .. tostring(err))
	end
end

local function record_sample_success(kind)
	storage.telemetry_failing = storage.telemetry_failing
		or { bots = false, force = false, machines = false }
	storage.telemetry_failing[kind] = false
end

-- What a mining character is actually mining, without touching
-- `LuaEntity.mining_target` -- that attribute belongs to mining drills, not
-- characters, and reading it off a character is exactly the crash this file
-- was patched for. Confirmed against
-- `workspace/factorio-api-docs/runtime-api.json`: `LuaEntity.mining_target`
-- exists ("The mining target, if any" -- mining drills), `LuaControl
-- .mining_state` exists and returns `{mining: bool, position: MapPosition}`,
-- and there is no `mining_target` anywhere on `LuaControl`.
--
-- `mining_state` gives a position, not a name, so the name is recovered with
-- `LuaSurface.find_entities_filtered{position=...}` (same file: "returns the
-- entities colliding with that position") -- an ordinary query that answers
-- with an empty array rather than raising when nothing, or something
-- surprising, is there. The character itself is excluded because the mining
-- position can coincide with the miner's own tile; the first remaining match
-- is reported as a best-effort label for telemetry, not a claim that only
-- one entity could ever occupy that spot.
local function character_mining_name(character)
	if character == nil then
		return nil
	end
	local state = character.mining_state
	if not state.mining or state.position == nil then
		return nil
	end
	local entities = character.surface.find_entities_filtered({ position = state.position })
	for _, entity in ipairs(entities) do
		if entity.valid and entity.type ~= "character" then
			return entity.name
		end
	end
	return nil
end

-- Bot inventories and positions, on a 1 s beat -- fast enough to see a bot
-- move or mine, slow enough not to compete with the 300-tick force beat.
--
-- Gated on an active session (F5): a run that never called
-- `rcon_sampling_start` produces no samples at all, which avoids writing a
-- stream nobody asked to correlate with anything.
local function sample_bots_body(tick)
	local session = storage.sampling
	if session == nil then
		return
	end
	local bots = {}
	for _, player in each_bot() do
		local character = player.connected and player.character or nil
		if character ~= nil then bots[#bots + 1] = {
			id = player.index,
			position = player.position,
			-- `inventory_counts` handles Factorio 2.0's get_contents(),
			-- which returns an array of {name, count, quality}, not a dict.
			inventory = character and inventory_counts(
				character.get_inventory(defines.inventory.character_main)
			) or {},
			crafting_queue = player.crafting_queue_size or 0,
			mining = character_mining_name(character),
		} end
	end
	write_sample({
		kind = "bots",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		-- F2: every line carries the run id (nil when the run was started
		-- untagged), so Rust can filter on it instead of on tick range alone.
		run = session.run,
		bots = bots,
	})
end

-- The only entry point anything outside this section should call: `pcall`
-- around `sample_bots_body` so nothing it does can reach the caller as a
-- raise. See the comment above `record_sample_failure` for why this exists.
local function sample_bots(tick)
	local ok, err = pcall(sample_bots_body, tick)
	if ok then
		record_sample_success("bots")
	else
		record_sample_failure("bots", tick, err)
	end
end

-- The only registration site for the bot-sample cadence. A distinct tick (60,
-- not 300) on purpose: `script.on_nth_tick(n, f)` replaces the handler
-- already registered for `n`, and the force sampler owns 300 (see the comment
-- below), so a second registration there would silently disable it instead of
-- adding to it.
script.on_nth_tick(SAMPLE_BOT_INTERVAL, function(event)
	sample_bots(event.tick)
end)

-- Force-wide research, production and power, on the 300-tick beat
-- (`on_sample_force_tick` below). `sample_bots` above got tick 60 rather than
-- sharing this one because `on_nth_tick(300, ...)` twice would *replace* this
-- handler, not add to it.
local function sample_force_body(tick)
	local session = storage.sampling
	if session == nil then
		return
	end
	local force = game.forces["player"]
	local research = nil
	if force.current_research then
		local tech = force.current_research
		research = {
			name = tech.name,
			progress = force.research_progress,
			eta_ticks = nil,
		}
	end
	local unlocked = 0
	for _, tech in pairs(force.technologies) do
		if tech.researched then unlocked = unlocked + 1 end
	end
	-- Cumulative since game start, not per-interval -- Rust reads these as
	-- running totals, same as the production statistics GUI does.
	local made, consumed = {}, {}
	-- SUMMED OVER EVERY SURFACE, not read off Nauvis.
	--
	-- `get_item_production_statistics` is per-surface, and this read
	-- `game.surfaces[1]` while the power and machine samplers **in this same
	-- sample line** already iterate `pairs(game.surfaces)`. Identical today,
	-- because Space Age is enabled in this workspace but no run has ever left
	-- Nauvis. The moment one does, `tools/run_analysis.py` would compute its
	-- `roster-fed` / `factory` / `unclear` verdict from production on one
	-- surface against machines on all of them -- two different populations,
	-- with nothing in the record able to reveal the mismatch.
	--
	-- Found by the surfaces survey
	-- (docs/superpowers/notes/2026-09-06-surfaces-survey.md) and fixed while
	-- it is still a no-op, which is the only cheap moment it will ever have.
	for _, surface in pairs(game.surfaces) do
		local ok, stats = pcall(function()
			return force.get_item_production_statistics(surface)
		end)
		if ok and stats ~= nil then
			for name, count in pairs(stats.input_counts) do
				made[name] = (made[name] or 0) + count
			end
			for name, count in pairs(stats.output_counts) do
				consumed[name] = (consumed[name] or 0) + count
			end
		end
	end

	write_sample({
		kind = "force",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		run = session.run,
		research = research,
		techs_unlocked = unlocked,
		production = { made = made, consumed = consumed },
		power = power_totals(force),
	})
end

-- The only entry point anything outside this section should call: `pcall`
-- around `sample_force_body`, for the same reason `sample_bots` wraps
-- `sample_bots_body` -- see the comment above `record_sample_failure`. A raise
-- here must not be able to take the surrounding game down with it.
local function sample_force(tick)
	local ok, err = pcall(sample_force_body, tick)
	if ok then
		record_sample_success("force")
	else
		record_sample_failure("force", tick, err)
	end
end

-- Item counts in one of an entity's indexed inventories, or nil.
--
-- Refuses a nil index rather than passing it to `get_inventory`, because
-- `CRAFTER_INPUT_INVENTORY` is resolved from `defines.inventory` with a
-- fallback and is allowed to come out nil on a Factorio that has neither
-- name. `get_inventory` also answers nil for an entity that simply has no such
-- inventory, which is not an error either.
local function machine_inventory(entity, index)
	if index == nil then
		return nil
	end
	local inventory = entity.get_inventory(index)
	if inventory == nil or not inventory.valid then
		return nil
	end
	return inventory_counts(inventory)
end

-- An inventory's counts, or nil when there is nothing in it.
--
-- Empty inventories are the common case -- most of a run's machines are idle
-- most of the time -- and a row carrying `"input":{},"output":{},"fuel":{}`
-- spends forty bytes saying nothing. Absence and emptiness mean the same thing
-- here and there is no third state, so collapsing them loses nothing; the Rust
-- and Python readers both default a missing key to an empty map.
local function nonempty_counts(counts)
	if counts == nil or next(counts) == nil then
		return nil
	end
	return counts
end

-- The key a machine is filed under, in the sample map and in
-- `storage.machine_output`. `unit_number` is nil for a handful of entity
-- kinds; none of the sampled types is one of them, but a key collision would
-- silently drop a machine (or merge two machines' lifetime counters), so the
-- fallback is a position that cannot collide rather than a guess.
-- `unit_number` is globally unique across surfaces, so the primary key needs
-- no qualification. The FALLBACK does: it is `name@x,y`, which collides across
-- surfaces exactly as `resource_key` did.
local function machine_key(entity)
	local key = entity.unit_number
	if key == nil then
		key = entity.surface.name .. "|" .. entity.name
			.. "@" .. entity.position.x .. "," .. entity.position.y
	end
	return tostring(key)
end

-- Items one completed craft of `recipe` yields.
--
-- `products_finished` counts **crafts**, not items, and the two differ for
-- every recipe with a yield above one -- `copper-cable` is 2. A run that
-- summed `products_finished` and compared it to `production.made` for cables
-- would be short by half and look like a lost-output bug.
--
-- The main product when the prototype names one, else the sum over products:
-- no recipe this project plans has more than one product, and summing is the
-- honest fallback rather than picking the first arbitrarily. `amount` is nil
-- for a ranged product, where the midpoint is the only defensible guess and
-- the result stops being exact -- said here rather than hidden.
local function recipe_yield(recipe)
	if recipe == nil then
		return 1
	end
	local function amount_of(product)
		if product.amount ~= nil then
			return product.amount
		end
		if product.amount_min ~= nil and product.amount_max ~= nil then
			return (product.amount_min + product.amount_max) / 2
		end
		return 1
	end
	local prototype = recipe.prototype
	if prototype ~= nil and prototype.main_product ~= nil then
		return amount_of(prototype.main_product)
	end
	local total = 0
	for _, product in pairs(recipe.products or {}) do
		total = total + amount_of(product)
	end
	if total <= 0 then
		return 1
	end
	return total
end

-- Lifetime per-machine item counts.
--
-- `storage.machine_output[key] = { produced = <items>, crafts = <last
-- products_finished>, target = <resource unit_number>, amount = <that
-- resource's last seen amount> }`. Not cleared by `sampling_start`: the count
-- is the machine's lifetime, and an interval's production is the difference
-- between two samples, which the analysis takes. It IS cleared by
-- `session_reset`, alongside every other cross-run leftover.
local function machine_output_state(key)
	storage.machine_output = storage.machine_output or {}
	local state = storage.machine_output[key]
	if state == nil then
		state = { produced = 0 }
		storage.machine_output[key] = state
	end
	return state
end

-- A crafting machine's lifetime item count, from the game's own craft counter.
--
-- Read on the machine-sample beat rather than per tick, which is sound because
-- `products_finished` is monotonic and never resets: the only thing a coarser
-- beat costs is the assumption that the recipe did not change *within* one
-- 300-tick window, and a recipe change is a deliberate `set_recipe` action.
--
-- A machine first seen with a non-zero counter (a resumed savepoint, or a
-- placement more than one beat before the first sample) has its whole backlog
-- credited to the recipe it is set to now. That is the only attribution
-- available and it is stated here rather than assumed away.
local function crafting_machine_produced(entity, key, finished, recipe)
	local state = machine_output_state(key)
	local previous = state.crafts or 0
	if finished > previous then
		state.produced = state.produced + (finished - previous) * recipe_yield(recipe)
	end
	-- Never below: `products_finished` cannot fall, and if a Factorio version
	-- ever made it, re-basing without crediting anything is the safe answer.
	state.crafts = finished
	return state.produced
end

-- How often the tracked-drill registry is rebuilt, in ticks.
--
-- The accumulator below has to run every tick (see `track_mining_drills`), but
-- *finding* the drills does not: a `find_entities_filtered` per tick over a
-- charted surface is a different order of cost from a dozen attribute reads.
-- One second is the window in which a freshly placed drill is untracked, and
-- it cannot cost an item: a burner drill mines 0.25 items/s and a drill mines
-- nothing at all until a bot has fuelled or powered it, which is a later
-- action than placing it.
local DRILL_REGISTRY_REFRESH = 60

-- The identity of a resource tile.
--
-- **`unit_number` is nil on a resource entity** -- measured against a live
-- 2.1.17 server, not assumed: `d.mining_target.unit_number` comes back nil
-- while `.amount` reads 290. The first version of this accumulator keyed the
-- drill's target by `unit_number` and therefore never credited a single item;
-- it reported 0 for a drill that had just mined 133 iron ore, and every stub
-- test passed because the fixture gave its resource a unit number the game
-- does not.
--
-- Position is the identity that exists. Resources sit at tile centres and one
-- tile holds one resource entity, so `name@x,y` cannot collide.
-- SURFACE-QUALIFIED, because a tile coordinate is not a place.
--
-- This used to be `name@x,y` and carried a comment asserting that one tile
-- holds one resource entity so the key cannot collide. True **per surface**,
-- false across surfaces, and a reader would have believed it: iron ore at
-- (10, 10) on Nauvis and iron ore at (10, 10) on Vulcanus are different tiles
-- with the same key, so one drill's accumulator would count the other's ore.
--
-- Space Age is enabled in this workspace (`workspace/mods/mod-list.json`), so
-- the only thing standing between here and that collision is one `if` in
-- `on_chunk_generated`. See
-- docs/superpowers/notes/2026-09-06-surfaces-survey.md.
local function resource_key(resource)
	return resource.surface.name .. "|" .. resource.name
		.. "@" .. resource.position.x .. "," .. resource.position.y
end

-- Every resource tile in a drill's reach, and its amount, recorded once when
-- the drill is first tracked.
--
-- **This is what the accumulator watches -- not the drill's `mining_target`.**
-- Three live measurements got it here, each one a smaller error than the last:
--
--   1. Keyed on `mining_target.unit_number`: 0 counted against 133 mined,
--      because a resource entity has no unit number.
--   2. Keyed on the target's position, compared against last tick's target:
--      120 of 133. A drill works several tiles and switches between them, and
--      each switch discards the item mined at the switch.
--   3. Per-tile memory plus a primed baseline, still reading only the current
--      target: 130 of 133. A drill can mine a tile and turn away in the same
--      tick, so that tile's last item is unseen until it is pointed at again
--      -- and if the drill stops for good (no fuel), it never is. One item per
--      tile in the area, permanently, exactly the three that were missing.
--
-- Reading every tile in the area instead costs four attribute reads per drill
-- per tick rather than one, on a set fixed at registration (a burner drill
-- covers 4 tiles, the biggest 25) -- against a mod that already walks every
-- bot's whole inventory every tick. It leaves nothing in flight.
--
-- One query per drill, once (`primed`), not per tick: the entity references
-- are kept, so the per-tick work is `resource.amount` and nothing else.
local function prime_drill_amounts(entity, key)
	local state = machine_output_state(key)
	if state.primed then
		return
	end
	state.primed = true
	state.amounts = state.amounts or {}
	state.tiles = {}
	local area = entity.mining_area
	if area == nil then
		return
	end
	for _, resource in pairs(entity.surface.find_entities_filtered({
		area = area, type = "resource",
	})) do
		if resource.valid then
			local tile = resource_key(resource)
			state.tiles[tile] = resource
			state.amounts[tile] = resource.amount
		end
	end
end

-- Rebuild the tracked set. Kept in `storage`, not in a file-local, because a
-- file-local is rebuilt at a different moment on a peer that joined mid-game
-- and the accumulator writes to `storage` -- which is the shape of a desync.
local function refresh_drill_registry()
	local registry = {}
	for _, surface in pairs(game.surfaces) do
		for _, entity in pairs(surface.find_entities_filtered({
			type = "mining-drill", force = game.forces["player"],
		})) do
			if entity.valid then
				local key = machine_key(entity)
				registry[key] = entity
				prime_drill_amounts(entity, key)
			end
		end
	end
	storage.drill_registry = registry
end

-- Per-drill production, accumulated, because Factorio counts nothing for a
-- mining drill.
--
-- **What was measured, not remembered.** Factorio 2.1.17's `runtime-api.json`
-- declares exactly three `MiningDrill` members on `LuaEntity` --
-- `mining_area`, `mining_drill_filter_mode` and `mining_target`. There is no
-- `mining_progress` attribute in this API version at all (only
-- `bonus_mining_progress`, which is the productivity bar), no drill inventory
-- define (`defines.inventory` has `mining_drill_modules` and nothing else),
-- and no drill-mined event (`on_player_mined_*`, `on_robot_mined_*` and
-- `on_resource_depleted` are the whole list). So neither a progress wrap nor
-- an output-inventory delta is available to count with, and the resource's own
-- `amount` is what remains.
--
-- **The rule.** One unit of a resource entity's `amount` is one item mined, so
-- a decrease in the amount of the tile a drill is pointed at is that drill's
-- output. Read every tick, not on the sample beat, because the tile a drill is
-- working changes when it depletes and the decrease is invisible afterwards.
--
-- **What it is exact about, and what it is not:**
--
--   * A single drill on a finite patch is exact. The one item that would be
--     lost -- the mine that takes the tile from 1 to 0 and destroys it, after
--     which no `amount` can be read -- is credited from
--     `on_resource_depleted`, which fires with the entity still identifiable.
--   * **Two drills whose areas overlap on the same tile both see the same
--     decrease and both take credit.** Their individual numbers are then upper
--     bounds and their sum double-counts. This is detected rather than
--     hidden: a drill sharing its target with another tracked drill in the
--     same tick is flagged, and the flag reaches the record as
--     `produced_shared`.
--   * **An infinite resource (crude oil) never decreases below its minimum
--     yield**, so a pumpjack's output is not countable this way at all. Such a
--     drill reports `produced_source = "unavailable"` and no count, rather
--     than a zero that reads like "produced nothing".
--   * **Mining productivity yields items without consuming the resource**, so
--     with that research the count is a lower bound. No run of this project
--     has researched it; when one does, this comment is the thing to revisit.
--   * Accumulation runs only while a sampling session is open, which is the
--     whole of a recorded run and starts before any drill is placed.
function track_mining_drills(tick)
	if storage.sampling == nil then
		return
	end
	if storage.drill_registry == nil or tick % DRILL_REGISTRY_REFRESH == 0 then
		refresh_drill_registry()
	end
	-- First pass: how many tracked drills reach each tile. Two drills whose
	-- areas overlap both see the same fall and both take credit, so their
	-- individual numbers are upper bounds -- known before any credit is given,
	-- and flagged on the rows it affects rather than silently summed.
	local sharers = {}
	for key, entity in pairs(storage.drill_registry) do
		if not entity.valid then
			storage.drill_registry[key] = nil
		else
			for tile in pairs(machine_output_state(key).tiles or {}) do
				sharers[tile] = (sharers[tile] or 0) + 1
			end
		end
	end
	-- Second pass: every tile in every tracked drill's area, not just the one
	-- the drill is pointed at. See `prime_drill_amounts` for why -- the
	-- pointer moves in the same tick as the mine, so a pointer-only reading
	-- leaves one item per tile permanently uncounted.
	for key in pairs(storage.drill_registry) do
		local state = machine_output_state(key)
		state.amounts = state.amounts or {}
		for tile, resource in pairs(state.tiles or {}) do
			if not resource.valid then
				-- Mined out: whatever was left on it when last read went into
				-- this drill. `on_resource_depleted` normally gets there
				-- first; this is the backstop for a tile that vanished
				-- without one.
				local remaining = state.amounts[tile]
				if remaining ~= nil and remaining > 0 then
					state.produced = state.produced + remaining
				end
				state.amounts[tile] = nil
				state.tiles[tile] = nil
			else
				local amount = resource.amount
				local previous = state.amounts[tile]
				if previous ~= nil and amount < previous then
					state.produced = state.produced + (previous - amount)
					if (sharers[tile] or 0) > 1 then
						state.shared = true
					end
				end
				state.amounts[tile] = amount
			end
		end
	end
end

-- The last item of a depleted tile, which no `amount` read can see.
--
-- `on_resource_depleted` fires with the resource at zero and still
-- identifiable, so every drill that was pointed at it is credited with what it
-- last read -- normally 1. Without this a run loses one item per depleted tile
-- per drill, which is small and systematic, and systematic is the kind of
-- error that ends up quoted.
function on_resource_depleted(event)
	if storage.sampling == nil or storage.drill_registry == nil then
		return
	end
	local entity = event.entity
	if entity == nil or not entity.valid then
		return
	end
	local unit = resource_key(entity)
	for key in pairs(storage.drill_registry) do
		local state = machine_output_state(key)
		local remaining = (state.amounts or {})[unit]
		-- Whatever this drill last saw on that tile is what it went on to mine
		-- out of it. Keyed by tile rather than by "the tile it is pointed at
		-- now", because a drill depletes one tile and moves on within the same
		-- tick, and the event arrives after it has moved.
		if remaining ~= nil and remaining > 0 then
			state.produced = state.produced + remaining
		end
		if state.amounts ~= nil then
			state.amounts[unit] = nil
		end
	end
end

-- What the row reports for a drill: the accumulated count and how it was got.
local function drill_produced(entity, key)
	local state = machine_output_state(key)
	local target = entity.mining_target
	if state.produced == 0 and target ~= nil and target.valid
		and target.prototype.infinite_resource then
		return nil, "unavailable", nil
	end
	return state.produced, "accumulated", state.shared
end

-- One machine's row.
--
-- Everything read here is either `subclasses: None` on `LuaEntity` (so safe on
-- any entity) or gated on `CRAFTING_MACHINE_TYPES` / an explicit type test.
-- That gating is not defensive style, it is the difference between a sample
-- and a dead server: `LuaEntity.crafting_progress` is declared for
-- `CraftingMachine` only, exactly like the `mining_target` read that took a
-- live run down from inside `sample_bots`.
local function machine_row(entity, key)
	local row = {
		name = entity.name,
		type = entity.type,
		position = entity.position,
	}
	local status = entity.status
	if status ~= nil then
		-- Name, not number -- see `entity_status_name` in types.lua, which is
		-- the one place this mod inverts `defines.entity_status`. An id this
		-- build cannot name still reaches the record, labelled as unresolved,
		-- rather than being written as a bare integer nobody can decode later.
		row.status = entity_status_name(status)
	end
	-- Nil for anything not wired to a network at all, which is itself the
	-- answer to "was the pole actually connected". Matched against the
	-- `sub_ids` of `power.networks` on the same tick's force sample.
	row.network = entity.electric_network_id
	if CRAFTING_MACHINE_TYPES[entity.type] then
		-- `get_recipe()` is the only honest verdict on what a machine is set
		-- to: `set_recipe` returns the items it *removed*, not a success flag,
		-- so a machine that ignored the call reads as configured everywhere
		-- except here.
		local recipe = entity.get_recipe()
		if recipe ~= nil then
			row.recipe = recipe.name
		end
		row.crafting = entity.is_crafting()
		-- Rounded to a thousandth: the raw double serialises to seventeen
		-- digits of noise, and no question anyone asks of this record needs
		-- more than three.
		row.progress = math.floor(entity.crafting_progress * 1000 + 0.5) / 1000
		-- The blunt instrument, and often the fastest one: a cell whose
		-- assemblers report `products_finished = 0` after twenty minutes did
		-- not produce, whatever else the row says.
		row.products_finished = entity.products_finished
		-- The owner's counter: lifetime ITEMS, not crafts. `products_finished`
		-- above is the raw game value and stays; `produced` is that value
		-- accumulated against each craft's yield, so a copper-cable assembler
		-- reads 204 next to a `products_finished` of 102.
		row.produced = crafting_machine_produced(entity, key, row.products_finished, recipe)
		row.produced_source = "game"
		row.input = nonempty_counts(machine_inventory(entity, CRAFTER_INPUT_INVENTORY))
	elseif entity.type == "lab" then
		row.input = nonempty_counts(machine_inventory(entity, LAB_INPUT_INVENTORY))
		-- A lab consumes science and emits research, never an item, so it has
		-- no lifetime item count and never will. Said in the row: a reader
		-- must be able to tell "makes nothing" from "the counter is missing".
		row.produced_source = "not-a-producer"
	elseif entity.type == "mining-drill" then
		-- Which patch it is on, so `no_minable_resources` can be told from a
		-- drill that was never placed over ore at all.
		local target = entity.mining_target
		if target ~= nil and target.valid then
			row.mining = target.name
		end
		row.produced, row.produced_source, row.produced_shared = drill_produced(entity, key)
	elseif CONTAINER_TYPES[entity.type] then
		-- Reported as `output`, not `input`: from the run's point of view a
		-- chest is a thing the cell *draws from*, and putting it in the same
		-- field as a furnace's result keeps "what is in this thing" one key
		-- rather than two that mean the same and differ by entity type.
		row.output = nonempty_counts(machine_inventory(entity, defines.inventory.chest))
	end
	-- Both `subclasses: None`, both nil-answering: an entity with no output or
	-- no fuel inventory returns nil rather than raising, so both are checked
	-- rather than assumed present.
	if row.output == nil then
		local output = entity.get_output_inventory()
		if output ~= nil and output.valid then
			row.output = nonempty_counts(inventory_counts(output))
		end
	end
	local fuel = entity.get_fuel_inventory()
	if fuel ~= nil and fuel.valid then
		row.fuel = nonempty_counts(inventory_counts(fuel))
	end
	-- Everything the branches above did not claim: a boiler makes steam, a
	-- steam engine makes electricity, a chest makes nothing. None of them puts
	-- an item into `production.made`, so none of them has a lifetime item
	-- count -- and the row says that in a field rather than by omitting one,
	-- because "makes nothing" and "the counter is missing" are different
	-- answers and an absent key cannot tell them apart.
	if row.produced_source == nil then
		row.produced_source = "not-a-producer"
	end
	return row
end

-- Per-machine state on the 300-tick beat, alongside the force sample.
--
-- Shares the force cadence rather than inventing a third one: a machine's
-- status, recipe and network membership change on the scale of a bot walking
-- somewhere, not of a tick, and the ingredient counts that *do* move fast are
-- readable from the bot beat's chest and inventory data anyway.
--
-- **Cost.** One `find_entities_filtered` per surface per 300 ticks with a
-- six-member type filter, which is the same shape and the same beat as the
-- `electric-pole` query `power_totals` has always made -- so this doubles an
-- existing per-sample cost rather than introducing a new kind of one. Per
-- machine it is roughly a dozen attribute reads and up to three inventory
-- walks.
--
-- Measured against the base `run-1788459085-32452` actually built (66 stone
-- furnaces, 6 assembling machines, 2 burner drills, a boiler, a steam engine
-- and a lab -- 77 machines): under a thousand API reads every five seconds,
-- amortising to about three per tick, and **14.8 KiB of JSON per sample**.
-- Across that run's 247 force samples that is **3.6 MiB** on the wire, and
-- about 4.3 MiB in the archive once Rust restores the empty inventories the
-- rows here omit -- against the 1.06 MiB `samples.jsonl` the run wrote
-- without this. So it roughly quadruples the record.
--
-- That is the honest number and it is worth paying. The comparison that
-- settles it is the feature this one replaces in the budget: the per-camera
-- screenshots retired on 2026-09-02 cost 947 MB for 45 minutes *and* rendered
-- synchronously inside the game loop, once per camera per capture. This costs
-- three thousandths of that and touches no renderer. If a base ever grows to
-- where 4 MiB is the wrong trade, `MACHINE_SAMPLE_LIMIT` is the dial, and it
-- reports what it cut.
--
-- `machines` is a map keyed by `unit_number`, not an array, for the reason
-- given on `power_totals.networks`: `helpers.table_to_json` cannot tell an
-- empty array from an empty object, and a run's first minutes legitimately
-- have no machines at all. As a map, "nothing yet" is `{}` and parses.
local function sample_machines_body(tick)
	local session = storage.sampling
	if session == nil then
		return
	end
	local force = game.forces["player"]
	local machines = {}
	local seen, written = 0, 0
	for _, surface in pairs(game.surfaces) do
		for _, entity in pairs(surface.find_entities_filtered({
			type = MACHINE_TYPES, force = force,
		})) do
			if entity.valid then
				seen = seen + 1
				if written < MACHINE_SAMPLE_LIMIT then
					written = written + 1
					-- See `machine_key`: the same key files this machine's
					-- lifetime counters in `storage.machine_output`, so the
					-- two must be derived in one place and not twice.
					local key = machine_key(entity)
					machines[key] = machine_row(entity, key)
				end
			end
		end
	end
	write_sample({
		kind = "machines",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		run = session.run,
		machines = machines,
		-- Zero on every normal run. Non-zero says "this line is the first
		-- MACHINE_SAMPLE_LIMIT of a larger base", which a reader must be able
		-- to distinguish from "this is all of it".
		truncated = seen - written,
	})
end

-- `pcall` wrapper, for the reason spelled out above `record_sample_failure`:
-- a bad attribute read in a sampler must cost one sample, not the server.
local function sample_machines(tick)
	local ok, err = pcall(sample_machines_body, tick)
	if ok then
		record_sample_success("machines")
	else
		record_sample_failure("machines", tick, err)
	end
end

-- Registered with `script.on_nth_tick` rather than as a modulus inside
-- `on_tick`: `on_tick` already runs real per-tick work for every client, and a
-- counter or a remainder in there would both add to that and reintroduce the
-- caller-side notion of cadence this exists to remove.
--
-- Multiplayer: this handler runs on every peer with the same replicated
-- `storage.sampling`, so every peer agrees on whether a session is running.
-- Only the server actually writes (`write_sample`). The gate is deliberately
-- in `storage` and not in `client_local_data`, which the top of this file
-- marks as desync-causing.
function on_sample_force_tick(event)
	if storage.sampling == nil then
		return
	end
	-- `game.tick` rather than `event.tick`: they are the same value here, and
	-- reading the one `stamp_tick` reads keeps a single source of "now".
	local tick = game.tick
	sample_force(tick)
	-- Called from here rather than registered separately, because
	-- `script.on_nth_tick(300, ...)` *replaces* the handler for 300 instead of
	-- adding to it -- a second registration would silently disable the force
	-- sampler, which is the trap `SAMPLE_BOT_INTERVAL` is 60 to avoid.
	--
	-- Same tick as the force sample, deliberately: `machines[*].network` is
	-- only meaningful against the `power.networks` written on the same tick,
	-- and a reader joining the two should never have to interpolate.
	sample_machines(tick)
end

-- `run_id` is an opaque tag for this session, stamped onto every sample line
-- and used for nothing else here.
--
-- Deliberately uninterpreted. The caller passes a job id, but this mod must
-- never learn that: it does not parse it, validate its shape, derive a
-- filename from it or compare it to anything. Both sides then hold the same
-- identifier while only the caller knows what it identifies. That ignorance is
-- the design -- a mod that understood the id would have to be changed every
-- time the caller's notion of a run changed.
--
-- Omitting it is a real choice, not a degraded one: a session nobody needs to
-- correlate simply writes no `run` key at all, and a consumer that finds none
-- knows it cannot tell rather than being told something false.
--
-- Starting a session is what produces `samples.jsonl` -- research, production,
-- per-network power, bot inventories and per-machine state. `sample_force` and
-- `sample_machines` (the 300-tick beat) and `sample_bots` (the 60-tick one)
-- all return early when `storage.sampling` is nil.
function rcon_sampling_start(run_id)
	-- Checked before anything is written, so a call this function is going to
	-- refuse cannot first truncate the previous run's samples. The check is on
	-- the *type* only -- reading the value would be interpreting it.
	if run_id ~= nil and type(run_id) ~= "string" then
		error("sampling run id must be a string or absent, got " .. type(run_id))
	end
	-- A fresh start: server-only, since only the server's copy exists to
	-- begin with, and `append = false` truncates the file rather than
	-- appending to whatever a previous run left in it.
	helpers.write_file(SAMPLE_FILE, "", false, 0)
	-- `run` is set only from the argument -- nil when the caller passed none.
	-- No fallback, no `run_id or storage.something`: `run = session.run` with
	-- a nil value omits the key entirely from the JSON line rather than
	-- writing it as null, so an untagged run's samples have no `run` key at
	-- all, and Rust falls back to tick-range filtering for them
	-- (`#[serde(default)]` treats an absent key the same as an explicit null).
	storage.sampling = { run = run_id }
	stamp_tick()
end

function rcon_sampling_stop()
	storage.sampling = nil
	stamp_tick()
end

-- Ask the server to write the world out under `name`, producing
-- `<instance>/saves/<name>.zip`.
--
-- **The name is required, and that is a safety property rather than a
-- convenience.** `game.server_save()` with no argument "overwrites the
-- currently running save" -- which for every instance this project starts is
-- `saves/level.zip`, the map every measurement is taken on and, in this
-- workspace, the only copy of a map whose seed was never recorded. There is
-- deliberately no way to reach the nameless form through this interface.
--
-- The pattern is what keeps the save inside the saves directory: Factorio
-- treats the name as a path relative to it, so a `/` or a `..` would escape.
-- Refused by name rather than sanitised, because a caller that passed a path
-- meant something by it and quietly saving somewhere else is worse than an
-- error.
--
-- **The save is not written when this returns.** The engine writes at the end
-- of the tick, into `<name>.tmp.zip`, and renames that onto `<name>.zip` when
-- it is done. Whoever asked has to watch the file system to find out; see
-- `crates/core/src/record/savepoint.rs`, which is the only caller and does
-- exactly that.
function rcon_savepoint(name)
	if type(name) ~= "string" or name == "" then
		error("savepoint name must be a non-empty string, got " .. type(name))
	end
	if not name:match("^[A-Za-z0-9_%-]+$") then
		error("savepoint name must be [A-Za-z0-9_-] only, got \"" .. name .. "\"")
	end
	game.server_save(name)
	stamp_tick()
end

-- Forget everything in `storage` that belonged to the *run* rather than to the
-- *world*, and report what was dropped.
--
-- # Why this has to exist
--
-- A save carries `script.dat`, which is this mod's `storage`. Factorio runs
-- migrations only when a mod's version changes, and `info.json` here is pinned
-- at 0.0.1 precisely so that it does not -- so a world resumed from a
-- savepoint arrives with the previous run's in-flight state fully intact and
-- nothing anywhere says so.
--
-- That state is not inert. `craft_actions` and `research_actions` are indexed
-- by the caller's `ActionId`, and those are minted `% 1000` from zero at the
-- start of every run (`FactorioRcon`, `crates/core/src/factorio/rcon.rs`). A
-- craft waiter left over from the run that took the save therefore does not
-- merely linger: the *next* run's action 7 is settled by the *previous* run's
-- action 7, silently and with a plausible-looking success.
--
-- # What is kept
--
-- `storage.resources`, `storage.map_area` and `storage.pathfinding` describe
-- the map, which is the whole reason for resuming from a save. They stay.
--
-- `storage.p` entries are kept but emptied: `get_player` uses the presence of
-- a key as "this player is known", so deleting the table would make every bot
-- unknown until it happened to be re-created by the tick handler.
--
-- `storage.n_clients` is deliberately left alone even though it *is*
-- run-scoped and does drift upward across a resume (it counts joins and
-- leaves, and a resumed world starts from the count the save was taken at).
-- Nothing reads it: `on_load` copies it into `my_client_id`, whose only
-- surviving use is one debug `print` -- the three `if my_client_id ~= 1`
-- guards below it are commented out. Resetting it would look like a fix and
-- change nothing, and the honest note is worth more than the line of code.
function rcon_session_reset()
	local dropped = { walking = 0, mining = 0, crafts = 0, research = 0 }
	for _, p in pairs(storage.p or {}) do
		if p.walking ~= nil then
			dropped.walking = dropped.walking + 1
			p.walking = nil
		end
		if p.mining ~= nil then
			dropped.mining = dropped.mining + 1
			p.mining = nil
		end
	end
	for _, per_player in pairs(storage.craft_actions or {}) do
		for _, bucket in pairs(per_player) do
			dropped.crafts = dropped.crafts + #bucket
		end
	end
	for _, waiting in pairs(storage.research_actions or {}) do
		dropped.research = dropped.research + #waiting
	end
	storage.craft_actions = {}
	storage.research_actions = {}
	-- The sampling session carries the *previous* run's id, and every sample
	-- line written before the new run calls `sampling_start` would be stamped
	-- with it. Stopping is right even though `sampling_start` would overwrite
	-- it: between load and that call there are ticks, and a sample written in
	-- them would belong to a run that ended.
	storage.sampling = nil
	storage.telemetry_failures = nil
	storage.telemetry_failing = nil
	-- Lifetime per-machine counters, and the drill set they are accumulated
	-- over. Cleared for the same reason the sampling session is: they are
	-- keyed by `unit_number`, and a reset precedes a *different* world whose
	-- unit numbers mean something else. A resumed run re-derives a crafting
	-- machine's total from `products_finished`, which the game itself kept.
	storage.machine_output = nil
	storage.drill_registry = nil
	-- The tick stamp first, as every call answers with; then the counts, which
	-- are the point. A reset that dropped nothing is the expected answer on a
	-- fresh world and the interesting one on a resumed world -- reporting it
	-- either way is what makes the difference visible.
	stamp_tick()
	rcon.print(helpers.table_to_json(dropped))
end

-- There is deliberately no settle-triggered sample here (an earlier
-- `rcon_sample_bots()` called `sample_bots(game.tick)` on demand, but nothing
-- ever called it -- it was not even reachable via the `botbridge` remote
-- interface -- so it was removed rather than kept as unreachable shape). The
-- 60-tick beat (`sample_bots` above, `on_nth_tick(60)`) already bounds
-- staleness at a failure to one second of game time, which is the beat the
-- bots themselves move on, so a failure-triggered sample would not learn
-- anything the next beat does not already carry.

-- Does a character collide with this tile, keyed by tile name.
--
-- Collision is a property of the tile *prototype* and a tile's name names its
-- prototype exactly, so one engine call per distinct name gives the same answer
-- as one per tile. That matters: `writeout_tiles` runs over a whole 32x32 chunk
-- and its own comment already calls it SLOW, so asking per tile would add 1024
-- crossings of the mod/engine boundary per chunk to the function least able to
-- afford them. Prototypes cannot change at runtime, so the cache never goes
-- stale; it is rebuilt from scratch on load because it is not in `storage`,
-- which is what we want.
local tile_player_collides = {}

function player_collides_with_tile(tile)
	local cached = tile_player_collides[tile.name]
	if cached == nil then
		-- Factorio 2.0 renamed every collision layer, dropping the `-layer`
		-- suffix: `player-layer` became `player`. The old name is not ignored,
		-- it raises ("Unknown collision-layer name: player-layer") -- which is
		-- what the TODO that used to stand here was worked around by
		-- hardcoding `0`, so every tile in every owned run came out walkable
		-- and no lake ever entered EntityGraph's blocked tree.
		-- `types.lua`'s serialize_tile has asked for it correctly all along;
		-- this is the stdout transport catching up with the RCON one.
		cached = tile.collides_with('player')
		tile_player_collides[tile.name] = cached
	end
	return cached
end

function writeout_tiles(tick, surface, area) -- SLOW! beastie can do ~2.8 per tick
	--if my_client_id ~= 1 then return end
	local header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..": "
	local tile = nil
	local line = {}
	for y = area.left_top.y, area.right_bottom.y-1 do
		for x = area.left_top.x, area.right_bottom.x-1  do
			tile = surface.get_tile(x,y)
			table.insert(line, tile.name .. (player_collides_with_tile(tile) and ":1" or ":0"))
		end
	end
	writeout(tick, "tiles", header .. table.concat(line, ","))
end

function writeout_resources(tick, surface, area) -- quite fast. beastie can do > 40, up to 75 per tick
	--if my_client_id ~= 1 then return end
	header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..": "
	line = ''
	lines={}
	for idx, ent in pairs(surface.find_entities_filtered{area=area, type='resource'}) do
		line=line..","..ent.name.." "..ent.position.x.." "..ent.position.y
		if idx % 100 == 0 then
			table.insert(lines,line)
			line=''
		end
	end
	table.insert(lines,line)
	writeout(tick, "resources", header ..  table.concat(lines,""))
	line=nil
end

function direction_str(d)
	if d == defines.direction.north then
		return "N"
	elseif d == defines.direction.east then
		return "E"
	elseif d == defines.direction.south then
		return "S"
	elseif d == defines.direction.west then
		return "W"
	else
		return "X"
	end
end

-- Entities `EntityGraph::add` throws away the instant they arrive.
--
-- `crates/core/src/graph/entity_graph.rs` opens its ingest loop with
--
--     if entity.entity_type == EntityType::FlyingText.to_string()
--         || entity.entity_type == EntityType::Fish.to_string()
--         || entity.bounding_box.width() == 0.
--     { continue; }
--
-- so these reach no quad tree, no `minables`, no `threats` and no petgraph
-- node. Serialising them is pure cost, and it is the ONLY "should not be sent"
-- category that can be stated as a fact rather than a preference -- trees,
-- rocks and cliffs all land in `blocked_tree` and the planner sites blocks
-- against them, so they stay.
--
-- Measured against a real run's `workspace/server-log.txt` (seed 31337, 1,424
-- chunks, 50,256 entity records, 10.5 MB of `entities` lines): **1,574 records
-- and 2.59% of the bytes** were discarded on arrival, essentially all of them
-- fish. On an endgame base the same predicate also covers every remnant,
-- corpse and particle source, which a finished factory has in quantity and a
-- fresh map has none of.
--
-- **Kept in step with the Rust gate by a test, not by care**:
-- `crates/core/tests/botbridge_bulk_entities.rs` names both types and the
-- zero-width rule. If the gate there changes, change this with it.
local INGEST_DISCARDS = { ["flying-text"] = true, ["fish"] = true }

function writeout_entities(tick, surface, area)
	--if my_client_id ~= 1 then return end
	local header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..":"
	local objects = {}
	for idx, ent in pairs(surface.find_entities(area)) do
		if ent.type ~= "character" and area.left_top.x <= ent.position.x and ent.position.x < area.right_bottom.x and area.left_top.y <= ent.position.y and ent.position.y < area.right_bottom.y then
			local bb = ent.bounding_box
			local zero_width = bb ~= nil and bb.right_bottom.x - bb.left_top.x == 0
			if not INGEST_DISCARDS[ent.type] and not zero_width then
				-- `omit_inventories`: identity and geometry in bulk, contents
				-- on demand. See `serialize_entity`.
				table.insert(objects, serialize_entity(ent, { omit_inventories = true }))
			end
		end
	end
	writeout(tick, "entities", header .. helpers.table_to_json(objects))
	line=nil
end

-- Stamps the game tick onto an RCON response.
--
-- Every RPC the executor dispatches ends with this line, so a caller can record
-- *when the game actually saw the command* instead of estimating it from the
-- schedule. It costs no extra round trip: the reply was already being sent.
--
-- The sentinel is what keeps the stamp out of the payload. `§tick§` cannot
-- occur in a JSON body or in any message `complain` builds, and the Rust client
-- (`crates/core/src/factorio/rcon.rs`, `take_tick_stamp`) strips these lines
-- before judging the rest -- so a response that used to be empty on success is
-- still empty, and one that carried a single JSON document still carries
-- exactly one. Adding a bare `rcon.print` here instead would be read as the
-- action's result and turn every success into a reported failure.
--
-- `game.tick` is a MapTick (uint64). It is printed whole; the narrowing to the
-- planner's 32-bit `Ticks` happens once, on the Rust side, where it can be
-- reported as absent rather than wrapped.
--
-- Safe outside an RCON command. `rcon_action_start_walk_waypoints` is also
-- called from `on_tick` (the "not mining the expected target, MOVING!" branch),
-- where there is no calling RCON interface; the API defines `rcon.print` as
-- printing "to the calling RCON interface *if any*", so that path stamps
-- nowhere rather than erroring out of the tick handler.
function stamp_tick()
	rcon.print("§tick§" .. game.tick)
end

function writeout(tick, key, value)
	print("§"..tick.."§"..key.."§"..tostring(value))
end

function rangestr(area)
	return coord({x=area.x1, y=area.y1}).." -- "..coord({x=area.x2, y=area.y2})
end

function coord(pos)
	return "("..pos.x.."/"..pos.y..")"
end

function on_load()
	my_client_id = storage.n_clients
end

-- End a cutscene the player is in, so it has a character to be seen by.
--
-- The safety net behind `disable_crashsite`, and the only one of the two that
-- works on a save created before this landed: `set_disable_crashsite` prevents
-- a cutscene, `exit_cutscene` ends one. See `disable_crashsite` for what the
-- cutscene costs.
--
-- Guarded on `controller_type`, and that guard is load-bearing rather than
-- defensive: `LuaPlayer::exit_cutscene` "Errors if not in a cutscene", so an
-- unguarded call would raise on every join of every bot on every run.
-- Freeplay's own `skip_crash_site_cutscene` guards it the same way. Exiting
-- raises `on_cutscene_cancelled`, which is where freeplay restores the
-- character's destructibility and clears the skip label, so this ends the
-- cutscene the same way pressing the skip button does.
function exit_cutscene_if_any(player_index)
	local player = game.players[player_index]
	if player == nil then
		return
	end
	if player.controller_type == defines.controllers.cutscene then
		print("ending the crash-site cutscene for player " .. tostring(player_index))
		player.exit_cutscene()
	end
end

function on_player_joined_game(event)
	if has_character_bots() then
		local joined = game.players[event.player_index]
		print("ERROR: player " .. joined.name .. " joined a world with character bots; a run is all clients or all characters")
		joined.print("This run uses character bots; clients are refused.")
		return
	end
--	print("player '"..game.players[event.player_index].name.."' joined")
--	game.write_file("players_connected.txt", game.players[event.player_index].name..'\n', true, 0) -- only on server
	storage.n_clients = storage.n_clients + 1
	exit_cutscene_if_any(event.player_index)
	wait_for_player_inventory(event)

	if client_local_data.whoami == "client1" then
		for chunk_y=-512,512,32 do
			for chunk_x=-512,512,32 do
				chunk_screenshot(chunk_x, chunk_y)
			end
		end
		for chunk_y=-512,512,256 do
			for chunk_x=-512,512,256 do
				chunk_screenshot2(chunk_x, chunk_y)
			end
		end
	end
end

function wait_for_player_inventory(event)
	local player_idx = event.player_index
	local main_inventory = game.players[player_idx].get_main_inventory()

	if main_inventory ~= nil then
		wait_for_player = false
		on_player_main_inventory_changed(event)
		on_player_changed_distance(event)
		on_player_changed_position(event)
	else
		wait_for_player = true
		local use1 = false
	    for k,v in pairs(todo_next_tick_other) do
			use1 = true
		end
		-- the two todo_next_tick queues are necessary to avoid endless loops
		if use1 then
			table.insert(todo_next_tick, function () wait_for_player_inventory(event) end)
		else
			table.insert(todo_next_tick_other, function () wait_for_player_inventory(event) end)
		end
	end
end

function on_player_left_game(event)
	storage.n_clients = storage.n_clients - 1
	local tick = event.tick
	local player_idx = event.player_index
	writeout(event.tick, "on_player_left_game", player_idx)
end

-- A bot died. Say so on stdout, and fail whatever it was doing NOW.
--
-- Two things happen here, and each closes a silence of its own.
--
-- **The writeout** is the record's only source for a death.
-- `output_parser.rs` queues it on `FactorioSurface` and `record.deaths()`
-- (crates/scripting_lua) drains it into `events.jsonl` as `bot_died` -- the
-- same road a teleport takes. `cause` is `event.cause`'s name when the game
-- names one (a worm, a biter, a train), `respawn_in` is `ticks_to_respawn` as
-- it stands the tick after death. Nothing in the 24 archived runs contains a
-- death, so this event has never been produced by a real game; the field
-- shapes are what the 2.1 runtime API documents.
--
-- **The failures** are what stop the executor waiting out its deadline. The
-- per-tick walker and miner are gated on `player.character`, so a walk or a
-- mine in flight when the character vanished is simply *skipped* -- no
-- `action_completed`, no `action_failed` -- until either the respawned
-- character resumes it from the spawn point with a stale path, or the
-- executor's deadline lapses and records `lost`. Both are worse than the
-- truth, which is that the action failed at this tick for this reason. Craft
-- waiters go the same way: a dead character's queue does not finish.
-- `ERROR:` rather than `Error:` because these are `action_failed` verdicts,
-- not reply bodies, and that is the prefix the other verdicts use.
function on_player_died(event)
	local idx = event.player_index
	local player = game.players[idx]
	local cause, cause_type = nil, nil
	if event.cause ~= nil and event.cause.valid then
		cause = event.cause.name
		cause_type = event.cause.type
	end
	local respawn_in = nil
	if player ~= nil then
		local ok, ticks = pcall(function() return player.ticks_to_respawn end)
		if ok and type(ticks) == "number" then respawn_in = math.floor(ticks) end
	end
	local position = nil
	if player ~= nil then
		local ok, pos = pcall(function() return player.position end)
		if ok and pos ~= nil then position = { x = pos.x, y = pos.y } end
	end
	writeout(event.tick, "player_died", helpers.table_to_json({
		player_id = idx,
		position = position,
		cause = cause,
		cause_type = cause_type,
		respawn_in = respawn_in,
	}))

	local why = "ERROR: player " .. tostring(idx) .. " has no character: died at tick "
		.. tostring(event.tick)
	if cause ~= nil then why = why .. " killed by " .. tostring(cause) end
	if respawn_in ~= nil then why = why .. ", respawns in " .. tostring(respawn_in) .. " ticks" end
	print(why)

	fail_bot_actions(idx, event.tick, why)
end

-- Fail everything bot `idx` was doing, now, with `why`: the walk or mine in
-- flight, then every craft it was awaited on and the whole bucket (if the
-- game also raises `on_player_cancelled_crafting` for a lost queue, it must
-- find nothing left to fail a second time). Shared by a player's death and a
-- character bot's.
function fail_bot_actions(idx, tick, why)
	local p = storage.p[idx]
	if p ~= nil then
		if p.walking ~= nil then
			if p.walking.action_id ~= nil then action_failed(tick, p.walking.action_id, why) end
			p.walking = nil
		end
		if p.mining ~= nil then
			if p.mining.action_id ~= nil then action_failed(tick, p.mining.action_id, why) end
			p.mining = nil
		end
	end
	local per_player = craft_actions()[idx]
	if per_player ~= nil then
		for _, waiting in pairs(per_player) do
			for _, waiter in ipairs(waiting) do
				action_failed(tick, waiter.id, why)
			end
		end
		craft_actions()[idx] = nil
	end
end

-- The character is back. Report it, and report where: the cached position on
-- the Rust side is what `RconActuator::walk` steers from, and the last thing
-- it heard was wherever the bot died.
function on_player_respawned(event)
	local idx = event.player_index
	local player = game.players[idx]
	local position = nil
	if player ~= nil and player.character ~= nil then
		local pos = player.character.position
		position = { x = pos.x, y = pos.y }
	end
	writeout(event.tick, "player_respawned", helpers.table_to_json({
		player_id = idx,
		position = position,
	}))
	writeout_player_position(event.tick, idx, player)
end

function on_player_mined_item(event)
--	name = event.item_stack.name
--	count = event.item_stack.count
--	if count == nil then
--		print("count was nil")
--		count = 1
--	end
--	writeout(event.tick, "mined_item", event.player_index.." "..name.." "..count.."\n")
end

function action_completed(tick, action_id)
	writeout(tick, "action_completed", "ok "..action_id)
end

-- The executor reads `reason` as the whole verdict for the action, so a call
-- that omits one used to send it the literal string "nil": a 2026-09-02 run
-- reported `game rejected the command: Unexpected Response: nil` for a mine two
-- bots raced for, and the word said nothing about what had happened. Every call
-- site now passes a reason; this default is what keeps the next one that forgets
-- from being indistinguishable from a real message.
function action_failed(tick, action_id, reason)
	if reason == nil then
		reason = "ERROR: the mod failed this action without saying why"
	end
	writeout(tick, "action_completed", "fail "..action_id .. " " .. tostring(reason))
end

function on_some_entity_created(event)
	local ent = event.entity or event.created_entity or nil
	if ent == nil then
		complain("wtf, on_some_entity_created has nil entity")
		return
	end

	writeout(event.tick, "on_some_entity_created", helpers.table_to_json(serialize_entity(ent)))
	tally_built_entity(ent)

--	if ent.type == "pipe" or ent.type == "pipe-to-ground" or ent.type == "wall" or ent.type == "heat-pipe" then -- HACK to semi-correctly assign an orientation to pipes etc
--		-- need to write out neighboring entities as well, because they might have changed their orientation by this event
--		writeout_objects(event.tick, ent.surface, {left_top={x=math.floor(ent.position.x)-1, y=math.floor(ent.position.y)-1}, right_bottom={x=math.floor(ent.position.x)+2, y=math.floor(ent.position.y)+2}})
--	else
--		writeout_objects(event.tick, ent.surface, {left_top={x=math.floor(ent.position.x), y=math.floor(ent.position.y)}, right_bottom={x=math.floor(ent.position.x)+1, y=math.floor(ent.position.y)+1}})
--	end

--	print("on_some_entity_created: "..ent.name.." at "..ent.position.x..","..ent.position.y)
end

function on_some_entity_updated(event)
	local ent = event.entity or event.created_entity or nil
	if ent == nil then
		complain("wtf, on_some_entity_updated has nil entity")
		return
	end
	writeout(event.tick, "on_some_entity_updated", helpers.table_to_json(serialize_entity(ent)))
end

function on_some_entity_deleted(event)
	local ent = event.entity
	if ent == nil then
		complain("wtf, on_some_entity_created has nil entity")
		return
	end
	if event.name == defines.events.on_entity_died then
		local bot_id = character_bot_id_of(ent)
		if bot_id ~= nil then on_character_bot_died(event, bot_id) end
	end
	writeout(event.tick, "on_some_entity_deleted", helpers.table_to_json(serialize_entity(ent)))

--	-- we can't do this now, because the entity still exists at this point. instead, we schedule the writeout for the next tick
--
--	local surface = ent.surface
--	local area = {left_top={x=math.floor(ent.position.x), y=math.floor(ent.position.y)}, right_bottom={x=math.floor(ent.position.x)+1, y=math.floor(ent.position.y)+1}}
--	local tick = event.tick
--
--	table.insert(todo_next_tick, function () writeout_objects(tick, surface, area ) end)
----	complain("on_some_entity_deleted: "..ent.name.." at "..ent.position.x..","..ent.position.y)
end

-- One craft finished. Count it against whatever asked for that recipe.
--
-- `on_player_crafted_item` carries the player and the recipe and nothing else
-- -- no request id, no queue position (`workspace/factorio-api-docs/runtime-api.json`,
-- Factorio 2.1.17) -- so `(player_index, recipe.name)` is the only join there
-- is. See `craft_actions()` for why it is a *count* against a per-recipe
-- bucket rather than a position in one list per player.
--
-- A craft nobody is waiting on settles nothing: that is the normal case for
-- every intermediate `begin_crafting` queues on its own way to the requested
-- item, and for anything a human at the keyboard makes.
function on_player_crafted_item(event)
	local tmp_recent_item_addition = {}
	tmp_recent_item_addition.tick = event.tick
	tmp_recent_item_addition.recipe = event.recipe
	tmp_recent_item_addition.itemlist = products_to_dict(event.recipe.products)
	tmp_recent_item_addition.action_id = settle_crafted_item(event)

	if recent_item_additions[event.player_index] == nil then recent_item_additions[event.player_index] = {} end
	table.insert(recent_item_additions[event.player_index], tmp_recent_item_addition)
end

-- The game says these crafts will not happen. Answer the actions waiting on
-- them, now, rather than leaving them to time out six minutes later.
--
-- `cancel_count` is a number of *crafts*, and the event does not say which
-- requests they belonged to. They are taken from the back: the game's crafting
-- queue is FIFO, so the earliest request is the one nearest completion and the
-- latest is the one a cancellation reaches first.
--
-- A request that loses any of its crafts **fails**. It asked for a count and
-- that count will not arrive, and reporting the partial yield as a success
-- would tell the executor a bot holds items it does not have.
--
-- A wrapper rather than registering `fail_cancelled_crafts` directly, for the
-- same reason `on_research_finished` wraps `settle_research_actions`: the
-- registration below runs before that function's `function` statement has
-- assigned the global, so registering it by name there would register nil.
function on_player_cancelled_crafting(event)
	fail_cancelled_crafts(event)
end

-- Count items in an inventory by name.
--
-- Factorio 2.0 changed `LuaInventory.get_contents()` from a `name -> count`
-- dictionary to an array of `{name, count, quality}`. Every in-Lua read in this
-- file still indexed it as a dictionary, so every lookup answered nil:
-- `rcon_place_blueprint` could never build anything and always left ghosts,
-- `rcon_revive_ghost` refused every request with "player has no <item>" no
-- matter what the bot held, and `sum_inventory` summed tables under the integer
-- keys of the array.
--
-- It stayed invisible because the Rust side had already adapted -- see
-- `item_counts_map_or_seq` in crates/core/src/types.rs, which accepts either
-- shape -- so everything crossing the wire looked healthy while the mod's own
-- reads did not work at all.
--
-- Counts are summed across qualities: callers ask "does this bot hold one of
-- these", and a normal and an uncommon inserter both answer yes.
function inventory_counts(inventory)
	local counts = {}
	for _, stack in pairs(inventory.get_contents()) do
		counts[stack.name] = (counts[stack.name] or 0) + stack.count
	end
	return counts
end

function sum_inventory(ent, is)
	local sum = {}
	for _,inv_type in ipairs(is) do
		local inv = ent.get_inventory(inv_type)
		for item, amount in pairs(inventory_counts(inv)) do
			sum[item] = (sum[item] or 0) + amount
		end
	end
	return sum
end

-- path :: array of Waypoint (optional)
-- id :: uint: Handle to associate the callback with a particular call to LuaSurface::request_path.
-- try_again_later :: boolean: Indicates that the pathfinder failed because it is too busy, and you can retry later.
function on_script_path_request_finished(event)
	-- Every path request in this mod is somebody's: a Rust caller is waiting on
	-- the handle in `world.path_requests` (`sleep_for_path_request_result`,
	-- crates/core/src/factorio/rcon.rs). The mod used to ask for paths of its
	-- own, for a stalled walk, and had to filter those answers out here; that
	-- re-path is gone -- retrying a stuck walk lives in `move_player_timed`,
	-- where the goal, the radius and the standability judgement are.
	local result = "Error: failed to path find"
	if event.path ~= nil then
		local positions = {}
		for k,v in pairs(event.path) do
			-- `needs_destroy_to_reach` -- "true if the path from the previous
			-- waypoint to this one goes through an entity that must be
			-- destroyed" -- used to be dropped here, so the Rust side never
			-- learned a leg was blocked and the bot walked into the
			-- obstruction until the leg timed out. Flattened onto the
			-- position rather than sent as a separate structure: the field
			-- is additive over the previous `{x=.., y=..}` shape, so an
			-- older Rust build parsing this as a plain position still works.
			table.insert(positions, {
				x = v.position.x,
				y = v.position.y,
				needs_destroy_to_reach = v.needs_destroy_to_reach or false,
			})
		end
		result = helpers.table_to_json(positions)
	elseif event.try_again_later then
		result = "Error: try again later!"
	end
	writeout(event.tick, "on_script_path_request_finished", tostring(event.id) .. "#" .. result)
end

function player_total_inventory(player_id)
	local i = defines.inventory
	local is = { i.character_main, i.character_guns, i.character_ammo, i.character_armor } -- TODO: maybe more?
	return sum_inventory(bot_handle(player_id), is)
end

function inventory_diff(inv1, inv2)
	local diff = {}
	for item,amount in pairs(inv1) do
		temp = amount - (inv2[item] or 0)
		if temp ~= 0 then
			diff[item] = temp
		end
	end
	for item,amount in pairs(inv2) do
		if inv1[item] == nil then
			diff[item] = -inv2[item]
		end
	end
	return diff
end

function dump_dict(dict)
	for key,val in pairs(dict) do
		print("> "..key.." -> "..val)
	end
end

function on_player_changed_position(event)
	local tick = event.tick
	local player_idx = event.player_index
	local player = game.players[player_idx]
	if player.connected and player.character ~= nil then
		local position = player.position

		writeout(event.tick, "on_player_changed_position", helpers.table_to_json({
			player_id = player_idx,
			position = position
		}))
	end
end
function to_i64(a)
	return math.min(a, 18446744073709541614)
end

function on_player_changed_distance(event)
	for idx, player in each_bot() do
		writeout(event.tick, "on_player_changed_distance", helpers.table_to_json({
			player_id = idx,
			build_distance = player.build_distance,
			reach_distance = player.reach_distance,
			drop_item_distance = player.drop_item_distance,
			item_pickup_distance = to_i64(math.ceil(player.item_pickup_distance)),
			loot_pickup_distance = to_i64(math.ceil(player.loot_pickup_distance)),
			-- NOT ceiled. This is the bound the caller checks a mine against, and
			-- rounding it *up* is the direction that makes the check pass a request
			-- the game will refuse: a character's real 2.7 was reported as 3, so a
			-- bot 2.9 tiles from an ore looked in reach and mined nothing. The Rust
			-- side has taken this as a `double` since 4651d609; the two ceiled
			-- neighbours above are unused and left alone.
			resource_reach_distance = to_i64(player.resource_reach_distance),
		}))
	end
end

function on_research_finished(event)
	writeout_recipes()
	on_player_changed_distance(event)
	writeout(event.tick, "on_research_finished", "")
	writeout_forces()
	-- Last, deliberately. stdout is ordered, so settling the action here means
	-- the executor has already read the recipes and the force data this
	-- technology unlocked by the time it is told the research succeeded. The
	-- other order lets the next plan step run against a world snapshot that
	-- does not know about the thing it just waited for.
	settle_research_actions(event)
end

-- Hand the game's own completion signal back to the actions that asked for it.
--
-- This is the join that did not exist: `on_research_finished` carried no action
-- id, so `Actuator::research` had nothing to wait on and reported success the
-- instant the technology was *queued*.
--
-- Fires for every finished research, including ones nobody here asked for --
-- a `cheat_technology`, a trigger technology, another player's queue. Those
-- find no entry and settle nothing.
function settle_research_actions(event)
	local tech = event.research
	if tech == nil then
		return
	end
	local waiting = research_actions()[tech.name]
	if waiting == nil then
		return
	end
	-- Cleared before the writeouts, so a second `on_research_finished` for the
	-- same technology cannot report the same action done twice.
	research_actions()[tech.name] = nil
	for _, action_id in ipairs(waiting) do
		action_completed(event.tick, action_id)
	end
end

function on_player_main_inventory_changed(event)
	local tick = event.tick
	local player_idx = event.player_index
	local main_inventory = game.players[player_idx].get_main_inventory()
	writeout(event.tick, "on_player_main_inventory_changed", helpers.table_to_json({
		player_id = player_idx,
		main_inventory = main_inventory.get_contents()
	}))
	recent_item_additions[player_idx] = {}
end

script.on_init(on_init)
script.on_load(on_load)
script.on_event(defines.events.on_tick, on_tick)
script.on_event(defines.events.on_player_joined_game, on_player_joined_game)
script.on_event(defines.events.on_player_left_game, on_player_left_game)
script.on_event(defines.events.on_player_died, on_player_died)
script.on_event(defines.events.on_player_respawned, on_player_respawned)
script.on_event(defines.events.on_sector_scanned, on_sector_scanned)
script.on_event(defines.events.on_chunk_generated, on_chunk_generated)
script.on_event(defines.events.on_player_mined_item, on_player_mined_item)
-- The one item per depleted tile that `track_mining_drills` cannot see.
script.on_event(defines.events.on_resource_depleted, on_resource_depleted)

script.on_event(defines.events.on_biter_base_built, on_some_entity_created) --entity
script.on_event(defines.events.on_built_entity, on_some_entity_created) --created_entity
script.on_event(defines.events.on_robot_built_entity, on_some_entity_created) --created_entity
script.on_event(defines.events.on_player_rotated_entity, on_some_entity_updated) --entity
script.on_event(defines.events.on_built_entity, on_some_entity_created) --entity

script.on_event(defines.events.on_entity_died, on_some_entity_deleted) --entity
script.on_event(defines.events.on_player_mined_entity, function (event) on_mined_entity(event); on_some_entity_deleted(event) end) --entity
script.on_event(defines.events.on_robot_mined_entity, on_some_entity_deleted) --entity
script.on_event(defines.events.on_resource_depleted, on_some_entity_deleted) --entity

script.on_event(defines.events.on_script_path_request_finished, on_script_path_request_finished)
script.on_event(defines.events.on_research_finished, on_research_finished)

script.on_event(defines.events.on_player_main_inventory_changed, on_player_main_inventory_changed)
script.on_event(defines.events.on_player_changed_position, on_player_changed_position)
--script.on_event(defines.events.on_player_gun_inventory_changed, on_inventory_changed)
--script.on_event(defines.events.on_player_ammo_inventory_changed, on_inventory_changed)
--script.on_event(defines.events.on_player_armor_inventory_changed, on_inventory_changed)

script.on_event(defines.events.on_player_crafted_item, on_player_crafted_item)
-- The other half of a craft's outcome. Without this a cancelled craft answered
-- nothing at all and its registry entry outlived the run.
script.on_event(defines.events.on_player_cancelled_crafting, on_player_cancelled_crafting)

-- The only registration site for the force-sample cadence. `on_nth_tick`
-- replaces the handler for a given period rather than adding to it, so
-- re-running this file on a load cannot end up with two handlers writing one
-- tick twice.
script.on_nth_tick(SAMPLE_FORCE_INTERVAL, on_sample_force_tick)


function rcon_action_start_walk_waypoints(action_id, player_id, waypoints) -- e.g. waypoints= { {0,0}, {3,3}, {42,1337} }
	local player = get_player(player_id)
	if player == nil then
		return
	end
	start_walk_waypoints(action_id, player_id, waypoints)
	stamp_tick()
end

-- The body of the walk dispatch, with **no RCON output of any kind**.
--
-- Split out of `rcon_action_start_walk_waypoints` for one reason:
-- `step_aside_from_footprint` dispatches a walk from inside
-- `rcon_place_entity`, whose reply body is read by `place_entity_timed`
-- (crates/core/src/factorio/rcon.rs) as the placement's entire verdict. A
-- second `rcon.print` there -- `get_player`'s error, or the tick stamp -- turns
-- a refusal that names its cause into `Unexpected Response`. Same trap as
-- debugging the mod with `rcon.print`, reached by accident instead of on
-- purpose.
--
-- `step_aside` marks a walk the mod dispatched for its own reasons rather than
-- one an executor action is waiting on. Nothing branches on it today; it is
-- what tells a reader of `storage` which walks were nobody's request.
function start_walk_waypoints(action_id, player_id, waypoints, step_aside)
	local player = bot_handle(player_id)
	if player == nil or not player.connected or player.character == nil then
		return false
	end
	if storage.p[player_id] == nil then
		storage.p[player_id] = {}
	end
	local tmp = {}
	for i = 1, #waypoints do
		tmp[i] = {x=waypoints[i][1], y=waypoints[i][2]}
	end
	--	game.print("waypoints: " .. table_to_string(storage.p[player_id]))
	-- `idx_tick` used to be left nil until the first waypoint was reached, so
	-- the stuck check below measured "ticks since the *previous* waypoint"
	-- rather than "ticks since this leg started" -- and for the first leg
	-- specifically, it measured nothing at all until arrival, exempting it
	-- from the check entirely. Stamping it here covers leg 1 the same way
	-- every later leg is covered: the progress clock (WALK_STALL_TICKS)
	-- counts from here until the first tick the leg gets closer.
	storage.p[player_id].walking = {
		idx = 1,
		waypoints = tmp,
		action_id = action_id,
		idx_tick = game.tick,
		-- Where this leg started, so a stall can say how far the character
		-- actually got and how long the leg was. See `walk_stall_cause` for
		-- why the answer is not already in the message.
		idx_pos = walk_leg_origin(player),
		step_aside = step_aside,
	}
	return true
end

function rcon_action_start_mining(action_id, player_id, name, position, count)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local ent = nil
	if name ~= nil and position ~= nil then
		ent = player.surface.find_entity(name, position)
	end
	if ent and ent.minable then
--		print("MINING DO")
		storage.p[player_id].mining = { entity = ent, action_id = action_id, prototype = ent.prototype, left = count }
		if is_character_bot(player_id) then
			-- No `on_player_mined_entity` will come: completion is read as an
			-- inventory delta over the products this entity yields.
			local inv = player.get_main_inventory()
			local before = {}
			local props = ent.prototype.mineable_properties
			for _, product in pairs((props and props.products) or {}) do
				if product.name then before[product.name] = inv.get_item_count(product.name) end
			end
			if next(before) == nil then before[ent.name] = inv.get_item_count(ent.name) end
			storage.p[player_id].mining.inventory_before = before
		end
	elseif name == "stop" then
--		print("MINING STOP")
		storage.p[player_id].mining = nil
	else
--		print("MINING ERROR")
		rcon.print("Error: no entity to mine")
		storage.p[player_id].mining = nil
		action_failed(last_tick, action_id)
	end
	stamp_tick()
end

-- `underground_half` is `"input"` or `"output"` (or `nil`) -- the fifth
-- argument `crates/core/src/factorio/rcon.rs`'s `place_entity_timed` sends,
-- carrying `FactorioEntity::underground_half`. Forwarded to
-- `surface.create_entity` as `type`, and ONLY for `underground-belt`: the
-- game rejects an unknown `type` key on any prototype that has none, so
-- sending it unconditionally would break every other placement.
function rcon_place_entity(player_id, item_name, entity_position, direction, underground_half)
	-- A placement pays for itself out of the bot's inventory. See above.
	mark_bot_inventory_dirty(player_id)
	local entproto = prototypes.item[item_name].place_result
	local player = bot_handle(player_id)
	-- Refused before anything else is asked, and stamped like every other
	-- exit. The sentence is outside the `can_place_entity said 'no'` family on
	-- purpose: `note_placement_refusal` (crates/core/src/factorio/rcon.rs)
	-- must not remember a dead bot as a fact about the ground.
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
		stamp_tick()
		return
	end
	if player.character == nil then
		rcon.print(no_character_error(player_id, player))
		stamp_tick()
		return
	end
	local surface = player.surface

	-- Every exit below stamps the tick, including the refusals.
	--
	-- They used to return bare, so a placement the game had *judged* -- it ran
	-- `can_place_entity` and said no -- came back with no tick on it. The
	-- executor then recorded the failure with `dispatched_tick = nil`, and
	-- `record.actions` writes nothing at all for an action with no ticks: the
	-- 2026-09-02 research run has a 95-step plan, a failure, and not one
	-- `action_dispatched` line explaining which step it was. `take_tick_stamp`
	-- (crates/core/src/factorio/ticks.rs) lifts the stamp out of the reply
	-- wherever it sits, so the callers that judge this reply by shape are
	-- unaffected.
	if entproto == nil then
		complain("cannot place item '"..item_name.."' because place_result is nil")
		stamp_tick()
		return
	end

	if player.get_item_count(item_name) <= 0 then
		complain("cannot place item '"..item_name.."' because the player '"..player.name.."' does not have any")
		stamp_tick()
		return
	end

	if not surface.can_place_entity(placement_check_args(entproto, entity_position, direction, player.force)) then
		local pos = {x = entity_position[1], y = entity_position[2]}
		-- The box the game just judged is the prototype's box turned to
		-- `direction`, and every question below is asked of THAT box. Asking
		-- the north-frame one instead is how `run-1788569499-05724` lost a
		-- plan: bot 1 stood at (42.24, -6.77), 0.55 tiles inside the east
		-- edge of a steam engine facing east at [40.5, -5.5]. The engine is
		-- 2.5 by 4.7 north-frame, so the unturned box reached only to x =
		-- 41.75 (42 expanded) and the actor was "outside" it; the turned one
		-- reaches x = 42.85. The game said no for the actor, all three
		-- branches below said "the ground", the site went into the refusal
		-- ledger and the next plan moved the whole plant. See
		-- `collision_box_facing`.
		local footprint = collision_box_facing(entproto.collision_box, direction)
		local bb = add_to_bounding_box(expand_rect_floor_ceil(footprint), pos)
		-- **Three answers, not two, and the order is deliberate.**
		--
		-- `can_place_entity` collides with characters like anything else, but
		-- a character is the one blocker that leaves on its own. Reporting one
		-- as a verdict about the ground is worse than reporting nothing: the
		-- generic wording below is what `note_placement_refusal`
		-- (crates/core/src/factorio/rcon.rs) remembers, and a remembered
		-- refusal ALSO suppresses `recover`'s tier-1 reschedule
		-- (`refused_by_the_game`, gated on `PlanState::is_site_refused`) --
		-- which is exactly the recovery a blocker that walks away needs. So
		-- the false ledger entry costs the site, and the suppressed retry
		-- costs the milestone.
		--
		-- Run 24 (`run-1788347034-00981`) is what that costs. Bot 4 was
		-- refused a stone furnace at `[-21, 24]`; bot 3 was parked at
		-- `(-21.47, 23.73)`, a third of a tile inside the footprint, having
		-- been left there by its own furnace at `[-23, 24]`. The site was
		-- fenced off for the rest of the run and the next plan fled to
		-- `[-26, 14]`.
		--
		-- Order:
		--   1. the ACTING player -- the RCON layer can walk it around eight
		--      compass points and retry, which beats a reschedule, so this
		--      case must win even when another character is in the box too;
		--   2. any OTHER character -- a transient, reported in wording
		--      deliberately outside the `can_place_entity said 'no'` family so
		--      nothing durable is learned and tier 1 stays available;
		--   3. the ground -- the durable fact the refusal memory exists for.
		--
		-- (2) is the same distinction `rcon_can_place_entities` already draws
		-- with `rec.character`, which the Rust side reads as
		-- `PlacementVerdict::is_durable_refusal`. Two call sites, one concept:
		-- any character, not just the acting one, is a transient. A third path
		-- asking this question must draw it the same way.
		if not report_character_in_footprint(surface, footprint, bb, pos, player, item_name) then
			-- The ground's verdict, and what was ON the ground when it was
			-- given. The bare sentence is what `note_placement_refusal`
			-- (crates/core/src/factorio/rcon.rs) matches and must stay
			-- intact; the parenthesis after it is what that function reads
			-- the blockers and the tile out of. Without it a refusal at
			-- dispatch reached the record as `blockers: []`, which reads as
			-- "nothing was there" and was, five runs running, the whole of
			-- what anyone had to go on.
			rcon.print("cannot place item '"..item_name.."' because surface.can_place_entity said 'no'"
				..describe_footprint(scan_footprint(surface, footprint, pos)))
		end
		stamp_tick()
		return
	end

	-- **Build first, charge second, and check both returns.**
	--
	-- This used to `remove_item` and then `create_entity`, discarding both
	-- return values, which is two bugs in two lines. `create_entity` returns
	-- "the created entity or `nil` if the creation failed" (verified against
	-- workspace/factorio-api-docs/runtime-api.json, Factorio 2.1.17, runtime
	-- api 6, `LuaSurface::create_entity`), so a failed placement left the item
	-- already taken and nothing built: the material was destroyed. And
	-- `LuaControl::remove_item` returns "the number of items that were
	-- actually removed", so a removal that moved *nothing* still built the
	-- entity -- a free build, the exact failure `charge_item_to` was written
	-- for one function down.
	--
	-- Placement failure is the dominant failure mode in this project's runs,
	-- so the destructive path was the well travelled one.
	--
	-- Order: create, then charge, then undo if the charge fails. The other
	-- order (charge, then create, then refund) has to hand the item back
	-- through `insert`, which can itself fall short if the inventory filled in
	-- between -- a refund that silently loses material is the bug again with
	-- more steps. Undoing a build is exact: `destroy()` cannot half-work.
	--
	-- Nothing is announced until it is paid for. `on_some_entity_created` --
	-- the only thing that tells the Rust `EntityGraph` this entity exists --
	-- runs *after* the charge succeeds, so the undo path never leaves a
	-- phantom behind and needs no matching deletion event. `create_entity`
	-- does not raise `script_raised_built` unless asked (`raise_built`
	-- defaults to false **[V]**), so the game does not announce it either.
	-- **Asked again at the instant of building, of the box about to be
	-- built.** `can_place_entity` above collides with characters and this
	-- handler runs between ticks, so nothing walks in between the two calls
	-- -- but the question is cheap, and a character is the one blocker that
	-- moves: the whole reason the two-way branch above exists is that the
	-- game's verdict is not always about the ground. A character in the box
	-- here, walking or standing, is refused with the transient wording (and
	-- an idle one asked aside), never built over.
	--
	-- Gated on the scan of the raw box, not on the actor's position against
	-- the expanded one: the expanded box reaches half a tile past what the
	-- game judged, and an actor standing there with the game saying yes is a
	-- placement that has always succeeded and must go on succeeding.
	do
		local pos = {x = entity_position[1], y = entity_position[2]}
		local footprint = collision_box_facing(entproto.collision_box, direction)
		if character_in_footprint(surface, footprint, pos) then
			local bb = add_to_bounding_box(expand_rect_floor_ceil(footprint), pos)
			report_character_in_footprint(surface, footprint, bb, pos, player, item_name)
			stamp_tick()
			return
		end
	end

	local create_args = {name=entproto.name,position=entity_position,direction=direction,force=player.force, fast_replace=true, player=player_identification(player_id), spill=true}
	-- Only `underground-belt` has a `type` (`belt_to_ground_type`, "input" or
	-- "output"); `LuaSurface.create_entity` raises on an unknown `type` key
	-- for any prototype that has none, so this must not be sent unconditionally.
	if entproto.name == "underground-belt" and underground_half ~= nil then
		create_args.type = underground_half
	end
	local result = surface.create_entity(create_args)

	if result == nil then
		complain("placing item '"..item_name.."' failed, surface.create_entity returned nil :(")
		stamp_tick()
		return
	end

	if player.remove_item({name=item_name,count=1}) ~= 1 then
		-- The affordability check above passed and the spend still took
		-- nothing. `get_item_count` and `remove_item` are the matched pair --
		-- both act on every inventory the player has -- so the two can only
		-- disagree if something moved in between, or if the item counted is
		-- not the item spendable (an `ItemStackDefinition` quality defaults to
		-- `normal`, so an uncommon stack counts and cannot be taken).
		--
		-- Whichever it was, this is a *material* refusal and not a verdict
		-- about the site: the wording deliberately stays out of the
		-- `can_place_entity said 'no'` family that `note_placement_refusal`
		-- (crates/core/src/factorio/rcon.rs) remembers and fences the planner
		-- out of, and is not the `§player_blocks_placement§` sentinel that
		-- makes the RCON layer walk the bot aside and retry.
		result.destroy()
		complain("cannot place item '"..item_name.."' because taking it from the player '"..player.name.."' removed nothing")
		stamp_tick()
		return
	end

	-- The entity stands and is paid for. If a character is inside it anyway
	-- -- the game placed the box somewhere other than where it was judged,
	-- or a check above was wrong about what the game collides with -- it is
	-- moved out now, the way the game moves a player a building lands on,
	-- and the move is in the reply and in the record. See
	-- `push_characters_out_of`.
	local pushed = push_characters_out_of(surface, result)
	on_some_entity_created({tick=last_tick, entity = result})
	local reply = serialize_entity(result)
	if #pushed > 0 then reply.pushed_out = pushed end
	rcon.print(helpers.table_to_json(reply))
	stamp_tick()
end

-- The two character arms of a placement refusal, in the order
-- `rcon_place_entity` documents: the acting player in the expanded box gets
-- the `§player_blocks_placement§` sentinel, any other character in the raw
-- box gets the transient wording and, when idle, a walk out. Prints the
-- reply and answers true when a character was the cause; false when the
-- caller has to report the ground.
function report_character_in_footprint(surface, footprint, bb, pos, player, item_name)
	if position_in_rect(player.position, bb) then
		rcon.print("§player_blocks_placement§")
		return true
	elseif character_in_footprint(surface, footprint, pos) then
		-- Ask whoever it is to move, so the next attempt has a chance of
		-- finding the ground it was always going to find. Before this, the
		-- classification was right and nothing acted on it: an idle bot in
		-- a footprint was a transient with no end. See
		-- `step_aside_from_footprint`.
		local found = step_aside_from_footprint(surface, footprint, pos, player)
		-- The sentence stays byte-identical up to the parenthesis: it is what
		-- `FOOTPRINT_CHARACTER_REFUSAL` matches and what `classify_failure`
		-- reads as `FailureKind::Blocked`. The clause after it is new, and is
		-- what lets the retry wait for a blocker that is busy rather than
		-- spending one fixed budget on every kind of blocker alike. See
		-- `describe_footprint_blockers`.
		rcon.print("cannot place item '"..item_name.."' because a character is standing in the footprint"
			..describe_footprint_blockers(found))
		return true
	end
	return false
end

--- How far from its own position a character overlapped by a freshly built
--- entity is allowed to be moved to stand clear of it, and how finely the
--- spot is chosen. A character is 0.4 tiles across and nothing this mod
--- builds is wider than a steam engine, so a few tiles always holds a spot.
PLACEMENT_PUSH_OUT_RADIUS = 4
PLACEMENT_PUSH_OUT_PRECISION = 0.25

-- Moves every character overlapping `entity`'s real bounding box to the
-- nearest spot the game says a character fits, and reports each move.
--
-- **What the game does for a player, done for a character.** A building
-- placed on a connected player pushes the player out; a server-side
-- `character` entity gets no such courtesy, and one left inside a furnace
-- has every path request refused from then on -- there is no legitimate
-- action that gets it out, because walking starts with the pathfinder. The
-- move is a `teleport`, recorded through `teleport_writeout` so the record
-- shows it as one (`record.teleports()`), and returned so the reply carries
-- it too: `{ bot = <id or 0>, from = {x,y}, to = {x,y} }` per character. A
-- character that fits nowhere within the radius is left and reported with
-- `to = nil` rather than silently.
--
-- Read off `entity.bounding_box` -- the box the game gave the entity, at the
-- position and orientation it actually has -- not off the prototype box the
-- checks before `create_entity` reasoned about. This is the one scan that
-- cannot be wrong about which box was built.
function push_characters_out_of(surface, entity)
	local pushed = {}
	local bb = entity.bounding_box
	if bb == nil then return pushed end
	for _, character in pairs(surface.find_entities_filtered{ area = bb, type = "character" }) do
		if character.valid ~= false and character ~= entity then
			local from = { x = character.position.x, y = character.position.y }
			local landing = surface.find_non_colliding_position(
				"character", from, PLACEMENT_PUSH_OUT_RADIUS, PLACEMENT_PUSH_OUT_PRECISION)
			local id = bot_of_character(character) or 0
			local record = { bot = id, from = from }
			if landing ~= nil and type(character.teleport) == "function" and character.teleport(landing) then
				record.to = { x = landing.x, y = landing.y }
				teleport_writeout(game.tick, id, "placement_pushed_out", from, record.to, nil)
			end
			pushed[#pushed + 1] = record
		end
	end
	return pushed
end

-- The exact question a build asks the game, in one place.
--
-- `rcon_place_entity` below and `rcon_can_place_entities` (the pre-flight
-- check the planner runs before it commits a plan to a site) MUST ask
-- `can_place_entity` the same question, or the pre-check answers about a
-- placement nobody is going to make. Two traps live in this table:
--
--   * `build_check_type` defaults to `ghost_revive`, NOT to `manual`. A ghost
--     check is the same family of mistake as `only_ghosts = true` on a
--     blueprint: ghosts do not collide, so it validates far less than it
--     looks like it does. `manual` is what a player building by hand runs,
--     which is what a bot placing an entity is.
--   * `force` decides whose entities count as friendly, so it has to be the
--     acting player's force and not the default `"neutral"`.
--
-- Verified against workspace/factorio-api-docs/runtime-api.json (Factorio
-- 2.1.17, runtime api version 6): `LuaSurface.can_place_entity` takes
-- {name, position, direction, force, build_check_type, forced, inner_name},
-- and `forced` is read only for the three `*_ghost` check types. There is no
-- `force_build`/`build_mode` parameter on this method at all -- that rename
-- belongs to blueprint building, not here.
function placement_check_args(entproto, position, direction, force)
	return {
		name = entproto.name,
		position = position,
		direction = direction,
		force = force,
		build_check_type = defines.build_check_type.manual,
	}
end

-- The prototype's collision box turned to face `direction`.
--
-- `LuaEntityPrototype.collision_box` is the box of the entity facing north
-- **[V]** (runtime-api.json, Factorio 2.1.17, api 6: "the bounding box used
-- for collision checking", given once, per prototype, not per direction). A
-- steam engine is 2.5 wide and 4.7 tall in that frame and 4.7 wide and 2.5
-- tall once it faces east or west; a boiler is 3x2 north and 2x3 east.
-- `can_place_entity` takes the direction and judges the turned box, so
-- anything that reasons about what that judgement covered has to turn the
-- box the same way -- `rcon_place_entity`'s three-way branch,
-- `character_in_footprint`, `step_aside_from_footprint` and
-- `rcon_can_place_entities` all do, through this.
--
-- Turned about the entity's own centre, clockwise, one quarter per cardinal:
-- east is (x, y) -> (-y, x), south (-x, -y), west (y, -x). Only the four
-- cardinals turn; a half-diagonal `direction` -- nothing this mod places
-- stands on one -- gets the north-frame box back rather than a guess.
-- Always a fresh table, never the prototype's own, which is read-only and
-- must not be handed to a caller that will shift it in place.
function collision_box_facing(bb, direction)
	local lt, rb = bb.left_top, bb.right_bottom
	if direction == defines.direction.east then
		return { left_top = { x = -rb.y, y = lt.x }, right_bottom = { x = -lt.y, y = rb.x } }
	elseif direction == defines.direction.south then
		return { left_top = { x = -rb.x, y = -rb.y }, right_bottom = { x = -lt.x, y = -lt.y } }
	elseif direction == defines.direction.west then
		return { left_top = { x = lt.y, y = -rb.x }, right_bottom = { x = rb.y, y = -lt.x } }
	end
	return { left_top = { x = lt.x, y = lt.y }, right_bottom = { x = rb.x, y = rb.y } }
end

-- Whether any character stands in the footprint `can_place_entity` just
-- tested.
--
-- `footprint` is the **raw** collision box, already turned to the placement's
-- direction (see `collision_box_facing`), not the floor/ceil-expanded one
-- `rcon_place_entity` uses for its acting-player test: the expanded box
-- reaches half a tile past what the game actually judged and would pull in a
-- bot standing legitimately clear, turning a real ground refusal into a
-- transient nobody learns from. `rcon_can_place_entities` scans the same raw
-- box for the same reason.
--
-- Filtered at the game by `type`, which is safe here in a way it is not for
-- the queries that feed `EntityGraph`: this asks only whether a character is
-- present, so nothing about trees is load-bearing.
function character_in_footprint(surface, footprint, position)
	local bb = add_to_bounding_box(footprint, position)
	return #surface.find_entities_filtered{ area = bb, type = "character" } > 0
end

-- What stands in `footprint` centred at `position`, and what tile is under
-- the centre: `{ character = bool, blockers = {sorted distinct names}, tile =
-- name or nil }`.
--
-- One scan for the two callers that report a refusal -- the pre-flight
-- `rcon_can_place_entities` and the dispatched `rcon_place_entity` -- so the
-- two cannot describe the same ground differently. `blockers` is every
-- entity in the box, characters included: this describes, it does not
-- judge, and the judging (`character` is a transient, the rest is the
-- ground) stays with the callers.
function scan_footprint(surface, footprint, position)
	local bb = add_to_bounding_box(footprint, position)
	local seen = {}
	local out = { character = false, blockers = {} }
	for _, e in pairs(surface.find_entities_filtered{ area = bb }) do
		if e.type == "character" then
			out.character = true
		end
		if not seen[e.name] then
			seen[e.name] = true
			out.blockers[#out.blockers + 1] = e.name
		end
	end
	table.sort(out.blockers)
	local tile = surface.get_tile(position.x, position.y)
	if tile ~= nil and tile.valid then
		out.tile = tile.name
	end
	return out
end

-- The parenthesis `rcon_place_entity` appends to the ground's refusal, from a
-- `scan_footprint` result: ` (in the footprint: a, b; tile: grass-1)`, or
-- ` (nothing in the footprint; tile: grass-1)` when the box held no entity
-- and the tile is the only thing left to blame. `note_placement_refusal`
-- (crates/core/src/factorio/rcon.rs) parses exactly this shape; the two
-- wordings are the only ones it knows.
function describe_footprint(scan)
	local what
	if #scan.blockers > 0 then
		what = "in the footprint: " .. table.concat(scan.blockers, ", ")
	else
		what = "nothing in the footprint"
	end
	if scan.tile ~= nil then
		return " (" .. what .. "; tile: " .. scan.tile .. ")"
	end
	return " (" .. what .. ")"
end

-- Where to send a character that is standing inside `bb`, and why that spot.
--
-- Every exit, **nearest first**: out through an edge, plus the character's own
-- half-width, plus `PLACEMENT_STEP_ASIDE_MARGIN`. Nearest, because a step
-- aside is meant to be a step: crossing the whole footprint to leave by the
-- far side is a longer walk to no better place, and the run this exists for
-- had its blocker 0.43 tiles from one edge and 1.37 from the other. The
-- others follow in order because the nearest exit is not always one the
-- character can use -- see `step_aside_from_footprint` for the crack between
-- two assemblers that made this a list.
--
-- Pure geometry, and deliberately so -- it makes no query and reads no state,
-- which is what lets a test pin the choice without a game. Ties resolve
-- west, east, north, south, in that order, so the answer does not depend on
-- table iteration.
--
-- The half-width comes from the character's own `bounding_box` rather than
-- from the prototype table: it is already in hand from the scan that found
-- this character, it is exact, and `rcon_place_entity` must not depend on a
-- prototype lookup that could be nil in the middle of a reply.
function placement_step_aside_half_box(character)
	local half_x, half_y = 0.2, 0.2
	local box = character.bounding_box
	if box ~= nil then
		half_x = (box.right_bottom.x - box.left_top.x) / 2.0
		half_y = (box.right_bottom.y - box.left_top.y) / 2.0
	end
	return half_x, half_y
end

function placement_step_aside_targets(bb, character)
	local pos = character.position
	local half_x, half_y = placement_step_aside_half_box(character)
	local exits = {
		{ out = pos.x - bb.left_top.x, order = 1,
		  target = { x = bb.left_top.x - half_x - PLACEMENT_STEP_ASIDE_MARGIN, y = pos.y } },
		{ out = bb.right_bottom.x - pos.x, order = 2,
		  target = { x = bb.right_bottom.x + half_x + PLACEMENT_STEP_ASIDE_MARGIN, y = pos.y } },
		{ out = pos.y - bb.left_top.y, order = 3,
		  target = { x = pos.x, y = bb.left_top.y - half_y - PLACEMENT_STEP_ASIDE_MARGIN } },
		{ out = bb.right_bottom.y - pos.y, order = 4,
		  target = { x = pos.x, y = bb.right_bottom.y + half_y + PLACEMENT_STEP_ASIDE_MARGIN } },
	}
	table.sort(exits, function(a, b)
		if a.out ~= b.out then return a.out < b.out end
		return a.order < b.order
	end)
	local targets = {}
	for i, e in ipairs(exits) do targets[i] = e.target end
	return targets
end

function placement_step_aside_target(bb, character)
	return placement_step_aside_targets(bb, character)[1]
end

-- The box a step-aside landing must stay out of: the footprint grown by the
-- character's half-box and by the walker's stopping box.
--
-- The walker declares a leg done anywhere within 0.3 of its waypoint
-- (`on_tick`), so a landing 0.1 tiles clear of the footprint is a character
-- that stops 0.2 tiles inside it, and the retry finds the footprint exactly
-- as occupied as before. Testing the landing against the raw box let that
-- through: in `run-1788609725-78284` bot 1 stood in the assembler footprint
-- at `[39.5, -9.5]`, its nearest exit was west, and west was the 0.6-tile
-- crack between that box and the assembler at `[36.5, -9.5]`. The target
-- itself collided with the neighbour, `find_non_colliding_position` answered
-- with a spot in the crack, and four step-aside walks each *completed* --
-- `action_completed ok 4712` -- 0.3 tiles further along the crack and still
-- in the way, until the action had failed and the milestone had replanned.
WALK_ARRIVAL_HALF = 0.3
function placement_step_aside_clearance(bb, character)
	local half_x, half_y = placement_step_aside_half_box(character)
	return {
		left_top = { x = bb.left_top.x - half_x - WALK_ARRIVAL_HALF, y = bb.left_top.y - half_y - WALK_ARRIVAL_HALF },
		right_bottom = { x = bb.right_bottom.x + half_x + WALK_ARRIVAL_HALF, y = bb.right_bottom.y + half_y + WALK_ARRIVAL_HALF },
	}
end

-- The first landing, nearest exit first, that the game says the character
-- fits at and that clears the footprint by enough for the walker's stopping
-- box, or nil when no exit offers one.
function placement_step_aside_landing(surface, bb, character)
	local clearance = placement_step_aside_clearance(bb, character)
	local targets = placement_step_aside_targets(bb, character)
	-- Radius outermost, exits innermost: every edge is asked at the near
	-- radius before any edge is asked at a wider one, so the nearest landing
	-- still wins and widening only ever adds answers. See
	-- `PLACEMENT_STEP_ASIDE_RADII`.
	for _, radius in ipairs(PLACEMENT_STEP_ASIDE_RADII) do
		for _, target in ipairs(targets) do
			local landing = surface.find_non_colliding_position(
				"character", target, radius, PLACEMENT_STEP_ASIDE_PRECISION)
			-- Nil is the game saying the character fits nowhere within this
			-- radius of there; a landing inside the clearance is a walk that
			-- costs time and changes nothing. Either way, try the next edge
			-- before widening: better no walk than a walk that ends where it
			-- began.
			if landing ~= nil and not position_in_rect(landing, clearance) then
				return landing
			end
		end
	end
	return nil
end

-- Asks every bot standing in a refused footprint to walk out of it.
--
-- **A transient is only transient if something ends it.** `537adf30` stopped
-- reporting a character in the footprint as a verdict about the ground, which
-- was right: a character moves on its own, and remembering one fences the
-- planner off open ground for the rest of the run. But "moves on its own" is
-- an assumption about a bot that has work to do, and an idle bot has none. It
-- parked where servicing its own furnace left it and it will stand there
-- forever.
--
-- Run 27 (`workspace/runs/run-1788353986-24634`) is that forever. Bot 3 stood
-- at `(-23.5078125, 16.203125)` from tick 18240 to the end of the run;
-- milestone 6 re-planned onto that ground and reported
-- `success=0 failed=1 lost=0 pending=30` three iterations running before
-- giving up. Nothing was wrong except that nobody had asked bot 3 to move.
--
-- This is the acting-player recovery widened to the bot that is actually in
-- the way. `rcon_place_entity` answers `§player_blocks_placement§` when the
-- ACTOR is in its own footprint and the RCON layer walks it around eight
-- compass points and retries; that has always been the better answer than a
-- reschedule, and it was only ever available to one of the characters that can
-- be standing there.
--
-- **Only bots the mod is not already steering.** `storage.p[idx].walking` and
-- `.mining` are how `on_tick` drives a bot through an action the executor is
-- waiting on; replacing either would strand that action until the 360-second
-- `ACTION_RESULT_DEADLINE` calls it lost, which is a worse outcome than the
-- refusal being fixed. It is also unnecessary -- a bot that is walking or
-- mining is going to leave. Only a bot with nothing to do is a permanent
-- blocker, and "the mod is not steering it" is exactly that condition. Decided
-- and acted on inside one RCON command, so it races nothing: the executor's
-- next dispatch for that bot is a later command, and it would simply replace
-- this walk, which is nobody's request and has nothing waiting on it.
--
-- **A legitimate walk, not a teleport.** The bot is handed to the same
-- `walking_state` machinery every other walk goes through.
--
-- This does not make the placement succeed. The action still fails, still
-- reports the transient wording, and still teaches the refusal ledger nothing;
-- the difference is that by the time anything asks again, the blocker is
-- somewhere else.
--
-- **Returns what it found, one phrase per character**, so the caller can say
-- it in the reply. The distinction between the blocker classes is already
-- made here, one line above the walk that acts on it, and it is the whole of
-- what the waiting side needs: see `describe_footprint_blockers`.
function step_aside_from_footprint(surface, footprint, position, acting_player)
	local bb = add_to_bounding_box(footprint, position)
	local found = {}
	for _, character in ipairs(surface.find_entities_filtered{ area = bb, type = "character" }) do
		-- Resolved through `bot_of_character`, not `LuaEntity.player`: the
		-- latter is nil for every character bot, and reading it here is what
		-- left a headless roster's blockers unasked -- `run-1788608648-56109`
		-- refused three placements four times each over 543 ticks, failed
		-- them, and replanned, while the same plan family with clients never
		-- refused one. A character nobody claims cannot be asked to walk and
		-- must not raise here -- a raise inside an RCON handler costs the
		-- caller its whole reply.
		local blocker_id, blocker = bot_of_character(character)
		if blocker_id == nil then
			found[#found + 1] = "an unclaimed character"
		elseif blocker_id ~= acting_player.index then
			if not (blocker.connected and blocker.character ~= nil) then
				-- A bot the registry knows and the game cannot steer: a
				-- disconnected client leaves its character standing.
				found[#found + 1] = "#" .. blocker_id .. " gone"
			else
				local state = storage.p[blocker_id]
				if state ~= nil and state.walking ~= nil then
					found[#found + 1] = "#" .. blocker_id .. " walking"
				elseif state ~= nil and state.mining ~= nil then
					found[#found + 1] = "#" .. blocker_id .. " mining"
				else
					local landing = placement_step_aside_landing(surface, bb, character)
					if landing ~= nil then
						start_walk_waypoints(PLACEMENT_STEP_ASIDE_ACTION_ID, blocker_id,
							{ { landing.x, landing.y } }, true)
						found[#found + 1] = "#" .. blocker_id .. " stepping aside"
					else
						-- Idle, asked nothing, and going nowhere: every exit
						-- the game offered was inside the clearance. Named
						-- apart from `stepping aside` because it is the one
						-- class that waiting cannot fix.
						found[#found + 1] = "#" .. blocker_id .. " stuck"
					end
				end
			end
		end
	end
	return found
end

-- The clause `report_character_in_footprint` appends to the transient refusal:
-- ` (blockers: #1 mining, #3 stepping aside)`, or nothing at all when the scan
-- named nobody.
--
-- **What it is doing is the whole of the retry policy.** The refusal wording
-- used to say only that *a* character was there, and
-- `place_entity_timed` (crates/core/src/factorio/rcon.rs) therefore had one
-- budget for two situations that need opposite answers: a blocker this
-- function has just asked to walk aside is gone in tens of ticks, and a
-- blocker it deliberately left alone -- `walking` or `mining` for an action of
-- its own -- takes as long as that action does. `run-1788655528-63394` is the
-- second: bot 1 stood at `(26.29, -47.33)` inside the stone furnace at
-- `[26, -48]` while mining copper ore from tick 5597 to 6079, and the
-- placement gave up at 6001 -- **78 ticks early** -- after four dispatches
-- over 114 ticks, failing the action and replanning milestone 1.
--
-- The vocabulary is closed and each word is a verdict about *time*, which is
-- the only thing the waiting side can act on:
--
--   `#N mining` / `#N walking`   busy for an action of its own, will leave
--   `#N stepping aside`          a walk out has just been dispatched
--   `#N stuck`                   idle and the game offered no landing
--   `#N gone`                    claimed by the registry, not steerable
--   `an unclaimed character`     no bot behind it at all
--
-- **No apostrophe anywhere in it**, deliberately: `classify_failure`
-- (crates/scripting_lua/src/globals/record.rs) reads a missing item's name out
-- of the first pair of single quotes in the message, and the item name in the
-- sentence this is appended to is that pair.
function describe_footprint_blockers(found)
	if found == nil or #found == 0 then return "" end
	return " (blockers: " .. table.concat(found, ", ") .. ")"
end

-- Answers, for a batch of candidate placements, whether the game would allow
-- each one -- and when it would not, what is standing there.
--
-- `sites` is an array of {player=<id>, item=<item name>, position={x,y},
-- direction=<defines.direction>}. The reply is ONE json document:
--
--   { "tick": <game.tick>, "sites": [ { "ok": bool,
--                                       "character": bool,
--                                       "blockers": [names],
--                                       "tile": <tile name> }, ... ] }
--
-- in the same order as `sites`, so a caller joins by index.
--
-- Two things this reports that a refused *placement* cannot. First,
-- `character`: `can_place_entity` says no when any character is in the
-- footprint, and a character is the one blocker that moves on its own, so it
-- must not be learned as a fact about the ground. `rcon_place_entity` makes
-- the same distinction for the *acting* player and this makes it for every
-- character, because at pre-check time the acting bot has not walked to the
-- site yet and any bot standing there is equally transient.
--
-- Second, `blockers`/`tile`: the game's own refusal names no cause, which is
-- why five separate runs ended on `can_place_entity said 'no'` with nothing
-- to go on. `find_entities_filtered` over the collision box the check just
-- tested names what is in it, and the tile name covers the case where the
-- ground itself (water, a cliff edge) is the answer and there is no entity to
-- find.
--
-- The box queried is the raw `collision_box`, turned to `direction` and
-- shifted to the position -- the box `can_place_entity` tested -- not the
-- floor/ceil-expanded one `rcon_place_entity` uses for its player-in-footprint
-- test, which would pull in neighbours that are not colliding with anything.
function rcon_can_place_entities(sites)
	local out = { tick = game.tick, sites = {} }
	for i, site in ipairs(sites) do
		local rec = { ok = false, character = false }
		local player = bot_handle(site.player)
		local itemproto = prototypes.item[site.item]
		local entproto = nil
		if itemproto ~= nil then
			entproto = itemproto.place_result
		end
		if player == nil then
			rec.error = "no player "..tostring(site.player)
		elseif entproto == nil then
			rec.error = "item '"..tostring(site.item).."' has no place_result"
		else
			local surface = player.surface
			local pos = { x = site.position[1], y = site.position[2] }
			if surface.can_place_entity(placement_check_args(entproto, pos, site.direction, player.force)) then
				rec.ok = true
			else
				-- The box the check just judged: turned to the site's
				-- direction, exactly as `placement_check_args` asked.
				local scan = scan_footprint(surface,
					collision_box_facing(entproto.collision_box, site.direction), pos)
				rec.character = scan.character
				-- Omitted rather than sent empty: `helpers.table_to_json`
				-- renders an empty Lua table as `{}`, which is an object, and
				-- the Rust side reads this field as a list.
				if #scan.blockers > 0 then
					rec.blockers = scan.blockers
				end
				rec.tile = scan.tile
			end
		end
		out.sites[i] = rec
	end
	rcon.print(helpers.table_to_json(out))
end

function add_to_bounding_box(bb, center_position)
	return {
		left_top = { x = bb.left_top.x + center_position.x, y = bb.left_top.y + center_position.y},
		right_bottom = { x = bb.right_bottom.x + center_position.x, y = bb.right_bottom.y + center_position.y},
	}
end

function expand_rect_floor_ceil(bb)
	return {
		left_top = { x = math.floor(bb.left_top.x * 2.0) / 2.0 , y = math.floor(bb.left_top.y * 2.0) / 2.0 },
		right_bottom = { x = math.ceil(bb.right_bottom.x * 2.0) / 2.0 , y = math.ceil(bb.right_bottom.y * 2.0) / 2.0},
	}
end

function position_in_rect(check_position, bb)
	return check_position.x >= bb.left_top.x and
		check_position.x <= bb.right_bottom.x and
		check_position.y >= bb.left_top.y and
		check_position.y <= bb.right_bottom.y
end


function rcon_insert_to_inventory(player_id, entity_name, entity_pos, inventory_type, items)
	-- Items leave this bot's inventory; scan it next tick rather than waiting
	-- for its turn in the stagger. See `BOT_INVENTORY_POLL_PERIOD`. Marked on
	-- entry, so a refusal below costs one redundant scan and never a missed
	-- one.
	mark_bot_inventory_dirty(player_id)
	local player = bot_handle(player_id)
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
		return
	end
	if player.character == nil then
		rcon.print(no_character_error(player_id, player))
		return
	end
	local entity = player.surface.find_entity(entity_name, entity_pos)
	if entity == nil then
		complain("cannot insert to inventory of nonexisting entity "..entity_name.." at "..pos_str(entity_pos))
		return
	end

	local inventory = entity.get_inventory(inventory_type)
	if inventory == nil then
		complain("cannot insert to nonexisting inventory of entity "..entity_name.." at "..pos_str(entity_pos))
		return
	end

	local count = 1
	if items.count ~= nil then count=items.count end

	local available_count = player.get_item_count(items.name)

	if available_count < count then
		complain("cannot insert "..count.."x "..items.name..", because player #"..player_id.." only has "..available_count..". clamping...")
		count = available_count
	end

	if count > 0 then
		local real_n = inventory.insert({name=items.name, count=count})

		if count ~= real_n then
			-- The destination took fewer than it was offered. The counts alone
			-- do not say *why*, and the two reasons are opposite outcomes: an
			-- inventory with no room left for this item cannot be made to hold
			-- more by anyone, while an inventory that simply would not take the
			-- item is a real failure. Only this side can see the difference, so
			-- report the destination's own state and let the caller judge --
			-- `judge_transfer_reply` in crates/core/src/factorio/rcon.rs is the
			-- reader, and it treats a suffix it does not recognise as a failure.
			--
			-- Read *after* the insert, so `holds` includes whatever just went
			-- in and `room` is what is left over now.
			local holds = inventory.get_item_count(items.name)
			local room = inventory.get_insertable_count(items.name)
			complain("tried to insert "..count.."x "..items.name.." but inserted "..real_n
				.." (destination holds "..holds..", room for "..room..")")
		end

		local check_n = player.remove_item({name=items.name, count=real_n})
		if check_n ~= real_n then
			complain("wtf, tried to take "..real_n.."x "..items.name.." from player #"..player_id.." but only got "..check_n..". Isn't supposed to happen?!")
		end
	end
	stamp_tick()
end

function rcon_remove_from_inventory(player_id, entity_name, entity_pos, inventory_type, items)
	-- Whatever comes out of the entity goes into this bot. See above.
	mark_bot_inventory_dirty(player_id)
	local player = bot_handle(player_id)
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
		return
	end
	if player.character == nil then
		rcon.print(no_character_error(player_id, player))
		return
	end
	local entity = player.surface.find_entity(entity_name, entity_pos)
	if entity == nil then
		complain("cannot remove from inventory of nonexisting entity "..entity_name.." at "..pos_str(entity_pos))
		return
	end

	local inventory = entity.get_inventory(inventory_type)
	if inventory == nil then
		complain("cannot remove from nonexisting inventory of entity "..entity_name.." at "..pos_str(entity_pos))
		return
	end

	local count = 1
	if items.count ~= nil then count=items.count end
	local real_n = inventory.remove(items)

	if count ~= real_n then
		complain("tried to remove "..count.." "..items.name.." but removed " .. real_n)
	end

	if real_n > 0 then
		local check_n = player.insert({name=items.name, count=real_n})

		if check_n ~= real_n then
			complain("wtf, couldn't insert "..real_n.."x "..items.name.." into player #"..player_id..", but only "..check_n..". dropping them :(.")
		end
	end
	stamp_tick()
end

-- Put a recipe on a crafting machine.
--
-- Stage 2 of the starter factory is an assembling machine *with a recipe on
-- it*. A machine placed, powered and fed with no recipe is dead: it costs
-- materials, occupies ground, passes every geometry check the planner makes,
-- and produces nothing.
--
-- **`LuaEntity.set_recipe` does not return a success flag.** The API
-- (`workspace/factorio-api-docs/runtime-api.json`, 2.1.17) says it returns an
-- array of `ItemWithQualityCount`: "Any items removed from this entity as a
-- result of setting the recipe" -- the old recipe's leftovers, evicted because
-- they no longer belong in the machine. Dropping that array deletes those
-- items from the game with nothing anywhere saying so, which is exactly what
-- discarding `remove_item`'s count, `create_entity`'s optional entity and
-- `player.teleport`'s boolean each cost this project once. So they are handed
-- to the acting bot, who is standing at the machine to operate it.
--
-- And because the return value is not a verdict, the verdict comes from
-- reading the recipe back with `get_recipe()`. Without that read a machine
-- that ignored the call would be reported as configured.
--
-- Three refusals before the game is touched at all, each a sentence rather
-- than a raise inside the remote call:
--
--  * **an unknown recipe** -- the name refers to nothing;
--  * **a recipe the force has not unlocked** -- `enabled` is force-scoped, and
--    `automation-science-pack` is `false` until its trigger technology fires.
--    Naming the recipe matters: a refusal that does not say which recipe and
--    why sends a reader looking at the machine instead of at the research;
--  * **an entity that is not an assembling machine** -- `set_recipe` is
--    defined on the `AssemblingMachine` subclass only, so calling it on a
--    furnace raises, and a raise reaches the executor as an unreadable reply
--    rather than as a refusal it can act on.
--
-- Everything printed here goes through `rcon.print`, which *is* the reply body
-- the executor reads as this action's result. That is why there is no
-- narration on this path: a debug line here would turn a success into a
-- reported failure. Use `writeout` (stdout) if one is ever needed.
function rcon_set_recipe(player_id, entity_name, entity_pos, recipe)
	-- A recipe change evicts the machine's ingredients into the bot. See above.
	mark_bot_inventory_dirty(player_id)
	local player = bot_handle(player_id)
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
		return
	end
	if player.character == nil then
		rcon.print(no_character_error(player_id, player))
		return
	end
	local known = player.force.recipes[recipe]
	if known == nil then
		rcon.print("Error: no such recipe: " .. tostring(recipe))
		return
	end
	if not known.enabled then
		rcon.print("Error: recipe " .. tostring(recipe) .. " is not enabled for this force")
		return
	end
	local entity = player.surface.find_entity(entity_name, entity_pos)
	if entity == nil then
		rcon.print("Error: cannot set a recipe on nonexisting entity " ..
			tostring(entity_name) .. " at " .. pos_str(entity_pos))
		return
	end
	if entity.type ~= "assembling-machine" then
		rcon.print("Error: cannot set a recipe on " .. tostring(entity_name) .. " at " ..
			pos_str(entity_pos) .. ": it is a " .. tostring(entity.type) ..
			", not an assembling-machine")
		return
	end

	local removed = entity.set_recipe(recipe)

	-- The verdict. `set_recipe` answered with items, not with a yes, so this
	-- read is the only thing that distinguishes it having worked from it
	-- having quietly done nothing.
	local current = entity.get_recipe()
	if current == nil or current.name ~= recipe then
		local got = "nothing"
		if current ~= nil then got = tostring(current.name) end
		rcon.print("Error: setting recipe " .. tostring(recipe) .. " on " ..
			tostring(entity_name) .. " at " .. pos_str(entity_pos) ..
			" left it making " .. got)
		return
	end

	-- Whatever the change evicted goes to the bot. `player.insert` returns how
	-- many it actually took -- another return value that must not be dropped,
	-- for the same reason.
	local lost = {}
	if removed ~= nil then
		for _, stack in pairs(removed) do
			local count = stack.count or 0
			if count > 0 then
				local taken = player.insert({name=stack.name, count=count})
				if taken < count then
					table.insert(lost, (count - taken) .. "x " .. tostring(stack.name))
				end
			end
		end
	end
	if #lost > 0 then
		-- The recipe **is** set; what failed is carrying away what the change
		-- threw out. Reporting success would leave the executor believing the
		-- bot holds items it does not, for the rest of the run. Reporting
		-- failure is safe here and nowhere else in this file: setting a recipe
		-- is idempotent, so the retry finds the recipe already set, evicts
		-- nothing and comes back clean.
		rcon.print("Error: recipe " .. tostring(recipe) .. " was set on " ..
			tostring(entity_name) .. " at " .. pos_str(entity_pos) ..
			", but player #" .. tostring(player_id) ..
			" could not carry what it displaced: " .. table.concat(lost, ", "))
		return
	end
	stamp_tick()
end

function rcon_whoami(who)
	if client_local_data.whoami == nil then
		client_local_data.whoami = who
		on_whoami()
	end
end

--function rcon_debug_mine_selected(action_id)
--	rcon_set_mining_target(action_id, game.player.index, game.player.selected.prototype.name, game.player.selected.position)
--end

function rcon_player_info(player_id)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	rcon.print(helpers.table_to_json(serialize_player(player)))
end

function dotted_path_get(tbl, path)
	local dot_pos = path:find(".")

	if dot_pos == nil then
		local array_pos = path:find("[")
		if dot_pos == nil then
			tbl[path] = value
		end
		return tbl[path]
	else
		local left = path:sub(1, dot_pos - 1)
		local right = path:sub(dot_pos + 1)
		if tbl[left] == nil then
			return nil
		end
		return dotted_path_get(tbl[left], right)
	end
end

function dotted_path_set(tbl, path, value)
	local dot_pos = path:find(".")
	if dot_pos == nil then

	else
		local left = path:sub(1, dot_pos - 1)
		local right = path:sub(dot_pos + 1)
		if tbl[left] == nil then
			tbl[left] = {}
		end
		dotted_path_set(tbl[left], right, value)
	end
end


function rcon_store_map_data(key, value)
	if storage.p["map_data"] == nil then
		storage.p["map_data"] = {}
	end
	storage.p["map_data"][key] = value
end

function rcon_retrieve_map_data(key)
	if storage.p["map_data"] == nil then
		return
	end
	if storage.p["map_data"][key] == nil then
		return
	end
	rcon.print(helpers.table_to_json(storage.p["map_data"][key]))
end

function rcon_players()
	local valid_players = {}
	for player_id, player in each_bot() do
		if player.connected and player.character then
			table.insert(valid_players, serialize_player(player))
		end
	end
	rcon.print(helpers.table_to_json(valid_players))
end

function rcon_player_force()
	rcon.print(helpers.table_to_json(collect_player_force()))
end

-- Everything about the world that used to reach Rust only on the server's
-- stdout, returned in one RCON reply.
--
-- This is what makes attaching to a server this process did not start useful:
-- without prototypes, recipes and a force there are no collision boxes, no
-- crafting graph and no technologies, so the planner cannot plan at all.
--
-- Deliberately *not* included, because each is already reachable over RCON and
-- duplicating it here would be a second definition to drift:
--   * entities and resources -- `find_entities_filtered`, which is area-bound;
--     a whole surface would be unbounded.
--   * players                -- `players` / `player_info`.
--   * tiles                  -- `find_tiles_filtered`.
--   * graphics               -- sprite atlas paths, for drawing, not planning.
--
-- One reply, not a paged protocol: Factorio answers an RCON command in a single
-- packet whose length header is a 32-bit int, and this payload is well under a
-- megabyte.
function rcon_world_snapshot()
	rcon.print(helpers.table_to_json({
		entity_prototypes = collect_entity_prototypes(),
		item_prototypes = collect_item_prototypes(),
		recipes = collect_recipes(),
		forces = {collect_player_force()},
		daylight = collect_surface_daylight(),
		surfaces = collect_surfaces(),
	}))
end

-- The furthest a single `generate_chunks` call may reach, in chunks.
--
-- **Four**, which is the reveal a character gets for free by standing
-- somewhere. Measured on a live 2.1.17 server rather than assumed: a character
-- placed on virgin ground causes the engine to generate a 9x9 block of chunks
-- centred on it -- `x 42..50, y 42..50` for a character at (1500, 1500), and
-- `x -51..-43, y 42..50` for one at (-1500, 1500), 81 chunks both times.
--
-- The clamp is the whole honesty argument for this verb, so it lives in the
-- mod where it cannot be argued away by a caller: one call buys exactly the
-- ground a character standing at that point would have been given, and no
-- more. A caller wanting a wider area has to walk a bot and ask again, which
-- is the cost a player pays.
local GENERATE_CHUNKS_MAX_RADIUS = 4

-- Ask the engine to generate the ground around a position.
--
-- **Why this exists.** A bot cannot walk into ungenerated ground: the game's
-- pathfinder returns no path for any destination past the edge of the
-- generated world, so `rcon.move` refuses before dispatching anything.
-- Measured on seed 31337, whose fresh map is 400 chunks spanning
-- `[-320, 320)`: x=100 and x=200 are reached, x=300 through x=600 all fail
-- with `failed to path find`, and a five-leg tour of the four diagonals
-- refused every leg. So exploration cannot be done by walking alone, and this
-- is the missing half.
--
-- **Why it is not the cheat it looks like.** A *human* player walks into
-- unexplored ground all the time -- they hold a key, and the engine generates
-- the ground around them as they go. Our bots cannot only because we drive
-- them through `request_path`, which will not path into chunks that do not
-- exist. That is an artefact of how we control a character, not a rule of the
-- game. Clamped to `GENERATE_CHUNKS_MAX_RADIUS`, this restores the parity a
-- player already has, one reveal at a time.
--
-- **What is still not honest about it**, and why the caller records it: the
-- ground is generated *before* the bot gets there rather than as it arrives,
-- so a plan can see one reveal further than a player would at the same moment.
-- That is small and bounded, and it is disclosed rather than argued away --
-- the executor writes an event naming every call. See
-- `docs/superpowers/notes/2026-09-06-exploration.md`.
--
-- It charts nothing: `force.is_chunk_charted` is false for every chunk this
-- makes, exactly as it is for ground a character is standing on. The world
-- model learns through `on_chunk_generated`, which this fires.
function rcon_generate_chunks(x, y, radius)
	local surface = game.surfaces[1]
	if radius == nil or radius > GENERATE_CHUNKS_MAX_RADIUS then
		radius = GENERATE_CHUNKS_MAX_RADIUS
	end
	if radius < 0 then
		radius = 0
	end
	local before = 0
	for _ in surface.get_chunks() do
		before = before + 1
	end
	surface.request_to_generate_chunks({x, y}, radius)
	-- Synchronously, so the reply describes ground that exists rather than
	-- ground that has been queued: the caller's next act is to walk a bot
	-- there, and a queued chunk would fail the pathfinder exactly as an
	-- ungenerated one does.
	surface.force_generate_chunk_requests()
	local after = 0
	for _ in surface.get_chunks() do
		after = after + 1
	end
	rcon.print(helpers.table_to_json({
		x = x,
		y = y,
		radius = radius,
		chunks_before = before,
		chunks_after = after,
		generated = after - before,
	}))
end

-- The action ids waiting on each technology: `research_actions()[name]` is an
-- array of ids, all of which settle when that technology finishes.
--
-- **In `storage`, not a module local.** `on_load` rebuilds nothing, so a
-- registry kept in a module local is empty after a save/load and every action
-- in flight across it waits out the executor's deadline. A research runs for
-- minutes, which makes it the action most likely to be in flight across a save.
--
-- **Keyed by technology name** because that is the only join the game offers:
-- `on_research_finished` carries the technology and nothing else -- no request
-- id, no queue position. `add_research` appends to the back of a queue that may
-- already hold other technologies, so completions do not arrive in the order
-- they were asked for and a positional match would settle the wrong action.
--
-- **An array per name, not one id**, because two actions may ask for the same
-- technology. Overwriting would leave the first waiting out the executor's
-- whole `ACTION_RESULT_DEADLINE` -- six minutes of silence for an action the
-- game finished.
--
-- Created lazily rather than in `on_init`: `on_init` runs only for a save that
-- never had this mod, and nothing registers `on_configuration_changed`, so a
-- save that gains this version of BotBridge would otherwise reach the handlers
-- with the key absent.
function research_actions()
	if storage.research_actions == nil then
		storage.research_actions = {}
	end
	return storage.research_actions
end

function forget_research_action(technology_name, action_id)
	local waiting = research_actions()[technology_name]
	if waiting == nil then
		return
	end
	for i, id in ipairs(waiting) do
		if id == action_id then
			table.remove(waiting, i)
			break
		end
	end
	if #waiting == 0 then
		research_actions()[technology_name] = nil
	end
end

-- The actions waiting on each craft: `craft_actions()[player_index][recipe]` is
-- an array of `{ id = action_id, remaining = crafts_still_owed }`, oldest
-- first.
--
-- **In `storage`**, for the reason above `research_actions()`. Its predecessor
-- was a module local, which is one of the several ways a craft could stop
-- reporting; see `docs/superpowers/notes/2026-09-02-crafts-that-never-report.md`.
--
-- **Keyed by `(player, recipe)`, and counted rather than positional.**
-- `on_player_crafted_item` carries the player and the recipe and nothing else.
-- The previous shape was one list per player with the requests concatenated in
-- order, matched by comparing the head's recipe to the event's -- and a crafted
-- item that did not match the head was *ignored, leaving the head in place*. So
-- a single entry that would never be crafted stopped every later craft for that
-- bot, permanently, with no timeout and no log line. Run
-- `run-1788347034-00981` lost eleven actions to exactly that shape, each for a
-- full `ACTION_RESULT_DEADLINE`. Buckets per recipe cannot block one another,
-- and a stuck bucket can no longer be created: the two ways to make one -- a
-- partial `begin_crafting` and a cancellation -- are both handled below.
--
-- **Counted, not matched one craft to one request**, because the event cannot
-- tell two requests for the same recipe apart. The game's crafting queue is
-- FIFO, so crafts of a recipe are attributed to the oldest request still owed
-- one. That is an attribution, not a measurement, and it is the strongest claim
-- the event supports.
--
-- Created lazily, for the reason above `research_actions()`.
function craft_actions()
	if storage.craft_actions == nil then
		storage.craft_actions = {}
	end
	return storage.craft_actions
end

-- The bucket for one player and one recipe, or nil when nothing waits on it.
-- With `create`, the bucket is made rather than reported absent.
function craft_waiters(player_index, recipe_name, create)
	local per_player = craft_actions()[player_index]
	if per_player == nil then
		if not create then return nil end
		per_player = {}
		craft_actions()[player_index] = per_player
	end
	local waiting = per_player[recipe_name]
	if waiting == nil then
		if not create then return nil end
		waiting = {}
		per_player[recipe_name] = waiting
	end
	return waiting
end

-- Drop a bucket, and the player's table with it, once they are empty. Without
-- this the registry grows one permanent entry per recipe ever crafted, and
-- `storage` is saved with the map.
function prune_craft_waiters(player_index, recipe_name)
	local per_player = craft_actions()[player_index]
	if per_player == nil then
		return
	end
	local waiting = per_player[recipe_name]
	if waiting ~= nil and #waiting == 0 then
		per_player[recipe_name] = nil
	end
	if next(per_player) == nil then
		craft_actions()[player_index] = nil
	end
end

function forget_craft_action(player_index, recipe_name, action_id)
	local waiting = craft_waiters(player_index, recipe_name)
	if waiting == nil then
		return
	end
	for i, waiter in ipairs(waiting) do
		if waiter.id == action_id then
			table.remove(waiting, i)
			break
		end
	end
	prune_craft_waiters(player_index, recipe_name)
end

-- Count one finished craft against the oldest request still owed one, and
-- settle that request when it is owed no more. Returns the action id settled,
-- or nil -- `on_player_crafted_item` records it on the item addition.
function settle_crafted_item(event)
	local waiting = craft_waiters(event.player_index, event.recipe.name)
	if waiting == nil or #waiting == 0 then
		return nil
	end
	local waiter = waiting[1]
	waiter.remaining = waiter.remaining - 1
	if waiter.remaining > 0 then
		return nil
	end
	table.remove(waiting, 1)
	prune_craft_waiters(event.player_index, event.recipe.name)
	action_completed(event.tick, waiter.id)
	return waiter.id
end

-- Fail the requests a cancellation took crafts from. See
-- `on_player_cancelled_crafting` for why it takes them from the back and why
-- losing any craft fails the whole request.
function fail_cancelled_crafts(event)
	local waiting = craft_waiters(event.player_index, event.recipe.name)
	if waiting == nil then
		return
	end
	local left = event.cancel_count
	while left > 0 and #waiting > 0 do
		local waiter = table.remove(waiting)
		left = left - waiter.remaining
		action_failed(event.tick, waiter.id,
			"the game cancelled " .. tostring(event.cancel_count) ..
			" craft(s) of " .. tostring(event.recipe.name))
	end
	prune_craft_waiters(event.player_index, event.recipe.name)
end

-- Queue a technology for research, and say so when the game will not.
--
-- `action_id` is optional. With one, the caller is *awaiting* the research and
-- `on_research_finished` settles it; without one, this is the old fire-and-
-- forget queueing that `rcon.add_research` and the REST endpoint use.
--
-- Two silent failures used to live here. An unknown name raised inside the
-- remote call, which Factorio reports in the reply body -- fine, except the
-- client discarded that body. And `LuaForce.add_research` returns a boolean
-- saying whether the technology actually entered the queue; throwing it away
-- reported success for the cases that never raise at all, such as a technology
-- that is already researched.
--
-- Checking the name here as well as the boolean is deliberate: `technologies`
-- is the force's own index, so this answers "no such technology" specifically,
-- rather than leaving every refusal to arrive as one undifferentiated raise.
function start_research(technology_name, action_id)
	local force = game.forces["player"]
	if force.technologies[technology_name] == nil then
		rcon.print("Error: no such technology: " .. tostring(technology_name))
		return
	end
	if action_id ~= nil then
		-- Registered *before* `add_research`, not after. Nothing documented
		-- says `on_research_finished` cannot be raised from inside that call,
		-- and if it ever is, the handler has to find the id already there --
		-- otherwise the completion is dropped and the action waits out the
		-- deadline. The refusal path below takes the entry back out.
		local waiting = research_actions()
		if waiting[technology_name] == nil then waiting[technology_name] = {} end
		table.insert(waiting[technology_name], action_id)
	end
	if not force.add_research(technology_name) then
		if action_id ~= nil then
			-- Nothing is waiting on a refusal: leaving the id registered would
			-- let somebody else's research settle this action as a success,
			-- which is the exact overclaim being removed here.
			forget_research_action(technology_name, action_id)
		end
		-- Say which of the several reasons it was. "Refused" alone sends the
		-- caller guessing, and the guesses are all plausible.
		--
		-- **The list has to be complete, or it is worse than saying nothing.**
		-- Rung 7 failed for a reason that was not on it: the technology was
		-- already the force's *current research*, so `add_research` refused
		-- while researched, enabled, trigger and prerequisites all read fine.
		-- Every retry therefore reported four reasons none of which were true
		-- and pointed the reader away from the one that was. A confidently
		-- incomplete diagnostic is worse than one that says "unknown". See
		-- `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`.
		--
		-- `current_research` and `research_queue` are reported for every
		-- refusal, including when there is none, so a reader can tell "nothing
		-- is being researched" from "this build does not report it".
		local tech = force.technologies[technology_name]
		local unmet = {}
		for name, prereq in pairs(tech.prerequisites) do
			if not prereq.researched then
				table.insert(unmet, name)
			end
		end
		local current = "nil"
		if force.current_research ~= nil then
			current = force.current_research.name
		end
		local in_queue = false
		for _, queued in pairs(force.research_queue) do
			if queued.name == technology_name then
				in_queue = true
			end
		end
		rcon.print("Error: cannot research " .. tostring(technology_name) ..
			": researched=" .. tostring(tech.researched) ..
			" enabled=" .. tostring(tech.enabled) ..
			" trigger=" .. tostring(tech.prototype.research_trigger ~= nil) ..
			" unmet_prerequisites=[" .. table.concat(unmet, ",") .. "]" ..
			" current_research=" .. current ..
			" in_queue=" .. tostring(in_queue) ..
			" research_enabled=" .. tostring(force.research_enabled))
		return
	end
	stamp_tick()
end

function rcon_add_research(technology_name)
	start_research(technology_name, nil)
end

-- Start a research the caller will wait for.
--
-- Named `action_start_*` like walking, mining and crafting because it is the
-- same contract: the reply body carries only the tick stamp, and the verdict
-- arrives later as an `action_completed` writeout. Research needs no `on_tick`
-- follower -- the game does the durative work itself and announces the end of
-- it, exactly as crafting does -- so nothing is added to the tick handler and
-- no `on_nth_tick` cadence is registered.
function rcon_action_start_research(action_id, technology_name)
	start_research(technology_name, action_id)
end

function rcon_inventory_contents_at(positions)
	local surface = game.surfaces[1]

	local result = {}

	for k,v in pairs(positions) do
		local entity = surface.find_entity(v.name, v.position)
		if entity ~= nil then
			-- snake_case, because that is what `InventoryResponse`
			-- (crates/core/src/types.rs) reads: it is `rename_all =
			-- "snake_case"`, and its two inventory fields are
			-- `Box<Option<..>>`, which serde does *not* treat as optional --
			-- a missing key is a hard "missing field" error, not a None. The
			-- camelCase spellings this used to emit therefore meant every
			-- `inventory_contents_at` reply failed to deserialise. Nothing
			-- else in the tree spells them camelCase; `serialize_entity` in
			-- types.lua already sends `output_inventory`/`fuel_inventory`.
			local rec = {}
			local output_inventory = entity.get_output_inventory()
			if output_inventory ~= nil then
				rec.output_inventory = output_inventory.get_contents()
			else
				rec.output_inventory = nil
			end
			local fuel_inventory = entity.get_fuel_inventory()
			if fuel_inventory ~= nil then
				rec.fuel_inventory = fuel_inventory.get_contents()
			else
				rec.fuel_inventory = nil
			end
			-- WHAT THE MACHINE WAS GIVEN AND HAS NOT TURNED INTO ANYTHING YET.
			--
			-- `serialize_entity` (types.lua) has sent this since 2026-09-07 and
			-- this path did not, which split the world in half by how you
			-- asked: a furnace's ore was visible when you dumped every entity
			-- and invisible when you asked about that one furnace. This is the
			-- reply `Planner::refresh_buffers` pulls, so it is the one the
			-- planner's own model is built from -- the half that was missing
			-- was the half that mattered.
			--
			-- **`nil` and empty are different answers and must stay
			-- different.** A `wooden-chest` has no input inventory at all and
			-- gets no key, which reaches Rust as `None`; a `stone-furnace`
			-- standing empty gets `{}`, which `option_vec_or_empty_map` reads
			-- as `Some(empty)`. `ObservedInventory::input` is an `Option` for
			-- exactly this reason -- collapsing the two would rebuild the
			-- ambiguity one layer up, where "this machine has nowhere to put
			-- ore" and "this machine is waiting for ore" would read alike.
			--
			-- The index comes from `input_inventory_index` in types.lua rather
			-- than being written out again here: two tables of
			-- `defines.inventory` names is two things a Factorio version can
			-- outgrow separately.
			local input_index = input_inventory_index(entity.type)
			if input_index ~= nil then
				local input_inventory = entity.get_inventory(input_index)
				if input_inventory ~= nil then
					rec.input_inventory = input_inventory.get_contents()
				end
			end
			rec.name = v.name
			rec.position = v.position
			table.insert(result, rec)
		end
	end
	rcon.print(helpers.table_to_json(result))
end

function rcon_find_entities_filtered(filters)
	local results = game.surfaces[1].find_entities_filtered(filters)
	local lines = {}
	for k, v in pairs(results) do
		table.insert(lines, serialize_entity(v))
	end
	rcon.print(helpers.table_to_json(lines))
end


function rcon_find_tiles_filtered(filters)
	local results = game.surfaces[1].find_tiles_filtered(filters)
	local lines = {}
	for k, v in pairs(results) do
		table.insert(lines, serialize_tile(v))
	end
	rcon.print(helpers.table_to_json(lines))
end


-- Start a craft the caller will wait for.
--
-- Same contract as walking, mining and researching: the reply body carries only
-- the tick stamp, and the verdict arrives later as an `action_completed`
-- writeout. The game's own crafting queue does the durative work, so nothing is
-- added to `on_tick` here.
--
-- **Every refusal is answered in the reply body**, which is where
-- `player_craft_timed` reads one, so a craft the game will not do costs a round
-- trip rather than the executor's whole `ACTION_RESULT_DEADLINE`. The name is
-- checked here rather than left to `begin_crafting`, which *raises* on a recipe
-- that does not exist -- a raise inside the remote call is a far worse answer
-- than a sentence.
--
-- **A partial start is a refusal and registers nothing.** `begin_crafting`
-- returns "the count that was actually started crafting", which can be less
-- than the count asked for. The previous version complained *and then
-- registered all `count` crafts anyway*, so the surplus sat in the registry
-- forever with nobody waiting on it -- and under the old positional match that
-- surplus silenced every later craft for that bot. What it does not do is undo
-- the crafts the game did start: those complete, find no waiter and are
-- ignored, which under-claims (the bot ends up holding items the executor was
-- told it did not get) in the direction this codebase chooses everywhere else.
-- Cancelling them instead would need a queue index and can cascade into other
-- crafts, per `LuaControl.cancel_crafting`.
function rcon_action_start_crafting(action_id, player_id, recipe, count)
	local player = bot_handle(player_id)
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
		return
	end
	-- A dead player has a crafting queue only until the character does, and
	-- `begin_crafting` on none is a raise in the reply body rather than a
	-- refusal. Same sentence `get_player` prints, for the same classifier.
	if player.character == nil then
		rcon.print(no_character_error(player_id, player))
		return
	end
	local known = player.force.recipes[recipe]
	if known == nil then
		rcon.print("Error: no such recipe: " .. tostring(recipe))
		return
	end
	if not known.enabled then
		rcon.print("Error: recipe " .. tostring(recipe) .. " is not enabled for this force")
		return
	end

	-- Registered *before* `begin_crafting`, and taken back out below if the
	-- game refuses: nothing documented says a crafted-item event cannot be
	-- raised from inside that call, and a completion arriving before the id is
	-- there would be dropped and cost the whole deadline. Same reasoning as
	-- `start_research`.
	local waiting = craft_waiters(player.index, recipe, true)
	table.insert(waiting, { id = action_id, remaining = count })

	-- Ingredients leave the inventory and the queue becomes non-empty. Both
	-- flags are set before the call, not after, because `begin_crafting` may
	-- raise and the state it leaves behind still has to be polled.
	mark_bot_inventory_dirty(player_id)
	if is_character_bot(player_id) then storage.bots[player_id].craft_active = true end
	local ret = player.begin_crafting{count=count, recipe=recipe}
	if ret ~= count then
		forget_craft_action(player.index, recipe, action_id)
		rcon.print("Error: could not have player " .. player.name .. " craft " ..
			tostring(count) .. " " .. tostring(recipe) ..
			" (the game started " .. tostring(ret) .. ")")
		return
	end
	stamp_tick()
end

function rcon_revive_ghost(player_id, name, x, y)
	-- Reviving spends the item. See above.
	mark_bot_inventory_dirty(player_id)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local main_inventory = player.get_main_inventory()
	local contents = inventory_counts(main_inventory)
	if contents[name] == nil or contents[name] < 1 then
		complain("Error: player has no " .. name)
		return
	end
	local ghosts = player.surface.find_entities_filtered({
		ghost_name = name,
		position = {x = x, y = y},
	})
	local ghost = nil
	for _,v  in pairs(ghosts) do
		ghost = v
	end
	if ghost == nil then
		complain("Error: failed to find ghost")
		return
	end
	local success, entity = ghost.revive()
	if entity ~= nil then
		main_inventory.remove({name=name, count=1})
		rcon.print(helpers.table_to_json(serialize_entity(entity)))
	else
		local prototype = prototypes.entity[ghost.ghost_name]
		local bb = add_to_bounding_box(expand_rect_floor_ceil(prototype.collision_box), {x = ghost.position.x, y = ghost.position.y})
		--				print("ghost bb: " .. helpers.table_to_json(bb))
		if position_in_rect(player.position, bb) then
			local dest = {x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1}
			teleport_writeout(game.tick, player_id, "revive_ghost_blocked", player.position, dest, nil)
			player.teleport(dest)
			local success, entity = ghost.revive()
			if entity ~= nil then
				main_inventory.remove({name=name, count=1})
				rcon.print(helpers.table_to_json(serialize_entity(entity)))
			else
				complain("Error: failed to revive ghost")
			end
		else
			complain("Error: failed to revive ghost")
		end
	end
end

-- Give a player items, and say so when it could not give them all.
--
-- `LuaControl.insert` returns how many items actually went in. A full or
-- filtered inventory takes fewer -- possibly none -- and discarding the count
-- reported success for a bot that was never stocked, which every later step
-- then assumed. `get_player` already reports an absent player, and that
-- refusal now reaches the caller too.
function rcon_cheat_item(player_id, item, count)
	mark_bot_inventory_dirty(player_id)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local inserted = player.insert{name=item, count=count}
	if inserted ~= count then
		rcon.print("Error: inserted " .. tostring(inserted) .. " of " ..
			tostring(count) .. " " .. tostring(item) .. " for player " ..
			tostring(player_id) .. " (inventory full or filtered)")
	end
end

function rcon_cheat_technology(tech)
	local force = game.forces["player"]
	-- Name the problem. Without this an unknown technology arrives as
	-- "attempt to index field '?' (a nil value)", which is true and useless.
	if force.technologies[tech] == nil then
		rcon.print("Error: no such technology: " .. tostring(tech))
		return
	end
	force.technologies[tech].researched=true
end

function rcon_cheat_all_technologies()
	local force = game.forces["player"]
	force.research_all_technologies()
end

-- Charge one `item` to the bot that was found holding it, and report whether it
-- was actually paid for.
--
-- The holder is whichever bot the inventory search in `rcon_place_blueprint`
-- found with the item, which is NOT necessarily the bot placing the blueprint.
-- Charging the placing bot unconditionally -- as the first revive path did
-- until 2026-08-31 -- builds the entity for free whenever a helper supplied the
-- material: `remove` finds nothing to take, returns 0, and that discarded
-- return value is what kept it silent.
function charge_item_to(holder_player_id, item)
	local holder = get_player(holder_player_id)
	if holder == nil then
		return false
	end
	return holder.get_main_inventory().remove({name=item, count=1}) == 1
end

-- Translate this mod's caller-facing `force_build` boolean into the parameter
-- `build_blueprint` has actually taken since Factorio 2.0.
--
-- **The old name was not renamed away, it was silently ignored.** 1.1's
-- `build_blueprint` took `force_build :: boolean`; 2.x takes `build_mode ::
-- defines.build_mode`. Verified against
-- workspace/factorio-api-docs/runtime-api.json (Factorio 2.1.17, runtime api
-- 6): `LuaItemCommon::build_blueprint` declares {surface, force, position,
-- direction, build_mode, skip_fog_of_war, by_player, raise_built} and there is
-- no `force_build` on it at all. An unknown key in a `takes_table` call is
-- simply not read, so every `force_build = true` since the 2.0 port has been
-- discarded and the *default* used instead -- and the default is the opposite
-- of what the caller asked for. The docs for the parameter, verbatim: "If
-- `normal`, blueprint will not be built if any one thing can't be built. If
-- `forced`, anything that can be built is built and obstructing nature
-- entities will be deconstructed. If `superforced`, all obstructions will be
-- deconstructed and the blueprint will be built", defaulting to `normal`. So
-- `force_build = true` -- "build what you can" -- has been getting
-- all-or-nothing.
--
-- `superforced` is deliberately not reachable from the boolean: it
-- deconstructs *all* obstructions, which is a bigger promise than any caller
-- here has made.
--
-- Do not confuse this with `can_place_entity`, which `placement_check_args`
-- above documents: that method has no build_mode/force_build parameter in any
-- version, and its `forced` field is a different thing read only for the ghost
-- check types.
--
-- One helper for both blueprint call sites (`rcon_place_blueprint` and
-- `rcon_cheat_blueprint`) so the two cannot drift into asking for different
-- build modes from the same flag.
function blueprint_build_mode(force_build)
	if force_build then
		return defines.build_mode.forced
	end
	return defines.build_mode.normal
end

function rcon_place_blueprint(player_id, blueprint, pos_x, pos_y, direction, force_build, only_ghosts, inventory_player_ids)
	-- A block is paid for out of a *list* of bots, and which of them actually
	-- gave up an item is decided deep inside. Marking the whole roster costs
	-- each bot one scan and is the honest statement of what is known here.
	mark_bot_inventory_dirty(nil)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local bp_entity = player.surface.create_entity{name='item-on-ground',position= {pos_x, pos_y}, stack='blueprint' }
	-- 0 if the import succeeded with no errors. -1 if the import succeeded with errors. 1 if the import failed.
	local success = bp_entity.stack.import_stack(blueprint)

	if success == 1 then
		complain("{\"error\": \"blueprint import failed\"}")
		bp_entity.destroy()
		return
	end
	if success == -1 then
		complain("{\"error\": \"blueprint import had errors\"}")
	end
	local ghosts = bp_entity.stack.build_blueprint({
		surface = player.surface,
		force = player.force,
		position = { pos_x, pos_y },
		-- by_player :: PlayerSpecification (optional): The player to use if any. If provided defines.events.on_built_entity will also be fired on successful entity creation.
		by_player = player_identification(player_id),
		-- direction :: defines.direction (optional): The direction to use when building
		direction = direction,
		-- build_mode :: defines.build_mode (optional), 2.0's replacement for
		-- 1.1's `force_build` boolean -- see `blueprint_build_mode`.
		build_mode = blueprint_build_mode(force_build)
	})
	bp_entity.destroy()

	local result = {}
	local main_inventory = player.get_main_inventory()
	local nothing = true
	for _, ghost in pairs(ghosts) do
		-- Reviving an entity destroys any ghost that overlapped it, so a ghost
		-- captured by `build_blueprint` above can already be invalid by the time
		-- this loop reaches it. Touching one raises, which aborted the whole
		-- placement and hid every entity that did get built. Skip it instead: a
		-- blueprint whose entities cannot all coexist should show up as a short
		-- result the caller can compare against what it asked for.
		if not ghost.valid then goto continue end
		nothing = false
		local item = ghost.ghost_name
		local item_source_player_id
		local inventory = inventory_counts(main_inventory)
		if inventory[item] ~= nil and inventory[item] > 0 then
			item_source_player_id = player_id
		else
			for _, inventory_player_id in pairs(inventory_player_ids) do
				local inventory_player = inventory_counts(get_player(inventory_player_id).get_main_inventory())
				if inventory_player[item] ~= nil and inventory_player[item] > 0 then
					item_source_player_id = inventory_player_id
				end
			end
		end
		if only_ghosts == false and item_source_player_id ~= nil then
			local success, entity = ghost.revive()
			if entity ~= nil then
				if charge_item_to(item_source_player_id, item) then
					table.insert(result, serialize_entity(entity))
				else
					-- The search above found a bot holding this item, so failing
					-- to take it would leave the entity standing unpaid for. Undo
					-- the build rather than report a placement nobody paid for.
					writeout(game.tick, "place_blueprint_unpaid", item)
					entity.destroy()
				end
			else
				local prototype = prototypes.entity[item]
--				print("player position: " .. helpers.table_to_json(player.position))
--				print("ghost position: " .. helpers.table_to_json(ghost.position))
--				print("ghost collision_box: " .. helpers.table_to_json(prototype.collision_box))
				local bb = add_to_bounding_box(expand_rect_floor_ceil(prototype.collision_box), {x = ghost.position.x, y = ghost.position.y})
--				print("ghost bb: " .. helpers.table_to_json(bb))
				if position_in_rect(player.position, bb) then
					local dest = {x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1}
					teleport_writeout(game.tick, player_id, "place_blueprint_blocked", player.position, dest, nil)
					player.teleport(dest)
					local success, entity = ghost.revive()
					if entity ~= nil then
						if charge_item_to(item_source_player_id, item) then
							table.insert(result, serialize_entity(entity))
						else
							writeout(game.tick, "place_blueprint_unpaid", item)
							entity.destroy()
						end
					else
						table.insert(result, serialize_entity(ghost))
					end
				else
					table.insert(result, serialize_entity(ghost))
				end
			end
		else
			table.insert(result, serialize_entity(ghost))
		end
		::continue::
	end
	-- DEFERRED GHOST WRITEOUT, under test. See
	-- docs/superpowers/notes/2026-09-06-ghosts-cannot-be-written-from-rcon.md
	--
	-- `writeout` from inside this function is swallowed: measured with a
	-- non-ghost control that also never arrived, so it is the channel and not
	-- the record. Factorio redirects console output to the RCON client while a
	-- command is in flight, and `writeout` is one `print`.
	--
	-- `todo_next_tick` is drained inside `on_tick`, an ordinary event handler
	-- with ordinary stdout. Records are captured by value because the ghosts
	-- may be revived or destroyed before the tick runs, and touching an invalid
	-- entity raises.
	--
	-- `todo_next_tick` rather than `todo_next_tick_other`: the drain is an
	-- `if/elseif`, so the "other" queue only runs on a tick where the first is
	-- empty. That is a second way to get nothing, and this test is trying to
	-- isolate one variable.
	local ghost_records = {}
	for _, entry in pairs(result) do
		if entry.name == "entity-ghost" then
			ghost_records[#ghost_records + 1] = helpers.table_to_json(entry)
		end
	end
	if #ghost_records > 0 then
		table.insert(todo_next_tick, function()
			for _, record in ipairs(ghost_records) do
				writeout(game.tick, "on_some_entity_created", record)
			end
		end)
	end
	if nothing == true then
		rcon.print("Error: failed to build anything")
	else
		rcon.print(helpers.table_to_json(result))
	end
end


function rcon_cheat_blueprint(player_id, blueprint, pos_x, pos_y, direction, force_build)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local surface = player.surface
	local bp_entity = surface.create_entity{name='item-on-ground',position= {pos_x, pos_y}, stack='blueprint' }
	-- 0 if the import succeeded with no errors. -1 if the import succeeded with errors. 1 if the import failed.
	local success = bp_entity.stack.import_stack(blueprint)
	if success == 1 then
		complain("{\"error\": \"blueprint import failed\"}")
		bp_entity.destroy()
		return
	end
	if success == -1 then
		print("{\"error\": \"blueprint import had errors\"}")
	end
	local ghosts = bp_entity.stack.build_blueprint({
		surface = player.surface,
		force = player.force,
		position = { pos_x, pos_y },
		-- by_player :: PlayerSpecification (optional): The player to use if any. If provided defines.events.on_built_entity will also be fired on successful entity creation.
		by_player = player_identification(player_id),
		-- direction :: defines.direction (optional): The direction to use when building
		direction = direction,
		-- build_mode :: defines.build_mode (optional), 2.0's replacement for
		-- 1.1's `force_build` boolean -- see `blueprint_build_mode`.
		build_mode = blueprint_build_mode(force_build)
	})
	bp_entity.destroy()
	local result = {}
	for _, ghost in pairs(ghosts) do
		local success, entity = ghost.revive()
		if entity ~= nil then
			table.insert(result, serialize_entity(entity))
		else
			table.insert(result, serialize_entity(ghost))
		end
	end
	rcon.print(helpers.table_to_json(result))
end

-- Produces the map-exchange string for the map this server is running -- the
-- counterpart of `rcon_parse_map_exchange_string` below, which only ever
-- consumed one.
--
-- **A seed is not a map.** A map is noise-generated from the seed *plus* the
-- map-gen settings (resource frequency/size/richness, water, trees, cliffs),
-- so the same seed under different settings, or on a Factorio version whose
-- defaults moved, is a different map. The exchange string encodes seed and
-- settings together, which is why it -- and not the seed -- is a map's
-- identity. Nothing in this project could produce one until 2026-09-06, which
-- is why every archived run carries `map_exchange_string: null`.
--
-- `game.get_map_exchange_string()` is documented as "the map exchange string
-- for the map generation settings that were used to create this map", which is
-- the identity wanted, rather than `surface.get_map_exchange_string()`'s
-- *current* settings for one surface.
--
-- `rcon.print`, not `writeout`: this is a question asked over RCON and answered
-- in the reply body. It is never called from inside an action's mod function,
-- so it cannot contaminate one's result.
function rcon_map_exchange_string()
	rcon.print(game.get_map_exchange_string())
end

function rcon_parse_map_exchange_string(name, map_exchange_str)
	helpers.write_file(name, helpers.table_to_json(helpers.parse_map_exchange_string(map_exchange_str)))
end

-- Asks the game for a path a *character* can walk, from where that character
-- stands to `goal`, and answers with the request handle.
--
-- Split out of `rcon_async_request_player_path` so the request shape has one
-- home. The bounding box, the collision mask and `entity_to_ignore` are what
-- make this a character's path rather than a generic one, and two copies of
-- that would drift. It had a second caller once, the mod's own stuck-walk
-- re-path; that is gone, and retrying a stuck walk lives in `move_player_timed`
-- (crates/core/src/factorio/rcon.rs) where the goal and the radius are.
--
-- **No RCON output of any kind**, for the same reason `start_walk_waypoints`
-- has none: this is called from `on_tick`, where there is no calling RCON
-- interface, and from inside a reply body that a caller reads as a verdict.
function request_player_path(player, goal, radius)
	if player == nil or player.character == nil then
		return nil
	end
	return player.surface.request_path({
		bounding_box = player.character.prototype.collision_box,
		collision_mask = player.character.prototype.collision_mask,
		start = player.position,
		goal = goal,
		force = player.force,
		radius = radius,
		pathfind_flags = {
			allow_destroy_friendly_entities = false,
			prefer_straight_paths = true,
			-- **Set, because the default is `true` and the default is wrong
			-- for us.** The 2.1 docs say a cached path "might fail to respond
			-- to changes in the environment", and we are what changes it: bots
			-- place furnaces, drills and belts on the tiles they are about to
			-- walk across.
			--
			-- This is also the experiment that tests that explanation. Run 30
			-- returned 12 of 75 paths with a waypoint strictly *inside* a
			-- furnace we had placed, every one flagged
			-- `needs_destroy_to_reach = false` -- the game saying it believes
			-- the tile is clear. If stale caching is the cause, that ratio
			-- goes to zero on the next run. If it does not, this flag is not
			-- the answer and the note should say so rather than leave it here
			-- looking settled. See
			-- `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`.
			cache = false,
		},
		entity_to_ignore = player.character,
	})
end

function rcon_async_request_player_path(player_id, goal, radius)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	rcon.print(request_player_path(player, goal, radius))
end

function rcon_async_request_path(start, goal, radius)
	local handle = game.surfaces[1].request_path({
		start = start,
		goal = goal,
		force = game.forces[1],
		radius = radius,
		pathfind_flags = {
			allow_destroy_friendly_entities = false,
			prefer_straight_paths = true,
		}
	})
	rcon.print(handle)
end

function rcon_test(foo)
end


function rcon_screenshot(args)
	game.take_screenshot(args)
end


function table_keys(tbl)
	local result = "["
	local found = false
	for k, v in pairs(tbl) do
		result = result..k..", "
		found = true
	end
	if found == true then
		result = result:sub(1, result:len()-2)
	end
	return result.."]"
end

function table_to_string(tbl)
	local result = "{"
	for k, v in pairs(tbl) do
		-- Check the key type (ignore any numerical keys - assume its an array)
		if type(k) == "string" then
			result = result.."[\""..k.."\"]".."="
		end

		-- Check the value type
		if type(v) == "table" then
			result = result..table_to_string(v)
		elseif type(v) == "boolean" then
			result = result..tostring(v)
		else
			result = result.."\""..v.."\""
		end
		result = result..","
	end
	-- Remove leading commas from the result
	if result ~= "" then
		result = result:sub(1, result:len()-1)
	end
	return result.."}"
end

-- Why a connected player has no character right now, or nil when it has one.
--
-- Three causes, and they used to be one word. `get_player` answered `not
-- connected` for a player whose `character` was nil, which is what a DEAD
-- player looks like: Factorio keeps the `LuaPlayer`, drops the character,
-- and respawns one after `ticks_to_respawn` (600 by default -- the
-- character prototype's `respawn_time` of 10 s). So a bot killed by a worm
-- was indistinguishable, in every reply the executor reads, from a client
-- that never connected -- and neither the record nor `just analyse` could
-- say a bot had died at all. `docs/superpowers/specs/2026-09-04-exploration-design.md`
-- names this as the prerequisite for sending a bot past the nest-free radius.
--
-- The wording is load-bearing on the Rust side: `classify_failure` and
-- `classify_walk_failure` (crates/scripting_lua/src/globals/record.rs) match
-- `has no character` and read `respawns in <n> ticks` out of the detail, and
-- a test there pins both strings. Change one, change the other.
--
-- Every read is under `pcall`: `ticks_to_respawn` is documented `uint32?`
-- and a property read that raises inside a remote call would turn a precise
-- refusal into a Lua traceback in the reply body.
function character_missing_reason(player)
	if player == nil or player.character ~= nil then
		return nil
	end
	local ok, ticks = pcall(function() return player.ticks_to_respawn end)
	if ok and type(ticks) == "number" then
		return "dead, respawns in " .. tostring(math.floor(ticks)) .. " ticks"
	end
	local ok2, controller = pcall(function() return player.controller_type end)
	if ok2 and controller == defines.controllers.cutscene then
		return "in a cutscene"
	end
	if ok2 and controller ~= nil then
		for name, value in pairs(defines.controllers) do
			if value == controller then
				return "controller '" .. tostring(name) .. "'"
			end
		end
	end
	return "no character, cause unknown"
end

-- The one sentence every entry point prints for a character-less player.
-- Prefixed `Error:` like its siblings so the Rust side's "any reply is an
-- error" rule reads it the same way; `has no character` is the substring the
-- classifier keys on.
function no_character_error(player_id, player)
	return "Error: player " .. tostring(player_id) .. " has no character: "
		.. tostring(character_missing_reason(player))
end


-- ---------------------------------------------------------------------------
-- Character bots
--
-- A bot is either a connected player (a graphical client) or a server-side
-- `character` entity this mod created (`rcon_spawn_bots`). The executor
-- addresses both by one small integer, and `bot_handle` is the one place
-- that resolves it. A character bot answers through a proxy: the members a
-- `LuaPlayer` has and a `LuaEntity` lacks (`connected`, `character`, `index`,
-- `name`, `print`, `ticks_to_respawn`, `controller_type`) are answered here;
-- everything else -- `walking_state`, `mining_state`, `position`, `surface`,
-- `force`, `update_selected_entity`, `can_reach_entity`, `get_main_inventory`,
-- `begin_crafting`, the reach distances -- is `LuaControl`, which the
-- character entity shares with a player, so it is forwarded as is. Factorio
-- API methods are dot-called with no `self`, which is what makes forwarding a
-- function value correct.
--
-- What a character bot does NOT get is any `on_player_*` event: none fires
-- for an entity without a player. `poll_character_bots` (on_tick) reads the
-- same facts off the entity each tick and emits the same writeouts and
-- settles, so the Rust side cannot tell the two kinds apart.
--
-- Ids: character bots are 1..N and so is the first joining player, so a run
-- is all clients or all characters -- `rcon_spawn_bots` and
-- `on_player_joined_game` each refuse the other kind by name.
--
-- The proxy is rebuilt on every `bot_handle` call and never stored: a
-- metatable does not survive `storage`. The registry entry does, entity
-- reference included, which is how a savepoint resume finds its bots.
-- ---------------------------------------------------------------------------

CHARACTER_RESPAWN_TICKS = 600 -- a player's respawn delay, 10 s

local CHARACTER_PROXY_OWN = {
	connected = function(b) return b.entity ~= nil and b.entity.valid end,
	character = function(b) if b.entity ~= nil and b.entity.valid then return b.entity end end,
	index = function(b) return b.id end,
	name = function(b) return b.name end,
	print = function() return function() end end,
	ticks_to_respawn = function(b) if b.respawn_at ~= nil then return b.respawn_at - game.tick end end,
	controller_type = function() return defines.controllers.character end,
	-- LuaPlayer has this as an attribute and a character entity has it too;
	-- listed so a dead bot answers 0 rather than raising.
	crafting_queue_size = function(b) if b.entity ~= nil and b.entity.valid then return b.entity.crafting_queue_size end return 0 end,
}

local function character_proxy(id, bot)
	local state = { id = id, name = bot.name, entity = bot.entity, respawn_at = bot.respawn_at }
	return setmetatable({}, {
		__index = function(_, key)
			local own = CHARACTER_PROXY_OWN[key]
			if own ~= nil then return own(state) end
			local ent = state.entity
			if ent == nil or not ent.valid then
				error("bot " .. tostring(id) .. " has no character (entity gone) while reading `" .. tostring(key) .. "`")
			end
			return ent[key]
		end,
		__newindex = function(_, key, value)
			local ent = state.entity
			if ent == nil or not ent.valid then
				error("bot " .. tostring(id) .. " has no character (entity gone) while writing `" .. tostring(key) .. "`")
			end
			ent[key] = value
		end,
	})
end

-- `storage` is nil in the stub interpreter the Rust tests run this file
-- in, so every reader below tolerates its absence: no storage, no bots.
function character_bots()
	if storage == nil then return nil end
	return storage.bots
end

function is_character_bot(id)
	local bots = character_bots()
	return bots ~= nil and bots[id] ~= nil
end

function has_character_bots()
	local bots = character_bots()
	return bots ~= nil and next(bots) ~= nil
end

-- What to pass to a Factorio field that wants a **PlayerIdentification** --
-- a `LuaPlayer`, a player index, or a player name -- and not a handle.
--
-- `bot_handle` answers every *read* the mod performs, which is why the proxy
-- reached these call sites unnoticed: `player.force` and `player.surface` are
-- fine, and then the engine rejects the table itself with
-- `Invalid PlayerIdentification. Expected LuaPlayer, index or name`. In the
-- first headless run every single placement failed that way and the run
-- halted `stuck` at milestone 1 with 165 steps planned and 11 succeeded.
--
-- For a connected player the bot id **is** the player index, so returning it
-- passes exactly the identification the call site passed before. For a
-- character bot there is nothing to name, and the field is omitted -- it is
-- optional at all three sites. The cost of omitting it is stated where it
-- matters: `create_entity` uses `player` only for the build's attribution and
-- for where `spill` puts leftovers, but `build_blueprint`'s `by_player` is
-- also what raises `on_built_entity`, so a blueprint built by a character bot
-- creates its entities without that event. Nothing in this mod's own
-- bookkeeping reads it (`on_some_entity_created` covers the paths the
-- executor uses), and character bots do not build blueprints today; if that
-- changes, this is the line to revisit.
function player_identification(player_id)
	if is_character_bot(player_id) then
		return nil
	end
	return player_id
end

function bot_handle(id)
	if is_character_bot(id) then
		return character_proxy(id, storage.bots[id])
	end
	return game.players[id]
end

-- Every bot the run has: players first, then character bots. Returns
-- `(id, handle)` pairs; callers keep their `player.connected and
-- player.character` guard, which the proxy answers.
function each_bot()
	local list = {}
	for idx, player in pairs(game.players) do list[#list + 1] = { idx, player } end
	for id, bot in pairs(character_bots() or {}) do list[#list + 1] = { id, character_proxy(id, bot) } end
	local i = 0
	return function()
		i = i + 1
		local entry = list[i]
		if entry ~= nil then return entry[1], entry[2] end
	end
end

-- The id of the character bot that owns `entity`, or nil.
--
-- Matched by identity first -- two `LuaEntity` values for the same entity
-- compare equal -- and by `unit_number` second, which is what a stub game
-- can supply and what survives the registry entry being a different Lua
-- value from the one `find_entities_filtered` handed back. `valid` is tested
-- against `false` rather than for truth so an entity that never had the
-- field (a stub) is not mistaken for a dead one.
function character_bot_id_of(entity)
	local bots = character_bots()
	if bots == nil or entity == nil or entity.valid == false
		or (entity.name ~= "character" and entity.type ~= "character") then
		return nil
	end
	for id, bot in pairs(bots) do
		local ent = bot.entity
		if ent ~= nil and ent.valid ~= false then
			if ent == entity then return id end
			local n = ent.unit_number
			if n ~= nil and n == entity.unit_number then return id end
		end
	end
	return nil
end

-- **The one way to get from a character entity to the bot it is.** Returns
-- `(bot id, handle)` -- the handle being what `bot_handle` gives for that id
-- -- or nil for a character nobody claims.
--
-- `LuaEntity.player` is "the player connected to this character, if any"
-- **[V]** (runtime-api.json, Factorio 2.1.17, api 6), and a character bot has
-- no player, so every site that identified a blocking character by that field
-- saw a headless roster as a crowd of undriven strangers: `walk_stall_describe`
-- reported a walking bot 1 as `character (no player)`, and
-- `step_aside_from_footprint` asked nobody to move, which is what turned a
-- transient into three failed placements and a replan in
-- `run-1788608648-56109`. A blocker is a *bot* in both modes, and the answer
-- has to be the same small integer the executor addresses it by.
function bot_of_character(entity)
	if entity == nil then return nil end
	local player = entity.player
	if player ~= nil then return player.index, player end
	local id = character_bot_id_of(entity)
	if id == nil then return nil end
	return id, bot_handle(id)
end

function rcon_spawn_bots(count)
	for _, player in pairs(game.connected_players) do
		rcon.print("Error: cannot spawn character bots: player " .. player.name ..
			" is connected; a run is all clients or all characters")
		return
	end
	storage.bots = storage.bots or {}
	local surface = game.surfaces[1]
	local force = game.forces["player"]
	local spawned, kept = {}, {}
	-- Where the characters that survive from a previous call stand, so the
	-- new ones are placed clear of them too. See `character_spawn_position`.
	local taken = {}
	for _, bot in pairs(storage.bots) do
		if bot.entity ~= nil and bot.entity.valid then
			local p = bot.entity.position
			taken[#taken + 1] = { x = p.x, y = p.y }
		end
	end
	for id = 1, count do
		local bot = storage.bots[id]
		if bot ~= nil and bot.entity ~= nil and bot.entity.valid then
			kept[#kept + 1] = id
		else
			local ent = create_bot_character(surface, force, id, taken)
			storage.bots[id] = { entity = ent, name = "bot-" .. id }
			-- The same starting inventory a joining player gets from freeplay.
			if remote.interfaces["freeplay"] and remote.interfaces["freeplay"]["get_created_items"] then
				for name, n in pairs(remote.call("freeplay", "get_created_items")) do
					ent.insert{ name = name, count = n }
				end
			end
			spawned[#spawned + 1] = id
		end
		if storage.p[id] == nil then storage.p[id] = {} end
		announce_character_bot(game.tick, id)
	end
	on_player_changed_distance({ tick = game.tick })
	rcon.print(helpers.table_to_json({ spawned = spawned, kept = kept }))
end

--- How far apart character bots are spawned, in tiles, and how many lattice
--- points past its own a bot may try before falling back to the game's own
--- search.
---
--- **Half a tile is not apart.** The first eight-bot run
--- (`run-1788614781-38058`) spawned its characters through
--- `find_non_colliding_position("character", spawn, 32, 0.5)`, which answered
--- `(0,0) (-0.5,-0.5) (-0.5,0.5) (0.5,-0.5) (0,-0.5) (0,0.5) (-0.5,0) (0.5,0)`:
--- eight characters inside one two-by-two tile square, legal by the collision
--- box (0.4 across) and useless to the pathfinder, which refused the first walk
--- of bots 1, 5 and 6 -- the three at `x = 0`, each with two neighbours on its
--- own tile -- with `failed to path find` from the spawn itself. The executor's
--- walk memory then learned three perfectly reachable destinations as
--- unreachable. A player joining a server is placed the same way and the same
--- thing would happen to eight of them, except that players move on their own
--- and a headless roster waits to be told.
---
--- So each bot gets its own tile, two apart, on a spiral about the spawn in
--- bot-id order -- deterministic, so bot 3 stands where bot 3 stood last run
--- -- and the game is asked only to confirm that tile centre (or the nearest
--- within a tile of it) is standable. A candidate within a tile of a character
--- already placed is skipped for the next lattice point, so the answer holds
--- whether or not the game counts characters as colliding.
CHARACTER_SPAWN_SPACING = 2
CHARACTER_SPAWN_TRIES = 64

-- The n-th point (n >= 1) of a square spiral on the integer lattice: ring 0 is
-- the origin, ring k the 8k points at Chebyshev distance k, each ring in a
-- fixed row-major order. Pure, so a test can enumerate it.
function character_spawn_offset(n)
	if n <= 1 then return { x = 0, y = 0 } end
	local rest = n - 1
	local k = 1
	while rest > 8 * k do
		rest = rest - 8 * k
		k = k + 1
	end
	local i = 0
	for dy = -k, k do
		for dx = -k, k do
			if math.max(math.abs(dx), math.abs(dy)) == k then
				i = i + 1
				if i == rest then return { x = dx, y = dy } end
			end
		end
	end
	return { x = k, y = k }
end

-- Where bot `id` is spawned: its own tile on the spiral, confirmed by the
-- game, and at least a tile from every position in `taken` (which this
-- function appends to). Falls back to the game's own wide search only when
-- every lattice point tried is water, cliff or somebody else.
function character_spawn_position(surface, origin, id, taken)
	local base = { x = math.floor(origin.x) + 0.5, y = math.floor(origin.y) + 0.5 }
	local n = id
	for _ = 1, CHARACTER_SPAWN_TRIES do
		local off = character_spawn_offset(n)
		local candidate = {
			x = base.x + off.x * CHARACTER_SPAWN_SPACING,
			y = base.y + off.y * CHARACTER_SPAWN_SPACING,
		}
		local pos = surface.find_non_colliding_position("character", candidate, 1, 0.5, true)
		if pos ~= nil then
			local clear = true
			for _, other in ipairs(taken) do
				if math.max(math.abs(other.x - pos.x), math.abs(other.y - pos.y)) < 1 then
					clear = false
					break
				end
			end
			if clear then
				taken[#taken + 1] = { x = pos.x, y = pos.y }
				return pos
			end
		end
		n = n + 1
	end
	local pos = surface.find_non_colliding_position("character", origin, 32, 0.5) or origin
	taken[#taken + 1] = { x = pos.x, y = pos.y }
	return pos
end

function create_bot_character(surface, force, id, taken)
	local origin = force.get_spawn_position(surface)
	local pos = character_spawn_position(surface, origin, id or 1, taken or {})
	return surface.create_entity{ name = "character", position = pos, force = force }
end

-- What a join emits for a player: inventory and position. The distance
-- writeout is force-wide and the caller emits it once.
function announce_character_bot(tick, id)
	local bot = storage.bots[id]
	local handle = bot_handle(id)
	bot.last_pos = nil
	bot.last_inventory = nil
	bot.last_queue = nil
	-- An announcement is unconditional: it must not wait for this bot's turn
	-- in the stagger, and it must read the crafting queue once even on a bot
	-- resumed from a savepoint mid-craft.
	bot.inventory_dirty = true
	bot.craft_active = true
	poll_character_bot(tick, id, handle)
end

function rcon_set_game_speed(v)
	game.speed = v
	rcon.print(tostring(game.speed))
end

function rcon_game_speed()
	rcon.print(tostring(game.speed))
end

-- Stops or restarts the game clock, and answers with the tick it did so at.
--
-- `game.tick_paused` freezes `game.tick` -- machines, characters, the
-- `on_tick` polling above -- while RCON is still served, so the executor can
-- ask the game questions (`can_place_entity`, `inventory_contents_at`) in a
-- world that is not moving. It is what `goal.plan` wraps around expansion:
-- the planner is wall-clock work, and a game left running through it is
-- charged `60 * game.speed` ticks per second of thinking -- 334 ticks for
-- automation at 1x, 1,837 at 10x, 6,438 for green at 5x -- which is the whole
-- of the "faster game, longer run" tax measured on 2026-09-05.
--
-- The stamp is the reply's tick, read *after* the assignment, so a resume
-- answers with the tick the clock restarted from and a pause with the tick it
-- stopped at. Asking for the state it already has is a no-op, so an unpause
-- issued against a running game is safe -- and it is issued on every plan's
-- exit path, error or not, for exactly that reason.
function rcon_set_tick_paused(v)
	game.tick_paused = (v == true or v == "true")
	stamp_tick()
	rcon.print(tostring(game.tick_paused))
end

-- ---------------------------------------------------------------------------
-- How often a bot's main inventory is scanned, and why it is not every tick.
--
-- **This is a throughput change for headless iteration, not a correctness or
-- playability fix.** Measured on 2026-09-06 by two-point differencing at
-- 60,000 and 180,000 ticks (so ~16 s of server startup cancels exactly):
-- 1 bot 5145 tps / 194 us per tick, 4 bots 4147 / 241, 8 bots 3242 / 308.
-- That fits `178 us + 16.3 us per bot per tick`, so at eight bots 130 of the
-- 308 us -- 42% of the tick -- was this scan. Against a 16,667 us real-time
-- budget it is **under 1% at 60 Hz**: invisible in normal play, and a ceiling
-- only for `--headless --game-speed N`, where the tick budget is whatever the
-- CPU can do.
--
-- What cost it: `get_main_inventory().get_contents()` allocates, each stack
-- builds a string, the signature is sorted and concatenated -- per bot, sixty
-- times a second -- to notice changes that happen a handful of times a minute.
--
-- **Two mechanisms, and neither is safe alone.**
--
-- `inventory_dirty` is the exact one. Most of what changes a bot's inventory
-- is something this mod *did*: an insert, a removal, a placement, a craft
-- start, a craft completing, a mining yield landing. Those set the flag at the
-- moment they happen, so the writeout is emitted on the very next tick -- no
-- latency at all, and no dependence on counts.
--
-- The stagger is the backstop, and it is why the flag does not have to be
-- complete. An inventory can change for reasons this mod did not cause, and an
-- audit of every such path is exactly the kind that is silently incomplete.
-- `tick % PERIOD == id % PERIOD` bounds the residual staleness to PERIOD ticks
-- for every bot and makes the work per tick **constant in the number of bots**
-- -- the property this project's notes record as missing ("bot count is what
-- costs tick rate").
--
-- 30 ticks is half a second at 1x. It is longer than the roster (so at most
-- one bot is scanned per tick up to 30 bots), and it is a bound on how stale
-- the *world model's* view of an inventory can be -- never on an action
-- settle: a craft settles through `poll_character_crafts` below, a walk
-- through the position check above it, and a mining yield through the miner's
-- own `inventory_before` accounting, none of which are staggered.
--
-- Rejected: gating on `get_item_count()` alone as the cheap pre-check. One
-- iron out and one copper in on the same tick preserves the total exactly, and
-- a missed inventory update is a stale world model -- correctness traded for
-- tick rate. A count check *behind* the flag would be fine; as the only signal
-- it is not.
-- ---------------------------------------------------------------------------
BOT_INVENTORY_POLL_PERIOD = 30

-- "Something changed this bot's inventory; scan it next tick." `id == nil`
-- means every bot, which is what a caller that touches several inventories at
-- once (a blueprint paid for from a list of bots) says rather than guessing.
--
-- Tolerates a `player_id` that is a connected client rather than a character
-- bot, and a `storage` that does not exist: the Rust stub interpreter loads
-- this file with neither.
function mark_bot_inventory_dirty(id)
	local bots = character_bots()
	if bots == nil then return end
	if id == nil then
		for _, bot in pairs(bots) do bot.inventory_dirty = true end
		return
	end
	local bot = bots[id]
	if bot ~= nil then bot.inventory_dirty = true end
end

-- The per-tick substitute for the four `on_player_*` events a character bot
-- never raises: position, main inventory, crafted items (queue deltas) and a
-- respawn after death. Mining completion is handled in the miner itself
-- (`inventory_before` on the mining record) because it needs the mining
-- record's accounting.
function poll_character_bots(tick)
	local bots = character_bots()
	if bots == nil then return end
	for id, bot in pairs(bots) do
		if bot.entity == nil or not bot.entity.valid then
			if bot.respawn_at ~= nil and tick >= bot.respawn_at then
				bot.entity = create_bot_character(game.surfaces[1], game.forces["player"])
				bot.respawn_at = nil
				local pos = bot.entity.position
				writeout(tick, "player_respawned", helpers.table_to_json({
					player_id = id,
					position = { x = pos.x, y = pos.y },
				}))
				announce_character_bot(tick, id)
			end
		else
			poll_character_bot(tick, id, bot_handle(id))
		end
	end
	emulate_research_triggers(tick)
end

-- How often the trigger sweep runs. A trigger technology completing a second
-- late costs nothing -- the executor is waiting on a research settle either
-- way -- and the sweep reads a statistics counter per candidate technology,
-- so it is not something to do 60 times a second.
RESEARCH_TRIGGER_PERIOD = 60

-- Complete a **trigger technology** the force has already earned.
--
-- Factorio 2.0 unlocks 32 technologies by doing rather than by researching --
-- `automation-science-pack` by crafting one lab, `electronics` by 10 copper
-- plates, `steam-power` by 50 iron plates. **The game fires those from the
-- player's own actions, and a server-side character has no player**, so on a
-- `--headless` run they never fire at all: the first acceptance run crafted a
-- lab, placed it, and still could not craft red science, because the
-- technology the lab unlocks stayed unresearched through 9 replans. That gate
-- is the whole early game.
--
-- **This is emulation, not a grant**, and the distinction is the owner's rule
-- about honest runs: the sweep completes a technology only when the force has
-- *already done the thing the trigger names*, which is exactly when a client
-- run would have been given it. It never runs ahead of the work.
--
-- The counter is the force's own **production statistics**, not a count of
-- hand-crafts, because that is what the trigger actually measures: nobody
-- hand-crafts 50 iron plates, they smelt them, and a client run reaches
-- `steam-power` that way. `get_input_count` on the item flow statistics is
-- everything the force produced by any means.
--
-- **Which kinds the game fires by itself, measured on 2026-09-05** on a
-- scratch headless server (Space Age 2.1.17, one character bot, no player;
-- `docs/superpowers/notes/2026-09-05-research-triggers.md` has the ticks):
--
-- * `mine-entity` -- the game fires it **without any player**: a character
--   bot chopping a `big-volcanic-rock` through `action_start_mining` earned
--   `tungsten-carbide`, a fuelled burner drill on calcite earned
--   `calcite-processing`, and a pumpjack on a well earned `oil-processing`.
--   Nothing here touches `mine-entity`; emulating it would fire a second time
--   at best and early at worst.
-- * `craft-item` -- split down the middle. **Machine output fires by
--   itself**: with this sweep switched off, a stone furnace's tenth copper
--   plate earned `electronics` within ~400 ticks. **A character's hand
--   craft does not**: with the sweep off, `electronics` and `steam-power`
--   researched and a lab crafted through `action_start_crafting` (in the
--   inventory, absent from the statistics), `automation-science-pack` stayed
--   open for 4,000 ticks and completed 4 ticks after the sweep came back.
--   So the emulation is *needed* for the hand-craft tally and merely
--   *redundant* for the statistics -- it reads both because the trigger does
--   not say which route a run will take, and the statistics path can only
--   ever complete a technology the game was about to complete itself.
-- * `build-entity` -- `surface.create_entity{force = player}`, which is how
--   every placement this mod makes lands, fired nothing for an
--   `asteroid-collector` with `space-platform` researched, with and without
--   `raise_built = true`. Emulated, from `storage.built_tally`, which
--   `on_some_entity_created` fills with every entity this force built.
-- * `capture-spawner` and `create-space-platform` -- no action in this mod
--   can capture a spawner or launch a platform, so there is no act to count
--   and nothing to emulate. The planner refuses them by name.
--
-- **Prerequisites gate the trigger, and the game counts the act before
-- them.** Measured twice: the rock mined with `planet-discovery-vulcanus`
-- unresearched earned nothing, and a `copper-stromatolite` mined *before*
-- `planet-discovery-gleba` was set researched earned `heating-tower` a few
-- ticks *after* it was. So the sweep skips a technology whose prerequisites
-- are open, and reads a counter that remembers the act: exactly the game's
-- behaviour, and the reason `automation-science-pack` (prerequisites
-- `electronics` and `steam-power`) can no longer complete in the same sweep
-- as, or before, the two plate triggers that unlock it.
--
-- Runs only while character bots exist. With real players the game does this
-- itself, and doing it twice would be both wrong and invisible.
-- `set_research_trigger_emulation(false)` over the remote interface switches
-- the sweep off for a measurement of what the game does on its own; the
-- switch is in `storage`, so it survives a save and is reported honestly
-- rather than lost with the Lua state.
function emulate_research_triggers(tick)
	if not has_character_bots() then return end
	if tick % RESEARCH_TRIGGER_PERIOD ~= 0 then return end
	if storage.research_trigger_emulation_off then return end
	local force = game.forces["player"]
	-- Summed over every surface, for the same reason the force sampler is:
	-- a research trigger asks what the FORCE has produced, and a plate smelted
	-- on another planet counts. Reading Nauvis alone would leave a technology
	-- unearned that the game itself considers earned -- and this sweep exists
	-- precisely because the game does not fire the trigger for us, so nothing
	-- downstream would notice the omission.
	local counts = {}
	local any = false
	for _, surface in pairs(game.surfaces) do
		local ok, s = pcall(function()
			return force.get_item_production_statistics(surface)
		end)
		if ok and s ~= nil then
			any = true
			for name, count in pairs(s.input_counts) do
				counts[name] = (counts[name] or 0) + count
			end
		end
	end
	if not any then return end
	local stats = { input_counts = counts }
	for name, tech in pairs(force.technologies) do
		if not tech.researched and tech.enabled and prerequisites_researched(tech) then
			local ok, trigger = pcall(function() return tech.prototype.research_trigger end)
			if ok and trigger ~= nil and trigger.type == "craft-item" and trigger.item ~= nil then
				local item = trigger.item
				if type(item) == "table" then item = item.name end
				local needed = trigger.count or 1
				-- `input_counts`, the table, rather than a getter: this is the
				-- shape `sample_force_body` already reads in this build, so it
				-- is known to exist here rather than assumed from the docs.
				local ok_count, produced = pcall(function()
					return stats.input_counts[item] or 0
				end)
				if not ok_count then produced = 0 end
				-- **The larger of the two counters, never their sum.**
				-- Machine production lands in the statistics; a hand craft
				-- does not, and is only in `crafted_tally`. Adding them would
				-- double-count any item that turns out to appear in both, and
				-- a trigger fired early is a technology this run did not earn
				-- -- the exact thing the owner's honest-run rule forbids.
				-- Taking the maximum can only ever fire *late*, which costs
				-- some ticks and claims nothing false.
				local tally = (storage.crafted_tally or {})[item] or 0
				if tally > produced then produced = tally end
				if produced >= needed then
					tech.researched = true
					writeout(tick, "research_trigger_emulated", helpers.table_to_json({
						technology = name,
						trigger = "craft-item",
						item = item,
						needed = needed,
						produced = produced,
					}))
					print("research trigger earned: " .. tostring(name) ..
						" (" .. tostring(item) .. " " .. tostring(produced) ..
						"/" .. tostring(needed) .. ")")
				end
			elseif ok and trigger ~= nil and trigger.type == "build-entity" then
				-- The shipped trigger (`space-science-pack`) spells its
				-- target `entity = {name = ...}`, singular, where
				-- `mine-entity` spells `entities = {...}`; `trigger_names`
				-- (types.lua) reads both, and any one of the names earns it.
				local names = trigger_names(trigger.entities)
				if #names == 0 then names = trigger_names(trigger.entity) end
				local needed = trigger.count or 1
				local built, earned_by = 0, nil
				for _, entity_name in ipairs(names) do
					local count = (storage.built_tally or {})[entity_name] or 0
					if count > built then built, earned_by = count, entity_name end
				end
				if #names > 0 and built >= needed then
					tech.researched = true
					writeout(tick, "research_trigger_emulated", helpers.table_to_json({
						technology = name,
						trigger = "build-entity",
						entity = earned_by,
						needed = needed,
						built = built,
					}))
					print("research trigger earned: " .. tostring(name) ..
						" (built " .. tostring(earned_by) .. " " .. tostring(built) ..
						"/" .. tostring(needed) .. ")")
				end
			end
		end
	end
end

-- Whether every prerequisite of `tech` is researched. A technology with no
-- prerequisites (`electronics`, `steam-power`) passes. Guarded because the
-- stub game the Rust tests load this file into does not always give a
-- technology a `prerequisites` table; an unreadable table reads as "open",
-- which can only ever delay a trigger, never grant one.
function prerequisites_researched(tech)
	local ok, met = pcall(function()
		for _, prerequisite in pairs(tech.prerequisites or {}) do
			if not prerequisite.researched then return false end
		end
		return true
	end)
	return ok and met
end

-- Switch the trigger sweep off (or on again). A measurement of what the game
-- fires on its own needs the sweep out of the way; nothing in a run calls
-- this. The state lives in `storage` so a save carries it, and the change is
-- in the record, because a run made with the sweep off and no line saying so
-- would read as a run in which the game fired every trigger itself.
function rcon_set_research_trigger_emulation(enabled)
	storage.research_trigger_emulation_off = not enabled
	writeout(game.tick, "research_trigger_emulation", helpers.table_to_json({
		enabled = enabled and true or false,
	}))
	rcon.print("research trigger emulation " .. (enabled and "on" or "off"))
end

-- What this force has built, per entity prototype, for the whole session --
-- the counter a `build-entity` trigger is read from. Fed by
-- `on_some_entity_created`, which is the one point every build this mod
-- knows about passes through: `rcon_place_entity` after the item is paid
-- for, and the game's own `on_built_entity` / `on_robot_built_entity`.
-- Only the player force's builds count: `on_biter_base_built` reaches the
-- same handler and a spawner the enemy raised is not something we built.
function tally_built_entity(ent)
	if storage == nil then return end
	storage.built_tally = storage.built_tally or {}
	local ok, ours = pcall(function()
		return ent.valid and ent.force ~= nil and ent.force.name == "player"
	end)
	if not ok or not ours then return end
	storage.built_tally[ent.name] = (storage.built_tally[ent.name] or 0) + 1
end

function poll_character_bot(tick, id, handle)
	local bot = storage.bots[id]
	if bot.entity == nil or not bot.entity.valid then return end
	local pos = bot.entity.position
	if bot.last_pos == nil or bot.last_pos.x ~= pos.x or bot.last_pos.y ~= pos.y then
		bot.last_pos = { x = pos.x, y = pos.y }
		writeout(tick, "on_player_changed_position", helpers.table_to_json({
			player_id = id,
			position = { x = pos.x, y = pos.y },
		}))
	end
	-- Dirty flag or stagger; see `BOT_INVENTORY_POLL_PERIOD`. The `%` is on the
	-- bot id so the roster's scans land on different ticks rather than all on
	-- the same one, which is what keeps the per-tick cost flat as bots are
	-- added.
	if bot.inventory_dirty
		or (tick % BOT_INVENTORY_POLL_PERIOD) == (id % BOT_INVENTORY_POLL_PERIOD) then
		bot.inventory_dirty = false
		local contents = bot.entity.get_main_inventory().get_contents()
		local sig = {}
		for _, stack in pairs(contents) do
			sig[#sig + 1] = stack.name .. ":" .. tostring(stack.count) .. ":" .. tostring(stack.quality)
		end
		table.sort(sig)
		local key = table.concat(sig, ",")
		if bot.last_inventory ~= key then
			bot.last_inventory = key
			writeout(tick, "on_player_main_inventory_changed", helpers.table_to_json({
				player_id = id,
				main_inventory = contents,
			}))
			recent_item_additions[id] = {}
		end
	end
	poll_character_crafts(tick, id, handle)
end

-- A drop in a recipe's queued count is that many finished crafts. Each one is
-- fed through `on_player_crafted_item` with the event shape the game would
-- have used, so `settle_crafted_item` and the craft waiters see one path.
-- Cancellation cannot happen without a player at the keyboard.
--
-- **This runs every tick while a craft is in flight, and must.** Batching it
-- would add its period to *every* craft settle, and a plan has hundreds of
-- crafts. What is skipped instead is the read itself when there is provably
-- nothing to read: `handle.crafting_queue` is a whole-queue scan, and a
-- character bot's queue can only become non-empty through `begin_crafting`,
-- which only `rcon_action_start_crafting` calls -- there is no player at a
-- keyboard to queue anything else. So "no craft was started and the queue was
-- empty last tick" is an exact statement that the queue is still empty, not an
-- approximation, and it costs no latency.
function poll_character_crafts(tick, id, handle)
	local bot = storage.bots[id]
	if not bot.craft_active then return end
	local now = {}
	local queue = handle.crafting_queue
	if queue ~= nil then
		for _, item in pairs(queue) do
			now[item.recipe] = (now[item.recipe] or 0) + item.count
		end
	end
	local before = bot.last_queue or {}
	for recipe_name, was in pairs(before) do
		local finished = was - (now[recipe_name] or 0)
		if finished > 0 then
			local recipe = handle.force.recipes[recipe_name]
			if recipe ~= nil then
				for _ = 1, finished do
					on_player_crafted_item({ tick = tick, player_index = id, recipe = recipe })
				end
				tally_crafted_products(recipe, finished)
			end
			-- The products landed in the bot's inventory this tick.
			mark_bot_inventory_dirty(id)
		end
	end
	bot.last_queue = now
	-- The queue has drained: nothing further can appear in it until another
	-- `begin_crafting`, which sets the flag again.
	if next(now) == nil then bot.craft_active = false end
end

-- What character bots have crafted by hand, per item, for the whole session.
--
-- **A hand craft does not appear in the force's production statistics.**
-- Measured, not assumed: at tick 55,200 of `run-1788597675-93375` the force
-- had made 202 iron plates and 71 copper plates (both smelted, both counted)
-- while three labs sat in bot inventories and `production.made.lab` was
-- absent entirely. That is why the statistics sweep alone unlocked
-- `electronics` (10 copper plates) and `steam-power` (50 iron plates) and
-- never `automation-science-pack`, whose trigger is one crafted lab.
function tally_crafted_products(recipe, times)
	storage.crafted_tally = storage.crafted_tally or {}
	local ok, products = pcall(function() return recipe.products end)
	if not ok or products == nil then return end
	for _, product in pairs(products) do
		if product.type == "item" and product.name ~= nil then
			local amount = product.amount
			if amount == nil then
				-- A probabilistic product cannot be counted as certain, and a
				-- trigger fired on a guess is a granted technology. Skip it:
				-- undercounting delays the unlock, overcounting invents it.
				amount = 0
			end
			if amount > 0 then
				storage.crafted_tally[product.name] =
					(storage.crafted_tally[product.name] or 0) + amount * times
			end
		end
	end
end

function on_character_bot_died(event, id)
	local bot = storage.bots[id]
	local cause, cause_type = nil, nil
	if event.cause ~= nil and event.cause.valid then
		cause = event.cause.name
		cause_type = event.cause.type
	end
	local position = nil
	if event.entity ~= nil and event.entity.valid then
		local pos = event.entity.position
		position = { x = pos.x, y = pos.y }
	end
	writeout(event.tick, "player_died", helpers.table_to_json({
		player_id = id,
		position = position,
		cause = cause,
		cause_type = cause_type,
		respawn_in = CHARACTER_RESPAWN_TICKS,
	}))
	local why = "ERROR: player " .. tostring(id) .. " has no character: died at tick " .. tostring(event.tick)
	if cause ~= nil then why = why .. " killed by " .. tostring(cause) end
	why = why .. ", respawns in " .. tostring(CHARACTER_RESPAWN_TICKS) .. " ticks"
	print(why)
	fail_bot_actions(id, event.tick, why)
	bot.entity = nil
	bot.respawn_at = event.tick + CHARACTER_RESPAWN_TICKS
end

function get_player(player_id)
	if storage.p[player_id] ~= nil then
		local player = bot_handle(player_id)
		if player == nil or not player.connected then
			rcon.print("Error: player " .. tostring(player_id) .. " not connected")
		elseif not player.character then
			rcon.print(no_character_error(player_id, player))
		else
			return player
		end
	else
		rcon.print("Error: player not found. valid players: " .. table_keys(storage.p))
	end
	return nil
end


remote.add_interface("botbridge", {
	test=rcon_test,
	screenshot=rcon_screenshot,
	sampling_start=rcon_sampling_start,
	sampling_stop=rcon_sampling_stop,
	savepoint=rcon_savepoint,
	session_reset=rcon_session_reset,
	whoami=rcon_whoami,
	surfaces=rcon_surfaces,

	cheat_item=rcon_cheat_item,
	cheat_technology=rcon_cheat_technology,
	cheat_all_technologies=rcon_cheat_all_technologies,
	place_blueprint=rcon_place_blueprint,
	cheat_blueprint=rcon_cheat_blueprint,
	store_map_data=rcon_store_map_data,
	retrieve_map_data=rcon_retrieve_map_data,
	players=rcon_players,
	spawn_bots=rcon_spawn_bots,
	set_game_speed=rcon_set_game_speed,
	game_speed=rcon_game_speed,
	set_tick_paused=rcon_set_tick_paused,
	player_force=rcon_player_force,
	world_snapshot=rcon_world_snapshot,
	generate_chunks=rcon_generate_chunks,
	add_research=rcon_add_research,
	player_info=rcon_player_info,
	place_entity=rcon_place_entity,
	can_place_entities=rcon_can_place_entities,
	inventory_contents_at=rcon_inventory_contents_at,
	find_entities_filtered=rcon_find_entities_filtered,
	find_tiles_filtered=rcon_find_tiles_filtered,
	insert_to_inventory=rcon_insert_to_inventory,
	remove_from_inventory=rcon_remove_from_inventory,
	set_recipe=rcon_set_recipe,
	map_exchange_string=rcon_map_exchange_string,
	parse_map_exchange_string=rcon_parse_map_exchange_string,
	revive_ghost=rcon_revive_ghost,
	async_request_player_path=rcon_async_request_player_path,
	async_request_path=rcon_async_request_path,
	action_start_walk_waypoints=rcon_action_start_walk_waypoints,
	action_start_mining=rcon_action_start_mining,
	action_start_crafting=rcon_action_start_crafting,
	action_start_research=rcon_action_start_research,
	set_research_trigger_emulation=rcon_set_research_trigger_emulation
})
