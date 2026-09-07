-- ORE TO PLATE IN ONE BLOCK: does the chain hold end to end?
--
-- Three blocks have each been proven alone: BurnerMinerLine got ore out of the
-- ground onto a belt and sited itself on ore; TJunctionSmelter merged ore and
-- coal on one belt and fed furnaces; SmeltingBlock showed a machine consuming
-- and producing. **They have never been run as one chain**, and the join is
-- exactly where a block that "places 100% correctly" tends to do nothing.
--
-- This is one blueprint, so ore never touches a chest between the ground and
-- the plate. The lane arithmetic is the whole design:
--
--   * drills sit WEST of the belt facing east, and a drill drops onto the FAR
--     lane -- from the west that is the EAST lane;
--   * coal joins from the WEST as a T-junction, and a sideload fills the NEAR
--     lane -- from the west that is the WEST lane;
--   * the furnace arms sit EAST of the belt picking west, so they meet the far
--     (coal) lane first and fall back to the near (ore) lane once a furnace's
--     fuel slot fills.
--
-- Two commodities, two lanes, one belt, and nothing merging them but the belts.
--
-- **No site is passed**, so `Site::Anywhere` reaches `nearest_ore_seed` and the
-- drills force the block onto ore. That also means the furnaces are sited by
-- the ore rather than by clear ground, which is new: every previous block chose
-- its own empty spot.
--
-- CHEATED, DISCLOSED: build materials; coal into the coal chest; coal into each
-- drill, because a burner drill mining IRON cannot fuel itself. Nothing else --
-- in particular the ore is NOT cheated, which is the point: it comes out of the
-- ground.
print("start ore to plate")

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

local ok, err = pcall(function() return goal.plan(goal.built(BP)) end)
if not ok then
  print("REFUSED: " .. tostring(err):gsub("%s+", " "):sub(1, 220))
  print("  A refusal is a result. Siting a block that needs BOTH ore under its")
  print("  drills and room for furnaces is strictly harder than either alone.")
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
local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
if type(drills) ~= "table" or #drills < 2 or type(furnaces) ~= "table" or #furnaces < 2 then
  print("FAIL: want 2 drills and 2 furnaces; got "
    .. tostring(type(drills) == "table" and #drills or 0) .. " and "
    .. tostring(type(furnaces) == "table" and #furnaces or 0))
  return
end
table.sort(furnaces, function(a, b) return a.position.y < b.position.y end)
print(string.format("drills at (%.1f,%.1f) (%.1f,%.1f); furnaces at (%.1f,%.1f) (%.1f,%.1f)",
  drills[1].position.x, drills[1].position.y, drills[2].position.x, drills[2].position.y,
  furnaces[1].position.x, furnaces[1].position.y, furnaces[2].position.x, furnaces[2].position.y))

local under = {}
for _, res in ipairs({ "iron-ore", "copper-ore", "coal", "stone" }) do
  local f = rcon.find_entities_in_radius(drills[1].position, 3, res)
  if type(f) == "table" and #f > 0 then under[#under + 1] = res .. "x" .. #f end
end
print("under the first drill: " .. ((#under > 0) and table.concat(under, ", ") or "NOTHING"))
if #under == 0 then print("FAIL: siting put drills on bare ground") return end

local chests = rcon.find_entities_in_radius(drills[1].position, 25, "iron-chest")
if type(chests) ~= "table" or #chests < 1 then print("FAIL: no coal chest") return end
rcon.insert_to_inventory(bot1, "iron-chest", chests[1].position, 1, "coal", COAL)
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
end
print("charged the coal chest and fuelled " .. #drills .. " drills; NO ore was cheated")

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0) .. " -- nothing touches the block from here")

local function count_in(inv, item)
  if type(inv) ~= "table" then return 0 end
  for _, s in ipairs(inv) do
    if type(s) == "table" and s.name == item then return s.count or 0 end
  end
  return 0
end
local ask = {}
for _, f in ipairs(furnaces) do
  ask[#ask + 1] = { name = "stone-furnace", x = f.position.x, y = f.position.y }
end

local deadline = (type(t0) == "number") and (t0 + 30000) or nil
local a, b, fa, fb, last, lastc = 0, 0, 0, 0, -1, t0
local first_plate = nil
for _ = 1, 5000 do
  local r = rcon.inventory_contents_at(ask)
  local g = function(i) return (type(r) == "table") and r[i] or nil end
  a  = count_in(type(g(1)) == "table" and g(1).output_inventory or nil, "iron-plate")
  b  = count_in(type(g(2)) == "table" and g(2).output_inventory or nil, "iron-plate")
  fa = count_in(type(g(1)) == "table" and g(1).fuel_inventory or nil, "coal")
  fb = count_in(type(g(2)) == "table" and g(2).fuel_inventory or nil, "coal")
  local t = rcon.game_tick()
  if first_plate == nil and (a + b) > 0 then
    first_plate = t
    print("  >> FIRST PLATE at tick " .. tostring(t)
      .. " -- ore came out of the GROUND, crossed a belt beside coal, and was smelted")
  end
  print(string.format("  tick %-7s plates=%s/%s  furnace coal=%s/%s", tostring(t),
    tostring(a), tostring(b), tostring(fa), tostring(fb)))
  if (a + b) ~= last then lastc = (type(t) == "number") and t or lastc end
  last = a + b
  if (a + b) > 0 and type(t) == "number" and (t - lastc) > 3000 then
    print("  plateau: no new plate for " .. tostring(t - lastc) .. " ticks") break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " ticks") break
  end
end

print("")
print(string.format("PLATES: %d + %d = %d", a, b, a + b))
print(string.format("coal still in the furnaces: %d / %d", fa, fb))
if (a + b) > 0 and fa > 0 and fb > 0 then
  print("RESULT: THE CHAIN HOLDS. Ore was mined, delivered onto a belt with no")
  print("  inserter, carried alongside coal on the other lane of that same belt,")
  print("  and both commodities were taken off it into furnaces that smelted --")
  print("  with no bot in the loop and not one item cheated into the flow.")
elseif (a + b) > 0 then
  print("RESULT: plates were made, but a furnace has no coal -- the ore lane")
  print("  works and the coal lane is the suspect.")
else
  print("RESULT: no plates. Check the furnace coal: if it is zero the coal lane")
  print("  never arrived; if it is non-zero the ORE lane is the failure.")
end
print("end ore to plate")
