-- Does `ensure_powered`'s Some survive contact with the game?
--
-- Run A of `2026-09-07-a-powered-block-that-is-not-powered.md` reported
-- 63 of 63 placed, done=true, and a block that never moved an item. The
-- handoff could not say why. This probe asks the two halves separately:
--
--   1. WHAT DID THE PLAN PROMISE? Every `place` step is printed with the
--      name and tile the planner chose, so the pole run and the plant are
--      visible before anything is built.
--   2. WHAT STANDS? After the build, every entity in a wide disc is read
--      back off the live surface, and each planned placement is matched to
--      a standing entity at the SAME position. A mismatch is a
--      model/game disagreement; a match means the plan was kept.
--   3. DOES THE STANDING GEOMETRY CARRY POWER? Wire connectivity over the
--      standing poles (7.5 tiles, small pole) and supply coverage of each
--      standing electric consumer (2.5 half-extent), computed here from the
--      GAME's positions rather than from the model's. If the model says
--      powered and this says dark, the disagreement is located.
--
-- Nothing here cheats power. That is the whole point.
print("start false power probe")

local BP = "0eNqd1s1ugzAMAOB3yRkq8kcJ9z3BjtM0UeZpSCGgJJ1WVbz7Urq1nQqTvSOx8iWxYocj29k9jL5zkdVH1kXoWX0zljHb7MCmsQcLbfRd+9iDjeBTBFzsYgeB1U/H88fhxe37XQrWPGOu6SHNi75xYRx8zJNzAschpGmDO633yWq50Rk7sLrY6Cljr51Py8xRNWV3rECzisJKNKsprEKzJYXVaHZLYUs0W1HYLZo1FLZCs7yguObidn5wefsO4Y9Lm/NkLii8uDIugD8XzBpyt7FiieTkSzrv7hdcLcGCDhcoWKJyeUHlSi4VIpcXRKByqRHkT/FwlFgixIokXgsnxMFB/rb3rmlhYaMzKhdzV2ER8wdiCNlSmLOJgpAtnMjJvVajHhxBbrY4V5K7Lc5V5HaLczW93+Lga+GEvrE2h+//jHwcLKy/62udV2xpXnlTk0tc9T/udGmn54x9gA9zVJfCKGO0VIJLLqbpC6RSGTE="

rcon.cheat_all_technologies()
for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 8)
  rcon.cheat_item(id, "inserter", 16)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "stone-furnace", 6)
  rcon.cheat_item(id, "small-electric-pole", 60)
  rcon.cheat_item(id, "boiler", 4)
  rcon.cheat_item(id, "steam-engine", 4)
  rcon.cheat_item(id, "offshore-pump", 4)
  rcon.cheat_item(id, "pipe", 40)
  rcon.cheat_item(id, "coal", 100)
  rcon.cheat_item(id, "iron-ore", 100)
end

local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

-- Run A shape: NO siting hint. The planner picks the anchor and the plant.
local ok, plan = pcall(function() return goal.plan(goal.built(BP)) end)
if not ok then
  print("PLAN REFUSED: " .. tostring(plan))
  print("end false power probe")
  return
end

