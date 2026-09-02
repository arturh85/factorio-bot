// @vitest-environment node
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useReplayStore} from './replayStore';
import * as client from '@/api/client';
import * as jobEvents from '@/api/jobEvents';
import {JobEventHandlers} from '@/api/jobEvents';
import {Job} from '@/api/types';
import {REALISTIC_REPLAY_JSON} from '@/api/replay.fixtures';
import {CLEAN_MANIFEST, CLEAN_TICKS} from '@/api/video.fixtures';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');

/** One call to `subscribeJobEvents`, with its own unsubscribe spy. */
interface Subscription {
    jobId: string;
    handlers: JobEventHandlers;
    unsubscribe: ReturnType<typeof vi.fn>;
}

let subscriptions: Subscription[] = [];

function subscription(index = subscriptions.length - 1): Subscription {
    if (subscriptions.length === 0) {
        throw new Error('nothing subscribed to the job event stream');
    }
    return subscriptions[index];
}

function job(id: string, replay: string | null, status: Job['status'] = 'succeeded'): Job {
    return {
        id,
        script: null,
        status,
        started_at_ms: Number(id) * 1000,
        finished_at_ms: status === 'running' ? null : Number(id) * 1000 + 1,
        stdout: '',
        stderr: '',
        error: null,
        replay
    };
}

beforeEach(() => {
    setActivePinia(createPinia());
    subscriptions = [];
    vi.mocked(jobEvents.subscribeJobEvents).mockImplementation(
        (jobId: string, handlers: JobEventHandlers) => {
            const unsubscribe = vi.fn();
            subscriptions.push({jobId, handlers, unsubscribe});
            return unsubscribe;
        }
    );
});

afterEach(() => {
    // The store's stream reference is module-level, like scriptStore's;
    // leaving one attached would have the next test's first refresh see a
    // `watchedJobId` from a job this test made up.
    useReplayStore().stopWatching();
});

describe('useReplayStore.refresh', () => {
    it('leaves the store empty when there are no jobs at all', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([]);
        const store = useReplayStore();

        await store.refresh();

        expect(store.getReplay).toBeNull();
        expect(store.getJobId).toBeNull();
        expect(subscriptions).toHaveLength(0);
    });

    it('displays the most recent job that already has a replay, not the overall most recent job', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([
            job('1', null),
            job('2', REALISTIC_REPLAY_JSON),
            job('3', null, 'running')
        ]);
        const store = useReplayStore();

        await store.refresh();

        expect(store.getJobId).toBe('2');
        expect(store.getReplay?.planned_makespan).toBe(250);
    });

    it('watches the overall most recent job live, even though it has no replay yet', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([
            job('1', null),
            job('2', REALISTIC_REPLAY_JSON),
            job('3', null, 'running')
        ]);
        const store = useReplayStore();

        await store.refresh();

        expect(subscriptions).toHaveLength(1);
        expect(subscription().jobId).toBe('3');
    });

    it('does not resubscribe on a second refresh when the latest job is unchanged', async () => {
        const jobs = [job('1', null), job('2', REALISTIC_REPLAY_JSON), job('3', null, 'running')];
        vi.mocked(client.listJobs).mockResolvedValue(jobs);
        const store = useReplayStore();

        await store.refresh();
        await store.refresh();

        expect(subscriptions).toHaveLength(1);
    });

    it('a live replay event for the watched job updates the display', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([
            job('1', null),
            job('2', REALISTIC_REPLAY_JSON),
            job('3', null, 'running')
        ]);
        const store = useReplayStore();
        await store.refresh();

        subscription().handlers.onReplay?.(REALISTIC_REPLAY_JSON);

        expect(store.getJobId).toBe('3');
        expect(store.getReplay?.planned_makespan).toBe(250);
    });

    it('does not let a later refresh downgrade the display to an older job', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([
            job('1', null),
            job('2', REALISTIC_REPLAY_JSON),
            job('3', null, 'running')
        ]);
        const store = useReplayStore();
        await store.refresh();
        subscription().handlers.onReplay?.(REALISTIC_REPLAY_JSON);
        expect(store.getJobId).toBe('3');

        // Job 3 still shows no replay in a re-listing (it has not finished
        // producing one yet as far as the server is concerned), and job 2 is
        // strictly older than what is already on screen.
        await store.refresh();

        expect(store.getJobId).toBe('3');
    });

    it('subscribes to a new job once it overtakes the previously watched one', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([job('1', null, 'running')]);
        const store = useReplayStore();
        await store.refresh();
        expect(subscription().jobId).toBe('1');

        vi.mocked(client.listJobs).mockResolvedValue([
            job('1', REALISTIC_REPLAY_JSON),
            job('2', null, 'running')
        ]);
        await store.refresh();

        expect(subscriptions).toHaveLength(2);
        expect(subscription().jobId).toBe('2');
        expect(store.getJobId).toBe('1');
    });
});

describe('useReplayStore live parsing', () => {
    it('records a parse error instead of throwing on a malformed live replay', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([job('1', null, 'running')]);
        const store = useReplayStore();
        await store.refresh();

        subscription().handlers.onReplay?.('{not json');

        expect(store.getParseError).not.toBeNull();
        expect(store.getReplay).toBeNull();
    });
});

describe('useReplayStore.stopWatching', () => {
    it('unsubscribes the live stream', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([job('1', null, 'running')]);
        const store = useReplayStore();
        await store.refresh();
        const sub = subscription();

        store.stopWatching();

        expect(sub.unsubscribe).toHaveBeenCalledTimes(1);
    });

    it('is safe to call when nothing was ever watched', () => {
        const store = useReplayStore();
        expect(() => store.stopWatching()).not.toThrow();
    });
});


describe('useReplayStore -- the live recording', () => {
    it('fetches the manifest and clock beside the replay', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([]);
        vi.mocked(client.video).mockResolvedValue(CLEAN_MANIFEST);
        vi.mocked(client.videoTicks).mockResolvedValue(CLEAN_TICKS);
        const store = useReplayStore();

        await store.refresh();

        expect(store.getVideo?.run).toBe('run-1');
        expect(store.getVideoTicks?.samples).toHaveLength(CLEAN_TICKS.samples.length);
    });

    it('keeps the replay when a server too old for the video routes 404s', async () => {
        // Video is opt-in in the first place; losing the timeline over it would
        // be the worst possible trade.
        vi.mocked(client.listJobs).mockResolvedValue([job('1', REALISTIC_REPLAY_JSON)]);
        vi.mocked(client.video).mockRejectedValue(new Error('404'));
        vi.mocked(client.videoTicks).mockRejectedValue(new Error('404'));
        const store = useReplayStore();

        await store.refresh();

        expect(store.getReplay).not.toBeNull();
        expect(store.getVideo).toBeNull();
        expect(store.getVideoTicks).toBeNull();
    });
});
