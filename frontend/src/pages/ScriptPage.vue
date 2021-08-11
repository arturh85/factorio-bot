<script setup lang="ts">
import Button from 'primevue/button';
import {computed} from 'vue';
import {useScriptStore} from '../store/scriptStore';
import {useToast} from 'primevue/usetoast';
import Editor from '@/components/Editor.vue'

const scriptStore = useScriptStore()
const toast = useToast();

const code = computed(() => scriptStore.getCode)
const updateCode = (code: string) => {
  scriptStore.setCode(code);
}
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
      <div class="card" style="height: 100%">
        <h5>
          Lua Script
          <Button @click="execute()"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Editor class="editor" :value="code" theme="vs-dark" @change="updateCode"></Editor>

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

.editor {
  width: 600px;
  height: 800px;
}
</style>
