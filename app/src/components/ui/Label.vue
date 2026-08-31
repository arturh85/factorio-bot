<script setup lang="ts">
defineProps<{for?: string}>();
</script>

<template>
  <label :for="$props.for" class="mb-1 block text-sm font-medium text-ink">
    <slot/>
  </label>
</template>
