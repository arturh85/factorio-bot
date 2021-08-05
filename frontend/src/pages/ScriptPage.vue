<template>
	<div class="p-grid">
		<div class="p-col-12">
			<div class="card">
				<h5>Script</h5>
        <Button @click="execute()"
                :label="isExecuting ? 'Running ...' : 'Run'"
                :disabled="isExecuting">
        </Button>
				<Textarea v-model="code"></Textarea>
			</div>
		</div>
	</div>
</template>

<script>
import Textarea from 'primevue/textarea';
import Button from 'primevue/button';
import {computed, defineComponent} from 'vue';
import {useScriptStore} from '../store/scriptStore';

export default defineComponent({
  components: {
    Textarea,
    Button

  },
  setup() {
    const scriptStore = useScriptStore()

    const code = computed({
      get() {
        return scriptStore.getCode
      },
      set(val) {
        scriptStore.setCode(val)
      }
    })
    return {
      execute: () => scriptStore.execute(),
      isExecuting: computed(() => scriptStore.isExecuting),
      code
    }
  }
})
</script>

<style scoped>

</style>
