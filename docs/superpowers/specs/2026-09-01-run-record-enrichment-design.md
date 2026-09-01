# Run Record Enrichment — Design

**Date:** 2026-09-01
**Status:** approved for planning
**Supersedes nothing.** Extends the record format introduced with
`crates/core/src/record/`.

## Why

A run record today answers *what the supervisor did*: milestones opened and
closed, actions dispatched and settled, frames captured. It cannot answer
*what the world looked like while that happened*, and the gap is not
theoretical. In `run-1788277287-11819`:

```
66164  milestone_satisfied  {index: 3, iterations: 2}
66164  milestone_started    {index: 4, goal: "smelt iron plates x20"}
66164  milestone_satisfied  {index: 4, iterations: 0}
66164  run_finished         {outcome: "done", elapsed_ticks: 4895}
```

Milestone 4 was satisfied in zero ticks. Either the bots already held 20 iron
plates, or the planner returned an empty plan and the supervisor's
"satisfaction is an empty plan" rule reported a failure as `done`. **The record
cannot distinguish these**, and the run is filed as a success either way.

Separately, `EventKind::PlanCreated` is declared and no code writes it. Four
milestones produced zero `plan_created` lines, so what the planner *intended*
is absent from every run recorded to date.

Two audiences need the missing data, and they want the same bytes framed
differently:

- **A person editing a video** wants a map with bots moving on it, production
  curves climbing, and a research bar filling — continuous, legible, joined to
  the frames already being captured.
- **An agent diagnosing a failed run** wants exactness: what each bot was
  carrying at the tick an action failed, what the planner intended versus what
  happened, and where our model of the world disagreed with the game.

## Non-goals

- **Live streaming of samples.** The viewer is post-hoc. Samples are archived
  at milestone boundaries and at finish, the same moments frames are archived.
- **Whole-map capture.** Keyframes cover the built bounding box plus a margin,
  not the surface.
- **Replacing `events.jsonl`.** It stays the narrative spine — small, ordered,
  `tail`-able. New streams are siblings, not replacements.
- **Fixing research-to-completion.** That is a separate track. This spec builds
  the instrument that makes the bug legible; it does not chase the bug.

## Decisions taken

| Decision | Choice | Why |
|---|---|---|
| Sampling location | BotBridge mod pushes | No RCON round-trips contending with the executor; in-game accurate |
| Placement source | Instrument the executor | Intent and truth arrive at the same moment, so drift costs nothing extra |
| Model vs game disagreement | Record both, flag it | The disagreement is the signal; discarding either half discards it |
| Bot sample cadence | 60 ticks (1 s) | A bot can fill and empty inside 5 s; 5 s inventory misses the failure moment |
| Force sample cadence | 300 ticks (5 s) | Production, power and research move slowly; matches the frame beat exactly |
| Extra sample trigger | On every action settle | The tick that matters for diagnosis is the tick something failed |
| Delivery | All three tiers, then review | Author's choice |

---

## 1 · File layout

A run directory gains two files beside the existing ones:

```
runs/<run-id>/
  events.jsonl      unchanged — the narrative spine
  samples.jsonl     periodic world state           (new)
  map.jsonl         placements, removals, keyframes (new)
  manifest.json     gains `samples` and `map` counts
  frames/           unchanged
```

Every line in every file carries `tick`, and `game.tick` remains the only
clock any two streams share. A viewer built before these files existed opens a
new run unchanged: it never asks for them. This mirrors the `#[serde(other)]
Unknown` catch-all already in `EventKind` — forward compatibility by omission
rather than by negotiation.

`manifest.json` gains two integer counts, `samples` and `map`, alongside the
existing `events` and `frames`. Counts, not sizes: a reader deciding whether to
fetch a stream cares how many records it will get.

---

## 2 · The mod side

### 2.1 Two beats, one handler set

`control.lua` already registers frame capture with
`script.on_nth_tick(FRAME_CAPTURE_INTERVAL, …)` where the interval is 300, and
the file documents that `on_nth_tick` runs on **every peer** — which is exactly
why each client writes frames into its own `script-output`.

Sampling must not fan out that way. Force-wide statistics are identical on
every peer, and bot inventories are readable from any peer, so four copies
would be four identical files to reconcile for no gain. The existing pattern at
`control.lua:581` is the answer:

