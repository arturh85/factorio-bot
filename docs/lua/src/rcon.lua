--- RCON interface
-- methods for sending rcon commands to running factorio instance
--
-- @module rcon

local rcon = {}

--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @param inventories table of x/y positions to check
-- @return table of inventory contents
function rcon.inventoryContentsAt(inventories)
end

--- Removes an item from an inventory
-- Sends /silent-command remote.call('remove_from_inventory', ...)
-- @int player_id id of plaer
-- @string entity_name name entity to remove
-- @param position  x/y position table of inventory
-- @string inventory_type which type of inventory to remove from
-- @string item_name which item to remove
-- @int item_count how many items to remove
function rcon.removeFromInventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end

--- Places an item by a player
-- Sends /silent-command remote.call('place_entity', ...)
-- @int player_id id of plaer
-- @string name name of item to craft
-- @param position  x/y position table
-- @int direction direction of placed entity
-- @return FactorioEntity object
function rcon.placeEntity(player_id, name, position, direction)
end

--- Mine a resource with player
-- Sends /silent-command remote.call('action_start_mining', ...)
-- @int player_id id of player to give the item to
-- @string name name of resource to mine
-- @param position x/y position
-- @int count how many to mine
function rcon.mine(player_id, name, position, count)
end

--- Move a player to a different position
-- Sends /silent-command remote.call('action_start_walk_waypoints', ...)
-- @int player_id id of player to give the item to
-- @param position x/y position
-- @int radius radius
function rcon.move(player_id, position, radius)
end

--- Craft an item with player
-- Sends /silent-command remote.call('action_start_crafting', ...)
-- @int player_id id of plaer
-- @string name name of item to craft
-- @int count how many to craft
function rcon.craft(player_id, name, count)
end

--- CHEATs all research
-- Sends /silent-command remote.call('cheat_all_technologies')
function rcon.cheatAllTechnologies()
end

--- places a whole blueprint
-- Sends /silent-command remote.call('place_blueprint', ...)
-- @int player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position x/y position
-- @int direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
-- @bool only_ghosts only places ghost version of entities
-- @param helper_player_ids array of player ids which may help
function rcon.placeBlueprint(player_id, blueprint, position, direction, force_build, only_ghosts, helper_player_ids)
end

--- find entities at given position/radius with optional filters
-- Sends /silent-command remote.call('find_entities_filtered', ...)
-- @param search_center x/y position
-- @int radius searches in circular radius around search_center
-- @string[opt] search_name name of entity to find
-- @string[opt] search_type type of entity to find
-- @return table list of FactorioEntity objects
function rcon.findEntitiesInRadius(search_center, radius, search_name, search_type)
end

--- print given message on the server
-- Sends /c print(message)
-- @string message
function rcon.print(message)
end

--- CHEATs given item
-- Sends /silent-command remote.call('cheat_item', ...)
-- @int player_id id of player to give the item to
-- @string name item name
-- @int count how many items to give player
function rcon.cheatItem(player_id, name, count)
end

--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('revive_ghost', ...)
-- @int player_id id of player to give the item to
-- @string name name of entity to revive
-- @param position x/y position
function rcon.reviveGhost(player_id, name, position)
end

--- CHEATs research
-- Sends /silent-command remote.call('cheat_technology', technology_name)
-- @string technology_name name of technology to CHEAT
function rcon.cheatTechnology(technology_name)
end

--- adds research to queue
-- Sends /silent-command remote.call('add_research', technology_name)
-- @string technology_name name of technology to research
function rcon.addResearch(technology_name)
end

--- CHEATs a whole blueprint
-- Sends /silent-command remote.call('cheat_blueprint', ...)
-- @int player_id id of player to give the item to
-- @string blueprint blueprint string
-- @param position x/y position
-- @int direction rotates the blueprint in given direction
-- @bool force_build forces the build even if other entities needs to be removed first
function rcon.cheatBlueprint(player_id, blueprint, position, direction, force_build)
end

--- Inserts an item into an inventory
-- Sends /silent-command remote.call('insert_to_inventory', ...)
-- @int player_id id of plaer
-- @string entity_name name entity to insert
-- @param position  x/y position table of inventory
-- @string inventory_type which type of inventory to place in
-- @string item_name which item to insert
-- @int item_count how many items to insert
function rcon.insertToInventory(player_id, entity_name, position, inventory_type, item_name, item_count)
end

return rcon
