-- Does a deletion land on the surface it happened on?
--
-- The world-record save (`workspace/wrload.toml`) is the only multi-surface
-- world this project has, and it is the only place the question is askable:
-- three space platforms fly through asteroid fields and destroy them
-- continuously, so `on_some_entity_deleted` fires for a surface that is not
-- Nauvis, hundreds of times a minute, with no bot doing anything.
--
-- Until 2026-09-07 every one of those was applied to Nauvis, because
-- `forget_inventory` and `entity_graph.remove` are keyed on position alone and
-- the parser had one surface. `b0bb7f53` routed the four writeouts that name a
-- surface on the wire; this script is what makes that routing observable in a
-- live run rather than only in a unit test.
--
-- **Read-only, deliberately.** Nothing is placed, nothing is mined, no RCON
-- command mutates. The WR save is the project's only oracle and a run that
-- changed it would be unrepeatable. All this does is hold the game open long
-- enough for asteroids to die, and let the parser narrate.
--
-- What to read afterwards, in the run's stdout:
--
--   "model now tracks surface platform-4 (2 in total)"   <- routing happened
--
-- and in `workspace/wrload/server-log.txt`:
--
--   grep -o 'on_some_entity_deleted.*' | grep -o '"surface":"[a-z0-9-]*"' | sort | uniq -c
--
-- which is the count each surface is entitled to. Nauvis's share of that is
-- the acceptance measure; everything else used to land there too.

local WAIT_TICKS = 3600 -- one game minute at 1x

print("== deleted-elsewhere probe: read-only, " .. WAIT_TICKS .. " ticks ==")

local ok_tick, start = pcall(rcon.game_tick)
if not ok_tick then
  print("FAILED to read the game tick: " .. tostring(start))
  return
end
print("start tick: " .. tostring(start))

local target = start + WAIT_TICKS
while rcon.game_tick() < target do
  -- busy-wait in game time; the sandbox has no sleep. See furnace_run.lua.
end

print("end tick: " .. tostring(rcon.game_tick()))
print("== probe done; nothing was changed ==")
