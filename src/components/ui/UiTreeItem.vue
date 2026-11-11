<script setup lang="ts">
// One row of the AST tree. Rows are compact: a short kind badge, the declared
// name, and a muted detail (signature, counts). Children render recursively.
import { computed, ref, watch } from "vue";
import type { TreeNode } from "../../api/lean";
import UiIcon from "./UiIcon.vue";

const props = withDefaults(
  defineProps<{
    node: TreeNode;
    depth: number;
    selectedIds: Set<string>;
    expandedIds: Set<string>;
    /** Kinds that show their proof state; empty means any node that has one. */
    statusKinds?: string[];
    /** Terms and namespaces can be dragged onto a theory row. */
    draggable?: boolean;
    /** Kinds of the rows that accept a drop. */
    dropKinds?: string[];
  }>(),
  { statusKinds: () => [], draggable: false, dropKinds: () => [] },
);

const emit = defineEmits<{
  pick: [string, boolean];
  open: [string];
  openTheory: [string];
  toggle: [string];
  drop: [ids: string[], target: string];
}>();

const KIND_CODES: Record<string, string> = {
  lib: "LIB",
  folder: "DIR",
  file: "FILE",
  astgroup: "AST",
  def: "def",
  theorem: "thm",
  lemma: "lem",
  axiom: "axm",
  structure: "str",
  class: "cls",
  inductive: "ind",
  instance: "ins",
  abbrev: "abr",
  opaque: "opq",
  example: "exm",
  namespace: "ns",
  theory: "thy",
  unassigned: "una",
  mutual: "mut",
  import: "imp",
  prelude: "pre",
  open: "opn",
  export: "exp",
  variable: "var",
  universe: "uni",
  set_option: "opt",
  attribute: "att",
  deriving: "drv",
  initialize: "ini",
  syntax: "syn",
  macro: "mac",
  macro_rules: "mcr",
  notation: "not",
  elab: "elb",
  field: "fld",
  constructor: "ctr",
  where: "whr",
  command: "cmd",
};

const expanded = computed(() => props.expandedIds.has(props.node.id));
const selected = computed(() => props.selectedIds.has(props.node.id));
const hasChildren = computed(() => props.node.child_count > 0 || props.node.children.length > 0);
const code = computed(() => KIND_CODES[props.node.kind] ?? props.node.kind.slice(0, 3));
const status = computed(() => props.node.status ?? null);
/** Terms (and the namespaces holding them) travel between theories. */
const MOVEABLE = new Set(["def", "theorem", "lemma", "axiom", "structure", "class", "namespace"]);
const canDrag = computed(() => props.draggable && MOVEABLE.has(props.node.kind));
const acceptsDrop = computed(() => props.dropKinds.includes(props.node.kind));
const dragOver = ref(false);
const statusKindsList = computed(() => props.statusKinds);
const dropKindsList = computed(() => props.dropKinds);
const statusKinds = computed(() => new Set(props.statusKinds));
const showStatus = computed(
  () =>
    Boolean(status.value) &&
    (statusKinds.value.size === 0 || statusKinds.value.has(props.node.kind)),
);

function onChildPick(id: string, additive: boolean) {
  emit("pick", id, additive);
}

function onDragStart(event: DragEvent) {
  if (!canDrag.value || !event.dataTransfer) return;
  event.dataTransfer.setData("text/plain", props.node.id);
  event.dataTransfer.effectAllowed = "move";
}

function onDrop(event: DragEvent) {
  if (!acceptsDrop.value) return;
  event.preventDefault();
  dragOver.value = false;
  const ids = (event.dataTransfer?.getData("text/plain") ?? "").split("\n").filter(Boolean);
  if (ids.length === 0) return;
  emit("drop", ids, props.node.id);
}
const statusTitle = computed(() => {
  if (status.value === "proven") return "no sorry left in the proof";
  if (status.value === "sorry") return "proof contains sorry";
  if (status.value === "error") return "structural error in this declaration";
  return "";
});
const tooltip = computed(() =>
  props.node.doc ? props.node.doc.split("\n").slice(0, 6).join("\n") : props.node.label,
);

const row = ref<HTMLElement | null>(null);

watch(selected, (isSelected) => {
  if (isSelected) row.value?.scrollIntoView({ block: "nearest", inline: "nearest" });
});
</script>

