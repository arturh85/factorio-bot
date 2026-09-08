-- Produce a dump that has seen the WATER and the OIL, on TODAY'S schema.
--
-- ---------------------------------------------------------------------------
-- THE PREMISE THIS SCRIPT WAS COMMISSIONED ON WAS WRONG, AND MEASURING IT
-- FIRST IS WHY THE SCRIPT IS SHORT
-- ---------------------------------------------------------------------------
--
-- The brief said: `map-31337-explored.json` was charted toward the oil, water
-- on seed 31337 is near spawn, so no dump has seen both and one must be made
-- by charting water as well as oil.
--
-- **Water needs no charting at all.** Measured 2026-09-08 by reading the tile
-- trees of the archived dumps directly:
--
--     map-31337-t0.json                      5,497 water tiles, nearest at 47.7
--     map.json                               5,497 water tiles, nearest at 47.7
--     map-31337-explored.json               42,790 water tiles, nearest at 47.7
--     map-31337-explored-with-categories    42,790 water tiles, nearest at 47.7
--
-- A fresh map is generated out to +/-320 tiles at creation, and this seed's
-- nearest shore is at 47.7 -- inside it by a factor of six. So **every dump
-- this project has ever taken has seen the water**, including the t=0 ones,
-- and the explored dump has seen both water and oil already.
--
-- What the explored dump has NOT seen is today's schema. Its
-- `fluidbox_prototypes` carry no `filter`, its offshore-pump prototype no
-- `fluid_source_offset`, its tiles no `fluid`. So the refusal that reads
--
--     Nothing standing on this map can be shown to supply water ... (considered
--     and rejected: ... the offshore-pump at [46.5, -8.5] ...)
--
-- is a fact about the **world file**, not about the planner or the map: an
-- offshore pump is standing at the water in that dump and was rejected
-- because nothing in the record says what its output box yields.
--
-- **So this script charts the oil and nothing else.** It is
-- `chart_then_drill.lua`'s phase 1, stopped before the milestone, with a dump
-- at the end. The name says "water and oil" because that is what the dump
-- has seen; it does not say this script went looking for both.
--
-- ---------------------------------------------------------------------------
-- WHY THE RINGS ARE WHERE THEY ARE
-- ---------------------------------------------------------------------------
--
-- Unchanged from `chart_then_drill.lua`, which measured it: `survey_plan`
-- walks Chebyshev rings on a **256-tile lattice**, each point buying +/-128
-- around itself, so radius 384 is exactly rings 0 and 1 -- nine points -- and
-- the cell at (256, -256) covers every one of this seed's seven crude-oil
-- tiles. One ring was enough on the measured run: 8 surveys, 0 failures,
-- 4,159 ticks.
--
-- No foreknowledge is used. The loop widens blindly and never names a
-- coordinate.
--
-- Run it:
--   factorio-bot lua chart_water_and_oil.lua --headless --bots 4 \
--     --seed 31337 --new --peaceful --game-speed 10 \
--     --settings <an isolated instance toml>

print("start chart-water-and-oil")

local RING_RADII = { 384, 640, 896 }
local DUMP_PATH = "map-31337-water-and-oil.json"

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("roster: " .. tostring(#bots) .. " bot(s)")

--- Classifies a `goal.plan` failure. `code` is nil when the error is not a
--- planner verdict at all -- a fault, not a refusal.
local function classify(err)
  local message = tostring(err)
  if type(goal.refusal) == "function" then
    local ok, r = pcall(goal.refusal, err)
    if ok and type(r) == "table" and type(r.message) == "string" then
      return r.code, r.message
    end
  end
  return nil, message
end

local function try_plan(g)
  local ok, result = pcall(goal.plan, g)
  if ok then return result, nil, nil end
  local code, message = classify(result)
  return nil, code, message
end

-- ---------------------------------------------------------------------------
-- THE WATER CHECK IS OFFLINE, DELIBERATELY, BECAUSE NO BINDING ASKS IT
-- ---------------------------------------------------------------------------
--
-- The obvious thing to write here is a live probe. There is none: the mod
-- exposes `find_tiles_filtered` over RCON, but no Lua binding does, and
-- `world.*` reads the model rather than the game. Rather than fake a probe
-- out of something that answers a different question, the check is made
-- against the dump this script writes -- which is the same map, one moment
-- later, and is the artefact anybody else will read anyway. The note beside
-- the fingerprint carries the number.
--
-- What must NOT happen is the check being skipped and the conclusion kept.

-- ---------------------------------------------------------------------------
-- Chart until `gathered:crude-oil` stops refusing for want of charted ground
-- ---------------------------------------------------------------------------

local gathered = goal.gathered("crude-oil")
local plan, code, message = try_plan(gathered)
print("before any charting: " ..
  (plan and "PLANNABLE" or ("REFUSED (" .. tostring(code) .. ") " .. tostring(message))))

local rings_walked = 0

if not plan then
  for _, radius in ipairs(RING_RADII) do
    if plan then break end
    -- `planner::not_charted` is the only refusal a bigger disc can clear.
    if code ~= "planner::not_charted" then
      print("refusal is " .. tostring(code) .. ", which charting cannot clear -- not widening")
      break
    end

    local disc = goal.charted(0, 0, radius)
    local held = goal.holds(disc)
    print(string.format("-- ring radius %d: goal.holds = %s --", radius, tostring(held)))
    if held == true then
      print("  already charted; widening")
    else
      local survey = goal.plan(disc)
      print(string.format("  survey: %d step(s), %d bot(s), makespan=%s",
        #survey.steps, #survey.bots, tostring(survey.makespan)))
      if #survey.steps == 0 then
        print("  EMPTY SURVEY PLAN although goal.holds did not say true -- widening")
      else
        local t0 = rcon.game_tick()
        local obs = goal.run(survey)
        local t1 = rcon.game_tick()
        rings_walked = rings_walked + 1
        print(string.format("  survey run: done=%s success=%s failed=%s lost=%s pending=%s elapsed_ticks=%s",
          tostring(obs.done), tostring(obs.success), tostring(obs.failed),
          tostring(obs.lost), tostring(obs.pending),
          (t0 and t1) and tostring(t1 - t0) or "?"))
        if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
          print("  first_error: " .. tostring(obs.first_error))
        end
      end
    end

    plan, code, message = try_plan(gathered)
    print("  after ring: " ..
      (plan and "PLANNABLE" or ("REFUSED (" .. tostring(code) .. ") " .. tostring(message))))
  end
end

print(string.format("charting done: %d ring(s) walked", rings_walked))

-- **The dump is written whether or not the oil was found.** A dump of a world
-- that failed to chart is still a dump on today's schema and still answers
-- half the question; refusing to write it would throw away the run. What it
-- charted is printed above, and the note beside the fingerprint records it.
if not plan then
  print(string.format("gathered:crude-oil STILL REFUSED (%s): %s", tostring(code), tostring(message)))
  print("  dumping anyway -- see the comment above")
else
  print("gathered:crude-oil IS PLANNABLE: the oil has been seen")
end

print("dumping world to " .. DUMP_PATH)
world.dump(DUMP_PATH)
print("dump written")
print("end chart-water-and-oil")
