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

local client_local_data = nil -- DO NOT USE, will cause desyncs
local last_tick = 0

local wait_for_player = false
local todo_next_tick = {}
local todo_next_tick_other = {}
local crafting_queue = {} -- array of lists. crafting_queue[character_idx] is a list
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

function on_init()
	print("on_init!")
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
function writeout_forces()
	local lines = {}
	for name, force in pairs(game.forces) do
		writeout(0, "force", helpers.table_to_json(serialize_force(force)))
	end
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
	if (client_local_data == nil) then
		client_local_data = {}
		client_local_data.whoami = nil
--		game.write_file("players_connected.txt", "server\n", true, 0) -- only on server
	end

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
					action_completed(event.tick, w.action_id)
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
							dx = 0
							dy = 0
						else
							dest = w.waypoints[w.idx]
							dx = dest.x - pos.x
							dy = dest.y - pos.y
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
					if w.idx_tick ~= nil and event.tick - w.idx_tick > 60 then
						if w.idx > #w.waypoints - 1 then -- if last waypoint just abort
							print("Player is stuck while moving to last waypoint, just stop moving")
							w.waypoints[w.idx] = nil
						else
							print("Player is stuck while moving, teleporting to next waypoint")
							player.teleport(w.waypoints[w.idx])
						end
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

					if (ent2 == nil) then
						print("wtf, not mining any target")
					elseif (ent.name ~= ent2.name or ent.position.x ~= ent2.position.x or ent.position.y ~= ent2.position.y) then
						if ent2.type == "tree" then
							print("mining: there's a tree in our way. deforesting...") -- HACK
							player.mining_state = { mining=true, position=ent.position }
						elseif ent2.name == "character" then
							print("wtf, not mining the expected target, MOVING! (expected: "..ent.name..", found: "..ent2.name..")")
							rcon_action_start_walk_waypoints(4711, idx, {{ ent.position.x - 2, ent.position.y - 2 }})
						else
							print("wtf, not mining the expected target (expected: "..ent.name..", found: "..ent2.name..")")
						end
					else
						player.mining_state = { mining=true, position=ent.position }
					end

					end
				else
					-- the entity to be mined has been deleted, but p[idx].mining is still true.
					-- this means that on_mined_entity() has *not* been called, indicating that something
					-- else has "stolen" what we actually wanted to mine :(
					action_failed(event.tick, storage.p[idx].mining.action_id)
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

function on_mined_entity(event)
	for idx, player in pairs(game.players) do
		-- if storage.p[idx].walking and player.connected then
		if storage.p[idx] and player.connected and player.character then -- TODO FIXME
			local mining = storage.p[idx].mining
			if mining then
				if mining.entity == event.entity then
--					complain("on_mined_entity mined the desired entity")
					--write_file("complete: mining "..idx.."\n")
--					complain("mined " .. mining.prototype.name)
					
					local proto = mining.prototype
					local mining_results = products_to_dict(proto.mineable_properties.products)
					local tmp_recent_item_addition = {}
					tmp_recent_item_addition.tick = event.tick
					tmp_recent_item_addition.action_id = mining.action_id
					tmp_recent_item_addition.itemlist = mining_results
					if recent_item_additions[idx] == nil then recent_item_additions[idx] = {} end
					table.insert(recent_item_additions[idx], tmp_recent_item_addition)
--					dump_dict(mining_results)
					print("mining: " .. helpers.table_to_json(mining))
					mining.left = mining.left - 1
					if mining.left <= 0 then
						action_completed(event.tick, mining.action_id)
						storage.p[idx].mining = nil
					end
				end
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
-- Tick-driven frame capture
-- ---------------------------------------------------------------------------
--
-- A frame's filename is a *measurement*, not a claim. The tick in it is read
-- from `game.tick` inside the game, at the moment the capture is requested --
-- the same source `stamp_tick` uses to tell an RCON caller when the game
-- actually saw their command. There is deliberately no second notion of "now"
-- here.
--
-- Why the cadence lives in the mod rather than in a caller's loop: an RCON
-- command arrives whenever it arrives. The sender does not know the tick and
-- cannot make one true by writing it into a filename. A loop that fires "every
-- five seconds" and names its frames 300 apart produces a contiguous,
-- plausible, authoritative-looking sequence whether or not the game agreed --
-- it *cannot fail* to look right.
--
-- That is not tidiness, because frames drop. `force_render` is asked for
-- below, but the API does not honour it on a multiplayer client that is
-- catching up to the server, which is exactly what our bots are. A name built
-- from `game.tick` turns a drop into a visible gap in the sequence; a name
-- built from a counter hides it, and the stream then misrepresents itself
-- precisely when something has gone wrong. So nothing in here renumbers,
-- backfills, interpolates or retries a missed frame: the gap is the record.
--
-- Off unless asked for: `storage.frame_capture` is nil until
-- `rcon_frame_capture_start` sets it, because the frames are big and there is
-- one set of them per camera. Measured: 0.72 MB per frame at JPEG quality 85
-- and 1920x1080, and 300 ticks is 12 frames a minute, so one camera costs
-- ~520 MB an hour and the three cameras of a one-bot run cost ~1.56 GB an
-- hour. The camera count is `2 + one per player the game knows of` (see
-- `rcon_frame_capture_start`), so each additional bot adds another ~520 MB an
-- hour. Wiped per run, never accumulated across runs.

local FRAME_CAPTURE_INTERVAL = 300 -- game ticks between frames (5 s at 60 UPS)
local FRAME_CAPTURE_DIR = "frames"
local FRAME_CAPTURE_RESOLUTION = {1920, 1080}
local FRAME_CAPTURE_QUALITY = 85 -- percent; JPEG only. PNG measured ~7x larger.
local FRAME_CAPTURE_ZOOM = 1

-- Tiles of empty ground kept between the outermost bot and the edge of an
-- `area` frame, so a bot that defines the bounding box is not sliced in half
-- by the frame it defines.
local FRAME_CAPTURE_AREA_MARGIN = 16
-- The zoom below which the `area` camera stops writing a frame rather than
-- start cropping. At 1920x1080 and 32 px per tile, 0.05 covers 1200x675 tiles
-- -- bots spread wider than that get no area frame for that tick, which is
-- the honest outcome; see `frame_capture_take_area`.
local FRAME_CAPTURE_AREA_MIN_ZOOM = 0.05

-- The run id sidecar lives *inside* `frames/`, not one level up in
-- script-output, and that placement is the whole point of it.
--
-- The id exists so a consumer can tell whether the frames it is looking at and
-- the replay document it is looking at came from the same run. Frames are
-- per-run because `rcon_frame_capture_start` wipes this directory; a replay is
-- per-job and any past job's replay can be opened. Without a shared id, job
-- 3's plan joins to job 7's pictures by tick number alone and the join
-- succeeds -- both sides name ticks from `game.tick`, so nothing complains.
--
-- Put the sidecar one level up and the wipe no longer reaches it. Then a run
-- that cleared the frames and failed before rewriting the id would leave an id
-- describing frames that no longer exist, and a consumer checking it would
-- confirm a match that is wrong. A stale id is worse than no id: it turns "I
-- cannot tell" into "I checked, they match". Inside the directory, the id can
-- only ever be as old as the frames beside it.
local FRAME_CAPTURE_RUN_FILE = FRAME_CAPTURE_DIR .. "/run.json"

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
-- peer, which is why each client writes its own frames -- but force statistics
-- are identical on every peer and bot inventories are readable from any of
-- them, so four copies would be four identical files to reconcile for nothing.
local function write_sample(line)
	helpers.write_file(SAMPLE_FILE, helpers.table_to_json(line) .. "\n", true, 0)
end

-- Camera ids must be filename-safe. Interior hyphens are allowed; leading,
-- trailing and doubled ones are not.
--
-- `tick-NNNNNNN-<camera>.jpg` is parsed by taking the digit run after `tick-`
-- and reading *everything after the next hyphen* as the camera id, so
-- `tick-0001800-bot-1.jpg` reads back as tick 1800, camera `bot-1`: an id may
-- contain hyphens without becoming ambiguous. What would be ambiguous is an id
-- that begins or ends with one, or contains an empty component, because the
-- name then no longer says which characters were the separator -- so those are
-- refused here rather than assumed absent. This is checked, not trusted,
-- because a mis-parsed name attributes a frame to a camera that did not take
-- it, and nothing downstream could notice.
function frame_capture_valid_camera_id(id)
	if type(id) ~= "string" then
		return false
	end
	if id:match("^[%w_%-]+$") == nil then
		return false
	end
	if id:sub(1, 1) == "-" or id:sub(-1) == "-" or id:find("%-%-") ~= nil then
		return false
	end
	return true
end

-- Flat, one directory for every camera: a scrubber sitting at tick T wants
-- every camera's frame at T, and a shared `tick-NNNNNNN-` prefix gives it that
-- in one listing, where a directory per camera would not.
--
-- Seven digits covers ~46 hours of game time. Past that `%07d` widens rather
-- than truncates: the name stays true and only lexical sort order suffers.
function frame_capture_path(tick, camera_id)
	return FRAME_CAPTURE_DIR .. "/tick-" .. string.format("%07d", tick) .. "-" .. camera_id .. ".jpg"
end

-- Every connected player, ordered by player index.
--
-- Ordered rather than however `pairs` happens to walk the table, because the
-- first entry decides which peer writes an `area` frame and which player's
-- vision it is rendered through. An unordered pick would make that vary
-- between ticks and between peers for no reason a reader of the frames could
-- see.
function frame_capture_connected_players()
	local indexes = {}
	for _, player in pairs(game.players) do
		if player.connected then
			table.insert(indexes, player.index)
		end
	end
	table.sort(indexes)
	local players = {}
	for _, index in ipairs(indexes) do
		table.insert(players, game.players[index])
	end
	return players
end

-- A camera that follows one player. `follow` and every `bot-N` are this.
function frame_capture_take_follow(camera, tick)
	local player = game.players[camera.player_index]
	-- No player to follow, so no frame -- and the absence is the record.
	-- This tick simply has no file, exactly as a dropped render has none.
	-- Nothing is substituted, deferred to the next tick, or written under
	-- a tick the game did not agree to.
	--
	-- This is the whole of what a per-bot camera does for a bot that is not
	-- connected, and it has to stay this: a placeholder frame -- black, or the
	-- previous one, or another camera's -- would be indistinguishable from a
	-- capture of that bot, and would put a picture of nothing beside a step
	-- that really happened.
	if player == nil or not player.connected then
		return
	end
	game.take_screenshot({
		player = player,
		-- One peer, not all of them. `on_nth_tick` runs on every peer in
		-- a multiplayer game, so without `by_player` each connected
		-- client would render and write its own copy of the same frame
		-- into its own script-output. Taking a screenshot reads game
		-- state and writes none, so the duplication is a waste rather
		-- than a desync -- but a camera must map to exactly one file for
		-- its name to mean anything. `by_player` is also what lets the
		-- per-bot cameras exist: each writes on the peer it photographs.
		by_player = player,
		surface = player.surface,
		position = player.position,
		resolution = FRAME_CAPTURE_RESOLUTION,
		zoom = FRAME_CAPTURE_ZOOM,
		path = frame_capture_path(tick, camera.id),
		quality = FRAME_CAPTURE_QUALITY,
		-- Asked for, not relied on: the API does not honour this on a
		-- multiplayer client catching up. See the header comment.
		force_render = true,
		show_entity_info = true,
		show_gui = false
	})
end

-- The `area` camera frames **the bounding box of all connected bots**: the
-- axis-aligned box through every connected player's position, widened by
-- `FRAME_CAPTURE_AREA_MARGIN`, centred on the box's centre, zoomed to fit.
--
-- That claim was chosen because it is self-describing and checkable against
-- the picture: "every bot that was connected at this tick is inside this
-- frame". It is deliberately not "where the action is" -- nothing in this mod
-- defines action, so such a camera would be pointing at a thing it had
-- invented, and a viewer could never tell whether it had found it.
--
-- The degenerate cases, which is where a framing claim usually turns into a
-- lie:
--
--   * **No connected bot: no file for this tick.** There is no bounding box of
--     nothing. A frame of the map origin would be a picture of nobody, filed
--     under a camera that says it shows everybody.
--   * **One connected bot: this is the `follow` camera with extra steps, and
--     it is written anyway.** The box is a point, the margin makes it 32 tiles
--     across, and the fit zoom clamps to `FRAME_CAPTURE_ZOOM` -- so the output
--     is `follow` pointed at that bot. Said plainly here rather than dressed
--     up as something else. It is still written because the claim it makes is
--     still true of it, and because a camera that vanished at one bot would
--     make its own absence mean two different things.
--   * **Bots far apart: it zooms out, and legibility is what gives way.** A
--     frame where the bots are specks still answers "where is everyone"
--     truthfully; a zoom held at a readable level would silently *crop* bots
--     out and hand back a picture that looks like the whole party. Cropping is
--     the one thing this must not do, so below `FRAME_CAPTURE_AREA_MIN_ZOOM`
--     it writes nothing instead. A consumer can tell that gap from a stopped
--     capture: the follow and per-bot frames for the same tick are there.
--   * **Bots on more than one surface: no file.** One image cannot contain two
--     surfaces, and framing one of them would show a subset under a name that
--     claims the set.
function frame_capture_take_area(camera, tick)
	local players = frame_capture_connected_players()
	if #players == 0 then
		return
	end
	-- Lowest connected index: the peer that writes the file and the vision the
	-- world is rendered through. Any single choice does, as long as it is the
	-- same one every tick.
	local anchor = players[1]
	local surface = anchor.surface
	local min_x, min_y = anchor.position.x, anchor.position.y
	local max_x, max_y = min_x, min_y
	for _, player in ipairs(players) do
		if player.surface.index ~= surface.index then
			return
		end
		local position = player.position
		if position.x < min_x then min_x = position.x end
		if position.x > max_x then max_x = position.x end
		if position.y < min_y then min_y = position.y end
		if position.y > max_y then max_y = position.y end
	end
	-- 32 pixels to a tile at zoom 1, so this is the zoom at which the box plus
	-- its margin exactly fills the frame. Never zoom *in* past
	-- FRAME_CAPTURE_ZOOM: a closer view of two bots standing together would
	-- not be more true, and it would make the scale jump around.
	local width = (max_x - min_x) + 2 * FRAME_CAPTURE_AREA_MARGIN
	local height = (max_y - min_y) + 2 * FRAME_CAPTURE_AREA_MARGIN
	local zoom = math.min(
		FRAME_CAPTURE_RESOLUTION[1] / (32 * width),
		FRAME_CAPTURE_RESOLUTION[2] / (32 * height),
		FRAME_CAPTURE_ZOOM)
	if zoom < FRAME_CAPTURE_AREA_MIN_ZOOM then
		return
	end
	game.take_screenshot({
		player = anchor,
		by_player = anchor,
		surface = surface,
		position = { x = (min_x + max_x) / 2, y = (min_y + max_y) / 2 },
		resolution = FRAME_CAPTURE_RESOLUTION,
		zoom = zoom,
		path = frame_capture_path(tick, camera.id),
		quality = FRAME_CAPTURE_QUALITY,
		force_render = true,
		show_entity_info = true,
		show_gui = false
	})
end

function frame_capture_take(camera, tick)
	if camera.kind == "follow" then
		frame_capture_take_follow(camera, tick)
	elseif camera.kind == "area" then
		frame_capture_take_area(camera, tick)
	else
		-- Unreachable from `rcon_frame_capture_start`, which registers only
		-- the kinds above. Raising rather than returning keeps a future camera
		-- kind from producing a silently empty run.
		error("unknown frame capture camera kind: " .. tostring(camera.kind))
	end
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
-- Unverified against a live game -- this task is static-only by design (see
-- the task brief) -- but read directly from `runtime-api.json`, not guessed.
local function power_totals(force)
	local generated, consumed, demanded = 0.0, 0.0, 0.0
	local seen_networks = {}
	for _, surface in pairs(game.surfaces) do
		for _, pole in pairs(surface.find_entities_filtered({
			type = "electric-pole", force = force,
		})) do
			-- A network has many poles; `electric_network_id` dedups so a
			-- network with N of the force's poles is not counted N times.
			local network_id = pole.electric_network_id
			if network_id and not seen_networks[network_id] then
				seen_networks[network_id] = true
				local network = pole.electric_network
				if network and network.valid then
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

-- Bot inventories and positions, on a 1 s beat -- fast enough to see a bot
-- move or mine, slow enough not to compete with the 300-tick frame cadence.
--
-- Gated on an active capture run (F5): a run started without one produces no
-- samples at all, matching frame capture's own all-or-nothing behaviour, and
-- this avoids writing a stream nobody asked to correlate with anything.
local function sample_bots(tick)
	local capture = storage.frame_capture
	if capture == nil then
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
			mining = character and character.mining_state.mining
				and character.mining_target and character.mining_target.name or nil,
		}
	end
	write_sample({
		kind = "bots",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		-- F2: every line carries the run id (nil when the run was started
		-- untagged), so Rust can filter on it instead of on tick range alone.
		run = capture.run,
		bots = bots,
	})
