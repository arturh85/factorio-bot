-- Does a sited block SMELT -- ore in, plates out, with no bot in the loop?
--
-- MovingBlock proved a block moves items. This is the next rung: a machine
-- consuming one item and producing another, with the inserters either side
-- moving them, and nothing carrying anything by hand.
--
-- CHEATED, DISCLOSED, and the disclosure matters here more than usual:
--   * iron ore and coal are cheated into the SOURCE chest;
--   * the OUTPUT inserter is given coal directly, in its fuel slot.
--
-- That second one is not a convenience, it is a limitation of the design and
-- the run exists partly to measure it. A burner inserter self-fuels from the
-- coal it carries -- which is why the speedrun session's belted cell ran 27,000
-- ticks on arms placed empty. But THIS block's output inserter carries iron
-- PLATES, never coal, so it has no fuel source at all. It runs exactly as long
-- as the charge it starts with. A burner-only block has a fuelled input side
-- and a starving output side, and that is worth stating rather than papering
-- over with a bigger charge.
print("start smelting live")

-- Inlined from scripts/rcontest.lua rather than include()d: a script that
-- silently gets nil for its blueprint would fail in a way that reads like
-- a result.
local BP = "0eNqV0N8KgjAUBvB3OddT/LfAXfYKXUaE2qlG80y2GYn47k0FCTLIy32H89vH6aFULTZGkgPRg3RYg/jIGKiiROWzQ43KSbrtla4ePkdy0km0II79/OjO1NYlGhAxAypq9FvSaAqqO9qRarT1K5rGn14geMgZdCCikA8D+0KSBSlbQ2gCSRaN86OfUuwlBhdpsJqn0YqbLq51mjC4er2ocEWdzHStW7a9W/ZPN77pcHw83InBE42dpnyX5Fme8zRL4jROhuEN0/adUw=="
local ORE, COAL = 50, 25

-- Materials FIRST, then plan. Planning against empty inventories makes the
-- planner solve gathering as well as building: the first attempt here planned
-- three furnaces rather than one, because it decided to smelt the plates for
-- the chests and inserters, and the block's own placements then waited on
-- production a single batch never finished. Cheating after planning changes
-- nothing about the plan already made.
--
-- Materials are cheated, entities are not: every placement below is a real
-- goal.run dispatch. This run's build time therefore EXCLUDES gathering and
-- must not be quoted as an honest end-to-end figure.
for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "burner-inserter", 4)
  rcon.cheat_item(id, "stone-furnace", 2)
  rcon.cheat_item(id, "coal", 50)
  rcon.cheat_item(id, "iron-ore", 100)
end

