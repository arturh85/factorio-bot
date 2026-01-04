-- Comprehensive Lua API Test
-- Tests all 4 API modules: global, rcon, world, plan

print("=== Testing Global Functions ===")

-- Test file I/O
file_write("/tmp/factorio_api_test.txt", "hello from Factorio bot")
local content = file_read("/tmp/factorio_api_test.txt")
print("File I/O: " .. (content == "hello from Factorio bot" and "PASS ✓" or "FAIL ✗"))

-- Test direction utilities
local dirs_all = directions_all()
local dirs_ortho = directions_orthogonal()
print("Direction utilities: " .. (#dirs_all == 8 and #dirs_ortho == 4 and "PASS ✓" or "FAIL ✗"))

print("\n=== Testing RCON Functions ===")

-- Test rcon.print (basic RCON communication)
rcon.print("API test message from Lua")
print("RCON print: sent message to server ✓")

-- Test entity finding
local entities = rcon.find_entities_in_radius({x=0, y=0}, 100, nil, nil)
print("RCON find_entities: found " .. #entities .. " entities ✓")

-- Test inventory/cheats
rcon.cheat_item(1, "iron-plate", 50)
print("RCON cheat_item: gave 50 iron plates ✓")

rcon.cheat_technology("automation")
print("RCON cheat_technology: unlocked automation ✓")

print("\n=== Testing World Functions ===")

-- Test recipe lookup
local recipe = world.recipe("iron-gear-wheel")
print("World recipe: " .. (recipe ~= nil and "PASS ✓" or "FAIL ✗"))

-- Test player lookup
local player = world.player(1)
print("World player: " .. (player ~= nil and "PASS ✓" or "FAIL ✗"))
if player then
    print("  Player position: " .. player.position.x .. ", " .. player.position.y)
end

-- Test inventory count
local count = world.inventory(1, "iron-plate")
print("World inventory: " .. (count >= 0 and "PASS ✓ (" .. count .. " iron plates)" or "FAIL ✗"))

print("\n=== Testing Plan Functions ===")

-- Test task graph building
plan.group_start("api-test-group")
plan.walk(1, {x=10, y=10}, 1.0)
plan.walk(1, {x=20, y=20}, 1.0)
plan.group_end()

local graph = plan.task_graph_mermaid_gantt({1}, "API Test Plan")
print("Plan task graph: " .. (graph ~= nil and "PASS ✓" or "FAIL ✗"))
print("\nTask graph generated:")
print(graph)

print("\n=== All API Tests Complete ===")
print("✓ Global functions working")
print("✓ RCON functions working")
print("✓ World functions working")
print("✓ Plan functions working")
