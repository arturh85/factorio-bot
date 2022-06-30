--- Types
--
-- NOTE: not available in scripts, only for documentation purposes
-- @module types

--- FactorioTile
FactorioTile = {
    name = '', -- string
    player_collidable = false, -- boolean
    position = nil, -- `Position`
}

--- FactorioTechnology
FactorioTechnology = {
   name = '', -- string
   enabled = false, -- boolean
   upgrade = false, -- boolean
   prerequisites = nil, -- {string}
   researched = false, -- boolean
   research_unit_ingredients = nil, -- {`FactorioIngredient`}
   research_unit_count = 0, -- number
   research_unit_energy = 0, -- number
   order = '', -- string
   level = 0, -- number
   valid = false, -- boolean
}

--- FactorioForce
FactorioForce = {
  name = '', -- string
  force_id = 0, -- number
  current_research = '', -- string
  research_progress = 0, -- number
  technologies = nil, -- {`FactorioTechnology`}
}

--- InventoryResponse
InventoryResponse = {
   name = '', -- string
   position = nil, -- `Position`
   output_inventory = nil, -- {[string]=int,...}
   fuel_inventory = nil, -- {[string]=int,...}
}

--- FactorioRecipe
FactorioRecipe = {
   name = '', -- string
   valid = false, -- boolean
   enabled = false, -- boolean
   category = '', -- string
   ingredients = nil, -- {`FactorioIngredient`}
   products = nil, -- {`FactorioProduct`}
   hidden = false, -- boolean
   energy = 0, -- number
   order = '', -- string
   group = '', -- string
   subgroup = '', -- string
}

--- FactorioIngredient
FactorioIngredient = {
   name = '', --string
   ingredient_type = '', --string
   amount = 0, -- number
}

--- FactorioProduct
FactorioProduct = {
    name = '', -- string
    product_type = '', -- string
    amount = 0, -- number
    probability = 0, -- number
}

--- FactorioPlayer
FactorioPlayer = {
    player_id = 0, -- number
    position = nil, -- `Position`
    main_inventory = nil, -- {[string]=int,...}
    build_distance = 0, -- number
    reach_distance = 0, -- number
    drop_item_distance = 0, -- number
    item_pickup_distance = 0, -- number
    loot_pickup_distance = 0, -- number
    resource_reach_distance = 0, -- number
}

--- ChunkPosition
ChunkPosition = {
    x = 0, -- number
    y = 0, -- number
}

--- Position
Position = {
    x = 0, -- number
    y = 0, -- number
}

--- Rect
Rect = {
    left_top = nil, -- `Position`
    right_bottom = nil, -- `Position`
}

--- FactorioGraphic
FactorioGraphic = {
    entity_name = '', -- string
    image_path = '', -- string
    width = 0, -- number
    height = 0, -- number
}

--- FactorioEntity
FactorioEntity = {
    name = '', -- string
    entity_type = '', -- string
    position = nil, -- `Position`
    bounding_box = nil, -- `Rect`
    direction = 0, -- number
    drop_position = nil, -- `Position`
    pickup_position = nil, -- `Position`
    output_inventory = nil, -- {[string]=int,...}
    fuel_inventory = nil, -- {[string]=int,...}
    amount = 0, -- number
    recipe = '', -- string
    ghost_name = '', -- string
    ghost_type = '', -- string
}

--- FactorioEntityPrototype
FactorioEntityPrototype = {
    name = '', -- string
    entity_type = '', -- string
    collision_mask = nil, -- {string}
    collision_box = nil, -- `Rect`
    mine_result = nil, -- {[string]=int,...}
    mining_time = 0, -- number
    mining_speed = 0, -- number
    crafting_speed = 0, -- number
    max_underground_distance = 0, -- number
    fluidbox_prototypes = nil, -- {`FactorioFluidBoxPrototype`}
}

--- FactorioItemPrototype
FactorioItemPrototype = {
    name = '', -- string
    item_type = '', -- string
    stack_size = 0, -- number
    fuel_value = 0, -- number
    place_result = '', -- string
    group = '', -- string
    subgroup = '', -- string
}

--- FactorioFluidBoxPrototype
FactorioFluidBoxPrototype = {
    pipe_connections = nil, -- {`FactorioFluidBoxConnection`}
    production_type = '', -- string
}

--- FactorioFluidBoxConnection
FactorioFluidBoxConnection = {
    max_underground_distance = 0, -- number
    connection_type = '', -- string
    positions = nil, -- {Position}
}

--- FactorioBlueprintInfo
FactorioBlueprintInfo = {
    label = '', -- string
    blueprint = '', -- string
    width = 0, -- number
    height = 0, --number
    rect = nil, -- `Rect`
    data = nil, -- data
}

--- InventoryLocation
InventoryLocation = {
    entity_name = '', -- string
    position = nil, -- {Position}
    inventory_type = 0, -- number
}

--- EntityPlacement
EntityPlacement = {
    item_name = '', -- string
    position = nil, -- {Position}
    direction = 0, -- number
}

--- PositionRadius
PositionRadius = {
    position = nil, -- {Position}
    radius = 0, -- number
}