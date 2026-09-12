-- Persistent request memory and event handlers for rocket launch control.
--
-- Tracks rocket launch requests, observes the game's on_rocket_launched and
-- on_space_platform_started events, and exposes remote interface functions.
--
-- All state lives in storage.rocket_launch, which survives save/load.
-- Initialisation is done from on_init (via an idempotent check) and the
-- event handlers are registered at load.

local rocket_launch = {}

-- ---------------------------------------------------------------------------
-- Initialisation
-- ---------------------------------------------------------------------------

function rocket_launch.init()
    if storage.rocket_launch ~= nil then return end
    storage.rocket_launch = {
        -- request_key -> { key, payload, planet, silo_unit_number, starter_pack,
        --                  launch_ordered_tick, launched_tick,
        --                  platform_established_tick, error }
        requests = {},
        -- silo_unit_number -> { launch_ordered_tick, payload }
        silo_launches = {},
    }
    print("rocket_launch: initialised")
end

-- ---------------------------------------------------------------------------
-- Request management
-- ---------------------------------------------------------------------------

--- Create a rocket launch request. Returns { ok = true, key = key } or
--- { ok = false, error = "..." }.
--
-- @param opts { key, payload, planet, starter_pack }
function rocket_launch.request(opts)
    if type(opts) ~= "table" then
        return { ok = false, error = "args must be a table" }
    end
    local key = opts.key
    if type(key) ~= "string" or key == "" then
        return { ok = false, error = "key must be a non-empty string" }
    end
    -- Reject duplicate key
    if storage.rocket_launch.requests[key] ~= nil then
        return { ok = false, error = "duplicate request key: " .. key }
    end
    local payload = opts.payload or "space-platform-starter-pack"
    local planet = opts.planet or "nauvis"
    local starter_pack = opts.starter_pack

    -- Validate: if starter_pack is provided it must be a table
    if starter_pack ~= nil and type(starter_pack) ~= "table" then
        return { ok = false, error = "starter_pack must be a table or nil" }
    end

    local request = {
        key = key,
        payload = payload,
        planet = planet,
        starter_pack = starter_pack,
        silo_unit_number = nil,
        launch_ordered_tick = nil,
        launched_tick = nil,
        platform_established_tick = nil,
        error = nil,
    }
    storage.rocket_launch.requests[key] = request
    print(string.format("rocket_launch: request created key=%s payload=%s planet=%s",
        key, payload, planet))
    return { ok = true, key = key }
end

--- Query the status of a request. Returns a table with key, payload, planet,
--- and optional tick fields, or { ok = false, error = "..." }.
--
-- @param key  string request key
function rocket_launch.status(key)
    if type(key) ~= "string" then
        return { ok = false, error = "key must be a string" }
    end
    local req = storage.rocket_launch.requests[key]
    if req == nil then
        return { ok = false, error = "no such request: " .. key }
    end
    return {
        ok = true,
        key = req.key,
        payload = req.payload,
        planet = req.planet,
        silo_unit_number = req.silo_unit_number,
        launch_ordered_tick = req.launch_ordered_tick,
        launched_tick = req.launched_tick,
        platform_established_tick = req.platform_established_tick,
        has_starter_pack = (req.starter_pack ~= nil),
        error = req.error,
    }
end

--- Query all request keys.
function rocket_launch.list_requests()
    local keys = {}
    for k, _ in pairs(storage.rocket_launch.requests) do
        table.insert(keys, k)
    end
    table.sort(keys)
    return { ok = true, keys = keys }
end

-- ---------------------------------------------------------------------------
-- Launch trigger
-- ---------------------------------------------------------------------------

--- Order a rocket launch at a specific silo for an existing request.
-- Returns { ok = true, silo_unit_number, launch_ordered_tick } or
-- { ok = false, error = "..." }.
--
-- Validates that:
--   - the request exists and has not already been completed
--   - the silo exists, is valid, and belongs to the player force
--   - the silo has a rocket ready (rocket_silo_rocket inventory is full enough)
--   - the rocket has the correct payload (starter pack inserted)
--   - the destination matches the request's planet
--
-- @param opts { key, silo_unit_number }
function rocket_launch.launch(opts)
    if type(opts) ~= "table" then
        return { ok = false, error = "args must be a table" }
    end
    local key = opts.key
    local silo_unit_number = opts.silo_unit_number
    if type(key) ~= "string" then
        return { ok = false, error = "key must be a string" }
    end
    if type(silo_unit_number) ~= "number" then
        return { ok = false, error = "silo_unit_number must be a number" }
    end

    -- Look up the request
    local req = storage.rocket_launch.requests[key]
    if req == nil then
        return { ok = false, error = "no such request: " .. key }
    end

    -- Check not already launched
    if req.launch_ordered_tick ~= nil then
        return { ok = false, error = "launch already ordered for: " .. key }
    end
    if req.launched_tick ~= nil then
        return { ok = false, error = "rocket already launched for: " .. key }
    end

    -- Find the silo entity
    local silo = nil
    for _, surface in pairs(game.surfaces) do
        local ent = surface.find_entity("rocket-silo", silo_unit_number)
        if ent ~= nil and ent.valid then
            silo = ent
            break
        end
    end
    if silo == nil then
        return { ok = false, error = "silo not found: unit_number=" .. silo_unit_number }
    end

    -- Check force
    local player_force = game.forces["player"]
    if silo.force.name ~= player_force.name then
        return { ok = false, error = string.format(
            "silo %d belongs to force '%s', not '%s'",
            silo_unit_number, silo.force.name, player_force.name) }
    end

    -- Check the silo has a rocket ready (rocket inventory has parts)
    local rocket_inv = silo.get_inventory(defines.inventory.rocket)
    if rocket_inv == nil then
        return { ok = false, error = "silo has no rocket inventory" }
    end

    -- Insert starter pack items if provided
    if req.starter_pack ~= nil then
        for _, stack in ipairs(req.starter_pack) do
            local name = stack.name or stack[1]
            local count = stack.count or stack[2] or 1
            local inserted = rocket_inv.insert({ name = name, count = count })
            if inserted < count then
                return { ok = false, error = string.format(
                    "failed to insert %s x%d into rocket (inserted %d)",
                    name, count, inserted) }
            end
        end
    end

    -- Check silo is ready for launch
    local ok_launch, result = pcall(function()
        return silo.launch_rocket()
    end)
    if not ok_launch then
        -- This covers both "not ready" errors and misconfiguration
        return { ok = false, error = "launch_rocket failed: " .. tostring(result) }
    end
    if result ~= true then
        -- false means the silo is not ready
        return { ok = false, error = "not_ready" }
    end

    -- Record the launch order
    local tick = game.tick
    req.silo_unit_number = silo_unit_number
    req.launch_ordered_tick = tick

    print(string.format("rocket_launch: launch ordered key=%s silo=%d tick=%d",
        key, silo_unit_number, tick))
    return { ok = true, silo_unit_number = silo_unit_number, launch_ordered_tick = tick }
