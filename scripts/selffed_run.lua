-- A CELL THAT FEEDS ITSELF, against a live game.
--
-- Design note: `docs/superpowers/notes/2026-09-06-standing-goals.md`.
-- This rung:   `docs/superpowers/notes/2026-09-06-a-cell-that-feeds-itself.md`.
--
-- The difference from `sustain_run.lua`, which is the run this one exists to
-- beat: rung 1 there is `goal.producing("iron-plate", 15)`, which builds a
-- burner cell and then **charges it by hand for ten minutes** -- 23 coal in
-- the drill, 14 in the furnace, 36,800 and 37,324 ticks. That run returned
-- `SUSTAINED` and the pass was unearned: its dispatch list ended at tick
-- 4,521 and the whole window ran off one hand charge.
--
-- Rung 1 here is `goal.sustain(...)`, whose expansion builds the supply as
-- well as the capacity: a burner drill on the coal patch dropping into an
-- iron chest, a burner inserter putting coal from that chest back into the
-- drill (so the source itself is standing, not charged), a belt haul to a
-- second chest beside the smelting cell, and two more belt runs from that
-- chest into the cell's drill and furnace. Every arm is a `burner-inserter`
-- because `inserter` is not craftable at t=0 and there is no power to run one
-- with. Each burner still gets **one** coal by hand, because a burner with an
-- empty fuel slot never turns over to receive the belt's first delivery.
--
-- `goal.sustain` closes `satisfied` when the whole arrangement stands -- see
-- the `planner::sustain_supply_not_standing` branch in `supervisor.lua`, and
-- read its comment before reading that word as a claim about the rate. The
-- rate is answered only by
--
--     tools/run_analysis.py --sustain iron-plate:15:7200:20000 <run-dir>
--
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
local run_id = record.start({video = false})
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
-- **Since the offtake, the watched machine is the CHEST and the number is
-- 7,200**, and both halves of that changed for the same reason -- see the
-- witness's own comment in the ladder below. The 432-tick fill above is still
-- the furnace's, and it is now only the FIRST of three stages: the coal has to
-- reach the cell's buffer and then the offtake arm's fuel slot before a plate
-- can move, and none of those belts is running when the milestone opens. 7,200
-- ticks is two minutes, which at 5x is 24 seconds of wall clock for a failure
-- and is stopped by the first plate for a success.
--
-- The cost stays asymmetric on purpose: `at_least = 1` means a working chain
-- stops the wait as soon as one plate lands, while only a dead one pays the
-- whole window. One plate is enough because no bot acts during it -- an item
-- that appears in a chest can only have been smelted by a machine and carried
-- there by an inserter.
--
-- `near`/`radius` are how the cell is found at all: the planner chose its site,
-- this script never learns it, so the witness sweeps for a container that a
-- burner inserter's own reported `drop_position` lands in.
local WITNESS_WITHIN_TICKS = 7200

-- The window, and the lead-in, both stated rather than derived.
--
-- 7,200 ticks is two minutes: 15/min asks for 30 plates in it, and the
-- capacity built by rung 1 is exactly one cell's 15/min, so the target is the
-- cell's whole nominal output with no margin either way.
--
-- 9,600 is one hand-charged stone furnace's input stack at 3.2 s a plate. It
-- CANNOT be derived from the record -- the mod reports `output_inventory` and
-- `fuel_inventory` and not input slots -- so it is stated here, from the
-- machine's input capacity and its consumption rate, and it is the caller's
-- claim rather than the library's guess.
local SUSTAIN_WINDOW_TICKS = 7200
-- **20,000, and the old 9,600 was the number that made the last run's pass
-- unearned.** A lead-in has to outlast the drain of the LONGEST-LASTING
-- hand-delivered input, and 9,600 was sized against the furnace's ORE stack --
-- an input that was already standing, since the drill drops into the furnace.
-- The hand-delivered input is COAL.
--
-- Measured off this plan rather than assumed
-- (`factorio-bot plan --world map.json --goal sustain:iron-plate:15:7200
-- --steps`): the cell's drill takes 6 coal by hand across the build and its
-- furnace 5, because the `Goal::Have` chain that makes the belts' own iron
-- smelts by hand and `smelt_steps` queues into a furnace that already stands
-- -- which, once the cell is up, is the cell's furnace. Six coal is 9,600
-- ticks in a drill and five is 13,330 in a furnace, so 20,000 covers the
-- longer of the two with margin.
--
-- It is still a stated claim and not a derivation: the mod reports
-- `fuel_inventory` but not input slots, so nothing archived can say how much
-- hand-delivered material was in a machine when the window opened. The
-- hand-credit mass balance being built on `sustain-mass-balance` is what
-- removes this parameter.
local SUSTAIN_LEAD_IN_TICKS = 20000

