<script setup lang="ts">
/**
 * Two rows for one scheduled step: what the scheduler planned, and what the
 * run actually observed. See `crates/executor/src/replay.rs` for the document
 * this renders and the absence semantics it depends on:
 *
 * - The planned bar is always drawn -- a schedule always knows where it put
 *   a step.
 * - The observed bar is drawn ONLY when both `observed_start_tick` and
 *   `observed_end_tick` are present. Ticks do not arrive in pairs: a step can
 *   be dispatched and never replied to, which is what `Lost` *is*. Drawing a
 *   bar from a known start to "now", to the planned end, or to zero length
 *   would fabricate an end the run never measured -- exactly the mistake the
 *   whole document exists to prevent. A known start with no known end (or,
 *   symmetrically, a known end with no known start -- not expected in
 *   practice, since it never emerges from `Replay::new`, but handled the same
 *   way rather than silently inventing the missing half) draws as a thin
 *   marker at the one honest position, never as a span.
 * - `Lost` and `Failed` use different icons, different colours and a
 *   different accessible label -- see `STATUS_VISUAL`. Collapsing them is the
 *   exact defect `replay.rs`'s own tests exist to catch on the Rust side.
 * - `Evidence.Believed` is rendered by `EvidenceMark`, deliberately quiet: it
 *   is the normal state of a walk row, not an outcome, and must not draw the
 *   eye the way `Failed`/`Lost` are meant to.
 */
import {computed} from 'vue';
import {RotateCcw} from '@lucide/vue';
import {ReplayStep} from '@/api/replay';
import EvidenceMark from './EvidenceMark.vue';
import {STATUS_VISUAL} from './statusVisual';

const props = defineProps<{
  /**
   * The tick observed positions are measured from — see `observedOrigin`.
   * Planned ticks count from zero at run start; observed ticks are absolute
   * `game.tick`. Without this shift the two rows share an axis their positions
   * are not comparable on.
   */
  observedOrigin?: number;step: ReplayStep; axisCeiling: number}>();

function pct(ticks: number): number {
    return props.axisCeiling <= 0 ? 0 : Math.max(0, Math.min(100, (ticks / props.axisCeiling) * 100));
}

const what = computed(() =>
    props.step.what.kind === 'act'
        ? props.step.what.label
        : `walk to (${props.step.what.to.x}, ${props.step.what.to.y})`);

const plannedStyle = computed(() => ({
    left: pct(props.step.planned_start_tick) + '%',
    width: pct(props.step.planned_end_tick - props.step.planned_start_tick) + '%'
}));


const visual = computed(() => STATUS_VISUAL[props.step.status]);

const hasFullObserved = computed(() =>
    props.step.observed_start_tick !== null && props.step.observed_end_tick !== null);

/** Exactly one of the two observed ticks is known -- a marker, never a bar. */
const partialObservedTick = computed(() => {
    const origin = props.observedOrigin ?? 0;
    const start = props.step.observed_start_tick === null ? null : props.step.observed_start_tick - origin;
    const end = props.step.observed_end_tick === null ? null : props.step.observed_end_tick - origin;
    if (start !== null && end === null) {
        return start;
    }
    if (start === null && end !== null) {
        return end;
    }
    return null;
});

const observedStyle = computed(() => {
    if (!hasFullObserved.value) {
        return {};
    }
    const start = (props.step.observed_start_tick as number) - (props.observedOrigin ?? 0);
    const end = props.step.observed_end_tick as number;
    return {left: pct(start) + '%', width: pct(end - start) + '%'};
});

const noClockAtAll = computed(() =>
    props.step.status !== 'Pending' &&
    props.step.observed_start_tick === null &&
    props.step.observed_end_tick === null);
</script>

<template>
  <div class="flex items-center gap-3 border-b border-divider py-2 text-xs">
    <div class="flex w-48 shrink-0 flex-col truncate">
      <span class="truncate">bot {{ step.bot }} -- {{ what }}</span>
      <EvidenceMark :evidence="step.evidence"/>
    </div>

    <div class="relative h-9 grow">
      <div class="absolute inset-y-2 left-0 right-0 rounded bg-surface"/>

      <div
        data-testid="planned-bar"
        class="absolute top-0.5 h-3 rounded border-2 border-ink-muted bg-transparent"
        :style="plannedStyle"
        :title="`planned ${step.planned_start_tick} - ${step.planned_end_tick}`"/>

      <div
        v-if="hasFullObserved"
        data-testid="observed-bar"
        :data-status="step.status"
        :aria-label="visual.label"
        :title="visual.label"
        class="absolute bottom-0.5 flex h-4 items-center justify-center gap-1 rounded"
        :class="visual.classes"
        :style="observedStyle">
        <component :is="visual.icon" v-if="visual.icon" class="size-3 shrink-0" aria-hidden="true"/>
      </div>

      <div
        v-else-if="partialObservedTick !== null"
        data-testid="observed-marker"
        :data-status="step.status"
        :aria-label="visual.label + ' (only one tick observed)'"
        :title="visual.label + ' -- only one tick observed, the other never arrived'"
        class="absolute bottom-0.5 h-4 w-1.5 -translate-x-1/2 rounded"
        :class="visual.classes"
        :style="{left: pct(partialObservedTick) + '%'}"/>

      <span
        v-else-if="noClockAtAll"
        class="absolute inset-y-0 left-1 flex items-center text-[0.65rem] italic text-ink-muted">
        no clock -- {{ step.status.toLowerCase() }} with no timing measurement
      </span>
    </div>

    <span
      v-if="step.attempt_number !== null && step.attempt_number > 1"
      data-testid="attempt-count"
      class="flex shrink-0 items-center gap-1 rounded-card border border-warn px-1.5 py-0.5 text-ink"
      :title="`attempted ${step.attempt_number} times`">
      <RotateCcw class="size-3" aria-hidden="true"/>
      &times;{{ step.attempt_number }}
    </span>

    <span v-if="step.error" class="w-48 shrink-0 truncate text-danger-dark" :title="step.error">
      {{ step.error }}
    </span>
  </div>
</template>
