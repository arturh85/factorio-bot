// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import ResearchBand from './ResearchBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};
const clock = {from: run.lo, to: run.hi};

describe('ResearchBand', () => {
    it('draws one fill per technology and labels it', () => {
        const w = mount(ResearchBand, {props: {scale, clock, cursor: run.lo, samples: run.samples}});
        expect(w.findAll('path.tech')).toHaveLength(1);
        expect(w.text()).toContain('automation');
        expect(w.text()).toMatch(/\d+% at 6:0\d/);
    });
    it('says so when nothing was ever queued', () => {
        const none = run.samples.map((s) => (s.kind === 'force' ? {...s, research: null} : s));
        const w = mount(ResearchBand, {props: {scale, clock, cursor: run.lo, samples: none}});
        expect(w.text()).toContain('no research queued in this run');
    });
    it('labels the technology on the analysis CLOCK, not on a trimmed axis', () => {
        const full = mount(ResearchBand, {props: {scale, clock, cursor: run.lo, samples: run.samples}});
        const label = /started \d+:\d\d · \d+% at \d+:\d\d/.exec(full.text())![0];
        const trimmed = mount(ResearchBand, {props: {scale: {from: run.lo + 175, to: run.hi}, clock, cursor: run.lo, samples: run.samples}});
        expect(trimmed.text()).toContain(label);
    });
    it('says the FORCE SAMPLES are missing, not that nothing was queued, when there are none', () => {
        // A run whose `/samples` came back empty recorded no research state at
        // all; saying "no research queued" would be a claim about the run.
        const w = mount(ResearchBand, {props: {scale, clock, cursor: run.lo, samples: []}});
        expect(w.text()).toContain('no force samples in this run');
        expect(w.text()).not.toContain('no research queued in this run');
    });
});
