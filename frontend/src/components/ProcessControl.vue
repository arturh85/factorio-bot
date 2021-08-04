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

export default defineComponent({
  components: {
    Button
  },
  setup() {
    const instanceStore = useInstanceStore()

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
      startInstances: () => instanceStore.startInstances(),
      stopInstances: () => instanceStore.stopInstances()
    }
  }
});
</script>

<style scoped>

</style>
