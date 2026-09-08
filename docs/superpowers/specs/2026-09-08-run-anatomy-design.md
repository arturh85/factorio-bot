# Run Anatomy — the `/runs` page as one tick-axis instrument

**Status:** approved in outline by the owner on 2026-09-08 ("looks great and I
agree with your recommendations"). Nothing implemented. The interactive design
proposal this spec formalises is published as an artifact
(`Run Anatomy`, 2026-09-08); its mockups are drawn from
`workspace/runs/run-1788696619-00325` through the routes the server already
serves.
**Date:** 2026-09-08

## Why

The owner asked for "a fancy way to understand a run: when certain production
rates were reached, how the bots moved, how the factory that was built flows".

`RunsPage.vue` today is correct for what it was built for — a run list, a splits
table, a range-input scrubber, one lane bar per bot, a video, an SVG map, and
three world-state panels — and it hangs off one cursor in game ticks, which is
the right spine. But every number on it is a cumulative count or a single tick.
Meanwhile the record samples, every 300 ticks, what the page never draws:
per-machine status with produced counters, per-network power, production
counters; and `tools/run_analysis.py` derives from those the single most
important number this project produces — the `roster-fed` / `factory` /
`hand-made` / `unclear` verdict per item and interval — which the browser never
sees. Idle time appears nowhere: the busiest bot of the sample run was idle for
31.8 % of its milestone across 85 gaps, and the lane bar cannot show it.

The design rule everything bends around, restated from `CLAUDE.md`: **a rising
production curve is not evidence of a working factory.** A hand-loaded stone
furnace is a machine and `production.made` counts it. So the verdict is painted
into the chart, not footnoted under it.

## Goals

1. One run is read on **one clock**: every panel is a lane on the same tick
   axis and the cursor is the only control.
2. **Rates with attribution.** Items per minute at fixed marks, with the
   verdict per interval visible in the chart and in the headline sentence.
3. **Idleness drawn, not implied**, per bot, with replan boundaries.
4. **Machine status over time**, per machine, joined to the map at the cursor.
5. **A flow view** that shows the gap between modelled capacity and measured
   production — never the model alone.
6. Every absence renders as absence: `null` ≠ `[]` ≠ `0`, a gap in the record
   reads "no record", a mark the run never reached reads by status.

## Non-goals

- Live streaming of a running run. The page is post-hoc, over the archive.
  (The `/tasks` replay panel stays the live view.)
- Modelling back-pressure or idleness in the flow graph. The view exposes the
  gap; closing it is planner work.
- Video changes beyond placement. The recorder, clock and join are untouched.
- Editing or re-running anything from the page.

## Decisions (owner-approved 2026-09-08)

| # | Decision | Ruling |
|---|---|---|
| D1 | Where the attribution rule lives | **TypeScript** port of `attribute_from_counters` in `app/src/lib/`, with a golden test pinned to `just analyse` output for a committed fixture run. Revisit only if the rule outgrows counter arithmetic. |
| D2 | Flow view in scope | **Yes, as Phase 3, gap-first only.** The model over-predicts up to 1.32× against the world-record base and the dominant error is idleness it cannot represent; the view draws model ÷ measured per node and says so. |
| D3 | Route shape | **Replace `/runs`.** `/runs/:id/analysis`'s overrun table, divergence list and failure inventories fold into lane and map hover; `/runs/:id` becomes a real deep link (closing gap G5 of the 2026-09-03 audit). |
| D4 | Video placement | A **tab in the map's slot that appears only when `/video` answers.** Most runs are headless now and have none. |
| D5 | Dark mode | **Yes.** Dark tokens are added to `tailwind.css` in Phase 1 while `RunsPage.vue` is moved onto tokens anyway; the theme follows `prefers-color-scheme` with an explicit toggle stored per viewer. |

## Rendering rules carried over

Already governing the replay components and the analysis tool; the page
inherits them.

- **Absent is not zero.** `stalls: null` (nobody watched) and `stalls: []`
  (watched, no stall) render differently. `unearned_ratio: null` is undefined,
  not 0. A mark the run never reached is reported by status
  (`run_ended` / `samples_end` / `no_sample`), never as 0/min.
- **Every row is a claim with a strength.** Success styling never asserts an
  effect on the world; `Evidence` decides the marker, and a new `Evidence`
  variant is a new label, not a new code path.
- **Ticks are the clock.** Wall time answers only "when was this run". Planned
  ticks start at zero, observed ticks are absolute, converted in exactly one
  place per component (the `observedOrigin()` pattern).
- **Never join on `id` across a replan.** Action ids restart per plan (44 of
  147 collide in one archived run). Every segment carries `(plan index, id)`;
  the boundary is drawn as a dashed rule across all lanes.
- **One axis per chart.** Power's two series share kW. Items with different
  scales are small multiples, never a second y-axis.
- **Colour follows the entity.** Bot 2 is the same hue in every band and on
  the map; a filter never repaints survivors. Verdict and status colours are a
  separate reserved set and always carry a text label.
- **A gap in the record is "no record".** Nine of 24 archived runs end on a
  `plan_created` with nothing after it; one healthy run was killed because its
  record went quiet. The coverage band says where data exists, and no other
  band may fill a gap with a default.

## 1 · Page anatomy

Route `/runs/:id` (list at `/runs`). Top to bottom:

1. **Headline and provenance strip.** The same sentence `just analyse` prints
   (`rates: iron 19 /min at 5 (roster-fed; no generator until 4:06) | milestone
   1 satisfied at 6:06`), then chips: seed, roster and `bot_mode`, `game_speed`,
   git commit and dirty flag, profile, mod set, sample coverage
   (`samples_lag_ticks`). A field the record lacks shows a chip reading
   *not captured* in the same place.
2. **Milestone ribbon.** One segment per split, labelled goal · outcome · tick.
3. **Axis.** Game minutes from `run_started`; the 5/10/15/… marks emphasised.
4. **Production band.** Small multiples per tracked item (the analysis tool's
   `DEFAULT_RATE_ITEMS` plus anything over its threshold), items/min over the
   trailing 2-minute window, exactly as `production_rates()`. Background per
   1-minute interval painted by verdict: hatched copper `roster-fed`, solid
   green `factory`, dotted `hand-made`, grey `unclear`/`mixed`, none for no
   output. Mark values labelled on the mark rule. Plateaus annotated with the
   cause `classify_plateau` gives.
5. **Power band.** Generated and consumed kW on one scale; a network whose
   demand exceeds generation gets a critical stripe; the first generation tick
   is labelled ("no generator until …").
6. **Research band.** Progress fill per technology. Trigger technologies are
   instant flips labelled by trigger, never bars.
7. **Bot lanes.** One row per bot; the row background is the idle hatch and
   only dispatched work paints over it. Segments coloured by verb class (walk ·
   mine/chop · craft · place · feed · research). Zero-length feeding verbs are
   fixed-width ticks. Failed/lost segments are outlined critical. Replan
   boundaries dashed across every row. Hover: action, `(plan, id)`, ticks,
   status, attempt number, evidence.
8. **Machine heatmap.** One row per sampled machine, one cell per 300-tick
   sample, coloured by status (`working` good, `no_ingredients` warn,
   `no_fuel` serious, `no_power`/`low_power` critical, else neutral). Rows
   grouped by kind, ordered by placement tick. Chests neutral with a fill bar.
   Clicking a row highlights the machine on the map.
9. **Record coverage.** Extent of events, bot samples, force/machine samples;
   `samples_lag_ticks` as the number.
10. **Cursor control.** One range input plus play/pause and step (existing).
11. **Map at cursor / Video tab.** The existing `MapPanel`, with entities at
    real footprint, machine fill = heatmap colour at the cursor, trail heading
    arrow, hollow endpoint for a believed walk. Video as a second tab (D4).
12. **Flow at cursor** (Phase 3). Nodes = machines/chests/drills/labs grouped by
    kind and recipe; edges = inserter/belt/pipe links with width = modelled
    rate. Each node: model /min, measured /min over the trailing window, and
    `gap ×` = model ÷ measured. Status stripe from the heatmap.

## 2 · Data flow

All Phase 1 inputs are already fetched by `runsStore.openRun` via
`Promise.allSettled`: `/runs/{id}`, `/lanes`, `/samples`, `/map`, `/video`,
`/video/ticks`, and `/events` (currently only on the analysis page; the store
gains it).

```
samples(force)   ─► lib/runRates.ts        ─► ProductionBand, PowerBand, ResearchBand
samples(machines)─► lib/machineTimeline.ts ─► MachineBand, MapPanel fill
samples(force+machines) + events(action_dispatched)
                 ─► lib/runAttribution.ts  ─► ProductionBand background, RunHeadline
lanes + events(plan_created)
                 ─► lib/runIdle.ts         ─► LaneBand
events + samples ─► lib/runCoverage.ts     ─► CoverageBand
map              ─► lib/runMap.ts (exists) ─► MapPanel
provenance (Phase 2) ─► RunHeadline chips
flow (Phase 3) + samples(machines) ─► lib/flowJoin.ts ─► FlowPanel
```

Every lib is pure: `(inputs, tick | window) → view model`, no fetch, no store,
so each is testable against fixture JSON cut from a real run.

### 2.1 `runAttribution.ts` — the port

Inputs per interval `(a, b]`: force `production.made` at `a` and `b` (step
function, last sample at or before, as `production_at` does); machine
`produced` per machine at `a` and `b` for the item (by `recipe`, `mining`, or
furnace output as `machine_production` resolves it); count of feeding
dispatches (`insert`, `stock`, `charge`, `fuel`, `take`, `mine`) in the
interval. Output: `{verdict, machine_made, roster_made, feeds, source, why}`
with the same five verdicts and the same thresholds (`machine_made > delta *
1.05 + 1` → `unclear`; `0` → `hand-made`; share ≥ 0.95 with feeds →
`roster-fed`, without → `factory`; else `mixed`). Runs whose machine rows carry
no counters get `source: "inference"` through a port of
`_attribute_by_inference`, and the band labels it *inferred*.

**Golden test.** `app/src/lib/__fixtures__/run-1788696619-00325/` holds the
run's `samples.jsonl`, `events.jsonl` (events under 100 KB; samples trimmed to
force+machines, ~200 KB) and the JSON `just analyse --json` emits for it. The
test asserts verdict, `machine_made` and `roster_made` per item per mark equal
the tool's. Drift in either implementation fails the suite.

