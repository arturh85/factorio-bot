-- Does our own production provoke the biters, on the timescales we run at?
--
-- The apparatus for the pollution/evolution measurement, committed rather than
-- left in a worktree: an experiment whose apparatus dies with its worktree
-- cannot be re-run, and this project has already lost three published results
-- that way.
--
-- WHAT IT DOES. It builds the thing that pollutes -- `researched:automation`
-- drags an offshore pump, a boiler, a steam engine, a pole, a lab and a row of
-- stone furnaces behind it, which is exactly the owner's "more than 2 steam
-- engines" scenario in miniature -- and then it HOLDS THE GAME OPEN for a
-- fixed number of GAME TICKS while the mod's 300-tick force sampler records
-- pollution and the enemy force's evolution.
--
-- WHY THE HOLD IS IN TICKS. A poll count is not a duration: on a faster box
-- more game ticks pass per RCON round trip, so `for _ = 1, N` covers a
-- different amount of game time on every machine. That error published a block
-- at 4.6x which re-measured at 2.6x. `HOLD_TICKS` is read against
-- `rcon.game_tick()` and is immune to whatever else the box is doing --
-- starvation only makes the wall clock longer.
--
-- WHAT IT DELIBERATELY DOES NOT DO. It does not fight, does not build
-- defences, and does not plan around pollution. The task is the observation.
-- If evolution is flat over this window, that is the finding and it says
-- combat is a later concern; a flat curve is a complete result, not a failed
-- run.
--
-- READ THE RESULT WITH `just analyse <run>` -- the POLLUTION AND ENEMY
-- EVOLUTION section. Do not read `samples.jsonl` by hand: a `pollution` key
-- that is absent means *not captured*, never zero, and the analysis is what
-- keeps those apart.
include("supervisor.lua")

-- Game ticks to hold after the last milestone, sampled at the mod's 300-tick
-- force beat. 90,000 ticks is 25 minutes of game time, so with the build
-- ahead of it the run covers the 30-minute horizon the question is asked over.
local HOLD_TICKS = 90000
-- How long to wait between tick reads. This bounds the RCON chatter, not the
-- measurement: the measurement is bounded by `HOLD_TICKS` above.
local POLL_CHUNK = 200

local function wait_for_roster(tries)
    local wanted = (type(all_bots) == "table") and all_bots or {}
    for attempt = 1, tries do
        local ok, ids = pcall(function() return rcon.players() end)
        -- By type, not truthiness: a `nil` across the bridge is mlua's null
        -- sentinel, which is light userdata and therefore TRUE.
        if ok and type(ids) == "table" and #ids > 0 then
            local have = {}
            for _, id in ipairs(ids) do have[id] = true end
            local missing = 0
            for _, id in ipairs(wanted) do if not have[id] then missing = missing + 1 end end
            if missing == 0 then
                print("roster ready: " .. #ids .. " bot(s) after " .. attempt .. " checks")
                return ids
            end
        end
        pcall(function() rcon.inventory_contents_at({}) end)
    end
    local ok, ids = pcall(function() return rcon.players() end)
    return (ok and type(ids) == "table") and ids or {}
end

local BOTS = wait_for_roster(600)
if #BOTS == 0 then
    print("ABORT: no bots connected at all")
    return
end

local run_id = record.start({})
print("recording run " .. run_id)

local goals = { goal.researched("automation") }
local names = { "automation researched, which needs a power plant and furnaces" }

local sup = supervisor.new(supervisor.list(goals),
    { bots = BOTS, stall_limit = 3, max_iterations = 10 })

local ok, err = pcall(function()
    repeat
        local t = sup:step()
        if t.action == "planned" and type(t.plan) == "table" then
            record.plan_created(t.milestone_index, t.plan, t.bots)
        end
        if t.action == "acquired" then
            record.milestone_started(t.milestone_index, names[t.milestone_index] or "?")
            print("-> " .. (names[t.milestone_index] or "?"))
        elseif t.action == "ran" then
            if t.steps ~= nil and t.actions ~= nil then record.actions(t.steps, t.actions) end
            if type(t.walks) == "table" then record.walks(t.walks) end
            record.teleports()
        elseif t.action == "satisfied" then
            record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
            print("<- satisfied " .. tostring(t.milestone_index)
                .. " (" .. tostring(t.reason) .. ")")
        end
    until sup:finished()
end)
if not ok then
    print("LADDER RAISED: " .. tostring(err))
end

-- THE HOLD. The build is over; from here nothing acts and the only thing
-- changing the world is the factory it left standing and the clock. That is
-- precisely the window the question is about: whatever evolution does now, our
-- own production and the passage of time did it, and the mod's four evolution
-- terms say which.
local start = rcon.game_tick()
print("hold: from tick " .. tostring(start) .. " for " .. HOLD_TICKS .. " game ticks")
local now = start
while now - start < HOLD_TICKS do
    for _ = 1, POLL_CHUNK do
        pcall(function() rcon.inventory_contents_at({}) end)
    end
    local ok2, t = pcall(function() return rcon.game_tick() end)
    if not ok2 or type(t) ~= "number" then
        print("hold: could not read the tick; stopping early at " .. tostring(now))
        break
    end
    now = t
    print("hold: tick " .. tostring(now) .. " (+" .. tostring(now - start) .. ")")
end
print("hold complete at tick " .. tostring(now) .. " (+" .. tostring(now - start) .. ")")

record.finish(ok and sup.state or "crashed")
