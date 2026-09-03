<script setup lang="ts">
/**
 * Puts a run's recording beside its replay timeline, joined on one axis: both
 * `ReplayStep.observed_*_tick` and the recording's clock are `game.tick`, the
 * same clock, so the join is a direct numeric comparison -- no scaling, no time
 * conversion. See `videoClock.ts` for the interpolation and `videoJoin.ts` for
 * the run-match and defect logic this component only renders the outcome of.
 *
 * This component owns the tick axis, the scrubber and the video; the per-step
 * rows are `ReplayView`, embedded unchanged below so the two stay
 * pixel-aligned: both use the same `w-48` label-column-plus-`grow`-track
 * layout, which is what makes this "one axis" rather than two axes that happen
 * to agree.
 *
 * **The per-camera screenshots this component was built around are gone**
 * (retired 2026-09-02), and with them the client/camera pickers, the frame
 * axis and the `frame-current` / `frame-stale` captions. The video is the
 * visual record now, and its three states are **not** those frame states
 * renamed -- a video has a picture at every timestamp, so "stale" is
 * meaningless and its *absence* must not be allowed to read as "current":
 *
 * - `video-out-of-range` -- the tick is before the recording's first sample or
 *   after its last. Not clamped to second 0, which shows a different moment.
 * - `video-clock-unknown` -- the tick falls in a stall. **This covers the video
 *   element rather than annotating it.** The frame underneath is a real frame of
 *   a real moment, and that moment is not this tick; leaving it visible with a
 *   caption would fabricate a continuity the recording does not have.
 * - `video-clock-unverified` -- the recording's rate was never checked, or was
 *   checked and failed. Shown for the whole run, because the offset may be right
 *   at the start and wrong by the end and nothing can say where.
 *
 * The seek itself is coalesced: assigning `currentTime` on every slider `input`
 * produces a seek storm, so assignments are deferred to an animation frame and
 * skipped while the element is already seeking, with the *last* requested
 * position applied once the seek completes.
 */
import {computed, onBeforeUnmount, ref, watch} from 'vue';
import {VideoOff} from '@lucide/vue';
import {Replay, observedOrigin, replayAxisCeiling} from '@/api/replay';
import {VideoManifest, VideoTicksResponse} from '@/api/types';
import {clockTickRange, parseVideoClock, tickToVideoSeconds} from '@/api/videoClock';
import {videoDefects} from '@/api/videoJoin';
import Slider from '@/components/ui/Slider.vue';
import ReplayView from './ReplayView.vue';

const props = defineProps<{
  replay: Replay | null;
  parseError?: string | null;
  /** `null`: not fetched, or the request failed. A run with no video answers a manifest, not a null. */
  videoManifest?: VideoManifest | null;
  videoTicks?: VideoTicksResponse | null;
  /**
   * Where the recording's bytes live. The caller decides live or archived, so
   * this component never has to know which run it is looking at.
   */
  videoSrc?: string | null;
}>();

const tick = defineModel<number>('tick', {default: 0});

const axisCeiling = computed(() => (props.replay !== null ? replayAxisCeiling(props.replay) : 0));

/**
 * The absolute `game.tick` that axis position 0 corresponds to.
 *
 * The scrubber works in **shifted** ticks, matching the step rows below it —
 * planned ticks count from zero while `game.tick` was already at 60,551 when
 * this run's first step dispatched, so an unshifted axis put the whole plan in
 * its first 1.4%. The recording's clock is absolute, so the cursor is shifted
 * back before it is asked anything: `tickToVideoSeconds` is asked in
 * `game.tick`, the only clock the recording knows.
 */
const origin = computed(() => (props.replay !== null ? observedOrigin(props.replay) : 0));

/** `null` when there is no recording to place anything in. */
const clock = computed(() =>
    parseVideoClock(props.videoManifest ?? null, props.videoTicks ?? null));

const showVideo = computed(() => clock.value !== null && (props.videoSrc ?? null) !== null);

/**
 * Where in the recording the cursor is.
 *
 * **`observedOrigin()` converts between the shifted axis and `game.tick` in
 * exactly one place**, and adding a second conversion site here would break
 * that invariant for a saving of nothing.
 */
const videoAt = computed(() =>
    clock.value === null ? null : tickToVideoSeconds(clock.value, tick.value + origin.value));

/**
 * Which of the three states the video is in. `null` and `out-of-range` are
 * different answers: one means there is no recording, the other means there is
 * one and this moment is not in it.
 */
