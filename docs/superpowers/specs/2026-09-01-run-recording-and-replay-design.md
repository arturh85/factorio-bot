# Run Recording and Replay — Design

**Status:** approved in outline, not yet implemented
**Date:** 2026-09-01

> **PARTLY OBSOLETE / STATUS STALE (2026-09-03):** the status line is wrong — the recorder, JSONL event log, splits, retention, `/api/v1/runs*` and the viewer all shipped. Separately, **every passage about frames is obsolete**: the per-camera screenshot feature was removed end to end in `15c85c1f` and video capture (`crates/core/src/record/video/`) replaces it. Markers below flag the affected sections.

## Why

A run today leaves almost nothing behind. Frames land in
`workspace/client<N>/script-output/frames/` and are wiped by the next run;
execution detail exists only as SSE to a browser that had to be watching; and
`GET /api/v1/frames` reports what is on disk *right now*, which is the current
run and nothing else. Two runs cannot be compared because only one exists at a
time.

We want the opposite: a run that is a durable artifact you can re-open, seek
through, watch, and compare against another run — good enough to cut a video
from, and good enough to answer "why was this run slower than that one".

## Goals

1. **Multi-phase plans execute correctly against a live game** and the record
   shows it — what each bot was doing, when, and whether it worked.
2. **A run is a durable, readable artifact.** Readable by a human with `less`,
   by `jq`, and by the web app.
3. **A timeline** that plays for video or seeks by hand.
4. **Task visualisation** — what each bot is working on at any moment.
5. **Speedrun-style splits** so runs can be compared at a glance.

## Non-goals

- Video encoding. We store frames; turning them into an mp4 is `ffmpeg` and
  does not need to live in this codebase.
- Live collaborative viewing. One viewer, reading finished or in-progress runs.
- Replaying a run *into* Factorio. This is observation, not determinism
  replay.

## Decisions

### D1 — Ticks are the clock; wall time is recorded but never compared

