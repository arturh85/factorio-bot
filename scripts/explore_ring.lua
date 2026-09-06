-- Exploration end-to-end proof: a goal refused as "not charted" becomes
-- plannable after the bots go and look.
--
-- Run headless on seed 31337:
--   factorio-bot lua explore_ring.lua --headless --bots 4 --seed 31337 --new
--
-- What it establishes, in the order it establishes it:
--
--   1. `researched:oil-processing` is refused with `planner::not_charted` --
--      the world model holds no crude oil at all, because a fresh seed-31337
--      map is generated out to +/-320 and the nearest well is at 372.5 tiles.
--   2. `goal.charted(0, 0, 384)` plans a ring of surveys -- one lattice cell
--      per point whose +/-128-tile reveal is not already in the model.
--   3. Running it walks the bots there, which makes the engine generate those
--      chunks, which the mod writes out, which the model ingests.
--   4. The same goal is asked again. The refusal is gone: the bots learned
--      something they could not see at t=0, by going and looking at it.
--
-- Nothing here cheats. No `force.chart`, no `set_chart_distance`, no
-- `cheat_*`: the only thing that happens is that four characters walk.

local function census()
  -- The model's own resource census, which is what a plan reads.
  local ok, dump = pcall(world.dump, "explore-census.json")
  return ok and dump or nil
end

local function try_plan(g)
  local ok, result = pcall(goal.plan, g)
  return ok, result
end

print("== 1. is oil-processing plannable before anybody looks? ==")
local ok_before, err_before = try_plan(goal.researched("oil-processing"))
print("plannable before: " .. tostring(ok_before))
if not ok_before then
  print("refusal before: " .. tostring(err_before))
end

print("")
print("== 2. plan the ring ==")
-- 384 tiles: ring 1 of the lattice. Its cells sit at +/-256 and each reveals
-- +/-128 around itself, so the ring covers out to 384 -- which is where the
-- nearest crude oil on this seed is (372.5 tiles).
local survey = goal.plan(goal.charted(0, 0, 384))
print("survey makespan: " .. tostring(survey.makespan))
print("survey bots: " .. tostring(#survey.bots))

print("")
print("== 3. run it ==")
local observed = goal.run(survey)
print("survey run finished; observed = " .. tostring(observed))

print("")
print("== 4. is oil-processing plannable now? ==")
local ok_after, err_after = try_plan(goal.researched("oil-processing"))
print("plannable after: " .. tostring(ok_after))
if not ok_after then
  print("refusal after: " .. tostring(err_after))
end

print("")
print("== verdict ==")
if not ok_before and ok_after then
  print("PROVED: the bots charted ground they could not see at t=0")
elseif not ok_before and not ok_after then
  -- The refusal text is what says whether anything moved: a `not_charted`
  -- naming a larger census is progress even when the goal still refuses for
  -- another reason further up the ladder.
  print("STILL REFUSED -- compare the two refusal texts above; a larger census")
  print("means ground was learned even if this goal refuses for another reason")
else
  print("INCONCLUSIVE: it was already plannable before the survey")
end

census()
