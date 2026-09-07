-- TWO BLOCKS ON ONE MAP: the first caller of `Site::Anchored`.
--
-- Until 2026-09-07 this was impossible, and the reason was not obvious.
-- `resolve_site` calls `recover_anchor` FIRST and unconditionally, and recovery
-- trusts an anchor once TWO of a blueprint's entities stand at the right
-- relative offsets. Our fixtures are variations of one another, so they recover
-- into each other -- measured, not supposed
-- (`crates/core/tests/recovery_crosstalk_probe.rs`):
--
--   MovingBlock finds 5 of its 9 entities inside a standing OreToPlateTee
--
-- So a second block planned on an occupied map is sited INSIDE the first one.
-- The owner ruled on 2026-09-07: persist the anchor with the goal. This script
-- is that feature's first caller, and it exercises three separate things the
-- unit tests can only assert about a fixture world:
--
--   1. a block sites itself, and hands its anchor back via the stamp
--   2. the SAME blueprint planned again with that anchor pinned is a no-op
--   3. a second block, placed by the caller, stands beside it rather than in it
--
-- ORDER MATTERS, and it is a design finding rather than a script detail: the
-- drill-bearing block must site itself FIRST, because it is the one that needs
-- particular ground (ore under every drill, which `drills_are_fed` enforces).
-- A drill-free block can go anywhere clear, so it is the one the caller places.
-- Siting the second block automatically is precisely the operation that gets
-- hijacked, so there is no version of this where both site themselves.
--
-- WHAT THIS DOES NOT CLAIM. Nothing here is a rate measurement, and no plates
-- are counted. It is an existence proof: two designed blocks, standing, at
-- anchors that are not each other's.
print("start two blocks on one map")

local TEE = "0eNqd1dFuwiAUgOF34bo1Hgpt6UtsF94ty9K6s42kpQZwmTF996FeaCYm5+xSSL8S4adHMYx73HnrouiOwkacRHczVoixH3BMY08eN/Pz2EfcIKZxdNFGi0F0L8fLj8Ob208DetFBIVw/YXoq+t6F3exjmZQTt5tDemx2p7f9iK5e6UIcRLde6aUQ79bj9jLbLsUdK9ksUNiKzUoKq9hsRWE1m1UUtmazmsI2bLamsC2bbSisYbMthYU12zUkl98ZkEKDf5RGSg34rQEpNqDXBo+uHJVz6blJlkvvrWK59OAUy6UXp1kuPTnzyAWZ+1bQm2t5MD26hgdfo7N+duX2C0Pu+N6e3xxzTWzYe4e+tC6gj2nu3lpz9kqqv/JknXWf5bu345gJ4kyXQPiwa56s6HJN/zeaR3d6fruupYU4Oyw/kt9vMXNwL2p2s1r+8u5v2vz6DG99AMvyWohv9OE8pWtplDG6UhKq9IblFxoeVrE="
local MOVER = "0eJyd0tGKwjAQBdD3/YplntPFpEm1+RURsTqwA3ZaJqkopf8uWxdZcCsh8xKYwLkM3BGa84C9EEfwI1DEFvyfnYILSqCOwbvK1LauXWmNLrVRgBwpEgbw24/P3xkf29ueh7ZBAa8V8KFF8EDScXH8xvCj9l2gOLMjXMGvvpyC2/xOCk4keHz8ria1bJun3QzCKAVxQIkorwF6KUCbdwnlMyHKgUPfSSwaPP9zgVkKsO98m+yXWb5L9m2WXyX7LstfJ/tVlr9Jb9A6r0F1Uv83y/2f6d003QHQziE+"

for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  rcon.cheat_item(id, "burner-mining-drill", 4)
  rcon.cheat_item(id, "transport-belt", 40)
  rcon.cheat_item(id, "burner-inserter", 10)
  rcon.cheat_item(id, "iron-chest", 8)
  rcon.cheat_item(id, "stone-furnace", 4)
  rcon.cheat_item(id, "coal", 160)
end

-- The anchor a plan chose, read off its `stamp_ghosts` step. This is the
-- documented way a caller learns where siting put a block, and it is pinned by
-- `the_stamp_carries_the_anchor_a_caller_must_pin` so it cannot drift from what
-- `resolve_site` decided.
local function anchor_of(plan)
  for _, st in ipairs(plan.steps) do
    if st.kind == "stamp_ghosts" and st.pos then return st.pos end
  end
  return nil
end

