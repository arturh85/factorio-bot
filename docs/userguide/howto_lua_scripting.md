# Howto: Lua Scripting

***work in progress***

## Declaring goals

Scripts state what they want and let the planner work out how. The `goal.*`
table is the whole surface:

| Function | What it does |
| --- | --- |
| `goal.have(item_name, count)` | plan for the bots to end up holding items; returns a plan handle |
| `goal.researched(technology_name)` | plan for a technology to be researched, prerequisites and science packs included; returns a plan handle |
| `goal.schedule(plan_handle, bot_count)` | assign the plan's actions to bots over time; returns the makespan in ticks |
| `goal.graphviz(plan_handle)` | render the plan as graphviz source |
| `goal.gantt(plan_handle, title)` | render the *scheduled* plan as a mermaid gantt chart, one section per bot |
| `goal.execute(plan_handle)` | start executing a scheduled plan and return **immediately**; returns a run handle |
| `goal.progress(run_handle)` | a live snapshot: `{pending, running, success, failed, done}` |
| `goal.wait(run_handle)` | block until the run finishes |

Order matters: `gantt` and `execute` both need the plan to have been scheduled
first, and `progress`/`wait` take a *run* handle, not a plan handle.

`goal.execute` is non-blocking, so the bots keep working while the script does
something else. That is about the call, not the script: a run still going when
the script ends is not abandoned — the script's own end blocks until every run
it started has finished.

The exact argument types, return values and error conditions are stated on each
function in the [Lua API
reference](https://arturh85.github.io/factorio-bot/lua/), which is generated
from the bindings themselves — the same reason the sandbox rules below are not
restated here.

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
