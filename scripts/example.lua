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

print("\n=== Testing Plan API ===")

-- Test task graph building (works in both modes)
plan.group_start("test-walk")
plan.walk(1, {x=5, y=5}, 1.0)
plan.group_end()

-- Generate task graph visualization
local graph = plan.task_graph_mermaid_gantt({1}, "Test Plan")
print("\nTask graph (Mermaid Gantt):")
print(graph)

print("\n=== All tests complete ===")
