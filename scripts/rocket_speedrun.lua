-- Driver script for the first-rocket module orchestration.
--
-- Uses the supervisor loop to drive policy.next() decisions through
-- the game. Converts normalized decision goals to goal.* constructors.
--
-- include("rocket_speedrun.lua")
--
-- The script expects:
--   rcon.* for game access
--   goal.* for planning
--   record.* for recording
--
-- Usage:
--   include("rocket_policy.lua")
--   include("rocket_speedrun.lua")
--   local run_id = rocket_speedrun.run({ bots = {1,2,3,4} })

include("supervisor.lua")

rocket_speedrun = {}

-- ---------------------------------------------------------------------------
-- Decision-to-goal conversion
-- ---------------------------------------------------------------------------

--- Convert a policy decision's goal description to a supervisor-compatible
--- milestone (a goal.* value, a witness, a sustain, or something the
--- supervisor already knows).
--
-- @param decision { kind, stage, goal, limits, reason }
-- @return a milestone for supervisor.new(), or nil if this decision
--         should be run as a non-planning step
function rocket_speedrun.goal_from_decision(decision)
    if type(decision) ~= "table" then return nil end
    local g = decision.goal
    if type(g) ~= "table" then return nil end

    local gtype = g.type
    if gtype == "have" then
        return goal.have(g.item, g.count or 1)
    elseif gtype == "produce" then
        return goal.produced(g.item, g.count or 1)
    elseif gtype == "researched" then
        return goal.researched(g.name)
    elseif gtype == "built" then
        return goal.have(g.prototype, g.count or 1)
    elseif gtype == "sustain" then
        -- Default window: 3 game-minutes lead-in, 3 game-minutes window
        return supervisor.sustain {
            item = g.item, per_minute = g.per_minute or 15,
            window_ticks = 10800, lead_in_ticks = 10800,
        }
    elseif gtype == "scale" then
        -- Rate scaling goal; use a sustain with higher target
        return goal.produced(g.item, g.count or 50)
    elseif gtype == "launch" then
        -- Launch is a special action; driver will handle it outside planning
        return nil
    elseif gtype == "observe" then
        return nil
    end
    return nil
end

