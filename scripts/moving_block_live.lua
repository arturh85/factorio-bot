-- "A block that MOVES" (2026-09-06) -- the honesty gap this closes: 138 of
-- FurnaceLine's 179 entities were proven to STAND and never shown to move a
-- single item (see docs/superpowers/notes/2026-09-05-first-block-built.md
-- and CLAUDE.md's `method::blueprint` section). This is the complement:
-- MOVING_BLOCK is nine entities -- iron-chest, burner-inserter, five
-- transport-belt, burner-inserter, iron-chest -- built live with
-- `goal.built`, then left alone while coal is watched crossing it.
--
-- Why this is possible with NO generator and NO research: burner inserters
-- self-fuel from the coal they carry (measured 2026-09-05/06, eight placed
-- EMPTY were still moving coal 27,000 ticks later) and belts need no power at
-- all. See .superpowers/sdd/2026-09-06-block-that-moves/progress.md.
--
-- CHEATS, DISCLOSED, exactly like every earlier block script in this
-- project (siting_furnace_live.lua, furnace_run.lua):
--   * every bot is given the NINE-entity material bill (2 iron-chest, 2
--     burner-inserter, 5 transport-belt) via rcon.cheat_item before
--     planning -- gathering is out of scope, same as every earlier task.
--   * once the block stands, coal is cheated into the SOURCE chest only:
--     rcon.cheat_item gives one bot coal, then rcon.insert_to_inventory
--     moves it from that bot's hands into the chest. Both are raw RCON
--     calls outside goal.run, so neither is a dispatched action and neither
--     touches the DESTINATION chest.
-- Everything else -- every belt tick, every inserter swing that carries a
-- piece of coal from the source chest to the destination chest -- happens
-- with NO bot in the loop and NO further rcon.* mutation of either chest
-- from this script. The evidence is the destination chest's own count
-- rising while the roster dispatches nothing.
print("start moving block live")

local MOVING_BLOCK = "0eJyd0tGKwjAQBdD3/YplntPFpEm1+RURsTqwA3ZaJqkopf8uWxdZcCsh8xKYwLkM3BGa84C9EEfwI1DEFvyfnYILSqCOwbvK1LauXWmNLrVRgBwpEgbw24/P3xkf29ueh7ZBAa8V8KFF8EDScXH8xvCj9l2gOLMjXMGvvpyC2/xOCk4keHz8ria1bJun3QzCKAVxQIkorwF6KUCbdwnlMyHKgUPfSSwaPP9zgVkKsO98m+yXWb5L9m2WXyX7LstfJ/tVlr9Jb9A6r0F1Uv83y/2f6d003QHQziE+"

-- The design table, verbatim (see the task brief and
-- crates/core/tests/blueprint_decode.rs's
-- `the_moving_block_decodes_to_its_nine_entities_with_every_direction_pinned`,
-- which pins exactly this). Field names are relative to the block's own
-- anchor, same convention as siting_furnace_live.lua's EXPECTED_REL.
local EXPECTED_REL = {
  { name = "iron-chest",       rx = 0.5, ry = 0.5, dir = 0 },  -- SOURCE
  { name = "burner-inserter",  rx = 1.5, ry = 0.5, dir = 12 },
  { name = "transport-belt",   rx = 2.5, ry = 0.5, dir = 4 },
  { name = "transport-belt",   rx = 3.5, ry = 0.5, dir = 4 },
  { name = "transport-belt",   rx = 4.5, ry = 0.5, dir = 4 },
  { name = "transport-belt",   rx = 5.5, ry = 0.5, dir = 4 },
  { name = "transport-belt",   rx = 6.5, ry = 0.5, dir = 4 },
  { name = "burner-inserter",  rx = 7.5, ry = 0.5, dir = 12 },
  { name = "iron-chest",       rx = 8.5, ry = 0.5, dir = 0 },  -- DESTINATION
}
print("EXPECTED_REL entities: " .. tostring(#EXPECTED_REL) .. " (want 9)")

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

-- ---------------------------------------------------------------- CHEATS --
print("CHEAT: giving every bot the full material set (2 iron-chest, 2 "
  .. "burner-inserter, 5 transport-belt) -- gathering is not what this run "
  .. "tests")
for _, b in ipairs(bots) do
  rcon.cheat_item(b, "iron-chest", 2)
  rcon.cheat_item(b, "burner-inserter", 2)
  rcon.cheat_item(b, "transport-belt", 5)
end

local run_id = record.start({ video = false })
print("recording run " .. run_id)
rcon.sampling_start(run_id)

-- THE THING UNDER TEST, PART 1: siting chooses the anchor -- this script
-- never names one.
--
-- MEASURED FIRST, not assumed: a bare `goal.built(MOVING_BLOCK)` (no second
-- argument, `Site::Anywhere`) refused here with "cannot build iron-chest at
-- tile (0.5, 0.5): character 1 ... is standing on it". `Site::Anywhere`
-- seeds the search at the world origin whenever the block has no mining
-- drill (`nearest_ore_seed` -- see crates/planner/src/method/blueprint.rs),
-- and every headless character bot spawns AT the origin -- so radius 0 of
-- the search is always exactly where a bot already stands. The search
-- itself ignores characters (`siting_occupant`, deliberately -- a bystander
-- is not durable ground), so it picks (0,0) anyway; the LATER footprint
-- pre-check does treat a character as an obstruction (correctly -- "cleared
-- by walking, not by moving the block") and refuses. Nothing in this script
-- moves the bots first, so a bare `Site::Anywhere` would refuse on every
-- retry, deterministically, for a reason that has nothing to do with the
-- block's geometry.
--
-- So this uses `Site::Near` with a hint 20 tiles from spawn: siting still
-- SEARCHES outward from that point for a clear footprint (this is not
-- `Site::At`, which would pin the one exact anchor) -- it just does not
-- start the search on top of a bot.
print("planning goal.built(MOVING_BLOCK, {near=...}) -- siting still searches for the "
  .. "exact clear tile; only the SEARCH'S OWN SEED is moved off of spawn, where a "
  .. "character bot always stands")
local plan_ok, plan = pcall(goal.plan, goal.built(MOVING_BLOCK, { near = { x = 20, y = 0 } }))
if not plan_ok then
  print("PLAN REFUSED: " .. tostring(plan))
  record.milestone_started(1, "MovingBlock (9 entities) built at a self-chosen site")
  record.milestone_stuck(1, "exhausted", tostring(plan), nil)
  local id = record.finish("incomplete")
  print("RUN FINISHED id=" .. id)
  print("end moving block live")
  return
end

print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))
local placed_plan = plan:count { kind = "place" }
print("planned placements: " .. tostring(placed_plan) .. " (want 9)")

local steps = {}
local planned = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s",
      s.entity, s.pos.x, s.pos.y, tostring(s.direction))
    planned[#planned + 1] = { entity = s.entity, pos = s.pos, direction = s.direction }
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
end
print("planned placements captured from the plan: " .. tostring(#planned))

-- Recover the anchor from the plan's own bounding box, exactly as
-- siting_furnace_live.lua does: the min corner of both sets must correspond
-- to the same physical entity, since the block is a rigid translation.
local rel_min_x, rel_min_y = math.huge, math.huge
for _, e in ipairs(EXPECTED_REL) do
  if e.rx < rel_min_x then rel_min_x = e.rx end
  if e.ry < rel_min_y then rel_min_y = e.ry end
end
local plan_min_x, plan_min_y = math.huge, math.huge
for _, p in ipairs(planned) do
  if p.pos.x < plan_min_x then plan_min_x = p.pos.x end
  if p.pos.y < plan_min_y then plan_min_y = p.pos.y end
end
local anchor = { x = plan_min_x - rel_min_x, y = plan_min_y - rel_min_y }
print(string.format("ANCHOR (recovered from the plan's own bounding box): (%.2f, %.2f)",
  anchor.x, anchor.y))

local source_pos = { x = anchor.x + 0.5, y = anchor.y + 0.5 }
local dest_pos = { x = anchor.x + 8.5, y = anchor.y + 0.5 }
print(string.format("SOURCE chest expected @ (%.2f,%.2f), DESTINATION chest expected @ (%.2f,%.2f)",
  source_pos.x, source_pos.y, dest_pos.x, dest_pos.y))

print("dispatching (this IS a bot action phase -- building the block)...")
local tick_before_build = rcon.game_tick()
local obs = goal.run(plan)
local tick_after_build = rcon.game_tick()
print(string.format("build run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before_build), tostring(tick_after_build),
  (tick_before_build and tick_after_build) and tostring(tick_after_build - tick_before_build) or "?"))

local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
record.teleports()
record.refusals()
record.enclosures()
print(string.format("recorded during build: %d action events, %d walk events", n_actions, n_walks))

record.milestone_started(1, "MovingBlock (9 entities) built at self-sited anchor")
if obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 and (obs.pending or 0) == 0 then
  record.milestone_satisfied(1, 1, "plan_empty")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
  print("BUILD DID NOT FINISH CLEANLY -- see events for detail. Continuing to read back "
    .. "whatever stands, honestly.")
end
print(string.format("done=%s success=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending)))

-- ------------------------------------------------ read the world back --
print("---- reading the world back: name + position + direction, per entity ----")
local footprint_center = { x = anchor.x + 4.5, y = anchor.y + 0.5 }
local NAMES = { "iron-chest", "burner-inserter", "transport-belt" }
local actual_by_key = {}
local total_found = 0
for _, name in ipairs(NAMES) do
  local found = rcon.find_entities_in_radius(footprint_center, 10, name)
  print(name .. ": " .. tostring(#found) .. " found near the chosen anchor")
  for _, e in ipairs(found or {}) do
    total_found = total_found + 1
    local key = string.format("%.1f,%.1f", e.position.x, e.position.y)
    actual_by_key[key] = { name = e.name, position = e.position, direction = e.direction }
  end
end
print("total entities found near the chosen anchor: " .. tostring(total_found) .. " (want 9)")

local matched, wrong_dir, missing = 0, 0, 0
for _, exp in ipairs(planned) do
  local key = string.format("%.1f,%.1f", exp.pos.x, exp.pos.y)
  local act = actual_by_key[key]
  if act == nil then
    missing = missing + 1
    print(string.format("MISSING: %s expected @ (%s) dir=%s -- nothing found there",
      exp.entity, key, tostring(exp.direction)))
  elseif act.name ~= exp.entity then
    missing = missing + 1
    print(string.format("WRONG ENTITY: expected %s @ (%s), found %s", exp.entity, key, act.name))
  elseif tostring(act.direction) ~= tostring(exp.direction) then
    wrong_dir = wrong_dir + 1
    print(string.format("WRONG DIRECTION: %s @ (%s) expected dir=%s, game reports dir=%s",
      exp.entity, key, tostring(exp.direction), tostring(act.direction)))
  else
    matched = matched + 1
  end
end
print(string.format(
  "VERDICT (standing): %d/%d planned placements matched name+position+direction, "
    .. "%d wrong direction, %d missing/wrong entity",
  matched, #planned, wrong_dir, missing))

if missing > 0 or wrong_dir > 0 then
  print("ABORT: the block did not stand as designed -- refusing to proceed to the "
    .. "moving half on a layout that is not the one under test.")
  local id = record.finish("incomplete")
  print("RUN FINISHED id=" .. id)
  print("end moving block live")
  return
end

-- ============================================================ PART 2 ==
-- THE THING UNDER TEST, PART 2: coal moves with no bot in the loop.
--
-- CHEAT, DISCLOSED: bots[1] is given coal it never mined, and that coal is
-- moved directly into the SOURCE chest's inventory by a raw RCON call
-- (rcon.insert_to_inventory), never through goal.run. Neither call is a
-- dispatched action; the destination chest is never touched by this script.
local CHARGE_BOT = bots[1]
local COAL_CHARGE = 20
print(string.format("CHEAT: giving bot %s %d coal and inserting it directly into the SOURCE "
  .. "chest @ (%.2f,%.2f) -- not through goal.run, not a dispatched action",
  tostring(CHARGE_BOT), COAL_CHARGE, source_pos.x, source_pos.y))
rcon.cheat_item(CHARGE_BOT, "coal", COAL_CHARGE)

-- `defines.inventory.chest` == 1 for this game version, confirmed LIVE
-- before this script was trusted with it: a probe placed a real iron-chest,
-- tried every candidate index 0..6 with rcon.insert_to_inventory, and read
-- the chest back with rcon.inventory_contents_at. Only index 1 both
-- succeeded and showed up in get_output_inventory() afterwards; every other
-- index the mod itself refused by name ("cannot insert to nonexisting
-- inventory of entity iron-chest"). See the task report for the transcript.
local INVENTORY_CHEST = 1
rcon.insert_to_inventory(CHARGE_BOT, "iron-chest", source_pos, INVENTORY_CHEST, "coal", COAL_CHARGE)

-- Due diligence on the coordinator's instruction to reuse the standing-goal
-- machinery rather than a home-grown check: `goal.sustain(item, per_minute,
-- window_ticks)` is `Goal::Sustain`, whose only method
-- (crates/planner/src/method/sustain.rs) builds a drill-on-ore + furnace +
-- belt arrangement via `method::produce::cell_spec`, which requires a
-- SMELTING recipe for the item. Coal has no recipe -- it is a raw resource,
-- not a crafted one -- so this is expected to refuse by name
-- (`PlannerError::NoCellProduces`) rather than measure anything about THIS
-- block. Attempted anyway, live, so the finding is measured and not assumed.
local sustain_ok, sustain_err = pcall(goal.plan, goal.sustain("coal", 5, 3600))
if sustain_ok then
  print("UNEXPECTED: goal.sustain('coal', ...) planned successfully: " .. tostring(sustain_err))
else
  print("goal.sustain('coal', ...) refused, as expected for a non-cell item: "
    .. tostring(sustain_err))
end

local tick_charge = rcon.game_tick()
print("charge complete at tick " .. tostring(tick_charge)
  .. ". From here on this script issues ONLY read-only rcon queries "
  .. "(inventory_contents_at, game_tick) -- no goal.run, no insert, no cheat -- "
  .. "until the sampling window below ends.")

-- ------------------------------------------------------- sample, sample, sample --
-- No goal.run call from here to the end of the window: every sample below is
-- a read-only rcon query. Any item that appears in the destination chest was
-- moved by the block itself, not by this script.
-- A 5-belt run plus two inserter swings is measured (not guessed) to take a
-- few hundred ticks end to end, and the first attempt at this (40 samples,
-- ~2 ticks/sample from RCON round-trip pacing alone) covered only 78 ticks
-- -- long enough to see the SOURCE lose its first coal and nowhere near long
-- enough to see it arrive. So this samples up to MAX_SAMPLES times, or until
-- every charged coal has arrived, whichever comes first; only a change in
-- either count is printed, to keep the log readable over what is otherwise
-- a few thousand identical polls.
local MAX_SAMPLES = 4000
local samples = {}
local function coal_count(inv)
  if inv == nil then return -1 end
  local items = inv.output_inventory
  if type(items) ~= "table" then return 0 end
  local n = 0
  for _, it in ipairs(items) do
    if it.name == "coal" then n = n + (it.count or 0) end
  end
  return n
end
local last_printed_src, last_printed_dst = nil, nil
local i = 0
while i < MAX_SAMPLES do
  i = i + 1
  local tick = rcon.game_tick()
  local res = rcon.inventory_contents_at({
    { name = "iron-chest", x = source_pos.x, y = source_pos.y },
    { name = "iron-chest", x = dest_pos.x, y = dest_pos.y },
  })
  local src_coal = coal_count(res[1])
  local dst_coal = coal_count(res[2])
  samples[i] = { tick = tick, src = src_coal, dst = dst_coal }
  if src_coal ~= last_printed_src or dst_coal ~= last_printed_dst then
    print(string.format("SAMPLE %4d: tick=%s source_coal=%s dest_coal=%s",
      i, tostring(tick), tostring(src_coal), tostring(dst_coal)))
    last_printed_src, last_printed_dst = src_coal, dst_coal
  end
  if dst_coal >= COAL_CHARGE then
    print("all charged coal has reached the destination -- stopping the sampling loop early")
    break
  end
end
print("total samples taken: " .. tostring(i))

local first, last = samples[1], samples[#samples]
print(string.format(
  "SAMPLING WINDOW: tick %s -> %s (span %s ticks), source coal %s -> %s, dest coal %s -> %s",
  tostring(first.tick), tostring(last.tick),
  (first.tick and last.tick) and tostring(last.tick - first.tick) or "?",
  tostring(first.src), tostring(last.src), tostring(first.dst), tostring(last.dst)))

if last.dst > (samples[1] and 0 or 0) and last.dst > 0 then
  print("EVIDENCE: the destination chest holds coal that arrived with NO bot in the loop.")
else
  print("NO EVIDENCE OF MOVEMENT: the destination chest never gained coal in this window. "
    .. "Reporting this honestly rather than extending the window to force a result.")
end

local id = record.finish("done")
print("RUN FINISHED id=" .. id)
print("end moving block live")
