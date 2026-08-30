-- Smoke test for the goal planner against a live Factorio world.
--
-- Everything the planner and executor know has been verified against fixtures
-- and mocks. This script is the first thing that asks a real game whether any
-- of it is true. It plans only -- it does not execute -- so it is safe to run
-- against any world.
print("start goal smoke")

local recipe = world.recipe("iron-plate")
assert(recipe ~= nil, "the live world knows no iron-plate recipe")
print("iron-plate recipe present, ingredients: " .. tostring(#recipe.ingredients))

-- Diagnostic: what does the live world think bot inventories are?
-- The planner seeds a synthesised bot with wood + stone-furnace + burner-drill,
-- but only when the player is ABSENT from the world. If the server has already
-- created empty player entries, seeding is skipped and every plan fails on a
-- precondition the fixtures always satisfy.
for _, id in ipairs(world.all_bots and world.all_bots() or {1}) do
  local pl = world.player(id)
  if pl == nil then
    print("bot " .. tostring(id) .. ": NOT PRESENT in world")
  else
    local n = 0
    for k, v in pairs(pl.main_inventory or {}) do
      n = n + 1
      print("bot " .. tostring(id) .. " holds " .. tostring(v) .. " x " .. tostring(k))
    end
    if n == 0 then print("bot " .. tostring(id) .. ": present, EMPTY inventory") end
  end
end

local p = goal.have("iron-plate", 5)
print("expanded ok")

-- Dump the plan BEFORE scheduling: if the world says bot 1 holds a furnace
-- but the scheduler disagrees, the plan's own shape is the place to look.
local dot = goal.graphviz(p)
print("plan actions: " .. tostring(select(2, dot:gsub("%[label=", ""))))
for line in dot:gmatch("[^\n]+") do
  if line:find("stone%-furnace") or line:find("Place") or line:find("Insert") then
    print("  " .. line:gsub("^%s+", ""))
  end
end

-- Schedule over the ACTUAL roster, not a hardcoded 1. Scheduling a plan that
-- was expanded for N bots onto fewer used to fail on a per-bot precondition --
-- goal.schedule now re-expands for the bots asked for, and this is what proves
-- it, because a hardcoded 1 only ever exercises the single-bot path.
local bots = (type(all_bots) == "table" and #all_bots > 0) and #all_bots or 1
print("scheduling over " .. tostring(bots) .. " bot(s)")
local makespan = goal.schedule(p, bots)
print("scheduled: makespan=" .. tostring(makespan))
assert(makespan > 0, "a real plan must take a positive number of ticks")

local dot = goal.graphviz(p)
assert(#dot > 0, "graphviz produced nothing")
print("graphviz bytes: " .. tostring(#dot))

local gantt = goal.gantt(p, "smoke")
assert(#gantt > 0, "gantt produced nothing")
print("gantt bytes: " .. tostring(#gantt))

print("end goal smoke")
