# Supervisor Loop — Design

**Status:** **IMPLEMENTED, and outgrown — history, not a work item.** Corrected 2026-09-03. `scripts/supervisor.lua` (929 lines) ships, driven by `crates/scripting_lua/src/supervisor_lib.rs` (1706 lines, 51 tests), which `include_str!`s the shipped file *inside the sandbox* (`supervisor_lib.rs:16-19,30`) — stricter than D5 asked. **Two of its premises are now false:** premise 3 (*"`Goal::Producing` has no method"*) — two methods claim it (`crates/planner/src/method/have.rs:1992,1997`) and the test it cites, `producing_has_no_method_in_this_increment`, no longer exists; and D3 (*an empty plan is satisfaction*) — the goal must **also** be confirmed to hold (`scripts/supervisor.lua:4-5`), pinned by `an_empty_plan_for_an_unheld_goal_is_refused_rather_than_reported_satisfied`. The "Future path" section is stale for the same reason.
**Date:** 2026-08-31

## Why

`goal.plan` / `goal.run` execute *one* plan against *one* world snapshot. Nothing
holds intent across plans, so every long-horizon question — "research automation,
then logistics, then get 200 steel" — is a thing a human types one line at a time.

Planning to the end instead is not the answer, for three independent reasons:

1. **Estimates drift.** Every quantity measured against a live game on 2026-08-30
   diverged from its prediction (mining 2x, walks 1.4-1.6x). A plan is a
   prediction; prediction error compounds with horizon.
2. **The snapshot is incomplete by construction.** Factorio reveals the map
   through exploration, and `crates/planner` has no concept of *uncharted* — it
   cannot distinguish "no ore here" from "we have not looked here". The
   information needed to plan the late game does not exist at tick 0.
3. **`Goal::Producing` has no method.** `producing_has_no_method_in_this_increment`
   asserts it. The only goal that plans is `Have`, expanded to hand-mining and
   hand-crafting. There is no factory in the model, so there is no rocket at any
   horizon.

So: short horizons, replanned. Planning is ~1s, cheap enough to redo constantly.
This spec defines the loop that does the redoing.

## Scope

**In:** an ordered-milestone driver that plans, executes, replans on completion or
failure, detects non-convergence, and reports honestly.

**Out, deliberately:**

- **Preemption.** Replanning happens *between* plans, never inside one. The
  executor has no cancel path and this design does not add one.
- **Therefore, threat response.** Reacting to biters requires cancelling work in
  flight. That is the next design and it needs executor cancellation first.
  Folding it in here would be a lie about what the loop can do.
- **Derived milestones.** See "Future path".
- **`Goal::Producing`.** Separate and much larger.

## Decisions

### D1 — Scripted milestones, behind a function interface

The milestone source is `source(history) -> goal | nil`, never a literal list.

A list would bake in "the remaining milestones are known upfront and never
revised". A decomposer must be able to revise what remains — you learn at
milestone 4 that an ore estimate was wrong and milestone 7 has to change. The
function signature supports mid-run revision from day one and costs nothing now.

`supervisor.list(goals)` is a closure over an index, for the scripted case.

Note the gap is narrower than it looks: the planner already derives *within* a
milestone (`goal.researched("automation")` expands to 105 steps unaided). What is
missing is derivation *across* milestones.

### D2 — Replan on completion or failure only

No mid-plan abandonment. This is what keeps preemption out of scope. Cost: a plan
drifting 3x slow runs to completion before anyone notices. Accepted for v1.

### D3 — Satisfaction is an empty plan

`satisfied(goal) := #goal.plan(goal, opts).steps == 0`

Uniform across goal kinds, and it asks the planner's own view of the world rather
than inventing a second source of truth that can disagree with the thing
generating the plans. It also sidesteps a real gap: **research state is not
readable from Lua at all** — there is no `world.technology`. Replanning is the
only check available for `goal.researched`, and it happens to be the right one.

Already covered by planner tests: `a_technology_the_world_already_has_expands_to_nothing`,
`an_already_held_item_expands_to_nothing`, `an_empty_network_schedules_to_zero`.

### D4 — Convergence: best-so-far no-progress, with a cap backstop

Track the smallest step count seen for the current milestone. `stall_limit`
consecutive iterations that set no new best means stuck. `max_iterations` is a
backstop against pathological oscillation.

A bare iteration cap was rejected: N means something different for a 5-step goal
than a 500-step one, and hitting it cannot distinguish "impossible" from "nearly
done". "Steps stopped going down" is a reportable diagnosis.

