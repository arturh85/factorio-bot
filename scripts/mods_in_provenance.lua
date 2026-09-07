-- Does a run record which mods it loaded?
--
-- Provenance already carries the seed, the map-exchange string, the game
-- version, the commit and whether the tree was dirty -- and until 2026-09-07 it
-- did NOT carry the mod set. Two runs on the same seed, commit and game version
-- are still not comparable if one loaded a mod that adds prototypes or supplies
-- infinite resources, and nothing afterwards could tell them apart. That is the
-- same shape that made every pre-2026-09-04 timing unidentifiable when `--seed`
-- was silently ignored.
--
-- The parser has unit tests. This is the part they cannot reach: whether the
-- QUERY works against a real game. `game.active_mods` and `rcon.print` are both
-- vanilla, deliberately -- asking BotBridge which mods loaded would answer only
-- when BotBridge loaded, which is the case nobody doubts.
print("start mods in provenance")
local run_id = record.start({})
print("run id: " .. tostring(run_id))
print("end mods in provenance")
