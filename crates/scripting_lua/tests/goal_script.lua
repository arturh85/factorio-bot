-- Exercises the planning half of the goal.* API against the fixture world.
--
-- goal.execute, goal.progress and goal.wait are deliberately absent: they need
-- a live Factorio server. They are covered in Rust, in globals/goal.rs, against
-- a stub actuator.

local plan = goal.have("automation-science-pack", 10)

-- A gantt chart of an unscheduled plan is an error, not an empty string: a
-- caller who forgot to schedule should hear about it rather than be handed a
-- blank chart.
local ok, err = pcall(goal.gantt, plan, "red science")
assert(not ok, "goal.gantt must refuse an unscheduled plan")
assert(string.find(tostring(err), "has not been scheduled", 1, true),
       "goal.gantt should say why it refused, got: " .. tostring(err))

local makespan = goal.schedule(plan, 4)
assert(makespan > 0, "expected a positive makespan")

local dot = goal.graphviz(plan)
assert(#dot > 0, "expected graphviz output")
assert(string.find(dot, "digraph", 1, true), "expected a graphviz digraph, got: " .. dot)

local gantt = goal.gantt(plan, "red science")
assert(string.find(gantt, "gantt", 1, true), "expected a mermaid gantt chart")
assert(string.find(gantt, "red science", 1, true), "the title must reach the chart")
-- One section per bot that got work; four bots were asked for, so more than one
-- bot must appear or the plan was never actually spread.
assert(string.find(gantt, "section bot 1", 1, true), "expected bot 1 to have a section")
assert(string.find(gantt, "section bot 2", 1, true), "expected the work to be spread over bots")

-- Research has no method in the planner yet. Two things are asserted here. That
-- the failure arrives as a catchable Lua error rather than a panic, which under
-- `panic = "abort"` would take the whole server down. And that it says research
-- is *unimplemented*: the planner's own "no method can satisfy goal" reads as
-- "that technology is unreachable", which sends a user hunting prerequisites
-- for a feature that was never built.
local ok, err = pcall(goal.researched, "automation")
assert(not ok, "goal.researched cannot succeed until a research method exists")
assert(string.find(tostring(err), "not implemented yet", 1, true),
       "the error must say research is unimplemented, got: " .. tostring(err))
assert(string.find(tostring(err), "automation", 1, true),
       "the error must still name the technology asked for, got: " .. tostring(err))

-- An unknown handle is likewise an error, not a crash.
local ok = pcall(goal.graphviz, 987)
assert(not ok, "goal.graphviz must refuse an unknown plan handle")
