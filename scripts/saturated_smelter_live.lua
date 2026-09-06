-- Does SATURATING the belt actually fix the starvation, or does the near end
-- take everything regardless?
--
-- `TwoRowSmelter` measured six furnaces off one under-supplied belt: the two
-- westmost took 117 of 150 plates (78%) and the two eastmost took 3, and the
-- starved ones ended with FULL fuel slots and no ore. From that I concluded --
-- from the mechanism, not from a measurement -- that coal balances itself
-- because a fuel slot caps at 5 and refuses more, while ore has no such small
-- ceiling, so the fix is to saturate the belt rather than to add furnaces.
--
-- **That conclusion is currently an inference, and it is already written into
-- CLAUDE.md.** This run is what turns it into a measurement or retracts it.
--
-- The block is IDENTICAL to TwoRowSmelter downstream of the coal junction:
-- same lanes, same takeoffs, same six furnace positions. The only change is
-- three ore loaders instead of one, so the belt is fed faster than six furnaces
-- consume. Holding everything else constant is what makes the comparison mean
-- anything.
--
-- What would REFUTE the inference: a west-heavy gradient anyway. What would
-- confirm it: six roughly equal shares.
--
-- CHEATED, DISCLOSED: build materials; ore split across the three ore chests
-- and coal into the coal chest; 5 coal into each of the three ore loaders,
-- which carry only ore and cannot self-fuel. In the real block a miner column
-- feeds the belt and there are no ore arms at all.
print("start saturated smelter")

local BP = "0eNqdlE1ugzAQRu/iNUT+wYC5RpZVVUEyVS2BiYypGkW5e02iJq3iJDPdgQc/Hh6+ObCun2HnrQusOTAbYGDNr7WM9W0HfVxbt2H2bYDteoA+gI8lcMEGCxNrXg7nm/2bm4cuFhuRMdcOEDcG37ppN/qQR9BC3I1T3Da65YVf8cmVztieNXyljxnbWg+bc7U4ZjdYicZKClahsYqCLdDYgoLVaKymYEs0tqRgKzS2omBrNLamYA0aayhYwfF54CQwIWikpIlr1KwfXb75gOlBevN4cUxhrtHqZu/A59ZN4M8D5B7rRpCnyAVKUD4T1HhBSRMsUYLqmWCFF1Q0wZo8SE6Sf8B1CmzoYI4BS4460gtUpY9UCvyRXlgSc6RS4sk/406gwNccTWF0kL9HfLuBBPYEVcnvLvB2hmSnsXbmgV1JGBWCpFdh9ZZxetevpvcW+VsbWnNzmfJTnN5dnJ8StPbe8ZP/6C9SUBEbvBi+ZuwT/HSq6VKawhitCimUiMVvK5iiXA=="
local ORE, COAL = 300, 90

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "iron-chest", 6)
  rcon.cheat_item(id, "burner-inserter", 14)
  rcon.cheat_item(id, "transport-belt", 28)
  rcon.cheat_item(id, "stone-furnace", 8)
  rcon.cheat_item(id, "iron-ore", ORE)
  rcon.cheat_item(id, "coal", COAL + 50)
end