```lua
game.write_file("players_connected.txt", "server\n", true, 0) -- only on server
```

The fourth argument restricts the write. Sample writes pass it, so samples land
once, in the server instance's `script-output`.

**`on_nth_tick(n, f)` replaces the handler registered for `n`**, a trap the
frame code already documents in place. Frame capture owns 300. So there is
exactly **one new registration**, and one edit to an existing handler:

- `script.on_nth_tick(60, …)` — new. Writes one `bots` line.
- The **existing 300-tick frame handler** additionally writes one `force` line.
  Registering a second handler on 300 would silently unregister frame capture
  and the run would produce no frames at all, with nothing to say why.

Folding the force sample into the frame handler is not merely a workaround: it
guarantees a force sample and a frame share a tick exactly, which is what lets
the production curve and the screenshot be read against each other without
interpolation.

Both append to `script-output/botbridge/samples.jsonl`. One file, two line
kinds, discriminated by a `kind` field — the same internally-tagged shape
`EventKind` uses, for the same reason: a reader that does not know a kind skips
the line instead of refusing the file.

### 2.2 The schema stamp

Every sample line carries an integer `schema`, currently `1`.

This exists because of a trap the repository already documents: in a debug
build `workspace/mods` wins over the repo checkout and **there is no refresh
path**, so editing `mods/BotBridge` has no effect on an existing workspace. The
mod's `info.json` has read `0.0.1` since the project began, so a version string
cannot detect staleness.

The Rust ingester **refuses a `schema` it does not know** and reports the run as
having no samples, naming the schema it found. A stale mod therefore produces a
loud, specific failure instead of quietly correct-looking numbers of the wrong
shape. Bump `schema` on every change to a sample's fields, including additions.

### 2.3 Sample shapes

A bot line, written every 60 ticks:

```json
{"kind":"bots","schema":1,"tick":61500,
 "bots":[{"id":1,"position":{"x":-40.5,"y":-48.5},
          "inventory":{"iron-ore":23,"stone-furnace":2},
          "crafting_queue":0,"mining":"iron-ore"}]}
```

`inventory` is a name→count map built with the `inventory_counts` helper
already in `control.lua`, which handles Factorio 2.0's
`LuaInventory.get_contents()` returning an array of `{name, count, quality}`
rather than a dict. `mining` is the entity name the bot is currently mining, or
`null`. `crafting_queue` is the queue length, not its contents — the contents
are large, change every tick, and no question we have asks for them.

Positions are written as the game reports them, which for a resource is a tile
centre (`-40.5`, never `-41`). Nothing rounds them on the way out.

A force line, written every 300 ticks:

```json
{"kind":"force","schema":1,"tick":61500,
 "research":{"name":"automation","progress":0.42,"eta_ticks":18000},
 "techs_unlocked":7,
 "production":{"made":{"iron-plate":120},"consumed":{"iron-ore":120}},
 "power":{"generated_kw":180.0,"consumed_kw":150.0,"satisfaction":1.0}}
```

`production` is **cumulative from the start of the game**, never per-interval.
Deltas are derivable from totals; totals are unrecoverable from deltas once one
sample is lost. `research` is `null` when nothing is queued — present and null,
never absent, so "we looked and nothing was researching" is distinguishable
from "we never asked".

`power` reads generation and consumption across the force's electric networks.
The repository already records that coverage is not capacity and that an
under-supplied network reads as *dead* rather than slow; this is the field that
makes that visible in a recorded run.

### 2.4 Settle-triggered samples

The 60-tick beat can still miss the moment: an action fails at tick 61,533 and
the nearest bot sample is 33 ticks stale, by which time the bot has moved or
handed off. So the mod also emits a `bots` line **on demand**, via a new RCON
function `rcon_sample_bots()`, which the executor calls immediately after an
action settles with a non-success status.

Only on failure. Sampling on every settle would roughly double the bot stream
for information the periodic beat already carries when nothing went wrong.

This is a deliberate exception to "the mod pushes, Rust does not poll": it is
one RCON call, on a path that is already failing, and it buys the single most
useful row in the analysis view. The rule it breaks exists to keep sampling off
the hot path, and a failed action is not the hot path.

