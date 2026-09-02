--- Short-horizon replanning loop.
--
-- Holds long intent across plans: asks a source for the next milestone, plans
-- it, runs it, replans, and moves on when the plan comes back empty **and the
-- goal is confirmed to hold**. See
-- docs/superpowers/specs/2026-08-31-supervisor-loop-design.md.
--
-- `include` returns nothing, so this file defines the global `supervisor`,
-- the same idiom as lib.lua and the `goal` / `world` / `rcon` tables.
--
--   include("supervisor.lua")
--   local sup = supervisor.new(supervisor.list {
--       goal.researched("automation"),
--       goal.have("steel-plate", 200),
--   }, { bots = {1,2,3,4} })
--   repeat sup:step() until sup:finished()
--   print(sup:report())

supervisor = {}

-- Progress tracking. Pure: no goals, no plans, no I/O, integers only. This is
-- the piece most likely to be subtly wrong -- it is the only state that spans
-- iterations -- so it is isolated where it can be tested with plain numbers.
local tracker = {}
supervisor.tracker = tracker

function tracker.new()
    return { best = nil, stall = 0 }
end

--- Observe a step count. Returns a NEW tracker; does not mutate.
-- An improvement is strictly fewer steps than the best seen. Equal is not an
-- improvement, and an increase is not an improvement -- a plan may legitimately
-- grow when a bot dies or the world reveals a longer path.
function tracker.observe(t, step_count)
    if t.best == nil or step_count < t.best then
        return { best = step_count, stall = 0 }
    end
    return { best = t.best, stall = t.stall + 1 }
end

--- A milestone source over a literal ordered list.
-- The source interface is `function(history) -> goal | nil`, never a list, so
-- that a decomposer can later revise what remains based on what has happened.
function supervisor.list(goals)
    if type(goals) ~= "table" then
        error("supervisor.list: expected a table of goal values")
    end
    local i = 0
    return function(_history)
        i = i + 1
        return goals[i]
    end
end

