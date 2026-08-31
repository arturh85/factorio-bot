<script setup lang="ts">

/*
import {ref} from 'vue';
let diagram = ref('gantt\n' +
    '    title Example Diagram\n' +
    '    dateFormat HH:mm:ss\n' +
    '    axisFormat %H:%M:%S\n' +
    '    test : milestone, m1, 00:03:19,0s\n' +
    '    section Bot 1\n' +
    '    Process Start : 00:00:00,0s\n' +
    '    Start﹕ Mine Stuff : 0s\n' +
    '    Walk to [10, 43] : 45s\n' +
    '    Mining rock-huge : 3s\n' +
    '    End : 3s\n' +
    '    Start﹕ Walk Around Stuff : 0s\n' +
    '    Walk to [10, 3] : 11s\n' +
    '    Walk to [10, 63] : 60s\n' +
    '    Walk to [0, 0] : 64s\n' +
    '    End : 13s\n' +
    '    section Bot 2\n' +
    '    Process Start : 00:00:00,0s\n' +
    '    Start﹕ Mine Stuff : 0s\n' +
    '    Walk to [20, 43] : 48s\n' +
    '    Mining rock-huge : 3s\n' +
    '    End : 0s\n' +
    '    Start﹕ Walk Around Stuff : 0s\n' +
    '    Walk to [20, 3] : 21s\n' +
    '    Walk to [20, 63] : 60s\n' +
    '    Walk to [0, 0] : 67s\n' +
    '    End : 0s')
*/

// Plain HTML/SVG with Tailwind, not mermaid gantt: mermaid's gantt vocabulary
// is one bar per row with a label, a start and a duration, and this document
// needs two rows per step, a distinct `Lost`, and evidence marking -- none of
// which mermaid can express. See `crates/executor/src/replay.rs` and
// `ReplayView.vue`.
import {computed, onMounted, onUnmounted} from 'vue';
import {useReplayStore} from '@/store/replayStore';
import ReplayView from '@/components/replay/ReplayView.vue';

const replayStore = useReplayStore();
const replay = computed(() => replayStore.getReplay);
const parseError = computed(() => replayStore.getParseError);

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
  <ReplayView :replay="replay" :parse-error="parseError"/>
</template>
