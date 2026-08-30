import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useScriptStore} from './scriptStore';
import * as client from '@/api/client';
import * as jobEvents from '@/api/jobEvents';
import {JobEventHandlers} from '@/api/jobEvents';
import {ApiError} from '@/api/http';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');

/**
 * Fixtures, chosen so that every assertion below can only pass because the
 * store moved the value it claims to move.
 *
 * The `.js` path is deliberate: the store's initial `language` is already
 * `'lua'`, so a `.lua` fixture would let a store that never calls
 * `languageFromPath` pass the language assertion unchanged.
 */
const SCRIPT_PATH = '/tas/step-3-smelting.lua';
const JS_TOOL_PATH = '/tools/report.js';
const DIRECTORY = '/tas';
const LOADED_CODE = 'game.print("loaded-8143")';
const EDITED_CODE = 'game.print("edited-2266")';
const INLINE_CODE = 'game.print("inline-6094")';

/** Job ids, all distinct from each other so no assertion can match by accident. */
const SCRIPT_JOB_ID = '4471';
const INLINE_JOB_ID = '8802';
const SECOND_JOB_ID = '8803';
const OCCUPYING_JOB_ID = '5309';
const STALE_JOB_ID = '1204';

const NODE = {
    key: '/tas/step-3-smelting.lua',
    label: 'step-3-smelting.lua',
    leaf: true,
    children: []
};

/** One call to `subscribeJobEvents`, with its own unsubscribe spy. */
interface Subscription {
    jobId: string;
    handlers: JobEventHandlers;
    unsubscribe: ReturnType<typeof vi.fn>;
}

/**
 * Every subscription made during one test.
 *
 * A spy per call, rather than one shared spy: the store keeps its unsubscribe
 * callback in a module-level variable, which outlives the Pinia instance that
 * `setActivePinia` replaces between tests. A single shared spy would therefore
 * record calls made by a *previous* test's store, and "unsubscribed once"
 * would pass on a store that never unsubscribed at all.
 */
let subscriptions: Subscription[] = [];

function subscription(): Subscription {
    if (subscriptions.length === 0) {
        throw new Error('nothing subscribed to the job event stream');
    }
    return subscriptions[subscriptions.length - 1];
}

/** The handlers of the most recent subscription, i.e. the live stream. */
function stream(): JobEventHandlers {
    return subscription().handlers;
}

/**
 * A promise settled by hand, so the window in which a request is still on the
 * wire is observable. Without it nothing distinguishes `executing = true` from
 * a store that never set it -- an awaited action has already settled.
 */
function deferred<T>() {
    let settle: (value: T) => void = () => undefined;
    const promise = new Promise<T>((resolve) => {
        settle = resolve;
    });
    return {promise, settle};
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    subscriptions = [];
    vi.mocked(jobEvents.subscribeJobEvents).mockImplementation((jobId, handlers) => {
        const unsubscribe = vi.fn();
        subscriptions.push({jobId, handlers, unsubscribe});
        return unsubscribe;
    });
});

afterEach(() => {
    // The store's stream reference is module-level; leaving one attached would
    // have the next test's first run unsubscribe from this test's job.
    useScriptStore().stopWatching();
});

