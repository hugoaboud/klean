<script setup lang="ts">
// The code surface of a node editor — Monaco, dressed as KLEAN.
//
// The editor itself is Monaco's: selections and several carets, find with
// Enter/Shift+Enter/Esc, undo/redo, word-wise movement and deletion, bracket
// pairing and completion UI all come from it, which is what makes the modal behave
// like VS Code instead of like a textarea with intentions.
//
// What this component owns is the seam:
//
// - one Monaco *model* per document (`docId`), so undo history, folding and
//   markers belong to that node and not to whatever was open before;
// - the text going out (`update:modelValue`) and coming back in (a revert, or a
//   save that changed it elsewhere) without a feedback loop;
// - Lean's diagnostics as Monaco markers, on the file's own line numbers;
// - the shortcut settings, bound to Monaco's own commands, so the app's keys are
//   the editor's keys rather than a second set invented next to them;
// - completions, asked of Lean's language server through the project.
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import * as api from "../../api/lean";
import { loadMonaco, monacoKeybinding, type MonacoApi } from "../../editor/monaco";
import { texCompletion, texPrefix, texSuggestions } from "../../lib/lean-tex";

/** A problem Lean reported, on the file's own line numbers. */
export interface EditorMarker {
  /** 1-based line in the file where the problem starts. */
  line: number;
  /** 1-based line where it ends; the same as `line` for a point. */
  end_line?: number;
  severity: string;
  /** What Lean said, shown when the marker is hovered. */
  message?: string;
}

/** The editor commands the settings can rebind. */
export interface EditorKeys {
  undo: string;
  redo: string;
  find: string;
  addOccurrence: string;
  allOccurrences: string;
  cursorAbove: string;
  cursorBelow: string;
  duplicateUp: string;
  duplicateDown: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    /** Identity of the document: a new one starts a fresh model and history. */
    docId: string;
    /** 1-based line of the file the document starts on, for the gutter. */
    startLine?: number;
    readonly?: boolean;
    markers?: EditorMarker[];
    keys?: EditorKeys;
  }>(),
  {
    startLine: 1,
    readonly: false,
    markers: () => [],
    keys: () => ({
      undo: "Ctrl+Z",
      redo: "Ctrl+Shift+Z",
      find: "Ctrl+F",
      addOccurrence: "Ctrl+D",
      allOccurrences: "Ctrl+Shift+L",
      cursorAbove: "Alt+Shift+ArrowUp",
      cursorBelow: "Alt+Shift+ArrowDown",
      duplicateUp: "Ctrl+Alt+Shift+ArrowUp",
      duplicateDown: "Ctrl+Alt+Shift+ArrowDown",
    }),
  },
);

const emit = defineEmits<{
  "update:modelValue": [string];
  /** Ctrl/Cmd+S: write the document to disk. */
  commit: [];
  /** Caret position: 1-based line inside the document, UTF-16 column from 0. */
  caret: [number, number];
  /** How many selections are live: more than one means several carets. */
  selections: [number];
}>();

type Editor = import("monaco-editor").editor.IStandaloneCodeEditor;
type TextDocument = import("monaco-editor").editor.ITextModel;
type Disposable = import("monaco-editor").IDisposable;

const host = ref<HTMLElement | null>(null);
const monaco = shallowRef<MonacoApi | null>(null);
const editor = shallowRef<Editor | null>(null);
const model = shallowRef<TextDocument | null>(null);
/** True while the component itself is writing into the model. */
let quiet = false;
/** The last text handed to the parent, so a round trip is not an edit. */
let lastEmitted = props.modelValue;
/** The shortcut rules of the current document. */
let rules: Disposable | null = null;
/**
 * The models this editor created, by document id.
 *
 * They are owned here, not by Monaco and not by whatever else is open: a model
 * belongs to the document it was made for, so an editor that closes must not take
 * another editor's document down with it.
 */
const documents = new Map<string, TextDocument>();
/** How many documents one editor keeps open: each holds its own undo history. */
const KEEP_DOCUMENTS = 16;
/** The completion providers belong to Monaco, not to a pane: one pair for the app. */
let completionsRegistered = false;
let texRegistered = false;

/**
 * LSP `CompletionItemKind` → Monaco's, which numbers them differently.
 *
 * The two lists are close enough to look interchangeable and different enough to
 * paint every keyword as a file, so the translation is written out.
 */
