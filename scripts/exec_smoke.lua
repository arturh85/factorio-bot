-- The smallest possible REAL execution: walk and mine.
--
-- Everything until now has been planning. This is the first script that asks
-- the executor to issue actual RCON commands to a live game, so it does the
-- least it can: no placement (no collision risk), no crafting, no smelting.
-- If this works, the executor's seam to the real game works.
print("start exec smoke")

local bots = (type(all_bots) == "table" and #all_bots > 0) and #all_bots or 1
print("bots: " .. tostring(bots))

local plan = goal.plan(goal.have("iron-ore", 5))
print("planned: makespan=" .. tostring(plan.makespan))

local run = goal.start(plan)
print("dispatched, waiting...")
local final = run:wait()

print("done="   .. tostring(final.done)
   .. " success=" .. tostring(final.success)
   .. " failed="  .. tostring(final.failed)
   .. " pending=" .. tostring(final.pending)
   .. " running=" .. tostring(final.running))

if final.failed > 0 then
  print("FAILURES PRESENT -- inspect above for the actuator error")
  print("first_error: " .. tostring(final.first_error))
  local fs = final:failures()
  for i = 1, #fs do
    print("failure[" .. i .. "] id=" .. tostring(fs[i].id)
      .. " status=" .. tostring(fs[i].status)
      .. " attempts=" .. tostring(fs[i].attempts)
      .. " error=" .. tostring(fs[i].error))
  end
end
print("end exec smoke")
