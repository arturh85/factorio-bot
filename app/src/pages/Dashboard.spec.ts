// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import Dashboard from './Dashboard.vue';

vi.mock('@/api/client');

const settingsFixture = (clientCount: number): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: clientCount,
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

describe('Dashboard', () => {
    it('renders one client tile per configured client, plus the summary tile', () => {
        const store = useAppStore();
        store.settings = settingsFixture(3);
        const wrapper = mount(Dashboard);

        expect(wrapper.get('[data-testid="instance-count"]').text()).toBe('3');
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(3);
        expect(wrapper.findAll('[data-testid="client-tile"]')[2].text()).toContain('client3');
    });

    it('follows a client-count change', async () => {
        const store = useAppStore();
        store.settings = settingsFixture(1);
        const wrapper = mount(Dashboard);
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(1);

        store.settings = settingsFixture(4);
        await wrapper.vm.$nextTick();

        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(4);
    });

    it('renders nothing client-shaped before settings have loaded', () => {
        const wrapper = mount(Dashboard);
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(0);
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const store = useAppStore();
        store.settings = settingsFixture(2);
        const wrapper = mount(Dashboard);
        const html = wrapper.html();
        for (const dead of ['p-grid', 'p-col-12', 'p-lg-4', 'p-fluid']) {
            expect(html).not.toContain(dead);
        }
    });

    it('does not fabricate a per-client status: a tile carries only the client name', () => {
        const store = useAppStore();
        store.settings = settingsFixture(3);
        const wrapper = mount(Dashboard);
        const html = wrapper.html();

        // `GET /api/v1/instance` reports for the group, not per client -- there
        // is no per-client status to show, so none may be invented. This is a
        // regression guard for a hardcoded 'not_initialized' constant that used
        // to be printed on every tile regardless of what was actually running.
        expect(html).not.toContain('not_initialized');
        const tiles = wrapper.findAll('[data-testid="client-tile"]');
        expect(tiles[0].text()).toBe('client1');
        expect(tiles[1].text()).toBe('client2');
        expect(tiles[2].text()).toBe('client3');
    });

    /**
     * The count is a figure, not a control.
     *
     * It used to be a `<button>` wired to a toast reading "Info Message /
     * Message Content" — Sigma-template demo wiring that survived the
     * redesign because porting it unchanged was the right call for a
     * restyling task, and removing it was a separate decision.
     *
     * Asserted rather than left to inspection: a clickable-looking count
     * promises an action, and the next person to want one here should have to
     * change a test that says so.
     */
    it('renders the instance count as a figure, not something clickable', () => {
        const store = useAppStore();
        store.settings = settingsFixture(2);
        const wrapper = mount(Dashboard);

        const count = wrapper.get('[data-testid="instance-count"]');
        expect(count.element.tagName).toBe('SPAN');
        expect(wrapper.find('[data-testid="instance-count"] button').exists()).toBe(false);
        expect(wrapper.html()).not.toContain('Info Message');
    });
});
