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
    it('draws an INFERRED verdict differently from a measured one, and says so in the word', () => {
        // Two mounts, because this fixture's machine counters cover every
        // minute it produced anything: on the run as archived every verdict is
        // counter-sourced, and the inference fallback is only reachable by
        // taking the `machines` samples away -- which is exactly the shape of
        // a run archived before those counters existed.
        const measured = mountBand(['iron-plate']).findAll('rect.verdict');
        expect(measured.length).toBeGreaterThan(0);
        expect(measured.every((r) => r.attributes('data-source') === 'counters')).toBe(true);
        expect(measured.some((r) => r.attributes('data-verdict') === 'roster-fed')).toBe(true);
        expect(measured[0].attributes('stroke-dasharray')).toBeUndefined();

        const preCounters = run.samples.filter((s) => s.kind !== 'machines');
        const w = mount(ProductionBand, {props: {scale, cursor: run.lo, samples: preCounters, events: run.events, items: ['iron-plate'], lo: run.lo, hi: run.hi}});
        const inferred = w.findAll('rect.verdict');
        expect(inferred.length).toBeGreaterThan(0);
        expect(inferred.every((r) => r.attributes('data-source') === 'inference')).toBe(true);
        expect(inferred[0].attributes('stroke')).toBe('var(--color-ink-muted)');
        expect(inferred[0].attributes('stroke-dasharray')).toBe('3 2');
        expect(w.text()).toContain('unclear (inferred)');
        expect(w.text()).toContain('roster-fed (inferred)');
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
    it('says the SAMPLES are missing, not that the items made nothing, when the run has no force samples', () => {
        // 5 of 21 archived runs answer `/samples` with an empty list. Deriving
        // "no output in this run" per item from that is a claim about the
        // factory the record never made.
        const w = mount(ProductionBand, {props: {scale, cursor: run.lo, samples: [], events: run.events, items: ['iron-plate', 'copper-plate'], lo: run.lo, hi: run.hi}});
        expect(w.text()).toContain('no production samples in this run');
        expect(w.text()).not.toContain('no output in this run');
        expect(w.findAll('rect.verdict')).toHaveLength(0);
    });
});
