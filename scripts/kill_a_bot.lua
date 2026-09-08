-- **Reproduce a bot death on purpose, and check the record can see it.**
--
-- `run-1788833726-34821` is the first run in this project's history in which a
-- bot died. Three did. Waiting for a biter is slow and non-deterministic, so
-- this script does not wait: it opens a window, a shell outside kills a
-- character through RCON while the window is open, and then the run does
-- exactly what any run does with a bot that has no character.
--
-- **THIS RUN CHEATS, DELIBERATELY, AND SAYS SO.** The owner's rule is that
-- cheating while developing is fine and a *measured* run must be honest about
-- it. Nothing here is a measurement: no timing, no production rate and no
-- makespan from this run is comparable to anything. It is a probe with three
-- yes/no questions.
--
-- ---------------------------------------------------------------------------
-- WHAT IT IS ASKING
-- ---------------------------------------------------------------------------
--
-- 1. Does a killed character bot produce a `bot_died` event in the record?
--    `on_player_died` never fires for a character bot -- no `on_player_*`
--    event does, the mod's own doc says so -- so the death has to arrive
--    through the character-bot path (`on_entity_died`), reach
--    `output_parser.rs`, and be drained by `record.deaths()`. Every link in
--    that chain existed before this run and NOT ONE OF THEM had ever carried
--    a real death.
--
-- 2. Is the refusal classified `no_character`, or is it "not connected"?
--    `get_player` asked `connected` before `character`, and for a character
--    bot those are the same fact: the proxy's `connected` is
--    `entity ~= nil and entity.valid`. So a dead bot answered `not
--    connected`, which `classify_walk_failure` files as `Other`. In the
--    archived run that is literally the last thing the record says about bots
--    1 and 2.
--
-- 3. Does the halt say how big it was? A failed walk ends its bot's slice,
--    and the abandoned steps are never dispatched, so they leave no attempt
--    and read as `pending`. That run ended `pending=2041` of 2,295 with
--    nothing anywhere saying where they went.
--
-- A run in which the bot is killed and the plan then finishes normally proves
-- nothing, so the goal is deliberately one that makes every bot WALK.

-- 1,800 ticks was the first attempt and it is **3 seconds of wall clock at
-- 10x** -- the window closed before the shell outside had finished starting a
-- second `factorio-bot` process, and the run went on to finish `automation`
-- 176/176 with nobody dead. Bounded in game ticks, as everything here is, but
-- the bound has to be long enough for a human-scale act: 60,000 ticks is ~100 s
-- of wall at 10x. The window closes EARLY the moment the kill is seen, so the
-- length costs nothing when the kill lands promptly.
local KILL_WINDOW_TICKS = 3000

print("== kill_a_bot: a DELIBERATE death, cheated in, on purpose ==")
print("This run is NOT a measurement. Nothing timed here is comparable.")

