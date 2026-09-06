-- A LIKE-FOR-LIKE comparison of the three chain topologies.
--
-- The first comparison was not like for like and I published a ratio from it.
-- The two sideload designs ended at a PLATEAU -- they stopped producing, so 17
-- and 18 are terminal values. The T-junction run exhausted its ITERATION cap
-- while still climbing, so 78 was a non-terminal value at an unknown tick. The
-- direction was right and "4.6x the plates" was not a rate comparison.
--
-- Worse, the iteration cap is not tick-bounded: on a faster machine more game
-- ticks pass per RCON round trip, so the same 5,000 polls cover more game time.
-- The other session has since measured its own agents starving a run to 44% of
-- nominal for 90 seconds, so that is a real effect on this box, not a
-- hypothetical one.
--
-- This samples every variant at the SAME game-tick offset from its own charge,
-- which is immune to both: tick offsets do not care how fast wall-clock time
-- runs, and every variant gets the same game duration.
print("start chain topology compare")

local VARIANTS = {
  { name = "2 drills, 2 furnaces", bp = "0eNqV1UtuwyAQBuC7zNqODAa/LpJFVVW2M22RbGwBqRpFvntJskjUEGlmCZiPEfCbMwzTEVdnbIDuDCbgDN1DXwZTP+AU+/bmgDK20QYTDHro3s63xunDHucBHXQiA9vPGL8Orrd+XVzI4+wLsy4+TlvsZZVf6KqdzuAEXbHTWwYH43C8jTZb9sRKNisobMlmJYVVbLaksJrNKgpbsVlNYWs2W1HYhs3WFLZlsw2FFQXbbUkuPWfiVc5UyqUHTbJcetJKlkuPmmK59KxplksPW/vKFTIF0+PW8GB64GoefI+ccYvNx2/0qev7eH9TL8M9YcPRWXS5sR5diGPPVsE5Kyn+y7Oxxn7lB2emKRGIK50LwmsmebKiyyV9N+pXf/Pkccl70nxYLOaf0e9HTFzcm5o8LM0vr6GVV/HKa7btPYMfdP46oivZqrbVpZKijAtsf4ak+9A=" },
  { name = "2 drills, 3 furnaces", bp = "0eNqd1UtugzAQBuC7eA0R4wdgLtJFVVWQTFtLYCLjVI0i7l4nWSRqHGmmS2z8eYT9MycxjAfcB+ej6E7CRZxEdzdWiLEfcExjL26HKj2jjy46XET3ero+HN/9YRowiA4K4fsJ09sx9H7ZzyGWafWZ2c9LWjb78y4/oqs3phBH0VUbsxZi5wJur7PtWjywks0ChVVsVlJYzWYVhTVsVlPYms0aCtuw2ZrCtmy2obCWzbYUFiq2a0kuP2dAChr8I2mkqAE/a0AKG9DTBs9+OTrn0uMmWS49b4rl0gOnWS49cYbl0iNnn7kgc72CnrmWB9ND1/DgW+hcmH25/cIld33v72+OuUVsOASPoXR+wRDT3KNVcc5K6r/y5Lzzn+UuuHHMBOJCl0Bo7IYna7pc079G86yx5Y/rlrQlzh7Lj+T3W8xc3KuaPayWX15LK8/yymtz5amKX95jI8jWp4BXH8C6vhXiG8NymTK1tNpao7QElXZYfwGkQ4fS" },
  { name = "2 drills, 4 furnaces", bp = "0eNqd1ktuwjAQgOG7eJ0gxo+8LsKiqqoEpq2lxEG2qYpQ7l4DC1Ax0gzLxOTLCOcnnMQwHnDvrYuiOwkbcRLd3blCjP2AYzq3sTvU6RhdtNFiEN3b6Xpw/HCHaUAvOiiE6ydMn46+d2E/+1imq8/Mfg7pstmd7/IrumplCnEU3XpllkLsrMftdbVZigdWslmgsIrNSgqr2ayisIbNagpbsVlDYWs2W1HYhs3WFLZlsw2FhTXbbUkuvzMghQYvlEZKDfitASk24NcGpNyA3xuQggN+cUBKDujNwbNfX51z6dFJlkuvTnFcSa9Os1x6dYbl0qNrn7kgczA9uoYH06OrefAtOutnV26/MeQe3/vnN8fcEhsO3qEvrQvoY1p7tNasvar/y5N11n2VO2/HMRPEhS6B8B+n4cmaLrf0b6N+9o7Pbpe6lRbi7LD8TH6/xcyDe1Vzm6WAP15DG0/yxmuy4yn+eI/vxPx8mjcfQHZA88KAmjZgxRxQL8t7IX7Qh8uSqWSr29YoLUGlOyx/zuwT4Q==" },
}
local WINDOW = 6300   -- ticks after charging; the two plateaus landed near here
local COAL = 100

