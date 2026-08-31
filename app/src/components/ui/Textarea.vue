<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  class?: HTMLAttributes['class'];
}>(), {
  class: undefined
});

const model = defineModel<string>({required: true});

const classes = computed(() => cn(
  'min-h-24 w-full rounded-card border border-divider bg-card px-3 py-2 font-mono text-sm text-ink transition-colors placeholder:text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus',
  props.class
));
</script>

<template>
  <textarea v-model="model" :class="classes"></textarea>
</template>
