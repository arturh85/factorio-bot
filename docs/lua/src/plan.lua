--- Plan Builder
-- Internally holds a graph of Task Nodes which can be grown by using the methods.  
-- @module plan

local plan = {}

--- adds a WALK node to graph
-- @int player_id id of player
-- @param position x/y position table 
-- @int radius how close to walk to
function plan.walk(player_id, position, radius)
end

--- adds a GROUP END node to graph
function plan.groupEnd()
end

--- adds a PLACE node to graph
-- If required first a WALK node is inserted to walk near the item to place 
-- @int player_id id of player
-- @param position x/y position table 
-- @string name name of item to place
-- @return FactorioEntity table
function plan.place(player_id, position, name)
end

--- adds a GROUP START node to graph
-- Groups are used to synchronize bots so their tasks are completed before the next group is started.
-- Should be closed ith groupEnd. 
-- @string label label of group
function plan.groupStart(label)
end

--- adds a MINE node to graph
-- If required first a WALK node is inserted to walk near the item to mine 
-- @int player_id id of player
-- @param position x/y position table
-- @string name name of item to mine
-- @int count how many items to mine
function plan.mine(player_id, position, name, count)
end

return plan
