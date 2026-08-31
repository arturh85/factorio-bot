<script setup lang="ts">
import Textarea from 'primevue/textarea';
import Button from '@/components/ui/Button.vue';
import {computed, ref} from 'vue';
import {useRconStore} from '../store/rconStore';
import {useToast} from '@/composables/useToast';

const rconStore = useRconStore()
const toast = useToast();

const command = ref('')
const execute = async(command: string) => {
  try {
    await rconStore.execute(command)
  } catch(err) {
    // Every rejection gets a toast. The `if (err instanceof Error)` this
    // replaces let a non-`Error` rejection -- a bare string, anything a
    // transport can reject with -- pass silently, leaving the button back to
    // "Run" with no indication the command had failed.
    toast.add({
      severity: 'error',
      summary: 'Failed to execute rcon',
      detail: err instanceof Error ? err.message : String(err),
      life: 10000
    });
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
          <Button :disabled="isExecuting" @click="execute(command)">{{ isExecuting ? 'Running ...' : 'Run' }}</Button>
        </h5>

        <Textarea class="input" :autoResize="true"  v-model="command"></Textarea>

        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)')">Cheat Furnaces</Button>
        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'transport-belt\', 100)')">Cheat belts</Button>
        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'burner-mining-drill\', 20)')">Cheat Drills</Button>
        <Button variant="ghost" @click="execute('/server-save')">Save</Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.input {
  width: 100%;
}
</style>
