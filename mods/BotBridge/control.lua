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
	global.resources = {}
	global.resources.last_index = 0
	global.resources.list = {} -- might be sparse, so the #-operator won't work
	global.resources.map = {}
	global.map_area = {x1=0, y1=0, x2=0, y2=0} -- a bounding box of all charted map chunks
	global.p = {} -- player-private data
	global.pathfinding = {}
	global.pathfinding.map = {}
	global.n_clients = 1
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

function writeout_entity_prototypes()
	local result = {}
	for k, v in pairs(game.entity_prototypes) do
		table.insert(result, serialize_entity_prototype(v))
	end
	rcon.print(game.table_to_json(result))
end

function writeout_pictures()
	-- this is dirty. in data-final-fixes.lua, we wrote out "serpent.dump(data.raw)" into the
	-- order strings of the "DATA_RAW"..i entities. We had to use multiple of those, because
	-- there's a limit of 200 characters per string.

	if game.entity_prototypes["DATA_RAW_LEN"] == nil then
		print("Error: no DATA_RAW_LEN entity prototype?!")
	end

	local n = tonumber(game.entity_prototypes["DATA_RAW_LEN"].order)
--	print("n is " .. n)

	local i = 0
	local string = ""

	local step = math.floor(n/20)
	if step <= 0 then step = 1 end
--	print("reading data.raw")
	local strings = {}
	for i = 1,n do
		if (i % step == 0) then print(string.format("%3.0f%%", 100*i/n)) end
		table.insert(strings, game.entity_prototypes["DATA_RAW"..i].order)
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

function writeout_entity_prototypes()
	local lines = {}
	for name, prot in pairs(game.entity_prototypes) do
		if string.sub(name, 1, 8) ~= "DATA_RAW" then
			table.insert(lines, game.table_to_json(serialize_entity_prototype(prot)))
		end
	end
	writeout(0, "entity_prototypes" ,table.concat(lines, "$"))
end

function writeout_item_prototypes()
	local lines = {}
	for name, prot in pairs(game.item_prototypes) do
		table.insert(lines, game.table_to_json(serialize_item_prototype(prot)))
	end
	writeout(0, "item_prototypes", table.concat(lines,"$"))
end

function simplify_amount(prod)
	if prod.amount ~= nil then
		return prod.amount
	else
		return (prod.amount_min + prod.amount_max) / 2 * prod.probability
	end
end

function writeout_recipes()
	-- FIXME: this assumes that there is only one player force
	local lines = {}
	for name, rec in pairs(game.forces["player"].recipes) do
		if rec.enabled then
			table.insert(lines, game.table_to_json(serialize_recipe(rec)))
		end
	end
	writeout(0, "recipes", table.concat(lines,"$"))
end
function writeout_forces()
	local lines = {}
	for name, force in pairs(game.forces) do
		writeout(0, "force", game.table_to_json(serialize_force(force)))
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
		-- if global.p[idx].walking and player.connected then

--		game.print("player " .. tostring(idx) .. " connected " .. tostring(player.connected) .. " character " .. tostring(player.character))
--		game.print("player " .. tostring(global.p[idx]))

		if global.p[idx] == nil then
			global.p[idx] = {}
		end


		if player.connected and player.character then -- TODO FIXME
--			game.print("player " .. tostring(idx))
			if global.p[idx].walking then