end

-- The only registration site for the bot-sample cadence. A distinct tick (60,
-- not 300) on purpose: `script.on_nth_tick(n, f)` replaces the handler
-- already registered for `n`, and frame capture owns 300 (see the comment
-- below), so a second registration there would silently disable it instead of
-- adding to it.
script.on_nth_tick(SAMPLE_BOT_INTERVAL, function(event)
	sample_bots(event.tick)
end)

-- Force-wide research, production and power. Folded into the existing
-- 300-tick frame handler (`on_frame_capture_tick` below) rather than given
-- its own registration, for the same reason `sample_bots` above got tick 60
-- instead of 300: a second `on_nth_tick(300, ...)` would replace frame
-- capture's handler, not add to it.
local function sample_force(tick)
	local capture = storage.frame_capture
	if capture == nil then
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
		run = capture.run,
		research = research,
		techs_unlocked = unlocked,
		production = { made = made, consumed = consumed },
		power = power_totals(force),
	})
end

-- Registered with `script.on_nth_tick` rather than as a modulus inside
-- `on_tick`: `on_tick` already runs real per-tick work for every client, and a
-- counter or a remainder in there would both add to that and reintroduce the
-- caller-side notion of cadence this exists to remove.
--
-- Multiplayer: this handler runs on every peer with the same replicated
-- `storage.frame_capture`, so every peer agrees on whether to capture. Which
-- peer actually writes the file is settled by `by_player` above. The gate is
-- deliberately in `storage` and not in `client_local_data`, which the top of
-- this file marks as desync-causing.
--
-- One client cannot capture the same tick twice, which matters because the
-- filename carries the tick and the camera but not the writer -- so a second
-- write to the same client's directory would overwrite the first with nothing
-- left to show it happened. Four things make it impossible rather than merely
-- unobserved:
--
--   * `script.on_nth_tick(n, f)` *replaces* the handler registered for `n`
--     rather than appending to it, and there is exactly one registration site
--     below. Re-running this file's top level on every load therefore cannot
--     stack handlers, however many times a client leaves and rejoins.
--   * A peer runs one Lua state. There is no separate "server-side context"
--     inside a client that could run the handler a second time, and however
--     many peers do run it, `by_player` narrows the write to one machine.
--   * The nth-tick event fires once for a tick, and a tick never recurs while
--     the game runs forward.
--   * Camera ids are checked unique at start, so one tick cannot yield two
--     frames with one name.
--
-- The one way a tick could be re-simulated is playback, and
-- `take_screenshot` does not run during replay: `allow_in_replay` is left at
-- its default of false. Loading a save from before the current tick starts a
-- new run, whose `rcon_frame_capture_start` wipes the directory.
function on_frame_capture_tick(event)
	local capture = storage.frame_capture
	if capture == nil then
		return
	end
	-- `game.tick` rather than `event.tick`: they are the same value here, and
	-- reading the one `stamp_tick` reads keeps a single source of "now".
	local tick = game.tick
	for _, camera in ipairs(capture.cameras) do
		frame_capture_take(camera, tick)
	end
	sample_force(tick)
