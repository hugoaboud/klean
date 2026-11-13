<script setup lang="ts">
// Body of one editor pane: the code, the comments above it, and the panel that
// shows what Lean says — the goals left to prove and the errors it found.
//
// Typing is checked, not saved. A pause (Settings → Editor → Check Debounce) sends
// the edited declaration to Rust, which splices it into the file *in memory* and
// hands it to Lean: diagnostics come back without a write. The disk changes only
// when the user saves (Ctrl/Cmd+S), and closing with unsaved changes asks first.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import hljs from "../lib/highlight";
import type { Goals, NodeDetail } from "../api/lean";
import type { EditorKeys } from "./ui/CodeEditor.vue";
import CodeEditor from "./ui/CodeEditor.vue";
import UiButton from "./ui/UiButton.vue";
import UiIcon from "./ui/UiIcon.vue";
import UiSpinner from "./ui/UiSpinner.vue";

const props = defineProps<{
  detail: NodeDetail | null;
  saving: boolean;
  error: string | null;
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
  /** The shortcut combinations, as configured. */
  keys?: EditorKeys;
  /** The editor holds more than one caret. */
  multi?: boolean;
  /** Goals of the whole declaration, not of the caret: what the icon shows. */
  nodeGoals?: Goals | null;
  /** A background step that is taking a while, e.g. re-reading the board. */
  busy?: string | null;
  /** The pane is fetching the node it was pointed at; the old one is still shown. */
  loading?: boolean;
  /**
   * Another editor of the same file holds an unsaved edit: this one reads only,
   * until that is saved or thrown away.
   */
  locked?: boolean;
}>();

const emit = defineEmits<{
  /** Save it: the one thing that writes the file and re-reads the tree. */
  save: [string];
  /** Whether there is an unsaved edit, for the close confirmation. */
  dirty: [boolean];
  /** Put the saved text back, in front of Lean too. */
  revert: [];
  comments: [string];
  file: [];
  /** Caret moved: file-absolute 1-based line, UTF-16 column. */
  goals: [number, number];
  /** Ask Lean for the goals at the end of this declaration. */
  refreshGoals: [];
  /** Ask the AI about this node: an AI session seeded with it. */
  prompt: [string];
  /** How many selections the editor holds (more than one means several carets). */
  selections: [number];
}>();

const text = ref("");
const comments = ref("");
const prompt = ref("");
const commentsBox = ref<HTMLTextAreaElement | null>(null);
/** The right-hand panel (term, proof state, diagnostics) on or off. */
const sideVisible = ref(true);
const promptBox = ref<HTMLTextAreaElement | null>(null);

/** The state panel is shown for proofs; the term stays available above it. */
const isTheorem = computed(() => {
  const kind = props.detail?.kind ?? "";
  return kind === "theorem" || kind === "lemma";
});

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

const dirty = ref(false);

watch(dirty, (value) => emit("dirty", value));

const editable = computed(() => (props.detail?.editable ?? false) && !props.locked);
/** What the footer says, and whether it is spinning. */
const status = computed(() => {
  if (props.error) return { tone: "error", label: props.error, busy: false };
  if (props.saving) return { tone: "busy", label: "saving…", busy: true };
  if (props.loading) return { tone: "busy", label: "opening…", busy: true };
  if (props.locked)
    return {
      tone: "dirty",
      label: "another editor of this file has unsaved changes",
      busy: false,
    };
  if (props.processing) return { tone: "busy", label: "lean is checking…", busy: true };
  if (props.busy) return { tone: "busy", label: `${props.busy}…`, busy: true };
  if (!editable.value) return { tone: "muted", label: "read only", busy: false };
  if (dirty.value) return { tone: "dirty", label: "unsaved — checked in memory", busy: false };
  return { tone: "ok", label: "saved", busy: false };
});

/** What Lean reported, on the file's own lines: the editor paints them. */
const markers = computed(() =>
  (props.detail?.diagnostics ?? []).map((diagnostic) => ({
    line: diagnostic.line,
    end_line: diagnostic.end_line,
    severity: diagnostic.severity,
    message: diagnostic.message,
  })),
);

