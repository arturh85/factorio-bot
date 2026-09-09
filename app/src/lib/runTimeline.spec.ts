import {describe, expect, it} from 'vitest';
import {
    leadInTicks,
    formatAgo,
    formatWhen,
    startedUnixOf,
    laneAt,
    laneBots,
    compareSplits,
    formatTicks,
    fractionOf,
    splitAt,
    stuckReasons,
    tickBounds
} from './runTimeline';
import {Event, Split} from '@/api/types';

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

describe('tickBounds', () => {
    it('spans splits and the recording together, so neither is clipped', () => {
        // The recording kept running past the last milestone.
        const bounds = tickBounds([split(1, 'a', 100, 400)], [], {from: 900, to: 900});
        expect(bounds).toEqual({from: 100, to: 900});
    });

    it('includes a milestone that started before the recording', () => {
        const bounds = tickBounds([split(1, 'a', 50, 400)], [], {from: 300, to: 300});
        expect(bounds?.from).toBe(50);
    });

    it('ignores an unfinished milestone had no end', () => {
        const bounds = tickBounds([split(1, 'a', 100, null)], []);
        expect(bounds).toEqual({from: 100, to: 100});
    });

    it('starts at the first lane bar, not at the milestone that opened before it', () => {
        // The run began, the planner thought, and nothing was drawn for 150
        // ticks. Spanning that spends axis width on a stretch with no lane bar
        // and no recording.
        const lanes = [
            {bot: 1, id: 0, action: 'mine', from_tick: 250, to_tick: 2000,
                status: 'success', error: null}
        ];
        const bounds = tickBounds([split(1, 'a', 100, 2000)], lanes);
        expect(bounds).toEqual({from: 250, to: 2000});
        expect(leadInTicks([split(1, 'a', 100, 2000)], lanes)).toBe(150);
    });

    it('starts at the first lane bar when the bots moved before the recording did', () => {
        const bounds = tickBounds(
            [split(1, 'a', 100, 2000)],
            [{bot: 1, id: 0, action: 'mine', from_tick: 250, to_tick: 900, status: 'success', error: null}],
            {from: 400, to: 2000}
        );
        expect(bounds?.from).toBe(250);
    });

    it('refuses a trim that would cut more than it keeps', () => {
        // The only thing drawn landed at the very end. Trimming to it would
        // throw away the extent the splits carry and collapse the axis.
        const lanes = [
            {bot: 1, id: 0, action: 'mine', from_tick: 900, to_tick: 900,
                status: 'success', error: null}
        ];
        const bounds = tickBounds([split(1, 'a', 100, 400)], lanes);
        expect(bounds).toEqual({from: 100, to: 900});
        expect(leadInTicks([split(1, 'a', 100, 400)], lanes)).toBe(0);
    });

    it('reports no lead-in for a run with nothing drawn at all', () => {
        expect(leadInTicks([split(1, 'a', 100, 400)], [])).toBe(0);
    });

    it('is null when there is nothing to place at all', () => {
        // A planning-only run. Zero-to-zero would look like a run that took
        // no time rather than one with nothing on the axis.
        expect(tickBounds([], [])).toBeNull();
    });
});

/**
 * The silent regression this pins. Per-camera screenshot ticks decided where
 * the axis started until the cameras were retired (2026-09-02); nothing errors
 * or is marked now that contributor is gone -- the axis is just computed from
 * splits, lanes and the recording. A run has to land somewhere honest.
 */
