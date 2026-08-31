import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from './appStore';
import * as client from '@/api/client';
import {AppSettings} from '@/models/settings';

vi.mock('@/api/client');

/**
 * Every field carries a value distinguishable from every other field of the
 * same type, so a getter or an update action that reaches for the wrong one
 * cannot read back the value the test expects. `client_count` and `rcon_port`
 * differ; `seed`, `map_exchange_string`, `workspace_path` and
 * `factorio_archive_path` all differ.
 */
function settingsFixture(): AppSettings {
    return {
        gui: {enable_autostart: false, enable_restapi: true},
        restapi: {port: 7492, web_root: null},
        factorio: {
            client_count: 2,
            factorio_archive_path: '/archives/factorio.tar.xz',
            map_exchange_string: '>>>fixture-exchange<<<',
            rcon_pass: 'foobar',
            rcon_port: 4321,
            recreate: false,
            seed: 'fixture-seed',
            workspace_path: '/tmp/workspace'
        }
    };
}

/** The body the store handed to `putSettings` on its `nth` (0-based) call. */
function sentOn(nth: number): AppSettings {
    return vi.mocked(client.putSettings).mock.calls[nth][0];
}

/**
 * Makes PUT behave like the real server for tests that do not care about
 * normalisation: it answers with what it was given.
 *
 * Snapshotting the argument matters. The store mutates its own state object in
 * place before the call, so echoing the same reference back would make "the
 * store adopted the response" and "the store kept its local object"
 * indistinguishable -- exactly the confusion the normalisation test below
 * exists to rule out.
 *
 * A JSON round-trip rather than `structuredClone`: what arrives here is Vue's
 * reactive proxy over the state, and `structuredClone` refuses a Proxy.
 * `AppSettings` is the JSON the server sends, so the round-trip is lossless.
 */
