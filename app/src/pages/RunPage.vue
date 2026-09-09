<!-- app/src/pages/RunPage.vue -->
<script setup lang="ts">
/**
 * One run, read on one clock. Every band is a lane on the same tick axis and
 * the cursor is the only control. The analysis window (`store.window`, from
 * `run_started`) feeds rates and verdicts; the drawn axis (`store.bounds`)
 * feeds positions. They differ by the lead-in and both are shown.
 */
import {computed, onBeforeUnmount, onMounted, watch} from 'vue';
import {useRoute} from 'vue-router';
import {useRunsStore} from '@/store/runsStore';
import {compareSplits, formatTicks, formatWhen, startedUnixOf, stuckReasons} from '@/lib/runTimeline';
import {rateItems} from '@/lib/runRates';
import {headline} from '@/lib/runHeadline';
import {machineRows, positionKey} from '@/lib/machineTimeline';
import BandFrame from '@/components/run/BandFrame.vue';
import TickAxis from '@/components/run/TickAxis.vue';
import ProductionBand from '@/components/run/ProductionBand.vue';
import PowerBand from '@/components/run/PowerBand.vue';
import ResearchBand from '@/components/run/ResearchBand.vue';
import LaneBand from '@/components/run/LaneBand.vue';
import MachineBand from '@/components/run/MachineBand.vue';
import CoverageBand from '@/components/run/CoverageBand.vue';
import CursorBar from '@/components/run/CursorBar.vue';
import RunHeadline from '@/components/run/RunHeadline.vue';
import MilestoneRibbon from '@/components/run/MilestoneRibbon.vue';
import RunSidePanel from '@/components/run/RunSidePanel.vue';

const route = useRoute();
const store = useRunsStore();
const id = computed(() => String(route.params.id));

let timer: number | null = null;
function stopTimer() { if (timer !== null) { window.clearInterval(timer); timer = null; } }
watch(() => store.playing, (playing) => { stopTimer(); if (playing) timer = window.setInterval(() => store.advance(), 100); });
onMounted(() => { store.loadRuns(); store.openRun(id.value); });
watch(id, (next) => store.openRun(next));
onBeforeUnmount(stopTimer);

const scale = computed(() => store.bounds);
/**
 * The analysis window, or why there is not one.
 *
 * `store.window` is null whenever `/events` returned nothing, and that has
 * two very different causes: the fetch FAILED (`store.eventsError` set,
 * `store.events` never populated), or the run genuinely recorded no events at
 * all. Only the second may borrow the drawn axis as a stand-in window --
 * substituting it for a fetch failure would draw rates and a verdict over a
 * span `/events` never vouched for, silently. The headline and the Items/min
 * band both check `win` directly (never `store.eventsError` a second time) so
 * their `v-else` branches narrow it to non-null for the template compiler.
 */
const windowIsFallback = computed(() => store.eventsError === null && store.events.length === 0);
const win = computed(() => store.window
    ?? (windowIsFallback.value && store.bounds ? {lo: store.bounds.from, hi: store.bounds.to} : null));
/**
 * The clock every band LABELS with, as against `scale`, which every band
 * POSITIONS with.
 *
 * The analysis window when there is one, so the ribbon's "satisfied at 6:06"
 * and the headline's are the same sentence about the same tick, and the
 * band's "5:00 mark" is `just analyse`'s tick 21,242 rather than the drawn
 * axis's 21,417. The `{from: 0, to: 0}` fallback is never rendered: every
 * band lives under `v-if="scale"`, and `win` is only null there when
 * `store.bounds` is not.
 */
const clock = computed(() => (win.value ? {from: win.value.lo, to: win.value.hi} : store.bounds ?? {from: 0, to: 0}));
const items = computed(() => (win.value ? rateItems(store.samples, win.value.lo, win.value.hi) : []));
const sentence = computed(() => {
    if (!win.value) return '';
    const base = headline({samples: store.samples, events: store.events, splits: store.detail?.splits ?? [], lo: win.value.lo, hi: win.value.hi, items: items.value});
    // Say so before the sentence, not after: a reader who has already taken
    // in "iron-plate 8/min at 5:00" should not have to notice a footnote to
    // learn those marks are not `just analyse`'s.
    return windowIsFallback.value ? `no events recorded — window taken from the drawn axis · ${base}` : base;
});
const deltas = computed(() => store.reference === null ? null
    : new Map(compareSplits(store.detail?.splits ?? [], store.reference.splits).map((r) => [r.goal, r])));
/**
 * The selected machine as a MAP position key.
 *
 * `store.selectedMachine` is the machines-sample key (`unit_number` as a
 * string, e.g. `"13"`); the map joins on `${x},${y}`. Nothing translated
 * between the two, so selecting a row in the Machines band changed nothing a
 * reader could see. `machineRows` already carries each row's position, so
 * this is the whole translation -- and a key with no row simply highlights
 * nothing, which is the honest answer for a machine the entity map has not
 * placed.
 */
