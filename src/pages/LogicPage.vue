<script setup lang="ts">
// Logic board: a stack of graphs. The canvas shows the root graph of theories or
// one theory's graph; the tree on the right lists every theory with the terms it
// holds, and is the place terms are dragged from to another theory.
import { computed, ref } from "vue";
import AxiomModal from "../components/AxiomModal.vue";
import LogicCanvas from "../components/LogicCanvas.vue";
import LogicTreePanel from "../components/LogicTreePanel.vue";
import UiPrompt from "../components/ui/UiPrompt.vue";
import UiSplitPane from "../components/ui/UiSplitPane.vue";
import type { LogicGraph, TreeNode } from "../api/lean";
import { actions, panKey, state } from "../store/workspace";

const props = defineProps<{
  logic: LogicGraph | null;
  selectedIds: string[];
}>();

const emit = defineEmits<{ open: [string, Event] }>();

const split = defineModel<number>("split", { default: 0.62 });

const tree = computed(() => props.logic?.tree ?? []);
const nodes = computed(() => props.logic?.nodes ?? []);
const links = computed(() => props.logic?.links ?? []);
const layers = computed(() => props.logic?.layers ?? []);
const external = computed(() => props.logic?.external ?? []);
const trail = computed(() => props.logic?.trail ?? [{ key: "", name: "Logic" }]);
const theory = computed(() => state.openTheory);

const total = computed(() => {
  let count = 0;
  const walk = (items: TreeNode[]) => {
    for (const item of items) {
      if (item.kind !== "namespace" && item.kind !== "theory" && item.kind !== "unassigned") {
        count += 1;
      }
      walk(item.children);
    }
  };
  walk(tree.value);
  return count;
});

const visible = computed(() => nodes.value.filter((node) => node.kind !== "theory").length);

/** Name modal: a new theory, a new layer, or a rename. */
const prompt = ref<{
  kind: "theory" | "layer" | "rename" | "rename-theory";
  id?: string;
} | null>(null);

const renameTitles: Record<string, string> = {
  theory: "New theory",
  layer: "New layer",
  rename: "Rename layer",
  "rename-theory": "Rename theory",
};
const axiomOpen = ref(false);

/** Lean files of the project, for the "pick file" side of the axiom modal. */
const leanFiles = computed(() => {
  const out: string[] = [];
  const walk = (items: TreeNode[]) => {
    for (const item of items) {
      if (item.kind === "file" && item.path) out.push(item.path);
      walk(item.children);
    }
  };
  walk(state.view?.tree ?? []);
  return out;
});

/** Directory of the file the open theory's terms come from, if any. */
const currentDir = computed(() => {
  const path = nodes.value.find((node) => node.path)?.path ?? "";
  const cut = path.lastIndexOf("/");
  return cut > 0 ? path.slice(0, cut) : "";
});

function onPick(id: string, additive: boolean) {
  if (additive) void actions.toggleSelected(id);
  else void actions.select(id);
}

function onConfirmName(name: string) {
  const current = prompt.value;
  prompt.value = null;
  if (!current) return;
  if (current.kind === "theory") void actions.createTheory(name);
  if (current.kind === "layer") void actions.createLayer(name, layerTargetY());
  if (current.kind === "rename" && current.id) void actions.renameLayer(current.id, name);
  if (current.kind === "rename-theory" && current.id) void actions.renameTheory(current.id, name);
}

/** The theory being renamed, so the modal opens with its current name. */
const renamingTheory = ref("");

function onRenameTheory(key: string) {
  renamingTheory.value = nodes.value.find((node) => node.key === key)?.label ?? "";
  prompt.value = { kind: "rename-theory", id: key };
}

function onAddAxiom(payload: { name: string; file: string | null; path: string | null }) {
  axiomOpen.value = false;
  void actions.addAxiom(payload.name, payload.file, payload.path);
}

/** The layer being renamed, so the modal opens with its current name. */
const renaming = ref<string>("");

function onRenameLayer(id: string) {
  renaming.value = layers.value.find((layer) => layer.id === id)?.name ?? "";
  prompt.value = { kind: "rename", id };
}

/** A new divider goes under the content that is already there. */
function layerTargetY(): number {
  const points = nodes.value
    .filter((node) => node.kind !== "theory")
    .map((node) => node.y);
  if (points.length === 0) return 0;
  return Math.max(...points) + 40;
}
</script>

<template>
  <UiSplitPane v-model:left="split" class="logic-split">
    <template #left>
      <LogicCanvas
        :nodes="nodes"
        :links="links"
        :layers="layers"
        :external="external"
        :trail="trail"
        :theory="theory"
        :selected-ids="selectedIds"
        :camera="logic?.camera ?? null"
        :hover-info="state.hover"
        :pan-key="panKey()"
        @pick="onPick"
        @select="actions.selectMany($event)"
        @open="(id: string, event: Event) => emit('open', id, event)"
        @open-theory="actions.openTheory($event)"
        @crumb="actions.openTheory($event)"
        @camera="actions.saveCamera('logic', $event)"
        @hover="actions.loadHover($event)"
        @move="actions.savePositions('logic', $event)"
        @move-layer="(id: string, y: number) => actions.moveLayer(id, y)"
        @assign="(ids: string[], target: string | null) => actions.assignTheory(ids, target)"
        @delete-theory="actions.deleteTheory($event)"
        @delete-layer="actions.deleteLayer($event)"
        @rename-layer="onRenameLayer"
        @rename-theory="onRenameTheory"
        @delete-axiom="actions.deleteNode($event)"
        @add-axiom="axiomOpen = true"
        @create-theory="prompt = { kind: 'theory' }"
        @add-layer="prompt = { kind: 'layer' }"
        @settings="actions.setSettingsOpen(true)"
      />
    </template>
    <template #right>
      <LogicTreePanel
        :tree="tree"
        :selected-ids="selectedIds"
        :visible="visible"
        :total="total"
        :uses="links.length"
        :open-theory="theory"
        @pick="onPick"
        @open="(id: string, event: Event) => emit('open', id, event)"
        @open-theory="actions.openTheory($event)"
        @select-many="actions.selectMany($event)"
        @assign="(ids: string[], target: string | null) => actions.assignTheory(ids, target)"
      />
    </template>
  </UiSplitPane>

  <UiPrompt
    :open="prompt !== null"
    :title="renameTitles[prompt?.kind ?? 'theory']"
    :label="prompt?.kind === 'layer' || prompt?.kind === 'rename' ? 'Layer name' : 'Theory name'"
    :placeholder="prompt?.kind === 'theory' ? 'Theorems' : 'Assumptions'"
    :initial="
      prompt?.kind === 'rename' ? renaming : prompt?.kind === 'rename-theory' ? renamingTheory : ''
    "
    :confirm-label="
      prompt?.kind === 'rename' || prompt?.kind === 'rename-theory'
        ? 'Rename'
        : prompt?.kind === 'layer'
          ? 'Add layer'
          : 'Create theory'
    "
    @cancel="prompt = null"
    @confirm="onConfirmName"
  />

  <AxiomModal
    :open="axiomOpen"
    :files="leanFiles"
    :default-dir="currentDir"
    @cancel="axiomOpen = false"
    @create="onAddAxiom"
  />
</template>

<style scoped>
.logic-split {
  flex: 1;
  min-height: 0;
}
</style>