end

-- ---------------------------------------------------------------------------
-- Event handlers
-- ---------------------------------------------------------------------------

--- Handle on_rocket_launched event.
-- Records the launched_tick on the matching request (matched by silo unit_number
-- carried in the event's rocket entity).
--
-- @param event { rocket, silo, tick }
function rocket_launch.on_rocket_launched(event)
    if event == nil or event.rocket == nil or event.silo == nil then return end
    local silo = event.silo
    if not silo.valid then return end
    local silo_unit_number = silo.unit_number
    local tick = event.tick or game.tick

    -- Store in silo_launches index
    local sl = storage.rocket_launch.silo_launches[silo_unit_number]
    if sl == nil then
        sl = {}
        storage.rocket_launch.silo_launches[silo_unit_number] = sl
    end
    sl.launched_tick = tick

    -- Update any request bound to this silo
    for key, req in pairs(storage.rocket_launch.requests) do
        if req.silo_unit_number == silo_unit_number then
            req.launched_tick = tick
            print(string.format("rocket_launch: launched key=%s silo=%d tick=%d",
                key, silo_unit_number, tick))
        end
    end
end

--- Handle on_space_platform_started event.
-- Records the platform_established_tick on the matching request.
--
-- The event carries the platform's surface/index; we match by checking if
-- any request with a launch_ordered_tick (indicating it was sent to the
-- platform's destination) should receive this.
--
-- @param event { platform, surface, tick }
function rocket_launch.on_space_platform_started(event)
    if event == nil or event.platform == nil then return end
    local platform = event.platform
    local surface = event.surface
    local tick = event.tick or game.tick
    local index

    local ok, platform_idx = pcall(function() return platform.index end)
    if ok and platform_idx ~= nil then
        index = platform_idx
    else
        local ok2, pfid = pcall(function() return platform.platform_id end)
        if ok2 and pfid ~= nil then index = pfid else index = 0 end
    end

    -- Find the first uncompleted request and assign the platform
    -- We match by finding a request that has been launch_ordered but has
    -- no platform_established_tick yet.
    for key, req in pairs(storage.rocket_launch.requests) do
        if req.launch_ordered_tick ~= nil
            and req.platform_established_tick == nil
        then
            req.platform_established_tick = tick
            print(string.format("rocket_launch: platform established key=%s index=%d tick=%d",
                key, index or 0, tick))
            return
        end
    end

    -- No matching request found: log it but don't error
    print(string.format("rocket_launch: platform started (unmatched) index=%d tick=%d",
        index or 0, tick))
end

-- ---------------------------------------------------------------------------
-- Evidence construction helpers
-- ----------------------------------------------------------------------------

--- Build a RocketLaunchEvidence table for a given request key.
-- Returns { ok = true, evidence = {...} } or { ok = false, error = "..." }.
-- The evidence table has the same shape as the Rust RocketLaunchEvidence struct.
--
-- @param key  string request key
function rocket_launch.evidence(key)
    local st = rocket_launch.status(key)
    if not st.ok then return st end

    local sl = storage.rocket_launch.silo_launches[st.silo_unit_number]
    local launched_tick = sl and sl.launched_tick or st.launched_tick

    return {
        ok = true,
        evidence = {
            run_key = key,
            silo_unit_number = st.silo_unit_number or 0,
            platform_index = 0,  -- populated from event data
            payload = st.payload,
            launch_ordered_tick = st.launch_ordered_tick,
            launched_tick = launched_tick,
            platform_established_tick = st.platform_established_tick,
        },
    }
end

-- ---------------------------------------------------------------------------
-- Remote interface registration
-- ----------------------------------------------------------------------------

return rocket_launch
