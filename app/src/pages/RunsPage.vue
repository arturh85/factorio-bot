<script setup lang="ts">
/**
 * Viewing an archived run: splits, a tick scrubber, the video and the map.
 *
 * Everything reads one cursor, in game ticks. Wall time appears only as "when
 * was this run" -- two runs are compared on the game's clock, because a
 * headless server and a graphical client do not run at the same speed.
 */
import {computed, onBeforeUnmount, onMounted, ref, watch, watchEffect} from 'vue';
import {useRunsStore} from '@/store/runsStore';
import MapPanel from '@/components/MapPanel.vue';
import {parseVideoClock, tickToVideoSeconds, videoSecondsToTick} from '@/api/videoClock';
import {videoDefects} from '@/api/videoJoin';
import {
    formatAgo,
    formatTicks,
    formatWhen,
    fractionOf,
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
        // that the eye can register each step.
        if (playing) timer = window.setInterval(() => store.advance(), 100);
    }
);

async function open(id: string) {
    selected.value = id;
    await store.openRun(id);
}

/**
 * The video half of this page.
 *
 * The clock is the whole point: a video has wall-clock frames and everything
 * else here is keyed on `game.tick`, and UPS is not constant, so the two cannot
 * be related by multiplying. `parseVideoClock` interpolates between the nearest
 * sampled pairs and **returns null rather than a number** outside the sampled
 * range or across a stall -- so an unanswerable tick shows a message instead of
 * a plausible wrong moment.
 */
const videoClock = computed(() => parseVideoClock(store.video, store.videoTicks));

/** The run's own copy, not `workspace/video/`, which the next run overwrites. */
const videoSrc = computed(() =>
    store.video?.video == null || store.detail == null
        ? null
        : `/api/v1/runs/${store.detail.summary.run_id}/video/file`
);

const videoAt = computed(() =>
    videoClock.value === null ? null : tickToVideoSeconds(videoClock.value, store.cursor)
);

/** Anything the manifest itself says is wrong -- a zero-byte file, an unverified rate. */
/** The ticks the recording actually covers; `null` when the clock observed nothing. */
/**
 * One derivation, deliberately.
 *
 * `clockTickRange(videoClock)` and `store.video.tick_range` describe the same
 * span from two sources, and the timeline axis is fed from the manifest's. If
 * they ever disagreed, the "recording starts at tick N" button below would seek
 * to a tick outside the axis it is drawn on -- a control that moves you somewhere
 * the page cannot show. So this reads the manifest, the same value the axis uses,
 * and the clock's own refusal (`videoAt === null`) still covers the case where it
 * cannot place a tick inside that span.
 */
const videoRange = computed(() => store.video?.tick_range ?? null);

const videoIssues = computed(() => (store.video === null ? [] : videoDefects(store.video)));

const videoEl = ref<HTMLVideoElement | null>(null);

/**
 * The two directions of the video/timeline sync, and the loop between them.
 *
 * Cursor -> video is a `watchEffect`; video -> cursor is `onVideoTime` below.
 * Together those are a cycle, so each end only acts on a difference bigger than
 * `SYNC_SLOP_S`: a round trip lands within the slop and stops, instead of the two
 * ends nudging each other forever. The slop is a quarter-second because the clock
 * samples at 2 Hz -- tighter than the data, tight enough not to be seen.
 *
 * `videoDriving` exists because a *seek* is not a difference to be corrected. When
 * the video is playing it owns the cursor, and the effect must not drag it back to
 * where the cursor was a moment ago.
 */
const SYNC_SLOP_S = 0.25;
const videoDriving = ref(false);

watchEffect(() => {
    const at = videoAt.value;
    const el = videoEl.value;
    // `videoAt` is null when the clock declines to place this tick -- inside a
    // stall, or outside the recording. Leave the element where it is rather than
    // seek to a guess.
    if (el === null || at === null || videoDriving.value) return;
    if (Math.abs(el.currentTime - at.seconds) > SYNC_SLOP_S) el.currentTime = at.seconds;
});

/**
 * The video is playing (or was scrubbed): move the cursor to match, so the
 * splits, map, lanes and world-state panels all follow the picture.
 *
 * A null answer is respected here too -- a position inside a stall is a real
 * picture of no tick this clock observed, and moving the cursor anyway would put
 * every other panel on a tick the video is not showing.
 */
