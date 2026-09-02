--- Short-horizon replanning loop.
--
-- Holds long intent across plans: asks a source for the next milestone, plans
-- it, runs it, replans, and moves on when the plan comes back empty **and the
-- goal is confirmed to hold**. See
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

--- Is this caught error a planner VERDICT, or a fault?
--
-- A verdict is a statement about the world: the goal cannot be reached from
-- here, and the planner says exactly why -- no route to the item, nowhere to
-- stand while mining it, a research whose lab would have no power. Nothing is
-- broken and nothing is retryable, so the honest thing for this loop to do is
-- close the milestone with the reason and let the run finish. A fault is a
-- defect -- a method contradicting itself, a name that refers to nothing --
-- and it must still end the run loudly. `goal.refusal` (`goal/mod.rs`) is
-- where that classification is made, variant by variant; this only asks.
--
-- `nil` whenever the question cannot be answered: no `goal` table, no
-- classifier on it, a classifier that itself raised, or an answer that is not
-- a table. Every one of those is read as *fault*, because "cannot classify"
-- must never come out as "only a verdict, carry on" -- that is how a real bug
-- gets recorded as a condition of the world. It is the same rule `goal.plan`'s
-- placement pre-check follows: an absent checker must never look like a green
-- answer.
local function refusal_of(err)
    if type(goal) ~= "table" or type(goal.refusal) ~= "function" then return nil end
    local ok, refusal = pcall(goal.refusal, err)
    if not ok or type(refusal) ~= "table" then return nil end
    if type(refusal.message) ~= "string" then return nil end
    return refusal
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
        refusal = nil,
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
        -- Kept apart from `first_error` on purpose. `first_error` is the first
        -- failed ACTION's own text; a refusal is why the milestone closed and
        -- no action ran at all. A milestone can have both (a run that failed,
        -- then a re-plan the planner refused), and collapsing them into one
        -- field would lose whichever was written second.
        refusal = self.refusal,
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
        self.refusal = nil
        self.state = "planning"
        return { action = "acquired", state = "planning", milestone_index = self.index }
    end

    if self.state == "planning" then
        -- Wrapped, and only just: `goal.plan` raises for two different reasons
        -- and exactly one of them is this loop's business.
        --
        -- A **fault** -- a construction error (an unknown item or technology, a
        -- bot outside the roster) or a defect in the planner -- propagates,
        -- costs no iteration, and is not retried. Retrying a typo burns the cap
        -- and then reports "stuck", which actively misleads; and a run that
        -- carries on past a genuine bug reports a world condition for what is
        -- really broken code.
        --
        -- A **verdict** closes the milestone instead. Run 31
        -- (`workspace/runs/run-1788372605-35170/`) is why: it satisfied six
        -- milestones and then asked to research automation in a world with no
        -- electric supply, the planner refused -- correctly, and with the most
        -- informative sentence it has ever produced -- and the raise went
        -- straight past this branch. No `_close`, so no history entry, so the
        -- summary printed milestones 1-6 and no `milestone 7` line at all. The
        -- refusal destroyed the record of itself.
        --
        -- Not retried either, and for a reason of its own: re-planning asks the
        -- same question of the same world -- nothing ran, so nothing changed --
        -- and gets the same answer, until the iteration cap turns a stated
        -- reason into `exhausted`.
        local ok, planned = pcall(goal.plan, self.milestone, { bots = self.bots })
        if not ok then
            local refusal = refusal_of(planned)
            -- `error(err, 0)`: re-raised as the value it was, with no position
            -- prefix bolted on. The original error object -- and the traceback
            -- mlua wrapped it in -- survives for the driver's own pcall.
            if refusal == nil then error(planned, 0) end
            self.refusal = refusal
            -- `stuck`, not `stuck_silent`: that one means "no progress and
            -- nothing to show for it", and this milestone has the planner's
            -- own reason to show. Not a word of its own either -- the outcome
            -- vocabulary (`crates/core/src/record/splits.rs`) says how far a
            -- milestone got, and "as far as the world allows" is `stuck`; what
            -- KIND of stuck it was is `h.refusal`, which is data and does not
            -- need a second verdict word to carry it.
            self:_close("stuck")
            self.state = "stuck"
            return { action = "halted", state = "stuck",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations, refusal = refusal }
        end
        local plan = planned
        local plan_steps = plan.steps
        local steps = #plan_steps
        -- Shaped once and carried on `t.plan` for either outcome below, so a
        -- driver can call `record.plan_created(t.milestone_index, t.plan)`
        -- whenever the field is present -- this loop never calls `record.*`
        -- itself (see the module comment), so it hands over the data rather
        -- than the call.
        local plan_for_recording = plan_for_record(plan_steps)

        if steps == 0 then
            -- An empty plan is not satisfaction. It was the only signal this
            -- loop had, so it stood in for one, and this branch used to carry
            -- a paragraph explaining that the substitution was an assumption
            -- it could not check: within `crates/planner`'s registry
            -- `AlreadySatisfied` claims a met goal before any other method is
            -- consulted and every other method refuses a goal with no
            -- shortfall, so an empty plan did mean the goal held -- but that
            -- is an internal invariant of that crate, not a contract the
            -- `goal.*` surface exposed, and reporting satisfaction on it was
            -- a guess.
            --
            -- `goal.holds` is that contract. It reads the same world snapshot
            -- `goal.plan` reads, for the same roster, and answers the question
            -- directly. The planner pins the agreement between the two
            -- (`an_empty_expansion_and_a_held_goal_agree`), so this is a check
            -- that should never fire -- which is exactly what makes it worth
            -- making, because the run it would have caught reported `done` for
            -- a milestone nothing was done for.
            --
            -- Three answers. `true` is satisfaction, and now says so. `nil` is
            -- "possession cannot settle this goal" -- a production is an event,
            -- not a state -- which is still `plan_empty`, the one fact
            -- actually observed. `false` is a contradiction between the
            -- planner and the world, and there is nothing sensible to do with
            -- it but stop: it is a defect in the planner, not a condition of
            -- the world, so it raises for the same reason `goal.plan`'s own
            -- construction errors are left unwrapped above. Retrying it would
            -- burn the iteration cap and then report "stuck", which would
            -- blame the world for a bug.
            local held = goal.holds(self.milestone, { bots = self.bots })
            if held == false then
                error("supervisor: the planner returned no steps for milestone "
                    .. tostring(self.index) .. " (" .. tostring(self.milestone)
                    .. "), but the goal does not hold; refusing to report it satisfied")
            end
            self:_close("satisfied")
            self.state = "acquiring"
            return { action = "satisfied", state = "acquiring",
                     milestone_index = self.index, steps = 0,
                     iteration = self.iterations,
                     plan = plan_for_recording,
                     reason = held == true and "already_satisfied" or "plan_empty" }
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
            -- The roster the planner actually expanded against, read off the
            -- plan itself rather than from `self.bots`. It is the field
            -- `record.plan_created` records, and nothing downstream of here
            -- can work it out: a driver that left it out got the roster the
            -- *process* was started with, which is a different set whenever
            -- fewer bots connected than were asked for. Run 30 recorded
            -- `bots: [1, 2]` for a plan made for `[2]` alone that way.
            bots = plan.bots,
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
    -- Four counts, two axes, all four kept apart.
    --
    --   failed        the game judged the action and said no
    --   lost          the game acknowledged it and never answered
    --   walks_failed  the same, for a walk (usually: the pathfinder refused)
    --   walks_lost    the same, for a walk (usually: a stuck-walk recovery
    --                 spun until the executor stopped waiting)
    --
    -- The walk terms exist because a walk has no action id, so it is counted
    -- in neither `failed` nor `lost`, and when one does not succeed the rest
    -- of that bot's slice is abandoned -- leaving every action `pending` and
    -- the whole run looking like `failed = 0`. Without them a run that
    -- dispatched nothing at all was reported `stuck_silent` with
    -- `last_error: null`, which says the opposite of what happened.
    -- `run-1788341905-92036` is that run for `walks_failed`;
    -- `run-1788344167-58471` is it for `walks_lost` (four executor deadlines
    -- on one spinning recovery).
    --
    -- `trouble` is the SUM, and it is deliberately a local: it decides whether
    -- this run counts as one that went wrong, and nothing else. It used to be
    -- returned as `t.failed`, next to a separate `t.lost` -- so a run with a
    -- single lost action printed `failed=1 lost=1`, which reads as two
    -- problems, is one, and misdirected a live diagnosis. A name that means
    -- "the game said no" must never carry a total.
    local trouble = (obs.failed or 0) + (obs.lost or 0)
        + (obs.walks_failed or 0) + (obs.walks_lost or 0)
    if trouble > 0 then
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
    --
    -- `walks` rides along for the same reason and is a THIRD thing, not part
    -- of either: a walk is not an action, it has no action id, and it is in
    -- neither `steps` nor `actions`. It is also most of the wall clock, and
    -- until it was carried here no walk reached the run record at all -- a run
    -- could fail three walks and leave one error string behind. See
    -- `record.walks`.
    -- The full tally, not just `failed`. A run that dispatched everything and
    -- learned nothing back and a run that dispatched nothing at all both report
    -- `failed = 0`, and they are completely different events -- the first is
    -- alarming, the second is usually a plan whose actions were all walks.
    -- `pending` is what separates them, so a caller should not have to guess.
    --
    -- The four trouble counts are normalised to numbers rather than passed
    -- through: a driver prints these, and `nil` printed as "nil" reads as
    -- "unknown" where "none" is what happened.
    return { action = "ran", state = "planning", milestone_index = self.index,
             failed = obs.failed or 0, lost = obs.lost or 0,
             walks_failed = obs.walks_failed or 0,
             walks_lost = obs.walks_lost or 0,
             first_error = obs.first_error,
             iteration = self.iterations,
             done = obs.done, pending = obs.pending, running = obs.running,
             success = obs.success,
             steps = steps, actions = obs.actions, walks = obs.walks }
end

--- Human-readable summary of everything closed so far.
function Sup:report()
    local lines = { "supervisor: " .. self.state }
    for _, h in ipairs(self._history) do
        local best = "-"
        for _, n in ipairs(h.step_counts) do
            if best == "-" or n < best then best = n end
        end
        -- One suffix, and the halt's own reason leads. A refusal is not an
        -- error the run made -- no action was dispatched, nothing failed -- so
        -- calling it "last error" would invent a failure that never happened;
        -- and a milestone that both failed a run and was then refused reports
        -- the refusal, because that is what closed it. The other text is still
        -- on the history entry for a reader who wants it.
        local why = ""
        if h.refusal then
            why = ", refused: " .. h.refusal.message
        elseif h.first_error then
            why = ", last error: " .. h.first_error
        end
        lines[#lines + 1] = string.format(
            "  milestone %d: %s after %d iteration(s), best %s steps%s",
            h.index, h.outcome, h.iterations, tostring(best), why)
    end
    return table.concat(lines, "\n")
end