<template>
  <div class="item">
    <div
      ref="row"
      class="item__row"
      :data-selected="selected || undefined"
      :data-kind="node.kind"
      :style="{ paddingLeft: 6 + depth * 11 + 'px' }"
      :title="tooltip"
      :draggable="canDrag || undefined"
      :data-drop="dragOver || undefined"
      @click="emit('pick', node.id, $event.ctrlKey || $event.metaKey)"
      @dblclick.stop="node.kind === 'theory' ? emit('openTheory', node.id) : emit('open', node.id)"
      @dragstart="onDragStart"
      @dragover.prevent="acceptsDrop ? (dragOver = true) : undefined"
      @dragleave="dragOver = false"
      @drop="onDrop"
    >
      <button
        v-if="hasChildren"
        class="item__toggle"
        type="button"
        :aria-expanded="expanded"
        @click.stop="emit('toggle', node.id)"
      >
        <UiIcon name="chevron" :size="9" :class="{ 'is-open': expanded }" />
      </button>
      <span v-else class="item__toggle item__toggle--empty" />

      <span v-if="showStatus" class="item__status" :data-status="status" :title="statusTitle">
        <UiIcon :name="status === 'proven' ? 'check' : status === 'sorry' ? 'alert' : 'bang'" :size="10" />
      </span>

      <span v-else class="item__code" :data-kind="node.kind">{{ code }}</span>
      <span class="item__label">{{ node.label }}</span>
      <span v-if="node.detail" class="item__detail">{{ node.detail }}</span>
    </div>

    <div v-if="hasChildren && expanded" class="item__children">
      <UiTreeItem
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :depth="depth + 1"
        :selected-ids="selectedIds"
        :expanded-ids="expandedIds"
        :status-kinds="statusKindsList"
        :draggable="draggable"
        :drop-kinds="dropKindsList"
        @pick="onChildPick"
        @open="emit('open', $event)"
        @open-theory="emit('openTheory', $event)"
        @toggle="emit('toggle', $event)"
        @drop="(ids: string[], target: string) => emit('drop', ids, target)"
      />
    </div>
  </div>
</template>

<style scoped>
.item__row {
  display: flex;
  align-items: center;
  gap: 5px;
  height: 19px;
  padding-right: 24px;
  border-left: 1px solid transparent;
  color: var(--text-dim);
  font-size: var(--fs-sm);
  white-space: nowrap;
  overflow: hidden;
  cursor: default;
  transition:
    background-color var(--t-fast) var(--ease),
    color var(--t-fast) var(--ease),
    border-color var(--t-fast) var(--ease);
}

.item__row:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.item__row[data-selected] {
  background: linear-gradient(90deg, var(--accent-soft), rgba(106, 169, 196, 0.02));
  border-left-color: var(--accent);
  color: var(--text);
}

.item__row[data-kind="lib"] {
  background: rgba(106, 169, 196, 0.055);
}

.item__row[data-kind="theory"] {
  background: rgba(106, 169, 196, 0.07);
  border-top: 1px solid var(--line);
}

.item__row[data-kind="theory"] .item__label,
.item__row[data-kind="unassigned"] .item__label {
  color: var(--accent-bright);
  letter-spacing: 0.06em;
}

.item__row[data-kind="unassigned"] {
  background: rgba(255, 255, 255, 0.03);
  border-top: 1px solid var(--line);
  color: var(--text-faint);
}

/* Row a dragged term is hovering: it is about to land in this theory. */
.item__row[data-drop] {
  background: rgba(124, 171, 136, 0.16);
  border-left-color: var(--ok);
  color: var(--text);
}

.item__row[data-kind="folder"],
.item__row[data-kind="namespace"] {
  background: rgba(255, 255, 255, 0.026);
}

.item__row[data-kind="file"] {
  background: rgba(255, 255, 255, 0.012);
}

.item__row[data-kind="lib"] .item__label {
  color: #93cbe0;
}

.item__row[data-kind="folder"] .item__label {
  color: #a8a2b8;
}

.item__row[data-kind="file"] .item__label {
  color: #c3c9ce;
}

.item__row[data-kind="lib"]:hover .item__label,
.item__row[data-kind="folder"]:hover .item__label,
.item__row[data-kind="file"]:hover .item__label {
  color: var(--text);
}

.item__toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 12px;
  height: 12px;
  color: var(--text-ghost);
  border-radius: 2px;
  transition: color var(--t) var(--ease), background-color var(--t) var(--ease);
}

.item__toggle:hover {
  color: var(--text);
  background: var(--bg-4);
}

.item__toggle :deep(svg) {
  transition: transform var(--t) var(--ease);
}

.item__toggle :deep(svg.is-open) {
  transform: rotate(90deg);
}

.item__toggle--empty {
  pointer-events: none;
}

.item__status {
  flex: none;
  display: inline-flex;
  margin-right: -1px;
  color: var(--text-ghost);
}




.item__status[data-status="proven"] {
  color: var(--ok);
}

.item__status[data-status="sorry"] {
  color: var(--warning);
}

.item__status[data-status="error"] {
  color: var(--danger);
}

.item__code {
  flex: none;
  width: 26px;
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.02em;
  text-align: right;
  color: var(--text-ghost);
  transition: color var(--t-fast) var(--ease);
}

.item__code[data-kind="lib"],
.item__code[data-kind="namespace"],
.item__code[data-kind="mutual"] {
  color: var(--accent);
}

.item__code[data-kind="def"],
.item__code[data-kind="abbrev"],
.item__code[data-kind="opaque"] {
  color: #82a894;
}

.item__code[data-kind="theorem"],
.item__code[data-kind="lemma"],
.item__code[data-kind="example"] {
  color: #9c94c4;
}

.item__code[data-kind="structure"],
.item__code[data-kind="class"],
.item__code[data-kind="inductive"] {
  color: #bda069;
}

.item__code[data-kind="instance"],
.item__code[data-kind="attribute"],
.item__code[data-kind="deriving"] {
  color: #b8877a;
}

.item__label {
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: var(--fs-sm);
}

.item__detail {
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-ghost);
  font-family: var(--mono);
  font-size: 10px;
  transition: color var(--t-fast) var(--ease);
}

.item__row:hover .item__detail,
.item__row[data-selected] .item__detail {
  color: var(--text-faint);
}
</style>
