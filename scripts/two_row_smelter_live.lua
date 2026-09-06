-- Two rows of furnaces off ONE mixed belt: does a scarce belt share fairly?
--
-- `TJunctionSmelter` proved one belt can feed two furnaces on one side, 39/39.
-- This is the shape the owner actually described and that `FurnaceLine` uses:
-- furnaces on BOTH sides of a central mixed belt. Three per side, six total.
--
-- THIS IS NOT A THROUGHPUT TEST, and reading it as one would be wrong. A single
-- burner loader arm is far slower than six stone furnaces consume (six need
-- 1.875 ore/s), so the block is **deliberately input-starved**. That is the
-- point: scarcity is what exposes unfairness. A belt with more ore than anyone
-- needs distributes perfectly no matter how badly it is designed.
--
-- Two questions, and neither has an obvious answer:
--
--   1. ROW BALANCE. The two rows draw OPPOSITE lanes first. An inserter takes
--      from the far lane before the near one, so arms SOUTH of the belt see the
--      north/COAL lane first, and arms NORTH of it see the south/ORE lane
--      first. Whether that self-corrects -- a furnace whose ore slot is full
--      forces its arm onto the other lane -- is a measurement, not an argument.
--
--   2. DISTANCE BALANCE. Ore enters at the west end and passes x=7 before x=11.
--      If the near furnaces take everything, a 24-furnace line starves at its
--      far end, and that decides whether this shape scales at all.
--
-- CHEATED, DISCLOSED: build materials to the bots; ore into the ore chest and
-- coal into the coal chest, separately; 5 coal into the ore loader, which
-- carries only ore and cannot self-fuel (in the real block a miner column feeds
-- the belt and there is no arm there). Every other arm must fuel itself.
print("start two row smelter")

local BP = "0eNqd09FugyAUBuB34VobQbDiY3S7W5ZFu7OMRLEBuq5p+u7DNrNbiss5uxTC5y/H/8S6fg87Z2xgzYmZAANrfqxlrG876OPa42HcjIeHAfoALq6DDSYY8Kx5Ol0fji92P3Rxs+EZs+0A8VRwrfW70YU8KhO3G308NtrpbZ+sKVcqY0fWFCt1ztircbC97spzdscKNCspbIlmFYWVaLaisArNrilshWZrCrtGs5rC1miWFxRX411OcXkxw8aNNt++g/+jDXnEzynmVqxu7yy43FgP7lrIJesuYJGSBbkEl5C/4DoF09t1nzgJS9SVzmi5cKUKf6WzJVBXWuHl76pyFHwrlQ+jhfwt8u0WEuwFLZPfXePTaVI6jU2nl9OJAp9ubiIqnuDYeFPZFvMJ+mxxv7UoacPNRTKfpE8XmU/RxruQr/rHfJEB18QBTwmfM/YBzl/2VCW01FqVUvCSx80vMsr/aA=="
local ORE, COAL = 150, 60

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "burner-inserter", 12)
  rcon.cheat_item(id, "transport-belt", 24)
  rcon.cheat_item(id, "stone-furnace", 8)
  rcon.cheat_item(id, "iron-ore", ORE)
  rcon.cheat_item(id, "coal", COAL + 10)
end

