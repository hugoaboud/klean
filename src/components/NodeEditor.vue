<script setup lang="ts">
// Body of the editor modal: doc block, rename field, code editor, diagnostics.
// Edits are flushed to Rust with a short debounce, and Ctrl/Cmd+S flushes now.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { NodeDetail } from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiCodeEditor from "./ui/UiCodeEditor.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiSpinner from "./ui/UiSpinner.vue";

const props = defineProps<{
  detail: NodeDetail | null;
  saving: boolean;
  saveError: string | null;
  /** The prompt row only exists when AI features are switched on. */
  aiEnabled?: boolean;
}>();

const emit = defineEmits<{
  apply: [string];
  comments: [string];
  rename: [string];
  file: [];
}>();

const text = ref("");
const comments = ref("");

const prompt = ref("");
const commentsBox = ref<HTMLTextAreaElement | null>(null);
const promptBox = ref<HTMLTextAreaElement | null>(null);

/** Textareas grow with their content, up to a limit, then scroll. */
function grow(box: HTMLTextAreaElement | null, limit: number) {
  if (!box) return;
  box.style.height = "auto";
  box.style.height = `${Math.min(box.scrollHeight, limit)}px`;
  box.style.overflowY = box.scrollHeight > limit ? "auto" : "hidden";
}

onMounted(() => {
  grow(commentsBox.value, 190);
  grow(promptBox.value, 120);
});
const name = ref("");
const dirty = ref(false);
let timer: number | undefined;

const editable = computed(() => props.detail?.editable ?? false);
const markers = computed(() =>
  (props.detail?.diagnostics ?? []).map((diagnostic) => ({
    line: diagnostic.line,
    severity: diagnostic.severity,
  })),
);
const canRename = computed(() => editable.value && Boolean(props.detail?.renamable));
const status = computed(() => {
  if (!editable.value) return { tone: "muted", label: "read only" };
  if (props.saveError) return { tone: "error", label: props.saveError };
  if (props.saving) return { tone: "busy", label: "writing…" };
  if (dirty.value) return { tone: "dirty", label: "unsaved" };
  return { tone: "ok", label: "in sync" };
});

watch(
  () => props.detail?.id,
  () => {
    window.clearTimeout(timer);
    text.value = props.detail?.text ?? "";
    comments.value = props.detail?.comments ?? "";
    name.value = props.detail?.name ?? "";
    dirty.value = false;
    // Boxes grow to fit whatever the node already carries.
    nextTick(() => {
      grow(commentsBox.value, 190);
      grow(promptBox.value, 120);
    });
  },
  { immediate: true },
);

watch(
  () => props.detail?.text,
  (value) => {
    if (value === undefined) return;
    if (value === text.value) return;
    if (dirty.value) return;
    text.value = value;
  },
);

function onInput(value: string) {
  text.value = value;
  if (!editable.value) return;
  dirty.value = true;
  window.clearTimeout(timer);
  timer = window.setTimeout(flush, 750);
}

function flush() {
  window.clearTimeout(timer);
  if (!dirty.value || !editable.value) return;
  const snapshot = text.value;
  dirty.value = false;
  emit("apply", snapshot);
}

function revert() {
  window.clearTimeout(timer);
  text.value = props.detail?.text ?? "";
  dirty.value = false;
}

let commentTimer: number | undefined;

function onCommentsInput(value: string) {
  comments.value = value;
  window.clearTimeout(commentTimer);
  commentTimer = window.setTimeout(() => emit("comments", comments.value), 750);
}

/** Placeholder: the prompt box is wired up later. */
function sendPrompt() {
  prompt.value = "";
}

function flushComments() {
  window.clearTimeout(commentTimer);
  emit("comments", comments.value);
}

function commitName() {
  const current = props.detail?.name ?? "";
  const next = name.value.trim();
  if (!next) {
    name.value = current;
    return;
  }
  if (next !== current) emit("rename", next);
}

onBeforeUnmount(() => {
  window.clearTimeout(timer);
  window.clearTimeout(commentTimer);
});
</script>