---

## 3 · The Rust side

### 3.1 Ingestion

`crates/core/src/record/samples.rs` mirrors `frames.rs`: it reads
`workspace/server/script-output/botbridge/samples.jsonl`, filters to lines whose
tick falls inside the run, validates `schema`, and appends them to
`runs/<id>/samples.jsonl`.

It runs at the same two moments `archive_frames` runs — milestone boundaries and
`finish` — so there is one archival path with one set of failure modes, not two.

Filtering by tick range uses the recorder's existing `not_before` high-water
mark. A sample file left behind by a previous run therefore cannot leak into
this one, which is the same hazard `frames.rs` already guards by filtering on
run id.

### 3.2 Placement log

`crates/executor/src/rcon_actuator.rs:309` calls `place_entity_timed`, which
returns the `FactorioEntity` the game actually created. That single call site
holds both halves of what we want:

- **intent** — the item, position and direction the executor asked for
- **truth** — the entity the game reports back

So the executor emits one `map.jsonl` line per placement:

```json
{"kind":"placed","tick":62010,"bot":3,
 "intent":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},
 "actual":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},
 "drift":null}
```

`drift` is `null` when intent and actual agree, and otherwise names the fields
that differ. The inserter-direction trap in this repository is exactly this
shape of bug: a layout that places 100% correctly and does nothing, because
placement and function are separate concerns. A drift field cannot catch a
wrong intent, but it catches every case where the game did something other than
what was asked.

Removals emit `{"kind":"removed", …}` from the same actuator.

### 3.3 Keyframes

A `keyframe` line is written at every milestone boundary and every 30th force
sample (9,000 ticks, 2.5 minutes), whichever comes first:

```json
{"kind":"keyframe","tick":66164,"bounds":{"left":-64,"top":-64,"right":64,"bottom":64},
 "game":[{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0}],
 "model":[{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0}],
 "divergence":[]}
```

`bounds` is the bounding box of every entity the bots have placed this run,
plus a 16-tile margin — not the surface. `game` is what
`find_entities_in_radius` reports; `model` is what our `EntityGraph` believes;
`divergence` lists entities present in exactly one of them, tagged with which.

Recording both is the decision taken above. It costs roughly double the bytes of
either alone, and it is the only way a run tells you that our model was wrong —
a class of bug that has already cost this project twice, once through ghosts
that do not collide and once through resource positions that are tile centres.

`EntityGraph` keys resources by `Pos(i32, i32)`, which floors. Anything reading
a position out of that map for a keyframe **must restore the half-tile offset**,
or every resource in `model` will diverge from `game` by (0.5, 0.5) and the
divergence list will be pure noise.

### 3.4 Planner introspection

Three changes to `events.jsonl`, all additive:

**`PlanCreated` gets written.** It is already declared and already carries
`steps`, `makespan` and `bots`. It gains the DAG:

```rust
PlanCreated {
    milestone_index: u32,
    steps: u32,
    makespan: u64,
    bots: Vec<u32>,
    /// The scheduled steps, in schedule order.
    plan: Vec<PlannedStep>,
}

pub struct PlannedStep {
    pub id: u32,
    pub bot: u32,
    pub action: String,
    /// Ids this step waits on.
    pub deps: Vec<u32>,
    /// Ticks from the plan's start, not absolute game ticks.
    pub planned_start: u64,
    pub planned_duration: u64,
}
```

`planned_start` is relative because a plan is computed before it is dispatched
and does not know its own origin tick. The viewer already converts between
planned and observed origins in exactly one place; this keeps that true.

**`MilestoneSatisfied` gains a reason.**

```rust
MilestoneSatisfied {
    index: u32,
    iterations: u32,
    reason: SatisfiedReason,   // AlreadySatisfied | PlanEmpty
}
```

This is the smallest change in the spec and the one that would have explained
milestone 4 on its own.

**`ActionSettled` gains a structured failure** beside the existing `error`
string: a `kind` (`missing_item`, `unreachable`, `blocked`, `rejected`,
`timeout`, `other`) and an optional `detail`. The string stays — it is what a
human reads — and the kind is what a query groups by.

---

## 4 · Presentation

One record, two routes.

### 4.1 `/runs/:id` — the video view