describe('scriptStore reading and writing scripts', () => {
    it('reports itself loading while the directory listing is in flight', async () => {
        const pending = deferred<typeof NODE[]>();
        vi.mocked(client.listScripts).mockReturnValue(pending.promise);
        const store = useScriptStore();

        const running = store.loadScriptsInDirectory(DIRECTORY);
        expect(store.getLoadingScriptsInDirectory).toBe(true);

        pending.settle([NODE]);
        const nodes = await running;

        expect(client.listScripts).toHaveBeenCalledWith(DIRECTORY);
        expect(nodes).toEqual([NODE]);
        expect(store.getLoadingScriptsInDirectory).toBe(false);
    });

    it('clears the loading flag even when the listing fails', async () => {
        const failure = new ApiError(400, 'not a directory: /tas', 1, {
            message: 'not a directory: /tas',
            code: 1
        });
        vi.mocked(client.listScripts).mockRejectedValue(failure);
        const store = useScriptStore();

        await expect(store.loadScriptsInDirectory(DIRECTORY)).rejects.toBe(failure);

        expect(store.getLoadingScriptsInDirectory).toBe(false);
    });

    it('unwraps the code field and derives the editor language from the path', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: LOADED_CODE});
        const store = useScriptStore();

        const code = await store.loadScriptFile(JS_TOOL_PATH);

        expect(client.readScript).toHaveBeenCalledWith(JS_TOOL_PATH);
        expect(code).toBe(LOADED_CODE);
        expect(store.getCode).toBe(LOADED_CODE);
        // `'js'`, not the store's initial `'lua'`: the derivation ran.
        expect(store.getLanguage).toBe('js');
        expect(store.getActiveScriptPath).toBe(JS_TOOL_PATH);
    });

    it('saves the editor buffer to the file it came from', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: LOADED_CODE});
        vi.mocked(client.writeScript).mockResolvedValue(undefined);
        const store = useScriptStore();
        await store.loadScriptFile(SCRIPT_PATH);

        await store.setCode(EDITED_CODE);

        // The edited text, not the loaded text, and against the active path.
        expect(client.writeScript).toHaveBeenCalledWith(SCRIPT_PATH, EDITED_CODE);
        expect(store.getCode).toBe(EDITED_CODE);
    });

    it('does not save an editor buffer that has no file behind it', async () => {
        const store = useScriptStore();

        await store.setCode(EDITED_CODE);

        // `PUT /api/v1/scripts/file` requires a path and answers 400 without
        // one; the buffer still has to hold the typed text so it can be run
        // inline. The positive control is the case above.
        expect(client.writeScript).not.toHaveBeenCalled();
        expect(store.getCode).toBe(EDITED_CODE);
    });
});

describe('scriptStore running a script', () => {
    it('starts the active script and streams its output into the two buffers', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: LOADED_CODE});
        vi.mocked(client.executeScript).mockResolvedValue({job_id: SCRIPT_JOB_ID});
        const store = useScriptStore();
        await store.loadScriptFile(SCRIPT_PATH);

        await store.executeScript();

        expect(client.executeScript).toHaveBeenCalledWith({path: SCRIPT_PATH});
        expect(subscription().jobId).toBe(SCRIPT_JOB_ID);
        expect(store.jobId).toBe(SCRIPT_JOB_ID);
        // A 202 says the run *started*, never that it succeeded.
        expect(store.isExecuting).toBe(true);
        expect(store.success).toBe(false);

        stream().onOutput('stdout', 'placing 12 furnaces');
        stream().onOutput('stderr', 'rcon timed out after 3s');

        expect(store.getStdout).toBe('placing 12 furnaces\n');
        expect(store.getStderr).toBe('rcon timed out after 3s\n');

        stream().onFinished('succeeded');

        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
        expect(store.error).toBe(false);
    });

    it('marks a failed run failed rather than succeeded', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: INLINE_JOB_ID});
        const store = useScriptStore();
        store.code = INLINE_CODE;
        await store.executeCode();

        stream().onFinished('failed');

        expect(store.isExecuting).toBe(false);
        expect(store.error).toBe(true);
        expect(store.success).toBe(false);
    });

    it('sends inline code with its language, and no path', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: INLINE_JOB_ID});
        const store = useScriptStore();
        store.code = INLINE_CODE;
        // Not the initial `'lua'`, so the language is observably passed on.
        store.language = 'js';

        await store.executeCode();

        expect(client.executeScript).toHaveBeenCalledWith({code: INLINE_CODE, language: 'js'});
        expect(store.jobId).toBe(INLINE_JOB_ID);
    });

    it('follows the run over time instead of settling when the request returns', async () => {
        // The 202 is the *start* of the story. If this test would pass with
        // the event loop removed, it is testing construction rather than a
        // stream: hence the timers, and the assertions between them.
        vi.useFakeTimers();
        try {
            vi.mocked(client.executeScript).mockResolvedValue({job_id: INLINE_JOB_ID});
            vi.mocked(jobEvents.subscribeJobEvents).mockImplementation((_jobId, handlers) => {
                setTimeout(() => handlers.onOutput('stdout', 'step 1 of 2'), 5);
                setTimeout(() => handlers.onOutput('stdout', 'step 2 of 2'), 10);
                setTimeout(() => handlers.onFinished('succeeded'), 15);
                return vi.fn();
            });
            const store = useScriptStore();
            store.code = INLINE_CODE;
            // The outcome of an earlier run, which must not survive this one.
            store.success = true;

            await store.executeCode();

            expect(store.isExecuting).toBe(true);
            expect(store.success).toBe(false);
            expect(store.getStdout).toBe('');

            await vi.advanceTimersByTimeAsync(5);

            expect(store.getStdout).toBe('step 1 of 2\n');
            expect(store.isExecuting).toBe(true);

            await vi.advanceTimersByTimeAsync(10);

            // Arrival order preserved, and the outcome only now.
            expect(store.getStdout).toBe('step 1 of 2\nstep 2 of 2\n');
            expect(store.isExecuting).toBe(false);
            expect(store.success).toBe(true);
        } finally {
            vi.useRealTimers();
        }
    });

    it('reports itself executing while the start request is still on the wire', async () => {
        const pending = deferred<{job_id: string}>();
        vi.mocked(client.executeScript).mockReturnValue(pending.promise);
        const store = useScriptStore();
        store.code = INLINE_CODE;

        const running = store.executeCode();

        // The Run button has to read "Running ..." from the click onwards. The
        // POST is a round trip of its own, and nothing else sets this flag
        // until the 202 comes back and the subscription is made.
        expect(store.isExecuting).toBe(true);
        expect(subscriptions).toHaveLength(0);

        pending.settle({job_id: INLINE_JOB_ID});
        await running;

        expect(store.isExecuting).toBe(true);
        expect(subscription().jobId).toBe(INLINE_JOB_ID);
    });

    it('refuses to run an empty buffer without disturbing the last run', async () => {
        const store = useScriptStore();
        store.stdout = 'output of the previous run';

        await expect(store.executeCode()).rejects.toThrow(/no code/);

        expect(client.executeScript).not.toHaveBeenCalled();
        expect(store.getStdout).toBe('output of the previous run');
        expect(store.isExecuting).toBe(false);
    });

    it('refuses to run when no script file is selected', async () => {
        const store = useScriptStore();
        store.stdout = 'output of the previous run';

        await expect(store.executeScript()).rejects.toThrow(/no script/);

        expect(client.executeScript).not.toHaveBeenCalled();
        expect(store.getStdout).toBe('output of the previous run');
        expect(store.isExecuting).toBe(false);
    });
});

