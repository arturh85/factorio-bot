-- THE `factory` ATTRIBUTION VERDICT: a run built to earn one, or to say why not.
--
-- Every archived run in this project reads `roster-fed`. This script exists to
-- find out whether `factory` is reachable at all, and it is written against the
-- analyser's own decision rule rather than against a feeling about what a
-- factory looks like.
--
-- ---------------------------------------------------------------------------
-- WHAT THE ANALYSER ACTUALLY DECIDES, READ OFF ITS SOURCE
-- ---------------------------------------------------------------------------
--
-- `tools/run_analysis.py::attribute_from_counters` is the live path -- the
-- machines' own counters, arithmetic, no inference -- and it says `factory`
-- when, **for one mark interval and one item**:
--
--     machines' own `produced` delta / force's `production.made` delta >= 0.95
--     AND `feed_actions` == 0
--
-- `feed_actions` is the count of `insert`/`stock`/`charge`/`fuel`/`take`/`mine`
-- DISPATCHES whose tick falls in `(lo, hi]` -- `interval_activity`. A dispatch
-- before `lo` that settles inside does not count. So the window this run needs
-- is one that OPENS after the last feeding dispatch and CLOSES while the
-- machines are still making packs.
--
-- Nothing about power enters that verdict. The inference fallback
-- (`_attribute_by_inference`) weighs kW and machine statuses, and it is only
-- reached for a run archived before the counters existed. This run has them
-- (`mods/BotBridge/control.lua` writes `produced` / `produced_source` on every
-- machine row), so the verdict here is arithmetic.
--
-- ---------------------------------------------------------------------------
-- WHY SUCH A WINDOW EXISTS AT ALL, AND WHY IT IS EXACTLY 9,000 TICKS LONG
-- ---------------------------------------------------------------------------
--
-- `crates/planner/src/method/assemble.rs`: a cell's chests are charged **once**,
-- by hand, with `CELL_CHARGE_TICKS` (9,000) worth of ingredients, and nothing
-- refills them. That is stated there as a limit and it is exactly what makes
-- this measurement possible: the charge is the last feeding dispatch, and for
-- the 9,000 ticks after it the machines run with no bot touching them.
--
-- **And the charge is the LAST thing the plan does**, which is what makes the
-- window free rather than something this script has to carve out. Read off the
-- offline plan at 191c5955: the one-cell plan's final two actions are
-- `#20 charge the feed chest with 30 iron-plate` and `#21 charge the supply
-- chest with 15 copper-plate`, both at ticks 22,447 -> 22,457, and the plan's
-- makespan is 22,457. So the observation window opens on the same tick the last
-- feeding dispatch settles, and the charge's whole 9,000-tick life is inside it.
-- 30 iron-plate and 15 copper-plate is 15 gears and 15 packs -- exactly
-- `charge_products`, and exactly 6 a minute for 2.5 minutes.
--
-- Two and a half game minutes is the entire budget. It is why this run needs
-- FINE MARKS to read (see the closing message): the analyser's default marks
-- are five minutes apart, so every default interval that contains packs also
-- contains the charge that paid for them, and the verdict can only ever be
-- `roster-fed`. That -- not a missing capability -- is the likeliest reason no
-- archived run has read `factory`.
--
-- ---------------------------------------------------------------------------
-- WHY SIX A MINUTE AND NOT THIRTY
-- ---------------------------------------------------------------------------
--
-- `goal.producing(item, N)` is a RATE, per minute, and `cells_for` turns it
-- into `ceil(N * ticks_per_item / 3600)` cells. An `assembling-machine-1` runs
-- red science at 600 ticks a pack, so 6/min is ONE cell and 30/min is FIVE.
--
-- Six is chosen, and the reason is not thrift:
--
--   * **The verdict is qualitative.** `machine_made / delta >= 0.95` does not
--     care whether the delta is 6 or 75. One cell can earn it.
--   * **Half the actions and less than half the makespan**: 316 actions /
--     22,457 ticks against 484 / 52,336, both read off the offline plan on
--     `map-31337-t0.json` at 191c5955. That is the failure surface between the
--     run and the window it exists to hold.
--   * **Power headroom, and the coal cap in particular.** A cell is two
--     `assembling-machine-1` at 75 kW plus four inserters at 13 kW = 202 kW.
--     One cell on the 900 kW plant the plan builds is 4.5x headroom; five cells
--     is 1,010 kW on the 1,800 kW plant (two engines) the plan builds for them,
--     1.78x. The binding number is not the nameplate but the FUEL: `boiler_coal`
--     is capped at one stack because a boiler's fuel inventory is one slot, and
--     the five-cell plan already asks for 10 + 38 = 48 coal of the 50 available.
--     The one-cell plan asks for 5 + 8 = 13. An under-supplied network reads as
--     completely DEAD rather than slow, so the tight one puts a power failure on
--     the critical path of a measurement that is not about power.
--
-- **One argument for six that was CHECKED AND IS FALSE**, kept because it is
-- the plausible one: that five cells would be charged in sequence, so cell 1
-- would be dead before cell 5 was charged and the clean window would hold fewer
-- than five live cells. The plan says otherwise -- all five `charge` pairs land
-- between ticks 52,237 and 52,336, a 99-tick spread at the very end. Five cells
-- would give a 5x larger delta over the same window. The case for six rests on
-- the two bullets above, not on this one.
--
-- The 30/min run is a rate claim and a fine second experiment. This one is an
-- attribution claim, and they are different questions.
--
-- ---------------------------------------------------------------------------
-- THE WITNESS RUNG IS KEPT, AND IT COSTS ALMOST NOTHING
-- ---------------------------------------------------------------------------
--
-- `supervisor.witness` dispatches no actions, so it adds no feeding dispatch
-- and cannot spoil the clean window. What it buys is the difference between
-- the two ways this run can produce no verdict: a cell that never worked, and
-- a window that missed a cell that did. A healthy cell satisfies it in about
-- 700 ticks; only a dead one pays the whole `within_ticks`.
--
-- ---------------------------------------------------------------------------
-- EVERY WINDOW HERE IS BOUND IN GAME TICKS
-- ---------------------------------------------------------------------------
--
-- Never in poll counts and never in wall time. `for _ = 1, 5000` polls is not a
-- duration: on a faster box more game ticks pass per RCON round trip, so the
-- same poll count covers more game time. A block compared that way was
-- published at 4.6x and re-measured at 2.6x on a fixed tick window. If
-- `rcon.game_tick()` cannot answer, the window is SKIPPED rather than
-- substituted with a wall clock -- a wall-clock window would not be the thing
-- this script claims to have held.
--
-- ---------------------------------------------------------------------------
-- WHAT THIS SCRIPT CLAIMS
-- ---------------------------------------------------------------------------
--
-- Nothing about attribution. It builds a cell, witnesses that packs reach the
-- chest with every bot idle, and then holds the game open, idle and
-- tick-bounded, so the sampling session covers the charge's whole life. The
-- verdict is `just analyse`'s, and a `unclear` is a result -- a wrong confident
-- label is worse than an honest one.

