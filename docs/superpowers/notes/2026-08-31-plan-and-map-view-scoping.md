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

3. **Walk drift is PROPORTIONAL — it is the pathfinder's detour. There is no
   fixed cost.**

   **This entry previously said the opposite** (a fixed ~135-tick per-walk
   overhead) and was wrong. That came from fitting multiplicative against
   additive models on two walks, both fitted against **straight-line
   distance**, which is not what the bot walks.

   A later run logged the game's full waypoint lists, separating actual path
   length from straight-line distance for the first time. Against
   `ceil(polyline_length / 0.15)`:

       walk 1   planned 217   observed 363   error vs polyline   +2 ticks
       walk 2   planned 575   observed 650   error vs polyline  +10 ticks
       walk 3   planned 528   observed 610   error vs polyline   +6 ticks

   Fit on walk 1, predict the others: 8 and 4 ticks, 1.2% and 0.7%.

   So `0.15` tiles/tick is correct and the whole discrepancy is detour —
   actual paths run 9-27% longer than the straight line. A detour proportional
   to distance *looks* like a constant across two samples of similar length,
   which is why the additive model fitted better by accident.

   **Consequence for rendering, and it is the reverse of what this note used to
   say: every walk is off by roughly its own detour ratio, and a long walk is
   off by MORE ticks than a short one. Do not render a fixed offset.**

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
tonight says those are still wrong for walks: the planner costs a walk by
straight-line distance, but the bot follows the pathfinder's route, which runs
9-27% longer. The error is proportional to the walk, not a constant — see
finding 3 above, which this note originally got backwards. Actions track observation at ~1.01x
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


---

# Later corrections and additions (same night, after the first live multi-step run)

## Two data sources that will mislead a timeline

  - **`world.player().main_inventory` is not live.** It reported a furnace the
    player had placed 1,400 ticks earlier. Do not drive a timeline or an
    inventory panel from it without establishing its staleness first.
  - **The executor's two internal move-asides — blocked placement and pre-mine
    — move the bot but never appear in `obs.walks`.** A timeline built from
    `obs.walks` alone will show unexplained position jumps. Either surface them
    or say in the legend that not every movement is recorded.

## `Status::Lost` fired correctly on its first live outing
*"the game reported no readable outcome: no action result received in time"* —
and the run distinguished it from a failure. The legend footnote in this file
still stands: appearing is trustworthy, not appearing is not proof.

## Two live positioning faults, in `crates/core/src/factorio/rcon.rs`
Not the frontend's to fix, but they bound what a replay can honestly show:

  - **A walk reported success 9.3 tiles from the planned target.** The target
    tile was occupied by the bot's own just-placed furnace, so pathfinding
    failed and `player_path`'s fallback silently retargeted ~10 tiles short —
    returning success. **A walk that did not arrive but said it did.**
  - **`player_mine` then dispatched from 3.345 tiles against a
    `resource_reach_distance` of 3.** The mod's guard is `> 6`, so it accepted,
    set `mining_state` every tick, and the game silently refused: zero position
    and zero inventory events for 21,400 ticks until the action was declared
    `Lost`.

**So a replay can show where the executor BELIEVED the bot was, and that is not
always where it was.** Until those are fixed, a position track is the
executor's belief, not ground truth, and should be labelled that way.


---

# DECISION: raw log over HTTP, rendered client-side. Not a server-rendered string.

Settled 2026-08-31 between the two sessions. The reason is concrete, not a
preference: **mermaid gantt cannot express the design.** Its vocabulary is one
bar per row with a label, a start and a duration. The settled design needs,
per step:

  - **two rows**, planned against observed, as the primary comparison
  - **`Lost` distinct from failure** — the entire point of the fifth state
  - **walk rows marked as belief, not measurement** (a walk reported success
    9.3 tiles from its target)
  - **the gap remainder as wait**, derived by subtracting two observed
    quantities
  - **a scrubber** — selection and time-seeking, which is interaction, not a
    diagram

Mermaid gives the first badly and none of the rest. A pre-rendered string is a
rendering decision taken in Rust that the UI cannot undo, and every bullet
above is a decision that would have to be undone.

## The data is already shaped for this
`ExecutionLog` derives `Serialize`. `Attempt` carries `status`, `number`,
`planned_start_tick` and `planned_end_tick` — the last two explicitly
documented "not observed". The walk map is `BTreeMap<BotId, BTreeMap<usize,
WalkObservation>>`, nested rather than tuple-keyed, specifically so it survives
JSON with contractual iteration order.

## Split of work
  - **`crates/executor`** owns pairing `Schedule` with `ExecutionLog` into one
    JSON document: per step, the planned interval, the observed interval **when
    there is one**, status, and attempt number. It belongs there and not in
    `crates/planner` because the planner is pure and the executor already
    depends on it; rendering there would invert the dependency.
  - **`crates/server`** owns the route.
  - **the frontend** owns all rendering.

