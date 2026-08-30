-- example.lua: Test all API categories
-- Works in both full mode (just lua) and connect mode (just lua-connect)
include("lib.lua")

print("=== Testing RCON API (works in connect mode) ===")

-- Test rcon.print
rcon.print("Hello from Lua script!")
print("Sent message to server console")

-- Test rcon.find_entities_in_radius (live server query)
local live_entities = rcon.find_entities_in_radius({x=0, y=0}, 50, nil, nil)
print("Live server query: " .. #live_entities .. " entities within radius 50")

print("\n=== Testing World API (only works in full mode) ===")

-- Test world.player (may be empty in connect mode)
local player = world.player(1)
if player and player.position then
    print("Player 1 found:")
    print("  Position: " .. player.position.x .. ", " .. player.position.y)
    print("  Reach distance: " .. tostring(player.reach_distance))
else
    print("SKIP: No player data (expected in --connect mode)")
end

-- Test world.recipe (may be empty in connect mode)
local recipe = world.recipe("iron-plate")
if recipe then
    print("Iron plate recipe found")
    dump(recipe, "iron-plate recipe")
else
    print("SKIP: No recipe data (expected in --connect mode)")
end

-- Test world.find_entities_in_radius (local cached data)
local nearby = world.find_entities_in_radius({x=0, y=0}, 100, nil, nil)
if #nearby > 0 then
    print("World cache: " .. #nearby .. " entities within radius 100")
else
    print("SKIP: No cached world data (expected in --connect mode)")
end

print("\n=== Testing Goal API ===")

-- Was `plan.walk(1, {x=5,y=5}, 1.0)` inside a `plan.group_start`/`plan.group_end`
-- bracket, then `plan.task_graph_mermaid_gantt`. A bare walk is not a goal in
-- the new model, so it is an immediate rcon command; the bracketing is gone
-- with no replacement, because the planner derives grouping from the goal.
rcon.move(1, {x=5, y=5}, 1.0)
print("Moved bot 1 towards (5, 5)")

-- Planning needs the cached world (recipes, resources), which connect mode does
-- not have -- hence the pcall. `goal.have` itself is pure and cannot fail on
-- that account; `goal.plan` is what expands and schedules against the world,
-- so that is the call this guards now.
local ok, plan = pcall(goal.plan, goal.have("iron-plate", 1))
if ok then
    print("\nSchedule (Mermaid Gantt):")
    print(plan:gantt("Test Plan"))
else
    print("SKIP: goal planning needs world data (expected in --connect mode): " .. tostring(plan))
end

print("\n=== All tests complete ===")
