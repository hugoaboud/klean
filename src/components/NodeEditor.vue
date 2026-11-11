<script setup lang="ts">
// Body of the editor modal: doc block, rename field, code editor, and the
// right-hand panel that shows what Lean says — the goals left to prove and the
// errors and warnings inside this node.
//
// Edits are flushed to Rust with a short debounce, and Ctrl/Cmd+S flushes now;
// each flush re-checks the file through the Lean toolchain.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import hljs from "../lib/highlight";
import type { Goals, NodeDetail } from "../api/lean";
import UiButton from "./ui/UiButton.vue";
import UiCodeEditor from "./ui/UiCodeEditor.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiSpinner from "./ui/UiSpinner.vue";

const props = defineProps<{
  detail: NodeDetail | null;
  saving: boolean;
  saveError: string | null;
  /** Goals at the caret, from Lean's language server. */
  goals: Goals | null;
  goalsBusy: boolean;
  /** True while Lean is still checking this file. */
  processing?: boolean;
  /** The type of the term at the caret, as Lean renders it. */
  term?: string | null;
  termBusy?: boolean;
  /** The prompt row only exists when AI features are switched on. */
  aiEnabled?: boolean;
}>();

const emit = defineEmits<{
  apply: [string];
  comments: [string];
  rename: [string];
  file: [];
  /** Caret moved: file-absolute 1-based line, UTF-16 column. */
  goals: [number, number];
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
    line: diagnostic.line - (props.detail?.start_line ?? 1) + 1,
    severity: diagnostic.severity,
  })),
);
const canRename = computed(() => editable.value && Boolean(props.detail?.renamable));
const status = computed(() => {
  if (!editable.value) return { tone: "muted", label: "read only" };
  if (props.saveError) return { tone: "error", label: props.saveError };
  if (props.processing) return { tone: "busy", label: "lean is checking…" };
  if (props.saving) return { tone: "busy", label: "saving…" };
  if (dirty.value) return { tone: "dirty", label: "unsaved" };
  return { tone: "ok", label: "in sync" };
});

/** Errors and warnings inside the node, worst first. */
const diagnostics = computed(() => {
  const order: Record<string, number> = { error: 0, warning: 1, info: 2 };
  return [...(props.detail?.diagnostics ?? [])].sort(
    (a, b) => (order[a.severity] ?? 3) - (order[b.severity] ?? 3) || a.line - b.line,
  );
});

const goalLines = computed(() => props.goals?.goals ?? []);

