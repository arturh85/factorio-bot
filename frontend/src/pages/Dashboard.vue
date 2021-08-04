<script>
import {defineComponent, ref, watch, computed} from 'vue';
import {useAppStore} from '@/store/appStore';

export default defineComponent({
  components: {},
  setup() {
    const appStore = useAppStore()
    const clients = ref([]);
    const updateClients = () => {
      let newClients = [];
      for (let i=0; i<appStore.settings.client_count; i++) {
        newClients.push({
          name: 'client' + (i+1),
          status: 'not_initialized'
        })
      }
      clients.value = newClients
    }

    watch(() => appStore.getClientCount, updateClients)
    if (appStore.settings) {
      updateClients()
    }
    return {
      clients,
      clientCount: computed(() => appStore.getClientCount),
      sendTestMessage: async () => {
        // console.log('listen to the_event');
        // await listen('the_event', (event) => {
        //   console.log('event from rust', event);
        //   // event.event is the event name (useful if you want to use a single callback fn for multiple event types)
        //   // event.payload is the payload object
        // });
        // await invoke('my_custom_command');
        //
        // const config = await invoke('load_config');
        // console.log('config:', config);
        // await invoke('save_config');
        /*
        // listen to the `click` event and get a function to remove the event listener
        // there's also a `once` function that subscribes to an event and automatically unsubscribes the listener on the first event


        // emits the `click` event with the object payload
        emit('click', {
          theMessage: 'Tauri is awesome!'
        })
         */
      }
    };
  }
});
</script>

<template>
  <div class="p-grid p-fluid dashboard">
    <div class="p-col-12 p-lg-4">
      <div class="card summary">
        <span class="title">Instances</span>
        <span class="detail">Number of configured instances</span>
        <span class="count visitors" @click="sendTestMessage()">{{  clientCount }}</span>
      </div>
    </div>
    <div class="p-col-12 p-lg-4" v-for="client in clients" :key="client.name">
      <div class="card summary">
        <span class="title">{{ client.name }}</span>
        <span class="detail">{{client.status }}</span>
      </div>
    </div>
  </div>
</template>

