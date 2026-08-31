<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  invalid?: boolean;
  class?: HTMLAttributes['class'];
}>(), {
  invalid: false,
  class: undefined
});

const model = defineModel<string>({required: true});

const classes = computed(() => cn(
  'h-9 w-full rounded-card border bg-card px-3 text-sm text-ink transition-colors placeholder:text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus disabled:opacity-60',
  props.invalid ? 'border-danger' : 'border-divider',
  props.class
));
</script>

<template>
  <input v-model="model" :class="classes" :aria-invalid="invalid || undefined">
</template>
