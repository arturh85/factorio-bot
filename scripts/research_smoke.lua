-- Can the planner plan a real research goal against a live game, and does it
-- charge for the work that research actually costs?
--
-- This is the acceptance test for two increments:
--
--  1. "plan through a locked recipe" -- a recipe disabled on a fresh map is
--     visible to the planner along with the technology that unlocks it, so a
--     plan can be made toward a future state.
--
--  2. "research triggers are costed" -- Factorio 2.0 technologies that
--     complete when the player *does* something (craft 50 iron plates) rather
--     than when a lab eats science packs. Their pack bill is empty and their
--     research time is zero, so a planner reading only the pack fields costed
--     them at *nothing*: `goal.researched("electronics")` used to come back
--     with makespan 0 and a single step. Correctly ordered, wrongly timed.
--
-- It plans only -- it never executes -- so it is safe against any world.
print("start research smoke")

-- The recipe has to be visible at all. It is disabled on a fresh map, which is
-- the whole point: a planner that only sees enabled recipes cannot plan toward
-- a future state.
local asp = world.recipe("automation-science-pack")
assert(asp ~= nil, "the live world knows no automation-science-pack recipe")
print("automation-science-pack recipe present, enabled=" .. tostring(asp.enabled)
      .. " category=" .. tostring(asp.category))

-- `electronics` is a trigger technology: "craft 10 copper plates". It has no
-- prerequisites and no science packs, so before triggers were modelled this
-- goal planned as a single zero-length step.
local p = goal.plan(goal.researched("electronics"))
print("electronics: makespan=" .. tostring(p.makespan)
      .. " steps=" .. tostring(#p.steps)
      .. " bots=" .. tostring(#p.bots))

assert(p.makespan > 0,
       "a trigger technology is not free: researching electronics means crafting "
       .. "10 copper plates, and a makespan of 0 means that work was never planned")
assert(#p.steps > 1,
       "the trigger's work must appear as steps, not just the research itself; "
       .. "got " .. tostring(#p.steps))

-- The plan has to actually research the technology it was asked for...
local researched = p:count { kind = "research", tech = "electronics" }
assert(researched > 0, "the plan never researches electronics")

-- ...and the work it charges for has to be the *trigger's* work. Copper plates
-- come out of a furnace, so the plan must mine copper ore and smelt it.
assert(p:count { kind = "mine", item = "copper-ore" } > 0,
       "the trigger needs copper plates, so copper ore must be mined")
assert(p:count { kind = "place", entity = "stone-furnace" } > 0,
       "and smelted")

-- Ordering: the research is the *last* thing that happens. The trigger fires
-- when the crafting is done, so nothing may be scheduled after it.
local research_end = nil
for _, st in ipairs(p:find { kind = "research" }) do
  if st.tech == "electronics" then research_end = st.finish end
end
assert(research_end ~= nil, "no finish time for the research")
for _, st in ipairs(p.steps) do
  assert(st.finish <= research_end,
         "step " .. tostring(st.kind) .. " finishes at " .. tostring(st.finish)
         .. ", after the research it is supposed to trigger (" .. tostring(research_end) .. ")")
end
print("ordering holds: the research completes only after its trigger work")

-- The locked-recipe half, on a technology whose trigger needs a locked recipe.
-- `steam-power` is "craft 50 iron plates" -- iron-plate is enabled, so this
-- also checks the trigger path does not gratuitously demand research.
local sp = goal.plan(goal.researched("steam-power"))
print("steam-power: makespan=" .. tostring(sp.makespan)
      .. " steps=" .. tostring(#sp.steps))
assert(sp.makespan > 0, "steam-power is 50 iron plates, not free")
assert(sp:count { kind = "mine", item = "iron-ore" } > 0, "iron must be mined")

-- KNOWN GAP, and deliberately not asserted as a success.
--
-- `goal.researched("automation")` needs `automation-science-pack`, whose
-- trigger is "craft 1 lab". A lab costs 10 iron gear wheels *and* 4 transport
-- belts, and transport belts are made of gear wheels too -- so two sibling
-- subgoals draw on the same intermediate. The planner sizes each share against
-- the same starting inventory, so the belts consume gears the lab craft was
-- counting on and the plan fails with "bot 1 has 8 iron-gear-wheel, needs 10".
--
-- That is a `Goal::Have` shortfall-accounting bug, NOT a research bug: plain
-- `goal.have("lab", 1)` fails the same way with no research involved. Before
-- triggers were costed, this step was simply invisible and `automation` planned
-- a makespan that omitted it. Refusing is the honest answer until shared
-- intermediates are sized correctly.
local ok, err = pcall(function() return goal.plan(goal.researched("automation")) end)
if ok then
  print("automation now plans -- the shared-intermediate gap may be fixed; "
        .. "consider promoting this to an assertion")
else
  print("automation still blocked (expected): " .. tostring(err))
end

print("end research smoke")