### D5 — A Lua library, tested from Rust

`scripts/supervisor.lua`. Policy stays readable and editable without a rebuild.

There is no automated Lua suite, which would normally make this the least-tested
part of a system with 929 Rust tests. It does not, because the pattern already
exists: `crates/scripting_lua/src/globals/goal/recovery.rs` tests install a stub
`goal` table and run Lua against it.

**The Rust tests load the shipped file** via
`include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/supervisor.lua"))`,
not a copy. A copy would drift, and the tests would then pass against code that is
not what runs.

### D6 — Stuck halts the run; it does not skip the milestone

Milestones are ordered because they depend on each other. Skipping produces a
cascade of downstream failures that all trace back to a cause already detected and
discarded.

## Architecture

The supervisor sits above `goal.*` and below the user's script. Its **only** view
of the world is the planner's view, through `goal.plan`. It does not query RCON or
entities directly — deliberately, so there is no second source of truth.

```lua
include("supervisor.lua")

local sup = supervisor.new(supervisor.list {
  goal.researched("automation"),
  goal.researched("logistics"),
  goal.have("steel-plate", 200),
}, { bots = {1,2,3,4}, stall_limit = 3, max_iterations = 50 })

repeat sup:step() until sup:finished()
print(sup:report())
```

`include` returns nothing (see `globals.include`), so `supervisor.lua` defines a
global `supervisor` table — the same idiom as `lib.lua`, `goal`, `world`, `rcon`.

## Components

### Milestone source

`source(history) -> goal | nil`. `nil` means the intent is complete.

No world argument: there is no world *value* to pass, and inventing one would be
fiction. A source reads the world through the `world.*` globals if it wants to.

`supervisor.list(goals)` returns a source over a literal ordered list.

### Progress tracker

Pure, and the only component with no dependency on goals, plans or I/O:

```lua
tracker.new()               -> { best = nil, stall = 0 }
tracker.observe(t, n)       -> new tracker (does not mutate)
```

Improvement iff `t.best == nil or n < t.best`. On improvement: `best = n`,
`stall = 0`. Otherwise `stall = t.stall + 1`.

Equal is **not** an improvement. An increase is **not** an improvement.

This is the piece most likely to be subtly wrong — stateful across iterations —
and isolating it as a pure integer function is most of why it will not be.

### Step machine

`sup:step()` performs exactly one action and returns a transition record.

```
acquire  -> source(history)
              nil        -> state = "done"
              goal       -> state = "planning"

plan     -> goal.plan(milestone, {bots = opts.bots})
              #steps == 0 -> record "satisfied", -> acquire
              else        -> tracker.observe(tracker, #steps)
                               stall >= stall_limit    -> terminal (see below)
                               iterations >= max_iters -> terminal (see below)
                               otherwise               -> state = "running"

run      -> goal.run(plan) -> observation
              record failed / first_error   -> state = "planning"
```

**An *iteration* is one plan that produced work.** The counter increments when a
plan comes back with more than zero steps — not on a satisfied plan, and not
separately for the run that follows. The tracker and the iteration counter both
**reset when a milestone is acquired**, so limits are per milestone, not per run.

**Which terminal state.** The stall and the cap mean opposite things and must not
share a name. The stall fires because progress *stopped*; the cap fires because
progress was *still happening and merely slow*.

- stall tripped, any run reported `failed > 0` or `lost > 0` -> `stuck`
- stall tripped, every run reported success                  -> `stuck_silent`
- cap tripped (best still improving)                         -> `exhausted`

Calling an `exhausted` run `stuck_silent` would accuse the executor of lying about
work that was in fact getting done, which is the opposite of the diagnosis.

Planning and running are separate steps so a test can assert on the plan before it
executes, and so a dry-run mode falls out for free: step until
`state == "running"`, inspect, stop.

**`step()` may block for minutes.** One step is one plan execution via blocking
`goal.run`. This is honest rather than hidden. `goal.start` is where preemption
lands later; using it now buys nothing without a `:cancel()`.

### Transition record

```lua
{ action   = "acquired" | "planned" | "ran" | "satisfied" | "stuck" | "finished",
  state    = "acquiring" | "planning" | "running"
           | "done" | "stuck" | "stuck_silent" | "exhausted",
  milestone_index = integer | nil,
  steps    = integer | nil,   -- on "planned"
  best     = integer | nil,
  stall    = integer | nil,
  iteration = integer | nil,
  failed   = integer | nil,   -- on "ran"
  first_error = string | nil }
```

