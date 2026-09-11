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
