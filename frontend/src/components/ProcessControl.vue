<template>
  <Button :label="buttonLabel" icon="pi pi-check" :model="items" class="p-button-success p-mr-2 p-mb-2" :disabled="isStopping || isStarting" @click="isStarted ? stopInstances() : startInstances()"></Button>
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
        } else if (instanceStore.isFailed) {
          return 'Failed'
        } else if (instanceStore.started) {
          return 'Stop'
        } else {
          return 'Start'
        }
      }),
      isFailed: instanceStore.isFailed,
      isStopping: instanceStore.isStopping,
      isStarting: instanceStore.isStarting,
      isStarted: instanceStore.isStarted,
      startInstances: () => {
        instanceStore.startInstances()
      },
      stopInstances: () => {
        instanceStore.stopInstances()
      },
      items: [
        {
          label: 'Update',
          icon: 'pi pi-refresh'
        },
        {
          label: 'Delete',
          icon: 'pi pi-times'
        },
        {
          separator:true
        },
        {
          label: 'Home',
          icon: 'pi pi-home'
        }
      ]
    }
  }
});
</script>

<style scoped>

</style>
