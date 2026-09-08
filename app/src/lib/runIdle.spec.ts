import {describe, expect, it} from 'vitest';
import {Event, Lane} from '@/api/types';
import {loadFixtureRun} from './fixtureRun';
import {idleIntervals, idleTicks, laneSegments, replanBoundaries, verbClass} from './runIdle';

const lane = (bot: number, from: number, to: number | null, action = 'mine 1 coal', id: number | null = 1): Lane =>
    ({bot, id, action, from_tick: from, to_tick: to, status: to === null ? null : 'success', error: null});

describe('verbClass', () => {
    it('groups the plan verbs into the six drawn classes', () => {
        expect(verbClass('walk to [1, 2]')).toBe('walk');
        expect(verbClass('mine 4 iron-ore')).toBe('mine');
        expect(verbClass('chop huge-rock')).toBe('mine');
        expect(verbClass('craft 2 pipe')).toBe('craft');
        expect(verbClass('place boiler at [1, 2]')).toBe('place');
        for (const v of ['insert', 'stock', 'take', 'fuel', 'charge']) expect(verbClass(`${v} 1 x`)).toBe('feed');
        expect(verbClass('research automation')).toBe('research');
        expect(verbClass('evacuate')).toBe('other');
    });
});

describe('replanBoundaries / laneSegments', () => {
    const plan = (tick: number): Event => ({kind: 'plan_created', tick, milestone_index: 1, steps: 1, makespan: 1, bots: [1], plan: []} as Event);
    it('numbers segments by the plan they were dispatched under', () => {
        const b = replanBoundaries([plan(100), plan(500)]);
        expect(b).toEqual([100, 500]);
        const segs = laneSegments([lane(1, 120, 130), lane(1, 600, 610)], b);
        expect(segs.map((s) => s.planIndex)).toEqual([1, 2]);
    });
    it('marks zero-length feeding acts as instant so they are drawn as ticks', () => {
        const [s] = laneSegments([lane(1, 100, 100, 'insert 2 coal')], []);
        expect(s.instant).toBe(true);
        expect(s.verb).toBe('feed');
    });
});

describe('idleIntervals', () => {
    const scale = {from: 0, to: 1000};
    it('is the axis minus the union of the bot\'s lanes', () => {
        const gaps = idleIntervals([lane(1, 100, 200), lane(1, 150, 300), lane(1, 600, 700)], 1, scale);
        expect(gaps).toEqual([{from: 0, to: 100}, {from: 300, to: 600}, {from: 700, to: 1000}]);
        expect(idleTicks(gaps)).toBe(700);
    });
    it('treats an unterminated lane as covering to the end of the axis', () => {
        expect(idleIntervals([lane(1, 100, null)], 1, scale)).toEqual([{from: 0, to: 100}]);
    });
    it('ignores other bots\' lanes', () => {
        expect(idleIntervals([lane(2, 0, 1000)], 1, scale)).toEqual([{from: 0, to: 1000}]);
    });
    it('reproduces the analysis tool\'s idle figure for bot 1 on the fixture run', () => {
        const run = loadFixtureRun();
        // `just analyse`: "busy 14998 + idle 6984 = 21982 ticks; idle is 31.8% of span"
        expect(idleTicks(idleIntervals(run.lanes, 1, {from: run.lo, to: run.hi}))).toBe(6984);
    });
});
