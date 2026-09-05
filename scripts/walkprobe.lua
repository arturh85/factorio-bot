-- Measures where a walk actually comes to rest, relative to what was asked
-- for.
--
-- `rcon.move(id, goal, radius)` asks the game for a path ending within
-- `radius` of `goal`; the mod's follower then walks it and stops somewhere.
-- The question this answers is the one the outer-ring aim depends on: how far
-- PAST the requested radius does a bot rest?
--
-- Positions are read from `world.player(id)`, which under `--headless` is
-- exact: `poll_character_bot` writes a character's position on every tick it
-- changes, unlike the per-tile `on_player_changed_position` a graphical
-- client raises.
--
-- One line per trial, prefixed PROBE, tab separated:
--   PROBE bot radius goal_x goal_y rest_x rest_y dist overshoot

local RADII = { 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0 }
local LEGS = {
  { 3, 0 }, { 0, 3 }, { -3, 0 }, { 0, -3 },
  { 5, 5 }, { -5, 5 }, { -5, -5 }, { 5, -5 },
  { 20, 0 }, { 0, 20 }, { -20, 0 }, { 0, -20 },
  { 14, 14 }, { -14, 14 }, { -14, -14 }, { 14, -14 },
  { 10, 0 }, { 0, 10 }, { -10, 0 }, { 0, -10 },
  { 7, 7 }, { -7, 7 }, { -7, -7 }, { 7, -7 },
}

local function dist(a, b)
  local dx, dy = a.x - b.x, a.y - b.y
  return math.sqrt(dx * dx + dy * dy)
end

local function pos_of(id)
  local p = world.player(id)
  if p == nil or type(p.position) ~= "table" then
    return nil
  end
  return { x = p.position.x, y = p.position.y }
end

local ids = rcon.players()
print("probing with " .. #ids .. " bots")

local trial = 0
for _, id in ipairs(ids) do
  for i, leg in ipairs(LEGS) do
    local radius = RADII[((trial) % #RADII) + 1]
    trial = trial + 1
    local here = pos_of(id)
    if here ~= nil then
      local goal = { x = here.x + leg[1], y = here.y + leg[2] }
      local ok, err = pcall(function()
        rcon.move(id, goal, radius)
      end)
      if ok then
        local rest = pos_of(id)
        if rest ~= nil then
          local d = dist(rest, goal)
          print(string.format(
            "PROBE\t%d\t%.4f\t%.4f\t%.4f\t%.4f\t%.4f\t%.4f\t%.4f",
            id, radius, goal.x, goal.y, rest.x, rest.y, d, d - radius))
        end
      else
        print(string.format("PROBEFAIL\t%d\t%.4f\t%.4f\t%.4f\t%s",
          id, radius, goal.x, goal.y, tostring(err)))
      end
    end
  end
end
print("probe done")