describe('scriptStore live output', () => {
    async function startRun(jobId: string) {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: jobId});
        const store = useScriptStore();
        store.code = INLINE_CODE;
        await store.executeCode();
        return store;
    }

    it('names the size of a gap the server reported', async () => {
        const store = await startRun(INLINE_JOB_ID);

        stream().onLagged(12);

        // Exact, not `toContain`: the unknown-count notice below must not be
        // able to satisfy this assertion, nor this one that.
        expect(store.getStdout).toBe(
            '... 12 lines skipped (the page could not keep up with the output) ...\n'
        );
    });

    it('does not claim nothing was lost when the gap has no count', async () => {
        const store = await startRun(INLINE_JOB_ID);

        // `subscribeJobEvents` passes 0 when the payload carried no readable
        // count. Rendering that as "0 lines skipped" would be a lie in the one
        // case the event exists to report.
        stream().onLagged(0);

        expect(store.getStdout).toBe(
            '... some lines skipped (the server did not say how many) ...\n'
        );
    });

    it('marks the run failed when the stream dies', async () => {
        const store = await startRun(INLINE_JOB_ID);

        stream().onError(new Error('lost connection to the server'));

        expect(store.isExecuting).toBe(false);
        expect(store.error).toBe(true);
        expect(store.success).toBe(false);
        expect(store.getStderr).toBe('... lost connection to the server ...\n');
    });

    it('attaches to a job by id, replacing whatever the pane was showing', () => {
        const store = useScriptStore();
        store.stdout = 'output of the previous run';
        store.stderr = 'failure of the previous run';
        store.success = true;

        store.attachToJob(OCCUPYING_JOB_ID);

        // The server replays a job's whole backlog to a new subscriber, so
        // keeping the old text would interleave two runs in one pane.
        expect(store.getStdout).toBe('');
        expect(store.getStderr).toBe('');
        expect(store.success).toBe(false);
        expect(store.jobId).toBe(OCCUPYING_JOB_ID);
        expect(store.isExecuting).toBe(true);
        expect(subscription().jobId).toBe(OCCUPYING_JOB_ID);
    });

    it('drops the previous stream when a new run starts', async () => {
        const store = await startRun(INLINE_JOB_ID);
        stream().onOutput('stdout', 'output of the previous run');
        const first = subscription();

        vi.mocked(client.executeScript).mockResolvedValue({job_id: SECOND_JOB_ID});
        await store.executeCode();

        expect(subscriptions).toHaveLength(2);
        expect(first.unsubscribe).toHaveBeenCalledTimes(1);
        expect(subscription().jobId).toBe(SECOND_JOB_ID);
        expect(store.jobId).toBe(SECOND_JOB_ID);
        expect(store.getStdout).toBe('');
    });

    it('unsubscribes once, and stops claiming to run, when told to stop watching', async () => {
        const store = await startRun(INLINE_JOB_ID);

        store.stopWatching();
        store.stopWatching();

        expect(subscription().unsubscribe).toHaveBeenCalledTimes(1);
        // Nothing is watching the job any more, so the store cannot know
        // whether it is still running -- and a stuck `true` would leave the
        // Run button disabled for the rest of the session.
        expect(store.isExecuting).toBe(false);
    });
});

