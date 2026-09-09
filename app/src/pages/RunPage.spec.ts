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
        outcome: 'satisfied', elapsed_ticks: 21982, events: run.events.length, splits: 1,
        samples: null, map: null, samples_lag_ticks: null
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
    getRunEvents: async () => ({events: run.events, skipped: 0}),
    // The page renders chips off provenance and plan counts off the replay
    // (see the seed-chip and plan-chip tests below); savepoints render a
    // resume chip nothing here asserts on. Resolved rather than left
    // undefined so `openRun`'s `Promise.allSettled` array can be built at
    // all -- the fixed values below are just what this file does not
    // otherwise exercise.
    getRunProvenance: async () => ({
        schema: 1, run_id: RUN_ID, started_unix: 1788696619, started_tick: 0,
        seed: null, map_exchange_string: null, map: null, factorio: null, mods: null,
        git: null, profile: 'release', roster_requested: [], workspace: null,
        resumed_from: null, bot_mode: null, game_speed: null, peaceful: null
    }),
    getRunReplay: async () => ({planned_makespan: 0, refused: null, unmatched_walks: [], steps: []}),
    getRunSavepoints: async () => ({savepoints: [], skipped: 0, missing_zip: []}),
    // Nothing here asserts on the Flow tab; resolved the same way the other
    // untested enrichments above are, so `Promise.allSettled` has something
    // to await from every one of them.
    getRunFlow: async () => ({flow: [], skipped: 0})
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
    it('reads the seed chip off store.provenance once it loads', async () => {
        const w = await mountPage();
        const store = useRunsStore();
        store.provenance = {
            schema: 1, run_id: RUN_ID, started_unix: 1788696619, started_tick: 0,
            seed: '31337', map_exchange_string: null, map: null, factorio: '2.1.17',
            mods: {base: '2.1.17', BotBridge: '0.0.1'}, git: null, profile: 'release',
            roster_requested: [1, 2, 3, 4], workspace: null, resumed_from: null,
            bot_mode: 'clients', game_speed: 1, peaceful: null
        };
        await flushPromises();
        expect(w.get('[data-chip="seed"]').text()).toContain('31337');
    });

    it('flags the plan chip truncated once the replay carries an abandoned step', async () => {
        const w = await mountPage();
        const store = useRunsStore();
        store.replay = {
            planned_makespan: 10,
            refused: null,
            unmatched_walks: [],
            steps: [{
                index: 0, bot: 1, bot_step_index: 0, what: {kind: 'act', action: 1, label: 'x'},
                planned_start_tick: 0, planned_end_tick: 1, observed_start_tick: null, observed_end_tick: null,
                status: 'Abandoned', attempt_number: null, evidence: {kind: 'believed', why: 'predecessor failed'}, error: 'x'
            }]
        };
        await flushPromises();
        const plan = w.get('[data-chip="plan"]');
        expect(plan.attributes('data-state')).toBe('truncated');
        expect(plan.text()).toContain('1 abandoned');
    });


    it('hands a failed replay fetch to the headline, which draws it', async () => {
        // The page is the only place that can be wrong about this: the store
        // records `replayError` and the headline can render it, and neither
        // half is any use if the page never passes it across.
        const w = await mountPage();
        const store = useRunsStore();
        store.replay = null;
        store.replayError = 'replay unavailable — this server does not provide /replay';
        await flushPromises();
        const plan = w.get('[data-chip="plan"]');
        expect(plan.attributes('data-state')).toBe('absent');
        expect(plan.text()).toContain('/replay');
    });

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

    it('shows the planner refusal behind the run\'s last stuck milestone', async () => {
        const w = await mountPage();
        const store = useRunsStore();
        store.detail = {
            ...DETAIL,
            splits: [{index: 1, goal: 'sustain iron-plate', started_tick: 3242, ended_tick: 25216, outcome: 'stuck', elapsed_ticks: 21974}]
        };
        store.events = [
            ...store.events,
            {
                kind: 'milestone_stuck', index: 1, outcome: 'stuck', best_steps: 12,
                last_error: 'nothing can carry coal from the buffer at [32.5,-41.5] to the iron-chest at [27.5,-40.5]: no belt route, blocked by 1 tile(s): [30.5,-39.5]',
                tick: 25216
            }
        ];
        await flushPromises();
        expect(w.get('[data-testid="stuck-reason"]').text()).toContain('no belt route, blocked by 1 tile');
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
