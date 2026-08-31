<script setup lang="ts">
defineProps<{title?: string}>();
</script>

<template>
  <section class="mb-4 rounded-card bg-card p-4 text-ink">
    <h2 v-if="$slots.title || title" class="mb-4 flex flex-wrap items-center gap-3 text-xl leading-tight">
      <slot name="title">{{ title }}</slot>
    </h2>
    <slot/>
  </section>
</template>
