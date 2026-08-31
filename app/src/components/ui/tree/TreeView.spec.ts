// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import type {ScriptTreeNode} from '@/api/types';
import TreeView from './TreeView.vue';

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string, children: ScriptTreeNode[] = []): ScriptTreeNode =>
    ({key, label, leaf: false, children});

const nodes = [dir('/sub', 'sub', [leaf('/sub/b.lua', 'b.lua')]), leaf('/a.lua', 'a.lua')];

function mountTree(expandedKeys: string[] = [], selectedKey: string | null = null) {
    return mount(TreeView, {props: {nodes, expandedKeys, selectedKey}});
}

describe('TreeView', () => {
    it('renders one row per node and marks the root list as a tree', () => {
        const wrapper = mountTree();
        expect(wrapper.element.getAttribute('role')).toBe('tree');
        expect(wrapper.findAll('[role="treeitem"]').length).toBe(2);
        expect(wrapper.findAll('button')[1].text()).toContain('a.lua');
    });

    it('hides the children of a collapsed directory', () => {
        const wrapper = mountTree();
        expect(wrapper.find('[role="group"]').exists()).toBe(false);
        expect(wrapper.text()).not.toContain('b.lua');
    });

    it('renders the children of an expanded directory in a nested group', () => {
        const wrapper = mountTree(['/sub']);
        expect(wrapper.find('[role="group"]').exists()).toBe(true);
        expect(wrapper.text()).toContain('b.lua');
    });

    it('reports expansion state on the directory row only', () => {
        const wrapper = mountTree(['/sub']);
        const rows = wrapper.findAll('[role="treeitem"]');
        expect(rows[0].attributes('aria-expanded')).toBe('true');
        expect(rows[1].attributes('aria-expanded')).toBeUndefined();
    });

    it('emits toggle for a directory and select for a file', async () => {
        const wrapper = mountTree();
        const buttons = wrapper.findAll('button');

        await buttons[0].trigger('click');
        await buttons[1].trigger('click');

        expect(wrapper.emitted('toggle')?.[0][0]).toMatchObject({key: '/sub'});
        expect(wrapper.emitted('select')?.[0][0]).toMatchObject({key: '/a.lua'});
        expect(wrapper.emitted('select')?.length).toBe(1);
    });

    it('marks the selected file and only that file', () => {
        const wrapper = mountTree([], '/a.lua');
        const rows = wrapper.findAll('[role="treeitem"]');
        expect(rows[1].attributes('aria-selected')).toBe('true');
        expect(rows[0].attributes('aria-selected')).toBe('false');
    });

    it('bubbles a nested selection up through the recursion', async () => {
        const wrapper = mountTree(['/sub']);
        const nested = wrapper.find('[role="group"] button');
        await nested.trigger('click');
        expect(wrapper.emitted('select')?.[0][0]).toMatchObject({key: '/sub/b.lua'});
    });
});
