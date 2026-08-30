import {beforeEach, describe, expect, it, vi} from 'vitest';
import {EventSourceLike, subscribeJobEvents} from './jobEvents';
import * as client from './client';
import {Job, JobStatus} from './types';

/**
 * `jobEventsUrl` is mocked to a shape the module could not have assembled by
 * accident: a bare path would be indistinguishable from one built inline, so
 * asserting on this URL is what proves the subscriber goes through the client
 * rather than spelling the route out a second time.
 */
vi.mock('./client', () => ({
    jobEventsUrl: (id: string) => 'http://stream.invalid/of/' + id,
    getJob: vi.fn()
}));

/**
 * A stand-in for the browser's `EventSource`.
 *
 * `emit` returns how many listeners it reached. Every "nothing was reported"
 * assertion below pairs with that count: without it, a test that stopped
 * registering listeners at all would pass for the wrong reason.
 */
class FakeEventSource implements EventSourceLike {
    listeners = new Map<string, Array<(event: MessageEvent) => void>>();
    closed = false;

    addEventListener(type: string, listener: (event: MessageEvent) => void) {
        const existing = this.listeners.get(type) ?? [];
        existing.push(listener);
        this.listeners.set(type, existing);
    }

    close() {
        this.closed = true;
    }

    /** Delivers `data` verbatim, so a test can send something that is not JSON. */
    emitRaw(type: string, data: string): number {
        const listeners = this.listeners.get(type) ?? [];
        for (const listener of listeners) {
            listener({data} as MessageEvent);
        }
        return listeners.length;
    }

    emit(type: string, data?: unknown): number {
        return this.emitRaw(type, data === undefined ? '' : JSON.stringify(data));
    }
}

function handlers() {
    return {
        onOutput: vi.fn(),
        onLagged: vi.fn(),
        onFinished: vi.fn(),
        onError: vi.fn()
    };
}

/**
 * Every field differs from what the assertions expect unless a test overrides
 * it: `status` here is `running`, which is the one status that must *not*
 * reach `onFinished`.
 */
function job(overrides: Partial<Job> = {}): Job {
    return {
        id: 'job-91',
        script: '/deep/thought.lua',
        status: 'running',
        started_at_ms: 1717171717000,
        finished_at_ms: null,
        stdout: 'buffered out',
        stderr: 'buffered err',
        error: null,
        ...overrides
    };
}

/** Lets the pending `getJob` promise and its handlers run to completion. */
function settle(): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, 0));
}

const finishedStatuses: JobStatus[] = ['succeeded', 'failed'];

let source: FakeEventSource;

beforeEach(() => {
    vi.resetAllMocks();
    source = new FakeEventSource();
});

