-- RE-MEASURE: what does OreToPlateTee make, now that its drills land on ore?
--
-- The parity fix (2026-09-07) changed where every drill block stands. Before
-- it, `Site::Anywhere` seeded at an ore tile CENTRE and `search_site` stepped
-- in whole tiles, so a block containing 2x2 drills was always anchored on the
-- wrong grid and the game moved every entity half a tile from where the planner
-- checked. In the run that found it, the game refused four drill placements
-- outright ("no entity in the footprint") -- a drill with no ore beneath it.
--
-- So every previously published figure for this block was measured on a block
-- that was partly misplaced. This re-measures it. The number that explains any
-- difference is ORE UNDER EACH DRILL: a burner drill works exactly its own 2x2,
-- so four tiles is the maximum and anything less is ground the fix should have
-- recovered.
--
-- MEASURED IN GAME TICKS at fixed marks, not in iterations. An iteration cap
-- measures the polling rate; this repo has published a wrong ratio that way
-- once already (4.6x, re-measured at 2.6x on a tick-bounded window).
--
-- CHEATED AND DISCLOSED: coal into the coal chest and into each drill's fuel
-- slot. No iron ore and no plates are cheated -- every plate counted here was
-- mined by a drill, carried by a belt, and smelted by a furnace.
print("start rate ore to plate")

local BP = "0eNqd1dFuwiAUgOF34bo1Hgpt6UtsF94ty9K6s42kpQZwmTF996FeaCYm5+xSSL8S4adHMYx73HnrouiOwkacRHczVoixH3BMY08eN/Pz2EfcIKZxdNFGi0F0L8fLj8Ob208DetFBIVw/YXoq+t6F3exjmZQTt5tDemx2p7f9iK5e6UIcRLde6aUQ79bj9jLbLsUdK9ksUNiKzUoKq9hsRWE1m1UUtmazmsI2bLamsC2bbSisYbMthYU12zUkl98ZkEKDf5RGSg34rQEpNqDXBo+uHJVz6blJlkvvrWK59OAUy6UXp1kuPTnzyAWZ+1bQm2t5MD26hgdfo7N+duX2C0Pu+N6e3xxzTWzYe4e+tC6gj2nu3lpz9kqqv/JknXWf5bu345gJ4kyXQPiwa56s6HJN/zeaR3d6fruupYU4Oyw/kt9vMXNwL2p2s1r+8u5v2vz6DG99AMvyWohv9OE8pWtplDG6UhKq9IblFxoeVrE="
local COAL = 100
local WINDOW = 30000   -- game ticks; 500 s of game time at 1x. Set to 6300 to
                       -- reproduce the basis chain_compare.lua used for the
                       -- pre-parity-fix figure of 44 plates.
local MARK   = 3000    -- report every 3000 ticks (50 s of game time)

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "burner-inserter", 10)
  rcon.cheat_item(id, "iron-chest", 6)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", COAL + 120)
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

-- Walk to the site first; without it the build loses placements, and a partial
-- block would make the rate a measurement of the build rather than the design.
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
  return
end

local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
if type(drills) ~= "table" or #drills == 0 or type(furnaces) ~= "table" or #furnaces == 0 then
  print("FAIL: no drills or no furnaces standing") return
end
table.sort(furnaces, function(a, b) return a.position.y < b.position.y end)

-- THE number that explains any change. A burner drill works its own 2x2, so
-- four tiles is the ceiling; fewer means it is standing partly off the patch.
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

local chests = rcon.find_entities_in_radius(drills[1].position, 25, "iron-chest")
if type(chests) ~= "table" or #chests == 0 then print("FAIL: no coal chest") return end
rcon.insert_to_inventory(bot1, "iron-chest", chests[1].position, 1, "coal", COAL)
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
end

