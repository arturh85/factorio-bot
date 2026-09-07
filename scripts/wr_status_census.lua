-- Re-dump the world-record save, now that `FactorioEntity` carries `status`.
--
-- `wr-census.json` was taken on 2026-09-06, before `input_inventory`,
-- `transport_lines` or `status` existed, so every furnace in it reads
-- `output_inventory: null` and there is no status key anywhere. A field cannot
-- be backfilled into an archive: a fresh dump is what turns it into evidence.
--
-- **Read-only.** Nothing is placed, mined, inserted or set. This is the only
-- copy of the save and it is the project's only oracle.
--
-- Written to `wr-census-status.json`, NOT over `wr-census.json`: the old census
-- is what every number in `2026-09-06-what-the-record-base-knows.md` was
-- computed from, and overwriting it would make that note uncheckable.

local function count(t)
  local n = 0
  if type(t) == "table" then for _ in pairs(t) do n = n + 1 end end
  return n
end

local ok_tick, tick = pcall(rcon.game_tick)
print("game tick: " .. tostring(ok_tick and tick or "FAILED"))

-- The same GAME-vs-MODEL discriminator `wr_probe.lua` uses: a loaded save has
-- every chunk already generated, so `on_chunk_generated` never fires and an
-- empty model looks exactly like a broken ingest unless both sides are asked.
for _, r in ipairs({ 1000, 5000 }) do
  local ok_g, g = pcall(rcon.find_entities_in_radius, { x = 0, y = 0 }, r, nil)
  local ok_m, m = pcall(world.find_entities_in_radius, { x = 0, y = 0 }, r, nil)
  print(string.format("radius %5d   GAME %s   MODEL %s",
    r,
    ok_g and tostring(count(g)) or "FAILED",
    ok_m and tostring(count(m)) or "FAILED"))
end

local ok_dump, err = pcall(world.dump, "wr-census-status.json")
print("world.dump: " .. (ok_dump and "ok" or ("FAILED: " .. tostring(err))))
