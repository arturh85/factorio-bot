-- Stage 1 of loading a world-record save: what breaks, and where.
--
-- Asks the GAME (`rcon.*`) and the MODEL (`world.*`) the same questions, because
-- the failure this is most likely to hit makes them indistinguishable: our world
-- model is built from `on_chunk_generated`, and a LOADED save has every chunk
-- already generated, so that event never fires. An empty model and a broken
-- ingest look identical unless both sides are asked.
--
-- Deliberately read-only. Nothing is placed, nothing is mined.

local function count(t) local n = 0; if type(t) == "table" then for _ in pairs(t) do n = n + 1 end end; return n end

print("== the game ==")
local ok_players, players = pcall(rcon.players)
print("players: " .. (ok_players and count(players) or ("FAILED: " .. tostring(players))))

local ok_tick, tick = pcall(rcon.game_tick)
print("game tick: " .. tostring(ok_tick and tick or "FAILED"))

-- Radii chosen against the quad tree's own bound: it is built for +/-5120 tiles
-- (entity_graph.rs), so 6000 asks a question the model cannot fully answer and
-- the gap between 5000 and 6000 is the first evidence of the bound biting.
for _, r in ipairs({ 200, 1000, 5000, 6000 }) do
  local ok_g, g = pcall(rcon.find_entities_in_radius, { x = 0, y = 0 }, r, nil)
  local ok_m, m = pcall(world.find_entities_in_radius, { x = 0, y = 0 }, r, nil)
  print(string.format("radius %5d   GAME %s   MODEL %s",
    r,
    ok_g and tostring(count(g)) or "FAILED",
    ok_m and tostring(count(m)) or "FAILED"))
end

print("")
print("== the model's own census ==")
local ok_dump, dump = pcall(world.dump, "wr-census.json")
print("world.dump: " .. (ok_dump and "ok" or ("FAILED: " .. tostring(dump))))

print("")
print("== verdict ==")
print("GAME >> MODEL means the ingest never ran -- the chunk replay is the suspect.")
print("GAME ~= MODEL at 6000 but equal at 5000 means the quadtree bound is biting.")
print("Both zero means the save did not load what we think it did.")
