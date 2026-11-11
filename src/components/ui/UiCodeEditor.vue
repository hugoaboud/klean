<script setup lang="ts">
// Code editor: a transparent textarea over a highlighted <pre>, with a gutter
// that marks diagnostic lines and a soft highlight per problem line.
import { computed, nextTick, ref, watch } from "vue";
import hljs from "../../lib/highlight";
import { texCompletion } from "../../lib/lean-tex";

export interface EditorMarker {
  line: number;
  severity: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    language?: string;
    readonly?: boolean;
    placeholder?: string;
    markers?: EditorMarker[];
  }>(),
  { language: "lean", readonly: false, placeholder: "", markers: () => [] },
);

const emit = defineEmits<{
  "update:modelValue": [string];
  commit: [];
}>();

/** Must match the editor CSS: line height and top padding in pixels. */
const LINE_HEIGHT = 18;
const PAD_TOP = 10;

const input = ref<HTMLTextAreaElement | null>(null);
const highlightLayer = ref<HTMLElement | null>(null);
const markerLayer = ref<HTMLElement | null>(null);

const highlighted = computed(() => {
  try {
    return hljs.highlight(props.modelValue, {
      language: props.language,
      ignoreIllegals: true,
    }).value;
  } catch {
    return escapeHtml(props.modelValue);
  }
});

const markers = computed(() =>
  props.markers
    .filter((marker) => marker.line >= 1)
    .map((marker) => ({
      ...marker,
      top: PAD_TOP + (marker.line - 1) * LINE_HEIGHT,
    })),
);

function escapeHtml(text: string) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function onInput(event: Event) {
  const target = event.target as HTMLTextAreaElement;
  emit("update:modelValue", target.value);
}

function syncScroll() {
  if (!input.value) return;
  const top = input.value.scrollTop;
  if (highlightLayer.value) {
    highlightLayer.value.scrollTop = top;
    highlightLayer.value.scrollLeft = input.value.scrollLeft;
  }
  if (markerLayer.value) markerLayer.value.style.transform = `translateY(${-top}px)`;
}

function onKeydown(event: KeyboardEvent) {
  if (!props.readonly && (event.key === " " || event.key === "Tab")) {
    const target = event.target as HTMLTextAreaElement;
    const before = target.value.slice(0, target.selectionStart);
    const completion = texCompletion(before);
    if (completion && event.key === " ") {
      // `\alpha` + space becomes `α `, like the Lean VS Code extension.
      event.preventDefault();
      const start = target.selectionStart - completion.name.length - 1;
      const next =
        target.value.slice(0, start) + completion.symbol + " " + target.value.slice(target.selectionEnd);
      emit("update:modelValue", next);
      nextTick(() => {
        target.selectionStart = target.selectionEnd = start + completion.symbol.length + 1;
      });
      return;
    }
  }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    emit("commit");
    return;
  }
  if (event.key === "Tab" && !props.readonly) {
    event.preventDefault();
    const target = event.target as HTMLTextAreaElement;
    const start = target.selectionStart;
    const end = target.selectionEnd;
    const value = target.value;
    const next = value.slice(0, start) + "  " + value.slice(end);
    emit("update:modelValue", next);
    nextTick(() => {
      target.selectionStart = target.selectionEnd = start + 2;
    });
  }
}

watch(
  () => props.modelValue,
  async () => {
    await nextTick();
    syncScroll();
  },
);

function focus() {
  input.value?.focus();
}

defineExpose({ focus });
</script>

<template>
  <div class="editor" :data-readonly="readonly || undefined">
    <div ref="markerLayer" class="editor__markers" aria-hidden="true">
      <div
        v-for="(marker, index) in markers"
        :key="`${marker.line}-${index}`"
        class="editor__line"
        :data-severity="marker.severity"
        :style="{ top: marker.top + 'px', height: LINE_HEIGHT + 'px' }"
      />
      <div
        v-for="(marker, index) in markers"
        :key="`icon-${marker.line}-${index}`"
        class="editor__mark"
        :data-severity="marker.severity"
        :title="marker.severity"
        :style="{ top: marker.top + 'px', height: LINE_HEIGHT + 'px' }"
      >
        <svg width="9" height="9" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M8 3.4v6M8 12.6v0.2" />
        </svg>
      </div>
    </div>

    <div ref="highlightLayer" class="editor__layer" aria-hidden="true">
      <pre class="editor__pre"><code class="hljs" v-html="highlighted + '\n'" /></pre>
    </div>

    <textarea
      ref="input"
      class="editor__input"
      :value="modelValue"
      :readonly="readonly"
      :placeholder="placeholder"
      spellcheck="false"
      autocapitalize="off"
      autocomplete="off"
      wrap="off"
      @input="onInput"
      @scroll="syncScroll"
      @keydown="onKeydown"
    />
  </div>
</template>

<style scoped>
.editor {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background: #0c0e10;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  transition: border-color var(--t) var(--ease), box-shadow var(--t) var(--ease);
}

.editor:focus-within {
  border-color: var(--line-strong);
  box-shadow: 0 0 0 1px var(--accent-soft) inset;
}

.editor__markers,
.editor__layer {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.editor__layer {
  overflow: hidden;
}

.editor__markers {
  z-index: 3;
  overflow: hidden;
}

/* A problem line: a soft wash across the code and a marker in the gutter. */
.editor__line {
  position: absolute;
  left: 26px;
  right: 0;
  background: rgba(201, 115, 107, 0.075);
  border-left: 1px solid rgba(201, 115, 107, 0.3);
}

.editor__line[data-severity="warning"] {
  background: rgba(194, 166, 112, 0.06);
  border-left-color: rgba(194, 166, 112, 0.3);
}

.editor__mark {
  position: absolute;
  left: 0;
  width: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--danger);
}

.editor__mark[data-severity="warning"] {
  color: var(--warning);
}

.editor__pre,
.editor__input {
  margin: 0;
  padding: 10px 12px 10px 32px;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 18px;
  tab-size: 2;
  white-space: pre;
  word-wrap: normal;
  letter-spacing: 0;
}

.editor__pre {
  min-width: 100%;
  min-height: 100%;
  color: var(--text);
}

.editor__pre code {
  display: block;
  background: none;
  padding: 0;
  font: inherit;
}

.editor__input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: transparent;
  caret-color: var(--accent-bright);
  overflow: auto;
}

.editor__input::selection {
  background: rgba(106, 169, 196, 0.22);
  color: transparent;
}

.editor[data-readonly] .editor__input {
  caret-color: transparent;
  cursor: default;
}
</style>
