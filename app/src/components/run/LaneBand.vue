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
import {idleIntervals, idleTicks, laneSegments, LaneSegment, refusedReplans, replanBoundaries} from '@/lib/runIdle';
import {laneBots} from '@/lib/runTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

/**
 * `scale` is the drawn axis (positions); `clock` is the analysis window
 * (labels, and the idle share's denominator and interval bound) -- the idle
 * percentage is the share of the run the headline talks about, not of
 * whatever the axis happens to be trimmed to. See the header of
 * `@/lib/tickScale`.
 */
const props = defineProps<{scale: TickScale; clock: TickScale; cursor: number; lanes: Lane[]; events: Event[]}>();
const ROW = 30;
const x = (t: number) => tickX(props.scale, t);
const bots = computed(() => laneBots(props.lanes));
const height = computed(() => Math.max(ROW, bots.value.length * ROW));
const boundaries = computed(() => replanBoundaries(props.events));
const refused = computed(() => refusedReplans(props.events));
const segments = computed(() => laneSegments(props.lanes, boundaries.value));
const idlePct = computed(() => Object.fromEntries(bots.value.map((b) => {
    const span = props.clock.to - props.clock.from;
    return [b, span > 0 ? Math.round((idleTicks(idleIntervals(props.lanes, b, props.clock)) / span) * 100) : 0];
})));
const botColor = (b: number) => `var(--color-bot-${Math.min(8, Math.max(1, b))})`;
const rowY = (b: number) => bots.value.indexOf(b) * ROW;

function width(s: LaneSegment): number {
    if (s.instant) return 1.6;
    const end = s.to === null ? props.scale.to : s.to;
    return Math.max(1.6, x(end) - x(s.from));
}
// An abandoned step never dispatched -- the executor wrote its settle for a
// step a predecessor's failure or halted walk kept from ever starting. Drawn
// hollow so it reads as "never happened" rather than as a span of work, and
// excluded from `idleIntervals` above for the same reason.
function fill(s: LaneSegment): string {
    return s.status === 'abandoned' ? 'none' : `var(--color-verb-${s.verb})`;
}
function segStroke(s: LaneSegment): string {
    if (s.status === 'abandoned') return 'var(--color-ink-muted)';
    return s.status === 'failed' || s.status === 'lost' ? 'var(--color-status-critical)' : 'none';
}
function title(s: LaneSegment): string {
    const ticks = s.to === null ? 'never settled' : `${(s.to - s.from).toLocaleString()} ticks`;
    const status = s.to === null ? '' : ` · ${s.status ?? 'no status'}`;
    const evidence = s.verb === 'walk' ? ' · evidence: believed (ticks measured, arrival not)' : '';
    const label = s.status === 'abandoned' ? `never dispatched: ${s.action}` : s.action;
    return `${label} · plan ${s.planIndex} · ${ticks}${status}${evidence}`;
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
          :fill="fill(s)" :opacity="s.to === null ? 0.5 : 1"
          :stroke="segStroke(s)" :stroke-dasharray="s.status === 'abandoned' ? '1 1' : undefined"
          :stroke-width="s.status === 'abandoned' ? 1.6 : 1.5">
      <title>{{ title(s) }}</title>
    </rect>
    <template v-for="(t, i) in boundaries" :key="`r${t}`">
      <line class="replan" :x1="x(t)" y1="0" :x2="x(t)" :y2="height" stroke="var(--color-ink)" stroke-width="1" stroke-dasharray="3 3"/>
      <text :x="x(t) + 4" :y="height - 2" font-size="9" fill="var(--color-ink-muted)">plan {{ i + 1 }} · ids restart here · {{ formatGameTime(clock, t) }}</text>
    </template>
    <template v-for="t in refused" :key="`rr${t}`">
      <line class="replan-refused" :x1="x(t)" y1="0" :x2="x(t)" :y2="height" stroke="var(--color-status-critical)" stroke-width="1.5" stroke-dasharray="4 2"/>
      <text :x="x(t) + 4" :y="14" font-size="9" fill="var(--color-status-critical)">replan refused · {{ formatGameTime(clock, t) }}</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
