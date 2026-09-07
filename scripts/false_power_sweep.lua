-- WHERE does `StampGhosts` sweep, and does it eat the pole run?
--
-- Probe 1 (false_power_probe.lua) found the executor mining a pole
-- `ensure_powered` had just placed:
--
--   WARN mining entity in build area: small-electric-pole @ 3.5/-2.5
--
-- `FactorioRcon::place_blueprint` clears its build area by mining every
-- non-character, non-resource entity inside it. It computes that area with
-- `blueprint_build_area`, which returns the rect in the blueprint's OWN offset
-- space, and then throws the position away and re-centres a same-sized rect on
-- the anchor:
--
--   width_2 = area.width()/2 ; build_area = position +/- (width_2, height_2)
--
-- For this blueprint the entities span x[3,11] y[-4,6] about the anchor, so
-- the swept rect should be x[-4,4] y[-5,5] -- overlapping the block it is
-- supposed to be clearing only in the strip x[3,4].
--
-- TWO MARKERS, and each is the other's control. Neither sits on a tile the
-- block wants, so neither can be refused as occupied ground:
--
--   A = (-3.5, 0.5)   INSIDE the (predicted) sweep, OUTSIDE the block
--   B = (10.5, -2.5)  OUTSIDE the sweep, INSIDE the block's own footprint
--
-- mis-centred  =>  A is mined, B survives
-- correct      =>  A survives, B is mined
--
-- A single marker could not tell those apart; that is why there are two.
print("start false power sweep")

rcon.cheat_all_technologies()
for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 12)
  rcon.cheat_item(id, "inserter", 16)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "stone-furnace", 6)
  rcon.cheat_item(id, "small-electric-pole", 60)
  rcon.cheat_item(id, "boiler", 4)
  rcon.cheat_item(id, "steam-engine", 4)
  rcon.cheat_item(id, "offshore-pump", 4)
  rcon.cheat_item(id, "pipe", 40)
end

local BP = "0eNqd1s1ugzAMAOB3yRkq8kcJ9z3BjtM0UeZpSCGgJJ1WVbz7Urq1nQqTvSOx8iWxYocj29k9jL5zkdVH1kXoWX0zljHb7MCmsQcLbfRd+9iDjeBTBFzsYgeB1U/H88fhxe37XQrWPGOu6SHNi75xYRx8zJNzAschpGmDO633yWq50Rk7sLrY6Cljr51Py8xRNWV3rECzisJKNKsprEKzJYXVaHZLYUs0W1HYLZo1FLZCs7yguObidn5wefsO4Y9Lm/NkLii8uDIugD8XzBpyt7FiieTkSzrv7hdcLcGCDhcoWKJyeUHlSi4VIpcXRKByqRHkT/FwlFgixIokXgsnxMFB/rb3rmlhYaMzKhdzV2ER8wdiCNlSmLOJgpAtnMjJvVajHhxBbrY4V5K7Lc5V5HaLczW93+Lga+GEvrE2h+//jHwcLKy/62udV2xpXnlTk0tc9T/udGmn54x9gA9zVJfCKGO0VIJLLqbpC6RSGTE="

local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

local A = { x = -3.5, y = 0.5 }
local B = { x = 10.5, y = -2.5 }
pcall(function() rcon.place_entity(bot1, "iron-chest", A, 0) end)
pcall(function() rcon.place_entity(bot1, "iron-chest", B, 0) end)

local function stands(p)
  local r = rcon.find_entities_in_radius(p, 0.4, "iron-chest")
  return type(r) == "table" and #r > 0
end
print(string.format("markers before the build: A=%s  B=%s", tostring(stands(A)), tostring(stands(B))))
if not (stands(A) and stands(B)) then
  print("FAIL: a marker did not stand, so its later absence would prove nothing")
  print("end false power sweep")
  return
end

local ok, plan = pcall(function() return goal.plan(goal.built(BP, { x = 0, y = 0 })) end)
if not ok then
  print("PLAN REFUSED: " .. tostring(plan))
  print("end false power sweep")
  return
end
local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))

local a, b = stands(A), stands(B)
print(string.format("markers after the build:  A=%s  B=%s", tostring(a), tostring(b)))
if (not a) and b then
  print("RESULT: the sweep is RE-CENTRED ON THE ANCHOR. It mined a marker the")
  print("  block never wanted and left one standing inside the block's own")
  print("  footprint -- so it clears the wrong ground in both directions.")
elseif a and (not b) then
  print("RESULT: the sweep covers the block's own footprint, as intended.")
elseif (not a) and (not b) then
  print("RESULT: both mined -- the sweep is wider than either rectangle.")
else
  print("RESULT: neither mined -- no sweep reached either marker.")
end

print("end false power sweep")
