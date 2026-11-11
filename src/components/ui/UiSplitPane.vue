<script setup lang="ts">
// Two-column layout with a draggable divider. The ratio is a 0..1 fraction of
// the container width owned by the left pane.
import { computed, onBeforeUnmount, ref } from "vue";

const props = withDefaults(
  defineProps<{ left: number; min?: number; max?: number }>(),
  { min: 0.18, max: 0.82 },
);
const emit = defineEmits<{ "update:left": [number] }>();

const host = ref<HTMLElement | null>(null);
const dragging = ref(false);

function onPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  dragging.value = true;
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  event.preventDefault();
}

function onPointerMove(event: PointerEvent) {
  if (!dragging.value || !host.value) return;
  const rect = host.value.getBoundingClientRect();
  if (rect.width <= 0) return;
  const ratio = (event.clientX - rect.left) / rect.width;
  emit("update:left", Math.min(props.max, Math.max(props.min, ratio)));
}

function onPointerUp() {
  dragging.value = false;
  detach();
}

function detach() {
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
}

onBeforeUnmount(detach);

const leftStyle = computed(() => ({ flexBasis: props.left * 100 + "%" }));
</script>

<template>
  <div ref="host" class="ui-split" :data-dragging="dragging || undefined">
    <div class="ui-split__pane" :style="leftStyle">
      <slot name="left" />
    </div>
    <div class="ui-split__handle" role="separator" @pointerdown="onPointerDown">
      <span class="ui-split__grip" />
    </div>
    <div class="ui-split__pane ui-split__pane--right">
      <slot name="right" />
    </div>
  </div>
</template>

<style scoped>
.ui-split {
  display: flex;
  align-items: stretch;
  height: 100%;
  min-height: 0;
}

.ui-split__pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  flex-shrink: 0;
}

.ui-split__pane--right {
  flex: 1 1 0;
}

.ui-split__handle {
  position: relative;
  flex: none;
  width: 1px;
  background: var(--line);
  cursor: col-resize;
  transition: background-color var(--t) var(--ease);
}

.ui-split__handle::before {
  content: "";
  position: absolute;
  inset: 0 -5px;
}

.ui-split__grip {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 3px;
  height: 34px;
  transform: translate(-50%, -50%);
  border-radius: 2px;
  background: var(--line-strong);
  opacity: 0;
  transition: opacity var(--t) var(--ease), background-color var(--t) var(--ease);
}

.ui-split__handle:hover .ui-split__grip,
.ui-split[data-dragging] .ui-split__grip {
  opacity: 1;
  background: var(--accent);
}

.ui-split[data-dragging] .ui-split__handle {
  background: var(--accent-line);
}
</style>
