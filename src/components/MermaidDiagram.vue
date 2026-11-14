<script setup lang="ts">
// A mermaid diagram, drawn from a fence in the model's answer.
//
// Mermaid is loaded the first time a diagram appears (it is a large library, and
// most sessions never draw one) and rendered to SVG. While the answer is still
// streaming the source is shown instead: re-rendering on every token would make
// the feed stutter for no benefit.
import { onBeforeUnmount, ref, watch } from "vue";

const props = withDefaults(
  defineProps<{
    source: string;
    /** False while the text is still arriving. */
    done?: boolean;
  }>(),
  { done: true },
);

const svg = ref("");
const error = ref<string | null>(null);
const rendering = ref(false);
let sequence = 0;

/** Mermaid needs a theme that matches the board, set once. */
async function renderer() {
  const module = await import("mermaid");
  const mermaid = module.default;
  mermaid.initialize({
    startOnLoad: false,
    theme: "dark",
    securityLevel: "strict",
    fontFamily: "var(--mono, monospace)",
    themeVariables: {
      background: "#0b0d0f",
      primaryColor: "#1b2126",
      primaryTextColor: "#c9d4da",
      primaryBorderColor: "#3d4a52",
      lineColor: "#5d6b74",
      secondaryColor: "#161a1d",
      tertiaryColor: "#121517",
    },
  });
  return mermaid;
}

async function draw() {
  if (!props.done || !props.source.trim()) {
    svg.value = "";
    return;
  }
  const token = ++sequence;
  rendering.value = true;
  try {
    const mermaid = await renderer();
    const result = await mermaid.render(`klean-mermaid-${token}`, props.source);
    if (token !== sequence) return;
    svg.value = result.svg;
    error.value = null;
  } catch (failure) {
    if (token !== sequence) return;
    // A model can write a diagram that does not parse: show what it wrote.
    svg.value = "";
    error.value = failure instanceof Error ? failure.message : String(failure);
  } finally {
    if (token === sequence) rendering.value = false;
  }
}

watch(() => [props.source, props.done] as const, draw, { immediate: true });

onBeforeUnmount(() => {
  sequence += 1;
});
</script>

<template>
  <div class="mermaid">
    <div v-if="svg && done" class="mermaid__svg" v-html="svg" />
    <pre v-else-if="done && error" class="mermaid__error k-mono">{{ source }}</pre>
    <pre v-else class="mermaid__source k-mono">{{ source }}</pre>
    <span v-if="rendering" class="mermaid__hint">drawing…</span>
  </div>
</template>

<style scoped>
.mermaid {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 6px 8px;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
  background: #0b0d0f;
  overflow: auto;
}

.mermaid__svg {
  display: flex;
  justify-content: center;
}

.mermaid__svg :deep(svg) {
  max-width: 100%;
  height: auto;
}

.mermaid__source,
.mermaid__error {
  margin: 0;
  color: var(--text-ghost);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
}

.mermaid__error::after {
  content: " · could not be drawn, showing the source";
  color: var(--warning);
}

.mermaid__hint {
  color: var(--text-ghost);
  font-size: var(--fs-micro);
}
</style>
