<script setup lang="ts">
import {useId} from 'vue';
import {CheckboxIndicator, CheckboxRoot} from 'reka-ui';
import {Check} from '@lucide/vue';

defineProps<{label: string}>();

const model = defineModel<boolean>({required: true});
const id = useId();
</script>

<template>
  <div class="flex items-center gap-2">
    <CheckboxRoot
      :id="id"
      v-model="model"
      class="flex size-5 cursor-pointer items-center justify-center rounded-card border border-divider bg-card text-white transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus data-[state=checked]:border-brand data-[state=checked]:bg-brand">
      <CheckboxIndicator class="flex items-center justify-center">
        <Check class="size-3.5"/>
      </CheckboxIndicator>
    </CheckboxRoot>
    <label :for="id" class="cursor-pointer text-sm text-ink">{{ label }}</label>
  </div>
</template>
