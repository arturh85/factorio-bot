import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useMapStore} from './mapStore';
import * as game from '@/api/game';
import {ApiError} from '@/api/http';
import {SPAWN_ENTITIES} from '@/api/game.fixtures';

// Partial mock: only `findEntities` is replaced. `game.fixtures.ts` imports
// `parseFactorioEntities` from this same module to build `SPAWN_ENTITIES`
// above, and a blanket `vi.mock('@/api/game')` would auto-mock THAT too,
// turning the fixture into `undefined` (a mocked function with no
// implementation returns `undefined`) rather than throwing -- a silent
// failure that would make every assertion against `SPAWN_ENTITIES` compare
// `undefined` to `undefined` and pass for the wrong reason.
vi.mock('@/api/game', async importOriginal => ({
    ...(await importOriginal<typeof import('@/api/game')>()),
    findEntities: vi.fn()
}));

const QUERY = {x: 0, y: 0, radius: 50, entityType: ''};

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

describe('mapStore', () => {
    it('starts not-queried, before anything has been fetched', () => {
        const store = useMapStore();
        expect(store.current.status).toBe('not-queried');
    });

    it('reports loading while the request is in flight, with nothing decided yet', async () => {
        const pending = deferred<typeof SPAWN_ENTITIES>();
        vi.mocked(game.findEntities).mockReturnValue(pending.promise);
        const store = useMapStore();

        const running = store.query(QUERY);
        expect(store.current.status).toBe('loading');

        pending.settle(SPAWN_ENTITIES);
        await running;
        expect(store.current.status).toBe('ready');
    });

    it('sends position/radius/entity_type built from the query', async () => {
        vi.mocked(game.findEntities).mockResolvedValue([]);
        const store = useMapStore();

        await store.query({x: 3, y: -7, radius: 25, entityType: 'resource'});

        expect(game.findEntities).toHaveBeenCalledWith({
            position: {x: 3, y: -7},
            radius: 25,
            entity_type: 'resource'
        });
    });

    it('omits entity_type from the request when the filter is blank', async () => {
        vi.mocked(game.findEntities).mockResolvedValue([]);
        const store = useMapStore();

        await store.query(QUERY);

        expect(game.findEntities).toHaveBeenCalledWith({
            position: {x: 0, y: 0},
            radius: 50,
            entity_type: undefined
        });
    });

    it('goes ready with the entities and a fetch timestamp on success', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const before = Date.now();
        const store = useMapStore();

        await store.query(QUERY);

        expect(store.current.status).toBe('ready');
        if (store.current.status !== 'ready') {
            throw new Error('unreachable');
        }
        expect(store.current.entities).toEqual(SPAWN_ENTITIES);
        expect(store.current.fetchedAtMs).toBeGreaterThanOrEqual(before);
        expect(store.current.fetchedAtMs).toBeLessThanOrEqual(Date.now());
    });

    it('goes ready with zero entities for an empty result -- not the same state as not-queried', async () => {
        vi.mocked(game.findEntities).mockResolvedValue([]);
        const store = useMapStore();

        await store.query(QUERY);

        expect(store.current.status).toBe('ready');
        if (store.current.status !== 'ready') {
            throw new Error('unreachable');
        }
        expect(store.current.entities).toEqual([]);
    });

    it('goes not-running on an ApiError with code 2, regardless of HTTP status', async () => {
        vi.mocked(game.findEntities).mockRejectedValue(
            new ApiError(503, 'not started', 2, {message: 'not started', code: 2})
        );
        const store = useMapStore();

        await store.query(QUERY);

        expect(store.current.status).toBe('not-running');
    });

    it('goes error, carrying the message, for any other failure', async () => {
        vi.mocked(game.findEntities).mockRejectedValue(new ApiError(500, 'boom', 6, {}));
        const store = useMapStore();

        await store.query(QUERY);

        expect(store.current.status).toBe('error');
        if (store.current.status !== 'error') {
            throw new Error('unreachable');
        }
        expect(store.current.message).toBe('boom');
    });

    it('handles a non-Error rejection without throwing', async () => {
        vi.mocked(game.findEntities).mockRejectedValue('a plain string rejection');
        const store = useMapStore();

        await store.query(QUERY);

        expect(store.current.status).toBe('error');
    });

    it('refresh() re-runs the last query unchanged', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const store = useMapStore();
        await store.query({x: 1, y: 2, radius: 10, entityType: 'resource'});
        vi.mocked(game.findEntities).mockClear();

        await store.refresh();

        expect(game.findEntities).toHaveBeenCalledWith({
            position: {x: 1, y: 2},
            radius: 10,
            entity_type: 'resource'
        });
    });

    it('refresh() is a no-op when nothing has been queried yet', async () => {
        const store = useMapStore();
        await store.refresh();
        expect(game.findEntities).not.toHaveBeenCalled();
        expect(store.current.status).toBe('not-queried');
    });
});
