// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import {tickX} from '@/lib/tickScale';
import PowerBand from './PowerBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};
const clock = {from: run.lo, to: run.hi};

describe('PowerBand', () => {
    it('names the first generation tick', () => {
        const w = mount(PowerBand, {props: {scale, clock, cursor: run.lo, samples: run.samples}});
        // first generated_kw > 0 sample is at tick 18000 -> 4:06 from 3242
        expect(w.text()).toContain('no generator until 4:06');
        expect(w.text()).toContain('900 kW');
    });
    it('draws no deficit stripe when demand is met, and one when it is not', () => {
        const ok = mount(PowerBand, {props: {scale, clock, cursor: run.lo, samples: run.samples}});
        expect(ok.findAll('rect.deficit')).toHaveLength(0);
        const starved = run.samples.map((s) => s.kind !== 'force' ? s : ({
            ...s, power: {...s.power, networks: {0: {sub_ids: [0], generated_kw: 0, consumed_kw: 0, demanded_kw: 60, satisfaction: 0}}}
        }));
        const bad = mount(PowerBand, {props: {scale, clock, cursor: run.lo, samples: starved}});
        const rects = bad.findAll('rect.deficit');
        expect(rects.length).toBeGreaterThan(0);
        // The first force sample's deficit has no prior sample to date its
        // start from, so it draws as the minimum-width mark, not a fabricated
        // 300-tick band.
        expect(rects[0].attributes('width')).toBe('1');
        // The fixture's force samples are at 3300, 3600, ... -- the second
        // rect spans the real interval between those two samples.
        expect(Number(rects[1].attributes('width'))).toBeCloseTo(tickX(scale, 3600) - tickX(scale, 3300));
    });
    it('dates the first generator on the analysis CLOCK, not on a trimmed axis', () => {
        const w = mount(PowerBand, {props: {scale: {from: run.lo + 175, to: run.hi}, clock, cursor: run.lo, samples: run.samples}});
        expect(w.text()).toContain('no generator until 4:06');
    });
    it('says so when there are no force samples', () => {
        const w = mount(PowerBand, {props: {scale, clock, cursor: run.lo, samples: []}});
        expect(w.text()).toContain('no power samples');
    });
});
