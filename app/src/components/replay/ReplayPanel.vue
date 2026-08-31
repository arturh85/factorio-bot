<script setup lang="ts">


// The `/tasks` panel: a run's replay, and the frames captured during it.
//
// Named `GanttChart.vue` until it rendered anything. It began as a
// `<div>TODO</div>` holding a commented-out mermaid gantt sketch, and that
// sketch is gone rather than carried: mermaid's gantt vocabulary is one bar
// per row with a label, a start and a duration, and this needs two rows per
// step, a distinct `Lost`, evidence marking and a scrubber. None of that is
// expressible in it, so the sketch was a design that had been rejected on the
// merits and kept out of politeness. Its intent is recorded in
// `docs/superpowers/notes/2026-08-31-plan-and-map-view-scoping.md`.
//
// See `crates/executor/src/replay.rs` for the document and `ReplayScrubber.vue`
// for the rendering.
import {computed, onMounted, onUnmounted} from 'vue';
import {useReplayStore} from '@/store/replayStore';
import ReplayScrubber from './ReplayScrubber.vue';

const replayStore = useReplayStore();
const replay = computed(() => replayStore.getReplay);
const parseError = computed(() => replayStore.getParseError);
const manifest = computed(() => replayStore.getManifest);
const jobId = computed(() => replayStore.getJobId);

onMounted(() => {
  void replayStore.refresh();
});

// The live subscription is module-level in the store (see replayStore.ts),
// exactly like scriptStore's: it outlives this component otherwise, and
// leaving it attached would have the next mount's first refresh see a
// `watchedJobId` left over from a page nobody is looking at any more.
onUnmounted(() => {
  replayStore.stopWatching();
});
</script>

<template>
  <ReplayScrubber :replay="replay" :manifest="manifest" :parse-error="parseError" :job-id="jobId"/>
</template>
