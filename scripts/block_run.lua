-- Task 4: build MinerLine (rcontest.lua's blueprint) against a live game, for
-- real -- goal.built(...) planned AND run, not just planned like
-- block_smoke.lua. Modelled on smelt_run.lua's planned-vs-observed report and
-- factory_stage2.lua's record.* calls, collapsed to one milestone since this
-- goal has no ladder.
--
-- CHEATED, DISCLOSED HERE AND IN THE NOTE: every bot is given the full
-- material set (13 electric-mining-drill, 21 transport-belt, 3
-- small-electric-pole) by `rcon.cheat_item` before planning. The thing under
-- test is whether 37 decoded entities land at the right position and
-- direction when four bots build them -- NOT whether bots can gather 13
-- drills' worth of ore/plates/gears, which is already answered offline (491
-- actions, researches and crafts them). So materials are cheated, entities
-- are not: every placement below is a real `goal.run` dispatch, not a
-- scripted `cheat_blueprint`. The construction time this reports EXCLUDES
-- gathering and must not be quoted as an honest end-to-end build time.
print("start block run")

-- The same MinerLine string as block_smoke.lua and rcontest.lua's
-- blueprints.MinerLine -- copied rather than `include`d, because rcontest.lua
-- fires rcon.cheat_* calls at its own top level and this run wants to control
-- exactly which cheats run and when.
local MINER_LINE = "0eNqdl11v2yAUhv9KxLWdBPBH7MtN603Vq3ZSt2ma/MEyJAwIcFcr8n8vcapoWjztwJWFDQ+Hw3l5zQm1YmTacOlQfUI9s53h2nElUY0e1Wg6Vm9+Oadtvdv9bDqnDFdLd7vt1LB74ez3Ln24+/rhy2d83368e+0n3d4PzyhBVjY6dSo9Gt6f4a+oLhM0oZrgOUFNa5UYHUvP3TSXR1Q7M7IE8U5Ji+pvJ2T5UTbiPNRNmvmAuGODB8tmOLeYYJ0zvEsHLv34tDdcCOTRXPbMT4bn7wli0nHH2QW4NKYfchxaZnyH/6ASpJXll2Qs4eNtvizAP/00PTd+1PKVzMkNnVzpzjTSamVc2jLhbrH0Hbv/G5utYGlo0Pm/gi5W6Flw0AQSdB6MxRBsEbuBOWQDyyvdDo0Q6XUOrQS7ZZN3tl/BvEI7BKcgg6SgCsZSCBbvY8ssg5QZxsFhF6Cww0WXg7jRqitB6chiC/kAKWQcLr8DKCtFMLcEcSOVh/fr0sPh2sOgsxiHq6+CcEm0+m7jXqs3Eu98BGR94TLEIB8hNBwMchKSRWecgjKeR2cc5FUkXIoY5C+kDAeDHIZEiBLkASRclBhkAjRelQWkRmi8KitIjVASeayW68cqjRAjyFdo+F8oBhkLzaN3sALtYIQKQYZAw1VIVhzM34eWK1T9x60vQaLxJP/ugWw3nxojps2TUv79CzP2UkwHnJVZVRYl3hd5Mc9vau2sKw=="
-- (10,10) -- block_smoke.lua's anchor -- sits on bare dirt on this seed's
-- fresh map, and (-27,-34), chosen to put every drill on real ore, still hit
-- a transport-belt refusal elsewhere in the footprint. Both crashed
-- `goal.plan` itself (see docs/superpowers/notes/2026-09-05-first-block-built.md).
-- Kept at the ore-aligned anchor for this attempt since it is still the
-- better-sited of the two.
local ANCHOR = { x = -33.0, y = -40.0 }

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

-- ---------------------------------------------------------------- CHEATS --
-- Materials only, never entities. Every bot gets the full set so whichever
-- band-splitting the planner does, each band's own bot already holds enough
-- -- this also means "the shortfall machinery" (Goal::Have subgoals per band)
-- should bill nothing, since nothing is missing.
print("CHEAT: giving every bot the full material set (13 electric-mining-drill, "
  .. "21 transport-belt, 3 small-electric-pole) -- gathering is not what this run tests")
for _, b in ipairs(bots) do
  rcon.cheat_item(b, "electric-mining-drill", 13)
  rcon.cheat_item(b, "transport-belt", 21)
  rcon.cheat_item(b, "small-electric-pole", 3)
end

local run_id = record.start({ video = false })
print("recording run " .. run_id)

-- The sandbox has no `os` library (table/string/math/coroutine only), so wall
-- time is not measurable from inside the script; game ticks (`rcon.last_tick`)
-- are the clock available here, and the harness's own log timestamps give
-- wall time from outside.
local plan = goal.plan(goal.built(MINER_LINE, ANCHOR))
print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