end

-- The camera that follows one bot. A per-bot camera has no `kind` of its own
-- because it has no behaviour of its own: it *is* a follow camera pointed at
-- player N, and a second name for one mechanism would only invite the two to
-- drift apart.
function frame_capture_bot_camera(player_index)
	return { id = "bot-" .. player_index, kind = "follow", player_index = player_index }
end

-- Two cameras sharing an id would write the same `tick-NNNNNNN-<id>.jpg` in
-- the same tick, and the second write would silently overwrite the first -- a
-- frame disappearing with nothing on disk to say it did. The fix is to refuse
-- the configuration, never to uniquify the filename: a name has to stay a
-- measurement of *when*, and a `-2` suffix would give the double-write a home
-- instead of preventing it.
function frame_capture_validate_cameras(cameras)
	local seen = {}
	for _, camera in ipairs(cameras) do
		if not frame_capture_valid_camera_id(camera.id) then
			error("frame capture camera id is not filename-safe: " .. tostring(camera.id))
		end
		if seen[camera.id] then
			error("duplicate frame capture camera id: " .. camera.id)
		end
		seen[camera.id] = true
	end
end

-- Gives a bot that joins *during* a run its own camera from the tick it
-- arrived, and does nothing at all when no capture is running.
--
-- Without this a late joiner would have no camera for the whole run, and its
-- total absence from the frames would be indistinguishable from a bot that was
-- there and never photographed. With it, the frames say what actually
-- happened: nothing before it joined, because it was not there, and frames
-- from the tick it was.
--
-- Deterministic across peers: `on_player_joined_game` fires on every peer with
-- the same event, and `storage` is replicated, so every peer appends the same
-- camera at the same tick. Nothing is removed on leave -- the camera staying
-- and writing nothing is exactly how a disconnected bot's absence is recorded.
function frame_capture_on_player_joined(player_index)
	local capture = storage.frame_capture
	if capture == nil then
		return
	end
	local camera = frame_capture_bot_camera(player_index)
	for _, existing in ipairs(capture.cameras) do
		if existing.id == camera.id then
			return
		end
	end
	frame_capture_validate_cameras({ camera })
	table.insert(capture.cameras, camera)