local ask = {}
for _, f in ipairs(furnaces) do
  ask[#ask + 1] = { name = "stone-furnace", x = f.position.x, y = f.position.y }
end

-- Read the furnaces' STATUS as well as their output. A rate that falls away
-- has several candidate causes -- ore, coal, the lane, the output -- and the
-- status field distinguishes them where a plate count cannot. It is the field
-- that corrected an ore-only inference about this very block once already:
-- `no_fuel` turned out to climb alongside `no_ingredients`.
--
-- The hypothesis this run exists to kill or confirm: **200 plates is exactly
-- two full output stacks.** A stone furnace's output slot holds 100 iron
-- plates, this block has two furnaces and NO OUTPUT SIDE -- a burner arm
-- carries coal and ore, never plates -- so the ceiling may be the furnaces
-- filling up rather than anything about supply at all.
-- NOTE (measured 2026-09-07): this returns "?" for every furnace, every time.
-- `inventory_contents_at` answers with output and fuel inventories and does NOT
-- carry `status`; that field is on the entity record from
-- `find_entities_in_radius`. Kept, reporting "?" honestly rather than deleted,
-- because the column being uniformly absent is the tell for a field that is not
-- on this response at all -- exactly the shape a stale binary produces, and
-- worth being able to recognise. The question this run asked was settled by the
-- per-furnace plate split below instead.
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

local function plates_now()
  local r = rcon.inventory_contents_at(ask)
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

-- **Drain the furnaces, or this measures storage rather than production.**
--
-- Measured 2026-09-07: this block stops at exactly 200 plates, which is two
-- full stone-furnace output stacks. It has no output side -- a burner arm
-- carries coal and ore, never plates, so nothing empties them -- and once a
-- furnace's result slot fills it simply stops. Every window long enough to
-- approach that ceiling is measuring the slot, not the block.
--
-- Emptying them here is DISCLOSED APPARATUS, not a fix: a real block would use
-- an electric inserter onto a belt, which needs `electronics`. This stands in
-- for that so the block's actual production rate can be seen, and so the
-- headroom above 200 is known before anyone designs the real thing.
--
-- `inventory_type` 3 is a furnace's result slot -- a `defines.inventory` index
-- handed to the game unchanged, not a name.
local FURNACE_RESULT = 3
local drained = 0
local function drain()
  local moved = 0
  for _, f in ipairs(furnaces) do
    local before = 0
    local r = rcon.inventory_contents_at({
      { name = "stone-furnace", x = f.position.x, y = f.position.y },
    })
    if type(r) == "table" and type(r[1]) == "table" and type(r[1].output_inventory) == "table" then
      for _, sl in ipairs(r[1].output_inventory) do
        if type(sl) == "table" and sl.name == "iron-plate" then before = sl.count or 0 end
      end
    end
    if before > 0 then
      pcall(function()
        rcon.remove_from_inventory(bot1, "stone-furnace", f.position, FURNACE_RESULT,
          "iron-plate", before)
      end)
      moved = moved + before
    end
  end
  drained = drained + moved
  return moved
end

local t0 = rcon.game_tick()
print(string.format("charged at tick %s; measuring %d ticks in %d-tick marks",
  tostring(t0), WINDOW, MARK))
print("  tick   elapsed  plates  plates/min  furnace statuses")

local next_mark = MARK
local last_plates = 0
while true do
  local t = rcon.game_tick()
  if type(t) ~= "number" or type(t0) ~= "number" then break end
  local elapsed = t - t0
  if elapsed >= next_mark then
    drain()
    local p = drained + plates_now()
    last_plates = p
    print(string.format("  %-7d %-8d %-7d %-11.1f %s",
      t, elapsed, p, elapsed > 0 and (p * 3600.0 / elapsed) or 0,
      table.concat(statuses(), " | ")))
    next_mark = next_mark + MARK
  end
  if elapsed >= WINDOW then break end
end

print("")
print(string.format("PLATES IN %d GAME TICKS: %d  (%.1f/min)  [%d drained + %d still in furnaces]",
  WINDOW, last_plates, last_plates * 3600.0 / WINDOW, drained, plates_now()))
print("The furnaces were EMPTIED as the run went, so this is production rather")
print("than storage. Compare with 200, which is what the same block reports")
print("when nothing takes the plates away: two full output stacks and a stall.")
-- Per furnace, because the ceiling hypothesis is about EACH slot filling and a
-- total of 200 across two furnaces is equally consistent with 100+100 (capped)
-- and with 150+50 (not capped). Only the split tells them apart.
do
  local r = rcon.inventory_contents_at(ask)
  if type(r) == "table" then
    for i, e in ipairs(r) do
      local n = 0
      if type(e) == "table" and type(e.output_inventory) == "table" then
        for _, sl in ipairs(e.output_inventory) do
          if type(sl) == "table" and sl.name == "iron-plate" then n = sl.count or 0 end
        end
      end
      print(string.format("  furnace %d: %d plates, status %s%s", i, n,
        (type(e) == "table" and e.status) and tostring(e.status) or "?",
        n >= 100 and "   <- FULL STACK, capped" or ""))
    end
  end
end

print("Every plate was mined by a drill, moved by a belt and smelted by a")
print("furnace. Only COAL was cheated, into the coal chest and the drills.")
print("end rate ore to plate")
