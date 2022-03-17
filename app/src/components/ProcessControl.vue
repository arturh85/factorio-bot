<script setup lang="ts">
import {computed} from 'vue';
import {useInstanceStore} from '@/store/instanceStore';
import Button from 'primevue/button';
import { useToast } from 'primevue/usetoast';
import {useAppStore} from '@/store/appStore';
import ToggleButton from 'primevue/togglebutton';


const instanceStore = useInstanceStore()
const appStore = useAppStore()
const toast = useToast();

const buttonLabel = computed(() => {
  if (instanceStore.starting) {
    return 'Starting ...'
  } else if (instanceStore.stopping) {
    return 'Stopping ...'
  } else if (instanceStore.failed) {
    return 'Failed'
  } else if (instanceStore.started) {
    return 'Stop'
  } else {
    return 'Start'
  }
})
// const isFailed = computed(() => instanceStore.isFailed)
const isStopping = computed(() => instanceStore.isStopping)
const isStarting = computed(() => instanceStore.isStarting)
const isStarted = computed(() => instanceStore.isStarted)

const recreateLevel = computed({
  get() {
    return appStore.getRecreateLevel
  },
  set(val ) {
    appStore.updateRecreateLevel(val as boolean)
  }
})
const startInstances = async() => {
  try {
    toast.add({severity:'success', summary: 'starting!', life: 1000});
    await instanceStore.startInstances()
    toast.add({severity:'success', summary: 'Started!', life: 1000});
  } catch(err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to start instances', detail: err.message, life: 10000});
    }
  }
}
const stopInstances = async() => {
  try {
    await instanceStore.stopInstances()
    toast.add({severity:'success', summary: 'Stopped!', life: 1000});
  } catch(err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to stop instances', detail: err.message, life: 10000});
    }
  }
}
</script>

<template>
  <ToggleButton v-model="recreateLevel" onLabel="Recreate Level" offLabel="Use existing Level" onIcon="pi pi-check" offIcon="pi pi-times" />
  <Button :icon="isStarted ? 'pi pi-check' : ''"
          :label="buttonLabel"
          :class="isStarted ? 'p-button-success' : 'p-button-error' + ' p-mr-2 p-mb-2'"
          :disabled="isStopping || isStarting" @click="isStarted ? stopInstances() : startInstances()">
  </Button>
</template>

<style scoped>

</style>
