export type PlaceEntity = {
 position: Position;
 direction: number;
}
export enum InventoryType {
 chest_or_fuel = 1,
 furnace_source = 2, // or lab input apparently
 furnace_result = 3,
}
export enum Direction {
 north,
 northeast,
 east,
 southeast,
 south,
 southwest,
 west,
 northwest,
}
export enum Entities {
 rockHuge = "rock-huge",
 rockBig = "rock-big",
 water = "water",
 coal = "coal",
 stone = "stone",
 ironOre = "iron-ore",
 copperOre = "copper-ore",
 burnerMiningDrill = "burner-mining-drill",
 stoneFurnace = "stone-furnace",
 offshorePump = "offshore-pump",
 ironPlate = "iron-plate",
 copperPlate = "copper-plate",
 stoneBrick = "stone-brick",
 ironChest = "iron-chest",
 steamEngine = "steam-engine",
 boiler = "boiler",
 splitter = "splitter",
 smallElectricPole = "small-electric-pole",
 pipe = "pipe",
 pipeToGround = "pipe-to-ground",
 transportBelt = "transport-belt",
 undergroundBelt = "underground-belt",
 lab = "lab",
 automationSciencePack = "automation-science-pack",
 wood = "wood",
 assemblingMachine1 = "assembling-machine-1",
 inserter = "inserter",
 ironGearWheel = "iron-gear-wheel",
 electricMiningDrill = "electric-mining-drill",
}
export enum EntityTypes {
 tree = 'tree',
 transportBelt = 'transport-belt',
 undergroundBelt = 'underground-belt',
 pipe = 'pipe',
 pipeToGround = "pipe-to-ground",
}
export enum Technologies {
 automation = "automation",
 logistics = "logistics",
 logisticSciencePack = "logistic-science-pack",
 steelProcessing = "steel-processing",
 rocketSilo = "rocket-silo",
}
export type FactorioPlayerById = { [playerIdString: string]: FactorioPlayer };

export type FactorioRecipeByName = { [name: string]: FactorioRecipe };

export type FactorioTechnologyByName = { [name: string]: FactorioTechnology };

export type FactorioEntityPrototypeByName = { [name: string]: FactorioEntityPrototype };

export type FactorioItemPrototypeByName = { [name: string]: FactorioItemPrototype };

export type FactorioInventory = { [name: string]: number };