### 2.2 Phase 2 routes (additive, through the OpenAPI seam)

| Route | Body | Why |
|---|---|---|
| `GET /api/v1/runs/{id}/provenance` | `provenance.json` verbatim (`Provenance` gains `ToSchema`) | chips; comparability |
| `RunSummary` gains `samples`, `map`, `samples_lag_ticks` | from `manifest.json` | coverage band |
| `GET /api/v1/runs/{id}/samples?from=&to=&kind=` and `/map?from=&to=` | filtered lines | large archives; heatmap wants machines only |
| `runs/<run>/replay.json` written at run end; `GET /api/v1/runs/{id}/replay` | the executor `Replay` | evidence markers survive the 50-job cap and restarts |
| `GET /api/v1/runs/{id}/savepoints` | the `savepoints/*.json` list | copyable `--resume-from` per milestone |

Each: utoipa schema → `UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p
factorio-bot-server --features lua --test openapi` → `types.ts` mirror →
`objectContract<T>`; then `pnpm lint`, which is part of the seam.

### 2.3 Phase 3 — `FlowExport`

New in `crates/core/src/graph/flow_export.rs`:

```rust
#[derive(Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct FlowExport { tick: u64, nodes: Vec<FlowExportNode>, edges: Vec<FlowExportEdge> }
pub struct FlowExportNode { id: u32, position: Position, name: String, kind: String,
                            recipe: Option<String>, miner_ore: Option<String> }
pub struct FlowExportEdge { from: u32, to: u32, lanes: Vec<Vec<(String, f64)>> } // 1 or 2 lanes
```