--				game.print("player WALKING " .. table_to_string(global.p[idx]))
				local w = global.p[idx].walking
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
							global.p[idx].walking = nil
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

			if global.p[idx].mining then
				local ent = global.p[idx].mining.entity

				if ent and ent.valid then -- mining complete
					if distance(player.position, ent.position) > 6 then
						action_failed(event.tick, global.p[idx].mining.action_id, "ERROR: too far too mine")
					end

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
				else
					-- the entity to be mined has been deleted, but p[idx].mining is still true.
					-- this means that on_mined_entity() has *not* been called, indicating that something
					-- else has "stolen" what we actually wanted to mine :(
					action_failed(event.tick, global.p[idx].mining.action_id)
					complain("failed to mine " .. global.p[idx].mining.prototype.name)
					global.p[idx].mining = nil
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
		-- if global.p[idx].walking and player.connected then
		if global.p[idx] and player.connected and player.character then -- TODO FIXME
			local mining = global.p[idx].mining
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
					print("mining: " .. game.table_to_json(mining))
					mining.left = mining.left - 1
					if mining.left <= 0 then
						action_completed(event.tick, mining.action_id)
						global.p[idx].mining = nil
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

	if chunk_x < global.map_area.x1 then global.map_area.x1 = chunk_x end
	if chunk_y < global.map_area.y1 then global.map_area.y1 = chunk_y end
	if chunk_xend > global.map_area.x2 then global.map_area.x2 = chunk_xend end
	if chunk_yend > global.map_area.y2 then global.map_area.y2 = chunk_yend end

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

function writeout_tiles(tick, surface, area) -- SLOW! beastie can do ~2.8 per tick
	--if my_client_id ~= 1 then return end
	local header = area.left_top.x..","..area.left_top.y..";"..area.right_bottom.x..","..area.right_bottom.y..": "
	local tile = nil
	local line = {}
	for y = area.left_top.y, area.right_bottom.y-1 do
		for x = area.left_top.x, area.right_bottom.x-1  do
			tile = surface.get_tile(x,y)
			table.insert(line, tile.name .. ":" .. (tile.collides_with('player-layer') and "1" or "0"))
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
	writeout(tick, "entities", header .. game.table_to_json(objects))
	line=nil
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
	my_client_id = global.n_clients
end

function on_player_joined_game(event)
--	print("player '"..game.players[event.player_index].name.."' joined")
--	game.write_file("players_connected.txt", game.players[event.player_index].name..'\n', true, 0) -- only on server
	global.n_clients = global.n_clients + 1
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
	global.n_clients = global.n_clients - 1
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

	writeout(event.tick, "on_some_entity_created", game.table_to_json(serialize_entity(ent)))

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
	writeout(event.tick, "on_some_entity_updated", game.table_to_json(serialize_entity(ent)))
end

function on_some_entity_deleted(event)
	local ent = event.entity
	if ent == nil then
		complain("wtf, on_some_entity_created has nil entity")
		return
	end
	writeout(event.tick, "on_some_entity_deleted", game.table_to_json(serialize_entity(ent)))

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

function sum_inventory(ent, is)
	local sum = {}
	for _,inv_type in ipairs(is) do
		inv = ent.get_inventory(inv_type)
		for item,amount in pairs(inv.get_contents()) do
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
		result = game.table_to_json(positions)
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

		writeout(event.tick, "on_player_changed_position", game.table_to_json({
			playerId = player_idx,
			position = position
		}))
	end
end

function on_player_changed_distance(event)
	for idx, player in pairs(game.players) do
		writeout(event.tick, "on_player_changed_distance", game.table_to_json({
			playerId = idx,
			buildDistance = player.build_distance,
			reachDistance = player.reach_distance,
			dropItemDistance = player.drop_item_distance,
			itemPickupDistance = math.ceil(player.item_pickup_distance),
			lootPickupDistance = math.ceil(player.loot_pickup_distance),
			resourceReachDistance = math.ceil(player.resource_reach_distance),
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
	writeout(event.tick, "on_player_main_inventory_changed", game.table_to_json({
		playerId = player_idx,
		mainInventory = main_inventory.get_contents()
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


function rcon_action_start_walk_waypoints(action_id, player_id, waypoints) -- e.g. waypoints= { {0,0}, {3,3}, {42,1337} }
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local tmp = {}
	for i = 1, #waypoints do
		tmp[i] = {x=waypoints[i][1], y=waypoints[i][2]}
	end
	--	game.print("waypoints: " .. table_to_string(global.p[player_id]))
	global.p[player_id].walking = {idx=1, waypoints=tmp, action_id=action_id }
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
		global.p[player_id].mining = { entity = ent, action_id = action_id, prototype = ent.prototype, left = count }
	elseif name == "stop" then
--		print("MINING STOP")
		global.p[player_id].mining = nil
	else
--		print("MINING ERROR")
		rcon.print("Error: no entity to mine")
		global.p[player_id].mining = nil
		action_failed(last_tick, action_id)
	end
end

function rcon_place_entity(player_id, item_name, entity_position, direction)
	local entproto = game.item_prototypes[item_name].place_result
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

	print("player position " .. game.table_to_json(player.position))
	print("entproto.collision_box " .. game.table_to_json(entproto.collision_box))
	print("entity_position " .. game.table_to_json(entity_position))

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
		rcon.print(game.table_to_json(serialize_entity(result)))
	end
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
	rcon.print(game.table_to_json(serialize_player(player)))
end

function rcon_store_map_data(key, value)
	if global.p["map_data"] == nil then
		global.p["map_data"] = {}
	end
	global.p["map_data"][key] = value
end

function rcon_retrieve_map_data(key)
	if global.p["map_data"] == nil then
		return
	end
	if global.p["map_data"][key] == nil then
		return
	end
	rcon.print(game.table_to_json(global.p["map_data"][key]))
end

function rcon_players()
	local valid_players = {}
	for player_id, player in pairs(game.players) do
		if player.connected and player.character then
			table.insert(valid_players, serialize_player(player))
		end
	end
	rcon.print(game.table_to_json(valid_players))
end

function rcon_player_force()
	rcon.print(game.table_to_json(serialize_force(game.forces["player"])))
end

function rcon_add_research(technology_name)
	local force = game.forces["player"]
	force.add_research(technology_name)
end

function rcon_inventory_contents_at(positions)
	local surface = game.surfaces[1]

	local result = {}

	for k,v in pairs(positions) do
		local entity = surface.find_entity(v.name, v.position)
		if entity ~= nil then
			local rec = {}
			local output_inventory = entity.get_output_inventory()
			if output_inventory ~= nil then
				rec.outputInventory = output_inventory.get_contents()
			else
				rec.outputInventory = nil
			end
			local fuel_inventory = entity.get_fuel_inventory()
			if fuel_inventory ~= nil then
				rec.fuelInventory = fuel_inventory.get_contents()
			else
				rec.fuelInventory = nil
			end
			rec.name = v.name
			rec.position = v.position
			table.insert(result, rec)
		end
	end
	rcon.print(game.table_to_json(result))
end

function rcon_find_entities_filtered(filters)
	local results = game.surfaces[1].find_entities_filtered(filters)
	local lines = {}
	for k, v in pairs(results) do
		table.insert(lines, serialize_entity(v))
	end
	rcon.print(game.table_to_json(lines))
end


function rcon_find_tiles_filtered(filters)
	local results = game.surfaces[1].find_tiles_filtered(filters)
	local lines = {}
	for k, v in pairs(results) do
		table.insert(lines, serialize_tile(v))
	end
	rcon.print(game.table_to_json(lines))
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
end

function rcon_revive_ghost(player_id, name, x, y)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	local main_inventory = player.get_main_inventory()
	local contents = main_inventory.get_contents()
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
		rcon.print(game.table_to_json(serialize_entity(entity)))
	else
		local prototype = game.entity_prototypes[ghost.ghost_name]
		local bb = add_to_bounding_box(expand_rect_floor_ceil(prototype.collision_box), {x = ghost.position.x, y = ghost.position.y})
		--				print("ghost bb: " .. game.table_to_json(bb))
		if position_in_rect(player.position, bb) then
			print("Player is standing inside entity ghost, teleporting player away!")
			player.teleport({x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1})
			local success, entity = ghost.revive()
			if entity ~= nil then
				main_inventory.remove({name=name, count=1})
				rcon.print(game.table_to_json(serialize_entity(entity)))
			else
				complain("Error: failed to revive ghost")
			end
		else
			complain("Error: failed to revive ghost")
		end
	end
end

function rcon_cheat_item(player_id, item, count)
	local player = get_player(player_id)
	if player == nil then
		return
	end
	player.insert{name=item, count=count}
end

function rcon_cheat_technology(tech)
	local force = game.forces["player"]
	force.technologies[tech].researched=true
end

function rcon_cheat_all_technologies()
	local force = game.forces["player"]
	force.research_all_technologies()
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
		nothing = false
		local item = ghost.ghost_name
		local item_source_player_id
		local inventory = main_inventory.get_contents()
		if inventory[item] ~= nil and inventory[item] > 0 then
			item_source_player_id = player_id
		else
			for _, inventory_player_id in pairs(inventory_player_ids) do
				local inventory_player = get_player(inventory_player_id).get_main_inventory().get_contents()
				if inventory_player[item] ~= nil and inventory_player[item] > 0 then
					item_source_player_id = inventory_player_id
				end
			end
		end
		if only_ghosts == false and item_source_player_id ~= nil then
			local success, entity = ghost.revive()
			if entity ~= nil then
				main_inventory.remove({name=item, count=1})
				inventory = main_inventory.get_contents()
				table.insert(result, serialize_entity(entity))
			else
				local prototype = game.entity_prototypes[item]
--				print("player position: " .. game.table_to_json(player.position))
--				print("ghost position: " .. game.table_to_json(ghost.position))
--				print("ghost collision_box: " .. game.table_to_json(prototype.collision_box))
				local bb = add_to_bounding_box(expand_rect_floor_ceil(prototype.collision_box), {x = ghost.position.x, y = ghost.position.y})
--				print("ghost bb: " .. game.table_to_json(bb))
				if position_in_rect(player.position, bb) then
					print("Player is standing inside entity ghost, teleporting player away!")
					player.teleport({x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1})
					local success, entity = ghost.revive()
					if entity ~= nil then
						get_player(item_source_player_id).get_main_inventory().remove({name=item, count=1})
						table.insert(result, serialize_entity(entity))
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
	end
	if nothing == true then
		rcon.print("Error: failed to build anything")
	else
		rcon.print(game.table_to_json(result))
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
	rcon.print(game.table_to_json(result))
end

function rcon_parse_map_exchange_string(name, map_exchange_str)
	game.write_file(name, game.table_to_json(game.parse_map_exchange_string(map_exchange_str)))
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
	if global.p[player_id] ~= nil then
		local player = game.players[player_id]
		if player == nil or not player.connected or not player.character then
			rcon.print("Error: player " .. tostring(player_id) .. " not connected")
		else
			return player
		end
	else
		rcon.print("Error: player not found. valid players: " .. table_keys(global.p))
	end
	return nil
end


remote.add_interface("botbridge", {
	test=rcon_test,
	screenshot=rcon_screenshot,
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
