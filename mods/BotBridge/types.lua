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
    -- Factorio 2.1 replaced LuaRecipe.category (one string) with categories (an
    -- array), so the pcall above yields nothing on 2.1. The planner branches on
    -- the crafting category ("smelting" vs "crafting"), so keep sending it under
    -- the old name.
    if record.category == nil then
        local ok, categories = pcall(function() return recipe.categories end)
        if ok and categories ~= nil then
            record.category = categories[1]
        end
    end
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

-- Factorio 2.1 (runtime-api.json, ItemProduct/FluidProduct) has no
-- `probability` field: it became `independent_probability` plus a
-- `shared_probability` {min, max} window. `amount` is optional there too --
-- randomised outputs carry `amount_min`/`amount_max` instead. Send whatever
-- this game version has and let FactorioProduct (crates/core/src/types.rs)
-- normalise it; absent keys are simply left out of the JSON.
function serialize_product(product)
    return table_properties(
        product,
        {
            "name", "type", "amount", "amount_min", "amount_max",
            "probability", "independent_probability", "shared_probability",
            "fluidbox_index"
        },
        {type = "product_type"}
    )
end

-- WHICH FLUIDBOX THIS INGREDIENT GOES INTO, when the recipe overrides the
-- default.
--
-- `fluidbox_index` is on `IngredientPrototype`/`ProductPrototype` and is
-- **absent whenever the recipe takes the default**, which is nearly always.
-- It is 1-based **within the production type**, not over
-- `fluidbox_prototypes`: measured on a live 2.1.17 server 2026-09-08,
-- `basic-oil-processing` declares `fluidbox_index = 2` for crude-oil and
-- `= 3` for petroleum-gas, and a placed `oil-refinery` with that recipe put
-- crude on input box **2** at offset (1,2) and petroleum on output box
-- **3** at (2,-2) -- the refinery's 2nd input and 3rd output, not its 2nd
-- and 3rd fluidboxes (box 3 is the FIRST output).
--
-- **This is the field that makes `method::pipe`'s port choice a fact rather
-- than a guess.** Without it the planner assigns fluids to input boxes
-- positionally, which is right whenever the counts match and wrong for
-- exactly this recipe -- and a pipe on the wrong box builds 100% correctly
-- and moves nothing, the silent class this repo has already paid for with
-- inserters and with pumps.
function serialize_ingredient(ingredient)
    return table_properties(
        ingredient,
        {"name", "type", "amount", "fluidbox_index"},
        {type = "ingredient_type"}
    )
end

-- `speed` and `durability` were in this list and are deliberately not any
-- more. Neither is declared on `FactorioItemPrototype` in
-- `crates/core/src/types.rs`, so both were serialised and then dropped at
-- deserialisation -- and `durability` was not even being read: 2.0 moved it to
-- `LuaItemPrototype::get_durability()`, so the attribute read raised and the
-- `pcall` in `table_properties` swallowed it. Confirmed against a live 2.1.17
-- capture: `repair-pack` is the only `repair-tool` in 342 item prototypes,
-- its `speed` arrived as 2, and its `durability` arrived nil.
--
-- Repairing the read would have added a field with no consumer, so the fix is
-- deletion. If durability is ever wanted, it needs the Rust struct first and
-- then `item.get_durability()` here, never the attribute.
function serialize_item_prototype(item)
    local record = table_properties(
        item,
        {"name", "stack_size", "fuel_value", "type"},
        {type = "item_type", stack_size = "stack_size", fuel_value = "fuel_value" }
    )
    record.place_result = item.place_result and item.place_result.name or ""
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
            index = "player_id",
            build_distance = "build_distance",
            reach_distance = "reach_distance",
            drop_item_distance = "drop_item_distance",
            item_pickup_distance = "item_pickup_distance",
            loot_pickup_distance = "loot_pickup_distance",
            resource_reach_distance = "resource_reach_distance"
        }
    )
    local main_inventory = player.get_main_inventory()
    record.main_inventory = main_inventory.get_contents()
    -- Where this bot IS, in the sense a coordinate cannot express. See
    -- serialize_entity. A bot that dies on another surface respawns on Nauvis
    -- (`create_bot_character(game.surfaces[1], ...)` in control.lua) and until
    -- now nothing in the record could show that it had moved.
    record.surface = player.surface and player.surface.name or nil
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
        -- manual_mining_speed_modifier: hand mining runs at
        -- character.mining_speed * (1 + this). Vanilla's steel-axe research
        -- sets it to 1, doubling it, so the planner cannot assume a constant.
        --
        -- The rest are its siblings: every LuaForce attribute that scales a
        -- rate rather than a distance or a slot count. A rate in this project
        -- is `prototype x force bonus x module effect` and only the first
        -- factor was ever carried, which made every rate a constant that
        -- research could not move. Sending them does not by itself make the
        -- planner read them -- today only manual_mining_speed_modifier is
        -- read -- but a datum that never leaves the game cannot be read at
        -- all, and that was the actual root cause.
        --
        -- Every name here was checked against
        -- workspace/factorio-api-docs/runtime-api.json (2.1.17), class
        -- LuaForce: all are non-optional read attributes, `double` except
        -- belt_stack_size_bonus and bulk_inserter_capacity_bonus which are
        -- `uint32`. table_properties pcalls each one, so a name this game
        -- version does not have is dropped rather than raised.
        {
            "name", "index", "research_progress",
            "manual_mining_speed_modifier",
            "manual_crafting_speed_modifier",
            "character_running_speed_modifier",
            "laboratory_speed_modifier",
            "laboratory_productivity_bonus",
            "mining_drill_productivity_bonus",
            "inserter_stack_size_bonus",
            "bulk_inserter_capacity_bonus",
            "belt_stack_size_bonus",
            "worker_robots_speed_modifier",
        },
        {index = "force_id", research_progress = "research_progress"}
    )
    if force.current_research ~= nil then
        record.current_research = force.current_research.name
    else
        record.current_research = nil
    end
    local technologies = {}
    for _, v in pairs(force.technologies) do
        technologies[v.name] = serialize_technology(v)
    end
    record.technologies = technologies
    return record
end

-- `connection_type` is read under its own name, NOT as `type` renamed. `conn`
-- is a `PipeConnectionDefinition`, and 2.1.17 (runtime-api.json) declares that
-- table with `connection_type` and no `type` at all. Because it is a plain Lua
-- table rather than a userdata, the missing key did not even raise -- it
-- returned nil, so `table_properties` dropped the key with nothing to catch,
-- which is why no pcall diagnosed this one. The live 2.1.17 capture had it on
-- 0 of 95 pipe connections while `positions`, its neighbour on the very same
-- table, arrived on 95 of 95.
--
-- What it now carries is `PipeConnectionType`: "normal" / "underground" /
-- "linked" -- 94 normal and 1 underground in vanilla, and that one underground
-- connection is exactly the one that also reports `max_underground_distance`.
-- It is NOT the input/output flow direction the commented-out
-- `FactorioFluidBoxConnectionType` in crates/core/src/types.rs assumes; 2.0
-- split that off into `flow_direction`, which no Rust type declares and this
-- function therefore does not send.
function serialize_fluidbox_connection(conn)
    local record = table_properties(
        conn,
        {"positions", "connection_type", "max_underground_distance"},
        {connection_type = "connection_type", max_underground_distance = "max_underground_distance"}
    )
    return record
end

