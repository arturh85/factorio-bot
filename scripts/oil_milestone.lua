-- The oil milestone, as the owner defined it: **a headless run that builds
-- the rig and where petroleum appears in the production samples.** A green
-- offline plan explicitly does not count, which is why this file exists at
-- all -- every headless run is driven by a Lua script, and until 2026-09-08
-- a script could not say either half of the goal:
--
--     All[ Gathered{crude-oil},
--          Produced{petroleum-gas, count 45, via basic-oil-processing} ]
--
-- `Gathered`, `Produced` and `Extracted` were reachable from `--goal-json`
-- and from the `plan` CLI's shorthand and from nowhere in Lua. So the
-- milestone was unreachable from the only path that can satisfy it.
--
-- ---------------------------------------------------------------------------
-- WHAT THIS SCRIPT CLAIMS, AND WHAT IT DOES NOT
-- ---------------------------------------------------------------------------
--
-- **A rising production curve is NOT evidence of a working factory.**
-- `production.made` counts what a *machine* produced, and a machine a bot
-- walked to and hand-loaded is a machine. A 179-entity furnace line was once
-- reported as smelting with no generator anywhere in it -- every plate came
-- from a bot carrying ore and coal in by hand.
--
-- So this script deliberately makes **no** verdict about attribution. It
-- starts the sampling session, holds the game open for a tick-bounded window
-- after the plan finishes so the samples cover the period petroleum would
-- appear in, and stops. The claim is made afterwards, by
-- `just analyse` / `tools/run_analysis.py`, which prints `roster-fed` /
-- `factory` / `unclear` per interval from the feeding-verb dispatches, the
-- roster's busy %, the kW generated and drawn, and the machine statuses.
-- **That verdict is the result of this run, not the numbers printed below.**
--
-- Oil is, as it happens, the one commodity where hand-feeding is not even
-- available: no character inventory can hold a fluid. That makes the
-- attribution unusually easy here -- but it is still the analyser's call and
-- not this script's.
--
-- ---------------------------------------------------------------------------
-- EVERY WINDOW HERE IS BOUND IN GAME TICKS
-- ---------------------------------------------------------------------------
--
-- Never in poll counts and never in wall time. `for _ = 1, 5000` polls is not
-- a duration: on a faster box more game ticks pass per RCON round trip, so
-- the same poll count covers more game time. A block compared that way was
-- published at 4.6x and re-measured at 2.6x on a fixed 6,300-tick window.
-- A tick-bounded window is immune to whatever else is running on the box --
-- starvation makes the wall clock longer and the run still covers the ticks
-- that were asked for.
--
-- ---------------------------------------------------------------------------
-- EXPECTED TO REFUSE, AS OF 2026-09-08
-- ---------------------------------------------------------------------------
--
-- Composing the goal is one thing; planning it is another. On
-- `map-31337-explored-with-categories.json` the composition currently refuses
-- with a chain-ownership error -- `bot N owns chain ChainId(...) because its
-- bill was sized against it, but has 5 iron-ore does not hold there`. A fix
-- is landing separately. **Reaching that refusal is the success condition for
-- the goal-expressibility work**: it proves the goal crossed into the planner
-- intact rather than dying at "unknown goal kind". This script reports the
-- refusal as a stuck milestone and finishes the run honestly rather than
-- pretending it planned.

print("start oil milestone")

-- How long to keep sampling after the plan is done, in GAME ticks. 18,000 is
-- five game minutes at 60 ticks a second -- long enough for the analyser's
-- 5-minute mark to land inside it, and it scales with `--game-speed` the way
-- everything tick-bounded does. Stated here as a named constant because the
-- window is the number that decides what a failure means, and a script that
-- buried it in a loop bound would hand back a verdict nobody derived.
local OBSERVE_TICKS = 18000

