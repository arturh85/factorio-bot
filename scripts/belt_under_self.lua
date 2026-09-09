-- Live check of the 2026-09-09 placement fix, against the real game:
--   1. a transport-belt placed on the tile the acting character stands on
--      is BUILT (the game allows it; the mod used to refuse it as
--      `§player_blocks_placement§`);
--   2. a stone-furnace whose box covers the actor still answers the
--      sentinel, now with a landing, and `place_entity_timed` walks the
--      actor there and re-issues, so the furnace stands too.
local function show(label, ok, r)
  if ok then
    if type(r) == "table" then
      print(label .. ": OK " .. tostring(r.name) .. " at " .. tostring(r.position.x) .. "," .. tostring(r.position.y))
    else
      print(label .. ": OK " .. tostring(r))
    end
  else
    print(label .. ": FAILED " .. tostring(r))
  end
end

rcon.cheat_item(1, "transport-belt", 10)
rcon.cheat_item(1, "stone-furnace", 2)
local pos = world.player(1).position
print("bot 1 stands at " .. pos.x .. "," .. pos.y)
local tile = { x = math.floor(pos.x) + 0.5, y = math.floor(pos.y) + 0.5 }

show("belt under own feet", pcall(rcon.place_entity, 1, "transport-belt", tile, 12))
local after = world.player(1).position
print("bot 1 after the belt: " .. after.x .. "," .. after.y)

-- Clean ground: walk the actor to (5.5, 0.5); a 2x2 furnace centred at (6, 0)
-- spans x 5..7, y -1..1, so the actor's tile is inside its footprint.
rcon.move(1, { x = 5.5, y = 0.5 }, 0.3)
after = world.player(1).position
print("bot 1 walked to: " .. after.x .. "," .. after.y)
local furnace_at = { x = 6, y = 0 }
show("furnace over own feet", pcall(rcon.place_entity, 1, "stone-furnace", furnace_at, 0))
local last = world.player(1).position
print("bot 1 after the furnace: " .. last.x .. "," .. last.y)
print("belt_under_self: done")
