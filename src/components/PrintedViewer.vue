<script setup lang="ts">
// Read-only viewer for Lean printed by the Rust printer.
import { computed } from "vue";
import type { Printed } from "../store/workspace";
import UiButton from "./ui/UiButton.vue";
import CodeEditor from "./ui/CodeEditor.vue";
import UiIcon from "./ui/UiIcon.vue";

const props = defineProps<{ printed: Printed | null }>();
const emit = defineEmits<{ save: []; copy: []; close: [] }>();

const lines = computed(() => props.printed?.text.split("\n").length ?? 0);
</script>

<template>
  <div v-if="printed" class="printed">
    <div class="printed__bar">
      <span class="k-label">{{ lines }} lines</span>
      <span class="k-spacer" />
      <UiButton size="sm" variant="ghost" @click="emit('copy')">
        <UiIcon name="copy" :size="11" /> copy
      </UiButton>
      <UiButton size="sm" variant="ghost" @click="emit('save')">
        <UiIcon name="save" :size="11" /> save as .lean
      </UiButton>
    </div>
    <div class="printed__body">
      <CodeEditor :model-value="printed.text" :doc-id="printed.title" language="lean" readonly />
    </div>
  </div>
</template>

<style scoped>
.printed {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

.printed__bar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 10px;
  border-bottom: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.012);
}

.printed__body {
  display: flex;
  flex: 1;
  min-height: 0;
  padding: 8px 10px 10px;
}
</style>
