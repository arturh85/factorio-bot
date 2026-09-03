-- Write the world to `scripts/map.json` and stop. Nothing else.
--
-- This is the map-generating half of the seed search (workstream 0b of
-- `docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md`). Everything
-- after it -- distances, water, makespan, verdict -- happens offline:
--
--   factorio-bot score-map --world <workspace>/scripts/map.json --bots 1,2,3,4
--
-- It plans nothing and executes nothing, so it costs one server start and the
-- initial chunk discovery (~7 s at one chunk per tick) rather than a 20-minute
-- run. Use `--clients 0`: a graphical client would add ~26 s of sprite loading
-- and a connect wait, and buys nothing here -- the map is the map before any
-- bot connects.
--
-- WHAT IS IN THE DUMP, AND WHAT IS NOT
--
-- `EntityGraph` holds CHARTED chunks. At this moment that is what the save was
-- created with: measured off `workspace/server-log.txt`, 418 chunks with tiles
-- spanning [-320, 320) on both axes. `score-map` probes that and prints a
-- `charting` line -- read it before believing any distance. Ore outside the
-- charted square is invisible here and is not evidence of a bad map.
--
-- The discovery pass has also been seen returning empty chunks whose real
-- contents arrived thousands of ticks later
-- (`docs/superpowers/notes/2026-09-02-resource-double-count.md`), so the
-- resource counts in a t=0 dump are a LOWER BOUND.
--
-- THE SEED IS NOT IN THE FILE NAME BECAUSE THIS SCRIPT CANNOT SEE IT
--
-- A Lua script gets no access to the CLI arguments, so the caller renames the
-- output -- `tools/seed_search.sh` does. Two dumps of two seeds both land at
-- `scripts/map.json` and the second overwrites the first; move it before the
-- next run. `score-map` prints a resource fingerprint, which is what tells two
-- maps apart after the fact (equal digests mean the same map; unequal means
-- unknown, because charting grows).
print("dumping the world")

world.dump("map.json")

print("wrote scripts/map.json -- score it with `factorio-bot score-map --world <that file>`")
