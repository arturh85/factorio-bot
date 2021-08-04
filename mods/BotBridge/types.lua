-- MIT License
--
-- Copyright (c) 2020       Artur Hallmann
--
-- Permission is hereby granted, free of charge, to any person obtaining a
-- copy of this factorio lua stub and associated
-- documentation files (the "Software"), to deal in the Software without
-- restriction, including without limitation the rights to use, copy, modify,
-- merge, publish, distribute, sublicense, and/or sell copies of the
-- Software, and to permit persons to whom the Software is furnished to do
-- so, subject to the following conditions:
--
-- The above copyright notice and this permission notice shall be included in
-- all copies or substantial portions of the Software.
--
-- THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
-- IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
-- FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
-- THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
-- LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
-- FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
-- DEALINGS IN THE SOFTWARE.

function serialize_recipe(recipe)
    local record = table_properties(recipe, {"name", "valid", "enabled", "category", "hidden", "energy", "order"})
    -- "ingredients", "products",
    local ingredients = {}
    local ingredients_found = false
    for _, v in pairs(recipe.ingredients) do
        table.insert(ingredients, serialize_ingredient(v))
        ingredients_found = true
    end
    if ingredients_found then
        record.ingredients = ingredients
    end
    local products = {}
    for _, v in pairs(recipe.products) do
        table.insert(products, serialize_product(v))
    end
    record.products = products
    record.group = recipe.group.name
    record.subgroup = recipe.subgroup.name
    return record
end

function serialize_product(product)
    return table_properties(product, {"name", "type", "amount", "probability"}, {type = "productType"})
end

function serialize_ingredient(ingredient)
    return table_properties(ingredient, {"name", "type", "amount"}, {type = "ingredientType"})
end

function serialize_item_prototype(item)
    local record = table_properties(
        item,
        {"name", "stack_size", "fuel_value", "type", "speed", "durability"},
        {type = "itemType", stack_size = "stackSize", fuel_value = "fuelValue" }
    )
    record.placeResult = item.place_result and item.place_result.name or ""
    record.group = item.group.name
    record.subgroup = item.subgroup.name
    return record
end


function serialize_player(player)
    local record = table_properties(
        player,
        {
            "name", "index", "position", "build_distance",
            "reach_distance", "drop_item_distance", "item_pickup_distance",
            "loot_pickup_distance", "resource_reach_distance"
        },
        {
            index = "playerId",
            build_distance = "buildDistance",
            reach_distance = "reachDistance",
            drop_item_distance = "dropItemDistance",
            item_pickup_distance = "itemPickupDistance",
            loot_pickup_distance = "lootPickupDistance",
            resource_reach_distance = "resourceReachDistance"
        }
    )
    local main_inventory = player.get_main_inventory()
    record.mainInventory = main_inventory.get_contents()
    return record
end

-- force.get_saved_technology_progress(technology) → double
-- technologies :: CustomDictionary string → LuaTechnology [R]
-- research_queue :: array of TechnologySpecification [RW]	The research queue of this force.
-- research_enabled :: boolean [R]	Whether research is enabled for this force, see LuaForce::enable_research and LuaForce::disable_research
-- force.add_research(technology) → boolean	Add this technology to the back of the research queue if the queue is enabled.

function serialize_force(force)
    local record = table_properties(
        force,
        {"name", "index", "research_progress"},
        {index = "forceId", research_progress = "researchProgress"}
    )
    if force.current_research ~= nil then
        record.currentResearch = force.current_research.name
    else
        record.currentResearch = nil
    end
    local technologies = {}
    for _, v in pairs(force.technologies) do
        technologies[v.name] = serialize_technology(v)
    end
    record.technologies = technologies
    return record
end

function serialize_fluidbox_connection(conn)
    local record = table_properties(
        conn,
        {"positions", "type", "max_underground_distance"},
        {type = "connectionType", max_underground_distance = "maxUndergroundDistance"}
    )
    return record
end

function serialize_fluidbox_prototype(fluidbox)
    local record = table_properties(
        fluidbox,
        {"production_type"},
        {production_type = "productionType"}
    )

    local pipe_connections = {}
    local pipe_connections_found = false
    for _,v in pairs(fluidbox.pipe_connections) do
        pipe_connections_found = true
        table.insert(pipe_connections, serialize_fluidbox_connection(v))
    end
    if pipe_connections_found then
        record.pipeConnections = pipe_connections
    end
    return record
