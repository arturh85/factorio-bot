-- WHY does the far furnace stop? Testing my own published claim.
--
-- I landed "adding furnaces adds nothing -- the far end starves rather than
-- sharing" off plate counts alone: 30/14/2/0 across four furnaces on fixed
-- supply. "Starves" is a claim about ORE, and I inferred it. `entity.status`
-- now ships as a name, so it can be read instead.
--
-- The prediction that would falsify me: if the idle furnaces read `no_fuel`
-- rather than `no_ingredients`, my mechanism is wrong -- the belt would be
-- failing to deliver COAL, not ore, and every conclusion about near-starves-far
-- would need re-deriving.
--
-- SAMPLED OVER THE WINDOW, not once. A single reading is an instant, not a duty
-- cycle: "87.7% working at tick N" and "working 87.7% of the time" are
-- different quantities and the other session was right to insist on the
-- distinction. This counts every sample per furnace and reports the share.
print("start why the far furnace stops")

local BP = "0eNqd1ktuwjAQgOG7eJ0gxo+8LsKiqqoEpq2lxEG2qYpQ7l4DC1Ax0gzLxOTLCOcnnMQwHnDvrYuiOwkbcRLd3blCjP2AYzq3sTvU6RhdtNFiEN3b6Xpw/HCHaUAvOiiE6ydMn46+d2E/+1imq8/Mfg7pstmd7/IrumplCnEU3XpllkLsrMftdbVZigdWslmgsIrNSgqr2ayisIbNagpbsVlDYWs2W1HYhs3WFLZlsw2FhTXbbUkuvzMghQYvlEZKDfitASk24NcGpNyA3xuQggN+cUBKDujNwbNfX51z6dFJlkuvTnFcSa9Os1x6dYbl0qNrn7kgczA9uoYH06OrefAtOutnV26/MeQe3/vnN8fcEhsO3qEvrQvoY1p7tNasvar/y5N11n2VO2/HMRPEhS6B8B+n4cmaLrf0b6N+9o7Pbpe6lRbi7LD8TH6/xcyDe1Vzm6WAP15DG0/yxmuy4yn+eI/vxPx8mjcfQHZA88KAmjZgxRxQL8t7IX7Qh8uSqWSr29YoLUGlOyx/zuwT4Q=="
local COAL = 100

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "burner-inserter", 8)
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "stone-furnace", 6)
  rcon.cheat_item(id, "coal", COAL + 80)
end
local first = rcon.players()[1]
local bot1 = (type(first) == "table") and first.player_id or first

local probe = goal.plan(goal.built(BP))
for _, st in ipairs(probe.steps) do
  if st.kind == "place" and st.pos then
    for _, b in ipairs(rcon.players()) do
      local id = (type(b) == "table") and b.player_id or b
      pcall(function() rcon.move(id, { x = st.pos.x, y = st.pos.y + 6 }, 6) end)
    end
    break
  end
end
local obs
for pass = 1, 3 do
  local plan = goal.plan(goal.built(BP))
  local places = 0
  for _, st in ipairs(plan.steps) do
    if st.kind == "place" then places = places + 1 end
  end
  if places == 0 then obs = obs or { failed = 0, pending = 0 } break end
  obs = goal.run(plan)
  if (obs.failed or 0) == 0 and (obs.pending or 0) == 0 then break end
end
if (obs.failed or 0) > 0 or (obs.pending or 0) > 0 then print("INCOMPLETE") return end

local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
table.sort(furnaces, function(a, b) return a.position.y < b.position.y end)
local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
local chests = rcon.find_entities_in_radius(furnaces[1].position, 30, "iron-chest")
rcon.insert_to_inventory(bot1, "iron-chest", chests[1].position, 1, "coal", COAL)
for _, d in ipairs(drills) do
  rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
end
print(string.format("%d furnaces, %d drills; charged", #furnaces, #drills))

local t0 = rcon.game_tick()
local tally = {}
for i = 1, #furnaces do tally[i] = {} end
local samples = 0
local t = t0
while type(t) == "number" and type(t0) == "number" and (t - t0) < 6300 do
  -- Re-query rather than reuse: status is live state, and the entity records
  -- from before the charge would report what the block was doing then.
  local now = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
  if type(now) == "table" and #now == #furnaces then
    table.sort(now, function(a, b) return a.position.y < b.position.y end)
    for i, f in ipairs(now) do
      local st = f.status or "<nil>"
      tally[i][st] = (tally[i][st] or 0) + 1
    end
    samples = samples + 1
  end
  t = rcon.game_tick()
end

print(string.format("%d samples over %d game ticks", samples,
  (type(t) == "number") and (t - t0) or -1))
print("")
for i = 1, #furnaces do
  local parts = {}
  for st, n in pairs(tally[i]) do
    parts[#parts+1] = string.format("%s %.0f%%", st, 100 * n / math.max(samples, 1))
  end
  table.sort(parts)
  print(string.format("  furnace %d (near->far): %s", i, table.concat(parts, ", ")))
end
print("")
print("A far furnace reading no_ingredients confirms ore starvation, which is")
print("what I published. no_fuel would refute it -- the belt would be failing to")
print("deliver COAL, and near-starves-far would need re-deriving.")
print("end why the far furnace stops")
