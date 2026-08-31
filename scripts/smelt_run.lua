-- The first multi-step goal driven end to end against a live game.
--
-- `goal.have("iron-ore", 5)` -- a walk and a mine -- is the only goal
-- execution has ever completed. `goal.have("iron-plate", 10)` is the first one
-- that needs the whole chain: mine ore, mine coal, craft/place a furnace,
-- insert ore and fuel, wait out the smelt, remove the plates.
--
-- The plan is printed BEFORE it is run, because `goal.run` spends the plan and
-- because a plan nobody looked at is a run nobody can interpret.
print("start smelt run")

local bots = (type(all_bots) == "table" and #all_bots > 0) and #all_bots or 1
print("bots: " .. tostring(bots))

local plan = goal.plan(goal.have("iron-plate", 10))

-- ---------------------------------------------------------------- the plan --
-- Captured into plain tables first: `goal.run` consumes the plan userdata, so
-- nothing may be read off it afterwards.
local steps = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "mine" then
    detail = string.format("%d x %s @ (%.2f,%.2f)", s.count, s.item, s.pos.x, s.pos.y)
  elseif s.kind == "craft" then
    detail = string.format("%d x %s", s.count, s.item)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f)", s.entity, s.pos.x, s.pos.y)
  elseif s.kind == "insert" or s.kind == "remove" then
    detail = string.format("%d x %s %s %s @ (%.2f,%.2f)",
      s.count, s.item, s.kind == "insert" and "into" or "from",
      tostring(s.entity) .. "/" .. tostring(s.slot), s.pos.x, s.pos.y)
  elseif s.kind == "research" then
    detail = tostring(s.tech)
  else
    detail = "?"
  end
  steps[i] = {
    index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
    start = s.start, finish = s.finish,
  }
end

print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #steps, #plan.bots, tostring(plan.makespan)))
print("  #  bot  id   kind     start  finish  dur | detail")
local kind_count, kind_ticks = {}, {}
for _, s in ipairs(steps) do
  local dur = s.finish - s.start
  kind_count[s.kind] = (kind_count[s.kind] or 0) + 1
  kind_ticks[s.kind] = (kind_ticks[s.kind] or 0) + dur
  print(string.format("%3d %4d %4s  %-8s %6d %7d %4d | %s",
    s.index, s.bot, tostring(s.id), s.kind, s.start, s.finish, dur, s.detail))
