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
-- The ring-widening source: going and LOOKING for something the plan needs
-- ---------------------------------------------------------------------------
--
-- `Goal::Charted` and its method (`crates/planner/src/method/scout.rs`) have
-- been complete and registered since 2026-09-08, and nothing emitted the
-- goal. That module's own doc says why, and says where the missing piece
-- goes:
--
--     **Stop on find.** A plan is expanded before anything runs, so nothing
--     at expansion time can know what a survey will reveal ... It belongs one
--     level up and is *already expressible*: because `Goal::Charted` is
--     idempotent and `PlanState` reads the live world, a supervisor that
--     plans ring by ring -- widening the radius and re-planning -- stops the
--     moment the goal it actually wanted stops raising `NotCharted`.
--
-- This is that loop, as a milestone source. It was written twice inline
-- before it was written once here (`scripts/chart_then_drill.lua` and
-- `scripts/chart_water_and_oil.lua` each carry a hand-rolled copy over a
-- literal radius list); those loops drive `goal.run` directly and so have
-- none of the recovery, stall, re-roster or record plumbing every other
-- milestone gets. A survey is the run where a bot walks furthest from help,
-- which makes it the *last* thing that should be executed off the supervisor.
--
-- # The stop condition is the TARGET's refusal, not a sighting
--
-- The source never asks "is there crude oil yet". It cannot: naming what to
-- look for would make this a search for a specific resource, and the goal
-- that wants it already answers the question exactly. Before every milestone
-- it plans the target and reads `goal.refusal`:
--
--   * `planner::not_charted` -- the ONE refusal a bigger disc can clear
--     (`method::extract::not_charted`; the message names where charted ground
--     ends). Issue the next ring.
--   * anything else, including no refusal at all -- charting is finished,
--     whatever the world's answer turns out to be. Issue the target and stop.
--
-- The probe costs one expansion per ring, and a *refusing* expansion is cheap
-- -- it stops at the missing resource. Exactly one probe, the last, expands
-- the whole target, and the supervisor then plans it again for real. That
-- second expansion is the price of the source not being allowed to hand the
-- supervisor a plan (it hands goals, and a plan made before a re-roster is a
-- plan for the wrong roster).
--
-- A raise the classifier does not vouch for propagates, exactly as it does in
-- `Sup:step()`: "cannot classify" must never come out as "carry on".
--
-- # The bound, and what exhaustion looks like
--
-- `max_rings` (default `CHART_MAX_RINGS`), counted in lattice rings and
-- reported in tiles as well, because those are the two units the answer is
-- ever quoted in. Widening forever on a map that simply has no oil is not an
-- option, and neither is stopping silently.
--
-- On exhaustion the source issues **the target anyway**, so the run ends on
-- the planner's own `planner::not_charted` -- with its frontier sentence --
-- recorded as a stuck milestone by whatever driver is already reading
-- `t.refusal`. Success and exhaustion are then distinguishable three ways:
-- `census.exhausted` is true, `census.reason` says `"exhausted"` rather than
-- `"plannable"` or `"refused:<code>"`, and the final milestone halts instead
-- of running. `census.reason` is `nil` only while the source has never been
-- asked -- absent is not a value.
--
-- # Where the rings are, and why they are not aimed
--
-- Blind, concentric, and centred on `around` (the spawn by default). Nothing
-- here reads a resource position: the roster has not charted the ground, so
-- reading it would be exactly the foreknowledge a human speedrunner's map
-- preview is disclosed for. A ring search that finds oil by walking is
-- honest; one that walks at oil it looked up is not.

--- The lattice pitch `method::scout::REVEAL_PITCH` measures, in tiles.
-- A character makes the engine generate 9x9 chunks around itself, so a visit
-- buys +/-128 and points 256 apart tile the plane. Mirrored here rather than
-- asked, because no binding exposes it; if it moves there it must move here.
supervisor.CHART_PITCH = 256

--- What one visit reveals around itself, in tiles: half the pitch.
supervisor.CHART_REVEAL = 128

--- The default bound, in rings.
--
-- Three, which is `radius = 896` and the same three radii the two inline
-- loops used. Chosen on cost, not on where anything is: ring `k` is `8k`
-- survey points at ~`256k` tiles out, and the one measured ring (ring 1, on
-- seed 31337) cost **8 surveys, 0 failures, 4,159 ticks**. Walking scales
-- with points x distance, so rings 1..3 are roughly 1 + 4 + 9 = 14 ring-1s,
-- about 58,000 ticks of game time. Ring 4 alone would add another 16.
supervisor.CHART_MAX_RINGS = 3

--- The radius that asks for rings 0..k of the survey lattice.
--
-- `survey_plan` takes `rings = floor(radius / 256)`, so any radius in
-- `[256k, 256k + 255]` names the same k. `256k + 128` is chosen among them
-- because it is **the ground the survey actually reaches**: a bot standing at
-- `256k` reveals to `256k + 128`. So the number in the goal, in the log and
-- in the record is the number that was looked at, and 384 / 640 / 896 are the
-- radii this project has already quoted.
function supervisor.chart_radius(rings)
    return supervisor.CHART_PITCH * rings + supervisor.CHART_REVEAL
end

local Chart = {}
Chart.__index = Chart

--- The name the record should show for milestone `index`, or nil.
-- The source issues more than one milestone, so a driver cannot hold a single
-- constant any more; it asks here.
function Chart:name_of(index)
    return self.names[index]
end