function serialize_fluidbox_prototype(fluidbox)
    local record = table_properties(
        fluidbox,
        {"production_type"},
        {production_type = "production_type"}
    )

    local pipe_connections = {}
    local pipe_connections_found = false
    for _,v in pairs(fluidbox.pipe_connections) do
        pipe_connections_found = true
        table.insert(pipe_connections, serialize_fluidbox_connection(v))
    end
    if pipe_connections_found then
        record.pipe_connections = pipe_connections
    end
    -- HOW MUCH THE BOX HOLDS.
    --
    -- Without it this record says exactly where a tank may be joined and
    -- nothing about its capacity, so anybody planning `Goal::Stored { fluid,
    -- amount, .. }` has to hard-code a table of vanilla numbers -- the same
    -- mod-compatibility defect as `pole_supply_half_extent`, and the same one
    -- the world-record base just falsified for smelting, where a copied
    -- `1/3.2` was exactly 2.0x low on every plate because 1,196 of 1,222
    -- furnaces are steel.
    --
    -- **`get_volume()` is a METHOD, not an attribute.** There is no `volume`
    -- on `LuaFluidBoxPrototype` in 2.1.17 at all (checked against
    -- `runtime-api.json`, not recalled) -- the same shape that made
    -- `crafting_speed` arrive nil for 1,028 prototypes and that
    -- `get_supply_area_distance()` above nearly repeated: the attribute read
    -- raises, this `pcall` swallows it, and the field is simply absent with
    -- nothing to say it should not be. No argument means normal quality,
    -- which is what the planner plans for.
    local ok, volume = pcall(function() return fluidbox.get_volume() end)
    if ok then record.volume = volume end

    -- WHICH FLUID THIS BOX WILL EVER ACCEPT.
    --
    -- `production_type` says which way fluid moves and `pipe_connections`
    -- says where a pipe may meet it. Neither says what the box wants, and
    -- geometry never can: the planner sees a pipe reaching an oil refinery's
    -- port and cannot see whether that port takes crude or water. That is the
    -- fact `crates/planner/src/method/pipe.rs` names as the one thing keeping
    -- its water rule unreachable.
    --
    -- **`filter` is an ATTRIBUTE on `LuaFluidBoxPrototype`, not a method** --
    -- checked in `runtime-api.json` for 2.1.17 rather than recalled, because
    -- the opposite mistake is what made `get_volume()` above need its `pcall`
    -- and what made `crafting_speed` arrive nil for 1,028 prototypes. It
    -- yields a `LuaFluidPrototype` or `nil`.
    --
    -- **Three states, and `nil` is the middle one, not the last.** A `nil`
    -- filter is the prototype saying *"this box takes anything"* -- which is
    -- every crafting machine, where the recipe picks the fluid. That is an
    -- answer. Sending nothing at all is the third state, and it is what the
    -- reader sees when this `pcall` fails or when a record predates this
    -- field; `FluidFilter`'s default is `unknown`, so those never masquerade
    -- as `any`.
    --
    -- This is the PROTOTYPE filter, deliberately. `LuaEntity::get_fluid_filter`
    -- is a different question -- what one standing pump is currently set to --
    -- and nothing in the Rust model has anywhere to put it.
    local filter_ok, filter = pcall(function() return fluidbox.filter end)
    if filter_ok then
        if filter == nil then
            record.filter = { kind = "any" }
        else
            record.filter = { kind = "only", fluid = filter.name }
        end
    end
    return record
end

-- The names a trigger payload field carries, as a list.
--
-- A trigger field arrives in one of three spellings depending on which
-- schema the runtime follows -- a bare name (`"crude-oil"`), an
-- `ItemIDFilter`/`EntityIDFilter` table (`{name = "lab", quality = ...}`), or
-- a list of either (`entities = {"crude-oil"}`) -- and this reads all three
-- into one shape. `nil`, an empty table and anything else read as no names,
-- which the caller turns into "send the type alone".
function trigger_names(value)
    local names = {}
    if type(value) == "string" then
        table.insert(names, value)
    elseif type(value) == "table" then
        if type(value.name) == "string" then
            table.insert(names, value.name)
        else
            for _, v in ipairs(value) do
                if type(v) == "string" then
                    table.insert(names, v)
                elseif type(v) == "table" and type(v.name) == "string" then
                    table.insert(names, v.name)
                end
            end
        end
    end
    return names
end

