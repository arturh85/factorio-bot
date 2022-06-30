function find_mine_with_bots(bots, search_center, name, type, count)
    local entities = world.find_entities_in_radius(search_center, 300, name, type)
    local label = name or type
    if #entities < #bots then
        error("not enough " .. label .. " in radius 300")
    end
    mine_with_bots(bots, entities, label, count)
end

-- FIXME: use all bots
function mine_with_bots(bots, entities, label, count)
    plan.group_start("Mine " .. label .. "x" .. tostring(count) .. " with " .. tostring(#bots) .. " Bots")
    for idx,playerId in pairs(bots) do
        for i=1,count do
            local entityIdx = ((idx - 1) * count) + i
            if entityIdx >= #entities then
                break
            end
            plan.mine(playerId, entities[entityIdx].position, entities[entityIdx].name, 1)
        end
    end
    plan.group_end()
end

function mine_rocks(bots, count)
    local player = world.player(bots[1])
    local huge_rocks = world.find_entities_in_radius(player.position, 100, "rock-huge")
    if #huge_rocks > 0 then
        mine_with_bots(bots, huge_rocks, "rocks", count)
    else
        local big_rocks = world.find_entities_in_radius(player.position, 100, "rock-big")
        if #big_rocks > 0 then
            mine_with_bots(bots, huge_rocks, "rocks", count)
        end
    end
end

function required_ingredients(recipe, search_ingredient, count)
    local sum = 0
    for idx,ingredient in pairs(recipe.ingredients) do
        if ingredient.name == search_ingredient then
            sum = sum + ingredient.amount * count
        else
            local subrecipe = world.recipe(ingredient.name)
            if subrecipe ~= nil then
                sum = sum + required_ingredients(subrecipe, search_ingredient, ingredient.amount * count)
            end
        end
    end
    return sum
end

function dump(tbl, label)
    print("dumping " .. label .. "\n" .. dump_deep(tbl, 0))
    print("------------")
end

function dump_deep(tbl, lvl)
    local output = ""
    for k,v in pairs(tbl) do
        for i=1,lvl do output = output .. " " end

        if type(v) == "table" then
            output = output .. ("- " .. tostring(k) .. ":\n")
            if lvl < 10 then
                output = output .. dump_deep(v, lvl + 2)
            end
        else
            output = output .. ("- " .. tostring(k) .. ": " .. tostring(v) .. "\n")
        end
    end
    return output
end
