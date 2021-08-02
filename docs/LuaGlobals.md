Lua Plan Globals
================

world
-----

Methods 
- world.recipe("inserter") -> FactorioRecipe 
- world.findFreeResourceRect("iron-ore", 4, 4, {x=0, y=0}) -> Option<Rect> 

plan
-----

Methods 
- plan.groupStart("Mine with Bots")
  - opens a new sync group with given label 
- plan.mine(playerId, {x=42,y=-2}, "rock-huge", 1)
  - mines given entity. automatically adds walk if too far away
- plan.walk(playerId, {x=21,y=-22}, 10)
  - walk to position with given radius
- plan.place(playerId, {x=21,y=-22}, {name="inserter", position={x=0,y=1}, direction=0)
  - walk to position with given radius
- plan.groupEnd() 

rcon
-----

Methods 
- rcon.findEntitiesInRadius({x=21,y=-22}, 500, name, entityName)  -> Vec<FactorioEntity>
- rcon.print("hello world!") 
- rcon.addResearch("automation") 
- rcon.placeBlueprint(playerId, "blueprint", {x=21,y=-22}, 0, true, false, {2,3,4})  -> Vec<FactorioEntity>
- rcon.reviveGhost(playerId, "inserter", {x=21,y=-22} -> FactorioEntity
- rcon.move(playerId, {x=21,y=-22}, 20) 
- rcon.mine(playerId, "coal", {x=21,y=-22}, 3) 
- rcon.craft(playerId, "inserter", 4) 
- rcon.inventoryContentsAt({{x=1,y=2,name="burner-mining-drill"},{x=3,y=4,name="iron-chest"}}) -> ? 
- rcon.placeEntity(playerId, "inserter", {x=21,y=-22}, 0)  -> FactorioEntity
- rcon.insertToInventory(playerId, "burner-mining-drill", {x=21,y=-22}, 1, "coal", 5)
- rcon.removeFromInventory(playerId, "iron-chest", {x=21,y=-22}, 1, "iron-plate", 5) 
    
- rcon.cheatBlueprint(playerId, "blueprint", {x=21,y=-22}, 0, true)  -> Vec<FactorioEntity>
- rcon.cheatItem(playerId, "inserter", 20) 
- rcon.cheatTechnology("automation") 
- rcon.cheatAllTechnologies() 
