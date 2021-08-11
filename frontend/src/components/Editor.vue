<template>
  <div ref="container" style=""></div>
</template>

<script setup lang="ts">
import {onMounted, onUnmounted, ref} from 'vue'
import {useResizeObserver} from '@vueuse/core'

// Import monaco
// https://github.com/vitejs/vite/discussions/1791
import * as monaco from 'monaco-editor'
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker'
import cssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker'
import htmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker'
import tsWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker'

// @ts-ignore
self.MonacoEnvironment = {
  getWorker(_: string, label: string) {
    if (label === 'json') {
      return new jsonWorker()
    }
    if (label === 'css' || label === 'scss' || label === 'less') {
      return new cssWorker()
    }
    if (label === 'html' || label === 'handlebars' || label === 'razor') {
      return new htmlWorker()
    }
    if (label === 'typescript' || label === 'javascript') {
      return new tsWorker()
    }
    return new editorWorker()
  }
}

const container = ref<HTMLDivElement | null>(null)

let editor: monaco.editor.IStandaloneCodeEditor

// const isDark = false

const props = defineProps({
  original: String,
  value: {
    type: String,
    required: true
  },
  theme: {
    type: String,
    default: 'vs'
  },
  language: String,
  options: Object,
  amdRequire: {
    type: Function
  },
  diffEditor: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits<(e: 'change', payload: string) => void>()

onMounted(() => {
  editor = monaco.editor.create(container.value!, {
    theme: props.theme,
    language: props.language || 'lua',
    value: props.value
  })
  // emit('change', editorValue.value)


  // @event `change`
  editor.onDidChangeModelContent(() => {
    const value = editor.getValue()
    if (props.value != value) {
      emit('change', value)
    }
  })
})

// watch(activeTab, (currentTab, prevTab) => {
//   monaco.editor.setModelLanguage(editor.getModel()!, currentTab)
//
//   editorState.value[prevTab] = editor.saveViewState()
//
//   if (editorValue.value[currentTab]) {
//     editor.setValue(editorValue.value[currentTab])
//   } else {
//     editor.setValue('')
//   }
//
//   if (editorState.value[currentTab]) {
//     editor.restoreViewState(editorState.value[currentTab]!)
//     editor.focus()
//   }
// })
//
// watch(isDark, (value) => {
//   editor.updateOptions({
//     theme: value ? 'vs-dark' : 'vs'
//   })
// })

const editorObserver = useResizeObserver(container, () => {
  editor.layout()
})

onUnmounted(() => {
  editor?.dispose()
  editorObserver.stop()
})
</script>