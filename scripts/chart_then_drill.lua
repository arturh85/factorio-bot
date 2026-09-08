-- Chart to the oil, then drill it: the oil milestone on a FRESH map.
--
-- `oil_milestone.lua` is the goal. This is the goal plus the thing that has to
-- happen before the goal can be planned at all. On a fresh seed-31337 map the
-- milestone refuses, correctly:
--
--     no crude-oil is charted anywhere this plan can see ... charted ground
--     covers 17 of 17 probes within 256 tiles of [0.5, 0.5]
--
-- That refusal is a fact about the world and not a defect. A fresh map is
-- generated out to +/-320 tiles at creation and the nearest crude-oil well on
-- this seed is at **372.5 tiles** -- outside it, by 52 tiles. No amount of
-- crafting, research or building clears it. Somebody has to go and look.
--
-- ---------------------------------------------------------------------------
-- THE SUPERVISOR LOOP, WHICH `goal.charted` ALREADY DESCRIBED
-- ---------------------------------------------------------------------------
--
-- This script invents nothing. `method::scout`'s own module doc says, under
-- "What it does *not* do":
--
--     Stop on find. A plan is expanded before anything runs, so nothing at
--     expansion time can know what a survey will reveal ... It belongs one
--     level up and is *already expressible*: because `Goal::Charted` is
--     idempotent and `PlanState` reads the live world, a supervisor that
--     plans ring by ring -- widening the radius and re-planning -- stops the
--     moment the goal it actually wanted stops raising `NotCharted`.
--
-- This is that supervisor. Ring by ring, widening, re-planning, stopping the
-- moment the milestone stops refusing for want of charted ground.
--
-- ---------------------------------------------------------------------------
-- WHY THE RINGS ARE WHERE THEY ARE, AND WHY THAT IS NOT A GUESS
-- ---------------------------------------------------------------------------
--
-- `REVEAL_PITCH` is **256 tiles**, measured on a live 2.1.17 headless server:
-- a lone character causes the engine to generate a 9x9 block of chunks centred
-- on it, i.e. +/-4 chunks = +/-128 tiles. So the lattice of survey points has
-- pitch 256 and each point buys +/-128 around itself, and
-- `survey_plan(centre, radius)` walks Chebyshev rings 0..floor(radius/256).
--
-- Radius 384 is therefore **exactly rings 0 and 1**: nine lattice points, the
-- eight outer ones at (+/-256, +/-256) and the cardinals between. The cell at
-- (256, -256) reveals x in [128, 384] and y in [-384, -128] -- and every one
-- of this seed's seven crude-oil tiles (x 131..154, y -365..-340) sits inside
-- it. One ring is enough. That is not a prediction this script relies on: the
-- loop widens if it is wrong, and says so.
--
-- ---------------------------------------------------------------------------
-- FOREKNOWLEDGE: NONE IS USED, AND THAT IS THE POINT
-- ---------------------------------------------------------------------------
--
-- The oil's position is known to whoever wrote this comment -- it was read out
-- of `map-31337-explored.json`, a dump of the same seed already explored, to
-- work out in advance how much walking this would cost. **It is not an input
-- to the run.** The script charts a disc centred on spawn and widens it
-- blindly; it never names the oil's coordinates, never calls `force.chart`,
-- never reads the map generator, and never cheats anything in. If the oil were
-- somewhere else the loop would find it one ring later and nothing here would
-- change.
--
-- The one thing the executor does that a human player could not is
-- `generate_chunks`, issued before each survey walk because **a bot cannot
-- path into ungenerated ground** (measured: x=300..600 all fail with `failed
-- to path find` on this seed). It is clamped mod-side to exactly the reveal a
-- character gets by standing there, so it buys no ground that walking would
-- not have bought -- and it is counted into `EventKind::BatchProgress` as
-- `ground_generated`, so the run record discloses it rather than this comment
-- being the only place it is written down.
--
-- ---------------------------------------------------------------------------
-- EXPLORATION IS A COMMITTED BREAK
-- ---------------------------------------------------------------------------
--
-- One ring, the whole roster, finished, then replan. Not a trickle of scouts
-- and not interleaved with building: `goal.plan(goal.charted(...))` is planned
-- and run to completion on its own before the milestone is asked again. The
-- cost that matters is the walk to the frontier, and the roster splits one
-- ring for free because its cells are all roughly the same distance out.
--
-- ---------------------------------------------------------------------------
-- THE COMPOSED GOAL PLANS; THE REFINERY HALF ALONE DOES NOT
-- ---------------------------------------------------------------------------
--
-- Measured offline on 2026-09-08 against
-- `map-31337-explored-with-categories.json`, four bots:
--
--     gathered:crude-oil                                 2,117 actions  325,138 ticks
--     produced:petroleum-gas:45:basic-oil-processing     REFUSED
--     both, as one goal.all                              2,295 actions  330,406 ticks
--
-- The refusal the middle row gets is a real one and worth knowing:
--
--     basic-oil-processing runs in oil-refinery (category oil-processing), and
--     the recipe wants 100 crude-oil -- a fluid, so it arrives by pipe rather
--     than in a hand. Nothing standing on this map can be shown to supply
--     crude-oil, so there is nothing to connect the oil-refinery to.
--
-- `FabricateRefusal` is a wall, not a shortfall. Asked on its own, the
-- refinery half looks at a map with no pumpjack on it and correctly says there
-- is nothing to pipe from. **Composed, it plans** -- and it costs only 178
-- actions more than the gathering half alone, which is what says the rig is
-- planned once and shared rather than twice.
--
-- **This corrects a premise this script was first written on.** The three-
-- phase split it started as -- gather, then replan the refinery against a
-- world with a tank standing in it -- was designed around the middle row and
-- would have planned the rig twice for no reason. One measurement of the
-- third row killed it. The sequential split survives only as a **fallback**,
-- taken if and only if the composed goal refuses with
-- `planner::no_fluid_source` live, which nothing predicts and which would mean
-- the live world differs from the dump in a way worth recording.
--
-- ---------------------------------------------------------------------------
-- WHAT THIS SCRIPT DOES NOT CLAIM
-- ---------------------------------------------------------------------------
--
-- Nothing about attribution. `production.made` counts what a *machine*
-- produced and a machine a bot hand-loaded is a machine. The verdict is
-- `just analyse`'s -- `roster-fed` / `factory` / `unclear`, per interval --
-- and not any number printed below. Oil is the one commodity where hand-
-- feeding is not even available (no character inventory holds a fluid), which
-- makes the attribution unusually easy here; it is still the analyser's call.
--
-- Run it:
--   factorio-bot lua chart_then_drill.lua --headless --bots 4 --seed 31337 --new


