<script setup lang="ts">
import Textarea from 'primevue/textarea';
import Button from 'primevue/button';
import {computed, ref} from 'vue';
import {useRconStore} from '../store/rconStore';
import {useToast} from 'primevue/usetoast';

const rconStore = useRconStore()
const toast = useToast();

const command = ref('')
const execute = async(command: string) => {
  try {
    await rconStore.execute(command)
  } catch(err) {
    toast.add({severity:'error', summary: 'Failed to execute rcon', detail:err.message, life: 10000});
  }
}
const isExecuting = computed(() => rconStore.isExecuting)

// const cheatCommands = [{
//   command: ''
// }]

</script>

<template>
  <div class="p-grid">
    <div class="p-col-12">
      <div class="card">
        <h5>
          RCON
          <Button @click="execute(command)"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Textarea class="input" :autoResize="true"  v-model="command"></Textarea>

        <Button @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)')" label="Cheat Furnaces" />
        <Button @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'transport-belt\', 100)')" label="Cheat belts" />
        <Button @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'burner-mining-drill\', 20)')" label="Cheat Drills" />
        <Button @click="execute('/server-save')" label="Save" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.input {
  width: 100%;
}
</style>
