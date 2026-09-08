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
-- WHY THIS RUNS ON THE SUPERVISOR (2026-09-08)
-- ---------------------------------------------------------------------------
--
-- Until now this script called `goal.run(plan)` once, raw, and went straight
-- to `record.milestone_stuck` on anything short of a clean finish. It was the
-- ONLY milestone script in `scripts/` that did: nine others -- factory_stage1
-- through 3, factory_starter, automation_speedrun, selffed_run, sustain_run,
-- research_run, pollution_watch -- all drive `supervisor.lua`. So the run
-- least likely to finish its first plan was the one with no way to continue
-- one.
--
-- That is not a theoretical gap. The single live attempt died after 246 of
-- 2,295 actions. Across 21 archived runs, **38 of 57 replans had a trigger
-- tier 1 is designed for**, and a walk is the dominant failure -- 76 failed
-- walks against 28 failed/lost actions, with 28 of 57 replans showing no
-- action failure at all. A plan of this size that throws itself away on one
-- failed walk has thrown away the whole run.
--
-- What the loop is used for and what it is NOT used for:
--
--   * The recovery, the stall detector, the iteration cap and the re-roster
--     are the library's. None of the five recovery refusals is reimplemented
--     here -- reexpanded, any lost action, the per-lineage limit, "a recovery
--     that adds no successes is not a recovery", and a divergence refused
--     without asking. Each was bought with archived runs and hand-rolling
--     `obs:recover()` into a driver would reproduce the bugs they encode.
--   * The observation window below is deliberately NOT `supervisor.sustain`.
--     That rung answers a rate question this milestone does not ask, closes
--     its own milestone in the record, and carries a poll budget. What is
--     wanted here is exactly what was already here: hold the game open,
--     tick-bounded, so the samples cover the period petroleum would appear
--     in. It is kept verbatim.
--
-- The re-roster (`roster = current_roster`) is not decoration on this
-- particular run. Crude oil on seed 31337 is 372.5 tiles out and the survey
-- that reached it revealed 32 enemy structures; the one live attempt lost two
-- bots, to a small-biter at tick 27,065 and a small-worm-turret at 53,619.
-- Without a re-roster every plan after a death dispatches work to a bot the
-- game refuses, action by action, with nothing in the record saying why.
--
-- **Nothing in the planner models a threat.** The re-roster makes a death
-- survivable, not planned for. Whether oil is a t=0 target on this seed at
-- all is an open owner decision.
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
-- **And this milestone closes `stuck`, on the record, even when every action
-- settles.** `goal.holds` answers `nil` for both `Goal::Gathered` and
-- `Goal::Produced` (`crates/planner/src/method/have.rs`) -- one names a fluid
-- arrangement `PlanState` does not model, the other names an *event* no
-- inventory read settles -- so the supervisor closes the milestone
-- `supervisor::unanswerable` rather than claiming it. That is the honest
-- answer and it agrees with the paragraph above: the milestone is the
-- production samples plus the attribution, and neither is a thing a script
-- inside the game can read.
--
-- It is also a change from what this file used to do, and the change is a
-- repair. The old success path called
-- `record.milestone_satisfied(1, 1, "plan_complete")`, and `plan_complete` is
-- not a `SatisfiedReason`: `parse_satisfied_reason` accepts
-- `already_satisfied` and `plan_empty` and refuses everything else by name.
-- So the ONE path where the oil run went perfectly would have raised inside
-- `record.milestone_satisfied` -- before the observation window and before
-- `record.finish`, i.e. it would have lost the samples the milestone is
-- defined against. It had never been executed, because no run has ever
-- finished this plan.
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
-- intact rather than dying at "unknown goal kind".
--
-- The refusal now arrives as a supervisor `halted` transition carrying
-- `t.refusal` -- the same classification `goal.refusal` made before, on the
-- channel every other driver already reads -- and is recorded as a stuck
-- milestone with the planner's own sentence. A refusal is a fact about the
-- world, not a defect, and recovery cannot recover a plan that never
-- schedules.

include("supervisor.lua")

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

