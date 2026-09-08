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
