-- Stage 1 of the starter factory, against a live game.
--
-- ONE goal: a burner mining drill standing on iron ore, dropping into a stone
-- furnace that is fuelled. Nine iron plates and ten stone; no research, no
-- power, no executor or mod change.
--
-- WHAT THIS CAN AND CANNOT TELL US. **Two rungs, and they answer two different
-- questions.** The first asks "do the bots build the cell" -- do they site a
-- drill on ore, put a furnace where the drill's drop point lands, and fuel
-- both. Satisfying it means the cell STANDS, and standing is not producing:
-- `PlanState` models neither fuel running out, nor output backing up, nor a
-- patch running dry, so all three leave the planner saying yes to a cell that
-- makes nothing.
--
-- The second rung is the witness, and it is the only line in this run that is
-- evidence about production. It **dispatches no actions at all**: it reads the
-- fed furnace's output inventory, waits, reads it again, and asserts the count
-- rose. Because no bot acted in between, a plate that appeared can only have
-- been smelted there. A `Producing` that holds is a claim about ground; only
-- the witness is evidence about production, which is why the ladder runs both.
--
-- That distinction is the whole reason this file says so at the top. A placed
-- machine reported as a working one is the same failure as the lab that was
-- placed and never powered.

--
-- Tracked, unlike its siblings in `workspace/scripts/` (`multibot.lua`,
-- `showcase.lua`, `record_smoke.lua`), which are gitignored scratch this file
-- is modelled on: this is what the run-record-enrichment plan's live-run task
-- actually executes, and a release build embeds `scripts/` into the binary at
-- compile time -- an untracked script simply does not ship. It must work from
-- a clean checkout, so it waits for whatever roster shows up rather than
-- assuming a fixed bot count or a pre-seeded inventory.
include("supervisor.lua")

