// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {SplitterGroup} from 'reka-ui';
import {useScriptStore} from '@/store/scriptStore';
import ScriptPage from './ScriptPage.vue';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');
// `stubs` below only replaces what vue-test-utils *renders* -- ScriptPage's
// own `<script setup>` still has a static `import Editor from
// '@/components/Editor.vue'`, which runs at module-load time regardless of
// any stub and pulls in monaco-editor. Monaco touches
// `document.queryCommandSupported` and other clipboard/DOM APIs jsdom does
// not implement, so it must be mocked out before that import ever happens.
vi.mock('@/components/Editor.vue', () => ({default: {template: '<div/>'}}));

// Monaco needs a real canvas and web workers, and ScriptTree talks to the
// store on mount; neither is what this page's own layout test is about.
const stubs = {Editor: true, ScriptTree: true};

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('ScriptPage', () => {
    it('shows only the tree until a script is opened', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.find('[data-testid="script-tree-pane"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="editor-pane"]').exists()).toBe(false);
    });

    it('shows the editor and output panes once a script is active', () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.find('[data-testid="editor-pane"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="output-pane"]').exists()).toBe(true);
    });

    it('persists the pane layout under a stable id', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.findComponent(SplitterGroup).props('autoSaveId')).toBe('luaScriptSplitter');
        expect(wrapper.findComponent(SplitterGroup).props('direction')).toBe('horizontal');
    });

    it('runs the active script when Run is pressed', async () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        const executeScript = vi.spyOn(store, 'executeScript').mockResolvedValue(undefined);
        const wrapper = mount(ScriptPage, {global: {stubs}});

        await wrapper.get('[data-testid="run-button"]').trigger('click');

        expect(executeScript).toHaveBeenCalledTimes(1);
    });

    it('renders stderr lines separately from stdout lines', () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        store.stdout = 'hello\nworld';
        store.stderr = 'careful';
        const wrapper = mount(ScriptPage, {global: {stubs}});

        expect(wrapper.findAll('[data-testid="stdout-line"]').length).toBe(2);
        expect(wrapper.findAll('[data-testid="stderr-line"]').length).toBe(1);
        expect(wrapper.get('[data-testid="stderr-line"]').text()).toBe('careful');
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.html()).not.toContain('p-grid');
        expect(wrapper.html()).not.toContain('p-col');
    });
});