--- Build a milestone source that reads from policy.next().
--
-- @param config  config table passed to policy.next()
-- @param initial_memory  initial memory table for policy.next()
-- @return a source function for supervisor.new()
function rocket_speedrun.source(config, initial_memory)
    local memory = initial_memory or {}
    local config = config or {}

    return function(_history)
        -- Get the next decision from policy
        local snapshot = rocket_speedrun.build_snapshot()
        local next_mem, decision = policy.next(config, memory, snapshot)
        memory = next_mem

        if decision.kind == policy.KINDS.COMPLETE then
            print("POLICY: " .. decision.reason)
            return nil
        end

        if decision.kind == policy.KINDS.BLOCKED then
            print("POLICY BLOCKED: " .. decision.reason)
            return nil
        end

        -- Log the decision
        local goal_info = ""
        if decision.goal then
            local g = decision.goal
            goal_info = string.format(" [%s %s=%s]", g.type or "?", g.name or g.item or "?", tostring(g.count or ""))
        end
        print(string.format("POLICY stage %d/%d: %s%s -- %s",
            decision.stage, #policy.STAGES, decision.kind, goal_info, decision.reason))

        -- Convert decision to milestone
        local milestone = rocket_speedrun.goal_from_decision(decision)
        if milestone then
            return milestone
        end

        -- For launch and observe decisions, we need non-planning steps.
        -- Return nil to let the supervisor finish, then driver handles separately.
        if decision.kind == policy.KINDS.LAUNCH then
            rocket_speedrun._pending_launch = true
            return nil
        end

        if decision.kind == policy.KINDS.OBSERVE or decision.kind == policy.KINDS.WAIT then
            -- Return a witness or sustain for passive observation
            if decision.goal and decision.goal.type == "sustain" then
                return supervisor.sustain {
                    item = decision.goal.item, per_minute = 15,
                    window_ticks = 10800, lead_in_ticks = 10800,
                }
            end
            -- For observe-only decisions, just wait a bit
            return supervisor.sustain {
                item = "iron-plate", per_minute = 1,
                window_ticks = 3600, lead_in_ticks = 3600,
            }
        end

        return nil
    end
end

-- ---------------------------------------------------------------------------
-- Build the snapshot of game state for policy.next()
-- ---------------------------------------------------------------------------

--- Build a snapshot table from the live game via rcon and the world model.
--- Unknown values are explicit nil rather than zero.
function rocket_speedrun.build_snapshot()
    local snapshot = {
        tick = nil,
        researched = {},
        recipe_enabled = {},
        accessible_stock = {},
        instances = {},
        flow_evidence = {},
        lab_capacity = nil,
        threats = nil,
        launch_evidence = {},
        module_shortfalls = nil,
        total_produced = {},
    }

    -- Get game tick
    if type(rcon) == "table" and type(rcon.game_tick) == "function" then
        snapshot.tick = rcon.game_tick()
    end

    -- Read researched technologies
    if type(rcon) == "table" and type(rcon.researched_technologies) == "function" then
        local techs = rcon.researched_technologies()
        if type(techs) == "table" then
            for _, tech in ipairs(techs) do
                snapshot.researched[tech] = true
            end
        end
    end

    -- Read accessible stock from the world model
    if type(world) == "table" and type(world.stock) == "function" then
        local stock = world.stock()
        if type(stock) == "table" then
            for item, count in pairs(stock) do
                snapshot.accessible_stock[item] = count
            end
        end
    end

    -- Read entity instances
    if type(world) == "table" and type(world.entity_counts) == "function" then
        local counts = world.entity_counts()
        if type(counts) == "table" then
            for proto, count in pairs(counts) do
                snapshot.instances[proto] = count
            end
        end
    end

    -- Read total production (lifetime counters)
    if type(rcon) == "table" and type(rcon.production_stats) == "function" then
        local stats = rcon.production_stats()
        if type(stats) == "table" then
            for item, count in pairs(stats) do
                snapshot.total_produced[item] = count
            end
        end
    end

    return snapshot
end

-- Run the full first-rocket speedrun.
--
-- @param opts { bots={...}, config={...} }
-- @return run_id
function rocket_speedrun.run(opts)
    opts = opts or {}
    local BOTS = opts.bots or {1, 2, 3, 4}
    local config = opts.config or {}

    -- Build the policy source
    local src = rocket_speedrun.source(config)

    -- Create supervisor
    local sup = supervisor.new(src, { bots = BOTS, stall_limit = 3, max_iterations = 20 })

    -- Run the supervisor loop
    local ok, err = pcall(function()
        repeat
            local t = sup:step()
            if t.action == "planned" and type(t.plan) == "table" then
                if type(record) == "table" and type(record.plan_created) == "function" then
                    record.plan_created(t.milestone_index, t.plan, t.bots)
                end
            end
            if t.action == "acquired" then
                print("-> milestone " .. t.milestone_index)
                if type(record) == "table" and type(record.milestone_started) == "function" then
                    record.milestone_started(t.milestone_index, "rocket stage")
                end
            elseif t.action == "planned" then
                print("   planned " .. t.steps .. " steps")
            elseif t.action == "ran" then
                print(string.format("   ran: success=%s pending=%s failed=%s lost=%s",
                    tostring(t.success), tostring(t.pending),
                    tostring(t.failed), tostring(t.lost)))
                if t.first_error then
                    print("        error: " .. t.first_error)
                end
            elseif t.action == "satisfied" then
                if type(record) == "table" and type(record.milestone_satisfied) == "function" then
                    record.milestone_satisfied(t.milestone_index, t.iteration or 0, t.reason)
                end
                print("   SATISFIED")
            elseif t.action == "halted" then
                local why = (t.refusal and t.refusal.message) or sup.first_error
                print("   HALTED: " .. t.state .. " -- " .. tostring(why))
                if type(record) == "table" and type(record.milestone_stuck) == "function" then
                    record.milestone_stuck(t.milestone_index, t.state, why, t.best)
                end
            end
        until sup:finished()
    end)

    if not ok then
        print("RAISED: " .. tostring(err))
        if type(record) == "table" and type(record.milestone_stuck) == "function" then
            record.milestone_stuck(sup.index or 0, "plan_error", tostring(err), nil)
        end
    end

    -- Handle pending launch (stage 10)
    if rocket_speedrun._pending_launch then
        print("INITIATING LAUNCH: space-platform-starter-pack")
        -- Launch command would go here via rcon
        rocket_speedrun._pending_launch = nil
    end

    -- Finish recording
    if type(record) == "table" and type(record.finish) == "function" then
        local id = record.finish(ok and sup.state or "crashed")
        print("RUN FINISHED state=" .. (ok and sup.state or "crashed") .. " id=" .. id)
    end

    print(sup:report())
    return ok
end

-- ============================================================================
-- Task 10: Oil access and static defense helpers
-- ============================================================================

--- Declare a defended construction node for oil corridor safety.
--
-- Emits goals for gun turrets, ammunition boxes, and repair supplies
-- at the named node position. Returns a milestone (or nil if already
-- satisfied) that the supervisor can run as a regular plan.
--
-- @param node_id  string identifying this node (e.g. "oil-corridor-1")
-- @param position { x, y } where the node is anchored
-- @param snapshot the game snapshot
-- @return goal milestone or nil
function rocket_speedrun.declare_defense_node(node_id, position, snapshot)
    if type(node_id) ~= "string" or type(position) ~= "table" then
        return nil
    end
    local sn = snapshot or {}

    -- Check if this node already has turrets
    local turret_count = (sn.instances or {})["gun-turret"] or 0

    -- Need at least 4 turrets per node for basic coverage
    if turret_count < 4 then
        return goal.have("gun-turret", 4)
    end

    -- Need ammunition (at least one full magazine stack per turret)
    local ammo = sn.accessible_stock and sn.accessible_stock["firearm-magazine"] or 0
    if ammo < 80 then
        return goal.have("firearm-magazine", 80)
    end

    -- Need repair packs for maintenance
    local repair = sn.accessible_stock and sn.accessible_stock["repair-pack"] or 0
    if repair < 10 then
        return goal.have("repair-pack", 10)
    end

    return nil  -- All defense requirements satisfied
end

--- Build the oil access survey snapshot from live game state.
--
-- Called before policy.oil_access to populate the snapshot fields
-- the pure helper expects.
function rocket_speedrun.build_oil_access_snapshot(base_snapshot)
    local sn = base_snapshot or rocket_speedrun.build_snapshot()

    -- Check whether oil is charted via the world model
    if type(world) == "table" and type(world.charted) == "function" then
        sn.oil_charted = world.charted("crude-oil")
    else
        sn.oil_charted = nil  -- unknown
    end

    -- Check route safety: query threat standoff around the nearest oil
    if type(world) == "table" and type(world.threats_from) == "function" then
        local oil_pos = world.nearest_resource("crude-oil")
        if oil_pos then
            local threats = world.threats_from(oil_pos, 50)
            sn.safe_route = (threats == nil or #threats == 0)
        else
            sn.safe_route = nil  -- unknown
        end
    else
        sn.safe_route = nil  -- unknown
    end

    -- Check defense stock readiness
    local turrets = (sn.instances or {})["gun-turret"] or 0
    local ammo = sn.accessible_stock and sn.accessible_stock["firearm-magazine"] or 0
    local repair = sn.accessible_stock and sn.accessible_stock["repair-pack"] or 0
    sn.defense_stock_ready = (turrets >= 4 and ammo >= 80 and repair >= 10)

    -- Check if defense is supported (turrets placed and loaded)
    local ammo_flow = sn.flow_evidence and sn.flow_evidence["firearm-magazine"]
    local turret_flow = sn.flow_evidence and sn.flow_evidence["gun-turret"]
    sn.defense_supported = (turret_flow == true and sn.defense_stock_ready)

    -- Survey budget remaining (default 3 rings, decrement as used)
    if sn.survey_budget_remaining == nil then
        sn.survey_budget_remaining = 3
    end

    -- Threats
    if type(world) == "table" and type(world.threats) == "function" then
        local threats = world.threats()
        if type(threats) == "table" then
            sn.threats = {
                nests_nearby = threats.nests_nearby or 0,
                worms_nearby = threats.worms_nearby or 0,
            }
        end
    end

    return sn
end

--- Place turrets at an oil corridor node using the planner.
--
-- This emits a build plan for gun turrets at the declared node position.
-- The actual placement is handled by goal.have() / goal.built() through
-- the normal plan execution.
--
-- Call from supervisor source when policy.oil_access returns BUILD
-- with prototype = "gun-turret".
function rocket_speedrun.oil_corridor_defense_plan()
    -- Return a milestone source that builds:
    -- 1. Gun turrets (4-6 for corridor coverage)
    -- 2. Ammunition stock (80+ firearm-magazine)
    -- 3. Repair supplies (10+ repair-pack)
    -- 4. Fuel for turret power (coal stack)
    -- All under the finite construction ledger.
    return supervisor.list {
        goal.have("gun-turret", 6),
        goal.have("firearm-magazine", 80),
        goal.have("repair-pack", 10),
    }
end

--- The full oil access preparation source for the supervisor.
--
-- Extends the policy pipeline: calls policy.oil_access until safe route
-- is established, then hands control back to the main policy source.
function rocket_speedrun.oil_access_source(config, initial_memory)
    local memory = initial_memory or {}
    local prev_state = "preparing"

    return function(_history)
        local snapshot = rocket_speedrun.build_oil_access_snapshot()
        local decision = policy.oil_access(config, snapshot)

        if decision == nil then
            print("POLICY: oil access established -- returning to main policy")
            return nil  -- Hand control back to main pipeline
        end

        -- Log the decision
        local goal_info = ""
        if decision.goal then
            local g = decision.goal
            goal_info = string.format(" [%s %s=%s]", g.type or "?", g.name or g.item or "?", tostring(g.count or ""))
        end
        print(string.format("POLICY oil_access: %s%s -- %s", decision.kind, goal_info, decision.reason or ""))

        -- Convert to milestone
        if decision.kind == policy.KINDS.BUILD and decision.goal then
            local gtype = decision.goal.type
            if gtype == "built" then
                return goal.have(decision.goal.prototype, decision.goal.count or 1)
            end
        end

        if decision.kind == policy.KINDS.SUPPORT then
            return supervisor.sustain {
                item = decision.goal and decision.goal.item or "firearm-magazine",
                per_minute = 5,
                window_ticks = 7200,
                lead_in_ticks = 3600,
            }
        end

        if decision.kind == policy.KINDS.BUILD or decision.kind == policy.KINDS.OBSERVE then
            -- Default observe
            return supervisor.sustain {
                item = "iron-plate",
                per_minute = 1,
                window_ticks = 3600,
                lead_in_ticks = 1800,
            }
        end

        if decision.kind == policy.KINDS.BLOCKED then
            print("POLICY BLOCKED (oil_access): " .. (decision.reason or "unsafe_oil_corridor"))
            if type(record) == "table" and type(record.milestone_stuck) == "function" then
                record.milestone_stuck(0, "blocked", "unsafe_oil_corridor", nil)
            end
            return nil
        end

        return nil
    end
end