end

-- `run_id` is an opaque tag for this capture run, echoed verbatim into
-- `frames/run.json` as `{"run":"<run_id>"}` and used for nothing else here.
--
-- Deliberately uninterpreted. The caller passes a job id, but this mod must
-- never learn that: it does not parse it, validate its shape, derive a
-- filename from it or compare it to anything. Both sides then hold the same
-- identifier while only the caller knows what it identifies. That ignorance is
-- the design -- a mod that understood the id would have to be changed every
-- time the caller's notion of a run changed.
--
-- Omitting it is a real choice, not a degraded one: a capture that nobody
-- needs to correlate simply has no sidecar, and a consumer that finds none
-- knows it cannot tell rather than being told something false.
function rcon_frame_capture_start(run_id)
	-- Checked before the wipe, so a call this function is going to refuse
	-- cannot first destroy the previous run's frames. The check is on the
	-- *type* only -- reading the value would be interpreting it.
	if run_id ~= nil and type(run_id) ~= "string" then
		error("frame capture run id must be a string or absent, got " .. type(run_id))
	end
	-- Wipe first, so the directory holds this run's frames and only this
	-- run's. Without it a leftover frame from an earlier run that landed on
	-- the same tick would fill a gap this run really had, which is the one
	-- failure mode the naming scheme exists to expose. Runs on every peer,
	-- each clearing its own script-output.
	--
	-- This takes `run.json` with it, and must: the wipe and the sidecar have
	-- to move together or the id can outlive the frames it names.
	helpers.remove_path(FRAME_CAPTURE_DIR)
	-- Samples get the same fresh start as frames (F5): server-only, since
	-- only the server's copy exists to begin with, and `append = false`
	-- truncates the file rather than appending to whatever a previous run
	-- left in it.
	helpers.write_file(SAMPLE_FILE, "", false, 0)
	-- Three vantage points, `2 + one per bot` cameras in total:
	--
	--   `follow`  one bot, player 1, the original camera and unchanged.
	--   `bot-N`   one per player the game knows of, following that player.
	--   `area`    all connected bots at once; see `frame_capture_take_area`
	--             for what it centres on and what it does when it cannot.
	--
	-- Costed before it is added to, not after: see the header comment for the
	-- measured per-frame size and what one more camera costs per hour.
	local cameras = {
		{ id = "follow", kind = "follow", player_index = 1 }
	}
	-- Every player the game knows, connected right now or not, and that is the
	-- point rather than an oversight. A camera whose player is absent writes
	-- no file for that tick (`frame_capture_take_follow`), so a bot that is
	-- offline for part of a run has frames either side of the gap and nothing
	-- in it, which is where the bot actually was. Registering only the
	-- connected ones would instead delete the camera and leave a viewer unable
	-- to tell "this bot was away" from "nobody ever pointed a camera at it".
	--
	-- `bot-<player_index>`, with the hyphen: a frame name is parsed by taking
	-- everything after the tick's separator, so `tick-0001800-bot-1.jpg` reads
	-- back as camera `bot-1` intact.
	local player_indexes = {}
	for _, player in pairs(game.players) do
		table.insert(player_indexes, player.index)
	end
	table.sort(player_indexes)
	for _, index in ipairs(player_indexes) do
		table.insert(cameras, frame_capture_bot_camera(index))
	end
	-- Last, so the per-bot cameras of a tick are written before the frame that
	-- claims to contain all of them. Nothing depends on the order; it just
	-- reads better in a directory listing.
	table.insert(cameras, { id = "area", kind = "area" })
	frame_capture_validate_cameras(cameras)
	-- `run` is set only from the argument -- nil when the caller passed none,
	-- exactly like `run.json` below. No fallback, no `run_id or
	-- storage.something`: an untagged run's samples carry `"run":null` and
	-- Rust falls back to tick-range filtering for them instead.
	storage.frame_capture = { cameras = cameras, run = run_id }
	-- After the wipe, and only when asked for. The ordering is what keeps the
	-- id honest: the directory is emptied first and the sidecar written
	-- second, so `run.json` is always newer than the wipe that preceded it.
	--
	-- A start with no id therefore leaves no `run.json` at all -- the previous
	-- run's went out with the wipe and nothing replaced it. An untagged run
	-- inheriting the last run's identity would be the worst outcome available
	-- here, and the only way to prevent it is to have no branch that writes
	-- the file with a remembered value: there is no fallback, no
	-- `run_id or storage.something`, nothing carried across.
	--
	-- Written on every peer, matching the wipe above: each peer clears its own
	-- script-output, so each peer's `frames/` gets its own sidecar.
	if run_id ~= nil then
		helpers.write_file(FRAME_CAPTURE_RUN_FILE, helpers.table_to_json({ run = run_id }), false)
	end
	stamp_tick()
