<script setup lang="ts">
import type {MenuEntry} from '@/models/dashboard';

defineProps<{items: MenuEntry[]}>();

const emit = defineEmits<{navigate: []}>();
</script>

<template>
  <nav class="pb-32" aria-label="Main">
    <ul class="m-0 list-none p-0">
      <li v-for="item in items" :key="item.to">
        <router-link
          :to="item.to"
          class="flex items-center gap-2 border-t border-sidebar-border p-4 text-sidebar-ink transition-colors hover:text-brand-light [&.router-link-active]:bg-sidebar-active [&.router-link-active]:text-sidebar-active-ink"
          @click="emit('navigate')">
          <component :is="item.icon" class="size-4 shrink-0" aria-hidden="true"/>
          <span>{{ item.label }}</span>
        </router-link>
      </li>
    </ul>
  </nav>
</template>
