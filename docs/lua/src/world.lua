--- Factorio World
-- Internal representation of a Factorio world.
--
-- @module world

local world = {}

--- find non-blocked rectangle with given resource
-- The ...
-- @string ore_name name of item to craft
-- @int width name of item to craft
-- @int height name of item to craft
-- @param near x/y table
-- @return table FactorioPlayer object
function world.findFreeResourceRect(ore_name, width, height, near)
end

--- counts how many of a given item the player has
-- The ...
-- @int player_id id of player
-- @string item_name name of item
-- @return table list of FactorioEntity objects
function world.inventory(player_id, item_name)
end

--- lookup player by id
-- The player id will start at 1 and increment.
-- @int player_id id of player
-- @return table FactorioPlayer object
function world.player(player_id)
end

--- find entities at given position/radius with optional filters
-- Sends 
-- @param search_center x/y position table 
-- @int radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return table list of FactorioEntity objects
function world.findEntitiesInRadius(search_center, radius, search_name, search_type)
end

--- lookup recipe by name
-- The name as defined by https://wiki.factorio.com/Materials_and_recipes
-- @string name name of item to craft
-- @return table FactorioRecipe object
function world.recipe(name)
end

return world