const highlight = computed(() => {
    if (store.selectedMachine === null) return null;
    const row = machineRows(store.samples).find((r) => r.key === store.selectedMachine);
    return row ? positionKey(row.position) : null;
});
const otherRuns = computed(() => store.runs.filter((r) => r.run_id !== id.value));

/** Every stuck milestone's planner refusal, by split index -- fed to the ribbon's segment titles. */
const reasons = computed(() => stuckReasons(store.events));
/**
 * The refusal behind the run's own last milestone, when it is a stuck one.
 *
 * Read off the *last* split rather than any stuck split: an earlier stuck
 * milestone that later recovered and closed is history, not the reason this
 * run is where it is now.
 */
const lastStuckReason = computed(() => {
    const splits = store.detail?.splits ?? [];
    const last = splits[splits.length - 1];
    if (last === undefined || last.outcome !== 'stuck') return null;
    return reasons.value.get(last.index) ?? null;
});

const LEGEND = [
    ['walk', 'verb-walk'], ['mine / chop', 'verb-mine'], ['craft', 'verb-craft'], ['place', 'verb-place'],
    ['feed (insert · stock · take · fuel)', 'verb-feed'], ['research', 'verb-research'],
    ['working', 'status-good'], ['no ingredients', 'status-warn'], ['no fuel', 'status-serious'], ['no power', 'status-critical'], ['normal / other', 'status-neutral']
] as const;
</script>

