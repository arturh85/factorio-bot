-- WHY did the chain plateau: did the ore run out, or did the lane jam?
--
-- `OreToPlate` stopped at 17 plates with both furnaces holding full coal, so
-- the coal lane was fine and the ore lane dried up. I recorded two candidates
-- and said neither was established, because nothing reads a belt.
--
-- But one of them IS readable. `FactorioEntity.amount` is populated for
-- resources, so the ore remaining under each drill can be measured before and
-- after. That separates the two outright:
--
--   ore under the drills falls to 0   -> exhaustion; a burner drill works only
--                                        its own 2x2 and it mined the patch out
--   ore remains and plates stopped    -> the ore LANE is the constraint, and
--                                        the belt blind spot is what hides it
--
-- A burner drill's radius is 0.99, so it works exactly its own 2x2 footprint --
-- four tiles, and no more. That is the number that makes exhaustion plausible
-- at all, and it is why this is worth measuring rather than assuming.
print("start ore to plate why")

local BP = "0eNqd09FugyAUgOF34VobD6BVX2K72N2yLNqebSSKDdBlTeO7j65L2k2anNNLIXwg+h9FP+xx54wNoj0KE3AU7dVYJoauxyGOPTh8mh6HLmAcRBtMMOhF+3w8Pxxe7X7s0YkWMmG7EeOS4Drrd5MLeSRO1m7ycdlkT1t9iVatykwcRFusyjkTW+Nwc56t52zBSjYLFFaxWUlhNZtVFLZks5rCVmy2pLBrNltR2JrNrilsw2ZrCgsF221ILr8zIIUGd5RGSg34rQEpNuDXBqTc4NJbv3cWXT4aa+x7vnVmGJa6PNt/YZ2Cq3vgkgBfmjNusvnmA33iGvLi6hdOMfX/8xnr0YU4t7CKWzWATMn0zOAWnHpxSc9Mslyg34S+2VnyKuQlNB8mi/lb3KDb4BKuftnUp5LqjgMq2gE184Bqnl8y8YnO/0yVlWx005RKS1Bxh/kb9fTEwQ=="
local COAL = 100

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 30)
  rcon.cheat_item(id, "burner-inserter", 6)
  rcon.cheat_item(id, "iron-chest", 3)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", COAL + 60)
end
local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

-- Build in up to three passes. The same blueprint on the same seed built
-- cleanly once and lost a placement the next time, so the build is NOT
-- deterministic -- walk routing is the likely cause, since the bots start at
-- spawn and walk about twenty tiles. Replanning is the designed recovery:
-- `goal.built` re-derives the entities not yet standing, so a second pass
-- finishes the block rather than doubling it.
-- Walk a bot to the site BEFORE building. Planning is pure, so the plan can be
-- made first and read for where the block will go; the bots then start beside
-- it rather than twenty tiles away. This is the fix that cured placement losses
-- on the saturated smelter, and it works by putting the bots near the site --
-- walk routing -- not by changing the ground.
do
  local probe = goal.plan(goal.built(BP))
  local tx, ty
  for _, st in ipairs(probe.steps) do
    if st.kind == "place" and st.pos then tx, ty = st.pos.x, st.pos.y break end
  end
  if tx then
    print(string.format("walking to the site at (%.1f,%.1f) before building", tx, ty))
    for _, b in ipairs(rcon.players()) do
      local id = (type(b) == "table") and b.player_id or b
      pcall(function() rcon.move(id, { x = tx, y = ty + 6 }, 6) end)
    end
  end
end

