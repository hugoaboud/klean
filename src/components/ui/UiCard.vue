<script setup lang="ts">
// Generic raised surface used by the welcome screen and the editor modal.
withDefaults(defineProps<{ padding?: "none" | "sm" | "md" | "lg"; tone?: "panel" | "raised" }>(), {
  padding: "md",
  tone: "panel",
});
</script>

<template>
  <section class="ui-card" :data-padding="padding" :data-tone="tone">
    <slot />
  </section>
</template>

<style scoped>
.ui-card {
  position: relative;
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  background: var(--grad-panel);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.02) inset,
    0 16px 40px -28px rgba(0, 0, 0, 0.9);
}

.ui-card[data-tone="raised"] {
  background: var(--grad-raised);
  border-color: var(--line-strong);
}

.ui-card[data-padding="none"] {
  padding: 0;
}

.ui-card[data-padding="sm"] {
  padding: 10px 12px;
}

.ui-card[data-padding="md"] {
  padding: 18px 20px;
}

.ui-card[data-padding="lg"] {
  padding: 28px 30px;
}
</style>
