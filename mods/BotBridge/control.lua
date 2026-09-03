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
	storage.pathfinding = {}
	storage.pathfinding.map = {}
	storage.n_clients = 1
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

-- How long the "stuck" check should wait before deciding a leg is not
-- progressing, sized to the leg being walked rather than a flat constant.
--
-- `LuaControl.character_running_speed` is "the current movement speed of
-- this character, including effects from exoskeletons, tiles, stickers and
-- shooting" -- tiles/tick, the same unit the planner's offline
-- `WALK_TILES_PER_TICK` constant (crates/planner/src/schedule.rs) has to
-- guess at. Reading it live here means the mod never hardcodes a speed.
--
-- The margin is generous on purpose: a real walk turns corners, decelerates
-- approaching a waypoint, and can be slowed by other entities in the way --
-- none of which this straight-line estimate models. 3x the straight-line
-- time, floored at 60 ticks (one second, matching the previous flat
-- constant, for very short legs), is chosen so an ordinary walk essentially
-- never times out while a genuinely stuck bot is still bounded.
function walk_leg_timeout_ticks(player, from_pos, to_pos)
	local speed = player.character_running_speed
	if speed == nil or speed <= 0 then
		-- Should not happen for a connected player with a character (the
		-- only case this is ever called for), but a walk must never divide
		-- by zero or a negative number over a fallback that never triggers.
		speed = 0.15
	end
	local leg_length = distance(from_pos, to_pos)
	return math.max(60, math.ceil((leg_length / speed) * 3))
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

--- The search radius and precision handed to `find_non_colliding_position`
--- when placing that target. Half a tile is the coarsest step that can always
--- find the gap beside an occupied position -- a character's collision box is
--- about 0.4 tiles across -- and a few tiles is room for a handful of stacked
--- bots and no more. A radius of 0 would search forever **[V]**, so it may not
--- be zero.
PLACEMENT_STEP_ASIDE_RADIUS = 4
PLACEMENT_STEP_ASIDE_PRECISION = 0.5

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
-- All three then reached `FactorioWorld::forces`, where
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
-- Nothing read the other two. The only readers of `FactorioWorld::forces` are
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
		
	for idx, player in pairs(game.players) do
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

					if (math.abs(dx) < 0.3 and math.abs(dy) < 0.3) then
						w.idx = w.idx + 1
						w.idx_tick = event.tick
						if w.idx > #w.waypoints then
							player.walking_state = {walking=false}
							action_completed(event.tick, w.action_id)
							storage.p[idx].walking = nil
							writeout_player_position(event.tick, idx, player)
							dx = 0
							dy = 0
						else
							dest = w.waypoints[w.idx]
							dx = dest.x - pos.x
							dy = dest.y - pos.y
							-- New leg: size its own timeout instead of
							-- inheriting the one the previous, differently
							-- sized leg computed.
							w.leg_timeout = walk_leg_timeout_ticks(player, pos, dest)
						end
					end

					if math.abs(dx) > 0.3 then
						if dx < 0 then dx = -1 else dx = 1 end
					else
						dx = 0
					end

					if math.abs(dy) > 0.3 then
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
					if w.idx_tick ~= nil and event.tick - w.idx_tick > (w.leg_timeout or 60) then
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
						w.stuck = "ERROR: stuck while walking, leg " .. w.idx .. " of "
							.. #w.waypoints .. " made no progress for "
							.. (event.tick - w.idx_tick) .. " ticks from "
							.. coord(pos) .. " to " .. coord(dest)
						-- Nil the waypoint being steered at rather than advancing past
						-- it: the `dest == nil` arm above then clears `walking` and
						-- reports `w.stuck` on the next tick, which is the one exit a
						-- failed walk has. Stop steering now -- `direction` above was
						-- computed for a leg this walk is no longer walking.
						direction = ""
						player.walking_state = {walking=false}
						w.waypoints[w.idx] = nil
					end

					if direction ~= "" then
						player.walking_state = {walking=true, direction=defines.direction[direction]}
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
					if distance(player.position, ent.position) > player.resource_reach_distance then
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
		if my_client_id ~= nil and who == "?" then print("my_client_id="..my_client_id..", who="..who) end
	end

	-- periodically update the objects around the player to ensure that nothing is missed
	-- This is merely a safety net and SHOULD be unnecessary, if all other updates don't miss anything
