<script setup lang="ts">
// Project settings.
//
// The tree comes from Rust; this component only edits the values. Rows are
// rendered by kind: a switch for booleans, a select for enums, an input for
// text. Nothing is written until "save" — closing with unsaved changes asks
// first.
import { computed, ref, watch } from "vue";
import type { SettingNode } from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiModal from "./ui/UiModal.vue";

const props = defineProps<{
  open: boolean;
  tree: SettingNode[];
  /** Group left open by default: the one belonging to the current page. */
  rootId: string;
}>();

const emit = defineEmits<{
  close: [];
  save: [[string, string][]];
}>();

const draft = ref<SettingNode[]>([]);
const expanded = ref<Set<string>>(new Set());
const confirming = ref(false);

function clone(nodes: SettingNode[]): SettingNode[] {
  return nodes.map((node) => ({ ...node, children: clone(node.children) }));
}

function collect(nodes: SettingNode[], out: Map<string, string>): Map<string, string> {
  for (const node of nodes) {
    if (node.kind !== "group") out.set(node.id, node.value);
    collect(node.children, out);
  }
  return out;
}

watch(
  () => [props.open, props.tree, props.rootId] as const,
  ([open]) => {
    if (!open) {
      confirming.value = false;
      return;
    }
    draft.value = clone(props.tree);
    expanded.value = new Set(props.rootId ? [props.rootId] : []);
    // Nested groups start closed except the ones that carry the page's own
    // sections, which would otherwise need two clicks to reach.
    for (const group of props.tree) {
      if (group.id !== props.rootId) continue;
      for (const child of group.children) {
        if (child.kind === "group" && child.id.endsWith(".sections")) expanded.value.add(child.id);
      }
    }
    confirming.value = false;
  },
  { immediate: true, deep: false },
);

const original = computed(() => collect(props.tree, new Map()));
const current = computed(() => collect(draft.value, new Map()));
const dirty = computed(() => {
  for (const [id, value] of current.value) {
    if ((original.value.get(id) ?? "") !== value) return true;
  }
  return false;
});

/** A setting is shown unless it waits on a switch that is off. */
function visible(node: SettingNode): boolean {
  if (!node.depends_on) return true;
  return current.value.get(node.depends_on) === "true";
}

