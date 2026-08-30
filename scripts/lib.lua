-- Shared helper library.
--
-- Migration note: the Lua `plan.*` table is gone. Work that used to be
-- hand-scheduled step by step is now *declared* as a goal (`goal.have`) and the
-- planner derives the mining, walking and placing itself. Work that was really
-- an immediate command against a live game is now `rcon.*`.

function find_mine_with_bots(bots, search_center, name, type, count)
    local entities = world.find_entities_in_radius(search_center, 300, name, type)
    local label = name or type
    if #entities < #bots then
        error("not enough " .. label .. " in radius 300")
    end
    -- The old code asked each bot for `count`, so the goal asks for the sum.
    return mine_with_bots(bots, label, count * #bots)
end

-- Was: one `plan.mine` per bot per entity, round-robining a list of nearby
-- entities by index arithmetic -- and carrying a `FIXME: use all bots`, because
-- it never used them all. `goal.have` owns both of those choices now, which
-- entity and which bot, so the caller only says how many it wants in the end.
-- The `entities` argument is gone with the index math that consumed it.
function mine_with_bots(bots, item_name, count)
    local plan = goal.have(item_name, count)
    goal.schedule(plan, #bots)
    return plan
end

-- Rocks are terrain to clear, not an item count: there is no meaningful
-- "have N rocks", so this stayed an immediate command instead of becoming a
-- goal. `rcon.mine` needs a live game -- without one there is no `rcon` global,
-- and nothing to clear either.
function mine_rocks(bots, count)
    if rcon == nil then
        print("SKIP mine_rocks: no game connected, nothing to clear")
        return
    end
    for _, bot_id in pairs(bots) do
        local player = world.player(bot_id)
        local rocks = world.find_entities_in_radius(player.position, 100, ENTITIES.ROCK_HUGE)
        if #rocks == 0 then
            -- The old fallback searched for ROCK_BIG and then re-passed the
            -- *empty* huge-rock list, so it never mined anything.
            rocks = world.find_entities_in_radius(player.position, 100, ENTITIES.ROCK_BIG)
        end
        for i = 1, math.min(count, #rocks) do
            rcon.mine(bot_id, rocks[i].name, rocks[i].position, 1)
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