Built by `FlowGraph::export()` from `inner_graph()` after `ensure_current()`;
rates in the name-sorted order the graph guarantees since `152a3ba0`, never
re-sorted by value. Written by the recorder as `flow.jsonl` at every keyframe
(same trigger as `record.keyframe()`), served at `/runs/{id}/flow`, and by
`/api/v1/game/flow` for the running world. The page joins each node to its
machine sample by position for the measured side; the panel header says
"modelled capacity vs measured" in every state.

## 3 · Components

New under `app/src/components/run/` (multi-word names; none need the ESLint
ignore list): `RunHeadline`, `MilestoneRibbon`, `TickAxis`, `ProductionBand`,
`PowerBand`, `ResearchBand`, `LaneBand`, `MachineBand`, `CoverageBand`,
`CursorBar`, `RunSidePanel` (map/video tabs), and in Phase 3 `FlowPanel`.
All bands are SVG with a fixed `viewBox` width and a shared left gutter, so
one `tickX(t)` from `TickAxis` positions everything. `RunsPage.vue` becomes a
thin composition over `runsStore`; its scoped hex CSS and the undefined
`--surface-border` / `--muted` fallbacks are replaced by theme tokens.

Theme tokens added to `app/src/assets/tailwind.css`: dark counterparts of
every existing `--color-*`, plus new reserved sets: `--color-status-{good,
warn,serious,critical,neutral}`, `--color-verb-{walk,mine,craft,place,feed,
research}` (the dataviz reference order, validated), `--color-bot-{1..8}`
(fixed by bot id), `--color-item-{iron,copper,circuit,gear,red}`. The `warn-dark`
class bug in `ReplayScrubber.vue` is fixed in passing by defining the token.

