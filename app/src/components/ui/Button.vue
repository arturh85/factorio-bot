<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';
import {buttonVariants, type ButtonVariants} from './button-variants';

// `class` is declared as a prop on purpose: that takes it out of $attrs, so
// Vue does not also auto-merge it onto the root element and defeat cn().
const props = withDefaults(defineProps<{
  variant?: ButtonVariants['variant'];
  size?: ButtonVariants['size'];
  type?: 'button' | 'submit';
  disabled?: boolean;
  class?: HTMLAttributes['class'];
}>(), {
  variant: 'primary',
  size: 'default',
  type: 'button',
  disabled: false,
  class: undefined
});

const classes = computed(() => cn(buttonVariants({variant: props.variant, size: props.size}), props.class));
</script>

<template>
  <button :type="type" :disabled="disabled" :class="classes">
    <slot/>
  </button>
</template>