end

function rcon_frame_capture_stop()
	storage.frame_capture = nil
	stamp_tick()
end

-- Samples bots on demand, called by the executor immediately after an action
-- settles with a non-success status.
--
-- Only on failure: the 60-tick beat already carries what a bot held when
-- nothing went wrong, and sampling every settle would roughly double the
-- stream for that. The tick that matters for diagnosis is the tick something
-- failed, and by the next beat the bot has moved or handed off.
--
-- Gated the same as the beat itself, via `sample_bots` (F5): a settle failure
-- outside an active capture run writes nothing.
function rcon_sample_bots()
	sample_bots(game.tick)
end

function writeout_tiles(tick, surface, area) -- SLOW! beastie can do ~2.8 per tick
	--if my_client_id ~= 1 then return end
	local header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..": "
	local tile = nil
	local line = {}
	for y = area.left_top.y, area.right_bottom.y-1 do
		for x = area.left_top.x, area.right_bottom.x-1  do
			tile = surface.get_tile(x,y)
			-- TODO: Factorio 2.0 changed collision layer API, need to update
			-- For now, assume tiles don't collide with player (walkable)
			table.insert(line, tile.name .. ":0")
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

function on_player_joined_game(event)
--	print("player '"..game.players[event.player_index].name.."' joined")
--	game.write_file("players_connected.txt", game.players[event.player_index].name..'\n', true, 0) -- only on server
	storage.n_clients = storage.n_clients + 1
	wait_for_player_inventory(event)
	frame_capture_on_player_joined(event.player_index)

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

