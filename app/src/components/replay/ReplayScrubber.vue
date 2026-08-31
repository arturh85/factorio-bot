<script setup lang="ts">
/**
 * Puts captured frames beside the replay timeline, joined on one axis: both
 * `ReplayStep.observed_*_tick` and a frame's tick are `game.tick`, the same
 * clock, so the join is a direct numeric comparison -- no scaling, no time
 * conversion. See `frameJoin.ts` for the join logic this component only
 * renders the outcome of.
 *
 * This component owns the frame-specific half of the view (the tick axis,
 * the scrubber, and the current frame); the existing per-step rows are
 * `ReplayView`, embedded unchanged below so the two stay pixel-aligned: both
 * use the same `w-48` label-column-plus-`grow`-track layout, which is what
 * makes this "one axis" rather than two axes that happen to agree.
 *
 * The four honesty requirements this exists to uphold (see the task brief):
 *
 * 1. **Refuse the join on a non-overlapping tick range.** Frames are wiped
 *    per run, but a replay can be reopened from any past job, so a plan and a
 *    frame directory can simply be unrelated. `tickOverlapCheck` /
 *    `combineRunMatchChecks` decide this; a `contradicted` verdict hides the
 *    whole frame section and states why, rendering the timeline alone. A
 *    verdict that *allows* frames is never treated as proof -- see the
 *    `run-match-caveat` note, always shown alongside a permitted frame
 *    section, labelling it a detector rather than a guarantee.
 * 2. **"No frame for this moment" is a rendered state.** `frameAtTick`
 *    returning `null` renders `no-frame-state`, not an empty box and not the
 *    previous frame held over.
 * 3. **A stale frame says how stale.** Any frame found with `age > 0` renders
 *    through `frame-stale`, which states the age; only an exact match
 *    (`age === 0`) renders as `frame-current`.
 * 4. **Gaps stay visible.** Every known frame tick is marked on the axis
 *    (`frame-tick-mark`), so a dropped capture shows as literal empty space
 *    between two marks. A scrubber sitting inside such a gap is just a large
 *    `age` on requirement 3's mechanism -- there is no separate "gap" state,
 *    because inventing one would be exactly the fabricated continuity this
 *    requirement forbids.
 */
import {computed, ref, watch} from 'vue';
import {Ban, Image, ImageOff} from '@lucide/vue';
import {Replay, replayAxisCeiling} from '@/api/replay';
import {FramesManifest} from '@/api/types';
import {frameUrl} from '@/api/client';
import {combineRunMatchChecks, frameAtTick, framesForClient, tickOverlapCheck} from '@/api/frameJoin';
import Slider from '@/components/ui/Slider.vue';
import ReplayView from './ReplayView.vue';

const props = defineProps<{
  replay: Replay | null;
  /** `null`: not fetched yet, or capture has never run. Distinct from `EMPTY_MANIFEST`-shaped data, which this component treats the same way (nothing to judge). */
  manifest: FramesManifest | null;
  parseError?: string | null;
}>();

const tick = defineModel<number>('tick', {default: 0});

const axisCeiling = computed(() => (props.replay !== null ? replayAxisCeiling(props.replay) : 0));

function pct(t: number): number {
    return axisCeiling.value <= 0 ? 0 : Math.max(0, Math.min(100, (t / axisCeiling.value) * 100));
}

/**
 * Requirement 1's verdict. `null` when there is nothing to judge yet (no
 * manifest at all) -- deliberately distinct from a verdict that permits
 * frames: this component renders no frame section in that case, rather than
 * a caveat about a check that never ran.
 */
const matchVerdict = computed(() => {
    if (props.replay === null || props.manifest === null) {
        return null;
    }
    return combineRunMatchChecks([tickOverlapCheck(props.replay, props.manifest)]);
});

const showFrameSection = computed(() => matchVerdict.value?.show === true);

const availableClients = computed(() => props.manifest?.clients ?? []);
const selectedClient = ref<number | null>(null);
const effectiveClient = computed(() => selectedClient.value ?? availableClients.value[0] ?? null);

// Reset the pick when it stops being one of the manifest's own clients (a
// fresh manifest, or the previously-picked client's directory disappearing).
watch(availableClients, (clients) => {
    if (selectedClient.value !== null && !clients.includes(selectedClient.value)) {
        selectedClient.value = null;
    }
});

const clientFrames = computed(() => {
    if (props.manifest === null || effectiveClient.value === null) {
        return [];
    }
    return framesForClient(props.manifest, effectiveClient.value);
});

const unparsedFrameCount = computed(() => props.manifest?.frames.filter((f) => f.tick === null).length ?? 0);
const totalFrameCount = computed(() => props.manifest?.frames.length ?? 0);

