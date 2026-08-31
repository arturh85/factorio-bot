// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {flushPromises, mount} from '@vue/test-utils';
import MapPage from './MapPage.vue';
import * as game from '@/api/game';
import {ApiError} from '@/api/http';
import {SPAWN_ENTITIES} from '@/api/game.fixtures';

// Partial mock -- see the identical comment in `mapStore.spec.ts`: a blanket
// `vi.mock('@/api/game')` would also mock `parseFactorioEntities`, which
// `game.fixtures.ts` needs for real to build `SPAWN_ENTITIES`.
vi.mock('@/api/game', async importOriginal => ({
    ...(await importOriginal<typeof import('@/api/game')>()),
    findEntities: vi.fn()
}));

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

function submit(wrapper: ReturnType<typeof mount>) {
    return wrapper.get('[data-testid="query-button"]').trigger('click');
}

describe('MapPage -- three empty states, all different (brief rule 4)', () => {
    it('shows a not-yet-queried state before any query has run, offering the query', () => {
        const wrapper = mount(MapPage);
        expect(wrapper.find('[data-testid="not-queried-state"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="not-running-state"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="empty-result-state"]').exists()).toBe(false);
        // The query control itself is the offered action.
        expect(wrapper.find('[data-testid="query-button"]').exists()).toBe(true);
    });

    it('shows "Factorio is not running" and nothing about entities when the API answers code 2', async () => {
        vi.mocked(game.findEntities).mockRejectedValue(
            new ApiError(503, 'not started', 2, {message: 'not started', code: 2})
        );
        const wrapper = mount(MapPage);

        await submit(wrapper);
        await flushPromises();

        const state = wrapper.get('[data-testid="not-running-state"]');
        expect(state.text().toLowerCase()).toContain('not running');
        expect(state.text().toLowerCase()).not.toContain('entit');
        expect(wrapper.find('[data-testid="not-queried-state"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="empty-result-state"]').exists()).toBe(false);
    });

    it('shows "the area is empty" when the instance is running but the query returns zero entities', async () => {
        vi.mocked(game.findEntities).mockResolvedValue([]);
        const wrapper = mount(MapPage);

        await submit(wrapper);
        await flushPromises();

        expect(wrapper.find('[data-testid="empty-result-state"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="not-running-state"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="not-queried-state"]').exists()).toBe(false);
    });

    it('renders all three empty states with different text', async () => {
        const wrapper = mount(MapPage);
        const notQueriedText = wrapper.get('[data-testid="not-queried-state"]').text();

        vi.mocked(game.findEntities).mockRejectedValue(
            new ApiError(503, 'not started', 2, {message: 'not started', code: 2})
        );
        await submit(wrapper);
        await flushPromises();
        const notRunningText = wrapper.get('[data-testid="not-running-state"]').text();

        vi.mocked(game.findEntities).mockResolvedValue([]);
        await submit(wrapper);
        await flushPromises();
        const emptyResultText = wrapper.get('[data-testid="empty-result-state"]').text();

        expect(new Set([notQueriedText, notRunningText, emptyResultText]).size).toBe(3);
    });
});

describe('MapPage -- snapshot labelling (brief rule 3)', () => {
    it('shows when the result was fetched, and a refresh control', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const wrapper = mount(MapPage);

        await submit(wrapper);
        await flushPromises();

        expect(wrapper.find('[data-testid="fetched-at"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="refresh-button"]').exists()).toBe(true);
    });

    it('does not auto-refresh: findEntities is called exactly once for one query', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const wrapper = mount(MapPage);

        await submit(wrapper);
        await flushPromises();

        expect(game.findEntities).toHaveBeenCalledTimes(1);
    });

    it('refresh re-issues the query and updates the fetched-at time', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const wrapper = mount(MapPage);
        await submit(wrapper);
        await flushPromises();

        await wrapper.get('[data-testid="refresh-button"]').trigger('click');
        await flushPromises();

        expect(game.findEntities).toHaveBeenCalledTimes(2);
    });
});

describe('MapPage -- shows a count of what is shown', () => {
    it('shows the number of entities found', async () => {
        vi.mocked(game.findEntities).mockResolvedValue(SPAWN_ENTITIES);
        const wrapper = mount(MapPage);

        await submit(wrapper);
        await flushPromises();

        expect(wrapper.get('[data-testid="entity-count"]').text()).toContain(String(SPAWN_ENTITIES.length));
    });
});

describe('MapPage -- entity_type filter', () => {
    it('sends the typed entity_type filter as part of the query', async () => {
        vi.mocked(game.findEntities).mockResolvedValue([]);
        const wrapper = mount(MapPage);

        await wrapper.get('[data-testid="entity-type-input"]').setValue('resource');
        await submit(wrapper);
        await flushPromises();

        expect(game.findEntities).toHaveBeenCalledWith(
            expect.objectContaining({entity_type: 'resource'})
        );
    });
});
