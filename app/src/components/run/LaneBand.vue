<!-- app/src/components/run/LaneBand.vue -->
<script setup lang="ts">
/**
 * One row per bot. The row's background IS the idle state; only dispatched
 * work paints over it. Zero-length feeding acts are fixed-width ticks (they
 * would vanish at any scale), replan boundaries are dashed across every row,
 * and a walk's title says its success is believed, not measured.
 */
import {computed} from 'vue';
import {Event, Lane} from '@/api/types';
import {idleIntervals, idleTicks, laneSegments, LaneSegment, replanBoundaries} from '@/lib/runIdle';
import {laneBots} from '@/lib/runTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; lanes: Lane[]; events: Event[]}>();
const ROW = 30;
const x = (t: number) => tickX(props.scale, t);
const bots = computed(() => laneBots(props.lanes));
const height = computed(() => Math.max(ROW, bots.value.length * ROW));
const boundaries = computed(() => replanBoundaries(props.events));
const segments = computed(() => laneSegments(props.lanes, boundaries.value));
const idlePct = computed(() => Object.fromEntries(bots.value.map((b) => {
    const span = props.scale.to - props.scale.from;
    return [b, span > 0 ? Math.round((idleTicks(idleIntervals(props.lanes, b, props.scale)) / span) * 100) : 0];
})));
const botColor = (b: number) => `var(--color-bot-${Math.min(8, Math.max(1, b))})`;
const rowY = (b: number) => bots.value.indexOf(b) * ROW;

function width(s: LaneSegment): number {
    if (s.instant) return 1.6;
    const end = s.to === null ? props.scale.to : s.to;
    return Math.max(1.6, x(end) - x(s.from));
}
function title(s: LaneSegment): string {
    const ticks = s.to === null ? 'never settled' : `${(s.to - s.from).toLocaleString()} ticks`;
    const status = s.to === null ? '' : ` · ${s.status ?? 'no status'}`;
    const evidence = s.verb === 'walk' ? ' · evidence: believed (ticks measured, arrival not)' : '';
    return `${s.action} · plan ${s.planIndex} · ${ticks}${status}${evidence}`;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="bot lanes">
    <defs>
      <pattern id="lane-idle" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--color-ink-muted)" stroke-width="0.8" opacity="0.5"/>
      </pattern>
    </defs>
    <template v-for="b in bots" :key="b">
      <rect class="idle" x="0" :y="rowY(b) + 5" :width="AXIS_WIDTH" :height="ROW - 10" fill="url(#lane-idle)"/>
      <text x="6" :y="rowY(b) + ROW / 2 + 4" font-size="11" font-weight="500" :fill="botColor(b)">bot {{ b }}</text>
      <text :x="AXIS_WIDTH - 4" :y="rowY(b) + ROW / 2 + 4" font-size="9" text-anchor="end" fill="var(--color-ink-muted)">idle {{ idlePct[b] }}%</text>
    </template>
    <rect v-for="(s, i) in segments" :key="`${s.bot}-${s.planIndex}-${s.id}-${s.from}-${i}`" class="segment"
          :data-verb="s.verb" :data-plan="s.planIndex"
          :x="x(s.from)" :y="rowY(s.bot) + 7" :width="width(s)" :height="ROW - 14" rx="1"
          :fill="`var(--color-verb-${s.verb})`" :opacity="s.to === null ? 0.5 : 1"
          :stroke="s.status === 'failed' || s.status === 'lost' ? 'var(--color-status-critical)' : 'none'" stroke-width="1.5">
      <title>{{ title(s) }}</title>
    </rect>
    <template v-for="(t, i) in boundaries" :key="`r${t}`">
      <line class="replan" :x1="x(t)" y1="0" :x2="x(t)" :y2="height" stroke="var(--color-ink)" stroke-width="1" stroke-dasharray="3 3"/>
      <text :x="x(t) + 4" :y="height - 2" font-size="9" fill="var(--color-ink-muted)">plan {{ i + 1 }} · ids restart here · {{ formatGameTime(scale, t) }}</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
