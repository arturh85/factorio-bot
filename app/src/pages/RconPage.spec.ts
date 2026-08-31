// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useRconStore} from '@/store/rconStore';
import {toastMessages, useToast} from '@/composables/useToast';
import RconPage from './RconPage.vue';

vi.mock('@/api/client');

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    useToast().clear();
});

afterEach(() => useToast().clear());

describe('RconPage', () => {
    it('sends the typed command when Run is pressed', async () => {
        const store = useRconStore();
        const execute = vi.spyOn(store, 'execute').mockResolvedValue(undefined);
        const wrapper = mount(RconPage);

        await wrapper.get('textarea').setValue('/server-save');
        await wrapper.get('[data-testid="run-button"]').trigger('click');

        expect(execute).toHaveBeenCalledWith('/server-save');
    });

    it('sends the cheat command wired to its own button', async () => {
        const store = useRconStore();
        const execute = vi.spyOn(store, 'execute').mockResolvedValue(undefined);
        const wrapper = mount(RconPage);

        await wrapper.get('[data-testid="cheat-furnaces"]').trigger('click');

        expect(execute).toHaveBeenCalledWith(
            '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)'
        );
    });

    it('reports a failed command as an error toast instead of silently', async () => {
        const store = useRconStore();
        vi.spyOn(store, 'execute').mockRejectedValue(new Error('no Factorio instance is running'));
        const wrapper = mount(RconPage);

        await wrapper.get('[data-testid="run-button"]').trigger('click');
        await flushPromises();

        expect(toastMessages.length).toBe(1);
        expect(toastMessages[0].severity).toBe('error');
        expect(toastMessages[0].detail).toBe('no Factorio instance is running');
    });

    it('disables Run and renames it while a command is in flight', async () => {
        const store = useRconStore();
        store.executing = true;
        const wrapper = mount(RconPage);

        const run = wrapper.get('[data-testid="run-button"]');
        expect(run.attributes('disabled')).toBeDefined();
        expect(run.text()).toBe('Running ...');
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const wrapper = mount(RconPage);
        expect(wrapper.html()).not.toContain('p-grid');
        expect(wrapper.html()).not.toContain('p-col');
    });
});
