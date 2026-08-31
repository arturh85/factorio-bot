<script setup lang="ts">
/**
 * The mount point for a run's replay: what the scheduler planned against what
 * the run observed, two rows per step. See `crates/executor/src/replay.rs`
 * for the document this renders.
 *
 * **This view shows the executor's conclusions, not the world.** Every row is
 * a claim about what the executor believes happened; a step's `Success` means
 * its ticks were measured, not that the bot arrived -- see `ReplayStepRow`
 * and `EvidenceMark` for how that is carried down to a single row. This
 * component's own job is the two facts that only make sense at the level of
 * the whole document:
 *
 * - `refused` vs. an attempted run that measured nothing. Both produce every
 *   row `Pending` with every tick `null` -- byte-for-byte identical rows --
 *   and mean opposite things. `refused` is the only field that tells them
 *   apart, so this is the only place that can caption them differently: a
 *   view that only ever renders rows draws the same picture for both, which
 *   is exactly the failure `Replay::refused` exists to prevent.
 * - `unmatched_walks`, reported rather than hidden.
 *
 * `planned_makespan` is the only makespan this document carries.
 * `replayAxisCeiling` extends it to cover any observed tick past it purely so
 * an axis has somewhere honest to end -- it is not, and must not be
 * presented as, an observed total. See `replay.ts`.
 */
import {computed} from 'vue';
import {AlertTriangle, Ban} from '@lucide/vue';
import {Replay, observedOrigin, replayAxisCeiling} from '@/api/replay';
import {LEGEND_ENTRIES} from './statusVisual';
import ReplayStepRow from './ReplayStepRow.vue';

const props = defineProps<{
  replay: Replay | null;
  /** Set when the most recently received document failed to parse. */
  parseError?: string | null;
}>();

/**
 * `refused: null` with every row `Pending`: dispatched and measured nothing.
 * Real and alarming -- unlike `refused` non-null, which is unremarkable.
 */
const attemptedButUnmeasured = computed(() =>
    props.replay !== null &&
    props.replay.refused === null &&
    props.replay.steps.length > 0 &&
    props.replay.steps.every((step) => step.status === 'Pending'));

const axisCeiling = computed(() => (props.replay !== null ? replayAxisCeiling(props.replay) : 0));
const origin = computed(() => (props.replay === null ? 0 : observedOrigin(props.replay)));
</script>

<template>
  <div v-if="replay === null" data-testid="empty-state" class="p-4 text-sm text-ink-muted">
    {{ parseError ? 'the last replay document did not parse: ' + parseError : 'no run has produced a replay yet' }}
  </div>

  <div v-else class="flex flex-col gap-3">
    <!--
      `refused` non-null: the run never started. Captioned as such and the
      plan below is greyed -- it shows what WOULD have run, not what did.
    -->
    <div
      v-if="replay.refused !== null"
      data-testid="refused-banner"
      class="flex items-start gap-2 rounded-card border border-ink-muted bg-surface p-3 text-sm text-ink">
      <Ban class="mt-0.5 size-4 shrink-0 text-ink-muted" aria-hidden="true"/>
      <span>This run never started, because {{ replay.refused }}. The plan below was never dispatched.</span>
    </div>

    <!--
      `refused: null` but every row `Pending`: attempted, and nothing came
      back. Byte-for-byte the same rows as the refused case above -- deliberately
      NOT the same banner, NOT the same colour, and the plan below is NOT
      greyed, because this is the alarming case and the refused one is not.
    -->
    <div
      v-else-if="attemptedButUnmeasured"
      data-testid="attempted-no-data-banner"
      class="flex items-start gap-2 rounded-card border border-danger bg-danger/10 p-3 text-sm text-ink">
      <AlertTriangle class="mt-0.5 size-4 shrink-0 text-danger-dark" aria-hidden="true"/>
      <span>This run was attempted and dispatched every step, but no step reported back anything at all.</span>
    </div>

    <div
      v-if="replay.unmatched_walks.length > 0"
      data-testid="unmatched-walks-banner"
      class="rounded-card border border-warn bg-warn/10 p-3 text-sm text-ink">
      {{ replay.unmatched_walks.length }} walk observation(s) matched no step in this schedule:
      <span v-for="(walk, i) in replay.unmatched_walks" :key="i">
        bot {{ walk.bot }} step {{ walk.bot_step_index }}<span v-if="i < replay.unmatched_walks.length - 1">, </span>
      </span>
    </div>

    <div data-testid="axis-label" class="text-xs text-ink-muted">
      ticks: 0 - {{ axisCeiling }}
    </div>

    <div
      data-testid="step-list"
      :class="replay.refused !== null ? 'opacity-50 grayscale pointer-events-none' : ''">
      <ReplayStepRow
        v-for="step in replay.steps"
        :key="step.index"
        :step="step"
        :observed-origin="origin"
        :axis-ceiling="axisCeiling"/>
    </div>

    <!--
      The key. Derived from `STATUS_VISUAL`, never restated: a hand-written
      legend is a mirror, and a mirror stops describing the view the moment a
      status is added or a colour changes, without anything going red.

      It matters more here than in most views because the whole point of this
      one is a set of distinctions -- measured against believed, a bad verdict
      against no verdict at all. A distinction the reader cannot decode is not
      a distinction that has been shown to them.
    -->
    <div data-testid="status-legend" class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-muted">
      <span
        v-for="[status, entry] in LEGEND_ENTRIES"
        :key="status"
        class="flex items-center gap-1.5"
        :data-testid="`legend-${status}`">
        <span class="inline-block h-3 w-6 rounded" :class="entry.classes"/>
        <span>{{ entry.label }}</span>
      </span>
      <span class="flex items-center gap-1.5">
        <span class="inline-block h-3 w-6 rounded border-2 border-ink-muted bg-transparent"/>
        <span>planned</span>
      </span>
    </div>

    <!--
      Always visible, rule 8: the executor makes internal move-asides that
      never appear as a step, so a bot's position can change with no row
      explaining it. Without this note, that reads as a rendering bug.
    -->
    <p data-testid="legend-note" class="text-xs italic text-ink-muted">
      Note: the executor may reposition a bot with an internal move-aside that
      never appears as a step here, so a bot's position can change between two
      rows with nothing above explaining why. That is expected, not a
      rendering error.
    </p>
  </div>
</template>
