-- Integration test for the planner: goal decomposition and scheduling.
--
-- MIGRATION HONESTY NOTE. This file used to be a white-box test of the old
-- `TaskGraph`/`PlanBuilder`: it hand-declared mine and insert tasks and then
-- inspected the *derived* dependency edges and the per-player resource
-- isolation. Neither is observable from Lua any more: the planner still owns
-- decomposition end to end, and a `PlanValue` exposes its *result* -- steps,
-- counts, per-bot slices, via `plan.steps`/`plan:count`/`plan:find` -- not the
-- internal dependency graph or the isolation bookkeeping that produced it. So
-- this is a smoke test of the planner's output, not a white-box test of a
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
    local plan = goal.plan(goal.have(want.item, want.count))

    assert(plan.makespan > 0, "a plan for " .. want.item .. " must take time, got " .. tostring(plan.makespan))
    print("  makespan: " .. plan.makespan .. " ticks over " .. #all_bots .. " bot(s)")

    local dot = plan:graphviz()
    assert(string.find(dot, "digraph", 1, true),
           "expected a graphviz digraph for " .. want.item .. ", got: " .. dot)

    local gantt = plan:gantt(want.item)
    assert(string.find(gantt, "section bot 1", 1, true),
           "expected bot 1 to have work for " .. want.item .. ", got: " .. gantt)
end

-- The old assertion here checked that goal.gantt refused an *unscheduled*
-- plan. That state no longer exists: goal.plan expands and schedules in one
-- call, so a PlanValue coming out of it is always scheduled -- there is
-- nothing left to forget. What still carries weight in this model is that
-- goal.plan itself refuses a goal naming an item no recipe produces, and
-- says which one.
local unknown = goal.have("not-a-real-item", 1)
local ok, err = pcall(goal.plan, unknown)
assert(not ok, "goal.plan must refuse an unknown item")
assert(tostring(err):find("not%-a%-real%-item"), "the error must name it: " .. tostring(err))

print("\n=== Planner Integration Test Complete ===")
print("Verified:")
print("  - goal.have builds a goal value; goal.plan decomposes and schedules it")
print("  - goal.plan spreads the work over the run's bots and reports a makespan")
print("  - plan:graphviz() and plan:gantt() render the result")
print("  - goal.plan refuses a goal naming an unknown item, and names it")
