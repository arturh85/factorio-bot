<!-- app/src/components/run/MachineBand.vue -->
<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {machineRows, sampleStepTicks, statusClass, statusMatrix} from '@/lib/machineTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]; selected?: string | null}>();
const emit = defineEmits<{select: [key: string]}>();
const ROW = 12;
const x = (t: number) => tickX(props.scale, t);
const rows = computed(() => machineRows(props.samples));
const matrix = computed(() => statusMatrix(props.samples, rows.value));
const step = computed(() => sampleStepTicks(props.samples) ?? 300);
const cellWidth = computed(() => x(props.scale.from + step.value) - x(props.scale.from));
const height = computed(() => Math.max(24, rows.value.length * ROW));

function shortName(name: string): string {
    return name.replace('burner-mining-drill', 'drill').replace('stone-furnace', 'furnace').replace('wooden-chest', 'chest');
}
function title(row: {name: string; key: string}, c: {tick: number; status: string | null; produced: number | null; fill: number | null}): string {
    let t = `${shortName(row.name)} #${row.key} · ${formatGameTime(props.scale, c.tick)} · ${c.status ?? 'no status'}`;
    if (c.produced !== null) t += ` · produced ${c.produced}`;
    if (c.fill !== null) t += ` · ${c.fill} items`;
    return t;
}
function fillOpacity(isContainer: boolean, fill: number | null): number {
    return isContainer ? 0.3 + 0.7 * Math.min(1, (fill ?? 0) / 100) : 1;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="machine status over time">
    <text v-if="rows.length === 0" :x="AXIS_WIDTH / 2" :y="height / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no machine samples in this run</text>
    <template v-for="(row, i) in rows" :key="row.key">
      <rect v-if="selected === row.key" class="selected" x="0" :y="i * ROW" :width="AXIS_WIDTH" :height="ROW" fill="none" stroke="var(--color-verdict-roster)" stroke-width="1"/>
      <rect v-for="c in matrix.get(row.key) ?? []" :key="c.tick" class="cell" :data-status="c.status ?? ''"
            :x="x(c.tick) - cellWidth + 0.5" :y="i * ROW + 0.5" :width="Math.max(0.5, cellWidth - 1)" :height="ROW - 1"
            :fill="`var(--color-status-${statusClass(c.status)})`" :opacity="fillOpacity(row.isContainer, c.fill)"
            style="cursor: pointer" @click="emit('select', row.key)">
        <title>{{ title(row, c) }}</title>
      </rect>
      <text class="row-label" x="6" :y="i * ROW + ROW - 3" font-size="8.5" fill="var(--color-ink)" style="cursor: pointer; paint-order: stroke" stroke="var(--color-plot)" stroke-width="2"
            @click="emit('select', row.key)">{{ shortName(row.name) }} #{{ row.key }}</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
