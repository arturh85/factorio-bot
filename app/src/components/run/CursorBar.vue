<script setup lang="ts">
import {formatGameTime, TickScale} from '@/lib/tickScale';

defineProps<{scale: TickScale; clock: TickScale; cursor: number; playing: boolean; rate: number}>();
const emit = defineEmits<{seek: [tick: number]; toggle: []; rate: [rate: number]}>();
</script>

<template>
  <div class="flex items-center gap-4 border-t border-divider bg-surface px-4 py-2">
    <button type="button" class="rounded border border-divider bg-card px-3 py-1 text-sm hover:bg-surface focus-visible:ring-2 focus-visible:ring-focus"
            @click="emit('toggle')">
      {{ playing ? 'Pause' : 'Play' }}
    </button>
    <span class="min-w-[13rem] font-mono text-sm tabular-nums text-ink-muted">
      cursor <b class="font-medium text-verdict-roster">tick {{ cursor.toLocaleString() }}</b>
      · {{ formatGameTime(clock, cursor) }}
    </span>
    <input type="range" class="flex-1 accent-verdict-roster focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus" :min="scale.from" :max="scale.to" :value="cursor"
           aria-label="tick cursor"
           @input="emit('seek', Number(($event.target as HTMLInputElement).value))"/>
    <label class="text-sm text-ink-muted">
      step
      <select class="ml-1 rounded border border-divider bg-card px-1 py-0.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus" :value="rate"
              @change="emit('rate', Number(($event.target as HTMLSelectElement).value))">
        <option :value="60">1s</option>
        <option :value="300">5s</option>
        <option :value="1800">30s</option>
      </select>
    </label>
  </div>
</template>
