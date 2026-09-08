<!-- app/src/components/run/RunSidePanel.vue -->
<script setup lang="ts">
import {computed, ref, watchEffect} from 'vue';
import {Bounds, EntitySnapshot, MapRecord, Position, VideoManifest, VideoTicksResponse} from '@/api/types';
import {parseVideoClock, tickToVideoSeconds, videoSecondsToTick} from '@/api/videoClock';
import {videoDefects} from '@/api/videoJoin';
import {BotDot} from '@/lib/mapFeatures';
import MapPanel from '@/components/MapPanel.vue';

const props = defineProps<{
    runId: string; cursor: number;
    video: VideoManifest | null; videoTicks: VideoTicksResponse | null; videoError: string | null;
    entities: EntitySnapshot[]; bots: BotDot[]; trail: Record<number, Position[]>; records: MapRecord[]; bounds: Bounds | null;
    mapError: string | null; fills: Map<string, string | null>;
}>();
const emit = defineEmits<{seek: [tick: number]; pause: []}>();

const hasVideo = computed(() => props.video?.video != null);
const tab = ref<'map' | 'video'>('map');
watchEffect(() => { if (!hasVideo.value) tab.value = 'map'; });

const videoClock = computed(() => parseVideoClock(props.video, props.videoTicks));
const videoSrc = computed(() => (hasVideo.value ? `/api/v1/runs/${props.runId}/video/file` : null));
const videoAt = computed(() => (videoClock.value === null ? null : tickToVideoSeconds(videoClock.value, props.cursor)));
const videoRange = computed(() => props.video?.tick_range ?? null);
const videoIssues = computed(() => (props.video === null ? [] : videoDefects(props.video)));
const videoEl = ref<HTMLVideoElement | null>(null);

// See the header comment that used to live in RunsPage.vue: two directions,
// one loop, broken by a quarter-second slop and a driving flag.
const SYNC_SLOP_S = 0.25;
const videoDriving = ref(false);
watchEffect(() => {
    const at = videoAt.value, el = videoEl.value;
    if (el === null || at === null || videoDriving.value) return;
    if (Math.abs(el.currentTime - at.seconds) > SYNC_SLOP_S) el.currentTime = at.seconds;
});
function onVideoTime() {
    const el = videoEl.value, clock = videoClock.value;
    if (el === null || clock === null || el.paused) return;
    const at = videoSecondsToTick(clock, el.currentTime);
    if (at === null) return;
    videoDriving.value = true;
    emit('seek', at.tick);
    setTimeout(() => (videoDriving.value = false), 0);
}
function onVideoPlay() { emit('pause'); }
</script>

<template>
  <section class="border-t border-divider">
    <div role="tablist" class="flex gap-1 border-b border-divider bg-surface px-3 pt-2">
      <button role="tab" type="button" :aria-selected="tab === 'map'" class="rounded-t border border-b-0 border-divider px-3 py-1 text-sm"
              :class="tab === 'map' ? 'bg-card text-ink' : 'text-ink-muted'" @click="tab = 'map'">Map</button>
      <button v-if="hasVideo" role="tab" type="button" :aria-selected="tab === 'video'" class="rounded-t border border-b-0 border-divider px-3 py-1 text-sm"
              :class="tab === 'video' ? 'bg-card text-ink' : 'text-ink-muted'" @click="tab = 'video'">Video</button>
    </div>
    <div v-if="tab === 'map'" class="p-3">
      <p v-if="mapError" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ mapError }}</p>
      <MapPanel v-else :entities="entities" :bots="bots" :trail="trail" :records="records" :bounds="bounds" :fills="fills"/>
    </div>
    <div v-else class="p-3">
      <p v-if="videoError" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ videoError }}</p>
      <p v-for="d in videoIssues" :key="d.kind" class="rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn-dark">{{ d.message }}</p>
      <video ref="videoEl" :src="videoSrc ?? undefined" preload="metadata" controls class="block max-h-[60vh] max-w-full border border-divider"
             @timeupdate="onVideoTime" @seeked="onVideoTime" @play="onVideoPlay"/>
      <p v-if="video?.video" class="mt-1 font-mono text-xs text-ink-muted">
        {{ video.video.width }}x{{ video.video.height }} · {{ video.video.fps }} fps · {{ ((video.bytes ?? 0) / 1048576).toFixed(0) }} MB ·
        <template v-if="videoAt">at {{ videoAt.seconds.toFixed(1) }}s</template>
        <template v-else-if="videoRange && cursor < videoRange.from">
          recording starts at <button type="button" class="underline" @click="emit('seek', videoRange.from)">tick {{ videoRange.from }}</button>
        </template>
        <template v-else-if="videoRange && cursor > videoRange.to">recording ended at tick {{ videoRange.to }}</template>
        <template v-else>the clock cannot place tick {{ cursor }}</template>
      </p>
    </div>
  </section>
</template>
