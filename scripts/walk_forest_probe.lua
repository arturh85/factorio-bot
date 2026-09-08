-- **Does a walk stall because the follower cannot hold the corridor the
-- pathfinder cleared for it?**
--
-- The archive holds exactly ONE `stuck while walking ... made no progress`
-- failure (`run-1788833726-34821`, bot 3, tick 7,980, blocked by a
-- `tree-01`), and one sample cannot answer a question about geometry. The
-- other eighteen stalls in the archive never reached `events.jsonl` at all:
-- `FactorioRcon::move_player_timed` retries a stalled walk up to
-- `WALK_ATTEMPTS` times with a fresh path, and a stall it recovers from
-- leaves nothing behind but a `warn!` line in a session log.
--
-- So this walks a bot a long way through natural forest and records every
-- walk, succeeded or failed, with the failure text verbatim. Two things come
-- out of it that the archive cannot give:
--
--   1. **A rate.** Stalls per walk, and per tile walked, on ground nobody
--      has built on -- so a stall here is never a stale path, which is the
--      cause the existing retry was written for.
--   2. **The geometry of each stall.** The mod's wording carries the leg's
--      origin, its waypoint, where the character stood, and where the probe
--      found the blocker. `tools/walk_stall_census.py` classifies the leg as
--      axis-aligned or diagonal from those, which is the falsifiable half of
--      the "the follower walks a staircase, not the line" hypothesis.
--
-- **Nothing is cheated and nothing is built.** The bot only walks, so every
-- obstacle it meets was generated with the map. That is the control the live
-- runs do not have: in a real run the blocker is usually a furnace we placed
-- one minute earlier, and "the path went stale" explains it without any
-- appeal to the follower.
--
-- One line per walk, tab separated, so the output can be cut straight out of
-- a log:
--
--   WALK  <n> <from_x> <from_y> <goal_x> <goal_y> <rest_x> <rest_y> <trees> ok
--   WALK  <n> <from_x> <from_y> <goal_x> <goal_y> - - <trees> FAIL <message>
--
-- `trees` is the tree count in a 16-tile disc about the goal, asked of the
-- GAME (`rcon.find_entities_in_radius`) rather than the model, because the
-- model only knows what has been charted and this walk is going somewhere
-- nobody has been.

-- Where the one archived tree stall happened: bot 3 was at (206, -190),
-- walking to (197.4, -179.1). The hops below march out into that quadrant
-- and then criss-cross it, so the probe is sampling the same forest rather
-- than a differently-treed part of the map.
local HOP = 40
local LEGS = {
  -- Out to the quadrant, in axis-aligned and diagonal hops alternately, so
  -- the two are sampled from the same terrain and a difference in stall rate
  -- between them cannot be a difference in where they were walked.
  { HOP, 0 }, { 0, -HOP }, { HOP, -HOP }, { HOP, 0 },
  { 0, -HOP }, { HOP, -HOP }, { HOP, 0 }, { 0, -HOP },
  { HOP, -HOP }, { -HOP, 0 }, { 0, HOP }, { -HOP, HOP },
  { HOP, 0 }, { 0, -HOP }, { HOP, -HOP }, { -HOP, 0 },
  { 0, HOP }, { -HOP, HOP }, { HOP, 0 }, { 0, -HOP },
  { HOP, -HOP }, { -HOP, 0 }, { 0, HOP }, { -HOP, HOP },
}

-- How close counts as arrived. 2.0 rather than an exact arrival because an
-- exact arrival refuses on any goal tile that happens to hold a tree, and a
-- refusal before dispatch is not the failure this probe is about.
local RADIUS = 2.0

local function pos_of(id)
  local p = world.player(id)
  if p == nil or type(p.position) ~= "table" then return nil end
  return { x = p.position.x, y = p.position.y }
end

local function tree_count(at)
  local ok, found = pcall(function()
    return rcon.find_entities_in_radius(at, 16)
  end)
  if not ok or type(found) ~= "table" then return -1 end
  local n = 0
  for _, e in ipairs(found) do
    local name = e.name or ""
    if string.sub(name, 1, 4) == "tree" then n = n + 1 end
  end
  return n
end

-- **Phase 2: short hops.** The first pass of this probe walked 24 hops of
-- 40-56 tiles through forest holding up to 98 trees in a 16-tile disc and
-- stalled **zero** times, which rules out "a tree anywhere near the route" as
-- the mechanism -- but says nothing about the walk that actually stalled.
-- That one was `leg 2 of 10` over a **14-tile** hop: ten waypoints in
-- fourteen tiles is a path threading obstacles, not crossing open ground, and
-- a 40-tile hop over grass is the opposite kind of path. So the second phase
-- hops short and repeatedly in one place, which is what makes the pathfinder
-- wind.
local SHORT = {
  { 9, -5 }, { -5, 9 }, { -9, 5 }, { 5, -9 },
  { 11, 0 }, { 0, 11 }, { -11, 0 }, { 0, -11 },
  { 7, 7 }, { -7, 7 }, { -7, -7 }, { 7, -7 },
  { 13, -3 }, { -3, 13 }, { -13, 3 }, { 3, -13 },
}
local SHORT_ROUNDS = 6

local ids = rcon.players()
print("start walk forest probe with " .. #ids .. " bots")

if ids[1] == nil then
  print("no bots; nothing to probe")
  return
end

local walks, fails, stalls = 0, 0, 0
local tiles = 0.0

local function hop(tag, i, id, leg)
  local here = pos_of(id)
  if here == nil then return end
  local goal = { x = here.x + leg[1], y = here.y + leg[2] }
  local trees = tree_count(goal)
  walks = walks + 1
  local ok, err = pcall(function() rcon.move(id, goal, RADIUS) end)
  if ok then
    local rest = pos_of(id) or here
    local dx, dy = rest.x - here.x, rest.y - here.y
    tiles = tiles + math.sqrt(dx * dx + dy * dy)
    print(string.format("%s\t%d\t%d\t%.3f\t%.3f\t%.3f\t%.3f\t%.3f\t%.3f\t%d\tok",
      tag, id, i, here.x, here.y, goal.x, goal.y, rest.x, rest.y, trees))
  else
    fails = fails + 1
    local msg = tostring(err)
    if string.find(msg, "made no progress", 1, true) ~= nil then
      stalls = stalls + 1
    end
    print(string.format("%s\t%d\t%d\t%.3f\t%.3f\t%.3f\t%.3f\t-\t-\t%d\tFAIL\t%s",
      tag, id, i, here.x, here.y, goal.x, goal.y, trees, msg))
  end
end

-- Phase 1 marches the whole roster out to the quadrant, one bot at a time, so
-- every bot is in the same forest for phase 2 and the two phases are walked on
-- the same ground.
for _, id in ipairs(ids) do
  for i, leg in ipairs(LEGS) do
    hop("WALK", i, id, leg)
  end
end

for round = 1, SHORT_ROUNDS do
  for _, id in ipairs(ids) do
    for i, leg in ipairs(SHORT) do
      hop("HOP", (round - 1) * #SHORT + i, id, leg)
    end
  end
end

print(string.format("PROBE-TOTAL\twalks=%d\tfailed=%d\tstalled=%d\ttiles=%.1f",
  walks, fails, stalls, tiles))
print("end walk forest probe")