const videoState = computed<'ok' | 'out-of-range' | 'clock-unknown' | null>(() => {
    if (!showVideo.value || clock.value === null) return null;
    if (videoAt.value !== null) return 'ok';
    const range = clockTickRange(clock.value);
    const absolute = tick.value + origin.value;
    if (range === null || absolute < range.from || absolute > range.to) return 'out-of-range';
    // Inside the recording's span, and the clock still will not say where: the
    // cursor is in a stall.
    return 'clock-unknown';
});

const videoUnverified = computed(() => clock.value !== null && !clock.value.rateVerified);

/** What is wrong with the recording itself, as distinct from whether it is ours. */
const defects = computed(() =>
    props.videoManifest ? videoDefects(props.videoManifest) : []);

const videoEl = ref<HTMLVideoElement | null>(null);
/** The last position asked for but not yet applied. */
let pendingSeek: number | null = null;
let frameHandle: number | null = null;

function applyPendingSeek(): void {
    frameHandle = null;
    const element = videoEl.value;
    if (element === null || pendingSeek === null) return;
    // Already seeking: leave the request pending. `onSeeked` applies the last
    // one, so the final position is right even when intermediate ones are
    // dropped.
    if (element.seeking) return;
    const target = pendingSeek;
    pendingSeek = null;
    // Within one frame period of where it already is, a seek buys nothing and
    // costs a decode.
    if (Math.abs(element.currentTime - target) < 1 / 60) return;
    element.currentTime = target;
}

function requestSeek(seconds: number): void {
    pendingSeek = seconds;
    if (frameHandle !== null) return;
    frameHandle = requestAnimationFrame(applyPendingSeek);
}

function onSeeked(): void {
    if (pendingSeek !== null) applyPendingSeek();
}

watch(videoAt, (position) => {
    if (position !== null) requestSeek(position.seconds);
});

onBeforeUnmount(() => {
    if (frameHandle !== null) cancelAnimationFrame(frameHandle);
});
</script>

<template>
  <div class="flex flex-col gap-3">
    <!-- The scrubber. Same label-column + grow-track layout as `ReplayStepRow`,
         so it lines up with the step bars beneath it. -->
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

    <!-- The video half. Absent entirely when the run recorded nothing, which
         is every run that did not ask -- there is no empty state here, because
         a panel explaining its own emptiness on every run page is what the
         retired frames panel already was. -->
    <div v-if="showVideo" data-testid="video-section" class="flex flex-col gap-2">
      <p
        v-if="videoUnverified"
        data-testid="video-clock-unverified"
        class="text-xs italic text-warn-dark">
        This recording's rate was never confirmed against the host clock, so every position in it is
        approximate -- the offset may be right at the start and wrong by the end.
      </p>

      <p
        v-for="defect in defects"
        :key="defect.kind"
        data-testid="video-defect"
        :data-kind="defect.kind"
        class="text-xs text-warn-dark">
        {{ defect.message }}
      </p>

      <div class="flex items-center gap-3">
        <div class="w-48 shrink-0 text-xs text-ink-muted">video</div>
        <div class="grow">
          <div
            v-if="videoState === 'out-of-range'"
            data-testid="video-out-of-range"
            class="flex items-center gap-2 text-sm text-ink-muted">
            <VideoOff class="size-4 shrink-0" aria-hidden="true"/>
            <span>tick {{ tick }} is outside this recording</span>
          </div>

          <!--
            The element stays mounted through a stall so the browser keeps its
            buffers, but it is COVERED, not captioned: the frame underneath is a
            real frame of a real moment, and that moment is not this tick.
          -->
          <div v-else class="relative w-fit">
            <video
              ref="videoEl"
              data-testid="video-element"
              :src="videoSrc ?? undefined"
              preload="metadata"
              controls
              class="max-h-64 w-auto rounded border border-divider"
              @seeked="onSeeked"/>
            <div
              v-if="videoState === 'clock-unknown'"
              data-testid="video-clock-unknown"
              class="absolute inset-0 flex items-center justify-center rounded bg-surface/95 p-2 text-center text-xs text-ink">
              the clock has no reading for tick {{ tick }} -- the game stalled here, and the picture
              at this position is of some other moment
            </div>
          </div>
        </div>
      </div>
    </div>

    <ReplayView :replay="replay" :parse-error="parseError"/>
  </div>
</template>
