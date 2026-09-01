--- Short-horizon replanning loop.
--
-- Holds long intent across plans: asks a source for the next milestone, plans
-- it, runs it, replans, and moves on when the plan comes back empty. See
-- docs/superpowers/specs/2026-08-31-supervisor-loop-design.md.
--
-- `include` returns nothing, so this file defines the global `supervisor`,
-- the same idiom as lib.lua and the `goal` / `world` / `rcon` tables.
--
--   include("supervisor.lua")
--   local sup = supervisor.new(supervisor.list {
--       goal.researched("automation"),
--       goal.have("steel-plate", 200),
--   }, { bots = {1,2,3,4} })
--   repeat sup:step() until sup:finished()
--   print(sup:report())

supervisor = {}

-- Progress tracking. Pure: no goals, no plans, no I/O, integers only. This is
-- the piece most likely to be subtly wrong -- it is the only state that spans
-- iterations -- so it is isolated where it can be tested with plain numbers.
local tracker = {}
supervisor.tracker = tracker

function tracker.new()
    return { best = nil, stall = 0 }
end

--- Observe a step count. Returns a NEW tracker; does not mutate.
-- An improvement is strictly fewer steps than the best seen. Equal is not an
-- improvement, and an increase is not an improvement -- a plan may legitimately
-- grow when a bot dies or the world reveals a longer path.
function tracker.observe(t, step_count)
    if t.best == nil or step_count < t.best then
        return { best = step_count, stall = 0 }
    end
    return { best = t.best, stall = t.stall + 1 }
end

--- A milestone source over a literal ordered list.
-- The source interface is `function(history) -> goal | nil`, never a list, so
-- that a decomposer can later revise what remains based on what has happened.
function supervisor.list(goals)
    if type(goals) ~= "table" then
        error("supervisor.list: expected a table of goal values")
    end
    local i = 0
    return function(_history)
        i = i + 1
        return goals[i]
    end
end

