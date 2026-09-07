-- RATE ONE BLOCK: stand a searched layout up and measure what it makes.
--
-- The live half of `factorio-bot search`. That command ranks layouts by the
-- flow graph's prediction; this script measures one of them in the game so the
-- prediction has a number to stand beside. `tools/rate_block.sh NAME` writes
-- the block's blueprint into `rate_block.txt` beside this file and runs it on
-- an isolated headless instance with a FRESH map, one map per variant.
--
-- Everything the measurement means is inherited from
-- `scripts/rate_three_drills.lua`, whose apparatus this is, generalised:
--
--   * MEASURED IN GAME TICKS at fixed marks, never in iterations.
--   * CHEATED AND DISCLOSED: coal into the coal chest (when the block has one)
--     and into every drill's fuel slot. No ore and no plates are cheated.
--   * THE FURNACES ARE EMPTIED as the run goes. A stone furnace's result slot
--     holds 100 plates and a block with no output side stops there; draining
--     makes this a production rate rather than a storage count. A searched
--     block carries an output arm into a chest, and the chest is counted too.
--
-- The last line is machine-readable: `RATE_BLOCK name=<n> plates=<p>
-- window=<ticks> per_min=<r> coverage=<c>/<possible>`.
print("start rate block")

-- `rate_block.txt` is three lines: the candidate's name, its blueprint
-- string, and the coal to charge the chest with. Written by the tool, read
-- through the sandbox's own `file_read`, which resolves against the scripts
-- directory and nowhere else.
local raw = file_read("rate_block.txt")
local lines = {}
for line in raw:gmatch("[^\n]+") do lines[#lines + 1] = line end
local spec = { name = lines[1], blueprint = lines[2], coal = tonumber(lines[3] or "150") }
local NAME = spec.name
local BP = spec.blueprint
local COAL = spec.coal or 150
local WINDOW = 30000 -- game ticks: 500 s of game time at 1x
local MARK = 3000

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 10)
  rcon.cheat_item(id, "transport-belt", 100)
  rcon.cheat_item(id, "burner-inserter", 20)
  rcon.cheat_item(id, "iron-chest", 12)
  rcon.cheat_item(id, "stone-furnace", 10)
  rcon.cheat_item(id, "coal", COAL + 400)
end
local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

local function places_in(plan)
  local n = 0
  for _, st in ipairs(plan.steps) do
    if st.kind == "place" then n = n + 1 end
  end
  return n
end
local function anchor_of(plan)
  for _, st in ipairs(plan.steps) do
    if st.kind == "stamp_ghosts" and st.pos then return st.pos end
  end
end

do
  local ok, probe = pcall(function() return goal.plan(goal.built(BP)) end)
  if ok then
    local a = anchor_of(probe)
    if a then
      for _, b in ipairs(rcon.players()) do
        local id = (type(b) == "table") and b.player_id or b
        pcall(function() rcon.move(id, { x = a.x, y = a.y + 6 }, 6) end)
      end
    end
  end
end

