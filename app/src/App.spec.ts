// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import App from './App.vue';
import AppTopbar from './AppTopbar.vue';

vi.mock('@/api/client');

const settings = {
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 1,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
};

const instance = {
    started: false,
    starting: false,
    client_count: 1,
    server_port: null,
    rcon_port: null,
    last_error: null
};

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    vi.mocked(client.getSettings).mockResolvedValue(settings);
    vi.mocked(client.getInstance).mockResolvedValue(instance);
});

async function mountApp() {
    const wrapper = mount(App, {
        global: {stubs: {RouterLink: true, RouterView: true, ProcessControl: true}}
    });
    await flushPromises();
    return wrapper;
}

describe('App shell', () => {
    it('starts with the sidebar shown on a desktop-width window', async () => {
        const wrapper = await mountApp();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('translate-x-0');
    });

    it('slides the sidebar out and back when the topbar asks', async () => {
        const wrapper = await mountApp();

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('-translate-x-full');

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('translate-x-0');
    });

    it('gives the main region the sidebar margin only while the sidebar is open', async () => {
        const wrapper = await mountApp();
        expect(wrapper.get('main').classes()).toContain('lg:ml-sidebar');

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('main').classes()).not.toContain('lg:ml-sidebar');
    });

    it('mounts exactly one toast host', async () => {
        const wrapper = await mountApp();
        expect(wrapper.findAll('[data-testid="toaster"]').length).toBe(1);
    });
});
