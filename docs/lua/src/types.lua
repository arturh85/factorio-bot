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
   research_unit_count = 0, -- int
   research_unit_energy = 0, -- int
   order = '', -- string
   level = 0, -- number
   valid = false, -- boolean
}

--- FactorioForce
FactorioForce = {
  name = '', -- string
  force_id = 0, -- int
  current_research = '', -- string
  research_progress = 0, -- int
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
   energy = 0, -- int
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
    probability = 0, -- int
}

--- FactorioPlayer
FactorioPlayer = {
    player_id = 0, -- int
    position = nil, -- `Position`
    main_inventory = nil, -- {[string]=int,...}
    build_distance = 0, -- int
    reach_distance = 0, -- int
    drop_item_distance = 0, -- int
    item_pickup_distance = 0, -- int
    loot_pickup_distance = 0, -- int
    resource_reach_distance = 0, -- int
}

--- ChunkPosition
ChunkPosition = {
    x = 0, -- int
    y = 0, -- int
}

--- Position
Position = {
    x = 0, -- int
    y = 0, -- int
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
    height = 0, -- int
}

--- FactorioEntity
FactorioEntity = {
    name = '', -- string
    entity_type = '', -- string
    position = nil, -- `Position`
    bounding_box = nil, -- `Rect`
    direction = 0, -- int
    drop_position = nil, -- `Position`
    pickup_position = nil, -- `Position`
    output_inventory = nil, -- {[string]=int,...}
    fuel_inventory = nil, -- {[string]=int,...}
    amount = 0, -- int
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
    mining_time = 0, -- int
    mining_speed = 0, -- int
    crafting_speed = 0, -- int
    max_underground_distance = 0, -- int
    fluidbox_prototypes = nil, -- {`FactorioFluidBoxPrototype`}
}

--- FactorioItemPrototype
FactorioItemPrototype = {
    name = '', -- string
    item_type = '', -- string
    stack_size = 0, -- number
    fuel_value = 0, -- int
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