function toggle(id: string) {
  const next = new Set(expanded.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  expanded.value = next;
}

function setValue(node: SettingNode, value: string) {
  node.value = value;
  // Rebuild so `dirty` recomputes against the mutated tree.
  draft.value = [...draft.value];
}

function save() {
  const values: [string, string][] = [...current.value.entries()];
  emit("save", values);
  confirming.value = false;
}

function requestClose() {
  if (dirty.value) {
    confirming.value = true;
    return;
  }
  emit("close");
}

function discard() {
  confirming.value = false;
  emit("close");
}
</script>

<template>
  <UiModal
    :open="open"
    title="settings"
    :subtitle="dirty ? 'unsaved changes' : 'stored in .klean.json'"
    :width="520"
    :height="440"
    @close="requestClose"
  >
    <div class="settings">
      <div v-if="confirming" class="settings__warn">
        <UiIcon name="alert" :size="12" />
        <span>discard unsaved changes?</span>
        <UiButton size="sm" variant="ghost" @click="confirming = false">keep editing</UiButton>
        <UiButton size="sm" variant="solid" @click="discard">discard</UiButton>
      </div>

      <div class="settings__tree">
        <template v-for="group in draft" :key="group.id">
          <button class="settings__group" type="button" @click="toggle(group.id)">
            <span class="settings__chevron">{{ expanded.has(group.id) ? "▾" : "▸" }}</span>
            <span class="settings__group-label">{{ group.label }}</span>
            <span v-if="group.help" class="settings__help">{{ group.help }}</span>
          </button>

          <template v-if="expanded.has(group.id)">
            <template v-for="child in group.children" :key="child.id">
              <template v-if="!visible(child)"></template>
              <button
                v-if="child.kind === 'group' && visible(child)"
                class="settings__group settings__group--nested"
                type="button"
                @click="toggle(child.id)"
              >
                <span class="settings__chevron">{{ expanded.has(child.id) ? "▾" : "▸" }}</span>
                <span class="settings__group-label">{{ child.label }}</span>
                <span v-if="child.help" class="settings__help">{{ child.help }}</span>
              </button>

              <template v-if="child.kind === 'group' && expanded.has(child.id)">
                <label
                  v-for="leaf in child.children.filter(visible)"
                  :key="leaf.id"
                  class="settings__row"
                >
                  <span class="settings__label">
                    {{ leaf.label }}
                    <span v-if="leaf.help" class="settings__help">{{ leaf.help }}</span>
                  </span>

                  <button
                    v-if="leaf.kind === 'bool'"
                    class="settings__switch"
                    :data-on="leaf.value === 'true' || undefined"
                    type="button"
                    @click="setValue(leaf, leaf.value === 'true' ? 'false' : 'true')"
                  >
                    <span />
                  </button>

                  <select
                    v-else-if="leaf.kind === 'enum'"
                    class="settings__select k-mono"
                    :value="leaf.value"
                    @change="setValue(leaf, ($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="option in leaf.options" :key="option" :value="option">
                      {{ option }}
                    </option>
                  </select>

                  <input
                    v-else
                    class="settings__input k-mono"
                    :type="leaf.id.endsWith('api_key') ? 'password' : 'text'"
                    autocomplete="off"
                    :value="leaf.value"
                    spellcheck="false"
                    @input="setValue(leaf, ($event.target as HTMLInputElement).value)"
                  />
                </label>
              </template>

              <label v-else-if="child.kind !== 'group' && visible(child)" class="settings__row">
                <span class="settings__label">
                  {{ child.label }}
                  <span v-if="child.help" class="settings__help">{{ child.help }}</span>
                </span>

                <button
                  v-if="child.kind === 'bool'"
                  class="settings__switch"
                  :data-on="child.value === 'true' || undefined"
                  type="button"
                  @click="setValue(child, child.value === 'true' ? 'false' : 'true')"
                >
                  <span />
                </button>

                <select
                  v-else-if="child.kind === 'enum'"
                  class="settings__select k-mono"
                  :value="child.value"
                  @change="setValue(child, ($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="option in child.options" :key="option" :value="option">
                    {{ option }}
                  </option>
                </select>

                <input
                  v-else
                  class="settings__input k-mono"
                  :value="child.value"
                  spellcheck="false"
                  @input="setValue(child, ($event.target as HTMLInputElement).value)"
                />
              </label>
            </template>
          </template>
        </template>

        <div v-if="draft.length === 0" class="settings__empty k-label">no settings</div>
      </div>
    </div>

    <template #footer>
      <span class="settings__state k-label">{{ dirty ? "changed" : "unchanged" }}</span>
      <UiButton size="sm" variant="ghost" @click="requestClose">close</UiButton>
      <UiButton size="sm" variant="solid" :disabled="!dirty" @click="save">save</UiButton>
    </template>
  </UiModal>
</template>

<style scoped>
.settings {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  gap: 8px;
}

.settings__warn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border: 1px solid rgba(194, 166, 112, 0.36);
  border-radius: var(--radius);
  background: var(--warning-soft);
  color: var(--warning);
  font-size: var(--fs-xs);
}

.settings__warn span:first-of-type {
  flex: 1;
}

.settings__tree {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
}

.settings__group {
  display: flex;
  align-items: baseline;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  border: 0;
  border-bottom: 1px solid var(--line-soft);
  background: var(--bg-2);
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  letter-spacing: 0.06em;
  text-align: left;
  text-transform: uppercase;
  cursor: pointer;
}

.settings__group:hover {
  background: var(--bg-3);
  color: var(--text);
}

.settings__group--nested {
  padding-left: 20px;
  background: var(--bg-1);
  text-transform: none;
  letter-spacing: 0.02em;
}

.settings__chevron {
  width: 9px;
  color: var(--text-ghost);
}

.settings__group-label {
  flex: none;
}

.settings__row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 8px 5px 34px;
  border-bottom: 1px solid var(--line-soft);
}

.settings__row:last-child {
  border-bottom: 0;
}

.settings__row:hover {
  background: var(--bg-2);
}

.settings__label {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 1px;
  font-size: var(--fs-xs);
  color: var(--text);
}

.settings__help {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  letter-spacing: 0.02em;
  text-transform: none;
  font-family: var(--font);
}

.settings__input,
.settings__select {
  width: 128px;
  padding: 3px 6px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-0);
  color: var(--text);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.settings__input:focus,
.settings__select:focus {
  outline: none;
  border-color: var(--accent-line);
}

.settings__switch {
  flex: none;
  width: 28px;
  height: 15px;
  padding: 2px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--bg-0);
  cursor: pointer;
  transition: background var(--t) var(--ease), border-color var(--t) var(--ease);
}

.settings__switch span {
  display: block;
  width: 9px;
  height: 9px;
  border-radius: 999px;
  background: var(--text-ghost);
  transition: transform var(--t) var(--ease), background var(--t) var(--ease);
}

.settings__switch[data-on] {
  background: var(--accent-soft);
  border-color: var(--accent-line);
}

.settings__switch[data-on] span {
  transform: translateX(13px);
  background: var(--accent);
}

.settings__empty {
  padding: 10px;
}

.settings__state {
  flex: 1;
}
</style>
