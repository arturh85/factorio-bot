<script setup lang="ts">
/**
 * The analysis view: joining a run's plan to what actually happened.
 *
 * This page loads its own copy of the run's events, map and samples rather
 * than sharing `runsStore` -- it answers "what went wrong", a different
 * question from the run viewer's "what did tick T look like", and does not
 * want that page's compare-run or playback state bleeding into this one. The
 * real work -- the join, the milestone grouping, the nearest-sample lookup --
 * lives in `@/lib/runDiff`, which is unit tested; this component is a thin
 * view over it.
 */
import {computed, onMounted, ref} from 'vue';
import {useRoute} from 'vue-router';
import {getRun, getRunEvents, getRunMap, getRunSamples} from '@/api/client';
import {Event, MapRecord, RunDetail, Sample} from '@/api/types';
import {
    DivergenceRow,
    FailureInventory,
    MilestoneRow,
    OutcomeRow,
    divergencesOf,
    inventoryAtFailure,
    joinRunOutcome,
    milestonesOf
} from '@/lib/runDiff';
import {boundsAt, entitiesAt} from '@/lib/runMap';
import {formatTicks} from '@/lib/runTimeline';
import MapPanel from '@/components/MapPanel.vue';

const route = useRoute();
const runId = computed(() => String(route.params.id));

const loading = ref(true);
const error = ref<string | null>(null);
const detail = ref<RunDetail | null>(null);
const events = ref<Event[]>([]);
const map = ref<MapRecord[]>([]);
const samples = ref<Sample[]>([]);
/** The tick the embedded map preview shows -- moved by the divergence list. */
const cursor = ref(0);

onMounted(load);

async function load() {
    loading.value = true;
    error.value = null;
    try {
        const [runDetail, eventsResponse, mapResponse, samplesResponse] = await Promise.all([
            getRun(runId.value),
            getRunEvents(runId.value),
            getRunMap(runId.value),
            getRunSamples(runId.value)
        ]);
        detail.value = runDetail;
        events.value = eventsResponse.events;
        map.value = mapResponse.map;
        samples.value = samplesResponse.samples;
        cursor.value = 0;
    } catch (err) {
        error.value = err instanceof Error ? err.message : String(err);
    } finally {
        loading.value = false;
    }
}

/**
 * The overrun table: every plan epoch of the run joined to its own outcomes
 * and concatenated -- never one join across the whole event log, because
 * action ids restart at zero with every `plan_created` and a whole-run join
 * would pair a row with whichever other plan happens to share its id. See
 * `@/lib/runDiff`'s module doc comment.
 */
const outcomes = computed<OutcomeRow[]>(() => joinRunOutcome(events.value));

const divergences = computed<DivergenceRow[]>(() => divergencesOf(map.value));

/** Per milestone: its plan, what ran under it (already epoch-scoped), and how it closed. */
const milestones = computed<MilestoneRow[]>(() => milestonesOf(events.value));

const failures = computed<FailureInventory[]>(() => inventoryAtFailure(events.value, samples.value));

function seekMap(tick: number) {
    cursor.value = tick;
}

const mapEntities = computed(() => entitiesAt(map.value, cursor.value));
const mapBounds = computed(() => boundsAt(map.value, cursor.value));

/** A tick delta, signed, or an em dash when there is none to show. */
function formatDelta(delta: number | null): string {
    if (delta === null) return '—';
    return delta > 0 ? `+${delta}` : `${delta}`;
}
</script>

