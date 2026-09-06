-- Does a T JUNCTION merge ore and coal onto one belt and feed a smelter?
--
-- The predecessor to this block put ore and coal in ONE chest behind ONE
-- loader, and it failed in an instructive way: over 2,500 ticks the chest's
-- coal drained 50 -> 17 while its ore never moved off 99. One commodity took
-- the belt entirely. The block made exactly one plate, from the single ore that
-- escaped before the coal took over.
--
-- So the lanes must be separated at the source, using two mechanics:
--   * an inserter drops on the belt's FAR lane -- the ore loader sits NORTH of
--     the main belt, so ore lands on the SOUTH lane;
--   * a belt running into the SIDE of another sideloads onto its NEAR lane --
--     the coal branch comes from the north, so coal lands on the NORTH lane.
-- No inserter merges the two. The belts do it.
--
-- CHEATED, DISCLOSED:
--   * build materials to the bots;
--   * iron ore into the ore chest, coal into the coal chest -- separately,
--     which is the entire point;
--   * the ORE LOADER is given coal in its fuel slot.
--
-- That last one is a stand-in, not a workaround, and the difference matters.
-- A burner inserter fuels itself from coal it carries; the ore loader carries
-- only ore, so it cannot. In the real block there is no inserter there at all:
-- ore arrives on a belt from a miner column, which outputs onto a belt
-- directly. The chest-and-arm is standing in for those miners. Every OTHER arm
-- here -- the coal loader and both takeoffs -- touches coal and must fuel
-- itself, and if any of them stops, that is a finding.
print("start tjunction smelter live")

local BP = "0eNqV08FuhCAQBuB34awbAXGFR+i1vTVNo3aakigawKYb47sXNWvbrNsMR5jwMfkzM5G6HWGw2niiJqI9dET9uktIW9XQhrunh9E0XvfmsYPWgw0lMF57DY6o52k7XF7N2NWhqGhCTNVBeOhtZdzQW58GaBGH3ukFWj78IoqfREIuRGUnMSfkTVtotmo+JzcsQ7N5DMvRrIhhczRbxLACzZ5j2ALNljHsGc3KGLZEszSLceXuatubtPkA98/QpjSYBwrNdqYerQGbauPAbntzz7rpLzuSafSsrk3+gcsjmMXDGQrmqEh3lN+JNMdHulsMFanAy9eNoii4wMNlFPyzVM73BtL3wFcNHPS7ovww0BKLyCvykpBPsG6tiILJXErBc0Y5ZfP8DXmZEQ8="
local ORE, COAL = 100, 50

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "burner-inserter", 8)
  rcon.cheat_item(id, "transport-belt", 20)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "iron-ore", ORE)
  rcon.cheat_item(id, "coal", COAL + 10)
end