--- Build a **ring-widening** milestone source: chart outwards until `target`
--- stops refusing `planner::not_charted`, then run `target`.
--
--     local ch = supervisor.chart_until { target = milestone }
--     local sup = supervisor.new(ch.source, { bots = bots })
--
-- Returns the object, not the function, so the census survives the run: a
-- source that returned two values would be spliced into `supervisor.new`'s
-- `opts` argument by Lua's own call semantics.
--
-- @param spec.target       the goal the rings exist to make plannable
-- @param spec.around       centre of the rings, `{ x = , y = }`, default 0,0
-- @param spec.max_rings    the bound, in rings; default `CHART_MAX_RINGS`
-- @param spec.plan_opts    opts for the probe -- a table, or a function
--                          returning one, so the probe can be made against
--                          the roster as it stands rather than as it started
function supervisor.chart_until(spec)
    if type(spec) ~= "table" then
        error("supervisor.chart_until: expected a table of options")
    end
    if spec.target == nil then
        error("supervisor.chart_until: `target` is required and has no default")
    end
    local around = spec.around or { x = 0, y = 0 }
    if type(around) ~= "table" or type(around.x) ~= "number" or type(around.y) ~= "number" then
        error("supervisor.chart_until: `around` must be a position, `{ x = ..., y = ... }`")
    end
    local max_rings = spec.max_rings or supervisor.CHART_MAX_RINGS
    if type(max_rings) ~= "number" or max_rings < 1 or max_rings % 1 ~= 0 then
        error("supervisor.chart_until: `max_rings` must be a positive integer")
    end
    if spec.plan_opts ~= nil and type(spec.plan_opts) ~= "table"
        and type(spec.plan_opts) ~= "function" then
        error("supervisor.chart_until: `plan_opts` must be a table, a function, or nil")
    end

    local self = setmetatable({
        target = spec.target,
        around = { x = around.x, y = around.y },
        max_rings = max_rings,
        plan_opts = spec.plan_opts,
        -- The census. Every field here is read after the run, so none of them
        -- may be inferable-only: `rings` is what was walked, `radii` is where,
        -- `probes` is how many expansions the widening cost, `reason` is why
        -- it stopped and `exhausted` says whether that was the bound.
        rings = 0,
        radii = {},
        probes = 0,
        reason = nil,
        exhausted = false,
        last_code = nil,
        -- The milestone index the TARGET was issued at, once it has been.
        -- `nil` until then, never 0: absent is not a value.
        target_index = nil,
        names = {},
        _index = 0,
        _done = false,
    }, Chart)

    --- Plan the target and answer its refusal code: nil when it plans, the
    -- code when it refuses. A raise nothing vouches for is re-raised.
    local function probe()
        local opts = self.plan_opts
        if type(opts) == "function" then opts = opts() end
        self.probes = self.probes + 1
        local ok, err
        if opts == nil then
            ok, err = pcall(goal.plan, self.target)
        else
            ok, err = pcall(goal.plan, self.target, opts)
        end
        if ok then return nil end
        local refusal = refusal_of(err)
        if refusal == nil then error(err, 0) end
        return refusal.code, refusal.message
    end

    local function issue(milestone, name)
        self._index = self._index + 1
        self.names[self._index] = name
        return milestone
    end

    --- Issue the target and stop. `target_index` is what a driver keys on to
    -- tell the milestone it cares about from the rings that made it
    -- reachable: a survey that settles every action it dispatched is not the
    -- rig standing, and a driver with one counter for both would say it was.
    local function issue_target(name, reason, message, exhausted)
        self._done = true
        self.reason = reason
        self.last_message = message
        self.exhausted = exhausted or false
        local g = issue(self.target, name)
        self.target_index = self._index
        return g
    end

    self.source = function(_history)
        if self._done then return nil end
        local code, message = probe()
        self.last_code = code
        if code ~= "planner::not_charted" then
            -- Either it plans now, or it refuses for something no amount of
            -- looking can change. Both end the widening; only one of them is
            -- good news, and the census says which.
            return issue_target("target: the goal the rings were for",
                (code == nil) and "plannable" or ("refused:" .. code), message)
        end
        if self.rings >= self.max_rings then
            -- The bound. Issue the target regardless: the run then ends on the
            -- planner's own not-charted sentence, in the record, rather than
            -- on this loop quietly running out of milestones -- which the
            -- supervisor would report as `done`.
            return issue_target(
                string.format("target after %d ring(s): STILL not charted", self.rings),
                "exhausted", message, true)
        end
        self.rings = self.rings + 1
        local radius = supervisor.chart_radius(self.rings)
        self.radii[#self.radii + 1] = radius
        return issue(goal.charted(self.around.x, self.around.y, radius),
            string.format("chart ring %d of %d: %d tiles around (%g, %g)",
                self.rings, self.max_rings, radius, self.around.x, self.around.y))
    end

    return self
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

--- Build a **sustain** milestone: the rung a `Goal::Sustain` is measured over.
--
--     supervisor.sustain {
--         item = "iron-plate", per_minute = 15,
--         window_ticks = 7200, lead_in_ticks = 9600,
--     }
--
-- Like `supervisor.witness` it dispatches nothing, and unlike it **it does not
-- reach a verdict about the rate**. It cannot, and saying why is the point:
--
--   * The measurement a standing goal needs is over **per-machine lifetime
--     counters** (`produced` / `produced_source`), and those ride on the mod's
--     sampling stream into `samples.jsonl`. They are **not** on the entities
--     `rcon.find_entities_in_radius` hands back -- `FactorioEntity` carries
--     `output_inventory` and `fuel_inventory` and no counter at all -- so a
--     script in the game cannot read them. `tools/run_analysis.py`'s
--     `sustained_rate` reads them from the record afterwards.
--   * An output-inventory count, which is what the witness uses, conflates
--     *made* with *moved* and cannot tell a machine finishing a queue from one
--     being resupplied. That is exactly the confusion a standing goal exists
--     to end, so this rung must not inherit it.
--
-- What it therefore does is establish the **conditions** the measurement needs
-- and nothing more: it holds the roster idle for `lead_in_ticks +
-- window_ticks`, so that the record's trailing window and the lead-in before
-- it contain no feeding dispatch this run could have made. Without the lead-in
-- the window proves nothing: a stone furnace's input slot holds one stack of
-- 50 ore at 3.2 s a plate -- 9,600 ticks of hand-fed running -- so every bot
-- can be idle for a 90-second window and every item in it can come from a
-- charge made before it began.
--
-- Neither duration has a default, for `within_ticks`' reason: they decide what
-- a failure means. `lead_in_ticks` in particular **cannot** be derived from
-- the record at all -- the mod does not report input slots -- so it is stated
-- by the caller from the machines' input capacity and consumption rate.
--
-- It closes `satisfied` when the clock really advanced the whole span, and
-- `stuck` (`supervisor::sustain_inconclusive`) when it did not. **`satisfied`
-- here means "the window happened", never "the rate held"**, and the
-- observation it carries says so in `verdict = "deferred"` along with the
-- exact command that answers it. A record that answered a question nobody
-- asked would be the confidently-wrong-object failure this project has already
-- paid for twice.
function supervisor.sustain(spec)
    if type(spec) ~= "table" then
        error("supervisor.sustain: expected a table of options")
    end
    if type(spec.item) ~= "string" or spec.item == "" then
        error("supervisor.sustain: `item` must be a string naming an item")
    end
    local function positive_integer(key, default)
        local v = spec[key]
        if v == nil then
            if default == nil then
                error("supervisor.sustain: `" .. key .. "` is required and has no default")
            end
            return default
        end
        if type(v) ~= "number" or v <= 0 or v ~= math.floor(v) then
            error("supervisor.sustain: `" .. key .. "` must be a positive integer")
        end
        return v
    end
    local window_ticks = positive_integer("window_ticks")
    local lead_in_ticks = positive_integer("lead_in_ticks")
    return {
        __sustain = true,
        item = spec.item,
        per_minute = positive_integer("per_minute"),
        window_ticks = window_ticks,
        lead_in_ticks = lead_in_ticks,
        probe_ticks = positive_integer("probe_ticks", 60),
        -- Same cap and same reason as the witness's: a game whose clock is not
        -- advancing must end the loop with a verdict of its own rather than
        -- spinning, and hitting it is `inconclusive`, never a failed rate.
        max_polls = positive_integer("max_polls", window_ticks + lead_in_ticks + 600),
    }
end

--- Is this milestone a sustain window rather than a goal or a witness?
function supervisor.is_sustain(milestone)
    return type(milestone) == "table" and milestone.__sustain == true
end

local Sup = {}
Sup.__index = Sup

local TERMINAL = { done = true, stuck = true, stuck_silent = true, exhausted = true }

function supervisor.new(source, opts)
    if type(source) ~= "function" then
        error("supervisor.new: source must be a function(history) -> goal | nil")
    end
    opts = opts or {}
    if opts.roster ~= nil and type(opts.roster) ~= "function" then
        error("supervisor.new: roster must be a function() -> {bot ids}, or nil")
    end
    return setmetatable({
        source = source,
        bots = opts.bots,
        -- Who the game has RIGHT NOW, asked before every plan. `nil` keeps
        -- the roster fixed for the whole run, which is what every caller did
        -- before this existed and what a test with no game still does.
        --
        -- The roster used to be computed once, at construction, and never
        -- again. A bot that dies keeps its `LuaPlayer` and loses its
        -- character; `rcon.players()` stops listing it; and every plan made
        -- for it from then on dispatched work to a bot the game refused,
        -- action by action, with nothing in the record saying why. That is
        -- the failure `docs/superpowers/specs/2026-09-04-exploration-design.md`
        -- names as the prerequisite for sending a bot anywhere near a nest.
        --
        -- The function is the driver's, not this loop's, because it is the
        -- driver that has `rcon` -- and because it may PACE: each call is one
        -- poll, and a driver whose poll costs a game tick gets the bound
        -- below in ticks.
        roster = opts.roster,
        -- How many polls of `roster` a rostered bot may be missing from
        -- before it is dropped. Polls, not ticks: this loop has no clock.
        -- Each poll is at least one RCON round trip, which the game answers
        -- on a tick, so 800 polls is at least 800 ticks -- past the 600 a
        -- default character takes to respawn (`respawn_time = 10` s in the
        -- prototype). A bot back within the bound costs the wait and nothing
        -- else; one still absent after it is dropped from the roster, with
        -- `left`/`reason` on the `rerostered` transition saying so.
        roster_patience = opts.roster_patience or 800,
        -- The roster this run STARTED with. The only ids a re-roster may ever
        -- add back: a bot that returns is one that was here, and a client
        -- that appears mid-run is not picked up, because "the first non-empty
        -- answer is the roster" is the bug that froze run 30 on `[2]`
        -- (`docs/superpowers/notes/2026-09-02-bot-one-idle.md`) and this
        -- must not be a second road to it.
        _roster_origin = (type(opts.bots) == "table") and { table.unpack(opts.bots) } or nil,
        stall_limit = opts.stall_limit or 3,
        max_iterations = opts.max_iterations or 50,
        -- How many tier-1 recoveries one plan LINEAGE may have before the loop
        -- gives up and re-plans. Two, so a plan is executed at most three
        -- times. `0` turns recovery off and restores the pre-recovery
        -- behaviour exactly, which is the escape hatch a run wants when a
        -- recovery is suspected of hiding something.
        --
        -- The cap is the supervisor's duty and cannot be moved inside
        -- `recover()`: that function is pure and has no memory of having been
        -- asked before, and `crates/executor/src/recover.rs` says so in as many
        -- words.
        recovery_limit = opts.recovery_limit or 2,
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
        -- The three fields below belong to a plan LINEAGE -- one plan plus the
        -- tier-1 proposals descended from it -- not to a milestone, and are
        -- reset by `_new_lineage` for every fresh `goal.plan` result.
        recoveries = 0,
        -- `obs.success` as it stood when the last recovery was accepted, or
        -- `nil` while this lineage has not recovered yet. The counter is
        -- CUMULATIVE across a chain -- `build_observation` iterates the
        -- handed-in log, and a tier-1 proposal carries the previous log
        -- forward -- so "did this recovery achieve anything" is a delta
        -- against this baseline and can never be an absolute.
        chain_success = nil,
        -- The `(bot, step_index, dispatched_tick)` of every walk already handed
        -- to a driver for this lineage. Same cause as `chain_success`: the
        -- carried log keeps the walks the narrower recovery schedule never
        -- re-walked, `obs.walks` re-offers them, and `record.walks` would write
        -- them a second time at a tick earlier than the record's position.
        _walks_seen = nil,
        -- The proposal accepted by a "ran" transition and replayed by the one
        -- after it. Held rather than acted on in place so the run that produced
        -- it still reports itself as a run: a driver records `record.actions`
        -- and `record.walks` on `t.action == "ran"` and `record.plan_created`
        -- on `t.action == "planned"`, and one transition cannot be both, so
        -- collapsing the two would drop one of the recordings on the floor.
        _recovery = nil,
    }, Sup)
end

--- Reset everything that belongs to a plan lineage rather than to a milestone.
-- A recovery budget must never be inherited by a plan that did not spend it,
-- and a walk of the previous plan must never suppress a walk of the next one.
function Sup:_new_lineage()
    self.recoveries = 0
    self.chain_success = nil
    self._walks_seen = {}
    self._recovery = nil
end

--- The walks of `list` this lineage has not already handed to a driver, plus
--- how many of those failed and how many were lost.
--
-- Pure apart from marking `seen`. The identity is `(bot, step_index,
-- dispatched_tick)`: `ExecutionLog::start_walk` keys walks by the first two and
-- overwrites, so a re-walk of the same slot arrives with a new dispatch tick
-- and is correctly new, while a survivor of the previous run arrives identical
-- and is correctly old.
--
-- The two counts are DERIVED from this list rather than read off
-- `obs.walks_failed` / `obs.walks_lost`, which count the whole carried log.
-- Deriving them is also the reading that does not depend on the
-- cumulative-counter argument being right: on a fresh log the list is the whole
-- log and the two agree exactly.
local function unseen_walks(seen, list)
    local out, failed, lost = {}, 0, 0
    if type(list) ~= "table" then return out, failed, lost end
    for _, w in ipairs(list) do
        if type(w) == "table" then
            local key = tostring(w.bot) .. "/" .. tostring(w.step_index)
                .. "@" .. tostring(w.dispatched_tick)
            if seen[key] == nil then
                seen[key] = true
                out[#out + 1] = w
                if w.status == "failed" then
                    failed = failed + 1
                elseif w.status == "lost" then
                    lost = lost + 1
                end
            end
        end
    end
    return out, failed, lost
end

function Sup:finished()
    return TERMINAL[self.state] == true
end

-- Which of `wanted` are not in `ids`.
local function missing_from(ids, wanted)
    local have = {}
    for _, id in ipairs(ids) do have[id] = true end
    local missing = {}
    for _, id in ipairs(wanted) do
        if not have[id] then missing[#missing + 1] = id end
    end
    return missing
end

--- Ask the game who it has, and change `self.bots` to match -- within rules.
--
-- Returns a change record `{ bots, left, returned, reason }` when the roster
-- changed, or nil when it did not (or could not be checked). Called before
-- every plan, and before a tier-1 recovery is accepted, because a schedule
-- made for a bot that has no character is work the game will refuse step by
-- step.
--
-- Three rules, and each one is a run this project has already lost:
--
--   * **Only REMOVE a bot that was rostered and is absent past the bound.**
--     A death is transient -- the character respawns in 600 ticks -- so an
--     absent bot is polled `roster_patience` times before it is dropped. A
--     roster that shrank on one unlucky poll would plan a milestone for three
--     bots while the fourth stood at spawn ten seconds later.
--   * **Only RE-ADD a bot that this run started with.** A returning bot is
--     picked up on the first poll that shows it; a bot that was never in
--     `_roster_origin` is never added, however many polls list it. See
--     `_roster_origin`.
--   * **An empty answer decides nothing.** `rcon.players()` answering `{}`
--     is every bot dead at once, or the game not answering -- and planning
--     for nobody is not a plan. The roster is kept and nothing is recorded,
--     which is the same as before this function existed.
function Sup:_reroster()
    if type(self.roster) ~= "function" or type(self.bots) ~= "table" then return nil end
    local origin = self._roster_origin or self.bots
    local latest, missing, polls = nil, {}, 0
    for attempt = 1, math.max(1, self.roster_patience) do
        polls = attempt
        local ok, ids = pcall(self.roster)
        -- By type and by length: a nil crossing the Rust bridge is mlua's
        -- null sentinel, which is truthy, and an empty list is inconclusive.
        if ok and type(ids) == "table" and #ids > 0 then
            latest = ids
            missing = missing_from(ids, self.bots)
            if #missing == 0 then break end
        end
    end
    if latest == nil then return nil end
    local have = {}
    for _, id in ipairs(latest) do have[id] = true end
    local kept, left, returned = {}, {}, {}
    for _, id in ipairs(self.bots) do
        if have[id] then kept[#kept + 1] = id else left[#left + 1] = id end
    end
    local rostered = {}
    for _, id in ipairs(self.bots) do rostered[id] = true end
    for _, id in ipairs(origin) do
        if have[id] and not rostered[id] then returned[#returned + 1] = id end
    end
    if #left == 0 and #returned == 0 then return nil end
    local bots = {}
    for _, id in ipairs(kept) do bots[#bots + 1] = id end
    for _, id in ipairs(returned) do bots[#bots + 1] = id end
    table.sort(bots)
    -- Never plan for nobody: a roster that would empty stays as it is, and
    -- the planner's own refusal is the one that names the situation.
    if #bots == 0 then return nil end
    local parts = {}
    if #left > 0 then
        parts[#parts + 1] = "bot(s) " .. table.concat(left, ", ")
            .. " had no character for " .. tostring(polls) .. " roster check(s)"
    end
    if #returned > 0 then
        parts[#parts + 1] = "bot(s) " .. table.concat(returned, ", ") .. " came back"
    end
    self.bots = bots
    return { bots = bots, left = left, returned = returned,
             reason = table.concat(parts, "; ") }
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

--- Hold the roster idle for a lead-in plus a window, and close.
--
-- See `supervisor.sustain` for why this reaches no verdict about the rate. It
-- has exactly two outcomes and neither of them is one:
--
--   * `satisfied` -- the game's clock advanced `lead_in_ticks + window_ticks`
--     with nothing dispatched. The window happened; what came out of the
--     machines during it is on the record, and `tools/run_analysis.py
--     --sustain <item>:<rate>:<window>:<lead-in>` is what answers it.
--   * `supervisor::sustain_inconclusive` -- the poll budget ran out before the
--     span did, so the wait did not happen and nothing follows from it. The
--     same refusal as `supervisor::witness_inconclusive` and for the same
--     reason: a wait that did not happen is not a rate that did not hold.
function Sup:_sustain(w)
    local span = w.lead_in_ticks + w.window_ticks
    local function observation(elapsed, polls, terminal)
        return {
            item = w.item, per_minute = w.per_minute,
            window_ticks = w.window_ticks, lead_in_ticks = w.lead_in_ticks,
            elapsed_ticks = elapsed, span_ticks = span, polls = polls,
            from_tick = terminal and (terminal - elapsed) or nil,
            at_tick = terminal,
            -- Said out loud, on the record, because the field a reader will
            -- assume this rung answers is the one it cannot.
            verdict = "deferred",
            answered_by = string.format(
                "tools/run_analysis.py --sustain %s:%d:%d:%d <run-dir>",
                w.item, w.per_minute, w.window_ticks, w.lead_in_ticks),
        }
    end
    local function halt(code, message, obs)
        self.refusal = { code = code, message = message }
        self:_close("stuck")
        self.state = "stuck"
        return { action = "halted", state = "stuck", milestone_index = self.index,
                 steps = 0, iteration = 0, refusal = self.refusal, sustain = obs }
    end

    -- A fault, not a verdict, exactly as the witness treats it: without a game
    -- the run was built wrong and nothing about the world is established.
    if type(rcon) ~= "table" or type(rcon.game_tick) ~= "function" then
        error("supervisor: milestone " .. tostring(self.index) .. " is a sustain "
            .. "window, but there is no game to wait in: `rcon` is missing "
            .. "`game_tick`", 0)
    end

    local t0 = rcon.game_tick()
    if type(t0) ~= "number" then
        return halt("supervisor::sustain_inconclusive", string.format(
            "milestone %d opens a %d-tick window for %s, but the game would not "
            .. "say what tick it is, so the wait cannot be timed and nothing "
            .. "follows from it", self.index, span, w.item),
            observation(0, 0, nil))
    end
    local now, polls = t0, 0
    while true do
        polls = polls + 1
        if polls > w.max_polls then
            return halt("supervisor::sustain_inconclusive", string.format(
                "milestone %d gave up after %d polls with only %d of %d ticks "
                .. "elapsed: the game's clock is not advancing (paused, saving "
                .. "or gone). No verdict about %s follows from a window that "
                .. "did not happen",
                self.index, polls - 1, now - t0, span, w.item),
                observation(now - t0, polls - 1, now))
        end
        local ok, tick = pcall(rcon.game_tick)
        if ok and type(tick) == "number" then now = tick end
        if now - t0 >= span then break end
    end

    self:_close("satisfied")
    self.state = "acquiring"
    return { action = "satisfied", state = "acquiring", milestone_index = self.index,
             steps = 0, iteration = 0,
             -- `already_satisfied` for the witness's reason: `SatisfiedReason`
             -- has two variants, both mirrored across the OpenAPI seam, and
             -- this is the one that is not a lie -- nothing was planned and
             -- nothing was asked to act.
             reason = "already_satisfied",
             sustain = observation(now - t0, polls, now) }
end

-- ---------------------------------------------------------------------------
-- Recovery: continuing a plan instead of throwing it away
-- ---------------------------------------------------------------------------
--
-- A replan is not only a recomputation, it is a **re-decision of layout**, and
-- half the layout is already built. `run-1788481380-80843` planned 194 steps,
-- succeeded at 154 of them, failed **one** placement on a character that moved
-- 53 ticks later, and replanned: 11,966 ticks of execution discarded and the
-- whole power plant re-sited 65 tiles from a pole that was already standing and
-- is now garbage nothing in the system knows about. Across 21 archived runs,
-- **38 of 57 replans had a trigger tier 1 is designed for**.
--
-- So: when a run does not finish its plan, ask `obs:recover()` first, and take
-- the answer only when it is tier 1 (`"rescheduled"`) -- the same actions minus
-- the succeeded ones, re-scheduled against the world as it is now, with every
-- retained action's preconditions re-checked. See
-- `docs/superpowers/specs/2026-09-04-recovery-instead-of-replan-design.md`.
--
-- Five refusals, and none of them is decoration:
--
--   * **`"reexpanded"` is refused.** Tier 2 is a replan by another name -- a
--     new network numbered from zero against a fresh log -- and taking it here
--     would bypass the `iterations` cap and the stall that already handle
--     replanning correctly, while pretending in the record to be a
--     continuation.
--   * **Any run with a LOST action is refused.** `recover()` retires an action
--     only on `Success`, so a `Lost` one comes back in the proposal and gets
--     re-dispatched -- and `Lost` means "dispatched, no verdict", not
--     "did not happen". `Insert`, `Remove` and `Place` are not idempotent:
--     re-taking a `take 13 coal` that actually landed empties the chest twice.
--     `recover.rs` states plainly that a caller who cannot tolerate that must
--     check the log itself. This is that check, and it costs 3 of 57 archived
--     replans' worth of benefit.
--   * **At most `recovery_limit` recoveries per lineage.**
--   * **A recovery that adds no successes is not a recovery.** This is the
--     loop breaker, and without it the FIRST walk-only failure spins forever.
--     A failed walk halts its bot and `abandon_rest` publishes `Failed` on the
--     watch channels while writing **nothing to the log**, so the abandoned
--     actions stay `Pending` and no action is ever dispatched again;
--     `exhausted_tier_one` looks for `Failed` with `attempts >= 3`, so its
--     budget is denominated in a unit this failure never produces and
--     `recover()` proposes `Rescheduled` forever. A walk is also the DOMINANT
--     failure -- 76 failed walks against 28 failed/lost actions, and 28 of 57
--     replans had no action failure at all -- so this is the common path, not
--     the corner.
--   * **A run whose failure is a DIVERGENCE is refused, and not even asked.**
--     `tried to remove 64 iron-plate but removed 40` is not a circumstance; it
--     is the game saying the plan's model of that container was wrong -- and
--     the 40 are already in the bot's hands. A reschedule issues the identical
--     take against the identical container, which can only answer with a
--     smaller number. `run-1788552801-73005` (green, seed 31337), batch 3,
--     spent a whole recovery learning exactly that:
--
--         first error: ... ["tried to remove 64 iron-plate but removed 40"]
--         planned 79 steps (best 302) -- recovered: rescheduled 1
--         first error: ... ["tried to remove 64 iron-plate but removed 0"]
--
--     `recover()` now refuses tier 1 for this class itself
--     (`crates/executor/src/recover.rs`, `diverged_from_the_world`), so the
--     ask would come back `reexpanded` and be declined anyway; refusing here
--     saves the `PlanState::from_world` and the expansion, and lets the
--     transition say why (`t.not_recovered = "divergence"`). A replan re-reads
--     the world -- the plates in the bot's inventory, the cell's real contents
--     -- which is what a resized take could not do without pretending to be a
--     reschedule while changing every downstream consumer's arithmetic.
--
-- Three things a recovery deliberately does NOT touch:
--
--   * `tracker`, `iterations` and `step_counts`. A tier-1 proposal's step count
--     is the REMAINDER -- 39 where the plan had 194 -- and feeding that to
--     `tracker.observe` reads as an enormous improvement and resets `stall`,
--     hiding a genuine one.
--   * `obs.walks`, beyond filtering it (`unseen_walks`).
--   * the `goal.plan` call. A recovery costs no planner expansion, which is the
--     whole point.

--- The world-model divergence a failed action's verdict reports, or nil.
--
-- The mod's transfer handlers complain, in three wordings, when the count
-- that moved is not the count the plan asked for:
--
--   tried to remove 64 iron-plate but removed 40
--   tried to insert 17x coal but inserted 3
--   cannot insert 20x iron-ore, because player #1 only has 18. clamping...
--
-- Each reaches `obs.actions[id].error` wrapped in `game rejected the command:
-- Unexpected Response: [...]`, so this matches the inner sentence. The same
-- three sentences are parsed by `crates/executor/src/divergence.rs` (which is
-- what makes `recover()` refuse the reschedule) and classified as
-- `partial_transfer` by `record.actions`; this is the third reader, and it
-- exists so the loop can refuse BEFORE asking and name the reason.
--
-- Returns `{ item, asked, moved }`. Parsed rather than substring-matched so a
-- wording change costs a nil -- the failure is then an ordinary one and the
-- ask is made, which is the behaviour before this existed -- rather than a
-- refusal on a sentence nobody read.
local function divergence(err)
    if type(err) ~= "string" then return nil end
    local asked, item, moved = err:match("tried to remove (%d+) (%S+) but removed (%d+)")
    if asked == nil then
        asked, item, moved = err:match("tried to insert (%d+)x (%S+) but inserted (%d+)")
    end
    if asked == nil then
        asked, item, moved = err:match("cannot insert (%d+)x (%S+), because .- only has (%d+)%. clamping")
    end
    if asked == nil then return nil end
    return { item = item, asked = tonumber(asked), moved = tonumber(moved) }
end
supervisor.divergence = divergence

--- The verdict of the lowest-id FAILED action of `obs` that is a divergence,
-- or nil. Lowest id rather than `pairs` order so two reads of one observation
-- name the same action. Falls back to `obs.first_error` when the binding hands
-- over no action table at all.
local function first_divergence(obs)
    if type(obs.actions) ~= "table" then
        return divergence(obs.first_error) and obs.first_error or nil
    end
    local ids = {}
    for id, a in pairs(obs.actions) do
        if type(a) == "table" and a.status == "failed" then ids[#ids + 1] = id end
    end
    table.sort(ids)
    for _, id in ipairs(ids) do
        local err = obs.actions[id].error
        if divergence(err) then return err end
    end
    return nil
end

--- May this observation be recovered rather than replanned?
-- `trouble` is this run's own trouble count, already de-cumulated.
--
-- Returns `false, reason` for a refusal of a run that HAD something to recover
-- (`"lost"`, `"limit"`, `"no_progress"`, `"divergence"`), and `false, nil` when
-- there was nothing to ask about in the first place -- recovery disabled, no
-- `recover` on the observation, nothing outstanding. The reason reaches the
-- driver as `t.not_recovered`, so a run that was replanned instead of
-- continued says which rule decided that; before this a refusal and a
-- recovery that was never available printed identically.
function Sup:_may_recover(obs, trouble)
    if (self.recovery_limit or 0) <= 0 then return false, nil end
    -- Absent rather than failing: every live `goal.run` installs `recover` on
    -- its observation, so this is a stub or an older binding, and the honest
    -- answer for "cannot ask" is the behaviour that existed before the question
    -- did -- replan.
    if type(obs) ~= "table" or type(obs.recover) ~= "function" then return false, nil end
    -- Nothing outstanding: `recover()` would answer `Complete` and we would
    -- have paid a `PlanState::from_world` to be told what we already know.
    if trouble <= 0 and (obs.pending or 0) <= 0 then return false, nil end
    if (obs.lost or 0) > 0 then return false, "lost" end
    -- Before the budget rules, because it is a fact about THIS run's verdict,
    -- not about the lineage: a diverged take is refused on a lineage's first
    -- ask exactly as on its last.
    if first_divergence(obs) ~= nil then return false, "divergence" end
    if self.recoveries >= self.recovery_limit then return false, "limit" end
    -- `nil` means this lineage has not recovered yet, so there is no previous
    -- recovery to have failed to make progress. The rule is about a recovery
    -- that achieved nothing, not about a first run that achieved nothing --
    -- a run whose opening walk failed has `success == 0` and is exactly the
    -- case tier 1 exists for.
    if self.chain_success ~= nil and (obs.success or 0) <= self.chain_success then
        return false, "no_progress"
    end
    return true, nil
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
        self:_new_lineage()
        self.state = "planning"
        return { action = "acquired", state = "planning", milestone_index = self.index }
    end

    -- The proposal the previous "ran" transition accepted, replayed as an
    -- ordinary plan.
    --
    -- It is `action = "planned"` on purpose: every driver already records
    -- `record.plan_created(t.milestone_index, t.plan, t.bots)` for that word,
    -- so **a driver that has never heard of recovery records the recovered plan
    -- correctly**, and `t.recovery` is there for one that has. The alternative
    -- -- folding the recovery into the "ran" transition -- would have silently
    -- cost that run its `record.actions` / `record.walks` call, because a
    -- driver's branch chain answers one word per transition.
    --
    -- No `goal.plan`, no `tracker.observe`, no iteration: this is the same
    -- plan, narrowed.
    if self.state == "recovering" then
        -- A proposal was made for the roster the plan was made for. If a bot
        -- has left it since, the remainder would hand that bot its share
        -- again; drop the proposal and let the planning branch below expand
        -- against who is actually there. The transition is the roster's, and
        -- the fresh plan follows on the next step.
        local change = self:_reroster()
        if change ~= nil then
            self._recovery = nil
            self.state = "planning"
            return { action = "rerostered", state = "planning",
                     milestone_index = self.index, bots = change.bots,
                     left = change.left, returned = change.returned,
                     reason = change.reason }
        end
        local proposal = self._recovery
        self._recovery = nil
        self.plan = proposal.plan
        self.recoveries = self.recoveries + 1
        self.state = "running"
        return { action = "planned", state = "running",
                 milestone_index = self.index, steps = #proposal.steps,
                 -- The tracker's own numbers, unchanged and re-reported, so a
                 -- driver printing "best N" prints the plan's best rather than
                 -- the remainder's.
                 best = self.tracker.best, stall = self.tracker.stall,
                 iteration = self.iterations,
                 plan = plan_for_record(proposal.steps),
                 bots = proposal.plan.bots,
                 -- Which tier answered, and how far into the budget we are.
                 -- Nothing downstream requires either yet -- until `S0` puts a
                 -- `cause` on `PlanCreated` there is no field in the record to
                 -- carry them -- but a driver can print them, and a run whose
                 -- log says "planned 39 steps" twice with no planner call
                 -- between is otherwise unreadable.
                 recovery = proposal.why, recoveries = self.recoveries }
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
        -- Same reasoning, one rung along: a sustain window dispatches nothing
        -- and must not cost the planner an expansion either, or the run whose
        -- window it opens would not be the run the plan described.
        if supervisor.is_sustain(self.milestone) then
            return self:_sustain(self.milestone)
        end

        -- Who is there to plan for, asked NOW rather than remembered from
        -- construction. A change is its own transition -- `rerostered`, with
        -- `bots`/`left`/`returned`/`reason` for `record.roster_changed` -- and
        -- the plan against the new roster is the next step's. A driver that
        -- has never heard the word skips it and still gets a plan whose
        -- `t.bots` says who it was made for.
        local change = self:_reroster()
        if change ~= nil then
            return { action = "rerostered", state = "planning",
                     milestone_index = self.index, bots = change.bots,
                     left = change.left, returned = change.returned,
                     reason = change.reason }
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
            -- ONE refusal is not a halt, and it is the one a standing goal
            -- ends on when it succeeds.
            --
            -- `Goal::Sustain` builds an arrangement -- a coal drill, a buffer,
            -- belts, burner inserters, the smelting cell -- and then, on the
            -- re-plan that finds all of it standing, refuses with
            -- `planner::sustain_supply_not_standing` rather than returning an
            -- empty plan. That refusal is deliberate and is argued in
            -- `crates/planner/src/method/sustain.rs`: an empty plan is this
            -- planner's word for "done", `goal.holds` answers nil for a
            -- sustain, and whether the RATE held is a fact about a window of
            -- history that no reading of the world settles. So the planner
            -- has two honest answers -- "here is what is still missing" and
            -- "nothing is missing and I cannot tell you whether it worked" --
            -- and only the first is a stuck milestone.
            --
            -- The milestone therefore closes `satisfied`, and the reason word
            -- it borrows (`already_satisfied`) is about **the buildable half
            -- only**: the world already holds every entity this goal can
            -- plan. It says nothing about the rate, and nothing here should be
            -- read as saying so -- `t.sustain_built` carries the refusal's own
            -- sentence so a driver prints what actually happened instead of
            -- the reason word. The verdict belongs to
            -- `tools/run_analysis.py --sustain`, exactly as
            -- `supervisor.sustain`'s does.
            if refusal.code == "planner::sustain_supply_not_standing" then
                self:_close("satisfied")
                self.state = "acquiring"
                return { action = "satisfied", state = "acquiring",
                         milestone_index = self.index, steps = 0,
                         iteration = self.iterations,
                         reason = "already_satisfied",
                         sustain_built = { message = refusal.message } }
            end
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
        -- The tick the plan was made at, for `record.plan_created` to stamp
        -- the event with. `nil` when the plan could not ask the game.
        plan_for_recording.tick = plan.tick

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
        -- A fresh plan is a fresh log, a fresh recovery budget and a fresh set
        -- of walks. Everything a lineage accumulated stops here.
        self:_new_lineage()
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
    --
    -- The two walk terms are **this run's own**, not `obs.walks_failed` and
    -- `obs.walks_lost`. Those count the whole log, and a tier-1 proposal runs
    -- against the log of the run it recovers, so on a recovery they include the
    -- previous run's failed walks and would report a clean recovery as having
    -- failed the walks that provoked it. `unseen_walks` de-cumulates both the
    -- list and the counts in one pass; on a fresh log it changes nothing,
    -- because there is nothing to have seen before.
    local walks, walks_failed, walks_lost = unseen_walks(self._walks_seen, obs.walks)
    if type(obs.walks) ~= "table" then
        -- No walk list to derive from (a stub, or a binding older than
        -- `obs.walks`). Fall back to the counters, and leave `t.walks` absent
        -- rather than substituting an empty table -- a driver reads `t.walks`
        -- by type and would otherwise be asked to record nothing at all.
        walks, walks_failed, walks_lost = nil, obs.walks_failed or 0, obs.walks_lost or 0
    end
    local trouble = (obs.failed or 0) + (obs.lost or 0) + walks_failed + walks_lost
    if trouble > 0 then
        self.any_failures = true
        if self.first_error == nil then
            self.first_error = obs.first_error
        end
    end

    -- Recover, or replan. `pcall` because `recover` raises rather than
    -- answering for a run that is not finished and for a roster that has lost
    -- a bot since the plan was made, and neither is a reason to end the run:
    -- both mean "replan", which is what the loop did before this branch
    -- existed.
    self.state = "planning"
    local recovery = nil
    -- Why this run is about to be replanned rather than continued, when a
    -- continuation was on the table at all: a `_may_recover` rule's name, or
    -- what became of the ask -- `"reexpanded"` (declined, see above),
    -- `"no_plan"` (an answer with nothing in it) or `"raised"`. Nil when the
    -- run is continued, and nil when there was nothing to ask about.
    local not_recovered = nil
    local may, refused = self:_may_recover(obs, trouble)
    if may then
        local ok, next_plan, why = pcall(obs.recover, obs)
        if ok and next_plan ~= nil and why == "rescheduled" then
            -- Read once: `PlanValue.steps` rebuilds the whole array per read.
            local next_steps = next_plan.steps
            if type(next_steps) == "table" and #next_steps > 0 then
                recovery = { plan = next_plan, steps = next_steps, why = why }
            else
                not_recovered = "no_plan"
            end
        elseif not ok then
            not_recovered = "raised"
        elseif next_plan == nil then
            not_recovered = "no_plan"
        else
            not_recovered = why
        end
    else
        not_recovered = refused
    end
    if recovery ~= nil then
        self._recovery = recovery
        -- The baseline the NEXT run has to beat. Set here rather than on every
        -- run so that it is what it says it is: the successes standing when
        -- this lineage last chose to continue.
        self.chain_success = obs.success or 0
        self.state = "recovering"
    end
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
    return { action = "ran", state = self.state, milestone_index = self.index,
             failed = obs.failed or 0, lost = obs.lost or 0,
             walks_failed = walks_failed,
             walks_lost = walks_lost,
             first_error = obs.first_error,
             iteration = self.iterations,
             done = obs.done, pending = obs.pending, running = obs.running,
             success = obs.success,
             -- Set when this run is about to be continued rather than
             -- replanned, so a driver can say which of two consecutive "ran"
             -- lines belong to one plan. The plan itself arrives on the next
             -- transition.
             recovering = recovery ~= nil and recovery.why or nil,
             -- Set when a continuation was possible and refused, naming the
             -- rule -- so a driver can print `-- not recovered: divergence`
             -- beside the replan instead of leaving the reader to infer the
             -- refusal from a `planned` line that carries no `recovery`.
             not_recovered = not_recovered,
             steps = steps, actions = obs.actions, walks = walks }
end

-- ---------------------------------------------------------------------------
-- A fault does not have to cost the world it happened in
-- ---------------------------------------------------------------------------
--
-- `Sup:step()` re-raises anything `refusal_of` will not vouch for -- a verdict
-- is a statement about the world and closes a milestone quietly, a fault is a
-- defect and must end the run loudly. Ending it loudly used to mean the script
-- returning, and **when the script returns the server dies**, taking the built
-- world with it. Run `run-1788895333-40607` settled 2,501 actions over sixteen
-- minutes and then exited on `bot 1 has 0 coal, needs 2`, with nobody left able
-- to ask which bot, which chest, or what the ground held.
--
-- This is the hook for that. Call it from a driver's fault branch, where the
-- raise has already unwound: `crates/executor` spawns no detached tasks, so by
-- the time a `pcall` has caught something there is no action in flight and no
-- wall-clock deadline still running against the clock we are about to stop.
-- Holding from inside a batch would freeze the game underneath deadlines that
-- keep running, and the state would be corrupted when it thawed.
--
-- Three things happen, in this order, and the order is the design:
--
--  1. **A savepoint, before the pause.** The engine writes a save at the end of
--     a tick and a paused game never ends one, so this cannot be done later.
--     With it, losing the held game costs nothing -- `--resume-from <run>:<n>`
--     brings the world back at leisure -- which is what makes a *short* hold
--     acceptable. Tonight's fault was sixteen minutes past the only savepoint
--     the run had.
--  2. **The hold**, bounded (`rcon.hold`, five minutes by default).
--  3. **A record entry**, whatever the outcome. A hold that lapsed unnoticed
--     must not read like a run that finished; `silence is not success`.
--
-- Returns a table, always, never `nil`:
--   `released`  "continue" | "stop" | "timeout" | "unavailable"
--   `savepoint` the path inside the run directory, or nil
--   `index`     the savepoint's milestone index, for `--resume-from`, or nil
--   `held`      true when the game was actually paused
--
-- `"unavailable"` is its own answer and is never "stop": there is no game to
-- pause (an offline plan), or this binary predates `rcon.hold`. Collapsing it
-- into a verdict would make "we could not hold" indistinguishable from "we held
-- and nobody came" -- absent is not a value.
function supervisor.hold_fault(err, opts)
    opts = opts or {}
    if type(opts) ~= "table" then
        error("supervisor.hold_fault: expected a table of options")
    end
    local out = { released = "unavailable", savepoint = nil, index = nil, held = false }

    -- The savepoint first, and never fatal: it is what makes losing the hold
    -- cheap, not a precondition of holding. `pcall` because a driver in a
    -- fault branch must not fault again.
    if type(record) == "table" and type(record.savepoint) == "function" then
        local ok_sp, file, index = pcall(record.savepoint, opts.index or 0)
        if ok_sp and type(file) == "string" then
            out.savepoint, out.index = file, index
        elseif not ok_sp then
            print_warn("hold: no savepoint at the fault: " .. tostring(file))
        end
    end

    if type(rcon) ~= "table" or type(rcon.hold) ~= "function" then
        print_warn("hold: this run has no game to pause (rcon.hold is absent) -- "
            .. "the fault ends the run as it always did")
        return out
    end

    local ok_hold, held = pcall(rcon.hold, {
        error = tostring(err),
        run = opts.run,
        timeout = opts.timeout,
        heartbeat = opts.heartbeat,
    })
    if not ok_hold or type(held) ~= "table" then
        print_warn("hold: the game could not be held: " .. tostring(held))
        return out
    end
    out.held = true
    out.released = held.released

    -- Loud on the way out, exactly as loud as on the way in. A lapsed hold
    -- reads as "nobody attached", never as a finished run.
    local how = (held.released == "timeout")
        and "nobody attached; the hold lapsed and tore itself down"
        or ("released with '" .. tostring(held.released) .. "'")
    local why = "held at tick " .. tostring(held.paused_at_tick) .. " for "
        .. tostring(held.held_seconds) .. "s -- " .. how
        .. (out.savepoint and (" -- savepoint " .. out.savepoint) or " -- NO savepoint")
        .. " -- fault: " .. tostring(err)
    print("HELD: " .. why)
    if out.index ~= nil then
        print("   resume this world later:  --resume-from <run-id>:" .. tostring(out.index))
    end
    if type(record) == "table" and type(record.milestone_stuck) == "function" then
        pcall(record.milestone_stuck, opts.index or 0, "held", why, nil)
    end
    return out
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