-- How many petroleum-gas to ask for. 45 is one full `basic-oil-processing`
-- batch's worth of output at the recipe's own ratio, i.e. small enough that
-- the goal is about the rig standing and pumping rather than about running a
-- refinery for an hour.
local PETROLEUM_COUNT = 45

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("roster: " .. tostring(#bots) .. " bot(s)")

local run_id = record.start({ video = false })
print("recording run " .. run_id)
-- The sampling session is what writes samples.jsonl -- research, production,
-- power and bot inventories on a 300/60-tick beat. It is the artefact the
-- milestone is defined against ("petroleum appears in the production
-- samples"), so it is started before anything is planned and never stopped
-- early: a session that ends at the last closed milestone once lost 199,449
-- ticks, the entire window the thing being measured existed in.
rcon.sampling_start(run_id)

-- The composition, exactly as the milestone states it.
--
--   * `goal.gathered("crude-oil")` -- the ENTITY in the ground, not an item:
--     a pumpjack on a well AND a tank for what it pumps, because no character
--     inventory can hold a fluid and until a tank stands there is nowhere for
--     crude to be.
--   * `goal.produced("petroleum-gas", 45, { via = "basic-oil-processing" })`
--     -- `produced`, not `have`, because `have` is satisfied by what a bot
--     holds and nothing can hold a fluid; and `via` because four recipes on
--     this install make petroleum-gas, so asking for the product alone is
--     ambiguous and is refused as such. The owner's ruling: the goal names
--     the recipe.
--
-- One `goal.all`, not two sequential plans, so the work shared between the
-- two halves -- the rig itself -- is planned once.
local milestone = goal.all {
  goal.gathered("crude-oil"),
  goal.produced("petroleum-gas", PETROLEUM_COUNT, { via = "basic-oil-processing" }),
}
print("goal: " .. tostring(milestone))

record.milestone_started(1, tostring(milestone))

-- `goal.plan` raises on a refusal, and a refusal is a fact about the world
-- rather than a defect: no charted oil, no route, a bill nobody can hold.
-- `goal.refusal` classifies one; `nil` back from it means the question could
-- not be answered at all, which is read as a fault and not as "carry on" --
-- an unanswerable classifier must never look like a green answer.
local ok, plan_or_err = pcall(goal.plan, milestone)

if not ok then
  local message = tostring(plan_or_err)
  local refusal = nil
  if type(goal.refusal) == "function" then
    local classified, r = pcall(goal.refusal, plan_or_err)
    if classified and type(r) == "table" and type(r.message) == "string" then refusal = r end
  end
  if refusal then
    print("PLAN REFUSED (" .. tostring(refusal.kind) .. "): " .. refusal.message)
  else
    print("PLAN REFUSED, unclassified: " .. message)
  end
  -- Recorded as stuck, not satisfied. A refusal is the run's result and it is
  -- reported as one; the analyser reads `milestone_stuck` and says so.
  record.milestone_stuck(1, "refused", message, nil)
  local id = record.finish("incomplete")
  print("RUN FINISHED id=" .. id .. " (plan refused; nothing was dispatched)")
  print("end oil milestone")
  return
end

local plan = plan_or_err
print(string.format("PLAN: %d step(s), %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))
record.plan_created(1, plan, plan.bots)

-- **An empty plan is not a satisfied milestone.** `furnace_run.lua` carries
-- this guard and the scar behind it: a run once reported milestone 1
-- satisfied with 89 of 179 entities standing. An empty plan means either the
-- goal already held or the planner found nothing to do, and neither of those
-- is "the rig was built by this run" -- so it is reported as its own outcome
-- and the run stops rather than claiming anything.
if #plan.steps == 0 then
  print("EMPTY PLAN: the planner found nothing to do for this goal.")
  print("  That is NOT the milestone. Either the goal already held on this map,")
  print("  or nothing was planned -- check goal.holds and the world before reading")
  print("  any production sample below as evidence of this run's work.")
  record.milestone_stuck(1, "plan_empty", nil, nil)
  local id = record.finish("incomplete")
  print("RUN FINISHED id=" .. id)
  print("end oil milestone")
  return
end

local steps = {}
local per_bot = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s", s.entity, s.pos.x, s.pos.y, tostring(s.direction))
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
  per_bot[s.bot] = (per_bot[s.bot] or 0) + 1
end
for _, b in ipairs(plan.bots) do
  print(string.format("bot %s: %s planned step(s)", tostring(b), tostring(per_bot[b] or 0)))
end

print("dispatching...")
local tick_before = rcon.game_tick()
local obs = goal.run(plan)
local tick_after = rcon.game_tick()
print(string.format("run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before), tostring(tick_after),
  (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
local n_refusals = record.refusals()
local n_enclosures = record.enclosures()
print(string.format("recorded: %d action event(s), %d walk event(s), %d refusal(s), %d enclosure(s)",
  n_actions, n_walks, n_refusals, n_enclosures))
if n_enclosures > 0 then print("WALLED IN: see record.enclosures() above") end

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

-- `obs.done` is the executor judging that no further progress is possible,
-- NOT that the plan completed -- so `done and failed == 0 and lost == 0`
-- would report a satisfied milestone over actions that were never dispatched.
-- `pending == 0` is the part that makes the claim true.
local dispatched_everything =
  obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 and (obs.pending or 0) == 0

if dispatched_everything then
  record.milestone_satisfied(1, 1, "plan_complete")
  print("MILESTONE 1: every planned action settled.")
  print("  This says the RIG WAS BUILT. It does NOT say petroleum was produced,")
  print("  and it does not say by whom -- the production samples and the")
  print("  analyser's attribution verdict answer that, not this line.")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
end

-- The observation window: hold the game open, tick-bounded, so the sampling
-- session covers the period petroleum would show up in. Nothing is dispatched
-- here and nothing is fed by hand -- an idle roster over the window is half of
-- what makes the analyser's `factory` verdict available at all.
--
-- The loop bound is a game tick, never a poll count. `rcon.game_tick()` is
-- the only clock this script, the record and the mod share.
local observe_from = rcon.game_tick()
if observe_from == nil then
  print("no game tick available: skipping the observation window entirely rather than")
  print("  substituting a wall-clock or poll-count one, which would not be a duration")
else
  local observe_to = observe_from + OBSERVE_TICKS
  print(string.format("OBSERVING: idle roster from tick %s to %s (%d ticks)",
    tostring(observe_from), tostring(observe_to), OBSERVE_TICKS))
  local next_mark = observe_from + 3600
  while true do
    local now = rcon.game_tick()
    if now == nil then
      -- The game went away underneath us. Say so; do not silently fall back
      -- to counting iterations, which is the failure this whole section is
      -- written against.
      print("  game tick unavailable mid-window: stopping the observation early")
      break
    end
    if now >= observe_to then break end
    if now >= next_mark then
      print(string.format("  tick=%s (+%s into the window)", tostring(now), tostring(now - observe_from)))
      next_mark = next_mark + 3600
      record.keyframe()
    end
  end
  print("observation window closed at tick " .. tostring(rcon.game_tick()))
end

local id = record.finish(dispatched_everything and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("")
print("THE RESULT IS NOT ON THIS PAGE. Read it with:")
print("  just analyse " .. id)
print("The milestone is 'petroleum appears in the production samples' AND the")
print("attribution for that interval is not `roster-fed`. A number without the")
print("verdict beside it is not the claim.")
print("end oil milestone")