/** Lean snippets in the state panel, coloured with the editor's own grammar. */
function highlighted(code: string): string {
  try {
    return hljs.highlight(code, { language: "lean", ignoreIllegals: true }).value;
  } catch {
    return code.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
}

/** The term of the caret as a code block, without Lean's markdown fences. */
const termCode = computed(() => {
  const text = props.term ?? "";
  return text
    .replace(/```[a-zA-Z]*\n?/g, "")
    .split("\n")
    .map((line) => line.replace(/^\s{2,}/, (indent) => indent.length >= 2 ? "" : indent))
    .join("\n")
    .trim();
});

/**
 * The width of the state column, dragged by the divider. Kept in the component:
 * it is view state, and the modal remembers it for as long as it is open.
 */
const sideWidth = ref(230);
let dragStart: { x: number; width: number } | null = null;

function startDrag(event: PointerEvent) {
  dragStart = { x: event.clientX, width: sideWidth.value };
  (event.target as HTMLElement).setPointerCapture(event.pointerId);
  event.preventDefault();
}

function onDrag(event: PointerEvent) {
  if (!dragStart) return;
  // Dragging left widens the state column, and it never eats the code editor.
  const next = dragStart.width - (event.clientX - dragStart.x);
  const room = window.innerWidth * 0.6;
  sideWidth.value = Math.max(160, Math.min(room, next));
}

function endDrag(event: PointerEvent) {
  if (!dragStart) return;
  dragStart = null;
  (event.target as HTMLElement).releasePointerCapture(event.pointerId);
}

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

/**
 * Caret inside the node, expressed in file coordinates for Lean: the node's own
 * line and, on its first line, the column it starts at.
 */
function onCaret(line: number, character: number) {
  const detail = props.detail;
  if (!detail) return;
  const first = line === 1 ? (detail.start_column ?? 0) : 0;
  emit("goals", detail.start_line + line - 1, first + character);
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
    <div class="node-editor__body">
      <div class="node-editor__main">
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
            @caret="onCaret"
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
      </div>

      <div
        class="node-editor__divider"
        title="Drag to resize the state panel"
        role="separator"
        aria-orientation="vertical"
        @pointerdown="startDrag"
        @pointermove="onDrag"
        @pointerup="endDrag"
        @pointercancel="endDrag"
      />

      <aside class="node-editor__side" :style="{ width: `${sideWidth}px` }">
        <div class="node-editor__section">
          <span class="k-label">
            term
            <UiSpinner v-if="termBusy" :size="9" />
          </span>
          <pre
            v-if="termCode"
            class="node-editor__goal hljs"
            v-html="highlighted(termCode)"
          />
          <p v-else class="node-editor__hint">
            put the caret on a term to see what Lean makes of it
          </p>
        </div>

        <div class="node-editor__section">
          <span class="k-label">
            proof state
            <UiSpinner v-if="goalsBusy" :size="9" />
          </span>
          <div v-if="goalLines.length" class="node-editor__goals">
            <pre
              v-for="(goal, index) in goalLines"
              :key="index"
              class="node-editor__goal hljs"
              v-html="highlighted(goal)"
            />
          </div>
          <p v-else class="node-editor__hint">
            {{ goals?.note ?? "move the caret into the proof to see the goals" }}
          </p>
        </div>

        <div class="node-editor__section node-editor__section--grow">
          <span class="k-label">
            diagnostics
            <span class="node-editor__count k-mono">{{ diagnostics.length }}</span>
          </span>
          <div v-if="diagnostics.length" class="node-editor__diagnostics">
            <div
              v-for="(diagnostic, index) in diagnostics"
              :key="index"
              class="node-editor__diagnostic"
              :data-severity="diagnostic.severity"
            >
              <span class="k-mono">{{ diagnostic.line }}</span>
              <span class="node-editor__message">{{ diagnostic.message }}</span>
            </div>
          </div>
          <p v-else class="node-editor__hint">Lean reports nothing inside this node.</p>
        </div>
      </aside>
    </div>

    <footer class="node-editor__bar">
      <span class="node-editor__status" :data-tone="status.tone">
        <UiSpinner v-if="saving || processing" :size="9" />
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

.node-editor__body {
  display: flex;
  flex: 1;
  min-height: 0;
  gap: 4px;
}

.node-editor__main {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  gap: 8px;
}

.node-editor__divider {
  width: 7px;
  flex: 0 0 auto;
  cursor: col-resize;
  border-radius: 3px;
  background: transparent;
  transition: background-color var(--t) var(--ease);
}

.node-editor__divider:hover {
  background: var(--accent-soft, rgba(120, 160, 210, 0.16));
}

.node-editor__side {
  display: flex;
  flex-direction: column;
  min-width: 160px;
  gap: 8px;
  padding-left: 8px;
  border-left: 1px solid var(--line);
  overflow: auto;
}

.node-editor__section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.node-editor__section--grow {
  min-height: 0;
}

.node-editor__section > .k-label {
  display: flex;
  align-items: center;
  gap: 5px;
}

.node-editor__count {
  color: var(--text-ghost);
  font-size: 9px;
}

.node-editor__goals {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.node-editor__goal {
  margin: 0;
  padding: 5px 7px;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: #0c0e10;
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
}

.node-editor__hint {
  margin: 0;
  color: var(--text-ghost);
  font-size: var(--fs-xs);
  font-style: italic;
  line-height: 1.5;
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
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.node-editor__diagnostic {
  display: flex;
  gap: 6px;
  padding: 3px 6px;
  border-left: 2px solid var(--line-strong);
  background: rgba(194, 166, 112, 0.03);
  font-size: var(--fs-xs);
  color: var(--text-faint);
}

.node-editor__diagnostic[data-severity="error"] {
  border-left-color: var(--danger);
  color: var(--danger);
}

.node-editor__diagnostic[data-severity="warning"] {
  border-left-color: var(--warning);
  color: var(--warning);
}

.node-editor__diagnostic[data-severity="info"] {
  border-left-color: var(--line-strong);
  color: var(--text-ghost);
}

.node-editor__message {
  white-space: pre-wrap;
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