Terminal states: `done`, `stuck`, `stuck_silent`, `exhausted`. `sup:finished()`
returns whether the state is terminal, so callers never enumerate them.

### History and report

Per milestone:

```lua
{ index = integer, iterations = integer, step_counts = { integer... },
  outcome = "satisfied" | "stuck" | "stuck_silent" | "exhausted",
  any_failures = boolean, first_error = string | nil }
```

`sup:history()` returns the structured form; `sup:report()` a human-readable
summary.

## Error handling

The load-bearing distinction is **construction errors versus world conditions**,
because they need opposite responses.

**Class 1 — `goal.plan` raises** (unknown item, unknown technology, a bot outside
the roster). A script bug surfacing late, since goal values are pure until they
meet a world. **Propagate immediately. Do not retry. Do not count an iteration.**
Retrying a typo burns `max_iterations` and then reports "stuck", a diagnosis that
actively misleads.

**Class 2 — `obs.failed > 0` or `obs.lost > 0`.** A world condition, and the normal
replan trigger. Record `first_error`, return to planning. The next plan is computed
against the world as it now is, so a mine that failed on exhausted ore replans onto
different ore without anyone encoding that rule.

**Class 3 — no progress.** Terminal. The report carries best-so-far, the current
step count, iterations used, **and the last error seen** — "stuck at 42 steps, last
error: no entity to mine" is a diagnosis; "stuck" is not.

**Class 4 — the source raises, or `goal.run` raises.** Propagate. A `goal.run`
raise means a consumed plan was run twice, impossible by construction here since a
fresh plan is built every iteration. If it fires it is a supervisor bug and should
be loud.

That last point has a useful consequence: the consumed-plan flag means the tempting
"optimization" of caching a plan across iterations **fails loudly rather than
silently re-running stale work.**

### `stuck_silent`

If every run reports `failed == 0` but the plan never shrinks, the executor is
claiming success for work that did not happen — the defect class chased on
2026-08-30. The loop detects it for free, so it gets its own terminal state:

- `stuck` — no progress, with failures. Ordinary: something in the world blocks.
- `stuck_silent` — no progress, **every run reported success**. Alarming:
  something is lying.
- `exhausted` — the cap tripped while the best was still improving. Neither
  alarming nor ordinary: the budget ran out, and raising it may well finish.

Same loop, three very different messages to wake up to.

## Testing

### Layer 1 — tracker, plain integers

No goals, no Lua stubs.

- a new best resets stall to 0
- an equal count is not an improvement (stall increments)
- an increase is not an improvement (stall increments)
- stall fires at exactly K, not K-1 or K+1
- the cap fires independently of stall

**Mutation targets**, each proven to fail its own test individually against a
baseline captured fresh on the current tree: `<` vs `<=` in the best comparison,
`>=` vs `>` in the stall check.

### Layer 2 — step machine against a stub `goal` table

The `recovery.rs` pattern: a stub whose `plan` returns a scripted sequence of step
counts and whose `run` returns scripted observations.

- satisfied immediately (0 steps) -> `done`, and **never calls run**
- converges: 10 -> 6 -> 0, satisfied after exactly two runs
- stalls: 10, 10, 10, 10 with `stall_limit = 3` -> `stuck`, and **no fourth run**
- cap backstop: improving by one step every iteration, cap 5 -> `exhausted` at 5
  (never `stuck_silent`, even though every run succeeded — the discriminating case
  for the split above)
- a plan raise propagates, with no retry and no iteration counted
- the source is called again after a milestone is satisfied
- `stuck_silent` is reported distinctly when every run succeeded

### Layer 3 — live smoke

`scripts/supervisor_smoke.lua`, two or three small milestones against a real game.
Not a CI test; it is the first-contact probe, and it is where the real defects will
appear.

## Future path

The seam is D1. Derived milestones implement the same `source(history)` signature;
the loop, the tracker, the error classes and the reporting are untouched.

What blocks derivation is not effort but modelling: deciding "to get automation
science at 150/min I need N assemblers, M furnaces, this much ore throughput" *is*
`Goal::Producing`. Until that exists, a decomposer would be designed against a
planner that cannot answer the questions it asks.

Hand-writing the milestone list is therefore not busywork — it is how we find out
what a decomposer needs to know, and where the planner lies.