include("supervisor.lua")

print("start factory verdict")

-- Packs a minute. Six is one cell; see the header for why not thirty.
local PER_MINUTE = 6

-- How long to hold the game open, idle, after the ladder finishes. GAME ticks.
--
-- `CELL_CHARGE_TICKS` is 9,000, so a cell charged at the end of the build is
-- dead 9,000 ticks later. 18,000 is deliberately twice that: the first half
-- covers the charge's life, which is the window the verdict can come out of,
-- and the second half is the flat part -- which is not waste. A plateau the
-- analyser can see is what turns "the cell stopped" into "the cell stopped
-- when its charge ran out", and the plateau detector needs the flat stretch to
-- detect one. At `--game-speed 10` the whole window is about 30 seconds of
-- wall clock.
local OBSERVE_TICKS = 18000

-- The witness. `within_ticks` has no default in the library, deliberately, so
-- it is derived here from the cell's own arithmetic rather than guessed:
-- `smelting_ticks` for the gear machine is 60 and for the pack machine 600, so
-- the first pack is 660 ticks away plus three inserter swings and every pack
-- after it is 600. Five packs is therefore ~3,060 ticks of work; 3,600 is that
-- plus a margin, and `at_least = 5` is one MORE than the four crafts an
-- undrained machine manages -- so a cell whose output inserter is missing or
-- backwards cannot satisfy this however long it waits.
local WITNESS_AT_LEAST = 5
local WITNESS_WITHIN_TICKS = 3600

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("roster: " .. tostring(#bots) .. " bot(s)")

local run_id = record.start({ video = false })
print("recording run " .. run_id)
-- Started before anything is planned and never stopped early: this session is
-- what writes `samples.jsonl`, and the machine rows in it -- `produced`,
-- `produced_source`, `recipe`, `status` -- are the ONLY input to the verdict
-- this run is about. A session that ended at the last closed milestone once
-- lost 199,449 ticks, the entire window the thing being measured existed in.
rcon.sampling_start(run_id)

-- The ladder. `researched("automation")` is not a rung of its own: the cell
-- needs `assembling-machine-1` and drags the research in, exactly as
-- `factory_starter.lua` does.
local goals = {
  goal.producing("automation-science-pack", PER_MINUTE),
  supervisor.witness {
    item = "automation-science-pack",
    -- The two ends of the link whose terminal holds the packs. The OUTPUT
    -- CHEST, not the machine: a cell with a drain attached keeps its machine's
    -- output slot at zero while it runs happily, so watching the machine would
    -- halt a healthy cell. Measured on a bench -- 94 packs made, output slot 0,
    -- status `working`.
    from = "inserter",
    into = "iron-chest",
    -- The planner chose the cell's site and this script never learns it, so
    -- the witness sweeps.
    near = { x = 0, y = 0 },
    radius = 300,
    at_least = WITNESS_AT_LEAST,
    within_ticks = WITNESS_WITHIN_TICKS,
  },
}

-- Index for index with `goals`: a mismatch would label a failure with the
-- wrong step.
local names = {
  "a red-science cell producing " .. PER_MINUTE .. "/min",
  "witness: " .. WITNESS_AT_LEAST .. " red packs reach the output chest in "
    .. WITNESS_WITHIN_TICKS .. " ticks with every bot idle",
}

-- Who the game has RIGHT NOW, asked before every plan. `nil` back means "could
-- not be checked", which the supervisor reads as "keep the roster" -- never as
-- "everyone is dead".
local function current_roster()
  local ok, ids = pcall(function() return rcon.players() end)
  if ok and type(ids) == "table" then return ids end
  return nil
end

local sup = supervisor.new(supervisor.list(goals),
  { bots = bots, stall_limit = 3, max_iterations = 25, roster = current_roster })

-- Whether the cell rung ever dispatched anything, and whether the last run of
-- it settled everything it dispatched. `obs.done` is the executor judging that
-- no further progress is possible, NOT that the plan completed, so `pending`
-- is the part that makes the claim true.
local cell_runs = 0
local dispatched_everything = false
local witnessed = nil

local ok, err = pcall(function()
repeat
  local t = sup:step()
  if t.action == "planned" and type(t.plan) == "table" then
    record.plan_created(t.milestone_index, t.plan, t.bots)
  end
  if t.action == "acquired" then
    record.milestone_started(t.milestone_index, names[t.milestone_index] or "?")
    print("-> " .. (names[t.milestone_index] or "?"))
  elseif t.action == "rerostered" then
    local nd = record.deaths()
    record.roster_changed(t.bots, t.left, t.returned, t.reason)
    print("   ROSTER CHANGED: now {" .. table.concat(t.bots, ", ") .. "}"
      .. " -- " .. tostring(t.reason)
      .. (nd > 0 and (" (+" .. nd .. " death/respawn events)") or ""))
  elseif t.action == "planned" then
    print("   planned " .. t.steps .. " steps (best " .. tostring(t.best) .. ")"
      .. (t.recovery and (" -- recovered: " .. t.recovery
        .. " " .. tostring(t.recoveries)) or ""))
  elseif t.action == "ran" then
    if t.milestone_index == 1 then cell_runs = cell_runs + 1 end
    local n = 0
    if t.steps ~= nil and t.actions ~= nil then n = record.actions(t.steps, t.actions) end
    local nw = 0
    if type(t.walks) == "table" then nw = record.walks(t.walks) end
    local nt = record.teleports()
    local nr = record.refusals()
    local ne = record.enclosures()
    if ne > 0 then print("   WALLED IN: " .. ne .. " bot(s) can no longer reach open ground") end
    local nd = record.deaths()
    if nd > 0 then print("   DEATHS/RESPAWNS: " .. nd .. " event(s) -- see the roster line") end
    local nrt = record.research_triggers()
    if nrt > 0 then print("   trigger technologies emulated: " .. nrt) end
    print(string.format(
      "   ran: success=%s pending=%s actions(failed=%s lost=%s) walks(failed=%s lost=%s) (+%d events, +%d walk events, +%d teleports, +%d refusals)",
      tostring(t.success), tostring(t.pending),
      tostring(t.failed), tostring(t.lost),
      tostring(t.walks_failed), tostring(t.walks_lost), n, nw, nt, nr))
    if t.first_error ~= nil then print("        first error: " .. tostring(t.first_error)) end
    if t.not_recovered ~= nil then
      print("        not recovered: " .. tostring(t.not_recovered))
    end
    if t.milestone_index == 1 then
      dispatched_everything = t.done == true and (t.failed or 0) == 0
        and (t.lost or 0) == 0 and (t.pending or 0) == 0
    end
  elseif t.action == "satisfied" then
    record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
    if type(t.witness) == "table" then
      witnessed = t.witness
      print(string.format(
        "   WITNESSED: %s in %d watched container(s) went %d -> %d (+%d, wanted %d) in %d of %d ticks, %d polls",
        tostring(t.witness.item), t.witness.watched, t.witness.before,
        t.witness.after, t.witness.gained, t.witness.at_least,
        t.witness.elapsed_ticks, t.witness.within_ticks, t.witness.polls))
    else
      print("   SATISFIED (" .. tostring(t.reason) .. ")")
    end
  elseif t.action == "halted" then
    local why = (t.refusal and t.refusal.message) or sup.first_error
    record.milestone_stuck(t.milestone_index, t.state, why, t.best)
    print("   HALTED: " .. t.state
      .. (t.refusal and (" -- " .. tostring(t.refusal.code) .. ": "
        .. t.refusal.message) or ""))
  end
until sup:finished()
end)

if not ok then
  print("RAISED: " .. tostring(err))
  record.milestone_stuck(sup.index, "plan_error", tostring(err),
    sup.tracker and sup.tracker.best)
end

-- Final flushes, BEFORE the observation window rather than after it: each of
-- these is a queue the mod fills and the record drains, and a run that died on
-- its last placement is precisely the run whose refused site or frozen bot
-- somebody will want to look up.
record.teleports()
record.refusals()
record.enclosures()
record.deaths()
record.research_triggers()

if cell_runs == 0 then
  print("NOTHING WAS DISPATCHED FOR THE CELL: no plan of it ever ran.")
  print("  That is NOT a failed attribution -- it is a refusal or an empty")
  print("  plan, and the HALTED line above carries which. Do not read any")
  print("  production number below as evidence of this run's work.")
end

-- ---------------------------------------------------------------------------
-- THE OBSERVATION WINDOW
-- ---------------------------------------------------------------------------
--
-- Nothing is dispatched here and nothing is fed by hand. An idle roster over a
-- window that opens after the last feeding dispatch is the entire mechanism by
-- which a `factory` verdict is available at all: it is what makes
-- `feed_actions == 0` true for some interval in which packs are still being
-- made. The tick this window opens at is printed, because it is the number
-- whoever picks `--marks` needs.
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
      print(string.format("  tick=%s (+%s into the window)", tostring(now),
        tostring(now - observe_from)))
      next_mark = next_mark + 3600
      record.keyframe()
    end
  end
  print("observation window closed at tick " .. tostring(rcon.game_tick()))
end

local state = ok and sup.state or "crashed"
local id = record.finish(state)
print("RUN FINISHED state=" .. state .. " id=" .. id)
if dispatched_everything then
  print("EVERY PLANNED ACTION SETTLED in the last run of the cell rung.")
  print("  This says the CELL STANDS and was charged. It does NOT say who made")
  print("  the packs -- that is the analyser's verdict, not this line.")
end
if witnessed ~= nil then
  print("THE CELL WAS ALIVE: packs reached the output chest with every bot idle.")
  print("  So a `no output` verdict below would be a window that missed a working")
  print("  cell, not a cell that never worked. Those are different findings.")
end
print(sup:report())
print("")
print("THE RESULT IS NOT ON THIS PAGE. Read it with:")
print("  just analyse " .. id .. " --marks <fine marks>")
print("")
print("AND THE MARKS MATTER, WHICH IS UNUSUAL AND IS THE POINT. The default")
print("marks are 5 game minutes apart. A cell's whole charge is 9,000 ticks --")
print("2.5 minutes -- so EVERY default interval containing packs also contains")
print("the `charge` dispatch that paid for them, and `feed_actions` can never be")
print("zero in one. Read this run at 1-minute marks, which is the granularity")
print("the charge's life admits:")
print("  just analyse " .. id .. " --marks 1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30")
print("")
print("The claim is `factory` for automation-science-pack in an interval lying")
print("wholly inside the observation window above. `unclear` and `no output` are")
print("real answers and are to be reported as such, with the analyser's own")
print("reason beside them.")
print("end factory verdict")
