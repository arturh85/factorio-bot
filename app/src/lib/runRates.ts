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
    /** `null` when the trailing window has zero width (a sample at `lo` itself) -- absent, not zero. */
    perMinute: number | null;
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
            out.push({tick: s.tick, perMinute: null});
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

/**
 * The default items, then any other item whose final count clears the threshold.
 *
 * `final` is read from the last `force` sample in `samples` regardless of
 * `hi`, matching Python's `production_rates()` (`final = _made(force[-1])`,
 * where `force` is unfiltered) -- `hi` bounds the marks, not the item list.
 */
export function rateItems(samples: Sample[], lo: number, hi: number, threshold = RATE_ITEM_THRESHOLD): string[] {
    const force = forceSamples(samples);
    if (force.length === 0) return [...DEFAULT_RATE_ITEMS];
    const final = force[force.length - 1].production.made;
    const base = forceAt(samples, lo)?.production.made ?? {};
    const extra = Object.keys(final)
        .filter((n) => !DEFAULT_RATE_ITEMS.includes(n) && (final[n] ?? 0) - (base[n] ?? 0) >= threshold)
        .sort();
    return [...DEFAULT_RATE_ITEMS, ...extra];
}
