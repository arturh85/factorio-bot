<script setup lang="ts">
import {computed} from 'vue';
import {Sample} from '@/api/types';
import {ForceSample} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

const props = defineProps<{scale: TickScale; cursor: number; samples: Sample[]}>();
const H = 44;
const x = (t: number) => tickX(props.scale, t);
const y = (p: number) => H - 8 - p * (H - 18);

interface Tech { name: string; points: {tick: number; progress: number}[] }

const techs = computed<Tech[]>(() => {
    const out: Tech[] = [];
    for (const s of props.samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick)) {
        if (s.research === null) continue;
        const last = out[out.length - 1];
        if (last && last.name === s.research.name) last.points.push({tick: s.tick, progress: s.research.progress});
        else out.push({name: s.research.name, points: [{tick: s.tick, progress: s.research.progress}]});
    }
    return out;
});
function area(t: Tech): string {
    const p = t.points;
    return `M${x(p[0].tick)},${y(0)} ${p.map((q) => `L${x(q.tick)},${y(q.progress)}`).join(' ')} L${x(p[p.length - 1].tick)},${y(0)} Z`;
}
function label(t: Tech): string {
    const first = t.points[0], last = t.points[t.points.length - 1];
    return `${t.name} · started ${formatGameTime(props.scale, first.tick)} · ${Math.round(last.progress * 100)}% at ${formatGameTime(props.scale, last.tick)}`;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${H}`" class="block h-auto w-full" aria-label="research progress">
    <line x1="0" :y1="y(0)" :x2="AXIS_WIDTH" :y2="y(0)" stroke="var(--color-plot-grid)"/>
    <text v-if="techs.length === 0" :x="AXIS_WIDTH / 2" :y="H / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no research queued in this run</text>
    <template v-for="t in techs" :key="`${t.name}-${t.points[0].tick}`">
      <path class="tech" :d="area(t)" fill="var(--color-item-red)" opacity="0.25"/>
      <text :x="x(t.points[0].tick) + 4" y="12" font-size="10" fill="var(--color-ink-muted)">{{ label(t) }}</text>
    </template>
    <text x="6" :y="H - 12" font-size="9" fill="var(--color-ink-muted)">trigger technologies (steam-power, electronics) are not research durations and are not drawn as bars</text>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="H" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
