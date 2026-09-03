import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useRunsStore} from './runsStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';
import {EntitySnapshot, Lane, MapRecord, RunDetail, RunSummary, Sample, Split} from '@/api/types';
import {CLEAN_MANIFEST, CLEAN_TICKS, NO_TICKS, NO_VIDEO_MANIFEST} from '@/api/video.fixtures';

vi.mock('@/api/client');

const summary = (id: string, over: Partial<RunSummary> = {}): RunSummary => ({
    run_id: id,
    finished: true,
    started_unix: 1000,
    finished_unix: 1100,
    outcome: 'done',
    elapsed_ticks: 871,
    events: 6,
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

/**
 * A lane bar starting 25 ticks after milestone 1 opened.
 *
 * These fixtures used to be screenshot frames, which decided where the axis
 * started until the cameras were retired (2026-09-02). A lane bar is drawn on
 * the same terms, so the trim these tests pin is unchanged -- only the
 * contributor that triggers it.
 */
const lane = (bot: number, from: number, to: number | null): Lane => ({
    bot,
    id: from,
    action: 'mine',
    from_tick: from,
    to_tick: to,
    status: to === null ? null : 'success',
    error: null
});

const DETAIL: RunDetail = {
    summary: summary('run-1'),
    splits: [split(1, 'iron', 59375, 59756), split(2, 'copper', 59756, 60246)]
};
const LANES = [lane(1, 59400, 59700), lane(2, 59700, 60000)];

beforeEach(() => {
    setActivePinia(createPinia());
    vi.mocked(client.listRuns).mockReset();
    vi.mocked(client.getRun).mockReset();
    vi.mocked(client.getRunLanes).mockReset();
    vi.mocked(client.getRunLanes).mockResolvedValue({lanes: []});
    vi.mocked(client.getRunSamples).mockReset();
    vi.mocked(client.getRunSamples).mockResolvedValue({samples: [], skipped: 0});
    vi.mocked(client.getRunMap).mockReset();
    vi.mocked(client.getRunMap).mockResolvedValue({map: [], skipped: 0});
    vi.mocked(client.getRunVideo).mockReset();
    vi.mocked(client.getRunVideo).mockResolvedValue(NO_VIDEO_MANIFEST);
    vi.mocked(client.getRunVideoTicks).mockReset();
    vi.mocked(client.getRunVideoTicks).mockResolvedValue(NO_TICKS);
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
    it('opens where the axis starts, which is the first thing drawn', async () => {
        // Milestone 1 opened at 59375, 25 ticks before the first lane bar. The
        // axis trims that away, so the cursor and the axis start agree and
        // the panels have something in them on load.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.cursor).toBe(59400);
        expect(store.bounds?.from).toBe(59400);
        expect(store.leadIn).toBe(25);
    });

    it('parks the cursor at the start of the axis', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        const store = useRunsStore();
        await store.openRun('run-1');

        expect(store.bounds).toEqual({from: 59400, to: 60246});
        expect(store.cursor).toBe(59400);
    });

    it('clears the previous run when loading fails', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        const store = useRunsStore();
        await store.openRun('run-1');

        vi.mocked(client.getRun).mockRejectedValue(new Error('gone'));
        await store.openRun('run-2');
        expect(store.detail).toBeNull();
        expect(store.lanes).toEqual([]);
        expect(store.error).toBe('gone');
    });

    it('handles a run with nothing drawn at all', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.bot).toBeNull();
        // Splits still place the axis: a planning-only run is still viewable,
        // and with nothing drawn there is no lead-in to trim.
        expect(store.bounds).toEqual({from: 59375, to: 60246});
        expect(store.leadIn).toBe(0);
    });

    it('still opens the run when one enrichment 404s, degrading only that stream', async () => {
        // This is the bug that made every run on an older server read as
        // "not found": /samples and /map did not exist yet, and folding
        // their failure into the run's own error hid the whole page behind
        // one missing route.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        vi.mocked(client.getRunSamples).mockRejectedValue(
            new ApiError(404, '404 Not Found', null, null)
        );
        const store = useRunsStore();
        await store.openRun('run-1');

        // The run itself opened fine: detail, lanes and the axis are intact.
        expect(store.error).toBeNull();
        expect(store.detail?.summary.run_id).toBe('run-1');
        expect(store.lanes).toEqual(LANES);
        expect(store.bounds).toEqual({from: 59400, to: 60246});

        // Only the failed stream is empty and flagged, distinctly from a run
        // that simply recorded no samples.
        expect(store.samples).toEqual([]);
        expect(store.sampleError).not.toBeNull();
    });

    it('attributes an enrichment failure to the stream that actually failed', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        vi.mocked(client.getRunSamples).mockRejectedValue(
            new ApiError(404, '404 Not Found', null, null)
        );
        vi.mocked(client.getRunMap).mockRejectedValue(new Error('network down'));
        const store = useRunsStore();
        await store.openRun('run-1');

        // Named for its own route, not a generic "not found" -- a 404 says
        // the server predates the route rather than the run being missing.
        expect(store.sampleError).toContain('world-state samples');
        expect(store.sampleError).toContain('/samples');
        expect(store.sampleError).toContain('this server does not provide');

        // A different failure on a different stream gets its own message and
        // does not bleed into the samples panel's.
        expect(store.mapError).toContain('entity map');
        expect(store.mapError).toContain('network down');
        expect(store.mapError).not.toBe(store.sampleError);

        // Streams that succeeded are untouched.
        expect(store.lanesError).toBeNull();
        expect(store.videoError).toBeNull();
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
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        const store = useRunsStore();
        await store.setReference('run-2');
        await store.openRun('run-9');
        expect(store.reference).toBeNull();
    });
});

