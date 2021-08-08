<template>
  <Button :icon="isStarted ? 'pi pi-check' : ''"
          :label="buttonLabel"
          :class="isStarted ? 'p-button-success' : 'p-button-error' + ' p-mr-2 p-mb-2'"
          :disabled="isStopping || isStarting" @click="isStarted ? stopInstances() : startInstances()">
  </Button>
</template>

<script>
import {defineComponent, computed} from 'vue';
import {useInstanceStore} from '@/store/instanceStore';
import Button from 'primevue/button';
import { useToast } from 'primevue/usetoast';

export default defineComponent({
  components: {
    Button
  },
  setup() {
    const instanceStore = useInstanceStore()
    const toast = useToast();

    return {
      buttonLabel: computed(() => {
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
      }),
      isFailed: computed(() => instanceStore.isFailed),
      isStopping: computed(() => instanceStore.isStopping),
      isStarting: computed(() => instanceStore.isStarting),
      isStarted: computed(() => instanceStore.isStarted),
      startInstances: async() => {
        try {
          toast.add({severity:'success', summary: 'starting!', life: 1000});
          await instanceStore.startInstances()
          toast.add({severity:'success', summary: 'Started!', life: 1000});
        } catch(err) {
          toast.add({severity:'error', summary: 'Failed to start instances', detail:err.message, life: 10000});
        }
      },
      stopInstances: async() => {
        try {
          await instanceStore.stopInstances()
          toast.add({severity:'success', summary: 'Stopped!', life: 1000});
        } catch(err) {
          toast.add({severity:'error', summary: 'Failed to stop instances', detail:err.message, life: 10000});
        }
      }
    }
  }
});
</script>

<style scoped>

</style>
