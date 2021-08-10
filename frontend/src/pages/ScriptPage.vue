<script setup>
import Textarea from 'primevue/textarea';
import Button from 'primevue/button';
import {computed} from 'vue';
import {useScriptStore} from '../store/scriptStore';
import {useToast} from 'primevue/usetoast';

const scriptStore = useScriptStore()
const toast = useToast();

const code = computed({
  get() {
    return scriptStore.getCode
  },
  set(val) {
    scriptStore.setCode(val)
  }
})
const stdout = computed(() => scriptStore.getStdout)
const stderr = computed(() => scriptStore.getStderr)
const execute = async () => {
  try {
    await scriptStore.execute()
  } catch (err) {
    toast.add({severity: 'error', summary: 'Failed to execute script', detail: err.message, life: 10000});
  }
}
const isExecuting = computed(() => scriptStore.isExecuting)
</script>

<template>
  <div class="p-grid">
    <div class="p-col-12">
      <div class="card">
        <h5>
          Lua Script
          <Button @click="execute()"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Textarea class="input" :autoResize="true" v-model="code"></Textarea>
        <pre class="stderr">{{ stderr }}</pre>
        <pre class="stdout">{{ stdout }}</pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.stderr {
  color: red
}

.input {
  width: 100%;
}
</style>