const COMPLETION_KIND: Record<number, number> = {
  1: 18, // Text
  2: 0, // Method
  3: 1, // Function
  4: 2, // Constructor
  5: 3, // Field
  6: 4, // Variable
  7: 5, // Class
  8: 7, // Interface
  9: 8, // Module
  10: 9, // Property
  11: 12, // Unit
  12: 13, // Value
  13: 15, // Enum
  14: 17, // Keyword
  15: 25, // Snippet
  16: 19, // Color
  17: 20, // File
  18: 21, // Reference
  19: 23, // Folder
  20: 16, // EnumMember
  21: 14, // Constant
  22: 6, // Struct
  23: 10, // Event
  24: 11, // Operator
  25: 24, // TypeParameter
};

const MARKER_SEVERITY: Record<string, number> = { error: 8, warning: 4, info: 2 };

/** Lean's font stack, so the editor and the rest of the modal are the same face. */
function editorFont(): { family: string; size: number } {
  const style = window.getComputedStyle(document.documentElement);
  const family = style.getPropertyValue("--mono").trim() || "monospace";
  const size = Number.parseFloat(style.getPropertyValue("--fs-sm")) || 12;
  return { family, size };
}

function uriOf(docId: string) {
  return `inmemory://klean/${encodeURIComponent(docId)}`;
}

onMounted(async () => {
  const instance = await loadMonaco();
  monaco.value = instance;
  if (!host.value) return;

  const font = editorFont();
  model.value = documentFor(props.docId, props.modelValue);
  const editorInstance = instance.editor.create(host.value, {
    model: model.value,
    theme: "klean-dark",
    readOnly: props.readonly,
    automaticLayout: true,
    // Tokenising is what colours the text; the two size guards Monaco applies at
    // model construction (20 MB, 300 k lines) only ever bite on generated files, but
    // a file that hits one would be grey for good — and the guard is not reversible.
    largeFileOptimizations: false,
    fontFamily: font.family,
    fontSize: font.size,
    lineHeight: 18,
    letterSpacing: 0,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    renderLineHighlight: "line",
    // The gutter shows the file's own numbering, not the node's.
    lineNumbers: (line: number) => String(props.startLine + line - 1),
    lineNumbersMinChars: 3,
    glyphMargin: false,
    folding: true,
    padding: { top: 8, bottom: 8 },
    tabSize: 2,
    insertSpaces: true,
    detectIndentation: false,
    smoothScrolling: false,
    // Lean files are wide: the horizontal scrollbar earns its place.
    wordWrap: "off",
    quickSuggestions: { other: true, comments: false, strings: false },
    suggestOnTriggerCharacters: true,
    fixedOverflowWidgets: true,
    overviewRulerLanes: 0,
    stickyScroll: { enabled: false },
    scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10 },
  });
  editor.value = editorInstance;

  editorInstance.onDidChangeModelContent(() => {
    if (quiet || !model.value) return;
    const text = model.value.getValue();
    if (text === lastEmitted) return;
    lastEmitted = text;
    emit("update:modelValue", text);
  });

  editorInstance.onDidChangeCursorSelection(() => {
    const position = editorInstance.getPosition();
    if (position) emit("caret", position.lineNumber, position.column - 1);
    emit("selections", editorInstance.getSelections()?.length ?? 1);
  });

  // Ctrl/Cmd+S is ours: the editor is not a file of its own, it saves a node.
  editorInstance.addCommand(instance.KeyMod.CtrlCmd | instance.KeyCode.KeyS, () =>
    emit("commit"),
  );

  applyKeys();
  applyMarkers();
  registerCompletions();
  registerTexInput();
  watchTexSpace(editorInstance);
  editorInstance.focus();
});

onBeforeUnmount(() => {
  rules?.dispose();
  editor.value?.dispose();
  // Only the documents this editor made: the other modals have their own.
  for (const document of documents.values()) document.dispose();
  documents.clear();
  model.value = null;
});

/**
 * The model for a document, made once and kept.
 *
 * Coming back to a node finds its undo history where it was left; the text is
 * refreshed when the file changed meanwhile.
 */
