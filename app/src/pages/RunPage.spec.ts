// @vitest-environment jsdom
/**
 * The run page assembled, not its bands in isolation.
 *
 * Two things can only be wrong HERE and are invisible to a band spec: which
 * clock the page hands each band (the ribbon said 6:03 while the headline
 * said 6:06 of the same tick), and whether a failed `/samples` fetch is
 * reported by every band that needs it. Both are asserted below against the
 * real archived run.
 */
import {describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {createMemoryHistory, createRouter} from 'vue-router';
import {flushPromises, mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import {RunDetail} from '@/api/types';
import {useRunsStore} from '@/store/runsStore';
import ProductionBand from '@/components/run/ProductionBand.vue';
import PowerBand from '@/components/run/PowerBand.vue';
import ResearchBand from '@/components/run/ResearchBand.vue';
import MachineBand from '@/components/run/MachineBand.vue';
import MapPanel from '@/components/MapPanel.vue';
import {machineRows, positionKey} from '@/lib/machineTimeline';
import RunPage from './RunPage.vue';

const run = loadFixtureRun();
const RUN_ID = 'run-1788696619-00325';

/**
 * The splits the server derives for this run, read off its own events:
 * `milestone_started` at 3242 and `milestone_satisfied` at 25216.
 */
const DETAIL: RunDetail = {
    summary: {
        run_id: RUN_ID, finished: true, started_unix: 1788696619, finished_unix: 1788697000,
        outcome: 'satisfied', elapsed_ticks: 21982, events: run.events.length, splits: 1
    },
    splits: [{index: 1, goal: 'research automation', started_tick: 3242, ended_tick: 25216, outcome: 'satisfied', elapsed_ticks: 21974}]
};

// The factory is hoisted above the imports, but nothing in it touches the
// module scope until one of these functions is CALLED, which is after the
// body has run.
vi.mock('@/api/client', () => ({
    listRuns: async () => ({runs: []}),
    getRun: async () => DETAIL,
    getRunLanes: async () => ({lanes: run.lanes, skipped: 0}),
    getRunSamples: async () => ({samples: run.samples, skipped: 0}),
    getRunMap: async () => ({map: [], skipped: 0}),
    getRunVideo: async () => { throw new Error('this run recorded no video'); },
    getRunVideoTicks: async () => { throw new Error('this run recorded no video'); },
    getRunEvents: async () => ({events: run.events, skipped: 0})
}));

async function mountPage() {
    setActivePinia(createPinia());
    const router = createRouter({
        history: createMemoryHistory(),
        routes: [
            {path: '/runs/:id', name: 'run', component: RunPage},
            {path: '/runs/:id/analysis', name: 'analysis', component: {template: '<div/>'}}
        ]
    });
    router.push(`/runs/${RUN_ID}`);
    await router.isReady();
    const wrapper = mount(RunPage, {global: {plugins: [router]}});
    await flushPromises();
    return wrapper;
}

describe('RunPage', () => {
    it('reads the headline and the milestone ribbon off ONE clock', async () => {
        const w = await mountPage();
        const store = useRunsStore();
        // The drawn axis is trimmed past `run_started`, which is exactly the
        // condition under which the two used to disagree.
        expect(store.bounds!.from).toBeGreaterThan(store.window!.lo);
        expect(w.text()).toContain('satisfied at 6:06');
        const seg = w.get('.seg');
        expect(seg.text()).toContain('satisfied at 6:06');
        expect(seg.text()).not.toContain('6:03');
    });

    it('translates a selected machine into the position key the map joins on', async () => {
        // `selectedMachine` is the machines-sample key (`unit_number`); the map
        // knows only positions. Nothing translated between them, so selecting
        // a row changed nothing a reader could see.
        const w = await mountPage();
        const store = useRunsStore();
        expect(w.findComponent(MapPanel).props('highlight')).toBeNull();
        const row = machineRows(store.samples)[0];
        store.selectMachine(row.key);
        await flushPromises();
        expect(row.key).toMatch(/^\d+$/);
        expect(w.findComponent(MapPanel).props('highlight')).toBe(positionKey(row.position));
    });

    it('reports a failed /samples fetch in every band that needs samples', async () => {
        const w = await mountPage();
        const store = useRunsStore();
        store.sampleError = 'world-state samples unavailable — x';
        await flushPromises();
        for (const band of [ProductionBand, PowerBand, ResearchBand, MachineBand]) {
            expect(w.findComponent(band).exists()).toBe(false);
        }
        const notices = w.findAll('p').filter((p) => p.text() === 'world-state samples unavailable — x');
        expect(notices).toHaveLength(4);
    });
});
