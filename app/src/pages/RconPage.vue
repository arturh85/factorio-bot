<script setup lang="ts">
import {computed, ref} from 'vue';
import {useRconStore} from '@/store/rconStore';
import {useToast} from '@/composables/useToast';
import Card from '@/components/ui/Card.vue';
import Button from '@/components/ui/Button.vue';
import Textarea from '@/components/ui/Textarea.vue';

const rconStore = useRconStore()
const toast = useToast()

const command = ref('')
const isExecuting = computed(() => rconStore.isExecuting)

const execute = async (command: string) => {
  try {
    await rconStore.execute(command)
  } catch (err) {
    // Every rejection gets a toast. Narrowing to `if (err instanceof Error)`
    // would let a non-`Error` rejection -- a bare string, anything a
    // transport can reject with -- pass silently, leaving the button back to
    // "Run" with no indication the command had failed.
    toast.add({
      severity: 'error',
      summary: 'Failed to execute rcon',
      detail: err instanceof Error ? err.message : String(err),
      life: 10000
    })
  }
}

const cheats = [
  {
    testid: 'cheat-furnaces',
    label: 'Cheat Furnaces',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)'
  },
  {
    testid: 'cheat-belts',
    label: 'Cheat belts',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'transport-belt\', 100)'
  },
  {
    testid: 'cheat-drills',
    label: 'Cheat Drills',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'burner-mining-drill\', 20)'
  },
  {
    testid: 'server-save',
    label: 'Save',
    command: '/server-save'
  }
]
</script>

<template>
  <div class="mx-auto max-w-3xl">
    <Card>
      <template #title>
        <span class="grow">RCON</span>
        <Button :disabled="isExecuting" data-testid="run-button" @click="execute(command)">
          {{ isExecuting ? 'Running ...' : 'Run' }}
        </Button>
      </template>

      <Textarea v-model="command" class="min-h-32" placeholder="/silent-command game.print('hello')"/>

      <div class="mt-4 flex flex-wrap gap-2">
        <Button
          v-for="cheat in cheats"
          :key="cheat.testid"
          variant="ghost"
          :data-testid="cheat.testid"
          @click="execute(cheat.command)">
          {{ cheat.label }}
        </Button>
      </div>
    </Card>
  </div>
</template>
