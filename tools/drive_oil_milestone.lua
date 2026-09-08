-- Drive the REAL scripts/oil_milestone.lua against the REAL scripts/supervisor.lua
-- with stubbed goal/rcon/record. Models one fact: charted ground reaches
-- __reach, the oil is at __want. Usage: lua drive_oil.lua <want>
local want = tonumber(arg[1] or "372.5")
local reach = 320

all_bots = { 1, 2, 3, 4 }

function include(name)
  local f = assert(loadfile("scripts/" .. name))
  f()
end

local tick = 1000
goal = {}
local target_ran = false
local last_code = nil
function goal.charted(x, y, r) return { kind = "charted", x = x, y = y, radius = r } end
function goal.all(gs) return { kind = "all", n = #gs } end
function goal.gathered(i) return { kind = "gathered", item = i } end
function goal.produced(i, n, o) return { kind = "produced", item = i } end
local function steps(n)
  local out = {}
  for i = 1, n do
    out[i] = { id = i, bot = ((i - 1) % 4) + 1, label = "step " .. i,
               start = (i - 1) * 10, finish = i * 10, deps = {} }
  end
  return out
end
function goal.refusal(_e)
  if last_code == nil then return nil end
  return { code = last_code, message = "no crude-oil is charted; ground reaches " .. reach }
end
function goal.plan(g, _o)
  if g.kind == "charted" then
    last_code = nil
    if g.radius <= reach then return { steps = {}, bots = all_bots, tick = tick } end
    return { steps = steps(8), bots = all_bots, tick = tick, radius = g.radius }
  end
  if reach < want then
    last_code = "planner::not_charted"
    error("goal: refused", 0)
  end
  last_code = nil
  if target_ran then return { steps = {}, bots = all_bots, tick = tick } end
  return { steps = steps(2295), bots = all_bots, tick = tick }
end
function goal.run(p)
  tick = tick + 4159
  if p.radius then
    if p.radius > reach then reach = p.radius end
  else
    target_ran = true
  end
  return { failed = 0, lost = 0, walks_failed = 0, walks_lost = 0, pending = 0,
           success = #p.steps, running = 0, done = true, actions = {}, walks = {} }
end
function goal.holds(g, _o)
  if g.kind == "charted" then return g.radius <= reach end
  return nil -- Gathered/Produced: unanswerable, as in production
end

rcon = {}
function rcon.players() return all_bots end
function rcon.sampling_start(_) end
function rcon.inventory_contents_at(_) return {} end
function rcon.game_tick() tick = tick + 200 return tick end

record = {}
function record.start(_) return "run-stub" end
function record.keyframe() end
function record.plan_created(...) end
function record.milestone_started(...) end
function record.milestone_satisfied(...) end
function record.milestone_stuck(...) end
function record.roster_changed(...) end
function record.actions(a, b) return 0 end
function record.walks(w) return 0 end
function record.teleports() return 0 end
function record.refusals() return 0 end
function record.enclosures() return 0 end
function record.deaths() return 0 end
function record.research_triggers() return 0 end
function record.finish(s) return "run-stub" end

local f = assert(loadfile("scripts/oil_milestone.lua"))
f()
