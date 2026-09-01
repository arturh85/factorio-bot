import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useRunsStore} from './runsStore';
import * as client from '@/api/client';
import {ArchivedFrame, RunDetail, RunSummary, Split} from '@/api/types';

vi.mock('@/api/client');

const summary = (id: string, over: Partial<RunSummary> = {}): RunSummary => ({
    run_id: id,
    finished: true,
    started_unix: 1000,
    finished_unix: 1100,
    outcome: 'done',
    elapsed_ticks: 871,
    events: 6,
    frames: 15,
    splits: 2,
    ...over
});

const split = (index: number, goal: string, from: number, to: number | null): Split => ({
    index,
    goal,
    started_tick: from,
    ended_tick: to,
    outcome: to === null ? 'unfinished' : 'satisfied',
    elapsed_ticks: to === null ? null : to - from
});

const frame = (bot: number, tick: number, camera: string): ArchivedFrame => ({
    bot,
    tick,
    camera,
    file: `frames/${bot}/tick-${tick}-${camera}.jpg`
});

const DETAIL: RunDetail = {
    summary: summary('run-1'),
    splits: [split(1, 'iron', 59375, 59756), split(2, 'copper', 59756, 60246)]
};
const FRAMES = [frame(1, 59400, 'front'), frame(1, 59700, 'front'), frame(2, 59700, 'area')];

beforeEach(() => {
    setActivePinia(createPinia());
    vi.mocked(client.listRuns).mockReset();
    vi.mocked(client.getRun).mockReset();
    vi.mocked(client.getRunFrames).mockReset();
    vi.mocked(client.getRunLanes).mockReset();
    vi.mocked(client.getRunLanes).mockResolvedValue({lanes: []});
});

describe('loadRuns', () => {
    it('populates the list', async () => {
        vi.mocked(client.listRuns).mockResolvedValue({runs: [summary('run-1')]});
        const store = useRunsStore();
        await store.loadRuns();
        expect(store.runs.map((r) => r.run_id)).toEqual(['run-1']);
        expect(store.error).toBeNull();
    });

    it('reports a failure rather than leaving an empty list that looks like no runs', async () => {
        vi.mocked(client.listRuns).mockRejectedValue(new Error('boom'));
        const store = useRunsStore();
        await store.loadRuns();
        expect(store.error).toBe('boom');
    });
});

describe('openRun', () => {
    it('opens on the first frame, which is where the axis now starts', async () => {
        // Milestone 1 opened at 59375, 25 ticks before the first frame. The
        // axis trims that away, so the cursor and the axis start agree and
        // the panel has something in it on load.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.cursor).toBe(59400);
        expect(store.bounds?.from).toBe(59400);
        expect(store.leadIn).toBe(25);
    });

    it('parks the cursor at the start of the axis, which is the first frame', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        const store = useRunsStore();
        await store.openRun('run-1');

        expect(store.bounds).toEqual({from: 59400, to: 60246});
        expect(store.bot).toBe(1);
        expect(store.camera).toBe('front');
    });

    it('clears the previous run when loading fails', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        const store = useRunsStore();
        await store.openRun('run-1');

        vi.mocked(client.getRun).mockRejectedValue(new Error('gone'));
        await store.openRun('run-2');
        expect(store.detail).toBeNull();
        expect(store.frames).toEqual([]);
        expect(store.error).toBe('gone');
    });

    it('handles a run with no frames at all', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: []});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.bot).toBeNull();
        expect(store.camera).toBeNull();
        // Splits still place the axis: a planning-only run is still viewable,
        // and with nothing drawn there is no lead-in to trim.
        expect(store.bounds).toEqual({from: 59375, to: 60246});
        expect(store.leadIn).toBe(0);
    });
});

describe('setReference', () => {
    it('loads another run to diff against', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.setReference('run-2');
        expect(store.reference?.summary.run_id).toBe('run-1');
    });

    it('clears rather than keeping a stale reference when loading fails', async () => {
        // A previous reference left in place would keep rendering deltas that
        // silently describe the wrong run.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.setReference('run-2');
        vi.mocked(client.getRun).mockRejectedValue(new Error('gone'));
        await store.setReference('run-3');
        expect(store.reference).toBeNull();
    });

    it('clears on null', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.setReference('run-2');
        await store.setReference(null);
        expect(store.reference).toBeNull();
    });

    it('drops the comparison when a different run is opened', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        const store = useRunsStore();
        await store.setReference('run-2');
        await store.openRun('run-9');
        expect(store.reference).toBeNull();
    });
});

describe('selectView', () => {
    beforeEach(async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        await useRunsStore().openRun('run-1');
    });

    it('offers only pairs the run captured', () => {
        const store = useRunsStore();
        // The fixture has front@1 twice and area@2 once -- three frames, two
        // pairs. A cross product would offer four.
        expect(store.views.map((v) => `${v.camera}@${v.bot}`)).toEqual(['area@2', 'front@1']);
    });

    it('sets bot and camera together', () => {
        // Never one at a time: only specific pairs exist, so changing one and
        // leaving the other names a combination that captured nothing.
        const store = useRunsStore();
        store.selectView({bot: 2, camera: 'area', count: 1, from: 59700});
        expect(store.bot).toBe(2);
        expect(store.camera).toBe('area');
    });

    it('moves the cursor forward when the view starts later than it', () => {
        const store = useRunsStore();
        store.seek(59400);
        store.selectView({bot: 2, camera: 'area', count: 1, from: 59700});
        expect(store.cursor).toBe(59700);
    });

    it('leaves the cursor alone when the view already covers it', () => {
        const store = useRunsStore();
        store.seek(60000);
        store.selectView({bot: 1, camera: 'front', count: 2, from: 59400});
        expect(store.cursor).toBe(60000);
    });
});

describe('seek and playback', () => {
    beforeEach(async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunFrames).mockResolvedValue({frames: FRAMES});
        await useRunsStore().openRun('run-1');
    });

    it('clamps the cursor to the axis', () => {
        const store = useRunsStore();
        store.seek(0);
        expect(store.cursor).toBe(59400);
        store.seek(999999);
        expect(store.cursor).toBe(60246);
    });

    it('advances by the step rate', () => {
        const store = useRunsStore();
        store.rate = 300;
        store.advance();
        // From the first frame (59400), not the axis start.
        expect(store.cursor).toBe(59700);
    });

    it('stops at the end instead of wrapping', () => {
        // Wrapping would look like a run that happened twice.
        const store = useRunsStore();
        store.playing = true;
        store.seek(60246);
        store.advance();
        expect(store.cursor).toBe(60246);
        expect(store.playing).toBe(false);
    });

    it('restarts from the beginning when play is pressed at the end', () => {
        const store = useRunsStore();
        store.seek(60246);
        store.togglePlay();
        expect(store.playing).toBe(true);
        expect(store.cursor).toBe(59400);
    });

    it('does nothing on a run with no axis', () => {
        const store = useRunsStore();
        store.detail = null;
        store.frames = [];
        store.seek(500);
        expect(store.cursor).toBe(0);
        store.advance();
        store.togglePlay();
        expect(store.playing).toBe(false);
    });
});
