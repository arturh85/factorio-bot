<script setup lang="ts">
import {computed, onUnmounted} from 'vue';
import ansiHTML from 'ansi-html';
import {SplitterGroup, SplitterPanel, SplitterResizeHandle} from 'reka-ui';
import {useDebounceFn} from '@vueuse/core';
import {useScriptStore} from '@/store/scriptStore';
import {useToast} from '@/composables/useToast';
import ScriptTree from '@/components/ScriptTree.vue';
import Editor from '@/components/Editor.vue';
import Card from '@/components/ui/Card.vue';
import Button from '@/components/ui/Button.vue';

const scriptStore = useScriptStore()
const toast = useToast()

const code = computed(() => scriptStore.getCode)
const language = computed(() => scriptStore.getLanguage)
const activeScriptPath = computed(() => scriptStore.getActiveScriptPath)
const stdout = computed(() => scriptStore.getStdout)
const stderr = computed(() => scriptStore.getStderr)
const isExecuting = computed(() => scriptStore.isExecuting)

const updateCode = useDebounceFn((code: string) => {
  scriptStore.setCode(code)
}, 1000)

const execute = async () => {
  try {
    await scriptStore.executeScript()
  } catch (err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to execute script', detail: err.message, life: 10000})
    }
  }
}

const loadScriptFile = (path: string) => scriptStore.loadScriptFile(path)

// The SSE connection outlives the component otherwise: the store is a
// singleton, so navigating away would leave an open stream appending into
// state nothing renders, and the job keeps running on the server either way.
// Coming back re-runs, or -- if that run still holds the slot -- attaches to
// it from the 409.
onUnmounted(() => {
  scriptStore.stopWatching()
})
</script>

<template>
  <Card>
    <template #title>
      <span class="grow">Lua Script <strong class="font-mono text-base">{{ activeScriptPath }}</strong></span>
      <Button :disabled="isExecuting" data-testid="run-button" @click="execute()">
        {{ isExecuting ? 'Running ...' : 'Run' }}
      </Button>
    </template>

    <SplitterGroup direction="horizontal" auto-save-id="luaScriptSplitter" class="h-[70vh] w-full">
      <SplitterPanel :default-size="20" :min-size="10" class="overflow-auto pr-2" data-testid="script-tree-pane">
        <ScriptTree @select="loadScriptFile($event)"/>
      </SplitterPanel>

      <SplitterResizeHandle class="w-1 rounded-card bg-divider transition-colors hover:bg-brand"/>

      <SplitterPanel v-if="activeScriptPath" :default-size="80" class="pl-2">
        <SplitterGroup direction="vertical" auto-save-id="luaScriptOutputSplitter" class="h-full">
          <SplitterPanel :default-size="70" class="overflow-hidden" data-testid="editor-pane">
            <Editor class="size-full" :value="code" :language="language" theme="vs-dark" @change="updateCode"/>
          </SplitterPanel>

          <SplitterResizeHandle class="h-1 rounded-card bg-divider transition-colors hover:bg-brand"/>

          <SplitterPanel :default-size="30" class="overflow-auto bg-card font-mono text-xs" data-testid="output-pane">
            <pre
              v-for="(line, idx) in stderr.split('\n')"
              :key="'stderr' + idx"
              class="m-0 whitespace-pre-wrap text-danger"
              data-testid="stderr-line"
              v-html="ansiHTML(line)"></pre>
            <pre
              v-for="(line, idx) in stdout.split('\n')"
              :key="'stdout' + idx"
              class="m-0 whitespace-pre-wrap text-ink"
              data-testid="stdout-line"
              v-html="ansiHTML(line)"></pre>
          </SplitterPanel>
        </SplitterGroup>
      </SplitterPanel>
    </SplitterGroup>
  </Card>
</template>
