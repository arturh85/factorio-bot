-- Can a DRILL feed a belt, and can the planner site a block ON ORE?
--
-- Two things this project has never done. CLAUDE.md: "no drill has ever been
-- placed against a belt", though `MinerLine` encodes the shape. And ore-aware
-- siting -- `nearest_ore_seed` picking the seed and `drills_are_fed` refusing a
-- site whose drills cover nothing extractable -- has unit tests and has never
-- run in a game.
--
-- **No site is passed on purpose.** `Site::Anywhere` is what reaches
-- `nearest_ore_seed`, so naming an anchor would skip the very path under test.
-- If the block lands somewhere with no ore under the drills, the run has
-- failed even if every entity stands.
--
-- Burner drills, so this needs no electricity and no research: it is the front
-- end of the t=0 chain whose middle (TJunctionSmelter) and end (the electric
-- block) already run.
--
-- CHEATED, DISCLOSED:
--   * build materials to the bots;
--   * coal into each drill -- a burner drill mining IRON cannot fuel itself,
--     though one mining coal could, which is a block worth building later;
--   * coal into the sink arm, which carries ore and so has no fuel source.
--     Same limit as every other burner output arm here.
print("start burner miner line")

local BP = "0eNqd1M1qwzAMAOB30dkp8V/+jj1vTzDGSFqxGRIn2M5YCXn3Oc3Y1jYbdi8GS+izwbImaNoRB6O0g2oC5bCD6leMQFs32PrYfjQazaPyy4NffAa1U06hheppWjenFz12DRqoKAFdd+jrnKm1HXrjEu8s4NBbX9br5bwPqPhOEjhBle7kTOCoDB7WbDGTG5ZFszSE5dEsC2FFNMtDWBnNihA2i2ZlCJtHs1kIW0SzeQhbRrNFCEvTb7c5/6OkU1rp1+RoVNve4mzt3UtYbMH0HlgGwCwO/urfS5iyLZnfI+chsriWlbZonM/9+Xzl9fOlW/DPl1Om18nhDe1/M2eZZfMzgXc09pyWGStFWUouGOX+7vMnnmXOXw=="

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 6)
  rcon.cheat_item(id, "transport-belt", 20)
  rcon.cheat_item(id, "burner-inserter", 4)
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "coal", 120)
end
local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

local ok, err = pcall(function() return goal.plan(goal.built(BP)) end)
if not ok then
  print("REFUSED: " .. tostring(err):gsub("%s+", " "):sub(1, 200))
  print("  A refusal here is a RESULT: `drills_are_fed` rejecting a site whose")
  print("  drills cover no extractable resource is the check working.")
  return
end
local plan = goal.plan(goal.built(BP))
print(string.format("plan: %d steps, %d bots", #plan.steps, #plan.bots))
local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
if (obs.failed or 0) > 0 or (obs.pending or 0) > 0 or (obs.lost or 0) > 0 then
  print("BUILD INCOMPLETE -- anything below measures a fraction of the block")
  return
end

local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
if type(drills) ~= "table" or #drills < 4 then
  print("FAIL: want 4 drills, found " .. tostring(type(drills) == "table" and #drills or 0))
  return
end
print("drills standing: " .. #drills)
for _, d in ipairs(drills) do
  print(string.format("  drill at (%.1f,%.1f)", d.position.x, d.position.y))
end

-- WHAT DID SITING CHOOSE? The whole point of passing no site. A block that
-- stands on bare grass has failed even though every entity is placed.
local ref = drills[1]
local under = {}
for _, res in ipairs({ "iron-ore", "copper-ore", "coal", "stone" }) do
  local found = rcon.find_entities_in_radius(ref.position, 3, res)
  if type(found) == "table" and #found > 0 then
    under[#under + 1] = res .. "x" .. #found
  end
end
print("resource within 3 tiles of the first drill: "
  .. ((#under > 0) and table.concat(under, ", ") or "NOTHING -- siting put drills on bare ground"))
if #under == 0 then
  print("FAIL: the drills have nothing to mine, so ore-aware siting did not work")
  return
end

local chests = rcon.find_entities_in_radius(ref.position, 20, "iron-chest")
if type(chests) ~= "table" or #chests < 1 then print("FAIL: no sink chest") return end
local sink = chests[1]
local arms = rcon.find_entities_in_radius(ref.position, 20, "burner-inserter")

-- Fuel: each drill, and the sink arm. Disclosed in the header.
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 10)
end
if type(arms) == "table" then
  for _, a in ipairs(arms) do
    rcon.insert_to_inventory(bot1, "burner-inserter", a.position, 1, "coal", 5)
  end
end
print("fuelled " .. #drills .. " drills and "
  .. tostring(type(arms) == "table" and #arms or 0) .. " arm(s)")

local function count_any(e)
  if type(e) ~= "table" or type(e.output_inventory) ~= "table" then return 0, "" end
  local total, names = 0, {}
  for _, slot in ipairs(e.output_inventory) do
    if type(slot) == "table" and slot.count then
      total = total + slot.count
      names[#names + 1] = slot.name .. "x" .. slot.count
    end
  end
  return total, table.concat(names, ",")
end

local t0 = rcon.game_tick()
print("fuelling complete at tick " .. tostring(t0) .. " -- nothing touches the block from here")

local ask = { { name = "iron-chest", x = sink.position.x, y = sink.position.y } }
local deadline = (type(t0) == "number") and (t0 + 20000) or nil
local total, what, last_change, last_total = 0, "", t0, -1
for _ = 1, 4000 do
  local r = rcon.inventory_contents_at(ask)
  total, what = count_any((type(r) == "table") and r[1] or nil)
  local t = rcon.game_tick()
  print(string.format("  tick %-7s sink=%-4s %s", tostring(t), tostring(total), what))
  if total ~= last_total then last_change = (type(t) == "number") and t or last_change end
  last_total = total
  if total > 0 and type(t) == "number" and (t - last_change) > 2000 then
    print("  plateau: nothing new for " .. tostring(t - last_change) .. " ticks") break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " ticks") break
  end
end

print("")
print("ORE DELIVERED TO THE SINK: " .. total .. "  (" .. what .. ")")
if total > 0 then
  print("RESULT: drills fed a belt with NO INSERTER between them, the planner")
  print("  sited the block on ore by itself, and the ore crossed the belt into a")
  print("  chest with no bot in the loop. This is the front end of the chain.")
else
  print("RESULT: nothing reached the sink. Drills standing on resource with fuel")
  print("  means the drill-to-belt geometry is the suspect -- a drill drops onto")
  print("  the tile in front of it, and a belt one tile off receives nothing.")
end
print("end burner miner line")