function documentFor(docId: string, text: string): TextDocument | null {
  const instance = monaco.value;
  if (!instance) return null;
  const existing = documents.get(docId);
  if (existing) {
    if (!existing.isDisposed() && existing.getValue() !== text) {
      quiet = true;
      existing.setValue(text);
      quiet = false;
    }
    return existing.isDisposed() ? null : existing;
  }
  const created = instance.editor.createModel(
    text,
    "lean",
    instance.Uri.parse(uriOf(docId)),
  );
  documents.set(docId, created);
  // Oldest first: a node visited long ago keeps its text and history only while the
  // editor is small enough to carry it.
  while (documents.size > KEEP_DOCUMENTS) {
    const oldest = documents.keys().next().value as string | undefined;
    if (!oldest || oldest === docId) break;
    const stale = documents.get(oldest);
    documents.delete(oldest);
    if (stale && stale !== model.value) stale.dispose();
  }
  return created;
}

/** Points the editor at this document. */
function adopt(docId: string, text: string) {
  const editorInstance = editor.value;
  if (!editorInstance) return;
  if (documents.has(docId) && model.value === documents.get(docId)) {
    documentFor(docId, text);
    return;
  }
  const next = documentFor(docId, text);
  if (!next) return;
  quiet = true;
  editorInstance.setModel(next);
  quiet = false;
  model.value = next;
  lastEmitted = next.getValue();
  applyKeys();
  applyMarkers();
}

/**
 * Binds the configured shortcuts to Monaco's own commands.
 *
 * Everything on this list already exists in Monaco (add a cursor above, copy the
 * line, find, undo…): only the keys are ours, because the settings are the app's,
 * and a second undocumented set of keys beside them is how editors drift.
 */
function applyKeys() {
  const instance = monaco.value;
  if (!instance) return;
  rules?.dispose();
  const bindings: { keybinding: number; command: string }[] = [];
  const bind = (shortcut: string, command: string) => {
    const keybinding = monacoKeybinding(instance, shortcut);
    if (keybinding !== null) bindings.push({ keybinding, command });
  };
  bind(props.keys.undo, "editor.action.undo");
  bind(props.keys.redo, "editor.action.redo");
  bind(props.keys.find, "actions.find");
  bind(props.keys.addOccurrence, "editor.action.addSelectionToNextFindMatch");
  bind(props.keys.allOccurrences, "editor.action.selectHighlights");
  bind(props.keys.cursorAbove, "editor.action.insertCursorAbove");
  bind(props.keys.cursorBelow, "editor.action.insertCursorBelow");
  bind(props.keys.duplicateUp, "editor.action.copyLinesUpAction");
  bind(props.keys.duplicateDown, "editor.action.copyLinesDownAction");
  if (bindings.length > 0) rules = instance.editor.addKeybindingRules(bindings);
}

/** Lean's diagnostics, on the lines of the file they were reported on. */
function applyMarkers() {
  const instance = monaco.value;
  const document = model.value;
  if (!instance || !document) return;
  const lineCount = document.getLineCount();
  const markers = props.markers
    .map((marker) => {
      const first = Math.min(Math.max(1, marker.line - props.startLine + 1), lineCount);
      const last = Math.min(
        Math.max(first, (marker.end_line ?? marker.line) - props.startLine + 1),
        lineCount,
      );
      return {
        startLineNumber: first,
        startColumn: 1,
        endLineNumber: last,
        endColumn: document.getLineMaxColumn(last),
        message: marker.message || marker.severity,
        severity: MARKER_SEVERITY[marker.severity] ?? 2,
      };
    })
    .filter((marker) => marker.severity > 0);
  instance.editor.setModelMarkers(document, "lean", markers);
}

/**
 * Completions, from Lean's own language server.
 *
 * Registered once for the `lean` language, which is app-wide; the document tells it
 * which node is asking (its URI carries the id), so any pane gets the right answer.
 */