local complete = false
for pass = 1, 3 do
  local ok, plan = pcall(function() return goal.plan(goal.built(BP)) end)
  if not ok then print("pass " .. pass .. ": REFUSED: " .. tostring(plan)) break end
  local todo = places_in(plan)
  local a = anchor_of(plan)
  print(string.format("pass %d: %d to place%s", pass, todo,
    a and string.format(", anchor (%.1f, %.1f)", a.x, a.y) or ""))
  if todo == 0 then complete = true break end
  local obs = goal.run(plan)
  print(string.format("pass %d: done=%s failed=%s pending=%s",
    pass, tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
  if (obs.pending or 0) == 0 and (obs.failed or 0) == 0 then complete = true break end
end
if not complete then
  print("BUILD INCOMPLETE -- refusing to quote a rate for a fraction of a block")
  print(string.format("RATE_BLOCK name=%s plates=0 window=0 per_min=0 coverage=0/0 incomplete=true", NAME))
  return
end

local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
if type(drills) ~= "table" or #drills == 0 or type(furnaces) ~= "table" or #furnaces == 0 then
  print("FAIL: no drills or no furnaces standing") return
end
table.sort(furnaces, function(a, b) return a.position.y < b.position.y end)

local total_tiles, total_ore = 0, 0
for i, d in ipairs(drills) do
  local found = rcon.find_entities_in_radius(d.position, 1.5, "iron-ore")
  local amt, tiles = 0, 0
  if type(found) == "table" then
    for _, o in ipairs(found) do tiles = tiles + 1; amt = amt + (o.amount or 0) end
  end
  total_tiles = total_tiles + tiles
  total_ore = total_ore + amt
  print(string.format("drill %d at (%.1f,%.1f): %d ore across %d of 4 tiles",
    i, d.position.x, d.position.y, amt, tiles))
end
print(string.format("DRILL COVERAGE: %d of %d possible tiles (%d ore in reach)",
  total_tiles, #drills * 4, total_ore))

-- The coal chest is the one on the belt row (y of the first belt), when the
-- block has one. Output chests sit beside furnaces and are NOT charged.
local chests = rcon.find_entities_in_radius(drills[1].position, 40, "iron-chest")
local coal_chest = nil
if type(chests) == "table" then
  for _, c in ipairs(chests) do
    if math.abs(c.position.y - (drills[1].position.y + 1.5)) < 0.01 then coal_chest = c end
  end
end
if coal_chest then
  rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
  print(string.format("coal chest at (%.1f,%.1f) charged with %d coal", coal_chest.position.x, coal_chest.position.y, COAL))
else
  print("NO coal chest in this block: only the drills are charged (the unfuelled control)")
end
-- Park on open ground NORTH of the drill row before charging. `insert_to_
-- inventory` walks the bot to reach first, and on a narrow block that walk
-- resolved to a tile the RCON layer refuses as "inside transport-belt" (a
-- character can stand on a belt; the check is stricter than the game) -- one
-- run died there before measuring anything. Every charge is a `pcall`, and an
-- unfuelled drill is COUNTED and printed rather than assumed charged.
local unfuelled = 0
table.sort(drills, function(a, b) return a.position.x < b.position.x end)
for _, d in ipairs(drills) do
  pcall(function() rcon.move(bot1, { x = d.position.x, y = d.position.y - 3 }, 2) end)
  local ok, err = pcall(function()
    rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
  end)
  if not ok then
    unfuelled = unfuelled + 1
    print(string.format("drill at (%.1f,%.1f) NOT charged: %s", d.position.x, d.position.y, tostring(err)))
  end
end
print(string.format("drills charged: %d of %d", #drills - unfuelled, #drills))

local ask = {}
for _, f in ipairs(furnaces) do
  ask[#ask + 1] = { name = "stone-furnace", x = f.position.x, y = f.position.y }
end
local out_ask = {}
if type(chests) == "table" then
  for _, c in ipairs(chests) do
    if c ~= coal_chest then out_ask[#out_ask + 1] = { name = "iron-chest", x = c.position.x, y = c.position.y } end
  end
end

local function count_plates(list)
  local r = rcon.inventory_contents_at(list)
  local total = 0
  if type(r) == "table" then
    for _, e in ipairs(r) do
      if type(e) == "table" and type(e.output_inventory) == "table" then
        for _, s in ipairs(e.output_inventory) do
          if type(s) == "table" and s.name == "iron-plate" then total = total + (s.count or 0) end
        end
      end
    end
  end
  return total
end
local function statuses()
  local r = rcon.inventory_contents_at(ask)
  local out = {}
  if type(r) == "table" then
    for i, e in ipairs(r) do
      out[i] = (type(e) == "table" and e.status) and tostring(e.status) or "?"
    end
  end
  return out
end

local FURNACE_RESULT = 3
local drained = 0
local function drain()
  local moved = 0
  for _, f in ipairs(furnaces) do
    local before = 0
    local r = rcon.inventory_contents_at({ { name = "stone-furnace", x = f.position.x, y = f.position.y } })
    if type(r) == "table" and type(r[1]) == "table" and type(r[1].output_inventory) == "table" then
      for _, sl in ipairs(r[1].output_inventory) do
        if type(sl) == "table" and sl.name == "iron-plate" then before = sl.count or 0 end
      end
    end
    if before > 0 then
      pcall(function()
        rcon.remove_from_inventory(bot1, "stone-furnace", f.position, FURNACE_RESULT, "iron-plate", before)
      end)
      moved = moved + before
    end
  end
  drained = drained + moved
  return moved
end

local t0 = rcon.game_tick()
print(string.format("charged at tick %s; measuring %d ticks in %d-tick marks", tostring(t0), WINDOW, MARK))
print("  tick   elapsed  plates  plates/min  furnace statuses")
local next_mark = MARK
local last_plates = 0
-- Plates at each mark, so the report can quote a STEADY-STATE rate over the
-- last third of the window beside the cumulative one. The cumulative figure
-- carries the belt-fill ramp (the first 3,000 ticks of a 5x4 block read 33.6
-- against a 70.3 cumulative and a ~75 steady state), and a steady-state model
-- has to be compared with a steady-state number -- "a terminal value against
-- a non-terminal one is not a comparison".
local marks = {}
while true do
  local t = rcon.game_tick()
  if type(t) ~= "number" or type(t0) ~= "number" then break end
  local elapsed = t - t0
  if elapsed >= next_mark then
    drain()
    local p = drained + count_plates(ask) + (#out_ask > 0 and count_plates(out_ask) or 0)
    last_plates = p
    marks[#marks + 1] = { elapsed = elapsed, plates = p }
    print(string.format("  %-7d %-8d %-7d %-11.1f %s", t, elapsed, p,
      elapsed > 0 and (p * 3600.0 / elapsed) or 0, table.concat(statuses(), " | ")))
    next_mark = next_mark + MARK
  end
  if elapsed >= WINDOW then break end
end

print("")
print(string.format("PLATES IN %d GAME TICKS: %d  (%.1f/min)", WINDOW, last_plates, last_plates * 3600.0 / WINDOW))
local steady = 0
do
  local n = #marks
  local from = marks[math.max(1, n - math.floor(n / 3))]
  local to = marks[n]
  if from and to and to.elapsed > from.elapsed then
    steady = (to.plates - from.plates) * 3600.0 / (to.elapsed - from.elapsed)
    print(string.format("STEADY STATE over the last %d ticks: %.1f/min", to.elapsed - from.elapsed, steady))
  end
end
print(string.format("RATE_BLOCK name=%s plates=%d window=%d per_min=%.2f steady_per_min=%.2f coverage=%d/%d unfuelled_drills=%d",
  NAME, last_plates, WINDOW, last_plates * 3600.0 / WINDOW, steady, total_tiles, #drills * 4, unfuelled))
print("end rate block")
