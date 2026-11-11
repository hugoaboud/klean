<script setup lang="ts">
// Small modal asking for one line of text, used for file creation.
import { ref, watch } from "vue";
import UiButton from "./UiButton.vue";
import UiModal from "./UiModal.vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    label?: string;
    placeholder?: string;
    initial?: string;
    confirmLabel?: string;
    danger?: boolean;
    message?: string;
  }>(),
  { label: "", placeholder: "", initial: "", confirmLabel: "create", danger: false, message: "" },
);

const emit = defineEmits<{ cancel: []; confirm: [string] }>();

const value = ref("");

watch(
  () => props.open,
  (open) => {
    if (open) value.value = props.initial;
  },
  { immediate: true },
);

function submit() {
  const text = value.value.trim();
  if (!text) return;
  emit("confirm", text);
}

function confirmEmpty() {
  emit("confirm", "");
}
</script>

<template>
  <UiModal :open="open" :title="title" :width="440" :height="186" @close="emit('cancel')">
    <div class="prompt">
      <p v-if="message" class="prompt__message">{{ message }}</p>
      <label v-if="!message" class="prompt__field">
        <span class="k-label">{{ label }}</span>
        <input
          v-model="value"
          class="prompt__input k-mono"
          :placeholder="placeholder"
          spellcheck="false"
          autofocus
          @keydown.enter.prevent="submit"
        />
      </label>
      <div class="prompt__actions">
        <UiButton size="sm" variant="ghost" @click="emit('cancel')">cancel</UiButton>
        <UiButton
          size="sm"
          :variant="danger ? 'outline' : 'solid'"
          :class="{ 'prompt__danger': danger }"
          @click="message ? confirmEmpty() : submit()"
        >
          {{ confirmLabel }}
        </UiButton>
      </div>
    </div>
  </UiModal>
</template>

<style scoped>
.prompt {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px 16px;
}

.prompt__message {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-md);
  line-height: 1.6;
}

.prompt__field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.prompt__input {
  height: 28px;
  padding: 0 9px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg-1);
  color: var(--text);
  font-size: var(--fs-sm);
  transition: border-color var(--t) var(--ease), background-color var(--t) var(--ease);
}

.prompt__input:focus {
  border-color: var(--accent-line);
  background: var(--bg-2);
}

.prompt__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.prompt__danger {
  border-color: rgba(201, 115, 107, 0.4);
  color: var(--danger);
}
</style>
