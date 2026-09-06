-- Does an ELECTRIC smelter run, and does its OUTPUT SIDE deliver?
--
-- Every burner block in this tree stops at the furnace. An arm carrying iron
-- plates never touches coal, so it has no fuel source and dies with its hand
-- charge -- measured on SmeltingBlock. `electronics` removes that limit, and a
-- burner block earns `electronics` from its own copper in about 37 seconds of
-- game time (block_earns_electronics.lua). This is the block that limit was
-- blocking.
--
-- CHEATED, DISCLOSED, and three of these are apparatus rather than shortcuts:
--
--   * `cheat_all_technologies`. The honest path to `electronics` is already
--     proven separately and takes 2,220 ticks; repeating it here would measure
--     that again rather than this.
--   * build materials to the bots.
--   * SOLAR PANELS placed by hand. `Goal::Built` cannot plan a generator yet --
--     `ensure_powered` charges a block's own consumers against its own budget,
--     so a 78 kW block asking for 78 kW is refused. Solar rather than a
--     boiler because no Lua binding exposes water tiles, and Factorio freeplay
--     starts at midday so solar output is deterministic at t=0.
--   * A SINK CHEST and one arm at the end of the output belt. This is
--     INSTRUMENTATION, not part of the block: the mod cannot read a belt's
--     contents at all, so plates delivered onto the output belt would be
--     invisible. The sink is how the output side becomes measurable.
print("start electric smelter live")

local BP = "0eNqd1s1ugzAMAOB3yRkq8kcJ9z3BjtM0UeZpSCGgJJ1WVbz7Urq1nQqTvSOx8iWxYocj29k9jL5zkdVH1kXoWX0zljHb7MCmsQcLbfRd+9iDjeBTBFzsYgeB1U/H88fhxe37XQrWPGOu6SHNi75xYRx8zJNzAschpGmDO633yWq50Rk7sLrY6Cljr51Py8xRNWV3rECzisJKNKsprEKzJYXVaHZLYUs0W1HYLZo1FLZCs7yguObidn5wefsO4Y9Lm/NkLii8uDIugD8XzBpyt7FiieTkSzrv7hdcLcGCDhcoWKJyeUHlSi4VIpcXRKByqRHkT/FwlFgixIokXgsnxMFB/rb3rmlhYaMzKhdzV2ER8wdiCNlSmLOJgpAtnMjJvVajHhxBbrY4V5K7Lc5V5HaLczW93+Lga+GEvrE2h+//jHwcLKy/62udV2xpXnlTk0tc9T/udGmn54x9gA9zVJfCKGO0VIJLLqbpC6RSGTE="
local ORE, COAL = 100, 40

rcon.cheat_all_technologies()
for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 6)
  rcon.cheat_item(id, "inserter", 12)
  rcon.cheat_item(id, "transport-belt", 30)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "small-electric-pole", 8)
  rcon.cheat_item(id, "solar-panel", 6)
  rcon.cheat_item(id, "iron-ore", ORE)
  rcon.cheat_item(id, "coal", COAL)
end

local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

-- Walk a bot out to the site first. Not superstition: a 33-entity block built
-- from spawn lost placements to walks that resolved inside collision boxes,
-- and walking first fixed it -- by putting the bots near the site, not by
-- changing the ground (2026-09-06, retracted and re-explained).
pcall(function() rcon.move(bot1, { x = 46, y = 0 }, 6) end)