--- Shapes `plan.steps` (a `PlanValue`'s own field, or the stand-in a test
-- stub hands back) into the array `record.plan_created` expects: `id`,
-- `bot`, `action`, `deps`, `planned_start`, `planned_duration`.
--
-- Takes the steps array itself, not the plan, so a caller who already read
-- `plan.steps` for its length (`#plan.steps`, below) does not pay for a
-- second one: `PlanValue.steps` is a Lua field getter, and each read rebuilds
-- the whole array from the schedule.
--
-- A step lacking `id` is skipped, not erroring -- a walk has no action id to
-- report (see `record.actions`'s own comment on the same point), and a test
-- stub's placeholder step is not a table at all. `deps` is read by type, not
-- by truthiness: `Option::None` reaches Lua as mlua's null sentinel, which is
-- light userdata and therefore truthy, so `s.deps or {}` would not substitute
-- the empty table for a step whose `deps` came from such a bridge.
local function plan_for_record(step_list)
    local out = {}
    for _, s in ipairs(step_list) do
        if type(s) == "table" and s.id ~= nil then
            local deps = {}
            if type(s.deps) == "table" then
                for _, d in ipairs(s.deps) do deps[#deps + 1] = d end
            end
            out[#out + 1] = {
                id = s.id,
                bot = s.bot,
                action = s.label,
                deps = deps,
                planned_start = s.start,
                planned_duration = (s.finish or s.start) - (s.start or 0),
            }
        end
    end
    return out
end

local Sup = {}
Sup.__index = Sup

local TERMINAL = { done = true, stuck = true, stuck_silent = true, exhausted = true }

function supervisor.new(source, opts)
    if type(source) ~= "function" then
        error("supervisor.new: source must be a function(history) -> goal | nil")
    end
    opts = opts or {}
    return setmetatable({
        source = source,
        bots = opts.bots,
        stall_limit = opts.stall_limit or 3,
        max_iterations = opts.max_iterations or 50,
        state = "acquiring",
        index = 0,
        _history = {},
        milestone = nil,
        plan = nil,
        tracker = nil,
        iterations = 0,
        step_counts = nil,
        any_failures = false,
        first_error = nil,
    }, Sup)
end

function Sup:finished()
    return TERMINAL[self.state] == true
end

function Sup:history()
    return self._history
end

function Sup:_close(outcome)
    table.insert(self._history, {
        index = self.index,
        iterations = self.iterations,
        step_counts = self.step_counts,
        outcome = outcome,
        any_failures = self.any_failures,
        first_error = self.first_error,
    })
    -- A keyframe at the boundary of every milestone -- the only place
    -- `map.jsonl` gets one; there is deliberately no tick timer driving it.
    -- Guarded because this loop must keep working for every caller that does
    -- not record: `record` is a global installed only alongside a live game
    -- connection, and even then only a script that called `record.start()`
    -- has one running, which this loop does not do on its own (see the
    -- comment on `actions`/`steps` in `:step()` below -- recording what
    -- happened is the caller's job, not this one's). `record.keyframe()`
    -- itself already answers "no recording is running" and "nothing has been
    -- placed yet" with `false`, not an error -- both are unremarkable here.
    --
    -- `pcall` is for what is left after that: a real failure, the game being
    -- unreachable or a bug in the glue. That must not kill the run either --
    -- a keyframe is a nicety, not core control flow -- but going completely
    -- silent about it is its own failure mode: a run that is actively
    -- recording but whose keyframe raises on *every* call would look
    -- identical to one that never asked for keyframes at all, and nothing
    -- would ever say why. `print_err` is the sandbox's own error-reporting
    -- output (`globals.lua`), which reaches the script job's SSE stream on
    -- the stderr side -- not `writeout`, which belongs to a different
    -- interpreter (the BotBridge mod's).
    if record ~= nil then
        local ok, err = pcall(record.keyframe)
        if not ok and type(print_err) == "function" then
            print_err("record.keyframe() failed at milestone " .. tostring(self.index)
                .. ": " .. tostring(err))
        end
    end
end

--- Perform exactly one action and return a transition record.
function Sup:step()
    if self:finished() then
        return { action = "finished", state = self.state }
    end

    if self.state == "acquiring" then
        -- `next_goal`, not `goal`: `goal` is the global binding table and
        -- shadowing it here would break planning two lines later.
        local next_goal = self.source(self._history)
        if next_goal == nil then
            self.state = "done"
            return { action = "finished", state = "done" }
        end
        self.index = self.index + 1
        self.milestone = next_goal
        self.tracker = tracker.new()
        self.iterations = 0
        self.step_counts = {}
        self.any_failures = false
        self.first_error = nil
        self.state = "planning"
        return { action = "acquired", state = "planning", milestone_index = self.index }
    end

    if self.state == "planning" then
        -- Not wrapped: a raise here is a construction error (unknown item or
        -- technology, a bot outside the roster), not a world condition. It
        -- propagates, costs no iteration, and is not retried -- retrying a typo
        -- burns the cap and then reports "stuck", which actively misleads.
        local plan = goal.plan(self.milestone, { bots = self.bots })
        local plan_steps = plan.steps
        local steps = #plan_steps
        -- Shaped once and carried on `t.plan` for either outcome below, so a
        -- driver can call `record.plan_created(t.milestone_index, t.plan)`
        -- whenever the field is present -- this loop never calls `record.*`
        -- itself (see the module comment), so it hands over the data rather
        -- than the call.
        local plan_for_recording = plan_for_record(plan_steps)

        if steps == 0 then
            self:_close("satisfied")
            self.state = "acquiring"
            return { action = "satisfied", state = "acquiring",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations,
                     plan = plan_for_recording,
                     -- Always "plan_empty", never "already_satisfied" --
                     -- and not a coin flip, a real investigation with a real
                     -- answer. `crates/planner`'s method registry claims a
                     -- `Have`/`Researched` goal as already met
                     -- (`AlreadySatisfied`) before any other method is even
                     -- consulted, and every other method refuses to apply at
                     -- all unless there is a real shortfall -- so *within
                     -- that registry*, today, an empty plan only ever means
                     -- the goal already held. But that is an internal
                     -- invariant of the planner crate, not a contract this
                     -- `goal.*` surface exposes: there is no binding that
                     -- answers "does this goal already hold" independently of
                     -- planning it, so this loop has no way to check the
                     -- invariant, only to assume it -- and asserting
                     -- "already_satisfied" on an assumption it cannot verify
                     -- is exactly the guess `SatisfiedReason` exists to rule
                     -- out. So this reports the one fact it actually
                     -- observed: the plan came back with nothing in it.
                     -- Distinguishing the two for real needs a `goal`
                     -- binding that checks satisfaction without planning
                     -- (nothing in `crates/scripting_lua/src/globals/goal/`
                     -- offers one today), or a way to tell "no applicable
                     -- method but the caller's goal is a vacuous `all{}`"
                     -- apart from "AlreadySatisfied fired for everything" --
                     -- either is a planner-side change, not a Lua-side one.
                     reason = "plan_empty" }
        end

        self.plan = plan
        self.iterations = self.iterations + 1
        table.insert(self.step_counts, steps)
        self.tracker = tracker.observe(self.tracker, steps)

        local t = {
            action = "planned", milestone_index = self.index, steps = steps,
            best = self.tracker.best, stall = self.tracker.stall,
            iteration = self.iterations,
            plan = plan_for_recording,
        }

        -- The stall and the cap mean opposite things and must not share a
        -- name: the stall fires because progress STOPPED, the cap because
        -- progress was still happening and merely slow.
        if self.tracker.stall >= self.stall_limit then
            local outcome = self.any_failures and "stuck" or "stuck_silent"
            self:_close(outcome)
            self.state = outcome
            t.action, t.state = "halted", outcome
            return t
        end
        if self.iterations >= self.max_iterations then
            self:_close("exhausted")
            self.state = "exhausted"
            t.action, t.state = "halted", "exhausted"
            return t
        end

        self.state = "running"
        t.state = "running"
        return t
    end

    -- running: one plan execution. Blocks; one step is one run.
    local plan = self.plan
    self.plan = nil
    -- Captured before the run, because a consumed plan is not a thing to go
    -- reading fields off afterwards.
    local steps = plan.steps
    local obs = goal.run(plan)
    local failed = (obs.failed or 0) + (obs.lost or 0)
    if failed > 0 then
        self.any_failures = true
        if self.first_error == nil then
            self.first_error = obs.first_error
        end
    end
    self.state = "planning"
    -- `steps` and `actions` ride along so a caller can record what each bot
    -- actually did: the plan knows which bot owns an action and what it is
    -- called, the observation knows when the game ran it and how it ended, and
    -- neither half carries both.
    -- The full tally, not just `failed`. A run that dispatched everything and
    -- learned nothing back and a run that dispatched nothing at all both report
    -- `failed = 0`, and they are completely different events -- the first is
    -- alarming, the second is usually a plan whose actions were all walks.
    -- `pending` is what separates them, so a caller should not have to guess.
    return { action = "ran", state = "planning", milestone_index = self.index,
             failed = failed, first_error = obs.first_error,
             iteration = self.iterations,
             done = obs.done, pending = obs.pending, running = obs.running,
             success = obs.success, lost = obs.lost,
             steps = steps, actions = obs.actions }
end

--- Human-readable summary of everything closed so far.
function Sup:report()
    local lines = { "supervisor: " .. self.state }
    for _, h in ipairs(self._history) do
        local best = "-"
        for _, n in ipairs(h.step_counts) do
            if best == "-" or n < best then best = n end
        end
        lines[#lines + 1] = string.format(
            "  milestone %d: %s after %d iteration(s), best %s steps%s",
            h.index, h.outcome, h.iterations, tostring(best),
            h.first_error and (", last error: " .. h.first_error) or "")
    end
    return table.concat(lines, "\n")
end