## 4 · Error handling

- Each enrichment fails independently (`allSettled`), as today; a band whose
  input failed renders its label and a one-line reason in place, never an
  empty plot.
- A 404 on a Phase 2 route reads "this server does not provide …" (the
  existing `enrichmentUnavailable` pattern), so an old server and a new UI
  degrade to Phase 1 cleanly.
- Samples older than schema 3 or lacking `produced` switch attribution to
  `inference` and say so. `machines: {}` with `truncated > 0` shows the
  truncation count on the heatmap.
- A `Position` with a non-finite float in `flow.jsonl` is skipped per node with
  a count, never a crash.

## 5 · Testing

- **Libs:** vitest over fixture JSON; the golden attribution test above; idle
  intervals merged (overlap-aware, `busy > 100 %` never appears); replan
  boundary refuses cross-plan joins; coverage reports `null` lag as unknown.
- **Components:** DOM assertions in the `MapPanel.spec.ts` style — segment
  `x`/`width`, heatmap cell fill by status, verdict rect per interval, cursor
  transform, `<title>` wording (the wording is the claim).
- **Seam:** Rust snapshot test, TS contract test, `pnpm lint` (vitest does not
  type-check; a stale test helper has already slipped through).
- **Rust:** `FlowExport` round-trips; `export()` on the fixture graph yields
  name-sorted rates; the recorder writes `flow.jsonl` at each keyframe.
- **Visual:** the coverage gate stays enforced (`pnpm run test:coverage`).

## 6 · Phasing

| Phase | Scope | Server change |
|---|---|---|
| 1 | libs, bands, headline, page recomposition, tokens + dark mode, `/runs/:id` route | none |
| 2 | provenance, manifest fields, tick/kind slices, persisted replay, savepoints | additive routes |
| 3 | `FlowExport`, `flow.jsonl`, `/runs/{id}/flow`, `/game/flow`, `FlowPanel` | one record file, two routes |

Each phase is shippable alone; Phase 1 is the whole visible redesign minus the
flow panel and the provenance chips.

## 7 · Risks

- **Attribution port drifts from the Python.** Mitigated by the golden test;
  the tool remains the reference until Rust hosts the rule.
- **Large archives.** `/samples` loads whole files today (hundreds of MB for
  video-era runs). Phase 1 tolerates it; Phase 2's slices fix it.
- **Keyframes carry no footprint.** Real-size entities need either a prototype
  lookup client-side (via `/game/entity-prototypes`, live only) or a `size`
  field on `EntitySnapshot`. Phase 1 falls back to the current uniform square
  for unknown names and a small hand table for the common machines.
- **Peer work in flight.** `WalkSettled.stalls` (`9f55d023`) already changed
  `types.ts`; rebase before touching `app/src/api/`. The planner and core graph
  files the peer listed are not touched by Phases 1–2; Phase 3's
  `flow_export.rs` is a new file beside `flow_graph.rs`.