<template>
  <div v-if="detail" class="node-editor">
    <div class="node-editor__meta">
      <div class="node-editor__facts">
        <span class="k-label">lines</span>
        <span class="k-mono">{{ detail.start_line }}–{{ detail.end_line }}</span>
        <span class="node-editor__sep">·</span>
        <span class="k-label">path</span>
        <span class="k-mono node-editor__path">{{ detail.path ?? "—" }}</span>
      </div>
      <div v-if="canRename" class="node-editor__rename">
        <span class="k-label">name</span>
        <input
          v-model="name"
          class="node-editor__input k-mono"
          spellcheck="false"
          @keydown.enter.prevent="commitName"
          @blur="commitName"
        />
        <UiButton size="sm" variant="subtle" title="Rename the declaration" @click="commitName">
          rename
        </UiButton>
      </div>
    </div>

    <blockquote v-if="detail.doc" class="node-editor__doc">{{ detail.doc }}</blockquote>

    <div v-if="detail.editable" class="node-editor__comments">
      <span class="k-label">comments</span>
      <textarea
        class="node-editor__comments-input k-mono"
        ref="commentsBox"
        :value="comments"
        rows="1"
        spellcheck="false"
        placeholder="-- plain comments directly above this declaration"
        @input="
          onCommentsInput(($event.target as HTMLTextAreaElement).value);
          grow(commentsBox, 190);
        "
        @blur="flushComments"
        @keydown.ctrl.s.prevent="flushComments"
        @keydown.meta.s.prevent="flushComments"
      />
    </div>

    <div class="node-editor__code">
      <UiCodeEditor
        v-if="detail.editable"
        :model-value="text"
        language="lean"
        :markers="markers"
        @update:model-value="onInput"
        @commit="flush"
      />
      <pre v-else class="node-editor__readonly">{{ detail.text }}</pre>
    </div>

    <div v-if="aiEnabled" class="node-editor__prompt">
      <textarea
        ref="promptBox"
        v-model="prompt"
        class="node-editor__prompt-input k-mono"
        rows="1"
        spellcheck="false"
        placeholder="ask for a change to this node…"
        @input="grow(promptBox, 120)"
      />
      <UiButton size="sm" variant="solid" :disabled="!prompt.trim()" @click="sendPrompt">
        send
      </UiButton>
    </div>

    <div v-if="detail.diagnostics.length" class="node-editor__diagnostics">
      <div
        v-for="(diagnostic, index) in detail.diagnostics"
        :key="index"
        class="node-editor__diagnostic"
        :data-severity="diagnostic.severity"
      >
        <span class="k-mono">{{ diagnostic.line }}</span>
        <span>{{ diagnostic.message }}</span>
      </div>
    </div>

    <footer class="node-editor__bar">
      <span class="node-editor__status" :data-tone="status.tone">
        <UiSpinner v-if="saving" :size="9" />
        <UiIcon v-else name="dot" :size="10" />
        {{ status.label }}
      </span>
      <span class="k-spacer" />
      <UiButton v-if="detail.editable" size="sm" variant="ghost" :disabled="!dirty" @click="revert">
        revert
      </UiButton>
      <UiButton size="sm" variant="ghost" @click="emit('file')" title="Show the file this term comes from">
        <UiIcon name="outline" :size="11" /> file
      </UiButton>
      <UiButton v-if="detail.editable" size="sm" variant="solid" :disabled="!dirty" @click="flush">
        <UiIcon name="save" :size="11" /> save
      </UiButton>
    </footer>
  </div>
</template>

<style scoped>
.node-editor {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  padding: 10px 12px 0;
  gap: 8px;
}

.node-editor__meta {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}

.node-editor__facts {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.node-editor__facts :deep(.k-mono) {
  color: var(--text-faint);
  font-size: 10px;
}

.node-editor__path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 260px;
}

.node-editor__sep {
  color: var(--text-ghost);
}

.node-editor__rename {
  display: flex;
  align-items: center;
  gap: 6px;
}

.node-editor__input {
  width: 150px;
  height: 21px;
  padding: 0 6px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--bg-1);
  color: var(--text);
  font-size: 10.5px;
  transition: border-color var(--t) var(--ease), background-color var(--t) var(--ease);
}

.node-editor__input:focus {
  border-color: var(--accent-line);
  background: var(--bg-2);
}

.node-editor__doc {
  margin: 0;
  padding: 6px 9px;
  border-left: 2px solid var(--line-strong);
  background: rgba(255, 255, 255, 0.012);
  border-radius: 0 2px 2px 0;
  color: var(--text-faint);
  font-size: var(--fs-xs);
  font-family: var(--mono);
  line-height: 1.55;
  max-height: 76px;
  overflow: auto;
  white-space: pre-wrap;
}

.node-editor__code {
  display: flex;
  flex: 1;
  min-height: 0;
}

.node-editor__readonly {
  flex: 1;
  margin: 0;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #0c0e10;
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.55;
  overflow: auto;
}

.node-editor__diagnostics {
  max-height: 62px;
  overflow: auto;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: rgba(194, 166, 112, 0.03);
}

.node-editor__diagnostic {
  display: flex;
  gap: 8px;
  padding: 3px 8px;
  font-size: var(--fs-xs);
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
}

.node-editor__diagnostic:last-child {
  border-bottom: none;
}

.node-editor__diagnostic[data-severity="error"] {
  color: var(--danger);
}

.node-editor__diagnostic[data-severity="warning"] {
  color: var(--warning);
}

.node-editor__prompt {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.node-editor__prompt-input {
  flex: 1;
  min-height: 28px;
  padding: 5px 8px;
  overflow: hidden;
  resize: none;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
  color: var(--text);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.node-editor__prompt-input:focus {
  border-color: var(--accent-line);
  background: var(--bg-2);
}

.node-editor__comments {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.node-editor__comments-input {
  overflow: hidden;
  resize: vertical;
  resize: vertical;
  min-height: 34px;
  max-height: 90px;
  padding: 6px 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #0c0e10;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.node-editor__comments-input:focus {
  border-color: var(--accent-line);
  color: var(--text);
}

.node-editor__bar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  margin: 0 -12px;
  padding: 0 12px;
  border-top: 1px solid var(--line);
  background: linear-gradient(180deg, #121517 0%, #0f1113 100%);
}

.node-editor__status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-micro);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-ghost);
}

.node-editor__status[data-tone="ok"] {
  color: var(--ok);
}

.node-editor__status[data-tone="dirty"] {
  color: var(--warning);
}

.node-editor__status[data-tone="error"] {
  color: var(--danger);
}

.node-editor__status[data-tone="busy"] {
  color: var(--accent);
}

.node-editor__status[data-tone="muted"] {
  color: var(--text-ghost);
}
</style>