local function places_in(plan)
  local n = 0
  for _, st in ipairs(plan.steps) do
    if st.kind == "place" then n = n + 1 end
  end
  return n
end

-- Build a block, replanning up to three times. A build is not deterministic --
-- walk routing loses placements -- and `goal.built` re-derives only what is not
-- yet standing, so a second pass finishes rather than doubles.
-- Returns the anchor of the plan it ACTUALLY EXECUTED, which is not the same
-- as the anchor of any earlier probe plan.
--
-- **This distinction cost the first run of this script.** Siting for
-- `Site::Anywhere` seeds at the nearest ore the block's drills can reach, and
-- that is read from CHARTED ground. Walking the bots to the site charts more
-- ground, so a plan made before the walk and a plan made after it can site the
-- block at different anchors -- and the block then stands where the second one
-- said, while the caller holds the first one's number. Pinning that number
-- refuses with `cannot build transport-belt ... occupied by transport-belt`:
-- the block's own belt, at an offset the stale anchor does not explain.
--
-- The rule, and it is general to `Site::Anchored`: **read the anchor from the
-- plan you run, never from a plan you only looked at.**
local function build(make_goal, label)
  local anchor
  for pass = 1, 3 do
    local plan = goal.plan(make_goal())
    local todo = places_in(plan)
    if todo == 0 then
      print(string.format("  %s pass %d: nothing left to place", label, pass))
      return anchor
    end
    local a = anchor_of(plan)
    if a then
      anchor = a
      print(string.format("  %s pass %d: %d to place, anchor (%.1f, %.1f)",
        label, pass, todo, a.x, a.y))
    else
      print(string.format("  %s pass %d: %d to place (no stamp; anchor unchanged)",
        label, pass, todo))
    end
    local obs = goal.run(plan)
    print(string.format("  %s pass %d: done=%s failed=%s pending=%s",
      label, pass, tostring(obs.done), tostring(obs.failed), tostring(obs.pending)))
    if (obs.pending or 0) == 0 and (obs.failed or 0) == 0 then return anchor end
  end
  return anchor
end

-- ---------------------------------------------------------------- block one
-- Sited automatically. This is the block with drills, so it must land on ore
-- and only siting can find that.
print("BLOCK ONE (OreToPlateTee), sited by the planner:")
local first_plan = goal.plan(goal.built(TEE))
local tee_anchor = anchor_of(first_plan)
if not tee_anchor then
  print("FAIL: no stamp step, so there is no anchor to pin -- cannot continue")
  return
end
print(string.format("  siting chose (%.1f, %.1f)", tee_anchor.x, tee_anchor.y))

-- Walk the bots to the site before building; this is the fix that cured
-- placement losses on the saturated smelter, and it works on walk routing
-- rather than on the ground.
for _, b in ipairs(rcon.players()) do
  local id = (type(b) == "table") and b.player_id or b
  pcall(function() rcon.move(id, { x = tee_anchor.x, y = tee_anchor.y + 6 }, 6) end)
end
local built_anchor = build(function() return goal.built(TEE) end, "tee")
if built_anchor and (built_anchor.x ~= tee_anchor.x or built_anchor.y ~= tee_anchor.y) then
  print(string.format(
    "  NOTE: the executed plan sited at (%.1f, %.1f), NOT the probe's (%.1f, %.1f)",
    built_anchor.x, built_anchor.y, tee_anchor.x, tee_anchor.y))
  print("        -- charting moved between the two plans. The executed one is")
  print("        the truth, and it is what gets pinned below.")
end
tee_anchor = built_anchor or tee_anchor

-- --------------------------------------------------------- the pin, checked
-- Replanning the SAME block with its anchor pinned must be a no-op. If the pin
-- did not hold, this would re-site and try to place the block a second time.
-- Both replan paths, compared. This is the decisive pair: recovery derives its
-- anchor FROM the standing entities, so it is consistent with them by
-- construction, while a pinned anchor is supplied from outside. If recovery
-- replans cleanly and the pin does not, the pin is off -- and the anchor the
-- stamp handed back is not where the block actually stands.
print("THE PIN: replanning block one, recovery vs recorded anchor")

local rec_ok, rec = pcall(function() return goal.plan(goal.built(TEE)) end)
if rec_ok then
  print(string.format("  recovery replan: %d to place (0 means it recognised the block)",
    places_in(rec)))
else
  print("  recovery replan REFUSED: " .. tostring(rec))
end