function serialize_technology(technology)
    local record = table_properties(
        technology,
        {
            "name", "enabled", "upgrade", "order", "researched",
            "level", "valid", "research_unit_count", "research_unit_energy"
        },
        {
            index = "force_id",
            research_unit_count = "research_unit_count",
            research_unit_energy = "research_unit_energy"
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
    record.research_unit_ingredients = ingredients
    record.prerequisites = prerequisites
    -- Which recipes researching this unlocks.
    --
    -- `effects` lives on LuaTechnologyPrototype, NOT on LuaTechnology (see
    -- runtime-api.json: LuaTechnology has no `effects` attribute at all), so
    -- it has to be reached through `.prototype`. Each entry is a
    -- TechnologyModifier -- a table whose `type` selects which other fields
    -- exist -- and the `unlock-recipe` variant carries a non-optional
    -- `recipe` string.
    --
    -- This is what lets the planner plan *through* a locked recipe: without
    -- it, a recipe that is disabled today is either invisible (the old
    -- behaviour, which broke goal.researched for every technology that
    -- unlocks anything) or visible but unreachable, which would be worse --
    -- plans that can never execute.
    --
    -- `record.effects` beside it is the WHOLE list, unlock-recipe included.
    -- Discarding every other effect is why nothing downstream could know that
    -- `steel-axe` grants `character-mining-speed +1` -- the planner was not
    -- ignoring the datum, it had never been sent one. `unlocked_recipes` is
    -- kept as its own key rather than derived on the Rust side because every
    -- existing consumer reads it and every archived payload has it; the
    -- duplication buys a seam that does not move.
    --
    -- The shape is flattened deliberately. `TechnologyModifier`
    -- (runtime-api.json 2.1.17) is a table tagged by `type` with 51 variant
    -- groups, and 44 of them carry exactly one field, `modifier`. Mirroring
    -- 51 variants into Rust would make every future Factorio version and
    -- every mod that adds a modifier type a deserialisation failure, which is
    -- the opposite of what carrying this data is for. So: `kind` is the
    -- `type` string verbatim, `modifier` is the number, and `target` is
    -- whichever single string field the variant uses to name what it acts on.
    --
    -- The two variants that do not spell their number `modifier`:
    -- `change-recipe-productivity` uses `change` (plus `recipe`), and
    -- `give-item` has `count` (plus `item`). Boolean modifiers -- seven
    -- variants, e.g. `mining-with-fluid` -- become 1 or 0 rather than being
    -- dropped, so "this technology enables it" survives as a number.
    -- `nothing` carries only a LocalisedString and reaches Rust as kind alone.
    local unlocked = {}
    local all_effects = {}
    local ok, effects = pcall(function() return technology.prototype.effects end)
    if ok and effects ~= nil then
        for _, effect in pairs(effects) do
            if effect.type == "unlock-recipe" and effect.recipe ~= nil then
                table.insert(unlocked, effect.recipe)
            end
            if effect.type ~= nil then
                local entry = {kind = effect.type}
                local amount = effect.modifier
                if amount == nil then amount = effect.change end
                if amount == nil then amount = effect.count end
                if type(amount) == "boolean" then
                    amount = amount and 1 or 0
                end
                if type(amount) == "number" then
                    entry.modifier = amount
                end
                -- Exactly one of these is present per variant; the order is
                -- only a way to ask for all of them at once.
                entry.target = effect.recipe or effect.ammo_category
                    or effect.turret_id or effect.item or effect.quality
                    or effect.space_location
                table.insert(all_effects, entry)
            end
        end
    end
    record.unlocked_recipes = unlocked
    record.effects = all_effects

    -- How this technology is unlocked, when it is NOT unlocked by science
    -- packs.
    --
    -- Factorio 2.0 added `research_trigger`: the technology completes when the
    -- player does a thing (craft 50 steel plates, mine an entity, build one)
    -- rather than when a lab consumes packs. For those,
    -- `research_unit_ingredients` is empty and `research_unit_energy` is zero,
    -- so a planner reading only the pack fields costs them at *nothing* -- it
    -- orders them correctly and times them wrongly, silently. Live 2.1.17 has
    -- 32 of them, including `electronics`, `steam-power`,
    -- `automation-science-pack` and `steel-axe`.
    --
    -- Like `effects` above, this lives on LuaTechnologyPrototype and NOT on
    -- LuaTechnology (runtime-api.json 2.1.17: `LuaTechnology` has no
    -- `research_trigger` attribute at all), so it has to be reached through
    -- `.prototype`. The pcall guards the same way.
    --
    -- Every trigger type carries whatever payload the runtime table holds,
    -- normalised to one shape per kind. Until 2026-09-05 only `craft-item`
    -- did, because the two shipped schemas disagree about `mine-entity`:
    -- runtime-api.json 2.1.17 documents a singular `entity :: string`, while
    -- `data/base/prototypes/technology.lua` writes `entities = {"crude-oil"}`,
    -- a list (and Space Age lists up to four names for one trigger). Nobody
    -- has captured what the runtime actually hands `.prototype.research_trigger`
    -- for one of these -- there is no live dump of it in the archive -- so
    -- rather than pick a schema, `trigger_names` accepts BOTH: a bare name, an
    -- `ItemIDFilter`/`EntityIDFilter` table with a `name`, or a list of
    -- either, and always sends `entities`, a list. The planner then chooses
    -- among the names instead of being told "trigger-based, and I cannot
    -- describe it" -- which is what the bare type used to mean, and which
    -- made `oil-processing` unplannable whether or not a well was charted.
    --
    -- What is still sent for a trigger that names nothing is the bare type,
    -- exactly as before, so an old dump and a new one disagree only by the
    -- payload's presence and the planner can tell "undescribed" from
    -- "unsupported".
    local ok_trigger, trigger = pcall(function()
        return technology.prototype.research_trigger
    end)
    if ok_trigger and trigger ~= nil and trigger.type ~= nil then
        local out = { type = trigger.type }
        if trigger.type == "craft-item" then
            local names = trigger_names(trigger.item)
            if #names > 0 then
                out.item = names[1]
                out.count = trigger.count or 1
            end
        elseif trigger.type == "craft-fluid" then
            local names = trigger_names(trigger.fluid)
            if #names > 0 then
                out.fluid = names[1]
                out.amount = trigger.amount or 1
            end
        elseif trigger.type == "mine-entity" or trigger.type == "build-entity" then
            -- The list wins when both spellings are present and it is not
            -- empty; the singular is the documented runtime shape.
            local names = trigger_names(trigger.entities)
            if #names == 0 then
                names = trigger_names(trigger.entity)
            end
            if #names > 0 then
                out.entities = names
                out.count = trigger.count or 1
            end
        elseif trigger.type == "send-item-to-orbit" then
            local names = trigger_names(trigger.item)
            if #names > 0 then
                out.item = names[1]
            end
        elseif trigger.type == "capture-spawner" then
            -- `entity` is optional here: the shipped `captivity` trigger
            -- names none, meaning any spawner.
            local names = trigger_names(trigger.entity)
            if #names > 0 then
                out.entity = names[1]
            end
        end
        -- Only send a craft-item trigger that actually named an item; a
        -- half-filled one would deserialise into a goal to craft nothing.
        if trigger.type ~= "craft-item" or out.item ~= nil then
            record.research_trigger = out
        end
    end
    return record
end

function serialize_entity_prototype(entity)
    local collision_mask = nil
    if entity.collision_mask ~= nil then
        -- The names of the collision layers this prototype collides with.
        --
        -- Iterate `.layers`, not the mask itself. Factorio 2.0 turned
        -- `collision_mask` from a flat set of layer names into a
        -- `CollisionMask` table -- `{layers = {name -> true}, plus
        -- colliding_with_tiles_only / consider_tile_transitions /
        -- not_colliding_with_itself}` (runtime-api.json, concept
        -- `CollisionMask`). `pairs()` over the mask therefore yielded that
        -- outer table's OWN keys and never a layer name: the live 2.1.17
        -- capture has literally `["layers"]` on 796 of 1028 prototypes and
        -- one of the three boolean flags alongside it on the rest.
        --
        -- That is worse than the nil this file's other 2.0 breakages produced,
        -- because a non-empty list of plausible-looking strings reads as
        -- working data. The pcall stays: it guards the iteration itself, which
        -- is what the original comment was worried about.
        local ok, _ = pcall(function()
            for k,v in pairs(entity.collision_mask.layers) do
                if collision_mask == nil then
                    collision_mask = {}
                end
                table.insert(collision_mask, k)
            end
        end)
    end
    local mine_result = {}
    local mining_time
    if entity.mineable_properties and entity.mineable_properties.minable then
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
    if entity.fluidbox_prototypes then
        for _,v in pairs(entity.fluidbox_prototypes) do
            fluidbox_found = true
            table.insert(fluidbox_prototypes, serialize_fluidbox_prototype(v))
        end
    end
    local record = table_properties(entity, {"name", "type"}, {type = "entity_type"})
    record.mining_time = mining_time
    -- These properties may not exist on all entity prototypes
    local ok, val
    ok, val = pcall(function() return entity.max_underground_distance end)
    if ok then record.max_underground_distance = val end
    ok, val = pcall(function() return entity.mining_speed end)
    if ok then record.mining_speed = val end
    -- crafting_speed: a furnace's own speed divides the recipe's time, so a
    -- steel or electric furnace (2) smelts in half a stone furnace's (1).
    --
    -- This is `get_crafting_speed()`, not `.crafting_speed`. Factorio 2.0's
    -- quality rework turned the attribute into a method taking an optional
    -- QualityID -- `LuaEntityPrototype::get_crafting_speed`, subclasses
    -- CraftingMachine and Character -- and left no attribute of that name. The
    -- attribute read below it used to be raised on every 2.x prototype, the
    -- pcall swallowed it, and the field arrived nil for all 1028 prototypes of
    -- a live 2.1.17 game while mining_speed (still an attribute) arrived fine.
    -- No argument means normal quality, which is what the planner plans for.
    ok, val = pcall(function() return entity.get_crafting_speed() end)
    if ok then record.crafting_speed = val end
    record.mine_result = mine_result
    -- Whether a CHARACTER can mine this by hand, which `minable` and
    -- `mine_result` cannot say: crude oil is `minable` with a product of ten
    -- crude oil, exactly as iron ore is `minable` with a product of one iron
    -- ore, and `products_to_dict` flattens the product's `type` away. `minable`
    -- is the flag a pumpjack uses. Verified live on 2026-09-04:
    -- `character.mine_entity(crude-oil)` returns false and leaves the well
    -- untouched. The game's own rule is categorical: a resource carries a
    -- `resource_category` (`category` at data stage) and a character or drill
    -- carries the `resource_categories` it supports, and mining is allowed iff
    -- the former is in the latter. Both halves are sent so the planner reads
    -- the rule off the prototypes instead of naming crude oil.
    -- `mining_drill_radius`: how far a drill reaches BEYOND the tile it stands
    -- on, which its collision box cannot say. Measured on a live 2.1.17 game:
    -- a burner-mining-drill is 1.40x1.40 with a radius of 0.99, so its mining
    -- area IS its own footprint; an electric-mining-drill is 2.70x2.70 with a
    -- radius of 2.49, so it works a 5x5 -- a full tile ring beyond itself.
    --
    -- Without this the planner cannot express "a drill mines a tile it does
    -- not stand on", and that single gap made two separate things needlessly
    -- conservative: ore-aware siting asked whether ore lay under the drill's
    -- own footprint because that was all the model offered, and the
    -- will-not-bury placement rule could not tell a belt over ore a drill can
    -- still reach from a belt over ore nobody can mine. For an electric drill
    -- the outer ring stays mineable and burial there is not a cost at all;
    -- for a burner drill it genuinely is, because its area is its footprint.
    ok, val = pcall(function() return entity.mining_drill_radius end)
    if ok then record.mining_drill_radius = val end
    -- `fluid_source_offset`: which TILE an offshore pump draws from, as an
    -- offset from its own position in the north frame. `{0, -1}` for the
    -- vanilla `offshore-pump`, i.e. the tile directly ahead of it.
    --
    -- **This is the field that makes the water rule derivable rather than
    -- written out.** A 2.0 pump's output fluidbox carries no filter -- it
    -- takes whatever the source tile gives -- so "does this pump produce
    -- water" is answered by pairing this offset with `FactorioTile::fluid`.
    -- Without it the planner would have to hard-code `{0, -1}` and the set of
    -- prototypes it applies to, which is the same mod-compatibility defect as
    -- the copied smelting rate and the hand-written pole supply table.
    --
    -- `subclasses = {"OffshorePump"}` on the runtime API, so `pcall` is doing
    -- real work here: it is nil or absent for every other prototype, and
    -- **that absence is the discriminator** -- an entity with no fluid source
    -- offset does not draw from the ground at all.
    ok, val = pcall(function() return entity.fluid_source_offset end)
    if ok and val ~= nil then
        record.fluid_source_offset = {x = val.x or val[1], y = val.y or val[2]}
    end
    -- INSERTER REACH. How far this inserter swings, as the two vectors the
    -- game itself uses: where it takes from and where it puts, both relative
    -- to its own position in the NORTH frame.
    --
    -- `crates/planner/src/state.rs`'s `inserter_reach` writes these out by
    -- hand -- `1` for every inserter and `2` for `long-handed-inserter` -- and
    -- says in its own doc that it is "the fifth hand-written table in this
    -- file". This is the field that deletes it, exactly as
    -- `get_supply_area_distance()` deleted `pole_supply_half_extent` and
    -- `get_max_wire_distance()` deleted `pole_wire_reach`.
    --
    -- **They are ATTRIBUTES, not methods**, unlike the two poles fields above
    -- -- checked against this install's `runtime-api.json` rather than
    -- recalled: `LuaEntityPrototype.inserter_pickup_position` (order 125) and
    -- `inserter_drop_position` (order 126), both `read_type = "Vector"`,
    -- both `subclasses = {"Inserter"}`, both optional.
    --
    -- **The two are NOT mirror images and a reader must not assume they are.**
    -- Vanilla is `pickup_position = {0, -1}` against
    -- `insert_position = {0, 1.2}` (base/prototypes/entity/entities.lua:2385),
    -- and the long-handed one is `{0, -2}` against `{0, 2.2}`. The 0.2 is real:
    -- an inserter's hand drops a fifth of a tile PAST the tile centre, so the
    -- two vectors differ in magnitude while landing on tiles that are
    -- symmetric about the inserter. Whoever converts these to tile offsets
    -- must floor into tiles rather than round the numbers, and must do it for
    -- each vector separately.
    --
    -- **`subclasses = {"Inserter"}` makes absence the discriminator**, the
    -- same shape as `fluid_source_offset` above: a `stone-furnace` has no
    -- pickup position because a furnace does not reach out and take from the
    -- chest beside it, and that is a fact about the prototype rather than a
    -- gap in this collector. What it cannot distinguish on its own is a world
    -- dumped before this field existed -- every archived dump in
    -- `workspace/scripts/` -- which is why the Rust side keeps a fallback
    -- table for vanilla names and documents it as a fallback for old senders
    -- and never as the definition of reach.
    ok, val = pcall(function() return entity.inserter_pickup_position end)
    if ok and val ~= nil then
        record.inserter_pickup_position = {x = val.x or val[1], y = val.y or val[2]}
    end
    ok, val = pcall(function() return entity.inserter_drop_position end)
    if ok and val ~= nil then
        record.inserter_drop_position = {x = val.x or val[1], y = val.y or val[2]}
    end
    -- BEACON AND POLE GEOMETRY. `FactorioEntityPrototype` carried nothing
    -- electrical at all, which is why `crates/planner/src/method/power.rs`
    -- writes `pole_supply_half_extent` out by hand as a table of vanilla
    -- names and says in its own doc that sending this field is the follow-up
    -- that deletes it. Two sessions were blocked on beacon spacing and both
    -- correctly refused to invent a number.
    --
    -- **`get_supply_area_distance()` is a METHOD, not an attribute.** There is
    -- no `supply_area_distance` on `LuaEntityPrototype` in 2.1.17 at all
    -- (checked against `runtime-api.json`, not recalled) -- the same shape
    -- that made `crafting_speed` arrive nil for all 1028 prototypes above: the
    -- attribute read raises, `pcall` swallows it, and the field is simply
    -- absent with nothing to say it should not be. No argument means normal
    -- quality, which is what the planner plans for.
    --
    -- It answers for an `electric-pole` as well as a `beacon` -- half the side
    -- of the square it supplies, so 2.5 for a small pole's 5x5.
    ok, val = pcall(function() return entity.get_supply_area_distance() end)
    if ok then record.supply_area_distance = val end
    -- How far this entity can throw a wire, in tiles. The supply area above
    -- says what ground a pole COVERS; this says what other pole it can REACH,
    -- and the two are independent numbers -- a `big-electric-pole` covers 4x4
    -- and spans 32 tiles, a `substation` covers 18x18 and spans 18.
    --
    -- **`get_max_wire_distance()` is a METHOD, and there is no
    -- `maximum_wire_distance` attribute on `LuaEntityPrototype` at all** in
    -- 2.1.17 (checked against this install's `runtime-api.json`, not
    -- recalled). `maximum_wire_distance` is the DATA-stage spelling, which is
    -- the trap: it is what `base/prototypes/entity/entities.lua` says and what
    -- anybody would try first, and reading it here raises, `pcall` swallows
    -- it, and the field arrives nil for every prototype in the game with
    -- nothing anywhere saying it should not have. That is exactly how
    -- `crafting_speed` was nil for all 1028 prototypes of a live game.
    --
    -- **Sent for EVERY entity, including the ones with no wires**, unlike
    -- `get_supply_area_distance()` above. This method carries no `subclasses`
    -- restriction and answers **0** for an entity that supports no wires, so
    -- the read never raises and there is nothing for the `pcall` to gate on.
    -- Sending the honest 0 is what keeps `Some(0.0)` -- "the game says this
    -- entity has no wires" -- distinguishable from `None` -- "the sender did
    -- not say", which is every world dumped before 2026-09-07. A collector
    -- that dropped the zeros would merge those two into one answer, and
    -- `crates/planner/src/state.rs`'s fallback table fires on exactly one of
    -- them.
    --
    -- **It is the maximum over EVERY wire kind, not a pole's copper span, and
    -- that is measured rather than reasoned about.** Over all 1,028
    -- prototypes of a live 2.1.17 game (seed 31337, 2026-09-07): 4 read a
    -- pole's span (7.5 / 9 / 32 / 18, matching the data stage exactly), 94
    -- more read a **circuit** wire distance -- `stone-furnace` 9,
    -- `wooden-chest` 9, `power-switch` 10, `agricultural-tower` 30 -- and 930
    -- read 0, which are trees, explosions, corpses and other things nothing
    -- connects to. So a reader that took this for "is a pole" would find 98
    -- of them. The gate that stops it lives in the planner
    -- (`entity_type == "electric-pole"`), not here: this collector's job is
    -- to report what the game says, and deciding what a pole is in Lua is the
    -- kind of prototype-shape decision no Rust test can see.
    --
    -- No argument means normal quality, which is what the planner plans for.
    ok, val = pcall(function() return entity.get_max_wire_distance() end)
    if ok then record.maximum_wire_distance = val end
    -- FLUID PRODUCED PER TICK by an offshore pump, or moved per tick by a
    -- normal pump. `crates/core/src/graph/flow_graph.rs`'s `OffshorePump` arm
    -- hard-coded **1.0 per second** from the day the file was written, which
    -- made water the largest wrong number in the whole model: 600/min
    -- supplied on the world-record base against 68,250/min that the machines
    -- the game has configured were eating. The game says 20 per tick.
    --
    -- **`get_pumping_speed()` is a METHOD and there is no `pumping_speed`
    -- attribute at all.** Not recalled -- measured on a live 2.1.17 server on
    -- 2026-09-07, which answered:
    --
    --     offshore get_pumping_speed=20  pump get=20
    --     attribute_ok=false
    --     attribute_val=LuaEntityPrototype doesn't contain key pumping_speed.
    --
    -- That error is precisely the shape the `pcall` around every read here
    -- swallows: reading the attribute raises, the field arrives nil for every
    -- prototype in the game, and nothing anywhere says it should not have.
    -- Four field pairs this week went attribute/method one each way and
    -- `crafting_speed` spent a whole session nil for 1028 live prototypes.
    --
    -- **The unit is fluid units per TICK**, the game's own, sent unconverted.
    -- `PrototypeBase::pumping_speed` at the data stage is documented "How many
    -- units of fluid are produced per tick" and `base/prototypes/entity/
    -- entities.lua` writes 20 for both `offshore-pump` and `pump`; the runtime
    -- method answers the same 20, not 1200, so it is per tick as well.
    -- `FactorioEntityPrototype::pumping_speed_per_second` does the x60 in one
    -- place so a reader cannot pick a different conversion. Verified against
    -- the running game rather than against the docs: an offshore pump piped
    -- into a storage tank filled it at **19.53 units/tick** over two
    -- consecutive 203-tick windows (3,964.84 units each), 97.7% of 20 with the
    -- pipe run taking the rest. Read as per *second* the same figure would
    -- have been 60x wrong and every check against a doc would still agree.
    --
    -- `subclasses` are `OffshorePump` and `Pump`, so this raises on anything
    -- else and the `pcall` is what makes a furnace report nothing -- the same
    -- gate `get_supply_area_distance()` above relies on. No argument means
    -- normal quality, which is what the planner plans for.
    ok, val = pcall(function() return entity.get_pumping_speed() end)
    if ok then record.pumping_speed = val end
    -- ELECTRICAL DRAW AND OUTPUT. `crates/planner/src/state.rs` writes
    -- `consumer_kw` and `generation_kw` out by hand -- 14 rows and 2 -- and
    -- says in its own doc that sending these is what deletes them. The
    -- milestone arithmetic (24 electric furnaces at 180 kW against a 1.8 MW
    -- plant) rests on numbers no code has ever checked against the game.
    --
    -- **`energy_usage` is an ATTRIBUTE and `get_max_energy_production()` is a
    -- METHOD**, checked against this install's `runtime-api.json` rather than
    -- recalled -- the two are opposite answers and reading a method as an
    -- attribute raises, `pcall` swallows it, and the field arrives nil with
    -- nothing saying it should not have (that is how `crafting_speed` was nil
    -- for all 1028 prototypes of a live game).
    --
    -- Both are **joules per tick**, the game's own unit, sent unconverted.
    -- `FactorioEntityPrototype::energy_usage_kw` does the x60/1000, in one
    -- place, so a reader cannot pick a different conversion.
    --
    -- **`energy_usage` is gated on the entity having an ELECTRIC energy
    -- source, and that gate is the whole safety of the field.** A stone
    -- furnace's `energy_usage` is 90 kW *of coal*; charged against an
    -- electric network's budget it is a number in the wrong units that every
    -- test would agree with. `state.rs` names exactly that as the reason
    -- burner machines are absent from `consumer_kw` rather than zero in it,
    -- so the gate lives here where the energy source is visible, and the
    -- field's name says it holds.
    ok, val = pcall(function()
        if entity.electric_energy_source_prototype == nil then return nil end
        return entity.energy_usage
    end)
    if ok then record.electric_energy_usage = val end
    -- The generation half. There is no `max_energy_production` attribute and
    -- no `fluid_usage_per_tick` on `LuaEntityPrototype` at all in 2.1.17, so
    -- the physics (fluid usage x heat capacity x temperature delta x
    -- effectivity) cannot be re-derived out here -- the runtime does it and
    -- hands over the answer. Not gated on an energy *source*: a generator
    -- consumes steam and produces electricity, so it has no electric energy
    -- source to test.
    ok, val = pcall(function() return entity.get_max_energy_production() end)
    if ok then record.max_energy_production = val end
    -- SOLAR, whose nameplate is a lie by itself. `max_energy_production`
    -- above answers 60 kW for a `solar-panel` and 300 kW for an
    -- `accumulator`, and both are instantaneous maxima: the panel's is its
    -- NOON output and the accumulator's is a discharge limit. The day/night
    -- curve that turns noon into an average is surface state, not prototype
    -- data -- see `serialize_surface_daylight` at the foot of this file --
    -- but its two ENDPOINTS are here, and without them the integral has a
    -- shape and no scale.
    --
    -- Both are attributes with `subclasses: ["SolarPanel"]`, so reading
    -- either on anything else raises and `pcall` drops it. That is the
    -- intended gate: the field's presence is what says "this is a solar
    -- panel", the same way `electric_energy_usage`'s presence says "this is
    -- electric".
    ok, val = pcall(function() return entity.solar_panel_performance_at_day end)
    if ok then record.solar_panel_performance_at_day = val end
    ok, val = pcall(function() return entity.solar_panel_performance_at_night end)
    if ok then record.solar_panel_performance_at_night = val end
    -- How many joules the entity's own electric buffer holds.
    --
    -- **This is the accumulator's real number**, and it is not on
    -- `LuaEntityPrototype` at all: `electric_energy_source_prototype` is an
    -- optional attribute returning a `LuaElectricEnergySourcePrototype`, and
    -- `buffer_capacity` is an attribute on THAT. A sub-prototype nothing here
    -- had ever reached through, which is why solar sizing stopped at the
    -- panel average and could not answer "how many accumulators".
    --
    -- Sent for every electric entity, not only accumulators. A machine's
    -- buffer is small and is not storage anybody plans with, but filtering it
    -- out here would mean this collector deciding what an accumulator is, and
    -- prototype-shape decisions made in Lua are the ones no Rust test can see.
    ok, val = pcall(function()
        local source = entity.electric_energy_source_prototype
        if source == nil then return nil end
        return source.buffer_capacity
    end)
    if ok then record.electric_buffer_capacity = val end
    -- Beacon only: the fraction of a module's effect the receiver gets.
    ok, val = pcall(function() return entity.distribution_effectivity end)
    if ok then record.distribution_effectivity = val end
    -- **Beacon effectiveness is NOT a single scalar in 2.0.** `profile` is an
    -- array of multipliers indexed by how many beacons reach one receiver, so
    -- the second beacon on a machine is worth a different amount from the
    -- first. Sending only `distribution_effectivity` would let a caller
    -- compute a per-beacon number that is right for exactly one beacon count
    -- and silently wrong for every other, which is the shape of defect this
    -- field exists to prevent rather than create.
    --
    -- Keyed `beacon_profile`, not `profile`: bare `profile` on a struct that
    -- describes every prototype in the game says nothing about what it
    -- profiles. `FactorioEntityPrototype` reads the same spelling -- a name
    -- that matches nothing on the Rust struct is dropped by serde SILENTLY,
    -- which has happened twice in this file (`pickupPosition`,
    -- `belt_to_ground_type`), so the pairing is pinned by a test that goes the
    -- whole way into the struct.
    ok, val = pcall(function()
        local profile = entity.profile
        if profile == nil then return nil end
        local multipliers = {}
        for _, multiplier in ipairs(profile) do
            table.insert(multipliers, multiplier)
        end
        -- nil rather than `{}` so an empty profile does not arrive as an empty
        -- *map* -- `helpers.table_to_json` renders an empty Lua table as `{}`.
        if #multipliers == 0 then return nil end
        return multipliers
    end)
    if ok then record.beacon_profile = val end
    ok, val = pcall(function() return entity.resource_category end)
    if ok then record.resource_category = val end
    ok, val = pcall(function()
        local categories = entity.resource_categories
        if categories == nil then return nil end
        local names = {}
        for name, _ in pairs(categories) do
            table.insert(names, name)
        end
        -- Sorted so the order is the data's and not `pairs()`'s; nil rather
        -- than `{}` so an empty set does not arrive as an empty *map*.
        if #names == 0 then return nil end
        table.sort(names)
        return names
    end)
    if ok then record.resource_categories = val end
    -- **The crafting half of exactly the same rule, and the field that says
    -- WHAT a machine crafts.** `crafting_speed` already says a prototype
    -- crafts; nothing on this wire said what. Twelve prototypes declare a
    -- crafting speed and `oil-refinery`, `chemical-plant`, `centrifuge`,
    -- `electromagnetic-plant` and all three assembling machines share one
    -- `entity_type` (`assembling-machine`), so a reader given a recipe's
    -- category -- `oil-processing`, `chemistry`, `centrifuging` -- had no
    -- field,
    -- and no combination of fields, from which the machine that runs it
    -- follows. Naming `oil-refinery` in Rust instead would be the hard-coded
    -- table this project's standing rule forbids, for the same reason the
    -- pole-supply and smelt-rate tables are defects: right for vanilla,
    -- wrong the moment a mod ships another refinery.
    --
    -- `LuaEntityPrototype.crafting_categories` is an **attribute** in 2.1.17
    -- (dictionary `string -> true`), unlike `get_crafting_speed()`,
    -- `get_supply_area_distance()` and `get_max_wire_distance()`, which are
    -- methods with no attribute at all. Read as an attribute, therefore, and
    -- verified against `runtime-api.json` rather than recalled -- reading a
    -- method as an attribute raises, the `pcall` swallows it, and the field
    -- arrives absent for every prototype in silence.
    --
    -- **`{}` rather than nil for an empty set, which is the one place this
    -- deliberately differs from `resource_categories` above.** A crafting
    -- machine that runs no category at all is a thing a mod can ship, and it
    -- has to stay distinguishable from a sender that never looked:
    -- `helpers.table_to_json` renders an empty Lua table as the JSON object
    -- `{}`, and `FactorioEntityPrototype`'s `option_vec_or_empty_map` reads
    -- that as `Some(vec![])` -- *it crafts nothing* -- while an omitted key
    -- reads as `None` -- *the sender did not say*. Every prototype that is not
    -- a crafting machine takes the `None` branch here, because the attribute
    -- is optional and reads nil.
    ok, val = pcall(function()
        local categories = entity.crafting_categories
        if categories == nil then return nil end
        local names = {}
        for name, _ in pairs(categories) do
            table.insert(names, name)
        end
        -- Sorted so the order is the data's and not `pairs()`'s.
        table.sort(names)
        return names
    end)
    if ok then record.crafting_categories = val end
    -- `required_fluid`: uranium ore needs sulfuric acid piped in, and a
    -- character has no pipe.
    if entity.mineable_properties and entity.mineable_properties.minable then
        record.mining_fluid = entity.mineable_properties.required_fluid
    end
    if fluidbox_found then
        record.fluidbox_prototypes = fluidbox_prototypes
    end
    record.collision_mask = collision_mask
    if entity.collision_box then
        record.collision_box = table_properties(entity.collision_box, {"left_top", "right_bottom"}, {left_top = "left_top", right_bottom = "right_bottom"})
    end

    return record
end

-- Which `defines.inventory` index holds this entity type's INPUT, or nil.
--
-- There is no `get_input_inventory()` on `LuaEntity` -- `get_output_inventory`
-- and `get_fuel_inventory` exist and their input counterpart does not -- so
-- the index has to be named per type, exactly as `machine_row` in control.lua
-- does it.
--
-- **Global, not local, because two paths need it.** `serialize_entity` below
-- is the bulk/query description of the world; `rcon_inventory_contents_at` in
-- control.lua is the on-demand reading the planner's buffer model pulls, and
-- it has to resolve the same index the same way or the two would disagree
-- about what a furnace is holding. `require "types"` at the top of control.lua
-- is what makes this visible there.
--
-- Factorio 2.1.17 renamed the crafting-machine inventories: this install's
-- `defines.inventory` has `crafter_input` and has **no** `furnace_source` or
-- `assembling_machine_input` at all (checked against
-- `workspace/factorio-api-docs/runtime-api.json`, not recalled). The fallback
-- is for an older Factorio, and `nil` is a supported outcome -- the caller
-- omits the field rather than passing nil to `get_inventory`.
--
-- `rawget(_G, "defines")` rather than a bare `defines`, because this file is
-- also loaded outside Factorio: `crates/core/tests/botbridge_serialisers.rs`
-- runs these serialisers in a plain Lua 5.4 state, where a bare global read
-- of a table that does not exist is nil and indexing it raises. The tests
-- install a `defines` stub shaped like the real one.
function input_inventory_index(entity_type)
    local defines_table = rawget(_G, "defines")
    if defines_table == nil or defines_table.inventory == nil then
        return nil
    end
    local inventory = defines_table.inventory
    if entity_type == "furnace" or entity_type == "assembling-machine" then
        return inventory.crafter_input or inventory.assembling_machine_input
    elseif entity_type == "lab" then
        return inventory.lab_input
    end
    return nil
end

-- `defines.transport_line`'s own name for a line index, or `unmapped_<n>`.
--
-- Built by inverting `defines.transport_line`, so the names are the game's and
-- not a list written down here that a Factorio version could quietly outgrow.
-- An index this build cannot name still reaches the record, labelled as
-- unresolved, rather than being written as a bare integer nobody can decode
-- later -- the same rule `entity_status_name` below follows.
--
-- The number alone is uninterpretable: line 3 is `left_underground_line` on an
-- underground belt and a different lane on a splitter, so a caller handed the
-- index would have to rebuild this mapping from the entity type and would be
-- guessing at it.
--
-- `rawget(_G, "defines")` for the same reason `input_inventory_index` uses it:
-- this file is loaded outside Factorio by the Rust tests.
local transport_line_names = nil
local function transport_line_name(index)
    if transport_line_names == nil then
        transport_line_names = {}
        local defines_table = rawget(_G, "defines")
        if defines_table ~= nil and defines_table.transport_line ~= nil then
            for name, value in pairs(defines_table.transport_line) do
                transport_line_names[value] = name
            end
        end
    end
    return transport_line_names[index] or ("unmapped_" .. tostring(index))
end

-- `defines.entity_status`'s own name for a status value, or `unmapped_<n>`.
--
-- **The NAME crosses the wire, never the number.** A status id is meaningless
-- without this table: `defines.entity_status` is an enum whose numbering is a
-- Factorio implementation detail, so an archive holding `12` would need the
-- exact game version's table to be readable at all, and a version bump could
-- silently make it mean something else. A value this build cannot name is
-- written as `unmapped_<n>` rather than dropped -- the same rule
-- `transport_line_name` above follows.
--
-- Built lazily by inverting the game's own table rather than from a list
-- written down here, which a Factorio version could quietly outgrow: 2.1.17
-- has 72 members.
--
-- `rawget(_G, "defines")` for the same reason `input_inventory_index` uses it:
-- this file is loaded outside Factorio by `crates/core/tests/
-- botbridge_serialisers.rs`, where a bare global read of a missing table is
-- nil and indexing it raises.
local entity_status_names = nil
function entity_status_name(status)
    if entity_status_names == nil then
        entity_status_names = {}
        local defines_table = rawget(_G, "defines")
        if defines_table ~= nil and defines_table.entity_status ~= nil then
            for name, value in pairs(defines_table.entity_status) do
                entity_status_names[value] = name
            end
        end
    end
    return entity_status_names[status] or ("unmapped_" .. tostring(status))
end

-- `opts.omit_inventories` -- IDENTITY AND GEOMETRY IN BULK, CONTENTS ON DEMAND.
--
-- This function serves two very different callers. The RCON queries
-- (`rcon_find_entities_filtered`, `find_entities_in_radius`, `world_snapshot`,
-- `rcon_place_entity`'s reply) answer a question somebody asked about a handful
-- of entities, and scripts genuinely read `output_inventory` off those --
-- `scripts/furnace_run.lua`, `two_row_smelter_live.lua` and others count plates
-- that way. They pass nothing and get the full record, unchanged.
--
-- `writeout_entities` is the other caller, and it is not a query: it ships
-- EVERY entity of EVERY chunk through a line-oriented text protocol on stdout,
-- once per chunk, for the whole map. Attaching each machine's
-- `get_output_inventory()` and `get_fuel_inventory()` contents there is cheap
-- on a fresh map -- measured at exactly **0 bytes across 50,256 records** in
-- `workspace/server-log.txt`, because a map of trees and ore has no machine to
-- own an inventory -- and is the entire world state, item by item, on a
-- finished base.
--
-- **Nothing reads them off a bulk-ingested entity.** `EntityGraph::add` clones
-- the whole entity into `entity_tree`, so the fields are stored, but they are
-- a snapshot taken when the chunk was generated and `add` refuses to re-add
-- over an occupied position -- so what is stored is permanently stale and no
-- caller in `crates/planner`, `crates/executor` or `crates/server` looks at it.
-- The planner's buffer model reads `FactorioSurface::inventories`, which is
-- filled only by `observe_inventories` from the RCON reply to
-- `inventory_contents_at`. That is the on-demand path, it already exists, and
-- it is the one the owner's rule names:
--
--   "For an endgame base we cannot model each individual item produced, we'd
--    need to simplify using flow rates too."
--
-- See docs/superpowers/notes/2026-09-06-identity-in-bulk-contents-on-demand.md.
function serialize_entity(entity, opts)
    local record = table_properties(entity, {"name", "direction", "type", "position", "drop_position"}, {type = "entity_type", drop_position = "drop_position"})
    -- WHICH SURFACE, BY NAME. Carried, not yet used.
    --
    -- Space Age is enabled in this workspace, so a position alone does not
    -- name a place: (10, 10) exists on Nauvis, on Vulcanus and on every
    -- orbital platform. `LuaSurface.name` is unique among surfaces, while
    -- `LuaSurface.index` is reused after a surface is deleted, so the name is
    -- the identity that survives into a record somebody reads later.
    --
    -- Rust reads it as `FactorioEntity::surface: Option<SurfaceId>` with
    -- `#[serde(default)]`, the same treatment `underground_half` got, so every
    -- archived run and every world dump written before today still loads --
    -- lacking the field, which reads as "the sender did not say" rather than
    -- as a claim about Nauvis. Nothing keys on it yet: the entity graph is
    -- still position-only and would alias two surfaces into one, which is what
    -- `on_chunk_generated`'s guard prevents.
    -- See docs/superpowers/notes/2026-09-06-surfaces-survey.md.
    record.surface = entity.surface and entity.surface.name or nil
    record.bounding_box = table_properties(entity.bounding_box, {"left_top", "right_bottom"}, {left_top = "left_top", right_bottom = "right_bottom"})
    -- WHAT THE MACHINE IS DOING RIGHT NOW.
    --
    -- The three inventory reads below say what a machine HOLDS. None of them
    -- says whether it is running, and that absence is the dominant term in the
    -- flow graph's error against a real base: measured at +16% to +23% on the
    -- world-record save, unbounded, against coverage at 1.6% and modules at 0%.
    -- Half of it is not derivable from the graph at any price -- 194 drills on
    -- that base were sitting at `waiting_for_space_in_destination`, which is a
    -- fact about back-pressure downstream that no ingredient balance can see.
    -- See docs/superpowers/notes/2026-09-07-a-machine-standing-still.md.
    --
    -- **An ATTRIBUTE, not a method.** `LuaEntity.status` is `optional: true`
    -- with `subclasses: None` in this install's `runtime-api.json` (2.1.17),
    -- so it is a plain read and it is safe on any entity. That was checked
    -- rather than assumed, because reading a *method* as an attribute yields a
    -- function rather than raising, and the field then goes missing in silence
    -- -- which is how `crafting_speed` arrived nil for 1,028 prototypes.
    --
    -- **`nil` and a name are different answers.** A tree, a chest and a belt
    -- have no status concept and get no key at all (`None` on the Rust side,
    -- "the sender did not say"); a machine that is stopped has a NAME for
    -- being stopped -- `no_ingredients`, `no_power`,
    -- `waiting_for_space_in_destination` -- and that name is the measurement.
    -- Defaulting the absent case to `working` would invent a duty cycle.
    --
    -- **Outside the `omit_inventories` guard, deliberately.** This is one
    -- small string, not an item-by-item inventory, and the bulk writeout is
    -- the *only* path that fills the world model a dumped world is built from
    -- -- gating it there would leave the duty cycle unmeasurable in exactly
    -- the artefact the question is asked of.
    local status = entity.status
    if status ~= nil then
        record.status = entity_status_name(status)
    end
    if not (opts and opts.omit_inventories) then
        local output_inventory = entity.get_output_inventory()
        if output_inventory ~= nil then
            record.output_inventory = output_inventory.get_contents()
        end
        local fuel_inventory = entity.get_fuel_inventory()
        if fuel_inventory ~= nil then
            record.fuel_inventory = fuel_inventory.get_contents()
        end
        -- WHAT THE MACHINE WAS GIVEN AND HAS NOT TURNED INTO ANYTHING YET.
        --
        -- The two reads above answer "what has it made" and "what is it
        -- burning", and between them they leave a hole a day was spent in: a
        -- furnace holding ore it is not smelting and a furnace no ore ever
        -- reached serialise IDENTICALLY -- `output_inventory` empty,
        -- `fuel_inventory` whatever, and nothing at all about the ore. A run
        -- that mined 46 ore and got 17 plates could not say where the other
        -- 29 went, and eliminating ore exhaustion, arm starvation and a full
        -- belt by measurement still left the question open, because the one
        -- inventory that would have answered it was never sent.
        --
        -- **`nil` and empty are different answers and must stay different.**
        -- A belt has no input inventory at all and gets no key (`None` on the
        -- Rust side); a furnace standing empty gets `{}`, which
        -- `option_vec_or_empty_map` reads as `Some(empty)`. Collapsing those
        -- would rebuild the same ambiguity one layer up.
        local input_index = input_inventory_index(entity.type)
        if input_index ~= nil then
            local input_inventory = entity.get_inventory(input_index)
            if input_inventory ~= nil then
                record.input_inventory = input_inventory.get_contents()
            end
        end
        -- WHAT IS RIDING ON THE BELT, LANE BY LANE.
        --
        -- The three reads above describe machines and say nothing at all about
        -- the thing between them. `LuaTransportLine::get_contents()` has now
        -- blocked four separate questions here, the fourth a diagnosis: a run
        -- mined 46 ore, made 17 plates and stranded 29, and neither a full belt
        -- nor an empty one could be ruled in or out because the belt's contents
        -- had never left the game.
        --
        -- **Lanes, not one number.** A `transport-belt` has two, an
        -- `underground-belt` four and a `splitter` eight, and which lane an
        -- item is on is exactly what decides whether an arm can take it -- an
        -- inserter drops on the FAR lane and a side-load arrives on the NEAR
        -- one, and this project has already measured a block where getting
        -- that backwards put ore and coal on one lane and produced a single
        -- plate.
        --
        -- **Counts, not positions.** `get_detailed_contents()` would give every
        -- item's position along the line; the owner's ruling at scale is the
        -- direction and what types of items are on it, so this is the
        -- aggregated `get_contents()` and nothing finer.
        --
        -- Guarded on `get_max_transport_line_index`, which is declared for
        -- `TransportBeltConnectable` only: reading it off a furnace raises,
        -- exactly like the `crafting_progress` read that once took a live run
        -- down from inside a sampler. A non-belt gets no key rather than an
        -- empty list, so "this belt is running empty" stays a different answer
        -- from "this is not a belt".
        if entity.get_max_transport_line_index ~= nil then
            local ok, max_index = pcall(function()
                return entity.get_max_transport_line_index()
            end)
            if ok and max_index ~= nil and max_index > 0 then
                local lines = {}
                for index = 1, max_index do
                    local line = entity.get_transport_line(index)
                    table.insert(lines, {
                        line = transport_line_name(index),
                        contents = line.get_contents(),
                    })
                end
                record.transport_lines = lines
            end
        end
    end

    if entity.type == "resource" then
        record.amount = entity.amount
    elseif entity.type == "inserter" then
        -- snake_case: `FactorioEntity.pickup_position` is the name the Rust
        -- side reads (`rename_all = "snake_case"`). It is an `Option`, so the
        -- camelCase spelling this used to emit did not fail loudly -- every
        -- inserter simply arrived with `pickup_position: None`, and
        -- `EntityGraph::connect` (crates/core/src/graph/entity_graph.rs) then
        -- silently never connected an inserter to what it picks up from.
        record.pickup_position = entity.pickup_position
    elseif entity.type == "entity-ghost" then
        record.ghost_name = entity.ghost_name
        record.ghost_type = entity.ghost_type
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
    elseif entity.type == "underground-belt" then
        -- Task 6 (2026-09-05): read back which half of an underground-belt
        -- pair this entity is. `FactorioEntity::underground_half` travels
        -- INTO the game via `rcon_place_entity`'s 5th argument (see
        -- control.lua), but nothing serialized it back OUT until now -- task
        -- 5's report named this gap explicitly. `entity.belt_to_ground_type`
        -- is Factorio's own field, "input" or "output".
        --
        -- KEYED AS `underground_half`, NOT `belt_to_ground_type` -- confirmed
        -- the hard way. `find_entities_in_radius`/`find_entities_filtered`
        -- deserialize this JSON into the strongly-typed Rust
        -- `FactorioEntity` (crates/core/src/factorio/rcon.rs), whose field
        -- for exactly this is `underground_half: Option<UndergroundHalf>`
        -- (crates/core/src/types.rs); a first attempt emitted
        -- `belt_to_ground_type` here, which matches nothing on that struct,
        -- so serde silently dropped it and every live read came back `nil`
        -- even though a raw `remote.call("botbridge","find_entities_filtered",
        -- ...)` showed the field present and correct. `UndergroundHalf`'s
        -- `#[serde(rename_all = "snake_case")]` already renders as
        -- "input"/"output", which is what Factorio's own field returns, so
        -- no value mapping is needed -- only the key.
        record.underground_half = entity.belt_to_ground_type
    end
    return record
end

-- Factorio 2.0 renamed every collision layer, dropping the `-layer` suffix:
-- `player-layer` became `player` (see `prototypes.collision_layer` in a live
-- 2.1 game, and the `LuaTile::collides_with` example in
-- `workspace/factorio-api-docs/runtime-api.json`). The old name is not ignored,
-- it *raises*: "Unknown collision-layer name: player-layer". That took
-- `find_tiles_filtered` down entirely on 2.1 -- the whole RCON call returned an
-- error string instead of JSON -- and nothing noticed, because no fixture had
-- ever been captured for `FactorioTile`.
function serialize_tile(tile)
    local record = table_properties(tile, {"name", "position"})
    record.player_collidable = tile.collides_with('player')
    -- The surface this tile is on, by NAME. See serialize_entity.
    record.surface = tile.surface and tile.surface.name or nil
    -- WHICH FLUID AN OFFSHORE PUMP HERE WOULD DRAW.
    --
    -- `control.lua`'s `tile_fluid` walks visible -> hidden -> double hidden,
    -- the order `LuaEntity::get_fluid_source_fluid` documents. This path is
    -- per-tile and low volume (`find_tiles_filtered`), so it always does the
    -- full walk -- unlike the bulk chunk writeout, which buys the same
    -- correctness with one `count_tiles_filtered` per chunk.
    --
    -- Three states on the wire: `{kind="yields", fluid=...}`, `{kind="dry"}`,
    -- and -- only if the read raises -- nothing at all, which the Rust side
    -- defaults to `unknown`. "Gives nothing" is an answer; "we did not look"
    -- is not, and an `Option<String>` would have merged them.
    local ok, fluid = pcall(function() return tile_fluid(tile) end)
    if ok then
        if fluid then
            record.fluid = {kind = "yields", fluid = fluid}
        else
            record.fluid = {kind = "dry"}
        end
    end
    return record
end

function table_properties(tbl, props, replacements)
    local filtered = {}
    for _, v in ipairs(props) do
        local target_v = v
        if replacements ~= nil and replacements[v] ~= nil then
            target_v = replacements[v]
        end
        -- Use pcall to safely access properties that might not exist in Factorio 2.0
        local ok, val = pcall(function() return tbl[v] end)
        if ok then
            filtered[target_v] = val
        end
    end
    return filtered
end

--- The daylight curve of one surface, which is **surface state and not
--- prototype data**.
---
--- A `solar-panel` prototype carries its **noon** figure only
--- (`get_max_energy_production()` = 60 kW in vanilla 2.1.17). What a day
--- averages is that figure times the fraction of the day the sun is up, and
--- every term of that fraction is on `LuaSurface`, not on any prototype. So a
--- planner that credits solar at nameplate sizes a base that is dead at night,
--- and one that refuses solar entirely -- which is what this project did until
--- this record existed -- cannot plan a solar base at all.
---
--- **Every field here is an ATTRIBUTE on `LuaSurface` in 2.1.17**, checked
--- against this install's `runtime-api.json` rather than recalled. That matters
--- because the pair `energy_usage` (attribute) / `get_max_energy_production()`
--- (method) in `serialize_entity_prototype` above landed one each way, and
--- reading a method as an attribute raises, `pcall` swallows it, and the field
--- arrives nil with nothing saying it should not have. All ten of the names
--- below are `read_type`/`write_type` attributes with `optional: false`; there
--- is no `get_dawn()` and no `get_ticks_per_day()` -- doclint-allow: names that
--- deliberately do NOT exist, and their absence is exactly the claim.
---
--- # What the four boundaries mean
---
--- `daytime` runs `[0, 1)` and **0 is noon**, so the sunlit half straddles the
--- wrap. In vanilla order the day runs `dusk` (0.25) -> `evening` (0.45) ->
--- `morning` (0.55) -> `dawn` (0.75): full sun from `dawn` through 0 to `dusk`,
--- a linear fade `dusk` -> `evening`, night from `evening` to `morning`, a
--- linear rise `morning` -> `dawn`. `crates/planner/src/state.rs` integrates
--- that trapezoid; nothing here interprets it, because a collector that
--- editorialises is a second copy of the model.
---
--- `always_day` and `freeze_daytime` are sent because either one makes the
--- integral wrong in a way no boundary would reveal: an `always_day` surface
--- produces the day figure around the clock, and a frozen one produces
--- whatever `daytime` was stopped at, forever.
---
--- `solar_power_multiplier` is the surface's own scaling (1 on Nauvis, and not
--- 1 everywhere -- it is exactly the knob that makes a solar array a different
--- size on a different planet).
function serialize_surface_daylight(surface)
    local record = table_properties(surface, {
        "ticks_per_day",
        "dawn",
        "dusk",
        "evening",
        "morning",
        "daytime",
        "solar_power_multiplier",
        "always_day",
        "freeze_daytime",
    })
    -- Named, because a curve is only about the surface it was read from and a
    -- record that cannot say which one is not a fact about anything.
    record.surface = surface.name
    return record
