-- Does a cell outlive its charge?
--
-- # The question
--
-- This project earned its first `factory` attribution verdict on 2026-09-08 --
-- an `assembling-machine-1` made 6 of 6 science packs in a minute with the
-- roster feeding nothing -- and then **stopped dead at 9:48**:
--
--     automation-science-pack plateaus at 9:48 (15) -- THE MACHINES STOPPED --
--     not one machine produced a single item after the plateau -- their own
--     lifetime counters, not an inference
--
-- The cell was draining a chest a bot had filled. `CELL_CHARGE_TICKS` is 9,000
-- ticks and nothing refilled it. So that verdict says "a machine ran
-- unattended for two and a half minutes", which is automation in the narrowest
-- sense and not a factory.
--
-- `3dadc04e` belts a standing stage-1 cell into the assembly cell's supply
-- chest, with an electric load arm -- electric because the belt carries plates
-- and a burner inserter there would have no fuel source at all. The hand
-- charge survives, demoted to an ignition charge, because a belt delivers
-- nothing while it is still filling.
--
-- # What would settle it
--
-- **`factory` across an interval with zero feeding dispatches, and NO PLATEAU
-- after minute ~10** -- where the charge-fed cell died. The window is 45,000
-- ticks so total game time passes 25 minutes: the 9:48 cell expired at about
-- two charges, so anything shorter cannot distinguish "the belt works" from
-- "the ignition charge was bigger this time".
--
-- Two readings that mean different things, and the record can tell them apart:
-- a plateau with `no_ingredients` on the assembling machine means the belt is
-- not delivering; a plateau with the supply chest FULL means the cell is
-- output-bound instead, which is a different bug.
--
-- # Judged on rate and census, not makespan
--
-- Priced offline at 878 actions / 46,554 ticks with 33 belts and both arms
-- named in the step list. One risk stated in advance: the load arm sits 16
-- tiles out on a four-pole run and poles cost wood, so a wood shortfall
-- refuses BY NAME (`Have 1 wood`) rather than silently.

include("supervisor.lua")

local PER_MINUTE = 6

