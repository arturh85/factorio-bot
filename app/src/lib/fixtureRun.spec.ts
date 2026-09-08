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
