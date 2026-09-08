<script setup lang="ts">
import {computed} from 'vue';
import {Event, Sample} from '@/api/types';
import {coverageOf, lagTicks} from '@/lib/runCoverage';
import {AXIS_WIDTH, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{
    scale: TickScale; cursor: number; events: Event[]; samples: Sample[]; runEnd: number;
    /**
     * Event lines the server could not parse -- in practice the torn last
     * line of a killed run. This band is where the record says what it does
     * NOT have, and a dropped line is exactly that; the store had the number
     * and threw it away. Optional and defaulting to 0, because a caller that
     * does not know is not the same as a run with none.
     */
    skipped?: number;
}>();
const ALL = ['events', 'bot samples', 'force + machine samples'];
const COLORS: Record<string, string> = {events: 'var(--color-ink-muted)', 'bot samples': 'var(--color-verb-walk)', 'force + machine samples': 'var(--color-verb-research)'};
const x = (t: number) => tickX(props.scale, t);
const rows = computed(() => coverageOf(props.events, props.samples));
const missing = computed(() => ALL.filter((l) => !rows.value.some((r) => r.label === l)));
const lag = computed(() => lagTicks(props.runEnd, props.samples));
const H = computed(() => Math.max(24, rows.value.length * 11 + 8));
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="record coverage">
    <template v-for="(r, i) in rows" :key="r.label">
      <rect class="extent" :data-stream="r.label" :x="x(r.from)" :y="4 + i * 11" :width="Math.max(1, x(r.to) - x(r.from))" height="8" rx="1" :fill="COLORS[r.label]" opacity="0.55"/>
      <text x="6" :y="4 + i * 11 + 7" font-size="8.5" fill="var(--color-ink)" style="paint-order: stroke" stroke="var(--color-plot)" stroke-width="2">{{ r.label }} · {{ r.count.toLocaleString() }}</text>
    </template>
    <text :x="AXIS_WIDTH - 4" y="11" font-size="9" text-anchor="end" fill="var(--color-ink-muted)">
      <!-- lag <= 0: the run's declared end sits at or before the last
           recorded sample, so the record covers the run to its end rather
           than trailing behind it -- not a gap to report as a negative tick
           count. -->
      {{ lag === null ? 'no samples' : lag <= 0 ? 'samples cover the run to its end' : `samples lag ${lag.toLocaleString()} ticks` }}<template v-if="missing.length"> · no record: {{ missing.join(', ') }}</template><template v-if="(skipped ?? 0) > 0"> · {{ skipped }} unreadable event line{{ skipped === 1 ? '' : 's' }}</template>
    </text>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
