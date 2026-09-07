-- Does the GAME move an entity to fit its footprint parity?
--
-- The half-tile finding (2026-09-07) established that a built block stands at
-- an anchor (+0.5, +0.5) from the one its stamp reported, and named this as the
-- leading hypothesis without proving it. This proves or kills it in one run.
--
-- The rule under test (`method::util::tile_alignment`): an entity with an EVEN
-- footprint sits on a tile BOUNDARY (integer coordinates), one with an ODD
-- footprint sits on a tile CENTRE (half-integers). A stone furnace is 2x2, a
-- transport belt is 1x1. So each has a legal parity and a wrong one.
--
-- `rcon.place_entity` returns the placed entity, and a `FactorioEntity` carries
-- its own position -- so this asks for a position and reads back where the
-- entity actually is. No inference: the game's own answer about its own grid.
--
-- Four cases, deliberately including the two CONTROLS. Without them a snap
-- reading would be indistinguishable from this script mis-computing parity.
print("start does the game snap")

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "stone-furnace", 8)
  rcon.cheat_item(id, "transport-belt", 8)
end
local first = rcon.players()[1]
local bot = (type(first) == "table") and first.player_id or first

local cases = {
  -- name, asked position, parity of that request, what we expect
  { "stone-furnace",  { x = 10.0, y = 10.0 }, "integer  (LEGAL for 2x2)",   false },
  { "stone-furnace",  { x = 20.5, y = 20.5 }, "half-int (WRONG for 2x2)",   true  },
  { "transport-belt", { x = 30.5, y = 30.5 }, "half-int (LEGAL for 1x1)",   false },
  { "transport-belt", { x = 40.0, y = 40.0 }, "integer  (WRONG for 1x1)",   true  },
}

local moved_when_expected, moved_when_not, failures = 0, 0, 0

for _, c in ipairs(cases) do
  local name, asked, parity, expect_move = c[1], c[2], c[3], c[4]
  local ok, got = pcall(function()
    return rcon.place_entity(bot, name, asked, 0)
  end)
  if not ok or type(got) ~= "table" or type(got.position) ~= "table" then
    print(string.format("  %-15s asked (%.1f, %.1f) %s -> PLACEMENT FAILED: %s",
      name, asked.x, asked.y, parity, tostring(got)))
    failures = failures + 1
  else
    local dx = got.position.x - asked.x
    local dy = got.position.y - asked.y
    local moved = (math.abs(dx) > 1e-6) or (math.abs(dy) > 1e-6)
    print(string.format("  %-15s asked (%.1f, %.1f) %s -> stands (%.1f, %.1f)  delta (%+.1f, %+.1f)%s",
      name, asked.x, asked.y, parity,
      got.position.x, got.position.y, dx, dy,
      moved and "  MOVED" or ""))
    if moved and expect_move then moved_when_expected = moved_when_expected + 1 end
    if moved and not expect_move then moved_when_not = moved_when_not + 1 end
  end
end

print("")
if failures > 0 then
  print("INCONCLUSIVE: " .. failures .. " placement(s) failed outright, so the")
  print("  comparison is missing cases. A refusal is not a snap.")
elseif moved_when_expected == 2 and moved_when_not == 0 then
  print("CONFIRMED: the game snaps an entity onto the legal grid for its")
  print("  footprint parity, and leaves a correctly-aligned one alone.")
  print("  So a planner that asks for a wrong-parity position gets an entity")
  print("  somewhere it did not ask for, and its model of the block is off by")
  print("  half a tile -- which is the half-tile finding, explained.")
elseif moved_when_expected == 0 and moved_when_not == 0 then
  print("REFUTED: nothing moved. The game honoured every position, including")
  print("  the two with wrong parity, so snapping is NOT the cause of the")
  print("  half-tile offset and that note needs a different hypothesis.")
else
  print(string.format("MIXED: %d moved when expected, %d moved when not.",
    moved_when_expected, moved_when_not))
  print("  Report the table above as measured; the simple parity rule does not")
  print("  describe what happened and should not be asserted.")
end
print("end does the game snap")