/** Errors and warnings inside the node, worst first. */
const diagnostics = computed(() => {
  const order: Record<string, number> = { error: 0, warning: 1, info: 2 };
  return [...(props.detail?.diagnostics ?? [])].sort(
    (a, b) => (order[a.severity] ?? 3) - (order[b.severity] ?? 3) || a.line - b.line,
  );
});

const goalLines = computed(() => props.goals?.goals ?? []);

/**
 * The proof state of the *declaration*, for the icon beside the comments.
 *
 * It comes from the goals at the end of the declaration, not from the caret: a
 * theorem that is proved stays green while the caret sits on an intermediate
 * tactic line, where there are goals to show.
 */
const proofState = computed(() => {
  const goals = props.nodeGoals;
  if (!goals) return { tone: "idle", text: "⊢", title: "Lean has not answered for this node yet" };
  if (goals.goals.length === 0) {
    return { tone: "done", text: "⊢", title: goals.note ?? "no goals left: the declaration is proved" };
  }
  return {
    tone: "open",
    text: String(goals.goals.length),
    title: `${goals.goals.length} goal${goals.goals.length === 1 ? "" : "s"} left at the end of the declaration`,
  };
});

/** A file has no file to open, and no terms of its own to hover. */
const isFile = computed(() => (props.detail?.kind ?? "") === "file");

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
    .map((line) => line.replace(/^\s{2,}/, (indent) => (indent.length >= 2 ? "" : indent)))
    .join("\n")
    .trim();
});

/**
 * The width of the state column, dragged by the divider. Kept in the component:
 * it is view state, and the pane remembers it for as long as it is open.
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
    text.value = props.detail?.text ?? "";
    comments.value = props.detail?.comments ?? "";
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
  // Nothing reaches Rust until the save: the text belongs to the editor, and a save
  // is the one thing that writes the file and re-reads the tree.
  dirty.value = true;
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

/** Saves what is in the editor. The only path that writes to disk. */
function save() {
  if (!dirty.value || !editable.value) return;
  dirty.value = false;
  emit("save", text.value);
}

/**
 * Puts the saved text back.
 *
 * Nothing was written while typing, so reverting is a reload: the parent re-reads
 * the node from the file and the editor takes the text it comes back with.
 */
function revert() {
  dirty.value = false;
  emit("revert");
}

let commentTimer: number | undefined;

function onCommentsInput(value: string) {
  comments.value = value;
  window.clearTimeout(commentTimer);
  commentTimer = window.setTimeout(() => emit("comments", comments.value), 750);
}

/** Sends what is in the prompt box as an AI request about this node. */
function sendPrompt() {
  const text = prompt.value.trim();
  if (!text) return;
  prompt.value = "";
  emit("prompt", text);
}

function flushComments() {
  window.clearTimeout(commentTimer);
  emit("comments", comments.value);
}

onBeforeUnmount(() => {
  window.clearTimeout(commentTimer);
});
</script>

