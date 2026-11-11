<script setup lang="ts">
// Bottom-centred switcher between the board views.
import type { BoardView } from "../../store/workspace";

defineProps<{ mode: BoardView }>();
const emit = defineEmits<{ change: [BoardView] }>();
</script>

<template>
  <div class="snackbar" role="tablist" aria-label="Board view">
    <button
      class="snackbar__item"
      type="button"
      role="tab"
      :aria-selected="mode === 'ast'"
      :data-active="mode === 'ast' || undefined"
      @click="emit('change', 'ast')"
    >
      AST
    </button>
    <span class="snackbar__sep" />
    <button
      class="snackbar__item"
      type="button"
      role="tab"
      :aria-selected="mode === 'logic'"
      :data-active="mode === 'logic' || undefined"
      @click="emit('change', 'logic')"
    >
      Logic
    </button>
  </div>
</template>

<style scoped>
.snackbar {
  position: absolute;
  left: 50%;
  bottom: 18px;
  transform: translateX(-50%);
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px;
  border: 1px solid var(--line-strong);
  border-radius: 999px;
  background: linear-gradient(180deg, #1b1f22 0%, #14181a 100%);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.03) inset,
    0 18px 40px -24px rgba(0, 0, 0, 0.95);
}

.snackbar__item {
  min-width: 74px;
  height: 26px;
  padding: 0 16px;
  border-radius: 999px;
  color: var(--text-faint);
  font-size: var(--fs-sm);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  transition:
    background-color var(--t) var(--ease),
    color var(--t) var(--ease);
}

.snackbar__item:hover {
  color: var(--text);
  background: rgba(255, 255, 255, 0.03);
}

.snackbar__item[data-active] {
  color: var(--accent-bright);
  background: var(--accent-soft);
  box-shadow: 0 0 0 1px var(--accent-line) inset;
}

.snackbar__sep {
  width: 1px;
  height: 14px;
  background: var(--line);
}
</style>
