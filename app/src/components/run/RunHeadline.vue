<script setup lang="ts">
/**
 * The first line, and the chips that say whether this run may be compared
 * with another. `provenance` is null until `/runs/{id}/provenance` exists
 * (Phase 2); every chip it would fill renders as "not captured" IN PLACE, so
 * absence is visible rather than silent.
 */
import {RunSummary} from '@/api/types';

defineProps<{summary: RunSummary; provenance: null; lagTicks: number | null; headline: string; roster: number[]}>();
const PROVENANCE_CHIPS = ['seed', 'mode', 'speed', 'commit', 'profile', 'mods'];
</script>

<template>
  <header class="flex flex-wrap items-baseline gap-x-6 gap-y-3 border-b border-divider px-5 pb-3 pt-4">
    <h2 class="text-xl font-semibold">{{ summary.run_id }}</h2>
    <div class="flex flex-wrap">
      <span data-chip="roster" data-state="present" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        roster <b class="font-medium text-ink">{{ roster.join(' ') }}</b>
      </span>
      <span v-for="c in PROVENANCE_CHIPS" :key="c" :data-chip="c" data-state="absent"
            class="mb-1.5 mr-1.5 rounded border border-dashed border-divider px-2 py-1 font-mono text-xs text-ink-muted">
        {{ c }} <i>not captured</i>
      </span>
      <span data-chip="samples" :data-state="lagTicks === null ? 'absent' : 'present'" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        samples <b class="font-medium text-ink">{{ lagTicks === null ? 'not sampled' : `lag ${lagTicks.toLocaleString()} ticks` }}</b>
      </span>
    </div>
    <p class="basis-full text-sm text-ink-muted"><b class="font-medium text-ink">{{ headline }}</b></p>
  </header>
</template>