function echoPut() {
    vi.mocked(client.putSettings).mockImplementation(
        (settings: AppSettings) => Promise.resolve(JSON.parse(JSON.stringify(settings)) as AppSettings)
    );
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('appStore', () => {
    it('loads settings from the api and exposes every getter from them', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        const store = useAppStore();

        await expect(store.loadSettings()).resolves.toEqual(settingsFixture());

        expect(store.getSettings).toEqual(settingsFixture());
        expect(store.getWorkspacePath).toBe('/tmp/workspace');
        expect(store.getFactorioArchivePath).toBe('/archives/factorio.tar.xz');
        expect(store.getMapExchangeString).toBe('>>>fixture-exchange<<<');
        expect(store.getSeed).toBe('fixture-seed');
        expect(store.getClientCount).toBe(2);
        expect(store.getRecreateLevel).toBe(false);
        expect(store.getEnableAutostart).toBe(false);
        // Reads `restapi.port` (7492), not `factorio.rcon_port` (4321).
        expect(store.getRestapiPort).toBe(7492);
        expect(client.getSettings).toHaveBeenCalledTimes(1);
    });

    it('reports null from the getters before anything is loaded', () => {
        const store = useAppStore();

        expect(store.getSettings).toBeNull();
        expect(store.getWorkspacePath).toBeNull();
        expect(store.getFactorioArchivePath).toBeNull();
        expect(store.getMapExchangeString).toBeNull();
        expect(store.getSeed).toBeNull();
        expect(store.getClientCount).toBeNull();
        expect(store.getEnableAutostart).toBeNull();
        expect(store.getRestapiPort).toBeNull();
        expect(store.getRecreateLevel).toBeUndefined();
    });

    it('adopts the settings PUT answered with, not the ones it sent', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        // The server is authoritative and may normalise. Here it strips the
        // trailing slash, so "sent" and "answered" are different strings and
        // the assertion can tell which one the store kept.
        const normalised = settingsFixture();
        normalised.factorio.workspace_path = '/srv/workspace';
        vi.mocked(client.putSettings).mockResolvedValue(normalised);

        const store = useAppStore();
        await store.loadSettings();
        await store.updateWorkspacePath('/srv/workspace/');

        expect(client.putSettings).toHaveBeenCalledTimes(1);
        expect(sentOn(0).factorio.workspace_path).toBe('/srv/workspace/');
        expect(store.getWorkspacePath).toBe('/srv/workspace');
    });

    /**
     * Reachable precisely because `_updateSettings` is a Pinia action: anything
     * holding the store can call it, not only the update actions that establish
     * `settings` first. Without the guard this PUTs `null` as the whole
     * settings document -- the server's own settings, overwritten with nothing,
     * from a store that had merely never loaded.
     */
    it('does not PUT when no settings have been loaded', async () => {
        const store = useAppStore();
        expect(store.settings).toBeNull();

        await store._updateSettings();

        expect(client.putSettings).not.toHaveBeenCalled();
        expect(store.settings).toBeNull();
    });

    describe('each update action sends its own field and nothing else', () => {
        /**
         * `apply` drives the store; `mutate` states, independently, the one
         * change the whole PUT body is then expected to show. Comparing the
         * body against a fixture built that way catches three faults with one
         * assertion: the field not being written, the wrong field being
         * written, and any other field being written as a side effect.
         */
        const cases: Array<{
            name: string;
            apply: (store: ReturnType<typeof useAppStore>) => Promise<void>;
            mutate: (settings: AppSettings) => void;
        }> = [
            {
                name: 'updateWorkspacePath',
                apply: (s) => s.updateWorkspacePath('/srv/other'),
                mutate: (s) => void (s.factorio.workspace_path = '/srv/other')
            },
            {
                name: 'updateFactorioArchivePath',
                apply: (s) => s.updateFactorioArchivePath('/srv/factorio.tar.xz'),
                mutate: (s) => void (s.factorio.factorio_archive_path = '/srv/factorio.tar.xz')
            },
            {
                name: 'updateRecreateLevel',
                apply: (s) => s.updateRecreateLevel(true),
                mutate: (s) => void (s.factorio.recreate = true)
            },
            {
                name: 'updateEnableAutostart',
                apply: (s) => s.updateEnableAutostart(true),
                mutate: (s) => void (s.gui.enable_autostart = true)
            },
            {
                name: 'updateRestapiPort',
                apply: (s) => s.updateRestapiPort(8080),
                mutate: (s) => void (s.restapi.port = 8080)
            },
            {
                name: 'updateClientCount',
                apply: (s) => s.updateClientCount(7),
                mutate: (s) => void (s.factorio.client_count = 7)
            },
            {
                name: 'updateMapExchangeString',
                apply: (s) => s.updateMapExchangeString('>>>changed<<<'),
                mutate: (s) => void (s.factorio.map_exchange_string = '>>>changed<<<')
            },
            {
                name: 'updateSeed',
                apply: (s) => s.updateSeed('12345'),
                mutate: (s) => void (s.factorio.seed = '12345')
            }
        ];

        for (const testCase of cases) {
            it(testCase.name, async () => {
                vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
                echoPut();

                const expected = settingsFixture();
                testCase.mutate(expected);
                // Fixture guard: if the case's new value equalled the loaded
                // one, the comparison below would pass with the write deleted.
                expect(expected).not.toEqual(settingsFixture());

                const store = useAppStore();
                await store.loadSettings();
                await testCase.apply(store);

                expect(client.putSettings).toHaveBeenCalledTimes(1);
                expect(sentOn(0)).toEqual(expected);
                expect(store.getSettings).toEqual(expected);
            });
        }
    });

    it('does not reach the api before settings are loaded, but does after', async () => {
        const store = useAppStore();

        await store.updateSeed('12345');
        expect(client.putSettings).not.toHaveBeenCalled();
        expect(store.getSettings).toBeNull();

        // Positive control for the assertion above: with the same mock and the
        // same call, loading first makes the PUT happen. Without this, a
        // broken `vi.mock` would leave the "not called" assertion green.
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        echoPut();
        await store.loadSettings();
        await store.updateSeed('12345');
        expect(client.putSettings).toHaveBeenCalledTimes(1);
        expect(sentOn(0).factorio.seed).toBe('12345');
    });

    it.each([true, false])(
        'answers fileExists with the servers verdict (%s), not the browsers',
        async (exists) => {
            vi.mocked(client.pathExists).mockResolvedValue({exists});
            const store = useAppStore();

            await expect(store.fileExists('/tmp/workspace')).resolves.toBe(exists);
            expect(client.pathExists).toHaveBeenCalledWith('/tmp/workspace');
        }
    );

    it('propagates a failed load and keeps the settings it already had', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        const store = useAppStore();
        await store.loadSettings();

        vi.mocked(client.getSettings).mockRejectedValue(new Error('boom'));
        await expect(store.loadSettings()).rejects.toThrow('boom');

        // Not null and not half-written: a transient failure must not blank a
        // settings page that was working a moment ago.
        expect(store.getSettings).toEqual(settingsFixture());
        expect(store.getWorkspacePath).toBe('/tmp/workspace');
    });

    it('leaves settings null when the very first load fails', async () => {
        vi.mocked(client.getSettings).mockRejectedValue(new Error('boom'));
        const store = useAppStore();

        await expect(store.loadSettings()).rejects.toThrow('boom');
        expect(store.getSettings).toBeNull();
    });

    it('propagates a failed save so the ui cannot report a persisted change', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        vi.mocked(client.putSettings).mockRejectedValue(new Error('disk full'));

        const store = useAppStore();
        await store.loadSettings();

        await expect(store.updateSeed('12345')).rejects.toThrow('disk full');
    });
});
