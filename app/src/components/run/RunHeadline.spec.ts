// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import RunHeadline from './RunHeadline.vue';

describe('RunHeadline', () => {
    it('renders the sentence and marks every provenance chip absent until the route exists', () => {
        const w = mount(RunHeadline, {props: {
            summary: {run_id: 'run-1', finished: true, started_unix: 1, finished_unix: 2, outcome: 'done', elapsed_ticks: 10, events: 1, splits: 1},
            provenance: null, lagTicks: 24, headline: 'rates: …', roster: [1, 2, 3, 4]
        }});
        expect(w.text()).toContain('rates: …');
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="seed"]').text()).toContain('not captured');
        expect(w.get('[data-chip="roster"]').text()).toContain('1 2 3 4');
        expect(w.get('[data-chip="samples"]').text()).toContain('lag 24 ticks');
    });
});
