<script setup lang="ts">
/**
 * Viewing an archived run: splits, a tick scrubber, and the frames.
 *
 * Everything reads one cursor, in game ticks. Wall time appears only as "when
 * was this run" -- two runs are compared on the game's clock, because a
 * headless server and a graphical client with three cameras do not run at the
 * same speed.
 */
import {computed, onBeforeUnmount, onMounted, ref, watch} from 'vue';
import {useRunsStore} from '@/store/runsStore';
import {runFrameUrl} from '@/api/client';
import {
    botsOf,
    camerasOf,
    formatAgo,
    formatTicks,
    formatWhen,
    fractionOf,
    frameAt,
    laneAt,
    laneBots,
    splitAt,
    startedUnixOf,
    compareSplits
} from '@/lib/runTimeline';

const store = useRunsStore();
const selected = ref<string | null>(null);
let timer: number | null = null;

// Refreshed every half minute so a page left open does not keep insisting a
// run happened "just now" an hour later.
const nowUnix = ref(Math.floor(Date.now() / 1000));
let clock: number | null = null;

onMounted(() => {
    store.loadRuns();
    clock = window.setInterval(() => (nowUnix.value = Math.floor(Date.now() / 1000)), 30_000);
});
onBeforeUnmount(() => {
    stopTimer();
    if (clock !== null) window.clearInterval(clock);
});

function stopTimer() {
    if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
    }
}

watch(
    () => store.playing,
    (playing) => {
        stopTimer();
        // Ten steps a second: fast enough to read as playback, slow enough
        // that each step lands on a frame the eye can register.
        if (playing) timer = window.setInterval(() => store.advance(), 100);
    }
);

async function open(id: string) {
    selected.value = id;
    await store.openRun(id);
}

const bots = computed(() => botsOf(store.placedFrames));
const cameras = computed(() => camerasOf(store.placedFrames));

const current = computed(() =>
    store.bot === null || store.camera === null
        ? null
        : frameAt(store.placedFrames, store.bot, store.camera, store.cursor)
);

const currentSplit = computed(() => splitAt(store.detail?.splits ?? [], store.cursor));

/** The earliest frame for the current bot and camera, for the empty state. */
const firstForSelection = computed(() => {
    if (store.bot === null || store.camera === null) return null;
    const mine = store.placedFrames.filter(
        (f) => f.bot === store.bot && f.camera === store.camera
    );
    return mine.length > 0 ? mine[0].tick : null;
});

const frameSrc = computed(() =>
    current.value && selected.value
        ? runFrameUrl(selected.value, current.value.bot, current.value.file)
        : null
);

/**
 * Splits with a delta column when a reference run is chosen.
 *
 * Matched by goal name, so a run that skipped or reordered a milestone does
 * not line up against whatever happened to sit at the same index.
 */
const deltas = computed(() =>
    store.reference === null
        ? null
        : new Map(
              compareSplits(store.detail?.splits ?? [], store.reference.splits).map((row) => [
                  row.goal,
                  row
              ])
          )
);

const otherRuns = computed(() =>
    store.runs.filter((r) => r.run_id !== store.detail?.summary.run_id)
);

const lanesByBot = computed(() =>
    laneBots(store.lanes).map((bot) => ({
        bot,
        entries: store.lanes.filter((l) => l.bot === bot),
        now: laneAt(store.lanes, bot, store.cursor)
    }))
);

/**
 * A lane's span as left/width percentages.
 *
 * An unterminated span runs to the end of the axis, because as far as the
 * record goes the bot never stopped -- drawing it as a sliver at its start
 * would hide the very stretch that went wrong.
 */
function laneStyle(from: number, to: number | null) {
    const bounds = store.bounds;
    if (!bounds) return {left: '0%', width: '0%'};
    const start = fractionOf(bounds, from);
    const end = to === null ? 1 : fractionOf(bounds, to);
    return {left: `${start * 100}%`, width: `${Math.max(0.5, (end - start) * 100)}%`};
}

function markerLeft(tick: number): string {
    const bounds = store.bounds;
    return bounds ? `${fractionOf(bounds, tick) * 100}%` : '0%';
}
</script>