describe('a run whose only capture is video', () => {
    const splits = [split(1, 'a', 100, 2000)];

    it('starts at the video clock rather than falling back to the splits', () => {
        // Without the video range this answers {from: 100}, silently, because
        // `drawn` would be empty and the axis would fall back to the splits.
        expect(tickBounds(splits, [], {from: 300, to: 2000})).toEqual({from: 300, to: 2000});
        expect(leadInTicks(splits, [], {from: 300, to: 2000})).toBe(200);
    });

    it('spans a recording that outlasted the last milestone', () => {
        expect(tickBounds([split(1, 'a', 100, 400)], [], {from: 100, to: 900}))
            .toEqual({from: 100, to: 900});
    });

    it('changes nothing for a run that recorded no video', () => {
        expect(tickBounds(splits, [], null)).toEqual(tickBounds(splits, []));
    });

    it('never cuts more than it keeps, even when the recording started late', () => {
        // The guard `axisFrom` applies has to apply to video too. Without it a
        // run whose recorder only came up near the end would throw the whole
        // extent the splits carry away and collapse to the tail.
        expect(tickBounds([split(1, 'a', 100, 400)], [], {from: 900, to: 1000}))
            .toEqual({from: 100, to: 1000});
        expect(leadInTicks([split(1, 'a', 100, 400)], [], {from: 900, to: 1000}))
            .toBe(0);
    });

    it('starts at the video when it precedes the first lane bar', () => {
        const lanes = [
            {bot: 1, id: 0, action: 'mine', from_tick: 1200, to_tick: 1800,
                status: 'success', error: null}
        ];
        expect(tickBounds(splits, lanes, {from: 300, to: 2000}))
            .toEqual({from: 300, to: 2000});
    });

    it('starts at the first lane bar when it precedes the video', () => {
        // Both are drawn, so the axis begins at whichever is drawn first.
        // Video does not get priority for being the visual record.
        const lanes = [
            {bot: 1, id: 0, action: 'mine', from_tick: 300, to_tick: 1800,
                status: 'success', error: null}
        ];
        expect(tickBounds(splits, lanes, {from: 1200, to: 2000}))
            .toEqual({from: 300, to: 2000});
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

describe('lanes', () => {
    const lane = (
        bot: number,
        id: number,
        action: string,
        from: number,
        to: number | null,
        status: string | null = 'success'
    ) => ({bot, id, action, from_tick: from, to_tick: to, status, error: null});

    it('lists the bots that did something', () => {
        expect(laneBots([lane(2, 0, 'a', 0, 1), lane(1, 0, 'b', 0, 1)])).toEqual([1, 2]);
    });

    it('finds what a bot was doing at a tick', () => {
        const lanes = [lane(1, 0, 'mine', 100, 500), lane(1, 1, 'craft', 500, 900)];
        expect(laneAt(lanes, 1, 200)?.action).toBe('mine');
        expect(laneAt(lanes, 1, 700)?.action).toBe('craft');
    });

    it('is null when the bot was between actions', () => {
        expect(laneAt([lane(1, 0, 'mine', 100, 200)], 1, 900)).toBeNull();
    });

    it('treats an unterminated span as still running', () => {
        // Showing the bot idle for exactly the stretch something went wrong
        // would hide the failure rather than reveal it.
        expect(laneAt([lane(1, 0, 'mine', 100, null, null)], 1, 99999)?.action).toBe('mine');
    });

    it('extends the axis to cover lanes', () => {
        const bounds = tickBounds([], [lane(1, 0, 'mine', 50, 800)]);
        expect(bounds).toEqual({from: 50, to: 800});
    });
});

describe('stuckReasons', () => {
    it('maps a stuck milestone to its refusal, skipping one with no error recorded', () => {
        const events: Event[] = [
            {kind: 'milestone_stuck', index: 1, outcome: 'stuck', best_steps: 3, last_error: 'no belt route', tick: 100},
            {kind: 'milestone_stuck', index: 2, outcome: 'stuck', best_steps: null, last_error: null, tick: 200}
        ];
        const reasons = stuckReasons(events);
        expect(reasons.size).toBe(1);
        expect(reasons.get(1)).toBe('no belt route');
        expect(reasons.has(2)).toBe(false);
    });
});

describe('run timestamps', () => {
    it('prefers the manifest start time', () => {
        expect(startedUnixOf({run_id: 'run-1000-5', started_unix: 4242})).toBe(4242);
    });

    it('falls back to the id for a run that never finished', () => {
        // The server reports `started_unix: null` on purpose there -- it does
        // not know -- but the list still has to show *when*, and the id
        // carries it. Same fallback the server orders by.
        expect(startedUnixOf({run_id: 'run-1788277287-11819', started_unix: null})).toBe(1788277287);
    });

    it('is null for an id that carries no timestamp', () => {
        expect(startedUnixOf({run_id: 'handwritten', started_unix: null})).toBeNull();
    });

    it('renders an em dash rather than an epoch date for an unknown time', () => {
        expect(formatWhen(null)).toBe('—');
    });

    it('renders distinct times distinctly', () => {
        const a = formatWhen(1788277287);
        const b = formatWhen(1788277287 + 86400 * 3);
        expect(a).not.toBe('');
        expect(a).not.toBe(b);
    });

    it('describes recency coarsely', () => {
        const now = 1788277287;
        expect(formatAgo(now - 10, now)).toBe('just now');
        expect(formatAgo(now - 720, now)).toBe('12 min ago');
        expect(formatAgo(now - 7200, now)).toBe('2 h ago');
        expect(formatAgo(now - 86400 * 2, now)).toBe('2 d ago');
        expect(formatAgo(null, now)).toBe('');
    });
});
