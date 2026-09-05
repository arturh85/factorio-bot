-- Quick site scan: for several candidate anchors, count entities (trees,
-- rocks, ore -- anything not ignorable) inside FurnaceLine's 29x11
-- footprint. Does NOT see water/cliffs (rcon.find_entities_in_radius
-- limitation, established in Task 4). Read-only, no cheats, no plan, no run.
print("start furnace_site_scan")
local IGNORE = {
  ["iron-ore"] = true, ["copper-ore"] = true, ["coal"] = true, ["stone"] = true,
  ["character"] = true,
}
local CANDIDATES = {
  { x = 40, y = 40 }, { x = 100, y = -60 }, { x = -100, y = 60 },
  { x = 60, y = -100 }, { x = -60, y = -100 }, { x = 0, y = 100 },
}
for _, a in ipairs(CANDIDATES) do
  local center = { x = a.x + 14.5, y = a.y + 5.5 }
  local scan = rcon.find_entities_in_radius(center, 17)
  local n = 0
  local sample = {}
  for _, e in ipairs(scan or {}) do
    local lx, ly = e.position.x - a.x, e.position.y - a.y
    if lx >= -1 and lx <= 30 and ly >= -1 and ly <= 12 and not IGNORE[e.name] then
      n = n + 1
      if #sample < 3 then sample[#sample + 1] = e.name .. "@" .. string.format("%.1f,%.1f", e.position.x, e.position.y) end
    end
  end
  print(string.format("anchor (%d,%d): %d obstacle(s) %s", a.x, a.y, n, table.concat(sample, "; ")))
end
print("end furnace_site_scan")
