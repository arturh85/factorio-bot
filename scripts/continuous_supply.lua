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
-- The cell was draining a chest a bot had filled. `CELL_CHARGE_TICKS` was
-- 9,000 ticks and nothing refilled it. So that verdict says "a machine ran
-- unattended for two and a half minutes", which is automation in the narrowest
-- sense and not a factory.
--
-- `3dadc04e` belted a standing stage-1 cell into the assembly cell's supply
-- chest and kept the hand charge as ignition; the cleanest run of the night
-- (`run-1788954737-06011`) still plateaued at 15 packs, because the IRON side
-- of the cell was still a chest a bot filled once.
--
-- # No chests in the line (2026-09-09)
--
-- Owner: *"have no chests ... inside a factory every chest would be a huge
-- bottleneck because the slow inserters at the beginning are way slower than
-- a belt."* Since then a red cell has no chest but the output one: both of
-- its plates are belted straight into the machine that eats them, from a
-- standing stage-1 cell each, and a `producing` goal with no such source is
-- REFUSED BY NAME (`assembly_no_standing_source`) rather than planned as a
-- cell that dies. So this script composes BOTH sustains ahead of the cell:
-- `goal.all` expands in order, and the cell is sited from the two plate
-- chests it is belted from. The rates are the cell's own demand -- two iron
-- plates and one copper plate a pack, six packs a minute.
--
-- # What would settle it
--
-- **`factory` across an interval with zero feeding dispatches, and NO PLATEAU
-- after minute ~10** -- where the charge-fed cell died. The window is 45,000
-- ticks so total game time passes 25 minutes: the 9:48 cell expired at about
-- two charges, so anything shorter cannot distinguish "the belts work" from
-- "the charge was bigger this time". There is no charge now, so a plateau at
-- fifteen would be a defect in the belts, not in a constant.
--
-- Two readings that mean different things, and the record can tell them apart:
-- a plateau with `no_ingredients` on the assembling machine means a belt is
-- not delivering (read which mouth's arm is idle); a plateau with the pack
-- machine on `full_output` and the lab full means the research finished and
-- nothing else was queued -- output-bound, which is the plan's shape and not
-- a bug in the line.
--
-- # Judged on rate and census, not makespan
--
-- Price it offline first: `factorio-bot plan --world workspace/scripts/map.json
-- --goal sustain:iron-plate:12:36000 --goal sustain:copper-plate:6:36000
-- --goal researched:logistics --bots 1,2,3,4`. One risk stated
-- in advance: each run's load arm sits at a burner cell with no network and
-- gets a wire of its own, and poles cost wood, so a wood shortfall refuses BY
-- NAME (`Have 1 wood`) rather than silently.

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

-- Both plates, in the order the cell's mouths take them. The sustains go
-- first because `goal.all` expands in order and the cell refuses without a
-- standing source for each.
--
-- **The cell's output goes into a lab, and the lab researches** (phase 3,
-- 2026-09-09): a research whose pack a standing-sourced cell makes is FED BY
-- THE CELL -- the cell is built inline with a lab as its sink, no pack is
-- crafted, carried or inserted by anybody, and the research waits on the
-- cell. `logistics` is twenty units of red at 900 ticks each, which one lab
-- burns at four packs a minute against the cell's six: one lab, the chain
-- length the rate supports (`assemble::labs_fed_by`). So the goal is the
-- research, not `producing` -- a `producing` goal alone would build the same
-- cell and lab and leave the packs to pile up in a lab nobody set to work.
local RESEARCH = "logistics"
local sup = supervisor.new(
  supervisor.list { goal.all { goal.sustain("iron-plate", 2 * PER_MINUTE, 36000),
                               goal.sustain("copper-plate", PER_MINUTE, 36000),
                               goal.researched(RESEARCH) } },
  { bots = bots, stall_limit = 3, max_iterations = 10, roster = current_roster }
)

print(string.format("goal: sustain iron-plate %d + sustain copper-plate %d + researched:%s (fed by the cell)",
  2 * PER_MINUTE, PER_MINUTE, RESEARCH))

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
    elseif t.action == "satisfied" then
      -- Closes the milestone in the record. Without this the analyser reads
      -- `OPEN (never ended; scored to the last recorded tick)` for every
      -- milestone of every run this driver has made, which is what it did.
      record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
      print("   SATISFIED (" .. tostring(t.reason) .. ")")
    elseif t.action == "halted" then
      -- **This branch is why three runs could not say why they stopped.**
      -- `record.refusals()` sounds like it covers this and does not: it drains
      -- `world.unreported_placement_refusals()`, i.e. refusals the GAME handed
      -- down, and reported `+0` in every run while the milestone was refusing
      -- by name the whole time. Two different things wearing one word.
      --
      -- `t.refusal` is the PLANNER's verdict about the world ("a cell already
      -- makes copper-plate ... and nothing can carry it to the supply chest"),
      -- not an action that failed. It leads, because it is what closed the
      -- milestone; `sup.first_error` may also be set from an earlier run in the
      -- same milestone, and that is a different fact.
      local why = (t.refusal and t.refusal.message) or sup.first_error
      record.milestone_stuck(t.milestone_index, t.state, why, t.best)
      print("   HALTED: " .. tostring(t.state)
        .. (t.refusal and (" -- refused: " .. t.refusal.message) or ""))
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