describe('scriptStore when the execution slot is taken', () => {
    it('follows the running job the 409 names', async () => {
        const failure = new ApiError(
            409,
            'a script is already running as job ' + OCCUPYING_JOB_ID,
            5,
            {
                message: 'a script is already running as job ' + OCCUPYING_JOB_ID,
                code: 5,
                running_job_id: OCCUPYING_JOB_ID
            }
        );
        vi.mocked(client.executeScript).mockRejectedValue(failure);
        const store = useScriptStore();
        store.code = INLINE_CODE;

        // The rejection is the honest answer for *this* run: it did not start.
        await expect(store.executeCode()).rejects.toBe(failure);

        expect(subscription().jobId).toBe(OCCUPYING_JOB_ID);
        expect(store.jobId).toBe(OCCUPYING_JOB_ID);
        expect(store.isExecuting).toBe(true);
        // Not an error state: the pane now shows a run that is genuinely
        // going, and the page reports the refusal from the rejection above.
        expect(store.error).toBe(false);

        stream().onOutput('stdout', 'the other run is on step 4');
        expect(store.getStdout).toBe('the other run is on step 4\n');
    });

    it('does not attach to anything when the conflict names no job', async () => {
        const failure = new ApiError(409, 'a script is already running', 5, {
            message: 'a script is already running',
            code: 5
        });
        vi.mocked(client.executeScript).mockRejectedValue(failure);
        const store = useScriptStore();
        store.code = INLINE_CODE;

        await expect(store.executeCode()).rejects.toBe(failure);

        expect(subscriptions).toHaveLength(0);
        expect(store.isExecuting).toBe(false);
        expect(store.error).toBe(true);
        expect(store.jobId).toBeNull();
    });

    it('does not follow a job id on a status that does not mean the slot is taken', async () => {
        // `code: 2` is a 503 from this route (`ErrorResponse::not_running`)
        // and a 400 from `POST /api/v1/rcon`, so the store cannot key on the
        // code either. The `running_job_id` here is something the server never
        // sends on a 503; it is present so that dropping the status check --
        // and hijacking the pane on an unrelated failure -- fails this test
        // rather than passing quietly.
        const failure = new ApiError(503, 'not started', 2, {
            message: 'not started',
            code: 2,
            running_job_id: OCCUPYING_JOB_ID
        });
        vi.mocked(client.executeScript).mockRejectedValue(failure);
        const store = useScriptStore();
        store.code = INLINE_CODE;
        store.jobId = STALE_JOB_ID;

        await expect(store.executeCode()).rejects.toBe(failure);

        expect(subscriptions).toHaveLength(0);
        expect(store.isExecuting).toBe(false);
        expect(store.error).toBe(true);
        // The failed run has no job, and the previous one is not this one.
        expect(store.jobId).toBeNull();
    });
});
