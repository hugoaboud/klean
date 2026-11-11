<script setup lang="ts">
// Recursive item tree with rubber-band multi-selection.
//
// Expansion is view state; node data and selection come from the store. Rows emit
// `pick` (with the additive flag for ctrl/cmd) and the container emits
// `selectMany` for a dragged rectangle. On the logic tree rows are also dragged
// onto a theory row, which emits `drop`.
import { onBeforeUnmount, reactive, ref, watch } from "vue";
import type { TreeNode } from "../../api/lean";
import UiTreeItem from "./UiTreeItem.vue";

const props = withDefaults(
  defineProps<{
    items: TreeNode[];
    selectedIds: string[];
    statusKinds?: string[];
    /** Terms and namespaces can be dragged onto a drop target. */
    draggable?: boolean;
    /** Kinds of the rows that accept a drop. */
    dropKinds?: string[];
    /** Rubber-band selection of several rows. Off where rows are dragged. */
    banding?: boolean;
  }>(),
  { statusKinds: () => [], draggable: false, dropKinds: () => [], banding: true },
);

const emit = defineEmits<{
  pick: [string, boolean];
  open: [string];
  openTheory: [string];
  selectMany: [string[]];
  drop: [ids: string[], target: string];
}>();

const expandedIds = reactive(new Set<string>());
const selected = ref(new Set<string>());
const host = ref<HTMLElement | null>(null);
const banding = ref(false);
const bandRect = reactive({ x: 0, y: 0, w: 0, h: 0, visible: false });
let cleanupBand: (() => void) | null = null;

watch(
  () => props.selectedIds,
  (ids) => {
    selected.value = new Set(ids);
  },
  { immediate: true },
);

/** Libraries, folders and columns start open; files stay closed until asked. */
function seed(nodes: TreeNode[]) {
  for (const node of nodes) {
    if (node.child_count === 0 && node.children.length === 0) continue;
    if (node.kind === "lib" || node.kind === "folder" || node.kind === "theory") {
      expandedIds.add(node.id);
      seed(node.children);
    }
  }
}

function pathTo(id: string, nodes: TreeNode[], trail: string[] = []): string[] | null {
  for (const node of nodes) {
    const next = [...trail, node.id];
    if (node.id === id) return next;
    const found = pathTo(id, node.children, next);
    if (found) return found;
  }
  return null;
}

watch(
  () => props.items,
  (items) => seed(items),
  { immediate: true },
);

// Reveal the selection coming from the graph.
watch(
  () => props.selectedIds,
  (ids) => {
    for (const id of ids) {
      const path = pathTo(id, props.items);
      if (!path) continue;
      for (const ancestor of path.slice(0, -1)) expandedIds.add(ancestor);
    }
  },
  { immediate: true },
);

function onPick(id: string, additive: boolean) {
  emit("pick", id, additive);
}

function toggle(id: string) {
  if (!expandedIds.delete(id)) expandedIds.add(id);
}

/** Drag on empty (or any) tree area selects every row inside the rectangle. */
function onPointerDown(event: PointerEvent) {
  if (!props.banding || event.button !== 0) return;
  if (!host.value) return;
  const startX = event.clientX;
  const startY = event.clientY;
  const bounds = host.value.getBoundingClientRect();
  let started = false;

  const move = (moveEvent: PointerEvent) => {
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    if (!started && Math.hypot(dx, dy) < 5) return;
    if (!started) {
      started = true;
      banding.value = true;
    }
    bandRect.x = Math.min(startX, moveEvent.clientX) - bounds.left;
    bandRect.y = Math.min(startY, moveEvent.clientY) - bounds.top + host.value!.scrollTop;
    bandRect.w = Math.abs(dx);
    bandRect.h = Math.abs(dy);
    bandRect.visible = true;
    moveEvent.preventDefault();
  };

  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    if (started) {
      const left = bandRect.x;
      const top = bandRect.y;
      const right = left + bandRect.w;
      const bottom = top + bandRect.h;
      const ids: string[] = [];
      for (const row of host.value?.querySelectorAll<HTMLElement>("[data-node-id]") ?? []) {
        const rect = row.getBoundingClientRect();
        const rowTop = rect.top - bounds.top + host.value!.scrollTop;
        const rowBottom = rowTop + rect.height;
        const rowLeft = rect.left - bounds.left;
        const rowRight = rowLeft + rect.width;
        if (rowBottom >= top && rowTop <= bottom && rowRight >= left && rowLeft <= right) {
          const id = row.dataset.nodeId;
          if (id) ids.push(id);
        }
      }
      emit("selectMany", ids);
    }
    banding.value = false;
    bandRect.visible = false;
    // Let the pointerup settle before rows accept clicks again.
    window.setTimeout(() => {
      cleanupBand = null;
    }, 0);
  };

  cleanupBand = up;
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

onBeforeUnmount(() => {
  cleanupBand?.();
});

defineExpose({
  collapseAll() {
    expandedIds.clear();
  },
  expandAll() {
    const visit = (nodes: TreeNode[]) => {
      for (const node of nodes) {
        if (node.children.length > 0) {
          expandedIds.add(node.id);
          visit(node.children);
        }
      }
    };
    visit(props.items);
  },
});
</script>

<template>
  <div
    ref="host"
    class="ui-tree"
    :data-banding="banding || undefined"
    @pointerdown="onPointerDown"
  >
    <UiTreeItem
      v-for="node in items"
      :key="node.id"
      :node="node"
      :depth="0"
      :selected-ids="selected"
      :expanded-ids="expandedIds"
      :status-kinds="statusKinds"
      :draggable="draggable"
      :drop-kinds="dropKinds"
      @pick="onPick"
      @open="emit('open', $event)"
      @open-theory="emit('openTheory', $event)"
      @toggle="toggle"
      @drop="(ids: string[], target: string) => emit('drop', ids, target)"
    />
    <div
      v-if="bandRect.visible"
      class="ui-tree__band"
      :style="{
        left: bandRect.x + 'px',
        top: bandRect.y + 'px',
        width: bandRect.w + 'px',
        height: bandRect.h + 'px',
      }"
    />
  </div>
</template>

<style scoped>
.ui-tree {
  position: relative;
  padding: 4px 0 12px;
  overflow-x: hidden;
  min-height: 100%;
}

.ui-tree[data-banding] :deep(.item__row) {
  pointer-events: none;
}

.ui-tree__band {
  position: absolute;
  border: 1px solid var(--accent-line);
  background: var(--accent-soft);
  pointer-events: none;
}
</style>