end
print("  kind totals (count / planned ticks):")
local kinds = {}
for k in pairs(kind_count) do kinds[#kinds + 1] = k end
table.sort(kinds)
for _, k in ipairs(kinds) do
  print(string.format("    %-8s %3d steps  %6d ticks", k, kind_count[k], kind_ticks[k]))
end

-- Where the furnace is supposed to end up, so placement can be checked after.
local placements = {}
for _, s in ipairs(steps) do
  if s.kind == "place" then
    placements[#placements + 1] = s
  end
end

-- ----------------------------------------------------------------- the run --
print("dispatching...")
local obs = goal.run(plan)
print("run returned")

-- Actions and walks share one axis: the tick the game dispatched them at.
local spans = {}

-- Plan step lookup, so an observation can be named by what it was meant to do.
local by_action = {}
for _, s in ipairs(steps) do
  if s.id ~= nil then by_action[s.id] = s end
end

local ids = {}
for id in pairs(obs.actions) do ids[#ids + 1] = id end
table.sort(ids)
for _, id in ipairs(ids) do
  local a = obs.actions[id]
  local ps = by_action[id]
  spans[#spans + 1] = {
    label = string.format("act %-3d %-7s %s", id,
      ps and ps.kind or "?", ps and ps.detail or "(no plan step)"),
    kind = ps and ps.kind or "?",
    planned_start = a.planned_start, planned_end = a.planned_end,
    dispatched = a.dispatched_tick, replied = a.replied_tick,
    status = a.status, attempts = a.attempts, error = a.error,
  }
end

for i = 1, #obs.walks do
  local w = obs.walks[i]
  spans[#spans + 1] = {
    label = string.format("walk b%d/s%-3d -> (%.2f,%.2f)", w.bot, w.step_index, w.to.x, w.to.y),
    kind = "walk",
    planned_start = w.planned_start, planned_end = w.planned_end,
    dispatched = w.dispatched_tick, replied = w.replied_tick,
    status = w.status, attempts = nil, error = w.error,
  }
end

table.sort(spans, function(l, r)
  if l.dispatched == nil then return false end
  if r.dispatched == nil then return true end
  if l.dispatched ~= r.dispatched then return l.dispatched < r.dispatched end
  return l.label < r.label
end)

print("walks reported: " .. tostring(#obs.walks))
print("---- planned vs observed, ordered by the game's dispatch tick ----")
print(string.format("%-46s | %6s %6s %5s | %8s %8s %5s | %s",
  "span", "p_start", "p_end", "p_dur", "disp", "reply", "o_dur", "status"))
for _, s in ipairs(spans) do
  local odur = "nil"
  if s.dispatched ~= nil and s.replied ~= nil and s.replied >= s.dispatched then
    odur = tostring(s.replied - s.dispatched)
  end
  local pdur = "nil"
  if s.planned_start ~= nil and s.planned_end ~= nil then
    pdur = tostring(s.planned_end - s.planned_start)
  end
  print(string.format("%-46s | %6s %6s %5s | %8s %8s %5s | %s",
    s.label,
    tostring(s.planned_start), tostring(s.planned_end), pdur,
    tostring(s.dispatched), tostring(s.replied), odur,
    tostring(s.status) .. (s.attempts and (" x" .. s.attempts) or "")))
  if s.error ~= nil then
    print("    error: " .. tostring(s.error))
  end
end

-- ------------------------------------------------- the walk overhead check --
-- The scheduler's model is ceil(distance / 0.15) ticks, so a planned walk
-- duration implies the distance it was built from. A prior audit claimed the
-- 0.15 rate is right but that ~135 ticks of fixed per-walk cost is missing.
-- Fitting on some walks and predicting others is the only way to test that
-- rather than restate it.
local wobs = {}
for _, s in ipairs(spans) do
  if s.kind == "walk" and s.dispatched ~= nil and s.replied ~= nil
     and s.replied >= s.dispatched and s.status == "success" then
    wobs[#wobs + 1] = { planned = s.planned_end - s.planned_start,
                        observed = s.replied - s.dispatched, label = s.label }
  end
end
print("---- walk overhead ----")
print(string.format("%-46s | %7s %8s %8s %7s", "walk", "planned", "observed",
  "residual", "ratio"))
for _, w in ipairs(wobs) do
  print(string.format("%-46s | %7d %8d %8d %7.3f", w.label, w.planned, w.observed,
    w.observed - w.planned,
    w.planned > 0 and (w.observed / w.planned) or 0))
end
if #wobs >= 4 then
  -- Fit the constant on the odd-indexed walks, predict the even-indexed ones.
  local sum, n = 0, 0
  for i = 1, #wobs, 2 do sum = sum + (wobs[i].observed - wobs[i].planned); n = n + 1 end
  local c = sum / n
  print(string.format("fitted constant on %d walks: %.1f ticks", n, c))
  for i = 2, #wobs, 2 do
    local pred = wobs[i].planned + c
    print(string.format("  predict %-40s %8.1f vs actual %d  (err %+.1f)",
      wobs[i].label, pred, wobs[i].observed, wobs[i].observed - pred))
  end
else
  print("too few successful walks (" .. #wobs .. ") to fit and predict; residuals above only")
end

-- --------------------------------------------------------- what got placed --
for _, p in ipairs(placements) do
  print("---- placement check: " .. p.detail .. " ----")
  local m = p.detail:match("^([%w%-]+)")
  local x, y = p.detail:match("%(([%-%d%.]+),([%-%d%.]+)%)")
  if x ~= nil then
    local found = rcon.find_entities_in_radius({ x = tonumber(x), y = tonumber(y) }, 3, m)
    local n = 0
    for _, e in ipairs(found or {}) do
      n = n + 1
      print(string.format("  found %s at (%.2f,%.2f) direction=%s",
        tostring(e.name), e.position.x, e.position.y, tostring(e.direction)))
    end
    if n == 0 then print("  NOTHING of that name within 3 tiles of the planned position") end
  end
end

-- ---------------------------------------------------------------- verdict --
print(string.format("done=%s success=%s failed=%s lost=%s pending=%s running=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
print("first_error: " .. tostring(obs.first_error))
if obs.failed > 0 or (obs.lost or 0) > 0 then
  local fs = obs:failures()
  for i = 1, #fs do
    local ps = by_action[fs[i].id]
    print(string.format("failure[%d] id=%s status=%s attempts=%s planned=%s..%s disp=%s reply=%s",
      i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts),
      tostring(fs[i].planned_start), tostring(fs[i].planned_end),
      tostring(fs[i].dispatched_tick), tostring(fs[i].replied_tick)))
    print("  step: " .. (ps and (ps.kind .. " " .. ps.detail) or "(unknown)"))
    print("  error: " .. tostring(fs[i].error))
  end
end

-- What the bot actually holds at the end -- the goal's own success criterion,
-- independent of the executor's bookkeeping.
for _, id in ipairs(world.all_bots and world.all_bots() or { 1 }) do
  local pl = world.player(id)
  if pl == nil then
    print("bot " .. tostring(id) .. ": NOT PRESENT")
  else
    local n = 0
    for k, v in pairs(pl.main_inventory or {}) do
      n = n + 1
      print("bot " .. tostring(id) .. " final: " .. tostring(v) .. " x " .. tostring(k))
    end
    if n == 0 then print("bot " .. tostring(id) .. ": final inventory EMPTY") end
  end
end

print("end smelt run")
