<script setup lang="ts">
defineProps<{title: string; subtitle?: string}>();
</script>

<template>
  <div class="grid grid-cols-[10.5rem_1fr] border-b border-divider last:border-b-0">
    <div class="border-r border-divider bg-surface px-3 py-2">
      <div class="text-[11px] font-semibold uppercase tracking-wider text-ink-muted">{{ title }}</div>
      <div v-if="subtitle" class="mt-0.5 text-xs leading-snug text-ink-muted/80">{{ subtitle }}</div>
    </div>
    <div class="relative min-w-0 bg-plot">
      <slot/>
    </div>
  </div>
</template>
