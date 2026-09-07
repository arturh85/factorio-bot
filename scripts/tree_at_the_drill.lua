-- Is there actually a tree there, and when did the model learn it?
--
-- `OreToPlate` loses a placement on pass 1 and its replan then refuses:
-- "cannot build burner-mining-drill at tile (-16,-14): occupied by a tree,
-- cliff, rock or unit". That is the same shape I RETRACTED earlier today after
-- probing (44.5,0.5) and finding the tile empty in both the game and the model.
--
-- So the mechanism is not established, and I am not going to re-assert it. The
-- discriminator is the one that settled the last case and it is already in the
-- bindings: `rcon.*` asks the GAME, `world.*` asks the PLANNER'S MODEL.
--
--   game empty + model empty   -> no tree; the refusal is about something else
--   game FULL  + model empty   -> a real tree the model has not learned
--   both full BEFORE the build -> siting chose a footprint it could already see
--                                 was blocked, which would be a siting defect
--
-- The last case is the one worth knowing, and only asking BEFORE the build can
-- tell it apart from the others.
print("start tree at the drill")

local BP = "0eNqd09FugyAUgOF34VobD6BVX2K72N2yLNqebSSKDdBlTeO7j65L2k2anNNLIXwg+h9FP+xx54wNoj0KE3AU7dVYJoauxyGOPTh8mh6HLmAcRBtMMOhF+3w8Pxxe7X7s0YkWMmG7EeOS4Drrd5MLeSRO1m7ycdlkT1t9iVatykwcRFusyjkTW+Nwc56t52zBSjYLFFaxWUlhNZtVFLZks5rCVmy2pLBrNltR2JrNrilsw2ZrCgsF221ILr8zIIUGd5RGSg34rQEpNuDXBqTc4NJbv3cWXT4aa+x7vnVmGJa6PNt/YZ2Cq3vgkgBfmjNusvnmA33iGvLi6hdOMfX/8xnr0YU4t7CKWzWATMn0zOAWnHpxSc9Mslyg34S+2VnyKuQlNB8mi/lb3KDb4BKuftnUp5LqjgMq2gE184Bqnl8y8YnO/0yVlWx005RKS1Bxh/kb9fTEwQ=="
local TILE = { x = -16, y = -14 }

local function census(label)
  local g = rcon.find_entities_in_radius(TILE, 3)
  local m = world.find_entities_in_radius(TILE, 3)
  local function names(t)
    if type(t) ~= "table" then return "(not a table)" end
    local seen = {}
    for _, e in ipairs(t) do seen[e.name or "?"] = (seen[e.name or "?"] or 0) + 1 end
    local parts = {}
    for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
    table.sort(parts)
    return (#parts > 0) and table.concat(parts, ", ") or "(nothing)"
  end
  print(string.format("%-22s GAME=%-3s MODEL=%s",
    label, tostring(type(g) == "table" and #g or -1), tostring(type(m) == "table" and #m or -1)))
  print("    game : " .. names(g))
  print("    model: " .. names(m))
end

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 30)
  rcon.cheat_item(id, "burner-inserter", 6)
  rcon.cheat_item(id, "iron-chest", 3)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", 160)
end

-- BEFORE anything is built or walked to. This is the reading the last probe
-- could not get, because that run had already built a pass.
census("before the build")

local plan = goal.plan(goal.built(BP))
local obs = goal.run(plan)
print(string.format("pass 1: done=%s failed=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))

census("after pass 1")

local ok, err = pcall(function() return goal.plan(goal.built(BP)) end)
if ok then
  print("replan ACCEPTS -- no refusal this time")
else
  print("replan REFUSES: " .. tostring(err):gsub("%s+", " "):sub(1, 160))
end

print("")
print("If GAME was non-empty BEFORE the build, siting picked a footprint the")
print("game could already have told it was blocked -- and whether the MODEL knew")
print("is the difference between a siting bug and an ingest gap.")
print("end tree at the drill")
