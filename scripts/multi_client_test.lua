-- multi_client_test.lua: Test multi-client setup
-- Run with: just lua multi_client_test.lua -c 2
include("lib.lua")

print("=== Multi-Client Test ===")
print("Available bots: " .. #all_bots)

for i, bot_id in ipairs(all_bots) do
    print("Bot " .. i .. " has player_id: " .. bot_id)
end

print("\n=== Testing Individual Bot Control ===")

-- Test each bot independently
for i, bot_id in ipairs(all_bots) do
    local player = world.player(bot_id)
    if player and player.position then
        print("Bot " .. bot_id .. " position: " .. player.position.x .. ", " .. player.position.y)
    else
        print("Bot " .. bot_id .. " - no player data (may be in connect mode)")
    end
end

print("\n=== Testing RCON Commands to Individual Bots ===")

-- Send a message identifying each bot
for i, bot_id in ipairs(all_bots) do
    rcon.print("Testing bot " .. bot_id .. " via RCON")
end

print("\n=== Testing Plan with Multiple Bots ===")

-- Create a simple plan with different tasks for each bot
plan.group_start("multi-bot-test")

if #all_bots >= 1 then
    -- Bot 1: walk north
    plan.walk(all_bots[1], {x=0, y=-10}, 1.0)
    print("Bot " .. all_bots[1] .. " assigned: walk to (0, -10)")
end

if #all_bots >= 2 then
    -- Bot 2: walk south
    plan.walk(all_bots[2], {x=0, y=10}, 1.0)
    print("Bot " .. all_bots[2] .. " assigned: walk to (0, 10)")
end

if #all_bots >= 3 then
    -- Bot 3: walk east
    plan.walk(all_bots[3], {x=10, y=0}, 1.0)
    print("Bot " .. all_bots[3] .. " assigned: walk to (10, 0)")
end

if #all_bots >= 4 then
    -- Bot 4: walk west
    plan.walk(all_bots[4], {x=-10, y=0}, 1.0)
    print("Bot " .. all_bots[4] .. " assigned: walk to (-10, 0)")
end

plan.group_end()

-- Generate visualization showing all bots
local bot_ids = {}
for i, bot_id in ipairs(all_bots) do
    table.insert(bot_ids, bot_id)
end

local graph = plan.task_graph_mermaid_gantt(bot_ids, "Multi-Bot Test Plan")
print("\nTask graph:")
print(graph)

print("\n=== Multi-Client Test Complete ===")
print("Tested " .. #all_bots .. " bots successfully")