describe('selectBot', () => {
    beforeEach(async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [{
                kind: 'bots',
                bots: [2, 1].map((id) => ({
                    id, position: {x: 0, y: 0}, inventory: {}, crafting_queue: 0, mining: null
                })),
                schema: 1,
                tick: 59400,
                run: 'run-1'
            } as Sample],
            skipped: 0
        });
        await useRunsStore().openRun('run-1');
    });

    it('offers every bot the run sampled, ascending', () => {
        // The samples are the only record of which bots a run had -- picking
        // the camera used to be what named one, and nothing else does.
        expect(useRunsStore().bots).toEqual([1, 2]);
    });

    it('opens on the lowest sampled bot rather than none at all', () => {
        expect(useRunsStore().bot).toBe(1);
    });

    it('points the inventory panel at another bot', () => {
        const store = useRunsStore();
        store.selectBot(2);
        expect(store.bot).toBe(2);
    });
});

describe('seek and playback', () => {
    beforeEach(async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
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
        store.lanes = [];
        store.seek(500);
        expect(store.cursor).toBe(0);
        store.advance();
        store.togglePlay();
        expect(store.playing).toBe(false);
    });
});

describe('samples', () => {
    const bots = (tick: number, entries: Array<{id: number; inventory?: Record<string, number>}>): Sample => ({
        kind: 'bots',
        bots: entries.map((e) => ({
            id: e.id,
            position: {x: 0, y: 0},
            inventory: e.inventory ?? {},
            crafting_queue: 0,
            mining: null
        })),
        schema: 1,
        tick,
        run: 'run-1'
    });

    const force = (
        tick: number,
        made: Record<string, number>,
        research: {name: string; progress: number; eta_ticks: number | null} | null = null
    ): Sample => ({
        kind: 'force',
        research,
        techs_unlocked: 0,
        production: {made, consumed: {}},
        power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1, networks: {}},
        schema: 1,
        tick,
        run: 'run-1'
    });

    beforeEach(() => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
    });

    it('reads the selected bot state at the cursor', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [bots(59380, [{id: 1, inventory: {'iron-plate': 2}}, {id: 2}])], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1'); // cursor lands on 59400, where the axis starts
        expect(store.botState?.inventory).toEqual({'iron-plate': 2});
    });

    it('is null before the first bots sample', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [bots(60000, [{id: 1}])], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.botState).toBeNull();
    });

    it('is null when the run sampled no bot to select', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.bot).toBeNull();
        expect(store.botState).toBeNull();
    });

    it('reports queued research', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [force(59380, {}, {name: 'automation', progress: 0.5, eta_ticks: 100})], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.forceState?.research?.name).toBe('automation');
    });

    it('distinguishes no research queued from no sample yet', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [force(59380, {}, null)], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.forceState).not.toBeNull();
        expect(store.forceState?.research).toBeNull();

        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [force(60000, {}, null)], skipped: 0});
        await store.openRun('run-1');
        expect(store.forceState).toBeNull();
    });

    it('reports cumulative production at the cursor, falling back to produced items when the goals name none', async () => {
        // DETAIL's goals are 'iron' and 'copper', neither of which parses as
        // a have/produce goal, so the fallback to produced items applies.
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [force(59380, {'iron-plate': 4}), force(59700, {'iron-plate': 9})], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1'); // cursor = 59400
        expect(store.production).toEqual([{item: 'iron-plate', made: 4}]);
        store.seek(59700);
        expect(store.production).toEqual([{item: 'iron-plate', made: 9}]);
    });

    it('clears samples when a run fails to load', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [force(59380, {'iron-plate': 4})], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        vi.mocked(client.getRun).mockRejectedValue(new Error('gone'));
        await store.openRun('run-2');
        expect(store.samples).toEqual([]);
    });
});

