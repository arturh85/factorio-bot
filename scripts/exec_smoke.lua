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

-- Real game ticks next to the scheduler's estimate.
--
-- `planned_*` is what the schedule predicted before anything ran;
-- `dispatched_tick`/`replied_tick` are `game.tick` as the game itself reported
-- it. If the two columns ever agree exactly, the observed fields are being
-- filled from the plan and none of this means anything.
local ids = {}
for id, _ in pairs(final.actions) do ids[#ids + 1] = id end
table.sort(ids)
print("action | planned_start planned_end | dispatched_tick replied_tick")
for _, id in ipairs(ids) do
  local a = final.actions[id]
  print(string.format("%6s | %13s %11s | %15s %12s  %s",
    tostring(id),
    tostring(a.planned_start), tostring(a.planned_end),
    tostring(a.dispatched_tick), tostring(a.replied_tick),
    tostring(a.status)))
end

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