local plan = goal.plan(goal.built(BP))
local bot1 = plan.bots[1]
bot1 = (type(bot1) == "table") and bot1.player_id or bot1
print(string.format("plan: %d steps, %d bots, makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
if (obs.pending or 0) > 0 or (obs.failed or 0) > 0 then
  print("BUILD INCOMPLETE -- anything below measures a block that is not all there")
end

local furnaces = rcon.find_entities_in_radius({x = 0, y = 0}, 120, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces < 2 then
  print("FAIL: want 2 furnaces, found " .. tostring(type(furnaces) == "table" and #furnaces or 0))
  return
end
table.sort(furnaces, function(a, b) return a.position.x < b.position.x end)
local fa, fb = furnaces[1], furnaces[2]
print(string.format("furnaces at (%.1f,%.1f) and (%.1f,%.1f)",
  fa.position.x, fa.position.y, fb.position.x, fb.position.y))

local all = rcon.find_entities_in_radius(fa.position, 14)
if type(all) == "table" then
  local seen = {}
  for _, e in ipairs(all) do seen[e.name] = (seen[e.name] or 0) + 1 end
  local parts = {}
  for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
  table.sort(parts)
  print("everything within 14 tiles: " .. table.concat(parts, ", "))
end

local belts = rcon.find_entities_in_radius(fa.position, 14, "transport-belt")
print("belt tiles standing: " .. tostring(type(belts) == "table" and #belts or 0) .. " (want 10)")

-- The coal chest sits further north than the ore chest (offsets -3.5 vs -1.5),
-- so y order names them. Asserting the count first means a mis-sited block
-- reads as a failure rather than as a silently mislabelled measurement.
local chests = rcon.find_entities_in_radius(fa.position, 14, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then
  print("FAIL: want 2 chests (ore and coal), found "
    .. tostring(type(chests) == "table" and #chests or 0))
  return
end
table.sort(chests, function(a, b) return a.position.y < b.position.y end)
local coal_chest, ore_chest = chests[1], chests[2]
print(string.format("coal chest (%.1f,%.1f)   ore chest (%.1f,%.1f)",
  coal_chest.position.x, coal_chest.position.y,
  ore_chest.position.x, ore_chest.position.y))

local arms = rcon.find_entities_in_radius(fa.position, 14, "burner-inserter")
if type(arms) ~= "table" then arms = {} end
table.sort(arms, function(a, b)
  if a.position.y ~= b.position.y then return a.position.y < b.position.y end
  return a.position.x < b.position.x
end)
print("arms found: " .. #arms .. " (want 4: coal loader, ore loader, 2 takeoffs)")
local labels = { "coal-loader", "ore-loader", "takeoff-A", "takeoff-B" }
for i, a in ipairs(arms) do
  print(string.format("  %-11s at (%.1f,%.1f)", labels[i] or "?", a.position.x, a.position.y))
end

rcon.insert_to_inventory(bot1, "iron-chest", ore_chest.position, 1, "iron-ore", ORE)
rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)

-- The ore loader is arms[2] by the y sort (-0.5, below the coal loader at
-- -2.5). Hand-fuelled, for the reason in the header: it carries only ore.
if arms[2] then
  rcon.insert_to_inventory(bot1, "burner-inserter", arms[2].position, 1, "coal", 5)
  print(string.format("hand-fuelled the ORE loader at (%.1f,%.1f) with 5 coal"
    .. " -- it carries no coal and stands in for a miner column",
    arms[2].position.x, arms[2].position.y))
end

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0)
  .. " -- the coal loader and both takeoffs were NOT fuelled; nothing touches the block from here")

local function count_in(inv, item)
  if type(inv) ~= "table" then return 0 end
  if type(inv[item]) == "number" then return inv[item] end
  for _, slot in ipairs(inv) do
    if type(slot) == "table" and slot.name == item then return slot.count or 0 end
  end
  return 0
end

local ask = {
  { name = "stone-furnace", x = fa.position.x, y = fa.position.y },
  { name = "stone-furnace", x = fb.position.x, y = fb.position.y },
  { name = "iron-chest",    x = ore_chest.position.x,  y = ore_chest.position.y },
  { name = "iron-chest",    x = coal_chest.position.x, y = coal_chest.position.y },
}
for _, a in ipairs(arms) do
  ask[#ask + 1] = { name = "burner-inserter", x = a.position.x, y = a.position.y }
end

local deadline = (type(t0) == "number") and (t0 + 18000) or nil
local last_total, last_change = -1, t0
local ore_left, coal_left, plates_a, plates_b, fuel_a, fuel_b = 0, 0, 0, 0, 0, 0
local armfuel = {}
local first_plate = nil

for _ = 1, 4000 do
  local r = rcon.inventory_contents_at(ask)
  local g = function(i) return (type(r) == "table") and r[i] or nil end
  local ra, rb, ro, rcoal = g(1), g(2), g(3), g(4)
  plates_a = count_in(type(ra) == "table" and ra.output_inventory or nil, "iron-plate")
  plates_b = count_in(type(rb) == "table" and rb.output_inventory or nil, "iron-plate")
  fuel_a   = count_in(type(ra) == "table" and ra.fuel_inventory or nil, "coal")
  fuel_b   = count_in(type(rb) == "table" and rb.fuel_inventory or nil, "coal")
  ore_left  = count_in(type(ro) == "table" and ro.output_inventory or nil, "iron-ore")
  coal_left = count_in(type(rcoal) == "table" and rcoal.output_inventory or nil, "coal")
  for k = 1, #arms do
    local rk = g(4 + k)
    armfuel[k] = count_in(type(rk) == "table" and rk.fuel_inventory or nil, "coal")
  end
  local t = rcon.game_tick()
  local total = plates_a + plates_b

  if first_plate == nil and total > 0 then
    first_plate = t
    print("  >> FIRST PLATE at tick " .. tostring(t))
  end
  print(string.format("  tick %-7s ore=%-4s coal=%-4s armfuel=%-12s furnfuel=%s/%s plates=%s/%s",
    tostring(t), tostring(ore_left), tostring(coal_left),
    table.concat(armfuel, "/"), tostring(fuel_a), tostring(fuel_b),
    tostring(plates_a), tostring(plates_b)))

  if total ~= last_total then
    last_change = (type(t) == "number") and t or last_change
  end
  last_total = total
  -- A plateau is measured in TICKS. A stone furnace needs 192 ticks per plate,
  -- so anything shorter than that measures the polling rate.
  if total > 0 and type(t) == "number" and type(last_change) == "number"
     and (t - last_change) > 2000 then
    print(string.format("  plateau: no new plate for %d game ticks", t - last_change))
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " game ticks elapsed")
    break
  end
end

print("")
print(string.format("ORE consumed:  %d of %d      COAL consumed: %d of %d",
  ORE - ore_left, ORE, COAL - coal_left, COAL))
print(string.format("PLATES: furnace A = %d, furnace B = %d, total = %d",
  plates_a, plates_b, plates_a + plates_b))
print("arm fuel at end: " .. table.concat(armfuel, "/")
  .. "  (coal-loader/ore-loader/takeoff-A/takeoff-B)")
print("")
print("BOTH commodities must have moved. Ore draining while coal does not, or")
print("the reverse, is the single-lane failure this block exists to fix -- and")
print("it reads as a working belt unless both counts are checked.")
print("end tjunction smelter live")
