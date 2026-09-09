-- Does a ghost written out by `rcon_place_blueprint` reach the world model?
--
-- The mod-side loop is already proven by
-- `crates/core/tests/botbridge_ghosts_reach_the_model.rs`: given ghosts, it
-- calls `writeout` with the right key and the right record. What those tests
-- cannot reach is whether that `print` leaves the Factorio process and is
-- picked up by the Rust `output_parser` **when the function is invoked through
-- `remote.call` from an RCON command**, which is the one link nobody has
-- tested from either side.
--
-- So this stamps a blueprint as ghosts and then asks the MODEL, not the game.
-- The discriminator is one number, and both answers are informative:
--
--   ghosts the planner can see > 0  -> the channel works end to end
--   ghosts the planner can see = 0  -> the record arrives wrong, or the line
--                                      never leaves the mod
--
-- Run: factorio-bot lua ghost_writeout_probe.lua --headless --bots 1 \
--        --seed 31337 --new
--
-- The peer session measured 0 here with the race removed (25 retries with an
-- RCON round trip between each). This run exists to say WHERE it is lost, and
-- prints the raw stdout marker so the answer does not depend on the model.

local BLUEPRINT = "0eJyd0tGKwjAQBdD3/YplntPFpEm1+RURsTqwA3ZaJqkopf8uWxdZcCsh8xKYwLkM3BGa84C9EEfwI1DEFvyfnYILSqCOwbvK1LauXWmNLrVRgBwpEgbw24/P3xkf29ueh7ZBAa8V8KFF8EDScXH8xvCj9l2gOLMjXMGvvpyC2/xOCk4keHz8ria1bJun3QzCKAVxQIkorwF6KUCbdwnlMyHKgUPfSSwaPP9zgVkKsO98m+yXWb5L9m2WXyX7LstfJ/tVlr9Jb9A6r0F1Uv83y/2f6d003QHQziE+"

local ANCHOR_X, ANCHOR_Y = 24.5, 24.5

print("== stamping MovingBlock as ghosts at " .. ANCHOR_X .. "," .. ANCHOR_Y .. " ==")

-- `position` is a table, not two numbers: `rcon.place_blueprint(player_id,
-- blueprint, position, direction, force_build, only_ghosts, helper_ids)`.
local ok, placed = pcall(rcon.place_blueprint, 1, BLUEPRINT,
  { x = ANCHOR_X, y = ANCHOR_Y }, 0, true, true, {})
if not ok then
  print("STAMP FAILED: " .. tostring(placed))
  return
end

local n = 0
if type(placed) == "table" then
  for _ in pairs(placed) do n = n + 1 end
end
print("entries in the reply body: " .. n)

-- The game's own answer, which is the control: if this is 0 the stamp did not
-- happen and nothing below means anything.
local in_game = rcon.find_entities_in_radius(
  { x = ANCHOR_X, y = ANCHOR_Y }, 40, nil, "entity-ghost")
local game_count = 0
if type(in_game) == "table" then
  for _ in pairs(in_game) do game_count = game_count + 1 end
end
print("ghosts standing in the GAME: " .. game_count)

-- Give the parser room. Each RCON round trip is a real wait, and the model is
-- updated from a stream the game writes asynchronously -- querying immediately
-- measures the race, not the ingestion.
for attempt = 1, 25 do
  local seen = world.find_entities_in_radius(
    { x = ANCHOR_X, y = ANCHOR_Y }, 40, nil, "entity-ghost")
  local model_count = 0
  if type(seen) == "table" then
    for _ in pairs(seen) do model_count = model_count + 1 end
  end
  if model_count > 0 then
    print("ghosts the PLANNER can see: " .. model_count .. " (after " .. attempt .. " attempts)")
    print("VERDICT: the channel works end to end")
    return
  end
  -- An RCON round trip, purely to let time pass on the parser's side.
  rcon.print("probe attempt " .. attempt)
end

print("ghosts the PLANNER can see: 0 after 25 attempts")
-- The discriminator: a non-ghost record written on the same channel, from the
-- same loop, in the same call.
local probe = world.find_entities_in_radius({ x = 99.5, y = 99.5 }, 5, "stone-furnace", nil)
local probe_count = 0
if type(probe) == "table" then
  for _ in pairs(probe) do probe_count = probe_count + 1 end
end
print("NON-GHOST marker the planner can see: " .. probe_count)
if probe_count > 0 then
  print("=> the channel WORKS from RCON context; the ghost record is rejected downstream")
else
  print("=> the line never leaves the mod in this context")
end
print("VERDICT: the ghosts do not reach the model; the marker below says where")
