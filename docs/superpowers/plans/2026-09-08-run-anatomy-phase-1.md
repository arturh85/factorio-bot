# Run Anatomy Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the `/runs` page as one tick-axis instrument — rate curves with the attribution verdict painted in, idle-aware bot lanes, a machine status heatmap, a coverage band, a headline, and the map/video in a side panel — using only the routes the server already serves.

**Architecture:** Six pure libs under `app/src/lib/` turn the already-fetched `samples`, `lanes`, `events` and `map` into view models; one `tickScale` places everything on a shared 1000-unit SVG axis; band components render SVG from those view models and read one cursor from `runsStore`. The attribution rule is a TypeScript port of `attribute_from_counters` in `tools/run_analysis.py`, pinned by a golden test to that tool's JSON output for one committed fixture run. Theme tokens gain dark counterparts and the page moves onto them.

**Tech Stack:** Vue 3, Pinia 4, Tailwind v4 (CSS-first `@theme`), vitest + `@vue/test-utils` (jsdom per file), TypeScript, `@lucide/vue`.

**Spec:** `docs/superpowers/specs/2026-09-08-run-anatomy-design.md` (§1 page anatomy, §2 data flow, §3 components, §4 error handling, §5 testing; Phase 1 row of §6).

## Global Constraints

- Every cargo command needs `nix develop -c`; this plan runs none (frontend only). Frontend commands run from `app/`: `pnpm test`, `pnpm run test:coverage`, `pnpm lint`, `pnpm run build:web`.
- **`pnpm lint` is part of the seam** — vitest does not type-check. Run it before calling any task done.
- Commit with explicit paths only: `git add -- <paths>` then `git commit -F <msgfile> -- <paths>`. Write commit messages to a file with a **quoted** heredoc (`<<'EOF'`): a backtick in `-m "..."` is command substitution. Never `git add -A`, never `--amend`, never `git checkout -- <file>`, never `git stash`.
- Never `cargo fmt --all`. Not relevant here (no Rust), stated for completeness.
- Do not touch `app/src/api/types.ts` or `app/src/api/openapi.snapshot.json` in this phase; no wire types change. Pull first: `WalkSettled.stalls` (`9f55d023`) already landed in both.
- `crates/core/tests/` is being consolidated by another session; this phase adds no Rust and no files there.
- Absent is not zero: every lib returns `null` for "no data" and the components render that state by name. A mark the run never reached is reported by status, never as 0/min.
- Never join anything on action `id` across a `plan_created`; segments carry `(planIndex, id)`.
- One axis per chart. Categorical colours are assigned in fixed order by entity (bot id, verb class), never cycled; status and verdict colours are a reserved set and always carry a text label.
- Coverage gate stays enforced (statements 90 / branches 80 / functions 90 / lines 90 over `src/api`, `src/store`, `src/lib`, `src/composables`, `src/components/ui`). New libs live in `src/lib/` and are covered; band components live in `src/components/run/` (outside the gate) but each still gets a DOM spec.
- New Vue components use multi-word names, so `eslint.config.mjs`'s ignore list needs no change.
- Component specs start with `// @vitest-environment jsdom` on line 1 (vitest defaults to node).

---

## File structure

**Create**
- `app/src/lib/__fixtures__/run-1788696619-00325/{events.jsonl,samples.jsonl,lanes.json,rates.json,README.md}` — one real run, and `just analyse --json` rates for it (golden).
- `app/src/lib/fixtureRun.ts` — loads that fixture in node tests.
- `app/src/lib/tickScale.ts` — `tickX`, `TickScale`, `formatGameTime`.
- `app/src/lib/runRates.ts` — cumulative → items/min, marks by status.
- `app/src/lib/runAttribution.ts` — `machineProduction`, `feedingDispatches`, `attributeInterval`, `attributionIntervals`.
- `app/src/lib/runIdle.ts` — verb classes, idle intervals, replan boundaries.
- `app/src/lib/machineTimeline.ts` — machine rows, status matrix, status class.
- `app/src/lib/runCoverage.ts` — stream extents.
- `app/src/composables/useTheme.ts` — explicit light/dark/system, stored per viewer.
- `app/src/components/ThemeToggle.vue` — mounted in `AppTopbar.vue`.
- `app/src/components/run/{TickAxis,ProductionBand,PowerBand,ResearchBand,LaneBand,MachineBand,CoverageBand,RunHeadline,MilestoneRibbon,CursorBar,RunSidePanel}.vue` + specs.
- `app/src/pages/RunPage.vue` — one run at `/runs/:id`.

**Modify**
- `app/src/assets/tailwind.css` — dark tokens, status/verb/bot/item token sets, `--color-warn-dark`.
- `app/src/store/runsStore.ts` — fetch `/events` as a sixth enrichment; new getters.
- `app/src/router.ts` — add `/runs/:id`.
- `app/src/pages/RunsPage.vue` — becomes the list; opens `/runs/:id`.
- `app/src/AppTopbar.vue` — mount `ThemeToggle`.

`RunAnalysisPage.vue` and its route stay untouched in Phase 1 (D3's fold-in is Phase 2 work once the replay route exists); `RunPage.vue` links to it.

---

### Task 1: Fixture run and loader

**Files:**
- Create: `app/src/lib/__fixtures__/run-1788696619-00325/README.md`, `events.jsonl`, `samples.jsonl`, `lanes.json`, `rates.json`
- Create: `app/src/lib/fixtureRun.ts`
- Test: `app/src/lib/fixtureRun.spec.ts`

**Interfaces:**
- Produces: `loadFixtureRun(): FixtureRun` where `FixtureRun = {events: Event[]; samples: Sample[]; lanes: Lane[]; rates: RatesGolden; lo: 3242; hi: 25224}` and `RatesGolden` mirrors the `rates` object of `run_analysis.py --json` (`marks[].items[item].{verdict, machine_made, roster_made, rate_interval, rate_window, made_in_interval, cumulative}`, `marks[].status`, `marks[].tick`, `items`).

- [ ] **Step 1: Copy the fixture from the archive (samples trimmed to `force` and `machines`; `bots` lines are 367 of 515 lines and no Phase 1 lib reads them)**

```bash
cd /home/arturh/projects/private/factorio-bot
D=app/src/lib/__fixtures__/run-1788696619-00325
mkdir -p $D
cp workspace/runs/run-1788696619-00325/events.jsonl $D/events.jsonl
grep -E '"kind":"(force|machines)"' workspace/runs/run-1788696619-00325/samples.jsonl > $D/samples.jsonl
curl -s http://localhost:7492/api/v1/runs/run-1788696619-00325/lanes > $D/lanes.json
python3 tools/run_analysis.py workspace/runs/run-1788696619-00325 --json \
  | python3 -c "import json,sys; print(json.dumps(json.load(sys.stdin)['rates'], indent=1))" > $D/rates.json
wc -c $D/*
```

Expected: `events.jsonl` ≈ 104 KB, `samples.jsonl` ≈ 330 KB, `lanes.json` ≈ 30 KB, `rates.json` ≈ 40 KB. If the viewer is not running on 7492, start it with `just serve` in another terminal first; the lanes are derived from the events on the server and nothing else produces them.

- [ ] **Step 2: Write the README**

```markdown
# Fixture run `run-1788696619-00325`

Seed 31337, four client bots, 1.0x, release `492e513a` clean, `research automation`
satisfied at 6:06 (tick 25216). Copied from `workspace/runs/` on 2026-09-08.

- `events.jsonl` — verbatim.
- `samples.jsonl` — `force` and `machines` lines only (`bots` lines dropped; nothing here reads them).
- `lanes.json` — `GET /api/v1/runs/{id}/lanes` verbatim.
- `rates.json` — the `rates` object of `python3 tools/run_analysis.py <dir> --json`.
  **This is the golden output for `runAttribution.spec.ts` and `runRates.spec.ts`.**
  Regenerate it only together with a deliberate change to the Python rule.

The run's window is ticks 3242 (`run_started`) to 25224 (`run_finished`).
```

- [ ] **Step 3: Write the failing loader test**

```ts
// app/src/lib/fixtureRun.spec.ts
import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';

describe('loadFixtureRun', () => {
    it('loads the archived run with its golden rates', () => {
        const run = loadFixtureRun();
        expect(run.lo).toBe(3242);
        expect(run.hi).toBe(25224);
        expect(run.events.filter((e) => e.kind === 'action_dispatched')).toHaveLength(176);
        expect(run.samples.filter((s) => s.kind === 'force')).toHaveLength(74);
        expect(run.samples.filter((s) => s.kind === 'machines')).toHaveLength(74);
        expect(run.lanes).toHaveLength(228);
        expect(run.rates.marks[0].label).toBe('5:00');
    });
});
```

- [ ] **Step 4: Run it to verify it fails**

Run: `cd app && pnpm vitest run src/lib/fixtureRun.spec.ts`
Expected: FAIL — `Cannot find module './fixtureRun'`.

- [ ] **Step 5: Write the loader**

```ts
// app/src/lib/fixtureRun.ts
/**
 * One real archived run, for tests that must agree with `tools/run_analysis.py`.
 *
 * Node-only (reads the filesystem); never imported by app code. The golden
 * `rates.json` is what pins `runAttribution.ts` to the Python rule.
 */
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {Event, Lane, Sample} from '@/api/types';

export type Verdict = 'roster-fed' | 'factory' | 'hand-made' | 'mixed' | 'unclear' | 'no output';

export interface GoldenItem {
    cumulative: number | null;
    made_in_interval: number | null;
    rate_interval: number | null;
    rate_window: number | null;
    machine_made?: number;
    roster_made?: number;
    source?: string;
    verdict: Verdict;
    why: string;
}

export interface GoldenMark {
    minute: number;
    label: string;
    tick: number;
    is_end: boolean;
    status: 'ok' | 'run_ended' | 'samples_end' | 'no_sample';
    sample_tick: number | null;
    lag_ticks: number | null;
    items: Record<string, GoldenItem>;
    attribution?: {from_tick: number; roster: {feed_actions: number}};
}

export interface RatesGolden {
    origin_tick: number;
    end_tick: number;
    window_minutes: number;
    items: string[];
    marks: GoldenMark[];
    first_generation_tick: number | null;
}

export interface FixtureRun {
    events: Event[];
    samples: Sample[];
    lanes: Lane[];
    rates: RatesGolden;
    lo: number;
    hi: number;
}

const DIR = join(__dirname, '__fixtures__', 'run-1788696619-00325');

function jsonl<T>(file: string): T[] {
    return readFileSync(join(DIR, file), 'utf8')
        .split('\n')
        .filter((line) => line.trim().length > 0)
        .map((line) => JSON.parse(line) as T);
}

export function loadFixtureRun(): FixtureRun {
    const events = jsonl<Event>('events.jsonl');
    const samples = jsonl<Sample>('samples.jsonl');
    const lanes = (JSON.parse(readFileSync(join(DIR, 'lanes.json'), 'utf8')) as {lanes: Lane[]}).lanes;
    const rates = JSON.parse(readFileSync(join(DIR, 'rates.json'), 'utf8')) as RatesGolden;
    return {events, samples, lanes, rates, lo: rates.origin_tick, hi: rates.end_tick};
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cd app && pnpm vitest run src/lib/fixtureRun.spec.ts`
Expected: PASS (1 test).

- [ ] **Step 7: Lint and commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
test(app): one archived run as a fixture, with `just analyse --json` as its golden

`run-1788696619-00325` (seed 31337, four bots, automation at 6:06) copied
under `app/src/lib/__fixtures__/`, plus the `rates` object the analysis tool
emits for it. Every attribution and rate lib in the run page is pinned to
this file so the TypeScript port and the Python rule cannot drift silently.
EOF
git add -- app/src/lib/__fixtures__/run-1788696619-00325 app/src/lib/fixtureRun.ts app/src/lib/fixtureRun.spec.ts
git commit -F /tmp/msg -- app/src/lib/__fixtures__/run-1788696619-00325 app/src/lib/fixtureRun.ts app/src/lib/fixtureRun.spec.ts
```

---

### Task 2: `tickScale.ts` — one axis for every band

**Files:**
- Create: `app/src/lib/tickScale.ts`
- Test: `app/src/lib/tickScale.spec.ts`

**Interfaces:**
- Produces: `interface TickScale {from: number; to: number}`; `AXIS_WIDTH = 1000`; `tickX(scale, tick): number` (0..1000, clamped); `formatGameTime(scale, tick): string` ("m:ss" from `scale.from`); `markTicks(scale, everyMinutes = 5): number[]`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/tickScale.spec.ts
import {describe, expect, it} from 'vitest';
import {AXIS_WIDTH, formatGameTime, markTicks, tickX} from './tickScale';

const scale = {from: 3242, to: 25224};

describe('tickX', () => {
    it('maps the axis start to 0 and the end to AXIS_WIDTH', () => {
        expect(tickX(scale, 3242)).toBe(0);
        expect(tickX(scale, 25224)).toBe(AXIS_WIDTH);
    });
    it('clamps outside the axis rather than drawing off the SVG', () => {
        expect(tickX(scale, 0)).toBe(0);
        expect(tickX(scale, 99999)).toBe(AXIS_WIDTH);
    });
    it('returns 0 for a zero-span scale instead of NaN', () => {
        expect(tickX({from: 5, to: 5}, 5)).toBe(0);
    });
});

describe('formatGameTime', () => {
    it('counts minutes and seconds from the axis start', () => {
        expect(formatGameTime(scale, 3242)).toBe('0:00');
        expect(formatGameTime(scale, 3242 + 18000)).toBe('5:00');
        expect(formatGameTime(scale, 25216)).toBe('6:06');
    });
});

describe('markTicks', () => {
    it('lists the 5-minute marks inside the axis, excluding the origin', () => {
        expect(markTicks(scale)).toEqual([3242 + 18000]);
        expect(markTicks({from: 0, to: 40000}, 5)).toEqual([18000, 36000]);
    });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd app && pnpm vitest run src/lib/tickScale.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/tickScale.ts
/**
 * The one tick axis every run-page band draws on.
 *
 * Bands are SVGs with `viewBox="0 0 1000 H"`, so a tick maps to an x in
 * 0..1000 and every band agrees on where a tick is without sharing a DOM
 * width. Clamped: a tick outside the axis lands on its edge, never off the
 * drawing.
 */
export interface TickScale {
    from: number;
    to: number;
}

export const AXIS_WIDTH = 1000;
export const TICKS_PER_MINUTE = 3600;

export function tickX(scale: TickScale, tick: number): number {
    const span = scale.to - scale.from;
    if (span <= 0) return 0;
    const clamped = Math.min(scale.to, Math.max(scale.from, tick));
    return ((clamped - scale.from) / span) * AXIS_WIDTH;
}

/** "m:ss" of game time since the axis start; ticks before it read 0:00. */
export function formatGameTime(scale: TickScale, tick: number): string {
    const seconds = Math.max(0, Math.round((tick - scale.from) / 60));
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, '0')}`;
}

/** The fixed game-time marks (5, 10, 15 … minutes from the start) inside the axis. */
export function markTicks(scale: TickScale, everyMinutes = 5): number[] {
    const step = everyMinutes * TICKS_PER_MINUTE;
    const out: number[] = [];
    for (let t = scale.from + step; t <= scale.to; t += step) out.push(t);
    return out;
}
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/tickScale.spec.ts` — Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `tickScale`, the one axis every run band draws on
EOF
git add -- app/src/lib/tickScale.ts app/src/lib/tickScale.spec.ts
git commit -F /tmp/msg -- app/src/lib/tickScale.ts app/src/lib/tickScale.spec.ts
```

---

### Task 3: `runRates.ts` — items per minute, marks by status

**Files:**
- Create: `app/src/lib/runRates.ts`
- Test: `app/src/lib/runRates.spec.ts`

**Interfaces:**
- Consumes: `Sample` from `@/api/types`; `loadFixtureRun` (Task 1); `TICKS_PER_MINUTE` (Task 2).
- Produces:
  - `forceAt(samples, tick): ForceSample | null` (latest `force` sample at or before `tick`).
  - `madeAt(samples, tick, item, baseTick): number | null` — cumulative relative to the sample at `baseTick` (the Python `count()`); `null` when no sample at or before `tick`.
  - `rateSeries(samples, item, lo, hi, windowTicks = 7200): RatePoint[]` with `RatePoint = {tick: number; perMinute: number}` — trailing-window items/min at every force sample in `[lo, hi]`, window clipped to `lo`.
  - `markAt(samples, lo, hi, minute, prevMinute, item): Mark` with `Mark = {minute, tick, status: 'ok'|'run_ended'|'samples_end'|'no_sample', cumulative: number|null, madeInInterval: number|null, rateInterval: number|null, rateWindow: number|null}`.
  - `rateItems(samples, lo, hi, threshold = 100): string[]` — the six default items plus any whose final made ≥ threshold (alphabetical after the defaults).
  - `DEFAULT_RATE_ITEMS`, `RATE_WINDOW_MINUTES = 2`, `DEFAULT_MARKS = [5, 10, 15, 20, 25, 30]`.

- [ ] **Step 1: Write the failing tests (golden against `rates.json`)**