<template>
    <div class="analysis">
        <header class="analysis__header">
            <h2>Analysis — {{ runId }}</h2>
            <router-link :to="`/runs`" class="linkish">back to runs</router-link>
        </header>

        <p v-if="loading" class="analysis__empty">loading…</p>
        <p v-else-if="error" class="analysis__error">{{ error }}</p>

        <template v-else>
            <section class="analysis__section">
                <h3>Overrun table</h3>
                <p v-if="outcomes.length === 0" class="analysis__empty">
                    no plan or settle events recorded for this run
                </p>
                <table v-else class="analysis__table">
                    <thead>
                        <tr>
                            <th>milestone</th><th>id</th><th>bot</th><th>action</th><th>planned</th>
                            <th>actual</th><th>delta</th><th>status</th><th>failure</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr
                            v-for="(row, i) in outcomes"
                            :key="`${row.milestoneIndex}-${row.id}-${i}`"
                            :class="{'is-never-ran': row.actualDuration === null}"
                        >
                            <!-- Ids restart at zero with every plan, so two rows can
                                 share both an id and an action name; the milestone
                                 they came from is what actually tells them apart. -->
                            <td class="num">{{ row.milestoneIndex ?? '—' }}</td>
                            <td class="num">{{ row.id }}</td>
                            <td class="num">{{ row.bot }}</td>
                            <td>{{ row.action }}</td>
                            <td class="num">{{ formatTicks(row.plannedDuration) }}</td>
                            <td class="num">{{ formatTicks(row.actualDuration) }}</td>
                            <td class="num">{{ formatDelta(row.delta) }}</td>
                            <td>{{ row.status }}</td>
                            <td>{{ row.failure ? `${row.failure.kind}${row.failure.detail ? ` — ${row.failure.detail}` : ''}` : '—' }}</td>
                        </tr>
                    </tbody>
                </table>
            </section>

            <section class="analysis__section">
                <h3>Divergence list</h3>
                <p v-if="divergences.length === 0" class="analysis__empty">
                    the model agreed with the game at every keyframe
                </p>
                <template v-else>
                    <table class="analysis__table">
                        <thead>
                            <tr><th>tick</th><th>only in</th><th>entity</th><th></th></tr>
                        </thead>
                        <tbody>
                            <tr v-for="(row, i) in divergences" :key="`${row.tick}-${i}`">
                                <td class="num">{{ row.tick }}</td>
                                <td>{{ row.divergence.only_in }}</td>
                                <td>{{ row.divergence.entity.name }}</td>
                                <td>
                                    <button type="button" class="linkish" @click="seekMap(row.tick)">
                                        show on map
                                    </button>
                                </td>
                            </tr>
                        </tbody>
                    </table>
                    <div class="analysis__map">
                        <MapPanel :entities="mapEntities" :bots="[]" :trail="{}" :bounds="mapBounds" />
                        <p class="analysis__caption num">map at tick {{ cursor }}</p>
                    </div>
                </template>
            </section>

            <section class="analysis__section">
                <h3>Milestone forensics</h3>
                <p v-if="milestones.length === 0" class="analysis__empty">
                    no milestones recorded for this run
                </p>
                <div v-for="milestone in milestones" :key="milestone.index" class="milestone">
                    <h4>#{{ milestone.index }} — {{ milestone.goal }}</h4>
                    <p v-if="milestone.iterations === 0" class="analysis__note">
                        satisfied with zero iterations —
                        <strong>{{ milestone.satisfiedReason }}</strong>
                        <template v-if="milestone.satisfiedReason === 'plan_empty'">
                            (the planner returned an empty plan, read as satisfied)
                        </template>
                        <template v-else-if="milestone.satisfiedReason === 'already_satisfied'">
                            (the world already met the goal)
                        </template>
                    </p>
                    <div class="milestone__columns">
                        <div>
                            <h5>Planned ({{ milestone.plan.length }} steps)</h5>
                            <ul v-if="milestone.plan.length > 0" class="milestone__list">
                                <li v-for="step in milestone.plan" :key="step.id">
                                    {{ step.id }}: {{ step.action }}
                                    <span v-if="step.deps.length > 0" class="analysis__note">
                                        (waits on {{ step.deps.join(', ') }})
                                    </span>
                                </li>
                            </ul>
                            <p v-else class="analysis__empty">no plan recorded</p>
                        </div>
                        <div>
                            <h5>Ran</h5>
                            <ul v-if="milestone.ran.length > 0" class="milestone__list">
                                <li v-for="row in milestone.ran" :key="row.id">
                                    {{ row.id }}: {{ row.action }} — {{ row.status }}
                                </li>
                            </ul>
                            <p v-else class="analysis__empty">nothing ran</p>
                        </div>
                    </div>
                </div>
            </section>

            <section class="analysis__section">
                <h3>Inventory at failure</h3>
                <p v-if="failures.length === 0" class="analysis__empty">no failed actions in this run</p>
                <div v-for="(failure, i) in failures" :key="`${failure.event.id}-${i}`" class="failure">
                    <h5>
                        action {{ failure.event.id }} — bot {{ failure.event.bot }} — tick {{ failure.event.tick }}
                        <span v-if="failure.event.failure" class="analysis__note">
                            ({{ failure.event.failure.kind }})
                        </span>
                    </h5>
                    <p v-if="!failure.inventory" class="analysis__empty">
                        no bot sample recorded for this bot
                    </p>
                    <ul v-else-if="Object.keys(failure.inventory.inventory).length > 0" class="milestone__list">
                        <li v-for="(count, item) in failure.inventory.inventory" :key="item">
                            {{ item }}: {{ count }}
                        </li>
                    </ul>
                    <p v-else class="analysis__empty">inventory empty at the nearest sample</p>
                </div>
            </section>
        </template>
    </div>
</template>

<style scoped>
.analysis {
    max-width: 60rem;
}
.analysis__header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 1rem;
}
.analysis__section {
    margin-bottom: 2rem;
}
.analysis__section h3 {
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
    text-transform: uppercase;
    opacity: 0.6;
}
.analysis__empty {
    font-size: 0.85rem;
    opacity: 0.6;
}
.analysis__error {
    color: #b91c1c;
}
.analysis__note {
    font-size: 0.8rem;
    opacity: 0.7;
}
.analysis__table {
    width: 100%;
    border-collapse: collapse;
}
.analysis__table th,
.analysis__table td {
    text-align: left;
    padding: 0.25rem 0.5rem;
    border-bottom: 1px solid var(--surface-border, #eee);
}
.analysis__table .num {
    font-family: monospace;
    text-align: right;
}
.analysis__table tr.is-never-ran {
    background: rgba(185, 28, 28, 0.08);
}
.linkish {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: #3b82f6;
    cursor: pointer;
    text-decoration: underline;
}
.analysis__map {
    margin-top: 0.75rem;
}
.analysis__caption {
    font-size: 0.8rem;
    opacity: 0.7;
}
.milestone {
    border: 1px solid var(--surface-border, #ccc);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.75rem;
}
.milestone h4 {
    margin: 0 0 0.35rem;
}
.milestone h5 {
    margin: 0 0 0.25rem;
    font-size: 0.8rem;
    opacity: 0.7;
}
.milestone__columns {
    display: flex;
    gap: 1.5rem;
    flex-wrap: wrap;
}
.milestone__columns > div {
    flex: 1 1 16rem;
    min-width: 12rem;
}
.milestone__list {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 0.85rem;
}
.failure {
    border: 1px solid var(--surface-border, #ccc);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.5rem;
}
.failure h5 {
    margin: 0 0 0.35rem;
    font-size: 0.85rem;
}
</style>