<template>
  <div v-if="!detail" class="node-editor node-editor--empty">
    <div class="node-editor__placeholder">
      <UiSpinner v-if="!error" :size="12" />
      <UiIcon v-else name="alert" :size="12" />
      <span>{{ error ?? "opening…" }}</span>
    </div>
  </div>

  <div v-else class="node-editor">
    <div class="node-editor__body">
      <div class="node-editor__main">
        <blockquote v-if="detail.doc" class="node-editor__doc">{{ detail.doc }}</blockquote>

        <div v-if="detail.editable" class="node-editor__comments">
          <textarea
            ref="commentsBox"
            class="node-editor__comments-input k-mono"
            :value="comments"
            rows="1"
            spellcheck="false"
            placeholder="-- no comments"
            @input="
              onCommentsInput(($event.target as HTMLTextAreaElement).value);
              grow(commentsBox, 190);
            "
            @blur="flushComments"
            @keydown.ctrl.s.prevent="flushComments"
            @keydown.meta.s.prevent="flushComments"
          />
          <!-- The panel on the right, on or off. It is the last thing in this row,
               after the proof state, so the eye finds both where it writes. -->
          <button
            class="node-editor__panel"
            type="button"
            :title="sideVisible ? 'Hide the term and proof state panel' : 'Show the term and proof state panel'"
            :aria-pressed="sideVisible"
            :data-on="sideVisible || undefined"
            @click="sideVisible = !sideVisible"
          >
            <UiIcon name="outline" :size="11" />
          </button>
          <!-- On a theorem, the proof state sits with the comments, where the eye
               already is while writing the declaration. -->
          <button
            v-if="isTheorem"
            class="node-editor__proof"
            :data-tone="proofState.tone"
            :title="proofState.title"
            type="button"
            @click="emit('refreshGoals')"
          >
            <span class="node-editor__proof-glyph">⊢</span>
            <span v-if="proofState.text !== '⊢'" class="node-editor__proof-count k-mono">
              {{ proofState.text }}
            </span>
            <UiSpinner v-if="goalsBusy" :size="9" />
          </button>
        </div>

        <div class="node-editor__code">
          <CodeEditor
            v-if="detail.editable"
            :model-value="text"
            :doc-id="detail.id"
            :readonly="!editable"
            language="lean"
            :start-line="detail.start_line"
            :markers="markers"
            :keys="keys"
            @update:model-value="onInput"
            @caret="onCaret"
            @selections="(count: number) => emit('selections', count)"
            @commit="save"
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
            placeholder="What do you need?"
            @input="grow(promptBox, 120)"
          />
          <UiButton size="sm" variant="solid" :disabled="!prompt.trim()" @click="sendPrompt">
            send
          </UiButton>
        </div>
      </div>

      <div
        v-if="sideVisible"
        class="node-editor__divider"
        title="Drag to resize the state panel"
        role="separator"
        aria-orientation="vertical"
        @pointerdown="startDrag"
        @pointermove="onDrag"
        @pointerup="endDrag"
        @pointercancel="endDrag"
      />

      <aside v-if="sideVisible" class="node-editor__side" :style="{ width: `${sideWidth}px` }">
        <div class="node-editor__section">
          <span class="k-label">
            term
            <UiSpinner v-if="termBusy && !multi" :size="9" />
          </span>
          <!-- Several carets are several terms: there is no single type to ask. -->
          <p v-if="multi" class="node-editor__hint">multiple terms</p>
          <pre
            v-else-if="termCode"
            class="node-editor__goal hljs"
            v-html="highlighted(termCode)"
          />
          <p v-else class="node-editor__hint">
            put the caret on a term to see what Lean makes of it
          </p>
        </div>

        <!-- A proof state belongs to proofs: declarations have nothing to prove. -->
        <div v-if="isTheorem" class="node-editor__section">
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
              @click="emit('goals', diagnostic.line, 0)"
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
        <UiSpinner v-if="status.busy" :size="9" />
        <UiIcon v-else name="dot" :size="10" />
        {{ status.label }}
      </span>
      <span class="k-spacer" />
      <UiButton v-if="detail.editable" size="sm" variant="ghost" :disabled="!dirty" @click="revert">
        revert
      </UiButton>
      <UiButton
        v-if="!isFile"
        size="sm"
        variant="ghost"
        title="Open the file this node comes from"
        @click="emit('file')"
      >
        <UiIcon name="outline" :size="11" /> file
      </UiButton>
      <UiButton
        v-if="detail.editable"
        class="node-editor__save"
        size="sm"
        variant="solid"
        :disabled="!dirty"
        title="Write the file to disk (Ctrl/Cmd+S)"
        @click="save"
      >
        <UiIcon name="save" :size="11" /> save
        <span v-if="dirty" class="node-editor__badge" aria-hidden="true" />
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
  flex: 0 0 5px;
  border-radius: 2px;
  background: transparent;
  cursor: col-resize;
}

.node-editor__divider:hover {
  background: var(--line-soft);
}

.node-editor__side {
  display: flex;
  flex-direction: column;
  min-width: 160px;
  min-height: 0;
  gap: 10px;
  overflow: auto;
}

