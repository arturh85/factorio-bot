-- AN ELECTRIC BLOCK WITH A REAL OUTPUT SIDE.
--
-- Every previous block here dead-ends at the furnace, because a burner arm
-- carrying plates has nothing to fuel itself with. That is why the 200-plate
-- ceiling existed and why the last measurement had to fake an output with a
-- script-side drain. Electric arms remove the whole problem: no fuel, no
-- cold-start ordering, and plates can leave.
--
-- It also removes the failure that stopped the five-drill burner block dead --
-- 0 plates, 0 coal moved -- where the coal arm never swung and every arm
-- downstream starved waiting for it. A chain whose startup depends on an
-- ordering nobody can predict is not a thing to scale.
--
-- WHAT IS CHEATED, AND WHY EACH:
--   * coal into a chest -- iron and coal are separate patches on this seed
--     (18.4 and 32.1 from spawn), so one block cannot reach both. A coal
--     outpost belting in is the next piece, not this one.
--   * `electronics` research -- so the run tests whether an electric block
--     BUILDS AND RUNS, not whether the planner can walk a research ladder.
--   * the entities themselves, as every block script here does.
-- Power is NOT cheated: the planner is asked for it and must build a plant.
print("start electric ore to plate")

local BP = "0eNqd102PmzAQgOH/4jNE/gTDfc/tYW9VVZHstLVkTGScVVcR/73eVbSiCjQzvkUkeoLB74Rc2dFf4BxdSKy/MpdgZP3qWMX8cASfjz15OKXoTl8iPE9f/ZAgv/kKcXZTYL20QredbK21TSdkxSAklxzMrP92ZWEY4Z29xACxHl1w4Vf9Ep332ThPc/7kO3Jlf1hf6wOv2Ft+IQ58qdiLi/mLP963N/btR7iMR4isF0tF1CVBl1SdE3C1wlMcwnyeYqrztU4bZ60O5gPmB/Ovq+9cTXEl3jUUV+DdhuJyvNsSXAJrCSzhKnQElnDTBCe4hE0mBMHVBFcSXENwKbU1e+7G/KHU1u65n/NyBVNysxSY0ltHgdfBuTBDTPnwfROE1sQ6NhenUJ9+w7wV2m5p/B7tCraCeLwVJC9wJcIVBa5CuLLA1Qi3JDWDcHWB2yBcU+C2CLcpcC3CbQvcDuHaki4QM1IWBYcoTpUUJxDJqXVyc5oC1D/z499wgq0RyW93jj+aPEqiJmS7txc2hq5StKGLMjV99UIglm9Iy7/fAVvn2tDWj0MphX3+9CDSVbYERrSruhIYE6/mRTKiXi2KZES+WlIfG4R6/Nyg16nN4+B9Dbe/xPV58vC/XYfQdbFuELqh6t3e5tvAm+X7svwFJPRQmA=="
local COAL = 200
local WINDOW = 30000
local MARK = 3000

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 6)
  rcon.cheat_item(id, "transport-belt", 60)
  rcon.cheat_item(id, "inserter", 10)
  rcon.cheat_item(id, "small-electric-pole", 10)
  rcon.cheat_item(id, "iron-chest", 6)
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
  local ok, probe = pcall(function() return goal.plan(goal.built(BP)) end)
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
  local ok, plan = pcall(function() return goal.plan(goal.built(BP)) end)
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

local drills = rcon.find_entities_in_radius({x=0,y=0}, 200, "burner-mining-drill")
local chests = rcon.find_entities_in_radius(drills[1].position, 40, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then print("FAIL: need coal + output chest") return end
-- The coal chest is the NORTH one (small y); the output chest is far south.
table.sort(chests, function(a,b) return a.position.y < b.position.y end)
local coal_chest, out_chest = chests[1], chests[#chests]
print(string.format("coal chest (%.1f,%.1f)  output chest (%.1f,%.1f)",
  coal_chest.position.x, coal_chest.position.y, out_chest.position.x, out_chest.position.y))

rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
end

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
print("end electric ore to plate")
