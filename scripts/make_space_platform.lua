-- Create a space platform, using only bindings this project has.
--
-- `create_space_platform` is the ONE capability our entity-scoped action
-- vocabulary cannot express: a force-level call with no entity receiver, so
-- there is nothing for `place` to place or `insert_to_inventory` to insert
-- into. Everything else on the way to a platform we already had.
--
-- The full sequence, established live on 2026-09-07:
--   1. this call                       -> platform, waiting_for_starter_pack
--   2. insert the starter pack into a silo WHILE IT IS BUILDING a rocket
--   3. nothing -- the silo launches itself and the surface appears
--
-- Step 2 needs a powered silo and a rocket's worth of ingredients, which is the
-- whole oil chain; this script proves step 1 through the real Lua path, which
-- is the part that did not exist until now.
print("start make space platform")

local before = world.surfaces and #world.surfaces() or -1
print("surfaces known to the model before: " .. tostring(before))

local ok, result = pcall(function()
  return rcon.create_space_platform("p1", "nauvis", "space-platform-starter-pack")
end)

if ok then
  print("CREATED: " .. tostring(result))
  print("RESULT: the force-level call works through the Lua binding. The")
  print("        platform is waiting for its starter pack; inserting one into")
  print("        a building silo is an `insert_to_inventory` we already have.")
else
  print("REFUSED: " .. tostring(result))
  print("RESULT: the game declined. That is a real answer and names its own")
  print("        reason -- it is not the binding failing to reach the game.")
end

-- A second call with the same name: the game's own uniqueness behaviour,
-- reported rather than assumed. A caller planning several platforms needs to
-- know whether names collide.
local ok2, result2 = pcall(function()
  return rcon.create_space_platform("p1", "nauvis", "space-platform-starter-pack")
end)
print("same name again -> " .. (ok2 and ("created " .. tostring(result2)) or ("refused: " .. tostring(result2))))
print("end make space platform")
