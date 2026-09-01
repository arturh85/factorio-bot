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
        local steps = #plan.steps

        if steps == 0 then
            self:_close("satisfied")
            self.state = "acquiring"
            return { action = "satisfied", state = "acquiring",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations }
        end

        self.plan = plan
        self.iterations = self.iterations + 1
        table.insert(self.step_counts, steps)
        self.tracker = tracker.observe(self.tracker, steps)

        local t = {
            action = "planned", milestone_index = self.index, steps = steps,
            best = self.tracker.best, stall = self.tracker.stall,
            iteration = self.iterations,
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
    return { action = "ran", state = "planning", milestone_index = self.index,
             failed = failed, first_error = obs.first_error,
             iteration = self.iterations,
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
