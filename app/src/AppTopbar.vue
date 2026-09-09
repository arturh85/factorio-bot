<script setup lang="ts">
import {computed} from 'vue';
import {Menu} from '@lucide/vue';
import {useAppStore} from '@/store/appStore';
import ProcessControl from '@/components/ProcessControl.vue';
import ThemeToggle from '@/components/ThemeToggle.vue';

defineProps<{sidebarOpen: boolean}>();

const emit = defineEmits<{'menu-toggle': []}>();

const appStore = useAppStore();
const settings = computed(() => appStore.getSettings);
</script>

<template>
  <header
    class="fixed inset-x-0 top-0 z-30 flex h-topbar items-center gap-4 bg-linear-to-r from-brand to-brand-light px-8 text-white transition-[left] duration-200"
    :class="sidebarOpen ? 'lg:left-sidebar' : ''">
    <button
      type="button"
      class="cursor-pointer text-white transition-colors hover:text-focus"
      aria-label="Toggle menu"
      data-testid="menu-toggle"
      @click="emit('menu-toggle')">
      <Menu class="size-6"/>
    </button>

    <div class="ml-auto flex items-center gap-3">
      <span v-if="settings" class="hidden sm:inline">Factorio with <strong>{{ settings.factorio.client_count }} Clients</strong></span>
      <ProcessControl v-if="settings"/>
      <ThemeToggle/>
    </div>
  </header>
</template>