--- Shapes `plan.steps` (a `PlanValue`'s own field, or the stand-in a test
-- stub hands back) into the array `record.plan_created` expects: `id`,
-- `bot`, `action`, `deps`, `planned_start`, `planned_duration`.
--
-- Takes the steps array itself, not the plan, so a caller who already read
-- `plan.steps` for its length (`#plan.steps`, below) does not pay for a
-- second one: `PlanValue.steps` is a Lua field getter, and each read rebuilds
-- the whole array from the schedule.
--
-- A step lacking `id` is skipped, not erroring -- a walk has no action id to
-- report (see `record.actions`'s own comment on the same point), and a test
-- stub's placeholder step is not a table at all. `deps` is read by type, not
-- by truthiness: `Option::None` reaches Lua as mlua's null sentinel, which is
-- light userdata and therefore truthy, so `s.deps or {}` would not substitute
-- the empty table for a step whose `deps` came from such a bridge.
local function plan_for_record(step_list)
    local out = {}
    for _, s in ipairs(step_list) do
        if type(s) == "table" and s.id ~= nil then
            local deps = {}
            if type(s.deps) == "table" then
                for _, d in ipairs(s.deps) do deps[#deps + 1] = d end
            end
            out[#out + 1] = {
                id = s.id,
                bot = s.bot,
                action = s.label,
                deps = deps,
                planned_start = s.start,
                planned_duration = (s.finish or s.start) - (s.start or 0),
            }
        end
    end
    return out
end

--- Is this caught error a planner VERDICT, or a fault?
--
-- A verdict is a statement about the world: the goal cannot be reached from
-- here, and the planner says exactly why -- no route to the item, nowhere to
-- stand while mining it, a research whose lab would have no power. Nothing is
-- broken and nothing is retryable, so the honest thing for this loop to do is
-- close the milestone with the reason and let the run finish. A fault is a
-- defect -- a method contradicting itself, a name that refers to nothing --
-- and it must still end the run loudly. `goal.refusal` (`goal/mod.rs`) is
-- where that classification is made, variant by variant; this only asks.
--
-- `nil` whenever the question cannot be answered: no `goal` table, no
-- classifier on it, a classifier that itself raised, or an answer that is not
-- a table. Every one of those is read as *fault*, because "cannot classify"
-- must never come out as "only a verdict, carry on" -- that is how a real bug
-- gets recorded as a condition of the world. It is the same rule `goal.plan`'s
-- placement pre-check follows: an absent checker must never look like a green
-- answer.
local function refusal_of(err)
    if type(goal) ~= "table" or type(goal.refusal) ~= "function" then return nil end
    local ok, refusal = pcall(goal.refusal, err)
    if not ok or type(refusal) ~= "table" then return nil end
    if type(refusal.message) ~= "string" then return nil end
    return refusal
end

-- ---------------------------------------------------------------------------
-- The witness: the durative half of "producing"
-- ---------------------------------------------------------------------------
--
-- A `Goal::Producing` is satisfied by STRUCTURE. The planner asks whether a
-- burner drill stands on ore and drops into a fuelled stone furnace, and it can
-- answer yes about a cell that is producing nothing at all: fuel run out,
-- output backed up, patch exhausted. `PlanState` models none of the three --
-- nothing in it reads a fuel level or a container's contents -- so `goal.holds`
-- keeps saying yes while the cell stands there dead. Those are rows 1, 3 and 4
-- of `docs/superpowers/specs/2026-09-03-starter-factory-design.md`'s failure
-- table, and the spec puts all three on this.
--
-- The witness is the other half, and its whole strength is one sentence:
-- **it dispatches no actions at all**. It reads a machine's output inventory,
-- waits, reads it again, and asserts the count rose. Because no bot acted in
-- between, an item that appeared can only have been made by a machine. That is
-- a property the structural predicate cannot have, which is why the two are not
-- redundant and why the ladder runs both.
--
-- It is the same failure this project has already paid for three times -- a lab
-- placed and never powered, pole coverage mistaken for generation, and
-- `only_ghosts = true` placing a blueprint whose entities overlap. A machine
-- that stands is not a machine that works.

--- How far two collision boxes may reach into each other before it counts.
-- `crates/planner/src/state.rs`'s `TOUCH_SLACK`, verbatim. A 1/512 tile is
-- finer than the 1/256 the game stores positions at, so nothing this admits is
-- a collision the game would see; it only keeps float noise from reading as one.
local TOUCH_SLACK = 1 / 512

--- Does `source` drop what it makes into `target`?
--
-- Pure, and answered entirely from what the **game itself** reported: the mod
-- sends `drop_position` and `bounding_box` on every entity it serialises
-- (`mods/BotBridge/types.lua`, `serialize_entity`), so this needs no copy of
-- the planner's hand-written `delivery_offset` table and cannot disagree with
-- it. A wrong table is exactly the risk row 5 of the spec's failure list names,
-- and the point of a witness is not to inherit it.
--
-- **Tiles, not the box, and the number that decides it.** A burner drill at an
-- integer position facing north drops at `(-0.35, -1.3)` from its own centre.
-- The stone furnace two tiles north -- the vanilla starter pair, the entire
-- content of stage 1 -- has a collision box of +/-0.69921875, so its near edge
-- sits at `-1.30078125` and the drop point misses it by **0.00078125 of a
-- tile**, one part in 1280. Asking whether the point is inside the box would
-- therefore answer "not fed" for the one layout stage 1 exists to build. A drop
-- point resolves to the tile it lands in, the furnace covers both of its tiles,
-- and the 1/1280 never comes up. `PlanState::delivers_into` decides it the same
-- way for the same reason.
--
-- `false` whenever either half cannot be read -- an entity with no drop point,
-- a target with no bounding box. Refusing to guess, in the direction that
-- reports less rather than more.
function supervisor.delivers_into(source, target)
    if type(source) ~= "table" or type(target) ~= "table" then return false end
    local drop = source.drop_position
    -- By type, not by truthiness: `Option::None` crosses the Rust bridge as
    -- mlua's null sentinel, which is light userdata and therefore TRUE. Every
    -- entity that is not a drill or an inserter carries one here.
    if type(drop) ~= "table" or type(drop.x) ~= "number" or type(drop.y) ~= "number" then
        return false
    end
    local box = target.bounding_box
    if type(box) ~= "table" or type(box.left_top) ~= "table"
        or type(box.right_bottom) ~= "table" then
        return false
    end
    local x0 = math.floor(box.left_top.x + TOUCH_SLACK)
    local x1 = math.floor(box.right_bottom.x - TOUCH_SLACK)
    local y0 = math.floor(box.left_top.y + TOUCH_SLACK)
    local y1 = math.floor(box.right_bottom.y - TOUCH_SLACK)
    local tx, ty = math.floor(drop.x), math.floor(drop.y)
    return tx >= x0 and tx <= x1 and ty >= y0 and ty <= y1
end

--- Which of `candidates` something in `sources` drops into.
--
-- Pure. This is how the witness decides **which machine to watch**, and it is
-- the reason the answer is trustworthy: the stage-1 bill hand-smelts in stone
-- furnaces of its own, and a witness that watched every furnace in the area
-- would count a bot's leftover batch finishing as machine-made production. A
-- furnace nothing drops into is not a cell, so it is not watched.
--
-- An entity is never its own source: an assembling machine that dropped into
-- itself would otherwise witness itself.
--
-- Order is `candidates`' own, preserved rather than sorted, because the caller
-- sums over the result and the sum does not depend on order. Nothing here reads
-- the order the game returned equal entities in.
function supervisor.fed_machines(sources, candidates)
    local out = {}
    if type(sources) ~= "table" or type(candidates) ~= "table" then return out end
    for _, target in ipairs(candidates) do
        for _, source in ipairs(sources) do
            if source ~= target and supervisor.delivers_into(source, target) then
                out[#out + 1] = target
                break
            end
        end
    end
    return out
end

--- How many of `item` sit in these entities' **output** inventories.
--
-- Output only, and that is the whole trick. A furnace's output holds what the
-- furnace made; its input holds what somebody put there. The mod reports both
-- `output_inventory` and `fuel_inventory` and does not report input slots at
-- all (`serialize_entity`), which is a real gap for other purposes and exactly
-- right for this one.
function supervisor.count_item(entities, item)
    local total = 0
    if type(entities) ~= "table" then return total end
    for _, entity in ipairs(entities) do
        local inv = type(entity) == "table" and entity.output_inventory or nil
        -- By type again: an entity with no output inventory arrives as the null
        -- sentinel, and `inv or {}` would hand `ipairs` light userdata.
        if type(inv) == "table" then
            for _, slot in ipairs(inv) do
                if type(slot) == "table" and slot.name == item then
                    total = total + (tonumber(slot.count) or 0)
                end
            end
        end
    end
    return total
end

--- Names a machine by kind and where it stands, for the watch set.
-- `%.5f` resolves the 1/256 of a tile the game stores positions at.
local function machine_key(entity)
    local p = type(entity) == "table" and entity.position or nil
    if type(p) ~= "table" then return nil end
    return string.format("%s@%.5f,%.5f", tostring(entity.name), p.x, p.y)
end

--- Build a witness milestone: a rung that dispatches nothing and asserts
--- something happened anyway.
--
--     supervisor.witness {
--         item = "iron-plate",
--         from = "burner-mining-drill", into = "stone-furnace",
--         near = { x = 0, y = 0 }, radius = 250,
--         at_least = 1, within_ticks = 2400,
--     }
--
-- Hand it to a source like any goal: `supervisor.list { goal.producing(...),
-- supervisor.witness { ... } }`. **It is a rung, not an automatic follow-up.**
-- A witness costs real game time -- a dead cell pays the whole window -- and
-- charging every milestone for one would be paying to watch a `have` goal that
-- has no machine in it. So the ladder says when to look, which is also the
-- spec's own position: that every `Producing` milestone is followed by its
-- witness is a fact about what a run proves, and belongs in the run.
--
-- `within_ticks` has **no default**, deliberately. It is the number that
-- decides what a failure means, and a library that guessed it would be handing
-- back a verdict nobody derived. Derive it from the cell's own predicted rate:
-- stage 1's drill takes 240 ticks an ore and its furnace 192 ticks a plate, so
-- the first plate is 432 ticks away and each one after that is 240.
--
-- `from`/`into` name the two ends of one machine-to-machine link, which is
-- stage 1's whole shape. A longer chain -- drill to furnace to assembler --
-- means naming the terminal, and choosing it is the caller's job because only
-- the caller knows which end of the chain the goal was about.
function supervisor.witness(spec)
    if type(spec) ~= "table" then
        error("supervisor.witness: expected a table of options")
    end
    local function required_string(key)
        if type(spec[key]) ~= "string" then
            error("supervisor.witness: `" .. key .. "` must be a string naming an entity or item")
        end
        return spec[key]
    end
    local function positive_number(key, default)
        local v = spec[key]
        if v == nil then
            if default == nil then
                error("supervisor.witness: `" .. key .. "` is required and has no default")
            end
            return default
        end
        if type(v) ~= "number" or v <= 0 then
            error("supervisor.witness: `" .. key .. "` must be a positive number")
        end
        return v
    end
    local item = required_string("item")
    local from = required_string("from")
    local into = required_string("into")
    local near = spec.near
    if type(near) ~= "table" or type(near.x) ~= "number" or type(near.y) ~= "number" then
        error("supervisor.witness: `near` must be a position, `{ x = ..., y = ... }`")
    end
    local within_ticks = positive_number("within_ticks")
    return {
        __witness = true,
        item = item,
        from = from,
        into = into,
        near = { x = near.x, y = near.y },
        radius = positive_number("radius", 250),
        at_least = positive_number("at_least", 1),
        within_ticks = within_ticks,
        -- How often the output inventory is re-read while waiting. Every 60
        -- ticks is once a game-second: frequent enough that a working cell
        -- stops the wait as soon as it has proved itself, cheap enough that the
        -- reply is a handful of furnaces.
        probe_ticks = positive_number("probe_ticks", 60),
        -- The loop has no sleep -- the sandbox has none -- so it waits by
        -- asking the game what tick it is, and Factorio processes rcon once per
        -- tick, which paces it at roughly one poll per tick. The cap is what
        -- stops a game whose clock is NOT advancing (paused, saving, gone) from
        -- spinning forever, and hitting it is its own verdict: inconclusive,
        -- never "dead".
        max_polls = positive_number("max_polls", within_ticks + 600),
    }
end

--- Is this milestone a witness rather than a goal?
-- A real goal value is userdata, so this cannot collide with one.
function supervisor.is_witness(milestone)
    return type(milestone) == "table" and milestone.__witness == true
end

local Sup = {}
Sup.__index = Sup

local TERMINAL = { done = true, stuck = true, stuck_silent = true, exhausted = true }

function supervisor.new(source, opts)
    if type(source) ~= "function" then
        error("supervisor.new: source must be a function(history) -> goal | nil")
    end
    opts = opts or {}
    return setmetatable({
        source = source,
        bots = opts.bots,
        stall_limit = opts.stall_limit or 3,
        max_iterations = opts.max_iterations or 50,
        state = "acquiring",
        index = 0,
        _history = {},
        milestone = nil,
        plan = nil,
        tracker = nil,
        iterations = 0,
        step_counts = nil,
        any_failures = false,
        first_error = nil,
        refusal = nil,
    }, Sup)
end

function Sup:finished()
    return TERMINAL[self.state] == true
end

function Sup:history()
    return self._history
end

function Sup:_close(outcome)
    table.insert(self._history, {
        index = self.index,
        iterations = self.iterations,
        step_counts = self.step_counts,
        outcome = outcome,
        any_failures = self.any_failures,
        first_error = self.first_error,
        -- Kept apart from `first_error` on purpose. `first_error` is the first
        -- failed ACTION's own text; a refusal is why the milestone closed and
        -- no action ran at all. A milestone can have both (a run that failed,
        -- then a re-plan the planner refused), and collapsing them into one
        -- field would lose whichever was written second.
        refusal = self.refusal,
    })
    -- A keyframe at the boundary of every milestone -- the only place
    -- `map.jsonl` gets one; there is deliberately no tick timer driving it.
    -- Guarded because this loop must keep working for every caller that does
    -- not record: `record` is a global installed only alongside a live game
    -- connection, and even then only a script that called `record.start()`
    -- has one running, which this loop does not do on its own (see the
    -- comment on `actions`/`steps` in `:step()` below -- recording what
    -- happened is the caller's job, not this one's). `record.keyframe()`
    -- itself already answers "no recording is running" and "nothing has been
    -- placed yet" with `false`, not an error -- both are unremarkable here.
    --
    -- `pcall` is for what is left after that: a real failure, the game being
    -- unreachable or a bug in the glue. That must not kill the run either --
    -- a keyframe is a nicety, not core control flow -- but going completely
    -- silent about it is its own failure mode: a run that is actively
    -- recording but whose keyframe raises on *every* call would look
    -- identical to one that never asked for keyframes at all, and nothing
    -- would ever say why. `print_err` is the sandbox's own error-reporting
    -- output (`globals.lua`), which reaches the script job's SSE stream on
    -- the stderr side -- not `writeout`, which belongs to a different
    -- interpreter (the BotBridge mod's).
    if record ~= nil then
        local ok, err = pcall(record.keyframe)
        if not ok and type(print_err) == "function" then
            print_err("record.keyframe() failed at milestone " .. tostring(self.index)
                .. ": " .. tostring(err))
        end
    end
end

--- Run a witness milestone to its verdict, and close it. Dispatches nothing.
--
-- Three ways this ends, and **they are three because their fixes are three**.
-- The one thing a witness must never do is report "your cell is dead" for a
-- cell that was never built, or for a wait that was cut short:
--
--   * `supervisor::no_cell` -- nothing in the area is fed by anything, so
--     there is no cell to watch. The build did not happen, or the drill faces
--     the wrong way and drops on the ground. **Says nothing about production**,
--     and says so.
--   * `supervisor::not_producing` -- a cell IS standing, the full window
--     elapsed, and its output did not rise by `at_least`. This is the verdict
--     the witness exists to be able to give: placed, fed, fuelled on paper, and
--     making nothing.
--   * `supervisor::witness_inconclusive` -- the poll budget ran out before the
--     window did, or the game would not say what tick it is. Nothing is
--     established either way, and calling that a dead cell would be the same
--     mistake as calling an unanswerable goal a satisfied one.
--
-- All three close the milestone `stuck` and carry their reason on `t.refusal`,
-- which is the family a planner refusal and `supervisor::unanswerable` are
-- already in: a verdict that halts cleanly with a stated reason, on the channel
-- every driver already reads. `code` is what tells them apart without reading
-- the sentence, and no `PlannerError` can produce any of these three.
function Sup:_witness(w)
    local function observation(before, after, elapsed, polls, watched, missing)
        return { item = w.item, watched = watched, missing = missing,
                 before = before, after = after, gained = after - before,
                 at_least = w.at_least, elapsed_ticks = elapsed,
                 within_ticks = w.within_ticks, polls = polls }
    end
    local function halt(code, message, obs)
        self.refusal = { code = code, message = message }
        self:_close("stuck")
        self.state = "stuck"
        return { action = "halted", state = "stuck", milestone_index = self.index,
                 steps = 0, iteration = 0, refusal = self.refusal, witness = obs }
    end
    local where = string.format("(%s, %s)", tostring(w.near.x), tostring(w.near.y))

    -- A witness with no game to look at is a FAULT, not a verdict: the run was
    -- built wrong, nothing about the world is established, and quietly halting
    -- would record a condition of the world for a defect in the script. Same
    -- rule `refusal_of` follows -- "cannot tell" is never "carry on".
    if type(rcon) ~= "table" or type(rcon.find_entities_in_radius) ~= "function"
        or type(rcon.game_tick) ~= "function" then
        error("supervisor: milestone " .. tostring(self.index) .. " is a witness, "
            .. "but there is no game to witness it in: `rcon` is missing "
            .. "`find_entities_in_radius` and/or `game_tick`", 0)
    end

    local sources = rcon.find_entities_in_radius(w.near, w.radius, w.from)
    local candidates = rcon.find_entities_in_radius(w.near, w.radius, w.into)
    if type(sources) ~= "table" then sources = {} end
    if type(candidates) ~= "table" then candidates = {} end
    local watched = supervisor.fed_machines(sources, candidates)
    if #watched == 0 then
        return halt("supervisor::no_cell", string.format(
            "milestone %d witnesses %s, but no %s within %d tiles of %s is fed "
            .. "by a %s: %d %s and %d %s stand there and none of them are a "
            .. "cell. Nothing was built to watch, so this says nothing about "
            .. "whether anything produces",
            self.index, w.item, w.into, w.radius, where, w.from,
            #sources, w.from, #candidates, w.into),
            observation(0, 0, 0, 0, 0, 0))
    end

    -- The watch set is fixed here, by name and position, and re-read by those
    -- keys. Re-deriving it every probe would let a machine placed mid-window
    -- into the sum, and nothing places anything mid-window -- but the guard
    -- costs one table and removes the question.
    local watch = {}
    for _, machine in ipairs(watched) do
        local key = machine_key(machine)
        if key ~= nil then watch[key] = true end
    end

    local function read()
        local ok, list = pcall(rcon.find_entities_in_radius, w.near, w.radius, w.into)
        if not ok or type(list) ~= "table" then return nil, nil end
        local seen, found = {}, 0
        for _, machine in ipairs(list) do
            local key = machine_key(machine)
            if key ~= nil and watch[key] then
                seen[#seen + 1] = machine
                found = found + 1
            end
        end
        return supervisor.count_item(seen, w.item), #watched - found
    end

    local t0 = rcon.game_tick()
    if type(t0) ~= "number" then
        return halt("supervisor::witness_inconclusive", string.format(
            "milestone %d watched %d %s but the game would not say what tick it "
            .. "is, so the wait cannot be timed and nothing follows from it",
            self.index, #watched, w.into),
            observation(0, 0, 0, 0, #watched, 0))
    end
    local before = supervisor.count_item(watched, w.item)
    local after, missing, now, polls = before, 0, t0, 0
    local next_probe = w.probe_ticks

    while true do
        polls = polls + 1
        if polls > w.max_polls then
            return halt("supervisor::witness_inconclusive", string.format(
                "milestone %d gave up after %d polls with only %d of %d ticks "
                .. "elapsed: the game's clock is not advancing (paused, saving "
                .. "or gone). %s went from %d to %d, and no verdict follows "
                .. "from a wait that did not happen",
                self.index, polls - 1, now - t0, w.within_ticks,
                w.item, before, after),
                observation(before, after, now - t0, polls - 1, #watched, missing))
        end
        -- `pcall`: one refused round trip in a wait thousands long must not end
        -- the run. A hiccup costs this poll and nothing else -- the clock is
        -- read again next time round, and the cap above still bounds the loop.
        local ok, tick = pcall(rcon.game_tick)
        if ok and type(tick) == "number" then now = tick end
        local elapsed = now - t0
        local over = elapsed >= w.within_ticks
        if over or elapsed >= next_probe then
            next_probe = elapsed + w.probe_ticks
            local seen, gone = read()
            if seen ~= nil then after, missing = seen, gone end
            if after - before >= w.at_least then
                self:_close("satisfied")
                self.state = "acquiring"
                return { action = "satisfied", state = "acquiring",
                         milestone_index = self.index, steps = 0, iteration = 0,
                         -- `already_satisfied`, and not a word of its own.
                         -- `SatisfiedReason` (`crates/core/src/record/mod.rs`)
                         -- has exactly two variants, both mirrored into
                         -- `app/src/api/types.ts` and the OpenAPI snapshot, so
                         -- a third is a cross-boundary change this does not
                         -- make. Of the two, this is the one that is not a lie:
                         -- no planning was attempted and the world met the
                         -- milestone without being asked to do anything. WHAT
                         -- was observed is on `t.witness`, and the milestone's
                         -- recorded name says "witness" out loud.
                         reason = "already_satisfied",
                         witness = observation(before, after, elapsed, polls,
                                               #watched, missing) }
            end
        end
        if over then break end
    end

    local gone = ""
    if missing > 0 then
        gone = string.format(" (%d of them is no longer there)", missing)
    end
    return halt("supervisor::not_producing", string.format(
        "milestone %d watched %d %s that a %s feeds%s for %d ticks and no bot "
        .. "acted: %s went from %d to %d, %d short of the %d that would have "
        .. "proved a machine made it. The cell stands and produces nothing",
        self.index, #watched, w.into, w.from, gone, now - t0,
        w.item, before, after, w.at_least - (after - before), w.at_least),
        observation(before, after, now - t0, polls, #watched, missing))
end

--- Perform exactly one action and return a transition record.
function Sup:step()
    if self:finished() then
        return { action = "finished", state = self.state }
    end

    if self.state == "acquiring" then
        -- `next_goal`, not `goal`: `goal` is the global binding table and
        -- shadowing it here would break planning two lines later.
        local next_goal = self.source(self._history)
        if next_goal == nil then
            self.state = "done"
            return { action = "finished", state = "done" }
        end
        self.index = self.index + 1
        self.milestone = next_goal
        self.tracker = tracker.new()
        self.iterations = 0
        self.step_counts = {}
        self.any_failures = false
        self.first_error = nil
        self.refusal = nil
        self.state = "planning"
        return { action = "acquired", state = "planning", milestone_index = self.index }
    end

    if self.state == "planning" then
        -- A witness never plans. It is checked here rather than in `acquiring`
        -- so it shares the milestone bookkeeping that branch sets up -- index,
        -- history, keyframe -- and so `goal.plan` is not called even once for
        -- it: a milestone that dispatches nothing must also not cost the
        -- planner an expansion, or a ladder with a witness in it would produce
        -- different plans from one without.
        if supervisor.is_witness(self.milestone) then
            return self:_witness(self.milestone)
        end

        -- Wrapped, and only just: `goal.plan` raises for two different reasons
        -- and exactly one of them is this loop's business.
        --
        -- A **fault** -- a construction error (an unknown item or technology, a
        -- bot outside the roster) or a defect in the planner -- propagates,
        -- costs no iteration, and is not retried. Retrying a typo burns the cap
        -- and then reports "stuck", which actively misleads; and a run that
        -- carries on past a genuine bug reports a world condition for what is
        -- really broken code.
        --
        -- A **verdict** closes the milestone instead. Run 31
        -- (`workspace/runs/run-1788372605-35170/`) is why: it satisfied six
        -- milestones and then asked to research automation in a world with no
        -- electric supply, the planner refused -- correctly, and with the most
        -- informative sentence it has ever produced -- and the raise went
        -- straight past this branch. No `_close`, so no history entry, so the
        -- summary printed milestones 1-6 and no `milestone 7` line at all. The
        -- refusal destroyed the record of itself.
        --
        -- Not retried either, and for a reason of its own: re-planning asks the
        -- same question of the same world -- nothing ran, so nothing changed --
        -- and gets the same answer, until the iteration cap turns a stated
        -- reason into `exhausted`.
        local ok, planned = pcall(goal.plan, self.milestone, { bots = self.bots })
        if not ok then
            local refusal = refusal_of(planned)
            -- `error(err, 0)`: re-raised as the value it was, with no position
            -- prefix bolted on. The original error object -- and the traceback
            -- mlua wrapped it in -- survives for the driver's own pcall.
            if refusal == nil then error(planned, 0) end
            self.refusal = refusal
            -- `stuck`, not `stuck_silent`: that one means "no progress and
            -- nothing to show for it", and this milestone has the planner's
            -- own reason to show. Not a word of its own either -- the outcome
            -- vocabulary (`crates/core/src/record/splits.rs`) says how far a
            -- milestone got, and "as far as the world allows" is `stuck`; what
            -- KIND of stuck it was is `h.refusal`, which is data and does not
            -- need a second verdict word to carry it.
            self:_close("stuck")
            self.state = "stuck"
            return { action = "halted", state = "stuck",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations, refusal = refusal }
        end
        local plan = planned
        local plan_steps = plan.steps
        local steps = #plan_steps
        -- Shaped once and carried on `t.plan` for either outcome below, so a
        -- driver can call `record.plan_created(t.milestone_index, t.plan)`
        -- whenever the field is present -- this loop never calls `record.*`
        -- itself (see the module comment), so it hands over the data rather
        -- than the call.
        local plan_for_recording = plan_for_record(plan_steps)

        if steps == 0 then
            -- An empty plan is not satisfaction. It was the only signal this
            -- loop had, so it stood in for one, and this branch used to carry
            -- a paragraph explaining that the substitution was an assumption
            -- it could not check: within `crates/planner`'s registry
            -- `AlreadySatisfied` claims a met goal before any other method is
            -- consulted and every other method refuses a goal with no
            -- shortfall, so an empty plan did mean the goal held -- but that
            -- is an internal invariant of that crate, not a contract the
            -- `goal.*` surface exposed, and reporting satisfaction on it was
            -- a guess.
            --
            -- `goal.holds` is that contract. It reads the same world snapshot
            -- `goal.plan` reads, for the same roster, and answers the question
            -- directly. The planner pins the agreement between the two
            -- (`an_empty_expansion_and_a_held_goal_agree`), so this is a check
            -- that should never fire -- which is exactly what makes it worth
            -- making, because the run it would have caught reported `done` for
            -- a milestone nothing was done for.
            --
            -- Three answers, and only one of them is a satisfaction.
            --
            -- `true` is satisfaction, and says so.
            --
            -- `false` is a contradiction between the planner and the world,
            -- and there is nothing sensible to do with it but stop: it is a
            -- defect in the planner, not a condition of the world, so it
            -- raises for the same reason `goal.plan`'s own construction errors
            -- are left unwrapped above. Retrying it would burn the iteration
            -- cap and then report "stuck", which would blame the world for a
            -- bug.
            --
            -- `nil` is "no method here can answer this goal at all" -- and it
            -- used to be reported as `satisfied`, reason `plan_empty`. That
            -- was the one branch of this loop that could still report a
            -- milestone done on no evidence whatsoever, and `plan_empty` is a
            -- `SatisfiedReason` (`crates/core/src/record/splits.rs`,
            -- `record/mod.rs`), so the absence of a verdict was recorded as
            -- success. It cost nothing while every goal a script could build
            -- was a `have` or a `researched` -- both of which `goal.holds`
            -- answers -- and it becomes a factory reported built on nothing
            -- the moment a goal kind `holds` returns `nil` for (today
            -- `Produced` and `Producing`, `crates/planner/src/method/have.rs`)
            -- can reach a method that emits no steps, for ANY reason: a site
            -- it believes is already built, a shortfall it computed as zero, a
            -- refusal it swallowed.
            --
            -- So `nil` closes the milestone `stuck`, and it is deliberately
            -- the same shape a planner refusal takes two branches up -- a
            -- verdict that halts cleanly, carrying its reason as data on
            -- `t.refusal` rather than earning a word of its own. Three reasons
            -- it belongs in that family rather than beside it:
            --
            --   * The outcome vocabulary says how FAR a milestone got, not
            --     what kind of stop it was (`record/splits.rs` argues exactly
            --     this for refusals, and a word nothing consumes is a word the
            --     viewer has no style for). "As far as this loop can
            --     establish" is `stuck`; not `stuck_silent`, which means "no
            --     progress and nothing to show for it", and this has a
            --     sentence to show.
            --   * `t.refusal` is already the channel a driver reads for "why
            --     the milestone closed with no action dispatched"
            --     (`scripts/research_run.lua` hands `t.refusal.message` to
            --     `record.milestone_stuck`). A second field would have to be
            --     taught to every driver, and a driver that had not learnt it
            --     would record this halt with no reason at all.
            --   * `code` is what keeps the two apart for a reader: a planner
            --     refusal carries the planner's own miette code
            --     (`planner::research_needs_power`), and this carries
            --     `supervisor::unanswerable`, which no planner error can
            --     produce.
            --
            -- It does NOT raise, unlike `false`: nothing is broken. `holds`
            -- answering `nil` is the planner correctly declining to model
            -- something, and a run must be able to end on that with its record
            -- intact -- which is the lesson run 31 paid for.
            --
            -- Nor is it retried, for the refusal's own reason: re-planning
            -- asks the same question of the same world -- nothing ran, so
            -- nothing changed -- and would get the same answer until the cap
            -- turned a stated reason into `exhausted`.
            local held = goal.holds(self.milestone, { bots = self.bots })
            if held == false then
                error("supervisor: the planner returned no steps for milestone "
                    .. tostring(self.index) .. " (" .. tostring(self.milestone)
                    .. "), but the goal does not hold; refusing to report it satisfied")
            end
            if held ~= true then
                self.refusal = {
                    code = "supervisor::unanswerable",
                    message = "the planner returned no steps for milestone "
                        .. tostring(self.index) .. " (" .. tostring(self.milestone)
                        .. ") and cannot say whether the goal holds; an "
                        .. "unanswerable goal is not a satisfied one",
                }
                self:_close("stuck")
                self.state = "stuck"
                return { action = "halted", state = "stuck",
                         milestone_index = self.index, steps = 0,
                         iteration = self.iterations,
                         plan = plan_for_recording,
                         refusal = self.refusal }
            end
            self:_close("satisfied")
            self.state = "acquiring"
            return { action = "satisfied", state = "acquiring",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations,
                     plan = plan_for_recording,
                     reason = "already_satisfied" }
        end

        self.plan = plan
        self.iterations = self.iterations + 1
        table.insert(self.step_counts, steps)
        self.tracker = tracker.observe(self.tracker, steps)

        local t = {
            action = "planned", milestone_index = self.index, steps = steps,
            best = self.tracker.best, stall = self.tracker.stall,
            iteration = self.iterations,
            plan = plan_for_recording,
            -- The roster the planner actually expanded against, read off the
            -- plan itself rather than from `self.bots`. It is the field
            -- `record.plan_created` records, and nothing downstream of here
            -- can work it out: a driver that left it out got the roster the
            -- *process* was started with, which is a different set whenever
            -- fewer bots connected than were asked for. Run 30 recorded
            -- `bots: [1, 2]` for a plan made for `[2]` alone that way.
            bots = plan.bots,
        }

        -- The stall and the cap mean opposite things and must not share a
        -- name: the stall fires because progress STOPPED, the cap because
        -- progress was still happening and merely slow.
        if self.tracker.stall >= self.stall_limit then
            local outcome = self.any_failures and "stuck" or "stuck_silent"
            self:_close(outcome)
            self.state = outcome
            t.action, t.state = "halted", outcome
            return t
        end
        if self.iterations >= self.max_iterations then
            self:_close("exhausted")
            self.state = "exhausted"
            t.action, t.state = "halted", "exhausted"
            return t
        end

        self.state = "running"
        t.state = "running"
        return t
    end

    -- running: one plan execution. Blocks; one step is one run.
    local plan = self.plan
    self.plan = nil
    -- Captured before the run, because a consumed plan is not a thing to go
    -- reading fields off afterwards.
    local steps = plan.steps
    local obs = goal.run(plan)
    -- Four counts, two axes, all four kept apart.
    --
    --   failed        the game judged the action and said no
    --   lost          the game acknowledged it and never answered
    --   walks_failed  the same, for a walk (usually: the pathfinder refused)
    --   walks_lost    the same, for a walk (usually: a stuck-walk recovery
    --                 spun until the executor stopped waiting)
    --
    -- The walk terms exist because a walk has no action id, so it is counted
    -- in neither `failed` nor `lost`, and when one does not succeed the rest
    -- of that bot's slice is abandoned -- leaving every action `pending` and
    -- the whole run looking like `failed = 0`. Without them a run that
    -- dispatched nothing at all was reported `stuck_silent` with
    -- `last_error: null`, which says the opposite of what happened.
    -- `run-1788341905-92036` is that run for `walks_failed`;
    -- `run-1788344167-58471` is it for `walks_lost` (four executor deadlines
    -- on one spinning recovery).
    --
    -- `trouble` is the SUM, and it is deliberately a local: it decides whether
    -- this run counts as one that went wrong, and nothing else. It used to be
    -- returned as `t.failed`, next to a separate `t.lost` -- so a run with a
    -- single lost action printed `failed=1 lost=1`, which reads as two
    -- problems, is one, and misdirected a live diagnosis. A name that means
    -- "the game said no" must never carry a total.
    local trouble = (obs.failed or 0) + (obs.lost or 0)
        + (obs.walks_failed or 0) + (obs.walks_lost or 0)
    if trouble > 0 then
        self.any_failures = true
        if self.first_error == nil then
            self.first_error = obs.first_error
        end
    end
    self.state = "planning"
    -- `steps` and `actions` ride along so a caller can record what each bot
    -- actually did: the plan knows which bot owns an action and what it is
    -- called, the observation knows when the game ran it and how it ended, and
    -- neither half carries both.
    --
    -- `walks` rides along for the same reason and is a THIRD thing, not part
    -- of either: a walk is not an action, it has no action id, and it is in
    -- neither `steps` nor `actions`. It is also most of the wall clock, and
    -- until it was carried here no walk reached the run record at all -- a run
    -- could fail three walks and leave one error string behind. See
    -- `record.walks`.
    -- The full tally, not just `failed`. A run that dispatched everything and
    -- learned nothing back and a run that dispatched nothing at all both report
    -- `failed = 0`, and they are completely different events -- the first is
    -- alarming, the second is usually a plan whose actions were all walks.
    -- `pending` is what separates them, so a caller should not have to guess.
    --
    -- The four trouble counts are normalised to numbers rather than passed
    -- through: a driver prints these, and `nil` printed as "nil" reads as
    -- "unknown" where "none" is what happened.
    return { action = "ran", state = "planning", milestone_index = self.index,
             failed = obs.failed or 0, lost = obs.lost or 0,
             walks_failed = obs.walks_failed or 0,
             walks_lost = obs.walks_lost or 0,
             first_error = obs.first_error,
             iteration = self.iterations,
             done = obs.done, pending = obs.pending, running = obs.running,
             success = obs.success,
             steps = steps, actions = obs.actions, walks = obs.walks }
end

--- Human-readable summary of everything closed so far.
function Sup:report()
    local lines = { "supervisor: " .. self.state }
    for _, h in ipairs(self._history) do
        local best = "-"
        for _, n in ipairs(h.step_counts) do
            if best == "-" or n < best then best = n end
        end
        -- One suffix, and the halt's own reason leads. A refusal is not an
        -- error the run made -- no action was dispatched, nothing failed -- so
        -- calling it "last error" would invent a failure that never happened;
        -- and a milestone that both failed a run and was then refused reports
        -- the refusal, because that is what closed it. The other text is still
        -- on the history entry for a reader who wants it.
        local why = ""
        if h.refusal then
            why = ", refused: " .. h.refusal.message
        elseif h.first_error then
            why = ", last error: " .. h.first_error
        end
        lines[#lines + 1] = string.format(
            "  milestone %d: %s after %d iteration(s), best %s steps%s",
            h.index, h.outcome, h.iterations, tostring(best), why)
    end
    return table.concat(lines, "\n")
end