local pin_ok, pinned = pcall(function()
  return goal.plan(goal.built(TEE, { anchored = tee_anchor }))
end)
if not pin_ok then
  print("  pinned replan REFUSED: " .. tostring(pinned))
  print("  ^ compare with the recovery line above. If recovery succeeded and")
  print("    this refused, the pinned anchor does not match where the block")
  print("    actually stands, and the stamp is the suspect.")
  pinned = nil
end
local pinned_places = pinned and places_in(pinned) or -1
print(string.format("  places on a pinned replan: %d (expect 0; -1 means refused)",
  pinned_places))
if pinned_places > 0 then
  print("  NOTE: not necessarily the pin failing -- an incomplete build also")
  print("        leaves entities to place. Compare against the anchor below.")
end
local pinned_anchor = (pinned and anchor_of(pinned)) or tee_anchor
print(string.format("  pinned plan's anchor: (%.1f, %.1f)", pinned_anchor.x, pinned_anchor.y))

-- ------------------------------------------------- the defect, demonstrated
-- Block two, sited automatically, on a map that now carries block one. This is
-- the hijack: recovery answers before siting ever runs.
print("THE DEFECT: block two sited automatically, with block one standing")
local hj_ok, hijacked = pcall(function() return goal.plan(goal.built(MOVER)) end)
if not hj_ok then
  print("  automatic siting REFUSED: " .. tostring(hijacked))
  print("  ^ this IS the hijack, surfacing as a refusal: recovery answered with")
  print("    an anchor inside block one, and the ground there is block one's.")
  hijacked = nil
end
local hijack_anchor = hijacked and anchor_of(hijacked)
if hijack_anchor then
  local dx = hijack_anchor.x - tee_anchor.x
  local dy = hijack_anchor.y - tee_anchor.y
  local dist = math.sqrt(dx * dx + dy * dy)
  print(string.format("  automatic siting chose (%.1f, %.1f) -- %.1f tiles from block one",
    hijack_anchor.x, hijack_anchor.y, dist))
  if dist < 12 then
    print("  ^ INSIDE OR ON TOP OF BLOCK ONE. This is the hijack: recovery")
    print("    matched 5 of MovingBlock's 9 entities against block one's")
    print("    layout and answered before siting ran.")
  else
    print("  ^ did NOT land on block one this time -- the collision depends on")
    print("    the two layouts overlapping in this world, so report it as")
    print("    measured rather than as the expected result.")
  end
else
  print("  no stamp step -- block two may have been refused; see any error above")
end

-- ----------------------------------------------------------- the fix, used
-- The caller chooses. 24 tiles east of block one is clear ground and well
-- outside its footprint.
local mover_anchor = { x = tee_anchor.x + 24, y = tee_anchor.y }
print(string.format("THE FIX: block two placed by the caller at (%.1f, %.1f)",
  mover_anchor.x, mover_anchor.y))
build(function() return goal.built(MOVER, { anchored = mover_anchor }) end, "mover")

-- ------------------------------------------------------------- what stands
-- Counted off the live surface, not inferred from the plans.
local function count_near(pos, radius)
  local seen = {}
  local all = rcon.find_entities_in_radius(pos, radius)
  if type(all) ~= "table" then return seen, 0 end
  local n = 0
  for _, e in ipairs(all) do
    if e.name ~= "character" then
      seen[e.name] = (seen[e.name] or 0) + 1
      n = n + 1
    end
  end
  return seen, n
end

local function describe(pos, radius, label)
  local seen, n = count_near(pos, radius)
  local parts = {}
  for name, c in pairs(seen) do parts[#parts + 1] = name .. "x" .. c end
  table.sort(parts)
  print(string.format("  %s at (%.1f, %.1f): %d entities -- %s",
    label, pos.x, pos.y, n, #parts > 0 and table.concat(parts, ", ") or "NOTHING"))
  return n
end

print("WHAT STANDS:")
local n_tee = describe(tee_anchor, 10, "block one")
local n_mover = describe(mover_anchor, 8, "block two")

print("")
if n_tee > 0 and n_mover > 0 then
  print("RESULT: TWO BLOCKS STAND ON ONE MAP, at anchors 24 tiles apart.")
  print("        Block one sited itself; block two was placed by the caller")
  print("        because siting it automatically is what gets hijacked.")
else
  print("RESULT: not both blocks stand. Read the per-pass lines above -- a")
  print("        failed build and a hijacked anchor look nothing alike, and")
  print("        the anchors printed above say which happened.")
end
print("end two blocks on one map")
