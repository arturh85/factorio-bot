# Goal Values: a composable, inspectable scripting surface

**Status:** approved 2026-08-30. Replaces the handle-based `goal.*` Lua API.

## The problem

The capability to script long-horizon goals nearly exists. `goal.researched("automation")`
already decomposes into science packs, crafting and mining; `Goal::All` exists in the
planner. What is missing is not capability but *surface*:

```lua
local p = goal.have("iron-plate", 5)   -- opaque u32 handle
goal.schedule(p, 4)                     -- mutates the handle
local r = goal.execute(p)               -- another opaque handle
local f = goal.wait(r)                  -- counts only
```

Three consequences, each of which has cost real time:

1. **Goals do not compose.** `Goal::All` is modelled in the planner and unexposed, so a
   script cannot ask for two things at once.
2. **Plans cannot be inspected.** A handle yields a graphviz *string*. Nothing can assert
   "this plan places at most four furnaces" or "no placement lands outside these bounds".
3. **Planning and scheduling are separate calls over separate rosters.** A plan expanded
   for four bots and scheduled onto one failed a per-bot precondition — a real defect that
   took a live game and a bisect-by-hand to find.

The third is the important one, because it is a *shape* problem: the API made an invalid
state expressible. This design removes it.

## Principle

**Goal, plan, observation are values.** Each is a plain Lua table you can build, print,
inspect and assert on. Effects happen in exactly one place — running a plan.

The property that makes this worth doing is not aesthetic. Planning is pure and needs no
game process beyond a world snapshot: a full four-bot red-science plan takes **~1 second**
headless. A surface that hands back inspectable plans turns that into a test loop.

## The surface

### Goals are pure constructors

```lua
goal.have(item, count)              -- anyone may hold it
goal.have(item, count, { bot = 2 }) -- bot 2 specifically must hold it
goal.researched(tech)
goal.all { g1, g2, ... }
```

They call nothing, cannot fail on the world, and nest freely. Each returns a table with a
metatable providing `__tostring`, so `print(g)` gives `all { have 50 iron-plate, researched
automation }`.

`{ bot = id }` maps to `Holder::Bot`; its absence maps to `Holder::Anyone`. `Holder::Share`
stays internal — it is a marker the expansion uses, not a thing a caller asks for.

**Shape errors raise at construction.** A negative count, a non-string item, an empty
`all {}` fail on the line that contains the mistake. **Semantic errors raise at plan
time**, because they need a world: an unknown item, a technology no method can satisfy, a
bot the roster does not have.

### Planning is one call

```lua
local plan = goal.plan(g)                        -- every bot in the run
local plan = goal.plan(g, { bots = { 1, 2 } })   -- a subset
```

`goal.plan` expands **and** schedules. There is no way to expand over one roster and
schedule over another, so the defect that motivated this design is unrepresentable rather
than merely fixed.

`bots` is a **list of bot ids**, defaulting to the whole roster. It is deliberately not a
count: `bots = 4` would mean "some four of them" and reintroduce the ambiguity this call
exists to remove.

### A plan is one filterable collection

```lua
plan.makespan          -- ticks
plan.bots              -- { 1, 2, 3, 4 }
plan.steps             -- array, in schedule order

plan:count(pred)       -- how many steps match
plan:find(pred)        -- the steps that match
plan:for_bot(id)       -- that bot's steps, in order
plan:graphviz()        -- lazy; builds the string only when asked
plan:gantt(title)      -- lazy
```

A step is one table shape, with `kind` discriminating:

```lua
{ kind="place",  bot=1, start=0,  finish=30, id=0, entity="stone-furnace",
  pos={x=39,y=-13}, label="place stone-furnace at [39, -13]" }
{ kind="walk",   bot=1, start=30, finish=61, to={x=40,y=-10} }
{ kind="mine",   bot=2, start=0,  finish=94, id=7, item="iron-ore", count=5, pos={...} }
```

Every step carries `kind`, `bot`, `start`, `finish`. Action steps also carry `id`
and `label`; `walk` carries neither, because a walk is not an action. The rest is
per kind, transcribed from `ActionKind`:

| `kind` | extra fields |
| --- | --- |
| `walk` | `to` |
| `mine` | `pos`, `item`, `count` |
| `craft` | `item`, `count` |
| `place` | `entity` (the prototype name), `pos` |
| `insert` | `pos`, `entity`, `slot`, `item`, `count` |
| `remove` | `pos`, `entity`, `slot`, `item`, `count` |
| `research` | `tech` |

**`finish`, not `end`.** `end` is a Lua keyword: `step.end` does not parse, and a
field only reachable as `step["end"]` is a trap. The Rust field stays
`ScheduledStep::end`; the Lua name diverges deliberately.

**Building a step is a join.** `ScheduledStep` holds only `action: ActionId` and a
`label` — `entity`, `item`, `count`, `pos` live on the `Action` in the
`ActionNetwork`. So a plan value retains *both* the network and the schedule and
joins them once, at construction. `place` flattens `ActionKind::Place`'s
`FactorioEntity` to its `name` and `position`, because a script asserting on a
plan wants those two and not a whole entity record.

**One collection, not two.** A `Schedule` has steps — including walks, which have no
action id — while an `ActionNetwork` has actions. Exposing both would make every
query ask which one it meant. Steps are the executable thing, so steps are what a
script sees.

