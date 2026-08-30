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

print("\n=== Testing Per-Bot Movement ===")

-- Was one `plan.walk` per bot inside a `plan.group_start`/`plan.group_end`
-- bracket, followed by `plan.task_graph_mermaid_gantt`. Pinning a *named* bot
-- to a *literal* destination is not a goal: `goal.have` deliberately lets the
-- planner choose who does what, and there is no way to force bot N to a point.
-- So this is an immediate `rcon.move` per bot now, and the gantt chart goes
-- with the plan it used to render -- there is no schedule here to chart.
local destinations = {
    { x = 0, y = -10 },  -- north
    { x = 0, y = 10 },   -- south
    { x = 10, y = 0 },   -- east
    { x = -10, y = 0 },  -- west
}

for i, bot_id in ipairs(all_bots) do
    -- As before, only the first four bots get a destination.
    local destination = destinations[i]
    if destination then
        rcon.move(bot_id, destination, 1.0)
        print("Bot " .. bot_id .. " moving to (" .. destination.x .. ", " .. destination.y .. ")")
    end
end

print("\n=== Multi-Client Test Complete ===")
print("Tested " .. #all_bots .. " bots successfully")