<template>
  <div class="mx-auto max-w-[1400px]">
    <p v-if="store.error" class="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger-dark">{{ store.error }}</p>
    <p v-else-if="store.loading && !store.detail" class="text-ink-muted">loading {{ id }}…</p>
    <section v-else-if="store.detail" class="overflow-hidden rounded-card border border-divider bg-card">
      <!-- The headline sentence needs the analysis window, which needs
           `/events` -- a genuine fetch failure is named here rather than
           read as "no events recorded", which is what an empty `sentence`
           would otherwise say. -->
      <p v-if="store.eventsError" class="px-3 py-2 text-sm text-warn-dark">{{ store.eventsError }}</p>
      <RunHeadline v-else :summary="store.detail.summary" :provenance="store.provenance" :provenance-error="store.provenanceError"
                   :lag-ticks="store.sampleLag" :headline="sentence" :roster="store.laneBotIds"
                   :replay-counts="store.replayCounts" :replay-error="store.replayError"
                   :savepoints="store.savepoints" :savepoints-error="store.savepointsError"/>
      <p class="px-5 py-1 text-xs text-ink-muted">
        {{ formatWhen(startedUnixOf(store.detail.summary)) }} ·
        <router-link :to="`/runs/${id}/analysis`" class="underline">overrun and divergence tables</router-link>
      </p>
      <p v-if="lastStuckReason" data-testid="stuck-reason" class="px-5 py-1 text-sm text-warn-dark">last refusal: {{ lastStuckReason }}</p>

      <!-- Gated on `scale` alone, not `scale && win`: a run with a drawn axis
           still has bands to show even when `/events` failed and left `win`
           null -- only the two bands below that actually need the analysis
           window (Items/min, and the headline above) say so themselves. -->
      <template v-if="scale">
        <div class="grid grid-cols-[10.5rem_1fr] border-b border-divider">
          <div class="border-r border-divider bg-surface px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-ink-muted">Milestones</div>
          <MilestoneRibbon :scale="scale" :clock="clock" :splits="store.detail.splits" :cursor="store.cursor" :reasons="reasons"/>
        </div>
        <BandFrame title="Axis" :subtitle="store.leadIn > 0 ? `axis starts ${formatTicks(store.leadIn)} after run start` : 'minutes of game time · 5-min marks'">
          <TickAxis :scale="scale" :clock="clock" :cursor="store.cursor"/>
        </BandFrame>
        <BandFrame title="Items / min" subtitle="trailing 2-min window · background is the attribution verdict per minute">
          <p v-if="store.sampleError" class="px-3 py-2 text-sm text-warn-dark">{{ store.sampleError }}</p>
          <p v-else-if="!win" class="px-3 py-2 text-sm text-warn-dark">{{ store.eventsError }}</p>
          <ProductionBand v-else :scale="scale" :clock="clock" :cursor="store.cursor" :samples="store.samples" :events="store.events" :items="items" :lo="win.lo" :hi="win.hi"/>
        </BandFrame>
        <BandFrame title="Power" subtitle="kW generated vs consumed · one scale">
          <p v-if="store.sampleError" class="px-3 py-2 text-sm text-warn-dark">{{ store.sampleError }}</p>
          <PowerBand v-else :scale="scale" :clock="clock" :cursor="store.cursor" :samples="store.samples"/>
        </BandFrame>
        <BandFrame title="Research" subtitle="progress of the current technology">
          <p v-if="store.sampleError" class="px-3 py-2 text-sm text-warn-dark">{{ store.sampleError }}</p>
          <ResearchBand v-else :scale="scale" :clock="clock" :cursor="store.cursor" :samples="store.samples"/>
        </BandFrame>
        <BandFrame title="Bots" subtitle="one row per bot · idle is hatched · feeding acts are ticks · replans are dashed">
          <p v-if="store.lanesError" class="px-3 py-2 text-sm text-warn-dark">{{ store.lanesError }}</p>
          <LaneBand v-else :scale="scale" :clock="clock" :cursor="store.cursor" :lanes="store.lanes" :events="store.events"/>
        </BandFrame>
        <BandFrame title="Machines" subtitle="status of every sampled machine, 5-s cells · grouped by kind, ordered by placement">
          <p v-if="store.sampleError" class="px-3 py-2 text-sm text-warn-dark">{{ store.sampleError }}</p>
          <MachineBand v-else :scale="scale" :clock="clock" :cursor="store.cursor" :samples="store.samples" :selected="store.selectedMachine" @select="store.selectMachine($event)"/>
        </BandFrame>
        <BandFrame title="Record" subtitle="where the record has data · a gap reads as “no record”">
          <p v-if="store.eventsError" class="px-3 py-2 text-sm text-warn-dark">{{ store.eventsError }}</p>
          <CoverageBand v-else :scale="scale" :cursor="store.cursor" :events="store.events" :samples="store.samples" :run-end="win?.hi ?? scale.to" :skipped="store.eventsSkipped"/>
        </BandFrame>
        <CursorBar :scale="scale" :clock="clock" :cursor="store.cursor" :playing="store.playing" :rate="store.rate"
                   @seek="store.seek($event)" @toggle="store.togglePlay()" @rate="store.rate = $event"/>
        <div class="flex flex-wrap gap-x-4 gap-y-1.5 border-t border-divider px-5 py-2 text-xs text-ink-muted">
          <span v-for="[label, token] in LEGEND" :key="label" class="inline-flex items-center gap-1.5">
            <i class="inline-block h-2 w-3 rounded-sm" :style="{background: `var(--color-${token})`}"/>{{ label }}
          </span>
          <span class="inline-flex items-center gap-1.5">
            <i class="inline-block h-2 w-3 rounded-sm" style="background: repeating-linear-gradient(135deg, var(--color-ink-muted) 0 1px, transparent 1px 4px)"/>idle — no dispatched action
          </span>
        </div>
      </template>
      <p v-else class="px-5 py-4 text-sm text-ink-muted">this run recorded nothing to place on an axis</p>

      <RunSidePanel :run-id="id" :cursor="store.cursor"
                    :video="store.video" :video-ticks="store.videoTicks" :video-error="store.videoError"
                    :entities="store.entities" :bots="store.mapBots" :trail="store.trail" :records="store.map" :bounds="store.mapBounds"
                    :map-error="store.mapError" :fills="store.machineFills" :highlight="highlight"
                    @seek="store.seek($event)" @pause="store.playing && store.togglePlay()"/>

      <div class="border-t border-divider px-5 py-4">
        <label v-if="otherRuns.length > 0" class="text-sm text-ink-muted">
          compare with
          <select class="ml-1 rounded border border-divider bg-card px-1 py-0.5 text-sm" :value="store.reference?.summary.run_id ?? ''"
                  @change="store.setReference(($event.target as HTMLSelectElement).value || null)">
            <option value="">— none —</option>
            <option v-for="r in otherRuns" :key="r.run_id" :value="r.run_id">{{ r.run_id }}</option>
          </select>
        </label>
        <table class="mt-3 w-full text-sm">
          <thead><tr class="text-left text-xs uppercase tracking-wider text-ink-muted"><th class="py-1">#</th><th>milestone</th><th>at</th><th>took</th><th v-if="deltas">vs ref</th><th></th></tr></thead>
          <tbody>
            <tr v-for="s in store.detail.splits" :key="`${s.index}-${s.started_tick}`" class="border-t border-divider">
              <td class="py-1">{{ s.index }}</td><td>{{ s.goal }}</td>
              <td class="font-mono tabular-nums">{{ s.started_tick }}</td>
              <td class="font-mono tabular-nums">{{ formatTicks(s.elapsed_ticks) }}</td>
              <td v-if="deltas" class="font-mono tabular-nums">
                <span v-if="deltas.get(s.goal)?.delta != null" :class="(deltas.get(s.goal)!.delta as number) < 0 ? 'text-success-dark' : 'text-danger-dark'">
                  {{ (deltas.get(s.goal)!.delta as number) > 0 ? '+' : '' }}{{ formatTicks(deltas.get(s.goal)!.delta) }}
                </span>
                <span v-else>—</span>
              </td>
              <td :class="s.outcome === 'stuck' || s.outcome === 'stuck_silent' ? 'text-danger-dark' : s.outcome === 'unfinished' ? 'text-ink-muted' : ''">{{ s.outcome }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>