**Absent must stay absent.** A zero and a missing measurement render
identically if the DTO defaults them. This is the rule the walk-tick work was
built on and it matters more here than anywhere.

**Keep `number` in the DTO.** An action attempted three times is a different
story from one attempted once, and it is the only record that a retry happened.

## Boundary — two views, two data paths
The mermaid gantt stays exactly as it is: it renders `Schedule` only, it
reaches the browser today as script stdout over SSE, and it is a **plan view**.
It is not the replay. Conflating them is how a plan view ends up quietly
claiming to be a replay.


---

# CORRECTION: there is no route to write. It is a fourth SSE event.

The decision above says "`crates/server` owns the route". **Measured after the
serialisation landed (`db8e651d`), that route has no source and should not
exist.**

  - `ExecutionLog` is created at `crates/scripting_lua/src/globals/goal/run.rs:321`
    inside an `Arc<Mutex<>>` **scoped to one script run**, and dropped when the
    script ends.
  - `grep -rn "ExecutionLog\|Schedule" crates/server/src/` returns **nothing**.
    The server process never holds either.
  - `Job` stores `stdout`, `stderr`, `status` and timestamps. Nothing
    structured.

`GET /api/v1/replay/{id}` would therefore need the server to hold execution
state it has no reason to hold, with a lifetime somebody would have to invent.

## What to build instead

`crates/server/src/manage/execute.rs:329` already has `WireEvent` with three
named SSE events — `output`, `finished`, `lagged` — each `json_data`-serialised
with an `event:` name the browser switches on, already consumed by
`app/src/api/jobEvents.ts`.

**Add a fourth variant carrying `Replay` verbatim.** The replay then arrives on
the same stream as the run it describes, which is the right coupling: a replay
without its run is meaningless. `subscribeJobEvents` gains `onReplay` beside
`onOutput`. No new route, no server-held state, no lifetime question.

## The one obstacle, and it is in the scripting crates

`OutputSink` (`crates/scripting/src/lib.rs:16`) is
`fn line(&self, stream: Stream, text: &str)` — **text only**. There is no path
from the Lua runtime to a structured event today. Preference order:

  1. a second trait method, `fn replay(&self, json: &str)`, defaulting to a
     no-op so no existing implementer breaks
  2. a separate channel alongside the sink
  3. **NOT** a sentinel-delimited blob scraped out of stdout — that is parsing
     our own output format forever

## Split, corrected
  - `crates/scripting` + `crates/scripting_lua` + `crates/executor`: getting a
    `Replay` from the run to the sink (the peer session's area)
  - `crates/server`: the `WireEvent` variant, its OpenAPI docs, the snapshot
  - frontend: `onReplay`, and all rendering


---

# RENDERER PRINCIPLE: a replay shows the executor's CONCLUSIONS, not the world

Forced by a second instance of the same problem, and worth stating as a frame
rather than patching caveats in one at a time.

**Instance 1 — walks.** `Status::Success` on a walk meant the ticks were
measured, not that the bot arrived. Handled by `Evidence::Believed { why }`.

**Instance 2 — transfers (open at time of writing).** A `remove` reported
success and the inventory read afterwards showed no plates. Two possibilities
with opposite consequences: either the plates exist and
`world.player().main_inventory` is stale (it has been, by 1,400 ticks), or the
plates do not exist and `remove` said success anyway. If the second, then
**`Status::Success` for a transfer means only "the game did not refuse the
command", not "items moved"** — and a green row in the replay can be a no-op.

## The frame

Do not treat these as a growing list of exceptions to "success means it
worked". Treat them as the normal case:

**Every row in the replay is a claim by the executor about what it believes
happened. `Evidence` records how strong that claim is. The renderer's job is to
show the claim and its strength, never to assert an effect on the world.**

Consequences for the design:

  - **Do not build success styling that asserts an effect.** A green bar that
    reads as "this worked" is a stronger statement than the data supports for
    at least walks, and possibly transfers.
  - `Evidence` currently marks **only walks**. If transfers turn out to be the
    weaker thing, the fix belongs in `Evidence` — a variant for transfers — and
    **not** in the renderer. The renderer should already be rendering whatever
    `Evidence` says, so a new variant is a new label and no new code path.
  - Build the evidence display **first**, before the happy path looks finished.
    A renderer that treats caveats as an afterthought will have styled success
    as certainty by the time the first caveat arrives.

This is the same family as everything else in this file: **not absent, not
wrong-looking — confidently right about the wrong thing.** The difference is
that here we know the shape in advance, so the renderer can be built to expect
it rather than retrofitted.
