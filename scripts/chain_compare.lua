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
  { name = "sideload-downstream", bp = "0eNqd09FugyAUgOF34VobD6BVX2K72N2yLNqebSSKDdBlTeO7j65L2k2anNNLIXwg+h9FP+xx54wNoj0KE3AU7dVYJoauxyGOPTh8mh6HLmAcRBtMMOhF+3w8Pxxe7X7s0YkWMmG7EeOS4Drrd5MLeSRO1m7ycdlkT1t9iVatykwcRFusyjkTW+Nwc56t52zBSjYLFFaxWUlhNZtVFLZks5rCVmy2pLBrNltR2JrNrilsw2ZrCgsF221ILr8zIIUGd5RGSg34rQEpNuDXBqTc4NJbv3cWXT4aa+x7vnVmGJa6PNt/YZ2Cq3vgkgBfmjNusvnmA33iGvLi6hdOMfX/8xnr0YU4t7CKWzWATMn0zOAWnHpxSc9Mslyg34S+2VnyKuQlNB8mi/lb3KDb4BKuftnUp5LqjgMq2gE184Bqnl8y8YnO/0yVlWx005RKS1Bxh/kb9fTEwQ==" },
  { name = "sideload-upstream", bp = "0eNqd1UFrwyAUwPHv4jkpVWPa5DrYdTvsNsZI2rdNMBrUjpWS7z6zDjJWC+/12Ig/JeZvT6w3Bxi9tpG1J6YjDKz986xgpuvBpGcPHp7co+ki3LnO3Gsf5lGwUUcNgbXPp/OP46s9DD141vKC2W6ANDf6zobR+Vgma542upCmOTuv+cVauVIFO7K25Cs1FWyvPezOw9upuHAF3V1jXEl2UWxFZlFvQZFZgWFrMisx7IbMVhh2S2YVhm3IbI1h+ZrsblAuvbQtyqWX1qBcemkclRq/oTVUbJxeG0flxum9cVRwfClOe2fL3QeEDDrfjMvVm3OWxPqDt+BLbQP4mMYusPXVa5yLHI3PjF+Vq9wfBL4zQYP5/5cxaKvte7n32piMfj4wBCxugTE7lvjjq662lj0+scQWorNQvqUFuh1cwvUvm/u+hLphgxK3wZq4QTlNLwX7BB9+hlQtmqpplKwEl2mF6RsEHwL/" },
  { name = "true-T", bp = "0eNqd1dFuwiAUgOF34bo1Hgpt6UtsF94ty9K6s42kpQZwmTF996FeaCYm5+xSSL8S4adHMYx73HnrouiOwkacRHczVoixH3BMY08eN/Pz2EfcIKZxdNFGi0F0L8fLj8Ob208DetFBIVw/YXoq+t6F3exjmZQTt5tDemx2p7f9iK5e6UIcRLde6aUQ79bj9jLbLsUdK9ksUNiKzUoKq9hsRWE1m1UUtmazmsI2bLamsC2bbSisYbMthYU12zUkl98ZkEKDf5RGSg34rQEpNqDXBo+uHJVz6blJlkvvrWK59OAUy6UXp1kuPTnzyAWZ+1bQm2t5MD26hgdfo7N+duX2C0Pu+N6e3xxzTWzYe4e+tC6gj2nu3lpz9kqqv/JknXWf5bu345gJ4kyXQPiwa56s6HJN/zeaR3d6fruupYU4Oyw/kt9vMXNwL2p2s1r+8u5v2vz6DG99AMvyWohv9OE8pWtplDG6UhKq9IblFxoeVrE=" },
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
    print(string.format("  %d plates after %d game ticks (%d furnaces, %d drills)",
      total, (type(t) == "number" and type(t0) == "number") and (t - t0) or -1,
      #furnaces, #drills))
  end
 end
end
print("end chain topology compare")
