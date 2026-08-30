-- Does a world read over RCON alone -- no stdout, no process we own -- carry
-- enough to plan with?
--
-- Run against a Factorio server this program did not start:
--
--   factorio-bot lua attach_smoke.lua --connect --clients 0 --bots 1
--
-- Every assertion here fails on the empty `FactorioWorld` that `--connect`
-- used to hand the planner, so passing it is evidence that the snapshot
-- arrived and not merely that the script ran.
print("start attach smoke")

-- 1. Recipes. Without them `goal.have` cannot decompose anything.
local recipe = world.recipe("iron-plate")
assert(recipe ~= nil, "the attached world knows no iron-plate recipe")
print("iron-plate recipe: category=" .. tostring(recipe.category)
  .. " energy=" .. tostring(recipe.energy))

-- 2. Entity prototypes. `PlanState::collision_area` reads collision_box from
--    them; with no prototype it answers None and every placement is refused.
--    Asked through find_free_resource_rect, which needs both the prototype
--    table and the resource half of the entity graph to answer at all.
local rect = world.find_free_resource_rect("iron-ore", 2, 2, {x = 0, y = 0})
local rect_w = rect.right_bottom.x - rect.left_top.x
print("free 2x2 iron-ore rect: "
  .. tostring(rect.left_top.x) .. "," .. tostring(rect.left_top.y) .. " -> "
  .. tostring(rect.right_bottom.x) .. "," .. tostring(rect.right_bottom.y))
assert(rect_w > 0, "no iron-ore patch in the attached world: the entity graph is empty")

-- 3. The rest of the resource layer, not just the one ore. A world that had
--    merely been handed a hard-coded iron patch would not know these.
--    (`find_entities_in_radius` is deliberately not used: it queries the
--    entity *tree*, which EntityGraph::add only fills with machines, and a
--    freeplay map that nobody has built on has none -- so it reads empty
--    whether the snapshot arrived or not, and proves nothing either way.)
for _, ore in ipairs({"coal", "stone", "copper-ore"}) do
  local r = world.find_free_resource_rect(ore, 2, 2, {x = 0, y = 0})
  local w = r.right_bottom.x - r.left_top.x
  print("free 2x2 " .. ore .. " rect at "
    .. tostring(r.left_top.x) .. "," .. tostring(r.left_top.y)
    .. " (width " .. tostring(w) .. ")")
  assert(w > 0, "no " .. ore .. " patch in the attached world")
end

-- 4. The whole point: plan and schedule.
local plan = goal.have("iron-plate", 5)
local makespan = goal.schedule(plan, 1)
print("scheduled: makespan=" .. tostring(makespan))
assert(makespan > 0, "a plan built from a snapshot must take a positive number of ticks")

print("end attach smoke")
