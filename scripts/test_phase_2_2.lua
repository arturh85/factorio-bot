-- Integration test for the executor: running a scheduled plan against a live
-- game. Needs a connected Factorio server -- `goal.start` has nothing to
-- drive without one.
--
-- MIGRATION HONESTY NOTE. This file used to assert on the old multi-bot
-- executor's internals: that all six task types (Mine/Walk/Craft/Place/Insert/
-- Remove) existed, that each task moved Planned -> Running -> Success/Failed,
-- and that dependency gating held per task. None of that is observable from Lua
-- any more: `run:progress()`/`run:wait()` report aggregate counts across the
-- whole run (pending/running/success/failed/done), not a per-task-type
-- breakdown.
--
-- It also never ran. Every version of this script called
-- `plan.insert_into_inventory`, which was never bound in Rust, so execution
-- stopped at that line with "attempt to call a nil value". Nothing after it has
-- ever been observed to work, and this port is from the script's evident intent
-- rather than from preserved behaviour: no behaviour parity is claimed here,
-- because there was none to preserve.
--
-- The `plan.group_start`/`plan.group_end` bracketing is gone with no
-- replacement; the planner derives grouping from the goal decomposition.

print("=== Executor Integration Test ===\n")

print("Phase 1: planning...")
local plan = goal.plan(goal.have("iron-plate", 10))
assert(plan.makespan > 0, "a plan must take time, got " .. tostring(plan.makespan))
print("  scheduled over " .. #all_bots .. " bot(s), makespan " .. plan.makespan .. " ticks")

print("\nPhase 2: executing...")
-- Execution is non-blocking now: `goal.start` returns a run value straight
-- away and the bots keep working while the script does something else.
local run = goal.start(plan)

local progress = run:progress()
local total = progress.pending + progress.running + progress.success + progress.failed
assert(total > 0, "the run must cover at least one action")
print("  started: " .. total .. " actions, " .. progress.pending .. " pending")

print("\nPhase 3: waiting for completion...")
local final = run:wait()
assert(final.done, "run:wait() must only return once the run is over")
print("  success: " .. final.success)
print("  failed:  " .. final.failed)
print("  pending: " .. final.pending .. " (a bot that fails abandons the rest of its work)")

-- The four counts always cover every action in the plan, both before and after
-- the run: that is what makes a partial run legible rather than a mystery.
local final_total = final.pending + final.running + final.success + final.failed
assert(final_total == total,
       "progress must account for every action; started with " .. total .. ", ended with " .. final_total)

if final.failed > 0 then
    print("\n" .. final.failed .. " action(s) failed -- see the log above for why")
else
    print("\nAll actions succeeded")
end

print("\n=== Executor Integration Test Complete ===")