-- Sited with a `near` hint 40 tiles east, NOT with the default.
--
-- `goal.built(bp)` with no site searches outward from the ROSTER'S CENTROID,
-- and siting deliberately treats a character as non-blocking -- a bot can walk
-- away, so it should not veto a site. That holds for a small block. This one is
-- 27 entities spanning roughly 16x8 with furnace rows above and below a central
-- corridor, and the first attempt sited it around the bots: the executor
-- reported `the character is already walled in here ... pocket_tiles=1.0`,
-- bot 2's walk to the far end ended inside a furnace's collision box, and the
-- build stopped with 13 of 29 steps pending and only 2 of 6 furnaces standing.
--
-- The hint is a workaround for THIS run, not a fix. The gap is real: a search
-- that ignores characters can enclose the roster it searched from.
local plan = goal.plan(goal.built(BP, { near = { x = 40, y = 0 } }))
local bot1 = plan.bots[1]
bot1 = (type(bot1) == "table") and bot1.player_id or bot1
print(string.format("plan: %d steps, %d bots", #plan.steps, #plan.bots))
local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
-- Restored from the earlier block scripts, where it was already written. Its
-- absence here cost a whole run: the first attempt printed `done=true` beside
-- `pending=13` and nothing said the two could not both be true.
if (obs.pending or 0) > 0 or (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
  print("BUILD INCOMPLETE -- " .. tostring(obs.pending) .. " steps never ran.")
  print("  `done=true` beside a non-zero pending is a contradiction, not a pass;")
  print("  anything measured below would be a fraction of the block.")
  return
end

local furnaces = rcon.find_entities_in_radius({x = 0, y = 0}, 120, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces < 6 then
  print("FAIL: want 6 furnaces, found "
    .. tostring(type(furnaces) == "table" and #furnaces or 0))
  return
end
-- Split by row, then order each row west to east. The rows sit at blueprint
-- offsets y=-2 and y=3, so whichever y is smaller is the north row wherever the
-- block was sited.
table.sort(furnaces, function(a, b)
  if a.position.y ~= b.position.y then return a.position.y < b.position.y end
  return a.position.x < b.position.x
end)
local north, south = {}, {}
local ymin = furnaces[1].position.y
for _, f in ipairs(furnaces) do
  if f.position.y == ymin then north[#north+1] = f else south[#south+1] = f end
end
print(string.format("north row: %d furnaces at y=%.1f   south row: %d",
  #north, ymin, #south))
if #north ~= 3 or #south ~= 3 then
  print("FAIL: rows are " .. #north .. "/" .. #south .. ", not 3/3 -- the block")
  print("  is not the shape this measures, so the numbers below would mislead.")
  return
end

local ref = south[1]
local all = rcon.find_entities_in_radius(ref.position, 16)
if type(all) == "table" then
  local seen = {}
  for _, e in ipairs(all) do seen[e.name] = (seen[e.name] or 0) + 1 end
  local parts = {}
  for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
  table.sort(parts)
  print("everything within 16 tiles: " .. table.concat(parts, ", "))
end

local chests = rcon.find_entities_in_radius(ref.position, 16, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then print("FAIL: want 2 chests") return end
table.sort(chests, function(a, b) return a.position.y < b.position.y end)
local coal_chest, ore_chest = chests[1], chests[2]

local arms = rcon.find_entities_in_radius(ref.position, 16, "burner-inserter")
if type(arms) ~= "table" then arms = {} end
print("arms standing: " .. #arms .. " (want 8)")
-- The ore loader is the arm sharing the ore chest's x, one tile south of it.
local ore_loader = nil
for _, a in ipairs(arms) do
  if math.abs(a.position.x - ore_chest.position.x) < 0.01
     and a.position.y > ore_chest.position.y then
    ore_loader = a
  end
end

rcon.insert_to_inventory(bot1, "iron-chest", ore_chest.position, 1, "iron-ore", ORE)
rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
if ore_loader then
  rcon.insert_to_inventory(bot1, "burner-inserter", ore_loader.position, 1, "coal", 5)
  print(string.format("hand-fuelled the ore loader at (%.1f,%.1f)",
    ore_loader.position.x, ore_loader.position.y))
else
  print("WARNING: no ore loader identified; it will starve and the run means little")
end

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0))

local function count_in(inv, item)
  if type(inv) ~= "table" then return 0 end
  if type(inv[item]) == "number" then return inv[item] end
  for _, slot in ipairs(inv) do
    if type(slot) == "table" and slot.name == item then return slot.count or 0 end
  end
  return 0
end

local ask = {}
for _, f in ipairs(north) do ask[#ask+1] = { name = "stone-furnace", x = f.position.x, y = f.position.y } end
for _, f in ipairs(south) do ask[#ask+1] = { name = "stone-furnace", x = f.position.x, y = f.position.y } end
ask[#ask+1] = { name = "iron-chest", x = ore_chest.position.x,  y = ore_chest.position.y }
ask[#ask+1] = { name = "iron-chest", x = coal_chest.position.x, y = coal_chest.position.y }

local plates, fuel = {}, {}
local ore_left, coal_left = ORE, COAL
local deadline = (type(t0) == "number") and (t0 + 30000) or nil
local last_total, last_change = -1, t0

for _ = 1, 6000 do
  local r = rcon.inventory_contents_at(ask)
  local total = 0
  for i = 1, 6 do
    local e = (type(r) == "table") and r[i] or nil
    plates[i] = count_in(type(e) == "table" and e.output_inventory or nil, "iron-plate")
    fuel[i]   = count_in(type(e) == "table" and e.fuel_inventory or nil, "coal")
    total = total + plates[i]
  end
  ore_left  = count_in((type(r) == "table" and type(r[7]) == "table") and r[7].output_inventory or nil, "iron-ore")
  coal_left = count_in((type(r) == "table" and type(r[8]) == "table") and r[8].output_inventory or nil, "coal")
  local t = rcon.game_tick()

  print(string.format("  tick %-7s ore=%-4s coal=%-4s N=%s/%s/%s S=%s/%s/%s fuelN=%s/%s/%s fuelS=%s/%s/%s",
    tostring(t), tostring(ore_left), tostring(coal_left),
    plates[1], plates[2], plates[3], plates[4], plates[5], plates[6],
    fuel[1], fuel[2], fuel[3], fuel[4], fuel[5], fuel[6]))

  if total ~= last_total then last_change = (type(t) == "number") and t or last_change end
  last_total = total
  if total > 0 and type(t) == "number" and (t - last_change) > 2500 then
    print(string.format("  plateau: no new plate for %d game ticks", t - last_change))
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " ticks")
    break
  end
end

local nsum = plates[1] + plates[2] + plates[3]
local ssum = plates[4] + plates[5] + plates[6]
print("")
print(string.format("NORTH row (arms see the ORE lane first): %d + %d + %d = %d",
  plates[1], plates[2], plates[3], nsum))
print(string.format("SOUTH row (arms see the COAL lane first): %d + %d + %d = %d",
  plates[4], plates[5], plates[6], ssum))
print(string.format("west -> east, both rows: N %d/%d/%d   S %d/%d/%d",
  plates[1], plates[2], plates[3], plates[4], plates[5], plates[6]))
print(string.format("ore consumed %d of %d, coal consumed %d of %d",
  ORE - ore_left, ORE, COAL - coal_left, COAL))
print("")
print("Both rows producing means the lane asymmetry self-corrects: a furnace")
print("with a full ore slot forces its arm onto the other lane. One row at zero")
print("means it does not, and a double-sided burner line does not work.")
print("A west-heavy gradient means near furnaces starve far ones, which is what")
print("would decide whether this shape reaches 24 furnaces.")
print("REMINDER: input-starved by design. These are SHARES of a scarce belt,")
print("not rates -- do not quote any number here as throughput.")
print("end two row smelter")
