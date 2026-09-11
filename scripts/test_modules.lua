-- Module pipeline test: produce 6 red science packs
local goals = { goal.producing("automation-science-pack", 6) }
goal.plan(goals[1], { planner_mode = "modules" })
