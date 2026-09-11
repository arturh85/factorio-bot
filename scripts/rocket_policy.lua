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
        deadline_ticks = 36000,  -- ~10 minutes at 60 tps
        description = "Establish burner drill and furnace pairs for iron and copper",
    },
    {
        id = 2,
        name = "burner-expansion-iron",
        label = "Expand iron to 8 total; copper to 4",
        iron_target = 8,
        copper_target = 4,
        deadline_ticks = 54000,  -- ~15 minutes
        description = "Double burner investment to increase plate throughput",
    },
    {
        id = 3,
        name = "burner-expansion-copper",
        label = "Expand iron to 12 total; copper to 6",
        iron_target = 12,
        copper_target = 6,
        deadline_ticks = 72000,  -- ~20 minutes
        description = "Triple burner investment for sustained science prep",
    },
    {
        id = 4,
        name = "coal-power",
        label = "Steam boiler + engine; electric drills; small poles enough to power drills and furnaces",
        iron_target = 12,
        copper_target = 6,
        deadline_ticks = 86400,  -- ~24 minutes
        description = "Basic electric power plant; start coal-fired steam generation",
    },
    {
        id = 5,
        name = "automation-science",
        label = "Automation science pack production; research automation",
        deadline_ticks = 108000,  -- ~30 minutes
        prerequisites = { "automation" },
        description = "Assemblers and science pack production for automation research",
    },
    {
        id = 6,
        name = "logistics-science",
        label = "Logistic science pack production; research logistic-science-pack",
        deadline_ticks = 144000,  -- ~40 minutes
        prerequisites = { "logistic-robotics", "logistic-science-pack" },
        description = "Logistic science to unlock belts and inserter upgrades",
    },
    {
        id = 7,
        name = "oil-processing",
        label = "Oil refinery, chemical plant, plastic production; advanced circuits",
        deadline_ticks = 180000,  -- ~50 minutes
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
    if total > 0 and remaining < total * 0.1 then
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
    local next_mem = {
        stage = memory.stage or 1,
        stage_name = memory.stage_name,
        deadline = memory.deadline,
        allocated_ids = memory.allocated_ids or {},
        constructed_ids = memory.constructed_ids or {},
        commissioned_ids = memory.commissioned_ids or {},
        stage_windows = memory.stage_windows or 0,
        stage_window_start = memory.stage_window_start,
        stage_state = memory.stage_state or "building",
        role_assignments = memory.role_assignments or {},
        research_progress = memory.research_progress or {},
        launch_readiness = memory.launch_readiness or {},
        total_produced = memory.total_produced or {},
        state = memory.state or "running",
    }

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

    -- Check stage prerequisites (researched technologies)
    if stage_def.prerequisites then
        for _, prereq in ipairs(stage_def.prerequisites) do
            if not researched(snapshot, prereq) then
                -- Emit research prerequisite
                return next_mem, {
                    kind = policy.KINDS.PREREQUISITE,
                    stage = stage_id,
                    goal = { type = "researched", name = prereq },
                    reason = string.format("stage %d needs research: %s", stage_id, prereq),
                }
            end
        end
    end

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

    -- Need at least some drills
    if drills < 2 then
        return { kind = policy.KINDS.BUILD, stage = 1,
                 goal = { type = "built", prototype = "burner-mining-drill", count = 2 },
                 limits = { max_new_copies = 2 },
                 reason = "need at least 2 burner mining drills for stage 1" }
    end

    -- Need some accessible plates (even hand-smelted)
    if iron < 4 then
        return { kind = policy.KINDS.BUILD, stage = 1,
                 goal = { type = "have", item = "iron-plate", count = 4 },
                 reason = "need 4 iron plates for stage 1" }
    end

    if copper < 2 then
        return { kind = policy.KINDS.BUILD, stage = 1,
                 goal = { type = "have", item = "copper-plate", count = 2 },
                 reason = "need 2 copper plates for stage 1" }
    end

    -- Check flow evidence — if we have the items but no flow, we need
    -- to observe to see if production is self-sustaining
    if not has_flow(snapshot, "iron-plate") or not has_flow(snapshot, "copper-plate") then
        return { kind = policy.KINDS.OBSERVE, stage = 1,
                 reason = "have items but no sustained flow yet; observing" }
    end

    return nil
end

--- Stage 2: Expand iron to 8, copper to 4
function stage2_burner_expansion_iron(cfg, memory, snapshot, stage_def)
    local iron = accessible(snapshot, "iron-plate") or 0
    local copper = accessible(snapshot, "copper-plate") or 0
    local drills = count_instances(snapshot, "burner-mining-drill")

    if iron >= 8 and copper >= 4 and has_flow(snapshot, "iron-plate") and has_flow(snapshot, "copper-plate") then
        return { kind = policy.KINDS.OBSERVE, stage = 2,
                 _advance_stage = true,
                 reason = "stage 2 iron/copper targets met, advancing" }
    end

    if drills < 4 then
        return { kind = policy.KINDS.BUILD, stage = 2,
                 goal = { type = "built", prototype = "burner-mining-drill", count = 4 },
                 limits = { max_new_copies = 2 },
                 reason = "need more burner drills for stage 2 iron expansion" }
    end

    if iron < 8 then
        return { kind = policy.KINDS.BUILD, stage = 2,
                 goal = { type = "have", item = "iron-plate", count = 8 },
                 reason = "need 8 iron plates for stage 2" }
    end

    if copper < 4 then
        return { kind = policy.KINDS.BUILD, stage = 2,
                 goal = { type = "have", item = "copper-plate", count = 4 },
                 reason = "need 4 copper plates for stage 2" }
    end

    return nil
end

--- Stage 3: Expand iron to 12, copper to 6
function stage3_burner_expansion_copper(cfg, memory, snapshot, stage_def)
    local iron = accessible(snapshot, "iron-plate") or 0
    local copper = accessible(snapshot, "copper-plate") or 0
    local drills = count_instances(snapshot, "burner-mining-drill")

    if iron >= 12 and copper >= 6 and has_flow(snapshot, "iron-plate") and has_flow(snapshot, "copper-plate") then
        return { kind = policy.KINDS.OBSERVE, stage = 3,
                 _advance_stage = true,
                 reason = "stage 3 iron/copper targets met, advancing to electric transition" }
    end

    if drills < 6 then
        return { kind = policy.KINDS.BUILD, stage = 3,
                 goal = { type = "built", prototype = "burner-mining-drill", count = 6 },
                 limits = { max_new_copies = 2 },
                 reason = "need more burner drills for stage 3 copper expansion" }
    end

    if iron < 12 then
        return { kind = policy.KINDS.BUILD, stage = 3,
                 goal = { type = "have", item = "iron-plate", count = 12 },
                 reason = "need 12 iron plates for stage 3" }
    end

    if copper < 6 then
        return { kind = policy.KINDS.BUILD, stage = 3,
                 goal = { type = "have", item = "copper-plate", count = 6 },
                 reason = "need 6 copper plates for stage 3" }
    end

    return nil
end

--- Stage 4: Coal power — steam boiler, engine, electric drills
function stage4_coal_power(cfg, memory, snapshot, stage_def)
    local boilers = count_instances(snapshot, "boiler")
    local engines = count_instances(snapshot, "steam-engine")

    -- Check we have researched steam-power
    if not researched(snapshot, "steam-power") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 4,
                 goal = { type = "researched", name = "steam-power" },
                 reason = "need steam-power research for stage 4" }
    end

    -- Need flow of coal
    if not has_flow(snapshot, "coal") then
        return { kind = policy.KINDS.SUPPORT, stage = 4,
                 goal = { type = "sustain", item = "coal" },
                 reason = "need sustained coal flow for power" }
    end

    if engines < 1 then
        return { kind = policy.KINDS.BUILD, stage = 4,
                 goal = { type = "built", prototype = "steam-engine", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need at least 1 steam engine for stage 4" }
    end

    if boilers < 1 then
        return { kind = policy.KINDS.BUILD, stage = 4,
                 goal = { type = "built", prototype = "boiler", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need at least 1 boiler for stage 4" }
    end

    -- Verify power production (flow evidence of electricity)
    if has_flow(snapshot, "electricity") or snapshot.lab_capacity ~= nil then
        return { kind = policy.KINDS.OBSERVE, stage = 4,
                 _advance_stage = true,
                 reason = "power plant operational, advancing to automation science" }
    end

    return nil
end

--- Stage 5: Automation science pack production
function stage5_automation_science(cfg, memory, snapshot, stage_def)
    local inserter = count_instances(snapshot, "inserter")
    local assembler = count_instances(snapshot, "assembling-machine-1")

    -- Need electronics (for inserters)
    if not researched(snapshot, "electronics") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 5,
                 goal = { type = "researched", name = "electronics" },
                 reason = "need electronics research for stage 5" }
    end

    if assembler < 1 then
        return { kind = policy.KINDS.BUILD, stage = 5,
                 goal = { type = "built", prototype = "assembling-machine-1", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need at least 1 assembling machine for stage 5" }
    end

    -- Check for automation science packs
    local packs = accessible(snapshot, "automation-science-pack") or 0
    if packs < 10 then
        return { kind = policy.KINDS.BUILD, stage = 5,
                 goal = { type = "produce", item = "automation-science-pack", count = 10 },
                 reason = "need 10 automation science packs for stage 5" }
    end

    -- Research automation
    if not researched(snapshot, "automation") then
        return { kind = policy.KINDS.RESEARCH, stage = 5,
                 goal = { type = "researched", name = "automation" },
                 reason = "research automation to complete stage 5" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 5,
             _advance_stage = true,
             reason = "automation science and research complete, advancing" }
end

--- Stage 6: Logistic science packs
function stage6_logistics_science(cfg, memory, snapshot, stage_def)
    local packs = accessible(snapshot, "logistic-science-pack") or 0

    -- Need automation and logistics researched
    if not researched(snapshot, "automation") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 6,
                 goal = { type = "researched", name = "automation" },
                 reason = "need automation research for stage 6" }
    end

    -- Logistic robotics is mandatory but we don't construct robots here
    if not researched(snapshot, "logistic-robotics") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 6,
                 goal = { type = "researched", name = "logistic-robotics" },
                 reason = "need logistic-robotics research (mandatory, but no robot construction)" }
    end

    if packs < 10 then
        return { kind = policy.KINDS.BUILD, stage = 6,
                 goal = { type = "produce", item = "logistic-science-pack", count = 10 },
                 reason = "need 10 logistic science packs for stage 6" }
    end

    if not researched(snapshot, "logistic-science-pack") then
        return { kind = policy.KINDS.RESEARCH, stage = 6,
                 goal = { type = "researched", name = "logistic-science-pack" },
                 reason = "research logistic-science-pack to complete stage 6" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 6,
             _advance_stage = true,
             reason = "logistics science complete, advancing to oil processing" }
end

--- Stage 7: Oil processing — refinery, chemical plant, plastic, advanced circuits
function stage7_oil_processing(cfg, memory, snapshot, stage_def)
    local refineries = count_instances(snapshot, "oil-refinery")
    local chem_plants = count_instances(snapshot, "chemical-plant")

    -- Check for prerequisites being researched
    if not researched(snapshot, "oil-processing") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 7,
                 goal = { type = "researched", name = "oil-processing" },
                 reason = "need oil-processing research for stage 7" }
    end

    if refineries < 1 then
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "built", prototype = "oil-refinery", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need at least 1 oil refinery for stage 7" }
    end

    if chem_plants < 1 then
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "built", prototype = "chemical-plant", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "need at least 1 chemical plant for stage 7" }
    end

    -- Need plastic flow
    if not has_flow(snapshot, "plastic-bar") then
        return { kind = policy.KINDS.SUPPORT, stage = 7,
                 goal = { type = "sustain", item = "plastic-bar" },
                 reason = "need sustained plastic production for stage 7" }
    end

    -- Check for advanced-circuit production (needs plastic)
    if not has_flow(snapshot, "advanced-circuit") then
        return { kind = policy.KINDS.BUILD, stage = 7,
                 goal = { type = "produce", item = "advanced-circuit", count = 20 },
                 reason = "need advanced circuits (red chips) for processing units" }
    end

    -- Need processing units for rocket parts
    if not has_flow(snapshot, "processing-unit") then
        return { kind = policy.KINDS.SUPPORT, stage = 7,
                 goal = { type = "sustain", item = "processing-unit" },
                 reason = "need sustained processing unit production for rocket parts" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 7,
             _advance_stage = true,
             reason = "oil processing established, advancing to rocket prerequisites" }
end

--- Stage 8: Rocket prerequisite products
function stage8_rocket_prerequisites(cfg, memory, snapshot, stage_def)
    local total_steel = (snapshot.total_produced and snapshot.total_produced["steel-plate"]) or 0
    local steel = accessible(snapshot, "steel-plate") or 0
    local LDS = accessible(snapshot, "low-density-structure") or 0
    local rocket_fuel = accessible(snapshot, "rocket-fuel") or 0

    -- Advanced material processing 2 is mandatory research
    if not researched(snapshot, "advanced-material-processing-2") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 8,
                 goal = { type = "researched", name = "advanced-material-processing-2" },
                 reason = "need advanced-material-processing-2 research (mandatory for steel furnaces)" }
    end

    -- Need steel plate flow established (for LDS and rocket silo)
    if not has_flow(snapshot, "steel-plate") then
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "produce", item = "steel-plate", count = 200 },
                 reason = "need steel plate production for rocket prerequisites" }
    end

    if LDS < 10 then
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "produce", item = "low-density-structure", count = 10 },
                 reason = "need low density structures for rocket parts" }
    end

    if rocket_fuel < 10 then
        return { kind = policy.KINDS.BUILD, stage = 8,
                 goal = { type = "produce", item = "rocket-fuel", count = 10 },
                 reason = "need rocket fuel for rocket parts" }
    end

    -- Check for LDS flow and rocket-fuel flow
    if not has_flow(snapshot, "low-density-structure") then
        return { kind = policy.KINDS.SUPPORT, stage = 8,
                 goal = { type = "sustain", item = "low-density-structure" },
                 reason = "need sustained LDS production for rocket parts" }
    end

    if not has_flow(snapshot, "rocket-fuel") then
        return { kind = policy.KINDS.SUPPORT, stage = 8,
                 goal = { type = "sustain", item = "rocket-fuel" },
                 reason = "need sustained rocket fuel production for rocket parts" }
    end

    return { kind = policy.KINDS.OBSERVE, stage = 8,
             _advance_stage = true,
             reason = "rocket prerequisites met, advancing to silo construction" }
