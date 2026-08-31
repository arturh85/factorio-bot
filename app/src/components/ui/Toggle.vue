<script setup lang="ts">
import {computed} from 'vue';
import {Check, X} from '@lucide/vue';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  onLabel: string;
  offLabel: string;
  class?: string;
}>(), {
  class: undefined
});

const model = defineModel<boolean>({required: true});

const classes = computed(() => cn(
  'inline-flex h-9 cursor-pointer items-center gap-2 rounded-card border px-4 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus',
  model.value ? 'border-brand bg-brand text-white' : 'border-divider bg-card text-ink',
  props.class
));
</script>

<template>
  <button type="button" :aria-pressed="model" :class="classes" @click="model = !model">
    <Check v-if="model" class="size-4" aria-hidden="true"/>
    <X v-else class="size-4" aria-hidden="true"/>
    <span>{{ model ? onLabel : offLabel }}</span>
  </button>
</template>