local plan = goal.plan(goal.built(BP))
local plan_bots1 = plan.bots[1]
print(string.format("plan: %d steps, %d bots, makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

-- What does the plan actually intend to place? A build that reports success
-- while placing a fraction of the block is indistinguishable from one that
-- worked, unless somebody counts.
local placed_names = {}
for _, st in ipairs(plan.steps) do
  if st.kind == "place" and st.entity then
    placed_names[st.entity] = (placed_names[st.entity] or 0) + 1
  end
end
local pp = {}
for n, c in pairs(placed_names) do pp[#pp+1] = n .. "x" .. c end
table.sort(pp)
print("plan intends to place: " .. (#pp > 0 and table.concat(pp, ", ") or "NOTHING"))

local obs = goal.run(plan)
print(string.format("build: done=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
if (obs.pending or 0) > 0 or (obs.failed or 0) > 0 then
  print("BUILD INCOMPLETE -- anything below measures a block that is not all there")
end

-- Find what actually stands, rather than assuming the plan's anchor.
local furnaces = rcon.find_entities_in_radius({x = 0, y = 0}, 120, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces == 0 then
  print("FAIL: no furnace standing; nothing to measure") return
end
local f = furnaces[1]
print(string.format("furnace stands at (%.1f,%.1f)", f.position.x, f.position.y))

-- List EVERYTHING first. A named query returning nothing is ambiguous between
-- "not built" and "built under a name I did not ask for", and guessing which
-- is how an afternoon disappears.
local all = rcon.find_entities_in_radius(f.position, 10)
if type(all) == "table" then
  local seen = {}
  for _, e in ipairs(all) do seen[e.name] = (seen[e.name] or 0) + 1 end
  local parts = {}
  for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
  table.sort(parts)
  print("everything within 10 tiles: " .. table.concat(parts, ", "))
end

local chests = rcon.find_entities_in_radius(f.position, 8, "iron-chest")
print("iron-chests near the furnace: " .. tostring(type(chests) == "table" and #chests or 0))
if type(chests) ~= "table" or #chests < 2 then
  print("FAIL: need a source and a sink") return
end
-- The source is the chest NORTH of the furnace (lower y), the sink SOUTH.
table.sort(chests, function(a, b) return a.position.y < b.position.y end)
local source, sink = chests[1], chests[#chests]
print(string.format("source (%.1f,%.1f)  sink (%.1f,%.1f)",
  source.position.x, source.position.y, sink.position.x, sink.position.y))

-- inventory_type: 1 = a chest's main inventory, 2 = a burner's fuel slot.
-- Named here because a bare integer at call site is the kind of thing that
-- silently inserts into the wrong inventory and reads as "nothing arrived".
-- FUEL_INV = 1, matching scripts/furnace_run.lua, which is a working call
-- site rather than my guess -- 2 gave 'cannot insert to nonexisting
-- inventory of entity burner-inserter'. A chest's main inventory is also 1.
local defines_chest, defines_fuel = 1, 1
local bot1 = (type(plan_bots1) == "table") and plan_bots1.player_id or plan_bots1
rcon.insert_to_inventory(bot1, "iron-chest", source.position, defines_chest, "iron-ore", ORE)
rcon.insert_to_inventory(bot1, "iron-chest", source.position, defines_chest, "coal", COAL)

-- Fuel the output inserter by hand -- see the header. It moves plates, so it
-- can never fuel itself the way the input arm does.
local arms = rcon.find_entities_in_radius(f.position, 4, "burner-inserter")
if type(arms) == "table" then
  for _, a in ipairs(arms) do
    if a.position.y > f.position.y then
      rcon.insert_to_inventory(bot1, "burner-inserter", a.position, defines_fuel, "coal", 5)
      print(string.format("charged the OUTPUT arm at (%.1f,%.1f) with 5 coal",
        a.position.x, a.position.y))
    end
  end
end

-- Confirm the charge actually landed, and show the response SHAPE once rather
-- than assuming it. A read that silently returns nothing looks identical to a
-- block that made nothing.
local probe = rcon.inventory_contents_at({
  { name = "iron-chest", x = source.position.x, y = source.position.y } })
print("raw inventory response type=" .. type(probe)
  .. " first=" .. type(probe and probe[1]))
if type(probe) == "table" and type(probe[1]) == "table" then
  for k, v in pairs(probe[1]) do print("   key " .. tostring(k) .. " = " .. tostring(v)) end
end

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0) .. " -- nothing touches the block from here")

local last_plates, plateau = -1, 0
local last_change = nil
-- inventory_contents_at takes a LIST of {name=, x=, y=} and answers one entry
-- per request in order -- not a position. Both chests are asked in one call so
-- the two readings are from the same moment rather than two.
local ask = {
  { name = "iron-chest", x = source.position.x, y = source.position.y },
  { name = "iron-chest", x = sink.position.x,   y = sink.position.y },
}
-- Wait on GAME TICKS, not iterations. The previous version sampled 40 times
-- in 36 ticks -- under a second of game time, while a stone furnace needs
-- about 3.2 seconds per plate. It measured the polling rate, not the furnace.
local deadline = (type(t0) == "number") and (t0 + 12000) or nil
last_change = t0
for i = 1, 3000 do
  local r = rcon.inventory_contents_at(ask)
  -- A chest's contents are under `output_inventory`, not `inventory` -- the
  -- shape probe above showed it. `fuel_inventory` comes back as mlua's null
  -- sentinel, which is light userdata and therefore TRUTHY, so every access
  -- here is type-guarded rather than `or`-defaulted.
  -- Factorio 2.0 changed `get_contents()` from a name->count MAP to an ARRAY
  -- of {name=, count=, quality=}. Indexing such an array by item name yields
  -- nil, which reads as "the chest is empty" -- which is exactly what the two
  -- previous runs of this script reported about a chest holding 50 ore.
  -- Both shapes are handled so the reading does not depend on which is true.
  local function count_of(e, item)
    if type(e) ~= "table" then return 0 end
    local inv = e.output_inventory
    if type(inv) ~= "table" then return 0 end
    if type(inv[item]) == "number" then return inv[item] end   -- map form
    for _, slot in ipairs(inv) do                              -- array form
      if type(slot) == "table" and slot.name == item then
        return slot.count or 0
      end
    end
    return 0
  end
  local ore_left = count_of(r and r[1], "iron-ore")
  local plates   = count_of(r and r[2], "iron-plate")
  local t = rcon.game_tick()
  print(string.format("  tick %-7s source ore=%-4s sink plates=%s",
    tostring(t), tostring(ore_left), tostring(plates)))
  -- A plateau must be measured in TICKS, not in consecutive readings. The
  -- previous version declared one after 8 identical samples about 2 ticks
  -- apart -- roughly 16 ticks, while a stone furnace needs about 192 ticks per
  -- plate. It was measuring the polling rate again, one loop further along
  -- than the last time it did that.
  if plates ~= last_plates then
    last_change = (type(t) == "number") and t or last_change
  end
  last_plates = plates
  if plates > 0 and type(t) == "number" and type(last_change) == "number"
     and (t - last_change) > 1500 then
    print(string.format("  plateau: no new plate for %d game ticks", t - last_change))
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " game ticks elapsed")
    break
  end
end

print("")
print("PLATES IN THE SINK: " .. tostring(last_plates))
if last_plates > 0 then
  print("RESULT: the block SMELTED -- ore consumed, plates produced by a machine, "
    .. "and moved into the sink by an inserter, with no bot involved after the charge.")
else
  print("RESULT: no plates. The block stands but did not make anything.")
end
print("NOTE: any plateau here is expected and is the finding -- the output arm "
  .. "carries plates, never coal, so it cannot self-fuel and stops when its "
  .. "hand charge burns out. A burner-only block starves on the output side.")
print("end smelting live")
