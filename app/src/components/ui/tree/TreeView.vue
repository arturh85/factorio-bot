<script setup lang="ts">
import {ChevronDown, ChevronRight, FileCode} from '@lucide/vue';
import type {ScriptTreeNode} from '@/api/types';

// Self-referencing by filename: Vue resolves <TreeView> inside this template
// to this component, which is how the recursion works without a named export.
const props = withDefaults(defineProps<{
  nodes: ScriptTreeNode[];
  expandedKeys: string[];
  selectedKey: string | null;
  level?: number;
}>(), {
  level: 0
});

const emit = defineEmits<{
  toggle: [node: ScriptTreeNode];
  select: [node: ScriptTreeNode];
}>();

function isExpanded(node: ScriptTreeNode): boolean {
  return props.expandedKeys.includes(node.key);
}

function activate(node: ScriptTreeNode): void {
  if (node.leaf) {
    emit('select', node);
  } else {
    emit('toggle', node);
  }
}
</script>

<template>
  <ul :role="level === 0 ? 'tree' : 'group'" class="m-0 list-none p-0 text-sm">
    <li
      v-for="node in nodes"
      :key="node.key"
      role="treeitem"
      :aria-expanded="node.leaf ? undefined : isExpanded(node)"
      :aria-selected="selectedKey === node.key">
      <button
        type="button"
        class="flex w-full cursor-pointer items-center gap-1 rounded-card px-2 py-1 text-left transition-colors hover:bg-divider focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
        :class="selectedKey === node.key ? 'bg-divider font-medium text-brand' : 'text-ink'"
        :style="{paddingLeft: (level * 12 + 8) + 'px'}"
        @click="activate(node)">
        <ChevronDown v-if="!node.leaf && isExpanded(node)" class="size-4 shrink-0" aria-hidden="true"/>
        <ChevronRight v-else-if="!node.leaf" class="size-4 shrink-0" aria-hidden="true"/>
        <FileCode v-else class="size-4 shrink-0 text-ink-muted" aria-hidden="true"/>
        <span class="truncate">{{ node.label }}</span>
      </button>

      <TreeView
        v-if="!node.leaf && isExpanded(node)"
        :nodes="node.children"
        :expanded-keys="expandedKeys"
        :selected-key="selectedKey"
        :level="level + 1"
        @toggle="emit('toggle', $event)"
        @select="emit('select', $event)"/>
    </li>
  </ul>
</template>
