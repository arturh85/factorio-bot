# Plan/map view — consolidated requirements before anyone designs it

Next work item after plan 6, per the user's brief. Written now because the
context is expensive to reconstruct and most of it came from the peer session,
which will not be around.

## The container already exists and already states its intended shape

`app/src/components/GanttChart.vue` is a `<div>TODO</div>` stub whose
**commented-out body is a mermaid gantt string** with `section Bot 1` /
`section Bot 2`, `Walk to [10, 43] : 45s`, `Mining rock-huge : 3s`, `Start﹕/End`
markers and a `milestone`. That is a design someone wrote down and stopped —
honest, unlike tonight's misleading artifacts, because it is labelled TODO.
Plan 6 Task 10 restyles its container only and leaves the stub. Reachable at
`/tasks`, already in the menu.

## What data exists NOW (all landed tonight, all measured not estimated)

  - **observed action ticks** — `ExecutionLog`, per action
  - **observed walk ticks** — `ExecutionLog.walks`,
    `BTreeMap<BotId, BTreeMap<usize, WalkObservation>>`, keyed `(bot, step_index)`
  - **`Status::Lost`** — a fifth state, Lua string `"lost"`, plus `obs.lost`

## Rendering rules settled by measurement — do not re-derive

1. **Everything observed, nothing inferred.** Walk bars observed exact; the
   gap remainder is wait, obtained by subtracting two OBSERVED quantities;
   action bars observed.

2. **Never subtract a planned quantity from an observed one.** Walks were
   wrong by 1.37–1.58x and actions by 2.01x, so the error had no stable sign.
   (After `0d93dc3b` actions track observation at ~1.01x; the rule stands
   because the reason it existed can recur.)

3. **Walk drift is a FIXED ~135-tick per-walk overhead, not proportional
   error.** Established by fitting multiplicative and additive models on walk 1
   and predicting walk 2: multiplicative residual +76 ticks, additive -3. The
   `0.15` tiles/tick rate is correct. **Consequence for rendering: short walks
   are dominated by the constant and long walks are nearly accurate.** A single
   ratio would be most wrong exactly where walks are shortest and most numerous.

4. **`Status::Lost` legend footnote, verbatim:**
   *Lost appearing is trustworthy. Lost not appearing is not proof the run
   didn't lose track.*
   Two reasons: it is plumbed-and-tested but not field-verified (no SDL video
   for a client here), and a failed RCON round trip cannot distinguish a failed
   connect from a failed read, so it stays `NotDispatched` — under-claiming
   deliberately.

## Known-wrong numbers NOT to surface until fixed
  - `flow_graph.rs` steel throughput was 5x overstated (3.2 s/craft against the
    16 s named in the comment above it). **Fixed by the peer in `64decdcd`**
    and extracted as `furnace_output`. Verify before trusting.
  - Research-trigger technologies (`electronics`, `steam-power`,
    `automation-science-pack`, `steel-axe`) are correctly ORDERED but
    UNDER-COSTED. A gantt will draw those bars too short. Label or omit; do not
    present as accurate.

## Map view
Two commented-out menu entries in `App.vue` — `Entities -> /workspace` and
`Map -> /workspace` — are the placeholders. **Those routes were unbuilt, not
unwanted.** Task 11 drops the comments; this file is where the intent lives.
Data source: `find-entities` + the entity graph.


---

# SCOPING, MEASURED 2026-08-31 — read this before designing anything

Three facts that change the shape of the work. All verified against the tree.

## 1. `mermaid_gantt` renders the PLAN, not the run

`crates/planner/src/render.rs:33` builds its bars from `step.start` /
`step.end` on the `Schedule`. **It does not read `ExecutionLog` at all** —
`grep -rn ExecutionLog crates/planner/src/render.rs` returns nothing.

So "render goal.gantt" gives you **predicted** durations. Everything measured
tonight says those are still wrong for walks: a fixed ~135-tick per-walk
overhead that the model does not carry. Actions track observation at ~1.01x
since `0d93dc3b`; walks do not.

**Consequence: a gantt drawn from the schedule alone is a plan view, not a
replay view, and must be labelled as one.** The observed data has NO renderer
today. Building one is a separate increment, and it is the one the scrubber
rules in this file were written for.

## 2. There is NO HTTP route exposing a plan, schedule, or execution log

Full published path list (from `openapi.snapshot.json`) contains
`/api/v1/game/*`, `/api/v1/instance*`, `/api/v1/jobs*`, `/api/v1/rcon`,
`/api/v1/scripts*`, `/api/v1/settings`, `/api/v1/fs/exists`. **Nothing for
plans.** `grep -rn "Schedule\|ExecutionLog" crates/server/src/` -> nothing;
the server process does not hold either. `Job` stores only id/script/status/
timestamps/stdout/stderr/error.

## 3. But the gantt IS already reachable in the browser — as text

The path that exists today:

    lua script: print(plan:gantt("title"))
      -> job stdout
      -> GET /api/v1/jobs/{id}/events   (SSE, already consumed by scriptStore)
      -> the script output pane

So a **v1 plan view needs no server change at all**: detect a mermaid gantt
block in job output and render it. That is cheap, honest about what it is, and
composes forward — when a real endpoint exists, the renderer is already built.

## The two options, stated so the choice is deliberate

**(a) Render what scripts already emit.** Zero server changes. The UI shows a
gantt when a script prints one. Limitation: the user must run a script; the UI
cannot ask for a plan on its own.

**(b) A planning endpoint** — e.g. `POST /api/v1/plan` taking a goal and
returning a schedule, plus something exposing `ExecutionLog` after a run. The
UI drives it. Materially more work, and it needs a decision about what a goal
looks like over HTTP.

**Recommendation: (a) first.** It is a genuine slice of the feature, it makes
the mermaid renderer exist, and it forces the labelling question (plan vs
observation) to be answered while the stakes are low. (b) then has one fewer
unknown.

## Reminder about the container
`GanttChart.vue` is the mount point, already routed at `/tasks` and already in
the menu. Its commented-out body is the intended shape. Task 10 restyled its
container and left the stub, deliberately.