```ts
// app/src/lib/runRates.spec.ts
import {describe, expect, it} from 'vitest';
import {Sample} from '@/api/types';
import {loadFixtureRun} from './fixtureRun';
import {DEFAULT_MARKS, forceAt, madeAt, markAt, rateItems, rateSeries} from './runRates';

const force = (tick: number, made: Record<string, number> = {}): Sample => ({
    kind: 'force', research: null, techs_unlocked: 0,
    production: {made, consumed: {}}, pollution: null,
    power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1, networks: {}},
    schema: 3, tick, run: 'r'
});

describe('madeAt', () => {
    it('is relative to the sample at the origin, so a resumed run reports what it made', () => {
        const s = [force(100, {'iron-plate': 40}), force(400, {'iron-plate': 55})];
        expect(madeAt(s, 400, 'iron-plate', 100)).toBe(15);
    });
    it('is null before the first sample, not zero', () => {
        expect(madeAt([force(100, {'iron-plate': 1})], 50, 'iron-plate', 100)).toBeNull();
    });
    it('carries an item missing from a later sample as zero made, not as a drop', () => {
        // The mod omits items with no production; `made[item] ?? 0` is right
        // for cumulative counters because a counter never goes down.
        const s = [force(100, {}), force(400, {'iron-plate': 3})];
        expect(madeAt(s, 400, 'iron-plate', 100)).toBe(3);
    });
});

describe('rateSeries', () => {
    it('reports trailing-window items per minute, clipped to the origin', () => {
        const s = [force(0, {}), force(3600, {x: 60}), force(7200, {x: 60}), force(10800, {x: 120})];
        const pts = rateSeries(s, 'x', 0, 10800, 7200);
        expect(pts.map((p) => p.tick)).toEqual([0, 3600, 7200, 10800]);
        // 3600: window clipped to [0,3600] -> 60 over 1 min
        expect(pts[1].perMinute).toBeCloseTo(60);
        // 7200: [0,7200] -> 60 over 2 min
        expect(pts[2].perMinute).toBeCloseTo(30);
        // 10800: [3600,10800] -> 60 over 2 min
        expect(pts[3].perMinute).toBeCloseTo(30);
    });
});

describe('markAt against just analyse --json', () => {
    const run = loadFixtureRun();
    it('matches every mark status and every rate on the fixture run', () => {
        let prev = 0;
        for (const golden of run.rates.marks.filter((m) => !m.is_end)) {
            for (const item of run.rates.items) {
                const mark = markAt(run.samples, run.lo, run.hi, golden.minute, prev, item);
                expect(mark.status, `${golden.label} ${item}`).toBe(golden.status);
                expect(mark.tick).toBe(golden.tick);
                if (golden.status !== 'ok') continue;
                const g = golden.items[item];
                expect(mark.cumulative).toBe(g.cumulative);
                expect(mark.madeInInterval).toBe(g.made_in_interval);
                expect(mark.rateInterval).toBeCloseTo(g.rate_interval as number, 6);
                expect(mark.rateWindow).toBeCloseTo(g.rate_window as number, 6);
            }
            prev = golden.minute;
        }
    });
    it('reports a mark past the run by status, never as zero', () => {
        const m = markAt(run.samples, run.lo, run.hi, 10, 5, 'iron-plate');
        expect(m.status).toBe('run_ended');
        expect(m.cumulative).toBeNull();
        expect(m.rateInterval).toBeNull();
    });
    it('lists the six default items plus anything over the threshold', () => {
        expect(rateItems(run.samples, run.lo, run.hi)).toEqual(run.rates.items);
        expect(DEFAULT_MARKS).toEqual([5, 10, 15, 20, 25, 30]);
    });
    it('forceAt is the last sample at or before the tick', () => {
        expect(forceAt(run.samples, 20100)?.tick).toBe(20100);
        expect(forceAt(run.samples, 20399)?.tick).toBe(20100);
        expect(forceAt(run.samples, 100)).toBeNull();
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/runRates.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/runRates.ts
/**
 * Production as RATES, not counts, on fixed game-time marks.
 *
 * A port of `production_rates()` in `tools/run_analysis.py`, pinned to its
 * JSON output by `runRates.spec.ts`. The owner's rule is "production rates at
 * given times over raw run time", so the page reads items per game minute at
 * 5/10/15… minutes from `run_started`, exactly where the tool reads them.
 *
 * A mark the run never reached is reported by `status`, never as 0/min: a
 * zero and a missing measurement must not render alike.
 */
import {Sample} from '@/api/types';
import {TICKS_PER_MINUTE} from './tickScale';

export type ForceSample = Extract<Sample, {kind: 'force'}>;

export const DEFAULT_RATE_ITEMS = [
    'iron-plate',
    'copper-plate',
    'iron-gear-wheel',
    'electronic-circuit',
    'automation-science-pack',
    'logistic-science-pack'
];
export const RATE_ITEM_THRESHOLD = 100;
export const RATE_WINDOW_MINUTES = 2;
export const DEFAULT_MARKS = [5, 10, 15, 20, 25, 30];

export interface RatePoint {
    tick: number;
    perMinute: number;
}

export type MarkStatus = 'ok' | 'run_ended' | 'samples_end' | 'no_sample';

export interface Mark {
    minute: number;
    tick: number;
    status: MarkStatus;
    cumulative: number | null;
    madeInInterval: number | null;
    rateInterval: number | null;
    rateWindow: number | null;
}

function forceSamples(samples: Sample[]): ForceSample[] {
    return samples
        .filter((s): s is ForceSample => s.kind === 'force')
        .sort((a, b) => a.tick - b.tick);
}

/** The latest `force` sample at or before `tick`, or null before the first. */
export function forceAt(samples: Sample[], tick: number): ForceSample | null {
    let best: ForceSample | null = null;
    for (const s of forceSamples(samples)) {
        if (s.tick <= tick) best = s;
        else break;
    }
    return best;
}

/**
 * Cumulative `made` of `item` at `tick`, relative to the sample at `baseTick`.
 *
 * Relative so a `--resume-from` run reports what *it* made, not what its
 * savepoint already held. `null` when no sample exists at or before `tick`.
 */
export function madeAt(samples: Sample[], tick: number, item: string, baseTick: number): number | null {
    const at = forceAt(samples, tick);
    if (at === null) return null;
    const base = forceAt(samples, baseTick);
    const baseMade = base === null ? 0 : (base.production.made[item] ?? 0);
    return (at.production.made[item] ?? 0) - baseMade;
}

/** Trailing-window items per game minute at every force sample in `[lo, hi]`. */
export function rateSeries(
    samples: Sample[],
    item: string,
    lo: number,
    hi: number,
    windowTicks = RATE_WINDOW_MINUTES * TICKS_PER_MINUTE
): RatePoint[] {
    const out: RatePoint[] = [];
    for (const s of forceSamples(samples)) {
        if (s.tick < lo || s.tick > hi) continue;
        const winFrom = Math.max(lo, s.tick - windowTicks);
        const winMinutes = (s.tick - winFrom) / TICKS_PER_MINUTE;
        if (winMinutes <= 0) {
            out.push({tick: s.tick, perMinute: 0});
            continue;
        }
        const now = madeAt(samples, s.tick, item, lo) ?? 0;
        const then = madeAt(samples, winFrom, item, lo) ?? 0;
        out.push({tick: s.tick, perMinute: (now - then) / winMinutes});
    }
    return out;
}

/** One fixed mark, as `production_rates()` measures it. */
export function markAt(
    samples: Sample[],
    lo: number,
    hi: number,
    minute: number,
    prevMinute: number,
    item: string,
    windowMinutes = RATE_WINDOW_MINUTES
): Mark {
    const tick = lo + Math.round(minute * TICKS_PER_MINUTE);
    const force = forceSamples(samples);
    const lastTick = force.length > 0 ? force[force.length - 1].tick : null;
    const base: Mark = {
        minute, tick, status: 'ok',
        cumulative: null, madeInInterval: null, rateInterval: null, rateWindow: null
    };
    if (tick > hi) return {...base, status: 'run_ended'};
    if (lastTick === null || tick > lastTick) return {...base, status: 'samples_end'};
    if (forceAt(samples, tick) === null) return {...base, status: 'no_sample'};

    const prevTick = lo + Math.round(prevMinute * TICKS_PER_MINUTE);
    const winTick = Math.max(lo, tick - Math.round(windowMinutes * TICKS_PER_MINUTE));
    const winMinutes = (tick - winTick) / TICKS_PER_MINUTE;
    // `?? 0` mirrors the Python `count(prev_t) or 0`: a missing sample at the
    // interval start counts as nothing made yet.
    const c = madeAt(samples, tick, item, lo) as number;
    const cPrev = madeAt(samples, prevTick, item, lo) ?? 0;
    const cWin = madeAt(samples, winTick, item, lo) ?? 0;
    return {
        ...base,
        cumulative: c,
        madeInInterval: c - cPrev,
        rateInterval: minute > prevMinute ? (c - cPrev) / (minute - prevMinute) : null,
        rateWindow: winMinutes > 0 ? (c - cWin) / winMinutes : null
    };
}

/** The default items, then any other item whose final count clears the threshold. */
export function rateItems(samples: Sample[], lo: number, hi: number, threshold = RATE_ITEM_THRESHOLD): string[] {
    const force = forceSamples(samples).filter((s) => s.tick <= hi);
    if (force.length === 0) return [...DEFAULT_RATE_ITEMS];
    const final = force[force.length - 1].production.made;
    const base = forceAt(samples, lo)?.production.made ?? {};
    const extra = Object.keys(final)
        .filter((n) => !DEFAULT_RATE_ITEMS.includes(n) && (final[n] ?? 0) - (base[n] ?? 0) >= threshold)
        .sort();
    return [...DEFAULT_RATE_ITEMS, ...extra];
}
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/runRates.spec.ts` — Expected: PASS (8 tests). If the golden comparison fails on `rateWindow`, check `winTick` rounding against the Python (`int(round(window * TICKS_PER_MINUTE))`); the two must agree before anything else is built on this.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `runRates`, items per minute at fixed marks, pinned to `just analyse`

