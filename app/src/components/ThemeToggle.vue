<script setup lang="ts">
import {computed} from 'vue';
import {Monitor, Moon, Sun} from '@lucide/vue';
import {useTheme} from '@/composables/useTheme';

const {theme, cycle} = useTheme();
const label = computed(() =>
    theme.value === 'system' ? 'Theme: follow system' : theme.value === 'dark' ? 'Theme: dark' : 'Theme: light'
);
</script>

<template>
  <button
    type="button"
    class="cursor-pointer text-white transition-colors hover:text-focus focus-visible:ring-2 focus-visible:ring-focus rounded"
    :aria-label="label"
    :title="label"
    data-testid="theme-toggle"
    @click="cycle()">
    <Monitor v-if="theme === 'system'" class="size-5"/>
    <Moon v-else-if="theme === 'dark'" class="size-5"/>
    <Sun v-else class="size-5"/>
  </button>
</template>