describe('subscribeJobEvents', () => {
    it('opens the stream at the url the client builds for the job', () => {
        const createSource = vi.fn(() => source);
        subscribeJobEvents('job-91', handlers(), createSource);
        expect(createSource).toHaveBeenCalledWith('http://stream.invalid/of/job-91');
    });

    it('falls back to the browser EventSource when no factory is given', () => {
        const constructed: string[] = [];
        class StubEventSource {
            constructor(url: string) {
                constructed.push(url);
            }
            addEventListener() {}
            close() {}
        }
        vi.stubGlobal('EventSource', StubEventSource);
        try {
            subscribeJobEvents('job-91', handlers());
        } finally {
            vi.unstubAllGlobals();
        }
        expect(constructed).toEqual(['http://stream.invalid/of/job-91']);
    });

    it('reports each output line with its own stream', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        source.emit('output', {stream: 'stdout', text: 'ordinary line'});
        source.emit('output', {stream: 'stderr', text: 'complaint'});
        expect(h.onOutput).toHaveBeenNthCalledWith(1, 'stdout', 'ordinary line');
        expect(h.onOutput).toHaveBeenNthCalledWith(2, 'stderr', 'complaint');
    });

    it('keeps an empty output line rather than dropping it', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        source.emit('output', {stream: 'stdout', text: ''});
        expect(h.onOutput).toHaveBeenCalledWith('stdout', '');
    });

    it('ignores an output event whose text is not a string', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        expect(source.emit('output', {stream: 'stdout', text: 17})).toBeGreaterThan(0);
        expect(h.onOutput).not.toHaveBeenCalled();
        expect(h.onError).not.toHaveBeenCalled();
    });

    it('survives a malformed data payload', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        expect(source.emitRaw('output', 'not json')).toBeGreaterThan(0);
        expect(h.onOutput).not.toHaveBeenCalled();
        expect(h.onError).not.toHaveBeenCalled();
    });

    it.each(finishedStatuses)('reports the %s outcome once and closes', (status) => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        source.emit('finished', {status});
        expect(h.onFinished).toHaveBeenCalledTimes(1);
        expect(h.onFinished).toHaveBeenCalledWith(status);
        expect(source.closed).toBe(true);
    });

    it('does not double-report when the stream ends right after finishing', () => {
        // A browser EventSource fires `error` on a clean end-of-stream too.
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        source.emit('finished', {status: 'failed'});
        expect(source.emit('error')).toBeGreaterThan(0);
        expect(h.onFinished).toHaveBeenCalledTimes(1);
        expect(h.onError).not.toHaveBeenCalled();
        expect(client.getJob).not.toHaveBeenCalled();
    });

    it('asks the server what happened when a finished event is unreadable', async () => {
        // Defaulting an unreadable outcome to "succeeded" is the same lie as
        // treating end-of-stream as success.
        const h = handlers();
        vi.mocked(client.getJob).mockResolvedValue(job({status: 'failed'}));
        subscribeJobEvents('job-91', h, () => source);
        source.emit('finished', {status: 'no idea'});
        await vi.waitFor(() => expect(h.onFinished).toHaveBeenCalledWith('failed'));
        expect(h.onFinished).toHaveBeenCalledTimes(1);
        expect(source.closed).toBe(true);
    });

    it('reports a lagged gap without tearing the stream down', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        source.emit('lagged', {skipped: 12});
        expect(h.onLagged).toHaveBeenCalledWith(12);
        expect(source.closed).toBe(false);
        expect(h.onError).not.toHaveBeenCalled();
        source.emit('output', {stream: 'stdout', text: 'still here'});
        expect(h.onOutput).toHaveBeenCalledWith('stdout', 'still here');
    });

    it('still reports a gap when the lagged count is unreadable', () => {
        const h = handlers();
        subscribeJobEvents('job-91', h, () => source);
        expect(source.emit('lagged', {skipped: 'lots'})).toBeGreaterThan(0);
        expect(h.onLagged).toHaveBeenCalledWith(0);
    });

    it('closes on a mid-stream error so the browser cannot silently reconnect and replay', async () => {
        // The server replays a job's buffered output to every new subscriber,
        // so an EventSource auto-reconnect would duplicate the whole run.
        const h = handlers();
        vi.mocked(client.getJob).mockResolvedValue(job({status: 'running'}));
        subscribeJobEvents('job-91', h, () => source);
        source.emit('error');
        expect(source.closed).toBe(true);
        await vi.waitFor(() => expect(h.onError).toHaveBeenCalled());
        expect(client.getJob).toHaveBeenCalledWith('job-91');
        expect(h.onError.mock.calls[0][0].message).toBe('lost the job output stream while the job was still running');
        expect(h.onFinished).not.toHaveBeenCalled();
    });

    it.each(finishedStatuses)('recovers a %s outcome the broken stream never delivered', async (status) => {
        // End of stream is not a result: a server that shut down mid-run closes
        // the connection cleanly and never sends `finished`.
        const h = handlers();
        vi.mocked(client.getJob).mockResolvedValue(job({status}));
        subscribeJobEvents('job-91', h, () => source);
        source.emit('error');
        await vi.waitFor(() => expect(h.onFinished).toHaveBeenCalledWith(status));
        expect(h.onError).not.toHaveBeenCalled();
    });

    it('reports an error when the server is gone entirely', async () => {
        const h = handlers();
        vi.mocked(client.getJob).mockRejectedValue(new TypeError('Failed to fetch'));
        subscribeJobEvents('job-91', h, () => source);
        source.emit('error');
        await vi.waitFor(() => expect(h.onError).toHaveBeenCalled());
        expect(h.onError.mock.calls[0][0].message).toBe('lost connection to the server');
    });

    it('stops delivering events after the caller unsubscribes', () => {
        const h = handlers();
        const unsubscribe = subscribeJobEvents('job-91', h, () => source);
        unsubscribe();
        expect(source.closed).toBe(true);
        // Each emit still reaches a registered listener -- the silence below is
        // the subscription refusing to report, not an absence of listeners.
        expect(source.emit('output', {stream: 'stdout', text: 'too late'})).toBeGreaterThan(0);
        expect(source.emit('lagged', {skipped: 3})).toBeGreaterThan(0);
        expect(source.emit('finished', {status: 'succeeded'})).toBeGreaterThan(0);
        expect(source.emit('error')).toBeGreaterThan(0);
        expect(h.onOutput).not.toHaveBeenCalled();
        expect(h.onLagged).not.toHaveBeenCalled();
        expect(h.onFinished).not.toHaveBeenCalled();
        expect(h.onError).not.toHaveBeenCalled();
        expect(client.getJob).not.toHaveBeenCalled();
    });

    it('stays quiet when the caller unsubscribes while the outcome lookup is in flight', async () => {
        // The same lookup reports `succeeded` in the recovery test above, so
        // this silence is the unsubscribe and not a lookup that never ran.
        const h = handlers();
        let deliver: (job: Job) => void = () => {};
        vi.mocked(client.getJob).mockReturnValue(new Promise<Job>((resolve) => {
            deliver = resolve;
        }));
        const unsubscribe = subscribeJobEvents('job-91', h, () => source);
        source.emit('error');
        expect(client.getJob).toHaveBeenCalledWith('job-91');
        unsubscribe();
        deliver(job({status: 'succeeded'}));
        await settle();
        expect(h.onFinished).not.toHaveBeenCalled();
        expect(h.onError).not.toHaveBeenCalled();
    });

    it('stays quiet when the caller unsubscribes and the outcome lookup then fails', async () => {
        // The rejected branch of the same lookup, which reports
        // `lost connection to the server` when nobody has unsubscribed.
        const h = handlers();
        let fail: (reason: Error) => void = () => {};
        vi.mocked(client.getJob).mockReturnValue(new Promise<Job>((resolve, reject) => {
            fail = reject;
        }));
        const unsubscribe = subscribeJobEvents('job-91', h, () => source);
        source.emit('error');
        expect(client.getJob).toHaveBeenCalledWith('job-91');
        unsubscribe();
        fail(new TypeError('Failed to fetch'));
        await settle();
        expect(h.onError).not.toHaveBeenCalled();
        expect(h.onFinished).not.toHaveBeenCalled();
    });
});
