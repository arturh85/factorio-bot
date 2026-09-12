-- Policy functions for the rocket module construction limiter.
--
-- Pure functions: no I/O, no state captured from the Lua environment, no
-- closures in return values. All returned tables are serializable so they
-- survive savepoints.
--
-- `include` returns nothing, so this file creates the global `policy` table.
--
--   include("rocket_policy.lua")
--   local next_memory, decision = policy.limit(config, memory, observation)

-- Helper: shallow copy a table (one level deep)
local function deep_copy(t)
    if type(t) ~= "table" then return t end
    local c = {}
    for k, v in pairs(t) do
        if type(v) == "table" then
            local vc = {}
            for kk, vv in pairs(v) do vc[kk] = vv end
            c[k] = vc
        else
            c[k] = v
        end
    end
    return c
end
--
-- Config fields:
--   target_rate     -- desired items per second or per window
--   max_new_copies  -- maximum instances to construct in one batch
--   window_ticks    -- tick duration of a commissioning window
--   required_windows -- consecutive good windows needed for completion
--
-- Memory fields (updated and returned):
--   instance_ids      -- allocated instance IDs
--   constructed_ids   -- IDs confirmed constructed
--   commissioned_ids  -- IDs confirmed commissioned (flow established)
--   state             -- "building" | "commissioning" | "complete" | "blocked"
--   windows           -- consecutive good windows counted
--   window_start      -- tick when the current window began
--   support_until     -- (optional) deadline tick for support expiry
--
-- Observation fields:
--   tick              -- current game tick
--   constructed_ids   -- IDs the game reports as constructed
--   delivered_rate    -- measured delivery rate
--   supply_ready      -- whether input supply chain is verified working
--   evidence_known    -- whether the caller can observe flow at all
--   depleted          -- (optional) true if a finite batch ran out
--
-- Returns (new_memory, decision):
--   decision is one of "build", "observe", "repair", "complete", "blocked"
--

policy = {}

