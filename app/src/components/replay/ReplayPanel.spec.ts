// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {nextTick} from 'vue';
import ReplayPanel from './ReplayPanel.vue';
import * as client from '@/api/client';
import * as jobEvents from '@/api/jobEvents';
import {Job} from '@/api/types';
import {REALISTIC_REPLAY_JSON} from '@/api/replay.fixtures';
import {useReplayStore} from '@/store/replayStore';

import '@/test/resizeObserverStub';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');

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

/**
 * `onMounted` fires `refresh()` without awaiting it, so a mount does not
 * settle until the mocked `listJobs()` promise, and the store write it
 * triggers, have both had a turn of the microtask queue.
 */
async function flush(): Promise<void> {
    await nextTick();
    await Promise.resolve();
    await Promise.resolve();
    await nextTick();
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    vi.mocked(jobEvents.subscribeJobEvents).mockImplementation(() => vi.fn());
    // `vi.mock('@/api/client')` auto-mocks every export, so `frames()` returns
    // `undefined` rather than a manifest and the store's `await` yields it.
    // Given a default here rather than in each test: a manifest is not what
    // any of these assert, and leaving it unset made the failure look like a
    // defect in the join logic instead of an unset mock.
    vi.mocked(client.frames).mockResolvedValue({clients: [], frames: [], run: null, client_runs: []});
});

afterEach(() => {
    // replayStore's stream reference is module-level, like scriptStore's;
    // leaving one attached would have the next test's first mount see a
    // `watchedJobId` left over from a job this test made up.
    useReplayStore().stopWatching();
});

describe('ReplayPanel', () => {
    it('shows the neutral empty state before any job has produced a replay', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([]);
        const wrapper = mount(ReplayPanel);
        await flush();

        expect(wrapper.find('[data-testid="empty-state"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="step-list"]').exists()).toBe(false);
    });

    it('renders the most recent job with a replay on mount', async () => {
        vi.mocked(client.listJobs).mockResolvedValue([job('1', REALISTIC_REPLAY_JSON)]);
        const wrapper = mount(ReplayPanel);
        await flush();

        expect(wrapper.find('[data-testid="step-list"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="empty-state"]').exists()).toBe(false);
    });

    it('unsubscribes the live stream on unmount', async () => {
        const unsubscribe = vi.fn();
        vi.mocked(jobEvents.subscribeJobEvents).mockImplementation(() => unsubscribe);
        vi.mocked(client.listJobs).mockResolvedValue([job('1', null, 'running')]);
        const wrapper = mount(ReplayPanel);
        await flush();

        wrapper.unmount();

        expect(unsubscribe).toHaveBeenCalledTimes(1);
    });
});
