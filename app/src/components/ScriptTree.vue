<script setup lang="ts">
import {computed, onMounted, ref} from 'vue';
import {Loader2} from '@lucide/vue';
import type {ScriptTreeNode} from '@/api/types';
import {useScriptStore} from '@/store/scriptStore';
import TreeView from '@/components/ui/tree/TreeView.vue';
import {mergeNodes} from '@/components/ui/tree/mergeNodes';

const scriptStore = useScriptStore()

const nodes = ref([] as ScriptTreeNode[])
const expandedKeys = ref([] as string[])
const selectedKey = ref(null as string | null)
const error = ref(null as string | null)

// Directories whose listing has already been fetched. Without it, collapsing
// and re-expanding a directory would re-fetch it every time.
const loadedKeys = ref([] as string[])

const loading = computed(() => scriptStore.getLoadingScriptsInDirectory)

const emit = defineEmits<{select: [key: string]}>()

async function list(path: string): Promise<ScriptTreeNode[] | null> {
  try {
    const listed = await scriptStore.loadScriptsInDirectory(path)
    error.value = null
    return listed
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    return null
  }
}

onMounted(async () => {
  const root = await list('/')
  if (root !== null) {
    nodes.value = root
  }
})

async function onToggle(node: ScriptTreeNode) {
  if (expandedKeys.value.includes(node.key)) {
    expandedKeys.value = expandedKeys.value.filter(key => key !== node.key)
    return
  }
  if (!loadedKeys.value.includes(node.key)) {
    const children = await list(node.key)
    if (children === null) {
      return
    }
    nodes.value = mergeNodes(nodes.value, node.key, children)
    loadedKeys.value = [...loadedKeys.value, node.key]
  }
  expandedKeys.value = [...expandedKeys.value, node.key]
}

function onSelect(node: ScriptTreeNode) {
  selectedKey.value = node.key
  emit('select', node.key)
}
</script>

<template>
  <div>
    <p v-if="error" class="mb-2 text-sm text-danger" data-testid="tree-error">{{ error }}</p>
    <p v-if="loading" class="mb-2 flex items-center gap-2 text-sm text-ink-muted">
      <Loader2 class="size-4 animate-spin" aria-hidden="true"/>
      Loading ...
    </p>
    <TreeView
      :nodes="nodes"
      :expanded-keys="expandedKeys"
      :selected-key="selectedKey"
      @toggle="onToggle"
      @select="onSelect"/>
  </div>
</template>