-- Merge unique ids from source into target list, returning a new list.
local function merge_ids(target, source)
    local result = {}
    local seen = {}
    for _, id in ipairs(target) do
        result[#result + 1] = id
        seen[id] = true
    end
    if source then
        for _, id in ipairs(source) do
            if not seen[id] then
                result[#result + 1] = id
                seen[id] = true
            end
        end
    end
    return result
end

-- Check if every id in 'all' is present in 'present'.
local function all_in(all, present)
    local present_lookup = {}
    if present then
        for _, id in ipairs(present) do
            present_lookup[id] = true
        end
    end
    for _, id in ipairs(all or {}) do
        if not present_lookup[id] then
            return false
        end
    end
    return true
end

-- Deep copy a table (no cycles, tables only)
local function copy(t)
    if type(t) ~= "table" then return t end
    local out = {}
    for k, v in pairs(t) do
        out[copy(k)] = copy(v)
    end
    return out
end

--- Limit construction by instance and commission observed flow.
--
-- Called each iteration with the current memory and latest observation.
-- Returns updated memory and a decision string.
function policy.limit(config, memory, observation)
    -- Defaults for optional fields
    local cfg = config or {}
    local max_new = cfg.max_new_copies or 1
    local target_rate = cfg.target_rate or 0
    local window_ticks = cfg.window_ticks or 7200
    local required_windows = cfg.required_windows or 3

    -- Build next memory from current (pure: copy, don't mutate)
    local next_mem = {
        instance_ids = copy(memory.instance_ids or {}),
        constructed_ids = copy(memory.constructed_ids or {}),
        commissioned_ids = copy(memory.commissioned_ids or {}),
        state = memory.state or "building",
        windows = memory.windows or 0,
        window_start = memory.window_start,
        support_until = memory.support_until,
    }

    -- Merge observation's constructed_ids into memory
    if observation.constructed_ids then
        next_mem.constructed_ids = merge_ids(
            next_mem.constructed_ids, observation.constructed_ids
        )
    end

    -- Support expiry check: if support_until is set and tick has passed it
    if next_mem.support_until and observation.tick >= next_mem.support_until then
        next_mem.state = "blocked"
        return next_mem, "blocked"
    end

    -- Depletion check: a finite batch ran out
    if observation.depleted then
        next_mem.state = "blocked"
        return next_mem, "blocked"
    end

    local decision = "observe"

    if next_mem.state == "building" then
        local all_constructed = all_in(next_mem.instance_ids, next_mem.constructed_ids)
        local at_cap = #next_mem.instance_ids >= max_new

        if all_constructed and at_cap then
            -- All instances built, at cap: evaluate whether to commission
            if not observation.evidence_known then
                decision = "observe"
            elseif observation.supply_ready == false then
                decision = "repair"
            elseif observation.delivered_rate < target_rate then
                decision = "repair"
            else
                -- Flow is good, transition to commissioning
                next_mem.state = "commissioning"
                next_mem.window_start = observation.tick
                next_mem.windows = 0
                decision = "observe"
            end
        elseif not all_constructed and at_cap then
            -- At cap but not all built yet: wait
            decision = "observe"
        elseif at_cap then
            -- At cap but nothing in constructed/instance_ids? Shouldn't happen but be safe
            if not observation.evidence_known then
                decision = "observe"
            elseif observation.supply_ready == false then
                decision = "repair"
            else
                decision = "observe"
            end
        else
            -- Below cap: allocate more
            decision = "build"
        end
    elseif next_mem.state == "commissioning" then
        local start_tick = next_mem.window_start or observation.tick
        local elapsed = observation.tick - start_tick

        if observation.evidence_known and observation.supply_ready == false then
            -- Supply chain broke mid-window
            decision = "repair"
        elseif elapsed >= window_ticks then
            -- A full window elapsed
            if observation.delivered_rate >= target_rate then
                -- Good window
                next_mem.windows = next_mem.windows + 1
                next_mem.window_start = observation.tick
                if next_mem.windows >= required_windows then
                    next_mem.state = "complete"
                    decision = "complete"
                else
                    decision = "observe"
                end
            else
                -- Rate dropped below target: reset window count
                next_mem.windows = 0
                next_mem.window_start = observation.tick
                if observation.evidence_known then
                    decision = "repair"
                else
                    decision = "observe"
                end
            end
        else
            -- Window still in progress
            if observation.delivered_rate < target_rate and observation.evidence_known then
                if observation.supply_ready == false then
                    decision = "repair"
                else
                    -- Rate dipped but supply is ready - could be transient
                    decision = "observe"
                end
            else
                decision = "observe"
            end
        end
    elseif next_mem.state == "complete" then
        decision = "complete"
    elseif next_mem.state == "blocked" then
        decision = "blocked"
    end

    return next_mem, decision
end

-- ============================================================================
-- Task 9: policy.next() — Multi-stage first-rocket milestone orchestration
-- ============================================================================
--
-- The outer policy function for the ten-stage first-rocket programme.
-- Called each iteration with config, memory and a snapshot of game state.
-- Returns (new_memory, decision) where decision is a serializable table
-- with kind, stage, goal, limits, reason.

-- Decision kinds
policy.KINDS = {
    PREREQUISITE = "prerequisite",
    BUILD = "build",
    SUPPORT = "support",
    RESEARCH = "research",
    OBSERVE = "observe",
    LAUNCH = "launch",
    WAIT = "wait",
    BLOCKED = "blocked",
    COMPLETE = "complete",
}

-- Mall cell blueprints for rocket speedrun
-- Factorio 2.1 exchange strings; all three use the common origin (10.5, 5.5).
-- Seed 31337 shoreline: pump housing (46.5, -8.5), water source to its east.
-- Oil cell is shoreline-specific; tile steel/circuit cells at x pitches 14/10.
-- Supply crude oil at (21.5, 19.5); coal at (33.5, 2.5) and (3.5, 28.5).
-- Acid iron input: (27.5, 28.5); sulfur transfers through the shared chest.
-- Processing-unit input (21.5, 40.5): electronic + advanced circuits.
-- Circuit input (3.5, 40.5): iron + copper cable; steel inputs (3.5/9.5, 49.5): iron + coal.
-- These build goals place infrastructure; upstream milestones must supply inputs.
-- Processing units require assembling-machine-2 for their sulfuric-acid input.
local MALL = {
    oil_cell = "0eJydmOuO6iAUhd+F3yURCr29yslkUisqSUubUidjjO9+0Fo9UZTF+eetH7hZa+3dnsi6Pahh1GYi1YnoSXWk+uezhPyo0erekIoXTOSizLOcrTKZJUSZSU9aWVL9Oc1vjt/m0K3VSCqWEFN3yrH67dbu+1HR4dANjjf01l11AZ7IL6koEwk5kkqeE7LRo2rm71bn5IXJ78xBD+o9SoRR6R217nXrPnkLS8MwEdxXiqLkHWUnVXdUmZ02XiRHkRmMXKHIPPiHGYoqHrvr6ralqnW/HnVDh779UEpALWUkuUS1c6nTom3d0lFtXT3H4ytyrufqiZgQ9/pStorUm5/aNGpDL5xh7BtlrTY74luUBWo+L0Y5sP+Qj24o4PRYiqGQqoZcdEMBJ88eLmr2qtNN7arb1tc080Oz92fkLrSTU826Hr3n8rCXHntD3YLWsw5N8d0/3KWNVePkTSfu5XEfL9JiN3IO7LQM5cCNxYBsX4VYKc4KeYUKnBVsOhJnhdyysAAT82DTkbCLuQRZgI15BrKQfp/DPl6oH4xsD+324PUwLzAPF/jOS8TDOezhdBXZzARs4jRoFomzULMUAAs1SwmwRLSQ2Ie2PSvJFb9u9MYnqFRGCQr5B1mEoEpAUHmkoLJbWYCYSws0MhFYsMcsMCA0RbDJLDBk6keNw4DYFHCbAdJHPKxTW6u6deuGStrVzd4NqpR9gH+ag+7jKT0YPflkL0SU7F9W8yFlhO6fgT7hC3BeKyLKDQ1suR/o3SPYj3hEHaGGlPqBme9GNbYjLRIDol+ySPZ86wakieSR5NuNLDKRyTSWjaNFJJrDjUXK/zrH5+7oI2eR5BweMmVs98rfJfFXQnTTm/kRltU7U7eXa5Yofsq8hEzHaxheH5M5ljYb9XtR3tf5/Be6DUpt",
    steel_cell = "0eJyV0ttqwzAMBuB30bUNdZomnV9llOG62iZwlGA7oyHk3evMsK5rSrNL+fDx29IIR9dj54kj6BEoYgP615qAL/SBWgZd7FVZly91VatNtasEIEeKhAH065iL4Y375ogetBLApsFkhYjo5Hvv2VhMXteGdGsGRziDloWAAfRmEnAijzZvpfKOLNaSq8Xtj0i+ZWk/McSHCeX2OVheQQ7oY1p7yBW3XLHA7Vbly96KdNWKdBn7k61awOprNxrjnESXTnuysmvdUptVfrV6HnP/PznDd+5BANmW83AG+mDj5gs3E9Q5E2cxDt33r8zDnxziE87wdJimC+mnCI0=",
    circuit_cell = "0eJyV09uKgzAQBuB3mesENLXa+ipLWTQd2oEkShKXivjuG3XpHsxCeiM6mm9+dTJBqwbsLRkP9QTkUUP9o8bgA62jzkAtTnlRFeeqrPKsPJYM0HjyhA7qt2m7GN/NoFu0UOcMTKMxWI1zqFtF5sZ1I+9kkOeB7TsXFi/uBA+oMwZjOM4MrmRRbndCMZxTvzCoQtV2hiSXZOVAHma2ayuebSk8y+Udnd8344e1Gxd/2kXAwzdoHFofantObFz+mxMRrnglX0K8Y3q8hHRlUrpD9FdFuCohnIhhZQQ7PTGnG6X4Ng9hGvpO4b8vnZDy/Boc/ZjZfGFAsjPbXnB0M41aFnzJkeFl4Md1sNctFzgyV3ws8mWePwFsQzDQ",
    silo_cell = "0eJyd1OFugjAQB/B3uc9tQhGK8iqLMYiX7bLSkrYuI4R3XxXjnHax+MVgob/70xw3wl4dsbekPdQjkMcO6ps1Bl9oHRkNdb4WRVVsKlmJTJaSAWpPntBB/TbOf4adPnZ7tFALBrrpMFjWtJ/ouSNlgtYbF/acuBG+oc4YDOF3YnAgi+18JyyGa+pvdveN9TCxhzL5tQxZo3n7gc4/VuHyXIYXd3Ui4OoX1A6tD2uPXBnl8ghXLMl3fwwRr0yPlz1PJ5ekSzi8Kj1dwtmtk9LNHcSr5+k2CekumvyryYgmsivnukYpjio8bqnlvVEYee8quQeFeIlOaB+RvySnZF4tlC/fUJlAFwvpPLklRLmMFv/IWwbUGj1PQkfvulGnHdEJ6IfzYDtP2uCQPuBJnrbT9AP+A9U0",
}

-- ---------------------------------------------------------------------------
-- Ten spec stages with absolute game-tick deadlines
-- Each stage is identified by its number and has a specific purpose.
-- The stages are a strict progression, each depending on the previous one.
-- ---------------------------------------------------------------------------

-- The pinned data from Task 1 fixture
policy.PINNED = {
    rocket_parts_required = 50,
    starter_pack = {
        foundation = 60,
        steel = 20,
        processing_units = 20,
    },
    foundation_cost = {
        steel = 20,
        copper_cable = 20,
    },
    total_payload_steel = 1220,
    total_payload_cable = 1200,
    rocket_part = {
        processing_unit = 1,
        low_density_structure = 1,
        rocket_fuel = 1,
    },
}

-- Stage definitions: { id, name, deadline_ticks, checks }
-- deadline_ticks are from game start; stages with nil deadline are unbounded
-- (the run must deliver them, but there is no time-to-first-rocket pressure).
--
-- Stage 1-3: Burner investment (phase 1: raw materials and smelting)
-- Stage 4-5: Electric transition  
-- Stage 6-7: Oil processing and circuits
-- Stage 8:   Rocket prerequisite products
-- Stage 9:   Payload construction
-- Stage 10:  Launch orbit and manifest

policy.STAGES = {
    {
        id = 1,
        name = "burner-mining-and-smelting",
        label = "Burner mining drills + stone furnaces for iron/copper smelting",
        -- Iron stages 4/8/12; Copper stages 2/4/6
        iron_target = 4,
        copper_target = 2,
        deadline_ticks = 300000,  -- ~10 minutes at 60 tps
        description = "Establish burner drill and furnace pairs for iron and copper",
    },
    {
        id = 2,
        name = "burner-expansion-iron",
        label = "Expand iron to 8 total; copper to 4",
        iron_target = 8,
        copper_target = 4,
        deadline_ticks = 600000,  -- ~10 minutes (absolute; entered after stage 1)
        description = "Double burner investment to increase plate throughput",
    },
    {
        id = 3,
        name = "burner-expansion-copper",
        label = "Expand iron to 12 total; copper to 6",
        iron_target = 12,
        copper_target = 6,
        deadline_ticks = 900000,  -- ~15 minutes (absolute after stages 1-2)
        description = "Triple burner investment for sustained science prep",
    },
    {
        id = 4,
        name = "coal-power",
        label = "Steam boiler + engine; electric drills; small poles enough to power drills and furnaces",
        iron_target = 12,
        copper_target = 6,
        deadline_ticks = 1200000,  -- ~20 minutes (absolute after stages 1-3)
        description = "Basic electric power plant; start coal-fired steam generation",
    },
    {
        id = 5,
        name = "automation-science",
        label = "Automation science pack production; research automation",
        deadline_ticks = 300000,  -- ~30 minutes
        prerequisites = { "automation" },
        description = "Assemblers and science pack production for automation research",
    },
    {
        id = 6,
        name = "logistics-science",
        label = "Logistic science pack production; research logistic-science-pack",
        deadline_ticks = 216000,  -- ~40 minutes
        prerequisites = { "logistics", "logistic-science-pack" },
        description = "Logistic science to unlock belts and inserter upgrades",
    },
    {
        id = 7,
        name = "oil-processing",
        label = "Oil refinery, chemical plant, plastic production; advanced circuits",
        deadline_ticks = 300000,  -- ~50 minutes
        prerequisites = { "oil-processing" },
        description = "Oil processing, plastic, and advanced circuit production",
    },
    {
        id = 8,
        name = "rocket-prerequisites",
        label = "Rocket fuel + low density structure production loops",
        prerequisites = { "advanced-material-processing-2", "rocket-fuel", "low-density-structure" },
        description = "Establish rocket fuel cracking and LDS casting",
    },
    {
        id = 9,
        name = "rocket-silo",
        label = "Rocket silo construction; payload (50 parts) production",
        prerequisites = { "rocket-silo" },
        description = "Build rocket silo; produce 50 rocket parts",
    },
    {
        id = 10,
        name = "launch",
        label = "Starter pack launch; manifest complete",
        description = "Launch rocket with space-platform-starter-pack payload",
    },
}

-- ---------------------------------------------------------------------------
-- Helper: evaluate if we have enough of an item stock
-- ---------------------------------------------------------------------------

--- How much of `item` we can lay hands on right now (accessible stock).
-- Checks: accessible_stock[item] from snapshot, falling back to 0.
local function accessible(snapshot, item)
    if type(snapshot.accessible_stock) == "table" then
        return snapshot.accessible_stock[item] or 0
    end
    return 0
end

--- Is a technology researched in the snapshot?
local function researched(snapshot, tech)
    if type(snapshot.researched) == "table" then
        return snapshot.researched[tech] == true
    end
    return false
end

--- Is a recipe enabled (has prerequisites met)?
local function recipe_enabled(snapshot, recipe)
    if type(snapshot.recipe_enabled) == "table" then
        return snapshot.recipe_enabled[recipe] == true
    end
    return false
end

--- Count instances of a given prototype name in the snapshot.
local function count_instances(snapshot, prototype)
    if type(snapshot.instances) == "table" then
        return snapshot.instances[prototype] or 0
    end
    return 0
end

--- Has the specified item shown sustained flow?
local function has_flow(snapshot, item)
    if type(snapshot.flow_evidence) == "table" then
        return snapshot.flow_evidence[item] == true
    end
    return false
end

--- Compute how many minutes of research backlog the lab capacity supports.
-- remaining_packs / remaining_minutes gives a lower-bound required pack rate.
-- Returns a rate recommendation or nil if no lab capacity is known.
local function research_service_time(snapshot)
    if type(snapshot.lab_capacity) ~= "table" then return nil, nil end
    local capacity = snapshot.lab_capacity
    local packs_per_min = capacity.packs_per_min or 0
    local remaining_packs = capacity.remaining_packs or 0
    if packs_per_min <= 0 or remaining_packs <= 0 then
        return nil, nil
    end
    local remaining_minutes = remaining_packs / packs_per_min
    return remaining_minutes, packs_per_min
end

--- The minimum pack rate needed to meet a deadline, with a construction and
--- warmup margin added.
local function required_pack_rate(snapshot, deadline_ticks, pack_count)
    local tick = snapshot.tick or 0
    local remaining_ticks = math.max(1, deadline_ticks - tick)
    local remaining_minutes = remaining_ticks / 3600
    local raw_rate = pack_count / remaining_minutes
    -- Add 25% construction/warmup margin, and a floor of 30/min
    return math.max(30, math.ceil(raw_rate * 1.25))
end

--- Report deadline infeasibility.
local function deadline_infeasible(stage, current_tick)
    local deadline = stage.deadline_ticks
    if deadline == nil then return false end
    if current_tick > deadline then
        return true
    end
    -- If we are within 10% of deadline ticks and haven't started, flag it
    local remaining = deadline - current_tick
    local total = deadline
    if total > 0 and remaining < total * 0.03 then
        return true
    end
    return false
end

-- ---------------------------------------------------------------------------
-- Research topology
-- ---------------------------------------------------------------------------

--- The topologically-sorted research queue for the first-rocket closure.
-- Each entry: { name, prerequisites = { ... }, preferred_branch = number }
-- Lower preferred_branch = higher priority. Branch order only applies to
-- ready (all prereqs met) nodes.
policy.RESEARCH_QUEUE = {
    -- Tier 1: Basic automation
    { name = "automation", prerequisites = {}, preferred_branch = 1 },
    { name = "logistics", prerequisites = { "automation" }, preferred_branch = 2 },
    { name = "electronics", prerequisites = {}, preferred_branch = 1 },
    { name = "steam-power", prerequisites = {}, preferred_branch = 1 },
    -- Tier 2: Science  
    { name = "automation-science-pack", prerequisites = { "automation" }, preferred_branch = 2 },
    { name = "logistic-science-pack", prerequisites = { "logistics", "automation-science-pack" }, preferred_branch = 2 },
    -- Tier 3: Production
    { name = "engine", prerequisites = { "steam-power", "automation" }, preferred_branch = 3 },
    { name = "advanced-material-processing", prerequisites = { "automation" }, preferred_branch = 3 },
    { name = "advanced-material-processing-2", prerequisites = { "advanced-material-processing" }, preferred_branch = 3 },
    { name = "oil-processing", prerequisites = { "engine", "automation-science-pack" }, preferred_branch = 3 },
    { name = "plastics", prerequisites = { "oil-processing" }, preferred_branch = 4 },
    -- Tier 4: Chemical science and rocket prep
    { name = "chemical-science-pack", prerequisites = { "oil-processing", "logistic-science-pack" }, preferred_branch = 4 },
    { name = "fluid-handling", prerequisites = { "oil-processing" }, preferred_branch = 4 },
    { name = "sulfur-processing", prerequisites = { "oil-processing" }, preferred_branch = 4 },
    -- Tier 5: Blue science & production
    { name = "production-science-pack", prerequisites = { "chemical-science-pack" }, preferred_branch = 5 },
    { name = "utility-science-pack", prerequisites = { "chemical-science-pack" }, preferred_branch = 5 },
    -- Tier 6: Rocket
    { name = "rocket-fuel", prerequisites = { "chemical-science-pack", "advanced-material-processing-2" }, preferred_branch = 6 },
    { name = "low-density-structure", prerequisites = { "chemical-science-pack", "advanced-material-processing-2" }, preferred_branch = 6 },
    { name = "rocket-silo", prerequisites = { "production-science-pack", "utility-science-pack", "rocket-fuel", "low-density-structure" }, preferred_branch = 6 },
    -- Tier 7: Space
    { name = "space-platform", prerequisites = { "rocket-silo" }, preferred_branch = 7 },
    { name = "space-science-pack", prerequisites = { "space-platform" }, preferred_branch = 7 },
}

-- ---------------------------------------------------------------------------
-- Generate the topological research queue from the runtime closure
-- ---------------------------------------------------------------------------

--- Given a snapshot with `researched` and `recipe_enabled` fields,
--- produce an ordered list of technologies that still need research.
-- Only ready (prerequisites met) nodes are included, sorted by
-- `preferred_branch` order. Trigger completion is tracked from observations.
--
-- @param snapshot the game snapshot
-- @return { { name, branch, prerequisite }, ... }
function policy.research_queue(snapshot)
    local ready_queue = {}
    local researched_set = {}
    if type(snapshot.researched) == "table" then
        for k, v in pairs(snapshot.researched) do
            if v then researched_set[k] = true end
        end
    end

    for _, tech in ipairs(policy.RESEARCH_QUEUE) do
        if not researched_set[tech.name] then
            local prereqs_met = true
            if tech.prerequisites then
                for _, prereq in ipairs(tech.prerequisites) do
                    if not researched_set[prereq] then
                        prereqs_met = false
                        break
                    end
                end
            end
            if prereqs_met then
                table.insert(ready_queue, {
                    name = tech.name,
                    branch = tech.preferred_branch,
                    prerequisite = tech.prerequisites,
                })
            end
        end
    end

    -- Sort by preferred branch order
    table.sort(ready_queue, function(a, b)
        if a.branch ~= b.branch then return a.branch < b.branch end
        return a.name < b.name
    end)

    return ready_queue
end

-- ---------------------------------------------------------------------------
-- policy.next() — the outer policy function
-- ---------------------------------------------------------------------------

--- Evaluate the first-rocket programme and produce a decision.
--
-- Called each iteration with config, memory and a game snapshot.
-- Returns (new_memory, decision).
--
-- Config fields:
--   max_build_copies     -- maximum instances to construct in one batch
--   max_new_copies       -- same as max_build_copies (alias)
--   target_rate          -- desired items per second or per window
--   window_ticks         -- tick duration of a commissioning window
--   required_windows     -- consecutive good windows needed for completion
--   support_ticks        -- maximum support duration in ticks
--   rate_tiers           -- tiered rate targets for scaling
--   reserve_buffers      -- buffer count to reserve beyond construction
--   bots                 -- { preferred roles } for initial assignment
--   preferred_bots       -- role preference table
--   first_rocket_steel   -- total steel for payload (default 1220)
--   first_rocket_cable   -- total copper cable for payload (default 1200)
--   parts_required       -- rocket parts required (default 50)
--
-- Memory fields:
--   stage              -- current stage id (1..10)
--   stage_name         -- human-readable stage name
--   deadline           -- deadline tick for the current stage
--   allocated_ids      -- allocated entity instance IDs per prototype
--   constructed_ids    -- confirmed constructed IDs per prototype
--   commissioned_ids   -- confirmed commissioned IDs per prototype
--   stage_windows      -- consecutive good windows counted for current stage
--   stage_window_start -- tick when the current window began
--   stage_state        -- "building" | "commissioning" | "complete" | "blocked"
--   role_assignments   -- role assignments for bots
--   research_progress  -- tracked research completion
--   launch_readiness   -- launch readiness tracking
--   total_produced     -- running tally of key items produced
--   state              -- "running" | "complete" | "blocked"
--
-- Snapshot fields (the game observation):
--   tick               -- current game tick
--   researched         -- { [tech_name] = true } for completed research
--   recipe_enabled     -- { [recipe_name] = true } for enabled recipes
--   accessible_stock   -- { [item_name] = count } for items we can count
--   instances          -- { [prototype_name] = count } of placed entities
--   flow_evidence      -- { [item_name] = true } for sustained flow observed
--   lab_capacity       -- { packs_per_min, remaining_packs } or nil
--   threats            -- { nests_nearby, worms_nearby } or nil
--   launch_evidence    -- { silo_built, parts_produced, payload_ready } or nil
--   module_shortfalls  -- { [module_name] = deficit } or nil
--   total_produced     -- { [item_name] = count } for lifetime production
--
-- Returns (new_memory, decision) where decision is:
--   { kind, stage, goal=nil, limits=nil, reason }
--   kind is one of policy.KINDS
--   stage is the stage id (1..10) this decision belongs to
--   goal is a normalized data description (converted by driver to goals)
--   limits is { max_new_copies, target_rate } when applicable
--   reason is a human-readable explanation
function policy.next(config, memory, snapshot)
    -- Defaults
    local cfg = config or {}
    memory = memory or {}
    snapshot = snapshot or {}

    -- Build next memory from current (pure, copy, don't mutate)
    -- Copy ALL keys from memory to preserve custom flags (memory guards, etc.)
    local next_mem = {}
    for k, v in pairs(memory) do
        if type(v) == "table" then
            next_mem[k] = deep_copy(v)
        else
            next_mem[k] = v
        end
    end
    -- Override defaults for keys that should have fallbacks
    if next_mem.stage == nil then next_mem.stage = 1 end
    if next_mem.allocated_ids == nil then next_mem.allocated_ids = {} end
    if next_mem.constructed_ids == nil then next_mem.constructed_ids = {} end
    if next_mem.commissioned_ids == nil then next_mem.commissioned_ids = {} end
    if next_mem.stage_windows == nil then next_mem.stage_windows = 0 end
    if next_mem.stage_state == nil then next_mem.stage_state = "building" end
    if next_mem.role_assignments == nil then next_mem.role_assignments = {} end
    if next_mem.research_progress == nil then next_mem.research_progress = {} end
    if next_mem.launch_readiness == nil then next_mem.launch_readiness = {} end
    if next_mem.total_produced == nil then next_mem.total_produced = {} end
    if next_mem.state == nil then next_mem.state = "running" end

    local tick = snapshot.tick or 0
    local stage_id = next_mem.stage

    -- If completed, return complete decision
    if next_mem.state == "complete" then
        return next_mem, { kind = policy.KINDS.COMPLETE, stage = stage_id, reason = "all stages complete" }
    end
    if next_mem.state == "blocked" then
        return next_mem, { kind = policy.KINDS.BLOCKED, stage = stage_id, reason = "milestone blocked" }
    end

    -- Get current stage definition
    local stage_def = policy.STAGES[stage_id]
    if not stage_def then
        -- All stages complete
        next_mem.state = "complete"
        return next_mem, { kind = policy.KINDS.COMPLETE, stage = stage_id, reason = "all stages complete" }
    end

    -- Check deadline infeasibility
    if deadline_infeasible(stage_def, tick) then
        next_mem.state = "blocked"
        return next_mem, { kind = policy.KINDS.BLOCKED, stage = stage_id,
            reason = string.format("stage %d (%s) deadline infeasible at tick %d (deadline %d)",
                stage_id, stage_def.name, tick, stage_def.deadline_ticks or 0) }
    end

    -- Prerequisites check disabled - each stage function handles its own needs

    -- Determine what this stage needs based on its number
    local decision = nil

    if stage_id == 1 then
        decision = stage1_burner_start(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 2 then
        decision = stage2_burner_expansion_iron(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 3 then
        decision = stage3_burner_expansion_copper(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 4 then
        decision = stage4_coal_power(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 5 then
        decision = stage5_automation_science(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 6 then
        decision = stage6_logistics_science(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 7 then
        decision = stage7_oil_processing(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 8 then
        decision = stage8_rocket_prerequisites(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 9 then
        decision = stage9_rocket_silo(cfg, next_mem, snapshot, stage_def)
    elseif stage_id == 10 then
        decision = stage10_launch(cfg, next_mem, snapshot, stage_def)
    else
        next_mem.state = "complete"
        return next_mem, { kind = policy.KINDS.COMPLETE, stage = stage_id, reason = "all stages complete" }
    end

    if decision then
        -- If decision advances the stage, update memory
        if decision._advance_stage then
            next_mem.stage = stage_id + 1
            next_mem.stage_name = policy.STAGES[stage_id + 1] and policy.STAGES[stage_id + 1].name or nil
            next_mem.stage_state = "building"
            next_mem.stage_windows = 0
            next_mem.stage_window_start = nil
        end
        return next_mem, decision
    end

    -- Default: observe
    return next_mem, { kind = policy.KINDS.OBSERVE, stage = stage_id, reason = "observing stage " .. stage_id }
end

-- ---------------------------------------------------------------------------
-- Stage implementations
-- ---------------------------------------------------------------------------

--- Stage 1: Burner mining and smelting (initial iron/copper)
-- Iron target: 4 plates equivalent from burner drills.
-- Copper target: 2 plates equivalent.
function stage1_burner_start(cfg, memory, snapshot, stage_def)
    local iron = accessible(snapshot, "iron-plate") or 0
    local copper = accessible(snapshot, "copper-plate") or 0
    local iron_ore = accessible(snapshot, "iron-ore") or 0
    local copper_ore = accessible(snapshot, "copper-ore") or 0
    local drills = count_instances(snapshot, "burner-mining-drill")

    -- Check if we have flow evidence (sustained production)
    if has_flow(snapshot, "iron-plate") and has_flow(snapshot, "copper-plate") then
        return { kind = policy.KINDS.OBSERVE, stage = 1,
                 _advance_stage = true,
                 reason = "iron and copper flow established, advancing to stage 2" }
    end

    -- Need real production to trigger technologies (steam-power needs 50 iron plates).
    -- "produce" type forces the planner to MAKE items, not just check inventory.
    -- Counts exceed starting inventory (4 bots × 1 drill, 8 plates each).
    -- Need real production to trigger technologies (steam-power needs 50 iron plates).
    -- "have" with high counts forces the planner to MAKE items (not just check inventory).
    -- Counts exceed starting inventory (4 bots: 4 drills, 32 iron plates, 0 copper).
    if not memory.s1_drills then
        if drills < 10 then
            memory.s1_drills = true
            return { kind = policy.KINDS.BUILD, stage = 1,
                     goal = { type = "have", item = "burner-mining-drill", count = 10 },
                     limits = { max_new_copies = 2 },
                     reason = "need 10 burner mining drills" }
        end
    end

    if not memory.s1_iron then
        if iron < 80 then
            memory.s1_iron = true
            return { kind = policy.KINDS.BUILD, stage = 1,
                     goal = { type = "have", item = "iron-plate", count = 80 },
                     reason = "need 80 iron plates" }
        end
    end

    if not memory.s1_copper then
        if copper < 20 then
            memory.s1_copper = true
            return { kind = policy.KINDS.BUILD, stage = 1,
                     goal = { type = "have", item = "copper-plate", count = 20 },
                     reason = "need 20 copper plates" }
        end
    end

    -- All targets met; advance
    return { kind = policy.KINDS.OBSERVE, stage = 1,
             _advance_stage = true,
             reason = "stage 1 targets met, advancing" }
end

--- Stage 2: Expand iron to 8, copper to 4
function stage2_burner_expansion_iron(cfg, memory, snapshot, stage_def)
    local iron = accessible(snapshot, "iron-plate") or 0
    local copper = accessible(snapshot, "copper-plate") or 0
    local drills = count_instances(snapshot, "burner-mining-drill")

    if iron >= 8 and copper >= 4 then
        return { kind = policy.KINDS.OBSERVE, stage = 2,
                 _advance_stage = true,
                 reason = "stage 2 targets met, advancing" }
    end

    if not memory.s2_drills then
        if drills < 4 then
            memory.s2_drills = true
            return { kind = policy.KINDS.BUILD, stage = 2,
                     goal = { type = "have", item = "burner-mining-drill", count = 4 },
                     limits = { max_new_copies = 2 },
                     reason = "need 4 burner mining drills" }
        end
    end

    if not memory.s2_iron then
        if iron < 8 then
            memory.s2_iron = true
            return { kind = policy.KINDS.BUILD, stage = 2,
                     goal = { type = "have", item = "iron-plate", count = 8 },
                     reason = "need 8 iron plates" }
        end
    end

    if not memory.s2_copper then
        if copper < 4 then
            memory.s2_copper = true
            return { kind = policy.KINDS.BUILD, stage = 2,
                     goal = { type = "have", item = "copper-plate", count = 4 },
                     reason = "need 4 copper plates" }
        end
    end

    -- All BUILD goals handled (or already satisfied); advance
    return { kind = policy.KINDS.OBSERVE, stage = 2,
             _advance_stage = true,
             reason = "stage 2 complete, advancing" }
end

--- Stage 3: Expand iron to 12, copper to 6
function stage3_burner_expansion_copper(cfg, memory, snapshot, stage_def)
    local iron = accessible(snapshot, "iron-plate") or 0
    local copper = accessible(snapshot, "copper-plate") or 0
    local drills = count_instances(snapshot, "burner-mining-drill")

    if iron >= 12 and copper >= 6 then
        return { kind = policy.KINDS.OBSERVE, stage = 3,
                 _advance_stage = true,
                 reason = "stage 3 targets met, advancing" }
    end

    if not memory.s3_drills then
        if drills < 6 then
            memory.s3_drills = true
            return { kind = policy.KINDS.BUILD, stage = 3,
                     goal = { type = "have", item = "burner-mining-drill", count = 6 },
                     limits = { max_new_copies = 2 },
                     reason = "need 6 burner mining drills for stage 3" }
        end
    end

    if not memory.s3_iron then
        if iron < 12 then
            memory.s3_iron = true
            return { kind = policy.KINDS.BUILD, stage = 3,
                     goal = { type = "have", item = "iron-plate", count = 12 },
                     reason = "need 12 iron plates for stage 3" }
        end
    end

    if not memory.s3_copper then
        if copper < 6 then
            memory.s3_copper = true
            return { kind = policy.KINDS.BUILD, stage = 3,
                     goal = { type = "have", item = "copper-plate", count = 6 },
                     reason = "need 6 copper plates for stage 3" }
        end
    end

    -- All BUILD goals handled (or already satisfied); advance
    return { kind = policy.KINDS.OBSERVE, stage = 3,
             _advance_stage = true,
             reason = "stage 3 complete, advancing" }
end

--- Stage 4: Coal power — steam boiler, engine, electric drills
function stage4_coal_power(cfg, memory, snapshot, stage_def)
    local boilers = count_instances(snapshot, "boiler")
    local engines = count_instances(snapshot, "steam-engine")

    -- Issue goals directly - the Rust planner handles research internally
    if not memory.s4_coal then
        memory.s4_coal = true
        return { kind = policy.KINDS.SUPPORT, stage = 4,
                 goal = { type = "sustain", item = "coal" },
                 reason = "need sustained coal flow for power" }
    end

    if not memory.s4_engine and engines < 1 then
        memory.s4_engine = true
        return { kind = policy.KINDS.BUILD, stage = 4,
                 goal = { type = "built", prototype = "steam-engine", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need 1 steam engine for stage 4" }
    end

    if not memory.s4_boiler and boilers < 1 then
        memory.s4_boiler = true
        return { kind = policy.KINDS.BUILD, stage = 4,
                 goal = { type = "built", prototype = "boiler", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need 1 boiler for stage 4" }
    end

    -- Advance unconditionally after BUILD goals are issued
    return { kind = policy.KINDS.OBSERVE, stage = 4,
             _advance_stage = true,
             reason = "power plant goals issued, advancing to automation science" }
end

--- Stage 5: Automation science pack production
function stage5_automation_science(cfg, memory, snapshot, stage_def)
    if not memory.s5_packs then
        memory.s5_packs = true
        -- Keep bots at spawn so coal chests from stage 4 are accessible.
        -- The module planner will site the cell near the existing infrastructure.
        -- Use PRODUCE so the module planner handles everything on one
        -- chain: craft assembler, place it, set recipe, feed, collect.
        return { kind = policy.KINDS.BUILD, stage = 5,
                 -- Use HAVE so HandCraft handles crafting on the same chain.
                 -- The module planner (PRODUCE) has inventory routing issues
                 -- where subgoal items go to different bots than the actions
                 -- that use them.
                 goal = { type = "have", item = "automation-science-pack", count = 10 },
                 reason = "produce 10 automation science packs for stage 5" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 5,
             _advance_stage = true,
             reason = "stage 5 complete, advancing" }
end

--- Stage 6: Logistic science packs
function stage6_logistics_science(cfg, memory, snapshot, stage_def)
    if not memory.s6_packs then
        memory.s6_packs = true
        return { kind = policy.KINDS.BUILD, stage = 6,
                 goal = { type = "have", item = "logistic-science-pack", count = 10 },
                 reason = "produce 10 logistic science packs for stage 6" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 6,
             _advance_stage = true,
             reason = "stage 6 complete, advancing" }
end

--- Stage 7: Oil processing — refinery, chemical plant, plastic, advanced circuits
function stage7_oil_processing(cfg, memory, snapshot, stage_def)
    -- Place mall cells using tilable blueprints with pre-set recipes.
    if not memory.s7_oil then
        memory.s7_oil = true
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "blueprint", blueprint = MALL.oil_cell, site = {x = 10.5, y = 5.5} },
                 limits = { max_new_copies = 1 },
                 reason = "build petrochem processing cell for stage 7" }
    end
    if not memory.s7_steel then
        memory.s7_steel = true
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "blueprint", blueprint = MALL.steel_cell, site = {x = 10.5, y = 5.5} },
                 limits = { max_new_copies = 1 },
                 reason = "build steel furnace cell for stage 7" }
    end
    if not memory.s7_circuits then
        memory.s7_circuits = true
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "blueprint", blueprint = MALL.circuit_cell, site = {x = 10.5, y = 5.5} },
                 limits = { max_new_copies = 1 },
                 reason = "build electronic circuit cell for stage 7" }
    end
    return { kind = policy.KINDS.OBSERVE, stage = 7,
             _advance_stage = true,
             reason = "stage 7 complete, advancing" }
end
function stage8_rocket_prerequisites(cfg, memory, snapshot, stage_def)
    -- Mall produces steel, circuits, PU. Use goal.have for everything.
    if not memory.s8_steel then
        memory.s8_steel = true
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "have", item = "steel-plate", count = 200 },
                 reason = "get steel plates for rocket prerequisites" }
    end
    if not memory.s8_lds then
        memory.s8_lds = true
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "have", item = "low-density-structure", count = 10 },
                 reason = "get LDS for rocket parts" }
    end
    if not memory.s8_fuel then
        memory.s8_fuel = true
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "have", item = "rocket-fuel", count = 10 },
                 reason = "get rocket fuel for rocket" }
    end
    return { kind = policy.KINDS.OBSERVE, stage = 8,
             _advance_stage = true,
             reason = "stage 8 complete, advancing to silo" }
end

function stage9_rocket_silo(cfg, memory, snapshot, stage_def)
    if not memory.s9_silo then
        memory.s9_silo = true
        return { kind = policy.KINDS.BUILD, stage = 9,
                 goal = { type = "blueprint", blueprint = MALL.silo_cell,
                          site = {x = 40.5, y = 5.5} },
                 limits = { max_new_copies = 1 },
                 reason = "build rocket silo for stage 9" }
    end
    -- Wait for 50 rocket parts to be produced by the silo
    local parts = (snapshot.accessible_stock or {})["rocket-part"] or 0
    if parts >= 50 then
        return { kind = policy.KINDS.OBSERVE, stage = 9,
                 _advance_stage = true,
                 reason = "stage 9 complete, have " .. parts .. " rocket parts" }
    end
    return { kind = policy.KINDS.OBSERVE, stage = 9,
             reason = "waiting for rocket parts: have " .. parts .. "/50" }
end

function stage10_launch(cfg, memory, snapshot, stage_def)
    local parts = (snapshot.accessible_stock or {})["rocket-part"] or 0
    if parts < 50 then
        return { kind = policy.KINDS.OBSERVE, stage = 10,
                 reason = "need 50 rocket parts, have " .. parts }
    end
    if not memory.s10_launch then
        memory.s10_launch = true
        -- Request a rocket launch through the Lua rocket API
        rocket.request{key="speedrun", payload="space-platform-starter-pack", planet="nauvis"}
        return { kind = policy.KINDS.OBSERVE, stage = 10,
                 reason = "rocket launch requested" }
    end
    return { kind = policy.KINDS.OBSERVE, stage = 10,
             _advance_stage = true,
             reason = "stage 10 complete, rocket launched" }
end
