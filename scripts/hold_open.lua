-- Holds the game open so another process can probe it over RCON.
print("start hold open")
local t0 = rcon.game_tick()
for _ = 1, 100000 do
  local t = rcon.game_tick()
  if type(t) == "number" and type(t0) == "number" and (t - t0) > 9000 then break end
end
print("end hold open")
