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
