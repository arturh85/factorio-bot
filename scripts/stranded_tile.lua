-- THE STRANDED TILE, with the instrument it always needed.
--
-- `OreToPlate` loses a placement on pass 1 and the replan then refuses at that
-- tile citing "a tree, cliff, rock or unit". Two separate investigations showed
-- the GAME has no tree there, before or after. The occupant is a box in
-- `blocked_tree`, and until tonight nothing could enumerate that structure from
-- a script -- so both of us could only say "a box is here and nothing I can ask
-- will name it".
--
-- `world.blocked_boxes` can now be diffed against the game over the same
-- rectangle. The label this case turns on is **MODEL ONLY**: a box the model
-- believes blocks ground the game says is clear. On clean ground that count was
-- measured at zero, so a non-zero here is the thing itself.
--
-- READ COVERAGE FIRST. An empty box list means "clear" only under `charted`;
-- under `unknown` it means nobody looked. That is the distinction the retracted
-- tree hypothesis lacked, and I am not repeating it.
print("start stranded tile")

local BP = "0eNqd09FugyAUgOF34VobD6BVX2K72N2yLNqebSSKDdBlTeO7j65L2k2anNNLIXwg+h9FP+xx54wNoj0KE3AU7dVYJoauxyGOPTh8mh6HLmAcRBtMMOhF+3w8Pxxe7X7s0YkWMmG7EeOS4Drrd5MLeSRO1m7ycdlkT1t9iVatykwcRFusyjkTW+Nwc56t52zBSjYLFFaxWUlhNZtVFLZks5rCVmy2pLBrNltR2JrNrilsw2ZrCgsF221ILr8zIIUGd5RGSg34rQEpNuDXBqTc4NJbv3cWXT4aa+x7vnVmGJa6PNt/YZ2Cq3vgkgBfmjNusvnmA33iGvLi6hdOMfX/8xnr0YU4t7CKWzWATMn0zOAWnHpxSc9Mslyg34S+2VnyKuQlNB8mi/lb3KDb4BKuftnUp5LqjgMq2gE184Bqnl8y8YnO/0yVlWx005RKS1Bxh/kb9fTEwQ=="

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "burner-inserter", 8)
  rcon.cheat_item(id, "iron-chest", 4)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", 160)
end

local plan = goal.plan(goal.built(BP))
local obs = goal.run(plan)
print(string.format("pass 1: done=%s failed=%s pending=%s",
  tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
if (obs.failed or 0) == 0 then
  print("pass 1 built cleanly -- the stranding case did not occur this run.")
  print("It is a coin flip; nothing below would be about it.")
  return
end

-- Where does the replan refuse? Its message names the tile.
local tile
local ok, err = pcall(function() return goal.plan(goal.built(BP)) end)
if ok then
  print("replan ACCEPTS -- no stranded tile this run")
  return
end
local msg = tostring(err):gsub("%s+", " ")
print("replan REFUSES: " .. msg:sub(1, 150))
local tx, ty = msg:match("at tile %(([-%d%.]+), ([-%d%.]+)%)")
if not tx then print("could not parse the tile out of the refusal") return end
tile = { x = tonumber(tx), y = tonumber(ty) }
print(string.format("refused tile: (%.1f,%.1f)", tile.x, tile.y))

-- ---- the model, over a small rectangle centred on that tile
local lt = { x = tile.x - 4, y = tile.y - 4 }
local rb = { x = tile.x + 4, y = tile.y + 4 }
local report = world.blocked_boxes(lt, rb)
assert(type(report) == "table" and type(report.boxes) == "table")
print(string.format("model coverage: %s (%s of %s tiles written out)",
  tostring(report.coverage), tostring(report.tiles_charted), tostring(report.tiles_in_area)))
print("model boxes: " .. #report.boxes)
if report.coverage ~= "charted" then
  print("  ^ coverage is not `charted`, so an empty or short box list here says")
  print("    nobody looked rather than that the ground is clear. Stopping.")
  return
end

-- ---- the game, over the SAME rectangle the model widened to
local area = report.area
local cx = (area.left_top.x + area.right_bottom.x) / 2
local cy = (area.left_top.y + area.right_bottom.y) / 2
local radius = math.max(area.right_bottom.x - cx, area.right_bottom.y - cy) + 1
local ents = rcon.find_entities_in_radius({ x = cx, y = cy }, radius)
local seen = {}
if type(ents) == "table" then
  for _, e in ipairs(ents) do seen[e.name] = (seen[e.name] or 0) + 1 end
end
local parts = {}
for n, c in pairs(seen) do parts[#parts+1] = n .. "x" .. c end
table.sort(parts)
print("game entities in the same rectangle: " .. ((#parts > 0) and table.concat(parts, ", ") or "(none)"))

-- ---- does a model box cover the refused tile, and does the game agree?
local covering = 0
for _, b in ipairs(report.boxes) do
  -- A box carries its rectangle under `area`, not as bare corners. The first
  -- version indexed `b.left_top` and raised on nil -- a shape assumed rather
  -- than read, which is the same class of error as everything else tonight.
  local a = b.area
  local l, t = a.left_top.x, a.left_top.y
  local r, bo = a.right_bottom.x, a.right_bottom.y
  if tile.x >= l and tile.x <= r and tile.y >= t and tile.y <= bo then
    covering = covering + 1
    print(string.format("  model box covering the refused tile: [%.2f,%.2f]..[%.2f,%.2f] minable=%s",
      l, t, r, bo, tostring(b.description or b.minable)))
  end
end
-- The last link: a mining drill needs ORE under it. If the refused tile has
-- none, the game's refusal is CORRECT and permanent, and the defect is upstream
-- in whatever sited a drill there.
local ore_here = rcon.find_entities_in_radius(tile, 1.5, "iron-ore")
local n_ore = (type(ore_here) == "table") and #ore_here or -1
print(string.format("iron-ore within the drill's own footprint at the refused tile: %d", n_ore))

print("")
print(string.format("model boxes covering the refused tile: %d", covering))
if covering > 0 and #parts == 0 then
  print("MODEL ONLY. The model blocks a tile the game says is empty -- that is")
  print("  the stranded tile, and it is a model defect rather than terrain.")
elseif covering > 0 then
  print("A box covers the tile AND the game has entities here. Compare the")
  print("  names above against the box: a real obstacle explains the refusal.")
elseif n_ore == 0 then
  print("NO model box, and NO ORE under the drill. The game refused because a")
  print("  mining drill cannot stand on ground with nothing to mine, which is")
  print("  CORRECT and permanent -- so the refusal is right and the defect is")
  print("  upstream: siting placed a drill off the patch.")
else
  print("NO model box covers the refused tile and there IS ore under it, so")
  print("  neither blocked_tree nor a bare tile explains the refusal.")
end
print("end stranded tile")
