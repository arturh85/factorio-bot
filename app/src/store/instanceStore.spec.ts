import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useInstanceStore} from './instanceStore';
import {useAppStore} from './appStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';
import {InstanceStatus} from '@/api/types';
import {AppSettings} from '@/models/settings';

vi.mock('@/api/client');

/**
 * `GET /api/v1/instance` as the server sends it. The defaults are the
 * "nothing is running" answer; every test overrides the fields it is about.
 *
 * The numbers are deliberately not the store's initial state: `clientCount`
 * starts at 0, so a fixture with `client_count: 0` would make "the store read
 * the response" and "the store never assigned" indistinguishable.
 */
function status(overrides: Partial<InstanceStatus> = {}): InstanceStatus {
    return {
        started: false,
        starting: false,
        client_count: 0,
        server_port: null,
        rcon_port: null,
        last_error: null,
        ...overrides
    };
}

function settingsFixture(archivePath: string): AppSettings {
    return {
        gui: {enable_autostart: false, enable_restapi: true},
        restapi: {port: 7492, web_root: null},
        factorio: {
            client_count: 2,
            factorio_archive_path: archivePath,
            map_exchange_string: '>>>fixture-exchange<<<',
            rcon_pass: 'foobar',
            rcon_port: 4321,
            recreate: false,
            seed: 'fixture-seed',
            workspace_path: '/tmp/workspace'
        }
    };
}

