<script setup lang="ts">
import {computed} from 'vue';
import {SliderRange, SliderRoot, SliderThumb, SliderTrack} from 'reka-ui';

withDefaults(defineProps<{
  min?: number;
  max?: number;
  step?: number;
  label?: string;
}>(), {
  min: 0,
  max: 100,
  step: 1,
  label: undefined
});

const model = defineModel<number>({required: true});

// reka-ui models every slider as a range, so its value is always an array.
// The app has one thumb everywhere, so the array stops here.
const values = computed({
  get: () => [model.value],
  set: (next: number[]) => {
    model.value = next[0];
  }
});
</script>

<template>
  <SliderRoot
    v-model="values"
    :min="min"
    :max="max"
    :step="step"
    :aria-label="label"
    class="relative flex h-5 w-full touch-none select-none items-center">
    <SliderTrack class="relative h-1 grow rounded-card bg-divider">
      <SliderRange class="absolute h-full rounded-card bg-brand"/>
    </SliderTrack>
    <SliderThumb class="block size-4 cursor-pointer rounded-full border border-brand bg-card shadow focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"/>
  </SliderRoot>
</template>
