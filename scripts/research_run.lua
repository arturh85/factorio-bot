-- Drives one full recorded run toward `goal.researched("automation")`.
--
-- Tracked, unlike its siblings in `workspace/scripts/` (`multibot.lua`,
-- `showcase.lua`, `record_smoke.lua`), which are gitignored scratch this file
-- is modelled on: this is what the run-record-enrichment plan's live-run task
-- actually executes, and a release build embeds `scripts/` into the binary at
-- compile time -- an untracked script simply does not ship. It must work from
-- a clean checkout, so it waits for whatever roster shows up rather than
-- assuming a fixed bot count or a pre-seeded inventory.
include("supervisor.lua")

-- Wait for the bots the *game* has, not the ones the planner imagines.
--
-- `world.player(id)` answers from the cached world, which holds a bot for
-- every id the run was started with whether or not that client ever
-- connected -- waiting on it always succeeds instantly and planning then
-- raises on a bot that is not there. `rcon.players()` asks the game and
-- reports only players with a character, because a connection that has not
-- finished spawning cannot do the work. See `workspace/scripts/multibot.lua`,
-- whose `wait_for_roster` this mirrors.
local function wait_for_roster(tries)
    local best = {}
    for attempt = 1, tries do
        local ok, ids = pcall(function() return rcon.players() end)
        if ok and ids ~= nil then
            best = ids
            if #ids > 0 then
                print("roster ready: " .. #ids .. " bot(s) after " .. attempt .. " checks")
                return ids
            end
        end
        pcall(function() rcon.inventory_contents_at({}) end)
    end
    return best
end

local BOTS = wait_for_roster(600)
if #BOTS == 0 then
    print("ABORT: no bots connected at all")
    return
end

local run_id = record.start()
print("recording run " .. run_id)

-- Milestones building toward automation: gather the ores, smelt the plates
-- automation's science pack needs, then research it. `goal.researched`
-- recurses through whatever the technology's own bill and prerequisites turn
-- out to require, so this list only has to name the destination and the raw
-- materials worth calling out as their own milestones.
-- A graded ladder: each rung must be reachable only by doing the thing it names.
--
-- `iron-plate 10` used to sit here and was worthless as a test. Factorio
-- freeplay hands every player 8 iron plates, so across a roster the goal was
-- already met before anyone moved -- it reported `already_satisfied` in zero
-- iterations on every run and never once exercised smelting. 50 is above what
-- the roster starts with, so it has to smelt.
--
-- `researched("automation")` was also not the next rung after plates but
-- several: gears, copper plates, science packs, a lab, and the research. Each
-- of those now stands on its own, so a failure names which step is broken
-- rather than "research did not happen".
local goals = {
    goal.have("iron-ore", 20),
    goal.have("copper-ore", 20),
    goal.have("iron-plate", 50),
    goal.have("copper-plate", 20),
    goal.have("iron-gear-wheel", 20),
    goal.have("automation-science-pack", 10),
    goal.researched("automation"),
}
-- Must stay aligned with `goals` above, index for index: these strings are what
-- the record shows for each milestone, and a mismatch would label a failure
-- with the wrong step.
local names = {
    "gather iron ore x20",
    "gather copper ore x20",
    "smelt iron plates x50",
    "smelt copper plates x20",
    "craft iron gear wheels x20",
    "craft automation science packs x10",
    "research automation",
}

local sup = supervisor.new(supervisor.list(goals),
    { bots = BOTS, stall_limit = 3, max_iterations = 10 })

-- The loop is wrapped so a raise still closes the recording. A run that died
-- part-way is the one most worth opening, and it is no use if it never got a
-- manifest, splits or its frames copied out of the workspace.
local ok, err = pcall(function()
repeat
    local t = sup:step()
    -- `t.plan` rides on both the "planned" and "satisfied" transitions (see
    -- `supervisor.lua`'s `Sup:step()`): the planner returned in both cases,
    -- and recording that is this driver's job, not the loop's own. But only
    -- "planned" is recorded: the "satisfied" transition's `t.plan` is always
    -- the empty table from the re-plan that discovered there was nothing
    -- left to do, and a consumer taking the LAST `plan_created` per
    -- milestone must see that milestone's real DAG, not have it shadowed by
    -- this empty closing re-plan.
    if t.action == "planned" and type(t.plan) == "table" then
        record.plan_created(t.milestone_index, t.plan)
    end
    if t.action == "acquired" then
        record.milestone_started(t.milestone_index, names[t.milestone_index] or "?")
        print("-> " .. (names[t.milestone_index] or "?"))
    elseif t.action == "planned" then
        print("   planned " .. t.steps .. " steps (best " .. tostring(t.best) .. ")")
    elseif t.action == "ran" then
        local n = 0
        if t.steps ~= nil and t.actions ~= nil then n = record.actions(t.steps, t.actions) end
        -- Flushes any `player.teleport` calls the mod made while this batch
        -- ran (a stuck walk leg, or a bot nudged clear of a ghost/blueprint
        -- bounding box) -- see `record.teleports()`. Called once per "ran"
        -- transition, same cadence as `record.actions` above, so a teleport
        -- is written close to when it happened rather than batched
        -- arbitrarily.
        local nt = record.teleports()
        -- The sites the game refused this batch. Each one is now excluded
        -- from every later plan for the rest of the run, so it has to be in
        -- the record: otherwise the next reader sees a planner that started
        -- choosing further-away tiles for no stated reason. Same cadence as
        -- the two flushes above, for the same reason.
        local nr = record.refusals()
        print(string.format("   ran: success=%s failed=%s lost=%s pending=%s (+%d events, +%d teleports, +%d refusals)",
            tostring(t.success), tostring(t.failed), tostring(t.lost),
            tostring(t.pending), n, nt, nr))
        if t.first_error ~= nil then print("        first error: " .. tostring(t.first_error)) end
    elseif t.action == "satisfied" then
        record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
        print("   SATISFIED (" .. tostring(t.reason) .. ")")
    elseif t.action == "halted" then
        -- `sup.first_error` is the first failed action's own text for this
        -- milestone (set by the "ran" branch above, nil for `stuck_silent`,
        -- where every run reported success and there is no error to give);
        -- `t.best` is the fewest steps any plan for it reached. Both ride
        -- along so a stuck milestone's record carries the reason it got
        -- stuck, not just the verdict.
        record.milestone_stuck(t.milestone_index, t.state, sup.first_error, t.best)
        print("   HALTED: " .. t.state)
    end
until sup:finished()
end)

if not ok then
    print("RAISED: " .. tostring(err))
    -- Recorded as a stuck milestone rather than silently: the run reached
    -- this goal and could not plan it, which is a fact about the run. `err`
    -- is the text a bare `pcall` around this whole loop caught -- previously
    -- discarded, leaving `last_error: null` as the only trace of a crash the
    -- record was supposed to explain.
    record.milestone_stuck(sup.index, "plan_error", tostring(err), sup.tracker and sup.tracker.best)
end

-- One last flush: a teleport queued after the final "ran" transition (e.g.
-- during the raise the pcall above just caught) would otherwise sit in
-- `FactorioWorld`'s queue and never reach this run's `events.jsonl` at all.
record.teleports()
-- And the same for a refusal, which is worse to lose: a run that died on its
-- last placement is precisely the run whose refused site someone will want to
-- look up.
record.refusals()

local id = record.finish(ok and sup.state or "crashed")
print("RUN FINISHED state=" .. (ok and sup.state or "crashed") .. " id=" .. id)
print(sup:report())
