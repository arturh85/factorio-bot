-- One-shot: list iron-ore tiles within 120 of spawn, to find a patch big
-- enough for MinerLine's 13-drill, two-column footprint (x in {a,a+4},
-- y spanning ~18 tiles).
print("start scan_ore")
local found = rcon.find_entities_in_radius({ x = 0, y = 0 }, 150, "iron-ore")
print("iron-ore tiles found: " .. tostring(#found))
local minx, maxx, miny, maxy = math.huge, -math.huge, math.huge, -math.huge
for _, e in ipairs(found or {}) do
  if e.position.x < minx then minx = e.position.x end
  if e.position.x > maxx then maxx = e.position.x end
  if e.position.y < miny then miny = e.position.y end
  if e.position.y > maxy then maxy = e.position.y end
end
print(string.format("bbox: x[%s,%s] y[%s,%s]", tostring(minx), tostring(maxx), tostring(miny), tostring(maxy)))
for _, e in ipairs(found or {}) do
  print(string.format("%.1f,%.1f", e.position.x, e.position.y))
end
print("end scan_ore")
