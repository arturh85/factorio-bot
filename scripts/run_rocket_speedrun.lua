-- Rocket speedrun entry point
-- Force-complete all technologies for headless mode (trigger technologies don't fire without character bots)
pcall(function() rcon.cheat_all_technologies() end)

-- Clear area around spawn to make room for stage 5+ building
-- The bots clutter the area with drills, furnaces, belts during stages 1-4
-- making pathfinding impossible for later stages
pcall(function()
    -- Move bots out of the way first
    for _, pid in ipairs({1,2,3,4}) do
        local ok, player_info = pcall(function()
            return rcon.player_info(pid)
        end)
        if ok and player_info and player_info.entity then
            -- Teleport bot to a clear spot
            rcon.move(pid, {x = 0, y = -40}, 0)
        end
    end
end)

record.start()
include("rocket_policy.lua")
include("rocket_speedrun.lua")
rocket_speedrun.run({ bots = {1,2,3,4} })
