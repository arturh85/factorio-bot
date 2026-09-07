-- ELECTRIC SMELTING, WITH A REAL OUTPUT SIDE, POWERED BY THE PLANNER.
--
-- No drills. A block with drills must stand on ore; a block with its own power
-- plant must stand near water; on seed 31337 those are far apart (iron 18.4
-- from spawn, water 48.1) and no anchor satisfies both. The drills were never
-- the part under test, so they are gone and ore arrives in a chest.
--
-- That leaves the thing that IS new: electric inserters, and plates leaving the
-- furnaces onto a belt. Every burner block here dead-ends at the furnace,
-- because an arm carrying plates has nothing to fuel itself with -- which is
-- why the 200-plate ceiling existed and why the last rate had to be measured
-- with a script-side drain standing in for an output side.
--
-- Sited NEAR THE WATER on purpose. A previous run had the planner build a
-- perfectly good plant 48 tiles from the block, out of pole reach, and the
-- block sat dead with 63 of 63 entities standing.
print("start electric smelt row")

local BP = "0eNqd10tugzAQBuC7eA0RfmHDvhdol1VVkdRVkcBExulDEXevW0UtUiDMzC4y4QsT+x/sM9t3J3cMrY+sPrM2up7Vs7GMdc3edWnsrnOHGNrDQ++6eD98pEvvLozt4FktLFemEsZaW1ZcZMz52MbWjax+PDPf9C4BbRh8fnhz4496HMb0hZ97z+yT1bnc6Yx9pQ98p6eMvbQh/drv9eKifT37U793gdV8yv5RP7oQ0+gNstgmxYyMofHjcQgxT4XfetYrV125EuMKuKswLoe7GuMWcLdEuAjWIFjEv2ARLGLSKgSLWGO8QLgK4XKEqxEuJmzlmmuvXUzYzJr7171mMCZtFgNj4lZh4BLUIjkianyetVutnK8GbaGTW8JS4IClUBFcse2KguBKgMsJrgK4lKhpgCsJbglwFcE1AFcTXAtwS4JbAVxDyQWgRwpS4ACJE5TEcUDk5DxyYxy8y19PwTcHt9Qii8vMFVudR8I2kWZtLSw0XSlwTRdkSnz1abq2y1eo8q9XwNKzalz9MBSTsL9XDyC60lBgQHalpcCQ8MqKJAPSqwqSDIiv4thtA5fb+wY1j9rYN12Xu8vxND8OnbtxuIKcBpXE8gqjK6z+HxmArsm6BuglVq/WkrOAm+lpmr4BxlpnLQ=="
local COAL = 200
local WINDOW = 30000
local MARK = 3000

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-ore", 900)
  rcon.cheat_item(id, "transport-belt", 60)
  rcon.cheat_item(id, "inserter", 12)
  rcon.cheat_item(id, "small-electric-pole", 10)
  rcon.cheat_item(id, "iron-chest", 8)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", COAL + 200)
  -- The plant the planner will decide it needs.
  rcon.cheat_item(id, "boiler", 4)
  rcon.cheat_item(id, "steam-engine", 4)
  rcon.cheat_item(id, "offshore-pump", 2)
  rcon.cheat_item(id, "pipe", 40)
  rcon.cheat_item(id, "pipe-to-ground", 10)
end
-- Force-wide, and it takes ONLY the technology name:
-- `rcon.cheat_technology(technology_name)`. Passing a player id first reads
-- that id as the technology and fails with "no such technology: 1".
rcon.cheat_technology("electronics")

local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

local function places_in(plan)
  local n = 0
  for _, st in ipairs(plan.steps) do if st.kind == "place" then n = n + 1 end end
  return n
end
local function anchor_of(plan)
  for _, st in ipairs(plan.steps) do
    if st.kind == "stamp_ghosts" and st.pos then return st.pos end
  end
end

do
  local ok, probe = pcall(function() return goal.plan(goal.built(BP, { near = { x = 45, y = -5 } })) end)
  if ok then
    local a = anchor_of(probe)
    if a then
      for _, b in ipairs(rcon.players()) do
        local id = (type(b) == "table") and b.player_id or b
        pcall(function() rcon.move(id, { x = a.x, y = a.y + 6 }, 6) end)
      end
    end
  else
    print("PLAN REFUSED before building: " .. tostring(probe))
  end
end

