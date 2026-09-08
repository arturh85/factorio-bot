// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import MilestoneRibbon from './MilestoneRibbon.vue';

describe('MilestoneRibbon', () => {
    const splits = [
        {index: 1, goal: 'research automation', started_tick: 3242, ended_tick: 25216, outcome: 'satisfied', elapsed_ticks: 21974},
        {index: 2, goal: 'green', started_tick: 25216, ended_tick: null, outcome: 'unfinished', elapsed_ticks: null}
    ];
    it('positions one segment per split and marks the current one', () => {
        const w = mount(MilestoneRibbon, {props: {scale: {from: 3242, to: 30000}, clock: {from: 3242, to: 30000}, splits, cursor: 10000}});
        const segs = w.findAll('.seg');
        expect(segs).toHaveLength(2);
        expect(segs[0].classes()).toContain('is-current');
        expect(segs[0].text()).toContain('m1 · research automation · satisfied at 6:06 · 21,974 ticks');
        expect(segs[1].text()).toContain('— ticks');
    });
    it('dates a milestone on the analysis clock, so the ribbon and the headline say the same time', () => {
        // The drawn axis trims a lead-in; reading the time off it made this
        // ribbon say 6:03 while the headline said 6:06 of the same tick.
        const w = mount(MilestoneRibbon, {props: {scale: {from: 3242 + 175, to: 30000}, clock: {from: 3242, to: 30000}, splits, cursor: 10000}});
        expect(w.findAll('.seg')[0].text()).toContain('satisfied at 6:06');
    });
});
