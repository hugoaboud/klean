<script setup lang="ts">
// Right column of the logic board: one root per theory, with its namespaces and
// terms inside and the theories nested in it. Terms are dragged from here onto a
// theory row (or onto a theory node on the canvas) to move them.
import { ref } from "vue";
import type { TreeNode } from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiTree from "./ui/UiTree.vue";

defineProps<{
  tree: TreeNode[];
  selectedIds: string[];
  visible: number;
  total: number;
  uses: number;
  openTheory: string;
}>();

const emit = defineEmits<{
  pick: [string, boolean];
  open: [string, MouseEvent];
  openTheory: [string];
  selectMany: [string[]];
  assign: [ids: string[], target: string | null];
}>();

const treeRef = ref<InstanceType<typeof UiTree> | null>(null);
</script>

<template>
  <section class="panel">
    <header class="panel__head">
      <span class="k-label">logic tree</span>
      <span class="panel__stats k-mono">{{ visible }}/{{ total }} terms · {{ uses }} links</span>
      <span class="k-spacer" />
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
        :status-kinds="[]"
        :draggable="true"
        :banding="false"
        :drop-kinds="['theory', 'unassigned']"
        @pick="(id: string, additive: boolean) => emit('pick', id, additive)"
        @open="(id: string, event: MouseEvent) => emit('open', id, event)"
        @open-theory="emit('openTheory', $event)"
        @select-many="emit('selectMany', $event)"
        @drop="
          (ids: string[], target: string) =>
            emit('assign', ids, target === 'theory:unassigned' ? null : target)
        "
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

.panel__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
</style>
