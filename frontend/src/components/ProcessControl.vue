<template>
  <SplitButton label="Start" icon="pi pi-check" :model="items" class="p-button-success p-mr-2 p-mb-2" @click="startInstances()"></SplitButton>
</template>

<script>
import {defineComponent} from 'vue';
import {useInstanceStore} from '@/store/instanceStore';
import SplitButton from 'primevue/splitbutton';

export default defineComponent({
  components: {
    SplitButton
  },
  setup() {
    const instanceStore = useInstanceStore()

    return {
      isFailed: instanceStore.isFailed,
      isStopping: instanceStore.isStopping,
      isStarting: instanceStore.isStarting,
      isStarted: instanceStore.isStarted,
      startInstances: () => {
        instanceStore.startInstances()
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
