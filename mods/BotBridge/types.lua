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
            "probability", "independent_probability", "shared_probability"
        },
        {type = "product_type"}
    )
end

function serialize_ingredient(ingredient)
    return table_properties(ingredient, {"name", "type", "amount"}, {type = "ingredient_type"})
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
        {"name", "index", "research_progress", "manual_mining_speed_modifier"},
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
    local unlocked = {}
    local ok, effects = pcall(function() return technology.prototype.effects end)
    if ok and effects ~= nil then
        for _, effect in pairs(effects) do
            if effect.type == "unlock-recipe" and effect.recipe ~= nil then
                table.insert(unlocked, effect.recipe)
            end
        end
    end
    record.unlocked_recipes = unlocked

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

function serialize_entity(entity)
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
    local output_inventory = entity.get_output_inventory()
    if output_inventory ~= nil then
        record.output_inventory = output_inventory.get_contents()
    end
    local fuel_inventory = entity.get_fuel_inventory()
    if fuel_inventory ~= nil then
        record.fuel_inventory = fuel_inventory.get_contents()
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