function action_failed(tick, action_id, reason)
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

function on_player_crafted_item(event)
	queue = crafting_queue[event.player_index]

	local tmp_recent_item_addition = {}
	tmp_recent_item_addition.tick = event.tick
	tmp_recent_item_addition.recipe = event.recipe
	tmp_recent_item_addition.itemlist = products_to_dict(event.recipe.products)

	if queue == nil then
--		complain("player "..game.players[event.player_index].name.." unexpectedly crafted "..event.recipe.name)
	else
		if queue[1].recipe == event.recipe.name then
			if queue[1].id == nil then
--				complain("player "..game.players[event.player_index].name.." has crafted "..queue[1].recipe..", but that's not all")
			else
--				complain("player "..game.players[event.player_index].name.." has finished crafting "..queue[1].recipe.." with id "..queue[1].id)
				action_completed(event.tick, queue[1].id)
				tmp_recent_item_addition.action_id = queue[1].id
			end
			table.remove(queue,1)
			if #queue == 0 then
--				complain("done crafting")
				crafting_queue[event.player_index] = nil
			end
		else
--			complain("player "..game.players[event.player_index].name.." crafted "..event.recipe.name.." which is probably an intermediate product")
		end
	end

	if recent_item_additions[event.player_index] == nil then recent_item_additions[event.player_index] = {} end
	table.insert(recent_item_additions[event.player_index], tmp_recent_item_addition)
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
	local result = "Error: failed to path find"
	if event.path ~= nil then
		local positions = {}
		for k,v in pairs(event.path) do
			table.insert(positions, v.position)
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