-- Which of `wanted` are missing from `ids`. Empty when every one is there --
-- including when nothing was wanted, which is what makes the wait below
-- degrade to "anybody at all" for a caller with no roster to compare against.
local function missing_from(ids, wanted)
    local have = {}
    for _, id in ipairs(ids) do have[id] = true end
    local missing = {}
    for _, id in ipairs(wanted) do
        if not have[id] then missing[#missing + 1] = id end
    end
    return missing
end

-- Wait for the bots the *game* has, not the ones the planner imagines -- and
-- for ALL of them, not merely the first to appear.
--
-- `world.player(id)` answers from the cached world, which holds a bot for
-- every id the run was started with whether or not that client ever
-- connected -- waiting on it always succeeds instantly and planning then
-- raises on a bot that is not there. `rcon.players()` asks the game and
-- reports only players with a character, because a connection that has not
-- finished spawning cannot do the work. See `workspace/scripts/multibot.lua`,
-- whose `wait_for_roster` this mirrors.
--
-- **The first non-empty answer is not the roster**, which is what this used to
-- return. Freeplay's `on_player_created` runs `crash_site.create_cutscene`
-- gated on `player_index == 1`, and a player in a cutscene has no `character`
-- -- exactly what `rcon_players()` filters on. So for its 750 ticks (12.5s)
-- player 1 alone is invisible here, while 2, 3 and 4 appear the moment they
-- spawn. Run 30 (`workspace/runs/run-1788365280-15443/`) polled into that gap,
-- got `[2]`, and froze it: bots 1, 3 and 4 stood still for 162,158 ticks with
-- zero dispatches while the record showed four connected clients. Four of
-- nineteen archived runs did this. See
-- `docs/superpowers/notes/2026-09-02-bot-one-idle.md`.
--
-- So the wait is for `all_bots` -- the roster this run was *started* with,
-- which the script already has -- and not for whoever answers first.
--
-- **On timeout it proceeds, and says who is missing.** Refusing outright would
-- throw away a run whose fourth client genuinely failed to launch, and that
-- case is real: `Planner::roster` reports only the clients that connected,
-- precisely because one that never does must produce no bot. Three bots can do
-- the ladder. What must not happen is the silent version, which is what run 30
-- was.
--
-- It does **not** keep re-checking once the run has started. A bot that
-- appears at second 13 would be useful for the remaining hour, but the roster
-- is fixed at `supervisor.new` and every plan is expanded against it; picking
-- one up mid-run means re-rostering the supervisor between milestones, which
-- is a change to the loop rather than to this wait.
local function wait_for_roster(tries)
    local wanted = (type(all_bots) == "table") and all_bots or {}
    local latest, missing = {}, wanted
    for attempt = 1, tries do
        local ok, ids = pcall(function() return rcon.players() end)
        -- By type, not by truthiness: a `nil` crossing the Rust bridge arrives
        -- as mlua's null sentinel, which is light userdata and therefore true.
        if ok and type(ids) == "table" then
            latest = ids
            missing = missing_from(ids, wanted)
            if #ids > 0 and #missing == 0 then
                print("roster ready: " .. #ids .. " bot(s) after " .. attempt .. " checks")
                return ids
            end
        end
        pcall(function() rcon.inventory_contents_at({}) end)
    end
    if #latest > 0 then
        print("WARNING: proceeding with " .. #latest .. " of " .. #wanted
            .. " bot(s) after " .. tries .. " checks; never appeared: "
            .. table.concat(missing, ", "))
    end
    return latest
end

local BOTS = wait_for_roster(600)
if #BOTS == 0 then
    print("ABORT: no bots connected at all")
    return
end

-- `video = true` films client 1's window at 720p. It is opt-in and
-- deliberately non-fatal: if the window cannot be found, resized or grabbed,
-- the capture reports `status: failed` with a reason and the run carries on
-- without it. The event log and `map.jsonl` are the tick-exact record; the
-- video is the watchable one, joined to them by `ticks.jsonl`.
local run_id = record.start({video = true})
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
--
-- **How long the witness waits, and why that number.** Stage 1's cell is a
-- burner mining drill dropping into a stone furnace, and the planner's own
-- arithmetic for it (`crates/planner/src/method/produce.rs`) is
--
--     drill:   ceil(60 * ore.mining_time / drill.mining_speed) = 240 ticks/ore
--     furnace: smelting_ticks(recipe, stone-furnace) / yield   = 192 ticks/plate
--
-- Those are the two stages of one pipeline, so the FIRST plate is 240 + 192 =
-- 432 ticks away -- the fill, not the period. Every plate after it is 240,
-- because the drill is the bottleneck.
--
-- 2400 is 5.5 times that fill. The margin is not for wall-clock slowness --
-- the witness counts game ticks, so a server running below 60 UPS costs it
-- seconds and not ticks -- it is for the two numbers above being a MODEL:
-- `mining_speed`, `mining_time` and the smelting time are read off the
-- prototypes, and the whole reason to witness a cell is that the model can be
-- optimistic. A window five times the prediction is one nothing but a genuinely
-- dead cell reaches.
--
-- And the cost is asymmetric on purpose: `at_least = 1` with a 2400-tick window
-- means a working cell stops the wait at ~500 ticks (about 8 seconds), while
-- only a dead one pays the whole 40. One plate is enough because no bot acts
-- during the window -- an item that appears in a furnace's output can only have
-- been smelted there.
--
-- `near`/`radius` are how the cell is found at all: the planner chose its site,
-- this script never learns it, so the witness sweeps for a stone furnace that a
-- burner drill's own reported `drop_position` lands in. The hand-smelt furnaces
-- the bill also builds have nothing dropping into them and are not watched.
local WITNESS_WITHIN_TICKS = 2400

local goals = {
    goal.producing("iron-plate", 15),
    supervisor.witness {
        item = "iron-plate",
        from = "burner-mining-drill",
        into = "stone-furnace",
        near = { x = 0, y = 0 },
        radius = 300,
        at_least = 1,
        within_ticks = WITNESS_WITHIN_TICKS,
    },
}
-- Must stay aligned with `goals` above, index for index: these strings are what
-- the record shows for each milestone, and a mismatch would label a failure
-- with the wrong step.
--
-- The second rung's name carries the word "witness" because the record cannot:
-- `record.milestone_satisfied` takes one of two `SatisfiedReason` strings and a
-- witness reports `already_satisfied`, which is true (nothing was planned and
-- the world met it) but does not say what kind of milestone it was. The name
-- does.
local names = {
    "an iron-plate cell producing 15/min",
    "witness: the cell's furnace fills while every bot stands still",
}

local sup = supervisor.new(supervisor.list(goals),
    { bots = BOTS, stall_limit = 3, max_iterations = 10 })

-- The loop is wrapped so a raise still closes the recording. A run that died
-- part-way is the one most worth opening, and it is no use if it never got a
-- manifest, splits or its video copied out of the workspace.
-- Wrapped in a retry, so a fault does not have to cost the world it happened
-- in. `supervisor.hold_fault` writes a savepoint at the fault, pauses the game
-- and holds it for a bounded window; released with 'continue' the supervisor
-- resumes here, released with 'stop' -- or left to lapse -- the run tears down
-- with the world still resumable from the savepoint.
local held
local function drive() return pcall(function()
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
        -- `t.bots` is the roster the planner expanded this plan against, which
        -- is not the roster this process was started with whenever fewer bots
        -- connected than were asked for -- and it is the difference the record
        -- has to state rather than paper over. Passed through even when it is
        -- nil: `record.plan_created` then writes null, which says "nobody told
        -- us" instead of naming a roster nobody established.
        record.plan_created(t.milestone_index, t.plan, t.bots)
    end
    if t.action == "acquired" then
        record.milestone_started(t.milestone_index, names[t.milestone_index] or "?")
        print("-> " .. (names[t.milestone_index] or "?"))
    elseif t.action == "planned" then
        -- `t.recovery` is present when this "plan" is not a plan at all but a
        -- tier-1 recovery of the one before it: the same plan minus what
        -- already succeeded, re-scheduled against the world as it now is, with
        -- no expansion done and the tracker deliberately untouched. Printed
        -- because until `PlanCreated` carries a cause, this line is the only
        -- place a reader can tell two consecutive plans of one milestone apart.
        print("   planned " .. t.steps .. " steps (best " .. tostring(t.best) .. ")"
            .. (t.recovery and (" -- recovered: " .. t.recovery
                .. " " .. tostring(t.recoveries)) or ""))
    elseif t.action == "ran" then
        local n = 0
        if t.steps ~= nil and t.actions ~= nil then n = record.actions(t.steps, t.actions) end
        -- The walks of the same batch. A separate call, not a third argument
        -- to the one above, because a walk is not an action: it has no action
        -- id, it is in neither `steps` nor `actions`, and `(bot, step_index)`
        -- is the only thing that names it. Walking is most of the wall clock
        -- in these plans and none of it used to reach `events.jsonl` at all --
        -- run 30 failed three walks and left one `last_error` string in the
        -- record, the other two surviving only in `workspace/server-log.txt`,
        -- which this run has already overwritten. Same cadence as the flushes
        -- around it, so a walk is written close to when it happened.
        local nw = 0
        if type(t.walks) == "table" then nw = record.walks(t.walks) end
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
        -- The bots this batch found walled in: a flood fill from where the bot
        -- stands that closed without reaching open ground. Only asked when the
        -- game's pathfinder has already refused that bot a route from that
        -- spot, so each one explains a failed walk beside it. Same cadence as
        -- the three flushes above. `run-1788432181-42528` ran its whole budget
        -- with two of four bots frozen for 77% of it and said so nowhere; this
        -- line is what makes that visible without reading `samples.jsonl` by
        -- hand.
        local ne = record.enclosures()
        if ne > 0 then print("   WALLED IN: " .. ne .. " bot(s) can no longer reach open ground") end
        -- Grouped, because the four trouble counts are two axes and a flat
        -- list of four `x=n` pairs invites exactly the misreading this line
        -- used to produce: `failed` and `lost` name the same distinction for
        -- actions that `walks failed` and `walks lost` name for walks -- the
        -- game saying no versus the game never answering. Printing them
        -- ungrouped, with `failed` holding the sum of all four, made one lost
        -- action read as `failed=1 lost=1`: two problems where there was one.
        print(string.format(
            "   ran: success=%s pending=%s actions(failed=%s lost=%s) walks(failed=%s lost=%s) (+%d events, +%d walk events, +%d teleports, +%d refusals)",
            tostring(t.success), tostring(t.pending),
            tostring(t.failed), tostring(t.lost),
            tostring(t.walks_failed), tostring(t.walks_lost), n, nw, nt, nr))
        if t.first_error ~= nil then print("        first error: " .. tostring(t.first_error)) end
    elseif t.action == "satisfied" then
        record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
        -- A witness's satisfaction is the one line in this run that is evidence
        -- about production rather than about ground, so it prints its numbers
        -- rather than the reason word it had to borrow.
        if type(t.witness) == "table" then
            print(string.format(
                "   WITNESSED: %s in %d watched machine(s) went %d -> %d (+%d, wanted %d) in %d of %d ticks, %d polls",
                tostring(t.witness.item), t.witness.watched, t.witness.before,
                t.witness.after, t.witness.gained, t.witness.at_least,
                t.witness.elapsed_ticks, t.witness.within_ticks, t.witness.polls))
        else
            print("   SATISFIED (" .. tostring(t.reason) .. ")")
        end
    elseif t.action == "halted" then
        -- `sup.first_error` is the first failed action's own text for this
        -- milestone (set by the "ran" branch above, nil for `stuck_silent`,
        -- where every run reported success and there is no error to give);
        -- `t.best` is the fewest steps any plan for it reached. Both ride
        -- along so a stuck milestone's record carries the reason it got
        -- stuck, not just the verdict.
        --
        -- `t.refusal` is present when the planner REFUSED this milestone --
        -- a verdict about the world ("a lab with no power researches nothing
        -- at all"), not an action that failed. It leads, because it is what
        -- closed the milestone; `first_error` may also be set, from a run
        -- earlier in the same milestone, and that is a different fact.
        local why = (t.refusal and t.refusal.message) or sup.first_error
        record.milestone_stuck(t.milestone_index, t.state, why, t.best)
        print("   HALTED: " .. t.state
            .. (t.refusal and (" -- refused: " .. t.refusal.message) or ""))
    end
until sup:finished()
end) end

local ok, err
repeat
    ok, err = drive()
    held = (not ok) and supervisor.hold_fault(err, { index = sup.index, run = run_id }) or nil
until ok or held == nil or held.released ~= "continue"

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
-- `FactorioSurface`'s queue and never reach this run's `events.jsonl` at all.
record.teleports()
-- And the same for a refusal, which is worse to lose: a run that died on its
-- last placement is precisely the run whose refused site someone will want to
-- look up.
record.refusals()
-- And the same for an enclosure, for the same reason: a run that ended with a
-- bot unable to move is precisely the run whose frozen bot someone will want to
-- look up.
record.enclosures()

-- `held` outranks `crashed`: a run somebody paused and looked at, or one that
-- lapsed unattended, is not the same event as one that exited on the spot --
-- and neither may read as a run that finished.
local final_state = ok and sup.state or ((held and held.held) and "held" or "crashed")
local id = record.finish(final_state)
print("RUN FINISHED state=" .. final_state .. " id=" .. id)
print(sup:report())