.node-editor__section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.node-editor__section--grow {
  flex: 1;
  min-height: 0;
}

/* The state panel is read at a glance while typing, so it runs a step larger than
   the rest of the modal: one scale here covers the labels, the goals, the hints and
   the diagnostics. */
.node-editor__side .k-label {
  font-size: calc(var(--fs-micro) * 1.2);
}

.node-editor__side .node-editor__goal,
.node-editor__side .node-editor__hint,
.node-editor__side .node-editor__diagnostic {
  font-size: calc(var(--fs-xs) * 1.2);
}

.node-editor__side .node-editor__count {
  font-size: calc(9px * 1.2);
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

.node-editor__doc {
  margin: 0;
  padding: 6px 9px;
  border-left: 2px solid var(--line-strong);
  background: rgba(255, 255, 255, 0.015);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
}

.node-editor__comments {
  display: flex;
  align-items: flex-start;
  gap: 6px;
}

.node-editor__comments-input {
  flex: 1;
  min-height: 24px;
  padding: 4px 7px;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  resize: none;
  outline: none;
  background: var(--bg-0);
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.node-editor__comments-input::placeholder {
  /* Darker than the text it stands in for: a hint, not content. */
  color: var(--text-ghost);
  opacity: 0.55;
}

.node-editor__comments-input:focus {
  border-color: var(--accent-line);
}

/* The panel toggle, the last control in the comments row. */
.node-editor__panel {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: var(--bg-0);
  color: var(--text-ghost);
  cursor: pointer;
}

.node-editor__panel:hover {
  color: var(--text);
  border-color: var(--line-strong);
}

.node-editor__panel[data-on] {
  color: var(--accent);
  border-color: var(--accent-line);
}

/* The proof state, beside the comments of a theorem. */
.node-editor__proof {
  display: flex;
  align-items: center;
  gap: 3px;
  padding: 3px 6px;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: var(--bg-0);
  color: var(--text-ghost);
  font-size: var(--fs-micro);
  cursor: pointer;
}

.node-editor__proof[data-tone="open"] {
  border-color: rgba(194, 166, 112, 0.4);
  color: var(--warning);
}

.node-editor__proof[data-tone="done"] {
  border-color: rgba(126, 176, 138, 0.4);
  color: var(--ok);
}

.node-editor__proof-glyph {
  font-size: var(--fs-xs);
}

.node-editor__proof-count {
  font-variant-numeric: tabular-nums;
}

.node-editor--empty {
  align-items: center;
  justify-content: center;
}

.node-editor__placeholder {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text-ghost);
  font-size: var(--fs-xs);
}

.node-editor__code {
  display: flex;
  flex: 1;
  min-height: 0;
}

.node-editor__readonly {
  flex: 1;
  margin: 0;
  padding: 8px 10px;
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: #0c0e10;
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 18px;
  white-space: pre;
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
  cursor: pointer;
}

.node-editor__diagnostic:hover {
  background: rgba(194, 166, 112, 0.08);
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
  min-height: 26px;
  padding: 5px 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  resize: none;
  outline: none;
  background: var(--bg-0);
  color: var(--text);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
}

.node-editor__prompt-input::placeholder {
  /* Darker than the text it stands in for: a hint, not content. */
  color: var(--text-ghost);
  opacity: 0.55;
}

.node-editor__prompt-input:focus {
  border-color: var(--accent-line);
}

.node-editor__bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 0 8px;
}

.node-editor__status {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-micro);
  color: var(--text-ghost);
}

.node-editor__status[data-tone="ok"] {
  color: var(--text-faint);
}

.node-editor__status[data-tone="dirty"] {
  color: var(--warning);
}

.node-editor__status[data-tone="error"] {
  color: var(--danger);
}

/* A dot on the save button: there is something the disk does not have yet. */
.node-editor__save {
  position: relative;
}

.node-editor__badge {
  position: absolute;
  top: -2px;
  right: -2px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
  box-shadow: 0 0 0 1px var(--bg-0);
}
</style>