-- `rcon.players()` answers an array of bot IDS, not of player tables. Read it
-- as what it is: the first version of this script indexed the elements and
-- died with "attempt to index a number value" after the server was already up.
local before = rcon.players()
print("roster before: " .. tostring(#before) .. " bot(s)")
for _, id in ipairs(before) do print("  bot " .. tostring(id)) end

record.start({})

-- ---------------------------------------------------------------------------
-- THE WINDOW. Bounded in GAME TICKS, never in polls or wall clock: a poll
-- count is not a duration (a faster box covers more game time per RCON round
-- trip), and this repo has published two wrong numbers that way.
-- ---------------------------------------------------------------------------
local opened = rcon.game_tick()
if opened == nil then
  print("no game tick available -- cannot bound the window, giving up rather than")
  print("  substituting a wall clock")
  record.finish("incomplete")
  return
end
print(string.format("KILL WINDOW OPEN at tick %s, closing at %s.", tostring(opened),
  tostring(opened + KILL_WINDOW_TICKS)))
print("  Outside, run:")
print("    factorio-bot rcon -s localhost --settings <settings> -- \\")
print("      \"/c remote.call('botbridge','test') storage.bots[2].entity.die()\"")

-- **The roster is a BAD instrument for this and the record is the good one.**
-- `rcon_players` filters on `player.connected and player.character`, so a dead
-- bot does drop out of it -- for exactly `CHARACTER_RESPAWN_TICKS`, which is
-- 600, one second of wall clock at 10x. Polling it every 3,000 ticks saw
-- roster=4 at every single mark and printed "NO KILL WAS SEEN" over a run that
-- had written a `bot_died` and a `bot_respawned` to `events.jsonl`. The poll
-- is kept, at 60 ticks, as a live progress line; **the verdict comes from
-- `record.deaths()` at the end**, which drains a queue and cannot miss a
-- death that has already happened.
local roster_at_open = #before
local next_mark = opened
local killed_at = nil
while true do
  local now = rcon.game_tick()
  if now == nil or now >= opened + KILL_WINDOW_TICKS then break end
  if now >= next_mark then
    local n = #rcon.players()
    print(string.format("  tick=%s roster=%d", tostring(now), n))
    if n < roster_at_open then
      killed_at = now
      print(string.format("  ROSTER SHRANK %d -> %d: the kill landed at tick %s",
        roster_at_open, n, tostring(now)))
      break
    end
    next_mark = now + 60
  end
end
print("KILL WINDOW CLOSED at tick " .. tostring(rcon.game_tick()))
if killed_at == nil then
  print("no roster shrink was SEEN in the window -- which is NOT the same as no kill:")
  print("  a character bot respawns 600 ticks later, so a kill can land and heal")
  print("  between two polls. record.deaths() at the end is the verdict.")
end

-- ---------------------------------------------------------------------------
-- NOW MAKE EVERY BOT WALK. A dead bot's walk is what goes through
-- `get_player`, and the walk failure is what halts the slice.
-- ---------------------------------------------------------------------------
record.milestone_started(1, "walk the roster with a bot missing its character")

local ok, plan = pcall(function() return goal.plan(goal.researched("automation")) end)
if not ok or plan == nil then
  print("PLAN REFUSED: " .. tostring(plan))
  record.milestone_stuck(1, "refused", tostring(plan), nil)
  record.finish("incomplete")
  return
end

local step_list = plan.steps
print(string.format("PLAN: %d step(s), %d bot(s), makespan=%s",
  #step_list, #plan.bots, tostring(plan.makespan)))

local record_steps = {}
local per_bot = {}
for _, s in ipairs(step_list) do
  per_bot[s.bot] = (per_bot[s.bot] or 0) + 1
  if s.id ~= nil then
    local deps = {}
    -- By TYPE, not truthiness: `Option::None` reaches Lua as mlua's null
    -- sentinel, which is light userdata and therefore truthy.
    if type(s.deps) == "table" then
      for _, d in ipairs(s.deps) do deps[#deps + 1] = d end
    end
    local start = s.start or 0
    record_steps[#record_steps + 1] = {
      id = s.id,
      bot = s.bot,
      action = s.label or s.kind or "step",
      deps = deps,
      planned_start = start,
      planned_duration = (s.finish or start) - start,
    }
  end
end
record_steps.tick = plan.tick
record.plan_created(1, record_steps, plan.bots)
for _, b in ipairs(plan.bots) do
  print(string.format("  bot %s: %s planned step(s)", tostring(b), tostring(per_bot[b] or 0)))
end

local obs = goal.run(plan)

local n_actions = record.actions(step_list, obs.actions)
local n_walks = record.walks(obs.walks)
local n_deaths = record.deaths()
local n_teleports = record.teleports()
print(string.format(
  "recorded: %d action event(s), %d walk event(s), %d death event(s), %d teleport(s)",
  n_actions, n_walks, n_deaths, n_teleports))

print(string.format("done=%s success=%s failed=%s lost=%s pending=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending)))
print("first_error: " .. tostring(obs.first_error))

-- The three answers, said in the run's own output as well as in the record,
-- so a reader who never opens `events.jsonl` still gets them.
print("")
print("== THE THREE QUESTIONS ==")
print(string.format("1. bot_died events written:            %d  (0 means the chain is still cold)", n_deaths))
if n_deaths == 0 then print("   ... no death reached the record, so these answers are about a healthy run.") end
local halted, classified_other = 0, 0
for _, w in ipairs(obs.walks or {}) do
  if w.status == "failed" then
    halted = halted + 1
    local e = tostring(w.error or "")
    if string.find(e, "not connected", 1, true) then classified_other = classified_other + 1 end
    print(string.format("   failed walk: bot %s step %s -- %s",
      tostring(w.bot), tostring(w.step_index), string.sub(e, 1, 120)))
  end
end
print(string.format("2. failed walks saying 'not connected': %d  (any at all is the masking bug)", classified_other))
print(string.format("3. failed walks at all:                %d", halted))
print("   `abandoned` is not on this table -- it is on the walk_settled EVENT.")
print("   Read it with: just analyse <run id>   (the BOT HALTS section)")

-- `record.milestone_satisfied` takes only "already_satisfied" or "plan_empty",
-- and neither describes a probe, so this one is reported as `stuck` with an
-- honest sentence either way. (The first version invented a reason and the
-- recorder refused it AFTER the run was over and every answer was already
-- printed -- a loud failure over a successful probe, which is the right way
-- round but still cost a run.)
record.milestone_stuck(1, "stuck", string.format(
  "probe: %d death event(s), %d failed walk(s), %d of them saying 'not connected'",
  n_deaths, halted, classified_other), nil)

local id = record.finish("incomplete")
print("RUN FINISHED id=" .. id)
print("This run CHEATED a death in. Do not quote a timing from it.")
