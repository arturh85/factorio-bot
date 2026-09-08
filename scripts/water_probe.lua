-- Does "which fluid does this tile yield" cross the bridge?
--
-- Nothing in the Lua bindings can ask a tile anything, so this dumps the whole
-- world and the answer is read out of the JSON afterwards. That is the point:
-- the dump is written from `EntityGraph`, so a `fluid` on a tile in it has
-- travelled mod -> `writeout_tiles` -> `OutputParser` -> `add_tiles` -> here.
-- A mod test cannot show that, which is exactly the gap that let
-- `entity.status` read nil for every furnace while the mod was correct.
--
-- The offshore pump's `fluid_source_offset` rides in the same dump under
-- `entity_prototypes`, and it is the other half of the rule: it says WHICH
-- tile a pump reads, and the tile says what is in it.
print("tick " .. rcon.game_tick())
print("players " .. #rcon.players())
world.dump("water-probe.json")
print("dumped")
