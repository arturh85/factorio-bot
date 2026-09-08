--- Proves the exposure record end to end, without a measured run.
--
--   factorio-bot lua exposure_probe.lua --headless --bots 1
--
-- Measured 2026-09-08 (run-1788901356-40092): observed=7 ours=4 foreign=3 --
-- a tick read, a cheat attempt and the release itself. Its control is
-- `exposure_control.lua`, whose exposure.json reads `holds: []`.
--
-- Deliberately does NOT go through `supervisor.hold_fault`: the workspace's
-- copy of `supervisor.lua` is stale relative to the checkout and shared with
-- whatever else is running here, so this calls `rcon.hold` and
-- `record.exposure` directly. The supervisor's wiring of the same two calls is
-- exercised by the Rust `supervisor_lib` tests.
--
-- While it holds, from a second terminal:
--
--   factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'
--   factorio-bot rcon -s localhost -- \
--     "/silent-command remote.call('botbridge','hold_release','stop')"
include("lib.lua")

local HOLD_SECONDS = 120

local run_id = record.start({})
print("census probe: run " .. tostring(run_id))
record.savepoint(0)

local held = rcon.hold({
    error = "census probe: a deliberate fault -- nothing is wrong with this game",
    run = run_id,
    timeout = HOLD_SECONDS,
    heartbeat = 30,
})
print("census probe: released=" .. tostring(held.released)
    .. " observed=" .. tostring(held.console_commands_observed)
    .. " ours=" .. tostring(held.console_commands_ours)
    .. " foreign=" .. tostring(held.foreign_console_commands)
    .. " command_used=" .. tostring(held.console_command_used))

record.exposure(held, "census probe: a deliberate fault")
print("RUN FINISHED state=held id=" .. record.finish("held"))
