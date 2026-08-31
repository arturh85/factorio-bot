// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import AppTopbar from './AppTopbar.vue';

vi.mock('@/api/client');

const settingsFixture = (): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 2,
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
});

describe('AppTopbar', () => {
    it('emits menu-toggle when the hamburger is pressed', async () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        await wrapper.get('[data-testid="menu-toggle"]').trigger('click');
        expect(wrapper.emitted('menu-toggle')).toHaveLength(1);
    });

    it('names the toggle for assistive tech', () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.get('[data-testid="menu-toggle"]').attributes('aria-label')).toBe('Toggle menu');
    });

    it('shows the configured client count once settings are loaded', () => {
        const store = useAppStore();
        store.settings = settingsFixture();
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.text()).toContain('2 Clients');
    });

    it('shows no client count before settings arrive', () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.text()).not.toContain('Clients');
    });

    it('indents itself past the sidebar only while the sidebar is open', () => {
        const open = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        const closed = mount(AppTopbar, {props: {sidebarOpen: false}, global: {stubs: {ProcessControl: true}}});
        expect(open.classes()).toContain('lg:left-sidebar');
        expect(closed.classes()).not.toContain('lg:left-sidebar');
    });
});
