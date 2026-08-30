<template>
  <div ref="container" style=""></div>
</template>

<script setup lang="ts">
import {onMounted, onUnmounted, ref, watch, toRefs} from 'vue'
import {useResizeObserver} from '@vueuse/core'

// Import monaco
// https://github.com/vitejs/vite/discussions/1791
//
// The worker specifiers drop the `esm/vs/` prefix they carried before monaco
// 0.56. That release added an `exports` map to monaco's package.json
// ("./*" -> "./esm/vs/*.js"), which stops deep paths resolving by filesystem
// walk: `monaco-editor/esm/vs/editor/editor.worker` now maps to
// `esm/vs/esm/vs/editor/editor.worker.js` and fails to resolve.
import * as monaco from 'monaco-editor'
import editorWorker from 'monaco-editor/editor/editor.worker?worker'
import jsonWorker from 'monaco-editor/language/json/json.worker?worker'
import cssWorker from 'monaco-editor/language/css/css.worker?worker'
import htmlWorker from 'monaco-editor/language/html/html.worker?worker'
import tsWorker from 'monaco-editor/language/typescript/ts.worker?worker'

console.log('load monaco');
(window as any)['MonacoEnvironment'] = {
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
};

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

const {value} = toRefs(props)


const emit = defineEmits<(e: 'change', payload: string) => void>()

onMounted(() => {
  editor = monaco.editor.create(container.value!, {
    theme: props.theme,
    language: props.language || 'lua',
    value: props.value
  })
  // emit('change', editorValue.value)

  let ignoreNext = false;

  watch(value, (value) => {
    if (ignoreNext) {
      ignoreNext = false;
      return;
    }
    editor.setValue(value)
  })


  // @event `change`
  editor.onDidChangeModelContent(() => {
    const value = editor.getValue()
    if (props.value != value) {
      emit('change', value)
      ignoreNext = true;
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