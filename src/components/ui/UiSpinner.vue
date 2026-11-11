<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 12 });
</script>

<template>
  <span class="ui-spinner" :style="{ width: size + 'px', height: size + 'px' }" />
</template>

<style scoped>
.ui-spinner {
  display: inline-block;
  border: 1.4px solid var(--line-strong);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: ui-spin 720ms linear infinite;
}

@keyframes ui-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