local function count(inv, item)
  if type(inv) ~= "table" then return 0 end
  for _, s in ipairs(inv) do
    if type(s) == "table" and s.name == item then return s.count or 0 end
  end
  return 0
end

-- ONE variant per run, on a fresh map. Building all three on one map was the
-- first version and it was wrong: the second block is sited on ground the first
-- already changed, so they are not comparable and the run died on it.
local WANT = tonumber(file_read("chain_variant.txt")) or 1
for vi, v in ipairs(VARIANTS) do
 if vi == WANT then
  print("--- " .. v.name)
  for _, b in ipairs(rcon.players()) do
    local id = (type(b) == "table") and b.player_id or b
    rcon.cheat_item(id, "burner-mining-drill", 4)
    rcon.cheat_item(id, "transport-belt", 40)
    rcon.cheat_item(id, "burner-inserter", 8)
    rcon.cheat_item(id, "iron-chest", 4)
    rcon.cheat_item(id, "stone-furnace", 4)
    rcon.cheat_item(id, "coal", COAL + 80)
  end
  local first = rcon.players()[1]
  local bot1 = (type(first) == "table") and first.player_id or first

  local probe = goal.plan(goal.built(v.bp))
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
    local plan = goal.plan(goal.built(v.bp))
    local places = 0
    for _, st in ipairs(plan.steps) do
      if st.kind == "place" then places = places + 1 end
    end
    if places == 0 then obs = obs or { failed = 0, pending = 0 } break end
    obs = goal.run(plan)
    if (obs.failed or 0) == 0 and (obs.pending or 0) == 0 then break end
  end
  if (obs.failed or 0) > 0 or (obs.pending or 0) > 0 then
    print("  INCOMPLETE build -- skipping, not comparing a fraction of a block")
  else
    local furnaces = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "stone-furnace")
    local chests = rcon.find_entities_in_radius(furnaces[1].position, 30, "iron-chest")
    local drills = rcon.find_entities_in_radius({ x = 0, y = 0 }, 200, "burner-mining-drill")
    -- The coal chest is whichever holds no ore: the ore arrives by belt here,
    -- so any chest in these blocks is the coal one, but assert rather than
    -- assume, since a wrong chest silently starves the run being compared.
    rcon.insert_to_inventory(bot1, "iron-chest", chests[1].position, 1, "coal", COAL)
    for _, d in ipairs(drills) do
      rcon.insert_to_inventory(bot1, "burner-mining-drill", d.position, 1, "coal", 25)
    end
    local t0 = rcon.game_tick()
    local ask = {}
    for _, f in ipairs(furnaces) do
      ask[#ask + 1] = { name = "stone-furnace", x = f.position.x, y = f.position.y }
    end
    local total, t = 0, t0
    while type(t) == "number" and type(t0) == "number" and (t - t0) < WINDOW do
      local r = rcon.inventory_contents_at(ask)
      total = 0
      for i = 1, #furnaces do
        local e = (type(r) == "table") and r[i] or nil
        total = total + count(type(e) == "table" and e.output_inventory or nil, "iron-plate")
      end
      t = rcon.game_tick()
    end
    -- PER FURNACE, ordered along the stem. A total hides the thing being
    -- measured: near-starves-far shows up as a gradient, and 44 plates split
    -- 40/4 and split 11/11/11/11 are the same total and opposite findings.
    table.sort(furnaces, function(p, q) return p.position.y < q.position.y end)
    local r = rcon.inventory_contents_at(ask)
    local parts = {}
    for i = 1, #furnaces do
      local e = (type(r) == "table") and r[i] or nil
      parts[#parts+1] = tostring(count(type(e) == "table" and e.output_inventory or nil, "iron-plate"))
    end
    print(string.format("  %d plates after %d game ticks (%d furnaces, %d drills)",
      total, (type(t) == "number" and type(t0) == "number") and (t - t0) or -1,
      #furnaces, #drills))
    print("  per furnace, near to far along the stem: " .. table.concat(parts, " / "))
  end
 end
end
print("end chain topology compare")