A port of `production_rates()`: trailing 2-minute window, marks at 5/10/15
minutes from `run_started`, counts relative to the origin sample, and a mark
the run never reached reported by status rather than as 0/min. The golden
test compares every mark and rate on the fixture run to the tool's JSON.
EOF
git add -- app/src/lib/runRates.ts app/src/lib/runRates.spec.ts
git commit -F /tmp/msg -- app/src/lib/runRates.ts app/src/lib/runRates.spec.ts
```

---
### Task 4: `runAttribution.ts` — who made it, from counters

**Files:**
- Create: `app/src/lib/runAttribution.ts`
- Test: `app/src/lib/runAttribution.spec.ts`

**Interfaces:**
- Consumes: `Event`, `Sample` from `@/api/types`; `forceAt`, `madeAt` (Task 3); `loadFixtureRun` (Task 1); `TICKS_PER_MINUTE` (Task 2).
- Produces:
  - `FEEDING_VERBS = ['insert','stock','charge','fuel','take','mine']`; `verbOf(action: string): string` (first word).
  - `feedingDispatches(events, lo, hi): number` — `action_dispatched` with a feeding verb and `lo < tick <= hi`.
  - `machineProduction(samples, lo, hi): MachineProduction` with `{available: boolean; byItem: Record<string, number>; byName: Record<string, Record<string, number>>; total: number; unattributed: number; shared: string[]}`.
  - `attributeInterval(samples, events, lo, hi, item): Attribution` with `Attribution = {verdict: Verdict; machineMade: number; rosterMade: number; feeds: number; delta: number; source: 'counters' | 'inference'; why: string}` and `Verdict = 'roster-fed' | 'factory' | 'hand-made' | 'mixed' | 'unclear' | 'no output'`.
  - `attributionIntervals(samples, events, lo, hi, item, stepTicks = 3600): (Attribution & {from: number; to: number})[]`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/runAttribution.spec.ts
import {describe, expect, it} from 'vitest';
import {Event, Sample} from '@/api/types';
import {loadFixtureRun} from './fixtureRun';
import {attributeInterval, attributionIntervals, feedingDispatches, machineProduction, verbOf} from './runAttribution';

const force = (tick: number, made: Record<string, number>): Sample => ({
    kind: 'force', research: null, techs_unlocked: 0, production: {made, consumed: {}}, pollution: null,
    power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1, networks: {}}, schema: 3, tick, run: 'r'
});
const machines = (tick: number, rows: Record<string, {name: string; recipe: string | null; produced: number | null; source: string | null}>): Sample => ({
    kind: 'machines', truncated: 0, schema: 3, tick, run: 'r',
    machines: Object.fromEntries(Object.entries(rows).map(([k, r]) => [k, {
        name: r.name, type: 'furnace', position: {x: 0, y: 0}, status: 'working', network: null,
        recipe: r.recipe, crafting: null, progress: null, products_finished: null,
        produced: r.produced, produced_source: r.source, produced_shared: false, mining: null,
        input: {}, output: {}, fuel: {}
    }]))
});
const dispatch = (tick: number, action: string): Event => ({
    kind: 'action_dispatched', tick, id: 1, bot: 1, action, target: null, delivery: null
} as Event);

describe('verbOf / feedingDispatches', () => {
    it('counts feeding verbs in (lo, hi], by count not ticks', () => {
        const ev = [dispatch(100, 'insert 2 coal'), dispatch(200, 'craft 1 pipe'), dispatch(300, 'mine 4 iron-ore'), dispatch(400, 'take 1 plate')];
        expect(verbOf('walk to [1, 2]')).toBe('walk');
        expect(feedingDispatches(ev, 100, 300)).toBe(1); // 100 excluded (lo <), 300 included
        expect(feedingDispatches(ev, 0, 400)).toBe(3);
    });
});

describe('machineProduction', () => {
    it('is unavailable, not zero, for rows without counters', () => {
        const s = [machines(300, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: null, source: null}})];
        expect(machineProduction(s, 0, 300).available).toBe(false);
    });
    it('names an idle furnace\'s output by the last recipe it was ever seen with', () => {
        const s = [
            machines(300, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 2, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: null, produced: 5, source: 'game'}})
        ];
        expect(machineProduction(s, 300, 600).byItem).toEqual({'iron-plate': 3});
    });
});

describe('attributeInterval', () => {
    const base = [force(0, {}), force(600, {'iron-plate': 10})];
    it('hand-made when no machine produced any', () => {
        const s = [...base, machines(0, {}), machines(600, {})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('hand-made');
    });
    it('roster-fed when machines made ≥95% and the roster fed', () => {
        const s = [...base, machines(0, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 10, source: 'game'}})];
        const a = attributeInterval(s, [dispatch(300, 'insert 5 iron-ore')], 0, 600, 'iron-plate');
        expect(a.verdict).toBe('roster-fed');
        expect(a.machineMade).toBe(10);
        expect(a.rosterMade).toBe(0);
    });
    it('factory when machines made ≥95% and nobody fed', () => {
        const s = [...base, machines(0, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 10, source: 'game'}})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('factory');
    });
    it('unclear when the machine counters exceed the force statistics', () => {
        const s = [...base, machines(0, {a: {name: 'burner-mining-drill', recipe: 'iron-plate', produced: 0, source: 'accumulated'}}),
            machines(600, {a: {name: 'burner-mining-drill', recipe: 'iron-plate', produced: 20, source: 'accumulated'}})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('unclear');
    });
    it('no output when nothing was made', () => {
        expect(attributeInterval([force(0, {}), force(600, {})], [], 0, 600, 'iron-plate').verdict).toBe('no output');
    });
});

describe('golden: every mark verdict equals just analyse --json', () => {
    const run = loadFixtureRun();
    it('agrees on verdict, machine_made and roster_made for every item at every reached mark', () => {
        let prevMinute = 0;
        for (const golden of run.rates.marks.filter((m) => !m.is_end && m.status === 'ok')) {
            const lo = run.lo + Math.round(prevMinute * 3600);
            const hi = golden.tick;
            for (const item of run.rates.items) {
                const g = golden.items[item];
                const a = attributeInterval(run.samples, run.events, lo, hi, item);
                expect(a.verdict, `${golden.label} ${item}`).toBe(g.verdict);
                if (g.verdict === 'no output') continue;
                expect(a.machineMade).toBe(g.machine_made);
                expect(a.rosterMade).toBe(g.roster_made);
                expect(a.feeds).toBe(golden.attribution?.roster.feed_actions);
            }
            prevMinute = golden.minute;
        }
    });
    it('paints one-minute intervals across the whole run', () => {
        const rows = attributionIntervals(run.samples, run.events, run.lo, run.hi, 'iron-plate');
        expect(rows).toHaveLength(7); // 21,982 ticks = 6.1 min -> 7 intervals, last one short
        expect(rows[0].from).toBe(run.lo);
        expect(rows[6].to).toBe(run.hi);
        expect(rows.every((r) => ['roster-fed', 'factory', 'hand-made', 'mixed', 'unclear', 'no output'].includes(r.verdict))).toBe(true);
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/runAttribution.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/runAttribution.ts
/**
 * WHO MADE IT. A port of `attribute_output` / `attribute_from_counters` /
 * `machine_production` / `feeding_dispatches` in `tools/run_analysis.py`,
 * pinned to that tool's JSON by `runAttribution.spec.ts`.
 *
 * `production.made` counts what a MACHINE produced, and a stone furnace a bot
 * hand-loaded is a machine, so a rising curve is not evidence of a working
 * factory. The split is arithmetic: what the machines' own counters say they
 * made against what the force's statistics say was made; the remainder is
 * hand work. The count of feeding dispatches — not their ticks, five of the
 * six settle in the tick they dispatch — decides roster-fed against factory.
 */
import {Event, MachineSample, Sample} from '@/api/types';
import {madeAt} from './runRates';
import {TICKS_PER_MINUTE} from './tickScale';

export const FEEDING_VERBS = ['insert', 'stock', 'charge', 'fuel', 'take', 'mine'];

export type Verdict = 'roster-fed' | 'factory' | 'hand-made' | 'mixed' | 'unclear' | 'no output';

export interface MachineProduction {
    /** False for a run archived before the counters existed; never "made nothing". */
    available: boolean;
    byItem: Record<string, number>;
    byName: Record<string, Record<string, number>>;
    total: number;
    unattributed: number;
    shared: string[];
}

export interface Attribution {
    verdict: Verdict;
    machineMade: number;
    rosterMade: number;
    feeds: number;
    delta: number;
    source: 'counters' | 'inference';
    why: string;
}

type MachinesSample = Extract<Sample, {kind: 'machines'}>;

export function verbOf(action: string): string {
    return action.trim().split(/\s+/)[0] ?? '';
}

/** Feeding-verb dispatches in `(lo, hi]` — the count, deliberately. */
export function feedingDispatches(events: Event[], lo: number, hi: number): number {
    let n = 0;
    for (const e of events) {
        if (e.kind !== 'action_dispatched') continue;
        if (!(lo < e.tick && e.tick <= hi)) continue;
        if (FEEDING_VERBS.includes(verbOf(e.action))) n += 1;
    }
    return n;
}

function itemOf(m: MachineSample): string | null {
    return m.recipe ?? m.mining ?? null;
}

/** What each machine itself produced in `(lo, hi]`, from its own counter. */
export function machineProduction(samples: Sample[], lo: number, hi: number): MachineProduction {
    const rows = samples
        .filter((s): s is MachinesSample => s.kind === 'machines')
        .sort((a, b) => a.tick - b.tick);
    let baseAt: MachinesSample | null = null;
    let endAt: MachinesSample | null = null;
    // The last item each machine was ever seen making, up to `hi`: an idle
    // stone furnace reports no recipe, and reading only the final row would
    // file every plate it smelted under "unattributed".
    const named = new Map<string, string>();
    for (const s of rows) {
        if (s.tick <= lo) baseAt = s;
        if (s.tick <= hi) endAt = s;
        else continue;
        for (const [key, m] of Object.entries(s.machines)) {
            const item = itemOf(m);
            if (item !== null) named.set(key, item);
        }
    }
    const baseRows = baseAt?.machines ?? {};
    const endRows = endAt?.machines ?? {};
    const out: MachineProduction = {
        available: Object.values(endRows).some((m) => m.produced_source !== null && m.produced_source !== undefined),
        byItem: {}, byName: {}, total: 0, unattributed: 0, shared: []
    };
    for (const [key, m] of Object.entries(endRows)) {
        const source = m.produced_source;
        if (source === 'not-a-producer' || source === 'unavailable') continue;
        if (source === null || source === undefined || m.produced === null) continue;
        const before = baseRows[key]?.produced ?? 0;
        const delta = m.produced - before;
        if (delta <= 0) continue;
        out.total += delta;
        const item = itemOf(m) ?? named.get(key) ?? null;
        if (item === null) {
            out.unattributed += delta;
            continue;
        }
        out.byItem[item] = (out.byItem[item] ?? 0) + delta;
        (out.byName[item] ??= {})[m.name] = (out.byName[item]?.[m.name] ?? 0) + delta;
        if (m.produced_shared) out.shared.push(m.name);
    }
    return out;
}

function fromCounters(delta: number, produced: MachineProduction, item: string, feeds: number): Attribution {
    const machineMade = produced.byItem[item] ?? 0;
    const names = Object.entries(produced.byName[item] ?? {}).map(([n, c]) => `${n}x${c}`).join(', ');
    const fed = `; the roster ran ${feeds} feeding action(s)`;
    const common = {machineMade, rosterMade: Math.max(0, delta - machineMade), feeds, delta, source: 'counters' as const};
    if (machineMade > delta * 1.05 + 1) {
        const shared = [...new Set(produced.shared)].sort().join(', ') || 'none flagged';
        return {...common, verdict: 'unclear', why: `the machines' own counters say ${machineMade} while the force's statistics say ${delta} was made -- they cannot both be right. Drills sharing a resource tile double-count and are flagged: ${shared}`};
    }
    if (machineMade === 0) {
        return {...common, verdict: 'hand-made', why: `no machine produced any of the ${delta} made in this interval -- every one of them came out of the roster's own hands (hand crafting and hand mining pass through no machine)${fed}`};
    }
    const share = machineMade / delta;
    if (share >= 0.95) {
        if (feeds > 0) {
            return {...common, verdict: 'roster-fed', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the roster ran ${feeds} feeding action(s), so the machines produced it and the bots carried what went in`};
        }
        return {...common, verdict: 'factory', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the roster fed nothing in this interval`};
    }
    return {...common, verdict: 'mixed', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the remaining ${delta - machineMade} was hand-made${fed}`};
}

/**
 * The fallback for a run archived before the per-machine counters existed.
 * It INFERS, and says so in `source`: feeding dispatches plus any generation
 * decide, and `unclear` is said freely.
 */
function byInference(delta: number, feeds: number, samples: Sample[], lo: number, hi: number): Attribution {
    const anyGeneration = samples.some((s) => s.kind === 'force' && s.tick > lo && s.tick <= hi && s.power.generated_kw > 0);
    const common = {machineMade: 0, rosterMade: 0, feeds, delta, source: 'inference' as const};
    if (feeds > 0 && !anyGeneration) return {...common, verdict: 'roster-fed', why: `the roster ran ${feeds} feeding action(s) and nothing generated electricity, so the bots carried what the machines ate (inferred: this run has no machine counters)`};
    if (feeds === 0 && anyGeneration) return {...common, verdict: 'factory', why: 'electricity was drawn and the roster fed nothing in this interval (inferred: this run has no machine counters)'};
    return {...common, verdict: 'unclear', why: `${feeds} feeding action(s) and ${anyGeneration ? 'some' : 'no'} generation -- the record cannot separate the roster from the factory here (inferred)`};
}

/** Who earned `item`'s output over `(lo, hi]`. */
export function attributeInterval(samples: Sample[], events: Event[], lo: number, hi: number, item: string): Attribution {
    const c = madeAt(samples, hi, item, lo);
    const cPrev = madeAt(samples, lo, item, lo) ?? 0;
    const delta = c === null ? 0 : c - cPrev;
    if (delta <= 0) {
        return {verdict: 'no output', machineMade: 0, rosterMade: 0, feeds: 0, delta: 0, source: 'counters', why: 'nothing made in this interval'};
    }
    const feeds = feedingDispatches(events, lo, hi);
    const produced = machineProduction(samples, lo, hi);
    if (produced.available) return fromCounters(delta, produced, item, feeds);
    return byInference(delta, feeds, samples, lo, hi);
}

/** The verdict per fixed-width interval across `[lo, hi]`; the last interval may be short. */
export function attributionIntervals(
    samples: Sample[], events: Event[], lo: number, hi: number, item: string, stepTicks = TICKS_PER_MINUTE
): (Attribution & {from: number; to: number})[] {
    const out: (Attribution & {from: number; to: number})[] = [];
    for (let from = lo; from < hi; from += stepTicks) {
        const to = Math.min(hi, from + stepTicks);
        out.push({from, to, ...attributeInterval(samples, events, from, to, item)});
    }
    return out;
}
```

Note on `madeAt(samples, lo, item, lo)`: relative to itself this is 0 whenever a sample exists at or before `lo`, and `null → 0` otherwise; both match the Python `count(prev_t) or 0` against the same base. Keep it written this way so the mark test and the interval test share one definition.

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/runAttribution.spec.ts` — Expected: PASS (10 tests). The golden test is the one that matters: a mismatch there is a porting bug, never a fixture to edit.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `runAttribution`, the roster-fed / factory verdict, pinned to the Python

A port of `attribute_from_counters` and `machine_production`: machine
counters against the force's statistics, hand work as the remainder, and the
COUNT of feeding dispatches deciding roster-fed against factory. Every mark
verdict on the fixture run equals `just analyse --json`.
EOF
git add -- app/src/lib/runAttribution.ts app/src/lib/runAttribution.spec.ts
git commit -F /tmp/msg -- app/src/lib/runAttribution.ts app/src/lib/runAttribution.spec.ts
```

---

### Task 5: `runIdle.ts` — verb classes, idle intervals, replan boundaries

**Files:**
- Create: `app/src/lib/runIdle.ts`
- Test: `app/src/lib/runIdle.spec.ts`

**Interfaces:**
- Consumes: `Lane`, `Event` from `@/api/types`; `verbOf` (Task 4); `loadFixtureRun` (Task 1).
- Produces:
  - `type VerbClass = 'walk' | 'mine' | 'craft' | 'place' | 'feed' | 'research' | 'other'`; `verbClass(action: string): VerbClass`.
  - `interface LaneSegment {bot: number; planIndex: number; id: number | null; action: string; verb: VerbClass; from: number; to: number | null; status: string | null; error: string | null; instant: boolean}`.
  - `laneSegments(lanes: Lane[], boundaries: number[]): LaneSegment[]` — `planIndex` = number of boundaries ≤ `from`.
  - `replanBoundaries(events: Event[]): number[]` — ticks of every `plan_created`, ascending.
  - `idleIntervals(lanes: Lane[], bot: number, scale: {from; to}): {from: number; to: number}[]` — merged gaps not covered by any of the bot's lanes (an unterminated lane covers to `scale.to`).
  - `idleTicks(intervals): number`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/runIdle.spec.ts
import {describe, expect, it} from 'vitest';
import {Event, Lane} from '@/api/types';
import {loadFixtureRun} from './fixtureRun';
import {idleIntervals, idleTicks, laneSegments, replanBoundaries, verbClass} from './runIdle';

const lane = (bot: number, from: number, to: number | null, action = 'mine 1 coal', id: number | null = 1): Lane =>
    ({bot, id, action, from_tick: from, to_tick: to, status: to === null ? null : 'success', error: null});

describe('verbClass', () => {
    it('groups the plan verbs into the six drawn classes', () => {
        expect(verbClass('walk to [1, 2]')).toBe('walk');
        expect(verbClass('mine 4 iron-ore')).toBe('mine');
        expect(verbClass('chop huge-rock')).toBe('mine');
        expect(verbClass('craft 2 pipe')).toBe('craft');
        expect(verbClass('place boiler at [1, 2]')).toBe('place');
        for (const v of ['insert', 'stock', 'take', 'fuel', 'charge']) expect(verbClass(`${v} 1 x`)).toBe('feed');
        expect(verbClass('research automation')).toBe('research');
        expect(verbClass('evacuate')).toBe('other');
    });
});

describe('replanBoundaries / laneSegments', () => {
    const plan = (tick: number): Event => ({kind: 'plan_created', tick, milestone_index: 1, steps: 1, makespan: 1, bots: [1], plan: []} as Event);
    it('numbers segments by the plan they were dispatched under', () => {
        const b = replanBoundaries([plan(100), plan(500)]);
        expect(b).toEqual([100, 500]);
        const segs = laneSegments([lane(1, 120, 130), lane(1, 600, 610)], b);
        expect(segs.map((s) => s.planIndex)).toEqual([1, 2]);
    });
    it('marks zero-length feeding acts as instant so they are drawn as ticks', () => {
        const [s] = laneSegments([lane(1, 100, 100, 'insert 2 coal')], []);
        expect(s.instant).toBe(true);
        expect(s.verb).toBe('feed');
    });
});

describe('idleIntervals', () => {
    const scale = {from: 0, to: 1000};
    it('is the axis minus the union of the bot\'s lanes', () => {
        const gaps = idleIntervals([lane(1, 100, 200), lane(1, 150, 300), lane(1, 600, 700)], 1, scale);
        expect(gaps).toEqual([{from: 0, to: 100}, {from: 300, to: 600}, {from: 700, to: 1000}]);
        expect(idleTicks(gaps)).toBe(700);
    });
    it('treats an unterminated lane as covering to the end of the axis', () => {
        expect(idleIntervals([lane(1, 100, null)], 1, scale)).toEqual([{from: 0, to: 100}]);
    });
    it('ignores other bots\' lanes', () => {
        expect(idleIntervals([lane(2, 0, 1000)], 1, scale)).toEqual([{from: 0, to: 1000}]);
    });
    it('reproduces the analysis tool\'s idle figure for bot 1 on the fixture run', () => {
        const run = loadFixtureRun();
        // `just analyse`: "busy 14998 + idle 6984 = 21982 ticks; idle is 31.8% of span"
        expect(idleTicks(idleIntervals(run.lanes, 1, {from: run.lo, to: run.hi}))).toBe(6984);
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/runIdle.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/runIdle.ts
/**
 * Idleness drawn, not implied.
 *
 * A verb histogram cannot see waiting: five feeding verbs settle in the tick
 * they dispatch and idle time appears nowhere. So the lane's background is
 * the idle state and only dispatched work paints over it; this file computes
 * the gaps (overlap-aware — a bot doing two things at once is not idle twice)
 * and numbers every segment by the plan it belongs to, because action ids
 * restart with every `plan_created` and must never be joined across one.
 */
import {Event, Lane} from '@/api/types';
import {verbOf} from './runAttribution';

export type VerbClass = 'walk' | 'mine' | 'craft' | 'place' | 'feed' | 'research' | 'other';

export function verbClass(action: string): VerbClass {
    switch (verbOf(action)) {
        case 'walk': return 'walk';
        case 'mine': case 'chop': return 'mine';
        case 'craft': return 'craft';
        case 'place': return 'place';
        case 'insert': case 'stock': case 'take': case 'fuel': case 'charge': return 'feed';
        case 'research': return 'research';
        default: return 'other';
    }
}

export interface LaneSegment {
    bot: number;
    /** 0 before the first `plan_created`, then 1, 2, … — never join across it. */
    planIndex: number;
    id: number | null;
    action: string;
    verb: VerbClass;
    from: number;
    to: number | null;
    status: string | null;
    error: string | null;
    /** Settled in its dispatch tick; drawn as a fixed-width tick, not a span. */
    instant: boolean;
}

/** Every `plan_created` tick, ascending. */
export function replanBoundaries(events: Event[]): number[] {
    return events.filter((e) => e.kind === 'plan_created').map((e) => e.tick).sort((a, b) => a - b);
}

export function laneSegments(lanes: Lane[], boundaries: number[]): LaneSegment[] {
    return lanes.map((l) => ({
        bot: l.bot,
        planIndex: boundaries.filter((b) => b <= l.from_tick).length,
        id: l.id,
        action: l.action,
        verb: verbClass(l.action),
        from: l.from_tick,
        to: l.to_tick,
        status: l.status,
        error: l.error,
        instant: l.to_tick !== null && l.to_tick === l.from_tick
    }));
}

export interface Interval {
    from: number;
    to: number;
}

/** The axis minus the union of this bot's lanes. An unterminated lane covers to the end. */
export function idleIntervals(lanes: Lane[], bot: number, scale: Interval): Interval[] {
    const busy = lanes
        .filter((l) => l.bot === bot)
        .map((l) => ({from: Math.max(scale.from, l.from_tick), to: Math.min(scale.to, l.to_tick ?? scale.to)}))
        .filter((i) => i.to > i.from)
        .sort((a, b) => a.from - b.from);
    const gaps: Interval[] = [];
    let cursor = scale.from;
    for (const b of busy) {
        if (b.from > cursor) gaps.push({from: cursor, to: b.from});
        cursor = Math.max(cursor, b.to);
    }
    if (cursor < scale.to) gaps.push({from: cursor, to: scale.to});
    return gaps;
}

export function idleTicks(intervals: Interval[]): number {
    return intervals.reduce((n, i) => n + (i.to - i.from), 0);
}
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/runIdle.spec.ts` — Expected: PASS (8 tests). If the fixture idle figure is off by a few ticks, compare against `idle_gaps` in `tools/run_analysis.py:1832` (it merges intervals the same way and bounds them by the window); do not loosen the assertion.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `runIdle`, idle intervals per bot and segments numbered by plan

Idle is the axis minus the union of a bot's lanes, overlap-aware, and equals
the analysis tool's figure for bot 1 on the fixture run (6,984 ticks). Every
segment carries its plan index so nothing is joined across a `plan_created`.
EOF
git add -- app/src/lib/runIdle.ts app/src/lib/runIdle.spec.ts
git commit -F /tmp/msg -- app/src/lib/runIdle.ts app/src/lib/runIdle.spec.ts
```

---

### Task 6: `machineTimeline.ts` — rows, status matrix, status class

**Files:**
- Create: `app/src/lib/machineTimeline.ts`
- Test: `app/src/lib/machineTimeline.spec.ts`

**Interfaces:**
- Consumes: `Sample`, `MachineSample`, `Position` from `@/api/types`; `loadFixtureRun`.
- Produces:
  - `type StatusClass = 'good' | 'warn' | 'serious' | 'critical' | 'neutral'`; `statusClass(status: string | null): StatusClass` (`working`→good; `no_ingredients`, `item_ingredient_shortage`, `no_minable_resources`, `missing_science_packs`→warn; `no_fuel`→serious; `no_power`, `low_power`→critical; else neutral).
  - `interface MachineRow {key: string; name: string; type: string; position: Position; firstTick: number; isContainer: boolean}`; `machineRows(samples): MachineRow[]` grouped by kind rank (drill 0, furnace 1, assembler 2, lab 3, other producer 4, container 5) then `firstTick`, then numeric key.
  - `interface StatusCell {tick: number; status: string | null; produced: number | null; fill: number | null}`; `statusMatrix(samples, rows): Map<string, StatusCell[]>` — one cell per `machines` sample the key appears in; `fill` is the summed `output` count for containers, else null.
  - `sampleStepTicks(samples): number | null` — the modal gap between consecutive `machines` samples (300 on every archived run), null with fewer than two.
  - `machineStatusAt(samples, tick): Map<string, string | null>` keyed by `"x,y"` position — the map panel's fill lookup.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/machineTimeline.spec.ts
import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';
import {machineRows, machineStatusAt, sampleStepTicks, statusClass, statusMatrix} from './machineTimeline';

describe('statusClass', () => {
    it('maps game statuses onto the reserved status set', () => {
        expect(statusClass('working')).toBe('good');
        expect(statusClass('no_ingredients')).toBe('warn');
        expect(statusClass('no_fuel')).toBe('serious');
        expect(statusClass('no_power')).toBe('critical');
        expect(statusClass('low_power')).toBe('critical');
        expect(statusClass('normal')).toBe('neutral');
        expect(statusClass(null)).toBe('neutral');
    });
});

describe('on the fixture run', () => {
    const run = loadFixtureRun();
    const rows = machineRows(run.samples);
    it('lists every sampled machine once, drills first, containers last, in placement order', () => {
        expect(rows).toHaveLength(17);
        expect(rows[0].name).toBe('burner-mining-drill');
        expect(rows[rows.length - 1].isContainer).toBe(true);
        const furnaces = rows.filter((r) => r.name === 'stone-furnace');
        expect(furnaces.map((r) => r.firstTick)).toEqual([...furnaces.map((r) => r.firstTick)].sort((a, b) => a - b));
    });
    it('gives one cell per sample a machine appears in, carrying its status and counter', () => {
        const m = statusMatrix(run.samples, rows);
        const cells = m.get('13') ?? [];
        expect(cells.length).toBeGreaterThan(50);
        expect(cells[cells.length - 1]).toMatchObject({tick: 25200, status: 'no_ingredients'});
        expect(cells.some((c) => c.status === 'working')).toBe(true);
    });
    it('reads the sample beat off the data', () => {
        expect(sampleStepTicks(run.samples)).toBe(300);
    });
    it('answers status by position at a tick, from the latest sample at or before it', () => {
        const at = machineStatusAt(run.samples, 20399);
        expect(at.get('-10,-21')).toBe('no_ingredients');
        expect(machineStatusAt(run.samples, 100).size).toBe(0);
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/machineTimeline.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/machineTimeline.ts
/**
 * Per-machine status over time, the band the record has always carried and
 * never drawn. It turned "the far end starves of ore" into "the far end
 * starves of everything" in one afternoon (CLAUDE.md, entity.status note).
 */
import {MachineSample, Position, Sample} from '@/api/types';

export type StatusClass = 'good' | 'warn' | 'serious' | 'critical' | 'neutral';

const WARN = ['no_ingredients', 'item_ingredient_shortage', 'no_minable_resources', 'missing_science_packs'];

export function statusClass(status: string | null): StatusClass {
    if (status === 'working') return 'good';
    if (status !== null && WARN.includes(status)) return 'warn';
    if (status === 'no_fuel') return 'serious';
    if (status === 'no_power' || status === 'low_power') return 'critical';
    return 'neutral';
}

export interface MachineRow {
    key: string;
    name: string;
    type: string;
    position: Position;
    firstTick: number;
    isContainer: boolean;
}

type MachinesSample = Extract<Sample, {kind: 'machines'}>;

function machinesSamples(samples: Sample[]): MachinesSample[] {
    return samples.filter((s): s is MachinesSample => s.kind === 'machines').sort((a, b) => a.tick - b.tick);
}

const CONTAINER_TYPES = ['container', 'logistic-container', 'infinity-container'];

function kindRank(m: MachineSample): number {
    if (m.type === 'mining-drill') return 0;
    if (m.type === 'furnace') return 1;
    if (m.type === 'assembling-machine') return 2;
    if (m.type === 'lab') return 3;
    if (CONTAINER_TYPES.includes(m.type)) return 5;
    return 4;
}

/** Every machine the run sampled, grouped by kind and ordered by first appearance. */
export function machineRows(samples: Sample[]): MachineRow[] {
    const first = new Map<string, {m: MachineSample; tick: number}>();
    for (const s of machinesSamples(samples)) {
        for (const [key, m] of Object.entries(s.machines)) {
            if (!first.has(key)) first.set(key, {m, tick: s.tick});
        }
    }
    return [...first.entries()]
        .map(([key, {m, tick}]) => ({
            key, name: m.name, type: m.type, position: m.position, firstTick: tick,
            isContainer: CONTAINER_TYPES.includes(m.type), rank: kindRank(m)
        }))
        .sort((a, b) => a.rank - b.rank || a.firstTick - b.firstTick || Number(a.key) - Number(b.key))
        .map(({rank: _rank, ...row}) => row);
}

export interface StatusCell {
    tick: number;
    status: string | null;
    produced: number | null;
    /** Items in a container's output inventory; null for a non-container. */
    fill: number | null;
}

export function statusMatrix(samples: Sample[], rows: MachineRow[]): Map<string, StatusCell[]> {
    const out = new Map<string, StatusCell[]>(rows.map((r) => [r.key, []]));
    for (const s of machinesSamples(samples)) {
        for (const row of rows) {
            const m = s.machines[row.key];
            if (!m) continue;
            const fill = row.isContainer ? Object.values(m.output).reduce((n, c) => n + c, 0) : null;
            out.get(row.key)!.push({tick: s.tick, status: m.status, produced: m.produced, fill});
        }
    }
    return out;
}

/** The most common gap between consecutive `machines` samples; null below two samples. */
export function sampleStepTicks(samples: Sample[]): number | null {
    const ticks = machinesSamples(samples).map((s) => s.tick);
    if (ticks.length < 2) return null;
    const counts = new Map<number, number>();
    for (let i = 1; i < ticks.length; i++) {
        const d = ticks[i] - ticks[i - 1];
        counts.set(d, (counts.get(d) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0])[0][0];
}

export function positionKey(p: Position): string {
    return `${p.x},${p.y}`;
}

/** Status by position from the latest `machines` sample at or before `tick`. */
export function machineStatusAt(samples: Sample[], tick: number): Map<string, string | null> {
    let latest: MachinesSample | null = null;
    for (const s of machinesSamples(samples)) {
        if (s.tick <= tick) latest = s;
        else break;
    }
    const out = new Map<string, string | null>();
    if (latest === null) return out;
    for (const m of Object.values(latest.machines)) out.set(positionKey(m.position), m.status);
    return out;
}
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/machineTimeline.spec.ts` — Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `machineTimeline`, every sampled machine's status over time
EOF
git add -- app/src/lib/machineTimeline.ts app/src/lib/machineTimeline.spec.ts
git commit -F /tmp/msg -- app/src/lib/machineTimeline.ts app/src/lib/machineTimeline.spec.ts
```

---

### Task 7: `runCoverage.ts` — where the record has data

**Files:**
- Create: `app/src/lib/runCoverage.ts`
- Test: `app/src/lib/runCoverage.spec.ts`

**Interfaces:**
- Consumes: `Event`, `Sample` from `@/api/types`.
- Produces: `interface StreamExtent {label: string; from: number; to: number; count: number}`; `coverageOf(events, samples): StreamExtent[]` — one row each for `events`, `bot samples`, `force + machine samples`, omitting streams with no data (an omitted row is drawn as "no record" by the band, by name); `lagTicks(runEnd: number, samples): number | null` — `runEnd - lastSampleTick`, null with no samples.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/runCoverage.spec.ts
import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';
import {coverageOf, lagTicks} from './runCoverage';

describe('coverageOf', () => {
    const run = loadFixtureRun();
    it('reports one extent per stream that has data', () => {
        const rows = coverageOf(run.events, run.samples);
        expect(rows.map((r) => r.label)).toEqual(['events', 'force + machine samples']); // fixture has no bot samples
        expect(rows[0]).toMatchObject({from: 3242, to: 25224});
        expect(rows[1]).toMatchObject({from: 3300, to: 25200, count: 148});
    });
    it('omits a stream with nothing rather than reporting a zero extent', () => {
        expect(coverageOf([], [])).toEqual([]);
    });
    it('lag is run end minus the last sample, null with no samples', () => {
        expect(lagTicks(25224, run.samples)).toBe(24);
        expect(lagTicks(25224, [])).toBeNull();
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/runCoverage.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/runCoverage.ts
/**
 * Where the record has data. Nine of 24 archived runs end on a `plan_created`
 * with nothing after it, and one healthy run was killed because its record
 * went quiet: a blank stretch in any band means "no record", and this is the
 * band that says so by name.
 */
import {Event, Sample} from '@/api/types';

export interface StreamExtent {
    label: string;
    from: number;
    to: number;
    count: number;
}

function extent(label: string, ticks: number[]): StreamExtent | null {
    if (ticks.length === 0) return null;
    return {label, from: Math.min(...ticks), to: Math.max(...ticks), count: ticks.length};
}

export function coverageOf(events: Event[], samples: Sample[]): StreamExtent[] {
    const rows = [
        extent('events', events.map((e) => e.tick)),
        extent('bot samples', samples.filter((s) => s.kind === 'bots').map((s) => s.tick)),
        extent('force + machine samples', samples.filter((s) => s.kind === 'force' || s.kind === 'machines').map((s) => s.tick))
    ];
    return rows.filter((r): r is StreamExtent => r !== null);
}

/** `runEnd - lastSampleTick`; null when nothing was sampled (not zero). */
export function lagTicks(runEnd: number, samples: Sample[]): number | null {
    if (samples.length === 0) return null;
    return runEnd - Math.max(...samples.map((s) => s.tick));
}
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/lib/runCoverage.spec.ts` — Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `runCoverage`, the extents of each record stream
EOF
git add -- app/src/lib/runCoverage.ts app/src/lib/runCoverage.spec.ts
git commit -F /tmp/msg -- app/src/lib/runCoverage.ts app/src/lib/runCoverage.spec.ts
```

---
### Task 8: Theme tokens, dark mode, and the theme toggle

**Files:**
- Modify: `app/src/assets/tailwind.css`
- Create: `app/src/composables/useTheme.ts`, `app/src/composables/useTheme.spec.ts`
- Create: `app/src/components/ThemeToggle.vue`
- Modify: `app/src/AppTopbar.vue`

**Interfaces:**
- Produces CSS custom properties usable as `var(--color-…)` in SVG and as Tailwind utilities (`bg-status-good`, `fill-verb-walk`, `text-ink-muted`…): `--color-status-{good,warn,serious,critical,neutral}`, `--color-verb-{walk,mine,craft,place,feed,research,other}`, `--color-bot-{1..8}`, `--color-item-{iron,copper,circuit,gear,red,green}`, `--color-verdict-roster`, `--color-plot`, `--color-plot-grid`, `--color-warn-dark`, plus dark counterparts of every existing token.
- Produces `useTheme(): {theme: Ref<'light'|'dark'|'system'>; set(theme): void; cycle(): void}`; `applyTheme(theme, root = document.documentElement)` stamps `data-theme` (or removes it for `system`).

- [ ] **Step 1: Write the failing composable test**

```ts
// app/src/composables/useTheme.spec.ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it} from 'vitest';
import {applyTheme, THEME_KEY, useTheme} from './useTheme';

describe('useTheme', () => {
    beforeEach(() => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
    });
    it('starts on system and stamps nothing', () => {
        const t = useTheme();
        expect(t.theme.value).toBe('system');
        expect(document.documentElement.dataset.theme).toBeUndefined();
    });
    it('stamps data-theme and remembers the choice per viewer', () => {
        const t = useTheme();
        t.set('dark');
        expect(document.documentElement.dataset.theme).toBe('dark');
        expect(localStorage.getItem(THEME_KEY)).toBe('dark');
        t.set('system');
        expect(document.documentElement.dataset.theme).toBeUndefined();
        expect(localStorage.getItem(THEME_KEY)).toBeNull();
    });
    it('cycles system -> dark -> light -> system', () => {
        const t = useTheme();
        t.cycle(); expect(t.theme.value).toBe('dark');
        t.cycle(); expect(t.theme.value).toBe('light');
        t.cycle(); expect(t.theme.value).toBe('system');
    });
    it('survives storage that throws', () => {
        const root = document.createElement('div');
        expect(() => applyTheme('light', root)).not.toThrow();
        expect(root.dataset.theme).toBe('light');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/composables/useTheme.spec.ts` — Expected: FAIL, module not found.

- [ ] **Step 3: Implement the composable**

```ts
// app/src/composables/useTheme.ts
/**
 * Light, dark, or follow the OS. Three states, not two: the un-stamped
 * document is the default and only `prefers-color-scheme` separates light
 * from dark there; an explicit choice stamps `data-theme` so it wins over the
 * OS in both directions. Stored per viewer; storage may be unavailable, so
 * every read and write is guarded.
 */
import {ref, Ref} from 'vue';

export type Theme = 'light' | 'dark' | 'system';
export const THEME_KEY = 'factorio-bot.theme';

function readStored(): Theme {
    try {
        const v = localStorage.getItem(THEME_KEY);
        return v === 'light' || v === 'dark' ? v : 'system';
    } catch {
        return 'system';
    }
}

export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
    if (theme === 'system') delete root.dataset.theme;
    else root.dataset.theme = theme;
}

const theme: Ref<Theme> = ref('system');
let initialised = false;

export function useTheme() {
    if (!initialised) {
        initialised = true;
        theme.value = readStored();
        if (typeof document !== 'undefined') applyTheme(theme.value);
    }
    function set(next: Theme) {
        theme.value = next;
        applyTheme(next);
        try {
            if (next === 'system') localStorage.removeItem(THEME_KEY);
            else localStorage.setItem(THEME_KEY, next);
        } catch {
            // storage unavailable: the choice still applies for this page
        }
    }
    function cycle() {
        set(theme.value === 'system' ? 'dark' : theme.value === 'dark' ? 'light' : 'system');
    }
    return {theme, set, cycle};
}

/** Test seam: forget the module-level state between specs. */
export function resetThemeForTests() {
    initialised = false;
    theme.value = 'system';
}
```

Add `resetThemeForTests()` to the spec's `beforeEach` (import it) so each test starts from `system`.

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/composables/useTheme.spec.ts` — Expected: PASS (4 tests).

- [ ] **Step 5: Add the tokens to `tailwind.css`**

Inside the existing `@theme { … }` block, after `--color-warn: #f9c851;`, add:

```css
    --color-warn-dark: #b8860b;

    /* Chart surfaces */
    --color-plot: #fcfcfb;
    --color-plot-grid: #e3e6ea;

    /* Reserved status set: machine status and lane failures. Always drawn with
     * a text label; never reused as a series colour. */
    --color-status-good: #1d8a4b;
    --color-status-warn: #c98500;
    --color-status-serious: #c85a1e;
    --color-status-critical: #c8322f;
    --color-status-neutral: #9aa1a9;

    /* Attribution verdict accent (roster-fed hatch); factory uses status-good. */
    --color-verdict-roster: #b8642a;
    --color-verdict-roster-soft: #f1dccb;

    /* Verb classes for bot lanes: the dataviz reference categorical order,
     * fixed by class, never cycled. Validated adjacent-pair CVD ΔE >= 8. */
    --color-verb-walk: #2a78d6;
    --color-verb-mine: #eb6834;
    --color-verb-place: #1baf7a;
    --color-verb-feed: #eda100;
    --color-verb-craft: #e87ba4;
    --color-verb-research: #008300;
    --color-verb-other: #4a3aa7;

    /* Bots: fixed by id, the same hue in every band and on the map. */
    --color-bot-1: #2a78d6;
    --color-bot-2: #eb6834;
    --color-bot-3: #1baf7a;
    --color-bot-4: #e87ba4;
    --color-bot-5: #eda100;
    --color-bot-6: #008300;
    --color-bot-7: #4a3aa7;
    --color-bot-8: #e34948;

    /* Tracked items: validated 5-slot palette, both modes. */
    --color-item-iron: #3b78c4;
    --color-item-copper: #d5722b;
    --color-item-circuit: #1f9e78;
    --color-item-gear: #8a6fd6;
    --color-item-red: #d94a63;
    --color-item-green: #1baf7a;
```

After the `@theme` block (before `@layer base`), add the dark overrides. `@theme` writes its variables onto `:root`, so a later `:root` rule with the same names wins:

```css
/* Dark theme. Three viewer states: an explicit `data-theme` stamp wins both
 * ways; the un-stamped document follows the OS. Only tokens change here --
 * every component is written against tokens, never against a literal. */
@layer base {
    @media (prefers-color-scheme: dark) {
        :root:not([data-theme="light"]) {
            color-scheme: dark;
            --color-surface: #14171b;
            --color-card: #1c2026;
            --color-divider: #343a42;
            --color-ink: #e7e9ec;
            --color-ink-muted: #b4bac2;
            --color-link: #62b0f6;
            --color-focus: #4a8fd6;
            --color-warn-dark: #d99a1a;
            --color-plot: #1a1e23;
            --color-plot-grid: #2c323a;
            --color-status-good: #3fae6c;
            --color-status-warn: #d99a1a;
            --color-status-serious: #df7a3e;
            --color-status-critical: #e0524f;
            --color-status-neutral: #6f7780;
            --color-verdict-roster: #d98a4c;
            --color-verdict-roster-soft: #3d2a1c;
            --color-verb-walk: #3987e5;
            --color-verb-mine: #d95926;
            --color-verb-place: #199e70;
            --color-verb-feed: #c98500;
            --color-verb-craft: #d55181;
            --color-verb-other: #9085e9;
            --color-bot-1: #3987e5;
            --color-bot-2: #d95926;
            --color-bot-3: #199e70;
            --color-bot-4: #d55181;
            --color-bot-5: #c98500;
            --color-bot-7: #9085e9;
            --color-bot-8: #e66767;
        }
    }
    :root[data-theme="dark"] {
        color-scheme: dark;
        --color-surface: #14171b;
        --color-card: #1c2026;
        --color-divider: #343a42;
        --color-ink: #e7e9ec;
        --color-ink-muted: #b4bac2;
        --color-link: #62b0f6;
        --color-focus: #4a8fd6;
        --color-warn-dark: #d99a1a;
        --color-plot: #1a1e23;
        --color-plot-grid: #2c323a;
        --color-status-good: #3fae6c;
        --color-status-warn: #d99a1a;
        --color-status-serious: #df7a3e;
        --color-status-critical: #e0524f;
        --color-status-neutral: #6f7780;
        --color-verdict-roster: #d98a4c;
        --color-verdict-roster-soft: #3d2a1c;
        --color-verb-walk: #3987e5;
        --color-verb-mine: #d95926;
        --color-verb-place: #199e70;
        --color-verb-feed: #c98500;
        --color-verb-craft: #d55181;
        --color-verb-other: #9085e9;
        --color-bot-1: #3987e5;
        --color-bot-2: #d95926;
        --color-bot-3: #199e70;
        --color-bot-4: #d55181;
        --color-bot-5: #c98500;
        --color-bot-7: #9085e9;
        --color-bot-8: #e66767;
    }
}
```

The sidebar tokens are already dark and stay as they are in both themes. `--color-brand*` stays: the topbar gradient reads fine on both grounds.

- [ ] **Step 6: Write `ThemeToggle.vue` and mount it**

```vue
<!-- app/src/components/ThemeToggle.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Monitor, Moon, Sun} from '@lucide/vue';
import {useTheme} from '@/composables/useTheme';

const {theme, cycle} = useTheme();
const label = computed(() =>
    theme.value === 'system' ? 'Theme: follow system' : theme.value === 'dark' ? 'Theme: dark' : 'Theme: light'
);
</script>

<template>
  <button
    type="button"
    class="cursor-pointer text-white transition-colors hover:text-focus focus-visible:ring-2 focus-visible:ring-focus rounded"
    :aria-label="label"
    :title="label"
    data-testid="theme-toggle"
    @click="cycle()">
    <Monitor v-if="theme === 'system'" class="size-5"/>
    <Moon v-else-if="theme === 'dark'" class="size-5"/>
    <Sun v-else class="size-5"/>
  </button>
</template>
```

In `app/src/AppTopbar.vue`, import it (`import ThemeToggle from '@/components/ThemeToggle.vue';`) and change the right-hand group so the toggle is always present:

```vue
    <div class="ml-auto flex items-center gap-3">
      <span v-if="settings" class="hidden sm:inline">Factorio with <strong>{{ settings.factorio.client_count }} Clients</strong></span>
      <ProcessControl v-if="settings"/>
      <ThemeToggle/>
    </div>
```

- [ ] **Step 7: Verify the whole app still passes**

Run: `cd app && pnpm lint && pnpm run test:coverage`
Expected: lint clean; all suites pass; coverage gate holds. Then `pnpm run build:web` builds without warnings about unknown utilities.

Open the dev server (`just start` in another terminal), click the toggle: the page ground, cards and ink flip; the map stays a dark island in both.

- [ ] **Step 8: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): dark theme tokens, a per-viewer theme toggle, and the reserved chart token sets

Adds dark counterparts for every shell token behind `prefers-color-scheme`
and an explicit `data-theme` stamp that wins both ways, plus the token sets
the run page draws with: a reserved status set, verb classes in the validated
categorical order, bot hues fixed by id, and the item palette. Also defines
`--color-warn-dark`, which `ReplayScrubber.vue` had been using without it
existing, so its two video warnings finally render in amber.
EOF
git add -- app/src/assets/tailwind.css app/src/composables/useTheme.ts app/src/composables/useTheme.spec.ts app/src/components/ThemeToggle.vue app/src/AppTopbar.vue
git commit -F /tmp/msg -- app/src/assets/tailwind.css app/src/composables/useTheme.ts app/src/composables/useTheme.spec.ts app/src/components/ThemeToggle.vue app/src/AppTopbar.vue
```

---

### Task 9: `TickAxis.vue` and `CursorBar.vue`

**Files:**
- Create: `app/src/components/run/TickAxis.vue`, `app/src/components/run/TickAxis.spec.ts`
- Create: `app/src/components/run/CursorBar.vue`, `app/src/components/run/CursorBar.spec.ts`
- Create: `app/src/components/run/BandFrame.vue` (the label-gutter + plot grid every band uses)

**Interfaces:**
- Consumes: `tickX`, `formatGameTime`, `markTicks`, `AXIS_WIDTH`, `TickScale` (Task 2).
- Produces:
  - `BandFrame` props `{title: string; subtitle?: string}`, slot `default` for the plot. Renders `<div class="grid grid-cols-[10.5rem_1fr] border-b border-divider">` with the label cell and a `<div class="relative bg-plot">` plot cell.
  - `TickAxis` props `{scale: TickScale; cursor: number}`. Emits nothing. Renders an SVG `viewBox="0 0 1000 30"`, with `data-testid="tick-axis"`, minute ticks, 5-minute marks with `class="mark"`, and a cursor `<line data-testid="cursor">`.
  - `CursorBar` props `{scale: TickScale; cursor: number; playing: boolean; rate: number}`; emits `seek(tick: number)`, `toggle()`, `rate(rate: number)`. Renders the range input, play/pause button, tick and game-time readout, step select.
  - Every band draws its own cursor line via the shared helper `cursorLine(scale, cursor)` → `x` (exported from `TickAxis.vue`? No — keep it in `tickScale.ts` as `tickX`; the bands use it directly).

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/TickAxis.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import TickAxis from './TickAxis.vue';

const scale = {from: 3242, to: 25224};

describe('TickAxis', () => {
    it('draws a minute tick per game minute and emphasises the 5-minute mark', () => {
        const w = mount(TickAxis, {props: {scale, cursor: 3242}});
        const ticks = w.findAll('[data-testid="tick-axis"] line.minute');
        expect(ticks).toHaveLength(6); // 1:00 .. 6:00
        const marks = w.findAll('[data-testid="tick-axis"] line.mark');
        expect(marks).toHaveLength(1);
        expect(w.text()).toContain('5:00');
    });
    it('places the cursor at tickX', () => {
        const w = mount(TickAxis, {props: {scale, cursor: 3242 + 10991}});
        const x = Number(w.get('[data-testid="cursor"]').attributes('x1'));
        expect(x).toBeCloseTo(500, 0);
    });
});
```

```ts
// app/src/components/run/CursorBar.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import CursorBar from './CursorBar.vue';

const scale = {from: 3242, to: 25224};

describe('CursorBar', () => {
    it('shows the tick and the game time, and seeks on input', async () => {
        const w = mount(CursorBar, {props: {scale, cursor: 21242, playing: false, rate: 300}});
        expect(w.text()).toContain('tick 21,242');
        expect(w.text()).toContain('5:00');
        await w.get('input[type="range"]').setValue('4000');
        expect(w.emitted('seek')?.[0]).toEqual([4000]);
    });
    it('toggles play and changes the step', async () => {
        const w = mount(CursorBar, {props: {scale, cursor: 3242, playing: true, rate: 300}});
        expect(w.get('button').text()).toBe('Pause');
        await w.get('button').trigger('click');
        expect(w.emitted('toggle')).toHaveLength(1);
        await w.get('select').setValue('1800');
        expect(w.emitted('rate')?.[0]).toEqual([1800]);
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run` — Expected: FAIL, modules not found.

- [ ] **Step 3: Implement the three components**

```vue
<!-- app/src/components/run/BandFrame.vue -->
<script setup lang="ts">
defineProps<{title: string; subtitle?: string}>();
</script>

<template>
  <div class="grid grid-cols-[10.5rem_1fr] border-b border-divider last:border-b-0">
    <div class="border-r border-divider bg-surface px-3 py-2">
      <div class="text-[11px] font-semibold uppercase tracking-wider text-ink-muted">{{ title }}</div>
      <div v-if="subtitle" class="mt-0.5 text-xs leading-snug text-ink-muted/80">{{ subtitle }}</div>
    </div>
    <div class="relative min-w-0 bg-plot">
      <slot/>
    </div>
  </div>
</template>
```

```vue
<!-- app/src/components/run/TickAxis.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {AXIS_WIDTH, formatGameTime, markTicks, TickScale, tickX, TICKS_PER_MINUTE} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number}>();

const minutes = computed(() => {
    const out: number[] = [];
    for (let t = props.scale.from + TICKS_PER_MINUTE; t <= props.scale.to; t += TICKS_PER_MINUTE) out.push(t);
    return out;
});
const marks = computed(() => markTicks(props.scale));
const x = (t: number) => tickX(props.scale, t);
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} 30`" class="block h-auto w-full" data-testid="tick-axis" aria-label="game-time axis">
    <line x1="0" y1="22" :x2="AXIS_WIDTH" y2="22" stroke="var(--color-divider)" stroke-width="1"/>
    <line v-for="t in minutes" :key="`m${t}`" class="minute" :x1="x(t)" y1="15" :x2="x(t)" y2="22"
          stroke="var(--color-ink-muted)" stroke-width="1"/>
    <template v-for="t in marks" :key="`k${t}`">
      <line class="mark" :x1="x(t)" y1="8" :x2="x(t)" y2="22" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
      <text :x="x(t) + 4" y="12" font-size="10" fill="var(--color-verdict-roster)" font-family="ui-monospace, monospace">
        {{ formatGameTime(scale, t) }} mark
      </text>
    </template>
    <text :x="AXIS_WIDTH - 2" y="12" font-size="10" text-anchor="end" fill="var(--color-ink-muted)" font-family="ui-monospace, monospace">
      tick {{ scale.to.toLocaleString() }}
    </text>
    <line data-testid="cursor" :x1="x(cursor)" y1="0" :x2="x(cursor)" y2="30" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

```vue
<!-- app/src/components/run/CursorBar.vue -->
<script setup lang="ts">
import {formatGameTime, TickScale} from '@/lib/tickScale';

defineProps<{scale: TickScale; cursor: number; playing: boolean; rate: number}>();
const emit = defineEmits<{seek: [tick: number]; toggle: []; rate: [rate: number]}>();
</script>

<template>
  <div class="flex items-center gap-4 border-t border-divider bg-surface px-4 py-2">
    <button type="button" class="rounded border border-divider bg-card px-3 py-1 text-sm hover:bg-surface focus-visible:ring-2 focus-visible:ring-focus"
            @click="emit('toggle')">
      {{ playing ? 'Pause' : 'Play' }}
    </button>
    <span class="min-w-[13rem] font-mono text-sm tabular-nums text-ink-muted">
      cursor <b class="font-medium text-verdict-roster">tick {{ cursor.toLocaleString() }}</b>
      · {{ formatGameTime(scale, cursor) }}
    </span>
    <input type="range" class="flex-1 accent-verdict-roster" :min="scale.from" :max="scale.to" :value="cursor"
           aria-label="tick cursor"
           @input="emit('seek', Number(($event.target as HTMLInputElement).value))"/>
    <label class="text-sm text-ink-muted">
      step
      <select class="ml-1 rounded border border-divider bg-card px-1 py-0.5 text-sm" :value="rate"
              @change="emit('rate', Number(($event.target as HTMLSelectElement).value))">
        <option :value="60">1s</option>
        <option :value="300">5s</option>
        <option :value="1800">30s</option>
      </select>
    </label>
  </div>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run` — Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): the run page's axis, band frame and cursor bar
EOF
git add -- app/src/components/run/BandFrame.vue app/src/components/run/TickAxis.vue app/src/components/run/TickAxis.spec.ts app/src/components/run/CursorBar.vue app/src/components/run/CursorBar.spec.ts
git commit -F /tmp/msg -- app/src/components/run/BandFrame.vue app/src/components/run/TickAxis.vue app/src/components/run/TickAxis.spec.ts app/src/components/run/CursorBar.vue app/src/components/run/CursorBar.spec.ts
```

---

### Task 10: `ProductionBand.vue` — rates with the verdict painted in

**Files:**
- Create: `app/src/components/run/ProductionBand.vue`, `app/src/components/run/ProductionBand.spec.ts`

**Interfaces:**
- Consumes: `rateSeries`, `markAt`, `DEFAULT_MARKS`, `RatePoint`, `Mark` (Task 3); `attributionIntervals`, `Verdict` (Task 4); `tickX`, `markTicks`, `formatGameTime`, `AXIS_WIDTH` (Task 2); `Sample`, `Event`.
- Props: `{scale: TickScale; cursor: number; samples: Sample[]; events: Event[]; items: string[]; lo: number; hi: number}`. `items` is the list to draw (the page passes `rateItems(...)`); each item is a small multiple of height 50, so the SVG height is `items.length * 50`.
- Renders per item: verdict rects with `data-verdict="<verdict>"` and `class="verdict"`, an area path `data-testid="area-<item>"`, a line path, a peak marker with text `peak N/min`, mark rules with the value text, and the verdict word at the bottom-left of each interval on the first row. The item colour comes from `itemColor(item)`: `iron-plate→var(--color-item-iron)`, `copper-plate→copper`, `electronic-circuit→circuit`, `iron-gear-wheel→gear`, `automation-science-pack→red`, `logistic-science-pack→green`, anything else `var(--color-ink-muted)`.
- Produces `itemColor(item: string): string` (exported from `app/src/lib/itemColor.ts`, created here, with a 2-case spec).

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/ProductionBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import ProductionBand from './ProductionBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

function mountBand(items = ['iron-plate', 'copper-plate']) {
    return mount(ProductionBand, {props: {scale, cursor: run.lo, samples: run.samples, events: run.events, items, lo: run.lo, hi: run.hi}});
}

describe('ProductionBand', () => {
    it('draws one small multiple per item with an area and a line', () => {
        const w = mountBand();
        expect(w.find('[data-testid="area-iron-plate"]').exists()).toBe(true);
        expect(w.find('[data-testid="area-copper-plate"]').exists()).toBe(true);
        expect(w.get('svg').attributes('viewBox')).toBe('0 0 1000 100');
    });
    it('paints the verdict per minute behind the curve, and every rect carries its verdict', () => {
        const w = mountBand(['iron-plate']);
        const rects = w.findAll('rect.verdict');
        expect(rects.length).toBeGreaterThan(0);
        // On this run every producing minute for iron is roster-fed.
        const verdicts = new Set(rects.map((r) => r.attributes('data-verdict')));
        expect(verdicts.has('roster-fed')).toBe(true);
        expect(verdicts.has('factory')).toBe(false);
        expect(w.text()).toContain('roster-fed');
    });
    it('labels the 5:00 mark with the tool\'s rate for that item', () => {
        const w = mountBand(['iron-plate']);
        // rate_window at 5:00 for iron-plate is 8.0 /min in rates.json
        expect(w.text()).toContain('8/min at 5:00');
    });
    it('says "no output" for an item nothing made, not 0/min everywhere', () => {
        const w = mountBand(['logistic-science-pack']);
        expect(w.findAll('rect.verdict')).toHaveLength(0);
        expect(w.text()).toContain('no output in this run');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/ProductionBand.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement `itemColor.ts` and the band**

```ts
// app/src/lib/itemColor.ts
/** A tracked item's series colour. Fixed by item, never by rank in the list. */
const ITEM_TOKENS: Record<string, string> = {
    'iron-plate': 'iron',
    'copper-plate': 'copper',
    'electronic-circuit': 'circuit',
    'iron-gear-wheel': 'gear',
    'automation-science-pack': 'red',
    'logistic-science-pack': 'green'
};

export function itemColor(item: string): string {
    const token = ITEM_TOKENS[item];
    return token === undefined ? 'var(--color-ink-muted)' : `var(--color-item-${token})`;
}
```

```ts
// app/src/lib/itemColor.spec.ts
import {describe, expect, it} from 'vitest';
import {itemColor} from './itemColor';

describe('itemColor', () => {
    it('is fixed by item', () => {
        expect(itemColor('iron-plate')).toBe('var(--color-item-iron)');
        expect(itemColor('automation-science-pack')).toBe('var(--color-item-red)');
    });
    it('falls back to muted ink for an unlisted item', () => {
        expect(itemColor('stone-brick')).toBe('var(--color-ink-muted)');
    });
});
```

```vue
<!-- app/src/components/run/ProductionBand.vue -->
<script setup lang="ts">
/**
 * Items per minute with the attribution verdict painted BEHIND the curve.
 *
 * A rising curve is not evidence of a working factory: a hand-loaded stone
 * furnace is a machine. So each minute interval's background is its verdict
 * (roster-fed hatch, factory solid, hand-made dotted, unclear grey) and the
 * word is written in the band, never left to a legend.
 */
import {computed} from 'vue';
import {Event, Sample} from '@/api/types';
import {attributionIntervals, Verdict} from '@/lib/runAttribution';
import {markAt, rateSeries} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, markTicks, TickScale, tickX} from '@/lib/tickScale';
import {itemColor} from '@/lib/itemColor';

const props = defineProps<{
    scale: TickScale; cursor: number; samples: Sample[]; events: Event[]; items: string[]; lo: number; hi: number;
}>();

const ROW = 50;
const height = computed(() => Math.max(ROW, props.items.length * ROW));
const x = (t: number) => tickX(props.scale, t);

interface Row {
    item: string;
    color: string;
    points: {tick: number; perMinute: number}[];
    max: number;
    peak: {tick: number; perMinute: number} | null;
    verdicts: {from: number; to: number; verdict: Verdict}[];
    marks: {tick: number; label: string}[];
    empty: boolean;
}

const rows = computed<Row[]>(() => props.items.map((item, i) => {
    const points = rateSeries(props.samples, item, props.lo, props.hi);
    const max = Math.max(1, ...points.map((p) => p.perMinute)) * 1.15;
    const peak = points.reduce<Row['peak']>((m, p) => (m === null || p.perMinute > m.perMinute ? p : m), null);
    const verdicts = attributionIntervals(props.samples, props.events, props.lo, props.hi, item)
        .filter((v) => v.verdict !== 'no output')
        .map(({from, to, verdict}) => ({from, to, verdict}));
    const marks = markTicks(props.scale).map((tick) => {
        const minute = (tick - props.lo) / 3600;
        const m = markAt(props.samples, props.lo, props.hi, minute, Math.max(0, minute - 5), item);
        const label = m.status === 'ok' && m.rateWindow !== null
            ? `${m.rateWindow.toFixed(0)}/min at ${formatGameTime(props.scale, tick)}`
            : `${m.status.replace('_', ' ')} at ${formatGameTime(props.scale, tick)}`;
        return {tick, label};
    });
    return {item, color: itemColor(item), points, max, peak, verdicts, marks, empty: verdicts.length === 0 && (peak?.perMinute ?? 0) === 0, row: i};
}));

function y0(i: number) { return (i + 1) * ROW - 6; }
function yFor(row: Row, i: number, v: number) { return y0(i) - (v / row.max) * (ROW - 16); }

function areaPath(row: Row, i: number): string {
    if (row.points.length === 0) return '';
    let d = `M${x(row.points[0].tick)},${y0(i)}`;
    for (const p of row.points) d += ` L${x(p.tick)},${yFor(row, i, p.perMinute)}`;
    return `${d} L${x(row.points[row.points.length - 1].tick)},${y0(i)} Z`;
}
function linePath(row: Row, i: number): string {
    return row.points.map((p, k) => `${k ? 'L' : 'M'}${x(p.tick)},${yFor(row, i, p.perMinute)}`).join(' ');
}
function verdictFill(v: Verdict): string {
    if (v === 'roster-fed') return 'url(#verdict-roster)';
    if (v === 'factory') return 'var(--color-status-good)';
    if (v === 'hand-made') return 'url(#verdict-hand)';
    return 'var(--color-status-neutral)';
}
function verdictOpacity(v: Verdict): number { return v === 'factory' ? 0.18 : v === 'roster-fed' || v === 'hand-made' ? 1 : 0.25; }
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="production rates with attribution">
    <defs>
      <pattern id="verdict-roster" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(135)">
        <rect width="6" height="6" fill="var(--color-verdict-roster-soft)"/>
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--color-verdict-roster)" stroke-width="1.2" opacity="0.55"/>
      </pattern>
      <pattern id="verdict-hand" width="5" height="5" patternUnits="userSpaceOnUse">
        <circle cx="2.5" cy="2.5" r="0.9" fill="var(--color-ink-muted)" opacity="0.7"/>
      </pattern>
    </defs>
    <template v-for="(row, i) in rows" :key="row.item">
      <rect v-for="v in row.verdicts" :key="`${row.item}-${v.from}`" class="verdict" :data-verdict="v.verdict"
            :x="x(v.from)" :y="i * ROW" :width="x(v.to) - x(v.from)" :height="ROW"
            :fill="verdictFill(v.verdict)" :opacity="verdictOpacity(v.verdict)"/>
      <line :x1="0" :y1="y0(i)" :x2="AXIS_WIDTH" :y2="y0(i)" stroke="var(--color-plot-grid)" stroke-width="1"/>
      <path :data-testid="`area-${row.item}`" :d="areaPath(row, i)" :fill="row.color" opacity="0.18"/>
      <path :d="linePath(row, i)" fill="none" :stroke="row.color" stroke-width="1.8" stroke-linejoin="round"/>
      <text :x="6" :y="i * ROW + 12" font-size="11" font-weight="500" fill="var(--color-ink)">{{ row.item }}</text>
      <template v-if="row.empty">
        <text :x="AXIS_WIDTH / 2" :y="i * ROW + ROW / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no output in this run</text>
      </template>
      <template v-else>
        <template v-if="row.peak">
          <circle :cx="x(row.peak.tick)" :cy="yFor(row, i, row.peak.perMinute)" r="3" :fill="row.color" stroke="var(--color-plot)" stroke-width="1.5"/>
          <text :x="x(row.peak.tick) + 6" :y="yFor(row, i, row.peak.perMinute) + 3" font-size="10" fill="var(--color-ink-muted)">peak {{ row.peak.perMinute.toFixed(0) }}/min</text>
        </template>
        <template v-for="m in row.marks" :key="`${row.item}-${m.tick}`">
          <line :x1="x(m.tick)" :y1="i * ROW + 4" :x2="x(m.tick)" :y2="y0(i)" stroke="var(--color-verdict-roster)" stroke-width="1" stroke-dasharray="2 3"/>
          <text :x="x(m.tick) + 4" :y="i * ROW + 24" font-size="10" font-weight="500" fill="var(--color-verdict-roster)">{{ m.label }}</text>
        </template>
        <text v-for="v in row.verdicts" :key="`w-${row.item}-${v.from}`" :x="x(v.from) + 4" :y="(i + 1) * ROW - 2" font-size="9"
              :fill="v.verdict === 'roster-fed' ? 'var(--color-verdict-roster)' : 'var(--color-ink-muted)'">{{ v.verdict }}</text>
      </template>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/ProductionBand.spec.ts src/lib/itemColor.spec.ts` — Expected: PASS (6 tests). If the `8/min at 5:00` assertion fails, check `markAt`'s `prevMinute` argument: the label uses the trailing-window rate, which does not depend on it, so a mismatch means `rateWindow` disagrees with the golden and Task 3 is where to look.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `ProductionBand`, rates per item with the verdict painted behind the curve
EOF
git add -- app/src/components/run/ProductionBand.vue app/src/components/run/ProductionBand.spec.ts app/src/lib/itemColor.ts app/src/lib/itemColor.spec.ts
git commit -F /tmp/msg -- app/src/components/run/ProductionBand.vue app/src/components/run/ProductionBand.spec.ts app/src/lib/itemColor.ts app/src/lib/itemColor.spec.ts
```

---

### Task 11: `PowerBand.vue` and `ResearchBand.vue`

**Files:**
- Create: `app/src/components/run/PowerBand.vue`, `PowerBand.spec.ts`, `ResearchBand.vue`, `ResearchBand.spec.ts`

**Interfaces:**
- Consumes: `ForceSample` (Task 3), `tickX`, `formatGameTime`, `AXIS_WIDTH`.
- `PowerBand` props `{scale; cursor; samples: Sample[]}`; height 70; two lines (generated solid `var(--color-verb-research)`, consumed dashed `var(--color-ink-muted)`) on ONE kW scale (max = `max(1000, ceil(maxGenerated/500)*500)`); a dashed rule and text `no generator until m:ss` at the first sample with `generated_kw > 0`; a critical stripe (`rect.deficit`, `fill="var(--color-status-critical)"`, opacity .25) over any sample interval where any network has `demanded_kw > generated_kw`; text `no power samples` when there are no force samples.
- `ResearchBand` props `{scale; cursor; samples: Sample[]}`; height 44; one area per contiguous run of the same `research.name` (progress 0..1), labelled `<name> · started m:ss · NN% at m:ss`; text `no research queued in this run` when every sample's research is null.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/PowerBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import PowerBand from './PowerBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

describe('PowerBand', () => {
    it('names the first generation tick', () => {
        const w = mount(PowerBand, {props: {scale, cursor: run.lo, samples: run.samples}});
        // first generated_kw > 0 sample is at tick 18000 -> 4:06 from 3242
        expect(w.text()).toContain('no generator until 4:06');
        expect(w.text()).toContain('900 kW');
    });
    it('draws no deficit stripe when demand is met, and one when it is not', () => {
        const ok = mount(PowerBand, {props: {scale, cursor: run.lo, samples: run.samples}});
        expect(ok.findAll('rect.deficit')).toHaveLength(0);
        const starved = run.samples.map((s) => s.kind !== 'force' ? s : ({
            ...s, power: {...s.power, networks: {0: {sub_ids: [0], generated_kw: 0, consumed_kw: 0, demanded_kw: 60, satisfaction: 0}}}
        }));
        const bad = mount(PowerBand, {props: {scale, cursor: run.lo, samples: starved}});
        expect(bad.findAll('rect.deficit').length).toBeGreaterThan(0);
    });
    it('says so when there are no force samples', () => {
        const w = mount(PowerBand, {props: {scale, cursor: run.lo, samples: []}});
        expect(w.text()).toContain('no power samples');
    });
});
```

```ts
// app/src/components/run/ResearchBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import ResearchBand from './ResearchBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

describe('ResearchBand', () => {
    it('draws one fill per technology and labels it', () => {
        const w = mount(ResearchBand, {props: {scale, cursor: run.lo, samples: run.samples}});
        expect(w.findAll('path.tech')).toHaveLength(1);
        expect(w.text()).toContain('automation');
        expect(w.text()).toMatch(/\d+% at 6:0\d/);
    });
    it('says so when nothing was ever queued', () => {
        const none = run.samples.map((s) => (s.kind === 'force' ? {...s, research: null} : s));
        const w = mount(ResearchBand, {props: {scale, cursor: run.lo, samples: none}});
        expect(w.text()).toContain('no research queued in this run');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/PowerBand.spec.ts src/components/run/ResearchBand.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

```vue
<!-- app/src/components/run/PowerBand.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {ForceSample} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]}>();
const H = 70;
const x = (t: number) => tickX(props.scale, t);

const force = computed(() => props.samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick));
const max = computed(() => Math.max(1000, Math.ceil(Math.max(0, ...force.value.map((f) => f.power.generated_kw)) / 500) * 500));
const y = (kw: number) => H - 8 - (kw / max.value) * (H - 22);
const gen = computed(() => force.value.map((f, k) => `${k ? 'L' : 'M'}${x(f.tick)},${y(f.power.generated_kw)}`).join(' '));
const cons = computed(() => force.value.map((f, k) => `${k ? 'L' : 'M'}${x(f.tick)},${y(f.power.consumed_kw)}`).join(' '));
const firstGen = computed(() => force.value.find((f) => f.power.generated_kw > 0) ?? null);
const deficits = computed(() => force.value.flatMap((f, k) => {
    const short = Object.values(f.power.networks).some((n) => n.demanded_kw > n.generated_kw);
    if (!short) return [];
    const prev = k > 0 ? force.value[k - 1].tick : f.tick - 300;
    return [{from: prev, to: f.tick}];
}));
const gridKw = computed(() => [0, max.value / 2, max.value]);
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="power generated and consumed">
    <template v-if="force.length === 0">
      <text :x="AXIS_WIDTH / 2" :y="H / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no power samples</text>
    </template>
    <template v-else>
      <template v-for="kw in gridKw" :key="kw">
        <line x1="0" :y1="y(kw)" :x2="AXIS_WIDTH" :y2="y(kw)" stroke="var(--color-plot-grid)" stroke-width="1"/>
        <text x="4" :y="y(kw) - 2" font-size="9" fill="var(--color-ink-muted)">{{ kw }} kW</text>
      </template>
      <rect v-for="d in deficits" :key="d.to" class="deficit" :x="x(d.from)" y="0" :width="Math.max(1, x(d.to) - x(d.from))" :height="H"
            fill="var(--color-status-critical)" opacity="0.25"/>
      <path :d="gen" fill="none" stroke="var(--color-verb-research)" stroke-width="1.8"/>
      <path :d="cons" fill="none" stroke="var(--color-ink-muted)" stroke-width="1.6" stroke-dasharray="4 3"/>
      <template v-if="firstGen">
        <line :x1="x(firstGen.tick)" y1="8" :x2="x(firstGen.tick)" :y2="H - 8" stroke="var(--color-verb-research)" stroke-width="1" stroke-dasharray="2 3"/>
        <text :x="x(firstGen.tick) - 4" y="16" font-size="10" text-anchor="end" fill="var(--color-ink-muted)">no generator until {{ formatGameTime(scale, firstGen.tick) }}</text>
        <text :x="x(firstGen.tick) + 5" :y="y(firstGen.power.generated_kw) + 12" font-size="10" font-weight="500" fill="var(--color-verb-research)">generated {{ firstGen.power.generated_kw.toFixed(0) }} kW</text>
      </template>
      <text v-else :x="AXIS_WIDTH - 4" y="16" font-size="10" text-anchor="end" fill="var(--color-ink-muted)">no generation in this run</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

```vue
<!-- app/src/components/run/ResearchBand.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {ForceSample} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]}>();
const H = 44;
const x = (t: number) => tickX(props.scale, t);
const y = (p: number) => H - 8 - p * (H - 18);

interface Tech { name: string; points: {tick: number; progress: number}[] }

const techs = computed<Tech[]>(() => {
    const out: Tech[] = [];
    for (const s of props.samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick)) {
        if (s.research === null) continue;
        const last = out[out.length - 1];
        if (last && last.name === s.research.name) last.points.push({tick: s.tick, progress: s.research.progress});
        else out.push({name: s.research.name, points: [{tick: s.tick, progress: s.research.progress}]});
    }
    return out;
});
function area(t: Tech): string {
    const p = t.points;
    return `M${x(p[0].tick)},${y(0)} ${p.map((q) => `L${x(q.tick)},${y(q.progress)}`).join(' ')} L${x(p[p.length - 1].tick)},${y(0)} Z`;
}
function label(t: Tech): string {
    const first = t.points[0], last = t.points[t.points.length - 1];
    return `${t.name} · started ${formatGameTime(props.scale, first.tick)} · ${Math.round(last.progress * 100)}% at ${formatGameTime(props.scale, last.tick)}`;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="research progress">
    <line x1="0" :y1="y(0)" :x2="AXIS_WIDTH" :y2="y(0)" stroke="var(--color-plot-grid)"/>
    <text v-if="techs.length === 0" :x="AXIS_WIDTH / 2" :y="H / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no research queued in this run</text>
    <template v-for="t in techs" :key="`${t.name}-${t.points[0].tick}`">
      <path class="tech" :d="area(t)" fill="var(--color-item-red)" opacity="0.25"/>
      <text :x="x(t.points[0].tick) + 4" y="12" font-size="10" fill="var(--color-ink-muted)">{{ label(t) }}</text>
    </template>
    <text x="6" :y="H - 12" font-size="9" fill="var(--color-ink-muted)">trigger technologies (steam-power, electronics) are not research durations and are not drawn as bars</text>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/PowerBand.spec.ts src/components/run/ResearchBand.spec.ts` — Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `PowerBand` and `ResearchBand` on the shared tick axis
EOF
git add -- app/src/components/run/PowerBand.vue app/src/components/run/PowerBand.spec.ts app/src/components/run/ResearchBand.vue app/src/components/run/ResearchBand.spec.ts
git commit -F /tmp/msg -- app/src/components/run/PowerBand.vue app/src/components/run/PowerBand.spec.ts app/src/components/run/ResearchBand.vue app/src/components/run/ResearchBand.spec.ts
```

---

### Task 12: `LaneBand.vue` — idle drawn, replans dashed

**Files:**
- Create: `app/src/components/run/LaneBand.vue`, `LaneBand.spec.ts`

**Interfaces:**
- Consumes: `laneSegments`, `idleIntervals`, `idleTicks`, `replanBoundaries`, `LaneSegment` (Task 5); `tickX`, `AXIS_WIDTH`, `formatGameTime`; `Lane`, `Event`.
- Props: `{scale; cursor; lanes: Lane[]; events: Event[]}`; row height 30 per bot (`laneBots` order); SVG height `bots * 30`.
- Renders per bot: a full-width `rect.idle` filled with the hatch pattern `url(#lane-idle)`; the label `bot N` in `var(--color-bot-N)` (N clamped to 8); one `rect.segment` per segment with `data-verb`, `data-plan`, fill `var(--color-verb-<verb>)`, width `max(1.6, …)` for instants, `stroke="var(--color-status-critical)"` when status is `failed` or `lost`, unterminated segments extending to `scale.to` with opacity .5; a `<title>` per segment: `"<action> · plan <planIndex> · <ticks> ticks · <status|never settled>"` plus `" · evidence: believed (ticks measured, arrival not)"` for walks; a dashed `line.replan` per boundary with text `plan N · ids restart here`; text `idle NN%` per bot at the row's right edge.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/LaneBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import LaneBand from './LaneBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

describe('LaneBand', () => {
    const w = mount(LaneBand, {props: {scale, cursor: run.lo, lanes: run.lanes, events: run.events}});
    it('draws one hatched idle row per bot with the bot\'s idle share', () => {
        expect(w.findAll('rect.idle')).toHaveLength(4);
        expect(w.text()).toContain('bot 1');
        expect(w.text()).toContain('idle 32%'); // 6984 / 21982
    });
    it('draws every lane as a segment carrying its verb and plan', () => {
        const segs = w.findAll('rect.segment');
        expect(segs).toHaveLength(228);
        expect(new Set(segs.map((s) => s.attributes('data-verb')))).toEqual(new Set(['walk', 'mine', 'craft', 'place', 'feed', 'research']));
        expect(segs.every((s) => s.attributes('data-plan') === '1')).toBe(true);
    });
    it('marks walks as believed in their title and draws the replan boundary', () => {
        const walk = w.findAll('rect.segment').find((s) => s.attributes('data-verb') === 'walk')!;
        expect(walk.find('title').text()).toContain('evidence: believed');
        expect(w.findAll('line.replan')).toHaveLength(1);
        expect(w.text()).toContain('plan 1 · ids restart here');
    });
    it('outlines a failed segment and extends an unterminated one to the axis end', () => {
        const lanes = [
            {bot: 1, id: 1, action: 'mine 1 coal', from_tick: 4000, to_tick: 5000, status: 'failed', error: 'x'},
            {bot: 1, id: 2, action: 'craft 1 pipe', from_tick: 6000, to_tick: null, status: null, error: null}
        ];
        const v = mount(LaneBand, {props: {scale, cursor: run.lo, lanes, events: []}});
        const [failed, open] = v.findAll('rect.segment');
        expect(failed.attributes('stroke')).toBe('var(--color-status-critical)');
        expect(Number(open.attributes('x')) + Number(open.attributes('width'))).toBeCloseTo(1000, 0);
        expect(open.find('title').text()).toContain('never settled');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/LaneBand.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

```vue
<!-- app/src/components/run/LaneBand.vue -->
<script setup lang="ts">
/**
 * One row per bot. The row's background IS the idle state; only dispatched
 * work paints over it. Zero-length feeding acts are fixed-width ticks (they
 * would vanish at any scale), replan boundaries are dashed across every row,
 * and a walk's title says its success is believed, not measured.
 */
import {computed} from 'vue';
import {Event, Lane} from '@/api/types';
import {idleIntervals, idleTicks, laneSegments, LaneSegment, replanBoundaries} from '@/lib/runIdle';
import {laneBots} from '@/lib/runTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; lanes: Lane[]; events: Event[]}>();
const ROW = 30;
const x = (t: number) => tickX(props.scale, t);
const bots = computed(() => laneBots(props.lanes));
const height = computed(() => Math.max(ROW, bots.value.length * ROW));
const boundaries = computed(() => replanBoundaries(props.events));
const segments = computed(() => laneSegments(props.lanes, boundaries.value));
const idlePct = computed(() => Object.fromEntries(bots.value.map((b) => {
    const span = props.scale.to - props.scale.from;
    return [b, span > 0 ? Math.round((idleTicks(idleIntervals(props.lanes, b, props.scale)) / span) * 100) : 0];
})));
const botColor = (b: number) => `var(--color-bot-${Math.min(8, Math.max(1, b))})`;
const rowY = (b: number) => bots.value.indexOf(b) * ROW;

function width(s: LaneSegment): number {
    if (s.instant) return 1.6;
    const end = s.to === null ? props.scale.to : s.to;
    return Math.max(1.6, x(end) - x(s.from));
}
function title(s: LaneSegment): string {
    const ticks = s.to === null ? 'never settled' : `${(s.to - s.from).toLocaleString()} ticks`;
    const status = s.to === null ? '' : ` · ${s.status ?? 'no status'}`;
    const evidence = s.verb === 'walk' ? ' · evidence: believed (ticks measured, arrival not)' : '';
    return `${s.action} · plan ${s.planIndex} · ${ticks}${status}${evidence}`;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="bot lanes">
    <defs>
      <pattern id="lane-idle" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--color-ink-muted)" stroke-width="0.8" opacity="0.5"/>
      </pattern>
    </defs>
    <template v-for="b in bots" :key="b">
      <rect class="idle" x="0" :y="rowY(b) + 5" :width="AXIS_WIDTH" :height="ROW - 10" fill="url(#lane-idle)"/>
      <text x="6" :y="rowY(b) + ROW / 2 + 4" font-size="11" font-weight="500" :fill="botColor(b)">bot {{ b }}</text>
      <text :x="AXIS_WIDTH - 4" :y="rowY(b) + ROW / 2 + 4" font-size="9" text-anchor="end" fill="var(--color-ink-muted)">idle {{ idlePct[b] }}%</text>
    </template>
    <rect v-for="(s, i) in segments" :key="`${s.bot}-${s.planIndex}-${s.id}-${s.from}-${i}`" class="segment"
          :data-verb="s.verb" :data-plan="s.planIndex"
          :x="x(s.from)" :y="rowY(s.bot) + 7" :width="width(s)" :height="ROW - 14" rx="1"
          :fill="`var(--color-verb-${s.verb})`" :opacity="s.to === null ? 0.5 : 1"
          :stroke="s.status === 'failed' || s.status === 'lost' ? 'var(--color-status-critical)' : 'none'" stroke-width="1.5">
      <title>{{ title(s) }}</title>
    </rect>
    <template v-for="(t, i) in boundaries" :key="`r${t}`">
      <line class="replan" :x1="x(t)" y1="0" :x2="x(t)" :y2="height" stroke="var(--color-ink)" stroke-width="1" stroke-dasharray="3 3"/>
      <text :x="x(t) + 4" :y="height - 2" font-size="9" fill="var(--color-ink-muted)">plan {{ i + 1 }} · ids restart here · {{ formatGameTime(scale, t) }}</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/LaneBand.spec.ts` — Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `LaneBand`, bot lanes with idle hatched and replan boundaries dashed
EOF
git add -- app/src/components/run/LaneBand.vue app/src/components/run/LaneBand.spec.ts
git commit -F /tmp/msg -- app/src/components/run/LaneBand.vue app/src/components/run/LaneBand.spec.ts
```

---
### Task 13: `MachineBand.vue` — the status heatmap

**Files:**
- Create: `app/src/components/run/MachineBand.vue`, `MachineBand.spec.ts`

**Interfaces:**
- Consumes: `machineRows`, `statusMatrix`, `sampleStepTicks`, `statusClass`, `MachineRow` (Task 6); `tickX`, `AXIS_WIDTH`, `formatGameTime`.
- Props: `{scale; cursor; samples: Sample[]; selected?: string | null}`; emits `select(key: string)`.
- Row height 12; SVG height `max(24, rows * 12)`. Each row: label text `<short name> #<key>` (`burner-mining-drill→drill`, `stone-furnace→furnace`, `wooden-chest→chest`, else the name) and one `rect.cell` per sample the machine appears in, `x = tickX(tick) - cellWidth`, `width = max(0.5, cellWidth - 1)` where `cellWidth = tickX(from + step) - tickX(from)`, fill `var(--color-status-<class>)`, `data-status`, and a `<title>`: `"<name> #<key> · <m:ss> · <status>[ · produced N][ · N items]"`. A container's cells use the neutral fill with `opacity = 0.3 + 0.7 * min(1, fill / 100)`. The selected row gets a `rect.selected` outline. Clicking any cell or label emits `select(key)`. With no `machines` samples the band shows the text `no machine samples in this run`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/MachineBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import MachineBand from './MachineBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

describe('MachineBand', () => {
    const w = mount(MachineBand, {props: {scale, cursor: run.lo, samples: run.samples, selected: null}});
    it('has one row per sampled machine and cells coloured by status', () => {
        expect(w.findAll('text.row-label')).toHaveLength(17);
        const statuses = new Set(w.findAll('rect.cell').map((c) => c.attributes('data-status')));
        expect(statuses.has('working')).toBe(true);
        expect(statuses.has('no_ingredients')).toBe(true);
        expect(statuses.has('no_fuel')).toBe(true);
        const working = w.findAll('rect.cell').find((c) => c.attributes('data-status') === 'working')!;
        expect(working.attributes('fill')).toBe('var(--color-status-good)');
    });
    it('titles every cell with the machine, time and status', () => {
        const cell = w.findAll('rect.cell').find((c) => c.attributes('data-status') === 'no_fuel')!;
        expect(cell.find('title').text()).toMatch(/furnace #\d+ · \d+:\d\d · no_fuel/);
    });
    it('emits the row key when a cell is clicked', async () => {
        await w.findAll('rect.cell')[0].trigger('click');
        expect(w.emitted('select')?.[0]?.[0]).toMatch(/^\d+$/);
    });
    it('says so when there are no machine samples', () => {
        const v = mount(MachineBand, {props: {scale, cursor: run.lo, samples: run.samples.filter((s) => s.kind !== 'machines'), selected: null}});
        expect(v.text()).toContain('no machine samples in this run');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/MachineBand.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

```vue
<!-- app/src/components/run/MachineBand.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {machineRows, sampleStepTicks, statusClass, statusMatrix} from '@/lib/machineTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]; selected?: string | null}>();
const emit = defineEmits<{select: [key: string]}>();
const ROW = 12;
const x = (t: number) => tickX(props.scale, t);
const rows = computed(() => machineRows(props.samples));
const matrix = computed(() => statusMatrix(props.samples, rows.value));
const step = computed(() => sampleStepTicks(props.samples) ?? 300);
const cellWidth = computed(() => x(props.scale.from + step.value) - x(props.scale.from));
const height = computed(() => Math.max(24, rows.value.length * ROW));

function shortName(name: string): string {
    return name.replace('burner-mining-drill', 'drill').replace('stone-furnace', 'furnace').replace('wooden-chest', 'chest');
}
function title(row: {name: string; key: string}, c: {tick: number; status: string | null; produced: number | null; fill: number | null}): string {
    let t = `${shortName(row.name)} #${row.key} · ${formatGameTime(props.scale, c.tick)} · ${c.status ?? 'no status'}`;
    if (c.produced !== null) t += ` · produced ${c.produced}`;
    if (c.fill !== null) t += ` · ${c.fill} items`;
    return t;
}
function fillOpacity(isContainer: boolean, fill: number | null): number {
    return isContainer ? 0.3 + 0.7 * Math.min(1, (fill ?? 0) / 100) : 1;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="machine status over time">
    <text v-if="rows.length === 0" :x="AXIS_WIDTH / 2" :y="height / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no machine samples in this run</text>
    <template v-for="(row, i) in rows" :key="row.key">
      <rect v-if="selected === row.key" class="selected" x="0" :y="i * ROW" :width="AXIS_WIDTH" :height="ROW" fill="none" stroke="var(--color-verdict-roster)" stroke-width="1"/>
      <rect v-for="c in matrix.get(row.key) ?? []" :key="c.tick" class="cell" :data-status="c.status ?? ''"
            :x="x(c.tick) - cellWidth + 0.5" :y="i * ROW + 0.5" :width="Math.max(0.5, cellWidth - 1)" :height="ROW - 1"
            :fill="`var(--color-status-${statusClass(c.status)})`" :opacity="fillOpacity(row.isContainer, c.fill)"
            style="cursor: pointer" @click="emit('select', row.key)">
        <title>{{ title(row, c) }}</title>
      </rect>
      <text class="row-label" x="6" :y="i * ROW + ROW - 3" font-size="8.5" fill="var(--color-ink)" style="cursor: pointer; paint-order: stroke" stroke="var(--color-plot)" stroke-width="2"
            @click="emit('select', row.key)">{{ shortName(row.name) }} #{{ row.key }}</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/MachineBand.spec.ts` — Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `MachineBand`, every sampled machine's status as a heatmap on the tick axis
EOF
git add -- app/src/components/run/MachineBand.vue app/src/components/run/MachineBand.spec.ts
git commit -F /tmp/msg -- app/src/components/run/MachineBand.vue app/src/components/run/MachineBand.spec.ts
```

---

### Task 14: `CoverageBand.vue`, `MilestoneRibbon.vue`, `RunHeadline.vue`

**Files:**
- Create: `app/src/components/run/CoverageBand.vue`, `CoverageBand.spec.ts`, `MilestoneRibbon.vue`, `MilestoneRibbon.spec.ts`, `RunHeadline.vue`, `RunHeadline.spec.ts`
- Create: `app/src/lib/runHeadline.ts`, `runHeadline.spec.ts`

**Interfaces:**
- `CoverageBand` props `{scale; cursor; events: Event[]; samples: Sample[]; runEnd: number}`; rows from `coverageOf`, 11 px each, each a `rect.extent` with `data-stream`; text `samples lag N ticks` or `no samples` from `lagTicks`; the absent streams are listed as text `no record: <label>[, <label>]`.
- `MilestoneRibbon` props `{scale; splits: Split[]; cursor}`; one `div.seg` per split, absolutely positioned by `tickX/10`%, text `m<index> · <goal> · <outcome> at <m:ss> · <elapsed> ticks` (elapsed `—` when null); the current split (`splitAt`) gets class `is-current`.
- `runHeadline.ts`: `headline(input: {samples; events; splits; lo; hi; items: string[]}): string` — the analysis tool's sentence shape: `rates: <item-short> <rate>/min at <mark> (<verdict>[; no generator until m:ss]) …; <red packs …> | milestone N <goal> <outcome> at m:ss`, built from `markAt` at the first reached mark for the first two items, `attributeInterval` over `(lo, mark]`, the first generation tick, and the last split. Returns `no marks reached and no milestones closed` when nothing applies.
- `RunHeadline` props `{summary: RunSummary; provenance: null; lagTicks: number | null; headline: string; roster: number[]}`; renders the run id, the headline, and chips: `seed`, `roster`, `mode`, `speed`, `commit`, `profile`, `mods`, `samples cover`. **`provenance` is always `null` in Phase 1** (no route yet); every provenance chip renders as `not captured` with `data-chip="<name>"` and `data-state="absent"`. The `samples cover` chip renders `lag N ticks` / `not sampled`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/lib/runHeadline.spec.ts
import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';
import {headline} from './runHeadline';

describe('headline', () => {
    const run = loadFixtureRun();
    it('reads like just analyse on the fixture run', () => {
        const h = headline({samples: run.samples, events: run.events, lo: run.lo, hi: run.hi, items: ['iron-plate', 'copper-plate'],
            splits: [{index: 1, goal: 'research automation', started_tick: 3242, ended_tick: 25216, outcome: 'satisfied', elapsed_ticks: 21974}]});
        expect(h).toBe('rates: iron-plate 8/min at 5:00 (roster-fed; no generator until 4:06) · copper-plate 0/min at 5:00 (roster-fed) | milestone 1 research automation satisfied at 6:06');
    });
    it('says when nothing applies', () => {
        expect(headline({samples: [], events: [], lo: 0, hi: 10, items: [], splits: []})).toBe('no marks reached and no milestones closed');
    });
});
```

```ts
// app/src/components/run/CoverageBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import CoverageBand from './CoverageBand.vue';

const run = loadFixtureRun();
describe('CoverageBand', () => {
    it('draws one extent per stream and names the missing ones', () => {
        const w = mount(CoverageBand, {props: {scale: {from: run.lo, to: run.hi}, cursor: run.lo, events: run.events, samples: run.samples, runEnd: run.hi}});
        expect(w.findAll('rect.extent').map((r) => r.attributes('data-stream'))).toEqual(['events', 'force + machine samples']);
        expect(w.text()).toContain('no record: bot samples');
        expect(w.text()).toContain('samples lag 24 ticks');
    });
});
```

```ts
// app/src/components/run/MilestoneRibbon.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import MilestoneRibbon from './MilestoneRibbon.vue';

describe('MilestoneRibbon', () => {
    const splits = [
        {index: 1, goal: 'research automation', started_tick: 3242, ended_tick: 25216, outcome: 'satisfied', elapsed_ticks: 21974},
        {index: 2, goal: 'green', started_tick: 25216, ended_tick: null, outcome: 'unfinished', elapsed_ticks: null}
    ];
    it('positions one segment per split and marks the current one', () => {
        const w = mount(MilestoneRibbon, {props: {scale: {from: 3242, to: 30000}, splits, cursor: 10000}});
        const segs = w.findAll('.seg');
        expect(segs).toHaveLength(2);
        expect(segs[0].classes()).toContain('is-current');
        expect(segs[0].text()).toContain('m1 · research automation · satisfied at 6:06 · 21,974 ticks');
        expect(segs[1].text()).toContain('— ticks');
    });
});
```

```ts
// app/src/components/run/RunHeadline.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import RunHeadline from './RunHeadline.vue';

describe('RunHeadline', () => {
    it('renders the sentence and marks every provenance chip absent until the route exists', () => {
        const w = mount(RunHeadline, {props: {
            summary: {run_id: 'run-1', finished: true, started_unix: 1, finished_unix: 2, outcome: 'done', elapsed_ticks: 10, events: 1, splits: 1},
            provenance: null, lagTicks: 24, headline: 'rates: …', roster: [1, 2, 3, 4]
        }});
        expect(w.text()).toContain('rates: …');
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="seed"]').text()).toContain('not captured');
        expect(w.get('[data-chip="roster"]').text()).toContain('1 2 3 4');
        expect(w.get('[data-chip="samples"]').text()).toContain('lag 24 ticks');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/lib/runHeadline.spec.ts src/components/run/CoverageBand.spec.ts src/components/run/MilestoneRibbon.spec.ts src/components/run/RunHeadline.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

```ts
// app/src/lib/runHeadline.ts
/** The first sentence a reader sees: the analysis tool's headline shape. */
import {Event, Sample, Split} from '@/api/types';
import {attributeInterval} from './runAttribution';
import {ForceSample, markAt} from './runRates';
import {formatGameTime, markTicks} from './tickScale';

export function headline(input: {samples: Sample[]; events: Event[]; splits: Split[]; lo: number; hi: number; items: string[]}): string {
    const {samples, events, splits, lo, hi, items} = input;
    const scale = {from: lo, to: hi};
    const parts: string[] = [];
    const mark = markTicks(scale)[0];
    if (mark !== undefined && items.length > 0) {
        const firstGen = samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick).find((s) => s.power.generated_kw > 0) ?? null;
        const rates = items.slice(0, 2).map((item, i) => {
            const m = markAt(samples, lo, hi, 5, 0, item);
            const a = attributeInterval(samples, events, lo, mark, item);
            const rate = m.status === 'ok' && m.rateWindow !== null ? `${m.rateWindow.toFixed(0)}/min` : m.status.replace('_', ' ');
            const gen = i === 0 && firstGen !== null && firstGen.tick > lo ? `; no generator until ${formatGameTime(scale, firstGen.tick)}` : '';
            return `${item} ${rate} at ${formatGameTime(scale, mark)} (${a.verdict}${gen})`;
        });
        parts.push(`rates: ${rates.join(' · ')}`);
    }
    const last = [...splits].reverse().find((s) => s.ended_tick !== null);
    if (last) parts.push(`milestone ${last.index} ${last.goal} ${last.outcome} at ${formatGameTime(scale, last.ended_tick as number)}`);
    return parts.length === 0 ? 'no marks reached and no milestones closed' : parts.join(' | ');
}
```

```vue
<!-- app/src/components/run/CoverageBand.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Event, Sample} from '@/api/types';
import {coverageOf, lagTicks} from '@/lib/runCoverage';
import {AXIS_WIDTH, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; events: Event[]; samples: Sample[]; runEnd: number}>();
const ALL = ['events', 'bot samples', 'force + machine samples'];
const COLORS: Record<string, string> = {events: 'var(--color-ink-muted)', 'bot samples': 'var(--color-verb-walk)', 'force + machine samples': 'var(--color-verb-research)'};
const x = (t: number) => tickX(props.scale, t);
const rows = computed(() => coverageOf(props.events, props.samples));
const missing = computed(() => ALL.filter((l) => !rows.value.some((r) => r.label === l)));
const lag = computed(() => lagTicks(props.runEnd, props.samples));
const H = computed(() => Math.max(24, rows.value.length * 11 + 8));
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="record coverage">
    <template v-for="(r, i) in rows" :key="r.label">
      <rect class="extent" :data-stream="r.label" :x="x(r.from)" :y="4 + i * 11" :width="Math.max(1, x(r.to) - x(r.from))" height="8" rx="1" :fill="COLORS[r.label]" opacity="0.55"/>
      <text x="6" :y="4 + i * 11 + 7" font-size="8.5" fill="var(--color-ink)" style="paint-order: stroke" stroke="var(--color-plot)" stroke-width="2">{{ r.label }} · {{ r.count.toLocaleString() }}</text>
    </template>
    <text :x="AXIS_WIDTH - 4" y="11" font-size="9" text-anchor="end" fill="var(--color-ink-muted)">
      {{ lag === null ? 'no samples' : `samples lag ${lag.toLocaleString()} ticks` }}<template v-if="missing.length"> · no record: {{ missing.join(', ') }}</template>
    </text>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
```

```vue
<!-- app/src/components/run/MilestoneRibbon.vue -->
<script setup lang="ts">
import {Split} from '@/api/types';
import {splitAt} from '@/lib/runTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; splits: Split[]; cursor: number}>();
const pct = (t: number) => `${(tickX(props.scale, t) / AXIS_WIDTH) * 100}%`;
function style(s: Split) {
    const end = s.ended_tick ?? props.scale.to;
    return {left: pct(s.started_tick), width: `calc(${(tickX(props.scale, end) - tickX(props.scale, s.started_tick)) / AXIS_WIDTH * 100}% - 2px)`};
}
function label(s: Split) {
    const at = s.ended_tick === null ? 'unfinished' : `${s.outcome} at ${formatGameTime(props.scale, s.ended_tick)}`;
    const ticks = s.elapsed_ticks === null ? '—' : s.elapsed_ticks.toLocaleString();
    return `m${s.index} · ${s.goal} · ${at} · ${ticks} ticks`;
}
</script>

<template>
  <div class="relative h-9">
    <div v-for="s in splits" :key="`${s.index}-${s.started_tick}`"
         class="seg absolute top-2 h-[22px] overflow-hidden text-ellipsis whitespace-nowrap rounded border border-verdict-roster bg-verdict-roster-soft px-2 font-mono text-[11px] leading-5 text-ink"
         :class="{'is-current ring-2 ring-verdict-roster': splitAt(splits, cursor)?.started_tick === s.started_tick}"
         :style="style(s)" :title="label(s)">{{ label(s) }}</div>
  </div>
</template>
```

```vue
<!-- app/src/components/run/RunHeadline.vue -->
<script setup lang="ts">
/**
 * The first line, and the chips that say whether this run may be compared
 * with another. `provenance` is null until `/runs/{id}/provenance` exists
 * (Phase 2); every chip it would fill renders as "not captured" IN PLACE, so
 * absence is visible rather than silent.
 */
import {RunSummary} from '@/api/types';

defineProps<{summary: RunSummary; provenance: null; lagTicks: number | null; headline: string; roster: number[]}>();
const PROVENANCE_CHIPS = ['seed', 'mode', 'speed', 'commit', 'profile', 'mods'];
</script>

<template>
  <header class="flex flex-wrap items-baseline gap-x-6 gap-y-3 border-b border-divider px-5 pb-3 pt-4">
    <h2 class="text-xl font-semibold">{{ summary.run_id }}</h2>
    <div class="flex flex-wrap">
      <span data-chip="roster" data-state="present" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        roster <b class="font-medium text-ink">{{ roster.join(' ') }}</b>
      </span>
      <span v-for="c in PROVENANCE_CHIPS" :key="c" :data-chip="c" data-state="absent"
            class="mb-1.5 mr-1.5 rounded border border-dashed border-divider px-2 py-1 font-mono text-xs text-ink-muted">
        {{ c }} <i>not captured</i>
      </span>
      <span data-chip="samples" :data-state="lagTicks === null ? 'absent' : 'present'" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        samples <b class="font-medium text-ink">{{ lagTicks === null ? 'not sampled' : `lag ${lagTicks.toLocaleString()} ticks` }}</b>
      </span>
    </div>
    <p class="basis-full text-sm text-ink-muted"><b class="font-medium text-ink">{{ headline }}</b></p>
  </header>
</template>
```

- [ ] **Step 4: Run to verify pass** — same command as Step 2 — Expected: PASS (6 tests). If the headline string differs, print `h` and compare word by word against `run_analysis.py`'s `rates_headline`; the copper rate is `rate_window` at 5:00 (0.0 in `rates.json`).

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): headline, milestone ribbon and coverage band for the run page
EOF
git add -- app/src/lib/runHeadline.ts app/src/lib/runHeadline.spec.ts app/src/components/run/CoverageBand.vue app/src/components/run/CoverageBand.spec.ts app/src/components/run/MilestoneRibbon.vue app/src/components/run/MilestoneRibbon.spec.ts app/src/components/run/RunHeadline.vue app/src/components/run/RunHeadline.spec.ts
git commit -F /tmp/msg -- app/src/lib/runHeadline.ts app/src/lib/runHeadline.spec.ts app/src/components/run/CoverageBand.vue app/src/components/run/CoverageBand.spec.ts app/src/components/run/MilestoneRibbon.vue app/src/components/run/MilestoneRibbon.spec.ts app/src/components/run/RunHeadline.vue app/src/components/run/RunHeadline.spec.ts
```

---

### Task 15: Store — fetch events, expose the run window and machine fills

**Files:**
- Modify: `app/src/store/runsStore.ts`
- Modify: `app/src/store/runsStore.spec.ts`

**Interfaces:**
- Consumes: `getRunEvents` from `@/api/client`; `machineStatusAt` (Task 6).
- Produces on the store: state `events: Event[]`, `eventsError: string | null`, `selectedMachine: string | null`; getters `window(): {lo: number; hi: number} | null` (the `run_started` tick to the `run_finished` tick or the last event's tick; `null` with no events — **this is the analysis window, distinct from `bounds`, which is the drawn axis**), `machineFills(): Map<string, string | null>` (status by `"x,y"` at the cursor), `laneBotIds(): number[]`; action `selectMachine(key: string | null)`.

- [ ] **Step 1: Write the failing store tests (append to `runsStore.spec.ts`)**

```ts
describe('events enrichment', () => {
    beforeEach(() => {
        setActivePinia(createPinia());
        vi.mocked(client.getRun).mockResolvedValue({summary: summary('run-1'), splits: []} as RunDetail);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: []});
        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [], skipped: 0});
        vi.mocked(client.getRunMap).mockResolvedValue({map: [], skipped: 0});
        vi.mocked(client.getRunVideo).mockResolvedValue(NO_VIDEO_MANIFEST);
        vi.mocked(client.getRunVideoTicks).mockResolvedValue(NO_TICKS);
    });
    it('derives the analysis window from run_started and run_finished', async () => {
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [
            {kind: 'run_started', tick: 3242, run_id: 'run-1', bots: [1], seed: null, factorio: null, git: null},
            {kind: 'run_finished', tick: 25224, outcome: 'done', elapsed_ticks: 21982}
        ] as Event[], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.window).toEqual({lo: 3242, hi: 25224});
        expect(store.eventsError).toBeNull();
    });
    it('falls back to the last event when the run never finished, and to null with no events', async () => {
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [
            {kind: 'run_started', tick: 10, run_id: 'run-1', bots: [1], seed: null, factorio: null, git: null},
            {kind: 'plan_created', tick: 50, milestone_index: 1, steps: 0, makespan: 0, bots: null, plan: []}
        ] as Event[], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.window).toEqual({lo: 10, hi: 50});
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 0});
        await store.openRun('run-1');
        expect(store.window).toBeNull();
    });
    it('records an events failure without failing the run', async () => {
        vi.mocked(client.getRunEvents).mockRejectedValue(new ApiError(404, 'not_found', ''));
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.detail).not.toBeNull();
        expect(store.eventsError).toContain('/events');
    });
});
```

Add `Event` to the `@/api/types` import at the top of the spec. Check `ApiError`'s constructor signature in `app/src/api/http.ts` and match it (the existing spec already constructs one — copy that call).

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/store/runsStore.spec.ts` — Expected: FAIL on `store.window` / `eventsError` undefined.

- [ ] **Step 3: Implement**

In `runsStore.ts`:
- Import `getRunEvents` from `@/api/client`, `Event` from `@/api/types`, `machineStatusAt` from `@/lib/machineTimeline`, `laneBots` from `@/lib/runTimeline`.
- State: add `events: [] as Event[]`, `eventsError: null as string | null`, `selectedMachine: null as string | null`.
- Getters:

```ts
        /**
         * The ANALYSIS window: `run_started` to `run_finished` (or the last
         * event). Distinct from `bounds`, the drawn axis, which trims a
         * lead-in. Rates and verdicts are measured from `run_started`, as
         * `just analyse` measures them; `null` when the run has no events.
         */
        window(): {lo: number; hi: number} | null {
            if (this.events.length === 0) return null;
            const started = this.events.find((e) => e.kind === 'run_started');
            const finished = this.events.find((e) => e.kind === 'run_finished');
            const lo = started?.tick ?? Math.min(...this.events.map((e) => e.tick));
            const hi = finished?.tick ?? Math.max(...this.events.map((e) => e.tick));
            return {lo, hi};
        },
        /** Machine status by "x,y" at the cursor — the map's fill lookup. */
        machineFills(): Map<string, string | null> {
            return machineStatusAt(this.samples, this.cursor);
        },
        laneBotIds(): number[] {
            return laneBots(this.lanes);
        },
```

- `openRun`: reset `this.eventsError = null; this.selectedMachine = null;` at the top; add `getRunEvents(id)` as the sixth entry of the `Promise.allSettled` array and destructure `eventsResult`; after the map block:

```ts
                if (eventsResult.status === 'fulfilled') {
                    this.events = eventsResult.value.events;
                } else {
                    this.events = [];
                    this.eventsError = enrichmentUnavailable('events', '/events', eventsResult.reason);
                }
```

  and in the `catch`, add `this.events = [];`.
- Action: `selectMachine(key: string | null) { this.selectedMachine = key; }`.

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/store/runsStore.spec.ts` — Expected: PASS (existing tests plus 3). Existing `openRun` tests mock five client calls; if any now fails with `getRunEvents is not a function`, add `vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 0})` to their `beforeEach`.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): the runs store fetches `/events` and exposes the analysis window

`window` is `run_started` to `run_finished`, the interval `just analyse`
measures from, and is deliberately not `bounds`, the drawn axis with its
lead-in trimmed. A failed `/events` degrades one panel, not the run.
EOF
git add -- app/src/store/runsStore.ts app/src/store/runsStore.spec.ts
git commit -F /tmp/msg -- app/src/store/runsStore.ts app/src/store/runsStore.spec.ts
```

---

### Task 16: `RunSidePanel.vue` — map with machine fills, video as a tab

**Files:**
- Create: `app/src/components/run/RunSidePanel.vue`, `RunSidePanel.spec.ts`
- Modify: `app/src/components/MapPanel.vue` (one new optional prop)

**Interfaces:**
- `MapPanel` gains `fills?: Map<string, string | null>` (default empty). In the entity `<rect>` rendering, when `fills.get(\`${e.position.x},${e.position.y}\`)` is a string, the rect's `fill` becomes `var(--color-status-<statusClass(status)>)` instead of the hashed entity colour, and its `<title>` gains ` · <status>`. `MapPanel.spec.ts` gains one test for this.
- `RunSidePanel` props `{video: VideoManifest | null; videoTicks: VideoTicksResponse | null; videoError: string | null; runId: string; cursor: number}` plus pass-through map props `{entities, bots, trail, records, bounds, mapError, fills}`; emits `seek(tick)`, `pause()`. Tabs `Map` and `Video`; the Video tab exists only when `video?.video` is non-null. The video element and its two-way sync move here verbatim from `RunsPage.vue` (`videoClock`, `videoSrc`, `videoAt`, `videoRange`, `videoIssues`, `SYNC_SLOP_S`, `videoDriving`, `watchEffect`, `onVideoTime`, `onVideoPlay`), with `store.seek` → `emit('seek', tick)` and `store.togglePlay()` → `emit('pause')`, and `store.cursor` → `props.cursor`.

- [ ] **Step 1: Write the failing tests**

```ts
// app/src/components/run/RunSidePanel.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {CLEAN_MANIFEST, CLEAN_TICKS, NO_VIDEO_MANIFEST} from '@/api/video.fixtures';
import RunSidePanel from './RunSidePanel.vue';

const base = {runId: 'run-1', cursor: 100, entities: [], bots: [], trail: {}, records: [], bounds: null, mapError: null, fills: new Map(), videoTicks: null, videoError: null};

describe('RunSidePanel', () => {
    it('shows only the map tab when the run recorded no video', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST}});
        expect(w.findAll('[role="tab"]').map((t) => t.text())).toEqual(['Map']);
        expect(w.find('video').exists()).toBe(false);
    });
    it('offers a video tab when a recording exists and switches to it', async () => {
        const w = mount(RunSidePanel, {props: {...base, video: CLEAN_MANIFEST, videoTicks: CLEAN_TICKS}});
        const tabs = w.findAll('[role="tab"]');
        expect(tabs.map((t) => t.text())).toEqual(['Map', 'Video']);
        await tabs[1].trigger('click');
        expect(w.find('video').exists()).toBe(true);
        expect(w.get('video').attributes('src')).toBe('/api/v1/runs/run-1/video/file');
    });
    it('shows the map error in place of the map', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST, mapError: 'entity map unavailable — x'}});
        expect(w.text()).toContain('entity map unavailable');
    });
});
```

And in `MapPanel.spec.ts`, add:

```ts
    it('fills a machine with its status colour when a fill is given for its position', () => {
        const entities: EntitySnapshot[] = [{name: 'stone-furnace', position: {x: 10, y: 12}, direction: 0}];
        const w = mountPanel({entities, fills: new Map([['10,12', 'no_fuel']])});
        const rect = w.get(`${MAP} rect[data-entity="stone-furnace"]`);
        expect(rect.attributes('fill')).toBe('var(--color-status-serious)');
        expect(rect.find('title').text()).toContain('no_fuel');
    });
```

(`mountPanel` is the spec's existing helper; extend its props type to accept `fills`. If the entity rect has no `data-entity` attribute yet, add `:data-entity="feature.name"` to the rect in `MapPanel.vue` — that is a test hook, not a behaviour change.)

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/RunSidePanel.spec.ts src/components/MapPanel.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement**

In `MapPanel.vue`: add `fills?: Map<string, string | null>` to the props with default `() => new Map()`; import `statusClass` from `@/lib/machineTimeline`; in the entity rect binding replace the fill with

```ts
:fill="fillFor(feature)"
```

and add

```ts
function fillFor(feature: {position: Position; color: string}): string {
    const status = props.fills.get(`${feature.position.x},${feature.position.y}`);
    return typeof status === 'string' ? `var(--color-status-${statusClass(status)})` : feature.color;
}
```

(adapt the field names to the entity feature shape in `mapFeatures.ts` — the feature carries `position` and its hashed colour; read the file, do not guess). Append ` · ${status}` to the tooltip/title text where the feature's sentence is composed, when a status exists.

```vue
<!-- app/src/components/run/RunSidePanel.vue -->
<script setup lang="ts">
import {computed, ref, watchEffect} from 'vue';
import {Bounds, EntitySnapshot, MapRecord, Position, VideoManifest, VideoTicksResponse} from '@/api/types';
import {parseVideoClock, tickToVideoSeconds, videoSecondsToTick} from '@/api/videoClock';
import {videoDefects} from '@/api/videoJoin';
import {BotDot} from '@/lib/mapFeatures';
import MapPanel from '@/components/MapPanel.vue';

const props = defineProps<{
    runId: string; cursor: number;
    video: VideoManifest | null; videoTicks: VideoTicksResponse | null; videoError: string | null;
    entities: EntitySnapshot[]; bots: BotDot[]; trail: Record<number, Position[]>; records: MapRecord[]; bounds: Bounds | null;
    mapError: string | null; fills: Map<string, string | null>;
}>();
const emit = defineEmits<{seek: [tick: number]; pause: []}>();

const hasVideo = computed(() => props.video?.video != null);
const tab = ref<'map' | 'video'>('map');
watchEffect(() => { if (!hasVideo.value) tab.value = 'map'; });

const videoClock = computed(() => parseVideoClock(props.video, props.videoTicks));
const videoSrc = computed(() => (hasVideo.value ? `/api/v1/runs/${props.runId}/video/file` : null));
const videoAt = computed(() => (videoClock.value === null ? null : tickToVideoSeconds(videoClock.value, props.cursor)));
const videoRange = computed(() => props.video?.tick_range ?? null);
const videoIssues = computed(() => (props.video === null ? [] : videoDefects(props.video)));
const videoEl = ref<HTMLVideoElement | null>(null);

// See the header comment that used to live in RunsPage.vue: two directions,
// one loop, broken by a quarter-second slop and a driving flag.
const SYNC_SLOP_S = 0.25;
const videoDriving = ref(false);
watchEffect(() => {
    const at = videoAt.value, el = videoEl.value;
    if (el === null || at === null || videoDriving.value) return;
    if (Math.abs(el.currentTime - at.seconds) > SYNC_SLOP_S) el.currentTime = at.seconds;
});
function onVideoTime() {
    const el = videoEl.value, clock = videoClock.value;
    if (el === null || clock === null || el.paused) return;
    const at = videoSecondsToTick(clock, el.currentTime);
    if (at === null) return;
    videoDriving.value = true;
    emit('seek', at.tick);
    setTimeout(() => (videoDriving.value = false), 0);
}
function onVideoPlay() { emit('pause'); }
</script>

<template>
  <section class="border-t border-divider">
    <div role="tablist" class="flex gap-1 border-b border-divider bg-surface px-3 pt-2">
      <button role="tab" type="button" :aria-selected="tab === 'map'" class="rounded-t border border-b-0 border-divider px-3 py-1 text-sm"
              :class="tab === 'map' ? 'bg-card text-ink' : 'text-ink-muted'" @click="tab = 'map'">Map</button>
      <button v-if="hasVideo" role="tab" type="button" :aria-selected="tab === 'video'" class="rounded-t border border-b-0 border-divider px-3 py-1 text-sm"
              :class="tab === 'video' ? 'bg-card text-ink' : 'text-ink-muted'" @click="tab = 'video'">Video</button>
    </div>
    <div v-if="tab === 'map'" class="p-3">
      <p v-if="mapError" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ mapError }}</p>
      <MapPanel v-else :entities="entities" :bots="bots" :trail="trail" :records="records" :bounds="bounds" :fills="fills"/>
    </div>
    <div v-else class="p-3">
      <p v-if="videoError" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ videoError }}</p>
      <p v-for="d in videoIssues" :key="d.kind" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ d.message }}</p>
      <video ref="videoEl" :src="videoSrc ?? undefined" preload="metadata" controls class="block max-h-[60vh] max-w-full border border-divider"
             @timeupdate="onVideoTime" @seeked="onVideoTime" @play="onVideoPlay"/>
      <p v-if="video?.video" class="mt-1 font-mono text-xs text-ink-muted">
        {{ video.video.width }}x{{ video.video.height }} · {{ video.video.fps }} fps · {{ ((video.bytes ?? 0) / 1048576).toFixed(0) }} MB ·
        <template v-if="videoAt">at {{ videoAt.seconds.toFixed(1) }}s</template>
        <template v-else-if="videoRange && cursor < videoRange.from">
          recording starts at <button type="button" class="underline" @click="emit('seek', videoRange.from)">tick {{ videoRange.from }}</button>
        </template>
        <template v-else-if="videoRange && cursor > videoRange.to">recording ended at tick {{ videoRange.to }}</template>
        <template v-else>the clock cannot place tick {{ cursor }}</template>
      </p>
    </div>
  </section>
</template>
```

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/RunSidePanel.spec.ts src/components/MapPanel.spec.ts` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd app && pnpm lint && cd ..
cat > /tmp/msg <<'EOF'
feat(app): `RunSidePanel`, the map with machine status fills and the video as an on-demand tab
EOF
git add -- app/src/components/run/RunSidePanel.vue app/src/components/run/RunSidePanel.spec.ts app/src/components/MapPanel.vue app/src/components/MapPanel.spec.ts
git commit -F /tmp/msg -- app/src/components/run/RunSidePanel.vue app/src/components/run/RunSidePanel.spec.ts app/src/components/MapPanel.vue app/src/components/MapPanel.spec.ts
```

---

### Task 17: `RunPage.vue` at `/runs/:id`, and `RunsPage.vue` as the list

**Files:**
- Create: `app/src/pages/RunPage.vue`
- Modify: `app/src/pages/RunsPage.vue` (the list only), `app/src/router.ts`

**Interfaces:**
- Consumes: everything above; `useRunsStore` getters `bounds`, `window`, `cursor`, `playing`, `rate`, `samples`, `events`, `lanes`, `detail`, `entities`, `mapBots`, `trail`, `map`, `mapBounds`, `video`, `videoTicks`, `*Error`, `machineFills`, `selectedMachine`; actions `openRun`, `seek`, `togglePlay`, `selectMachine`, `setReference`; `rateItems` (Task 3); `headline` (Task 14); `lagTicks` (Task 7); `compareSplits` (existing).
- `RunPage` reads `route.params.id`, calls `store.openRun(id)` on mount and on param change, runs the play timer (moved from `RunsPage.vue`), and composes: `RunHeadline` → `MilestoneRibbon` → `TickAxis` → `ProductionBand` → `PowerBand` → `ResearchBand` → `LaneBand` → `MachineBand` → `CoverageBand` → `CursorBar` → legend → `RunSidePanel`, each band inside a `BandFrame`. The compare-with select and the splits table with deltas stay, below the side panel, moved verbatim from `RunsPage.vue` but styled with tokens (`text-success-dark` / `text-danger-dark` for faster/slower).
- `RunsPage` becomes the list: the existing sidebar list markup, as `<router-link :to="\`/runs/${run.run_id}\`">` items, full width, no viewer.

- [ ] **Step 1: Add the route**

In `router.ts`, after the `/runs` entry:

```ts
    {
        path: '/runs/:id',
        name: 'run',
        component: () => import('./pages/RunPage.vue')
    },
```

- [ ] **Step 2: Write `RunPage.vue`**

```vue
<!-- app/src/pages/RunPage.vue -->
<script setup lang="ts">
/**
 * One run, read on one clock. Every band is a lane on the same tick axis and
 * the cursor is the only control. The analysis window (`store.window`, from
 * `run_started`) feeds rates and verdicts; the drawn axis (`store.bounds`)
 * feeds positions. They differ by the lead-in and both are shown.
 */
import {computed, onBeforeUnmount, onMounted, watch} from 'vue';
import {useRoute} from 'vue-router';
import {useRunsStore} from '@/store/runsStore';
import {compareSplits, formatTicks, formatWhen, startedUnixOf} from '@/lib/runTimeline';
import {rateItems} from '@/lib/runRates';
import {headline} from '@/lib/runHeadline';
import {lagTicks} from '@/lib/runCoverage';
import BandFrame from '@/components/run/BandFrame.vue';
import TickAxis from '@/components/run/TickAxis.vue';
import ProductionBand from '@/components/run/ProductionBand.vue';
import PowerBand from '@/components/run/PowerBand.vue';
import ResearchBand from '@/components/run/ResearchBand.vue';
import LaneBand from '@/components/run/LaneBand.vue';
import MachineBand from '@/components/run/MachineBand.vue';
import CoverageBand from '@/components/run/CoverageBand.vue';
import CursorBar from '@/components/run/CursorBar.vue';
import RunHeadline from '@/components/run/RunHeadline.vue';
import MilestoneRibbon from '@/components/run/MilestoneRibbon.vue';
import RunSidePanel from '@/components/run/RunSidePanel.vue';

const route = useRoute();
const store = useRunsStore();
const id = computed(() => String(route.params.id));

let timer: number | null = null;
function stopTimer() { if (timer !== null) { window.clearInterval(timer); timer = null; } }
watch(() => store.playing, (playing) => { stopTimer(); if (playing) timer = window.setInterval(() => store.advance(), 100); });
onMounted(() => { store.loadRuns(); store.openRun(id.value); });
watch(id, (next) => store.openRun(next));
onBeforeUnmount(stopTimer);

const scale = computed(() => store.bounds);
const win = computed(() => store.window ?? (store.bounds ? {lo: store.bounds.from, hi: store.bounds.to} : null));
const items = computed(() => (win.value ? rateItems(store.samples, win.value.lo, win.value.hi) : []));
const sentence = computed(() => win.value
    ? headline({samples: store.samples, events: store.events, splits: store.detail?.splits ?? [], lo: win.value.lo, hi: win.value.hi, items: items.value})
    : 'no events recorded');
const lag = computed(() => (win.value ? lagTicks(win.value.hi, store.samples) : null));
const deltas = computed(() => store.reference === null ? null
    : new Map(compareSplits(store.detail?.splits ?? [], store.reference.splits).map((r) => [r.goal, r])));
const otherRuns = computed(() => store.runs.filter((r) => r.run_id !== id.value));

const LEGEND = [
    ['walk', 'verb-walk'], ['mine / chop', 'verb-mine'], ['craft', 'verb-craft'], ['place', 'verb-place'],
    ['feed (insert · stock · take · fuel)', 'verb-feed'], ['research', 'verb-research'],
    ['working', 'status-good'], ['no ingredients', 'status-warn'], ['no fuel', 'status-serious'], ['no power', 'status-critical'], ['normal / other', 'status-neutral']
] as const;
</script>

<template>
  <div class="mx-auto max-w-[1400px]">
    <p v-if="store.error" class="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger-dark">{{ store.error }}</p>
    <p v-else-if="store.loading && !store.detail" class="text-ink-muted">loading {{ id }}…</p>
    <section v-else-if="store.detail" class="overflow-hidden rounded-card border border-divider bg-card">
      <RunHeadline :summary="store.detail.summary" :provenance="null" :lag-ticks="lag" :headline="sentence" :roster="store.laneBotIds"/>
      <p class="px-5 py-1 text-xs text-ink-muted">
        {{ formatWhen(startedUnixOf(store.detail.summary)) }} ·
        <router-link :to="`/runs/${id}/analysis`" class="underline">overrun and divergence tables</router-link>
      </p>

      <template v-if="scale && win">
        <div class="grid grid-cols-[10.5rem_1fr] border-b border-divider">
          <div class="border-r border-divider bg-surface px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-ink-muted">Milestones</div>
          <MilestoneRibbon :scale="scale" :splits="store.detail.splits" :cursor="store.cursor"/>
        </div>
        <BandFrame title="Axis" :subtitle="store.leadIn > 0 ? `axis starts ${formatTicks(store.leadIn)} after run start` : 'minutes of game time · 5-min marks'">
          <TickAxis :scale="scale" :cursor="store.cursor"/>
        </BandFrame>
        <BandFrame title="Items / min" subtitle="trailing 2-min window · background is the attribution verdict per minute">
          <p v-if="store.sampleError" class="px-3 py-2 text-sm text-warn-dark">{{ store.sampleError }}</p>
          <ProductionBand v-else :scale="scale" :cursor="store.cursor" :samples="store.samples" :events="store.events" :items="items" :lo="win.lo" :hi="win.hi"/>
        </BandFrame>
        <BandFrame title="Power" subtitle="kW generated vs consumed · one scale">
          <PowerBand :scale="scale" :cursor="store.cursor" :samples="store.samples"/>
        </BandFrame>
        <BandFrame title="Research" subtitle="progress of the current technology">
          <ResearchBand :scale="scale" :cursor="store.cursor" :samples="store.samples"/>
        </BandFrame>
        <BandFrame title="Bots" subtitle="one row per bot · idle is hatched · feeding acts are ticks · replans are dashed">
          <p v-if="store.lanesError" class="px-3 py-2 text-sm text-warn-dark">{{ store.lanesError }}</p>
          <LaneBand v-else :scale="scale" :cursor="store.cursor" :lanes="store.lanes" :events="store.events"/>
        </BandFrame>
        <BandFrame title="Machines" subtitle="status of every sampled machine, 5-s cells · grouped by kind, ordered by placement">
          <MachineBand :scale="scale" :cursor="store.cursor" :samples="store.samples" :selected="store.selectedMachine" @select="store.selectMachine($event)"/>
        </BandFrame>
        <BandFrame title="Record" subtitle="where the record has data · a gap reads as “no record”">
          <p v-if="store.eventsError" class="px-3 py-2 text-sm text-warn-dark">{{ store.eventsError }}</p>
          <CoverageBand :scale="scale" :cursor="store.cursor" :events="store.events" :samples="store.samples" :run-end="win.hi"/>
        </BandFrame>
        <CursorBar :scale="scale" :cursor="store.cursor" :playing="store.playing" :rate="store.rate"
                   @seek="store.seek($event)" @toggle="store.togglePlay()" @rate="store.rate = $event"/>
        <div class="flex flex-wrap gap-x-4 gap-y-1.5 border-t border-divider px-5 py-2 text-xs text-ink-muted">
          <span v-for="[label, token] in LEGEND" :key="label" class="inline-flex items-center gap-1.5">
            <i class="inline-block h-2 w-3 rounded-sm" :style="{background: `var(--color-${token})`}"/>{{ label }}
          </span>
          <span class="inline-flex items-center gap-1.5">
            <i class="inline-block h-2 w-3 rounded-sm" style="background: repeating-linear-gradient(135deg, var(--color-ink-muted) 0 1px, transparent 1px 4px)"/>idle — no dispatched action
          </span>
        </div>
      </template>
      <p v-else class="px-5 py-4 text-sm text-ink-muted">this run recorded nothing to place on an axis</p>

      <RunSidePanel :run-id="id" :cursor="store.cursor"
                    :video="store.video" :video-ticks="store.videoTicks" :video-error="store.videoError"
                    :entities="store.entities" :bots="store.mapBots" :trail="store.trail" :records="store.map" :bounds="store.mapBounds"
                    :map-error="store.mapError" :fills="store.machineFills"
                    @seek="store.seek($event)" @pause="store.playing && store.togglePlay()"/>

      <div class="border-t border-divider px-5 py-4">
        <label v-if="otherRuns.length > 0" class="text-sm text-ink-muted">
          compare with
          <select class="ml-1 rounded border border-divider bg-card px-1 py-0.5 text-sm" :value="store.reference?.summary.run_id ?? ''"
                  @change="store.setReference(($event.target as HTMLSelectElement).value || null)">
            <option value="">— none —</option>
            <option v-for="r in otherRuns" :key="r.run_id" :value="r.run_id">{{ r.run_id }}</option>
          </select>
        </label>
        <table class="mt-3 w-full text-sm">
          <thead><tr class="text-left text-xs uppercase tracking-wider text-ink-muted"><th class="py-1">#</th><th>milestone</th><th>at</th><th>took</th><th v-if="deltas">vs ref</th><th></th></tr></thead>
          <tbody>
            <tr v-for="s in store.detail.splits" :key="`${s.index}-${s.started_tick}`" class="border-t border-divider">
              <td class="py-1">{{ s.index }}</td><td>{{ s.goal }}</td>
              <td class="font-mono tabular-nums">{{ s.started_tick }}</td>
              <td class="font-mono tabular-nums">{{ formatTicks(s.elapsed_ticks) }}</td>
              <td v-if="deltas" class="font-mono tabular-nums">
                <span v-if="deltas.get(s.goal)?.delta != null" :class="(deltas.get(s.goal)!.delta as number) < 0 ? 'text-success-dark' : 'text-danger-dark'">
                  {{ (deltas.get(s.goal)!.delta as number) > 0 ? '+' : '' }}{{ formatTicks(deltas.get(s.goal)!.delta) }}
                </span>
                <span v-else>—</span>
              </td>
              <td :class="s.outcome === 'stuck' || s.outcome === 'stuck_silent' ? 'text-danger-dark' : s.outcome === 'unfinished' ? 'text-ink-muted' : ''">{{ s.outcome }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>
```

- [ ] **Step 3: Reduce `RunsPage.vue` to the list**

Replace the whole file with:

```vue
<script setup lang="ts">
/** The archive. One run is read at `/runs/:id`. */
import {onBeforeUnmount, onMounted, ref} from 'vue';
import {useRunsStore} from '@/store/runsStore';
import {formatAgo, formatTicks, formatWhen, startedUnixOf} from '@/lib/runTimeline';

const store = useRunsStore();
const nowUnix = ref(Math.floor(Date.now() / 1000));
let clock: number | null = null;
onMounted(() => {
    store.loadRuns();
    clock = window.setInterval(() => (nowUnix.value = Math.floor(Date.now() / 1000)), 30_000);
});
onBeforeUnmount(() => { if (clock !== null) window.clearInterval(clock); });
</script>

<template>
  <div class="mx-auto max-w-[900px]">
    <h2 class="mb-3 text-xl font-semibold">Runs</h2>
    <p v-if="store.error" class="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger-dark">{{ store.error }}</p>
    <p v-else-if="store.runs.length === 0" class="text-ink-muted">No runs recorded yet.</p>
    <ul class="grid gap-1.5">
      <li v-for="run in store.runs" :key="run.run_id">
        <router-link :to="`/runs/${run.run_id}`"
                     class="block rounded-card border border-divider bg-card px-3 py-2 hover:border-brand focus-visible:ring-2 focus-visible:ring-focus">
          <span class="block font-semibold">{{ formatWhen(startedUnixOf(run)) }}
            <em class="ml-1.5 font-normal not-italic text-ink-muted">{{ formatAgo(startedUnixOf(run), nowUnix) }}</em></span>
          <span class="block font-mono text-xs text-ink-muted">{{ run.run_id }}</span>
          <span class="block text-xs text-ink-muted">
            <!-- `finished: false` means crashed OR still going; the server cannot tell them apart, so neither does this. -->
            {{ run.finished ? run.outcome : 'unfinished' }} · {{ formatTicks(run.elapsed_ticks) }}
          </span>
        </router-link>
      </li>
    </ul>
  </div>
</template>
```

- [ ] **Step 4: Verify in the browser and with the suite**

Run: `cd app && pnpm lint && pnpm run test:coverage && pnpm run build:web`
Expected: lint clean, all suites pass, coverage gate holds, build succeeds.

Then with `just serve` running: open `http://localhost:7492/#/runs`, click `run-1788696619-00325`. Check, against the `just analyse` output for that run:
- headline reads `rates: iron-plate 8/min at 5:00 (roster-fed; no generator until 4:06) …`;
- the plates band is hatched copper across every producing minute, the 5:00 mark labelled;
- power shows `no generator until 4:06` and 900 kW after;
- four bot rows, bot 1 `idle 32%`, one dashed replan rule at the start;
- 17 machine rows; drag the cursor and watch furnace fills on the map change;
- the Record band reads `samples lag 24 ticks · no record: —` (this run has bot samples, so no `no record`);
- toggle the theme: every band's ground, grid and ink flip; the map stays dark.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): `/runs/:id`, one run read on one clock

The runs page becomes the archive list; a run opens at its own URL as a
stacked tick-axis instrument: headline with the attribution verdict,
milestone ribbon, rates per item with the verdict painted behind the curve,
power, research, bot lanes with idle drawn, the machine status heatmap, the
record coverage band, one cursor bar, and the map (with machine status fills)
beside the video as an on-demand tab. Hard-coded hex and the undefined
`--surface-border` fallbacks are gone; everything reads the theme tokens.
EOF
git add -- app/src/pages/RunPage.vue app/src/pages/RunsPage.vue app/src/router.ts
git commit -F /tmp/msg -- app/src/pages/RunPage.vue app/src/pages/RunsPage.vue app/src/router.ts
```

---

### Task 18: Final verification and the note

**Files:**
- Create: `docs/superpowers/notes/2026-09-08-run-anatomy-phase-1.md`

- [ ] **Step 1: Run the full frontend gate**

```bash
cd app && pnpm run precommit:check 2>&1 | tail -40
```

Expected: lint clean, coverage gate holds, build succeeds, the cargo steps (unchanged Rust) pass. If `cargo` steps fail with `pkg-config` errors, run the cargo half under `nix develop -c` from the repo root: `nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings && nix develop -c cargo test --workspace`. Report the real result; a red step is reported red.

- [ ] **Step 2: Write the note**

```markdown
# Run Anatomy, Phase 1 — what landed and what it showed

2026-09-08. Implements Phase 1 of
`docs/superpowers/specs/2026-09-08-run-anatomy-design.md`.

## What landed
- `/runs/:id`: headline, milestone ribbon, tick axis, items/min with the
  attribution verdict painted per minute, power, research, bot lanes with idle
  hatched and replans dashed, machine status heatmap, record coverage, one
  cursor bar; map with machine status fills, video as an on-demand tab.
- Six pure libs under `app/src/lib/`; the attribution and rate libs are pinned
  to `just analyse --json` for `run-1788696619-00325` by a golden test.
- Dark theme tokens and a per-viewer toggle; `--color-warn-dark` now exists.

## What the fixture run reads, against the tool
<paste the headline sentence and the 5:00 verdict rows from the page and from `just analyse`, side by side>

## What is not here (Phase 2/3)
- provenance chips read "not captured" — no route yet;
- `/samples` and `/map` load whole files; slices are Phase 2;
- the replay's `Evidence` is not on disk, so lane titles say "believed" for
  every walk by rule, not per row;
- no flow view.
```

Fill the middle section from the browser and the tool output; do not leave the angle-bracket placeholder in.

- [ ] **Step 3: Commit**

```bash
cat > /tmp/msg <<'EOF'
docs(notes): Run Anatomy Phase 1, what landed and what the fixture run reads
EOF
git add -- docs/superpowers/notes/2026-09-08-run-anatomy-phase-1.md
git commit -F /tmp/msg -- docs/superpowers/notes/2026-09-08-run-anatomy-phase-1.md
```

---

## Self-review against the spec (done while writing; recorded here)

- **§1 page anatomy**: 1 headline/chips → Task 14/17 (chips absent by design in Phase 1); 2 ribbon → 14; 3 axis → 9; 4 production → 10 (plateau annotation deferred: `classify_plateau` needs the machine-status inference and is Phase 2 work — noted as a gap); 5 power → 11; 6 research → 11; 7 lanes → 12 (attempt number needs the persisted replay, Phase 2); 8 heatmap → 13; 9 coverage → 14; 10 cursor → 9; 11 map/video → 16; 12 flow → Phase 3.
- **§2 data flow**: every lib in the table exists (Tasks 3–7, 14); the store gains `/events` (Task 15).
- **§3 components**: all named components created; `RunsPage.vue` moved onto tokens (Task 17); `warn-dark` fixed (Task 8).
- **§4 error handling**: each enrichment fails independently and renders its reason in place (Tasks 15, 17); inference source labelled (Task 4); truncation count on the heatmap **not done** — `truncated` is not surfaced; add to Phase 2 with the slices.
- **§5 testing**: golden tests (3, 4), idle equality (5), DOM specs per component, `pnpm lint` in every task.
- **Type consistency**: `TickScale {from,to}` everywhere; `window {lo,hi}` on the store vs `scale` for drawing — deliberate and named; `Verdict` union identical in `fixtureRun.ts` and `runAttribution.ts`; `BotDot` for map bots; `MachineRow.key` is the string sample key throughout.
