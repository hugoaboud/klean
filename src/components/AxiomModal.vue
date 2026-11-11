<script setup lang="ts">
// Asks for a name and where the axiom goes: an existing file picked from the
// project, or a new one at a path the user types.
import { computed, ref, watch } from "vue";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiModal from "./ui/UiModal.vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** Lean files of the project, as paths relative to its root. */
    files: string[];
    /** Directory the current board looks at, used to suggest a new file name. */
    defaultDir?: string;
  }>(),
  { defaultDir: "" },
);

const emit = defineEmits<{
  cancel: [];
  create: [{ name: string; file: string | null; path: string | null }];
}>();

const name = ref("");
const mode = ref<"pick" | "new">("pick");
const file = ref("");
const path = ref("");
const filter = ref("");

const visible = computed(() => {
  const needle = filter.value.trim().toLowerCase();
  const files = needle
    ? props.files.filter((entry) => entry.toLowerCase().includes(needle))
    : props.files;
  return files.slice(0, 200);
});

const ready = computed(() => {
  if (!name.value.trim()) return false;
  if (mode.value === "pick") return file.value.trim().length > 0;
  return path.value.trim().length > 0;
});

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    name.value = "";
    filter.value = "";
    mode.value = props.files.length > 0 ? "pick" : "new";
    file.value = props.files[0] ?? "";
    path.value = props.defaultDir ? `${props.defaultDir}/New.lean` : "New.lean";
  },
  { immediate: true },
);

function submit() {
  if (!ready.value) return;
  const trimmed = name.value.trim();
  if (mode.value === "pick") {
    emit("create", { name: trimmed, file: file.value.trim(), path: null });
  } else {
    emit("create", { name: trimmed, file: null, path: path.value.trim() });
  }
}
</script>

<template>
  <UiModal :open="open" title="New axiom" subtitle="written as `axiom name : Prop`" :width="440" :height="430" @close="emit('cancel')">
    <div class="axiom">
      <label class="axiom__field">
        <span class="k-label">name</span>
        <input
          v-model="name"
          class="axiom__input k-mono"
          type="text"
          placeholder="assumption"
          spellcheck="false"
          autofocus
          @keydown.enter="submit"
        />
      </label>

      <div class="axiom__modes">
        <button
          class="axiom__mode"
          type="button"
          :data-on="mode === 'pick' || undefined"
          :disabled="files.length === 0"
          @click="mode = 'pick'"
        >
          pick file
        </button>
        <button
          class="axiom__mode"
          type="button"
          :data-on="mode === 'new' || undefined"
          @click="mode = 'new'"
        >
          new file
        </button>
      </div>

      <template v-if="mode === 'pick'">
        <input
          v-model="filter"
          class="axiom__input axiom__input--filter k-mono"
          type="text"
          placeholder="filter files"
          spellcheck="false"
        />
        <div class="axiom__files">
          <button
            v-for="entry in visible"
            :key="entry"
            class="axiom__file"
            type="button"
            :data-on="entry === file || undefined"
            :title="entry"
            @click="file = entry"
          >
            <UiIcon name="file-plus" :size="10" />
            <span>{{ entry }}</span>
          </button>
          <p v-if="visible.length === 0" class="axiom__empty">no file matches</p>
        </div>
      </template>

      <label v-else class="axiom__field">
        <span class="k-label">path of the new file</span>
        <input
          v-model="path"
          class="axiom__input k-mono"
          type="text"
          placeholder="Lib/Assumptions.lean"
          spellcheck="false"
          @keydown.enter="submit"
        />
      </label>

      <footer class="axiom__bar">
        <span class="axiom__hint k-mono">
          {{ mode === "pick" ? file || "no file picked" : path || "no path" }}
        </span>
        <span class="k-spacer" />
        <UiButton size="sm" variant="ghost" @click="emit('cancel')">cancel</UiButton>
        <UiButton size="sm" variant="solid" :disabled="!ready" @click="submit">
          <UiIcon name="plus" :size="11" /> add axiom
        </UiButton>
      </footer>
    </div>
  </UiModal>
</template>

<style scoped>
.axiom {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  min-height: 0;
  padding: 10px 12px 10px;
}

.axiom__field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.axiom__input {
  height: 24px;
  padding: 0 7px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-2);
  color: var(--text);
  font-size: var(--fs-sm);
  outline: none;
  transition: border-color var(--t-fast) var(--ease);
}

.axiom__input:focus {
  border-color: var(--accent-line);
}

.axiom__input--filter {
  height: 22px;
  color: var(--text-dim);
  font-size: var(--fs-micro);
}

.axiom__modes {
  display: flex;
  gap: 4px;
}

.axiom__mode {
  padding: 3px 9px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
  color: var(--text-faint);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: color var(--t-fast) var(--ease), border-color var(--t-fast) var(--ease),
    background var(--t-fast) var(--ease);
}

.axiom__mode:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--line-strong);
}

.axiom__mode[data-on] {
  border-color: var(--accent-line);
  background: var(--accent-soft);
  color: var(--accent-bright);
}

.axiom__mode:disabled {
  opacity: 0.5;
  cursor: default;
}

.axiom__files {
  flex: 1;
  min-height: 90px;
  overflow: auto;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
  background: var(--bg-2);
  padding: 3px;
}

.axiom__file {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 2px 6px;
  border: 0;
  border-radius: 2px;
  background: transparent;
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-micro);
  text-align: left;
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.axiom__file:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.axiom__file[data-on] {
  background: var(--accent-soft);
  color: var(--accent-bright);
}

.axiom__empty {
  margin: 6px;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}

.axiom__bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-top: 2px;
}

.axiom__hint {
  overflow: hidden;
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
