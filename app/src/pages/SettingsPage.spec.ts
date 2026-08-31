// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import SettingsPage from './SettingsPage.vue';

vi.mock('@/api/client');

// jsdom does not implement ResizeObserver, which reka-ui's SliderRoot (behind
// the client-count Slider on this page) needs on mount to measure the track.
class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}
globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

const settingsFixture = (): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 4,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    vi.mocked(client.pathExists).mockResolvedValue({exists: true});
});

async function mountPage() {
    const store = useAppStore();
    store.settings = settingsFixture();
    const wrapper = mount(SettingsPage);
    await flushPromises();
    return {store, wrapper};
}

describe('SettingsPage', () => {
    it('shows the persisted values in the fields', async () => {
        const {wrapper} = await mountPage();
        const values = wrapper.findAll('input').map(input => (input.element as HTMLInputElement).value);
        expect(values).toContain('/tmp/factorio.tar.xz');
        expect(values).toContain('/tmp/workspace');
        expect(values).toContain('1234');
    });

    it('persists a new seed through the store', async () => {
        const {store, wrapper} = await mountPage();
        const updateSeed = vi.spyOn(store, 'updateSeed').mockResolvedValue(undefined);
        await wrapper.get('[data-testid="seed-input"]').setValue('abcd');
        expect(updateSeed).toHaveBeenCalledWith('abcd');
    });

    it('persists an autostart change through the store', async () => {
        const {store, wrapper} = await mountPage();
        const updateEnableAutostart = vi.spyOn(store, 'updateEnableAutostart').mockResolvedValue(undefined);
        await wrapper.get('[data-testid="autostart-checkbox"] [role="checkbox"]').trigger('click');
        expect(updateEnableAutostart).toHaveBeenCalledWith(true);
    });

    it('flags a workspace path the server cannot see', async () => {
        vi.mocked(client.pathExists).mockResolvedValue({exists: false});
        const {wrapper} = await mountPage();
        expect(wrapper.text()).toContain('no such directory on the server');
        expect(wrapper.get('[data-testid="workspace-input"]').attributes('aria-invalid')).toBe('true');
    });

    it('flags a factorio archive path the server cannot see', async () => {
        vi.mocked(client.pathExists).mockResolvedValue({exists: false});
        const {wrapper} = await mountPage();
        expect(wrapper.text()).toContain('no such file on the server');
        expect(wrapper.get('[data-testid="archive-input"]').attributes('aria-invalid')).toBe('true');
    });

    it('shows the client count next to its slider', async () => {
        const {wrapper} = await mountPage();
        expect(wrapper.get('[data-testid="client-count-label"]').text()).toBe('Client Instances: 4');
    });

    it('treats an unloaded recreate-level setting as unchecked, matching ProcessControl', async () => {
        const store = useAppStore();
        store.settings = {...settingsFixture(), factorio: {...settingsFixture().factorio, recreate: undefined as unknown as boolean}};
        const wrapper = mount(SettingsPage);
        await flushPromises();
        expect(wrapper.get('[data-testid="recreate-checkbox"] [role="checkbox"]').attributes('aria-checked')).toBe('false');
    });

    it('carries no PrimeFlex grid classes any more', async () => {
        const {wrapper} = await mountPage();
        const html = wrapper.html();
        for (const dead of ['p-grid', 'p-col', 'p-formgrid', 'p-field', 'p-fluid', 'p-inputgroup', 'p-invalid']) {
            expect(html).not.toContain(dead);
        }
    });
});
