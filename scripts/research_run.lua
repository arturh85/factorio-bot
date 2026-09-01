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
local goals = {
    goal.have("iron-ore", 20),
    goal.have("copper-ore", 20),
    goal.have("iron-plate", 10),
    goal.researched("automation"),
}
local names = {
    "gather iron ore x20",
    "gather copper ore x20",
    "smelt iron plates x10",
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
    -- and recording that is this driver's job, not the loop's own.
    if type(t.plan) == "table" then
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
        print(string.format("   ran: success=%s failed=%s lost=%s pending=%s (+%d events)",
            tostring(t.success), tostring(t.failed), tostring(t.lost),
            tostring(t.pending), n))
        if t.first_error ~= nil then print("        first error: " .. tostring(t.first_error)) end
    elseif t.action == "satisfied" then
        record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
        print("   SATISFIED (" .. tostring(t.reason) .. ")")
    elseif t.action == "halted" then
        record.milestone_stuck(t.milestone_index, t.state)
        print("   HALTED: " .. t.state)
    end
until sup:finished()
end)

if not ok then
    print("RAISED: " .. tostring(err))
    -- Recorded as a stuck milestone rather than silently: the run reached
    -- this goal and could not plan it, which is a fact about the run.
    record.milestone_stuck(sup.index, "plan_error")
end

local id = record.finish(ok and sup.state or "crashed")
print("RUN FINISHED state=" .. (ok and sup.state or "crashed") .. " id=" .. id)
print(sup:report())
