import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useRconStore} from './rconStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';

vi.mock('@/api/client');

/**
 * A command no assertion below could match by accident, and not one of the
 * literals `RconPage.vue` sends from its cheat buttons: a store that ignored
 * its argument and posted a constant would still have to post *this*.
 */
const COMMAND = '/silent-command game.print("fixture-9317")';

/**
 * A promise this test resolves by hand, so the in-flight window of `execute`
 * is observable. Without it nothing can distinguish `executing = true` from a
 * store that never set it: the flag is back to `false` by the time an awaited
 * `execute` returns.
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
});

describe('rconStore', () => {
    it('posts the command it was given and reports success', async () => {
        vi.mocked(client.sendRcon).mockResolvedValue(undefined);
        const store = useRconStore();

        await store.execute(COMMAND);

        expect(client.sendRcon).toHaveBeenCalledTimes(1);
        expect(client.sendRcon).toHaveBeenCalledWith(COMMAND);
        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
        expect(store.error).toBe(false);
        expect(store.lastError).toBeNull();
    });

    it('clears the previous failure when a later command succeeds', async () => {
        vi.mocked(client.sendRcon).mockResolvedValue(undefined);
        const store = useRconStore();
        // The state a failed run leaves behind. Starting from the store's
        // defaults instead would make the three resets in `execute`
        // unobservable -- they would be assigning what was already there.
        store.error = true;
        store.lastError = 'the previous command failed';

        await store.execute(COMMAND);

        expect(store.error).toBe(false);
        expect(store.lastError).toBeNull();
        expect(store.success).toBe(true);
    });

    it('reports itself executing until the request settles', async () => {
        const pending = deferred<void>();
        vi.mocked(client.sendRcon).mockReturnValue(pending.promise);
        const store = useRconStore();

        const running = store.execute(COMMAND);
        expect(store.isExecuting).toBe(true);
        // Nothing is known yet, so neither outcome may be showing: a store
        // that set `success` before awaiting would light the button green
        // while the command was still on the wire.
        expect(store.success).toBe(false);
        expect(store.error).toBe(false);

        pending.settle();
        await running;

        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
    });

    it('rejects an empty command without calling the api', async () => {
        const store = useRconStore();
        // A successful earlier run. The guard has to reject *before* the
        // resets, so this must survive; a guard moved below them would wipe
        // the page's "Run" feedback over an accidental empty submit.
        store.success = true;

        await expect(store.execute('')).rejects.toThrow(/no command/);

        expect(client.sendRcon).not.toHaveBeenCalled();
        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
        expect(store.error).toBe(false);
    });

    it('rethrows a server failure instead of reporting success', async () => {
        // The Tauri store this replaced caught the error and then set
        // `success = true` as well, so `RconPage.vue`'s error toast -- which
        // fires from its own `catch` -- was unreachable.
        const failure = new ApiError(500, 'rcon connection reset', 6, {
            message: 'rcon connection reset',
            code: 6
        });
        vi.mocked(client.sendRcon).mockRejectedValue(failure);
        const store = useRconStore();
        store.success = true;

        // The same object, not a copy: the page shows `err.message`, and a
        // rewrapped error would drop the server's `code` on the way out.
        await expect(store.execute(COMMAND)).rejects.toBe(failure);

        expect(store.error).toBe(true);
        expect(store.success).toBe(false);
        expect(store.isExecuting).toBe(false);
        expect(store.lastError).toBe('rcon connection reset');
    });

    /**
     * "No Factorio instance is running" is `code: 2` in
     * `crates/server/src/error.rs`, and the two constructors for it disagree
     * about the status: `ErrorResponse::not_started()` -- what
     * `POST /api/v1/rcon` itself sends -- answers `400`, while
     * `ErrorResponse::not_running()` answers `503`.
     *
     * Unlike `instanceStore.stopInstances`, this store has nothing to
     * reconcile: the user asked for a command to run and it did not run, so
     * `code: 2` is a failure like any other. What these cases pin is that it
     * is never turned into a success, and that neither status is singled out
     * -- a handler keyed on `400` or on `503` leaves one of them red.
     */
    it.each([
        {status: 400, message: 'not started', label: 'the 400 rcon itself answers'},
        {status: 503, message: 'no instance is running', label: 'the 503 sibling routes answer'}
    ])('surfaces code 2 on $label as a failure', async ({status, message}) => {
        vi.mocked(client.sendRcon).mockRejectedValue(
            new ApiError(status, message, 2, {message, code: 2})
        );
        const store = useRconStore();

        await expect(store.execute(COMMAND)).rejects.toThrow(message);

        expect(store.error).toBe(true);
        expect(store.success).toBe(false);
        expect(store.isExecuting).toBe(false);
        expect(store.lastError).toBe(message);
    });

    it('reports a transport failure that carries no application code', async () => {
        // `fetch` rejects with a `TypeError` when the server is unreachable;
        // it never reaches `errorFromResponse`, so there is no code to read.
        vi.mocked(client.sendRcon).mockRejectedValue(new TypeError('network down'));
        const store = useRconStore();

        await expect(store.execute(COMMAND)).rejects.toThrow('network down');

        expect(store.error).toBe(true);
        expect(store.lastError).toBe('network down');
        expect(store.isExecuting).toBe(false);
    });

    it('records a rejection that is not an Error at all', async () => {
        vi.mocked(client.sendRcon).mockRejectedValue('kaboom');
        const store = useRconStore();

        await expect(store.execute(COMMAND)).rejects.toBe('kaboom');

        expect(store.error).toBe(true);
        // Stringified rather than left null: `lastError` is what a caller
        // reads back off the store, and `null` there would read as "no
        // failure" on a run that plainly failed.
        expect(store.lastError).toBe('kaboom');
        expect(store.isExecuting).toBe(false);
    });
});
