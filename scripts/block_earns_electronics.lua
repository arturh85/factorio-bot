-- Can a t=0 burner block EARN the research that unlocks its own upgrade?
--
-- TJunctionSmelter runs on nothing but burner inserters, belts and stone
-- furnaces, and it cannot grow an output side: an arm carrying iron plates has
-- no fuel source. The fix is the electric `inserter`, gated behind
-- `electronics` -- which is a TRIGGER technology fired by 10 copper plates, no
-- lab and no science packs.
--
-- So the claim under test is that the block bootstraps itself: run it on
-- COPPER, and its own output unlocks the part it was missing.
--
-- THE CONTROL IS THE POINT. Crafting `copper-cable` is attempted BEFORE the
-- block smelts anything, with copper plates already in hand -- so the attempt
-- can only fail because the RECIPE is disabled, not because ingredients are
-- missing. A control that could fail for two reasons proves neither. If that
-- first craft succeeds, this test is void and says so rather than continuing.
--
-- CHEATED, DISCLOSED:
--   * build materials to the bots;
--   * copper ore into the ore chest, coal into the coal chest;
--   * 5 coal into the ore loader (it carries only ore and cannot self-fuel --
--     in the real block a miner column feeds the belt and there is no arm);
--   * 5 copper plates and some iron plates to bot 1, for the CONTROL only.
--     Five is deliberately below the trigger's threshold of ten, so the cheat
--     cannot fire the technology the block is supposed to earn.
print("start block earns electronics")

local BP = "0eNqV08FuhCAQBuB34awbAXGFR+i1vTVNo3aakigawKYb47sXNWvbrNsMR5jwMfkzM5G6HWGw2niiJqI9dET9uktIW9XQhrunh9E0XvfmsYPWgw0lMF57DY6o52k7XF7N2NWhqGhCTNVBeOhtZdzQW58GaBGH3ukFWj78IoqfREIuRGUnMSfkTVtotmo+JzcsQ7N5DMvRrIhhczRbxLACzZ5j2ALNljHsGc3KGLZEszSLceXuatubtPkA98/QpjSYBwrNdqYerQGbauPAbntzz7rpLzuSafSsrk3+gcsjmMXDGQrmqEh3lN+JNMdHulsMFanAy9eNoii4wMNlFPyzVM73BtL3wFcNHPS7ovww0BKLyCvykpBPsG6tiILJXErBc0Y5ZfP8DXmZEQ8="
local ORE, COAL = 100, 50

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "burner-inserter", 8)
  rcon.cheat_item(id, "transport-belt", 20)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "copper-ore", ORE)
  rcon.cheat_item(id, "coal", COAL + 10)
end

local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first
rcon.cheat_item(bot1, "copper-plate", 5)
rcon.cheat_item(bot1, "iron-plate", 10)

-- ---------------------------------------------------------------- CONTROL
local function try_craft(what, n)
  local ok, err = pcall(function() return rcon.craft(bot1, what, n) end)
  return ok, tostring(err)
end

local before_ok, before_err = try_craft("copper-cable", 1)
print("CONTROL: craft copper-cable BEFORE the block runs -> "
  .. (before_ok and "SUCCEEDED" or ("failed: " .. before_err)))
if before_ok then
  print("TEST VOID: copper-cable was already craftable, so a later success")
  print("  proves nothing about electronics. Stopping rather than reporting a")
  print("  result this run cannot support.")
  return
end

-- ---------------------------------------------------------------- THE BLOCK
local plan = goal.plan(goal.built(BP))
print(string.format("plan: %d steps, %d bots", #plan.steps, #plan.bots))
local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))

