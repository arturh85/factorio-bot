<script setup>
import Textarea from 'primevue/textarea';
import Button from 'primevue/button';
import {computed, ref} from 'vue';
import {useRconStore} from '../store/rconStore';
import {useToast} from 'primevue/usetoast';

const rconStore = useRconStore()
const toast = useToast();

const command = ref('')
const execute = async() => {
  try {
    await rconStore.execute(command.value)
  } catch(err) {
    toast.add({severity:'error', summary: 'Failed to execute rcon', detail:err.message, life: 10000});
  }
}
const isExecuting = computed(() => rconStore.isExecuting)
</script>

<template>
  <div class="p-grid">
    <div class="p-col-12">
      <div class="card">
        <h5>
          RCON
          <Button @click="execute()"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Textarea class="input" :autoResize="true"  v-model="command"></Textarea>
      </div>
    </div>
  </div>
</template>

<style scoped>
.input {
  width: 100%;
}
</style>
