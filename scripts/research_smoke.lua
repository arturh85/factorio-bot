-- Can the planner plan a real research goal against a live game?
--
-- This is the acceptance test for the "plan through a locked recipe"
-- increment. It plans only -- it never executes -- so it is safe against any
-- world.
--
-- What it is really asking: `goal.researched("automation")` needs 10
-- automation science packs, and that recipe is *disabled* on a fresh map
-- until the `automation-science-pack` technology is researched. Until the mod
-- started sending disabled recipes (with their `enabled` flag) plus each
-- technology's unlock-recipe effects, the recipe was absent from world data
-- altogether and this goal failed with "no method can satisfy".
print("start research smoke")

-- The recipe has to be visible at all. It is disabled on a fresh map, which is
-- the whole point: a planner that only sees enabled recipes cannot plan toward
-- a future state.
local asp = world.recipe("automation-science-pack")
assert(asp ~= nil, "the live world knows no automation-science-pack recipe")
print("automation-science-pack recipe present, enabled=" .. tostring(asp.enabled)
      .. " category=" .. tostring(asp.category))

local p = goal.plan(goal.researched("automation"))
print("planned: makespan=" .. tostring(p.makespan)
      .. " steps=" .. tostring(#p.steps)
      .. " bots=" .. tostring(#p.bots))
assert(p.makespan > 0, "a real plan must take a positive number of ticks")

-- The plan has to actually research the technology it was asked for.
local researched = p:count { kind = "research", tech = "automation" }
print("research automation steps: " .. tostring(researched))
assert(researched > 0, "the plan never researches automation")

-- ...and it has to craft the packs the research consumes, which is only
-- possible through the locked recipe this increment made visible.
local packs = p:count { kind = "craft", item = "automation-science-pack" }
print("craft automation-science-pack steps: " .. tostring(packs))
assert(packs > 0, "the plan never crafts the science packs")

-- The locked recipe must not be crafted before the technology unlocking it is
-- researched. On a fresh map that technology is `automation-science-pack`,
-- which is also a prerequisite of `automation`, so it must appear too.
local unlock = p:count { kind = "research", tech = "automation-science-pack" }
print("research automation-science-pack steps: " .. tostring(unlock))
assert(unlock > 0, "the recipe's unlocking technology is never researched")

for _, st in ipairs(p:find { kind = "research" }) do
  print("  research step: " .. tostring(st.tech)
        .. " start=" .. tostring(st.planned_start or st.start))
end

-- Ordering: every craft of the locked recipe starts after the unlocking
-- research finishes. This is what the new `Condition::Researched` buys; without
-- it the plan would be one the game refuses to run.
local unlock_end = nil
for _, st in ipairs(p:find { kind = "research" }) do
  if st.tech == "automation-science-pack" then unlock_end = st.finish end
end
assert(unlock_end ~= nil, "no finish time for the unlocking research")
for _, st in ipairs(p:find { kind = "craft" }) do
  if st.item == "automation-science-pack" then
    assert(st.start >= unlock_end,
           "a locked-recipe craft starts at " .. tostring(st.start)
           .. ", before its unlocking research finishes at " .. tostring(unlock_end))
  end
end
print("ordering holds: every pack craft starts after the unlock research")

print("end research smoke")
