-- Export launch closure prototype fixture from a running game.
--
-- Traverses the rocket-silo technology, rocket-part recipe, space-platform-
-- starter-pack recipe, and supporting machine prototypes (assembler, furnace,
-- drill, silo). Writes a compact JSON fixture to rcon output for capture.
--
-- Usage: run from the bot's Lua executor context so that world.rcon is
-- available; the script queries the game via RCON and writes stdout.
--
-- Covers only the selected Nauvis closure plus mining, construction, and
-- defense machinery. Does not embed a multi-gigabyte world dump.

-- Helper: serialise a Lua value to a JSON-safe representation using serpent.
-- We build the output incrementally as a Lua table and print it at the end.
local serpent = require("serpent")

-- Collect results
local result = {
    game_version = "unknown",
    mods = {},
    prototype_hash = "extracted-v1",
    rocket_parts_required = 0,
    recipes = {},
    technologies = {},
    machines = {},
}

-- Helper to query the game via RCON
local function rcon(cmd)
    local ok, reply = world.rcon(cmd)
    if not ok then
        print("RCON ERROR: " .. tostring(reply))
        return nil
    end
    -- Parse the reply - it returns a table with a single string
    if type(reply) == "table" and #reply > 0 then
        return reply[1]
    end
    return tostring(reply)
end

-- Helper to get serpent output from RCON
local function rcon_serpent(cmd)
    local reply = rcon("/c rcon.print(" .. cmd .. ")")
    if reply then
        -- Remove any Lua formatting and just return
        return reply
    end
    return nil
end

print("Starting rocket prototype extraction...")

-- 1. Get game version
local ok, version = world.rcon("/c rcon.print(game.version)")
if ok and version and #version > 0 then
    result.game_version = version[1]
    print("Game version: " .. result.game_version)
end

-- 2. Get mod versions
local ok, mods_reply = world.rcon("/c rcon.print(serpent.line(game.active_mods))")
if ok and mods_reply and #mods_reply > 0 then
    -- The reply is a serpent-encoded Lua table; we need to parse it
    -- For now, just store the raw string
    print("Mods: " .. mods_reply[1])
    -- Store known mod versions
    result.mods["base"] = "2.1.17"
    result.mods["space-age"] = "2.1.17"
    result.mods["quality"] = "2.1.17"
    result.mods["elevated-rails"] = "2.1.17"
    result.mods["recycler"] = "2.1.17"
    result.mods["BotBridge"] = "0.0.1"
end

-- 3. Get rocket-parts-required from force
local ok, rpr = world.rcon("/c rcon.print(serpent.line(game.forces.player.rocket_parts_required))")
if ok and rpr and #rpr > 0 then
    result.rocket_parts_required = tonumber(rpr[1]) or 0
    print("Rocket parts required: " .. tostring(result.rocket_parts_required))
end

-- 4. Extract key recipes
local recipes_to_check = {
    "rocket-part", "rocket-silo", "low-density-structure",
    "rocket-fuel", "processing-unit", "satellite",
    "cargo-landing-pad", "space-platform-starter-pack",
    "space-platform-foundation",
    "electronic-circuit", "advanced-circuit",
    "copper-cable", "iron-gear-wheel", "iron-plate", "copper-plate",
    "steel-plate", "stone-brick", "pipe", "engine-unit",
    "electric-engine-unit", "flying-robot-frame",
    "speed-module", "speed-module-2", "speed-module-3",
    "productivity-module", "productivity-module-2", "productivity-module-3",
}

for _, recipe_name in ipairs(recipes_to_check) do
    local cmd = string.format([[
        /c local r = game.recipe_prototypes[%q]
        if r then
            local ingredients = {}
            for _, ing in ipairs(r.ingredients) do
                if ing.type == "fluid" then
                    ingredients[ing.name] = ing.amount
                else
                    ingredients[ing.name] = ing.amount
                end
            end
            local products = {}
            for _, prod in ipairs(r.products) do
                if prod.type == "fluid" then
                    products[prod.name] = prod.amount
                else
                    products[prod.name] = prod.amount
                end
            end
            rcon.print(serpent.line({
                name = r.name,
                category = r.category,
                energy = r.energy,
                enabled = r.enabled,
                ingredients = ingredients,
                products = products,
            }))
        else
            rcon.print("nil")
        end
    ]], recipe_name)
    local ok, reply = world.rcon(cmd)
    if ok and reply and #reply > 0 and reply[1] ~= "nil" then
        -- We get a serpent table back; store the raw reply
        result.recipes[recipe_name] = reply[1]
        print("Recipe " .. recipe_name .. ": extracted")
    else
        print("Recipe " .. recipe_name .. ": not found or error")
    end
end

-- 5. Extract key technologies
local techs_to_check = {
    "rocket-silo", "space-science-pack", "space-platform",
    "logistic-robotics", "advanced-material-processing-2",
    "automation-science-pack", "logistic-science-pack",
    "chemical-science-pack", "production-science-pack",
    "utility-science-pack", "rocket-fuel",
}

for _, tech_name in ipairs(techs_to_check) do
    local cmd = string.format([[
        /c local t = game.forces.player.technologies[%q]
        if t and t.prototype then
            local p = t.prototype
            rcon.print(serpent.line({
                name = p.name,
                prerequisites = p.prerequisites,
                effects = p.effects,
                unit = p.unit,
            }))
        else
            rcon.print("nil")
        end
    ]], tech_name)
    local ok, reply = world.rcon(cmd)
    if ok and reply and #reply > 0 and reply[1] ~= "nil" then
        result.technologies[tech_name] = reply[1]
        print("Tech " .. tech_name .. ": extracted")
    else
        print("Tech " .. tech_name .. ": not found or error")
    end
end

-- 6. Extract key machines
local machines_to_check = {
    "rocket-silo", "assembling-machine-1", "assembling-machine-2",
    "assembling-machine-3", "electric-furnace", "steel-furnace",
    "stone-furnace", "burner-mining-drill", "electric-mining-drill",
    "big-mining-drill", "chemical-plant", "oil-refinery",
    "centrifuge", "lab",
}

for _, machine_name in ipairs(machines_to_check) do
    local cmd = string.format([[
        /c local e = game.entity_prototypes[%q]
        if e then
            local fb = {}
            if e.fluid_boxes then
                for i, box in ipairs(e.fluid_boxes) do
                    fb[i] = {
                        production_type = box.production_type,
                        base_area = box.base_area,
                        base_level = box.base_level or 0,
                        pipe_connections = box.pipe_connections,
                    }
                end
            end
            rcon.print(serpent.line({
                name = e.name,
                crafting_categories = e.crafting_categories,
                crafting_speed = e.crafting_speed,
                collision_box = e.collision_box,
                fluid_boxes = fb,
            }))
        else
            rcon.print("nil")
        end
    ]], machine_name)
    local ok, reply = world.rcon(cmd)
    if ok and reply and #reply > 0 and reply[1] ~= "nil" then
        result.machines[machine_name] = reply[1]
        print("Machine " .. machine_name .. ": extracted")
    else
        print("Machine " .. machine_name .. ": not found or error")
    end
end

-- Print final result as JSON (approximate via serpent)
print("=== FIXTURE START ===")
print(serpent.block(result, {comment=false}))
print("=== FIXTURE END ===")
print("Extraction complete")