local goals = {
    goal.sustain("iron-plate", 15, SUSTAIN_WINDOW_TICKS),
    -- **The witness watches the OFFTAKE CHEST, not the furnace, and the
    -- offtake is exactly why.**
    --
    -- `supervisor.count_item` reads `output_inventory`, which for a furnace is
    -- what the furnace has made and not yet given away. That was the right
    -- thing to watch while nothing took the plates -- and it is the wrong
    -- thing now, because the whole point of this rung is that the plates do
    -- not stay there. A working offtake keeps the furnace's output near zero,
    -- so a furnace witness would read `+0` and report a DEAD CELL precisely
    -- when the cell is working best: a confident answer about the wrong
    -- object, which is the failure shape this project has paid for most.
    --
    -- The chest is both safe and stronger. `get_output_inventory()` on a
    -- container is its contents (`mods/BotBridge/types.lua`), so the same
    -- counter works unchanged; and a plate sitting in a chest, with no bot
    -- acting for the whole window, was smelted by a machine AND carried there
    -- by an inserter. That is the entire chain, not one link of it.
    --
    -- The coal chests are in the watch set too -- they are also fed by an arm
    -- -- and contribute nothing, because the count is of `iron-plate`.
    --
    -- `within_ticks` is 7,200 rather than 2,400 because the chain being
    -- witnessed is three belt runs long and starts cold: the coal has to reach
    -- the cell's buffer, then the offtake arm's own fuel slot, before the
    -- first plate can move. The offtake arm gets **no ignition charge** -- it
    -- does not need to swing to be filled, and `run-1788679826-02267` placed
    -- eight arms with no charge at all and they all started.
    supervisor.witness {
        item = "iron-plate",
        from = "burner-inserter",
        into = "iron-chest",
        near = { x = 0, y = 0 },
        radius = 300,
        at_least = 1,
        within_ticks = WITNESS_WITHIN_TICKS,
    },
    supervisor.sustain {
        item = "iron-plate",
        per_minute = 15,
        window_ticks = SUSTAIN_WINDOW_TICKS,
        lead_in_ticks = SUSTAIN_LEAD_IN_TICKS,
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
    "an iron-plate cell that feeds itself at 15/min",
    "witness: the offtake chest fills while every bot stands still",
    -- The lead-in matches `SUSTAIN_LEAD_IN_TICKS` above. It read 9600 here
    -- while the constant said 20,000, so every record this run wrote named a
    -- milestone by a parameter it was not measured with -- a confident field
    -- about the wrong object, in the record's own index.
    "sustain iron-plate 15/min over 7200 ticks (lead-in 20000)",
}

local sup = supervisor.new(supervisor.list(goals),
    { bots = BOTS, stall_limit = 3, max_iterations = 10 })

-- The loop is wrapped so a raise still closes the recording. A run that died
-- part-way is the one most worth opening, and it is no use if it never got a
-- manifest, splits or its video copied out of the workspace.
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
        if type(t.sustain_built) == "table" then
            -- The planner refused because there is nothing left to build, and
            -- `supervisor.lua` turned that into satisfaction of the BUILDABLE
            -- half. Printed as what it is, because the reason word
            -- (`already_satisfied`) is about entities and a reader will
            -- otherwise take it for a claim about the rate.
            print("   ARRANGEMENT STANDS (the rate is NOT claimed here): "
                .. tostring(t.sustain_built.message))
        elseif type(t.sustain) == "table" then
            -- Printed as what it is: a window that happened, not a rate that
            -- held. Claiming the second here would be the `obs.done` failure
            -- -- a confident field about the wrong object.
            print(string.format(
                "   WINDOW HELD: %d of %d ticks idle (%d lead-in + %d window) for %s %d/min; "
                .. "verdict is DEFERRED to the record: %s",
                t.sustain.elapsed_ticks, t.sustain.span_ticks,
                t.sustain.lead_in_ticks, t.sustain.window_ticks,
                tostring(t.sustain.item), t.sustain.per_minute, t.sustain.answered_by))
        elseif type(t.witness) == "table" then
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

local id = record.finish(ok and sup.state or "crashed")
print("RUN FINISHED state=" .. (ok and sup.state or "crashed") .. " id=" .. id)
print(sup:report())