--	if event.tick % 300 == 0 and false then -- don't do that for now, as it eats up too much cpu on the c++ part
--		for idx, player in pairs(game.players) do
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
	for idx, player in pairs(game.players) do
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

	if surface ~= game.surfaces['nauvis'] then -- TODO we only support one surface
		print("unknown surface")
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
-- A *session* is what makes this mod write anything about the world: both
-- samplers -- `sample_force` on the 300-tick beat and `sample_bots` on the
-- 60-tick one -- return early unless `storage.sampling` is set, and every
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
-- taken `samples.jsonl` -- research, production, power, bot inventories --
-- with it, silently.

local SAMPLE_FORCE_INTERVAL = 300 -- game ticks between force samples (5 s at 60 UPS)

-- Sample schema. Bumped deliberately on every field change, because
-- info.json has read 0.0.1 since the project began and cannot tell a stale
-- workspace/mods from a current one. Rust refuses a schema it does not know.
local SAMPLE_SCHEMA = 1
local SAMPLE_DIR = "botbridge"
local SAMPLE_FILE = SAMPLE_DIR .. "/samples.jsonl"
local SAMPLE_BOT_INTERVAL = 60 -- 1 s at 60 UPS

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
local function power_totals(force)
	local generated, consumed, demanded = 0.0, 0.0, 0.0
	local seen_subnetworks = {}
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
					for _, sibling in pairs(network.sub_networks) do
						seen_subnetworks[sibling.id] = true
					end
					local flow = network.flow_last_tick
					generated = generated + flow.maximum_production * 60 / 1000
					consumed = consumed + flow.total_transfer * 60 / 1000
					demanded = demanded + flow.maximum_consumption * 60 / 1000
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
	storage.telemetry_failures = storage.telemetry_failures or { bots = 0, force = 0 }
	storage.telemetry_failing = storage.telemetry_failing or { bots = false, force = false }
	storage.telemetry_failures[kind] = (storage.telemetry_failures[kind] or 0) + 1
	if not storage.telemetry_failing[kind] then
		storage.telemetry_failing[kind] = true
		writeout(tick, "sample_error", kind .. " sampler failed (#"
			.. storage.telemetry_failures[kind] .. "): " .. tostring(err))
	end
end

local function record_sample_success(kind)
	storage.telemetry_failing = storage.telemetry_failing or { bots = false, force = false }
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
	for _, player in pairs(game.connected_players) do
		local character = player.character
		bots[#bots + 1] = {
			id = player.index,
			position = player.position,
			-- `inventory_counts` handles Factorio 2.0's get_contents(),
			-- which returns an array of {name, count, quality}, not a dict.
			inventory = character and inventory_counts(
				character.get_inventory(defines.inventory.character_main)
			) or {},
			crafting_queue = player.crafting_queue_size or 0,
			mining = character_mining_name(character),
		}
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
	local stats = force.get_item_production_statistics(game.surfaces[1])
	for name, count in pairs(stats.input_counts) do made[name] = count end
	for name, count in pairs(stats.output_counts) do consumed[name] = count end

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
	sample_force(game.tick)
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
-- power, bot inventories. `sample_force` (the 300-tick beat) and `sample_bots`
-- (the 60-tick one) both return early when `storage.sampling` is nil.
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

