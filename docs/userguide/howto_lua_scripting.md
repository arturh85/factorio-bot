# Howto: Lua Scripting

***work in progress***

## Declaring goals

Scripts state what they want and let the planner work out how. Nothing in the
`goal.*` table is a handle: every call below hands back an ordinary value the
script can inspect, print and pass around.

| Function | What it does |
| --- | --- |
| `goal.have(item_name, count, opts)` | builds a **goal value**: the bots end up holding items. Pure — checked against the world only at `goal.plan` |
| `goal.researched(technology_name)` | builds a goal value: a technology is researched, prerequisites and science packs included |
| `goal.all(goals)` | builds a goal value combining several sub-goals, planned together so shared work is done once |
| `goal.plan(goal, opts)` | expands a goal value and schedules it against one roster, in one call; returns a **`PlanValue`** |
| `goal.start(plan)` | starts executing a `PlanValue` and returns immediately; returns a **`RunValue`** |
| `goal.run(plan)` | exactly `goal.start(plan):wait()` — starts a plan and blocks until it finishes; returns an observation table |

A `PlanValue` carries the schedule it was given and answers questions about
it directly as fields and methods: `plan.makespan`, `plan.bots`, `plan.steps`,
`plan:count{...}`, `plan:find{...}`, `plan:for_bot(id)`, `plan:graphviz()`,
`plan:gantt(title)`. Expansion and scheduling happen together in `goal.plan`
— there is no separate "schedule this plan" call — because a network expanded
for one roster only makes sense scheduled on that same roster; combining the
two calls makes that mismatch unrepresentable.

A `RunValue` is what `goal.start` returns: poll it with `run:progress()` for a
live snapshot, or block with `run:wait()`, which returns the same observation
table `goal.run` returns directly: `{ done, pending, running, success, failed,
first_error, actions, failures }`. A `PlanValue` may be taken for a run only
once — a second `goal.start`/`goal.run` on the same plan raises.

`goal.start` is non-blocking, so the bots keep working while the script does
something else. That is about the call, not the script: a run still going when
the script ends is not abandoned — the script's own end blocks until every run
it started has finished.

The exact argument types, return values and error conditions are stated on each
function in the [Lua API
reference](https://arturh85.github.io/factorio-bot/lua/), which is generated
from the bindings themselves — the same reason the sandbox rules below are not
restated here.

## Known gaps

- **`goal.researched` does not currently work against a live Factorio 2.1.17
  Space Age game.** `goal.researched("automation")` expands correctly into a
  goal that needs 10 `automation-science-pack`, but that expansion then fails
  because `world.recipe("automation-science-pack")` is `nil` — the recipe is
  missing from world data entirely. The root cause is
  `mods/BotBridge/control.lua:487`'s `collect_recipes()`, which serialises
  only recipes where `rec.enabled`; a recipe unlocked by the very technology
  being researched is therefore never sent to the planner. It is not a
  `research_trigger` problem and not a recipe-category problem. Fixing this is
  a separate increment.

- **Planning is pure and headless.** `goal.plan` (and everything upstream of
  it — `goal.have`, `goal.researched`, `goal.all`) touches neither RCON nor
  wall-clock time, so a script that only plans can run with no game at all:
  `factorio-bot lua <script>.lua --clients 0 --bots N` finishes in about a
  second. A script that stops at `goal.plan` (or calls `plan:graphviz()` /
  `plan:gantt()` and asserts on the output) is therefore a fast, deterministic
  test — only `goal.start`/`goal.run` need a connected game.

## The sandbox

Scripts do not run in a stock Lua interpreter. They are reachable from an
unauthenticated HTTP endpoint, so the standard library is an allow-list:
`table`, `string`, `math` and `coroutine` are available, `io`, `os` and
`package` are not loaded, `require`, `dofile` and `loadfile` are removed, and
`load` compiles source text only — it refuses binary chunks.

That leaves `include`, `file_read`, `file_write` and `world.draw` as the only
way to reach the filesystem, and all four are bounded to the scripts
directory. Paths are relative to the calling script, and a path that would
leave the scripts directory is refused rather than clamped.

The per-function rules — which paths must already exist, how symlinks are
treated — are stated on each function in the [Lua API
reference](https://arturh85.github.io/factorio-bot/lua/), which is generated
from the bindings themselves. They are deliberately not repeated here, because
a second copy is a copy that goes stale.
