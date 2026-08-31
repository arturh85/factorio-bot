-- Live check of the frame-capture run id sidecar.
--
-- Everything this proves is filesystem behaviour inside a running game, so the
-- evidence is a directory listing taken from outside while this runs, not an
-- assertion in here. The script's job is to reach each state and then hold
-- still long enough for a watcher to see it.
--
-- Decoys are seeded before the game starts: a stale `run.json`, a file no
-- frame could ever be named, and a frame-shaped name at a tick this run will
-- never reach. If any survives, "wiped" meant "overwritten".

local ALPHA = "job-alpha-4f21c9"
local BETA  = "job-beta-7ae30d"

local bot = all_bots[1]
print("bot: " .. tostring(bot))

-- Burns game time by walking, which is also what gives the follow camera
-- something to photograph. Blocking calls, so this is real elapsed ticks.
local function walk_a_while(laps)
    for _ = 1, laps do
        for _, dest in ipairs({ { x = 20, y = 0 }, { x = -20, y = 0 } }) do
            local ok, err = pcall(function() rcon.move(bot, dest, 2.0) end)
            if not ok then print("  walk refused: " .. tostring(err)) end
        end
    end
end

print("MARK phase-A-start-with-id " .. ALPHA)
local tick = rcon.frame_capture_start(ALPHA)
print("MARK phase-A-started tick=" .. tostring(tick))
walk_a_while(3)
print("MARK phase-A-settled")
walk_a_while(1)

-- The case most likely to be got wrong: no id must leave no run.json, and in
-- particular not ALPHA's. There is no argument at all here, not an empty one.
print("MARK phase-B-start-without-id")
tick = rcon.frame_capture_start()
print("MARK phase-B-started tick=" .. tostring(tick))
walk_a_while(3)
print("MARK phase-B-settled")
walk_a_while(1)

print("MARK phase-C-start-with-different-id " .. BETA)
tick = rcon.frame_capture_start(BETA)
print("MARK phase-C-started tick=" .. tostring(tick))
walk_a_while(3)
print("MARK phase-C-settled")
walk_a_while(1)

print("MARK stop")
tick = rcon.frame_capture_stop()
print("MARK stopped tick=" .. tostring(tick))
print("MARK done")
