import {describe, expect, it} from 'vitest';
import {
    botsOf,
    camerasOf,
    compareSplits,
    formatTicks,
    fractionOf,
    frameAt,
    placeable,
    splitAt,
    tickBounds
} from './runTimeline';
import {ArchivedFrame, Split} from '@/api/types';

const frame = (bot: number, tick: number | null, camera: string | null): ArchivedFrame => ({
    bot,
    tick,
    camera,
    file: `frames/${bot}/tick-${tick}-${camera}.jpg`
});

const split = (
    index: number,
    goal: string,
    started: number,
    ended: number | null,
    outcome = 'satisfied'
): Split => ({
    index,
    goal,
    started_tick: started,
    ended_tick: ended,
    outcome,
    elapsed_ticks: ended === null ? null : ended - started
});

describe('placeable', () => {
    it('drops frames that cannot be positioned rather than inventing a tick', () => {
        const placed = placeable([frame(1, 300, 'front'), frame(1, null, null)]);
        expect(placed).toHaveLength(1);
        expect(placed[0].tick).toBe(300);
    });

    it('sorts by tick so the axis order is not filesystem order', () => {
        const placed = placeable([frame(1, 600, 'a'), frame(1, 300, 'a')]);
        expect(placed.map((f) => f.tick)).toEqual([300, 600]);
    });
});

describe('frameAt', () => {
    const frames = placeable([
        frame(1, 300, 'front'),
        frame(1, 600, 'front'),
        frame(1, 900, 'front'),
        frame(2, 600, 'front'),
        frame(1, 600, 'area')
    ]);

    it('takes the latest frame at or before the cursor, not an exact match', () => {
        // Capture is every 300 ticks and frames drop; an exact match would
        // blank the panel for almost every cursor position.
        expect(frameAt(frames, 1, 'front', 750)?.tick).toBe(600);
    });

    it('takes the frame exactly on the cursor when there is one', () => {
        expect(frameAt(frames, 1, 'front', 600)?.tick).toBe(600);
    });

    it('is null before the first frame, which is a real state', () => {
        expect(frameAt(frames, 1, 'front', 100)).toBeNull();
    });

    it('never crosses to another bot or camera', () => {
        expect(frameAt(frames, 2, 'front', 750)?.bot).toBe(2);
        expect(frameAt(frames, 1, 'area', 750)?.camera).toBe('area');
        expect(frameAt(frames, 3, 'front', 750)).toBeNull();
    });
});

describe('botsOf / camerasOf', () => {
    it('reports each bot and camera once', () => {
        const frames = placeable([
            frame(2, 300, 'front'),
            frame(1, 300, 'front'),
            frame(1, 600, 'area')
        ]);
        expect(botsOf(frames)).toEqual([1, 2]);
        expect(camerasOf(frames)).toEqual(['area', 'front']);
    });
});

describe('tickBounds', () => {
    it('spans splits and frames together, so neither is clipped', () => {
        // Capture kept running past the last milestone.
        const bounds = tickBounds(
            [split(1, 'a', 100, 400)],
            placeable([frame(1, 900, 'front')])
        );
        expect(bounds).toEqual({from: 100, to: 900});
    });

    it('includes a milestone that started before the first frame', () => {
        const bounds = tickBounds(
            [split(1, 'a', 50, 400)],
            placeable([frame(1, 300, 'front')])
        );
        expect(bounds?.from).toBe(50);
    });

    it('ignores an unfinished milestone had no end', () => {
        const bounds = tickBounds([split(1, 'a', 100, null)], []);
        expect(bounds).toEqual({from: 100, to: 100});
    });

    it('is null when there is nothing to place at all', () => {
        // A planning-only run. Zero-to-zero would look like a run that took
        // no time rather than one with nothing on the axis.
        expect(tickBounds([], [])).toBeNull();
    });
});

describe('fractionOf', () => {
    it('maps a tick across the axis', () => {
        expect(fractionOf({from: 100, to: 200}, 150)).toBeCloseTo(0.5);
    });

    it('clamps outside the bounds', () => {
        expect(fractionOf({from: 100, to: 200}, 50)).toBe(0);
        expect(fractionOf({from: 100, to: 200}, 500)).toBe(1);
    });

    it('reports the start for a zero-length axis instead of NaN', () => {
        // NaN would reach a style attribute and render nothing, silently.
        expect(fractionOf({from: 100, to: 100}, 100)).toBe(0);
    });
});

describe('splitAt', () => {
    const splits = [split(1, 'a', 100, 400), split(2, 'b', 400, 900)];

    it('finds the milestone covering the cursor', () => {
        expect(splitAt(splits, 200)?.goal).toBe('a');
        expect(splitAt(splits, 500)?.goal).toBe('b');
    });

    it('is null outside every milestone', () => {
        expect(splitAt(splits, 50)).toBeNull();
        expect(splitAt(splits, 5000)).toBeNull();
    });

    it('treats an unfinished milestone as running to the end of time', () => {
        expect(splitAt([split(1, 'a', 100, null)], 99999)?.goal).toBe('a');
    });
});

describe('compareSplits', () => {
    it('matches by goal, not by position', () => {
        // The second run skipped a milestone; matching by index would compare
        // "steel" against "green science" and report a confident nonsense.
        const runA = [split(1, 'red', 0, 100), split(2, 'steel', 100, 400)];
        const runB = [split(1, 'red', 0, 160)];
        const rows = compareSplits(runA, runB);
        expect(rows[0]).toEqual({
            goal: 'red',
            ticks: 100,
            referenceTicks: 160,
            delta: -60
        });
        expect(rows[1].goal).toBe('steel');
        expect(rows[1].delta).toBeNull();
    });

    it('has no delta when either side never finished', () => {
        const rows = compareSplits([split(1, 'a', 0, null)], [split(1, 'a', 0, 100)]);
        expect(rows[0].delta).toBeNull();
    });
});

describe('formatTicks', () => {
    it('renders ticks as game-time duration', () => {
        expect(formatTicks(60)).toBe('1.0s');
        expect(formatTicks(3660)).toBe('1m 1.0s');
    });

    it('shows an em dash rather than a zero for an unknown duration', () => {
        expect(formatTicks(null)).toBe('—');
    });
});
