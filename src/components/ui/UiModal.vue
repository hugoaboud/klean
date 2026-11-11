<script setup lang="ts">
// Draggable modal surface. Position is kept per open cycle: it re-centres when
// the modal opens and can be dragged by its title bar afterwards.
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import UiIcon from "./UiIcon.vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title?: string;
    subtitle?: string;
    width?: number;
    height?: number;
    /** Extra label shown next to the title, e.g. the node kind. */
    badge?: string;
    /** Show a corner grip that lets the user resize the modal. */
    resizable?: boolean;
  }>(),
  { title: "", subtitle: "", width: 780, height: 580, badge: "", resizable: false },
);

const emit = defineEmits<{ close: [] }>();

const pos = reactive({ x: 0, y: 0 });
const dragging = ref(false);
const size = reactive({ w: 0, h: 0 });
const drag = { x: 0, y: 0, ox: 0, oy: 0 };

function measure() {
  size.w = Math.min(props.width, Math.max(360, window.innerWidth - 96));
  size.h = Math.min(props.height, Math.max(260, window.innerHeight - 96));
}

function centre() {
  measure();
  pos.x = Math.round((window.innerWidth - size.w) / 2);
  pos.y = Math.round((window.innerHeight - size.h) / 2);
}

function onPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  dragging.value = true;
  drag.x = event.clientX;
  drag.y = event.clientY;
  drag.ox = pos.x;
  drag.oy = pos.y;
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  event.preventDefault();
}

function onPointerMove(event: PointerEvent) {
  if (!dragging.value) return;
  const maxX = window.innerWidth - Math.min(size.w, 240);
  const maxY = window.innerHeight - 44;
  pos.x = Math.min(Math.max(0, drag.ox + event.clientX - drag.x), maxX);
  pos.y = Math.min(Math.max(0, drag.oy + event.clientY - drag.y), maxY);
}

function onPointerUp() {
  dragging.value = false;
  detach();
}

function detach() {
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

function onResizeDown(event: PointerEvent) {
  if (event.button !== 0) return;
  event.preventDefault();
  event.stopPropagation();
  const startX = event.clientX;
  const startY = event.clientY;
  const startW = size.w;
  const startH = size.h;

  const move = (moveEvent: PointerEvent) => {
    size.w = Math.max(360, Math.min(window.innerWidth - 24, startW + moveEvent.clientX - startX));
    size.h = Math.max(240, Math.min(window.innerHeight - 24, startH + moveEvent.clientY - startY));
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      centre();
      window.addEventListener("keydown", onKeydown);
    } else {
      window.removeEventListener("keydown", onKeydown);
      detach();
      dragging.value = false;
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  detach();
});

const style = computed(() => ({
  left: pos.x + "px",
  top: pos.y + "px",
  width: size.w + "px",
  height: size.h + "px",
}));
</script>

<template>
  <Transition name="k-modal">
    <div v-if="open" class="ui-modal" :style="style" :data-dragging="dragging || undefined" role="dialog">
      <header class="ui-modal__bar" @pointerdown="onPointerDown">
        <span class="ui-modal__grip"><UiIcon name="drag" :size="11" /></span>
        <div class="ui-modal__id">
          <span v-if="badge" class="k-kind" :data-kind="badge">{{ badge }}</span>
          <span class="ui-modal__title">{{ title }}</span>
          <span v-if="subtitle" class="ui-modal__subtitle k-mono">{{ subtitle }}</span>
        </div>
        <div class="ui-modal__actions" @pointerdown.stop>
          <slot name="actions" />
          <button class="ui-modal__close" type="button" title="Close (Esc)" @click="emit('close')">
            <UiIcon name="close" :size="11" />
          </button>
        </div>
      </header>
      <div class="ui-modal__body">
        <slot />
      </div>
      <footer v-if="$slots.footer" class="ui-modal__footer">
        <slot name="footer" />
      </footer>
      <span
        v-if="resizable"
        class="ui-modal__resize"
        title="Drag to resize"
        @pointerdown="onResizeDown"
      />
    </div>
  </Transition>
</template>

<style scoped>
.ui-modal {
  position: fixed;
  z-index: 40;
  display: flex;
  flex-direction: column;
  min-width: 360px;
  min-height: 240px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-lg);
  background: linear-gradient(180deg, #14171a 0%, #0e1012 100%);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.03) inset,
    0 36px 80px -40px rgba(0, 0, 0, 0.95),
    0 0 0 1px rgba(0, 0, 0, 0.4);
  overflow: hidden;
}

.ui-modal[data-dragging] {
  border-color: var(--accent-line);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.03) inset,
    0 44px 90px -40px rgba(0, 0, 0, 0.98),
    0 0 0 1px var(--accent-soft);
}

.ui-modal__bar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 34px;
  padding: 0 8px 0 7px;
  border-bottom: 1px solid var(--line);
  background: var(--grad-header);
  cursor: grab;
  user-select: none;
}

.ui-modal[data-dragging] .ui-modal__bar {
  cursor: grabbing;
}

.ui-modal__grip {
  color: var(--text-ghost);
  display: flex;
  padding: 2px 1px;
  transition: color var(--t) var(--ease);
}

.ui-modal__bar:hover .ui-modal__grip {
  color: var(--text-faint);
}

.ui-modal__id {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.ui-modal__title {
  font-size: var(--fs-md);
  color: var(--text);
  white-space: nowrap;
}

.ui-modal__subtitle {
  color: var(--text-ghost);
  font-size: var(--fs-xs);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ui-modal__actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 6px;
}

.ui-modal__close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 2px;
  color: var(--text-faint);
  transition: background-color var(--t) var(--ease), color var(--t) var(--ease);
}

.ui-modal__close:hover {
  background: var(--danger-soft);
  color: var(--danger);
}

.ui-modal__body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.ui-modal__footer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-top: 1px solid var(--line);
  background: linear-gradient(180deg, #121517 0%, #0f1113 100%);
}

.ui-modal__resize {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 16px;
  height: 16px;
  cursor: nwse-resize;
  background:
    linear-gradient(135deg, transparent 46%, var(--line-strong) 46%, var(--line-strong) 54%, transparent 54%),
    linear-gradient(135deg, transparent 66%, var(--line) 66%, var(--line) 74%, transparent 74%);
  transition: filter var(--t) var(--ease);
}

.ui-modal__resize:hover {
  filter: brightness(1.5);
}

/* Enter/leave: a whisper of motion, no bounce. */
.k-modal-enter-active,
.k-modal-leave-active {
  transition:
    opacity var(--t) var(--ease),
    transform var(--t) var(--ease);
}

.k-modal-enter-from,
.k-modal-leave-to {
  opacity: 0;
  transform: translateY(6px) scale(0.994);
}
</style>
