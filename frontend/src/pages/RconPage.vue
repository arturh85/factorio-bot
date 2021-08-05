<template>
	<div class="p-grid">
		<div class="p-col-12">
			<div class="card">
				<h5>RCON</h5>
        <Button @click="execute()"
                :label="isExecuting ? 'Running ...' : 'Run'"
                :disabled="isExecuting">
        </Button>
				<Textarea v-model="command"></Textarea>
			</div>
		</div>
	</div>
</template>

<script>
import Textarea from 'primevue/textarea';
import Button from 'primevue/button';
import {computed, defineComponent, ref} from 'vue';
import {useRconStore} from '../store/rconStore';

export default defineComponent({
  components: {
    Textarea,
    Button
  },
  setup() {
    const rconStore = useRconStore()

    const command = ref('')
    return {
      execute: () => rconStore.execute(command.value),
      isExecuting: computed(() => rconStore.isExecuting),
      command
    }
  }
})
</script>

<style scoped>

</style>