-- The only registration site for the frame cadence. `on_nth_tick` replaces
-- the handler for a given period rather than adding to it, so re-running this
-- file on a load cannot end up with two handlers writing one tick twice.
script.on_nth_tick(FRAME_CAPTURE_INTERVAL, on_frame_capture_tick)


function rcon_action_start_walk_waypoints(action_id, player_id, waypoints) -- e.g. waypoints= { {0,0}, {3,3}, {42,1337} }
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local tmp = {}
	for i = 1, #waypoints do
		tmp[i] = {x=waypoints[i][1], y=waypoints[i][2]}
	end
	--	game.print("waypoints: " .. table_to_string(storage.p[player_id]))
	storage.p[player_id].walking = {idx=1, waypoints=tmp, action_id=action_id }
	stamp_tick()
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

	if entproto == nil then
		complain("cannot place item '"..item_name.."' because place_result is nil")
		return
	end

	if player.get_item_count(item_name) <= 0 then
		complain("cannot place item '"..item_name.."' because the player '"..player.name.."' does not have any")
		return
	end

	print("player position " .. helpers.table_to_json(player.position))
	print("entproto.collision_box " .. helpers.table_to_json(entproto.collision_box))
	print("entity_position " .. helpers.table_to_json(entity_position))

	if not surface.can_place_entity{name=entproto.name, position=entity_position, direction=direction, force=player.force, build_check_type=defines.build_check_type.manual} then
		local bb = add_to_bounding_box(expand_rect_floor_ceil(entproto.collision_box), {x = entity_position[1], y = entity_position[2]})
		if position_in_rect(player.position, bb) then
			rcon.print("§player_blocks_placement§")
		else
			rcon.print("cannot place item '"..item_name.."' because surface.can_place_entity said 'no'")
		end
		return
	end

	player.remove_item({name=item_name,count=1})
	result = surface.create_entity{name=entproto.name,position=entity_position,direction=direction,force=player.force, fast_replace=true, player=player, spill=true}

	if result == nil then
		complain("placing item '"..item_name.."' failed, surface.create_entity returned nil :(")
	else
		on_some_entity_created({tick=last_tick, entity = result})
		rcon.print(helpers.table_to_json(serialize_entity(result)))
	end
	stamp_tick()
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