local complete = false
for pass = 1, 3 do
  local ok, plan = pcall(function() return goal.plan(goal.built(BP, { near = { x = 45, y = -5 } })) end)
  if not ok then print("pass " .. pass .. " REFUSED: " .. tostring(plan)) break end
  local todo = places_in(plan)
  print(string.format("pass %d: %d to place", pass, todo))
  if todo == 0 then complete = true break end
  local obs = goal.run(plan)
  print(string.format("pass %d: done=%s failed=%s pending=%s",
    pass, tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
  if (obs.pending or 0) == 0 and (obs.failed or 0) == 0 then complete = true break end
end
if not complete then
  print("BUILD INCOMPLETE -- refusing to quote a rate for a fraction of a block")
  return
end

local furnaces0 = rcon.find_entities_in_radius({x=0,y=0}, 300, "stone-furnace")
if type(furnaces0) ~= "table" or #furnaces0 == 0 then print("FAIL: no furnace") return end
local chests = rcon.find_entities_in_radius(furnaces0[1].position, 40, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then print("FAIL: need coal + output chest") return end
-- The coal chest is the NORTH one (small y); the output chest is far south.
-- Three chests now: ore (north-west), coal (north-east), output (far south).
table.sort(chests, function(a,b) return a.position.y < b.position.y end)
local out_chest = chests[#chests]
local ore_chest, coal_chest
for _, c in ipairs(chests) do
  if c ~= out_chest then
    if not ore_chest or c.position.x < ore_chest.position.x then ore_chest = c end
    if not coal_chest or c.position.x > coal_chest.position.x then coal_chest = c end
  end
end
print(string.format("ore chest (%.1f,%.1f)", ore_chest.position.x, ore_chest.position.y))
print(string.format("coal chest (%.1f,%.1f)  output chest (%.1f,%.1f)",
  coal_chest.position.x, coal_chest.position.y, out_chest.position.x, out_chest.position.y))

rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
rcon.insert_to_inventory(bot1, "iron-chest", ore_chest.position, 1, "iron-ore", 800)

local function count_in(chest, item)
  local r = rcon.inventory_contents_at({{ name="iron-chest", x=chest.position.x, y=chest.position.y }})
  if type(r)=="table" and type(r[1])=="table" and type(r[1].output_inventory)=="table" then
    for _, s in ipairs(r[1].output_inventory) do
      if type(s)=="table" and s.name==item then return s.count or 0 end
    end
  end
  return 0
end

-- **The plant the planner built needs FUEL.** It placed 17 entities beyond the
-- block -- boiler, engine, pump, pipes, poles -- and a boiler with no coal
-- makes no steam, so no power, so no inserter swings and the whole block reads
-- as dead. Reported as well as charged, so a run says which it was.
do
  local boilers = rcon.find_entities_in_radius({x=0,y=0}, 300, "boiler")
  local engines = rcon.find_entities_in_radius({x=0,y=0}, 300, "steam-engine")
  local pumps = rcon.find_entities_in_radius({x=0,y=0}, 300, "offshore-pump")
  print(string.format("PLANT: %d boiler(s), %d engine(s), %d pump(s)",
    type(boilers)=="table" and #boilers or 0,
    type(engines)=="table" and #engines or 0,
    type(pumps)=="table" and #pumps or 0))
  if type(boilers) == "table" then
    for _, b in ipairs(boilers) do
      rcon.insert_to_inventory(bot1, "boiler", b.position, 1, "coal", 50)
      print(string.format("  fuelled boiler at (%.1f,%.1f) with 50 coal",
        b.position.x, b.position.y))
    end
  end
end

local t0 = rcon.game_tick()
print(string.format("charged at tick %s; measuring %d ticks", tostring(t0), WINDOW))
print("  tick   elapsed  plates(out chest)  plates/min  coal left")
local next_mark, last = MARK, 0
while true do
  local t = rcon.game_tick()
  if type(t) ~= "number" or type(t0) ~= "number" then break end
  local elapsed = t - t0
  if elapsed >= next_mark then
    last = count_in(out_chest, "iron-plate")
    print(string.format("  %-7d %-8d %-18d %-11.1f %d", t, elapsed, last,
      elapsed > 0 and (last * 3600.0 / elapsed) or 0, count_in(coal_chest, "coal")))
    next_mark = next_mark + MARK
  end
  if elapsed >= WINDOW then break end
end

print("")
print(string.format("PLATES INTO THE OUTPUT CHEST: %d  (%.1f/min)", last, last * 3600.0 / WINDOW))
print("No drain, no apparatus: an electric arm took every one of these out of a")
print("furnace and put it on a belt. That is the first block here that empties")
print("itself, and the first whose plates are counted somewhere a bot never went.")
print("end electric smelt row")
