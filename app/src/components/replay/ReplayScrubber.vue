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
 *
 * The video half has **three states of its own**, and they are not the frame
 * states renamed. `frame-current` / `frame-stale` have no video analogue at
 * all: a video has a picture at every timestamp, so "stale" is meaningless and
 * its *absence* must not be allowed to read as "current".
 *
 * - `video-out-of-range` -- the tick is before the recording's first sample or
 *   after its last. Not clamped to second 0, which shows a different moment.
 * - `video-clock-unknown` -- the tick falls in a stall. **This covers the video
 *   element rather than annotating it.** The frame underneath is a real frame of
 *   a real moment, and that moment is not this tick; leaving it visible with a
 *   caption is exactly the fabricated continuity requirement 4 forbids for
 *   frames.
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
import {Ban, Image, ImageOff, VideoOff} from '@lucide/vue';
import {Replay, observedOrigin, replayAxisCeiling} from '@/api/replay';
import {FramesManifest, VideoManifest, VideoTicksResponse} from '@/api/types';
import {frameUrl} from '@/api/client';
import {camerasForClient, combineRunMatchChecks, frameAtTick, framesForClient, runIdCheck, staleClients, tickOverlapCheck} from '@/api/frameJoin';
import {clockTickRange, parseVideoClock, tickToVideoSeconds} from '@/api/videoClock';
import {videoDefects} from '@/api/videoJoin';
import Slider from '@/components/ui/Slider.vue';
import ReplayView from './ReplayView.vue';

const props = defineProps<{
  replay: Replay | null;
  /** `null`: not fetched yet, or capture has never run. Distinct from `EMPTY_MANIFEST`-shaped data, which this component treats the same way (nothing to judge). */
  manifest: FramesManifest | null;
  parseError?: string | null;
  /**
   * The job this replay came from, compared against the manifest's opaque run
   * identifier. `null` when unknown, which makes that check inconclusive
   * rather than a mismatch.
   */
  jobId?: string | null;
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
 * its first 1.4%. Frame ticks are absolute, so they are shifted onto this axis
 * for display and shifted back for the lookup: `frameAtTick` is asked in the
 * clock the filenames are written in, which is the only clock the frames know.
 */
const origin = computed(() => (props.replay !== null ? observedOrigin(props.replay) : 0));

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
    // Two checks, deliberately ordered cheapest-first and both kept. The
    // range check stays useful even with an identifier: it is the filter that
    // needs no cooperation from the capture side, so it still answers when a
    // run was captured without an id.
    return combineRunMatchChecks([
        tickOverlapCheck(props.replay, props.manifest),
        runIdCheck(props.manifest.run, props.jobId ?? null)
    ]);
});

const showFrameSection = computed(() => matchVerdict.value?.show === true);

const availableClients = computed(() => props.manifest?.clients ?? []);

/**
 * Clients whose frames belong to a different run than this replay's.
 *
 * A client that sat out this run keeps the previous run's frames on disk, and
 * they list in the manifest looking exactly like current ones. Marking the
 * client in the picker is the whole point of the manifest reporting each
 * client's own run id: the frames stay reachable — they are real frames of a
 * real run — but nobody is told they show this one.
 */
const staleClientSet = computed(
    () => new Set(props.manifest !== null ? staleClients(props.manifest, props.jobId ?? null) : []));
const selectedClient = ref<number | null>(null);
const effectiveClient = computed(() => selectedClient.value ?? availableClients.value[0] ?? null);

// Reset the pick when it stops being one of the manifest's own clients (a
// fresh manifest, or the previously-picked client's directory disappearing).
watch(availableClients, (clients) => {
    if (selectedClient.value !== null && !clients.includes(selectedClient.value)) {
        selectedClient.value = null;
    }
});

/**
 * The cameras this client actually produced frames for, derived from the
 * manifest rather than from a list this component knows. A camera added to the
 * capture appears here without a frontend change; one that stopped producing
 * disappears.
 */
const availableCameras = computed(() =>
    props.manifest === null || effectiveClient.value === null
        ? []
        : camerasForClient(props.manifest, effectiveClient.value));

const selectedCamera = ref<string | null>(null);
const effectiveCamera = computed(() => {
    const cameras = availableCameras.value;
    if (selectedCamera.value !== null && cameras.includes(selectedCamera.value)) {
        return selectedCamera.value;
    }
    return cameras[0] ?? null;
});

// Clear a selection the manifest no longer offers, rather than leaving a
// camera selected that produces nothing: a stale selection would render "no
// frame at this tick" for every tick and look like a capture failure.
watch(availableCameras, (cameras) => {
    if (selectedCamera.value !== null && !cameras.includes(selectedCamera.value)) {
        selectedCamera.value = null;
    }
});

const clientFrames = computed(() => {
    if (props.manifest === null || effectiveClient.value === null) {
        return [];
    }
    return framesForClient(props.manifest, effectiveClient.value, effectiveCamera.value ?? undefined);
});

const unparsedFrameCount = computed(() => props.manifest?.frames.filter((f) => f.tick === null).length ?? 0);
const totalFrameCount = computed(() => props.manifest?.frames.length ?? 0);

// `tick` is on the shifted axis; frames are keyed by absolute `game.tick`.
// The conversion lives here and nowhere else, so there is one place where the
// two clocks meet rather than a subtraction scattered through the template.
const current = computed(() =>
    clientFrames.value.length > 0 ? frameAtTick(clientFrames.value, tick.value + origin.value) : null);

// --- the video half -------------------------------------------------------

/** `null` when there is no recording to place anything in. */
const clock = computed(() =>
    parseVideoClock(props.videoManifest ?? null, props.videoTicks ?? null));

const showVideo = computed(() => clock.value !== null && (props.videoSrc ?? null) !== null);

/**
 * Where in the recording the cursor is.
 *
 * **The same `origin` the frame lookup uses.** `observedOrigin()` converts
 * between the shifted axis and `game.tick` in exactly one place, and adding a
 * second conversion site here would break that invariant for a saving of
 * nothing.
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
          <option v-for="c in availableClients" :key="c" :value="c">
            client {{ c }}{{ staleClientSet.has(c) ? ' — frames from another run' : '' }}
          </option>
        </select>
      </div>

      <!--
        Only when there is a choice to make. One camera needs no selector, and
        an empty one would imply the capture supports something it does not.
      -->
      <div v-if="availableCameras.length > 1" class="flex items-center gap-2 text-xs text-ink-muted">
        <label for="frame-camera-select">camera</label>
        <select
          id="frame-camera-select"
          data-testid="camera-select"
          class="rounded border border-divider bg-card px-1 py-0.5 text-ink"
          :value="effectiveCamera"
          @change="selectedCamera = ($event.target as HTMLSelectElement).value">
          <option v-for="cam in availableCameras" :key="cam" :value="cam">{{ cam }}</option>
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
            :style="{left: pct((frame.tick as number) - origin) + '%'}"
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

    <!--
      The video half. Deliberately outside `showFrameSection`: a run can record
      video and no frames, and hiding the recording because there is nothing to
      judge the *frames* against would lose the only artefact it has.
    -->
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
