<template>
  <div class="layout-topbar">
    <button class="p-link layout-menu-button" @click="onMenuToggle">
      <span class="pi pi-bars"></span>
    </button>

    <div class="layout-topbar-actions" v-if="settings">
      <span>Factorio {{ settings.factorio_version }}</span> with <strong>{{ settings.client_count }} Clients</strong>
      &nbsp;
      <ProcessControl/>
    </div>
  </div>
</template>

<script setup lang="ts">
import ProcessControl from '@/components/ProcessControl.vue'
import {computed} from 'vue';
import {useAppStore} from '@/store/appStore';

const appStore = useAppStore();
const emit = defineEmits(['menu-toggle']);
const settings = computed(() => appStore.getSettings)

function onMenuToggle(event: any) {
  emit('menu-toggle', event);
}
</script>