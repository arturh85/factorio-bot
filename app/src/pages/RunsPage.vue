<script setup lang="ts">
/** The archive. One run is read at `/runs/:id`. */
import {onBeforeUnmount, onMounted, ref} from 'vue';
import {useRunsStore} from '@/store/runsStore';
import {formatAgo, formatTicks, formatWhen, startedUnixOf} from '@/lib/runTimeline';

const store = useRunsStore();
const nowUnix = ref(Math.floor(Date.now() / 1000));
let clock: number | null = null;
onMounted(() => {
    store.loadRuns();
    clock = window.setInterval(() => (nowUnix.value = Math.floor(Date.now() / 1000)), 30_000);
});
onBeforeUnmount(() => { if (clock !== null) window.clearInterval(clock); });
</script>

<template>
  <div class="mx-auto max-w-[900px]">
    <h2 class="mb-3 text-xl font-semibold">Runs</h2>
    <p v-if="store.error" class="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger-dark">{{ store.error }}</p>
    <p v-else-if="store.runs.length === 0" class="text-ink-muted">No runs recorded yet.</p>
    <ul class="grid gap-1.5">
      <li v-for="run in store.runs" :key="run.run_id">
        <router-link :to="`/runs/${run.run_id}`"
                     class="block rounded-card border border-divider bg-card px-3 py-2 hover:border-brand focus-visible:ring-2 focus-visible:ring-focus">
          <span class="block font-semibold">{{ formatWhen(startedUnixOf(run)) }}
            <em class="ml-1.5 font-normal not-italic text-ink-muted">{{ formatAgo(startedUnixOf(run), nowUnix) }}</em></span>
          <span class="block font-mono text-xs text-ink-muted">{{ run.run_id }}</span>
          <span class="block text-xs text-ink-muted">
            <!-- `finished: false` means crashed OR still going; the server cannot tell them apart, so neither does this. -->
            {{ run.finished ? run.outcome : 'unfinished' }} · {{ formatTicks(run.elapsed_ticks) }}
          </span>
        </router-link>
      </li>
    </ul>
  </div>
</template>