The existing page gains a **map panel**: a canvas drawing, at the tick cursor,
ore patches as background, placed entities as coloured cells reconstructed from
`map.jsonl` deltas since the last keyframe, and each bot as a dot trailing its
last 30 seconds of position. All of it driven by the single cursor that already
drives frames and lanes, so nothing interpolates between clocks.

Beside it, a **production panel** — cumulative curves for the items the run's
milestones name — and a **research bar** showing the current technology and its
progress. Research is the spine of the run this record was built for, and a bar
that fills is the clearest possible statement of progress for a viewer.

### 4.2 `/runs/:id/analysis` — the diagnosis view

Its own route, because its density is wrong for the video view and because a
URL that opens straight to the failure is worth having.

- **Overrun table** — one row per planned step, joined to its dispatch and
  settle by `id`: planned duration, actual duration, delta, status. Sorted by
  delta descending, so the worst overrun is the first row.
- **Divergence list** — every keyframe's `divergence`, with tick and entity, and
  a marker on the map panel at that tick.
- **Milestone forensics** — for each milestone, the plan DAG beside what
  actually ran, and the `reason` when it was satisfied without doing anything.
- **Inventory at failure** — for every failed action, the settle-triggered bot
  sample: what that bot was holding at the tick it failed.

### 4.3 Contract

Both views read new API routes under `/api/v1/runs/{id}/`: `samples` and `map`.
Each is added to the OpenAPI spec, regenerated into
`app/src/api/openapi.snapshot.json`, and mirrored in `app/src/api/types.ts`
with an `objectContract<T>` declaration. The three-guard seam fails from both
ends by design; nothing here is exempt from it.

---

## 5 · Sizing

At 60 UPS, a 100,000-tick research run is about 28 minutes.

| Stream | Records | Rough size |
|---|---|---|
| Bot samples (60 ticks, 4 bots) | 1,667 | ~700 KB |
| Force samples (300 ticks) | 333 | ~120 KB |
| Placements | tens | negligible |
| Keyframes (~11, 200 entities) | 11 | ~400 KB |
| **Total** | | **~1.2 MB** |

Frames dominate a run directory by two orders of magnitude, so this changes
nothing about retention. The keyframe interval is the only knob that matters:
at sample cadence it would be 130 MB, which is why it is milestone-scoped.

---

## 6 · Testing

- **Mod:** sample lines are JSON and carry `schema`. Verified by ingesting a
  real run, not by unit test — `control.lua` has no test harness, and asserting
  against a hand-written fixture would test the fixture.
- **Ingestion:** unit tests over fixture `samples.jsonl` files — correct schema,
  unknown schema (refused, named), tick outside the run (excluded), truncated
  final line (skipped, counted, as `read_events` already does).
- **Placement drift:** a unit test where intent and actual differ, asserting
  `drift` names the differing field; and one where they agree, asserting `null`.
- **Keyframe divergence:** a test with a resource at a tile centre in `game` and
  the floored `Pos` in `model`, asserting the half-tile offset is restored and
  `divergence` is empty. This is the test that fails if the known trap is walked
  into again.
- **Frontend:** pure functions in `app/src/lib/` for map reconstruction (apply
  deltas from the last keyframe to a tick) and for the planned-vs-actual join,
  tested directly. Coverage gates as they stand: lines 90, statements 90,
  branches 80.
- **End to end:** a live run with `--clients 2`, confirming `samples.jsonl` is
  non-empty, its ticks lie inside the run, and the map panel draws entities the
  run actually placed.

## 7 · Risks

**The mod-staleness trap is now owned deliberately.** Every sample-shape change
needs a `workspace/mods` refresh. The `Using mods directory <path> (<why>)` line
that every run logs is the authoritative answer to "did my edit ship", and the
schema stamp is the second net. Neither removes the cost; both make it loud.

**`on_nth_tick` replaces handlers by interval.** Registering 60 or 300 elsewhere
would silently unregister sampling. Frame capture already owns 300 — the force
sampler must therefore extend the existing 300 handler rather than register a
second one, or frame capture stops.

**Keyframes read the whole built area.** On a large base this is the most
expensive thing in the spec. It runs at milestone boundaries, where a pause is
already expected, not on the hot beat.