const current = computed(() => (clientFrames.value.length > 0 ? frameAtTick(clientFrames.value, tick.value) : null));
</script>

<template>
  <div class="flex flex-col gap-3">
    <div
      v-if="matchVerdict !== null && !matchVerdict.show"
      data-testid="run-mismatch-banner"
      class="flex items-start gap-2 rounded-card border border-danger bg-danger/10 p-3 text-sm text-ink">
      <Ban class="mt-0.5 size-4 shrink-0 text-danger-dark" aria-hidden="true"/>
      <span>No frames are shown: {{ matchVerdict.reason }}.</span>
    </div>

    <div v-if="showFrameSection" data-testid="frame-section" class="flex flex-col gap-2">
      <!--
        Never a claim this HAS matched -- only that nothing contradicted it.
        Overlapping ticks are necessary, not sufficient: two different runs
        can share a tick range. See `combineRunMatchChecks`.
      -->
      <p data-testid="run-match-caveat" class="text-xs italic text-ink-muted">
        Frames shown below are matched to this run by overlapping tick ranges only -- a partial check, not a
        guarantee of a match.
      </p>

      <div v-if="availableClients.length > 1" class="flex items-center gap-2 text-xs text-ink-muted">
        <label for="frame-client-select">camera client</label>
        <select
          id="frame-client-select"
          data-testid="client-select"
          class="rounded border border-divider bg-card px-1 py-0.5 text-ink"
          :value="effectiveClient"
          @change="selectedClient = Number(($event.target as HTMLSelectElement).value)">
          <option v-for="c in availableClients" :key="c" :value="c">client {{ c }}</option>
        </select>
      </div>

      <!-- The frame axis: same label-column + grow-track layout as `ReplayStepRow`, so its marks line up with the step bars beneath it. -->
      <div class="flex items-center gap-3 text-xs">
        <div class="w-48 shrink-0 text-ink-muted">frame ticks</div>
        <div data-testid="frame-axis" class="relative h-4 grow rounded bg-surface">
          <span
            v-for="frame in clientFrames"
            :key="frame.name"
            data-testid="frame-tick-mark"
            :data-tick="frame.tick"
            class="absolute top-0 h-full w-0.5 -translate-x-1/2 bg-brand"
            :style="{left: pct(frame.tick as number) + '%'}"
            :title="`frame at tick ${frame.tick}`"/>
        </div>
      </div>

      <!-- The scrubber itself, on the same track width as the axis above. -->
      <div class="flex items-center gap-3 text-xs">
        <div class="w-48 shrink-0 text-ink-muted">scrubber: tick {{ tick }}</div>
        <div class="grow">
          <Slider
            :model-value="tick"
            :min="0"
            :max="Math.max(axisCeiling, 1)"
            :step="1"
            label="replay scrubber"
            @update:model-value="tick = $event"/>
        </div>
      </div>

      <p
        v-if="totalFrameCount > 0"
        data-testid="frame-count"
        class="text-xs text-ink-muted">
        {{ clientFrames.length }} of {{ totalFrameCount }} captured frame(s) shown for this client
        <span v-if="unparsedFrameCount > 0">({{ unparsedFrameCount }} could not be read from their filename and are excluded from the axis)</span>
      </p>

      <!-- The current frame, or the appropriate honest state. -->
      <div class="flex items-center gap-3">
        <div class="w-48 shrink-0"/>
        <div class="grow">
          <div v-if="current === null" data-testid="no-frame-state" class="flex items-center gap-2 text-sm text-ink-muted">
            <ImageOff class="size-4 shrink-0" aria-hidden="true"/>
            <span>no frame captured for tick {{ tick }} yet</span>
          </div>

          <div v-else class="flex flex-col gap-1">
            <img
              data-testid="frame-image"
              :src="frameUrl(current.frame.client, current.frame.name)"
              :alt="`captured frame at tick ${current.frame.tick}`"
              class="max-h-64 w-auto rounded border border-divider"/>

            <p v-if="current.age === 0" data-testid="frame-current" class="flex items-center gap-1 text-xs text-ink-muted">
              <Image class="size-3.5 shrink-0" aria-hidden="true"/>
              showing tick {{ current.frame.tick }} exactly
            </p>
            <p v-else data-testid="frame-stale" class="flex items-center gap-1 text-xs text-warn-dark">
              <Image class="size-3.5 shrink-0" aria-hidden="true"/>
              showing tick {{ current.frame.tick }} -- {{ current.age }} tick(s) old, the most recent capture at or
              before tick {{ tick }}
            </p>
          </div>
        </div>
      </div>
    </div>

    <ReplayView :replay="replay" :parse-error="parseError"/>
  </div>
</template>
