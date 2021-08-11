<script setup lang="ts">
import {computed} from 'vue';
import {useScriptStore} from '../store/scriptStore';
import {useToast} from 'primevue/usetoast';
import Editor from '@/components/Editor.vue'
import ScriptTree from '@/components/ScriptTree.vue'
import Splitter from 'primevue/splitter';
import SplitterPanel from 'primevue/splitterpanel';
import Button from 'primevue/button';

const onResize = () => {

}


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

        <Splitter style="min-height: 400px;" stateKey="luaScriptSplitter" stateStorage="local">
          <SplitterPanel :size="20">
            <ScriptTree></ScriptTree>
          </SplitterPanel>
          <SplitterPanel :size="80">
            <Splitter style="height: 100%" layout="vertical"  @resizeend="onResize()">
              <SplitterPanel>
                <Editor class="editor" :value="code" theme="vs-dark" @change="updateCode"></Editor>
              </SplitterPanel>
              <SplitterPanel>
                <div class="outputs">
                  <pre class="stderr">{{ stderr }}</pre>
                  <pre class="stdout">{{ stdout }}</pre>
                </div>
              </SplitterPanel>
            </Splitter>
          </SplitterPanel>
        </Splitter>
      </div>
    </div>
  </div>
</template>

<style scoped>
.stderr {
  color: red
}

.editor {
  width: 100%;
  height: 100%;
}
.outputs {
  width: 100%;
}
</style>