local obs
for pass = 1, 3 do
  local plan = goal.plan(goal.built(BP))
  local places = 0
  for _, st in ipairs(plan.steps) do
    if st.kind == "place" then places = places + 1 end
  end
  if places == 0 then
    print("pass " .. pass .. ": nothing left to place")
    obs = obs or { done = true, failed = 0, pending = 0 }
    break
  end
  obs = goal.run(plan)
  print(string.format("pass %d: %d placements -> done=%s failed=%s pending=%s",
    pass, places, tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
  if (obs.failed or 0) == 0 and (obs.pending or 0) == 0 then break end
end
if (obs.failed or 0) > 0 or (obs.pending or 0) > 0 then
  print("INCOMPLETE after 3 passes -- not measuring a fraction of the block")
  return
end

local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
table.sort(furnaces, function(a, b) return a.position.y < b.position.y end)

-- A burner drill works its own 2x2, so ore within 1.5 tiles of its centre is
-- what it can actually reach. Wider than that would count ore it can never mine
-- and would make exhaustion look impossible when it is not.
local function ore_under(d)
  local found = rcon.find_entities_in_radius(d.position, 1.5, "iron-ore")
  local total, tiles = 0, 0
  if type(found) == "table" then
    for _, o in ipairs(found) do
      tiles = tiles + 1
      total = total + (o.amount or 0)
    end
  end
  return total, tiles
end

local before = {}
for i, d in ipairs(drills) do
  local amt, tiles = ore_under(d)
  before[i] = amt
  print(string.format("drill %d at (%.1f,%.1f): %d ore across %d tiles it can reach",
    i, d.position.x, d.position.y, amt, tiles))
end

local chests = rcon.find_entities_in_radius(drills[1].position, 25, "iron-chest")
rcon.insert_to_inventory(bot1, "iron-chest", chests[1].position, 1, "coal", COAL)
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
end
local t0 = rcon.game_tick()
print("charged at tick " .. tostring(t0))

local function plates(e)
  if type(e) ~= "table" or type(e.output_inventory) ~= "table" then return 0 end
  for _, s in ipairs(e.output_inventory) do
    if type(s) == "table" and s.name == "iron-plate" then return s.count or 0 end
  end
  return 0
end
-- Also read the ARMS and the COAL CHEST. "The belt backed up" is still an
-- inference; a likelier and testable mechanism is that the coal ran out. The
-- furnace arms are burner inserters that self-fuel from the coal lane, so an
-- empty coal chest starves them, they stop swinging, and ore strands on the
-- belt with the furnaces still showing full fuel -- which is exactly the
-- reading that plateau produced.
local arms = rcon.find_entities_in_radius(drills[1].position, 30, "burner-inserter")
if type(arms) ~= "table" then arms = {} end
table.sort(arms, function(a, b) return a.position.y < b.position.y end)
print("arms found: " .. #arms)

local ask = {}
for _, f in ipairs(furnaces) do
  ask[#ask + 1] = { name = "stone-furnace", x = f.position.x, y = f.position.y }
end
local COAL_IDX = #ask + 1
ask[COAL_IDX] = { name = "iron-chest", x = chests[1].position.x, y = chests[1].position.y }
local ARM0 = #ask
for _, a in ipairs(arms) do
  ask[#ask + 1] = { name = "burner-inserter", x = a.position.x, y = a.position.y }
end

local a, b, last, lastc = 0, 0, -1, t0
local coal_left, armfuel = 0, {}
local deadline = (type(t0) == "number") and (t0 + 30000) or nil
for _ = 1, 5000 do
  local r = rcon.inventory_contents_at(ask)
  a = plates((type(r) == "table") and r[1] or nil)
  b = plates((type(r) == "table") and r[2] or nil)
  local t = rcon.game_tick()
  coal_left = 0
  do
    local ce = (type(r) == "table") and r[COAL_IDX] or nil
    if type(ce) == "table" and type(ce.output_inventory) == "table" then
      for _, sl in ipairs(ce.output_inventory) do
        if type(sl) == "table" and sl.name == "coal" then coal_left = sl.count or 0 end
      end
    end
  end
  armfuel = {}
  for k = 1, #arms do
    local ae = (type(r) == "table") and r[ARM0 + k] or nil
    local f = 0
    if type(ae) == "table" and type(ae.fuel_inventory) == "table" then
      for _, sl in ipairs(ae.fuel_inventory) do
        if type(sl) == "table" and sl.name == "coal" then f = sl.count or 0 end
      end
    end
    armfuel[k] = f
  end
  if (a + b) ~= last then lastc = (type(t) == "number") and t or lastc end
  last = a + b
  if (a + b) > 0 and type(t) == "number" and (t - lastc) > 3000 then
    print("plateau at tick " .. tostring(t) .. " with " .. (a + b) .. " plates")
    print("  at the plateau: coal chest=" .. coal_left
      .. "  arm fuel=" .. table.concat(armfuel, "/"))
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("deadline at " .. tostring(t - t0) .. " ticks with " .. (a + b) .. " plates") break
  end
end

print("")
local mined, left_total = 0, 0
for i, d in ipairs(drills) do
  local amt, tiles = ore_under(d)
  local used = before[i] - amt
  mined = mined + used
  left_total = left_total + amt
  print(string.format("drill %d: %d -> %d ore (mined %d), %d tiles reachable",
    i, before[i], amt, used, tiles))
end
print(string.format("TOTAL ore mined from the ground: %d;  still in reach: %d", mined, left_total))
print(string.format("plates in the furnaces: %d + %d = %d", a, b, a + b))
print(string.format("coal left in the chest: %d;  arm fuel: %s",
  coal_left, table.concat(armfuel, "/")))
local dead_arms = 0
for _, f in ipairs(armfuel) do if f == 0 then dead_arms = dead_arms + 1 end end
if dead_arms > 0 then
  print("  " .. dead_arms .. " of " .. #armfuel .. " arms have NO FUEL -- a burner arm")
  print("  self-fuels from the coal it carries, so an empty coal lane stops it and")
  print("  strands whatever is on the belt behind it.")
end
print("")
if left_total == 0 then
  print("VERDICT: EXHAUSTION. The drills mined out every tile they can reach. A")
  print("  burner drill works only its own 2x2, so a block like this is bounded")
  print("  by four tiles per drill and stops when they are empty -- not a defect,")
  print("  a property of the machine, and an argument for electric drills whose")
  print("  5x5 reach is six times the ground.")
elseif mined > (a + b) then
  print("VERDICT: THE LANE. Ore is still reachable and was still being mined, so")
  print("  the ore stopped between the drill and the furnace. " .. (mined - (a + b))
    .. " ore is unaccounted for -- on the belt, which nothing can read.")
else
  print("VERDICT: unclear. Ore remains and mining matches plates, so the drills")
  print("  themselves stopped -- fuel or a status this run cannot see.")
end
print("end ore to plate why")
