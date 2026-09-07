-- What does one bot cost in tick rate?
--
-- CLAUDE.md records "eight bots still held 242 of 300 requested tps" and
-- attributes it to the mod polling every bot every tick. That is ONE data point
-- with no baseline: it says where eight bots ended up, not what one costs,
-- because there is nothing to divide by. This runs the same fixed span of GAME
-- TICKS at 1, 4 and 8 bots and does nothing else, so the difference between the
-- runs is the polling and nothing else.
--
-- Game speed is set far above what the machine can deliver, deliberately: at a
-- speed the box can keep up with, every roster would report the requested rate
-- and the measurement would show nothing. Unbounded demand makes the delivered
-- rate the machine's actual ceiling.
--
-- DISCLOSED: the loop polls `rcon.game_tick()`, and each poll is an RCON round
-- trip, so some of the wall time is this probe rather than the game. It is the
-- same probe in all three runs, so the COMPARISON holds even though the
-- absolute tps is depressed. The poll count is printed so that assumption can
-- be checked rather than trusted.
print("start tickrate probe")

-- Report the roster the GAME actually has, not the one the flag asked for.
-- CLAUDE.md: "a one-bot run misread as a four-bot regression cost a good commit
-- a revert." A 1/4/8 comparison where 8 silently came up as 4 is worse than no
-- comparison, because the shape still looks like a trend.
local players = rcon.players()
print("BOTS_PRESENT=" .. tostring((type(players) == "table") and #players or -1))

local t0 = rcon.game_tick()
local SPAN = tonumber(file_read("tickrate_span.txt")) or 6000
print("t0=" .. tostring(t0) .. " span=" .. SPAN)
local t, polls = t0, 0
while type(t) == "number" and type(t0) == "number" and (t - t0) < SPAN do
  t = rcon.game_tick()
  polls = polls + 1
end
print("TICKS=" .. tostring(t - t0) .. " POLLS=" .. polls)
print("end tickrate probe")
