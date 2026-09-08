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
