-- Can a bot walk PAST the generation frontier?
--
-- Settles the open question from
-- `docs/superpowers/notes/2026-09-06-exploration.md`: a survey ring walked to
-- +/-256 tiles and the world model's resource census did not change by one
-- tile. Either walking does not cause the engine to generate ground, or that
-- ring simply landed inside the ground a fresh map is created with.
--
-- **Boolean, not a timing.** Nothing here should be quoted as a duration.
--
-- Run headless, on its own instance and ports:
--   factorio-bot lua frontier_walk.lua --settings <cfg> --headless --bots 1 \
--       --game-speed 5 --seed 31337 --new
--
-- A fresh seed-31337 map is created with 400 chunks -- a 20x20 block, tiles
-- [-320, 320) -- and spawning one character bot takes it to 436 chunks with
-- the eastern frontier at 448 tiles. The probes below step across that
-- frontier, so the distance at which walking starts refusing IS the frontier
-- if walking cannot cross it, and no distance refuses if it can.
--
-- `rcon.move` is used rather than the mod's raw `action_start_walk_waypoints`
-- because the raw verb does not path: dispatched straight at (600, 0) a bot
-- walks into the first obstacle and stops -- measured, it stopped at
-- x = 63.7 -- which would have read as "walking generates nothing" for
-- entirely the wrong reason.

local probes = { 100, 200, 300, 400, 440, 480, 560, 600 }

print("FRONTIER-PROBE: walking east in steps, reporting where it starts refusing")

for _, x in ipairs(probes) do
  local ok, err = pcall(rcon.move, 1, { x = x, y = 0 }, 16)
  if ok then
    print(string.format("FRONTIER-PROBE: x=%d  REACHED", x))
  else
    local text = tostring(err)
    local why = text:match("no path") and "NO PATH" or "REFUSED"
    print(string.format("FRONTIER-PROBE: x=%d  %s -- %s", x, why, text))
  end
end

print("FRONTIER-PROBE: done")
