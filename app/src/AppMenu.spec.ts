// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount, RouterLinkStub} from '@vue/test-utils';
import {Cog, Home, Network} from '@lucide/vue';
import AppMenu from './AppMenu.vue';
import type {MenuEntry} from '@/models/dashboard';

const items: MenuEntry[] = [
    {label: 'Dashboard', icon: Home, to: '/'},
    {label: 'Settings', icon: Cog, to: '/settings'},
    {label: 'Tasks', icon: Network, to: '/tasks'}
];

describe('AppMenu', () => {
    it('renders one link per entry, in order, pointing where the entry says', () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        const links = wrapper.findAllComponents(RouterLinkStub);
        expect(links.length).toBe(3);
        expect(links[2].props('to')).toBe('/tasks');
        expect(links[2].text()).toContain('Tasks');
    });

    it('renders the entry icon component, not an icon-font class string', () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        expect(wrapper.findComponent(Home).exists()).toBe(true);
        expect(wrapper.html()).not.toContain('pi-fw');
    });

    it('emits navigate when a link is clicked, so the mobile sidebar can close', async () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        await wrapper.findAllComponents(RouterLinkStub)[0].trigger('click');

        expect(wrapper.emitted('navigate')).toHaveLength(1);
    });
});
