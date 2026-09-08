-- Holds a headless game open long enough for a shell RCON driver to run the
-- defence experiment against it.
--
-- `hold_open.lua` stops after 9,000 ticks, which at 10x is about 15 seconds of
-- wall clock -- shorter than one arm-and-fight sequence. This is the same
-- shape with a budget read from a file, so the bound stays in GAME TICKS
-- (CLAUDE.md: a poll count is not a duration) while still being adjustable
-- without an edit.
--
-- DISCLOSED: every iteration is an RCON round trip on the script's own
-- connection, so this probe costs the server some tick rate. Nothing here is
-- measured against wall clock, so that cost changes no number this run
-- produces.
print("start defence hold")
local players = rcon.players()
print("BOTS_PRESENT=" .. tostring((type(players) == "table") and #players or -1))

local budget = tonumber(file_read("defence_hold_ticks.txt")) or 60000
local t0 = rcon.game_tick()
print("t0=" .. tostring(t0) .. " budget=" .. tostring(budget))
-- Between polls, burn time in the SCRIPT rather than in the server. A tight
-- `rcon.game_tick()` loop is one RCON round trip per iteration and the server
-- answers those between ticks, so the hold itself starved the game to ~2 tps
-- and a 24-sample fight covered 170 ticks. The burn is host-side Lua and
-- costs the game nothing.
local t = t0
while type(t) == "number" and type(t0) == "number" and (t - t0) < budget do
  t = rcon.game_tick()
  local x = 0
  for i = 1, 300000 do x = x + i end
end
print("HELD_TICKS=" .. tostring(t - t0))
print("end defence hold")
