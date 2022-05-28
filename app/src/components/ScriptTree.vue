<script setup lang="ts">
import {ref, onMounted, computed} from 'vue';
import Tree from 'primevue/tree';
import {useScriptStore} from '@/store/scriptStore';
import {PrimeVueTreeNode} from '@/models/types';

const nodes = ref(null as PrimeVueTreeNode[] | null)

const scriptStore = useScriptStore()
const loadingScriptsInDirectory = computed(() => scriptStore.getLoadingScriptsInDirectory)

onMounted(async() => {
  const realNodes = await scriptStore.loadScriptsInDirectory('/')
  nodes.value = realNodes
})

const emit = defineEmits(['select']);

function mergeNodes(nodes: PrimeVueTreeNode[], new_key: string, new_nodes: PrimeVueTreeNode[]):PrimeVueTreeNode[] {
  const parts = new_key.substr(1).split('/')
  let pointer: PrimeVueTreeNode | null = null;
  for (let part of parts) {
    if (pointer) {
      pointer = pointer.children.find(node => node.label === part) || null
    } else {
      pointer = nodes.find(node => node.label === part) || null
    }
  }
  if (pointer) {
    pointer.children = new_nodes
  }
  return nodes
}


const onNodeExpand = async(node: PrimeVueTreeNode) => {
  const subNodes = await scriptStore.loadScriptsInDirectory(node.key)
  nodes.value = mergeNodes(nodes.value as PrimeVueTreeNode[], node.key, subNodes)
}
const onNodeSelect = async(node: PrimeVueTreeNode) => {
  if (node.leaf) {
    emit('select', node.key)
  }
}

</script>

<template>
  <Tree selectionMode="single"
        v-if="nodes"
        :value="nodes"
        @nodeSelect="onNodeSelect"
        @nodeExpand="onNodeExpand"
        :loading="loadingScriptsInDirectory"></Tree>
</template>

<style scoped>

</style>