describe('map', () => {
    const snap = (name: string, x: number, y: number): EntitySnapshot => ({
        name,
        position: {x, y},
        direction: 0
    });

    const placed = (tick: number, name: string, x: number, y: number): MapRecord => {
        const entity = snap(name, x, y);
        return {kind: 'placed', tick, bot: 1, intent: entity, actual: entity, drift: null};
    };

    const keyframe = (tick: number, bounds = {left: -8, top: -8, right: 8, bottom: 8}): MapRecord => ({
        kind: 'keyframe',
        tick,
        bounds,
        game: [],
        model: [],
        divergence: []
    });

    const bots = (tick: number, entries: Array<{id: number; position: {x: number; y: number}}>): Sample => ({
        kind: 'bots',
        bots: entries.map((e) => ({
            id: e.id,
            position: e.position,
            inventory: {},
            crafting_queue: 0,
            mining: null
        })),
        schema: 1,
        tick,
        run: 'run-1'
    });

    beforeEach(() => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
    });

    it('reconstructs the entities at the cursor from the archived map', async () => {
        vi.mocked(client.getRunMap).mockResolvedValue({
            map: [placed(59380, 'stone-furnace', -12, 8)], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1'); // cursor lands on 59400, where the axis starts
        expect(store.entities).toEqual([snap('stone-furnace', -12, 8)]);
    });

    it('reports the latest keyframe bounds at the cursor, or null before one exists', async () => {
        vi.mocked(client.getRunMap).mockResolvedValue({map: [], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.mapBounds).toBeNull();

        vi.mocked(client.getRunMap).mockResolvedValue({map: [keyframe(59380)], skipped: 0});
        await store.openRun('run-1');
        expect(store.mapBounds).toEqual({left: -8, top: -8, right: 8, bottom: 8});
    });

    it('reports every bot from the latest bots sample at the cursor', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [bots(59380, [{id: 1, position: {x: 0, y: 0}}, {id: 2, position: {x: 5, y: 5}}])], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.mapBots.map((b) => b.id)).toEqual([1, 2]);
    });

    it('is empty before the first bots sample', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [bots(60000, [{id: 1, position: {x: 0, y: 0}}])], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.mapBots).toEqual([]);
    });

    it('builds a trail of one bot\'s positions over the last 1,800 ticks up to the cursor', async () => {
        vi.mocked(client.getRunSamples).mockResolvedValue({
            samples: [
                bots(59380, [{id: 1, position: {x: 0, y: 0}}]),
                bots(59700, [{id: 1, position: {x: 1, y: 0}}])
            ], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1'); // cursor = 59400
        expect(store.trail).toEqual({1: [{x: 0, y: 0}]});
        store.seek(59700);
        // Both samples are well within 1,800 ticks of 59700, so both appear.
        expect(store.trail).toEqual({1: [{x: 0, y: 0}, {x: 1, y: 0}]});
    });

    it('clears the map when a run fails to load', async () => {
        vi.mocked(client.getRunMap).mockResolvedValue({map: [placed(59380, 'stone-furnace', -12, 8)], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        vi.mocked(client.getRun).mockRejectedValue(new Error('gone'));
        await store.openRun('run-2');
        expect(store.entities).toEqual([]);
        expect(store.mapBounds).toBeNull();
    });
});


describe('the video enrichment', () => {
    it('loads the recording and its clock beside the other enrichments', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunVideo).mockResolvedValue(CLEAN_MANIFEST);
        vi.mocked(client.getRunVideoTicks).mockResolvedValue(CLEAN_TICKS);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.video?.run).toBe('run-1');
        expect(store.videoTicks?.samples).toHaveLength(CLEAN_TICKS.samples.length);
        expect(store.videoError).toBeNull();
    });

    it('does not lose the run when a server too old for the video routes 404s', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: LANES});
        vi.mocked(client.getRunVideo).mockRejectedValue(new ApiError(404, '404 Not Found', null, null));
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.error).toBeNull();
        expect(store.lanes).toHaveLength(2);
        expect(store.video).toBeNull();
        expect(store.videoError).not.toBeNull();
    });

    it('keeps neither half when only the clock fails', async () => {
        // A manifest without its clock can place nothing, and showing half a
        // recording is worse than showing none.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunVideo).mockResolvedValue(CLEAN_MANIFEST);
        vi.mocked(client.getRunVideoTicks).mockRejectedValue(new Error('boom'));
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.video).toBeNull();
        expect(store.videoTicks).toBeNull();
        expect(store.videoError).not.toBeNull();
    });

    it('sizes the axis from the recording when nothing else is drawn', async () => {
        // The silent one: without the video range this axis would be computed
        // from splits and lanes alone, with nothing erroring and nothing marked.
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunVideo).mockResolvedValue({
            ...CLEAN_MANIFEST,
            tick_range: {from: 59400, to: 60246}
        });
        vi.mocked(client.getRunVideoTicks).mockResolvedValue(CLEAN_TICKS);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.bounds).toEqual({from: 59400, to: 60246});
        expect(store.leadIn).toBe(25);
    });
});
