import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';
import {headline} from './runHeadline';

describe('headline', () => {
    const run = loadFixtureRun();
    it('reads like just analyse on the fixture run', () => {
        const h = headline({samples: run.samples, events: run.events, lo: run.lo, hi: run.hi, items: ['iron-plate', 'copper-plate'],
            splits: [{index: 1, goal: 'research automation', started_tick: 3242, ended_tick: 25216, outcome: 'satisfied', elapsed_ticks: 21974}]});
        expect(h).toBe('rates: iron-plate 8/min at 5:00 (roster-fed · 94 items; no generator until 4:06) · copper-plate 0/min at 5:00 (roster-fed · 26 items) | milestone 1 research automation satisfied at 6:06');
    });
    it('says when nothing applies', () => {
        expect(headline({samples: [], events: [], lo: 0, hi: 10, items: [], splits: []})).toBe('no marks reached and no milestones closed');
    });
});
