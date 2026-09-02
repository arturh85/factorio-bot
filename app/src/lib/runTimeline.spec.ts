import {describe, expect, it} from 'vitest';
import {
    botsOf,
    leadInTicks,
    formatAgo,
    formatWhen,
    startedUnixOf,
    viewsOf,
    laneAt,
    laneBots,
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

    it('starts at the first frame, not at the milestone that opened before it', () => {
        // The run began, the planner thought, and capture produced nothing for
        // 200 ticks. Spanning that spends axis width on a stretch with no
        // frame and no lane bar.
        const bounds = tickBounds(
            [split(1, 'a', 100, 2000)],
            placeable([frame(1, 300, 'front'), frame(1, 2000, 'front')])
        );
        expect(bounds).toEqual({from: 300, to: 2000});
        expect(leadInTicks([split(1, 'a', 100, 2000)], placeable([frame(1, 300, 'front'), frame(1, 2000, 'front')]))).toBe(200);
    });

    it('starts at the first lane bar when the bots moved before capture did', () => {
        const bounds = tickBounds(
            [split(1, 'a', 100, 2000)],
            placeable([frame(1, 400, 'front'), frame(1, 2000, 'front')]),
            [{bot: 1, id: 0, action: 'mine', from_tick: 250, to_tick: 900, status: 'success', error: null}]
        );
        expect(bounds?.from).toBe(250);
    });

    it('refuses a trim that would cut more than it keeps', () => {
        // The one frame landed at the very end. Trimming to it would throw
        // away the extent the splits carry and collapse the axis.
        const bounds = tickBounds([split(1, 'a', 100, 400)], placeable([frame(1, 900, 'front')]));
        expect(bounds).toEqual({from: 100, to: 900});
        expect(leadInTicks([split(1, 'a', 100, 400)], placeable([frame(1, 900, 'front')]))).toBe(0);
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
 * The silent regression this pins. Frame ticks have always decided where the
 * axis starts, and nothing errors or is marked when that contributor goes
 * missing -- the axis is just different, computed from splits and lanes alone.
 * A run that recorded video instead of frames has to land somewhere honest.
 */
describe('a run whose only capture is video', () => {
    const splits = [split(1, 'a', 100, 2000)];

    it('starts at the video clock when there are no frames at all', () => {
        // Without the video range this answers {from: 100}, silently, because
        // `drawn` would be empty and the axis would fall back to the splits.
        expect(tickBounds(splits, [], [], {from: 300, to: 2000})).toEqual({from: 300, to: 2000});
        expect(leadInTicks(splits, [], [], {from: 300, to: 2000})).toBe(200);
    });

    it('spans a recording that outlasted the last milestone', () => {
        expect(tickBounds([split(1, 'a', 100, 400)], [], [], {from: 100, to: 900}))
            .toEqual({from: 100, to: 900});
    });

    it('leaves the axis exactly where the frames put it when a run has both', () => {
        // Video is the opt-in second artefact and must not move an axis that
        // frames already decide -- two runs' axes have to keep lining up.
        const frames = placeable([frame(1, 300, 'front'), frame(1, 2000, 'front')]);
        const withoutVideo = tickBounds(splits, frames);
        expect(tickBounds(splits, frames, [], {from: 120, to: 2400})).toEqual(withoutVideo);
        expect(leadInTicks(splits, frames, [], {from: 120, to: 2400}))
            .toBe(leadInTicks(splits, frames));
    });

    it('changes nothing for a run that recorded no video', () => {
        expect(tickBounds(splits, [], [], null)).toEqual(tickBounds(splits, []));
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
        const bounds = tickBounds([], [], [lane(1, 0, 'mine', 50, 800)]);
        expect(bounds).toEqual({from: 50, to: 800});
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

describe('viewsOf', () => {
    it('offers only the pairs that exist, never the cross product', () => {
        // The real shape: camera bot-3 lives in client 1's folder, and area /
        // bot-1 both live in client 2's. Two free dropdowns would offer bot 1 +
        // area, which captured nothing and never could.
        const views = viewsOf(
            placeable([
                frame(1, 300, 'bot-3'),
                frame(1, 600, 'bot-3'),
                frame(2, 300, 'area'),
                frame(2, 300, 'bot-1')
            ])
        );
        expect(views.map((v) => `${v.camera}@${v.bot}`)).toEqual([
            'area@2',
            'bot-1@2',
            'bot-3@1'
        ]);
        expect(views.find((v) => v.camera === 'bot-3')?.count).toBe(2);
        expect(views.find((v) => v.camera === 'bot-3')?.from).toBe(300);
    });

    it('is empty for a run that captured nothing', () => {
        expect(viewsOf([])).toEqual([]);
    });
});
