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
