-- What the GAME says is standing on a rectangle, beside what the MODEL's
-- blocked-box tree says blocks there.
--
-- # Why this exists
--
-- A build refused with "cannot build burner-mining-drill at tile (-16,-14):
-- occupied by a tree, cliff, rock or unit". There was no tree. Two sessions
-- asked the game and asked the model and neither could show one, because
-- neither had a way to ask the question: the refusal is decided against
-- `EntityGraph`'s *blocked* tree, and nothing in Lua could enumerate it.
-- `world.find_entities_in_radius` reads a different structure -- a whitelist
-- of factory entity types, which trees, small rocks, cliffs, units and water
-- never enter -- so its silence proved nothing at all.
--
-- `world.blocked_boxes` is that missing binding, and this script is the diff
-- it exists for. It is read-only: it queries, it prints, it changes nothing.
--
-- # How to read the output
--
-- `coverage` comes first and everything else is conditional on it.
--   charted        every tile of the rectangle has been written out to the
--                  model, so an empty box list means the ground is clear
--   partial        some of it has; absence over the rest means nothing
--   unknown        NONE of it has. An empty box list here is "nobody looked",
--                  not "clear". Never read it as clear.
--   outside_model  the rectangle leaves the region the index covers at all
--
-- A box reads as "a box, minable, source unknown". That is not a hedge -- the
-- tree stores one bit per box and no name, so anything more specific would be
-- invented. `minable = true` is a tree or a rock; `false` is anything else
-- with a collision box, including a water tile.
--
-- # What a disagreement means
--
-- The interesting row is `MODEL ONLY`: a box the model believes blocks ground
-- the game says is empty. That is the shape of the (-16,-14) refusal, and this
-- is the first tool that can display it.
--
-- Two honest limits, stated because a silent one is what got us here:
--   * `rcon.find_entities_in_radius` is the only game-side query the bindings
--     expose, and it returns ENTITIES. Water is a tile, so a water box is
--     `MODEL ONLY` here for a completely ordinary reason. Check the model's
--     own tile data before reading a not-minable box as a phantom.
--   * `EntityGraph::add` deliberately keeps three kinds of entity OUT of the
--     blocked tree: resources (ore is asked about by tile), rails, and ghosts
--     (a ghost does not collide). Those are counted as EXPECTED game-only and
--     summarised rather than listed -- one 16x16 rectangle over an iron patch
--     is 110 ore tiles and would bury everything else.
--
-- # Usage
--
-- Edit the rectangle below, or run it as-is around the origin.
--   factorio-bot lua blocked_diff.lua --clients 0 --bots 1

-- The ground to ask about. Keep it small: the query walks it tile by tile and
-- refuses more than 16384 tiles.
local LEFT_TOP = { x = -24, y = -112 }
local RIGHT_BOTTOM = { x = 0, y = -96 }

print("blocked diff over ["
  .. LEFT_TOP.x .. "," .. LEFT_TOP.y .. "] .. ["
  .. RIGHT_BOTTOM.x .. "," .. RIGHT_BOTTOM.y .. "]")

-- ---------------------------------------------------------------------------
-- The model
-- ---------------------------------------------------------------------------

local report = world.blocked_boxes(LEFT_TOP, RIGHT_BOTTOM)

-- `report.boxes` is always a table, so no `or {}` guard is needed -- and a
-- guard would not have worked anyway: a nil from Rust arrives as mlua's null
-- sentinel, which is light userdata and therefore TRUTHY, so `x or {}` does
-- not substitute the default and the next `pairs` raises.
assert(type(report) == "table", "world.blocked_boxes answered a non-table")
assert(type(report.boxes) == "table", "report.boxes was not a table")

print("model coverage: " .. tostring(report.coverage)
  .. " (" .. tostring(report.tiles_charted) .. " of "
  .. tostring(report.tiles_in_area) .. " tiles written out to the model)")
