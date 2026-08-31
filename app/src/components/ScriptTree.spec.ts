// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import type {ScriptTreeNode} from '@/api/types';
import ScriptTree from './ScriptTree.vue';

vi.mock('@/api/client');

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: false, children: []});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

async function mountTree() {
    const wrapper = mount(ScriptTree);
    await flushPromises();
    return wrapper;
}

describe('ScriptTree', () => {
    it('lists the workspace root on mount', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        expect(client.listScripts).toHaveBeenCalledWith('/');
        expect(wrapper.findAll('[role="treeitem"]').length).toBe(2);
    });

    it('fetches a directory the first time it is expanded and shows its files', async () => {
        vi.mocked(client.listScripts)
            .mockResolvedValueOnce([dir('/sub', 'sub')])
            .mockResolvedValueOnce([leaf('/sub/b.lua', 'b.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(client.listScripts).toHaveBeenLastCalledWith('/sub');
        expect(wrapper.text()).toContain('b.lua');
    });

    it('does not re-fetch a directory that is collapsed and expanded again', async () => {
        vi.mocked(client.listScripts)
            .mockResolvedValueOnce([dir('/sub', 'sub')])
            .mockResolvedValueOnce([leaf('/sub/b.lua', 'b.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();
        await wrapper.get('button').trigger('click');
        await flushPromises();
        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(vi.mocked(client.listScripts).mock.calls.map(call => call[0])).toEqual(['/', '/sub']);
        expect(wrapper.text()).toContain('b.lua');
    });

    it('emits the key of a selected file and nothing for a directory', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        await wrapper.findAll('button')[1].trigger('click');
        await wrapper.findAll('button')[0].trigger('click');
        await flushPromises();

        expect(wrapper.emitted('select')).toEqual([['/a.lua']]);
    });

    it('keeps the last selected file marked', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(wrapper.get('[role="treeitem"]').attributes('aria-selected')).toBe('true');
    });

    it('reports a failed listing instead of leaving an empty tree', async () => {
        vi.mocked(client.listScripts).mockRejectedValue(new Error('scripts directory is unreadable'));
        const wrapper = await mountTree();

        expect(wrapper.text()).toContain('scripts directory is unreadable');
    });
});