Every event carries `tick` (Factorio's `game.tick`) and, until 2026-09-02,
`wall_ms`. Comparison between runs is **always** on ticks.

Wall time measures the machine: a headless server and a graphical client with
three cameras do not run at the same speed, and neither matches a run made
while a build was compiling. Ticks are the game's own clock and are what a
speedrun actually measures. Recording both costs nothing; comparing on wall
time would silently compare hardware.

Ticks are also already the join key: the mod names frames from `game.tick`.

**Amendment, 2026-09-02: `wall_ms` was removed, not merely left uncompared.**
"Recording both costs nothing" assumed the stamp reflected *when the event
happened*. It didn't: `RunRecorder::record()` stamped it from
`self.started.elapsed()` at the moment `record()` was *called*, and
`record.actions()`/`record.teleports()`/`record.refusals()` are each called
once per supervisor "ran" transition -- after an entire multi-bot plan has
finished executing. Every event flushed by one such call got the wall clock
reading from the moment that whole batch was written, not the moment each
event actually happened, which is what
`docs/superpowers/notes/2026-09-02-inventory-shortfall.md` caught as `wall_ms`
jumping `33780 -> 738866` while `tick` moved `10`. Making it mean the latter
would require the executor to capture a real timestamp at the point it
dispatches/observes each action and carry that across the mlua boundary into
`record.actions()` -- a cross-crate change to the executor's attempt/walk
types, not a fix to a wrong stamp. Nothing read the field (not
`app/src/lib/runTimeline.ts`, not `runDiff.ts`, not the analysis page), so it
was removed rather than fixed. See `factorio_bot_core::record::Event`'s doc
comment for the full account. An old `events.jsonl` line's leftover
`"wall_ms"` key still reads fine -- it deserializes as an ordinary ignored
extra field.

### D2 — The record is append-only JSONL

`events.jsonl`, one JSON object per line, written as the run proceeds.

A single JSON document is only readable once it is closed, which is exactly
wrong for a long run that may crash — and the runs most worth reading are the
ones that failed. JSONL survives truncation (every complete line before the
crash is still valid), tails live, greps, and diffs between runs. It is also
the format `jq` is best at, which matters for "readable format".

Every event has `tick`, `wall_ms`, `kind`. Unknown kinds must be ignored by
readers rather than rejected, so an older viewer can open a newer run.

### D3 — A run is a directory, archived out of the workspace

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature ("frames") this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it. There is no `frames/` directory, no `frames/index.json`, no `/api/v1/frames*` or `/runs/{id}/frames*` route, and no `ArchivedFrame`/`EventKind::Frame`.

```
runs/<run-id>/
  manifest.json     -- id, started/finished, seed, bots, versions, git sha
  events.jsonl      -- the record
  splits.json       -- milestone timings, derived at finish
  frames/
    index.json      -- (bot, camera, tick) -> file, sorted
    <bot>/<camera>/<tick>.jpg
```

The workspace copy is transient by design and is wiped by the next run. The
archive is written at run end (and incrementally for `events.jsonl`, so a
crashed run still leaves a readable partial record).

`<run-id>` is minted by the recorder. Today nothing mints it: the mod echoes
whatever string `rcon.frame_capture_start(run_id)` is handed, so identity is
currently "whoever remembers to pass the same string twice".

**Starting the recorder also starts frame capture, with the id it just
minted.** One call, one id, no way for the log and the frames to disagree
about which run they belong to. Two call sites that must agree on a string is
precisely the kind of arrangement that works until the day it doesn't.

**Frames are matched to a run by that id, never by tick range** — tick ranges
of two runs overlap trivially, since every run starts near tick 0. A stale
frame left in the workspace by an earlier run is excluded by id, which is the
bug that bit us on 2026-08-30.

`runs/` lives at `<workspace>/runs/`, beside the `client<N>` directories the
frames are copied from.

**Bots and clients are 1:1**: frames are captured into `client<N>/` and
archived under the bot id `N`. The archive uses bot ids throughout because
that is what every event already refers to; `client<N>` is a detail of where
the file happened to land.

### D4 — Splits are derived from the log, then materialised

`splits.json` is computed at run end from the milestone events, not recorded
independently. Derived, so it cannot disagree with the log it summarises;
materialised, so the viewer and any comparison tool do not each re-derive it.

A split is `{index, goal, started_tick, satisfied_tick, outcome}`. Comparing
two runs is comparing two split lists by goal name.

### D5 — One scrubber; everything indexes by tick

The viewer has a single timeline over the run's tick range. Frames, action
lanes and splits are all positioned on it. Seeking sets a tick; every panel
renders whatever it holds at that tick. Playing advances the tick at a chosen
rate.

This is why D1 matters structurally and not just for comparison: one clock
means one control.

## Event schema

```jsonc
{"tick":0,     "wall_ms":0,     "kind":"run_started",
 "run_id":"...", "seed":123, "bots":[1,2], "factorio":"2.1.17", "git":"7ad798f4"}

{"tick":120,   "wall_ms":2011,  "kind":"milestone_started",   "index":0,
 "goal":"researched(automation)"}
{"tick":24707, "wall_ms":41022, "kind":"milestone_satisfied", "index":0,
 "iterations":3, "elapsed_ticks":24587}
{"tick":30000, "wall_ms":50100, "kind":"milestone_stuck",     "index":1,
 "outcome":"stuck_silent", "best_steps":42, "last_error":null}

{"tick":130,   "wall_ms":2140,  "kind":"plan_created", "milestone_index":0,
 "steps":105, "makespan":24587, "bots":[1,2,3,4]}

{"tick":140,   "wall_ms":2300,  "kind":"action_dispatched", "id":17, "bot":2,
 "action":"mine", "target":{"x":-40.5,"y":-48.5}}
{"tick":260,   "wall_ms":4180,  "kind":"action_settled",    "id":17, "bot":2,
 "status":"success", "elapsed_ticks":120, "error":null}

{"tick":300,   "wall_ms":5000,  "kind":"frame", "bot":1, "camera":"overview",
 "file":"frames/1/overview/300.jpg"}

{"tick":41000, "wall_ms":68000, "kind":"run_finished", "outcome":"done",
 "elapsed_ticks":41000}
```

Every event's own `tick` is *when it happened*; a duration is always
`elapsed_ticks`. The two were both called `ticks` in the first draft of this
schema, which is the sort of thing that reads fine until someone plots it.

`action_dispatched` / `action_settled` pair on `id`. The pair is what produces
a per-bot span, and therefore the task lanes. A dispatched action with no
settle is drawn as unterminated rather than dropped — that is a real state
(`Lost`) and hiding it would make the picture prettier than the run.

`error` and `last_error` are present-and-null rather than absent when there is
nothing to report, following the existing `Replay` convention: a key that is
always there says "we looked", where a missing key cannot be distinguished
from "we never asked".

## Components

### 1. `RunRecorder` (new, `crates/core/src/record/`)

Owns the run directory and appends events. One method per event kind rather
than a generic `log(json)`, so the schema is checked at the call site by the
type system instead of by review.

Flushes every event: a crashed run must leave a readable record, and these events
are rare enough (hundreds, not millions) that buffering buys nothing.

### 2. Executor and supervisor call sites

The executor already holds everything `action_dispatched`/`action_settled`
need — id, bot, planned ticks, dispatch verdict, status — in `ExecutionLog`.
The recorder is fed from there rather than from a second traversal, so the
record and the existing `Replay` cannot diverge.

Milestone events come from the supervisor loop
(`docs/superpowers/specs/2026-08-31-supervisor-loop-design.md`), which is the
thing that knows what a phase is.

**That supervisor is specified and not yet implemented, and this is a hard
dependency for one goal and not the other.** Without it a run is a single plan:
`events.jsonl`, frames, task lanes and the viewer all work, and `splits.json`
holds exactly one entry. Multi-phase runs — and therefore meaningful splits and
run comparison — arrive only when the supervisor does. Implementing the
supervisor is a prerequisite of goal 1, not of goals 2-5.

### 3. Archive step

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature ("frames") this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it. There is no `frames/` directory, no `frames/index.json`, no `/api/v1/frames*` or `/runs/{id}/frames*` route, and no `ArchivedFrame`/`EventKind::Frame`.

At run end: write `manifest.json` and `splits.json`, copy
`workspace/client<N>/script-output/frames/` into `runs/<id>/frames/` **filtered
by the run id in `run.json`**, and write `frames/index.json`.

### 4. HTTP API (`crates/server/src/runs/`)

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature ("frames") this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it. There is no `frames/` directory, no `frames/index.json`, no `/api/v1/frames*` or `/runs/{id}/frames*` route, and no `ArchivedFrame`/`EventKind::Frame`.

```
GET  /api/v1/runs                     list: id, started, outcome, ticks, splits summary
GET  /api/v1/runs/{id}                manifest + splits
GET  /api/v1/runs/{id}/events         events.jsonl (streamed; `?kind=` filter)
GET  /api/v1/runs/{id}/frames         frames index
GET  /api/v1/runs/{id}/frames/{bot}/{camera}/{tick}   the image
```

The existing `GET /api/v1/frames` stays as-is: it answers "what is on disk for
the *current* run", which is a different and still-useful question.

### 5. Web viewer (`app/src/`)

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature ("frames") this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it. There is no `frames/` directory, no `frames/index.json`, no `/api/v1/frames*` or `/runs/{id}/frames*` route, and no `ArchivedFrame`/`EventKind::Frame`.

One route, `/runs/:id`, with four panels over a shared tick cursor:

- **Timeline** — scrubber across the tick range, play/pause, rate control.
  Split markers drawn on it.
- **Frames** — the frame at or before the cursor, per selected bot and camera.
  "At or before" matters: frames are every 300 ticks and some drop, so exact
  match would blank the panel most of the time.
- **Task lanes** — a row per bot, spans from dispatched/settled pairs,
  coloured by status. This is the "what are the bots working on" view.
- **Splits** — the speedrun panel: goal, tick, delta against a chosen
  reference run.

Run comparison is selecting a second run in the splits panel; it loads only
that run's `splits.json`, not its whole log.

## Disk

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature ("frames") this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it. There is no `frames/` directory, no `frames/index.json`, no `/api/v1/frames*` or `/runs/{id}/frames*` route, and no `ArchivedFrame`/`EventKind::Frame`.

Frames dominate. The mod captures at 300-tick intervals — 12 frames per minute
per camera — with three cameras configured, at 1920x1080. A 20-minute run is
therefore of the order of 700 frames and a few hundred megabytes. Fifty
archived runs is several gigabytes.

So the archive needs a retention policy from day one rather than as a later
fix: keep the N most recent runs (default 20, configurable), and never
auto-delete a run the user has marked kept. Deleting a run deletes its whole
directory; a run is the unit of retention, because an event log whose frames
have been reaped is a viewer full of gaps that look like dropped frames.

Retention runs at archive time, not on a timer: a background reaper deleting
directories a viewer is reading is a race nobody needs.

## Error handling

- **A run that crashes mid-flight** leaves `events.jsonl` valid up to the last
  complete line and no `splits.json`. The viewer must render such a run —
  showing it as unfinished — rather than refusing it. These are the runs most
  worth looking at.
- **A frame file named in an event but missing on disk** renders as a gap, not
  an error. Frames drop; the mod already documents this.
- **An unknown event kind** is skipped by readers, so an old viewer opens a
  new run.
- **A run directory with no frames** is valid: planning-only runs produce a
  full event log and no images.

## Testing

- **Recorder unit tests**: every event kind round-trips; a truncated
  `events.jsonl` (cut mid-line) parses to the events before the cut and does
  not raise; unknown kinds survive a read.
- **Splits derivation**: pure function from events to splits, tested with
  plain data — including a run with a stuck milestone and a run that never
  finished.
- **Archive**: frames belonging to a *different* run id present in the
  workspace are not copied. This is the decoy case that caught a real bug on
  2026-08-30 and it stays covered.
- **API**: list/get/events/frames against a fixture run directory; a missing
  run is 404, an unfinished run is 200 with `splits: null`.
- **Viewer**: vitest over the tick-cursor logic — "frame at or before cursor"
  with gaps, span construction from unpaired dispatch events, and split delta
  arithmetic against a reference.
- **Live**: a real multi-phase run producing an archive that the viewer opens.

## Implementation order

1. `RunRecorder` + event schema + recorder tests.
2. Executor/supervisor call sites; a live run produces `events.jsonl`.
3. Splits derivation + archive step (frames filtered by run id).
4. HTTP API over a fixture run.
5. Viewer: timeline + frames first, then task lanes, then splits/comparison.

Each step leaves something usable: after 2 there is a readable record even
with no UI at all, which is already more than exists today.
