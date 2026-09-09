-- How much of a world-record save is Nauvis, and how much is everywhere else?
--
-- The owner expects the other planets to be far smaller: a speedrun visits,
-- takes what it needs and leaves. If that holds, the ingest problem is a
-- Nauvis problem and enumerating every surface is cheap -- which decides
-- whether the chunk-replay rate or the surface count is the thing to fix.
--
-- Asks the GAME directly. Our own replay is hard-coded to `game.surfaces[1]`,
-- so the model cannot answer this: it has never looked at any other surface,
-- and the drop counter reads zero for that reason rather than because there is
-- nothing there. Absence of data is not absence of the thing.

local cmd = "/c local out = {} " ..
  "for _, s in pairs(game.surfaces) do " ..
  "  local chunks = 0 " ..
  "  for _ in s.get_chunks() do chunks = chunks + 1 end " ..
  "  local ents = s.count_entities_filtered{force='player'} " ..
  "  out[#out+1] = s.name .. ' chunks=' .. chunks .. ' player_entities=' .. ents " ..
  "end " ..
  "rcon.print(table.concat(out, ' | '))"

print("== surfaces, asked of the game ==")
local ok, reply = pcall(rcon.silent_print, cmd)
if not ok then
  -- fall back to whatever binding exists
  ok, reply = pcall(rcon.print, cmd)
end
print(tostring(reply))