<template>
    <div class="runs">
        <aside class="runs__list">
            <h2>Runs</h2>
            <p v-if="store.error" class="runs__error">{{ store.error }}</p>
            <p v-else-if="store.runs.length === 0" class="runs__empty">
                No runs recorded yet.
            </p>
            <ul>
                <li v-for="run in store.runs" :key="run.run_id">
                    <button
                        type="button"
                        :class="{'is-active': selected === run.run_id}"
                        @click="open(run.run_id)"
                    >
                        <span class="runs__when">
                            {{ formatWhen(startedUnixOf(run)) }}
                            <em>{{ formatAgo(startedUnixOf(run), nowUnix) }}</em>
                        </span>
                        <span class="runs__id">{{ run.run_id }}</span>
                        <span class="runs__meta">
                            <!-- `finished: false` means crashed OR still going; the
                                 server cannot tell them apart, so neither does this. -->
                            {{ run.finished ? run.outcome : 'unfinished' }}
                            · {{ formatTicks(run.elapsed_ticks) }}
                            · {{ run.frames ?? 0 }} frames
                        </span>
                    </button>
                </li>
            </ul>
        </aside>

        <section v-if="store.detail" class="runs__viewer">
            <header class="runs__header">
                <h2>{{ store.detail.summary.run_id }}</h2>
                <span class="runs__when">
                    {{ formatWhen(startedUnixOf(store.detail.summary)) }}
                </span>
                <span v-if="currentSplit" class="runs__now">{{ currentSplit.goal }}</span>
            </header>

            <table class="splits">
                <thead>
                    <tr>
                        <th>#</th><th>milestone</th><th>at</th><th>took</th>
                        <th v-if="deltas">vs ref</th><th></th>
                    </tr>
                </thead>
                <tbody>
                    <tr
                        v-for="split in store.detail.splits"
                        :key="`${split.index}-${split.started_tick}`"
                        :class="{'is-current': currentSplit?.started_tick === split.started_tick}"
                    >
                        <td>{{ split.index }}</td>
                        <td>{{ split.goal }}</td>
                        <td class="num">{{ split.started_tick }}</td>
                        <td class="num">{{ formatTicks(split.elapsed_ticks) }}</td>
                        <td v-if="deltas" class="num">
                            <!-- An em dash, not a zero: no delta exists when
                                 either side never finished, and zero would read
                                 as "exactly the same". -->
                            <span
                                v-if="deltas.get(split.goal)?.delta != null"
                                :class="(deltas.get(split.goal)!.delta as number) < 0 ? 'faster' : 'slower'"
                            >
                                {{ (deltas.get(split.goal)!.delta as number) > 0 ? '+' : ''
                                }}{{ formatTicks(deltas.get(split.goal)!.delta) }}
                            </span>
                            <span v-else>—</span>
                        </td>
                        <td :class="['outcome', `outcome--${split.outcome}`]">
                            {{ split.outcome }}
                        </td>
                    </tr>
                </tbody>
            </table>

            <div v-if="store.bounds" class="timeline">
                <div class="timeline__track">
                    <span
                        v-for="split in store.detail.splits"
                        :key="`m-${split.index}-${split.started_tick}`"
                        class="timeline__marker"
                        :style="{left: markerLeft(split.started_tick)}"
                        :title="split.goal"
                    />
                </div>
                <input
                    type="range"
                    :min="store.bounds.from"
                    :max="store.bounds.to"
                    :value="store.cursor"
                    @input="store.seek(Number(($event.target as HTMLInputElement).value))"
                />
                <div class="timeline__controls">
                    <button type="button" @click="store.togglePlay()">
                        {{ store.playing ? 'Pause' : 'Play' }}
                    </button>
                    <span class="num">tick {{ store.cursor }}</span>
                    <label>
                        step
                        <select :value="store.rate" @change="store.rate = Number(($event.target as HTMLSelectElement).value)">
                            <option :value="60">1s</option>
                            <option :value="300">5s</option>
                            <option :value="1800">30s</option>
                        </select>
                    </label>
                </div>
            </div>

            <label v-if="otherRuns.length > 0" class="compare">
                compare with
                <select
                    :value="store.reference?.summary.run_id ?? ''"
                    @change="store.setReference(($event.target as HTMLSelectElement).value || null)"
                >
                    <option value="">— none —</option>
                    <option v-for="r in otherRuns" :key="r.run_id" :value="r.run_id">
                        {{ r.run_id }}
                    </option>
                </select>
            </label>

            <div v-if="lanesByBot.length > 0 && store.bounds" class="lanes">
                <div v-for="row in lanesByBot" :key="row.bot" class="lanes__row">
                    <span class="lanes__label">bot {{ row.bot }}</span>
                    <div class="lanes__track">
                        <span
                            v-for="(entry, i) in row.entries"
                            :key="`${entry.id}-${entry.from_tick}-${i}`"
                            :class="[
                                'lanes__span',
                                `lanes__span--${entry.status ?? 'running'}`,
                                {'is-now': row.now === entry}
                            ]"
                            :style="laneStyle(entry.from_tick, entry.to_tick)"
                            :title="`${entry.action} — ${entry.status ?? 'never settled'}`"
                        />
                    </div>
                    <span class="lanes__now">{{ row.now?.action ?? '—' }}</span>
                </div>
            </div>

            <div class="frame">
                <div class="frame__picker">
                    <label>
                        bot
                        <select :value="store.bot ?? ''" @change="store.bot = Number(($event.target as HTMLSelectElement).value)">
                            <option v-for="b in bots" :key="b" :value="b">{{ b }}</option>
                        </select>
                    </label>
                    <label>
                        camera
                        <select :value="store.camera ?? ''" @change="store.camera = ($event.target as HTMLSelectElement).value">
                            <option v-for="c in cameras" :key="c" :value="c">{{ c }}</option>
                        </select>
                    </label>
                </div>
                <img v-if="frameSrc" :src="frameSrc" :alt="`frame at tick ${current?.tick}`" />
                <!-- Before the first frame is a real state: the run had begun
                     and capture had not yet produced anything. -->
                <!-- Say where the frames start rather than leaving a dead end:
                     "none here" and "none at all" are different answers. -->
                <p v-else class="frame__none">
                    <template v-if="firstForSelection !== null">
                        No frame yet at tick {{ store.cursor }} — this camera starts at
                        <button type="button" class="linkish" @click="store.seek(firstForSelection)">
                            tick {{ firstForSelection }}
                        </button>.
                    </template>
                    <template v-else>
                        This bot and camera captured no frames in this run.
                    </template>
                </p>
                <p v-if="current" class="frame__caption num">
                    frame tick {{ current.tick }} · {{ current.camera }}
                </p>
            </div>
        </section>
    </div>
