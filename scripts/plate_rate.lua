-- The plate-rate milestone: stand enough drill-and-furnace cells to make iron
-- plate at a target rate, and let `just analyse` say what actually happened.
--
-- # Why this run exists
--
-- Two world-record Space Age runs place **73 burner mining drills by minute
-- 10** and never another, reaching 261 / 390 / 840 iron-plate per minute at
-- minutes 5 / 10 / 15. Across all 78 archived runs of this project we had
-- dispatched **168 drill placements in total**, best single run **11**, and
-- reached 51 / 67 / 13-27. `producing:iron-plate:261` used to refuse
-- (`TooManyCells`); it now plans 178 actions standing 18 cells.
--
-- So this run asks one question: **does standing eighteen cells actually
-- produce plate, or do they stand idle like the 27 furnaces a census caught
-- reading `0 working` at minute 20?**
--
-- # What would make it a success, and what would not
--
-- Not the makespan. The owner's standing metric is **production rate at fixed
-- game-time marks**, and the census that landed alongside it reports machines
-- **standing AND working** at those same marks. A run that stands 18 cells and
-- reads `mining-drill 18 / 0` has failed, however green its plan.
--
-- # The prediction this run exists to falsify
--
-- The agent that raised `MAX_CELLS` predicted: this reads `roster-fed` at
-- every mark, and **decays after ~36,000 ticks** when `CELL_FUELLED_TICKS`
-- expires, with nothing detecting it. **If iron/min does not fall between
-- minute 10 and minute 15, that fuel model is wrong.** Say so if it does not.
--
-- # Why there is no `supervisor.witness` here
--
-- A witness dispatches nothing and asserts something happened anyway, which is
-- the right shape for a *cell* whose output lands in one chest. Plate arrives
-- in eighteen furnaces' own output slots, so the thing to read is the
-- production curve and the machine census, not one container. The observation
-- window below is what makes both legible.

-- `include` returns nothing and defines the global `supervisor`, the same idiom
-- as `lib.lua` and the `goal` / `world` / `rcon` tables. Omitting it fails at
-- the first `supervisor.` with "attempt to index a nil value", which is how
-- this line came to be here.
include("supervisor.lua")

local PER_MINUTE = 261

-- Long enough to cross the fuel-decay boundary the prediction above names.
-- `CELL_FUELLED_TICKS` is ~36,000 and the plan's own makespan is ~42,800, so a
-- window of 18,000 puts the last mark well past both. Bounded in GAME TICKS,
-- never a poll count: on a faster box more ticks pass per RCON round trip, and
-- a block once published at 4.6x re-measured at 2.6x on a fixed tick window.
local OBSERVE_TICKS = 18000

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
  supervisor.list { goal.producing("iron-plate", PER_MINUTE) },
  { bots = bots, stall_limit = 3, max_iterations = 10, roster = current_roster }
)

print(string.format("goal: producing:iron-plate:%d", PER_MINUTE))

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
      record.milestone_started(t.milestone_index, "iron-plate rate")
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