end

--- Stage 9: Rocket silo, payload production (50 parts)
function stage9_rocket_silo(cfg, memory, snapshot, stage_def)
    local parts_required = cfg.parts_required or policy.PINNED.rocket_parts_required
    local parts_produced = accessible(snapshot, "rocket-part") or 0
    local silos = count_instances(snapshot, "rocket-silo")

    -- Need production/utility science
    if not researched(snapshot, "rocket-silo") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 9,
                 goal = { type = "researched", name = "rocket-silo" },
                 reason = "need rocket-silo research for stage 9" }
    end

    if silos < 1 then
        return { kind = policy.KINDS.BUILD, stage = 9,
                 goal = { type = "built", prototype = "rocket-silo", count = 1 },
                 limits = { max_new_copies = 1 },
                 reason = "build rocket silo for stage 9" }
    end

    if parts_produced < parts_required then
        return { kind = policy.KINDS.BUILD, stage = 9,
                 goal = { type = "produce", item = "rocket-part", count = parts_required },
                 reason = string.format("produce %d rocket parts for launch", parts_required) }
    end

    -- Compute research service time to verify lab capacity for needed science
    local svc_time, rate = research_service_time(snapshot)
    if svc_time then
        local needed_rate = required_pack_rate(snapshot,
            stage_def.deadline_ticks or 180000, parts_required * 2)
        if rate and rate < needed_rate then
            return { kind = policy.KINDS.SUPPORT, stage = 9,
                     goal = { type = "scale", item = "science-pack", rate = needed_rate },
                     reason = string.format("lab capacity at %d/min, need %d/min for deadline", rate, needed_rate) }
        end
    end

    return { kind = policy.KINDS.OBSERVE, stage = 9,
             _advance_stage = true,
             reason = "rocket silo and parts ready, advancing to launch" }
