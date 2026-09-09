// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {Event} from '@/api/types';
import {loadFixtureRun} from '@/lib/fixtureRun';
import LaneBand from './LaneBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};
const clock = {from: run.lo, to: run.hi};

describe('LaneBand', () => {
    const w = mount(LaneBand, {props: {scale, clock, cursor: run.lo, lanes: run.lanes, events: run.events}});
    it('draws one hatched idle row per bot with the bot\'s idle share', () => {
        expect(w.findAll('rect.idle')).toHaveLength(4);
        expect(w.text()).toContain('bot 1');
        expect(w.text()).toContain('idle 32%'); // 6984 / 21982
    });
    it('draws every lane as a segment carrying its verb and plan', () => {
        const segs = w.findAll('rect.segment');
        expect(segs).toHaveLength(228);
        expect(new Set(segs.map((s) => s.attributes('data-verb')))).toEqual(new Set(['walk', 'mine', 'craft', 'place', 'feed', 'research']));
        expect(segs.every((s) => s.attributes('data-plan') === '1')).toBe(true);
    });
    it('marks walks as believed in their title and draws the replan boundary', () => {
        const walk = w.findAll('rect.segment').find((s) => s.attributes('data-verb') === 'walk')!;
        expect(walk.find('title').text()).toContain('evidence: believed');
        expect(w.findAll('line.replan')).toHaveLength(1);
        expect(w.text()).toContain('plan 1 · ids restart here');
    });
    it('dates the replan boundary on the analysis clock, not on a trimmed axis', () => {
        const at = /ids restart here · (\d+:\d\d)/.exec(w.text())![1];
        const trimmed = mount(LaneBand, {props: {scale: {from: run.lo + 175, to: run.hi}, clock, cursor: run.lo, lanes: run.lanes, events: run.events}});
        expect(trimmed.text()).toContain(`ids restart here · ${at}`);
    });
    it('outlines a failed segment and extends an unterminated one to the axis end', () => {
        const lanes = [
            {bot: 1, id: 1, action: 'mine 1 coal', from_tick: 4000, to_tick: 5000, status: 'failed', error: 'x'},
            {bot: 1, id: 2, action: 'craft 1 pipe', from_tick: 6000, to_tick: null, status: null, error: null}
        ];
        const v = mount(LaneBand, {props: {scale, clock, cursor: run.lo, lanes, events: []}});
        const [failed, open] = v.findAll('rect.segment');
        expect(failed.attributes('stroke')).toBe('var(--color-status-critical)');
        expect(Number(open.attributes('x')) + Number(open.attributes('width'))).toBeCloseTo(1000, 0);
        expect(open.find('title').text()).toContain('never settled');
    });
    it('draws an abandoned step as a hollow mark that does not count as work', () => {
        const lanes = [{bot: 1, id: 9, action: 'abandoned: predecessor 5 failed', from_tick: 5000, to_tick: 5000, status: 'abandoned', error: 'abandoned: predecessor 5 failed'}];
        const w = mount(LaneBand, {props: {scale, clock, cursor: run.lo, lanes, events: []}});
        const seg = w.get('rect.segment');
        expect(seg.attributes('fill')).toBe('none');
        expect(seg.find('title').text()).toContain('never dispatched');
        expect(w.text()).toContain('idle 100%');
    });
    it('draws a refused replan as a critical dashed line across every row', () => {
        const events: Event[] = [
            {kind: 'planning_timed', tick: 452, planning_ms: 1, paused: true, reason: null, tick_before: 452, tick_after: 452} as Event,
            {kind: 'plan_created', tick: 452, milestone_index: 1, steps: 1, makespan: 1, bots: [1], plan: []} as Event,
            {kind: 'planning_timed', tick: 48017, planning_ms: 1, paused: true, reason: null, tick_before: 48017, tick_after: 48017} as Event
        ];
        const w = mount(LaneBand, {props: {scale, clock, cursor: run.lo, lanes: run.lanes, events}});
        const lines = w.findAll('line.replan-refused');
        expect(lines).toHaveLength(1);
        expect(w.text()).toContain('replan refused');
    });
});
