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
--
-- Actions and walks are printed *together*, ordered by the tick the game
-- actually dispatched them at, because that is the only view in which the
-- interleaving is visible. A walk has no action id -- `(bot, step_index)` is
-- what names it -- so the two kinds are gathered into one list first.
local spans = {}

local ids = {}
for id, _ in pairs(final.actions) do ids[#ids + 1] = id end
table.sort(ids)
for _, id in ipairs(ids) do
  local a = final.actions[id]
  spans[#spans + 1] = {
    label = string.format("action %d", id),
    planned_start = a.planned_start, planned_end = a.planned_end,
    dispatched = a.dispatched_tick, replied = a.replied_tick,
    status = a.status, is_walk = false,
  }
end

for i = 1, #final.walks do
  local w = final.walks[i]
  spans[#spans + 1] = {
    label = string.format("walk b%d/s%d -> (%.1f,%.1f)",
      w.bot, w.step_index, w.to.x, w.to.y),
    planned_start = w.planned_start, planned_end = w.planned_end,
    dispatched = w.dispatched_tick, replied = w.replied_tick,
    status = w.status, is_walk = true,
  }
end

-- Ordered by the game's own dispatch tick. An unmeasured span (nil ticks) has
-- no place on that axis and is deliberately sorted last rather than being
-- given a stand-in number.
table.sort(spans, function(l, r)
  if l.dispatched == nil then return false end
  if r.dispatched == nil then return true end
  if l.dispatched ~= r.dispatched then return l.dispatched < r.dispatched end
  return l.label < r.label
end)

print("walks reported: " .. tostring(#final.walks))
print("step                            | planned_start planned_end | dispatched_tick replied_tick | status")
for _, s in ipairs(spans) do
  print(string.format("%-31s | %13s %11s | %15s %12s | %s",
    s.label,
    tostring(s.planned_start), tostring(s.planned_end),
    tostring(s.dispatched), tostring(s.replied),
    tostring(s.status)))
end

-- The property that proves the walk ticks came from the game and not from
-- anywhere else: one bot dispatches one thing at a time, so a walk's ticks must
-- fall between the previous span's reply and the next span's dispatch. A
-- violation is a finding, not something to smooth over -- so it is printed
-- loudly rather than ignored.
local problems = 0
local measured = {}
for _, s in ipairs(spans) do
  if s.dispatched ~= nil and s.replied ~= nil then measured[#measured + 1] = s end
end
for i = 1, #measured do
  local s = measured[i]
  if s.replied < s.dispatched then
    print("OUT OF ORDER: " .. s.label .. " replied before it was dispatched")
    problems = problems + 1
  end
  if i > 1 then
    local prev = measured[i - 1]
    if s.dispatched < prev.replied then
      print(string.format("OVERLAP: %s dispatched at %d but %s only replied at %d",
        s.label, s.dispatched, prev.label, prev.replied))
      problems = problems + 1
    end
  end
end

-- And the check the whole change exists for: an observed tick that equals its
-- planned counterpart means the field was filled from the schedule.
for _, s in ipairs(spans) do
  if s.dispatched ~= nil and s.dispatched == s.planned_start then
    print("SUSPECT: " .. s.label .. " dispatched_tick equals planned_start -- from the plan?")
    problems = problems + 1
  end
  if s.replied ~= nil and s.replied == s.planned_end then
    print("SUSPECT: " .. s.label .. " replied_tick equals planned_end -- from the plan?")
    problems = problems + 1
  end
end
print("interleaving problems: " .. tostring(problems))

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
