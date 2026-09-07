-- Does `rcon.inventory_contents_at` -- the reply the PLANNER reads -- carry a
-- furnace's INPUT inventory, against a real Factorio?
--
-- The mod-level Lua tests drive `rcon_inventory_contents_at` in a stub game
-- with a hand-written `defines` table. That proves the record is built; it
-- cannot prove `defines.inventory.crafter_input` resolves on this install, nor
-- that `entity.get_inventory(index)` answers inside an RCON-invoked function.
-- Only a live query can, and this is the shortest one that does.
--
-- CHEATED AND DISCLOSED: the furnace, the ore and the coal are cheated in.
-- Nothing here is a measurement of anything -- it is a shape probe.
--
-- The control is the second half and it is the important one: a `wooden-chest`
-- has NO input inventory, so its reply must carry NO `input_inventory` key.
-- If both a furnace and a chest answered the same way the field would be
-- worthless -- "absent" and "empty" have to stay apart.
print("start input_inventory probe")

local bots = rcon.players()
local first = bots[1]
local bot1 = (type(first) == "table") and first.player_id or first
print("bot1 = " .. tostring(bot1))

rcon.cheat_item(bot1, "stone-furnace", 1)
rcon.cheat_item(bot1, "wooden-chest", 1)
rcon.cheat_item(bot1, "iron-ore", 34)
rcon.cheat_item(bot1, "coal", 10)

-- `rcon.players()` answers `types.FactorioPlayer`, which carries `position`.
local here = (type(first) == "table") and first.position or { x = 0, y = 0 }
local FURNACE = { x = math.floor(here.x) + 20.5, y = math.floor(here.y) + 0.5 }
local CHEST = { x = math.floor(here.x) + 23.5, y = math.floor(here.y) + 0.5 }

rcon.place_entity(bot1, "stone-furnace", FURNACE, 0)
rcon.place_entity(bot1, "wooden-chest", CHEST, 0)
print(string.format("placed furnace at (%.1f,%.1f), chest at (%.1f,%.1f)",
  FURNACE.x, FURNACE.y, CHEST.x, CHEST.y))

-- `inventory_type` goes straight to `entity.get_inventory(N)`. A burner's fuel
-- slot is 1, a furnace's SOURCE slot is 2 and its RESULT slot is 3; a chest's
-- main inventory is 1. Named rather than left as bare integers, because a
-- wrong index here inserts nowhere and reads as "nothing arrived".
local FUEL, FURNACE_SOURCE, CHEST_MAIN = 1, 2, 1
rcon.insert_to_inventory(bot1, "stone-furnace", FURNACE, FUEL, "coal", 5)
rcon.insert_to_inventory(bot1, "stone-furnace", FURNACE, FURNACE_SOURCE, "iron-ore", 34)
rcon.insert_to_inventory(bot1, "wooden-chest", CHEST, CHEST_MAIN, "coal", 1)

-- One call, both entities, so the two readings are from the same moment.
local reply = rcon.inventory_contents_at({
  { name = "stone-furnace", x = FURNACE.x, y = FURNACE.y },
  { name = "wooden-chest", x = CHEST.x, y = CHEST.y },
})
print("reply type=" .. type(reply) .. " n=" .. tostring(type(reply) == "table" and #reply))

-- `Option::None` reaches Lua as mlua's null sentinel, which is light userdata
-- and therefore TRUTHY -- so every access is type-guarded rather than
-- `or`-defaulted. This is the trap that makes an absent field look present.
local function count_of(entry, key, item)
  if type(entry) ~= "table" then return nil end
  local inv = entry[key]
  if type(inv) ~= "table" then return nil end
  if type(inv[item]) == "number" then return inv[item] end
  for _, slot in ipairs(inv) do
    if type(slot) == "table" and slot.name == item then return slot.count or 0 end
  end
  return 0
end

for i, entry in ipairs(reply) do
  local keys = {}
  for k, _ in pairs(entry) do keys[#keys + 1] = tostring(k) end
  table.sort(keys)
  print(string.format("[%d] %s keys: %s", i, tostring(entry.name), table.concat(keys, ", ")))
  print(string.format("     input_inventory is a table: %s",
    tostring(type(entry.input_inventory) == "table")))
end

local furnace_ore = count_of(reply[1], "input_inventory", "iron-ore")
local chest_input = type(reply[2]) == "table" and type(reply[2].input_inventory) == "table"

print("furnace input_inventory iron-ore = " .. tostring(furnace_ore))
print("chest has an input_inventory table = " .. tostring(chest_input))

-- NOT `== 34`. The furnace is fuelled before the ore goes in, so it starts
-- smelting in the ticks between the insert and the query -- the first run of
-- this probe read 33 of 34 and reported a failure that was the assertion being
-- wrong, not the mechanism. What is being probed is that the slot TRAVELS, not
-- an exact count.
--
-- `chest_input` is a TYPE test, not a presence test, and it has to be: the
-- reply passes through `InventoryResponse` on the way back, so Rust's `None`
-- reaches Lua as mlua's null sentinel -- light userdata, and therefore TRUTHY.
-- The chest's record does have an `input_inventory` key in Lua. It is just not
-- a table.
if type(furnace_ore) == "number" and furnace_ore > 0 and not chest_input then
  print("PROBE PASSED: the on-demand reply carries the furnace's ore ("
    .. tostring(furnace_ore) .. "), and a chest has no input inventory")
else
  print("PROBE FAILED: furnace_ore=" .. tostring(furnace_ore)
    .. " chest_input=" .. tostring(chest_input))
end
