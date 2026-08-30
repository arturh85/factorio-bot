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

-- Research plans like anything else. The harness world carries one force with
-- one technology, `automation`: 10 units of one automation science pack each.
-- Asserted through the real binding because that is the only thing that proves
-- the planner's research method is reachable from Lua at all -- it used to be
-- bound to a method that did not exist.
local research = goal.researched("automation")
local research_makespan = goal.schedule(research, 4)
assert(research_makespan > 0, "expected a positive makespan for the research plan")
local research_dot = goal.graphviz(research)
assert(string.find(research_dot, "research automation", 1, true),
       "the research action must reach the plan, got: " .. research_dot)
-- The packs are actually produced rather than assumed: the same plan has to
-- mine for them.
assert(string.find(research_dot, "mine", 1, true),
       "the science packs must be produced by the plan, got: " .. research_dot)

-- A technology no force defines is a catchable Lua error rather than a panic,
-- which under `panic = "abort"` would take the whole server down -- and it
-- names the technology, so the reader can see it is the *name* that is wrong
-- rather than the world that is short of resources.
local ok, err = pcall(goal.researched, "teleportation")
assert(not ok, "goal.researched must refuse a technology no force defines")
assert(string.find(tostring(err), "teleportation", 1, true),
       "the error must name the technology asked for, got: " .. tostring(err))
assert(string.find(tostring(err), "defines no technology named", 1, true),
       "the error must say the technology is unknown, got: " .. tostring(err))

-- An unknown handle is likewise an error, not a crash.
local ok = pcall(goal.graphviz, 987)
assert(not ok, "goal.graphviz must refuse an unknown plan handle")
