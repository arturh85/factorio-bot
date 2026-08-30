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

local g = goal.have("iron-plate", 5)
print("goal value built")

-- Expansion and scheduling used to be two calls -- `goal.have` expanded the
-- network, then `goal.schedule(plan, n)` re-expanded it for whichever bot
-- count was asked for, which is exactly how a plan built for N bots could be
-- scheduled onto fewer and fail on a per-bot precondition (the smoke test
-- this file replaced hit that live). `goal.plan` does both at once, against
-- one roster, so that mismatch is no longer something a script can even
-- write: there is no unscheduled plan to inspect before this call, only a
-- scheduled one after it.
local bots = (type(all_bots) == "table" and #all_bots > 0) and #all_bots or 1
print("planning over " .. tostring(bots) .. " bot(s)")
local p = goal.plan(g)
print("scheduled: makespan=" .. tostring(p.makespan))
assert(p.makespan > 0, "a real plan must take a positive number of ticks")

local dot = p:graphviz()
print("plan actions: " .. tostring(select(2, dot:gsub("%[label=", ""))))
for line in dot:gmatch("[^\n]+") do
  if line:find("stone%-furnace") or line:find("Place") or line:find("Insert") then
    print("  " .. line:gsub("^%s+", ""))
  end
end
assert(#dot > 0, "graphviz produced nothing")
print("graphviz bytes: " .. tostring(#dot))

local gantt = p:gantt("smoke")
assert(#gantt > 0, "gantt produced nothing")
print("gantt bytes: " .. tostring(#gantt))

-- The capability this whole increment buys: a plan's shape can be asserted on
-- directly, with no running game involved.
local p2 = goal.plan(goal.all { goal.have("iron-plate", 5), goal.researched("automation") })
assert(p2:count { kind = "place", entity = "stone-furnace" } <= #p2.bots,
       "no more furnaces than bots")
for _, s in ipairs(p2:find { kind = "mine" }) do
    assert(s.count > 0, "a mine step for nothing is a planner bug")
end

print("end goal smoke")
