<script setup lang="ts">
// Monochrome 16x16 stroke icons, drawn in `currentColor`.
withDefaults(defineProps<{ name: string; size?: number; stroke?: number }>(), {
  size: 14,
  stroke: 1.4,
});

const PATHS: Record<string, string> = {
  chevron: "M5.5 3.5 L10 8 L5.5 12.5",
  "chevron-down": "M3.5 6 L8 10.5 L12.5 6",
  close: "M4.2 4.2 L11.8 11.8 M11.8 4.2 L4.2 11.8",
  refresh: "M13 8a5 5 0 1 1-1.7-3.8M13.2 2.8v3.4H9.8",
  folder: "M2.5 4.4h3.8l1.2 1.7h6v5.9h-11z",
  archive: "M2.6 3.4h10.8v2.3H2.6zM3.6 5.7h8.8v6.9H3.6zM6.4 8.2h3.2",
  download: "M8 3v7.4M5.1 7.6 L8 10.5 L10.9 7.6M3.4 12.6h9.2",
  upload: "M8 10.8V3.4M5.1 6.3 L8 3.4 L10.9 6.3M3.4 12.6h9.2",
  copy: "M5.6 5.6h6.9v6.9H5.6zM3.5 10.4V3.5h6.9",
  save: "M3.4 3.4h6.8l2.4 2.4v6.8H3.4zM5.6 3.4v2.9h3.8V3.4M5.6 12.6V8.9h4.8v3.7",
  outline: "M2.8 3.8h10.4M2.8 8h10.4M2.8 12.2h6.4",
  dot: "M8 6.8a1.2 1.2 0 1 0 0 2.4 1.2 1.2 0 0 0 0-2.4",
  plus: "M8 4.4v7.2M4.4 8h7.2",
  minus: "M4.4 8h7.2",
  search: "M10.4 10.4 L13.4 13.4M11.6 7.2a4.4 4.4 0 1 1-8.8 0 4.4 4.4 0 0 1 8.8 0",
  layers: "M8 2.6 13.4 5.4 8 8.2 2.6 5.4zM2.6 8.4 8 11.2 13.4 8.4M2.6 11.2 8 14 13.4 11.2",
  code: "M5.6 5.2 2.8 8l2.8 2.8M10.4 5.2 13.2 8l-2.8 2.8",
  drag: "M8 2.8v10.4M4.4 5.6 8 2.8l3.6 2.8M4.4 10.4 8 13.2l3.6-2.8",
  settings:
    "M8 5.9a2.1 2.1 0 1 0 0 4.2 2.1 2.1 0 0 0 0-4.2M8 1.9v1.6M8 12.5v1.6M2.5 8h1.6M11.9 8h1.6M4.1 4.1l1.1 1.1M10.8 10.8l1.1 1.1M11.9 4.1l-1.1 1.1M5.2 10.8l-1.1 1.1",
  check: "M3.4 8.3 L6.4 11.3 L12.6 4.7",
  tools:
    "M6.2 2.6a2.6 2.6 0 0 0 0 5.2 2.6 2.6 0 0 0 0-5.2M4.2 7.1 2.2 13.4h8l-2-6.3M10.6 8.4l2.6 2.6M12.6 7.4l1.6 1.6-3.2 3.2-1.6-1.6z",
  alert: "M8 2.5 L14.4 13.3 H1.6 Z M8 6.3 v3.3 M8 11.5 v0.1",
  // AI: a spark, for the assistant.
  ai: "M8 2.2 9.3 6.4 13.8 8 9.3 9.6 8 13.8 6.7 9.6 2.2 8 6.7 6.4z",
  send: "M2.6 8 13.2 3.2 10.6 13.2 8 9.4z M8 9.4 13.2 3.2",
  bang: "M8 3.2 v5.8 M8 12.4 v0.2",
  "file-plus": "M4 2.6h5.2L12.6 5.9v7.5H4zM9.2 2.6v3.3h3.4M8 8.2v3.2M6.4 9.8h3.2",
  trash: "M3.2 4.6h9.6M6.2 2.6h3.6M4.6 4.6l0.6 8.8h5.6l0.6-8.8M6.8 7.1v3.9M9.2 7.1v3.9",
  eye: "M1.6 8s2.4-4.2 6.4-4.2S14.4 8 14.4 8s-2.4 4.2-6.4 4.2S1.6 8 1.6 8z M8 6.1a1.9 1.9 0 1 0 0 3.8 1.9 1.9 0 0 0 0-3.8",
  "eye-off": "M2 2l12 12M4.7 4.9C2.8 6.2 1.6 8 1.6 8s2.4 4.2 6.4 4.2c1.2 0 2.2-.4 3.1-.9M7.2 3.9c.3 0 .5-.1.8-.1 4 0 6.4 4.2 6.4 4.2s-.5.9-1.4 1.9",
};
</script>

<template>
  <svg
    class="ui-icon"
    :width="size"
    :height="size"
    viewBox="0 0 16 16"
    fill="none"
    :stroke-width="stroke"
    stroke="currentColor"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <path :d="PATHS[name] ?? PATHS.dot" />
  </svg>
</template>

<style scoped>
.ui-icon {
  display: block;
  flex: none;
  transition: transform var(--t) var(--ease), opacity var(--t) var(--ease);
}
</style>
