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
// `editor/editor.api`, not the `monaco-editor` package entry. The entry pulls
// in EVERY language definition and every language service, which is what put
// 90 unused grammar chunks and four language-service workers into the bundle:
// 9.15 MB of a 14.1 MB build, for TypeScript/CSS/HTML/JSON IntelliSense in an
// editor that only ever opens Lua.
//
// Note the specifiers, and see the comment above about the worker paths: 0.56's
// export map is `"./*": "./esm/vs/*.js"`, so the `esm/vs/` prefix is supplied
// BY the map and writing it yourself resolves to `esm/vs/esm/vs/...`. Lua also
// moved -- it is `languages/definitions/lua/register`, not the
// `basic-languages/.../lua.contribution` path that earlier versions used.
//
// Lua is the only language this editor opens: `scripts/` contains only `.lua`
// and the execute endpoint's DEFAULT_LANGUAGE is "lua" with nothing else
// implemented. Adding one back is a single `register` import; adding a
// language *service* back means a worker, and `ts.worker` alone is 6.91 MB.
import * as monaco from 'monaco-editor/editor/editor.api'
import 'monaco-editor/languages/definitions/lua/register'
import editorWorker from 'monaco-editor/editor/editor.worker?worker'

console.log('load monaco');
// Only the generic editor worker. The json/css/html/typescript workers were
// registered here and cost 9.15 MB of the 14.1 MB JavaScript bundle -- for
// language *services* (schema validation, type checking, CSS linting) in an
// editor that edits Lua.
//
// Syntax highlighting is unaffected: monaco's highlighting comes from its
// Monarch grammars, which are separate ~2-5 KB chunks and still shipped. What
// goes is IntelliSense for four languages this editor never opens.
//
// If a language service is ever wanted back, re-add just that one worker --
// `ts.worker` alone is 6.91 MB, so add it deliberately rather than by
// restoring the whole block.
(window as any)['MonacoEnvironment'] = {
  getWorker() {
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