<script setup lang="ts">
import {Split} from '@/api/types';
import {splitAt} from '@/lib/runTimeline';
import {AXIS_WIDTH, formatGameTime, TickScale, tickX} from '@/lib/tickScale';

/**
 * `scale` is the drawn axis (positions); `clock` is the analysis window
 * (labels) -- so a milestone's time here is the one the headline states.
 * See the header of `@/lib/tickScale`.
 */
const props = withDefaults(
    defineProps<{scale: TickScale; clock: TickScale; splits: Split[]; cursor: number; reasons?: Map<number, string>}>(),
    {reasons: () => new Map()}
);
const pct = (t: number) => `${(tickX(props.scale, t) / AXIS_WIDTH) * 100}%`;
function style(s: Split) {
    const end = s.ended_tick ?? props.scale.to;
    return {left: pct(s.started_tick), width: `calc(${(tickX(props.scale, end) - tickX(props.scale, s.started_tick)) / AXIS_WIDTH * 100}% - 2px)`};
}
function label(s: Split) {
    const at = s.ended_tick === null ? 'unfinished' : `${s.outcome} at ${formatGameTime(props.clock, s.ended_tick)}`;
    const ticks = s.elapsed_ticks === null ? '—' : s.elapsed_ticks.toLocaleString();
    return `m${s.index} · ${s.goal} · ${at} · ${ticks} ticks`;
}
/** The label, plus the planner's refusal when this split is a stuck one. */
function title(s: Split) {
    const reason = props.reasons.get(s.index);
    return reason === undefined ? label(s) : `${label(s)} · ${reason}`;
}
</script>

<template>
  <div class="relative h-9">
    <div v-for="s in splits" :key="`${s.index}-${s.started_tick}`"
         class="seg absolute top-2 h-[22px] overflow-hidden text-ellipsis whitespace-nowrap rounded border bg-verdict-roster-soft px-2 font-mono text-[11px] leading-5 text-ink"
         :class="[reasons.get(s.index) !== undefined ? 'border-status-critical' : 'border-verdict-roster',
                  {'is-current ring-2 ring-verdict-roster': splitAt(splits, cursor)?.started_tick === s.started_tick}]"
         :data-stuck="reasons.get(s.index) !== undefined ? 'true' : undefined"
         :style="style(s)" :title="title(s)">{{ label(s) }}</div>
  </div>
</template>
