-- Live check of the per-bot (`bot-N`) and `area` cameras.
--
-- Everything worth proving here is a file that does or does not appear on
-- disk, so the evidence is a directory listing taken from outside while this
-- runs, not an assertion in here. This script's job is to reach each state,
-- announce it on stdout, and then hold still long enough for a watcher to see
-- it. The MARK lines are what the watcher synchronises on.
--
-- Run it with two clients to see the whole thing:
--
--     factorio-bot lua camera_check.lua -c 2
--
-- Phase A puts the bots far apart, so the `area` camera has a real bounding
-- box rather than a point and its zoom is visibly not the follow camera's.
-- Phase B is the window in which the watcher kills one client's process: from
-- that tick on, that bot's `bot-N` camera must write nothing while every other
-- camera keeps writing. Absence per camera is the property; a substituted or
-- repeated frame would destroy it.
--
-- With `-c 0` there is no connected bot at all, and then the honest output is
-- no frames whatsoever -- not a frame of the map origin. The walks below all
-- refuse in that case, which is why every one of them is wrapped.

local RUN_ID = "job-cameras-1c7f0e"

print("MARK bots=" .. #all_bots)
for i, bot in ipairs(all_bots) do
    print("MARK bot " .. i .. " player_id=" .. tostring(bot))
end

-- Burns real game time, which is what gives the cameras something to
-- photograph and what advances `game.tick` between frames. Blocking, and
-- refusals are expected when the bot it names is gone -- that is the case
-- under test, not an error.
local function walk(bot, x, y)
    local ok, err = pcall(function() rcon.move(bot, { x = x, y = y }, 2.0) end)
    if not ok then
        print("  walk refused for bot " .. tostring(bot) .. ": " .. tostring(err))
    end
end

print("MARK start " .. RUN_ID)
-- `true` explicitly: screenshot cameras are retired and a capture registers
-- none by default, which for this script would mean checking nothing at all.
local tick = rcon.frame_capture_start(RUN_ID, true)
print("MARK started tick=" .. tostring(tick))

-- Phase A: drive the bots apart and back, so the area camera's box changes
-- size between frames.
for lap = 1, 3 do
    for i, bot in ipairs(all_bots) do
        local sign = (i % 2 == 0) and 1 or -1
        walk(bot, sign * 30, sign * 12)
        walk(bot, sign * 6, 0)
    end
    print("MARK phase-A-lap " .. lap)
end
print("MARK phase-A-settled")

-- Phase B: the watcher kills one client here. Everything after this line has
-- to keep working for the bots that remain.
print("MARK phase-B-kill-window")
for lap = 1, 10 do
    for i, bot in ipairs(all_bots) do
        local sign = (i % 2 == 0) and 1 or -1
        walk(bot, sign * 20, 0)
        walk(bot, sign * 4, sign * 8)
    end
    print("MARK phase-B-lap " .. lap)
end
print("MARK phase-B-settled")

print("MARK stop")
tick = rcon.frame_capture_stop()
print("MARK stopped tick=" .. tostring(tick))
print("MARK done")