export type StarterMinerFurnace = {
 minerPosition: Position
 minerType: string
 furnacePosition: Position
 furnaceType: string
 oreName: string
 plateName: string
}
export type StarterMinerChest = {
 minerPosition: Position
 minerType: string
 chestPosition: Position
 chestType: string
 oreName: string
}
export type StarterCoalLoop = {
 minerPosition: Position
 minerType: string
}
export type World = {
 starterMinerFurnaces: StarterMinerFurnace[] | null
 starterMinerChests: StarterMinerChest[] | null
 starterCoalLoops: StarterCoalLoop[] | null
 starterOffshorePump: Position | null
 starterSteamEngineBlueprints: FactorioEntity[][] | null
 starterScienceBlueprints: FactorioEntity[][] | null
 minerLineByOreName: {[oreName: string]: FactorioEntity[][]} | null
}
export type FactorioBlueprintResult = {
 blueprint: FactorioBlueprint
}
export type FactorioBlueprintIcon = {
 index: number,
 signal: {
 name: string,
 type: string
 }
}
export type FactorioBlueprint = {
 entities: FactorioEntity[],
 icons: FactorioBlueprintIcon[],
 item: string,
 label: string,
 label_color: string,
 version: string
}
// --- AUTOGENERATED from types.rs starting here - do not remove this line ---
export type FactorioFluidBoxPrototype = { pipeConnections: FactorioFluidBoxConnection [] | null; productionType: string };
export type FactorioFluidBoxConnection = { maxUndergroundDistance: number | null; connectionType: string; positions: Position [] };
export type FactorioBlueprintInfo = { label: string; blueprint: string; width: number; height: number; rect: Rect; data: object };
export type PlayerChangedDistanceEvent = { playerId: number; buildDistance: number; reachDistance: number; dropItemDistance: number; itemPickupDistance: number; lootPickupDistance: number; resourceReachDistance: number };
export type PlayerChangedPositionEvent = { playerId: number; position: Position };
export type PlayerChangedMainInventoryEvent = { playerId: number; mainInventory: { [key: string]: number } };
export type PlayerLeftEvent = { playerId: number };
export type RequestEntity = { name: string; position: Position };
export type FactorioTile = { name: string; playerCollidable: boolean; position: Position; color: number [] | null };
export type FactorioTechnology = { name: string; enabled: boolean; upgrade: boolean; researched: boolean; prerequisites: string [] | null; researchUnitIngredients: FactorioIngredient []; researchUnitCount: number; researchUnitEnergy: number; order: string; level: number; valid: boolean };
export type FactorioForce = { name: string; forceId: number; currentResearch: string | null; researchProgress: number | null; technologies: { [key: string]: FactorioTechnology } };
export type InventoryResponse = { name: string; position: Position; outputInventory: { [key: string]: number } | null; fuelInventory: { [key: string]: number } | null };
export type FactorioRecipe = { name: string; valid: boolean; enabled: boolean; category: string; ingredients: FactorioIngredient [] | null; products: FactorioProduct []; hidden: boolean; energy: number; order: string; group: string; subgroup: string };
export type PlaceEntityResult = { player: FactorioPlayer; entity: FactorioEntity };
export type PlaceEntitiesResult = { player: FactorioPlayer; entities: FactorioEntity [] };
export type FactorioIngredient = { name: string; ingredientType: string; amount: number };
export type FactorioProduct = { name: string; productType: string; amount: number; probability: number };
export type FactorioPlayer = { playerId: number; position: Position; mainInventory: { [key: string]: number };
 buildDistance: number; reachDistance: number; dropItemDistance: number; itemPickupDistance: number; lootPickupDistance: number; resourceReachDistance: number };
export type ChunkPosition = { x: number; y: number };
export type Position = { x: number; y: number };
export type Rect = { leftTop: Position; rightBottom: Position };
export type FactorioChunk = { entities: FactorioEntity [] };
export type ChunkObject = { name: string; position: Position; direction: string; boundingBox: Rect; outputInventory: { [key: string]: number } | null; fuelInventory: { [key: string]: number } | null };
export type ChunkResource = { name: string; position: Position };
export type FactorioGraphic = { entityName: string; imagePath: string; width: number; height: number };
export type FactorioEntity = { name: string; entityType: string; position: Position; boundingBox: Rect; direction: number; dropPosition: Position | null; pickupPosition: Position | null; outputInventory: { [key: string]: number } | null; fuelInventory: { [key: string]: number } | null; amount: number | null; recipe: string | null; ghostName: string | null; ghostType: string | null };
export type FactorioEntityPrototype = { name: string; entityType: string; collisionMask: string [] | null; collisionBox: Rect; mineResult: { [key: string]: number } | null; miningTime: number | null; miningSpeed: number | null; craftingSpeed: number | null; maxUndergroundDistance: number | null; fluidboxPrototypes: FactorioFluidBoxPrototype [] | null };
export type FactorioItemPrototype = { name: string; itemType: string; stackSize: number; fuelValue: number; placeResult: string; group: string; subgroup: string };
export type FactorioResult = { success: boolean; output: string [] };
export type PrimeVueTreeNode = { key: string; label: string; leaf: boolean; children: PrimeVueTreeNode [] };
export type FactorioSettings = { client_count: number; factorio_archive_path: string; map_exchange_string: string; rcon_pass: string; rcon_port: number; recreate: boolean; restapi_port: number; seed: string; workspace_path: string };
export type RestApiSettings = { port: number };