-- Sited with a `near` hint 40 tiles east, NOT with the default.
--
-- `goal.built(bp)` with no site searches outward from the ROSTER'S CENTROID,
-- and siting deliberately treats a character as non-blocking -- a bot can walk
-- away, so it should not veto a site. That holds for a small block. This one is
-- 27 entities spanning roughly 16x8 with furnace rows above and below a central
-- corridor, and the first attempt sited it around the bots: the executor
-- reported `the character is already walled in here ... pocket_tiles=1.0`,
-- bot 2's walk to the far end ended inside a furnace's collision box, and the
-- build stopped with 13 of 29 steps pending and only 2 of 6 furnaces standing.
--
-- The hint is a workaround for THIS run, not a fix. The gap is real: a search
-- that ignores characters can enclose the roster it searched from.
-- Build in up to three passes, replanning between them.
--
-- The first attempt at this block lost 4 placements, all the same shape: a bot
-- routed to a tile the block itself had just filled ("the walk to [51.5, 1.5]
-- would end at [42.5, 3.5], inside a collision box"). `Goal::Built` splits
-- entities into bands per bot but has **no build-order reasoning** -- nothing
-- stops an early placement boxing a bot out of a later one, and this block is
-- dense enough to do it.
--
-- Replanning is the designed answer and has never been exercised live: the goal
-- re-derives *the entities the blueprint names that are not yet standing*, so a
-- second pass finishes the block rather than doubling it. If that property
-- holds, the pass count converges; if it does not, this loop will show it by
-- failing the same count every time.
-- ASK THE GAME WHERE THE GROUND IS CLEAR, then demand that exact anchor.
--
-- Two attempts failed here first, and the second one is the interesting one.
--
--   1. `near` sited the block onto a tree: pass 1 lost 4 placements, and the
--      replan then refused by name -- `cannot build transport-belt at tile
--      (44.5, 0.5): occupied by a tree, cliff, rock or unit`. The refusal was
--      right; the siting was blind.
--   2. Charting first did not help: `goal.charted(40, 0, 30)` planned **zero
--      steps**, i.e. the disc was already charted, and the search still picked
--      the tree. So this is not an exploration gap. **Being charted is not the
--      same as the planner's world model knowing what stands there**, and
--      siting believes the model. The model learned that tree only when a bot
--      walked into it, which is why pass 2 knew what pass 1 did not.
--
-- `rcon.` asks the GAME, which is never wrong about this, so the script picks
-- the anchor and then uses the EXPLICIT form -- the only one that refuses for
-- occupied ground rather than searching past it. If the choice is wrong we get
-- a named refusal at plan time instead of a half-built block.
local FOOTPRINT_R = 10
local function ground_is_clear(cx, cy)
  local here = rcon.find_entities_in_radius({ x = cx, y = cy }, FOOTPRINT_R)
  if type(here) ~= "table" then return false, "query failed" end
  for _, e in ipairs(here) do
    local n = e.name or ""
    if n:find("tree") or n:find("rock") or n:find("cliff") then
      return false, n .. string.format(" at (%.1f,%.1f)", e.position.x, e.position.y)
    end
  end
  return true, #here .. " entities, none blocking"
end

-- WALK A BOT THERE FIRST. Factorio generates chunks lazily, so an ungenerated
-- chunk genuinely holds nothing: the world model AND a live `rcon` query both
-- honestly report clear ground, and it fills with trees the moment something
-- forces generation. The previous attempt read `candidate anchor (40,0): CLEAR
-- -- 0 entities` off the running game and then hit a tree at (44.5, 0.5) once
-- the bots arrived. Zero entities in a 10-tile disc of a fresh map was the tell.
--
-- So: send a bot to each candidate before believing anything about it. The walk
-- is what makes the ground real.
local scout = (type(rcon.players()[1]) == "table")
  and rcon.players()[1].player_id or rcon.players()[1]

local SITE_X, SITE_Y = nil, nil
for _, cand in ipairs({ {40,0}, {40,25}, {40,-25}, {-40,0}, {0,40}, {0,-40}, {60,20}, {-40,-30} }) do
  pcall(function() rcon.move(scout, { x = cand[1] + 6, y = cand[2] }, 6) end)
  local ok, why = ground_is_clear(cand[1] + 6, cand[2])
  print(string.format("candidate anchor (%d,%d): %s -- %s",
    cand[1], cand[2], ok and "CLEAR" or "blocked", why))
  if ok then SITE_X, SITE_Y = cand[1], cand[2] break end
end
if SITE_X == nil then
  print("FAIL: no clear anchor among the candidates. Not siting onto trees and")
  print("  calling the result a distribution measurement.")
  return
end
print(string.format("building at the exact anchor (%d,%d)", SITE_X, SITE_Y))
local site = { x = SITE_X, y = SITE_Y }
local bot1, obs
for pass = 1, 3 do
  local plan = goal.plan(goal.built(BP, site))
  if pass == 1 then
    bot1 = plan.bots[1]
    bot1 = (type(bot1) == "table") and bot1.player_id or bot1
  end
  local places = 0
  for _, st in ipairs(plan.steps) do
    if st.kind == "place" then places = places + 1 end
  end
  print(string.format("pass %d: %d steps, %d placements still outstanding",
    pass, #plan.steps, places))
  if places == 0 then
    print("  nothing left to place -- the block is complete")
    obs = obs or { done = true, failed = 0, lost = 0, pending = 0 }
    break
  end
  obs = goal.run(plan)
  print(string.format("  build: done=%s failed=%s lost=%s pending=%s",
    tostring(obs.done), tostring(obs.failed), tostring(obs.lost), tostring(obs.pending)))
  if (obs.failed or 0) == 0 and (obs.pending or 0) == 0 and (obs.lost or 0) == 0 then
    break
  end
end

-- Name the counts that are actually non-zero. The previous version of this
-- guard said "0 steps never ran" while reporting a build with failed=4, because
-- it printed the pending count whatever the failure was -- a message that
-- describes the wrong number is worse than none.
local bad = {}
if (obs.failed or 0) > 0 then bad[#bad+1] = obs.failed .. " failed" end
if (obs.pending or 0) > 0 then bad[#bad+1] = obs.pending .. " never dispatched" end
if (obs.lost or 0) > 0 then bad[#bad+1] = obs.lost .. " lost" end
if #bad > 0 then
  print("BUILD INCOMPLETE after 3 passes: " .. table.concat(bad, ", ") .. ".")
  print("  `done=true` beside any of those is a contradiction, not a pass;")
  print("  anything measured below would be a fraction of the block.")
  return
end

local furnaces = rcon.find_entities_in_radius({x = 0, y = 0}, 120, "stone-furnace")
if type(furnaces) ~= "table" or #furnaces < 6 then
  print("FAIL: want 6 furnaces, found "
    .. tostring(type(furnaces) == "table" and #furnaces or 0))
  return
end
-- Split by row, then order each row west to east. The rows sit at blueprint
-- offsets y=-2 and y=3, so whichever y is smaller is the north row wherever the
-- block was sited.
table.sort(furnaces, function(a, b)
  if a.position.y ~= b.position.y then return a.position.y < b.position.y end
  return a.position.x < b.position.x
end)
local north, south = {}, {}
local ymin = furnaces[1].position.y
for _, f in ipairs(furnaces) do
  if f.position.y == ymin then north[#north+1] = f else south[#south+1] = f end
end
print(string.format("north row: %d furnaces at y=%.1f   south row: %d",
  #north, ymin, #south))
if #north ~= 3 or #south ~= 3 then
  print("FAIL: rows are " .. #north .. "/" .. #south .. ", not 3/3 -- the block")
  print("  is not the shape this measures, so the numbers below would mislead.")
  return
end

local ref = south[1]
local all = rcon.find_entities_in_radius(ref.position, 16)
if type(all) == "table" then
  local seen = {}
  for _, e in ipairs(all) do seen[e.name] = (seen[e.name] or 0) + 1 end
  local parts = {}
  for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
  table.sort(parts)
  print("everything within 16 tiles: " .. table.concat(parts, ", "))
end

local chests = rcon.find_entities_in_radius(ref.position, 18, "iron-chest")
if type(chests) ~= "table" or #chests < 4 then
  print("FAIL: want 4 chests (3 ore + 1 coal), found "
    .. tostring(type(chests) == "table" and #chests or 0)) return
end
-- The coal chest is the northernmost (offset y=-3.5 against the ore chests'
-- -1.5); the rest are ore. Asserting the split rather than assuming it, because
-- a mislabelled chest would silently put coal on the ore lane and the whole
-- comparison would measure the wrong thing.
table.sort(chests, function(a, b)
  if a.position.y ~= b.position.y then return a.position.y < b.position.y end
  return a.position.x < b.position.x
end)
local coal_chest = chests[1]
local ore_chests = {}
for i = 2, #chests do ore_chests[#ore_chests+1] = chests[i] end
if #ore_chests ~= 3 then
  print("FAIL: expected 3 ore chests, got " .. #ore_chests) return
end
local ore_chest = ore_chests[1]

local arms = rcon.find_entities_in_radius(ref.position, 16, "burner-inserter")
if type(arms) ~= "table" then arms = {} end
print("arms standing: " .. #arms .. " (want 10: 3 ore loaders, 1 coal loader, 6 takeoffs)")
-- The ore loader is the arm sharing the ore chest's x, one tile south of it.
-- An ore loader is the arm one tile south of an ore chest, sharing its x. The
-- north takeoffs sit at the SAME y, so x is what separates them -- picking by y
-- alone would hand coal to a takeoff and starve the block for no visible reason.
local ore_loaders = {}
for _, oc in ipairs(ore_chests) do
  for _, a in ipairs(arms) do
    if math.abs(a.position.x - oc.position.x) < 0.01
       and math.abs(a.position.y - (oc.position.y + 1)) < 0.01 then
      ore_loaders[#ore_loaders+1] = a
    end
  end
end
print("ore loaders identified: " .. #ore_loaders .. " (want 3)")
local ore_loader = ore_loaders[1]

-- Ore split evenly across the three chests so no single feed is the bottleneck.
local per = math.floor(ORE / #ore_chests)
for _, oc in ipairs(ore_chests) do
  rcon.insert_to_inventory(bot1, "iron-chest", oc.position, 1, "iron-ore", per)
end
rcon.insert_to_inventory(bot1, "iron-chest", coal_chest.position, 1, "coal", COAL)
for _, a in ipairs(ore_loaders) do
  rcon.insert_to_inventory(bot1, "burner-inserter", a.position, 1, "coal", 5)
end
print(string.format("charged %d ore into each of %d chests, %d coal, and 5 coal into each of %d ore loaders",
  per, #ore_chests, COAL, #ore_loaders))
local ORE_TOTAL = per * #ore_chests

local t0 = rcon.game_tick()
print("charge complete at tick " .. tostring(t0))

local function count_in(inv, item)
  if type(inv) ~= "table" then return 0 end
  if type(inv[item]) == "number" then return inv[item] end
  for _, slot in ipairs(inv) do
    if type(slot) == "table" and slot.name == item then return slot.count or 0 end
  end
  return 0
end

local ask = {}
for _, f in ipairs(north) do ask[#ask+1] = { name = "stone-furnace", x = f.position.x, y = f.position.y } end
for _, f in ipairs(south) do ask[#ask+1] = { name = "stone-furnace", x = f.position.x, y = f.position.y } end
for _, oc in ipairs(ore_chests) do
  ask[#ask+1] = { name = "iron-chest", x = oc.position.x, y = oc.position.y }
end
ask[#ask+1] = { name = "iron-chest", x = coal_chest.position.x, y = coal_chest.position.y }

local plates, fuel = {}, {}
local ore_left, coal_left = ORE, COAL
local _ = ORE_TOTAL
local deadline = (type(t0) == "number") and (t0 + 30000) or nil
local last_total, last_change = -1, t0

for _ = 1, 6000 do
  local r = rcon.inventory_contents_at(ask)
  local total = 0
  for i = 1, 6 do
    local e = (type(r) == "table") and r[i] or nil
    plates[i] = count_in(type(e) == "table" and e.output_inventory or nil, "iron-plate")
    fuel[i]   = count_in(type(e) == "table" and e.fuel_inventory or nil, "coal")
    total = total + plates[i]
  end
  ore_left = 0
  for k = 1, #ore_chests do
    local e = (type(r) == "table") and r[6 + k] or nil
    ore_left = ore_left + count_in(type(e) == "table" and e.output_inventory or nil, "iron-ore")
  end
  local ce = (type(r) == "table") and r[6 + #ore_chests + 1] or nil
  coal_left = count_in(type(ce) == "table" and ce.output_inventory or nil, "coal")
  local t = rcon.game_tick()

  print(string.format("  tick %-7s ore=%-4s coal=%-4s N=%s/%s/%s S=%s/%s/%s fuelN=%s/%s/%s fuelS=%s/%s/%s",
    tostring(t), tostring(ore_left), tostring(coal_left),
    plates[1], plates[2], plates[3], plates[4], plates[5], plates[6],
    fuel[1], fuel[2], fuel[3], fuel[4], fuel[5], fuel[6]))

  if total ~= last_total then last_change = (type(t) == "number") and t or last_change end
  last_total = total
  if total > 0 and type(t) == "number" and (t - last_change) > 2500 then
    print(string.format("  plateau: no new plate for %d game ticks", t - last_change))
    break
  end
  if deadline and type(t) == "number" and t > deadline then
    print("  deadline: " .. tostring(t - t0) .. " ticks")
    break
  end
end

local nsum = plates[1] + plates[2] + plates[3]
local ssum = plates[4] + plates[5] + plates[6]
print("")
print(string.format("NORTH row (arms see the ORE lane first): %d + %d + %d = %d",
  plates[1], plates[2], plates[3], nsum))
print(string.format("SOUTH row (arms see the COAL lane first): %d + %d + %d = %d",
  plates[4], plates[5], plates[6], ssum))
print(string.format("west -> east, both rows: N %d/%d/%d   S %d/%d/%d",
  plates[1], plates[2], plates[3], plates[4], plates[5], plates[6]))
print(string.format("ore consumed %d of %d, coal consumed %d of %d",
  ORE_TOTAL - ore_left, ORE_TOTAL, COAL - coal_left, COAL))
local lo, hi = plates[1], plates[1]
for i = 2, 6 do
  if plates[i] < lo then lo = plates[i] end
  if plates[i] > hi then hi = plates[i] end
end
print(string.format("SPREAD: lowest furnace %d, highest %d, ratio %.2f",
  lo, hi, (lo > 0) and (hi / lo) or 0))
print("Under-supplied, TwoRowSmelter's spread was 1 to 59 -- a ratio of 59.")
print("A ratio near 1 confirms that saturation is the fix. A ratio still in the")
print("tens refutes it, and CLAUDE.md must be corrected rather than defended.")
print("")
print("Both rows producing means the lane asymmetry self-corrects: a furnace")
print("with a full ore slot forces its arm onto the other lane. One row at zero")
print("means it does not, and a double-sided burner line does not work.")
print("A west-heavy gradient means near furnaces starve far ones, which is what")
print("would decide whether this shape reaches 24 furnaces.")
print("REMINDER: this block is fed by THREE loaders on purpose. It is not the")
print("same experiment as TwoRowSmelter and its totals are not comparable --")
print("only the SPREAD across the six furnaces is.")
print("end saturated smelter")
