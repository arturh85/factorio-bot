import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useRunsStore} from './runsStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';
import {EntitySnapshot, Event, FlowExport, Lane, MapRecord, RunDetail, RunSummary, Sample, Split} from '@/api/types';
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
    samples: null,
    map: null,
    samples_lag_ticks: null,
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
    vi.mocked(client.getRunEvents).mockReset();
    vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 0});
    vi.mocked(client.getRunProvenance).mockReset();
    vi.mocked(client.getRunProvenance).mockRejectedValue(new ApiError(404, 'not_found', null, ''));
    vi.mocked(client.getRunReplay).mockReset();
    vi.mocked(client.getRunReplay).mockRejectedValue(new ApiError(404, 'not_found', null, ''));
    vi.mocked(client.getRunSavepoints).mockReset();
    vi.mocked(client.getRunSavepoints).mockRejectedValue(new ApiError(404, 'not_found', null, ''));
    vi.mocked(client.getRunFlow).mockReset();
    vi.mocked(client.getRunFlow).mockRejectedValue(new ApiError(404, 'not_found', null, ''));
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
        pollution: null,
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

describe('events enrichment', () => {
    beforeEach(() => {
        setActivePinia(createPinia());
        vi.mocked(client.getRun).mockResolvedValue({summary: summary('run-1'), splits: []} as RunDetail);
        vi.mocked(client.getRunLanes).mockResolvedValue({lanes: []});
        vi.mocked(client.getRunSamples).mockResolvedValue({samples: [], skipped: 0});
        vi.mocked(client.getRunMap).mockResolvedValue({map: [], skipped: 0});
        vi.mocked(client.getRunVideo).mockResolvedValue(NO_VIDEO_MANIFEST);
        vi.mocked(client.getRunVideoTicks).mockResolvedValue(NO_TICKS);
    });
    it('derives the analysis window from run_started and run_finished', async () => {
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [
            {kind: 'run_started', tick: 3242, run_id: 'run-1', bots: [1], seed: null, factorio: null, git: null},
            {kind: 'run_finished', tick: 25224, outcome: 'done', elapsed_ticks: 21982}
        ] as Event[], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.window).toEqual({lo: 3242, hi: 25224});
        expect(store.eventsError).toBeNull();
    });
    it('falls back to the last event when the run never finished, and to null with no events', async () => {
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [
            {kind: 'run_started', tick: 10, run_id: 'run-1', bots: [1], seed: null, factorio: null, git: null},
            {kind: 'plan_created', tick: 50, milestone_index: 1, steps: 0, makespan: 0, bots: null, plan: []}
        ] as Event[], skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.window).toEqual({lo: 10, hi: 50});
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 0});
        await store.openRun('run-1');
        expect(store.window).toBeNull();
    });
    it('records an events failure without failing the run', async () => {
        vi.mocked(client.getRunEvents).mockRejectedValue(new ApiError(404, 'not_found', null, ''));
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.detail).not.toBeNull();
        expect(store.eventsError).toContain('/events');
        // No answer is not "none skipped": the count belongs to a reply that
        // never came.
        expect(store.eventsSkipped).toBe(0);
    });
    it('keeps the count of unreadable event lines instead of dropping it', async () => {
        // The coverage band's job is to say what the record does NOT have,
        // and a line nobody could parse -- the torn last line of a killed run
        // -- is exactly that. The store had the number and threw it away.
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 3});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.eventsSkipped).toBe(3);
        vi.mocked(client.getRunEvents).mockResolvedValue({events: [], skipped: 0});
        await store.openRun('run-1');
        expect(store.eventsSkipped).toBe(0);
    });
});

describe('phase 2 enrichments', () => {
    it('a 404 on provenance is "not captured", not an error for the run', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.detail).not.toBeNull();
        expect(store.provenance).toBeNull();
        expect(store.provenanceError).toContain('/provenance');
    });

    it('keeps a served provenance and the replay counts', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        vi.mocked(client.getRunProvenance).mockResolvedValue({
            schema: 1,
            run_id: 'run-1',
            started_unix: 1,
            started_tick: 0,
            seed: '31337',
            map_exchange_string: null,
            map: null,
            factorio: '2.1.17',
            mods: {base: '2.1.17'},
            git: {commit: 'abc', dirty: false, source: 'working-tree-at-run-start'},
            profile: 'release',
            roster_requested: [1, 2],
            workspace: null,
            resumed_from: null,
            bot_mode: 'clients',
            game_speed: 1,
            peaceful: null
        });
        vi.mocked(client.getRunReplay).mockResolvedValue({
            planned_makespan: 10,
            refused: null,
            unmatched_walks: [],
            steps: [
                {
                    index: 0, bot: 1, bot_step_index: 0,
                    what: {kind: 'act', action: 1, label: 'craft 1 pipe'},
                    planned_start_tick: 0, planned_end_tick: 5,
                    observed_start_tick: 0, observed_end_tick: 6,
                    status: 'Success', attempt_number: 1,
                    evidence: {kind: 'measured'}, error: null
                },
                {
                    index: 1, bot: 1, bot_step_index: 1,
                    what: {kind: 'walk', to: {x: 1, y: 2}},
                    planned_start_tick: 5, planned_end_tick: 9,
                    observed_start_tick: 6, observed_end_tick: 9,
                    status: 'Success', attempt_number: 1,
                    evidence: {kind: 'believed', why: 'ticks measured, arrival not'}, error: null
                },
                {
                    index: 2, bot: 1, bot_step_index: 2,
                    what: {kind: 'act', action: 2, label: 'place x'},
                    planned_start_tick: 9, planned_end_tick: 10,
                    observed_start_tick: null, observed_end_tick: null,
                    status: 'Abandoned', attempt_number: 0,
                    evidence: {kind: 'measured'}, error: 'abandoned: predecessor 1 failed'
                }
            ]
        });
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.provenance?.seed).toBe('31337');
        expect(store.replayCounts).toEqual({steps: 3, abandoned: 1, lost: 0, failed: 0, pending: 0, believed: 1});
    });

    it('a 404 on flow is "not captured", not an error for the run', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.detail).not.toBeNull();
        expect(store.flow).toEqual([]);
        expect(store.flowError).toContain('/flow');
    });

    it('keeps a served flow', async () => {
        vi.mocked(client.getRun).mockResolvedValue(DETAIL);
        const FLOW: FlowExport[] = [{tick: 59400, nodes: [], edges: []}];
        vi.mocked(client.getRunFlow).mockResolvedValue({flow: FLOW, skipped: 0});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.flow).toEqual(FLOW);
        expect(store.flowError).toBeNull();
    });

    it('prefers the manifest lag over the derived one', async () => {
        vi.mocked(client.getRun).mockResolvedValue({
            summary: summary('run-1', {samples: 10, map: 2, samples_lag_ticks: 42}),
            splits: []
        } as RunDetail);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.sampleLag).toBe(42);
    });
});
