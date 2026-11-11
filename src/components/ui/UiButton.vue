<script setup lang="ts">
// Small, quiet button. Variants: ghost (default), outline, solid, subtle.
withDefaults(
  defineProps<{
    variant?: "ghost" | "outline" | "solid" | "subtle";
    size?: "sm" | "md";
    disabled?: boolean;
    active?: boolean;
    title?: string;
  }>(),
  { variant: "ghost", size: "md", disabled: false, active: false, title: "" },
);
</script>

<template>
  <button
    class="ui-button"
    :data-variant="variant"
    :data-size="size"
    :data-active="active || undefined"
    :disabled="disabled"
    :title="title || undefined"
    type="button"
  >
    <slot />
  </button>
</template>

<style scoped>
.ui-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-3);
  color: var(--text-dim);
  font-size: var(--fs-sm);
  letter-spacing: 0.01em;
  white-space: nowrap;
  transition:
    background-color var(--t) var(--ease),
    border-color var(--t) var(--ease),
    color var(--t) var(--ease),
    transform var(--t-fast) var(--ease),
    box-shadow var(--t) var(--ease);
}

.ui-button[data-size="sm"] {
  height: 22px;
  padding: 0 8px;
  font-size: var(--fs-xs);
}

.ui-button:hover:not(:disabled) {
  background: var(--bg-4);
  border-color: var(--line-strong);
  color: var(--text);
}

.ui-button:active:not(:disabled) {
  transform: translateY(0.5px);
  background: var(--bg-active);
}

.ui-button:disabled {
  opacity: 0.45;
  cursor: default;
}

.ui-button[data-variant="outline"] {
  background: transparent;
  border-color: var(--line-strong);
}

.ui-button[data-variant="solid"] {
  background: linear-gradient(180deg, #20262a 0%, #191d20 100%);
  border-color: #33404a;
  color: #dfe7ec;
}

.ui-button[data-variant="solid"]:hover:not(:disabled) {
  border-color: var(--accent-line);
  box-shadow: 0 0 0 1px var(--accent-soft) inset;
  color: var(--accent-bright);
}

.ui-button[data-variant="subtle"] {
  border-color: transparent;
  background: transparent;
  color: var(--text-faint);
}

.ui-button[data-variant="subtle"]:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-dim);
}

.ui-button[data-active] {
  border-color: var(--accent-line);
  color: var(--accent-bright);
  background: var(--accent-soft);
}
</style>
