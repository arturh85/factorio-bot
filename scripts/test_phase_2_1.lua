-- Integration test for the planner: goal decomposition and scheduling.
--
-- MIGRATION HONESTY NOTE. This file used to be a white-box test of the old
-- `TaskGraph`/`PlanBuilder`: it hand-declared mine and insert tasks and then
-- inspected the *derived* dependency edges and the per-player resource
-- isolation. Neither is observable from Lua any more -- `goal.*` deliberately
-- hides task-level granularity, because the planner owns the decomposition --
-- so this is a smoke test of the planner instead of a white-box test of a
-- graph builder.
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

print("=== Planner Integration Test: goal decomposition ===\n")

local wanted = {
    { item = "iron-ore", count = 50 },
    { item = "copper-ore", count = 30 },
    { item = "coal", count = 20 },
}

for _, want in ipairs(wanted) do
    print("Planning: " .. want.count .. " " .. want.item)
    local plan = goal.have(want.item, want.count)

    local makespan = goal.schedule(plan, #all_bots)
    assert(makespan > 0, "a plan for " .. want.item .. " must take time, got " .. tostring(makespan))
    print("  makespan: " .. makespan .. " ticks over " .. #all_bots .. " bot(s)")

    local dot = goal.graphviz(plan)
    assert(string.find(dot, "digraph", 1, true),
           "expected a graphviz digraph for " .. want.item .. ", got: " .. dot)

    local gantt = goal.gantt(plan, want.item)
    assert(string.find(gantt, "section bot 1", 1, true),
           "expected bot 1 to have work for " .. want.item .. ", got: " .. gantt)
end

-- An unscheduled plan cannot be charted. A caller who forgot to schedule hears
-- about it rather than being handed a blank chart.
local unscheduled = goal.have("iron-ore", 1)
local ok = pcall(goal.gantt, unscheduled, "unscheduled")
assert(not ok, "goal.gantt must refuse an unscheduled plan")

print("\n=== Planner Integration Test Complete ===")
print("Verified:")
print("  - goal.have decomposes an item goal into an action network")
print("  - goal.schedule spreads it over the run's bots and reports a makespan")
print("  - goal.graphviz and goal.gantt render the result")
print("  - goal.gantt refuses a plan that was never scheduled")