local furnaces = rcon.find_entities_in_radius({x = 0, y = 0}, 120, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces < 2 then print("FAIL: no furnaces") return end
table.sort(furnaces, function(a, b) return a.position.x < b.position.x end)
local fa, fb = furnaces[1], furnaces[2]

local chests = rcon.find_entities_in_radius(fa.position, 14, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then print("FAIL: want 2 chests") return end
table.sort(chests, function(a, b) return a.position.y < b.position.y end)
local coal_chest, ore_chest = chests[1], chests[2]

local arms = rcon.find_entities_in_radius(fa.position, 14, "burner-inserter")
table.sort(arms, function(a, b)
  if a.position.y ~= b.position.y then return a.position.y < b.position.y end
  return a.position.x < b.position.x
end)

rcon.insert_to_inventory(bot1, "iron-chest", ore_chest.position, 1, "copper-ore", ORE)
rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
if arms[2] then
  rcon.insert_to_inventory(bot1, "burner-inserter", arms[2].position, 1, "coal", 5)
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

local ask = {
  { name = "stone-furnace", x = fa.position.x, y = fa.position.y },
  { name = "stone-furnace", x = fb.position.x, y = fb.position.y },
}

-- Smelt past the threshold and then keep going, because the trigger is checked
-- on a 60-tick sweep and the tenth plate is not the tick it fires. Waiting in
-- GAME TICKS, not polling iterations -- a stone furnace needs 192 ticks a plate
-- and this script has twice-recorded ancestors that measured their own poll rate.
local TARGET = 20
local plates, reached_at = 0, nil
local deadline = (type(t0) == "number") and (t0 + 20000) or nil
for _ = 1, 4000 do
  local r = rcon.inventory_contents_at(ask)
  local pa = count_in(type(r) == "table" and type(r[1]) == "table"
    and r[1].output_inventory or nil, "copper-plate")
  local pb = count_in(type(r) == "table" and type(r[2]) == "table"
    and r[2].output_inventory or nil, "copper-plate")
  plates = pa + pb
  local t = rcon.game_tick()
  if reached_at == nil and plates >= 10 then
    reached_at = t
    print("  >> the block has smelted 10 copper plates at tick " .. tostring(t))
  end
  if plates >= TARGET and reached_at ~= nil and type(t) == "number"
     and (t - reached_at) > 600 then
    print("  copper plates: " .. plates .. " at tick " .. tostring(t)
      .. " (600+ ticks past the tenth, so the 60-tick trigger sweep has run)")
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline at " .. tostring(t - t0) .. " ticks, plates=" .. plates)
    break
  end
end

if plates < 10 then
  print("FAIL: only " .. plates .. " copper plates; the trigger threshold is 10.")
  print("  Nothing below would mean anything, so stopping.")
  return
end

-- ------------------------------------------------- THE SAME CRAFT, AFTER
local after_ok, after_err = try_craft("copper-cable", 1)
print("RETRY:   craft copper-cable AFTER the block smelted " .. plates
  .. " copper plates -> " .. (after_ok and "SUCCEEDED" or ("failed: " .. after_err)))

if not after_ok then
  print("RESULT: the block smelted past the threshold and copper-cable is still")
  print("  refused. Either the trigger did not fire or the unlock does not reach")
  print("  a hand craft -- check the run log for a `trigger technology` line")
  print("  before concluding which.")
  print("end block earns electronics")
  return
end

-- Go the whole way to the part the block actually needs. Each step is a recipe
-- that `electronics` unlocks or that depends on one, so a failure names which
-- rung broke rather than reporting a bare "no inserter".
--
-- Take the block's OWN copper out of the furnaces first, so the inserter is
-- made from what it smelted rather than from the five plates cheated in for the
-- control. `inventory_type` 3 is a furnace's result slot (the mod passes the
-- number straight to `entity.get_inventory`); the furnace's output count is
-- re-read afterwards, because a removal that silently did nothing would leave
-- the cheated plates to cover the craft and the claim would be wrong.
local before_take = plates
for _, f in ipairs({ fa, fb }) do
  pcall(function()
    rcon.remove_from_inventory(bot1, "stone-furnace", f.position, 3, "copper-plate", 8)
  end)
end
local rr = rcon.inventory_contents_at(ask)
local left = count_in(type(rr) == "table" and type(rr[1]) == "table"
    and rr[1].output_inventory or nil, "copper-plate")
  + count_in(type(rr) == "table" and type(rr[2]) == "table"
    and rr[2].output_inventory or nil, "copper-plate")
print(string.format("took the block's own copper: furnaces held %d, now hold %d (moved %d)",
  before_take, left, before_take - left))
local own_copper = before_take - left

-- Counts are exact, because "craft more than you can afford" is refused as a
-- whole rather than partially: 1 copper-plate -> 2 copper-cable, and an
-- electronic-circuit needs 3 cable, so two cable crafts (4 cable) cover it.
--   inserter = 1 iron-plate + 1 iron-gear-wheel + 1 electronic-circuit
--   iron-gear-wheel = 2 iron-plate
--   electronic-circuit = 1 iron-plate + 3 copper-cable
local chain = {
  { "copper-cable", 2 },
  { "iron-gear-wheel", 1 },
  { "electronic-circuit", 1 },
  { "inserter", 1 },
}
local made_inserter = false
for _, step in ipairs(chain) do
  local ok, err = try_craft(step[1], step[2])
  print(string.format("  craft %-20s x%-2d -> %s", step[1], step[2],
    ok and "ok" or ("FAILED: " .. err)))
  if not ok then break end
  if step[1] == "inserter" then made_inserter = true end
end

print("")
if made_inserter then
  print("RESULT: the block EARNED ITS OWN UPGRADE. copper-cable was refused")
  print("  before it ran and crafted after, on identical materials, and the")
  print("  chain went all the way to an electric `inserter` -- the exact part a")
  print("  burner-only block cannot fuel and therefore cannot use for an output")
  print("  side. No lab, no science pack, no research action was planned.")
  if own_copper > 0 then
    print("  The " .. own_copper .. " copper plates it was built from came out of")
    print("  the block's own furnaces.")
  else
    print("  NOTE: no copper came out of the furnaces, so the craft ran on the")
    print("  five plates cheated in for the control. The unlock is still proven;")
    print("  the provenance of the metal is not.")
  end
else
  print("RESULT: electronics unlocked (copper-cable went from refused to")
  print("  crafted), but the chain to an inserter did not complete. The unlock")
  print("  is proven; the craft chain is not.")
end
print("end block earns electronics")
