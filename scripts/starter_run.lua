-- Task 4, retargeted: StarterSteamEngineBoiler (rcontest.lua) -- 6 entities:
-- two steam-engine, two small-electric-pole, one boiler, one pipe. Small
-- footprint (fits in an 11-tile span), chosen because it does not need
-- siting the way MinerLine's 21-tile belt run does (see
-- docs/superpowers/notes/2026-09-05-first-block-built.md). Same everything
-- else: own ports/workspace, headless 4 bots at 5x, materials cheated in
-- and disclosed, goal.built + goal.run for real.
print("start starter run")

-- rcontest.lua's blueprints.StarterSteamEngineBoiler, copied rather than
-- `include`d for the same reason as block_run.lua: rcontest.lua fires
-- rcon.cheat_* at its own top level and this run controls its own cheats.
local STARTER = "0eNqdkdEKwjAMRf8lz504nRv0V0Rkm0ECbVrWThxj/242RQXrgz6VhHtPLr0jNKZH3xFH0CNQ6ziA3o8Q6My1mXdx8AgaKKIFBVzbeQoRa5shn4kRJgXEJ7yCzqeDAuRIkfDOWYbhyL1tsBNBmqDAuyAmx/NFAWUiHOQppkl9QDYviK2NydBgGztqM+9MgvVAlSnU9rc8eYpR/BMnSdo9SY0jI5tvOYrVLuUvn35P/utpsUpLS5/6rX4FF+zCIq6qbZ5X1brcyP/fAHsdtKc="
-- Bare dirt, walkable, no ore needed -- this block needs none. The same spot
-- MinerLine's attempt 1 found refused for drills (which DO need ore); a
-- boiler/engine/pole/pipe block has no such requirement.
local ANCHOR = { x = 10, y = 10 }

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

-- ---------------------------------------------------------------- CHEATS --
print("CHEAT: giving every bot the full material set (2 steam-engine, "
  .. "2 small-electric-pole, 1 boiler, 1 pipe) -- gathering is not what this run tests")
for _, b in ipairs(bots) do
  rcon.cheat_item(b, "steam-engine", 2)
  rcon.cheat_item(b, "small-electric-pole", 2)
  rcon.cheat_item(b, "boiler", 1)
  rcon.cheat_item(b, "pipe", 1)
end

local run_id = record.start({ video = false })
print("recording run " .. run_id)

local plan = goal.plan(goal.built(STARTER, ANCHOR))
print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

local placed_plan = plan:count { kind = "place" }
print("planned placements: " .. tostring(placed_plan))

local steps = {}
local per_bot_steps = {}
local expected = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s", s.entity, s.pos.x, s.pos.y, tostring(s.direction))
    expected[#expected + 1] = { entity = s.entity, pos = s.pos, direction = s.direction }
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
  per_bot_steps[s.bot] = (per_bot_steps[s.bot] or 0) + 1
end
for _, b in ipairs(bots) do
  print("bot " .. b .. ": " .. tostring(per_bot_steps[b] or 0) .. " planned step(s)")
end
print("expected placements captured from the plan: " .. tostring(#expected))
for _, e in ipairs(expected) do
  print(string.format("  expect %s @ (%.2f,%.2f) dir=%s", e.entity, e.pos.x, e.pos.y, tostring(e.direction)))
end

print("dispatching...")
local tick_before = rcon.game_tick()
local obs = goal.run(plan)
local tick_after = rcon.game_tick()
print(string.format("run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before), tostring(tick_after),
  (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
local n_teleports = record.teleports()
local n_refusals = record.refusals()
local n_enclosures = record.enclosures()
print(string.format("recorded: %d action events, %d walk events, %d teleports, %d refusals, %d enclosures",
  n_actions, n_walks, n_teleports, n_refusals, n_enclosures))
if n_enclosures > 0 then print("WALLED IN: see record.enclosures() above") end

record.milestone_started(1, "StarterSteamEngineBoiler (6 entities) built at " .. ANCHOR.x .. "," .. ANCHOR.y)
if obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 then
  record.milestone_satisfied(1, 1, "plan_empty")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
end

print(string.format("done=%s success=%s failed=%s lost=%s pending=%s running=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
print("first_error: " .. tostring(obs.first_error))
if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
  local fs = obs:failures()
  for i = 1, #fs do
    print(string.format("failure[%d] id=%s status=%s attempts=%s error=%s",
      i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts), tostring(fs[i].error)))
  end
end

-- ------------------------------------------------ read the world back --
print("---- reading the world back: position + direction, per entity ----")
local NAMES = { "steam-engine", "small-electric-pole", "boiler", "pipe" }
local actual_by_key = {}
local total_found = 0
for _, name in ipairs(NAMES) do
  local found = rcon.find_entities_in_radius(ANCHOR, 15, name)
  print(name .. ": " .. tostring(#found) .. " found near anchor")
  for _, e in ipairs(found or {}) do
    total_found = total_found + 1
    local key = string.format("%.1f,%.1f", e.position.x, e.position.y)
    actual_by_key[key] = { name = e.name, position = e.position, direction = e.direction }
    print(string.format("  GAME: %s @ (%.2f,%.2f) dir=%s", e.name, e.position.x, e.position.y, tostring(e.direction)))
  end
end
print("total entities found near anchor: " .. tostring(total_found) .. " (want 6)")

local matched, wrong_dir, missing = 0, 0, 0
for _, exp in ipairs(expected) do
  local key = string.format("%.1f,%.1f", exp.pos.x, exp.pos.y)
  local act = actual_by_key[key]
  if act == nil then
    missing = missing + 1
    print(string.format("MISSING: %s expected @ (%s) dir=%s -- nothing found there",
      exp.entity, key, tostring(exp.direction)))
  elseif act.name ~= exp.entity then
    missing = missing + 1
    print(string.format("WRONG ENTITY: expected %s @ (%s), found %s", exp.entity, key, act.name))
  elseif tostring(act.direction) ~= tostring(exp.direction) then
    wrong_dir = wrong_dir + 1
    print(string.format("WRONG DIRECTION: %s @ (%s) expected dir=%s, game reports dir=%s",
      exp.entity, key, tostring(exp.direction), tostring(act.direction)))
  else
    matched = matched + 1
  end
end
print(string.format("VERDICT: %d/%d matched position+direction, %d wrong direction, %d missing/wrong entity",
  matched, #expected, wrong_dir, missing))

local id = record.finish(obs.done and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("end starter run")
