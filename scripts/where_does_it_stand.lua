-- WHERE does a built block actually stand, versus where its stamp said?
--
-- Measured, not reasoned: build OreToPlateTee, then pin a GRID of candidate
-- anchors around the stamp's answer and count how many entities each pinned
-- plan still wants to place. The anchor the block truly stands at is the one
-- that wants ZERO -- that is the definition `already_stands` uses.
--
-- The question exists because a recovery replan reads the block as complete
-- (0 to place) while a replan pinned to the stamp's anchor refuses on the
-- block's own belt. The only input differing between those two paths is the
-- anchor, so the stamp and the ground disagree, and this says by how much.
print("start where does it stand")

local TEE = "0eNqd1dFuwiAUgOF34bo1Hgpt6UtsF94ty9K6s42kpQZwmTF996FeaCYm5+xSSL8S4adHMYx73HnrouiOwkacRHczVoixH3BMY08eN/Pz2EfcIKZxdNFGi0F0L8fLj8Ob208DetFBIVw/YXoq+t6F3exjmZQTt5tDemx2p7f9iK5e6UIcRLde6aUQ79bj9jLbLsUdK9ksUNiKzUoKq9hsRWE1m1UUtmazmsI2bLamsC2bbSisYbMthYU12zUkl98ZkEKDf5RGSg34rQEpNqDXBo+uHJVz6blJlkvvrWK59OAUy6UXp1kuPTnzyAWZ+1bQm2t5MD26hgdfo7N+duX2C0Pu+N6e3xxzTWzYe4e+tC6gj2nu3lpz9kqqv/JknXWf5bu345gJ4kyXQPiwa56s6HJN/zeaR3d6fruupYU4Oyw/kt9vMXNwL2p2s1r+8u5v2vz6DG99AMvyWohv9OE8pWtplDG6UhKq9IblFxoeVrE="

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "burner-inserter", 10)
  rcon.cheat_item(id, "iron-chest", 8)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", 160)
end

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

-- Build it, reading the anchor off the plan that is actually executed.
-- Build to COMPLETION before sweeping. A partial block is the wrong subject:
-- recovery votes on whatever stands, so with half the block down it can settle
-- on an anchor no complete block would have, and the sweep would then be
-- measuring an artefact of the partial build rather than the disagreement.
-- Walk the bots to the site FIRST. Without this the build loses placements
-- (pending=15 twice here, with `placement_pushed_out` teleports in the log),
-- and a partial block makes the whole sweep meaningless: recovery votes on
-- whatever stands, so it settles on an anchor no complete block would have.
do
  local probe_ok, probe = pcall(function() return goal.plan(goal.built(TEE)) end)
  if probe_ok then
    local a = anchor_of(probe)
    if a then
      print(string.format("walking to (%.1f, %.1f) before building", a.x, a.y))
      for _, b in ipairs(rcon.players()) do
        local id = (type(b) == "table") and b.player_id or b
        pcall(function() rcon.move(id, { x = a.x, y = a.y + 6 }, 6) end)
      end
    end
  end
end

local stamped
for pass = 1, 4 do
  local ok, plan = pcall(function() return goal.plan(goal.built(TEE)) end)
  if not ok then
    print("pass " .. pass .. ": plan REFUSED: " .. tostring(plan))
    break
  end
  local a = anchor_of(plan)
  if a then stamped = a end
  local todo = places_in(plan)
  print(string.format("pass %d: %d to place, stamp %s", pass, todo,
    a and string.format("(%.1f, %.1f)", a.x, a.y) or "(none)"))
  if todo == 0 then break end
  local obs = goal.run(plan)
  print(string.format("pass %d: done=%s failed=%s pending=%s",
    pass, tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
end
if not stamped then
  print("no stamp anywhere -- nothing to sweep around") return
end

-- Recovery's own verdict, for the control. It may refuse, and a refusal here
-- is itself the finding: it names the anchor recovery chose.
local rec_ok, rec = pcall(function() return goal.plan(goal.built(TEE)) end)
if rec_ok then
  print(string.format("recovery replan wants %d place(s)", places_in(rec)))
else
  print("recovery replan REFUSED: " .. tostring(rec))
  print("  ^ the anchor named in that message is RECOVERY's answer -- compare")
  print("    it with the stamp above; a constant offset is the bug.")
end

-- Now sweep pinned anchors around the stamp. Half-tile steps, since belts sit
-- on half-integers and a whole-tile-only sweep could step straight over the
-- answer.
print("pinned sweep (dx, dy -> places; 0 means THIS is where it stands):")
local best, bestd = nil, nil
for dy = -4, 4 do
  local row = {}
  for dx = -4, 4 do
    local a = { x = stamped.x + dx * 0.5, y = stamped.y + dy * 0.5 }
    local ok, p = pcall(function() return goal.plan(goal.built(TEE, { anchored = a })) end)
    local n = ok and places_in(p) or -1
    row[#row + 1] = string.format("%3d", n)
    if ok and (best == nil or n < best) then
      best, bestd = n, { dx = dx * 0.5, dy = dy * 0.5, x = a.x, y = a.y }
    end
  end
  print(string.format("  dy=%+.1f  %s", dy * 0.5, table.concat(row, " ")))
end

print("")
if bestd then
  print(string.format("BEST pinned anchor: (%.1f, %.1f), offset (%+.1f, %+.1f) from the stamp, %d to place",
    bestd.x, bestd.y, bestd.dx, bestd.dy, best))
  if best == 0 and (bestd.dx ~= 0 or bestd.dy ~= 0) then
    print("RESULT: the block stands at a DIFFERENT anchor than its stamp reported.")
    print("        The offset above is the bug, and it is constant, not noise.")
  elseif best == 0 then
    print("RESULT: the stamp's own anchor reads as complete. Then the earlier")
    print("        refusal came from something other than the anchor value.")
  else
    print("RESULT: NO pinned anchor in this window reads the block as complete,")
    print("        so the disagreement is not a translation -- pinning differs")
    print("        from recovery in some way other than the anchor.")
  end
end
print("end where does it stand")
