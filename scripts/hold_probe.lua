--- Proves the fault-hold path without paying for a real run.
--
-- Everything about a hold that can go wrong -- the savepoint arriving before
-- the pause, the game actually freezing, the release channel, the bound, the
-- record entry -- is independent of *why* the script faulted. So this faults on
-- purpose, immediately, and exercises the whole path in a few seconds plus the
-- hold window. It must never be part of a measured run.
--
--   factorio-bot lua hold_probe.lua --headless --bots 1
--
-- While it holds, from another terminal:
--
--   factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'   -- twice: the
--                                                                 -- number must
--                                                                 -- not move
--   factorio-bot rcon -s localhost -- \
--     "/silent-command remote.call('botbridge','hold_release','stop')"
--
-- Left alone it lapses after HOLD_SECONDS and tears down, which is the case
-- worth watching at least once: the default must be teardown.
include("lib.lua")
include("supervisor.lua")

local HOLD_SECONDS = 60

local run_id = record.start({})
print("hold probe: run " .. tostring(run_id) .. ", holding for up to " .. HOLD_SECONDS .. "s")

local ok, err = pcall(function()
    error("hold probe: a deliberate fault -- nothing is wrong with this game")
end)
if ok then
    error("hold probe: the fault did not raise, so this proves nothing")
end

local held = supervisor.hold_fault(err, {
    index = 0,
    run = run_id,
    timeout = HOLD_SECONDS,
    heartbeat = 10,
})
print("hold probe: released=" .. tostring(held.released)
    .. " held=" .. tostring(held.held)
    .. " savepoint=" .. tostring(held.savepoint)
    .. " index=" .. tostring(held.index))
-- **The freeze is not provable from inside this script**, and an earlier
-- version of this probe pretended it was: it printed a tick from before the
-- hold and one from after and invited the reader to compare them. They are
-- never equal and should not be -- the savepoint costs a few ticks on the way
-- in and the resume costs a couple on the way out -- so a "before/after"
-- reading here is a number that looks like evidence and is not.
--
-- The freeze is proved from OUTSIDE, while the hold is up, by asking the game
-- twice seconds apart:
--
--   factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'
--
-- Measured 2026-09-08: 427 and 427, three seconds apart, and the "still HELD"
-- heartbeats reported 427 at 10 s and at 20 s.

local final_state = held.held and "held" or "crashed"
local id = record.finish(final_state)
print("RUN FINISHED state=" .. final_state .. " id=" .. id)