end

--- One row of the surface census: what a surface IS, not what is on it.
---
--- **This exists so that "this save has no other surfaces" stops looking like
--- "we never looked".** The Nauvis guard in `control.lua`'s
--- `on_chunk_generated` drops every non-Nauvis chunk and says so, which is
--- honest about each chunk it refuses -- but nothing anywhere enumerated
--- `game.surfaces`, so a world model holding one surface was equally
--- consistent with a single-planet save and with a five-planet save whose
--- other four were never mentioned. The world-record base was loaded and read
--- as 39,237 entities all on `nauvis`; that number could not distinguish the
--- two readings either.
---
--- Deliberately three fields and no chunk counts, entities or tiles: this is
--- the census, not the ingest. Ingesting a second surface is gated on moving
--- the game-global fields off `FactorioSurface` and is a much larger task
--- (`FactorioWorld::insert_surface` refuses the second surface by name).
---
--- `planet` is the **planet's own name** (`nauvis`, `vulcanus`, `gleba`), from
--- `LuaSurface.planet`, an optional attribute returning a `LuaPlanet` --
--- verified against `runtime-api.json`, not recalled. `nil` is the honest
--- answer for a surface that is not a planet at all, which is what a space
--- platform is, and that is exactly the distinction a caller wants: a platform
--- moves and a planet does not.
---
--- `index` is `LuaSurface.index`, and it is why `game.surfaces[1]` is not the
--- same claim as `game.surfaces['nauvis']` -- the numeric index says
--- "whichever surface was made first".
function serialize_surface(surface)
    local record = {
        name = surface.name,
        index = surface.index,
    }
    -- `pcall` because `planet` is optional and a Factorio without it must
    -- report a surface with no planet rather than failing the whole census.
    local ok, planet = pcall(function() return surface.planet end)
    if ok and planet ~= nil then
        record.planet = planet.name
    end
    return record
end