</template>

<style scoped>
.runs {
    display: flex;
    gap: 1.5rem;
    align-items: flex-start;
}
.runs__list {
    flex: 0 0 22rem;
}
.runs__list ul {
    list-style: none;
    margin: 0;
    padding: 0;
}
.runs__list button {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.25rem;
    border: 1px solid var(--surface-border, #ccc);
    border-radius: 4px;
    background: transparent;
    cursor: pointer;
}
.runs__list button.is-active {
    border-color: #3b82f6;
}
.runs__when {
    display: block;
    font-weight: 600;
}
.runs__when em {
    font-weight: 400;
    font-style: normal;
    opacity: 0.6;
    margin-left: 0.4rem;
}
.runs__id {
    display: block;
    font-family: monospace;
    font-size: 0.75rem;
    opacity: 0.55;
}
.runs__meta {
    display: block;
    font-size: 0.8rem;
    opacity: 0.7;
}
.runs__viewer {
    flex: 1 1 auto;
    min-width: 0;
}
.runs__header {
    display: flex;
    align-items: baseline;
    gap: 1rem;
}
.runs__now {
    opacity: 0.7;
}
.splits {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 1rem;
}
.splits th,
.splits td {
    text-align: left;
    padding: 0.25rem 0.5rem;
    border-bottom: 1px solid var(--surface-border, #eee);
}
.splits .num {
    font-family: monospace;
    text-align: right;
}
.splits tr.is-current {
    background: rgba(59, 130, 246, 0.12);
}
.outcome--stuck,
.outcome--stuck_silent {
    color: #b91c1c;
}
.outcome--unfinished {
    opacity: 0.6;
}
.timeline {
    margin-bottom: 1rem;
}
.timeline__track {
    position: relative;
    height: 6px;
    background: var(--surface-border, #eee);
    border-radius: 3px;
}
.timeline__marker {
    position: absolute;
    top: -2px;
    width: 2px;
    height: 10px;
    background: #3b82f6;
}
.timeline input[type='range'] {
    width: 100%;
}
.timeline__controls {
    display: flex;
    gap: 1rem;
    align-items: center;
}
.frame img {
    max-width: 100%;
    display: block;
    border: 1px solid var(--surface-border, #ccc);
}
.frame__picker {
    display: flex;
    gap: 1rem;
    margin-bottom: 0.5rem;
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
.frame__caption,
.frame__none {
    font-size: 0.85rem;
    opacity: 0.7;
}
.num {
    font-family: monospace;
}
.compare {
    display: inline-block;
    margin-bottom: 1rem;
    font-size: 0.85rem;
}
.faster {
    color: #15803d;
}
.slower {
    color: #b91c1c;
}
.lanes {
    margin-bottom: 1rem;
}
.lanes__row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
}
.lanes__label {
    flex: 0 0 4rem;
    font-size: 0.85rem;
    opacity: 0.7;
}
.lanes__track {
    position: relative;
    flex: 1 1 auto;
    height: 14px;
    background: var(--surface-border, #eee);
    border-radius: 3px;
}
.lanes__span {
    position: absolute;
    top: 0;
    height: 14px;
    border-radius: 3px;
    background: #3b82f6;
}
.lanes__span--failed,
.lanes__span--lost {
    background: #b91c1c;
}
/* Never settled: no verdict ever arrived, so it is neither success nor
   failure and must not be coloured as either. */
.lanes__span--running {
    background: repeating-linear-gradient(45deg, #9ca3af, #9ca3af 4px, #d1d5db 4px, #d1d5db 8px);
}
.lanes__span.is-now {
    outline: 2px solid #1d4ed8;
}
.lanes__now {
    flex: 0 0 12rem;
    font-size: 0.8rem;
    opacity: 0.8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
</style>
