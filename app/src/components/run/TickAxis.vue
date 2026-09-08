<script setup lang="ts">
import {computed} from 'vue';
import {AXIS_WIDTH, formatGameTime, markTicks, TickScale, tickX, TICKS_PER_MINUTE} from '@/lib/tickScale';

/**
 * `scale` is the drawn axis (positions); `clock` is the analysis window
 * (labels and marks). See the header of `@/lib/tickScale`.
 */
const props = defineProps<{scale: TickScale; clock: TickScale; cursor: number}>();

// Unlabelled hairlines dividing the DRAWN axis into minutes -- they say
// nothing about game time, so they come off `scale` and stay there.
const minutes = computed(() => {
    const out: number[] = [];
    for (let t = props.scale.from + TICKS_PER_MINUTE; t <= props.scale.to; t += TICKS_PER_MINUTE) out.push(t);
    return out;
});
// The tool's fixed 5-minute marks, off the analysis clock. A mark outside
// the drawn axis is dropped rather than clamped onto its edge, where it
// would name a time that is not there.
const marks = computed(() => markTicks(props.clock).filter((t) => t >= props.scale.from && t <= props.scale.to));
const x = (t: number) => tickX(props.scale, t);
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} 30`" class="block h-auto w-full" data-testid="tick-axis" aria-label="game-time axis">
    <line x1="0" y1="22" :x2="AXIS_WIDTH" y2="22" stroke="var(--color-divider)" stroke-width="1"/>
    <line v-for="t in minutes" :key="`m${t}`" class="minute" :x1="x(t)" y1="15" :x2="x(t)" y2="22"
          stroke="var(--color-ink-muted)" stroke-width="1"/>
    <template v-for="t in marks" :key="`k${t}`">
      <line class="mark" :x1="x(t)" y1="8" :x2="x(t)" y2="22" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
      <text :x="x(t) + 4" y="12" font-size="10" fill="var(--color-verdict-roster)" font-family="ui-monospace, monospace">
        {{ formatGameTime(clock, t) }} mark
      </text>
    </template>
    <text :x="AXIS_WIDTH - 2" y="12" font-size="10" text-anchor="end" fill="var(--color-ink-muted)" font-family="ui-monospace, monospace">
      tick {{ scale.to.toLocaleString() }}
    </text>
    <line data-testid="cursor" :x1="x(cursor)" y1="0" :x2="x(cursor)" y2="30" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