-- The supervisor's cap on how many times this milestone may be PLANNED. It is
-- a plan count, not an action count and not a recovery count: one iteration is
-- one `goal.plan` expansion plus the run of it, so a 2,295-action plan spends
-- exactly one. Tier-1 recoveries are deliberately outside it (`recovery_limit`
-- 2 per lineage, so a plan is executed at most three times), which is what
-- makes continuing a plan cheap and re-planning expensive, and is the whole
-- reason this script now has both.
--
-- 10 is the project default for a milestone driver. It is not derived from
-- this plan's size, and it cannot be: no oil run has ever produced a
-- step-count sequence to derive one from. The evidence that would move it is
-- the sequence itself -- `factory_starter` raised its cap to 25 on exactly
-- that basis, after stage 3 halted `exhausted` with steps still falling
-- 456, 465, 392, 330, 294, 285. If this run halts `exhausted` while its
-- printed step counts are still falling, the cap is the binding constraint
-- and that run is the argument for raising it. If it halts `stuck` on the
-- stall detector instead, the cap was never in play.
local MAX_ITERATIONS = 10

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
-- two halves -- the rig itself -- is planned once. It is also ONE milestone
-- in the supervisor's ladder for the same reason: two rungs would plan the
-- rig twice.
local milestone = goal.all {
  goal.gathered("crude-oil"),
  goal.produced("petroleum-gas", PETROLEUM_COUNT, { via = "basic-oil-processing" }),
}
print("goal: " .. tostring(milestone))

-- Must stay aligned with the single-goal list below: this string is what the
-- record shows for the milestone.
local MILESTONE_NAME = "oil: crude gathered and " .. PETROLEUM_COUNT
  .. " petroleum-gas produced via basic-oil-processing"

-- Who the game has RIGHT NOW, asked before every plan and before a tier-1
-- recovery is accepted. `nil` back means "could not be checked", which the
-- supervisor reads as "keep the roster" -- never as "everyone is dead".
local function current_roster()
  local ok, ids = pcall(function() return rcon.players() end)
  pcall(function() rcon.inventory_contents_at({}) end)
  if ok and type(ids) == "table" then return ids end
  return nil
end

-- Forward-declared: the chart source's probe closes over it, so that the
-- probe is planned for the roster the supervisor currently holds.
local sup

-- ---------------------------------------------------------------------------
-- THE RINGS: how this run is allowed to FIND the oil
-- ---------------------------------------------------------------------------
--
-- On a fresh seed-31337 map this milestone refused in **one second**:
--
--     planner::not_charted: no crude-oil is charted anywhere this plan can
--     see ... charted ground covers 17 of 17 probes within 256 tiles
--
-- and that refusal is correct. Crude oil on this seed is 372.5 tiles out; a
-- fresh map is generated to +/-320, so it is outside by 52. Every offline plan
-- of this milestone ran against a dump of an already-explored world.
--
-- `supervisor.chart_until` is the loop that closes it: plan the milestone,
-- and while it refuses `planner::not_charted`, walk one more lattice ring and
-- ask again. The stop condition is the **refusal ceasing**, never a sighting
-- -- this script does not name crude oil to the search, and the search does
-- not read a resource position. Rings are concentric on the spawn, blind, and
-- widen by exactly one lattice pitch. Nothing here uses foreknowledge: what
-- the run knows about where the oil is, it learnt by walking there.
--
-- The bound is three rings (896 tiles). Exhaustion is not silence: the source
-- issues the milestone anyway, so the run ends on the planner's own
-- not-charted sentence in the record, and `chart.reason` says `exhausted`
-- rather than `plannable`.
local chart = supervisor.chart_until {
  target = milestone,
  -- The probe is made against the roster as it STANDS, not as the run
  -- started. Bots die on this map -- two did, on the one live attempt -- and
  -- a probe planned for a bot the game has no character for would refuse for
  -- a reason that has nothing to do with charted ground.
  plan_opts = function() return { bots = (sup and sup.bots) or bots } end,
}

sup = supervisor.new(chart.source,
  { bots = bots, stall_limit = 3, max_iterations = MAX_ITERATIONS,
    roster = current_roster })

-- Whether a run of this milestone ever dispatched anything, and whether the
-- last one settled everything it dispatched.
--
-- `obs.done` is the executor judging that no further progress is possible,
-- NOT that the plan completed -- so `done and failed == 0 and lost == 0`
-- would report a satisfied milestone over actions that were never dispatched.
-- `pending == 0` is the part that makes the claim true. Carried on the driver
-- rather than asked of the supervisor because the supervisor's verdict is
-- about the GOAL (unanswerable, above) and this is about the PLAN.
local runs = 0
local target_runs = 0
local dispatched_everything = false

