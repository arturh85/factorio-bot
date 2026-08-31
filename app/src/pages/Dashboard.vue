<script setup lang="ts">
import {computed} from 'vue';
import {useAppStore} from '@/store/appStore';
import {useToast} from '@/composables/useToast';

const toast = useToast()
const appStore = useAppStore()

const clientCount = computed(() => appStore.getClientCount ?? 0)

// One tile per *configured* client slot. There is no per-client status to
// show -- `GET /api/v1/instance` answers for the group, not per client -- so
// the tile carries only the name. Do not add a status field here backed by a
// constant: that was tried, it read 'not_initialized' on every tile
// regardless of what was actually running, and a user with Factorio up
// reasonably read it as a reported failure. See Dashboard.spec.ts's
// "does not fabricate a per-client status" test.
const clients = computed(() => Array.from({length: clientCount.value}, (_unused, index) => 'client' + (index + 1)))

const sendTestMessage = () => {
  toast.add({severity: 'info', summary: 'Info Message', detail: 'Message Content', life: 3000})
}
</script>

<template>
  <div class="grid grid-cols-1 gap-4 lg:grid-cols-3">
    <section class="relative rounded-card bg-card p-4 text-ink">
      <span class="text-xl">Instances</span>
      <span class="mt-2 block text-ink-muted">Number of configured instances</span>
      <button
        type="button"
        class="absolute right-2 top-2 cursor-pointer rounded-card bg-success px-3 py-1 text-2xl text-white"
        data-testid="instance-count"
        @click="sendTestMessage()">{{ clientCount }}</button>
    </section>

    <section
      v-for="client in clients"
      :key="client"
      class="rounded-card bg-card p-4 text-ink"
      data-testid="client-tile">
      <span class="text-xl">{{ client }}</span>
    </section>
  </div>
</template>