end

function serialize_technology(technology)
    local record = table_properties(
        technology,
        {
            "name", "enabled", "upgrade", "order", "researched",
            "level", "valid", "research_unit_count", "research_unit_energy"
        },
        {
            index = "forceId",
            research_unit_count="researchUnitCount",
            research_unit_energy = "researchUnitEnergy"
        }
    )
    local ingredients = {}
    for _, v in pairs(technology.research_unit_ingredients) do
        table.insert(ingredients, serialize_ingredient(v))
    end
    local prerequisites = nil
    for _, v in pairs(technology.prerequisites) do
        if prerequisites == nil then
            prerequisites = {}
        end
        table.insert(prerequisites, v.name)
    end
    record.researchUnitIngredients = ingredients
    record.prerequisites = prerequisites
    return record
end

function serialize_entity_prototype(entity)
    local collision_mask = nil
    if entity.collision_mask ~= nil then
        for k,v in pairs(entity.collision_mask) do
            if collision_mask == nil then
                collision_mask = {}
            end
            table.insert(collision_mask, k)
        end
    end
    local mine_result = {}
    local mining_time
    if entity.mineable_properties.minable then
        local array = {}
        if (entity.mineable_properties.products == nil) then
            -- print("wtf, entity "..entity.name.." is mineable, but has no products?!")
        else
            for itemname,amount in pairs(products_to_dict(entity.mineable_properties.products)) do
                mine_result[itemname] = amount
            end
        end
        mining_time = entity.mineable_properties.mining_time
    else
        mine_result = nil
    end
    local fluidbox_prototypes = {}
    local fluidbox_found = false
    for _,v in pairs(entity.fluidbox_prototypes) do
        fluidbox_found = true
        table.insert(fluidbox_prototypes, serialize_fluidbox_prototype(v))
    end
    local record = table_properties(entity, {"name", "type"}, {type = "entityType"})
    record.miningTime = mining_time
    record.maxUndergroundDistance = entity.max_underground_distance
    record.miningSpeed = entity.mining_speed
    record.craftingSpeed = entity.crafting_speed
    record.mineResult = mine_result
    if fluidbox_found then
        record.fluidboxPrototypes = fluidbox_prototypes
    end
    record.collisionMask = collision_mask
    record.collisionBox = table_properties(entity.collision_box, {"left_top", "right_bottom"}, {left_top = "leftTop", right_bottom = "rightBottom"})

    return record
end

function serialize_entity(entity)
    local record = table_properties(entity, {"name", "direction", "type", "position", "drop_position"}, {type = "entityType", drop_position = "dropPosition"})
    record.boundingBox = table_properties(entity.bounding_box, {"left_top", "right_bottom"}, {left_top = "leftTop", right_bottom = "rightBottom"})
    local output_inventory = entity.get_output_inventory()
    if output_inventory ~= nil then
        record.outputInventory = output_inventory.get_contents()
    end
    local fuel_inventory = entity.get_fuel_inventory()
    if fuel_inventory ~= nil then
        record.fuelInventory = fuel_inventory.get_contents()
    end

    if entity.type == "resource" then
        record.amount = entity.amount
    elseif entity.type == "inserter" then
        record.pickupPosition = entity.pickup_position
    elseif entity.type == "entity-ghost" then
        record.ghostName = entity.ghost_name
        record.ghostType = entity.ghost_type
        if entity.ghost_type == "assembling-machine" then
            local recipe = entity.get_recipe()
            if recipe ~= nil then
                record.recipe = recipe.name
            end
        end
    elseif entity.type == "assembling-machine" then
        local recipe = entity.get_recipe()
        if recipe ~= nil then
            record.recipe = recipe.name
        end
    end
    return record
end

function serialize_tile(tile)
    local record = table_properties(tile, {"name", "position"})
    record.playerCollidable = tile.collides_with('player-layer')
    return record
end

function table_properties(tbl, props, replacements)
    local filtered = {}
    for _, v in ipairs(props) do
        local target_v = v
        if replacements ~= nil and replacements[v] ~= nil then
            target_v = replacements[v]
        end
        filtered[target_v] = tbl[v]
    end
    return filtered
end