end

--- Stage 10: Launch — starter pack, orbit, manifest complete
function stage10_launch(cfg, memory, snapshot, stage_def)
    local launch_evidence = snapshot.launch_evidence or {}

    -- Space platform research (starter pack unlock)
    if not researched(snapshot, "space-platform") then
        return { kind = policy.KINDS.PREREQUISITE, stage = 10,
                 goal = { type = "researched", name = "space-platform" },
                 reason = "need space-platform research for launch" }
    end

    -- Need starter pack materials
    local steel = accessible(snapshot, "steel-plate") or 0
    local processing_units = accessible(snapshot, "processing-unit") or 0
    local found = accessible(snapshot, "space-platform-foundation") or 0

    local pack_steel = policy.PINNED.starter_pack.steel
    local pack_pu = policy.PINNED.starter_pack.processing_units
    local pack_found = policy.PINNED.starter_pack.foundation

    if steel < pack_steel then
        return { kind = policy.KINDS.BUILD, stage = 10,
                 goal = { type = "have", item = "steel-plate", count = pack_steel },
                 reason = string.format("need %d steel for starter pack", pack_steel) }
    end

    if processing_units < pack_pu then
        return { kind = policy.KINDS.BUILD, stage = 10,
                 goal = { type = "have", item = "processing-unit", count = pack_pu },
                 reason = string.format("need %d processing units for starter pack", pack_pu) }
    end

    if found < pack_found then
        return { kind = policy.KINDS.BUILD, stage = 10,
                 goal = { type = "produce", item = "space-platform-foundation", count = pack_found },
                 reason = string.format("need %d space platform foundations for starter pack", pack_found) }
    end

    -- Reserve finite silo and payload quantities
    -- Total payload steel = 1,220; cable = 1,200
    local total_steel_needed = policy.PINNED.total_payload_steel
    local total_cable_needed = policy.PINNED.total_payload_cable
    local produced_steel = (snapshot.total_produced and snapshot.total_produced["steel-plate"]) or 0
    local produced_cable = (snapshot.total_produced and snapshot.total_produced["copper-cable"]) or 0

    -- Stop buffer production after next-two-batches plus repair/defense
    local buffer_limit = pack_steel * 3 + 200  -- 3 batches + repair stock
    if produced_steel > total_steel_needed + buffer_limit then
        -- Buffer complete; no more steel needed for payload
    end

    -- Check launch evidence
    if launch_evidence.silo_launched then
        next_mem.state = "complete"
        return nil, { kind = policy.KINDS.COMPLETE, stage = 10,
                       reason = "rocket launched, starter pack in orbit" }
    end

    -- Can't verify launch yet - ask to observe
    if not launch_evidence.silo_built then
        return { kind = policy.KINDS.OBSERVE, stage = 10,
                 goal = { type = "observe", target = "launch" },
                 reason = "waiting for silo construction confirmation" }
    end

    -- Issue launch command
    return { kind = policy.KINDS.LAUNCH, stage = 10,
             goal = { type = "launch", payload = "space-platform-starter-pack" },
             reason = "all prerequisites met; initiate launch" }
