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

    it('reports unreadable event lines, and says nothing when there are none', () => {
        const props = {scale: {from: run.lo, to: run.hi}, cursor: run.lo, events: run.events, samples: run.samples, runEnd: run.hi};
        expect(mount(CoverageBand, {props}).text()).not.toContain('unreadable');
        expect(mount(CoverageBand, {props: {...props, skipped: 0}}).text()).not.toContain('unreadable');
        expect(mount(CoverageBand, {props: {...props, skipped: 1}}).text()).toContain('1 unreadable event line');
        expect(mount(CoverageBand, {props: {...props, skipped: 3}}).text()).toContain('3 unreadable event lines');
    });

    it('reads "cover to end", not a negative tick count, when the last sample is AFTER the declared run end', () => {
        // The fixture's own samples run past `run.hi`: pick a runEnd earlier
        // than the last recorded sample tick, the same shape a real run
        // produces when `run_finished` lands before the last sample beat.
        const lastSampleTick = Math.max(...run.samples.map((s) => s.tick));
        const runEnd = lastSampleTick - 36;
        const w = mount(CoverageBand, {props: {scale: {from: run.lo, to: run.hi}, cursor: run.lo, events: run.events, samples: run.samples, runEnd}});
        expect(w.text()).toContain('samples cover the run to its end');
        expect(w.text()).not.toMatch(/lag -?\d/);
    });
});
