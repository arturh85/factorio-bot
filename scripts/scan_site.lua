-- One-shot: for a list of candidate anchors, check whether MinerLine's 37
-- entities would all sit on legal ground -- drills need iron-ore under them,
-- everything else needs to be clear of trees/rocks/water/other entities.
--
-- Kept, unlike synth_plan_check.lua, for the same reason as scan_ore.lua:
-- this is the reproduction trail behind
-- docs/superpowers/notes/2026-09-05-first-block-built.md's MinerLine
-- finding, specifically the negative result that this heuristic is
-- UNRELIABLE -- it reported anchor `(-33, -40)` clean and the live game
-- still refused a belt there, because `find_entities_in_radius` cannot see
-- terrain (water, cliffs) at all. The `CANDIDATES` list below is exactly
-- the abandoned search from that session, left as-is on purpose: it is the
-- evidence for "this many anchors were tried", not a reusable tool. The
-- hardcoded `OFFSETS` are MinerLine's own decoded entity positions.
print("start scan_site")

local OFFSETS = {
  {1.5,1.5,"electric-mining-drill"}, {3.5,0.5,"transport-belt"}, {5.5,1.5,"electric-mining-drill"},
  {3.5,2.5,"transport-belt"}, {3.5,1.5,"transport-belt"}, {1.5,5.5,"electric-mining-drill"},
  {2.5,3.5,"small-electric-pole"}, {3.5,4.5,"transport-belt"}, {3.5,3.5,"transport-belt"},
  {5.5,4.5,"electric-mining-drill"}, {3.5,6.5,"transport-belt"}, {3.5,5.5,"transport-belt"},
  {5.5,7.5,"electric-mining-drill"}, {1.5,8.5,"electric-mining-drill"}, {3.5,8.5,"transport-belt"},
  {3.5,7.5,"transport-belt"}, {2.5,10.5,"small-electric-pole"}, {3.5,10.5,"transport-belt"},
  {3.5,9.5,"transport-belt"}, {5.5,10.5,"electric-mining-drill"}, {1.5,12.5,"electric-mining-drill"},
  {3.5,12.5,"transport-belt"}, {3.5,11.5,"transport-belt"}, {5.5,13.5,"electric-mining-drill"},
  {1.5,15.5,"electric-mining-drill"}, {3.5,14.5,"transport-belt"}, {3.5,13.5,"transport-belt"},
  {3.5,16.5,"transport-belt"}, {3.5,15.5,"transport-belt"}, {5.5,16.5,"electric-mining-drill"},
  {1.5,19.5,"electric-mining-drill"}, {2.5,17.5,"small-electric-pole"}, {3.5,18.5,"transport-belt"},
  {3.5,17.5,"transport-belt"}, {5.5,19.5,"electric-mining-drill"}, {3.5,19.5,"transport-belt"},
  {3.5,20.5,"transport-belt"},
}
print("offsets: " .. #OFFSETS)

local CANDIDATES = {
  {-27.0, -34.0}, {-27.0, -40.0}, {-30.0, -40.0}, {-35.0, -34.0},
  {-38.0, -40.0}, {-38.0, -34.0}, {-40.0, -30.0}, {-40.0, -20.0},
  {-35.0, -18.0}, {-30.0, -18.0}, {-38.0, -28.0}, {-33.0, -40.0},
}

for _, c in ipairs(CANDIDATES) do
  local ax, ay = c[1], c[2]
  local bad = 0
  local report = {}
  for _, o in ipairs(OFFSETS) do
    local px, py, name = ax + o[1], ay + o[2], o[3]
    if name == "electric-mining-drill" then
      local ore = rcon.find_entities_in_radius({x=px,y=py}, 0.4, "iron-ore")
      if #ore == 0 then
        bad = bad + 1
        report[#report+1] = string.format("no ore @ (%.1f,%.1f)", px, py)
      end
    else
      -- Anything NOT a resource sitting on this tile blocks placement:
      -- trees, rocks, cliffs, water. Widened to 1.1: trees sit at arbitrary
      -- sub-tile positions and their collision box can overlap a neighbour
      -- tile's centre from more than 0.4 away.
      local obstacles = rcon.find_entities_in_radius({x=px,y=py}, 1.1)
      local blocking = 0
      for _, e in ipairs(obstacles or {}) do
        if e.name ~= "iron-ore" and e.name ~= "copper-ore" and e.name ~= "coal" and e.name ~= "stone" then
          blocking = blocking + 1
        end
      end
      if blocking > 0 then
        bad = bad + 1
        report[#report+1] = string.format("obstacle @ (%.1f,%.1f)", px, py)
      end
    end
  end
  print(string.format("anchor (%.1f,%.1f): %d/%d offsets bad", ax, ay, bad, #OFFSETS))
  for _, r in ipairs(report) do print("  " .. r) end
end
print("end scan_site")
