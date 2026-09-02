-- Exercises the planning half of the goal.* API against the fixture world.
--
-- goal.start and goal.run are deliberately absent: they need a live Factorio
-- server. They are covered in Rust, in globals/goal/run.rs, against a stub
-- actuator.

-- A goal value is pure: nothing is planned, and nothing is checked against the
-- world, until goal.plan is called.
local g = goal.have("automation-science-pack", 10)
assert(g.kind == "have", "a goal value carries its own shape")
assert(g.item == "automation-science-pack", "and the item it asks for")
assert(g.count == 10, "and how many")

local plan = goal.plan(g)
assert(plan.makespan > 0, "expected a positive makespan")
assert(#plan.bots == 4, "the run has four bots and none was asked to sit out")
assert(#plan.steps > 0, "a plan is its steps")
-- More than one bot must appear or the plan was never actually spread.
assert(#plan:for_bot(1) > 0, "expected bot 1 to have work")
assert(#plan:for_bot(2) > 0, "expected the work to be spread over bots")

local dot = plan:graphviz()
assert(#dot > 0, "expected graphviz output")
assert(string.find(dot, "digraph", 1, true), "expected a graphviz digraph, got: " .. dot)

local gantt = plan:gantt("red science")
assert(string.find(gantt, "gantt", 1, true), "expected a mermaid gantt chart")
assert(string.find(gantt, "red science", 1, true), "the title must reach the chart")
assert(string.find(gantt, "section bot 1", 1, true), "expected bot 1 to have a section")
assert(string.find(gantt, "section bot 2", 1, true), "expected the work to be spread over bots")

-- Research needs a lab, and a lab needs power. The harness world carries one
-- force with one technology, `automation`, and no generator anywhere -- which
-- is every world the fixtures build, because `EntityGraph` does not expose
-- poles or generators by name (see
-- `docs/superpowers/notes/2026-09-02-research-needs-power.md`). So the planner
-- refuses, by name, instead of returning a plan whose last step could never
-- complete.
--
-- This is `workspace/runs/run-1788365280-15443/` milestone 7 as a one-liner:
-- that run planned `craft 1 lab; research automation` five times, never placed
-- the lab, generated 0.0 kW throughout, and sat at research progress 0.0 for
-- 60,661 ticks before giving up. Asserted through the real binding because it
-- is the only thing that proves the planner's research method is reachable
-- from Lua at all -- it used to be bound to a method that did not exist -- and
-- the refusal is raised from inside that method.
local unpowered = goal.researched("automation")
local ok, err = pcall(goal.plan, unpowered)
assert(not ok, "a research whose lab would have no power must be refused")
assert(string.find(tostring(err), "automation", 1, true),
       "the refusal must name the technology, got: " .. tostring(err))
assert(string.find(tostring(err), "electric supply", 1, true),
       "and say what is missing, got: " .. tostring(err))

-- A goal built for several goals at once plans as one plan, not two.
local both = goal.plan(goal.all { goal.have("iron-plate", 2), goal.have("coal", 2) })
assert(both:count { kind = "mine" } > 0, "goal.all reaches the planner")

-- A technology no force defines is a catchable Lua error rather than a panic,
-- which under `panic = "abort"` would take the whole server down -- and it
-- names the technology, so the reader can see it is the *name* that is wrong
-- rather than the world that is short of resources.
--
-- It raises at goal.plan, not at goal.researched: the constructor is pure and
-- has no world to check the name against.
local value = goal.researched("teleportation")
assert(value.technology == "teleportation", "constructing an unknown technology is fine")
ok, err = pcall(goal.plan, value)
assert(not ok, "goal.plan must refuse a technology no force defines")
assert(string.find(tostring(err), "teleportation", 1, true),
       "the error must name the technology asked for, got: " .. tostring(err))
assert(string.find(tostring(err), "defines no technology named", 1, true),
       "the error must say the technology is unknown, got: " .. tostring(err))

-- Something that is not a goal value at all is likewise an error, not a crash.
local ok = pcall(goal.plan, { kind = "no-such-kind" })
assert(not ok, "goal.plan must refuse a table that is not a goal")