print("start chart-then-drill")

-- Ring radii, in tiles, in the order they are tried. Each lands squarely on a
-- lattice ring boundary rather than between two: 384 is rings 0-1, 640 is
-- 0-2, 896 is 0-3. Stated as a list rather than a loop bound because the
-- *schedule* of rings is the policy decision, and a reader must be able to see
-- how far this run was ever willing to walk.
local RING_RADII = { 384, 640, 896 }

-- How long to keep sampling after the last plan is done, in GAME ticks.
-- 18,000 is five game minutes at 60 ticks a second, so the analyser's
-- 5-minute mark lands inside it. It scales with `--game-speed` the way
-- everything tick-bounded does.
local OBSERVE_TICKS = 18000

-- One full `basic-oil-processing` batch's worth of output at the recipe's own
-- ratio: the goal is about the rig standing and pumping, not about running a
-- refinery for an hour.
local PETROLEUM_COUNT = 45

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("roster: " .. tostring(#bots) .. " bot(s)")

local run_id = record.start({ video = false })
print("recording run " .. run_id)
rcon.sampling_start(run_id)

-- The three goals. `gathered` is the charting probe as well as phase 2's work:
-- it is the half that names a resource in the ground, so it is the half that
-- refuses `planner::not_charted`.
local gathered = goal.gathered("crude-oil")
local refined = goal.produced("petroleum-gas", PETROLEUM_COUNT, { via = "basic-oil-processing" })
local milestone = goal.all { gathered, refined }
print("milestone: " .. tostring(milestone))

--- Classifies a `goal.plan` failure.
--
-- Returns `code, message`. `code` is `nil` when the error is not a planner
-- verdict at all -- a fault, not a refusal -- which is read as a reason to
-- stop rather than as a reason to carry on. An unanswerable classifier must
-- never look like a green answer.
local function classify(err)
  local message = tostring(err)
  if type(goal.refusal) == "function" then
    local ok, r = pcall(goal.refusal, err)
    if ok and type(r) == "table" and type(r.message) == "string" then
      return r.code, r.message
    end
  end
  return nil, message
end

--- Tries to plan `g`. Returns `plan | nil, code, message`.
local function try_plan(g)
  local ok, result = pcall(goal.plan, g)
  if ok then return result, nil, nil end
  local code, message = classify(result)
  return nil, code, message
end

--- Dispatches one plan and records it under milestone `n`.
--
-- Returns `true` when every planned action settled. `obs.done` alone is the
-- executor judging that no further progress is possible, NOT that the plan
-- completed, so `pending == 0` is the part that makes the claim true.
local function dispatch(n, plan, steps)
  print("  dispatching...")
  local tick_before = rcon.game_tick()
  local obs = goal.run(plan)
  local tick_after = rcon.game_tick()
  print(string.format("  run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
    tostring(tick_before), tostring(tick_after),
    (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

  local n_actions = record.actions(steps, obs.actions)
  local n_walks = record.walks(obs.walks)
  local n_refusals = record.refusals()
  local n_enclosures = record.enclosures()
  -- **Without this the record cannot tell a death from a cutscene.** The first
  -- run of this script omitted it, and `just analyse` correctly reported "BOT
  -- DEATHS AND ROSTER CHANGES: none recorded" over a run in which two bots
  -- were killed -- one by a small-biter at tick 27,065, one by a
  -- small-worm-turret at 53,619, both narrated on stdout by the run's own
  -- `paris` lines. The analyser then says, exactly right: "4 failure(s) ARE
  -- classified no_character, so this build knows the wording; a missing
  -- character with no death recorded is a cutscene, a controller switch, or a
  -- death the mod did not see." It was none of those. It was a recorder that
  -- was never called.
  local n_deaths = record.deaths()
  local n_teleports = record.teleports()
  print(string.format(
    "  recorded: %d action event(s), %d walk event(s), %d refusal(s), %d enclosure(s), %d death event(s), %d teleport(s)",
    n_actions, n_walks, n_refusals, n_enclosures, n_deaths, n_teleports))
  if n_enclosures > 0 then print("  WALLED IN: see record.enclosures() above") end

  print(string.format("  done=%s success=%s failed=%s lost=%s pending=%s running=%s",
    tostring(obs.done), tostring(obs.success), tostring(obs.failed),
    tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
  print("  first_error: " .. tostring(obs.first_error))
  if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
    local fs = obs:failures()
    for i = 1, math.min(#fs, 20) do
      print(string.format("  failure[%d] id=%s status=%s attempts=%s error=%s",
        i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts), tostring(fs[i].error)))
    end
    if #fs > 20 then print(string.format("  ... and %d more failure(s)", #fs - 20)) end
  end

  local complete = obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0
    and (obs.pending or 0) == 0
  if complete then
    -- `"plan_complete"` is what this line wants to say and `SatisfiedReason`
    -- cannot say it: the enum has exactly `already_satisfied` and
    -- `plan_empty`, so **every driver script in this tree records a plan that
    -- ran to completion as `plan_empty`** (`furnace_run.lua`,
    -- `block_run.lua`, `starter_run.lua`, `moving_block_live.lua`). That is
    -- the convention and it is followed here rather than diverged from, but
    -- it is a lie in the record: the most common way a milestone is reached
    -- is indistinguishable from the planner having found nothing to do.
    -- `oil_milestone.lua` passes `"plan_complete"` on this same path and
    -- would raise the moment it ever got there.
    record.milestone_satisfied(n, n, "plan_empty")
  else
    record.milestone_stuck(n, obs.done and "stuck" or "exhausted", obs.first_error, nil)
  end
  return complete
end

--- Plans `g` and runs it as milestone `n`. Returns `true` on completion.
--
-- **An empty plan is not a satisfied milestone.** It means either the goal
-- already held or the planner found nothing to do, and neither is "this run
-- built it", so it is reported as its own outcome.
local function phase(n, label, g)
  print("")
  print(string.format("== phase %d: %s ==", n, label))
  record.milestone_started(n, label)
  local plan, code, message = try_plan(g)
  if not plan then
    print(string.format("  PLAN REFUSED (%s): %s", tostring(code), tostring(message)))
    record.milestone_stuck(n, "refused", tostring(message), nil)
    return false, code, message
  end
  -- `PlanValue.steps` is a Lua field getter and each read rebuilds the whole
  -- array from the schedule, so it is read ONCE here and both shapes are
  -- built from that one array. At 2,798 steps the difference is not academic.
  local step_list = plan.steps
  print(string.format("  PLAN: %d step(s), %d bot(s), makespan=%s",
    #step_list, #plan.bots, tostring(plan.makespan)))

  -- Two different shapes, for two different recorders, and passing one to the
  -- other is exactly the mistake this script made on its second run:
  -- `record.plan_created` takes an **array of shaped step tables**, not the
  -- `PlanValue`, and a `PlanValue` is userdata, so it fails with `bad
  -- argument #2: error converting Lua userdata to table` -- after the plan is
  -- made and before anything is dispatched, which is the most expensive place
  -- to fail. `oil_milestone.lua` has the same call and would fail the same
  -- way the moment its goal ever planned.
  --
  -- `deps` is read by TYPE, not by truthiness: `Option::None` reaches Lua as
  -- mlua's null sentinel, which is light userdata and therefore truthy, so
  -- `s.deps or {}` does not substitute the empty table.
  local record_steps = {}
  local per_bot = {}
  for _, s in ipairs(step_list) do
    per_bot[s.bot] = (per_bot[s.bot] or 0) + 1
    -- A step with no `id` is skipped rather than erroring: a walk has no
    -- action id to report.
    if s.id ~= nil then
      local deps = {}
      if type(s.deps) == "table" then
        for _, d in ipairs(s.deps) do deps[#deps + 1] = d end
      end
      -- `planned_step_from_lua` requires `action` to be a string and the two
      -- tick fields to be numbers, and raises otherwise -- **after** the plan
      -- has been made, which on this goal is three minutes of paused game.
      -- Defaulted rather than trusted: a step with no label is recorded under
      -- its kind, which is worse information than a label and infinitely
      -- better than losing the run to it. `supervisor.lua` passes `s.label`
      -- bare; nothing has yet shown a step without one, so this is belt and
      -- braces, not a fix for an observed gap.
      local start = s.start or 0
      record_steps[#record_steps + 1] = {
        id = s.id,
        bot = s.bot,
        action = s.label or s.kind or "step",
        deps = deps,
        planned_start = start,
        planned_duration = (s.finish or start) - start,
      }
    end
  end
  -- The tick `goal.plan` handed the plan back at. Without it the event takes
  -- the last RCON reply's tick, which on a headless run is the run's start --
  -- so the plan would read as made before the minutes of planning that went
  -- into it.
  record_steps.tick = plan.tick
  record.plan_created(n, record_steps, plan.bots)

  for _, b in ipairs(plan.bots) do
    print(string.format("  bot %s: %s planned step(s)", tostring(b), tostring(per_bot[b] or 0)))
  end

  if #step_list == 0 then
    print("  EMPTY PLAN: the planner found nothing to do for this goal.")
    print("    That is NOT the milestone -- either it already held or nothing was")
    print("    planned. Check goal.holds and the world before reading any")
    print("    production sample as evidence of this run's work.")
    record.milestone_stuck(n, "plan_empty", nil, nil)
    return false, "plan_empty", nil
  end
  -- **`plan.steps` itself, not a reshape of it.** `record.actions` reads
  -- `label`, and `kind`/`item`/`count`/`entity`/`slot` to reconstruct what a
  -- hand delivery put into what. `oil_milestone.lua` (and this script, until
  -- the shapes were checked against the recorder rather than copied) passes a
  -- reshaped array carrying `detail` instead of `label` and none of the
  -- delivery fields, so every `action_dispatched` in its record would have an
  -- empty action name and no delivery at all -- silently, since a missing
  -- field is read as "no delivery" and not as an error.
  return dispatch(n, plan, step_list), nil, nil
end

--- Closes the run: the idle observation window, then `record.finish`.
--
-- The window is what makes the sampling session cover the period petroleum
-- would show up in. Nothing is dispatched inside it and nothing is fed by
-- hand -- an idle roster over the window is half of what makes the analyser's
-- `factory` verdict available at all. Its bound is a game tick, never a poll
-- count.
local function close(outcome)
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

  local id = record.finish(outcome)
  print("RUN FINISHED id=" .. id)
  print("")
  print("THE RESULT IS NOT ON THIS PAGE. Read it with:")
  print("  just analyse " .. id)
  print("The milestone is 'petroleum appears in the production samples' AND the")
  print("attribution for that interval is not `roster-fed`. A number without the")
  print("verdict beside it is not the claim.")
  print("end chart-then-drill")
end

-- ---------------------------------------------------------------------------
-- Phase 1: chart until `gathered:crude-oil` stops refusing for want of ground
-- ---------------------------------------------------------------------------

print("")
print("== phase 1: chart to the oil ==")
record.milestone_started(1, "chart to the oil")

local plan, code, message = try_plan(gathered)
print("before any charting: " ..
  (plan and "PLANNABLE" or ("REFUSED (" .. tostring(code) .. ") " .. tostring(message))))

local rings_walked = 0
local survey_actions = 0

if not plan then
  for _, radius in ipairs(RING_RADII) do
    if plan then break end
    -- `planner::not_charted` is the only refusal a bigger disc can clear.
    -- Anything else -- a bill nobody can hold, a recipe no machine runs, no
    -- site -- is a fact about the world that more walking does not touch, and
    -- widening the ring in the face of it would be a loop that looks like
    -- progress and is not.
    if code ~= "planner::not_charted" then
      print("refusal is " .. tostring(code) .. ", which charting cannot clear -- not widening")
      break
    end

    local disc = goal.charted(0, 0, radius)
    local held = goal.holds(disc)
    print(string.format("-- ring radius %d: goal.holds = %s --", radius, tostring(held)))
    if held == true then
      -- Already looked at. `Goal::Charted` is idempotent, so re-issuing it
      -- costs nothing and the loop simply widens.
      print("  already charted; widening")
    else
      local survey = goal.plan(disc)
      print(string.format("  survey: %d step(s), %d bot(s), makespan=%s",
        #survey.steps, #survey.bots, tostring(survey.makespan)))
      if #survey.steps == 0 then
        -- `holds` said no and the plan is empty. Those disagree, and the
        -- honest response is to say so and widen rather than to pick one.
        print("  EMPTY SURVEY PLAN although goal.holds did not say true -- widening")
      else
        -- The committed break: the whole roster, this ring, run to completion.
        local obs = goal.run(survey)
        rings_walked = rings_walked + 1
        survey_actions = survey_actions + #survey.steps
        print(string.format("  survey run: done=%s success=%s failed=%s lost=%s pending=%s",
          tostring(obs.done), tostring(obs.success), tostring(obs.failed),
          tostring(obs.lost), tostring(obs.pending)))
        if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
          print("  first_error: " .. tostring(obs.first_error))
        end
        record.walks(obs.walks)
      end
    end

    -- Replan. This is the stop-on-find `method::scout` says belongs here.
    plan, code, message = try_plan(gathered)
    print("  after ring: " ..
      (plan and "PLANNABLE" or ("REFUSED (" .. tostring(code) .. ") " .. tostring(message))))
  end
end

print(string.format("charting done: %d ring(s) walked, %d survey action(s)", rings_walked, survey_actions))

if not plan then
  print(string.format("PLAN REFUSED (%s): %s", tostring(code), tostring(message)))
  record.milestone_stuck(1, "refused", tostring(message), nil)
  close("incomplete")
  return
end

-- See the note beside the other `milestone_satisfied` call: the enum has two
-- reasons and neither is "a plan ran". `already_satisfied` is honest when no
-- ring was walked -- the ground was already looked at -- and `plan_empty` is
-- the tree's convention for the case where one was.
record.milestone_satisfied(1, math.max(rings_walked, 1),
  rings_walked == 0 and "already_satisfied" or "plan_empty")
print("MILESTONE 1: the ground the milestone needs has been looked at.")

-- ---------------------------------------------------------------------------
-- Phase 2: the milestone itself -- the rig and the refinery, planned together
-- ---------------------------------------------------------------------------

local ok, code2, message2 = phase(2, tostring(milestone), milestone)

if ok then
  print("MILESTONE 2: every planned action settled.")
  print("  This says the RIG AND REFINERY WERE BUILT. It does NOT say how much")
  print("  petroleum exists, and it does not say by whom -- the production")
  print("  samples and the analyser's attribution verdict answer that.")
  close("done")
  return
end

-- The fallback, and the narrow condition it is for. `planner::no_fluid_source`
-- is the one refusal that ordering can clear: it says nothing standing pipes
-- crude, which building the rig would change. Offline the composed goal plans
-- straight through it, so reaching here at all is a finding and is printed as
-- one. Every other outcome -- a refusal of another kind, or actions that
-- failed -- is reported as it is and nothing is retried, because retrying
-- until something nicer comes out is how a run stops meaning anything.
if code2 ~= "planner::no_fluid_source" then
  print("not the fluid-source refusal, so splitting the goal cannot help. Stopping.")
  close("incomplete")
  return
end

print("")
print("UNEXPECTED: the composed goal refused with planner::no_fluid_source live,")
print("  where it planned offline on map-31337-explored-with-categories.json.")
print("  Falling back to the sequential split -- build the rig, then replan the")
print("  refinery against a world that has a fluid source standing in it. This")
print("  plans the rig twice; the fallback is taken because the run is worth")
print("  more than the duplicated planning, and the fact that it was taken is")
print("  the finding.")

local gathered_ok = phase(3, tostring(gathered), gathered)
if not gathered_ok then
  print("the rig was not completed, so nothing standing supplies crude-oil and the")
  print("  refinery still cannot be planned. Stopping.")
  close("incomplete")
  return
end
print("MILESTONE 3: every planned action settled -- the rig stands.")

local refined_ok = phase(4, tostring(refined), refined)
if refined_ok then
  print("MILESTONE 4: every planned action settled.")
  print("  This says the REFINERY WAS BUILT AND FED. It does NOT say how much")
  print("  petroleum exists, and it does not say by whom.")
end

close(refined_ok and "done" or "incomplete")
