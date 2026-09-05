-- Task 4, closing the migration gap: a SYNTHETIC blueprint, hand-built to
-- prove the eight-to-sixteen point direction migration rather than merely
-- confirm that real exports decode (the two real blueprints -- MinerLine,
-- StarterSteamEngineBoiler -- already cover that, and StarterSteamEngineBoiler's
-- own run only exercised the trivial direction-0 identity case: every entity
-- in it was unrotated).
--
-- Four transport-belt at four distinct adjacent tiles in a row, each
-- carrying a different raw PRE-2.0 (eight-point) direction: 0, 2, 4, 6.
-- `import_stack`'s migration doubles a pre-2.0 direction onto the sixteen-
-- point scale, so the belts must arrive facing 0, 4, 8, 12. Any odd value,
-- or a mismatch with this table, is the defect this whole decoder exists to
-- prevent -- a factory that places 100% correctly and moves nothing.
--
--   entity   raw (8-pt, pre-2.0)   expected (16-pt)
--   belt 1   0  (north)            0
--   belt 2   2  (east)             4
--   belt 3   4  (south)            8
--   belt 4   6  (west)             12
--
-- Encoded exactly the way crates/core/tests/blueprint_decode.rs's
-- `encode_blueprint` helper does (version byte '0' + base64 + zlib + JSON
-- body `{"item":"blueprint","version":281474976710656,"entities":[...]}`),
-- reproduced in Python rather than invented from scratch -- see the note for
-- the exact script. `version = 281474976710656` is 1.0.0.0, which is what
-- makes `import_stack` migrate on decode instead of leaving directions
-- alone (a 2.x-versioned blueprint's directions are already on the 16-point
-- scale and must NOT be doubled -- not exercised by this fixture, which is
-- deliberately pre-2.0 only).
--
-- CHEATED, DISCLOSED: every bot gets 4 transport-belt via rcon.cheat_item
-- before planning -- gathering is not what this run tests, and nothing here
-- cheats an ENTITY into the world; every placement is a real goal.run
-- dispatch.
print("start synth run")

local SYNTH = "0eJyNzEEKgzAUBNC7zDotJsbY5ipSirZ/EdAoSSyVkLtX66ZQBVef+cO8iKYdaXDGBugIE6iD/vkxvMh501toceGylNdSlTxThWIgG0ww5KGruIbpbseuIQfNGWzd0WwFV1s/9C6cGmoXcOj9PFvEiDd0di4Ypu9NDE/j6LG2WWJ/rDjM8j1WbLD5YVbssXKDlYfZfI9V6ZbSB1Eyimo="
-- (0,0) refused live (a fixed offline check found the game would refuse
-- transport-belt at [0.5, 0.5] there -- something occupies the spawn tile).
-- (10, 10) is the same bare-dirt anchor the StarterSteamEngineBoiler run
-- built six entities around cleanly; transport-belt is a 1x1 footprint,
-- an easier fit than any of those six.
local ANCHOR = { x = 10, y = 10 }

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

print("CHEAT: giving every bot 4 transport-belt -- gathering is not what this run tests")
for _, b in ipairs(bots) do
  rcon.cheat_item(b, "transport-belt", 4)
end

local run_id = record.start({ video = false })
print("recording run " .. run_id)

local plan = goal.plan(goal.built(SYNTH, ANCHOR))
print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

local steps = {}
local expected_positions = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f)", s.entity, s.pos.x, s.pos.y)
    expected_positions[#expected_positions + 1] = { entity = s.entity, pos = s.pos }
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
end
print("expected placements: " .. tostring(#expected_positions))
for _, e in ipairs(expected_positions) do
  print(string.format("  expect %s @ (%.2f,%.2f)", e.entity, e.pos.x, e.pos.y))
end

print("dispatching...")
local tick_before = rcon.game_tick()
local obs = goal.run(plan)
local tick_after = rcon.game_tick()
print(string.format("run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before), tostring(tick_after),
  (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
record.teleports()
record.refusals()
record.enclosures()
print(string.format("recorded: %d action events, %d walk events", n_actions, n_walks))

record.milestone_started(1, "synthetic 4-belt direction-migration fixture built at "
  .. ANCHOR.x .. "," .. ANCHOR.y)
if obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 then
  record.milestone_satisfied(1, 1, "plan_empty")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
end

print(string.format("done=%s success=%s failed=%s lost=%s pending=%s running=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
print("first_error: " .. tostring(obs.first_error))
if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
  local fs = obs:failures()
  for i = 1, #fs do
    print(string.format("failure[%d] id=%s status=%s attempts=%s error=%s",
      i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts), tostring(fs[i].error)))
  end
end

-- ------------------------------------------------ read the world back --
-- The whole point: read directions off the SURFACE, not the record.
print("---- reading the world back: position + direction, per belt ----")
local found = rcon.find_entities_in_radius(ANCHOR, 10, "transport-belt")
print("transport-belt: " .. tostring(#found) .. " found near anchor")
-- Sort by x so the four print in the same left-to-right order they were
-- authored in (belt 1..4 at x=10.5..13.5).
table.sort(found, function(a, b) return a.position.x < b.position.x end)
local EXPECTED_16PT = { 0, 4, 8, 12 } -- doubled from raw 0, 2, 4, 6
local RAW = { 0, 2, 4, 6 }
local all_match = true
for i, e in ipairs(found) do
  local exp = EXPECTED_16PT[i]
  local ok = (exp ~= nil) and (tostring(e.direction) == tostring(exp))
  if not ok then all_match = false end
  print(string.format("  belt[%d] raw=%s @ (%.2f,%.2f) expected_16pt=%s game_reports=%s %s",
    i, tostring(RAW[i]), e.position.x, e.position.y, tostring(exp), tostring(e.direction),
    ok and "OK" or "MISMATCH"))
end
print(string.format("VERDICT: %s (%d belts found, want 4)",
  (all_match and #found == 4) and "ALL FOUR DIRECTIONS MATCH THE MIGRATED EXPECTATION"
    or "MIGRATION DEFECT OR MISSING BELT -- SEE ABOVE",
  #found))

local id = record.finish(obs.done and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("end synth run")
