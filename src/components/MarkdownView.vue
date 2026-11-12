<script setup lang="ts">
// Model output as a preview: markdown through `marked`, diagrams through mermaid.
//
// Everything the model writes is untrusted, so the HTML is sanitised in
// `lib/markdown.ts` before it reaches `v-html`, and a diagram that does not parse
// falls back to its own source.
import { computed } from "vue";
import { renderMarkdown, splitBlocks } from "../lib/markdown";
import MermaidDiagram from "./MermaidDiagram.vue";

const props = withDefaults(
  defineProps<{
    text: string;
    /** False while the answer is still streaming; diagrams wait for the end. */
    done?: boolean;
  }>(),
  { done: true },
);

const blocks = computed(() =>
  splitBlocks(props.text).map((block) => ({
    ...block,
    html: block.kind === "markdown" ? renderMarkdown(block.text) : "",
  })),
);
</script>

<template>
  <div class="md">
    <template v-for="(block, index) in blocks" :key="index">
      <MermaidDiagram v-if="block.kind === 'mermaid'" :source="block.text" :done="done" />
      <div v-else class="md__html" v-html="block.html" />
    </template>
  </div>
</template>

<style scoped>
.md {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--text-dim);
  /* Every size inside is relative to this, so `--md-scale` enlarges the whole
     block — prose, headings and code together — instead of only the paragraphs. */
  font-size: calc(var(--fs-sm) * var(--md-scale, 1));
  line-height: 1.62;
}

.md__html :deep(> :first-child) {
  margin-top: 0;
}

.md__html :deep(> :last-child) {
  margin-bottom: 0;
}

.md__html :deep(p) {
  margin: 0 0 6px;
}

.md__html :deep(h1),
.md__html :deep(h2),
.md__html :deep(h3),
.md__html :deep(h4) {
  margin: 8px 0 4px;
  color: var(--text);
  font-size: 1.12em;
  font-weight: 600;
  letter-spacing: 0.01em;
}

.md__html :deep(h1) {
  font-size: 1.34em;
}

.md__html :deep(ul),
.md__html :deep(ol) {
  margin: 0 0 6px;
  padding-left: 18px;
}

.md__html :deep(li) {
  margin: 1px 0;
}

.md__html :deep(code) {
  padding: 0 3px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.05);
  font-family: var(--mono);
  font-size: 0.92em;
}

.md__html :deep(.md-code) {
  margin: 4px 0;
  padding: 7px 9px;
  overflow: auto;
  border: 1px solid var(--line-soft);
  border-radius: 2px;
  background: #0b0d0f;
  white-space: pre;
}

.md__html :deep(.md-code code) {
  padding: 0;
  background: none;
  font-size: 1em;
  line-height: 1.5;
}

.md__html :deep(blockquote) {
  margin: 4px 0;
  padding: 2px 9px;
  border-left: 2px solid var(--line-strong);
  color: var(--text-faint);
}

.md__html :deep(a) {
  color: var(--accent-bright);
  text-decoration: none;
}

.md__html :deep(a:hover) {
  text-decoration: underline;
}

.md__html :deep(table) {
  border-collapse: collapse;
  margin: 4px 0;
}

.md__html :deep(th),
.md__html :deep(td) {
  padding: 3px 7px;
  border: 1px solid var(--line-soft);
}

.md__html :deep(th) {
  color: var(--text-faint);
  font-weight: 600;
}

.md__html :deep(hr) {
  border: none;
  border-top: 1px solid var(--line);
  margin: 8px 0;
}
</style>