-- The loop is wrapped so a raise still closes the recording. A run that died
-- part-way is the one most worth opening, and it is no use if it never got a
-- manifest, splits or its samples copied out of the workspace.
local ok, err = pcall(function()
repeat
  local t = sup:step()
  -- `t.plan` rides on both the "planned" and "satisfied" transitions: the
  -- planner returned in both cases. Only "planned" is recorded -- the
  -- "satisfied" transition's `t.plan` is the empty table from the re-plan that
  -- found nothing left to do, and a consumer taking the LAST `plan_created`
  -- per milestone must see the real DAG rather than have it shadowed.
  if t.action == "planned" and type(t.plan) == "table" then
    record.plan_created(t.milestone_index, t.plan, t.bots)
  end
  if t.action == "acquired" then
    -- One name per milestone, asked of the source: the ladder is no longer a
    -- single rung, and a run whose record called three surveys and a rig by
    -- one name would be unreadable.
    local name = chart:name_of(t.milestone_index) or MILESTONE_NAME
    record.milestone_started(t.milestone_index, name)
    print("-> " .. name)
  elseif t.action == "rerostered" then
    -- A bot has had no character past the supervisor's respawn wait and was
    -- dropped, or one dropped earlier is back. Recorded before the plan that
    -- follows, so the `plan_created.bots` that shrinks has the line explaining
    -- it directly above. On this run that is the biter answer, and it is the
    -- only one there is.
    local nd = record.deaths()
    record.roster_changed(t.bots, t.left, t.returned, t.reason)
    print("   ROSTER CHANGED: now {" .. table.concat(t.bots, ", ") .. "}"
      .. (#t.left > 0 and (" left: " .. table.concat(t.left, ", ")) or "")
      .. (#t.returned > 0 and (" returned: " .. table.concat(t.returned, ", ")) or "")
      .. " -- " .. tostring(t.reason)
      .. (nd > 0 and (" (+" .. nd .. " death/respawn events)") or ""))
  elseif t.action == "planned" then
    -- `t.recovery` is present when this "plan" is not a plan at all but a
    -- tier-1 recovery of the one before it: the same plan minus what already
    -- succeeded, re-scheduled against the world as it now is, with no
    -- expansion done and the tracker deliberately untouched. Printed because
    -- until `PlanCreated` carries a cause, this line is the only place a
    -- reader can tell two consecutive plans of one milestone apart -- and on a
    -- 2,295-action plan it is the difference between continuing a run and
    -- starting it again.
    print("   planned " .. t.steps .. " steps (best " .. tostring(t.best) .. ")"
      .. (t.recovery and (" -- recovered: " .. t.recovery
        .. " " .. tostring(t.recoveries)) or ""))
    -- Per-bot planned steps, off the plan the record was given, so the shares
    -- are the ones the planner actually made rather than a second derivation.
    local per_bot = {}
    for _, s in ipairs(t.plan or {}) do
      if type(s) == "table" and s.bot ~= nil then
        per_bot[s.bot] = (per_bot[s.bot] or 0) + 1
      end
    end
    for _, b in ipairs(t.bots or {}) do
      print(string.format("     bot %s: %s planned step(s)", tostring(b),
        tostring(per_bot[b] or 0)))
    end
  elseif t.action == "ran" then
    runs = runs + 1
    -- Kept apart from `runs`: a survey ring that settles every walk it
    -- dispatched is not the rig standing, and one counter for both would
    -- report the charting as the milestone.
    if chart.target_index ~= nil and t.milestone_index == chart.target_index then
      target_runs = target_runs + 1
    end
    local n = 0
    if t.steps ~= nil and t.actions ~= nil then n = record.actions(t.steps, t.actions) end
    -- The walks of the same batch. A separate call, not a third argument to
    -- the one above, because a walk is not an action: it has no action id and
    -- it is in neither `steps` nor `actions`. On a 372-tile trek to the oil
    -- field, walking is most of the run.
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
    -- Grouped, because the four trouble counts are two axes: `failed` and
    -- `lost` name the game saying no versus the game never answering, and the
    -- two walk terms name the same distinction for walks. Printed ungrouped
    -- once, they made one lost action read as two problems.
    print(string.format(
      "   ran: success=%s pending=%s actions(failed=%s lost=%s) walks(failed=%s lost=%s) (+%d events, +%d walk events, +%d teleports, +%d refusals)",
      tostring(t.success), tostring(t.pending),
      tostring(t.failed), tostring(t.lost),
      tostring(t.walks_failed), tostring(t.walks_lost), n, nw, nt, nr))
    if t.first_error ~= nil then print("        first error: " .. tostring(t.first_error)) end
    if t.not_recovered ~= nil then
      -- Which rule declined a continuation, so a replan of a 2,295-action plan
      -- is never unexplained. `lost`, `divergence`, `limit`, `no_progress`,
      -- `reexpanded`, `no_plan`, `raised` -- each is a refusal bought with
      -- archived runs, not a failure of this script.
      print("        not recovered: " .. tostring(t.not_recovered))
    end
    dispatched_everything =
      chart.target_index ~= nil and t.milestone_index == chart.target_index
      and t.done == true and (t.failed or 0) == 0 and (t.lost or 0) == 0
      and (t.pending or 0) == 0
  elseif t.action == "satisfied" then
    record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
    print("   SATISFIED (" .. tostring(t.reason) .. ")")
  elseif t.action == "halted" then
    -- `t.refusal` leads when present: it is what closed the milestone -- a
    -- planner verdict about the world ("no charted crude oil", the chain
    -- ownership error above), or `supervisor::unanswerable`, which is what a
    -- COMPLETED oil milestone looks like from inside the game (see the header).
    -- `sup.first_error` is the first failed action's own text, and is a
    -- different fact.
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
  -- Recorded as a stuck milestone rather than silently: the run reached this
  -- goal and could not carry it, which is a fact about the run. `err` is the
  -- text this pcall caught -- discarded, it would leave `last_error: null` as
  -- the only trace of a crash the record was supposed to explain.
  record.milestone_stuck(sup.index, "plan_error", tostring(err),
    sup.tracker and sup.tracker.best)
end

-- Final flushes, before the observation window rather than after it: each of
-- these is a queue the mod fills and the record drains, and a run that died on
-- its last placement is precisely the run whose refused site, frozen bot or
-- dead character somebody will want to look up.
record.teleports()
record.refusals()
record.enclosures()
record.deaths()
record.research_triggers()

-- **Nothing dispatched is not a satisfied milestone.** `furnace_run.lua`
-- carries this guard and the scar behind it: a run once reported milestone 1
-- satisfied with 89 of 179 entities standing. Zero runs means the planner
-- refused, or returned nothing to do, and neither of those is "the rig was
-- built by this run" -- so it is said out loud rather than left to be inferred
-- from a production sample below.
-- What the widening did, before anything is said about the milestone. It is
-- printed whatever happened, because "how much ground did this run have to
-- look at" is half of what the run costs and is not derivable from anything
-- else in this output.
print(string.format("CHARTING: %d ring(s) walked%s, %d probe expansion(s), stopped because: %s",
  chart.rings,
  (#chart.radii > 0) and (" (radii " .. table.concat(chart.radii, ", ") .. ")") or "",
  chart.probes, tostring(chart.reason)))
if chart.exhausted then
  print("  THE BOUND WAS REACHED: " .. chart.rings .. " ring(s) of charting did not")
  print("  make this milestone plannable. That is a statement about this map and")
  print("  this bound -- not about the planner, and not about the oil. The HALTED")
  print("  line above carries where charted ground ended.")
end

if target_runs == 0 then
  print("NOTHING WAS DISPATCHED FOR THE MILESTONE ITSELF: no plan of it ever ran.")
  print("  (Charting rings, above, are a different milestone and do not count.)")
  print("  That is NOT the milestone. Either the planner refused (the HALTED")
  print("  line above carries its reason), or it found nothing to do -- check")
  print("  goal.holds and the world before reading any production sample as")
  print("  evidence of this run's work.")
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

-- The supervisor's own verdict, not a second one invented here. `stuck` with
-- `supervisor::unanswerable` is what a COMPLETED oil milestone reads as (see
-- the header); `dispatched_everything` below is the separate claim about the
-- plan, and the two are printed apart because they are different facts.
local state = ok and sup.state or "crashed"
local id = record.finish(state)
print("RUN FINISHED state=" .. state .. " id=" .. id)
if dispatched_everything then
  print("EVERY PLANNED ACTION SETTLED in the last run of this milestone.")
  print("  This says the RIG WAS BUILT. It does NOT say petroleum was produced,")
  print("  and it does not say by whom -- the production samples and the")
  print("  analyser's attribution verdict answer that, not this line.")
elseif target_runs > 0 then
  print("THE LAST RUN DID NOT SETTLE EVERYTHING IT PLANNED: see the `ran` line")
  print("  above for pending/failed/lost, and the HALTED line for why the loop")
  print("  stopped continuing.")
end
print(sup:report())
print("")
print("THE RESULT IS NOT ON THIS PAGE. Read it with:")
print("  just analyse " .. id)
print("The milestone is 'petroleum appears in the production samples' AND the")
print("attribution for that interval is not `roster-fed`. A number without the")
print("verdict beside it is not the claim.")
print("end oil milestone")