end

-- Export internal helpers for testing
policy.accessible = accessible
policy.researched = researched
policy.recipe_enabled = recipe_enabled
policy.count_instances = count_instances
policy.has_flow = has_flow
policy.research_service_time = research_service_time
policy.required_pack_rate = required_pack_rate
policy.deadline_infeasible = deadline_infeasible
policy.stage1_burner_start = stage1_burner_start
policy.stage2_burner_expansion_iron = stage2_burner_expansion_iron
policy.stage3_burner_expansion_copper = stage3_burner_expansion_copper
policy.stage4_coal_power = stage4_coal_power
policy.stage5_automation_science = stage5_automation_science
policy.stage6_logistics_science = stage6_logistics_science
policy.stage7_oil_processing = stage7_oil_processing
policy.stage8_rocket_prerequisites = stage8_rocket_prerequisites
policy.stage9_rocket_silo = stage9_rocket_silo
policy.stage10_launch = stage10_launch

-- Initialise stage helper: generate the 10-stage milestone source
-- from policy. Returns an array of milestone descriptions the driver uses.
function policy.milestones()
    local ms = {}
    for _, stage in ipairs(policy.STAGES) do
        ms[#ms + 1] = {
            kind = "stage",
            stage_id = stage.id,
            name = stage.name,
            label = stage.label,
            description = stage.description,
            deadline_ticks = stage.deadline_ticks,
            iron_target = stage.iron_target,
            copper_target = stage.copper_target,
            prerequisites = stage.prerequisites,
        }
    end
    return ms
end

-- Hash all policy definitions for the experiment manifest.
function policy.hash()
    local h = {}
    for _, stage in ipairs(policy.STAGES) do
        h[stage.name] = {
            id = stage.id,
            iron_target = stage.iron_target,
            copper_target = stage.copper_target,
            deadline_ticks = stage.deadline_ticks,
            prerequisites = stage.prerequisites,
        }
    end
    h.rocket_parts_required = policy.PINNED.rocket_parts_required
    h.starter_pack = policy.PINNED.starter_pack
    h.foundation_cost = policy.PINNED.foundation_cost
    h.total_payload_steel = policy.PINNED.total_payload_steel
    h.total_payload_cable = policy.PINNED.total_payload_cable
    return h
end