/** Puts a configured archive path in `appStore`, which `startInstances` requires. */
function withArchivePath(archivePath = '/archives/factorio.tar.xz') {
    useAppStore().settings = settingsFixture(archivePath);
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

afterEach(() => {
    vi.useRealTimers();
});

describe('instanceStore', () => {
    describe('checkInstanceState', () => {
        it('mirrors the servers instance status', async () => {
            vi.mocked(client.getInstance).mockResolvedValue(
                status({started: true, client_count: 3})
            );
            const store = useInstanceStore();

            await expect(store.checkInstanceState()).resolves.toBe(true);

            expect(store.isStarted).toBe(true);
            expect(store.isStarting).toBe(false);
            expect(store.isFailed).toBe(false);
            // 3, not the 0 the store starts with and not the 2 configured in
            // the settings: this comes from the instance route alone.
            expect(store.clientCount).toBe(3);
        });

        it('shows an in-flight start as starting, not as started', async () => {
            vi.mocked(client.getInstance).mockResolvedValue(status({starting: true}));
            const store = useInstanceStore();

            await expect(store.checkInstanceState()).resolves.toBe(false);

            expect(store.isStarting).toBe(true);
            expect(store.isStarted).toBe(false);
        });

        it('surfaces a background start failure through lastError and failed', async () => {
            // The failure happens after the 202, so this is the only channel
            // the browser has to learn about it.
            vi.mocked(client.getInstance).mockResolvedValue(
                status({last_error: 'archive missing'})
            );
            const store = useInstanceStore();

            await store.checkInstanceState();

            expect(store.isFailed).toBe(true);
            expect(store.getLastError).toBe('archive missing');
        });

        it('keeps the status it already had when a poll fails', async () => {
            vi.mocked(client.getInstance).mockResolvedValue(
                status({started: true, client_count: 3})
            );
            const store = useInstanceStore();
            await store.checkInstanceState();

            vi.mocked(client.getInstance).mockRejectedValue(new ApiError(502, '502', null, ''));
            await expect(store.checkInstanceState()).rejects.toThrow('502');

            // A proxy hiccup must not make a running Factorio look stopped:
            // these are the values the *successful* poll above installed.
            expect(store.isStarted).toBe(true);
            expect(store.clientCount).toBe(3);
            expect(store.isFailed).toBe(false);
        });
    });

    /**
     * The store owns no timer -- App.vue does -- so these drive one over the
     * store's single-poll action and assert on transitions the first response
     * did not produce.
     */
    describe('driven by a two second poll', () => {
        it('flips starting to started when a later poll says so', async () => {
            vi.useFakeTimers();
            vi.mocked(client.getInstance)
                .mockResolvedValueOnce(status({starting: true}))
                .mockResolvedValueOnce(status({starting: true}))
                .mockResolvedValue(status({started: true, client_count: 3}));
            const store = useInstanceStore();
            const timer = setInterval(() => void store.checkInstanceState(), 2000);

            await vi.advanceTimersByTimeAsync(2000);
            expect(store.isStarting).toBe(true);
            expect(store.isStarted).toBe(false);

            await vi.advanceTimersByTimeAsync(4000);
            clearInterval(timer);

            expect(client.getInstance).toHaveBeenCalledTimes(3);
            expect(store.isStarted).toBe(true);
            expect(store.isStarting).toBe(false);
            expect(store.clientCount).toBe(3);
        });

        it('clears a failure once a later poll no longer reports one', async () => {
            vi.useFakeTimers();
            vi.mocked(client.getInstance)
                .mockResolvedValueOnce(status({last_error: 'archive missing'}))
                .mockResolvedValue(status({started: true, client_count: 3}));
            const store = useInstanceStore();
            const timer = setInterval(() => void store.checkInstanceState(), 2000);

            await vi.advanceTimersByTimeAsync(2000);
            expect(store.isFailed).toBe(true);
            expect(store.getLastError).toBe('archive missing');

            await vi.advanceTimersByTimeAsync(2000);
            clearInterval(timer);

            // A retry that worked has to clear the banner; a `failed` flag
            // that only ever latches true would leave the UI stuck on
            // "Failed" over a running game.
            expect(client.getInstance).toHaveBeenCalledTimes(2);
            expect(store.isFailed).toBe(false);
            expect(store.getLastError).toBeNull();
        });
    });

    describe('startInstances', () => {
        it('returns as soon as the start is accepted rather than waiting for factorio', async () => {
            vi.mocked(client.startInstance).mockResolvedValue({accepted: true});
            withArchivePath();
            const store = useInstanceStore();

            await store.startInstances();

            // The 202 says "accepted", never "started". Only a poll of
            // GET /api/v1/instance may set `started`.
            expect(client.startInstance).toHaveBeenCalledTimes(1);
            expect(store.isStarting).toBe(true);
            expect(store.isStarted).toBe(false);
            expect(store.isFailed).toBe(false);
        });

        it('refuses to start without a configured factorio archive path', async () => {
            withArchivePath('');
            const store = useInstanceStore();

            await expect(store.startInstances()).rejects.toThrow(/archive path/);

            expect(client.startInstance).not.toHaveBeenCalled();
            expect(store.isStarting).toBe(false);
        });

        it('refuses to start before the settings have loaded', async () => {
            const store = useInstanceStore();

            await expect(store.startInstances()).rejects.toThrow(/archive path/);

            expect(client.startInstance).not.toHaveBeenCalled();
        });

        it('refuses a second start while one instance is already known to run', async () => {
            withArchivePath();
            const store = useInstanceStore();
            store.started = true;

            await expect(store.startInstances()).rejects.toThrow(/already started/);

            expect(client.startInstance).not.toHaveBeenCalled();
        });

        it('reports a 409 from the server with the servers own words', async () => {
            vi.mocked(client.startInstance).mockRejectedValue(
                new ApiError(409, 'instance is already starting', 5, {})
            );
            withArchivePath();
            const store = useInstanceStore();

            await expect(store.startInstances()).rejects.toThrow('instance is already starting');

            expect(store.isStarting).toBe(false);
            expect(store.isFailed).toBe(true);
            expect(store.getLastError).toBe('instance is already starting');
        });

        it('clears the previous failure before attempting a new start', async () => {
            vi.mocked(client.getInstance).mockResolvedValue(
                status({last_error: 'archive missing'})
            );
            vi.mocked(client.startInstance).mockResolvedValue({accepted: true});
            withArchivePath();
            const store = useInstanceStore();
            await store.checkInstanceState();
            expect(store.isFailed).toBe(true);

            await store.startInstances();

            // The server clears `last_start_error` on an accepted start, so a
            // stale message here would contradict the very next poll.
            expect(store.isFailed).toBe(false);
            expect(store.getLastError).toBeNull();
        });
    });

    describe('stopInstances', () => {
        it('clears started once the stop succeeds', async () => {
            vi.mocked(client.stopInstance).mockResolvedValue(undefined);
            const store = useInstanceStore();
            store.started = true;
            store.clientCount = 3;

            await store.stopInstances();

            expect(client.stopInstance).toHaveBeenCalledTimes(1);
            expect(store.isStarted).toBe(false);
            expect(store.isStopping).toBe(false);
            expect(store.isFailed).toBe(false);
            expect(store.clientCount).toBe(0);
        });

        it('does not call the server when nothing is known to run', async () => {
            const store = useInstanceStore();

            await expect(store.stopInstances()).rejects.toThrow(/not started/);

            expect(client.stopInstance).not.toHaveBeenCalled();
        });

        /**
         * `code: 2` is "no Factorio instance is running". `stop` answers it as
         * a 400 (`ErrorResponse::not_started`) while other routes answer the
         * same condition as a 503 (`ErrorResponse::not_running`), so the store
         * keys off the code. Both statuses are driven here: a branch written
         * against either number alone leaves one of these red.
         */
        it.each([
            {status: 400, label: 'the 400 stop answers'},
            {status: 503, label: 'the 503 other routes answer'}
        ])('treats code 2 on $label as already stopped', async ({status: httpStatus}) => {
            vi.mocked(client.stopInstance).mockRejectedValue(
                new ApiError(httpStatus, 'not started', 2, {message: 'not started', code: 2})
            );
            const store = useInstanceStore();
            store.started = true;
            store.clientCount = 3;

            // The user asked for no Factorio and there is no Factorio. That is
            // the outcome they wanted, not an error to put in front of them.
            await expect(store.stopInstances()).resolves.toBeUndefined();

            expect(store.isStarted).toBe(false);
            expect(store.clientCount).toBe(0);
            expect(store.isFailed).toBe(false);
            expect(store.isStopping).toBe(false);
        });

        it('reports a stop the server could not carry out and keeps started set', async () => {
            vi.mocked(client.stopInstance).mockRejectedValue(
                new ApiError(500, 'failed to stop instance: no such process', 6, {})
            );
            const store = useInstanceStore();
            store.started = true;
            store.clientCount = 3;

            await expect(store.stopInstances()).rejects.toThrow('failed to stop instance');

            // A 500 means the children may well still be alive; claiming the
            // instance is gone would hide a Factorio the user cannot reach.
            expect(store.isStarted).toBe(true);
            expect(store.clientCount).toBe(3);
            expect(store.isFailed).toBe(true);
            expect(store.getLastError).toBe('failed to stop instance: no such process');
            expect(store.isStopping).toBe(false);
        });

        it('reports a transport failure that carries no application code', async () => {
            // A proxy's 502 has no `{message, code}` body, so `code` is null
            // -- which must not be mistaken for the code 2 reconcile above.
            vi.mocked(client.stopInstance).mockRejectedValue(new TypeError('network down'));
            const store = useInstanceStore();
            store.started = true;

            await expect(store.stopInstances()).rejects.toThrow('network down');

            expect(store.isStarted).toBe(true);
            expect(store.isFailed).toBe(true);
            expect(store.getLastError).toBe('network down');
            expect(store.isStopping).toBe(false);
        });
    });
});
