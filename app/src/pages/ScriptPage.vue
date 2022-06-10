<script setup lang="ts">
import {computed} from 'vue';
import ansiHTML from 'ansi-html';
import {useScriptStore} from '@/store/scriptStore';
import {useToast} from 'primevue/usetoast';
import ScriptTree from '@/components/ScriptTree.vue'
import Editor from '@/components/Editor.vue'
import Splitter, {SplitterResizeEndEvent} from 'primevue/splitter';
import SplitterPanel from 'primevue/splitterpanel';
import Button from 'primevue/button';
import {useDebounceFn} from '@vueuse/core';

const scriptStore = useScriptStore()
const toast = useToast();

const onResize = (e: SplitterResizeEndEvent) => {
  console.log('onResize', e)
}


const code = computed(() => scriptStore.getCode)
const language = computed(() => scriptStore.getLanguage)
const updateCode = useDebounceFn((code: string) => {
  scriptStore.setCode(code)
}, 1000);
const activeScriptPath = computed(() => scriptStore.getActiveScriptPath)
const stdout = computed(() => scriptStore.getStdout)
const stderr = computed(() => scriptStore.getStderr)
const execute = async () => {
  try {
    await scriptStore.executeScript()
  } catch (err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to execute script', detail: err.message, life: 10000});
    }
  }
}
const isExecuting = computed(() => scriptStore.isExecuting)
const loadScriptFile = (path: string) => scriptStore.loadScriptFile(path)

</script>

<template>
  <div class="p-grid">
    <div class="p-col-12">
      <div class="card" style="height: 100%">
        <h5>
          Lua Script <strong>{{ activeScriptPath }}</strong>
          <Button @click="execute()"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Splitter style="min-height: 800px; width: 800px" stateKey="luaScriptSplitter" stateStorage="local">
          <SplitterPanel :size="20">
            <ScriptTree @select="loadScriptFile($event)"></ScriptTree>
          </SplitterPanel>
          <SplitterPanel v-if="activeScriptPath" :size="80">
            <Splitter style="height: 100%" layout="vertical" @resizeend="onResize">
              <SplitterPanel>
                <Editor class="editor" :value="code" :language="language" theme="vs-dark" @change="updateCode"></Editor>
              </SplitterPanel>
              <SplitterPanel>
                <div class="outputs">
                  <pre v-for="(line, idx) in stderr.split('\n')" :key="idx" class="stderr"
                       :innerHTML="ansiHTML(line)"></pre>
                  <pre v-for="(line, idx) in stdout.split('\n')" :key="idx" class="stdout"
                       :innerHTML="ansiHTML(line)"></pre>
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
  color: red;
  margin: 0;
}

.stdout {
  margin: 0;
}

.editor {
  width: 100%;
  height: 100%;
}

.outputs {
  width: 100%;
}
</style>
