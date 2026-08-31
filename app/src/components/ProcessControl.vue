<script setup lang="ts">
import {computed} from 'vue';
import {useInstanceStore} from '@/store/instanceStore';
import Button from '@/components/ui/Button.vue';
import {Check} from '@lucide/vue';
import {useToast} from '@/composables/useToast';
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
    await instanceStore.startInstances()
    // Not "Started!": the server answers 202 the moment it accepts the start
    // and then runs it detached, because a first-run archive extraction takes
    // minutes. The button's "Starting ..." label and the poll of
    // GET /api/v1/instance are what report the outcome.
    toast.add({severity:'success', summary: 'Starting ...', detail: 'the first start extracts the archive, which can take several minutes', life: 4000});
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
  <Button :variant="isStarted ? 'success' : 'danger'"
          :disabled="isStopping || isStarting"
          @click="isStarted ? stopInstances() : startInstances()">
    <Check v-if="isStarted" class="size-4" aria-hidden="true"/>
    {{ buttonLabel }}
  </Button>
</template>

<style scoped>

</style>
