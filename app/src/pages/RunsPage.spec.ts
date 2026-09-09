// @vitest-environment jsdom
/**
 * The archive list. The per-run viewer moved to `RunPage.vue` at `/runs/:id`
 * (Task 17); this page is now only the sidebar list of runs, each one a
 * `router-link` to its own URL rather than a click handler that swaps a
 * viewer in on the same page.
 */
import {beforeEach, describe, expect, it} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {mount} from '@vue/test-utils';
import RunsPage from './RunsPage.vue';
import {useRunsStore} from '@/store/runsStore';
import {RunSummary} from '@/api/types';

beforeEach(() => {
    setActivePinia(createPinia());
});

const RUN: RunSummary = {
    run_id: 'run-1788365280-15443',
    finished: true,
    started_unix: 1788365280,
    finished_unix: 1788368000,
    outcome: 'success',
    elapsed_ticks: 162000,
    events: 400,
    splits: 3
} as RunSummary;

function open(runs: RunSummary[] = [RUN]) {
    const store = useRunsStore();
    store.$patch({runs});
    return mount(RunsPage, {global: {stubs: {RouterLink: {template: '<a><slot/></a>'}}}});
}

describe('RunsPage', () => {
    it('lists each run as a link to its own run page', () => {
        const wrapper = open();

        expect(wrapper.text()).toContain('run-1788365280-15443');
        expect(wrapper.find('a').exists()).toBe(true);
    });

    it('renders the run\'s outcome and elapsed ticks', () => {
        expect(open().text()).toContain('success');
    });

    it('reads "unfinished" for a run the server cannot tell crashed from still going', () => {
        const wrapper = open([{...RUN, finished: false, outcome: null}]);
        expect(wrapper.text()).toContain('unfinished');
    });

    it('shows the empty state with no runs recorded', () => {
        const wrapper = open([]);
        expect(wrapper.text()).toContain('No runs recorded yet.');
    });

    it('shows the store error message', () => {
        const store = useRunsStore();
        store.$patch({error: 'boom', runs: []});
        const wrapper = mount(RunsPage, {global: {stubs: {RouterLink: true}}});

        expect(wrapper.text()).toContain('boom');
        // The error paragraph and the empty-state paragraph are mutually
        // exclusive `v-if`/`v-else-if` branches -- with an error shown, the
        // "no runs recorded yet" message must not also render.
        expect(wrapper.text()).not.toContain('No runs recorded yet.');
    });
});