-- Queue a technology for research, and say so when the game will not.
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
function rcon_add_research(technology_name)
	local force = game.forces["player"]
	if force.technologies[technology_name] == nil then
		rcon.print("Error: no such technology: " .. tostring(technology_name))
		return
	end
	if not force.add_research(technology_name) then
		-- Say which of the several reasons it was. "Refused" alone sends the
		-- caller guessing, and the guesses are all plausible.
		local tech = force.technologies[technology_name]
		local unmet = {}
		for name, prereq in pairs(tech.prerequisites) do
			if not prereq.researched then
				table.insert(unmet, name)
			end
		end
		rcon.print("Error: cannot research " .. tostring(technology_name) ..
			": researched=" .. tostring(tech.researched) ..
			" enabled=" .. tostring(tech.enabled) ..
			" trigger=" .. tostring(tech.prototype.research_trigger ~= nil) ..
			" unmet_prerequisites=[" .. table.concat(unmet, ",") .. "]")
		return
	end
	stamp_tick()
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


function rcon_action_start_crafting(action_id, player_id, recipe, count)
	local player = game.players[player_id]
	local ret = player.begin_crafting{count=count, recipe=recipe}
	if ret ~= count then
		complain("could not have player "..player.name.." craft "..count.." "..recipe.." (but only "..ret..")")
	end

	for i = 1,count do
		local aid = nil
		if i == count then aid = action_id end
		if crafting_queue[player_id] == nil then crafting_queue[player_id] = {} end
		table.insert(crafting_queue[player_id], {recipe=recipe, id=aid})
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
			print("Player is standing inside entity ghost, teleporting player away!")
			player.teleport({x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1})
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
		-- force_build :: boolean (optional): When true, anything that can be built is else nothing is built if any one thing can't be built
		force_build = force_build
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
					print("Player is standing inside entity ghost, teleporting player away!")
					player.teleport({x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1})
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
		-- force_build :: boolean (optional): When true, anything that can be built is else nothing is built if any one thing can't be built
		force_build = force_build
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

function rcon_async_request_player_path(player_id, goal, radius)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local handle = player.surface.request_path({
		bounding_box = player.character.prototype.collision_box,
		collision_mask = player.character.prototype.collision_mask,
		start = player.position,
		goal = goal,
		force = player.force,
		radius = radius,
		pathfind_flags = {
			allow_destroy_friendly_entities = false,
			prefer_straight_paths = true,
		},
		entity_to_ignore = player.character,
	})
	rcon.print(handle)
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
	frame_capture_start=rcon_frame_capture_start,
	frame_capture_stop=rcon_frame_capture_stop,
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
	inventory_contents_at=rcon_inventory_contents_at,
	find_entities_filtered=rcon_find_entities_filtered,
	find_tiles_filtered=rcon_find_tiles_filtered,
	insert_to_inventory=rcon_insert_to_inventory,
	remove_from_inventory=rcon_remove_from_inventory,
	parse_map_exchange_string=rcon_parse_map_exchange_string,
	revive_ghost=rcon_revive_ghost,
	async_request_player_path=rcon_async_request_player_path,
	async_request_path=rcon_async_request_path,
	action_start_walk_waypoints=rcon_action_start_walk_waypoints,
	action_start_mining=rcon_action_start_mining,
	action_start_crafting=rcon_action_start_crafting
})