function writeout_entities(tick, surface, area)
	--if my_client_id ~= 1 then return end
	local header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..":"
	local objects = {}
	for idx, ent in pairs(surface.find_entities(area)) do
		if ent.type ~= "character" and area.left_top.x <= ent.position.x and ent.position.x < area.right_bottom.x and area.left_top.y <= ent.position.y and ent.position.y < area.right_bottom.y then
			table.insert(objects, serialize_entity(ent))
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
	return sum_inventory(game.players[player_id], is)
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
	for idx, player in pairs(game.players) do
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
script.on_event(defines.events.on_sector_scanned, on_sector_scanned)
script.on_event(defines.events.on_chunk_generated, on_chunk_generated)
script.on_event(defines.events.on_player_mined_item, on_player_mined_item)

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
	local player = game.players[player_id]
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
	-- from the check entirely. Stamping it here, with a timeout sized to this
	-- leg's own length, covers leg 1 the same way every later leg is covered.
	local leg_timeout = 60
	if tmp[1] ~= nil then
		leg_timeout = walk_leg_timeout_ticks(player, player.character.position, tmp[1])
	end
	storage.p[player_id].walking = {
		idx = 1,
		waypoints = tmp,
		action_id = action_id,
		idx_tick = game.tick,
		leg_timeout = leg_timeout,
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

function rcon_place_entity(player_id, item_name, entity_position, direction)
	local entproto = prototypes.item[item_name].place_result
	local player = game.players[player_id]
	local surface = game.players[player_id].surface

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
		local bb = add_to_bounding_box(expand_rect_floor_ceil(entproto.collision_box), pos)
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
		if position_in_rect(player.position, bb) then
			rcon.print("§player_blocks_placement§")
		elseif character_in_footprint(surface, entproto, pos) then
			-- Ask whoever it is to move, so the next attempt has a chance of
			-- finding the ground it was always going to find. Before this, the
			-- classification was right and nothing acted on it: an idle bot in
			-- a footprint was a transient with no end. See
			-- `step_aside_from_footprint`.
			step_aside_from_footprint(surface, entproto, pos, player)
			rcon.print("cannot place item '"..item_name.."' because a character is standing in the footprint")
		else
			rcon.print("cannot place item '"..item_name.."' because surface.can_place_entity said 'no'")
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
	local result = surface.create_entity{name=entproto.name,position=entity_position,direction=direction,force=player.force, fast_replace=true, player=player, spill=true}

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

	on_some_entity_created({tick=last_tick, entity = result})
	rcon.print(helpers.table_to_json(serialize_entity(result)))
	stamp_tick()
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

-- Whether any character stands in the footprint `can_place_entity` just
-- tested.
--
-- The **raw** collision box, not the floor/ceil-expanded one
-- `rcon_place_entity` uses for its acting-player test: the expanded box
-- reaches half a tile past what the game actually judged and would pull in a
-- bot standing legitimately clear, turning a real ground refusal into a
-- transient nobody learns from. `rcon_can_place_entities` scans the same raw
-- box for the same reason.
--
-- Filtered at the game by `type`, which is safe here in a way it is not for
-- the queries that feed `EntityGraph`: this asks only whether a character is
-- present, so nothing about trees is load-bearing.
function character_in_footprint(surface, entproto, position)
	local bb = add_to_bounding_box(entproto.collision_box, position)
	return #surface.find_entities_filtered{ area = bb, type = "character" } > 0
end

-- Where to send a character that is standing inside `bb`, and why that spot.
--
-- Out through the **nearest** edge, plus the character's own half-width, plus
-- `PLACEMENT_STEP_ASIDE_MARGIN`. Nearest, because a step aside is meant to be
-- a step: crossing the whole footprint to leave by the far side is a longer
-- walk to no better place, and the run this exists for had its blocker 0.43
-- tiles from one edge and 1.37 from the other.
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
function placement_step_aside_target(bb, character)
	local pos = character.position
	local half_x, half_y = 0.2, 0.2
	local box = character.bounding_box
	if box ~= nil then
		half_x = (box.right_bottom.x - box.left_top.x) / 2.0
		half_y = (box.right_bottom.y - box.left_top.y) / 2.0
	end
	local out_west = pos.x - bb.left_top.x
	local out_east = bb.right_bottom.x - pos.x
	local out_north = pos.y - bb.left_top.y
	local out_south = bb.right_bottom.y - pos.y

	local best = out_west
	local target = { x = bb.left_top.x - half_x - PLACEMENT_STEP_ASIDE_MARGIN, y = pos.y }
	if out_east < best then
		best = out_east
		target = { x = bb.right_bottom.x + half_x + PLACEMENT_STEP_ASIDE_MARGIN, y = pos.y }
	end
	if out_north < best then
		best = out_north
		target = { x = pos.x, y = bb.left_top.y - half_y - PLACEMENT_STEP_ASIDE_MARGIN }
	end
	if out_south < best then
		target = { x = pos.x, y = bb.right_bottom.y + half_y + PLACEMENT_STEP_ASIDE_MARGIN }
	end
	return target
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
function step_aside_from_footprint(surface, entproto, position, acting_player)
	local bb = add_to_bounding_box(entproto.collision_box, position)
	for _, character in ipairs(surface.find_entities_filtered{ area = bb, type = "character" }) do
		-- `LuaEntity.player` is "the player connected to this character, if
		-- any" **[V]** (runtime-api.json, Factorio 2.1.17, api 6). Nil for a
		-- character nobody is driving, which cannot be asked to walk and must
		-- not raise here -- a raise inside an RCON handler costs the caller
		-- its whole reply.
		local blocker = character.player
		if blocker ~= nil and blocker.index ~= acting_player.index
			and blocker.connected and blocker.character ~= nil then
			local state = storage.p[blocker.index]
			if state ~= nil and state.walking == nil and state.mining == nil then
				local target = placement_step_aside_target(bb, character)
				local landing = surface.find_non_colliding_position(
					"character", target,
					PLACEMENT_STEP_ASIDE_RADIUS, PLACEMENT_STEP_ASIDE_PRECISION)
				-- Nil is the game saying the character fits nowhere near
				-- there, and a landing back inside the footprint is a walk
				-- that costs time and changes nothing. Either way, better no
				-- walk than a walk that ends in a leg timeout.
				if landing ~= nil and not position_in_rect(landing, bb) then
					start_walk_waypoints(PLACEMENT_STEP_ASIDE_ACTION_ID, blocker.index,
						{ { landing.x, landing.y } }, true)
				end
			end
		end
	end
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
-- The box queried is the raw `collision_box` shifted to the position -- the
-- box `can_place_entity` tested -- not the floor/ceil-expanded one
-- `rcon_place_entity` uses for its player-in-footprint test, which would pull
-- in neighbours that are not colliding with anything.
function rcon_can_place_entities(sites)
	local out = { tick = game.tick, sites = {} }
	for i, site in ipairs(sites) do
		local rec = { ok = false, character = false }
		local player = game.players[site.player]
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
				local bb = add_to_bounding_box(entproto.collision_box, pos)
				local seen = {}
				local blockers = {}
				for _, e in pairs(surface.find_entities_filtered{ area = bb }) do
					if e.type == "character" then
						rec.character = true
					end
					if not seen[e.name] then
						seen[e.name] = true
						blockers[#blockers + 1] = e.name
					end
				end
				table.sort(blockers)
				-- Omitted rather than sent empty: `helpers.table_to_json`
				-- renders an empty Lua table as `{}`, which is an object, and
				-- the Rust side reads this field as a list.
				if #blockers > 0 then
					rec.blockers = blockers
				end
				local tile = surface.get_tile(pos.x, pos.y)
				if tile ~= nil and tile.valid then
					rec.tile = tile.name
				end
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
	local player = game.players[player_id]
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
			complain("tried to insert "..count.."x "..items.name.." but inserted " .. real_n)
		end

		local check_n = player.remove_item({name=items.name, count=real_n})
		if check_n ~= real_n then
			complain("wtf, tried to take "..real_n.."x "..items.name.." from player #"..player_id.." but only got "..check_n..". Isn't supposed to happen?!")
		end
	end
	stamp_tick()
end

function rcon_remove_from_inventory(player_id, entity_name, entity_pos, inventory_type, items)
	local player = game.players[player_id]
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
	local player = game.players[player_id]
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
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
	for player_id, player in pairs(game.players) do
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
	local player = game.players[player_id]
	if player == nil then
		rcon.print("Error: no such player: " .. tostring(player_id))
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
		by_player = player,
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
		by_player = player,
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

function get_player(player_id)
	if storage.p[player_id] ~= nil then
		local player = game.players[player_id]
		if player == nil or not player.connected or not player.character then
			rcon.print("Error: player " .. tostring(player_id) .. " not connected")
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
	whoami=rcon_whoami,

	cheat_item=rcon_cheat_item,
	cheat_technology=rcon_cheat_technology,
	cheat_all_technologies=rcon_cheat_all_technologies,
	place_blueprint=rcon_place_blueprint,
	cheat_blueprint=rcon_cheat_blueprint,
	store_map_data=rcon_store_map_data,
	retrieve_map_data=rcon_retrieve_map_data,
	players=rcon_players,
	player_force=rcon_player_force,
	world_snapshot=rcon_world_snapshot,
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
	parse_map_exchange_string=rcon_parse_map_exchange_string,
	revive_ghost=rcon_revive_ghost,
	async_request_player_path=rcon_async_request_player_path,
	async_request_path=rcon_async_request_path,
	action_start_walk_waypoints=rcon_action_start_walk_waypoints,
	action_start_mining=rcon_action_start_mining,
	action_start_crafting=rcon_action_start_crafting,
	action_start_research=rcon_action_start_research
})
