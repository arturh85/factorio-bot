<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {ForceSample} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]}>();
const H = 70;
const x = (t: number) => tickX(props.scale, t);

const force = computed(() => props.samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick));
const max = computed(() => Math.max(1000, Math.ceil(Math.max(0, ...force.value.map((f) => f.power.generated_kw)) / 500) * 500));
const y = (kw: number) => H - 8 - (kw / max.value) * (H - 22);
const gen = computed(() => force.value.map((f, k) => `${k ? 'L' : 'M'}${x(f.tick)},${y(f.power.generated_kw)}`).join(' '));
const cons = computed(() => force.value.map((f, k) => `${k ? 'L' : 'M'}${x(f.tick)},${y(f.power.consumed_kw)}`).join(' '));
const firstGen = computed(() => force.value.find((f) => f.power.generated_kw > 0) ?? null);
const deficits = computed(() => force.value.flatMap((f, k) => {
    const short = Object.values(f.power.networks).some((n) => n.demanded_kw > n.generated_kw);
    if (!short) return [];
    const prev = k > 0 ? force.value[k - 1].tick : f.tick - 300;
    return [{from: prev, to: f.tick}];
}));
const gridKw = computed(() => [0, max.value / 2, max.value]);
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="power generated and consumed">
    <template v-if="force.length === 0">
      <text :x="AXIS_WIDTH / 2" :y="H / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no power samples</text>
    </template>
    <template v-else>
      <template v-for="kw in gridKw" :key="kw">
        <line x1="0" :y1="y(kw)" :x2="AXIS_WIDTH" :y2="y(kw)" stroke="var(--color-plot-grid)" stroke-width="1"/>
        <text x="4" :y="y(kw) - 2" font-size="9" fill="var(--color-ink-muted)">{{ kw }} kW</text>
      </template>
      <rect v-for="d in deficits" :key="d.to" class="deficit" :x="x(d.from)" y="0" :width="Math.max(1, x(d.to) - x(d.from))" :height="H"
            fill="var(--color-status-critical)" opacity="0.25"/>
      <path :d="gen" fill="none" stroke="var(--color-verb-research)" stroke-width="1.8"/>
      <path :d="cons" fill="none" stroke="var(--color-ink-muted)" stroke-width="1.6" stroke-dasharray="4 3"/>
      <template v-if="firstGen">
        <line :x1="x(firstGen.tick)" y1="8" :x2="x(firstGen.tick)" :y2="H - 8" stroke="var(--color-verb-research)" stroke-width="1" stroke-dasharray="2 3"/>
        <text :x="x(firstGen.tick) - 4" y="16" font-size="10" text-anchor="end" fill="var(--color-ink-muted)">no generator until {{ formatGameTime(scale, firstGen.tick) }}</text>
        <text :x="x(firstGen.tick) + 5" :y="y(firstGen.power.generated_kw) + 12" font-size="10" font-weight="500" fill="var(--color-verb-research)">generated {{ firstGen.power.generated_kw.toFixed(0) }} kW</text>
      </template>
      <text v-else :x="AXIS_WIDTH - 4" y="16" font-size="10" text-anchor="end" fill="var(--color-ink-muted)">no generation in this run</text>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
