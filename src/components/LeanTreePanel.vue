<script setup lang="ts">
// Right column: the AST item tree with file management in the header.
import { computed, ref } from "vue";
import type { TreeNode } from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiTree from "./ui/UiTree.vue";

const props = defineProps<{
  tree: TreeNode[];
  selectedIds: string[];
  files: number;
  nodes: number;
}>();

const emit = defineEmits<{
  pick: [string, boolean];
  open: [string, MouseEvent];
  selectMany: [string[]];
  create: [];
  delete: [string];
}>();

const treeRef = ref<InstanceType<typeof UiTree> | null>(null);

function onPick(id: string, additive: boolean) {
  emit("pick", id, additive);
}

function findNode(nodes: TreeNode[], id: string): TreeNode | null {
  for (const node of nodes) {
    if (node.id === id) return node;
    const found = findNode(node.children, id);
    if (found) return found;
  }
  return null;
}

/** The file the delete button acts on: the single selected file, or its file. */
const target = computed(() => {
  if (props.selectedIds.length !== 1) return null;
  const id = props.selectedIds[0];
  const node = findNode(props.tree, id);
  if (!node) return null;
  if (node.kind === "file") return node;
  const base = id.split("#")[0];
  const rel = base.startsWith("ast:") ? base.slice(4) : node.path;
  return rel ? findNode(props.tree, `file:${rel}`) : null;
});
</script>

<template>
  <section class="panel">
    <header class="panel__head">
      <span class="k-label">ast tree</span>
      <span class="panel__stats k-mono">{{ files }} files · {{ nodes }} nodes</span>
      <span class="k-spacer" />
      <UiButton size="sm" variant="subtle" title="New Lean file" @click="emit('create')">
        <UiIcon name="file-plus" :size="11" />
      </UiButton>
      <UiButton
        size="sm"
        variant="subtle"
        :disabled="!target"
        :title="target ? `Delete ${target.label}` : 'Select one file to delete'"
        @click="target && emit('delete', target.path ?? '')"
      >
        <UiIcon name="trash" :size="11" />
      </UiButton>
      <span class="panel__sep" />
      <UiButton size="sm" variant="subtle" title="Collapse all" @click="treeRef?.collapseAll()">
        <UiIcon name="minus" :size="11" />
      </UiButton>
      <UiButton size="sm" variant="subtle" title="Expand all" @click="treeRef?.expandAll()">
        <UiIcon name="plus" :size="11" />
      </UiButton>
    </header>
    <div class="panel__body">
      <UiTree
        ref="treeRef"
        :items="tree"
        :selected-ids="selectedIds"
        :status-kinds="['lib', 'folder', 'file']"
        @pick="onPick"
        @open="(id: string, event: MouseEvent) => emit('open', id, event)"
        @select-many="emit('selectMany', $event)"
      />
    </div>
  </section>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  background: var(--bg-1);
}

.panel__head {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 8px 0 10px;
  border-bottom: 1px solid var(--line);
  background: var(--grad-header);
}

.panel__stats {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.panel__sep {
  width: 1px;
  height: 14px;
  margin: 0 2px;
  background: var(--line);
}

.panel__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
</style>