-- Long enough to cross the fuel-decay boundary the prediction above names.
-- `CELL_FUELLED_TICKS` is ~36,000 and the plan's own makespan is ~42,800, so a
-- window of 18,000 puts the last mark well past both. Bounded in GAME TICKS,
-- never a poll count: on a faster box more ticks pass per RCON round trip, and
-- a block once published at 4.6x re-measured at 2.6x on a fixed tick window.
local OBSERVE_TICKS = 45000

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("roster: " .. tostring(#bots) .. " bot(s)")

local run_id = record.start({ video = false })
print("recording run " .. run_id)
-- Started before anything is planned and never stopped early: this session
-- writes `samples.jsonl`, and its machine rows are the only input to the
-- census. A session that ended at the last closed milestone once lost 199,449
-- ticks -- the entire window the thing being measured existed in.
rcon.sampling_start(run_id)

-- `roster` is a FUNCTION, not a table -- `supervisor.new` refuses a table by
-- name, which is how this comment came to exist. Taken whole from
-- `factory_stage3.lua`: ask the game who it has, and let the supervisor decide
-- what to do about a bot that died. Eighteen cells is a lot of walking and this
-- run is not peaceful, so a death is the expected case rather than the corner.
local function current_roster()
  local ok, ids = pcall(function() return rcon.players() end)
  pcall(function() rcon.inventory_contents_at({}) end)
  if ok and type(ids) == "table" then return ids end
  return nil
end

local sup = supervisor.new(
  supervisor.list { goal.all { goal.sustain("copper-plate", 15, 36000),
                               goal.producing("automation-science-pack", PER_MINUTE) } },
  { bots = bots, stall_limit = 3, max_iterations = 10, roster = current_roster }
)

print(string.format("goal: sustain copper-plate + producing:automation-science-pack:%d", PER_MINUTE))

-- **The supervisor does not record; the driver does.** `sup:step()` returns a
-- transition and the record plumbing hangs off it -- `record.actions`,
-- `record.walks`, `record.teleports`, `record.plan_created`, `record.deaths`.
--
-- The first version of this script was `repeat sup:step() until sup:finished()`
-- and threw every transition away. It ran, built 24 furnaces and 3 drills, made
-- 270 plates, got stuck, and left an `events.jsonl` containing five event kinds
-- -- none of them an action, a walk or a refusal. So the run could not say why
-- it had eight times as many furnaces as drills, which was the one question it
-- existed to answer. A measurement that cannot diagnose its own failure is
-- worse than no measurement, because it looks like one.
--
-- Modelled on `factory_stage3.lua`'s loop, trimmed to this run's transitions.
while not sup:finished() do
  local t = sup:step()
  if type(t) == "table" then
    if t.action == "planned" and type(t.plan) == "table" then
      -- `t.bots` passed through even when nil: `record.plan_created` then
      -- writes null, which says "nobody told us" rather than naming a roster
      -- nobody established.
      record.plan_created(t.milestone_index, t.plan, t.bots)
      print(string.format("   planned %s steps (best %s)%s",
        tostring(t.steps), tostring(t.best),
        t.recovery and ("  -- recovered: " .. tostring(t.recovery)) or ""))
    elseif t.action == "acquired" then
      record.milestone_started(t.milestone_index, "continuous supply")
    elseif t.action == "rerostered" then
      local nd = record.deaths()
      record.roster_changed(t.bots, t.left, t.returned, t.reason)
      print(string.format("   ROSTER CHANGED -- %s%s",
        tostring(t.reason), nd > 0 and (" (+" .. nd .. " death events)") or ""))
    elseif t.action == "ran" then
      local n = 0
      if t.steps ~= nil and t.actions ~= nil then n = record.actions(t.steps, t.actions) end
      -- A separate call, not a third argument: a walk is not an action. It has
      -- no action id, is in neither `steps` nor `actions`, and `(bot,
      -- step_index)` is the only thing that names it. Walking is most of the
      -- wall clock in these plans.
      local nw = 0
      if type(t.walks) == "table" then nw = record.walks(t.walks) end
      local nt = record.teleports()
      local nr = record.refusals()
      print(string.format("   ran: +%d action(s), +%d walk(s), +%d teleport(s), +%d refusal(s)",
        n, nw, nt, nr))
      if nr > 0 then
        print("   REFUSALS above -- a placement the game or the planner declined;")
        print("     these are what explain a cell count lower than the plan asked for.")
      end
    end
  end
end
print(sup:report())
-- Anything the loop did not drain, drained once at the end.
record.refusals()
record.enclosures()
record.deaths()

-- The observation window. Nothing is dispatched and nothing is fed by hand --
-- an idle roster over the window is half of what makes the analyser's
-- attribution verdict available at all, because `feed_actions == 0` is one of
-- its two conditions.
local observe_from = rcon.game_tick()
if observe_from == nil then
  print("no game tick available: skipping the observation window entirely rather")
  print("  than substituting a wall-clock or poll-count one, which is not a duration")
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
      print(string.format("  tick=%s (+%s into the window)",
        tostring(now), tostring(now - observe_from)))
      next_mark = next_mark + 3600
      record.keyframe()
    end
  end
  print("observation window closed at tick " .. tostring(rcon.game_tick()))
end

local id = record.finish("done")
print("RUN FINISHED id=" .. id)
print("")
print("THE RESULT IS NOT ON THIS PAGE. Read it with:")
print("  just analyse " .. id)
print("Judge it on iron-plate/min at minutes 5 and 10, and on the MACHINES AT")
print("FIXED MARKS table -- `mining-drill N / W` and `furnace N / W`, standing")
print("over working. Eighteen cells standing and none working is a FAILURE, and")
print("a plan that went green does not change that.")
print("end plate rate")
