<script setup lang="ts">
import {ref, watch, computed} from 'vue';
import {useAppStore} from '@/store/appStore';
import {useToast} from 'primevue/usetoast';

interface MyFactorioClient {
  name: string,
  status: string
}

const toast = useToast();
const appStore = useAppStore()
const clients = ref([] as MyFactorioClient[]);
const updateClients = () => {
  let newClients = [];
  if (appStore.settings) {
    for (let i = 0; i < appStore.settings.client_count; i++) {
      newClients.push({
        name: 'client' + (i + 1),
        status: 'not_initialized'
      })
    }
  }
  clients.value = newClients
}
watch(() => appStore.getClientCount, updateClients)
if (appStore.settings) {
  updateClients()
}
const clientCount = computed(() => appStore.getClientCount)
const sendTestMessage = async () => {
  toast.add({severity:'info', summary: 'Info Message', detail:'Message Content', life: 3000});
}
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