print("model boxes: " .. tostring(#report.boxes))

if report.coverage == "unknown" then
  print("  ^ NOTHING here has been written out to the model. An empty box list")
  print("    above says nobody looked, NOT that the ground is clear.")
elseif report.coverage == "partial" then
  print("  ^ part of this rectangle was never written out; the boxes below are")
  print("    real but their absence elsewhere in it means nothing.")
elseif report.coverage == "outside_model" then
  print("  ^ this rectangle leaves the region the index covers, so the query")
  print("    came back short by construction.")
end

-- ---------------------------------------------------------------------------
-- The game
-- ---------------------------------------------------------------------------

-- The rectangle answered about, which `world.blocked_boxes` widened to whole
-- tiles. Asking the game about the same ground is the whole point.
local area = report.area
local cx = (area.left_top.x + area.right_bottom.x) / 2
local cy = (area.left_top.y + area.right_bottom.y) / 2
local half_w = (area.right_bottom.x - area.left_top.x) / 2
local half_h = (area.right_bottom.y - area.left_top.y) / 2
-- A circle that contains the rectangle; the extra 2 covers an entity whose
-- centre is outside it but whose body reaches in.
local radius = math.sqrt(half_w * half_w + half_h * half_h) + 2

local live = rcon.find_entities_in_radius({ x = cx, y = cy }, radius)
assert(type(live) == "table", "rcon.find_entities_in_radius answered a non-table")

local function overlaps(box)
  return box.left_top.x < area.right_bottom.x
    and area.left_top.x < box.right_bottom.x
    and box.left_top.y < area.right_bottom.y
    and area.left_top.y < box.right_bottom.y
end

local function key(box)
  return string.format("%.3f,%.3f,%.3f,%.3f",
    box.left_top.x, box.left_top.y, box.right_bottom.x, box.right_bottom.y)
end

-- The game's entities, keyed by their collision box, so a model box can be
-- matched to one by geometry -- the only thing the two sides have in common,
-- since the model side has no name to match on.
local EXPECTED_ABSENT = {
  ["resource"] = true,     -- ore: kept in EntityGraph::resources, by tile
  ["straight-rail"] = true,
  ["curved-rail"] = true,
  ["entity-ghost"] = true, -- a ghost does not collide
  ["tile-ghost"] = true,
}

local by_box = {}
local in_area, expected_absent = 0, 0
local expected_by_type = {}
for _, ent in ipairs(live) do
  local box = ent.bounding_box
  if type(box) == "table" and overlaps(box) then
    in_area = in_area + 1
    local kind = ent.entity_type
    if EXPECTED_ABSENT[kind] then
      expected_absent = expected_absent + 1
      expected_by_type[kind] = (expected_by_type[kind] or 0) + 1
    else
      local k = key(box)
      by_box[k] = by_box[k] or {}
      table.insert(by_box[k], ent.name)
    end
  end
end
print("game entities overlapping the same rectangle: " .. tostring(in_area))
if expected_absent > 0 then
  local parts = {}
  for kind, n in pairs(expected_by_type) do
    table.insert(parts, kind .. " x" .. tostring(n))
  end
  table.sort(parts)
  print("  of which " .. tostring(expected_absent)
    .. " are kinds the blocked tree deliberately excludes ("
    .. table.concat(parts, ", ") .. "); not diffed")
end

-- ---------------------------------------------------------------------------
-- The diff
-- ---------------------------------------------------------------------------

-- `report.boxes` is deliberately NOT deduplicated -- see the binding's docs --
-- so a repeat of a box already matched is its own row and its own count. It
-- must not be reported as MODEL ONLY: the model does believe something is
-- there and the game agrees; what it has is one box too many, which is a
-- bookkeeping fault and not a phantom obstacle.
local matched, model_only, duplicate = 0, 0, 0
local seen = {}
for _, b in ipairs(report.boxes) do
  local k = key(b.area)
  seen[k] = (seen[k] or 0) + 1
  local names = by_box[k]
  if names ~= nil and #names >= seen[k] then
    matched = matched + 1
    print(string.format("BOTH       %s  model: %s  game: %s",
      k, b.description, names[seen[k]]))
  elseif names ~= nil then
    duplicate = duplicate + 1
    print(string.format("DUPLICATE  %s  model: %s  game: %s (already matched; the model holds this box %d times)",
      k, b.description, names[1], seen[k]))
  else
    model_only = model_only + 1
    print(string.format("MODEL ONLY %s  model: %s  game: nothing at this box",
      k, b.description))
  end
end

local game_only = 0
for k, names in pairs(by_box) do
  local extra = #names - (seen[k] or 0)
  if extra > 0 then
    for i = #names - extra + 1, #names do
      game_only = game_only + 1
      print(string.format("GAME ONLY  %s  game: %s  model: no box here", k, names[i]))
    end
  end
end

print(string.format(
  "summary: %d both, %d duplicate, %d model only, %d game only, %d expected-absent, coverage %s",
  matched, duplicate, model_only, game_only, expected_absent, tostring(report.coverage)))

if duplicate > 0 then
  print("DUPLICATE rows mean the model holds more boxes than the game has")
  print("entities at that rectangle. That is a bookkeeping fault, not a")
  print("phantom: the ground really is blocked. It is reported rather than")
  print("collapsed because collapsing it is how it stayed invisible.")
end

if model_only > 0 then
  print("MODEL ONLY rows are the ones worth chasing: the model believes")
  print("something blocks ground the game says is empty. A water tile is the")
  print("ordinary explanation for a not-minable one -- rule that out first.")
end

print("blocked diff done")
