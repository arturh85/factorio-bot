<script setup lang="ts">
import {computed} from 'vue';
import ansiHTML from 'ansi-html';
import {useScriptStore} from '@/store/scriptStore';
import {useToast} from 'primevue/usetoast';
import ScriptTree from '@/components/ScriptTree.vue'
import Splitter from 'primevue/splitter';
import SplitterPanel from 'primevue/splitterpanel';
import Button from 'primevue/button';

const scriptStore = useScriptStore()
const toast = useToast();

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
          Lua Script {{ activeScriptPath }}
          <Button @click="execute()"
                  :label="isExecuting ? 'Running ...' : 'Run'"
                  :disabled="isExecuting">
          </Button>
        </h5>

        <Splitter style="min-height: 800px;" stateKey="luaScriptSplitter" stateStorage="local">
          <SplitterPanel :size="20">
            <ScriptTree @select="loadScriptFile($event)"></ScriptTree>
          </SplitterPanel>
          <SplitterPanel :size="80">
                <div class="outputs">
                  <pre v-for="(line, idx) in stderr.split('\n')" :key="idx" class="stderr"
                       :innerHTML="ansiHTML(line)"></pre>
                  <pre v-for="(line, idx) in stdout.split('\n')" :key="idx" class="stdout"
                       :innerHTML="ansiHTML(line)"></pre>
                </div>
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