function onVideoTime() {
    const el = videoEl.value;
    const clock = videoClock.value;
    if (el === null || clock === null || el.paused) return;
    const at = videoSecondsToTick(clock, el.currentTime);
    if (at === null) return;
    videoDriving.value = true;
    store.seek(at.tick);
    // Released on the next macrotask so the cursor-driven effect sees the new
    // value with the guard still up, and does not immediately seek back.
    setTimeout(() => (videoDriving.value = false), 0);
}

/** Two clocks driving one cursor is one too many. */
function onVideoPlay() {
    if (store.playing) store.togglePlay();
}

const currentSplit = computed(() => splitAt(store.detail?.splits ?? [], store.cursor));

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

/** Research progress as a whole-number percentage, e.g. "42%". */
function researchPct(progress: number): string {
    return `${Math.round(progress * 100)}%`;
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
                <router-link :to="`/runs/${store.detail.summary.run_id}/analysis`" class="linkish">
                    analysis
                </router-link>
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
                <!-- The axis starts at the first lane bar or the recording,
                     not at the run. Say so, rather than let the clipped first
                     milestone look like a disagreement with the splits
                     table. -->
                <p v-if="store.leadIn > 0" class="timeline__leadin num">
                    axis starts at the first capture · {{ formatTicks(store.leadIn) }} of planning
                    before it, not shown
                </p>
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

            <p v-if="store.lanesError" class="stream-warning">{{ store.lanesError }}</p>
            <div v-else-if="lanesByBot.length > 0 && store.bounds" class="lanes">
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

            <!-- The run's visual record. Since the per-camera screenshots were
                 retired (2026-09-02) this is the only one, and a run that
                 recorded none renders nothing here rather than a panel
                 explaining its own emptiness. -->
            <div v-if="store.video?.video" class="video">
                <h3>Video</h3>
                <p v-if="store.videoError" class="stream-warning">{{ store.videoError }}</p>
                <p v-for="d in videoIssues" :key="d.kind" class="stream-warning">{{ d.message }}</p>
                <video
                    ref="videoEl"
                    :src="videoSrc ?? undefined"
                    preload="metadata"
                    controls
                    class="video__player"
                    @timeupdate="onVideoTime"
                    @seeked="onVideoTime"
                    @play="onVideoPlay"
                />
                <p class="video__meta num">
                    {{ store.video.video.width }}x{{ store.video.video.height }} ·
                    {{ store.video.video.fps }} fps ·
                    {{ ((store.video.bytes ?? 0) / 1048576).toFixed(0) }} MB ·
                    <template v-if="videoAt">at {{ videoAt.seconds.toFixed(1) }}s</template>
                    <template v-else-if="videoRange && store.cursor < videoRange.from">
                        recording starts at
                        <button type="button" class="linkish" @click="store.seek(videoRange.from)">
                            tick {{ videoRange.from }}
                        </button>
                        — the run had already begun
                    </template>
                    <template v-else-if="videoRange && store.cursor > videoRange.to">
                        recording ended at tick {{ videoRange.to }}
                    </template>
                    <template v-else>the clock cannot place tick {{ store.cursor }}</template>
                </p>
            </div>

            <div class="map">
                <h3>Map</h3>
                <p v-if="store.mapError" class="stream-warning">{{ store.mapError }}</p>
                <MapPanel
                    v-else
                    :entities="store.entities"
                    :bots="store.mapBots"
                    :trail="store.trail"
                    :records="store.map"
                    :bounds="store.mapBounds"
                />
            </div>

            <div class="worldstate">
                <div class="panel">
                    <h3>Research</h3>
                    <!-- A fetch failure is neither "no samples yet" nor "no
                         research queued" -- it means the stream never
                         arrived, and saying so is the whole point of this
                         defect fix, so it comes first. -->
                    <p v-if="store.sampleError" class="stream-warning">{{ store.sampleError }}</p>
                    <!-- Three states, not two: no sample yet at this tick is
                         different from a sample that says nothing is queued. -->
                    <p v-else-if="!store.forceState" class="worldstate__empty">
                        no world-state samples recorded for this run
                    </p>
                    <p v-else-if="!store.forceState.research" class="worldstate__empty">
                        no research queued
                    </p>
                    <p v-else class="research__line">
                        {{ store.forceState.research.name }}
                        <span class="num">{{ researchPct(store.forceState.research.progress) }}</span>
                    </p>
                </div>

                <div class="panel">
                    <h3>Production</h3>
                    <p v-if="store.sampleError" class="stream-warning">{{ store.sampleError }}</p>
                    <p v-else-if="store.production.length === 0" class="worldstate__empty">
                        nothing tracked for this run
                    </p>
                    <ul v-else class="worldstate__list">
                        <li v-for="row in store.production" :key="row.item">
                            <span>{{ row.item }}</span>
                            <span class="num">{{ row.made }}</span>
                        </li>
                    </ul>
                </div>

                <div class="panel">
                    <h3>Inventory<template v-if="store.bot !== null"> — bot {{ store.bot }}</template></h3>
                    <p v-if="store.sampleError" class="stream-warning">{{ store.sampleError }}</p>
                    <p v-else-if="!store.botState" class="worldstate__empty">
                        {{
                            store.bot === null
                                ? 'this run sampled no bots'
                                : 'no bot sample yet at this tick'
                        }}
                    </p>
                    <template v-else>
                        <ul
                            v-if="Object.keys(store.botState.inventory).length > 0"
                            class="worldstate__list"
                        >
                            <li v-for="(count, item) in store.botState.inventory" :key="item">
                                <span>{{ item }}</span>
                                <span class="num">{{ count }}</span>
                            </li>
                        </ul>
                        <p v-else class="worldstate__empty">inventory empty</p>
                        <p class="inventory__meta">
                            mining {{ store.botState.mining ?? '—' }} · crafting queue
                            {{ store.botState.crafting_queue }}
                        </p>
                    </template>
                </div>
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
    /* Sticky, because the scrubber is what you steer the rest of the page with:
     * the splits table, the video and the map all answer "what was happening at
     * this tick", and scrolling to any of them used to take the control that sets
     * the tick off-screen.
     *
     * `top` clears the fixed 50px topbar rather than 0, or the track hides behind
     * it. `z-index` stays *below* the topbar's z-30 so it slides under, not over.
     *
     * The opaque background is load-bearing, not decoration -- without it the
     * content scrolling underneath shows straight through the 6px track and its
     * markers, which is worse than not being sticky at all. */
    position: sticky;
    top: var(--spacing-topbar, 50px);
    z-index: 10;
    margin-bottom: 1rem;
    padding: 0.5rem 0;
    background: var(--color-surface, #edf0f5);
    border-bottom: 1px solid var(--surface-border, #ddd);
}
.timeline__leadin {
    margin: 0 0 0.35rem;
    font-size: 0.75rem;
    color: var(--muted, #8b8b8b);
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
.video {
    margin-bottom: 1rem;
}
.video__player {
    max-width: 100%;
    max-height: 60vh;
    display: block;
    border: 1px solid var(--surface-border, #ccc);
}
.video__meta {
    margin: 0.35rem 0 0;
    font-size: 0.75rem;
    color: var(--muted, #8b8b8b);
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
.map {
    margin-bottom: 1rem;
}
.map h3 {
    margin: 0 0 0.4rem;
    font-size: 0.85rem;
    text-transform: uppercase;
    opacity: 0.6;
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
.worldstate {
    display: flex;
    gap: 1rem;
    margin-top: 1rem;
    flex-wrap: wrap;
}
.worldstate .panel {
    flex: 1 1 14rem;
    min-width: 12rem;
    border: 1px solid var(--surface-border, #ccc);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
}
.worldstate .panel h3 {
    margin: 0 0 0.4rem;
    font-size: 0.85rem;
    text-transform: uppercase;
    opacity: 0.6;
}
.worldstate__empty {
    font-size: 0.85rem;
    opacity: 0.6;
    margin: 0;
}
.worldstate__list {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 0.85rem;
}
.worldstate__list li {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.1rem 0;
}
.research__line {
    display: flex;
    justify-content: space-between;
    margin: 0;
}
.inventory__meta {
    margin: 0.4rem 0 0;
    font-size: 0.75rem;
    opacity: 0.7;
}
/*
 * One enrichment stream failed to load but the run itself is fine -- distinct
 * from `.runs__error`, which means the run could not be opened at all. Amber
 * rather than red: this is a missing panel, not a broken page.
 */
.stream-warning {
    font-size: 0.85rem;
    color: #92400e;
    background: rgba(217, 119, 6, 0.1);
    border: 1px solid rgba(217, 119, 6, 0.3);
    border-radius: 4px;
    padding: 0.4rem 0.6rem;
    margin: 0 0 0.5rem;
}
</style>
