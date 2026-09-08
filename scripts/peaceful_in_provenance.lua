-- Does a run record whether biters were hunting it?
--
-- Provenance carries the seed, the map-exchange string, the game version, the
-- commit, the mod set and the bot mode -- and until 2026-09-08 it did NOT
-- carry peaceful mode. A peaceful run and a hostile one were byte-identical in
-- every one of those fields, so a run that never lost a bot because nothing was
-- hunting it would have compared directly against one that did, and the
-- difference would have been attributed to whatever change was under test.
-- That is the same shape that made `--seed` worthless while it was silently
-- ignored.
--
-- The parsers have unit tests. This is the part they cannot reach: whether the
-- QUERY works against a real game, and whether what `--peaceful` asked for is
-- what the world holds. Both queries are vanilla `/silent-command` --
-- `LuaSurface.peaceful_mode` is read/write on 2.1.17 -- so no BotBridge
-- function is involved and this answers even on a server whose bridge failed
-- to load.
--
-- Run it twice, on two isolated instances, one with `--peaceful` and one
-- without, and diff `provenance.peaceful`:
--
--   factorio-bot lua peaceful_in_provenance.lua --settings workspace/headless-c.toml \
--       --headless --bots 4 --game-speed 10 --seed 31337 --new --peaceful
--   factorio-bot lua peaceful_in_provenance.lua --settings workspace/headless-d.toml \
--       --headless --bots 4 --game-speed 10 --seed 31337 --new
--
-- It then holds the game open so the enemy census can be taken from outside
-- over RCON:
--
--   factorio-bot rcon -s localhost --settings workspace/headless-c.toml -- \
--     '/silent-command local s=game.surfaces[1] rcon.print(s.peaceful_mode)'
--
-- **The census is the point, and it corrects the obvious expectation.**
-- Peaceful mode does NOT remove biters: nests, worms and units all still
-- generate and still stand on the map, and `find_entities_filtered` finds
-- exactly as many of them either way. What changes is that they do not attack
-- unprovoked. A run reported peaceful is not a run with no enemies in it.
print("start peaceful in provenance")
local run_id = record.start({})
print("run id: " .. tostring(run_id))
-- Hold the game open, bounded in TICKS rather than in poll count or wall
-- clock, so a loaded box makes this take longer and never makes it cover less.
local t0 = rcon.game_tick()
for _ = 1, 200000 do
  local t = rcon.game_tick()
  if type(t) == "number" and type(t0) == "number" and (t - t0) > 400000 then break end
end
print("end peaceful in provenance")