local planned = {}
local nplace = 0
for _, s in ipairs(plan.steps) do
  if s.kind == "place" then
    nplace = nplace + 1
    planned[#planned + 1] = { name = s.entity, x = s.pos.x, y = s.pos.y }
  end
end
print(string.format("plan: %d steps, %d place steps, %d bots",
  #plan.steps, nplace, #plan.bots))
print("PLANNED PLACEMENTS:")
for _, p in ipairs(planned) do
  print(string.format("  plan %-22s (%.2f, %.2f)", p.name, p.x, p.y))
end

local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))

-- What stands, read off the live surface over a disc wide enough to hold
-- both the block and whatever plant the planner sited.
local centre = { x = 0, y = 0 }
if #planned > 0 then
  local sx, sy = 0, 0
  for _, p in ipairs(planned) do sx = sx + p.x; sy = sy + p.y end
  centre = { x = sx / #planned, y = sy / #planned }
end
print(string.format("scanning a 160-tile disc about (%.1f, %.1f)", centre.x, centre.y))
local standing = rcon.find_entities_in_radius(centre, 160)
if type(standing) ~= "table" then
  print("FAIL: the surface returned no entity list")
  print("end false power probe")
  return
end

local function key(name, x, y)
  return string.format("%s@%.2f,%.2f", name, x, y)
end

local at = {}
for _, e in ipairs(standing) do
  at[key(e.name, e.position.x, e.position.y)] = e
end

-- 2. Was the plan kept, entity by entity and tile by tile?
local kept, moved = 0, 0
for _, p in ipairs(planned) do
  if at[key(p.name, p.x, p.y)] then
    kept = kept + 1
  else
    moved = moved + 1
    -- Where did it go? Nearest standing entity of the same name.
    local best, bestd = nil, 1e18
    for _, e in ipairs(standing) do
      if e.name == p.name then
        local dx, dy = e.position.x - p.x, e.position.y - p.y
        local d = dx * dx + dy * dy
        if d < bestd then bestd = d; best = e end
      end
    end
    if best then
      print(string.format("  MOVED %-22s planned (%.2f, %.2f) nearest standing (%.2f, %.2f) d=%.2f",
        p.name, p.x, p.y, best.position.x, best.position.y, math.sqrt(bestd)))
    else
      print(string.format("  ABSENT %-22s planned (%.2f, %.2f) -- none of this name stands",
        p.name, p.x, p.y))
    end
  end
end
print(string.format("plan kept at the planned tile: %d of %d (%d elsewhere or absent)",
  kept, #planned, moved))

-- 3. Does the STANDING geometry carry power? Computed from the game's own
--    positions, with vanilla's numbers written out so this check shares no
--    code with the model it is testing.
local WIRE = 7.5
local SUPPLY = 2.5
local DRAW = {
  ["inserter"] = 13, ["lab"] = 60, ["assembling-machine-1"] = 75,
  ["assembling-machine-2"] = 150, ["electric-mining-drill"] = 90,
  ["electric-furnace"] = 180,
}
local GEN = { ["steam-engine"] = 900 }

local poles, gens, consumers = {}, {}, {}
for _, e in ipairs(standing) do
  if e.name == "small-electric-pole" then poles[#poles + 1] = e end
  if GEN[e.name] then gens[#gens + 1] = e end
  if DRAW[e.name] then consumers[#consumers + 1] = e end
end
print(string.format("standing: %d poles, %d generators, %d electric consumers",
  #poles, #gens, #consumers))

local parent = {}
for i = 1, #poles do parent[i] = i end
local function find(i) while parent[i] ~= i do parent[i] = parent[parent[i]]; i = parent[i] end return i end
for a = 1, #poles do
  for b = a + 1, #poles do
    local dx = poles[a].position.x - poles[b].position.x
    local dy = poles[a].position.y - poles[b].position.y
    if math.sqrt(dx * dx + dy * dy) <= WIRE then
      local ra, rb = find(a), find(b)
      if ra ~= rb then parent[ra] = rb end
    end
  end
end

-- Which components hold a generator? A generator is on a network when a pole's
-- supply area meets its footprint; a steam-engine is 3x5, so a half-box of
-- 1.5 x 2.5 is used and the orientation is not known here -- 2.5 both ways is
-- the generous reading, which can only make this check MORE optimistic than
-- the game.
local generating = {}
for _, g in ipairs(gens) do
  for i, p in ipairs(poles) do
    local dx = math.abs(p.position.x - g.position.x)
    local dy = math.abs(p.position.y - g.position.y)
    if dx <= SUPPLY + 2.5 and dy <= SUPPLY + 2.5 then
      generating[find(i)] = (generating[find(i)] or 0) + GEN[g.name]
    end
  end
end
local ncomp = 0
for _ in pairs(generating) do ncomp = ncomp + 1 end
print(string.format("pole components carrying a generator: %d", ncomp))

local dark, lit = 0, 0
for _, c in ipairs(consumers) do
  local powered = false
  for i, p in ipairs(poles) do
    local dx = math.abs(p.position.x - c.position.x)
    local dy = math.abs(p.position.y - c.position.y)
    -- a 1x1 consumer's box is 0.4 either side of its centre
    if dx <= SUPPLY + 0.4 and dy <= SUPPLY + 0.4 and generating[find(i)] then
      powered = true
      break
    end
  end
  if powered then lit = lit + 1 else
    dark = dark + 1
    print(string.format("  DARK %-22s (%.2f, %.2f)", c.name, c.position.x, c.position.y))
  end
end
print(string.format("VERDICT from the GAME's own positions: %d consumers lit, %d dark",
  lit, dark))

-- Hold the game open so the fluid side -- which no Lua binding here can read
-- -- can be asked over RCON directly. A boiler with fuel and no WATER makes no
-- steam, and `electric_supply_kw` counts NAMEPLATE capacity by its own
-- admission, so that failure is invisible to the model.
print("holding open -- probe the live game now")
local t0 = rcon.game_tick()
for _ = 1, 400000 do
  local t = rcon.game_tick()
  if type(t) == "number" and type(t0) == "number" and (t - t0) > 90000 then break end
end

print("end false power probe")
