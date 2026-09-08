-- Holds the game open so another process can probe it over RCON.
-- Longer than hold_open.lua: the fluidbox probe needs several round trips.
print("start hold long")
local t0 = rcon.game_tick()
for _ = 1, 5000000 do
  local t = rcon.game_tick()
  if type(t) == "number" and type(t0) == "number" and (t - t0) > 200000 then break end
end
print("end hold long")
