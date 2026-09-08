-- Stage 2 of the starter factory, against a live game.
--
-- An assembling machine making red science, fed by inserters, on the power
-- plant the planner already builds. Three rungs, and they answer three
-- different questions.
--
-- RUNG 1 -- `researched("automation")`. This is stage 2's own prerequisite and
-- it is a rung of its own so that a failure names it: `assembling-machine-1` is
-- locked until `automation`, `automation` is ten red science packs in a lab,
-- and a lab needs 60 kW, so this rung drags the offshore pump, the boiler, the
-- steam engine and the pole behind it. If it halts, nothing below it could
-- have run anyway.
--
-- RUNG 2 -- `producing("automation-science-pack", 6)`. The cell: an iron-plate
-- chest, an inserter, an assembling machine set to `iron-gear-wheel`, an
-- inserter, an assembling machine set to `automation-science-pack`, an
-- inserter and a copper-plate chest, and -- since the output path landed -- an
-- inserter taking finished packs out of the pack machine into a chest of their
-- own, with one small pole in the gap between the two machines reaching all
-- six electric parts. Six a minute is one machine's output: 5 s of recipe
-- divided by an assembling-machine-1's 0.5 crafting speed is 10 s a pack.
--
-- Satisfying it means the cell STANDS -- ten buildings, two recipes, eight
-- delivery links and enough uncommitted capacity on the network for all of it.
-- Standing is not producing. `PlanState` reads no container contents and no
-- fuel level, so a cell whose chests are empty or whose boiler has run dry
-- satisfies this rung exactly as a working one does.
--
-- "Whose output has backed up" used to be on that list and is not any more.
-- Before the output inserter existed the pack machine halted on `full_output`
-- after FOUR crafts and stayed there -- 1,249 of 1,281 `full_output` samples
-- on the reference run were this cell's two machines -- so the rate this rung
-- names was one nothing had ever reached. `cells_standing` now requires a
-- drain, which is structure and belongs here rather than in a witness that
-- could not see it either.
--
-- RUNG 3 -- the witness, and the only line in this run that is evidence about
-- production. It **dispatches no actions at all**: it reads the output chest,
-- waits, reads it again, and asserts the count of `automation-science-pack`
-- rose by at least `at_least` inside `within_ticks`. Because no bot acted in
-- between, a pack that appeared can only have been assembled there.
--
-- WHAT IT WATCHES, AND WHY THAT MOVED. It used to watch the assembling
-- machines' own output slots (`from = "inserter"`, `into =
-- "assembling-machine-1"`, which catches both machines because an inserter
-- drops into each, and the ITEM picks the terminal because the gear machine's
-- output never holds packs). That reading is no longer the right one and would
-- now fail on a HEALTHY cell: the output inserter drains the pack machine's
-- output slot continuously, so it sits at zero -- measured on a bench cell at
-- 94 packs made, output slot 0, status `working`.
--
-- So the witness watches the OUTPUT CHEST instead: `from = "inserter"`,
-- `into = "iron-chest"`. `supervisor.fed_machines` keeps the selector honest
-- without naming a tile -- the cell's feed and supply chests are filled BY
-- HAND and no inserter drops into either, so the output chest is the only
-- `iron-chest` in the sweep that anything drops into. The mod reports a
-- container's contents as `output_inventory` (`get_output_inventory()` in
-- `mods/BotBridge/types.lua`), which is what `supervisor.count_item` reads,
-- so this needed no new machinery either.
--
-- AND IT IS NOW A RATE, WHICH IT COULD NOT PREVIOUSLY BE. `at_least = 1` was
-- the strongest claim available while the terminal was a machine output slot:
-- that slot holds three or four items and then the machine STOPS, so "the
-- count rose" and "it is producing at 6/min" were indistinguishable, and six
-- witnesses across six runs all fired inside the same 780 ticks -- before the
-- jam. The chest accumulates monotonically, so `at_least` over `within_ticks`
-- is a genuine floor on the rate, at no extra cost and with no new API.
--
-- WHY FIVE, AND WHY 5400 TICKS. Five is one more than the four crafts an
-- undrained machine manages before it halts, so this is specifically a claim
-- the defective cell could not satisfy however long it waited. The planner's
-- own arithmetic for this cell
-- (`crates/planner/src/method/assemble.rs`) is
--
--     gear machine: smelting_ticks(iron-gear-wheel, AM1)         =  60 ticks
--     pack machine: smelting_ticks(automation-science-pack, AM1) = 600 ticks
--
-- Those are the two stages of one pipeline, so the FIRST pack is 660 ticks
-- away plus a few inserter swings -- the fill, not the period -- and every
-- pack after it is 600, because the pack machine is the bottleneck. Five packs
-- is therefore about 3,060 ticks nominal, and 5,400 leaves ~76 % margin for
-- the same reason the old 3600 left five times the fill: the two numbers above
-- are a MODEL, read off prototypes, and the whole point of witnessing a cell
-- is that the model can be optimistic. Five packs is also well inside the one
-- charge the cell is built with (`CELL_CHARGE_TICKS` is sized for fifteen).
--
-- The cost is asymmetric on purpose. A working cell stops the wait as soon as
-- the fifth pack lands, about 51 seconds; only a dead or jammed one pays the
-- whole 90.
--
-- `near`/`radius` are how the cell is found at all: the planner chose its site
-- and this script never learns it, so the witness sweeps for chests that an
-- inserter's own reported `drop_position` lands in.
--
-- WHAT THIS RUN CANNOT TELL US, said here rather than discovered later. The
-- chests are charged BY HAND, with two and a half minutes' worth of
-- ingredients (`CELL_CHARGE_TICKS`), and nothing refills them. So a green
-- rung 3 says "machines assembled red science with every bot idle", which is
-- what stage 2 is for, and it does not say "this factory runs by itself".
-- Making the inputs arrive by machine is stage 3.
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

-- The ladder. Each rung must be reachable only by doing the thing it names,
-- and each one is stated separately so a failure says WHICH step is broken --
-- the reason `researched("automation")` is a rung of its own rather than
-- something the cell drags in silently.
--
-- Rung 2 does not repeat rung 1's work. The plant it needs is already standing
-- in the world by then, and `nearest_supply_anchor` finds it by name: an
-- `electric-pole` and a `generator` are both on `EntityGraph::add`'s whitelist
-- since 2026-09-02, so a plant a previous milestone built is readable rather
-- than invisible. Before that whitelist widened, this ladder would have built
-- a second power plant on rung 2 and said nothing about it.
local WITNESS_WITHIN_TICKS = 5400
-- One more than the four crafts a machine with no output path manages before
-- it halts on `full_output`. See RUNG 3 above: this is the number that makes
-- the witness a rate claim rather than an existence claim.
local WITNESS_AT_LEAST = 5

local goals = {
    goal.researched("automation"),
    goal.producing("automation-science-pack", 6),
    supervisor.witness {
        item = "automation-science-pack",
        from = "inserter",
        into = "iron-chest",
        near = { x = 0, y = 0 },
        radius = 300,
        at_least = WITNESS_AT_LEAST,
        within_ticks = WITNESS_WITHIN_TICKS,
    },
}
-- Must stay aligned with `goals` above, index for index: these strings are what
-- the record shows for each milestone, and a mismatch would label a failure
-- with the wrong step.
local names = {
    "automation researched, which needs a lab and therefore a power plant",
    "a red-science cell producing 6/min, on the plant's own network",
    "witness: 5 packs reach the output chest in 90 s with every bot idle",
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
        -- Surfaces the mod refused chunks from. Expected to be zero on every
        -- run this project has made: the world model keys by position alone,
        -- so `on_chunk_generated` drops any chunk that is not on Nauvis rather
        -- than aliasing two planets onto one tile. Space Age IS enabled here,
        -- so a non-zero count is possible, and it means this run planned
        -- against an incomplete world -- worth a loud line, not a counter in a
        -- summary nobody reads. Same cadence as the flushes above.
        local ns = record.surface_chunks_dropped()
        if ns > 0 then print("   OTHER SURFACE: chunks discarded from " .. ns .. " surface(s) -- this run's world model is Nauvis-only") end
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