**One predicate shape.** `pred` is a table; a step matches when every key in `pred`
equals the step's value, comparing scalars only:

```lua
assert(plan:count { kind = "place", entity = "stone-furnace" } <= 4)
for _, s in ipairs(plan:find { kind = "place" }) do assert(in_bounds(s.pos)) end
```

This replaces a family of specific accessors (`plan.furnaces_placed()` and its future
siblings) with one idea. A predicate key that no step of any kind defines is a
script bug rather than a query that matches nothing, so it raises.

### Running is the only effect

```lua
local obs = goal.run(plan)         -- start and wait; the common case
local run = goal.start(plan)       -- non-blocking; returns a run value
local snap = run:progress()        -- an observation, now
local obs  = run:wait()            -- the final observation
```

`goal.run(plan)` is exactly `goal.start(plan):wait()`. `goal.start` stays non-blocking —
that was decided deliberately and is not reversed here — but returns a value with methods
rather than an opaque integer.

An observation:

```lua
obs.done                     -- boolean
obs.success, obs.failed, obs.pending, obs.running
obs.first_error              -- string or nil; convenience for assert messages
obs.actions[id]              -- { status, attempts, error, planned_start, planned_end }
obs:failures()               -- the failed actions, with their errors
```

`obs.actions` is keyed by the same `id` a step carries, so an observation joins to its plan
without a mapping layer.

**`planned_start` / `planned_end`, never `observed_*`.** These are scheduler estimates. The
executor has no game-clock source, and a field named for measurement that carries an
estimate is the exact defect corrected earlier in this project. Real observed timings are
named as follow-up work below.

### Running a plan twice raises

A plan is computed against a world state. Running it again after that state has changed
would re-issue placements and mining against a world that no longer matches. The second
call raises. To retry, re-plan.

The error does **not** name the first run: a run value carries a log, a network and a
completion signal, and no identity a second caller could be pointed at, so there is nothing
truthful to name. It says the plan is spent and that a plan may be executed at most once.

**Only a run that actually dispatches spends the plan.** `goal.start` can fail for reasons
that are not the plan — no game connected, so no actuator — and those must leave the plan
runnable, so the next attempt reports the real cause rather than a plan the previous
attempt burned on nothing.

## Testing

The plan half needs no game process beyond a world, and planning is ~1 second:

```
factorio-bot lua my_script.lua --clients 0 --bots 4
```

A script that only plans is a test. It asserts on `plan.steps` with `assert` and the two
queries; no framework is introduced, because Lua already has one that fits.

Deliberately **not** added: a Lua assertion library, custom matchers, or a test runner.
`assert` plus `plan:count`/`plan:find` covers the cases this surface has.

## Migration

This replaces the current surface rather than sitting beside it; two surfaces would defeat
the purpose. The eight scripts under `scripts/` that call `goal.*` — `lib.lua`,
`example.lua`, `api_test.lua`, `goal_smoke.lua`, `exec_smoke.lua`, `attach_smoke.lua`,
`test_phase_2_1.lua`, `test_phase_2_2.lua` — are migrated with it, and the commit carries a
`BREAKING CHANGE:` footer with this table — the changelog is generated from commit messages,
and a script calling the old API fails at runtime, not at build time.

| old | new |
| --- | --- |
| `goal.have(i, c)` | `goal.have(i, c)` — returns a value, not a handle |
| `goal.researched(t)` | `goal.researched(t)` — same |
| `goal.schedule(h, n)` | `goal.plan(g, { bots = ... })` |
| `goal.graphviz(h)` | `plan:graphviz()` |
| `goal.gantt(h, t)` | `plan:gantt(t)` |
| `goal.execute(h)` | `goal.start(plan)` |
| `goal.progress(r)` | `run:progress()` |
| `goal.wait(r)` | `run:wait()` |
| — | `goal.all { ... }` (new) |
| — | `goal.run(plan)` (new) |
| — | `plan:count`, `plan:find`, `plan:for_bot` (new) |

Blast radius is those scripts. The HTTP effort calls `run_lua`, not `goal.*`.

## Out of scope, deliberately

- **A supervisor layer** — retry policy, budgets, progress callbacks. It belongs *on top of*
  inspectable plans. Built first it would get an opaque substrate and be hard to debug when
  it misbehaves; built after, it is easy.
- **`Goal::Producing { item, rate }`** — declared in the planner, claimed by no method.
  Exposing it would add a constructor whose plan always errors.
- **Observed ticks.** `Attempt` already documents why `planned_*` cannot become a
  measurement here: `Actuator` returns `Result<(), ActuatorError>` with no tick in it,
  and nothing in the executor reads `game.tick`. The fix is to widen `Actuator` so a
  dispatch returns the game tick — cheapest as a `tick` field on every BotBridge RPC
  response, since the executor already pays that round trip. This has a **named
  consumer**: the peer session's screenshot scrubber aligns captured frames to plan
  ticks and will show drift as what looks like a rendering bug. It is the increment
  after this one, not part of it.
- **Planning against a snapshot file.** The RCON `world_snapshot` path could be persisted and
  replayed, giving script tests that run in milliseconds with no Factorio at all. The natural
  next increment; not this one.

## Risk

`goal.run` will be exercised against a game that **has never accepted a single action**.
Execution's first contact is unproven, and first-contact defects are expected. That is the
value of driving it live rather than a side effect of doing so.
