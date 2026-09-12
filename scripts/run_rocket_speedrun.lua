-- Rocket speedrun entry point
pcall(function() rcon.cheat_all_technologies() end)
record.start()
include("rocket_policy.lua")
include("rocket_speedrun.lua")
rocket_speedrun.run({ bots = {1,2,3,4} })