local plan = goal.plan(goal.built(BP, { x = 40, y = 0 }))
print(string.format("plan: %d steps, %d bots", #plan.steps, #plan.bots))
local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
if (obs.failed or 0) > 0 or (obs.pending or 0) > 0 or (obs.lost or 0) > 0 then
  print("BUILD INCOMPLETE -- anything below measures a fraction of the block")
  return
end

local function count_in(inv, item)
  if type(inv) ~= "table" then return 0 end
  if type(inv[item]) == "number" then return inv[item] end
  for _, slot in ipairs(inv) do
    if type(slot) == "table" and slot.name == item then return slot.count or 0 end
  end
  return 0
end

local furnaces = rcon.find_entities_in_radius({ x = 46, y = 3 }, 20, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces < 2 then print("FAIL: no furnaces") return end
table.sort(furnaces, function(a, b) return a.position.x < b.position.x end)
local fa, fb = furnaces[1], furnaces[2]
print(string.format("furnaces at (%.1f,%.1f) and (%.1f,%.1f)",
  fa.position.x, fa.position.y, fb.position.x, fb.position.y))

local all = rcon.find_entities_in_radius(fa.position, 18)
if type(all) == "table" then
  local seen = {}
  for _, e in ipairs(all) do seen[e.name] = (seen[e.name] or 0) + 1 end
  local parts = {}
  for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
  table.sort(parts)
  print("standing: " .. table.concat(parts, ", "))
end

-- POWER, by hand. Panels go beside the block's own pole column so they join the
-- network it already carries -- `blueprint_power` established its three poles
-- are one component supplying all six arms, so feeding any one feeds all.
--
-- Panels are 3x3. The first attempt put a pole at (45.5,7.5), inside a panel it
-- had just placed, and the game refused it by name: "blocked by solar-panel".
-- Panels now go south of the pole column rather than through it.
for i = 0, 3 do
  pcall(function()
    rcon.place_entity(bot1, "solar-panel", { x = 41.5 + i * 3, y = 10.5 }, 0)
  end)
end
-- A pole chain from the block down to the panels, and ONE MORE at the far end
-- of the output belt. The first run left the sink arm at (51.5,5.5) outside
-- every pole's supply area -- the nearest covers x 44..49 -- so the belt filled
-- and the furnaces backed up with nothing able to unload them. That was the
-- apparatus failing, not the block.
for _, p in ipairs({ {45.5, 5.5}, {45.5, 6.5}, {44.5, 7.5}, {51.5, 4.5} }) do
  pcall(function()
    rcon.place_entity(bot1, "small-electric-pole", { x = p[1], y = p[2] }, 0)
  end)
end

-- VERIFY the apparatus rather than counting pcall successes. The first run
-- reported "solar panels placed: 4" because four pcalls returned without
-- raising, which is not the same as four panels standing -- and a pole that the
-- game refused by name was counted as placed. Ask the world what is there.
local built = rcon.find_entities_in_radius({ x = 46, y = 4 }, 22)
local have = {}
if type(built) == "table" then
  for _, e in ipairs(built) do have[e.name] = (have[e.name] or 0) + 1 end
end
print(string.format("apparatus standing: solar-panel x%d, small-electric-pole x%d",
  have["solar-panel"] or 0, have["small-electric-pole"] or 0))
if (have["solar-panel"] or 0) == 0 then
  print("FAIL: no solar panel stands, so the block has no power. Stopping rather")
  print("  than reporting a smelting failure that is really a power failure.")
  return
end

-- INSTRUMENTATION: a sink at the end of the output belt, because the mod cannot
-- read a belt's contents at all.
local SINK = { x = 52.5, y = 5.5 }
pcall(function() rcon.place_entity(bot1, "iron-chest", SINK, 0) end)
pcall(function() rcon.place_entity(bot1, "inserter", { x = 51.5, y = 5.5 }, 12) end)
local sink_here = rcon.find_entities_in_radius(SINK, 2, "iron-chest")
print("sink chest standing: " .. tostring(type(sink_here) == "table" and #sink_here or 0))

local chests = rcon.find_entities_in_radius(fa.position, 18, "iron-chest")
if type(chests) ~= "table" or #chests < 2 then print("FAIL: chests missing") return end
table.sort(chests, function(a, b) return a.position.y < b.position.y end)
local coal_chest, ore_chest = chests[1], chests[2]
rcon.insert_to_inventory(bot1, "iron-chest", ore_chest.position, 1, "iron-ore", ORE)
rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
print(string.format("charged ore chest (%.1f,%.1f) and coal chest (%.1f,%.1f)",
  ore_chest.position.x, ore_chest.position.y, coal_chest.position.x, coal_chest.position.y))

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0) .. " -- nothing touches the block from here")

local ask = {
  { name = "stone-furnace", x = fa.position.x, y = fa.position.y },
  { name = "stone-furnace", x = fb.position.x, y = fb.position.y },
  { name = "iron-chest", x = SINK.x, y = SINK.y },
  { name = "iron-chest", x = ore_chest.position.x, y = ore_chest.position.y },
}
local deadline = (type(t0) == "number") and (t0 + 30000) or nil
local sink, fa_out, fb_out, ore_left = 0, 0, 0, 0
local first_delivered = nil
local last_change = t0

for _ = 1, 4000 do
  local r = rcon.inventory_contents_at(ask)
  local g = function(i) return (type(r) == "table") and r[i] or nil end
  fa_out = count_in(type(g(1)) == "table" and g(1).output_inventory or nil, "iron-plate")
  fb_out = count_in(type(g(2)) == "table" and g(2).output_inventory or nil, "iron-plate")
  local s = count_in(type(g(3)) == "table" and g(3).output_inventory or nil, "iron-plate")
  ore_left = count_in(type(g(4)) == "table" and g(4).output_inventory or nil, "iron-ore")
  local t = rcon.game_tick()
  if first_delivered == nil and s > 0 then
    first_delivered = t
    print("  >> FIRST PLATE REACHED THE SINK at tick " .. tostring(t)
      .. " -- the output side works, which no burner block here can do")
  end
  if s ~= sink then last_change = (type(t) == "number") and t or last_change end
  sink = s
  print(string.format("  tick %-7s ore=%-4s furnaceA=%-3s furnaceB=%-3s SINK=%s",
    tostring(t), tostring(ore_left), tostring(fa_out), tostring(fb_out), tostring(sink)))
  if sink > 0 and type(t) == "number" and (t - last_change) > 2500 then
    print("  plateau: no new plate in the sink for " .. tostring(t - last_change) .. " ticks")
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " ticks") break
  end
end

print("")
print(string.format("PLATES DELIVERED TO THE SINK: %d", sink))
print(string.format("still in the furnaces: A=%d B=%d   ore left: %d", fa_out, fb_out, ore_left))
if sink > 0 then
  print("RESULT: an ELECTRIC smelter ran and its OUTPUT SIDE delivered. Plates")
  print("  left the furnaces, crossed a belt and reached a chest with no bot in")
  print("  the loop -- the rung every burner block in this tree stops one short")
  print("  of, because an arm carrying plates has no fuel.")
else
  print("RESULT: no plates in the sink. Check the furnace counts: if they are")
  print("  rising, smelting works and the OUTPUT arms are the failure; if they")
  print("  are zero, the block never started and power is the first suspect.")
end
print("end electric smelter live")