local placed_plan = plan:count { kind = "place" }
print("planned placements: " .. tostring(placed_plan))

-- Captured into plain tables first: `goal.run` consumes the plan userdata.
local steps = {}
local per_bot_steps = {}
local expected = {} -- place steps only: what the plan intends to stand
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s", s.entity, s.pos.x, s.pos.y, tostring(s.direction))
    expected[#expected + 1] = { entity = s.entity, pos = s.pos, direction = s.direction }
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
  per_bot_steps[s.bot] = (per_bot_steps[s.bot] or 0) + 1
end
for _, b in ipairs(bots) do
  print("bot " .. b .. ": " .. tostring(per_bot_steps[b] or 0) .. " planned step(s)")
end
print("expected placements captured from the plan: " .. tostring(#expected))

print("dispatching...")
local tick_before = rcon.game_tick()
local obs = goal.run(plan)
local tick_after = rcon.game_tick()
print(string.format("run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before), tostring(tick_after),
  (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

-- Feed the map/event record the same way the supervisor loop does.
local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
local n_teleports = record.teleports()
local n_refusals = record.refusals()
local n_enclosures = record.enclosures()
print(string.format("recorded: %d action events, %d walk events, %d teleports, %d refusals, %d enclosures",
  n_actions, n_walks, n_teleports, n_refusals, n_enclosures))
if n_enclosures > 0 then print("WALLED IN: see record.enclosures() above") end

record.milestone_started(1, "MinerLine (37 entities) built at " .. ANCHOR.x .. "," .. ANCHOR.y)
-- `obs.done` is true when the executor judges no further progress possible,
-- NOT only when the plan is complete, so `done and failed == 0 and lost == 0`
-- reports a satisfied milestone with actions never dispatched. A run of the
-- block work recorded milestone 1 satisfied at 89 of 179 entities standing.
-- `pending == 0` is the part that makes the claim true.
if obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 and (obs.pending or 0) == 0 then
  record.milestone_satisfied(1, 1, "plan_empty")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
end

print(string.format("done=%s success=%s failed=%s lost=%s pending=%s running=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
print("first_error: " .. tostring(obs.first_error))
if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
  local fs = obs:failures()
  for i = 1, #fs do
    print(string.format("failure[%d] id=%s status=%s attempts=%s error=%s",
      i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts), tostring(fs[i].error)))
  end
end

-- ------------------------------------------------ read the world back --
-- The deliverable: every one of the 37 entities, checked against the LIVE
-- game (not just the plan's intent) for position AND direction. Queried
-- directly over rcon.find_entities_in_radius, which is better evidence than
-- reading map.jsonl's `placed` lines, since it asks the surface itself
-- rather than what the executor believed it asked for.
print("---- reading the world back: position + direction, per entity ----")
local NAMES = { "electric-mining-drill", "transport-belt", "small-electric-pole" }
local actual_by_key = {}
local total_found = 0
for _, name in ipairs(NAMES) do
  local found = rcon.find_entities_in_radius(ANCHOR, 40, name)
  print(name .. ": " .. tostring(#found) .. " found near anchor")
  for _, e in ipairs(found or {}) do
    total_found = total_found + 1
    local key = string.format("%.1f,%.1f", e.position.x, e.position.y)
    actual_by_key[key] = { name = e.name, position = e.position, direction = e.direction }
  end
end
print("total entities found near anchor: " .. tostring(total_found) .. " (want 37: 13+21+3)")

local matched, wrong_dir, missing = 0, 0, 0
for _, exp in ipairs(expected) do
  local key = string.format("%.1f,%.1f", exp.pos.x, exp.pos.y)
  local act = actual_by_key[key]
  if act == nil then
    missing = missing + 1
    print(string.format("MISSING: %s expected @ (%s) dir=%s -- nothing found there",
      exp.entity, key, tostring(exp.direction)))
  elseif act.name ~= exp.entity then
    missing = missing + 1
    print(string.format("WRONG ENTITY: expected %s @ (%s), found %s", exp.entity, key, act.name))
  elseif tostring(act.direction) ~= tostring(exp.direction) then
    wrong_dir = wrong_dir + 1
    print(string.format("WRONG DIRECTION: %s @ (%s) expected dir=%s, game reports dir=%s",
      exp.entity, key, tostring(exp.direction), tostring(act.direction)))
  else
    matched = matched + 1
  end
end
print(string.format("VERDICT: %d/%d matched position+direction, %d wrong direction, %d missing/wrong entity",
  matched, #expected, wrong_dir, missing))

local id = record.finish(obs.done and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("end block run")