function registerCompletions() {
  const instance = monaco.value;
  if (!instance || completionsRegistered) return;
  completionsRegistered = true;
  instance.languages.registerCompletionItemProvider("lean", {
    triggerCharacters: [".", " "],
    provideCompletionItems: async (document, position) => {
      const id = docIdOf(document.uri);
      if (!id) return { suggestions: [] };
      const word = document.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: position.column,
      };
      try {
        const items = await api.leanCompletions(id, position.lineNumber, position.column - 1);
        return {
          suggestions: items.map((item) => ({
            label: item.label,
            kind: (COMPLETION_KIND[item.kind] ?? 18) as import("monaco-editor").languages.CompletionItemKind,
            detail: item.detail || undefined,
            documentation: item.documentation || undefined,
            insertText: item.label,
            range,
          })),
        };
      } catch {
        return { suggestions: [] };
      }
    },
  });
}

/** The node id a document's URI stands for, if it is one of ours. */
function docIdOf(uri: import("monaco-editor").Uri): string | null {
  if (uri.scheme !== "inmemory" || !uri.path.startsWith("/klean/")) return null;
  return decodeURIComponent(uri.path.slice("/klean/".length));
}

/**
 * Lean's LaTeX-style input: `\all`, `\->`, `\alpha` — the way the Lean extension
 * does it, and the way the editor did it before the rewrite.
 *
 * Two halves: typing `\` offers the list of symbols (filtered as the name is typed),
 * and pressing space after a complete name replaces it with the symbol, so a name can
 * also be typed straight through without looking at the list.
 */
function registerTexInput() {
  const instance = monaco.value;
  if (!instance || texRegistered) return;
  texRegistered = true;
  instance.languages.registerCompletionItemProvider("lean", {
    triggerCharacters: ["\\"],
    provideCompletionItems: (document, position) => {
      const before = document.getLineContent(position.lineNumber).slice(0, position.column - 1);
      const prefix = texPrefix(before);
      if (!prefix) return { suggestions: [] };
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: prefix.start + 1,
        endColumn: position.column,
      };
      return {
        suggestions: texSuggestions(prefix.name).map((entry) => ({
          // The list shows both the name and what it becomes.
          label: { label: `\\${entry.name}`, description: entry.symbol },
          filterText: `\\${entry.name}`,
          insertText: entry.symbol,
          detail: entry.name,
          kind: 14 as import("monaco-editor").languages.CompletionItemKind,
          range,
        })),
      };
    },
  });
}

/** `\name` + space becomes its symbol, keeping the space. */
function watchTexSpace(instance: Editor) {
  instance.onKeyDown((event) => {
    const api_ = monaco.value;
    if (!api_ || event.keyCode !== api_.KeyCode.Space) return;
    if (instance.getOption(api_.editor.EditorOption.readOnly)) return;
    const position = instance.getPosition();
    const model_ = instance.getModel();
    if (!position || !model_) return;
    const line = model_.getLineContent(position.lineNumber).slice(0, position.column - 1);
    const completion = texCompletion(line);
    if (!completion) return;
    // The space would otherwise go in after the name: take it, and type the symbol.
    event.preventDefault();
    event.stopPropagation();
    const start = position.column - completion.name.length - 1;
    const text = `${completion.symbol} `;
    const caret = start + text.length;
    instance.executeEdits(
      "klean.tex",
      [
        {
          range: new api_.Range(position.lineNumber, start, position.lineNumber, position.column),
          text,
          forceMoveMarkers: true,
        },
      ],
      [new api_.Selection(position.lineNumber, caret, position.lineNumber, caret)],
    );
  });
}

watch(
  () => props.modelValue,
  (value) => {
    if (value === lastEmitted) return;
    // An edit from elsewhere (a revert, a save): it becomes the new text.
    lastEmitted = value;
    adopt(props.docId, value);
  },
);

watch(
  () => props.docId,
  () => adopt(props.docId, props.modelValue),
);

watch(() => props.markers, applyMarkers, { deep: true });

watch(() => props.keys, applyKeys, { deep: true });

watch(
  () => props.readonly,
  (readonly) => editor.value?.updateOptions({ readOnly: readonly }),
);

defineExpose({
  focus: () => editor.value?.focus(),
});
</script>

<template>
  <div class="code">
    <div ref="host" class="code__monaco" />
  </div>
</template>

<style scoped>
.code {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  transition: border-color var(--t) var(--ease), box-shadow var(--t) var(--ease);
}

.code:focus-within {
  border-color: var(--line-strong);
  box-shadow: 0 0 0 1px var(--accent-soft) inset;
}

.code__monaco {
  flex: 1;
  min-width: 0;
  min-height: 0;
}
</style>
